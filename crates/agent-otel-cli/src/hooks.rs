/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientTarget {
    Antigravity,
    ClaudeCode,
    Codex,
    Grok,
    Pi,
    Named(String),
    All,
}

impl ClientTarget {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        let lower = s.trim().to_ascii_lowercase();
        match lower.as_str() {
            "all" | "*" | "" => ClientTarget::All,
            "antigravity" | "agy" | "gemini" => ClientTarget::Antigravity,
            "claude" | "claude-code" | "claudecode" => ClientTarget::ClaudeCode,
            "codex" | "openai" | "codex-cli" => ClientTarget::Codex,
            "grok" | "xai" | "grok-cli" => ClientTarget::Grok,
            "pi" | "pi-cli" => ClientTarget::Pi,
            other => ClientTarget::Named(other.to_string()),
        }
    }
}

impl std::str::FromStr for ClientTarget {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from_str(s))
    }
}

pub fn get_antigravity_project_path(base: Option<&Path>) -> Option<PathBuf> {
    let root = base.unwrap_or_else(|| Path::new("."));
    Some(root.join(".gemini").join("hooks.json"))
}

pub fn get_antigravity_config_path(project: bool) -> Option<PathBuf> {
    if project {
        get_antigravity_project_path(None)
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

pub fn get_claude_project_path(base: Option<&Path>) -> Option<PathBuf> {
    let root = base.unwrap_or_else(|| Path::new("."));
    Some(root.join(".claude").join("settings.json"))
}

pub fn get_claude_config_path(project: bool) -> Option<PathBuf> {
    if project {
        get_claude_project_path(None)
    } else {
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok()
            .map(|home| PathBuf::from(home).join(".claude").join("settings.json"))
    }
}

pub fn get_codex_project_path(base: Option<&Path>) -> Option<PathBuf> {
    let root = base.unwrap_or_else(|| Path::new("."));
    Some(root.join(".codex").join("hooks.json"))
}

pub fn get_codex_config_path(project: bool) -> Option<PathBuf> {
    if project {
        get_codex_project_path(None)
    } else {
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok()
            .map(|home| PathBuf::from(home).join(".codex").join("hooks.json"))
    }
}

pub fn get_grok_project_path(base: Option<&Path>) -> Option<PathBuf> {
    let root = base.unwrap_or_else(|| Path::new("."));
    Some(root.join(".grok").join("hooks").join("agent-otel.json"))
}

pub fn get_grok_config_path(project: bool) -> Option<PathBuf> {
    if project {
        get_grok_project_path(None)
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

pub fn get_pi_project_path(base: Option<&Path>) -> Option<PathBuf> {
    let root = base.unwrap_or_else(|| Path::new("."));
    Some(root.join(".pi").join("hooks.json"))
}

pub fn get_pi_config_path(project: bool) -> Option<PathBuf> {
    if project {
        get_pi_project_path(None)
    } else {
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok()
            .map(|home| PathBuf::from(home).join(".pi").join("hooks.json"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookScope {
    /// Purely global configuration (e.g. Grok, Codex CLI, Pi).
    /// Projects never override or shadow global telemetry.
    GlobalOnly,
    /// Namespace-merged configuration (e.g. Google Antigravity).
    /// Projects can define local hooks in their own namespaces without shadowing the bridge.
    NamespaceMerged,
    /// Array-overridden configuration (e.g. Claude Code).
    /// A project's local config file completely shadows the global hooks array unless the bridge is also present in the project.
    ProjectShadowsGlobal,
}

pub type InstallHookFn = fn(&Path, &str, Option<&str>) -> Result<bool, Box<dyn std::error::Error>>;
pub type UninstallHookFn = fn(&Path, &str) -> Result<bool, Box<dyn std::error::Error>>;

#[derive(Clone, Copy)]
pub struct ClientAdapter {
    pub id: &'static str,
    pub display_name: &'static str,
    pub aliases: &'static [&'static str],
    pub client_tag: Option<&'static str>,
    pub scope: HookScope,
    pub workspace_markers: &'static [&'static str],
    pub global_config_fn: fn() -> Option<PathBuf>,
    pub project_config_fn: fn(Option<&Path>) -> Option<PathBuf>,
    pub install_fn: InstallHookFn,
    pub uninstall_fn: UninstallHookFn,
    pub is_registered_fn: fn(&Path) -> bool,
}

impl ClientAdapter {
    pub fn matches_name(&self, s: &str) -> bool {
        let lower = s.to_ascii_lowercase();
        self.id == lower || self.aliases.iter().any(|&a| a == lower)
    }
}

impl ClientTarget {
    pub fn matches_adapter(&self, adapter: &ClientAdapter) -> bool {
        match self {
            ClientTarget::All => true,
            ClientTarget::Antigravity => adapter.matches_name("antigravity"),
            ClientTarget::ClaudeCode => adapter.matches_name("claude"),
            ClientTarget::Codex => adapter.matches_name("codex"),
            ClientTarget::Grok => adapter.matches_name("grok"),
            ClientTarget::Pi => adapter.matches_name("pi"),
            ClientTarget::Named(name) => adapter.matches_name(name),
        }
    }
}

pub const CLIENT_ADAPTERS: &[ClientAdapter] = &[
    ClientAdapter {
        id: "antigravity",
        display_name: "Google Antigravity",
        aliases: &["agy", "gemini"],
        client_tag: None,
        scope: HookScope::NamespaceMerged,
        workspace_markers: &[".gemini", ".agents"],
        global_config_fn: || get_antigravity_config_path(false),
        project_config_fn: get_antigravity_project_path,
        install_fn: install_antigravity_hooks,
        uninstall_fn: |path, _bin| uninstall_antigravity_hooks(path),
        is_registered_fn: |path| {
            path.exists()
                && fs::read_to_string(path)
                    .map(|s| s.contains("agent-otel-bridge") || s.contains("agent-hook"))
                    .unwrap_or(false)
        },
    },
    ClientAdapter {
        id: "claude",
        display_name: "Claude Code",
        aliases: &["claude-code", "claudecode"],
        client_tag: None,
        scope: HookScope::ProjectShadowsGlobal,
        workspace_markers: &[".claude"],
        global_config_fn: || get_claude_config_path(false),
        project_config_fn: get_claude_project_path,
        install_fn: |path, binary, _tag| install_claude_hooks(path, binary),
        uninstall_fn: uninstall_claude_hooks,
        is_registered_fn: |path| {
            path.exists()
                && fs::read_to_string(path)
                    .map(|s| s.contains("agent-hook"))
                    .unwrap_or(false)
        },
    },
    ClientAdapter {
        id: "codex",
        display_name: "OpenAI Codex",
        aliases: &["openai", "codex-cli"],
        client_tag: Some("codex"),
        scope: HookScope::GlobalOnly,
        workspace_markers: &[".codex"],
        global_config_fn: || get_codex_config_path(false),
        project_config_fn: get_codex_project_path,
        install_fn: install_standard_hooks,
        uninstall_fn: uninstall_standard_hooks,
        is_registered_fn: |path| {
            path.exists()
                && fs::read_to_string(path)
                    .map(|s| s.contains("agent-hook") || s.contains("agent-otel-bridge"))
                    .unwrap_or(false)
        },
    },
    ClientAdapter {
        id: "grok",
        display_name: "xAI Grok",
        aliases: &["xai", "grok-cli"],
        client_tag: Some("grok"),
        scope: HookScope::GlobalOnly,
        workspace_markers: &[".grok"],
        global_config_fn: || get_grok_config_path(false),
        project_config_fn: get_grok_project_path,
        install_fn: install_grok_hooks,
        uninstall_fn: uninstall_standard_hooks,
        is_registered_fn: |path| {
            path.exists()
                && fs::read_to_string(path)
                    .map(|s| s.contains("agent-hook") || s.contains("agent-otel-bridge"))
                    .unwrap_or(false)
        },
    },
    ClientAdapter {
        id: "pi",
        display_name: "Pi (pi.dev)",
        aliases: &["pi-cli"],
        client_tag: Some("pi"),
        scope: HookScope::GlobalOnly,
        workspace_markers: &[".pi"],
        global_config_fn: || get_pi_config_path(false),
        project_config_fn: get_pi_project_path,
        install_fn: install_pi_hooks,
        uninstall_fn: |path, _bin| uninstall_antigravity_hooks(path),
        is_registered_fn: |path| {
            path.exists()
                && fs::read_to_string(path)
                    .map(|s| s.contains("agent-otel-bridge") || s.contains("agent-hook"))
                    .unwrap_or(false)
        },
    },
];

pub fn is_bridge_command(cmd: &str) -> bool {
    let lower = cmd.to_ascii_lowercase();
    lower.contains("agent-hook") || lower.contains("agent-otel-bridge hook")
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
        format_cmd_hook_command(binary, event, client_tag)
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

    let pre_tool = format_antigravity_hook_command(binary, "PreToolUse", client_tag);
    let post_tool = format_antigravity_hook_command(binary, "PostToolUse", client_tag);
    let pre_inv = format_antigravity_hook_command(binary, "PreInvocation", client_tag);
    let post_inv = format_antigravity_hook_command(binary, "PostInvocation", client_tag);
    let stop = format_antigravity_hook_command(binary, "Stop", client_tag);

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
    install_hooks_with_command_formatter(path, binary, client_tag, format_hook_command)
}

fn install_hooks_with_command_formatter(
    path: &Path,
    binary: &str,
    client_tag: Option<&str>,
    formatter: fn(&str, &str, Option<&str>) -> String,
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
        let target_cmd = formatter(binary, event, client_tag);

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

pub fn install_pi_hooks(
    path: &Path,
    binary: &str,
    client_tag: Option<&str>,
) -> Result<bool, Box<dyn std::error::Error>> {
    install_hooks_with_command_formatter(path, binary, client_tag, format_hook_command)
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

    for adapter in CLIENT_ADAPTERS {
        if !target.matches_adapter(adapter) {
            continue;
        }

        let config_path = if project {
            (adapter.project_config_fn)(None)
        } else {
            (adapter.global_config_fn)()
        };

        let Some(path) = config_path else {
            continue;
        };

        let is_explicit = target != ClientTarget::All;
        let should_install =
            is_explicit || project || path.parent().map(|p| p.exists()).unwrap_or(false);

        if should_install {
            match (adapter.install_fn)(&path, binary, adapter.client_tag) {
                Ok(_) => {
                    println!(
                        "  [ok] {} hooks registered at: {}",
                        adapter.display_name,
                        path.display()
                    );
                    installed_any = true;

                    if adapter.id == "grok" && !project {
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
                Err(e) => {
                    println!(
                        "  [fail] Failed to install {} hooks: {}",
                        adapter.display_name, e
                    )
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

    for adapter in CLIENT_ADAPTERS {
        if !target.matches_adapter(adapter) {
            continue;
        }

        let config_path = if project {
            (adapter.project_config_fn)(None)
        } else {
            (adapter.global_config_fn)()
        };

        let Some(path) = config_path else {
            continue;
        };

        match (adapter.uninstall_fn)(&path, binary) {
            Ok(true) => println!(
                "  [ok] {} hooks removed from: {}",
                adapter.display_name,
                path.display()
            ),
            Ok(false) => println!(
                "  [info] {} hooks were not configured in: {}",
                adapter.display_name,
                path.display()
            ),
            Err(e) => println!(
                "  [fail] Error uninstalling {} hooks: {}",
                adapter.display_name, e
            ),
        }

        if adapter.id == "grok" && !project {
            if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
                let legacy_grok_hooks = PathBuf::from(home).join(".grok").join("hooks.json");
                let _ = uninstall_antigravity_hooks(&legacy_grok_hooks);
            }
        }
    }

    println!("\nHooks uninstallation complete.\n");
    Ok(())
}

pub fn run_sync(
    workspace_opt: Option<&Path>,
    binary_opt: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let base = workspace_opt.unwrap_or_else(|| Path::new("."));
    let canonical_base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let resolved_binary = resolve_canonical_hook_binary(binary_opt)?;
    let binary = &resolved_binary;

    println!("\n=== agent-otel-bridge hooks sync ===");
    println!("Workspace: {}", canonical_base.display());
    println!("Target binary: {}\n", binary);

    let mut actions_taken = 0;

    for adapter in CLIENT_ADAPTERS {
        let proj_cfg = (adapter.project_config_fn)(Some(&canonical_base));
        let Some(path) = proj_cfg else {
            continue;
        };

        match adapter.scope {
            HookScope::GlobalOnly => {
                println!(
                    "  {:<20} [info] Purely global (no workspace shadowing)",
                    adapter.display_name
                );
            }
            HookScope::NamespaceMerged => {
                if path.exists() {
                    if (adapter.is_registered_fn)(&path) {
                        println!(
                            "  {:<20} [ok] Up-to-date in workspace ({})",
                            adapter.display_name,
                            path.display()
                        );
                    } else {
                        (adapter.install_fn)(&path, binary, adapter.client_tag)?;
                        println!(
                            "  {:<20} [synced] Injected into workspace ({})",
                            adapter.display_name,
                            path.display()
                        );
                        actions_taken += 1;
                    }
                } else {
                    println!(
                        "  {:<20} [clean] No local override (global hooks active)",
                        adapter.display_name
                    );
                }
            }
            HookScope::ProjectShadowsGlobal => {
                if path.exists() {
                    if (adapter.is_registered_fn)(&path) {
                        println!(
                            "  {:<20} [ok] Up-to-date in workspace ({})",
                            adapter.display_name,
                            path.display()
                        );
                    } else {
                        println!(
                            "  {:<20} [shadow] Local config without bridge detected! Syncing hook...",
                            adapter.display_name
                        );
                        (adapter.install_fn)(&path, binary, adapter.client_tag)?;
                        println!(
                            "  {:<20} [synced] Successfully updated workspace ({})",
                            adapter.display_name,
                            path.display()
                        );
                        actions_taken += 1;
                    }
                } else {
                    println!(
                        "  {:<20} [clean] No local override (global hooks active)",
                        adapter.display_name
                    );
                }
            }
        }
    }

    if actions_taken > 0 {
        println!("\nSync completed: {actions_taken} workspace configuration(s) updated.\n");
    } else {
        println!("\nSync completed: all clients are already aligned.\n");
    }

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

    println!("\nClient Hook Registrations (Global):");
    for adapter in CLIENT_ADAPTERS {
        if let Some(p) = (adapter.global_config_fn)() {
            let configured = (adapter.is_registered_fn)(&p);
            println!(
                "  {:<20} {}",
                format!("{}:", adapter.display_name),
                if configured {
                    format!("[ok] configured ({})", p.display())
                } else {
                    format!("[unset] not registered ({})", p.display())
                }
            );
        }
    }

    if let Ok(current_dir) = std::env::current_dir() {
        let mut has_project_cfgs = false;
        for adapter in CLIENT_ADAPTERS {
            if let Some(p) = (adapter.project_config_fn)(Some(&current_dir)) {
                if p.exists() {
                    if !has_project_cfgs {
                        println!("\nCurrent Workspace Overrides ({}):", current_dir.display());
                        has_project_cfgs = true;
                    }
                    let registered = (adapter.is_registered_fn)(&p);
                    let status_str = match adapter.scope {
                        HookScope::ProjectShadowsGlobal if !registered => {
                            format!("[shadow] active WITHOUT bridge hook ({})", p.display())
                        }
                        HookScope::ProjectShadowsGlobal => {
                            format!("[ok] active with bridge hook ({})", p.display())
                        }
                        _ => format!("[ok] present ({})", p.display()),
                    };
                    println!(
                        "  {:<20} {}",
                        format!("{}:", adapter.display_name),
                        status_str
                    );
                }
            }
        }
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
        assert_eq!(cmd1, "\"C:\\Tools\\agent-hook.exe\" PreToolUse");

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
    fn test_format_hook_command_preserves_quoted_and_unix_paths() {
        let quoted = format_hook_command(
            "\"C:\\Program Files\\Bridge\\agent-hook.exe\"",
            "Stop",
            None,
        );
        assert_eq!(quoted, "\"C:\\Program Files\\Bridge\\agent-hook.exe\" Stop");

        let unix = format_hook_command("/opt/Agent Otel/agent-hook", "Stop", None);
        assert_eq!(unix, "\"/opt/Agent Otel/agent-hook\" Stop");
    }

    #[test]
    fn powershell_renderer_uses_call_operator_for_quoted_executable() {
        let command = format_powershell_hook_command(
            "C:\\Program Files\\Agent Bridge\\agent-hook.exe",
            "PreToolUse",
            Some("grok"),
        );
        assert_eq!(
            command,
            "& \"C:\\Program Files\\Agent Bridge\\agent-hook.exe\" PreToolUse --client grok"
        );
    }

    #[test]
    fn cmd_renderer_nests_quotes_around_command_line() {
        assert_eq!(
            format_cmd_hook_command("C:\\Program Files\\agent-hook.exe", "Stop", None),
            "call \"C:\\Program Files\\agent-hook.exe\" Stop"
        );
    }

    #[test]
    fn grok_registration_is_powershell_safe_and_idempotent() {
        let dir = std::env::temp_dir().join(format!("grok_renderer_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agent-otel.json");
        install_grok_hooks(&path, "C:\\Program Files\\agent-hook.exe", Some("grok")).unwrap();
        let first = fs::read_to_string(&path).unwrap();
        install_grok_hooks(&path, "C:\\Program Files\\agent-hook.exe", Some("grok")).unwrap();
        let second = fs::read_to_string(&path).unwrap();
        assert_eq!(first, second);
        let parsed: Value = serde_json::from_str(&first).unwrap();
        let command = parsed["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(command.starts_with("& \""));
        assert!(command.ends_with(" PreToolUse --client grok"));
        assert!(command.contains("C:\\Program Files\\agent-hook.exe"));
        let _ = fs::remove_dir_all(dir);
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
        let command = bridge["PreToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        if cfg!(windows) {
            assert_eq!(command, "call \"C:\\Bridge\\agent-hook.exe\" PreToolUse");
        } else {
            assert_eq!(command, "\"C:\\Bridge\\agent-hook.exe\" PreToolUse");
        }

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
            "\"C:\\Bridge\\agent-hook.exe\" PreToolUse"
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
            "\"C:\\NewPath\\agent-hook.exe\" PreToolUse"
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
        assert_eq!(ClientTarget::from_str("*"), ClientTarget::All);
        assert_eq!(
            ClientTarget::from_str("hermes"),
            ClientTarget::Named("hermes".to_string())
        );
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

    #[test]
    fn test_client_adapters_registry_and_sync() {
        assert_eq!(CLIENT_ADAPTERS.len(), 5);
        let claude = CLIENT_ADAPTERS.iter().find(|a| a.id == "claude").unwrap();
        assert_eq!(claude.scope, HookScope::ProjectShadowsGlobal);

        let agy = CLIENT_ADAPTERS
            .iter()
            .find(|a| a.id == "antigravity")
            .unwrap();
        assert_eq!(agy.scope, HookScope::NamespaceMerged);

        let codex = CLIENT_ADAPTERS.iter().find(|a| a.id == "codex").unwrap();
        assert_eq!(codex.scope, HookScope::GlobalOnly);

        let temp_dir = std::env::temp_dir().join(format!("sync_test_{}", std::process::id()));
        fs::create_dir_all(&temp_dir).unwrap();

        // Seed project claude settings without bridge hook
        let claude_dir = temp_dir.join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        let settings_path = claude_dir.join("settings.json");
        fs::write(
            &settings_path,
            json!({
                "hooks": {
                    "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "command": "my-check.sh" }] }]
                }
            })
            .to_string(),
        )
        .unwrap();

        // Sync should detect shadowing and inject bridge hook
        let sync_res = run_sync(Some(&temp_dir), Some("C:\\Tools\\agent-hook.exe"));
        assert!(sync_res.is_ok());

        let synced_content = fs::read_to_string(&settings_path).unwrap();
        let parsed: Value = serde_json::from_str(&synced_content).unwrap();
        let pre_tool = parsed["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre_tool.len(), 2);
        assert_eq!(pre_tool[0]["hooks"][0]["command"], "my-check.sh");
        assert_eq!(
            pre_tool[1]["hooks"][0]["command"],
            "\"C:\\Tools\\agent-hook.exe\" PreToolUse"
        );

        // Running sync a second time is idempotent
        let sync_res2 = run_sync(Some(&temp_dir), Some("C:\\Tools\\agent-hook.exe"));
        assert!(sync_res2.is_ok());

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
