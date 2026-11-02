/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use super::command::format_grok_hook_command;
use super::standard::install_hooks_with_command_formatter;
use std::path::Path;

/// Grok evaluates its command field through PowerShell on Windows. Keep its
/// registration isolated while retaining the standard JSON shape and update
/// semantics used by the other adapters.
pub fn install_grok_hooks(
    path: &Path,
    binary: &str,
    client_tag: Option<&str>,
) -> Result<bool, Box<dyn std::error::Error>> {
    install_hooks_with_command_formatter(path, binary, client_tag, format_grok_hook_command)
}
