/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_core::platform::PlatformDescriptor;
use agent_otel_core::quota::QuotaSnapshot;
use std::path::Path;

pub trait PlatformQuotaProvider: PlatformDescriptor {
    /// Checks whether this platform harness is actively installed or configured on the machine.
    fn is_installed(&self, home: &Path) -> bool;

    /// Harvests dynamic quota snapshot (from session files, state cache, or JSON).
    fn harvest_quota(&self, home: &Path) -> Option<QuotaSnapshot>;

    /// Fallback baseline used ONLY when the platform is detected/installed, but has no live state.
    fn fallback_baseline(&self) -> QuotaSnapshot;

    /// Token burn rate used to dynamically adjust headroom on tool execution.
    fn burn_per_token(&self) -> f64 {
        1.0 / 500_000.0
    }
}
