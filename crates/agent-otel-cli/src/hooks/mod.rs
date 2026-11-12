/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

mod antigravity;
mod claude;
mod codex;
mod command;
mod grok;
#[cfg(target_os = "linux")]
mod home;
#[cfg(target_os = "linux")]
pub mod home_apply;
mod paths;
mod pi;
mod standard;

pub use antigravity::{
    antigravity_bridge_hooks, install_antigravity_hooks, uninstall_antigravity_hooks,
};
pub use claude::{install_claude_hooks, uninstall_claude_hooks};
pub use codex::{
    format_codex_hook_command, install_codex_hooks, load_codex_hook_shell, CodexHookShell,
};
#[cfg(target_os = "linux")]
pub use command::format_posix_hook_command;
pub use command::{
    check_binary_in_path, claude_imported_grok_guard_script, find_binary_in_path,
    format_antigravity_hook_command, format_claude_hook_command,
    format_claude_windows_hook_command, format_cmd_hook_command, format_grok_hook_command,
    format_hook_command, format_powershell_hook_command, has_bridge_command, is_bridge_command,
    resolve_canonical_hook_binary,
};
pub use grok::install_grok_hooks;
#[cfg(target_os = "linux")]
pub use home::{plan_home_hooks, HomeHookChange};
pub use paths::{
    get_antigravity_config_path, get_antigravity_project_path, get_claude_config_path,
    get_claude_project_path, get_codex_config_path, get_codex_project_path, get_grok_config_path,
    get_grok_project_path, get_pi_config_path, get_pi_project_path,
};
pub use pi::{install_pi_hooks, uninstall_pi_hooks};
pub use standard::{install_standard_hooks, uninstall_standard_hooks};

use pi::pi_extension_path;
use serde_json::Value;
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
            fs::read_to_string(path)
                .ok()
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .is_some_and(|value| has_bridge_command(&value))
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
        install_fn: install_codex_hooks,
        uninstall_fn: uninstall_standard_hooks,
        is_registered_fn: |path| {
            path.exists()
                && fs::read_to_string(path)
                    .ok()
                    .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                    .is_some_and(|value| has_bridge_command(&value))
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
        uninstall_fn: uninstall_pi_hooks,
        is_registered_fn: |path| {
            pi_extension_path(path)
                .ok()
                .and_then(|extension| fs::read_to_string(extension).ok())
                .is_some_and(|s| s.starts_with("// Managed by agent-otel-bridge."))
        },
    },
];

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
    let mut failures = Vec::new();

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
                    );
                    failures.push(format!("{}: {e}", adapter.id));
                }
            }
        }
    }

    if !failures.is_empty() {
        return Err(format!(
            "Hook installation incomplete (successful registrations remain applied): {}",
            failures.join("; ")
        )
        .into());
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
            "[info] not active in local runtime bin/ (run 'agent-otel-bridge local install --activate')"
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
mod tests;
