/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use super::command::{format_hook_command, is_bridge_command};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

#[allow(dead_code)] // Retained public compatibility entry point.
pub fn install_standard_hooks(
    path: &Path,
    binary: &str,
    client_tag: Option<&str>,
) -> Result<bool, Box<dyn std::error::Error>> {
    install_hooks_with_command_formatter(path, binary, client_tag, format_hook_command)
}

pub(crate) fn install_hooks_with_command_formatter(
    path: &Path,
    binary: &str,
    client_tag: Option<&str>,
    formatter: fn(&str, &str, Option<&str>) -> String,
) -> Result<bool, Box<dyn std::error::Error>> {
    install_formatted_hooks(path, binary, client_tag, formatter, false)
}

pub(crate) fn install_formatted_hooks(
    path: &Path,
    binary: &str,
    client_tag: Option<&str>,
    formatter: impl Fn(&str, &str, Option<&str>) -> String,
    codex_windows: bool,
) -> Result<bool, Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut root: Value = if path.exists() {
        serde_json::from_str(&fs::read_to_string(path)?)?
    } else {
        json!({})
    };
    merge_formatted_hooks(&mut root, binary, client_tag, formatter, codex_windows)?;
    let serialized = serde_json::to_string_pretty(&root)?;
    fs::write(path, serialized)?;
    Ok(true)
}

pub(crate) fn merge_formatted_hooks(
    root: &mut Value,
    binary: &str,
    client_tag: Option<&str>,
    formatter: impl Fn(&str, &str, Option<&str>) -> String,
    codex_windows: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if !root.is_object() || root.get("hooks").is_some_and(|v| !v.is_object()) {
        return Err("Invalid hook configuration; configuration unchanged".into());
    }
    for event in ["PreToolUse", "PostToolUse", "Stop"] {
        if let Some(groups) = root.get("hooks").and_then(|hooks| hooks.get(event)) {
            let groups = groups.as_array().ok_or("Invalid hook event array")?;
            for group in groups {
                let hooks = group
                    .get("hooks")
                    .and_then(Value::as_array)
                    .ok_or("Invalid hook group")?;
                if hooks.iter().any(|hook| !hook.is_object()) {
                    return Err("Invalid hook handler".into());
                }
            }
        }
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

        let mut entry = json!({
            "matcher": ".*",
            "hooks": [
                {
                    "type": "command",
                    "command": target_cmd,
                    "timeout": 5
                }
            ]
        });

        if event == "Stop" {
            entry.as_object_mut().unwrap().remove("matcher");
        }
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
                            if codex_windows {
                                if let Some(command) = h.get("commandWindows") {
                                    if !command.as_str().is_some_and(is_bridge_command) {
                                        return Err("Codex bridge hook has an unowned commandWindows override; configuration unchanged".into());
                                    }
                                    // Codex prioritizes this field. Do not leave
                                    // an old bridge command overriding the fix.
                                    h["commandWindows"] = json!(&target_cmd);
                                }
                            }
                            h["command"] = json!(&target_cmd);
                            updated = true;
                        }
                    }
                }
            }

            if !updated {
                arr.push(entry);
            }
            if event == "Stop" {
                for group in arr.iter_mut() {
                    let owned = group
                        .get("hooks")
                        .and_then(Value::as_array)
                        .is_some_and(|hooks| {
                            !hooks.is_empty()
                                && hooks.iter().all(|hook| {
                                    hook.get("command")
                                        .and_then(Value::as_str)
                                        .is_some_and(is_bridge_command)
                                })
                        });
                    if owned {
                        if let Some(object) = group.as_object_mut() {
                            object.remove("matcher");
                        }
                    }
                }
            }
        } else {
            hooks_obj.insert(event.to_string(), json!([entry]));
        }
    }

    Ok(())
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
                for group in arr.iter_mut() {
                    if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                        let before = handlers.len();
                        handlers.retain(|hook| {
                            !hook
                                .get("command")
                                .and_then(Value::as_str)
                                .is_some_and(is_bridge_command)
                        });
                        changed |= before != handlers.len();
                    }
                }
                arr.retain(|group| {
                    !group
                        .get("hooks")
                        .and_then(Value::as_array)
                        .is_some_and(Vec::is_empty)
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

#[cfg(test)]
mod preservation_tests {
    use super::*;
    #[test]
    fn invalid_configuration_is_preserved() {
        let path =
            std::env::temp_dir().join(format!("bridge-malformed-{}.json", std::process::id()));
        fs::write(&path, "malformed user configuration").unwrap();
        let result = install_standard_hooks(&path, "/opt/agent-hook", Some("claude"));
        let content = fs::read_to_string(&path).unwrap();
        let _ = fs::remove_file(path);
        assert!(result.is_err());
        assert_eq!(content, "malformed user configuration");
    }
}
