/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use super::standard::uninstall_standard_hooks;
use std::fs;
use std::path::{Path, PathBuf};

pub fn install_pi_hooks(
    path: &Path,
    binary: &str,
    _client_tag: Option<&str>,
) -> Result<bool, Box<dyn std::error::Error>> {
    let extension = pi_extension_path(path)?;
    if extension.exists()
        && !fs::read_to_string(&extension)?.starts_with("// Managed by agent-otel-bridge.")
    {
        return Err("Refusing to replace an unowned Pi extension".into());
    }
    fs::create_dir_all(extension.parent().ok_or("Missing Pi extension directory")?)?;
    let content = include_str!("../pi-extension.ts").replace(
        "__AGENT_OTEL_HOOK_PATH__",
        &serde_json::to_string(binary.trim_matches('"'))?,
    );
    fs::write(extension, content)?;
    Ok(true)
}

pub(crate) fn pi_extension_path(path: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(path
        .parent()
        .ok_or("Missing Pi config directory")?
        .join("agent/extensions/agent-otel-bridge.ts"))
}

pub fn uninstall_pi_hooks(path: &Path, binary: &str) -> Result<bool, Box<dyn std::error::Error>> {
    let mut changed = uninstall_standard_hooks(path, binary)?;
    let extension = pi_extension_path(path)?;
    if extension.exists()
        && fs::read_to_string(&extension)?.starts_with("// Managed by agent-otel-bridge.")
    {
        fs::remove_file(extension)?;
        changed = true;
    }
    Ok(changed)
}
