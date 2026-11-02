/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use super::command::{format_hook_command, format_powershell_hook_command, is_bridge_command};
use super::standard::install_formatted_hooks;
use base64::Engine;
use serde::Deserialize;
use serde_json::Value;
use std::fs;
use std::path::Path;

/// Codex uses the session shell (PowerShell on this station), or CMD as fallback.
/// A quote-free, absolute launcher token works in both; encode the native path
/// and its arguments so the outer interpreter cannot expand them.
pub fn format_codex_hook_command(
    binary: &str,
    event: &str,
    client_tag: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    if !cfg!(windows) {
        return Ok(format_hook_command(binary, event, client_tag));
    }
    let root = std::env::var("SystemRoot")?;
    format_codex_windows_command(&root, binary, event, client_tag)
}

/// Installer policy, never inferred from the shell running this installer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodexHookShell {
    Portable,
    Powershell,
}

impl CodexHookShell {
    pub fn render(
        self,
        binary: &str,
        event: &str,
        tag: Option<&str>,
    ) -> Result<String, Box<dyn std::error::Error>> {
        if cfg!(windows) && self == Self::Powershell {
            Ok(format_powershell_hook_command(binary, event, tag))
        } else {
            format_codex_hook_command(binary, event, tag)
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CodexHookPolicy {
    version: u32,
    windows_hook_shell: CodexHookShell,
}

/// Read bridge-owned metadata only during installation. Codex's configuration
/// schema and native hook execution need no new fields or file reads.
pub fn load_codex_hook_shell(
    policy_path: &Path,
) -> Result<CodexHookShell, Box<dyn std::error::Error>> {
    match fs::read_to_string(policy_path) {
        Ok(raw) => {
            let policy: CodexHookPolicy = serde_json::from_str(&raw)?;
            if policy.version != 1 {
                return Err(format!(
                    "Unsupported Codex bridge policy version: {}",
                    policy.version
                )
                .into());
            }
            Ok(policy.windows_hook_shell)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(CodexHookShell::Portable),
        Err(err) => Err(err.into()),
    }
}

pub(crate) fn format_codex_windows_command(
    system_root: &str,
    binary: &str,
    event: &str,
    client_tag: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let root = system_root.replace('\\', "/");
    let bytes = root.as_bytes();
    if bytes.len() < 3
        || !bytes[0].is_ascii_alphabetic()
        || &bytes[1..3] != b":/"
        || !bytes[3..]
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-'))
    {
        return Err(
            "Codex hooks require a token-safe absolute SystemRoot; configuration unchanged".into(),
        );
    }
    let script = format_powershell_hook_command(binary, event, client_tag);
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let root = root.replace('/', "\\");
    Ok(format!("{root}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe -NoProfile -NonInteractive -EncodedCommand {encoded}"))
}

pub fn install_codex_hooks(
    path: &Path,
    binary: &str,
    client_tag: Option<&str>,
) -> Result<bool, Box<dyn std::error::Error>> {
    if path.exists() {
        // Unlike a missing file, unreadable/invalid configuration must never
        // turn into an empty document and erase the user's other hooks.
        let root: Value = serde_json::from_str(&fs::read_to_string(path)?)?;
        if !root.is_object() || root.get("hooks").is_some_and(|v| !v.is_object()) {
            return Err("Invalid Codex hook configuration; configuration unchanged".into());
        }
        for event in ["PreToolUse", "PostToolUse", "Stop"] {
            if let Some(groups) = root.get("hooks").and_then(|h| h.get(event)) {
                let groups = groups.as_array().ok_or("Invalid Codex hook event array")?;
                for group in groups {
                    let hooks = group
                        .get("hooks")
                        .and_then(Value::as_array)
                        .ok_or("Invalid Codex hook group")?;
                    for hook in hooks {
                        if cfg!(windows)
                            && hook
                                .get("commandWindows")
                                .and_then(Value::as_str)
                                .is_some_and(is_bridge_command)
                            && !hook
                                .get("command")
                                .and_then(Value::as_str)
                                .is_some_and(is_bridge_command)
                        {
                            return Err("Codex Windows-only bridge override requires ownership review; configuration unchanged".into());
                        }
                    }
                }
            }
        }
    }
    // A saved choice survives all installer callers, including local install.
    // Non-Windows rendering ignores this Windows-only station policy.
    let shell = if cfg!(windows) {
        load_codex_hook_shell(&path.with_file_name("agent-otel-bridge-policy.json"))?
    } else {
        CodexHookShell::Portable
    };
    let commands = ["PreToolUse", "PostToolUse", "Stop"]
        .map(|event| shell.render(binary, event, client_tag))
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    install_formatted_hooks(
        path,
        binary,
        client_tag,
        |_, event, _| {
            commands[match event {
                "PreToolUse" => 0,
                "PostToolUse" => 1,
                "Stop" => 2,
                _ => unreachable!("standard hook event"),
            }]
            .clone()
        },
        cfg!(windows),
    )
}
