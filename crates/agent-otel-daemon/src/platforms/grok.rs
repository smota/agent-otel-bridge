/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::platform::PlatformQuotaProvider;
use crate::quota::{current_unix_nano, read_quota_file};
use agent_otel_core::platform::{GrokDescriptor, PlatformDescriptor};
use agent_otel_core::quota::QuotaSnapshot;
use std::path::Path;

pub struct GrokQuotaProvider;

impl PlatformDescriptor for GrokQuotaProvider {
    fn id(&self) -> &'static str {
        GrokDescriptor.id()
    }
    fn display_name(&self) -> &'static str {
        GrokDescriptor.display_name()
    }
    fn aliases(&self) -> &'static [&'static str] {
        GrokDescriptor.aliases()
    }
    fn wire_client_id(&self) -> u8 {
        GrokDescriptor.wire_client_id()
    }
    fn pre_tool_response(&self) -> agent_otel_core::platform::HookResponse {
        GrokDescriptor.pre_tool_response()
    }
}

impl PlatformQuotaProvider for GrokQuotaProvider {
    fn is_installed(&self, home: &Path) -> bool {
        home.join(".grok").exists() || std::env::var("XAI_API_KEY").is_ok()
    }

    fn harvest_quota(&self, home: &Path) -> Option<QuotaSnapshot> {
        let p = home.join(".grok").join("quota.json");
        if p.exists() {
            read_quota_file(&p)
        } else {
            None
        }
    }

    fn fallback_baseline(&self) -> QuotaSnapshot {
        QuotaSnapshot {
            remaining_fraction: 0.45,
            seconds_to_reset: 3600.0,
            observed_at_unix_nano: current_unix_nano(),
            bucket: "grok".to_string(),
            group: "xai".to_string(),
        }
    }
}
