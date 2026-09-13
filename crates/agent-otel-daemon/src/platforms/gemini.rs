/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::platform::PlatformQuotaProvider;
use crate::quota::{current_unix_nano, read_quota_file};
use agent_otel_core::platform::{AntigravityDescriptor, PlatformDescriptor};
use agent_otel_core::quota::QuotaSnapshot;
use std::path::Path;

pub struct GeminiQuotaProvider;

impl PlatformDescriptor for GeminiQuotaProvider {
    fn id(&self) -> &'static str {
        AntigravityDescriptor.id()
    }
    fn display_name(&self) -> &'static str {
        AntigravityDescriptor.display_name()
    }
    fn aliases(&self) -> &'static [&'static str] {
        AntigravityDescriptor.aliases()
    }
    fn wire_client_id(&self) -> u16 {
        AntigravityDescriptor.wire_client_id()
    }
    fn pre_tool_response(&self) -> agent_otel_core::platform::HookResponse {
        AntigravityDescriptor.pre_tool_response()
    }
}

impl PlatformQuotaProvider for GeminiQuotaProvider {
    fn is_installed(&self, home: &Path) -> bool {
        home.join(".gemini").exists()
            || home.join(".state").join("quota.json").exists()
            || std::env::var("GEMINI_API_KEY").is_ok()
    }

    fn harvest_quota(&self, home: &Path) -> Option<QuotaSnapshot> {
        let candidates = [
            home.join(".gemini").join("quota.json"),
            home.join(".state").join("quota.json"),
        ];
        for p in candidates {
            if p.exists() {
                if let Some(snap) = read_quota_file(&p) {
                    return Some(snap);
                }
            }
        }
        None
    }

    fn fallback_baseline(&self) -> QuotaSnapshot {
        QuotaSnapshot {
            remaining_fraction: 0.35,
            seconds_to_reset: 1800.0,
            observed_at_unix_nano: current_unix_nano(),
            bucket: "gemini".to_string(),
            group: "google".to_string(),
        }
    }
}
