/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use base64::Engine;
use serde_json::Value;
use std::path::{Path, PathBuf};

#[cfg(not(target_os = "linux"))]
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

/// On Linux ownership is a recognized direct invocation, never a substring in
/// an arbitrary shell command. Unrecognized wrappers remain user-owned.
#[cfg(target_os = "linux")]
pub fn is_bridge_command(command: &str) -> bool {
    let command = command.trim();
    let guard = "if [ -n \"${GROK_WORKSPACE_ROOT:-}\" ]; then printf '{}'; else ";
    let command = if let Some(inner) = command.strip_prefix(guard) {
        let Some(inner) = inner.strip_suffix("; fi") else {
            return false;
        };
        inner
    } else {
        command
    };
    // Recognize only the exact encoded launchers emitted by the existing
    // Windows renderers; a legacy registration can be inspected on Linux.
    let outer: Vec<_> = command.split_whitespace().collect();
    if outer.len() == 5
        && outer[0].replace('\\', "/").ends_with("/powershell.exe")
        && outer[1..4] == ["-NoProfile", "-NonInteractive", "-EncodedCommand"]
    {
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(outer[4]) else {
            return false;
        };
        if bytes.len() % 2 != 0 {
            return false;
        }
        let words = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect::<Vec<_>>();
        let Ok(script) = String::from_utf16(&words) else {
            return false;
        };
        return recognized_powershell_invocation(&script);
    }
    let Some(mut tokens) = literal_shell_words(command) else {
        return false;
    };
    if tokens
        .first()
        .is_some_and(|word| word.starts_with("AGENT_OTEL_SOCKET="))
    {
        tokens.remove(0);
    }
    let Some(executable) = tokens.first() else {
        return false;
    };
    let basename = executable.rsplit(['/', '\\']).next().unwrap_or("");
    let arguments = if basename.eq_ignore_ascii_case("agent-hook")
        || basename.eq_ignore_ascii_case("agent-hook.exe")
    {
        &tokens[1..]
    } else if (basename.eq_ignore_ascii_case("agent-otel-bridge")
        || basename.eq_ignore_ascii_case("agent-otel-bridge.exe"))
        && tokens.get(1).is_some_and(|word| word == "hook")
    {
        &tokens[2..]
    } else {
        return false;
    };
    recognized_arguments(arguments)
}

#[cfg(target_os = "linux")]
fn recognized_arguments(arguments: &[String]) -> bool {
    matches!(
        arguments.first().map(String::as_str),
        Some("PreToolUse" | "PostToolUse" | "Stop" | "PreInvocation" | "PostInvocation")
    ) && (arguments.len() == 1
        || (arguments.len() == 3
            && arguments[1] == "--client"
            && matches!(
                arguments[2].as_str(),
                "claude" | "codex" | "agy" | "antigravity" | "gemini" | "grok" | "pi"
            )))
}

