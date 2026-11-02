/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use base64::Engine;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn is_bridge_command(cmd: &str) -> bool {
    let lower = cmd.to_ascii_lowercase();
    if let Some((_, encoded)) = cmd.split_once("-EncodedCommand ") {
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded.trim()) {
            if bytes.len() % 2 == 0 {
                let words: Vec<u16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect();
                if let Ok(script) = String::from_utf16(&words) {
                    return script.to_ascii_lowercase().contains("agent-hook");
                }
            }
        }
    }
    lower.contains("agent-hook") || lower.contains("agent-otel-bridge hook")
}

pub fn has_bridge_command(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object
                .get("command")
                .and_then(Value::as_str)
                .is_some_and(is_bridge_command)
                || object.values().any(has_bridge_command)
        }
        Value::Array(array) => array.iter().any(has_bridge_command),
        _ => false,
    }
}

pub fn format_claude_hook_command(binary: &str, event: &str, client_tag: Option<&str>) -> String {
    if !cfg!(windows) {
        return format_hook_command(binary, event, client_tag);
    }
    format_claude_windows_hook_command(binary, event)
}

/// Windows Claude launcher. Grok imports Claude settings but supplies its own
/// native bridge hooks. The encoded script emits `{}` when `GROK_WORKSPACE_ROOT`
/// is set, without disabling imported third-party hooks.
pub fn format_claude_windows_hook_command(binary: &str, event: &str) -> String {
    let script = claude_imported_grok_guard_script(binary, event);
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let system_root = std::env::var("SystemRoot")
        .unwrap_or_else(|_| "C:\\Windows".to_string())
        .replace('\\', "/");
    // Forward slashes survive Claude's Git Bash as well as Grok's PowerShell.
    format!("{system_root}/System32/WindowsPowerShell/v1.0/powershell.exe -NoProfile -NonInteractive -EncodedCommand {encoded}")
}

pub fn claude_imported_grok_guard_script(binary: &str, event: &str) -> String {
    let invocation = format_powershell_hook_command(binary, event, Some("claude"));
    format!("if ($env:GROK_WORKSPACE_ROOT) {{ '{{}}' }} else {{ {invocation} }}")
}

pub fn format_hook_command(binary: &str, event: &str, client_tag: Option<&str>) -> String {
    let client_arg = match client_tag {
        Some(tag) => format!(" --client {tag}"),
        None => String::new(),
    };

    // Always protect the executable path from shell splitting.  Keep a path
    // that the caller has already quoted unchanged.
    let bin_str = if binary.starts_with('"') && binary.ends_with('"') {
        binary.to_string()
    } else {
        format!("\"{binary}\"")
    };

    format!("{bin_str} {event}{client_arg}")
}

/// Render a command for adapters whose interpreter evaluates hook commands as
/// PowerShell expressions.  A quoted executable path by itself is a string
/// expression in PowerShell; the call operator is required before arguments
/// can be passed (otherwise the harness reports a ParserError and continues).
pub fn format_powershell_hook_command(
    binary: &str,
    event: &str,
    client_tag: Option<&str>,
) -> String {
    let path = binary
        .trim_matches('"')
        .replace('`', "``")
        .replace('$', "`$")
        .replace('"', "`\"");
    let client_arg = client_tag
        .map(|tag| format!(" --client {tag}"))
        .unwrap_or_default();
    format!("& \"{path}\" {event}{client_arg}")
}

/// Render the nested quoting required by `cmd.exe /c` when the executable
/// path contains spaces. This is useful for adapters that expose a CMD shell
/// contract; it remains opt-in so existing adapter semantics stay unchanged.
#[allow(dead_code)] // Public compatibility renderer; AGY now needs encoded PowerShell.
pub fn format_cmd_hook_command(binary: &str, event: &str, client_tag: Option<&str>) -> String {
    let path = binary.trim_matches('"');
    let client_arg = client_tag
        .map(|tag| format!(" --client {tag}"))
        .unwrap_or_default();
    format!("call \"{path}\" {event}{client_arg}")
}

pub fn format_grok_hook_command(binary: &str, event: &str, client_tag: Option<&str>) -> String {
    if cfg!(windows) {
        format_powershell_hook_command(binary, event, client_tag)
    } else {
        format_hook_command(binary, event, client_tag)
    }
}

pub fn format_antigravity_hook_command(
    binary: &str,
    event: &str,
    client_tag: Option<&str>,
) -> String {
    if cfg!(windows) {
        // AGY passes the command through Windows argv escaping before cmd /c.
        // Embedded quotes become literal backslash-quotes. Keep the outer
        // command quote-free; decode the quoted absolute hook path in PowerShell.
        // Resolve PowerShell through the OS directory, never the user's PATH.
        let script = format_powershell_hook_command(binary, event, client_tag);
        let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        format!("%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe -NoProfile -NonInteractive -EncodedCommand {encoded}")
    } else {
        format_hook_command(binary, event, client_tag)
    }
}

pub fn check_binary_in_path(binary: &str) -> bool {
    find_binary_in_path(binary).is_some()
}

pub fn find_binary_in_path(binary: &str) -> Option<PathBuf> {
    if let Ok(path_var) = std::env::var("PATH") {
        let bin_name = if cfg!(windows) && !binary.ends_with(".exe") {
            format!("{binary}.exe")
        } else {
            binary.to_string()
        };

        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(&bin_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

pub fn resolve_canonical_hook_binary(
    user_provided: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(bin) = user_provided {
        let p = Path::new(bin);
        if p.is_file() || check_binary_in_path(bin) {
            return Ok(bin.to_string());
        }
        return Ok(bin.to_string());
    }

    // 1. Check local runtime canonical bin
    let canonical = crate::local::get_canonical_hook_path();
    if canonical.is_file() {
        return Ok(canonical.to_string_lossy().to_string());
    }

    // 2. Check sibling of current running executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let neighbor = dir.join(if cfg!(windows) {
                "agent-hook.exe"
            } else {
                "agent-hook"
            });
            if neighbor.is_file() {
                return Ok(neighbor.to_string_lossy().to_string());
            }
        }
    }

    // 3. Check PATH
    if let Some(found) = find_binary_in_path("agent-hook") {
        return Ok(found.to_string_lossy().to_string());
    }

    Err(
        "Could not find an installed 'agent-hook' binary.\nRun 'agent-otel-bridge local install' to build and activate the local runtime, or specify --binary <path>."
            .into(),
    )
}
