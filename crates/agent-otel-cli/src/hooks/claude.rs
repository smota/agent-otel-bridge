/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use super::command::format_claude_hook_command;
use super::standard::{install_hooks_with_command_formatter, uninstall_standard_hooks};
use std::path::Path;

pub fn install_claude_hooks(path: &Path, binary: &str) -> Result<bool, Box<dyn std::error::Error>> {
    install_hooks_with_command_formatter(path, binary, None, format_claude_hook_command)
}

pub fn uninstall_claude_hooks(
    path: &Path,
    binary: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    uninstall_standard_hooks(path, binary)
}