#[cfg(target_os = "linux")]
fn literal_shell_words(command: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote = None;
    let mut chars = command.chars().peekable();
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (Some('\''), '\'') | (Some('"'), '"') => quote = None,
            (None, '\'' | '"') => {
                quote = Some(ch);
                started = true;
            }
            (Some('\''), _) => {
                word.push(ch);
                started = true;
            }
            (_, '\\') => {
                let next = chars.next()?;
                if quote == Some('"') && !matches!(next, '$' | '`' | '"' | '\\' | '\n') {
                    word.push('\\');
                }
                word.push(next);
                started = true;
            }
            (_, '$' | '`') => return None,
            (
                None,
                ';' | '|' | '&' | '<' | '>' | '(' | ')' | '*' | '?' | '[' | ']' | '{' | '}' | '~'
                | '\n' | '\r',
            ) => return None,
            (None, ch) if ch.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            (_, _) => {
                word.push(ch);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    if started {
        words.push(word);
    }
    Some(words)
}

#[cfg(target_os = "linux")]
fn recognized_powershell_invocation(script: &str) -> bool {
    let guard = "if ($env:GROK_WORKSPACE_ROOT) { '{}' } else { ";
    let (script, guarded) = if let Some(inner) = script.strip_prefix(guard) {
        let Some(inner) = inner.strip_suffix(" }") else {
            return false;
        };
        (inner, true)
    } else {
        (script, false)
    };
    let Some(inner) = script.strip_prefix("& \"") else {
        return false;
    };
    let mut binary = String::new();
    let mut chars = inner.char_indices();
    while let Some((offset, ch)) = chars.next() {
        match ch {
            '`' => {
                let Some((_, escaped)) = chars.next() else {
                    return false;
                };
                binary.push(escaped);
            }
            '"' => {
                let Some(arguments) = literal_shell_words(inner[offset + 1..].trim()) else {
                    return false;
                };
                let basename = binary.rsplit(['/', '\\']).next().unwrap_or("");
                if !matches!(basename, "agent-hook" | "agent-hook.exe")
                    || !recognized_arguments(&arguments)
                {
                    return false;
                }
                let tag = arguments.get(2).map(String::as_str);
                return (!guarded || tag == Some("claude"))
                    && script == format_powershell_hook_command(&binary, &arguments[0], tag);
            }
            _ => binary.push(ch),
        }
    }
    false
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
        #[cfg(target_os = "linux")]
        return format_posix_claude_hook_command(
            binary,
            event,
            client_tag,
            std::env::var("AGENT_OTEL_SOCKET").ok().as_deref(),
        );
        #[cfg(not(target_os = "linux"))]
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
    #[cfg(target_os = "linux")]
    {
        format_posix_hook_command(
            binary,
            event,
            client_tag,
            std::env::var("AGENT_OTEL_SOCKET").ok().as_deref(),
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
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
        format_hook_command(
            binary,
            event,
            if cfg!(target_os = "linux") {
                client_tag.or(Some("agy"))
            } else {
                client_tag
            },
        )
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

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;
    #[test]
    fn ownership_does_not_claim_foreign_audit_or_wrapper_commands() {
        for command in [
            "printf agent-hook-audit >> /tmp/foreign-audit",
            "echo /opt/agent-hook PreToolUse",
            "sh -c '/opt/agent-hook PreToolUse'",
            "cat /opt/agent-hook",
            "/opt/agent-hook-audit PreToolUse",
            "echo agent-otel-bridge hook",
            "/opt/*/agent-hook PreToolUse",
        ] {
            assert!(
                !is_bridge_command(command),
                "foreign invocation was claimed"
            );
        }
    }

    #[test]
    fn merge_preserves_foreign_command_that_mentions_agent_hook() {
        let path =
            std::env::temp_dir().join(format!("bridge-foreign-audit-{}.json", std::process::id()));
        let foreign = serde_json::json!({"matcher":".*","hooks":[{"type":"command","command":"printf agent-hook-audit >> /tmp/foreign-audit"}]});
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({"hooks":{"PreToolUse":[foreign.clone()]}}))
                .unwrap(),
        )
        .unwrap();
        super::super::install_standard_hooks(&path, "/opt/agent-hook", Some("codex")).unwrap();
        let merged: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(merged["hooks"]["PreToolUse"][0], foreign);
        assert_eq!(merged["hooks"]["PreToolUse"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn native_harnesses_have_explicit_client_tags() {
        assert!(
            format_claude_hook_command("/opt/agent-hook", "PreToolUse", None)
                .contains("--client claude")
        );
        assert!(
            format_antigravity_hook_command("/opt/agent-hook", "PreToolUse", None)
                .ends_with("--client agy")
        );
    }
    #[test]
    fn imported_claude_telemetry_does_not_duplicate_native_grok_events() {
        let command =
            format_posix_claude_hook_command("/bin/false", "PreToolUse", Some("claude"), None);
        let imported = std::process::Command::new("/bin/sh")
            .args(["-c", &command])
            .env("GROK_WORKSPACE_ROOT", "/workspace")
            .output()
            .unwrap();
        assert!(imported.status.success());
        assert_eq!(imported.stdout, b"{}");
        let native = std::process::Command::new("/bin/sh")
            .args(["-c", &command])
            .env_remove("GROK_WORKSPACE_ROOT")
            .output()
            .unwrap();
        assert!(!native.status.success());
    }

    #[test]
    fn posix_path_is_literal_not_shell_expansion() {
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!(
                "printf '%s' {}",
                format_hook_command(
                    "/tmp/$HOME`printf injected`/agent-hook",
                    "Stop",
                    Some("codex")
                )
            ))
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "/tmp/$HOME`printf injected`/agent-hookStop--clientcodex"
        );
    }
}

/// Installation-time POSIX renderer. Persist the socket because isolated seat
/// launches may clear the parent's environment before invoking the hook.
#[cfg(target_os = "linux")]
pub fn format_posix_hook_command(
    binary: &str,
    event: &str,
    client_tag: Option<&str>,
    socket: Option<&str>,
) -> String {
    let quote = |value: &str| format!("'{}'", value.replace('\'', "'\"'\"'"));
    let prefix = socket
        .map(|path| format!("AGENT_OTEL_SOCKET={} ", quote(path)))
        .unwrap_or_default();
    // Retain double-quoted executable presentation while escaping every POSIX
    // expansion character. No shell is spawned by the client itself.
    let mut path = String::new();
    let mut chars = binary.trim_matches('"').chars().peekable();
    while let Some(ch) = chars.next() {
        if matches!(ch, '$' | '`' | '"')
            || (ch == '\\'
                && chars
                    .peek()
                    .is_none_or(|next| matches!(next, '$' | '`' | '"' | '\\' | '\n')))
        {
            path.push('\\');
        }
        path.push(ch);
    }
    let client = client_tag
        .map(|tag| {
            format!(
                " --client {}",
                if tag.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                    tag.to_string()
                } else {
                    quote(tag)
                }
            )
        })
        .unwrap_or_default();
    format!(
        "{prefix}\"{path}\" {}{client}",
        if event.bytes().all(|b| b.is_ascii_alphanumeric()) {
            event.to_string()
        } else {
            quote(event)
        }
    )
}

#[cfg(target_os = "linux")]
pub(crate) fn format_posix_claude_hook_command(
    binary: &str,
    event: &str,
    client_tag: Option<&str>,
    socket: Option<&str>,
) -> String {
    let invocation =
        format_posix_hook_command(binary, event, client_tag.or(Some("claude")), socket);
    // Grok imports Claude settings and loads its native hooks too. Skip only
    // this bridge command, preserving all foreign Claude hooks.
    format!("if [ -n \"${{GROK_WORKSPACE_ROOT:-}}\" ]; then printf '{{}}'; else {invocation}; fi")
}
