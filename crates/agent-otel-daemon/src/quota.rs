/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use std::path::PathBuf;
use agent_otel_core::quota::QuotaSnapshot;

pub struct QuotaEngine {
    state_file: Option<PathBuf>,
}

impl QuotaEngine {
    pub fn new() -> Self {
        let state_file = std::env::var("AGENT_OTEL_QUOTA_FILE")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                // Check default state directory if present
                dirs_fallback().map(|d| d.join(".state").join("quota.json"))
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

                        return QuotaSnapshot {
                            remaining_fraction: remaining.clamp(0.0, 1.0),
                            seconds_to_reset: reset.max(0.0),
                            observed_at_unix_nano: current_unix_nano(),
                            bucket: "gemini-weekly".to_string(),
                            group: "gemini".to_string(),
                        };
                    }
                }
            }
        }

        // Heartbeat / Synthetic default when no cached file is found
        QuotaSnapshot {
            remaining_fraction: 1.0,
            seconds_to_reset: 0.0,
            observed_at_unix_nano: current_unix_nano(),
            bucket: "gemini-weekly".to_string(),
            group: "gemini".to_string(),
        }
    }
}

fn dirs_fallback() -> Option<PathBuf> {
    std::env::var("USERPROFILE").ok().map(PathBuf::from)
}

fn current_unix_nano() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}
