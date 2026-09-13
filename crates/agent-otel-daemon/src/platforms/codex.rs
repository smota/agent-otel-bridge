/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::platform::PlatformQuotaProvider;
use crate::quota::{current_unix_nano, read_quota_file};
use agent_otel_core::platform::{CodexDescriptor, PlatformDescriptor};
use agent_otel_core::quota::QuotaSnapshot;
use std::path::Path;

pub struct CodexQuotaProvider;

impl PlatformDescriptor for CodexQuotaProvider {
    fn id(&self) -> &'static str {
        CodexDescriptor.id()
    }
    fn display_name(&self) -> &'static str {
        CodexDescriptor.display_name()
    }
    fn aliases(&self) -> &'static [&'static str] {
        CodexDescriptor.aliases()
    }
    fn wire_client_id(&self) -> u16 {
        CodexDescriptor.wire_client_id()
    }
    fn pre_tool_response(&self) -> agent_otel_core::platform::HookResponse {
        CodexDescriptor.pre_tool_response()
    }
}

impl PlatformQuotaProvider for CodexQuotaProvider {
    fn is_installed(&self, home: &Path) -> bool {
        home.join(".codex").exists() || std::env::var("OPENAI_API_KEY").is_ok()
    }

    fn harvest_quota(&self, home: &Path) -> Option<QuotaSnapshot> {
        let p = home.join(".codex").join("quota.json");
        if p.exists() {
            read_quota_file(&p)
        } else {
            None
        }
    }

    fn fallback_baseline(&self) -> QuotaSnapshot {
        QuotaSnapshot {
            remaining_fraction: 0.40,
            seconds_to_reset: 2400.0,
            observed_at_unix_nano: current_unix_nano(),
            bucket: "codex".to_string(),
            group: "openai".to_string(),
        }
    }
}
