/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

pub mod claude;
pub mod codex;
pub mod gemini;
pub mod grok;
pub mod pi;

use crate::platform::PlatformQuotaProvider;
use std::path::Path;

pub use claude::ClaudeQuotaProvider;
pub use codex::CodexQuotaProvider;
pub use gemini::GeminiQuotaProvider;
pub use grok::GrokQuotaProvider;
pub use pi::PiQuotaProvider;

pub static BUILTIN_PROVIDERS: &[&dyn PlatformQuotaProvider] = &[
    &GeminiQuotaProvider,
    &ClaudeQuotaProvider,
    &CodexQuotaProvider,
    &GrokQuotaProvider,
    &PiQuotaProvider,
];

pub fn find_provider_by_name(name: &str) -> Option<&'static (dyn PlatformQuotaProvider + 'static)> {
    BUILTIN_PROVIDERS
        .iter()
        .copied()
        .find(|p| p.matches_name(name))
}

pub fn find_provider_by_wire_id(id: u16) -> Option<&'static (dyn PlatformQuotaProvider + 'static)> {
    BUILTIN_PROVIDERS
        .iter()
        .copied()
        .find(|p| p.wire_client_id() == id)
}

pub fn detect_installed_providers(
    home: &Path,
) -> Vec<&'static (dyn PlatformQuotaProvider + 'static)> {
    BUILTIN_PROVIDERS
        .iter()
        .copied()
        .filter(|p| p.is_installed(home))
        .collect()
}
