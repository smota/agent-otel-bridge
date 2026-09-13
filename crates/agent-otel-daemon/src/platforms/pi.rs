/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::platform::PlatformQuotaProvider;
use crate::quota::{current_unix_nano, read_quota_file};
use agent_otel_core::platform::{PiDescriptor, PlatformDescriptor};
use agent_otel_core::quota::QuotaSnapshot;
use std::path::Path;

pub struct PiQuotaProvider;

impl PlatformDescriptor for PiQuotaProvider {
    fn id(&self) -> &'static str {
        PiDescriptor.id()
    }
    fn display_name(&self) -> &'static str {
        PiDescriptor.display_name()
    }
    fn aliases(&self) -> &'static [&'static str] {
        PiDescriptor.aliases()
    }
    fn wire_client_id(&self) -> u8 {
        PiDescriptor.wire_client_id()
    }
    fn pre_tool_response(&self) -> agent_otel_core::platform::HookResponse {
        PiDescriptor.pre_tool_response()
    }
}

impl PlatformQuotaProvider for PiQuotaProvider {
    fn is_installed(&self, home: &Path) -> bool {
        home.join(".pi").exists()
    }

    fn harvest_quota(&self, home: &Path) -> Option<QuotaSnapshot> {
        let p = home.join(".pi").join("quota.json");
        if p.exists() {
            read_quota_file(&p)
        } else {
            None
        }
    }

    fn fallback_baseline(&self) -> QuotaSnapshot {
        QuotaSnapshot {
            remaining_fraction: 0.95,
            seconds_to_reset: 3600.0,
            observed_at_unix_nano: current_unix_nano(),
            bucket: "pi".to_string(),
            group: "pi".to_string(),
        }
    }
}
