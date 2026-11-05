/*
 * Copyright agent-otel-bridge contributors
 * SPDX-License-Identifier: Apache-2.0
 */

//! Read-only Linux wiring plans. Applying a plan belongs to the caller's
//! serialized, journaled transaction; planning never registers a plugin or
//! starts a daemon. The explicit HOME also covers isolated MTN seat homes.
use super::antigravity::antigravity_bridge_hooks_with_formatter;
use super::command::{format_posix_claude_hook_command, format_posix_hook_command};
use super::pi::render_pi_extension;
use super::standard::merge_formatted_hooks;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeHookChange {
    pub path: PathBuf,
    /// Exact rollback/concurrency evidence; None means the file was absent.
    pub before: Option<Vec<u8>>,
    pub after: Vec<u8>,
}

/// Plan native hook files for the selected harnesses without touching HOME.
/// `socket` is persisted literally, including for homes launched with clearenv.
/// Pass AGENT_OTEL_SOCKET here explicitly when selecting it from the environment.
/// Harness trust/approval is still required by its native policy on first load.
/// Antigravity and Gemini CLI have different readers; select `gemini-cli` for
/// Google's Gemini CLI settings schema, and `agy` for native Antigravity.
pub fn plan_home_hooks(
    home: &Path,
    clients: &[&str],
    binary: &Path,
    socket: Option<&Path>,
) -> Result<Vec<HomeHookChange>, Box<dyn std::error::Error>> {
    if !home.is_absolute() || !binary.is_absolute() || socket.is_some_and(|p| !p.is_absolute()) {
        return Err("HOME, hook executable and socket must be absolute paths".into());
    }
    let binary = binary.to_str().ok_or("Hook executable must be UTF-8")?;
    let socket = socket
        .map(|p| p.to_str().ok_or("Socket must be UTF-8"))
        .transpose()?;
    if binary.contains('\0') || socket.is_some_and(|s| s.contains('\0')) {
        return Err("Hook executable and socket cannot contain NUL".into());
    }
    let mut changes: Vec<HomeHookChange> = Vec::new();
    for client in clients {
        let (relative, tag) = match *client {
            "claude" => (".claude/settings.json", "claude"),
            "codex" => (".codex/hooks.json", "codex"),
            "agy" | "antigravity" => (".gemini/config/hooks.json", "agy"),
            "gemini-cli" => (".gemini/settings.json", "agy"),
            "grok" => (".grok/hooks/agent-otel.json", "grok"),
            "pi" => (".pi/agent/extensions/agent-otel-bridge.ts", "pi"),
            _ => return Err(format!("Unsupported native hook reader: {client}").into()),
        };
        let path = home.join(relative);
        if changes.iter().any(|change| change.path == path) {
            continue;
        }
        let before = match fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => return Err(err.into()),
        };
        let after = if tag == "pi" {
            if before
                .as_ref()
                .is_some_and(|bytes| !bytes.starts_with(b"// Managed by agent-otel-bridge."))
            {
                return Err("Refusing to replace an unowned Pi extension".into());
            }
            render_pi_extension(binary, socket)?.into_bytes()
        } else {
            let mut root: Value = match &before {
                Some(bytes) => serde_json::from_slice(bytes)?,
                None => json!({}),
            };
            if !root.is_object() {
                return Err("Invalid native hook configuration; configuration unchanged".into());
            }
            let formatter = |binary: &str, event: &str, _: Option<&str>| {
                if tag == "claude" {
                    format_posix_claude_hook_command(binary, event, Some(tag), socket)
                } else {
                    format_posix_hook_command(binary, event, Some(tag), socket)
                }
            };
            if *client == "agy" || *client == "antigravity" {
                root["agent-otel-bridge"] =
                    antigravity_bridge_hooks_with_formatter(binary, Some(tag), formatter);
            } else if *client == "gemini-cli" {
                merge_gemini_hooks(&mut root, binary, socket)?;
            } else {
                merge_formatted_hooks(&mut root, binary, Some(tag), formatter, false)?;
            }
            let mut bytes = serde_json::to_vec_pretty(&root)?;
            bytes.push(b'\n');
            bytes
        };
        changes.push(HomeHookChange {
            path,
            before,
            after,
        });
    }
    Ok(changes)
}

