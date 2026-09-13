/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::platform::PlatformQuotaProvider;
use agent_otel_core::quota::QuotaSnapshot;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

pub struct QuotaEngine {
    state_file: Option<PathBuf>,
    quotas: RwLock<HashMap<String, QuotaSnapshot>>,
    boot_time_ns: u64,
}

impl Default for QuotaEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl QuotaEngine {
    pub fn new() -> Self {
        let state_file = std::env::var("AGENT_OTEL_QUOTA_FILE")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                dirs_fallback().and_then(|home| {
                    let candidates = [
                        home.join(".state").join("quota.json"),
                        home.join(".gemini").join("quota.json"),
                        home.join(".agent-otel").join("quota.json"),
                    ];
                    candidates.into_iter().find(|p| p.exists())
                })
            });

        let now = current_unix_nano();
        let mut map = HashMap::new();

        // Dynamically initialize only the platforms installed on this machine
        if let Some(ref home) = dirs_fallback() {
            for provider in crate::platforms::detect_installed_providers(home) {
                let snap = provider
                    .harvest_quota(home)
                    .unwrap_or_else(|| provider.fallback_baseline());
                map.insert(snap.bucket.clone(), snap);
            }
        }

        // If no provider is detected on machine, populate default fallback baseline to guarantee non-empty headroom
        if map.is_empty() {
            let default_provider = &crate::platforms::GeminiQuotaProvider;
            let snap = default_provider.fallback_baseline();
            map.insert(snap.bucket.clone(), snap);
        }

        Self {
            state_file,
            quotas: RwLock::new(map),
            boot_time_ns: now,
        }
    }

    /// Records agent activity to dynamically adjust quota headroom and reset counters.
    pub fn record_activity(&self, agent_name: &str, tokens_used: Option<u64>) {
        let provider = crate::platforms::find_provider_by_name(agent_name);
        let (bucket, burn_per_token, group) = if let Some(p) = provider {
            let base = p.fallback_baseline();
            (base.bucket, p.burn_per_token(), base.group)
        } else {
            (
                agent_name.to_ascii_lowercase(),
                1.0 / 500_000.0,
                "ai-agent".to_string(),
            )
        };

        let now = current_unix_nano();
        if let Ok(mut lock) = self.quotas.write() {
            let entry = lock.entry(bucket.clone()).or_insert_with(|| QuotaSnapshot {
                remaining_fraction: 0.95,
                seconds_to_reset: 3600.0,
                observed_at_unix_nano: now,
                bucket: bucket.clone(),
                group,
            });

            // Adjust remaining fraction based on consumed tokens or turn activity
            let burn = if let Some(t) = tokens_used {
                (t as f64) * burn_per_token
            } else {
                0.005
            };
            entry.remaining_fraction = (entry.remaining_fraction - burn).clamp(0.0, 1.0);
            entry.observed_at_unix_nano = now;
        }
    }

    /// Returns the single default snapshot (backward compatibility for tests and legacy callers).
    pub fn snapshot(&self) -> QuotaSnapshot {
        if let Some(ref path) = self.state_file {
            if let Some(snap) = read_quota_file(path) {
                return snap;
            }
        }

        let default_bucket = std::env::var("AGENT_OTEL_QUOTA_BUCKET")
            .unwrap_or_else(|_| agent_otel_core::semconv::QUOTA_DEFAULT_BUCKET.to_string());
        let default_group = std::env::var("AGENT_OTEL_QUOTA_GROUP")
            .unwrap_or_else(|_| agent_otel_core::semconv::QUOTA_DEFAULT_GROUP.to_string());

        QuotaSnapshot {
            remaining_fraction: 1.0,
            seconds_to_reset: 0.0,
            observed_at_unix_nano: current_unix_nano(),
            bucket: default_bucket,
            group: default_group,
        }
    }

    /// Returns multi-provider quota snapshots across all active AI coding harnesses on this machine.
    pub fn snapshots(&self) -> Vec<QuotaSnapshot> {
        let now = current_unix_nano();
        let elapsed_sec = if now > self.boot_time_ns {
            ((now - self.boot_time_ns) / 1_000_000_000) as f64
        } else {
            0.0
        };

        // Dynamically harvest fresh quotas from installed providers
        if let Some(home) = dirs_fallback() {
            if let Ok(mut lock) = self.quotas.write() {
                for provider in crate::platforms::detect_installed_providers(&home) {
                    if let Some(fresh_snap) = provider.harvest_quota(&home) {
                        lock.insert(fresh_snap.bucket.clone(), fresh_snap);
                    }
                }

                // Check directory ~/.agent-otel/quotas/*.json for custom drop-ins
                let quotas_dir = home.join(".agent-otel").join("quotas");
                if quotas_dir.is_dir() {
                    if let Ok(entries) = std::fs::read_dir(quotas_dir) {
                        for entry in entries.flatten() {
                            let p = entry.path();
                            if p.extension().map(|e| e == "json").unwrap_or(false) {
                                if let Some(snap) = read_quota_file(&p) {
                                    lock.insert(snap.bucket.clone(), snap);
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Ok(lock) = self.quotas.read() {
            let mut res: Vec<QuotaSnapshot> = lock
                .values()
                .cloned()
                .map(|mut s| {
                    s.observed_at_unix_nano = now;
                    if s.seconds_to_reset > elapsed_sec {
                        s.seconds_to_reset =
                            (s.seconds_to_reset - (elapsed_sec % s.seconds_to_reset)).max(10.0);
                    }
                    s
                })
                .collect();
            // Sort by bucket for deterministic presentation
            res.sort_by(|a, b| a.bucket.cmp(&b.bucket));
            res
        } else {
            vec![self.snapshot()]
        }
    }
}

pub fn read_quota_file(path: &Path) -> Option<QuotaSnapshot> {
    let bytes = std::fs::read(path).ok()?;
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).ok()?;

    let mut remaining = parsed
        .get("remaining_fraction")
        .or_else(|| parsed.get("remainingFraction"))
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0);

    let raw_reset = parsed
        .get("seconds_to_reset")
        .or_else(|| parsed.get("secondsToReset"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let file_age_sec = path
        .metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|m| m.elapsed().ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);

    let seconds_to_reset = if raw_reset > file_age_sec {
        raw_reset - file_age_sec
    } else {
        0.0
    };

    // If quota was marked 0.0 but the reset duration has elapsed, restore to healthy fraction
    if remaining == 0.0 && raw_reset > 0.0 && file_age_sec >= raw_reset {
        remaining = 0.75;
    }

    let bucket = parsed
        .get("bucket")
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| std::env::var("AGENT_OTEL_QUOTA_BUCKET").ok())
        .unwrap_or_else(|| agent_otel_core::semconv::QUOTA_DEFAULT_BUCKET.to_string());

    let group = parsed
        .get("group")
        .or_else(|| parsed.get("provider"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| std::env::var("AGENT_OTEL_QUOTA_GROUP").ok())
        .unwrap_or_else(|| agent_otel_core::semconv::QUOTA_DEFAULT_GROUP.to_string());

    Some(QuotaSnapshot {
        remaining_fraction: remaining.clamp(0.0, 1.0),
        seconds_to_reset: if seconds_to_reset > 0.0 {
            seconds_to_reset
        } else {
            3600.0
        },
        observed_at_unix_nano: current_unix_nano(),
        bucket,
        group,
    })
}

pub fn dirs_fallback() -> Option<PathBuf> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()
        .map(PathBuf::from)
}

pub fn current_unix_nano() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}
