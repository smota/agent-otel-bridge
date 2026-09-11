/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_core::quota::QuotaSnapshot;
use std::path::PathBuf;

pub struct QuotaEngine {
    state_file: Option<PathBuf>,
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
                // Check default state directory and common agent state fallbacks
                dirs_fallback().and_then(|home| {
                    let candidates = [
                        home.join(".state").join("quota.json"),
                        home.join(".gemini").join("quota.json"),
                        home.join(".agent-otel").join("quota.json"),
                    ];
                    candidates.into_iter().find(|p| p.exists())
                })
            });

        Self { state_file }
    }

    pub fn snapshot(&self) -> QuotaSnapshot {
        if let Some(ref path) = self.state_file {
            if path.exists() {
                if let Ok(bytes) = std::fs::read(path) {
                    if let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        let remaining = parsed
                            .get("remaining_fraction")
                            .or_else(|| parsed.get("remainingFraction"))
                            .and_then(|v| v.as_f64())
                            .unwrap_or(1.0);

                        let reset = parsed
                            .get("seconds_to_reset")
                            .or_else(|| parsed.get("secondsToReset"))
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0);

                        let bucket = parsed
                            .get("bucket")
                            .and_then(|v| v.as_str())
                            .map(String::from)
                            .or_else(|| std::env::var("AGENT_OTEL_QUOTA_BUCKET").ok())
                            .unwrap_or_else(|| {
                                agent_otel_core::semconv::QUOTA_DEFAULT_BUCKET.to_string()
                            });

                        let group = parsed
                            .get("group")
                            .or_else(|| parsed.get("provider"))
                            .and_then(|v| v.as_str())
                            .map(String::from)
                            .or_else(|| std::env::var("AGENT_OTEL_QUOTA_GROUP").ok())
                            .unwrap_or_else(|| {
                                agent_otel_core::semconv::QUOTA_DEFAULT_GROUP.to_string()
                            });

                        return QuotaSnapshot {
                            remaining_fraction: remaining.clamp(0.0, 1.0),
                            seconds_to_reset: reset.max(0.0),
                            observed_at_unix_nano: current_unix_nano(),
                            bucket,
                            group,
                        };
                    }
                }
            }
        }

        let default_bucket = std::env::var("AGENT_OTEL_QUOTA_BUCKET")
            .unwrap_or_else(|_| agent_otel_core::semconv::QUOTA_DEFAULT_BUCKET.to_string());
        let default_group = std::env::var("AGENT_OTEL_QUOTA_GROUP")
            .unwrap_or_else(|_| agent_otel_core::semconv::QUOTA_DEFAULT_GROUP.to_string());

        // Heartbeat / Synthetic default when no cached file is found
        QuotaSnapshot {
            remaining_fraction: 1.0,
            seconds_to_reset: 0.0,
            observed_at_unix_nano: current_unix_nano(),
            bucket: default_bucket,
            group: default_group,
        }
    }
}

fn dirs_fallback() -> Option<PathBuf> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()
        .map(PathBuf::from)
}

fn current_unix_nano() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}
