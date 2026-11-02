/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::hooks::{
    antigravity_bridge_hooks, install_claude_hooks, install_codex_hooks, install_grok_hooks,
    install_pi_hooks, uninstall_pi_hooks, uninstall_standard_hooks,
};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const PLUGIN_VERSION: &str = "0.7.0";
const PLUGIN_DESCRIPTION: &str = "OpenTelemetry lifecycle hooks";

pub trait ProcessRunner {
    fn run_command(&mut self, program: &str, args: &[String]) -> Result<i32, String>;
}

pub struct ProductionProcess;

impl ProcessRunner for ProductionProcess {
    fn run_command(&mut self, program: &str, args: &[String]) -> Result<i32, String> {
        run_command(program, args)
    }
}

pub fn run_command(program: &str, args: &[String]) -> Result<i32, String> {
    Command::new(program)
        .args(args)
        .status()
        .map(|status| status.code().unwrap_or(1))
        .map_err(|err| format!("cannot spawn {program}: {err}"))
}

pub fn user_home() -> Result<PathBuf, String> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".to_string())
}

pub fn ensure_daemon_running() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Starting agent-otel-bridge daemon ===");

    if agent_otel_ipc::client::try_send(agent_otel_ipc::frame::MsgType::HealthPing, &[]).is_ok() {
        println!("  [ok] Daemon is already running and listening on named pipe.\n");
        return Ok(());
    }

    agent_otel_ipc::client::spawn_daemon_detached();

    let start = std::time::Instant::now();
    let mut started = false;
    while start.elapsed() < std::time::Duration::from_millis(1500) {
        std::thread::sleep(std::time::Duration::from_millis(50));
        if agent_otel_ipc::client::try_send(agent_otel_ipc::frame::MsgType::HealthPing, &[]).is_ok()
        {
            started = true;
            break;
        }
    }

    if started {
        println!("  [ok] Background daemon successfully launched and listening on named pipe!\n");
    } else {
        println!("  [warn] Spawn signal sent, but daemon has not yet responded on named pipe.");
        println!("         Run 'agent-otel-bridge doctor' for pipeline diagnostics.\n");
    }

    Ok(())
}

pub fn enable_app(
    app: &str,
    home: &Path,
    stage: &Path,
    binary: &str,
    runner: &mut impl ProcessRunner,
    mut daemon_running: impl FnMut() -> bool,
    mut start_daemon: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    let app = canonical_app(app)?;
    if !daemon_running() {
        start_daemon()?;
    }
    match app {
        "claude" => enable_claude(stage, binary, runner),
        "codex" => enable_codex(stage, binary, runner),
        "antigravity" => enable_antigravity(stage, binary, runner),
        "grok" => install_grok_hooks(&grok_config(home), binary, Some("grok"))
            .map(|_| ())
            .map_err(|err| err.to_string()),
        "pi" => install_pi_hooks(&pi_config(home), binary, Some("pi"))
            .map(|_| ())
            .map_err(|err| err.to_string()),
        _ => unreachable!("canonical app"),
    }
}

pub fn disable_app(app: &str, home: &Path, runner: &mut impl ProcessRunner) -> Result<(), String> {
    match canonical_app(app)? {
        "claude" => invoke(
            runner,
            "claude",
            &[
                "plugin",
                "uninstall",
                "agent-otel-bridge@agent-otel-bridge",
                "--scope",
                "user",
                "--json",
            ],
        ),
        // `plugin remove` examples require PLUGIN@MARKETPLACE, not a bare name.
        "codex" => invoke(
            runner,
            "codex",
            &[
                "plugin",
                "remove",
                "agent-otel-bridge@agent-otel-bridge",
                "--json",
            ],
        ),
        "antigravity" => invoke(runner, "agy", &["plugin", "uninstall", "agent-otel-bridge"]),
        "grok" => uninstall_standard_hooks(&grok_config(home), "agent-hook")
            .map(|_| ())
            .map_err(|err| err.to_string()),
        "pi" => uninstall_pi_hooks(&pi_config(home), "agent-hook")
            .map(|_| ())
            .map_err(|err| err.to_string()),
        _ => unreachable!("canonical app"),
    }
}

