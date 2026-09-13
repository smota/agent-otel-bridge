/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_bridge::hooks::{
    format_hook_command, install_antigravity_hooks, install_claude_hooks,
    uninstall_antigravity_hooks, uninstall_claude_hooks,
};
use agent_otel_bridge::local::{
    compute_sha256, safe_copy_or_replace, BinaryArtifactInfo, LocalVersionManifest,
};
use serde_json::{json, Value};
use std::fs;

#[test]
fn test_safe_copy_or_replace_and_sha256() {
    let temp_dir = std::env::temp_dir().join(format!("safe_copy_test_{}", std::process::id()));
    fs::create_dir_all(&temp_dir).unwrap();

    let src = temp_dir.join("source.bin");
    let dst = temp_dir.join("destination.bin");

    fs::write(&src, b"hello open telemetry bridge 12345").unwrap();

    let sha_src = compute_sha256(&src).unwrap();
    assert!(!sha_src.is_empty());

    // Initial copy
    safe_copy_or_replace(&src, &dst).unwrap();
    assert!(dst.is_file());

    let sha_dst = compute_sha256(&dst).unwrap();
    assert_eq!(sha_src, sha_dst);

    // Replace with new content
    fs::write(&src, b"updated content for atomic replacement").unwrap();
    let new_sha = compute_sha256(&src).unwrap();
    assert_ne!(sha_src, new_sha);

    safe_copy_or_replace(&src, &dst).unwrap();
    let replaced_sha = compute_sha256(&dst).unwrap();
    assert_eq!(new_sha, replaced_sha);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_manifest_serialization_and_deserialization() {
    let manifest = LocalVersionManifest {
        version: "0.4.0".to_string(),
        version_id: "0.4.0-abcdef1-dirty".to_string(),
        git_commit: "abcdef1234567890".to_string(),
        git_branch: "feat/test".to_string(),
        is_dirty: true,
        built_at: "1726000000".to_string(),
        target_triple: "x86_64-pc-windows-msvc".to_string(),
        binaries: vec![
            BinaryArtifactInfo {
                filename: "agent-hook.exe".to_string(),
                sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                    .to_string(),
                size_bytes: 204800,
            },
            BinaryArtifactInfo {
                filename: "agent-otel-bridge.exe".to_string(),
                sha256: "ca978112ca1bbdccafac231b39a23dc4da786eff8147c4e72b9807785afee48b"
                    .to_string(),
                size_bytes: 4096000,
            },
        ],
    };

    let serialized = serde_json::to_string_pretty(&manifest).unwrap();
    let deserialized: LocalVersionManifest = serde_json::from_str(&serialized).unwrap();

    assert_eq!(deserialized.version, "0.4.0");
    assert_eq!(deserialized.version_id, "0.4.0-abcdef1-dirty");
    assert!(deserialized.is_dirty);
    assert_eq!(deserialized.binaries.len(), 2);
    assert_eq!(deserialized.binaries[0].filename, "agent-hook.exe");
}

#[test]
fn test_hooks_command_path_with_spaces_and_quotes() {
    let path_with_space = "C:\\Program Files\\AI Agent Bridge\\bin\\agent-hook.exe";
    let cmd = format_hook_command(path_with_space, "PreToolUse", Some("claude"));
    assert_eq!(
        cmd,
        "\"C:\\Program Files\\AI Agent Bridge\\bin\\agent-hook.exe\" PreToolUse --client claude"
    );

    // Already quoted path should not double quote
    let already_quoted = "\"C:\\Program Files\\AI Agent Bridge\\bin\\agent-hook.exe\"";
    let cmd2 = format_hook_command(already_quoted, "Stop", None);
    assert_eq!(
        cmd2,
        "\"C:\\Program Files\\AI Agent Bridge\\bin\\agent-hook.exe\" Stop"
    );
}

#[test]
fn test_antigravity_hooks_full_lifecycle_preservation() {
    let temp_dir = std::env::temp_dir().join(format!("agy_lifecycle_{}", std::process::id()));
    let hooks_json = temp_dir.join("hooks.json");
    fs::create_dir_all(&temp_dir).unwrap();

    // 1. Third-party hooks exist
    let third_party = json!({
        "custom-security-gate": {
            "PreToolUse": [{ "command": "c:\\sec\\gate.exe", "timeout": 3 }]
        },
        "herdr-sync": {
            "Stop": [{ "command": "powershell herdr.ps1", "timeout": 10 }]
        }
    });
    fs::write(&hooks_json, serde_json::to_string(&third_party).unwrap()).unwrap();

    // 2. Install bridge hooks
    let bin_path = "C:\\Users\\samue\\AppData\\Local\\agent-otel-bridge\\bin\\agent-hook.exe";
    install_antigravity_hooks(&hooks_json, bin_path, None).unwrap();

    let content: Value = serde_json::from_str(&fs::read_to_string(&hooks_json).unwrap()).unwrap();
    assert!(content.get("custom-security-gate").is_some());
    assert!(content.get("herdr-sync").is_some());
    assert!(content.get("agent-otel-bridge").is_some());

    let bridge = &content["agent-otel-bridge"];
    assert_eq!(
        bridge["PreToolUse"][0]["hooks"][0]["command"],
        format!("{bin_path} PreToolUse")
    );

    // 3. Reinstall with updated path (e.g. new version)
    let new_bin_path = "C:\\Users\\samue\\AppData\\Local\\agent-otel-bridge\\bin\\agent-hook.exe";
    install_antigravity_hooks(&hooks_json, new_bin_path, None).unwrap();

    let content2: Value = serde_json::from_str(&fs::read_to_string(&hooks_json).unwrap()).unwrap();
    assert!(content2.get("custom-security-gate").is_some());
    assert!(content2.get("herdr-sync").is_some());
    assert!(content2.get("agent-otel-bridge").is_some());

    // 4. Uninstall
    uninstall_antigravity_hooks(&hooks_json).unwrap();
    let content3: Value = serde_json::from_str(&fs::read_to_string(&hooks_json).unwrap()).unwrap();
    assert!(content3.get("agent-otel-bridge").is_none());
    assert!(content3.get("custom-security-gate").is_some());
    assert!(content3.get("herdr-sync").is_some());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_claude_hooks_upgrade_from_relative_to_absolute_without_duplication() {
    let temp_dir = std::env::temp_dir().join(format!("claude_upgrade_{}", std::process::id()));
    let settings_json = temp_dir.join("settings.json");
    fs::create_dir_all(&temp_dir).unwrap();

    // Old configuration with relative command and third party
    let initial = json!({
        "hooks": {
            "PreToolUse": [
                {
                    "matcher": "Bash",
                    "hooks": [{ "command": "rtk hook claude", "type": "command" }]
                },
                {
                    "matcher": ".*",
                    "hooks": [{ "command": "agent-hook PreToolUse --client claude", "type": "command" }]
                }
            ],
            "PostToolUse": [
                {
                    "matcher": ".*",
                    "hooks": [{ "command": "agent-hook PostToolUse --client claude", "type": "command" }]
                }
            ]
        }
    });
    fs::write(&settings_json, serde_json::to_string(&initial).unwrap()).unwrap();

    // Upgrade to canonical absolute path
    let abs_bin = "C:\\Users\\samue\\AppData\\Local\\agent-otel-bridge\\bin\\agent-hook.exe";
    install_claude_hooks(&settings_json, abs_bin).unwrap();

    let updated: Value =
        serde_json::from_str(&fs::read_to_string(&settings_json).unwrap()).unwrap();
    let pre_tool = updated["hooks"]["PreToolUse"].as_array().unwrap();
    // Must remain exactly 2 items: rtk + bridge (not 3!)
    assert_eq!(pre_tool.len(), 2);
    assert_eq!(pre_tool[0]["hooks"][0]["command"], "rtk hook claude");
    assert_eq!(
        pre_tool[1]["hooks"][0]["command"],
        format!("{abs_bin} PreToolUse")
    );

    // Uninstall
    uninstall_claude_hooks(&settings_json, abs_bin).unwrap();
    let uninstalled: Value =
        serde_json::from_str(&fs::read_to_string(&settings_json).unwrap()).unwrap();
    let pre_tool_after = uninstalled["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(pre_tool_after.len(), 1);
    assert_eq!(pre_tool_after[0]["hooks"][0]["command"], "rtk hook claude");

    let _ = fs::remove_dir_all(&temp_dir);
}
