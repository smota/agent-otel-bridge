/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientTarget {
    Antigravity,
    ClaudeCode,
    Codex,
    Grok,
    Pi,
    All,
}

impl ClientTarget {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "antigravity" | "agy" | "gemini" => ClientTarget::Antigravity,
            "claude" | "claude-code" | "claudecode" => ClientTarget::ClaudeCode,
            "codex" | "openai" | "codex-cli" => ClientTarget::Codex,
            "grok" | "xai" | "grok-cli" => ClientTarget::Grok,
            "pi" | "inflection" | "pi-cli" => ClientTarget::Pi,
            _ => ClientTarget::All,
        }
    }
}

impl std::str::FromStr for ClientTarget {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from_str(s))
    }
}

pub fn get_antigravity_config_path(project: bool) -> Option<PathBuf> {
    if project {
        Some(PathBuf::from(".gemini").join("hooks.json"))
    } else {
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok()
            .map(|home| {
                PathBuf::from(home)
                    .join(".gemini")
                    .join("config")
                    .join("hooks.json")
            })
    }
}

pub fn get_claude_config_path(project: bool) -> Option<PathBuf> {
    if project {
        Some(PathBuf::from(".claude").join("settings.json"))
    } else {
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok()
            .map(|home| PathBuf::from(home).join(".claude").join("settings.json"))
    }
}

pub fn get_codex_config_path(project: bool) -> Option<PathBuf> {
    if project {
        Some(PathBuf::from(".codex").join("hooks.json"))
    } else {
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok()
            .map(|home| PathBuf::from(home).join(".codex").join("hooks.json"))
    }
}

pub fn get_grok_config_path(project: bool) -> Option<PathBuf> {
    if project {
        Some(PathBuf::from(".grok").join("hooks").join("agent-otel.json"))
    } else {
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok()
            .map(|home| {
                PathBuf::from(home)
                    .join(".grok")
                    .join("hooks")
                    .join("agent-otel.json")
            })
    }
}

pub fn get_pi_config_path(project: bool) -> Option<PathBuf> {
    if project {
        Some(PathBuf::from(".pi").join("hooks.json"))
    } else {
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok()
            .map(|home| PathBuf::from(home).join(".pi").join("hooks.json"))
    }
}

pub fn is_bridge_command(cmd: &str) -> bool {
    let lower = cmd.to_ascii_lowercase();
    lower.contains("agent-hook") || lower.contains("agent-otel-bridge hook")
}