fn canonical_app(app: &str) -> Result<&'static str, String> {
    match app.trim().to_ascii_lowercase().as_str() {
        "claude" => Ok("claude"),
        "codex" => Ok("codex"),
        "antigravity" => Ok("antigravity"),
        "grok" => Ok("grok"),
        "pi" => Ok("pi"),
        other => Err(format!("unknown application '{other}'")),
    }
}

fn grok_config(home: &Path) -> PathBuf {
    home.join(".grok").join("hooks").join("agent-otel.json")
}

fn pi_config(home: &Path) -> PathBuf {
    home.join(".pi").join("hooks.json")
}

fn enable_claude(
    stage: &Path,
    binary: &str,
    runner: &mut impl ProcessRunner,
) -> Result<(), String> {
    let root = stage.join("claude");
    write_json(
        &root.join(".claude-plugin").join("marketplace.json"),
        &json!({
            "name": "agent-otel-bridge",
            "owner": {"name": "agent-otel-bridge"},
            "description": PLUGIN_DESCRIPTION,
            "plugins": [{
                "name": "agent-otel-bridge",
                "source": "./plugins/agent-otel-bridge",
                "description": PLUGIN_DESCRIPTION
            }]
        }),
    )?;
    let plugin = root.join("plugins").join("agent-otel-bridge");
    write_json(
        &plugin.join(".claude-plugin").join("plugin.json"),
        &json!({
            "name": "agent-otel-bridge",
            "version": PLUGIN_VERSION,
            "description": PLUGIN_DESCRIPTION,
            "author": {"name": "agent-otel-bridge"},
            "license": "Apache-2.0"
        }),
    )?;
    install_claude_hooks(&plugin.join("hooks").join("hooks.json"), binary)
        .map_err(|err| err.to_string())?;
    let root_arg = path_arg(&root)?;
    invoke(
        runner,
        "claude",
        &[
            "plugin",
            "marketplace",
            "add",
            &root_arg,
            "--scope",
            "user",
            "--json",
        ],
    )?;
    invoke(
        runner,
        "claude",
        &[
            "plugin",
            "install",
            "agent-otel-bridge@agent-otel-bridge",
            "--scope",
            "user",
            "-y",
            "--json",
        ],
    )
}

fn enable_codex(stage: &Path, binary: &str, runner: &mut impl ProcessRunner) -> Result<(), String> {
    let root = stage.join("codex");
    let plugin = root.join("plugins").join("agent-otel-bridge");
    // Local marketplace filename required by `codex plugin marketplace add`.
    write_json(
        &root
            .join(".agents")
            .join("plugins")
            .join("marketplace.json"),
        &json!({
            "name": "agent-otel-bridge",
            "interface": {"displayName": "agent-otel-bridge"},
            "plugins": [{
                "name": "agent-otel-bridge",
                "source": {"source": "local", "path": "./plugins/agent-otel-bridge"},
                "policy": {"installation": "AVAILABLE", "authentication": "ON_INSTALL"},
                "category": "Productivity"
            }]
        }),
    )?;
    write_json(
        &plugin.join(".codex-plugin").join("plugin.json"),
        &json!({
            "name": "agent-otel-bridge",
            "version": PLUGIN_VERSION,
            "description": PLUGIN_DESCRIPTION,
            "hooks": "./hooks/hooks.json"
        }),
    )?;
    install_codex_hooks(
        &plugin.join("hooks").join("hooks.json"),
        binary,
        Some("codex"),
    )
    .map_err(|err| err.to_string())?;
    let root_arg = path_arg(&root)?;
    invoke(
        runner,
        "codex",
        &["plugin", "marketplace", "add", &root_arg],
    )?;
    invoke(
        runner,
        "codex",
        &[
            "plugin",
            "add",
            "agent-otel-bridge@agent-otel-bridge",
            "--json",
        ],
    )
}

