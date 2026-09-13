/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::platform::PlatformQuotaProvider;
use crate::quota::{current_unix_nano, read_quota_file};
use agent_otel_core::platform::{ClaudeCodeDescriptor, PlatformDescriptor};
use agent_otel_core::quota::QuotaSnapshot;
use std::path::{Path, PathBuf};

pub struct ClaudeQuotaProvider;

impl PlatformDescriptor for ClaudeQuotaProvider {
    fn id(&self) -> &'static str {
        ClaudeCodeDescriptor.id()
    }
    fn display_name(&self) -> &'static str {
        ClaudeCodeDescriptor.display_name()
    }
    fn aliases(&self) -> &'static [&'static str] {
        ClaudeCodeDescriptor.aliases()
    }
    fn wire_client_id(&self) -> u8 {
        ClaudeCodeDescriptor.wire_client_id()
    }
    fn pre_tool_response(&self) -> agent_otel_core::platform::HookResponse {
        ClaudeCodeDescriptor.pre_tool_response()
    }
}

impl PlatformQuotaProvider for ClaudeQuotaProvider {
    fn is_installed(&self, home: &Path) -> bool {
        home.join(".claude").exists() || std::env::var("ANTHROPIC_API_KEY").is_ok()
    }

    fn harvest_quota(&self, home: &Path) -> Option<QuotaSnapshot> {
        let direct_file = home.join(".claude").join("quota.json");
        if direct_file.exists() {
            if let Some(snap) = read_quota_file(&direct_file) {
                return Some(snap);
            }
        }

        detect_claude_session_quota(home)
    }

    fn fallback_baseline(&self) -> QuotaSnapshot {
        QuotaSnapshot {
            remaining_fraction: 0.75,
            seconds_to_reset: 3600.0,
            observed_at_unix_nano: current_unix_nano(),
            bucket: "claude".to_string(),
            group: "anthropic".to_string(),
        }
    }

    fn burn_per_token(&self) -> f64 {
        1.0 / 20_000_000.0
    }
}

fn detect_claude_session_quota(home: &Path) -> Option<QuotaSnapshot> {
    let projects_dir = home.join(".claude").join("projects");
    if !projects_dir.is_dir() {
        return None;
    }

    let mut latest_file: Option<(PathBuf, std::time::SystemTime)> = None;
    if let Ok(entries) = std::fs::read_dir(&projects_dir) {
        for proj in entries.flatten() {
            let p_path = proj.path();
            if p_path.is_dir() {
                if let Ok(files) = std::fs::read_dir(&p_path) {
                    for f in files.flatten() {
                        let path = f.path();
                        if path.extension().map(|e| e == "jsonl").unwrap_or(false) {
                            if let Ok(meta) = path.metadata() {
                                if let Ok(modified) = meta.modified() {
                                    if latest_file
                                        .as_ref()
                                        .map(|(_, t)| modified > *t)
                                        .unwrap_or(true)
                                    {
                                        latest_file = Some((path, modified));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let (file_path, _) = latest_file?;
    let file = std::fs::File::open(&file_path).ok()?;
    use std::io::{BufRead, BufReader};
    let reader = BufReader::new(file);
    let mut last_quota_limit: Option<(String, f64)> = None;
    let mut last_tokens_left: Option<u64> = None;
    let mut has_recent_messages = false;

    let now_sec = (current_unix_nano() / 1_000_000_000) as f64;

    for line in reader.lines().map_while(Result::ok) {
        if line.contains("total_tokens_reminder") || line.contains("tokens left") {
            if let Some(pos) = line.find("<total_tokens>") {
                let after = &line[pos + 14..];
                if let Some(end_pos) = after.find(" tokens left") {
                    if let Ok(num) = after[..end_pos].trim().parse::<u64>() {
                        last_tokens_left = Some(num);
                    }
                }
            }
        }

        if line.contains("quotaLimits") {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) {
                if let Some(ql) = val.get("quotaLimits") {
                    let status = ql
                        .get("status")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    let resets_at = ql.get("resetsAt").and_then(|r| r.as_f64()).unwrap_or(0.0);
                    last_quota_limit = Some((status, resets_at));
                }
            }
        }

        if line.contains("\"role\":\"assistant\"") || line.contains("\"type\":\"assistant\"") {
            has_recent_messages = true;
        }
    }

    let mut remaining_fraction = 0.75;
    let mut seconds_to_reset = 3600.0;

    if let Some((status, resets_at)) = last_quota_limit {
        let diff = resets_at - now_sec;
        if diff > 0.0 && status == "rejected" {
            remaining_fraction = 0.0;
            seconds_to_reset = diff;
            return Some(QuotaSnapshot {
                remaining_fraction,
                seconds_to_reset,
                observed_at_unix_nano: current_unix_nano(),
                bucket: "claude".to_string(),
                group: "anthropic".to_string(),
            });
        }
    }

    if let Some(tokens) = last_tokens_left {
        remaining_fraction = ((tokens as f64) / 20_000_000.0).clamp(0.05, 1.0);
        seconds_to_reset = 3600.0;
    } else if has_recent_messages {
        remaining_fraction = 0.75;
        seconds_to_reset = 3600.0;
    }

    Some(QuotaSnapshot {
        remaining_fraction,
        seconds_to_reset,
        observed_at_unix_nano: current_unix_nano(),
        bucket: "claude".to_string(),
        group: "anthropic".to_string(),
    })
}
