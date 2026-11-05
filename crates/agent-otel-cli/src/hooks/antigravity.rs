/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use super::command::format_antigravity_hook_command;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

pub fn antigravity_bridge_hooks(binary: &str, client_tag: Option<&str>) -> Value {
    antigravity_bridge_hooks_with_formatter(binary, client_tag, format_antigravity_hook_command)
}

pub(crate) fn antigravity_bridge_hooks_with_formatter(
    binary: &str,
    client_tag: Option<&str>,
    formatter: impl Fn(&str, &str, Option<&str>) -> String,
) -> Value {
    let pre_tool = formatter(binary, "PreToolUse", client_tag);
    let post_tool = formatter(binary, "PostToolUse", client_tag);
    let pre_inv = formatter(binary, "PreInvocation", client_tag);
    let post_inv = formatter(binary, "PostInvocation", client_tag);
    let stop = formatter(binary, "Stop", client_tag);
    json!({
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
    })
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
        serde_json::from_str(&content)?
    } else {
        json!({})
    };

    if !root.is_object() {
        return Err("Invalid Antigravity hook configuration; configuration unchanged".into());
    }

    // Update only the agent-otel-bridge section, preserving third-party keys intact.
    root["agent-otel-bridge"] = antigravity_bridge_hooks(binary, client_tag);

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