fn enable_antigravity(
    stage: &Path,
    binary: &str,
    runner: &mut impl ProcessRunner,
) -> Result<(), String> {
    let root = stage.join("antigravity");
    write_json(
        &root.join("plugin.json"),
        &json!({
            "$schema": "https://antigravity.google/schemas/v1/plugin.json",
            "name": "agent-otel-bridge",
            "displayName": "Agent OTel Bridge",
            "version": PLUGIN_VERSION,
            "description": PLUGIN_DESCRIPTION
        }),
    )?;
    // Top-level key is the named hook group. The value is the object the
    // Antigravity installer stores under `agent-otel-bridge`.
    write_json(
        &root.join("hooks.json"),
        &json!({
            "agent-otel-bridge": antigravity_bridge_hooks(binary, None)
        }),
    )?;
    let root_arg = path_arg(&root)?;
    invoke(runner, "agy", &["plugin", "install", &root_arg])
}

fn invoke(runner: &mut impl ProcessRunner, program: &str, args: &[&str]) -> Result<(), String> {
    let owned: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    match runner.run_command(program, &owned) {
        Ok(0) => Ok(()),
        Ok(code) => Err(format!("{program} exited with status {code}")),
        Err(err) => Err(err),
    }
}

fn path_arg(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| format!("path is not utf-8: {}", path.display()))
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let body = serde_json::to_string_pretty(value).map_err(|err| err.to_string())?;
    fs::write(path, body + "\n").map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::{
        format_antigravity_hook_command, format_claude_hook_command, CodexHookShell,
    };
    use serde_json::Value;

    struct Recorder {
        calls: Vec<(String, Vec<String>)>,
        results: Vec<Result<i32, String>>,
    }

    impl ProcessRunner for Recorder {
        fn run_command(&mut self, program: &str, args: &[String]) -> Result<i32, String> {
            self.calls.push((program.to_string(), args.to_vec()));
            if self.results.is_empty() {
                Ok(0)
            } else {
                self.results.remove(0)
            }
        }
    }

    fn temp_home(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "enable_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn noop_up() -> bool {
        true
    }

    #[test]
    fn unknown_app_writes_nothing_and_does_not_start_the_daemon() {
        let home = temp_home("unknown");
        let stage = home.join("stage");
        let mut runner = Recorder {
            calls: Vec::new(),
            results: Vec::new(),
        };
        let err = enable_app(
            "not-an-app",
            &home,
            &stage,
            "/opt/agent-hook",
            &mut runner,
            || panic!("probe"),
            || panic!("start"),
        )
        .unwrap_err();
        assert!(err.contains("unknown application"));
        assert!(!stage.exists());
        assert!(runner.calls.is_empty());
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn daemon_down_starts_once_and_up_does_not_start() {
        let home = temp_home("daemon");
        let stage = home.join("stage");
        let mut starts = 0;
        enable_app(
            "grok",
            &home,
            &stage,
            "/opt/agent-hook",
            &mut Recorder {
                calls: Vec::new(),
                results: Vec::new(),
            },
            || false,
            || {
                starts += 1;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(starts, 1);
        assert!(grok_config(&home).is_file());

        let mut starts_when_up = 0;
        enable_app(
            "pi",
            &home,
            &stage,
            "/opt/agent-hook",
            &mut Recorder {
                calls: Vec::new(),
                results: Vec::new(),
            },
            noop_up,
            || {
                starts_when_up += 1;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(starts_when_up, 0);
        assert!(!home.join(".pi").join("hooks.json").exists());
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn daemon_start_failure_does_not_write_config() {
        let home = temp_home("daemon-fail");
        let err = enable_app(
            "grok",
            &home,
            &home.join("stage"),
            "/opt/agent-hook",
            &mut Recorder {
                calls: Vec::new(),
                results: Vec::new(),
            },
            || false,
            || Err("daemon did not start".to_string()),
        )
        .unwrap_err();
        assert!(err.contains("daemon did not start"));
        assert!(!grok_config(&home).exists());
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn claude_enable_stages_then_refuses_to_touch_home_when_cli_is_missing() {
        let home = temp_home("claude-missing");
        fs::create_dir_all(home.join(".claude")).unwrap();
        fs::write(home.join(".claude").join("settings.json"), "keep").unwrap();
        let stage = home.join("stage");
        let err = enable_app(
            "claude",
            &home,
            &stage,
            "/opt/agent-hook",
            &mut Recorder {
                calls: Vec::new(),
                results: vec![Err("cannot spawn claude: no such file".to_string())],
            },
            noop_up,
            || panic!("start"),
        )
        .unwrap_err();
        assert!(err.contains("cannot spawn claude"));
        assert_eq!(
            fs::read_to_string(home.join(".claude").join("settings.json")).unwrap(),
            "keep"
        );
        assert!(stage
            .join("claude")
            .join(".claude-plugin")
            .join("marketplace.json")
            .is_file());
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn vendor_enable_and_disable_use_recorded_commands() {
        let home = temp_home("vendors");
        let stage = home.join("stage");
        let binary = "/opt/agent-otel-bridge/bin/agent-hook";
        fs::write(home.join("legacy-grok"), "untouched").unwrap();
        fs::create_dir_all(home.join(".grok")).unwrap();
        fs::write(
            home.join(".grok").join("hooks.json"),
            "legacy-owned-elsewhere",
        )
        .unwrap();

        let mut claude = Recorder {
            calls: Vec::new(),
            results: Vec::new(),
        };
        enable_app(
            "claude",
            &home,
            &stage,
            binary,
            &mut claude,
            noop_up,
            || panic!("start"),
        )
        .unwrap();
        let claude_root = stage.join("claude");
        assert_eq!(
            claude.calls,
            vec![
                (
                    "claude".to_string(),
                    vec![
                        "plugin".into(),
                        "marketplace".into(),
                        "add".into(),
                        claude_root.to_str().unwrap().to_string(),
                        "--scope".into(),
                        "user".into(),
                        "--json".into(),
                    ]
                ),
                (
                    "claude".to_string(),
                    vec![
                        "plugin".into(),
                        "install".into(),
                        "agent-otel-bridge@agent-otel-bridge".into(),
                        "--scope".into(),
                        "user".into(),
                        "-y".into(),
                        "--json".into(),
                    ]
                )
            ]
        );
        let hooks: Value = serde_json::from_slice(
            &fs::read(claude_root.join("plugins/agent-otel-bridge/hooks/hooks.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            hooks["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            format_claude_hook_command(binary, "PreToolUse", None)
        );
        assert!(hooks["hooks"].get("Stop").is_some());
        assert!(!home.join(".claude").exists());

        let mut codex = Recorder {
            calls: Vec::new(),
            results: Vec::new(),
        };
        enable_app("codex", &home, &stage, binary, &mut codex, noop_up, || {
            panic!("start")
        })
        .unwrap();
        let codex_root = stage.join("codex");
        assert!(codex_root
            .join(".agents/plugins/marketplace.json")
            .is_file());
        assert_eq!(
            codex.calls[0].1,
            vec![
                "plugin".to_string(),
                "marketplace".into(),
                "add".into(),
                codex_root.to_str().unwrap().to_string(),
            ]
        );
        assert_eq!(
            codex.calls[1].1,
            vec![
                "plugin".to_string(),
                "add".into(),
                "agent-otel-bridge@agent-otel-bridge".into(),
                "--json".into(),
            ]
        );
        let codex_hooks: Value = serde_json::from_slice(
            &fs::read(codex_root.join("plugins/agent-otel-bridge/hooks/hooks.json")).unwrap(),
        )
        .unwrap();
        let expected = CodexHookShell::Portable
            .render(binary, "PostToolUse", Some("codex"))
            .unwrap();
        assert_eq!(
            codex_hooks["hooks"]["PostToolUse"][0]["hooks"][0]["command"],
            expected
        );
        assert!(!home.join(".codex").exists());

        let mut agy = Recorder {
            calls: Vec::new(),
            results: Vec::new(),
        };
        enable_app(
            "antigravity",
            &home,
            &stage,
            binary,
            &mut agy,
            noop_up,
            || panic!("start"),
        )
        .unwrap();
        let agy_root = stage.join("antigravity");
        assert_eq!(
            agy.calls[0].1,
            vec![
                "plugin".to_string(),
                "install".into(),
                agy_root.to_str().unwrap().to_string(),
            ]
        );
        let agy_hooks: Value =
            serde_json::from_slice(&fs::read(agy_root.join("hooks.json")).unwrap()).unwrap();
        assert_eq!(
            agy_hooks["agent-otel-bridge"]["PreToolUse"][0]["hooks"][0]["command"],
            format_antigravity_hook_command(binary, "PreToolUse", None)
        );
        assert!(!home.join(".gemini").exists());

        enable_app("grok", &home, &stage, binary, &mut agy, noop_up, || {
            panic!("start")
        })
        .unwrap();
        assert_eq!(
            fs::read_to_string(home.join(".grok/hooks.json")).unwrap(),
            "legacy-owned-elsewhere"
        );
        assert!(fs::read_to_string(grok_config(&home))
            .unwrap()
            .contains("agent-hook"));
        enable_app("pi", &home, &stage, binary, &mut agy, noop_up, || {
            panic!("start")
        })
        .unwrap();
        let extension = home.join(".pi/agent/extensions/agent-otel-bridge.ts");
        assert!(fs::read_to_string(&extension)
            .unwrap()
            .starts_with("// Managed by agent-otel-bridge."));
        assert!(!home.join(".pi/hooks.json").exists());

        fs::write(&extension, "// user owned\n").unwrap();
        assert!(enable_app("pi", &home, &stage, binary, &mut agy, noop_up, || Ok(())).is_err());
        assert_eq!(fs::read_to_string(&extension).unwrap(), "// user owned\n");
        fs::write(&extension, "// Managed by agent-otel-bridge.\n").unwrap();

        let mut disable = Recorder {
            calls: Vec::new(),
            results: Vec::new(),
        };
        disable_app("claude", &home, &mut disable).unwrap();
        disable_app("codex", &home, &mut disable).unwrap();
        disable_app("antigravity", &home, &mut disable).unwrap();
        assert_eq!(
            disable.calls,
            vec![
                (
                    "claude".to_string(),
                    vec![
                        "plugin".into(),
                        "uninstall".into(),
                        "agent-otel-bridge@agent-otel-bridge".into(),
                        "--scope".into(),
                        "user".into(),
                        "--json".into(),
                    ]
                ),
                (
                    "codex".to_string(),
                    vec![
                        "plugin".into(),
                        "remove".into(),
                        "agent-otel-bridge@agent-otel-bridge".into(),
                        "--json".into(),
                    ]
                ),
                (
                    "agy".to_string(),
                    vec![
                        "plugin".into(),
                        "uninstall".into(),
                        "agent-otel-bridge".into(),
                    ]
                ),
            ]
        );
        disable_app("grok", &home, &mut disable).unwrap();
        disable_app("pi", &home, &mut disable).unwrap();
        assert!(!extension.exists());
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn missing_agy_does_not_write_gemini() {
        let home = temp_home("agy-missing");
        fs::create_dir_all(home.join(".gemini")).unwrap();
        fs::write(home.join(".gemini").join("marker"), "stay").unwrap();
        let err = enable_app(
            "antigravity",
            &home,
            &home.join("stage"),
            "/opt/agent-hook",
            &mut Recorder {
                calls: Vec::new(),
                results: vec![Err("cannot spawn agy: no such file".to_string())],
            },
            noop_up,
            || panic!("start"),
        )
        .unwrap_err();
        assert!(err.contains("cannot spawn agy"));
        assert_eq!(
            fs::read_to_string(home.join(".gemini/marker")).unwrap(),
            "stay"
        );
        assert!(!home.join(".gemini/config").exists());
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn production_runner_reports_a_missing_program() {
        let err = run_command("agent-otel-bridge-missing-cli", &[]).unwrap_err();
        assert!(err.contains("cannot spawn agent-otel-bridge-missing-cli"));
    }
}