fn merge_gemini_hooks(
    root: &mut Value,
    binary: &str,
    socket: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    if root.get("hooks").is_some_and(|hooks| !hooks.is_object()) {
        return Err("Invalid Gemini CLI hooks object".into());
    }
    if root.get("hooks").is_none() {
        root["hooks"] = json!({});
    }
    for (source, target) in [
        ("BeforeTool", "PreToolUse"),
        ("AfterTool", "PostToolUse"),
        ("SessionStart", "PreInvocation"),
        ("SessionEnd", "PostInvocation"),
    ] {
        let events = root["hooks"].as_object_mut().unwrap();
        let groups = events
            .entry(source)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or("Invalid Gemini CLI hook array")?;
        let command = format_posix_hook_command(binary, target, Some("agy"), socket);
        let name = format!("agent-otel-bridge-{source}");
        let mut found = false;
        for group in groups.iter_mut() {
            let handlers = group
                .get_mut("hooks")
                .and_then(Value::as_array_mut)
                .ok_or("Invalid Gemini CLI hook group")?;
            for handler in handlers {
                if handler.get("name").and_then(Value::as_str) == Some(&name) {
                    handler["command"] = json!(command);
                    found = true;
                }
            }
        }
        if !found {
            groups.push(json!({"matcher":".*","hooks":[{"name":name,"type":"command","command":command,"timeout":1000}]}));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_home(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "bridge-home-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn plans_are_read_only_preserve_foreign_hooks_and_are_idempotent() {
        let home = fixture_home("preserve");
        let clients = ["claude", "codex", "agy", "gemini-cli", "grok", "pi"];
        let binary = Path::new("/shared/agent bridge/agent-hook");
        let socket = Path::new("/shared/owner's bridge/run/socket");
        let empty = plan_home_hooks(&home, &clients, binary, Some(socket)).unwrap();
        assert_eq!(empty.len(), 6);
        assert_eq!(fs::read_dir(&home).unwrap().count(), 0);
        for change in &empty {
            if change.path.extension().and_then(|s| s.to_str()) == Some("ts") {
                continue;
            }
            fs::create_dir_all(change.path.parent().unwrap()).unwrap();
            let foreign = if change.path.ends_with("config/hooks.json") {
                json!({"Moshi":{"PreInvocation":[{"command":"moshi"}]},"OpenRig":{"PreInvocation":[{"command":"rig"}]}})
            } else {
                json!({"foreignSetting":{"holoself":true,"wmux":true},"hooks":{"PreToolUse":[{"matcher":".*","hooks":[{"type":"command","command":"rtk hook claude"},{"type":"command","command":"moshi"}]}],"BeforeTool":[{"matcher":".*","hooks":[{"type":"command","command":"rig"}]}]}})
            };
            fs::write(&change.path, serde_json::to_vec(&foreign).unwrap()).unwrap();
        }
        let planned = plan_home_hooks(&home, &clients, binary, Some(socket)).unwrap();
        for change in &planned {
            if let Some(before) = &change.before {
                assert_eq!(fs::read(&change.path).unwrap(), *before);
                let before: Value = serde_json::from_slice(before).unwrap();
                let after: Value = serde_json::from_slice(&change.after).unwrap();
                if change.path.ends_with("config/hooks.json") {
                    assert_eq!(before["Moshi"], after["Moshi"]);
                    assert_eq!(before["OpenRig"], after["OpenRig"]);
                } else {
                    assert_eq!(before["foreignSetting"], after["foreignSetting"]);
                    assert_eq!(
                        before["hooks"]["PreToolUse"][0],
                        after["hooks"]["PreToolUse"][0]
                    );
                    assert_eq!(
                        before["hooks"]["BeforeTool"][0],
                        after["hooks"]["BeforeTool"][0]
                    );
                }
            }
            fs::create_dir_all(change.path.parent().unwrap()).unwrap();
            fs::write(&change.path, &change.after).unwrap();
        }
        let again = plan_home_hooks(&home, &clients, binary, Some(socket)).unwrap();
        for (first, second) in planned.iter().zip(&again) {
            assert_eq!(first.after, second.after);
            assert_eq!(second.before.as_ref(), Some(&second.after));
        }
        let pi = String::from_utf8(planned[5].after.clone()).unwrap();
        assert!(pi.contains("env: { ...process.env, ...socketEnv }"));
        assert!(pi.contains(
            "const socketEnv = {\"AGENT_OTEL_SOCKET\":\"/shared/owner's bridge/run/socket\"}"
        ));
        assert!(!pi.contains("__AGENT_OTEL_"));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn malformed_and_unowned_configs_are_refused_without_writes() {
        let home = fixture_home("refuse");
        for (client, relative, original) in [
            ("claude", ".claude/settings.json", "invalid"),
            ("codex", ".codex/hooks.json", "{\"hooks\":[]}"),
            ("agy", ".gemini/config/hooks.json", "[]"),
            (
                "gemini-cli",
                ".gemini/settings.json",
                "{\"hooks\":{\"BeforeTool\":{}}}",
            ),
            ("pi", ".pi/agent/extensions/agent-otel-bridge.ts", "// mine"),
        ] {
            let path = home.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, original).unwrap();
            assert!(plan_home_hooks(&home, &[client], Path::new("/bin/agent-hook"), None).is_err());
            assert_eq!(fs::read_to_string(path).unwrap(), original);
        }
        assert!(plan_home_hooks(&home, &["unknown"], Path::new("/bin/agent-hook"), None).is_err());
        assert!(plan_home_hooks(
            Path::new("relative"),
            &["codex"],
            Path::new("/bin/agent-hook"),
            None
        )
        .is_err());
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn persisted_socket_survives_empty_environment_and_shell_metacharacters() {
        use std::os::unix::fs::PermissionsExt;
        let home = fixture_home("literal ' $ ` quote");
        let binary = home.join("agent-hook");
        fs::write(
            &binary,
            "#!/bin/sh\nprintf '%s\\n' \"$AGENT_OTEL_SOCKET\" \"$1\" \"$2\" \"$3\"\n",
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let socket = home.join("owner's $HOME `false` socket");
        let socket_str = socket.to_str().unwrap();
        let command = format_posix_hook_command(
            binary.to_str().unwrap(),
            "PreToolUse",
            Some("codex"),
            Some(socket_str),
        );
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", &command])
            .env_clear()
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("{socket_str}\nPreToolUse\n--client\ncodex\n")
        );
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    #[ignore = "requires installed native agy and grok; validates temporary config only"]
    fn installed_native_readers_accept_generated_configs() {
        let home = fixture_home("native-readers");
        for change in plan_home_hooks(
            &home,
            &["agy", "grok", "pi"],
            Path::new("/bin/agent-hook"),
            Some(Path::new("/tmp/test.socket")),
        )
        .unwrap()
        {
            fs::create_dir_all(change.path.parent().unwrap()).unwrap();
            fs::write(change.path, change.after).unwrap();
        }
        let plugin = home.join("agy-plugin");
        fs::create_dir_all(&plugin).unwrap();
        fs::copy(
            home.join(".gemini/config/hooks.json"),
            plugin.join("hooks.json"),
        )
        .unwrap();
        fs::write(plugin.join("plugin.json"), serde_json::to_vec(&json!({"name":"agent-otel-bridge", "displayName":"Agent OTel Bridge", "version":"1.0.0", "description":"OpenTelemetry lifecycle hooks"})).unwrap()).unwrap();
        let result = std::process::Command::new("agy")
            .args(["plugin", "validate", plugin.to_str().unwrap(), "--json"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "native AGY validator rejected generated fixture: {}",
            String::from_utf8_lossy(&result.stdout)
        );
        let result = std::process::Command::new("grok")
            .args(["inspect", "--json"])
            .env("HOME", &home)
            .env("GROK_HOME", home.join(".grok"))
            .current_dir(&home)
            .output()
            .unwrap();
        assert!(result.status.success(), "native Grok inspect failed");
        let config: Value = serde_json::from_slice(&result.stdout).unwrap();
        let hooks = config["hooks"].as_array().expect("Grok native hook list");
        assert_eq!(hooks.len(), 3);
        assert!(hooks.iter().all(|hook| hook["target"]
            .as_str()
            .is_some_and(|command| command.contains("--client grok")
                && command.contains("AGENT_OTEL_SOCKET="))));
        // Optional Pi native loader: uses installed extension code, no sessions/models.
        if let Ok(loader) = std::env::var("AGENT_OTEL_PI_LOADER") {
            let result = std::process::Command::new("node").args(["--input-type=module", "-e", "const {loadExtensions}=await import(process.argv[1]); const r=await loadExtensions([process.argv[2]],process.argv[3]); if(r.errors.length||r.extensions.length!==1)process.exit(1); console.log('Pi native extension loaded');", &loader, home.join(".pi/agent/extensions/agent-otel-bridge.ts").to_str().unwrap(), home.to_str().unwrap()]).output().unwrap();
            assert!(
                result.status.success(),
                "Pi native extension loader rejected generated fixture: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        let _ = fs::remove_dir_all(home);
    }
}