pub fn format_hook_command(binary: &str, event: &str, client_tag: Option<&str>) -> String {
    let client_arg = match client_tag {
        Some(tag) => format!(" --client {tag}"),
        None => String::new(),
    };

    let bin_str = if binary.contains(' ') && !binary.starts_with('"') && !binary.ends_with('"') {
        format!("\"{binary}\"")
    } else {
        binary.to_string()
    };

    format!("{bin_str} {event}{client_arg}")
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

pub fn install_antigravity_hooks(
    path: &Path,
    binary: &str,
    client_tag: Option<&str>,
) -> Result<bool, Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut root: Value = if path.exists() {
        let content = fs::read_to_string(path)?;
        serde_json::from_str(&content).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    if !root.is_object() {
        root = json!({});
    }

    let pre_tool = format_hook_command(binary, "PreToolUse", client_tag);
    let post_tool = format_hook_command(binary, "PostToolUse", client_tag);
    let pre_inv = format_hook_command(binary, "PreInvocation", client_tag);
    let post_inv = format_hook_command(binary, "PostInvocation", client_tag);
    let stop = format_hook_command(binary, "Stop", client_tag);

    let hook_spec = json!({
        "PreToolUse": [
            {
                "matcher": ".*",
                "hooks": [{ "command": pre_tool, "timeout": 5, "type": "command" }]
            }
        ],
        "PostToolUse": [
            {
                "matcher": ".*",
                "hooks": [{ "command": post_tool, "timeout": 5, "type": "command" }]
            }
        ],
        "PreInvocation": [{ "command": pre_inv, "timeout": 5, "type": "command" }],
        "PostInvocation": [{ "command": post_inv, "timeout": 5, "type": "command" }],
        "Stop": [{ "command": stop, "timeout": 5, "type": "command" }]
    });

    // Update only the agent-otel-bridge section, preserving third-party keys intact
    root["agent-otel-bridge"] = hook_spec;

    let serialized = serde_json::to_string_pretty(&root)?;
    fs::write(path, serialized)?;
    Ok(true)
}

pub fn uninstall_antigravity_hooks(path: &Path) -> Result<bool, Box<dyn std::error::Error>> {
    if !path.exists() {
        return Ok(false);
    }

    let content = fs::read_to_string(path)?;
    let mut root: Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Ok(false),
    };

    if let Some(obj) = root.as_object_mut() {
        if obj.remove("agent-otel-bridge").is_some() {
            let serialized = serde_json::to_string_pretty(&root)?;
            fs::write(path, serialized)?;
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn install_standard_hooks(
    path: &Path,
    binary: &str,
    client_tag: Option<&str>,
) -> Result<bool, Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut root: Value = if path.exists() {
        let content = fs::read_to_string(path)?;
        serde_json::from_str(&content).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    if !root.is_object() {
        root = json!({});
    }

    // Clean up legacy "agent-otel-bridge" root key if present
    if let Some(obj) = root.as_object_mut() {
        obj.remove("agent-otel-bridge");
    }

    if root.get("hooks").is_none() || !root["hooks"].is_object() {
        root["hooks"] = json!({});
    }

    let hooks_obj = root["hooks"].as_object_mut().unwrap();

    let events = ["PreToolUse", "PostToolUse", "Stop"];
    for event in events {
        let target_cmd = format_hook_command(binary, event, client_tag);

        let entry = json!({
            "matcher": ".*",
            "hooks": [
                {
                    "type": "command",
                    "command": target_cmd,
                    "timeout": 5
                }
            ]
        });

        if let Some(arr) = hooks_obj.get_mut(event).and_then(|v| v.as_array_mut()) {
            let mut updated = false;
            for item in arr.iter_mut() {
                if let Some(harr) = item.get_mut("hooks").and_then(|h| h.as_array_mut()) {
                    for h in harr.iter_mut() {
                        let is_bridge = h
                            .get("command")
                            .and_then(|c| c.as_str())
                            .map(is_bridge_command)
                            .unwrap_or(false);
                        if is_bridge {
                            h["command"] = json!(&target_cmd);
                            updated = true;
                        }
                    }
                }
            }

            if !updated {
                arr.push(entry);
            }
        } else {
            hooks_obj.insert(event.to_string(), json!([entry]));
        }
    }

    let serialized = serde_json::to_string_pretty(&root)?;
    fs::write(path, serialized)?;
    Ok(true)
}

pub fn uninstall_standard_hooks(
    path: &Path,
    _binary: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    if !path.exists() {
        return Ok(false);
    }

    let content = fs::read_to_string(path)?;
    let mut root: Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Ok(false),
    };

    let mut changed = false;
    if let Some(hooks_obj) = root.get_mut("hooks").and_then(|v| v.as_object_mut()) {
        for event in ["PreToolUse", "PostToolUse", "Stop"] {
            if let Some(arr) = hooks_obj.get_mut(event).and_then(|v| v.as_array_mut()) {
                let initial_len = arr.len();
                arr.retain(|item| {
                    let matches_bridge = item
                        .get("hooks")
                        .and_then(|h| h.as_array())
                        .map(|harr| {
                            harr.iter().any(|h| {
                                h.get("command")
                                    .and_then(|c| c.as_str())
                                    .map(is_bridge_command)
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false);
                    !matches_bridge
                });
                if arr.len() != initial_len {
                    changed = true;
                }
            }
        }
    }

    if changed {
        let serialized = serde_json::to_string_pretty(&root)?;
        fs::write(path, serialized)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn install_claude_hooks(path: &Path, binary: &str) -> Result<bool, Box<dyn std::error::Error>> {
    install_standard_hooks(path, binary, None)
}

pub fn uninstall_claude_hooks(
    path: &Path,
    binary: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    uninstall_standard_hooks(path, binary)
}

pub fn run_install(
    client_str: &str,
    project: bool,
    binary_opt: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let target = ClientTarget::from_str(client_str);
    let resolved_binary = resolve_canonical_hook_binary(binary_opt)?;
    let binary = &resolved_binary;

    println!("\n=== Installing agent-otel-bridge hooks ===");
    println!("Target binary: {}", binary);

    let mut installed_any = false;

    if target == ClientTarget::Antigravity || target == ClientTarget::All {
        if let Some(path) = get_antigravity_config_path(project) {
            let should_install = target == ClientTarget::Antigravity
                || project
                || path.parent().map(|p| p.exists()).unwrap_or(false);
            if should_install {
                match install_antigravity_hooks(&path, binary, None) {
                    Ok(_) => {
                        println!("  [ok] Antigravity hooks registered at: {}", path.display());
                        installed_any = true;
                    }
                    Err(e) => println!("  [fail] Failed to install Antigravity hooks: {}", e),
                }
            }
        }
    }

    if target == ClientTarget::ClaudeCode || target == ClientTarget::All {
        if let Some(path) = get_claude_config_path(project) {
            let should_install = target == ClientTarget::ClaudeCode
                || project
                || path.parent().map(|p| p.exists()).unwrap_or(false);
            if should_install {
                match install_claude_hooks(&path, binary) {
                    Ok(_) => {
                        println!("  [ok] Claude Code hooks registered at: {}", path.display());
                        installed_any = true;
                    }
                    Err(e) => println!("  [fail] Failed to install Claude Code hooks: {}", e),
                }
            }
        }
    }

    if target == ClientTarget::Codex || target == ClientTarget::All {
        if let Some(path) = get_codex_config_path(project) {
            let should_install = target == ClientTarget::Codex
                || (project && target == ClientTarget::Codex)
                || path.parent().map(|p| p.exists()).unwrap_or(false);
            if should_install {
                match install_standard_hooks(&path, binary, Some("codex")) {
                    Ok(_) => {
                        println!(
                            "  [ok] OpenAI Codex hooks registered at: {}",
                            path.display()
                        );
                        installed_any = true;
                    }
                    Err(e) => println!("  [fail] Failed to install Codex hooks: {}", e),
                }
            }
        }
    }

    if target == ClientTarget::Grok || target == ClientTarget::All {
        if let Some(path) = get_grok_config_path(project) {
            let should_install = target == ClientTarget::Grok
                || (project && target == ClientTarget::Grok)
                || path.parent().map(|p| p.exists()).unwrap_or(false);
            if should_install {
                match install_standard_hooks(&path, binary, Some("grok")) {
                    Ok(_) => {
                        println!("  [ok] xAI Grok hooks registered at: {}", path.display());
                        installed_any = true;

                        // Clean up duplicate legacy ~/.grok/hooks.json if present
                        if !project {
                            if let Ok(home) =
                                std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME"))
                            {
                                let legacy_grok_hooks =
                                    PathBuf::from(home).join(".grok").join("hooks.json");
                                if legacy_grok_hooks.is_file() {
                                    let _ = uninstall_antigravity_hooks(&legacy_grok_hooks);
                                    println!(
                                        "  [cleanup] Removed duplicate legacy hooks in: {}",
                                        legacy_grok_hooks.display()
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => println!("  [fail] Failed to install Grok hooks: {}", e),
                }
            }
        }
    }

    if target == ClientTarget::Pi || target == ClientTarget::All {
        if let Some(path) = get_pi_config_path(project) {
            let should_install = target == ClientTarget::Pi
                || (project && target == ClientTarget::Pi)
                || path.parent().map(|p| p.exists()).unwrap_or(false);
            if should_install {
                match install_antigravity_hooks(&path, binary, Some("pi")) {
                    Ok(_) => {
                        println!("  [ok] Pi (pi.dev) hooks registered at: {}", path.display());
                        installed_any = true;
                    }
                    Err(e) => println!("  [fail] Failed to install Pi hooks: {}", e),
                }
            }
        }
    }

    if installed_any {
        println!("\nHooks installation complete!");
    } else {
        println!(
            "\nNo configuration files were updated. Use --client <antigravity|claude|codex|grok|pi> to force creation."
        );
    }

    Ok(())
}

pub fn run_uninstall(
    client_str: &str,
    project: bool,
    binary_opt: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let target = ClientTarget::from_str(client_str);
    let binary = binary_opt.unwrap_or("agent-hook");

    println!("\n=== Uninstalling agent-otel-bridge hooks ===");

    if target == ClientTarget::Antigravity || target == ClientTarget::All {
        if let Some(path) = get_antigravity_config_path(project) {
            match uninstall_antigravity_hooks(&path) {
                Ok(true) => println!("  [ok] Antigravity hooks removed from: {}", path.display()),
                Ok(false) => println!(
                    "  [info] Antigravity hooks were not configured in: {}",
                    path.display()
                ),
                Err(e) => println!("  [fail] Error uninstalling Antigravity hooks: {}", e),
            }
        }
    }

    if target == ClientTarget::ClaudeCode || target == ClientTarget::All {
        if let Some(path) = get_claude_config_path(project) {
            match uninstall_claude_hooks(&path, binary) {
                Ok(true) => println!("  [ok] Claude Code hooks removed from: {}", path.display()),
                Ok(false) => println!(
                    "  [info] Claude Code hooks were not configured in: {}",
                    path.display()
                ),
                Err(e) => println!("  [fail] Error uninstalling Claude Code hooks: {}", e),
            }
        }
    }

    if target == ClientTarget::Codex || target == ClientTarget::All {
        if let Some(path) = get_codex_config_path(project) {
            match uninstall_standard_hooks(&path, binary) {
                Ok(true) => println!("  [ok] OpenAI Codex hooks removed from: {}", path.display()),
                Ok(false) => println!(
                    "  [info] OpenAI Codex hooks were not configured in: {}",
                    path.display()
                ),
                Err(e) => println!("  [fail] Error uninstalling Codex hooks: {}", e),
            }
        }
    }

    if target == ClientTarget::Grok || target == ClientTarget::All {
        if let Some(path) = get_grok_config_path(project) {
            match uninstall_standard_hooks(&path, binary) {
                Ok(true) => println!("  [ok] xAI Grok hooks removed from: {}", path.display()),
                Ok(false) => println!(
                    "  [info] xAI Grok hooks were not configured in: {}",
                    path.display()
                ),
                Err(e) => println!("  [fail] Error uninstalling Grok hooks: {}", e),
            }
        }
        if !project {
            if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
                let legacy_grok_hooks = PathBuf::from(home).join(".grok").join("hooks.json");
                let _ = uninstall_antigravity_hooks(&legacy_grok_hooks);
            }
        }
    }

    if target == ClientTarget::Pi || target == ClientTarget::All {
        if let Some(path) = get_pi_config_path(project) {
            match uninstall_antigravity_hooks(&path) {
                Ok(true) => println!("  [ok] Pi hooks removed from: {}", path.display()),
                Ok(false) => println!(
                    "  [info] Pi hooks were not configured in: {}",
                    path.display()
                ),
                Err(e) => println!("  [fail] Error uninstalling Pi hooks: {}", e),
            }
        }
    }

    println!("\nHooks uninstallation complete.\n");
    Ok(())
}

pub fn run_status() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== agent-otel-bridge hooks status ===");

    let canonical_hook = crate::local::get_canonical_hook_path();
    let local_installed = canonical_hook.is_file();
    println!(
        "Canonical local agent-hook: {}",
        if local_installed {
            format!("[ok] present ({})", canonical_hook.display())
        } else {
            "[info] not staged in local runtime bin/ (run 'agent-otel-bridge local install')"
                .to_string()
        }
    );

    let hook_in_path = check_binary_in_path("agent-hook");
    let bridge_in_path = check_binary_in_path("agent-otel-bridge");

    println!("Binaries in PATH:");
    println!(
        "  agent-hook:        {}",
        if hook_in_path {
            "[ok] found"
        } else {
            "[info] not in PATH (not required when absolute path is configured)"
        }
    );
    println!(
        "  agent-otel-bridge: {}",
        if bridge_in_path {
            "[ok] found"
        } else {
            "[info] not in PATH"
        }
    );

    println!("\nClient Hook Registrations:");
    // Antigravity global
    if let Some(p) = get_antigravity_config_path(false) {
        let configured = p.exists()
            && fs::read_to_string(&p)
                .map(|s| s.contains("agent-otel-bridge") || s.contains("agent-hook"))
                .unwrap_or(false);
        println!(
            "  Google Antigravity: {}",
            if configured {
                format!("[ok] configured ({})", p.display())
            } else {
                format!("[unset] not registered ({})", p.display())
            }
        );
    }

    // Claude Code global
    if let Some(p) = get_claude_config_path(false) {
        let configured = p.exists()
            && fs::read_to_string(&p)
                .map(|s| s.contains("agent-hook"))
                .unwrap_or(false);
        println!(
            "  Claude Code:        {}",
            if configured {
                format!("[ok] configured ({})", p.display())
            } else {
                format!("[unset] not registered ({})", p.display())
            }
        );
    }

    // OpenAI Codex global
    if let Some(p) = get_codex_config_path(false) {
        let configured = p.exists()
            && fs::read_to_string(&p)
                .map(|s| s.contains("agent-hook") || s.contains("agent-otel-bridge"))
                .unwrap_or(false);
        println!(
            "  OpenAI Codex:       {}",
            if configured {
                format!("[ok] configured ({})", p.display())
            } else {
                format!("[unset] not registered ({})", p.display())
            }
        );
    }

    // xAI Grok global
    if let Some(p) = get_grok_config_path(false) {
        let configured = p.exists()
            && fs::read_to_string(&p)
                .map(|s| s.contains("agent-hook") || s.contains("agent-otel-bridge"))
                .unwrap_or(false);
        println!(
            "  xAI Grok:           {}",
            if configured {
                format!("[ok] configured ({})", p.display())
            } else {
                format!("[unset] not registered ({})", p.display())
            }
        );
    }

    // Pi (pi.dev) global
    if let Some(p) = get_pi_config_path(false) {
        let configured = p.exists()
            && fs::read_to_string(&p)
                .map(|s| s.contains("agent-otel-bridge") || s.contains("agent-hook"))
                .unwrap_or(false);
        println!(
            "  Pi (pi.dev):        {}",
            if configured {
                format!("[ok] configured ({})", p.display())
            } else {
                format!("[unset] not registered ({})", p.display())
            }
        );
    }

    println!();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_hook_command_with_and_without_spaces() {
        let cmd1 = format_hook_command("C:\\Tools\\agent-hook.exe", "PreToolUse", None);
        assert_eq!(cmd1, "C:\\Tools\\agent-hook.exe PreToolUse");

        let cmd2 = format_hook_command(
            "C:\\Program Files\\Bridge\\agent-hook.exe",
            "PreToolUse",
            Some("claude"),
        );
        assert_eq!(
            cmd2,
            "\"C:\\Program Files\\Bridge\\agent-hook.exe\" PreToolUse --client claude"
        );
    }

    #[test]
    fn test_antigravity_hooks_install_and_uninstall_preserves_third_party() {
        let temp_dir = std::env::temp_dir().join(format!("agy_test_{}", std::process::id()));
        let hooks_path = temp_dir.join("hooks.json");

        // Seed with third-party hook
        fs::create_dir_all(&temp_dir).unwrap();
        let initial_json = json!({
            "herdr": {
                "PreInvocation": [{ "command": "powershell herdr.ps1", "timeout": 10 }]
            }
        });
        fs::write(&hooks_path, serde_json::to_string(&initial_json).unwrap()).unwrap();

        // Install
        let res = install_antigravity_hooks(&hooks_path, "C:\\Bridge\\agent-hook.exe", None);
        assert!(res.is_ok());
        assert!(hooks_path.exists());

        let content = fs::read_to_string(&hooks_path).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();

        // Check third-party hook is preserved
        assert!(parsed.get("herdr").is_some());
        assert_eq!(
            parsed["herdr"]["PreInvocation"][0]["command"],
            "powershell herdr.ps1"
        );

        // Check bridge hooks
        let bridge = &parsed["agent-otel-bridge"];
        assert_eq!(
            bridge["PreToolUse"][0]["hooks"][0]["command"],
            "C:\\Bridge\\agent-hook.exe PreToolUse"
        );

        // Uninstall
        let uninst = uninstall_antigravity_hooks(&hooks_path);
        assert!(uninst.is_ok());
        assert!(uninst.unwrap());

        let after_content = fs::read_to_string(&hooks_path).unwrap();
        let after_parsed: Value = serde_json::from_str(&after_content).unwrap();
        assert!(after_parsed.get("agent-otel-bridge").is_none());
        assert!(after_parsed.get("herdr").is_some());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_claude_hooks_preserves_third_party_and_idempotent() {
        let temp_dir = std::env::temp_dir().join(format!("claude_test_{}", std::process::id()));
        let settings_path = temp_dir.join("settings.json");

        // Seed with third-party hook
        fs::create_dir_all(&temp_dir).unwrap();
        let initial_json = json!({
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "Bash",
                        "hooks": [{ "command": "rtk hook claude", "type": "command" }]
                    }
                ]
            }
        });
        fs::write(
            &settings_path,
            serde_json::to_string(&initial_json).unwrap(),
        )
        .unwrap();

        // Install
        let res = install_claude_hooks(&settings_path, "C:\\Bridge\\agent-hook.exe");
        assert!(res.is_ok());

        let content = fs::read_to_string(&settings_path).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();
        let pre_tool = parsed["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre_tool.len(), 2);
        assert_eq!(pre_tool[0]["hooks"][0]["command"], "rtk hook claude");
        assert_eq!(
            pre_tool[1]["hooks"][0]["command"],
            "C:\\Bridge\\agent-hook.exe PreToolUse"
        );

        // Re-install (idempotent update check)
        let res2 = install_claude_hooks(&settings_path, "C:\\NewPath\\agent-hook.exe");
        assert!(res2.is_ok());

        let content2 = fs::read_to_string(&settings_path).unwrap();
        let parsed2: Value = serde_json::from_str(&content2).unwrap();
        let pre_tool2 = parsed2["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre_tool2.len(), 2);
        assert_eq!(pre_tool2[0]["hooks"][0]["command"], "rtk hook claude");
        assert_eq!(
            pre_tool2[1]["hooks"][0]["command"],
            "C:\\NewPath\\agent-hook.exe PreToolUse"
        );

        // Uninstall
        let uninst = uninstall_claude_hooks(&settings_path, "C:\\NewPath\\agent-hook.exe");
        assert!(uninst.is_ok());
        assert!(uninst.unwrap());

        let after_content = fs::read_to_string(&settings_path).unwrap();
        let after_parsed: Value = serde_json::from_str(&after_content).unwrap();
        let pre_tool_after = after_parsed["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre_tool_after.len(), 1);
        assert_eq!(pre_tool_after[0]["hooks"][0]["command"], "rtk hook claude");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_client_target_parsing() {
        assert_eq!(
            ClientTarget::from_str("antigravity"),
            ClientTarget::Antigravity
        );
        assert_eq!(ClientTarget::from_str("agy"), ClientTarget::Antigravity);
        assert_eq!(ClientTarget::from_str("claude"), ClientTarget::ClaudeCode);
        assert_eq!(
            ClientTarget::from_str("claude-code"),
            ClientTarget::ClaudeCode
        );
        assert_eq!(ClientTarget::from_str("codex"), ClientTarget::Codex);
        assert_eq!(ClientTarget::from_str("grok"), ClientTarget::Grok);
        assert_eq!(ClientTarget::from_str("pi"), ClientTarget::Pi);
        assert_eq!(ClientTarget::from_str("all"), ClientTarget::All);
        assert_eq!(ClientTarget::from_str("anything_else"), ClientTarget::All);
    }

    #[test]
    fn test_codex_grok_pi_hooks_install_and_uninstall() {
        let temp_dir =
            std::env::temp_dir().join(format!("other_agents_test_{}", std::process::id()));
        let codex_path = temp_dir.join(".codex").join("hooks.json");
        let grok_path = temp_dir.join(".grok").join("hooks.json");
        let pi_path = temp_dir.join(".pi").join("hooks.json");

        for path in &[&codex_path, &grok_path, &pi_path] {
            let res = install_antigravity_hooks(path, "agent-hook", None);
            assert!(res.is_ok());
            assert!(path.exists());
            let content = fs::read_to_string(path).unwrap();
            assert!(content.contains("agent-otel-bridge"));

            let uninst = uninstall_antigravity_hooks(path);
            assert!(uninst.is_ok());
            assert!(uninst.unwrap());
            let after = fs::read_to_string(path).unwrap();
            assert!(!after.contains("agent-otel-bridge"));
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
