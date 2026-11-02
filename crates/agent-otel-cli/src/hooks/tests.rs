/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use super::codex::format_codex_windows_command;
use super::*;
use serde_json::json;

#[test]
fn codex_policy_is_explicit_and_strict() {
    let dir = std::env::temp_dir().join(format!("codex_policy_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let policy = dir.join("agent-otel-bridge-policy.json");
    let hooks = dir.join("hooks.json");
    assert_eq!(
        load_codex_hook_shell(&policy).unwrap(),
        CodexHookShell::Portable
    );
    let third = json!({"type":"command","command":"herdr status","timeout":17});
    fs::write(
        &hooks,
        serde_json::to_vec(&json!({"hooks":{"PreToolUse":[{"hooks":[third]}]}})).unwrap(),
    )
    .unwrap();
    let selected = r#"{"version":1,"windows_hook_shell":"powershell"}"#;
    fs::write(&policy, selected).unwrap();
    install_codex_hooks(&hooks, "C:/space $&`/agent-hook.exe", Some("codex")).unwrap();
    let first = fs::read(&hooks).unwrap();
    install_codex_hooks(&hooks, "C:/space $&`/agent-hook.exe", Some("codex")).unwrap();
    assert_eq!(first, fs::read(&hooks).unwrap());
    assert_eq!(fs::read_to_string(&policy).unwrap(), selected);
    let value: Value = serde_json::from_slice(&first).unwrap();
    assert_eq!(value["hooks"]["PreToolUse"][0]["hooks"][0], third);
    let stop = value["hooks"]["Stop"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert_eq!(stop.starts_with("& \""), cfg!(windows));
    assert!(!stop.contains("EncodedCommand"));
    for invalid in [
        "{",
        r#"{"version":2,"windows_hook_shell":"powershell"}"#,
        r#"{"version":1,"windows_hook_shell":"cmd"}"#,
        r#"{"version":1,"windows_hook_shell":"powershell","typo":true}"#,
        r#"{"version":1}"#,
    ] {
        fs::write(&policy, invalid).unwrap();
        assert!(load_codex_hook_shell(&policy).is_err());
        if cfg!(windows) {
            assert!(install_codex_hooks(&hooks, "C:/agent-hook.exe", Some("codex")).is_err());
            assert_eq!(fs::read(&hooks).unwrap(), first);
        }
    }
    fs::remove_file(&policy).unwrap();
    fs::create_dir(&policy).unwrap();
    assert!(load_codex_hook_shell(&policy).is_err());
    fs::remove_dir(policy).unwrap();
    fs::remove_file(hooks).unwrap();
    fs::remove_dir(dir).unwrap();
}

#[test]
fn codex_launcher_rejects_unsafe_system_root() {
    for root in [
        "",
        "Windows",
        "C:/Program Files/Windows",
        "C:/Win&dows",
        "C:/$env",
        "C:/Win%TEMP%",
    ] {
        assert!(
            format_codex_windows_command(root, "agent-hook.exe", "Stop", Some("codex")).is_err()
        );
    }
    let command = format_codex_windows_command(
        "C:/Windows",
        "C:/a $&`/agent-hook.exe",
        "Stop",
        Some("codex"),
    )
    .unwrap();
    assert!(command.starts_with("C:\\Windows\\System32\\"));
    assert!(!command.contains('"'));
    assert!(is_bridge_command(&command));
}

#[cfg(windows)]
#[test]
fn codex_updates_owned_override_and_preserves_third_party() {
    let dir = std::env::temp_dir().join(format!("codex_renderer_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("hooks.json");
    let third = json!({"type":"command", "command":"herdr status", "commandWindows":"herdr.exe status", "timeout":17});
    let initial = json!({"custom":true, "hooks":{"PreToolUse":[{"matcher":".*", "hooks":[third, {"type":"command", "command":"agent-hook.exe PreToolUse", "commandWindows":"agent-hook.exe PreToolUse", "timeout":5}]}]}});
    fs::write(&path, serde_json::to_vec(&initial).unwrap()).unwrap();
    install_codex_hooks(&path, "C:/a $&`/agent-hook.exe", Some("codex")).unwrap();
    let first = fs::read(&path).unwrap();
    install_codex_hooks(&path, "C:/a $&`/agent-hook.exe", Some("codex")).unwrap();
    assert_eq!(first, fs::read(&path).unwrap());
    let value: Value = serde_json::from_slice(&first).unwrap();
    assert_eq!(value["hooks"]["PreToolUse"][0]["hooks"][0], third);
    let owned = &value["hooks"]["PreToolUse"][0]["hooks"][1];
    assert_eq!(owned["command"], owned["commandWindows"]);
    assert_eq!(value["custom"], true);
    let adapter = CLIENT_ADAPTERS.iter().find(|a| a.id == "codex").unwrap();
    assert!((adapter.is_registered_fn)(&path));
    let mut conflict = value;
    conflict["hooks"]["PreToolUse"][0]["hooks"][1]["commandWindows"] = json!("custom.exe");
    fs::write(&path, serde_json::to_vec(&conflict).unwrap()).unwrap();
    let before = fs::read(&path).unwrap();
    assert!(install_codex_hooks(&path, "C:/agent-hook.exe", Some("codex")).is_err());
    assert_eq!(before, fs::read(&path).unwrap());
    for invalid in [
        "{invalid",
        "[]",
        r#"{"hooks":{"PreToolUse":{}}}"#,
        r#"{"hooks":{"Stop":[{"hooks":[{"command":"herdr", "commandWindows":"agent-hook.exe Stop"}]}]}}"#,
    ] {
        fs::write(&path, invalid).unwrap();
        assert!(install_codex_hooks(&path, "C:/agent-hook.exe", Some("codex")).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), invalid);
    }
    fs::remove_file(path).unwrap();
    fs::remove_dir(dir).unwrap();
}

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
    if cfg!(windows) {
        assert!(command.starts_with("& \""));
    } else {
        assert!(command.starts_with("\""));
    }
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
        assert_eq!(
            command,
            format_antigravity_hook_command("C:\\Bridge\\agent-hook.exe", "PreToolUse", None)
        );
        assert!(!command.contains('"'));
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
        format_claude_hook_command("C:\\Bridge\\agent-hook.exe", "PreToolUse", None)
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
        format_claude_hook_command("C:\\NewPath\\agent-hook.exe", "PreToolUse", None)
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

fn command_strings(value: &Value) -> Vec<&str> {
    let mut out = Vec::new();
    fn walk<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
        match value {
            Value::Array(items) => {
                for item in items {
                    walk(item, out);
                }
            }
            Value::Object(map) => {
                if let Some(command) = map.get("command").and_then(Value::as_str) {
                    out.push(command);
                }
                for (key, child) in map {
                    if key != "command" {
                        walk(child, out);
                    }
                }
            }
            _ => {}
        }
    }
    walk(value, &mut out);
    out
}

fn assert_bridge_commands(path: &std::path::Path, expected: [String; 3]) {
    let value: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(value["custom"], true);
    let commands = command_strings(&value);
    assert!(commands.contains(&"herdr status"));
    assert_eq!(
        commands
            .iter()
            .filter(|command| is_bridge_command(command))
            .count(),
        expected.len()
    );
    for command in &expected {
        assert!(
            commands.iter().any(|found| *found == command),
            "missing bridge command {command}"
        );
        assert!(is_bridge_command(command));
    }
}

fn assert_only_third_party(path: &std::path::Path) {
    let value: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(value["custom"], true);
    assert_eq!(command_strings(&value), vec!["herdr status"]);
    assert!(path.exists());
}

#[test]
fn test_codex_grok_pi_hooks_install_and_uninstall() {
    let temp_dir = std::env::temp_dir().join(format!("other_agents_test_{}", std::process::id()));
    let codex_path = get_codex_project_path(Some(&temp_dir)).unwrap();
    let grok_path = get_grok_project_path(Some(&temp_dir)).unwrap();
    let pi_path = get_pi_project_path(Some(&temp_dir)).unwrap();

    let third_party = json!({
        "custom": true,
        "hooks": {
            "PreToolUse": [{
                "matcher": ".*",
                "hooks": [{ "type": "command", "command": "herdr status", "timeout": 17 }]
            }]
        }
    });
    for path in [&codex_path, &grok_path, &pi_path] {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_vec(&third_party).unwrap()).unwrap();
    }
    let extension = super::pi::pi_extension_path(&pi_path).unwrap();
    let third_party_ext = extension.with_file_name("third-party.ts");
    fs::create_dir_all(third_party_ext.parent().unwrap()).unwrap();
    fs::write(&third_party_ext, "// retain me").unwrap();

    install_codex_hooks(&codex_path, "agent-hook", Some("codex")).unwrap();
    install_grok_hooks(&grok_path, "agent-hook", Some("grok")).unwrap();
    install_pi_hooks(&pi_path, "agent-hook", Some("pi")).unwrap();

    assert_bridge_commands(
        &codex_path,
        ["PreToolUse", "PostToolUse", "Stop"].map(|event| {
            CodexHookShell::Portable
                .render("agent-hook", event, Some("codex"))
                .unwrap()
        }),
    );
    assert_bridge_commands(
        &grok_path,
        ["PreToolUse", "PostToolUse", "Stop"]
            .map(|event| format_grok_hook_command("agent-hook", event, Some("grok"))),
    );
    assert_bridge_commands(
        &pi_path,
        ["PreToolUse", "PostToolUse", "Stop"]
            .map(|event| format_hook_command("agent-hook", event, Some("pi"))),
    );
    let extension_text = fs::read_to_string(&extension).unwrap();
    assert!(extension_text.starts_with("// Managed by agent-otel-bridge."));
    assert!(extension_text.contains("\"agent-hook\""));
    assert_eq!(
        fs::read_to_string(&third_party_ext).unwrap(),
        "// retain me"
    );

    assert!(uninstall_standard_hooks(&codex_path, "agent-hook").unwrap());
    assert!(uninstall_standard_hooks(&grok_path, "agent-hook").unwrap());
    assert!(uninstall_pi_hooks(&pi_path, "agent-hook").unwrap());

    for path in [&codex_path, &grok_path, &pi_path] {
        assert_only_third_party(path);
    }
    assert!(!extension.exists());
    assert_eq!(
        fs::read_to_string(&third_party_ext).unwrap(),
        "// retain me"
    );

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
        format_claude_hook_command("C:\\Tools\\agent-hook.exe", "PreToolUse", None)
    );

    // Running sync a second time is idempotent
    let sync_res2 = run_sync(Some(&temp_dir), Some("C:\\Tools\\agent-hook.exe"));
    assert!(sync_res2.is_ok());

    let _ = fs::remove_dir_all(&temp_dir);
}
