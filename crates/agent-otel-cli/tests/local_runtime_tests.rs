/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_bridge::hooks::{
    format_antigravity_hook_command, format_hook_command, install_antigravity_hooks,
    install_claude_hooks, uninstall_antigravity_hooks, uninstall_claude_hooks,
};
use agent_otel_bridge::local::{
    compute_sha256, safe_copy_or_replace, BinaryArtifactInfo, LocalVersionManifest,
};
use serde_json::{json, Value};
use std::fs;

#[test]
fn pi_extension_install_is_owned_idempotent_and_reversible() {
    let dir = std::env::temp_dir().join(format!("pi_bridge_install_{}", std::process::id()));
    let path = dir.join("hooks.json");
    let extension = dir.join("agent/extensions/agent-otel-bridge.ts");
    fs::create_dir_all(extension.parent().unwrap()).unwrap();
    let third_party = extension.with_file_name("third-party.ts");
    fs::write(&third_party, "// retain me").unwrap();
    let binary = r"C:\Program Files\Bridge\agent-hook.exe";
    agent_otel_bridge::hooks::install_pi_hooks(&path, binary, Some("pi")).unwrap();
    assert!(!path.exists());
    let first = fs::read(&extension).unwrap();
    agent_otel_bridge::hooks::install_pi_hooks(&path, binary, Some("pi")).unwrap();
    assert_eq!(first, fs::read(&extension).unwrap());
    assert!(!path.exists());
    assert!(!String::from_utf8(first)
        .unwrap()
        .contains("__AGENT_OTEL_HOOK_PATH__"));
    agent_otel_bridge::hooks::uninstall_pi_hooks(&path, binary).unwrap();
    assert!(!extension.exists());
    assert_eq!(fs::read_to_string(&third_party).unwrap(), "// retain me");
    fs::write(
        &path,
        r#"{"custom":true,"hooks":{"PreToolUse":[{"hooks":[{"command":"agent-hook PreToolUse"}]},{"hooks":[{"command":"herdr status"}]}]}}"#,
    )
    .unwrap();
    fs::write(&extension, "// Managed by agent-otel-bridge.\n").unwrap();
    assert!(agent_otel_bridge::hooks::uninstall_pi_hooks(&path, binary).unwrap());
    assert!(!extension.exists());
    let legacy: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(legacy["custom"], true);
    assert_eq!(
        legacy["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
        "herdr status"
    );
    fs::write(&extension, "// user owned").unwrap();
    assert!(agent_otel_bridge::hooks::install_pi_hooks(&path, binary, Some("pi")).is_err());
    assert_eq!(fs::read_to_string(&extension).unwrap(), "// user owned");
    fs::remove_dir_all(dir).unwrap();
}

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
    let event = if cfg!(target_os = "linux") {
        assert!(bridge.get("PreToolUse").is_none());
        "PostToolUse"
    } else {
        "PreToolUse"
    };
    assert_eq!(
        bridge[event][0]["hooks"][0]["command"],
        format_antigravity_hook_command(bin_path, event, None)
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
    install_claude_hooks(&settings_json, abs_bin).unwrap();
    let adapter = agent_otel_bridge::hooks::CLIENT_ADAPTERS
        .iter()
        .find(|adapter| adapter.id == "claude")
        .unwrap();
    assert!((adapter.is_registered_fn)(&settings_json));

    let updated: Value =
        serde_json::from_str(&fs::read_to_string(&settings_json).unwrap()).unwrap();
    let pre_tool = updated["hooks"]["PreToolUse"].as_array().unwrap();
    // Must remain exactly 2 items: rtk + bridge (not 3!)
    assert_eq!(pre_tool.len(), 2);
    assert_eq!(pre_tool[0]["hooks"][0]["command"], "rtk hook claude");
    assert_eq!(
        pre_tool[1]["hooks"][0]["command"],
        agent_otel_bridge::hooks::format_claude_hook_command(abs_bin, "PreToolUse", None)
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

fn exe_names() -> (&'static str, &'static str) {
    if cfg!(windows) {
        ("agent-hook.exe", "agent-otel-bridge.exe")
    } else {
        ("agent-hook", "agent-otel-bridge")
    }
}

fn create_test_dir(prefix: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), nanos));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn is_os_error_1314(err: &std::io::Error) -> bool {
    err.raw_os_error() == Some(1314)
}

fn handle_symlink_result(res: std::io::Result<()>, link: &std::path::Path) -> bool {
    match res {
        Ok(()) => true,
        Err(err) if is_os_error_1314(&err) => {
            eprintln!(
                "skipping symlink test step at {}: symlink privilege not held (OS error 1314)",
                link.display()
            );
            false
        }
        Err(err) => panic!("failed to create symlink at {}: {err}", link.display()),
    }
}

fn try_make_symlink_dir(target: &std::path::Path, link: &std::path::Path) -> bool {
    #[cfg(unix)]
    let res = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let res = std::os::windows::fs::symlink_dir(target, link);
    handle_symlink_result(res, link)
}

fn try_make_symlink_file(target: &std::path::Path, link: &std::path::Path) -> bool {
    #[cfg(unix)]
    let res = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let res = std::os::windows::fs::symlink_file(target, link);
    handle_symlink_result(res, link)
}

fn write_verified_build(
    dir: &std::path::Path,
    hook_bytes: &[u8],
    bridge_bytes: &[u8],
) -> (String, String) {
    let (hook_name, bridge_name) = exe_names();
    fs::create_dir_all(dir).unwrap();
    let hook_path = dir.join(hook_name);
    let bridge_path = dir.join(bridge_name);
    fs::write(&hook_path, hook_bytes).unwrap();
    fs::write(&bridge_path, bridge_bytes).unwrap();
    (
        compute_sha256(&hook_path).unwrap(),
        compute_sha256(&bridge_path).unwrap(),
    )
}

fn seed_active_symlink_runtime(
    runtime_home: &std::path::Path,
    version_id: &str,
    hook_bytes: &[u8],
    bridge_bytes: &[u8],
) -> Option<(String, String)> {
    let (hook_name, bridge_name) = exe_names();
    let version_dir = runtime_home.join("versions").join(version_id);
    let (hook_sha, bridge_sha) = write_verified_build(&version_dir, hook_bytes, bridge_bytes);

    let manifest = LocalVersionManifest {
        version: "0.9.0".to_string(),
        version_id: version_id.to_string(),
        git_commit: "1111111".to_string(),
        git_branch: "main".to_string(),
        is_dirty: false,
        built_at: "1700000000".to_string(),
        target_triple: std::env::consts::ARCH.to_string(),
        binaries: vec![
            BinaryArtifactInfo {
                filename: hook_name.to_string(),
                sha256: hook_sha.clone(),
                size_bytes: hook_bytes.len() as u64,
            },
            BinaryArtifactInfo {
                filename: bridge_name.to_string(),
                sha256: bridge_sha.clone(),
                size_bytes: bridge_bytes.len() as u64,
            },
        ],
    };
    let manifest_json = serde_json::to_string_pretty(&manifest).unwrap();
    fs::write(version_dir.join("manifest.json"), &manifest_json).unwrap();
    fs::write(runtime_home.join("active.json"), &manifest_json).unwrap();

    let current_link = runtime_home.join("current");
    if !try_make_symlink_dir(
        &std::path::Path::new("versions").join(version_id),
        &current_link,
    ) {
        return None;
    }

    let bin_dir = runtime_home.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    if !try_make_symlink_file(
        &std::path::Path::new("..").join("current").join(hook_name),
        &bin_dir.join(hook_name),
    ) {
        return None;
    }
    if !try_make_symlink_file(
        &std::path::Path::new("..").join("current").join(bridge_name),
        &bin_dir.join(bridge_name),
    ) {
        return None;
    }

    Some((hook_sha, bridge_sha))
}

fn run_bridge_cli(
    runtime_home: &std::path::Path,
    user_home: &std::path::Path,
    args: &[&str],
) -> std::process::Output {
    run_bridge_cli_with_env(runtime_home, user_home, args, &[])
}

fn run_bridge_cli_with_env(
    runtime_home: &std::path::Path,
    user_home: &std::path::Path,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> std::process::Output {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_agent-otel-bridge"));
    cmd.args(args)
        .env("AGENT_OTEL_HOME", runtime_home)
        .env("LOCALAPPDATA", user_home.join("AppData/Local"))
        .env("USERPROFILE", user_home)
        .env("HOME", user_home)
        .env("XDG_DATA_HOME", user_home.join(".local/share"));
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

#[test]
fn test_local_install_without_activate_stages_only_and_leaves_active_state_untouched() {
    let root = create_test_dir("stage_only_test");
    let runtime_home = root.join("runtime");
    let user_home = root.join("home");
    let candidate_build = root.join("candidate_build");
    fs::create_dir_all(&user_home).unwrap();

    let (hook_name, bridge_name) = exe_names();
    let Some((old_hook_sha, old_bridge_sha)) = seed_active_symlink_runtime(
        &runtime_home,
        "old",
        b"old-immutable-hook-binary",
        b"old-immutable-bridge-binary",
    ) else {
        let _ = fs::remove_dir_all(&root);
        return;
    };
    let (new_hook_sha, new_bridge_sha) = write_verified_build(
        &candidate_build,
        b"new-staged-hook-binary-v2",
        b"new-staged-bridge-binary-v2",
    );

    // Seed a user hook config to verify staging never touches hooks
    let claude_settings = user_home.join(".claude/settings.json");
    fs::create_dir_all(claude_settings.parent().unwrap()).unwrap();
    fs::write(&claude_settings, r#"{"hooks":{}}"#).unwrap();

    let output = run_bridge_cli(
        &runtime_home,
        &user_home,
        &[
            "local",
            "install",
            "--from-build",
            candidate_build.to_str().unwrap(),
        ],
    );
    assert!(
        output.status.success(),
        "staging failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // 1. current pointer must remain untouched (-> versions/old)
    let current_target = fs::read_link(runtime_home.join("current")).unwrap();
    assert_eq!(current_target, std::path::Path::new("versions").join("old"));

    // 2. active.json must remain untouched (old)
    let active_manifest: LocalVersionManifest =
        serde_json::from_str(&fs::read_to_string(runtime_home.join("active.json")).unwrap())
            .unwrap();
    assert_eq!(active_manifest.version_id, "old");

    // 3. previous/ must NOT have been created
    assert!(!runtime_home.join("previous").exists());

    // 4. old version binaries and canonical bin symlinks must still resolve to old hashes
    assert!(
        fs::symlink_metadata(runtime_home.join("bin").join(hook_name))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        fs::symlink_metadata(runtime_home.join("bin").join(bridge_name))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        compute_sha256(&runtime_home.join("versions/old").join(hook_name)).unwrap(),
        old_hook_sha
    );
    assert_eq!(
        compute_sha256(&runtime_home.join("versions/old").join(bridge_name)).unwrap(),
        old_bridge_sha
    );
    assert_eq!(
        compute_sha256(&runtime_home.join("bin").join(hook_name)).unwrap(),
        old_hook_sha
    );
    assert_eq!(
        compute_sha256(&runtime_home.join("bin").join(bridge_name)).unwrap(),
        old_bridge_sha
    );

    // 5. new version directory must be staged in versions/ with the new hashes
    let staged_versions: Vec<_> = fs::read_dir(runtime_home.join("versions"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy() != "old")
        .collect();
    assert_eq!(staged_versions.len(), 1);
    let new_ver_dir = staged_versions[0].path();
    assert_eq!(
        compute_sha256(&new_ver_dir.join(hook_name)).unwrap(),
        new_hook_sha
    );
    assert_eq!(
        compute_sha256(&new_ver_dir.join(bridge_name)).unwrap(),
        new_bridge_sha
    );

    // 6. hooks untouched
    assert_eq!(
        fs::read_to_string(&claude_settings).unwrap(),
        r#"{"hooks":{}}"#
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_local_install_with_activate_replaces_pointers_without_writing_through_symlinks() {
    let root = create_test_dir("activate_symlink_test");
    let runtime_home = root.join("runtime");
    let user_home = root.join("home");
    let candidate_build = root.join("candidate_build");
    fs::create_dir_all(&user_home).unwrap();

    let (hook_name, bridge_name) = exe_names();
    let Some((old_hook_sha, old_bridge_sha)) = seed_active_symlink_runtime(
        &runtime_home,
        "old",
        b"old-writable-version-hook-bytes",
        b"old-writable-version-bridge-bytes",
    ) else {
        let _ = fs::remove_dir_all(&root);
        return;
    };
    let (new_hook_sha, new_bridge_sha) = write_verified_build(
        &candidate_build,
        b"new-activated-hook-bytes-v2",
        b"new-activated-bridge-bytes-v2",
    );

    let output = run_bridge_cli(
        &runtime_home,
        &user_home,
        &[
            "local",
            "install",
            "--activate",
            "--from-build",
            candidate_build.to_str().unwrap(),
        ],
    );
    assert!(
        output.status.success(),
        "activate failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // 1. Immutable old version files MUST NOT be overwritten through symlinks
    assert_eq!(
        compute_sha256(&runtime_home.join("versions/old").join(hook_name)).unwrap(),
        old_hook_sha,
        "versions/old hook binary was overwritten through symlink!"
    );
    assert_eq!(
        compute_sha256(&runtime_home.join("versions/old").join(bridge_name)).unwrap(),
        old_bridge_sha,
        "versions/old bridge binary was overwritten through symlink!"
    );

    // 2. active.json updated to new version_id
    let active_manifest: LocalVersionManifest =
        serde_json::from_str(&fs::read_to_string(runtime_home.join("active.json")).unwrap())
            .unwrap();
    assert_ne!(active_manifest.version_id, "old");

    // 3. current pointer atomically updated to versions/<new_version_id>
    let current_target = fs::read_link(runtime_home.join("current")).unwrap();
    assert_eq!(
        current_target,
        std::path::Path::new("versions").join(&active_manifest.version_id)
    );

    // 4. bin/ entries remain symlinks, point directly at ../versions/<id>/<exe> (not ../current),
    // and resolve to the new version
    assert!(
        fs::symlink_metadata(runtime_home.join("bin").join(hook_name))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        fs::symlink_metadata(runtime_home.join("bin").join(bridge_name))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_link(runtime_home.join("bin").join(hook_name)).unwrap(),
        std::path::Path::new("..")
            .join("versions")
            .join(&active_manifest.version_id)
            .join(hook_name)
    );
    assert_eq!(
        fs::read_link(runtime_home.join("bin").join(bridge_name)).unwrap(),
        std::path::Path::new("..")
            .join("versions")
            .join(&active_manifest.version_id)
            .join(bridge_name)
    );
    assert_eq!(
        compute_sha256(&runtime_home.join("bin").join(hook_name)).unwrap(),
        new_hook_sha
    );
    assert_eq!(
        compute_sha256(&runtime_home.join("bin").join(bridge_name)).unwrap(),
        new_bridge_sha
    );

    // 5. previous/ contains the intact old version for rollback
    let prev_manifest: LocalVersionManifest = serde_json::from_str(
        &fs::read_to_string(runtime_home.join("previous/manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(prev_manifest.version_id, "old");
    assert_eq!(
        compute_sha256(&runtime_home.join("previous").join(hook_name)).unwrap(),
        old_hook_sha
    );
    assert_eq!(
        compute_sha256(&runtime_home.join("previous").join(bridge_name)).unwrap(),
        old_bridge_sha
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_safe_copy_or_replace_does_not_write_through_symlink_target() {
    let root = create_test_dir("safe_copy_symlink_test");
    let immutable_target = root.join("immutable_target.bin");
    let symlink_dst = root.join("alias.bin");
    let new_src = root.join("new_source.bin");

    fs::write(&immutable_target, b"immutable-v1-payload").unwrap();
    let original_sha = compute_sha256(&immutable_target).unwrap();
    if !try_make_symlink_file(&immutable_target, &symlink_dst) {
        let _ = fs::remove_dir_all(&root);
        return;
    }

    fs::write(&new_src, b"replacement-v2-payload").unwrap();
    let new_sha = compute_sha256(&new_src).unwrap();

    safe_copy_or_replace(&new_src, &symlink_dst).unwrap();

    // The immutable target MUST remain untouched
    assert_eq!(
        compute_sha256(&immutable_target).unwrap(),
        original_sha,
        "safe_copy_or_replace wrote through symlink and corrupted target!"
    );
    assert_eq!(compute_sha256(&symlink_dst).unwrap(), new_sha);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_failed_activation_leaves_no_partial_previous_and_rolls_back_prior_previous() {
    let root = create_test_dir("failed_activation_rollback_test");
    let runtime_home = root.join("runtime");
    let user_home = root.join("home");
    let candidate_build = root.join("candidate_build");
    fs::create_dir_all(&user_home).unwrap();

    let (hook_name, bridge_name) = exe_names();
    let Some((old_hook_sha, old_bridge_sha)) = seed_active_symlink_runtime(
        &runtime_home,
        "old",
        b"old-active-hook-v1",
        b"old-active-bridge-v1",
    ) else {
        let _ = fs::remove_dir_all(&root);
        return;
    };
    write_verified_build(
        &candidate_build,
        b"new-candidate-hook-v2",
        b"new-candidate-bridge-v2",
    );

    // Block activation by replacing active.json with a non-empty directory after seeding
    let active_json_path = runtime_home.join("active.json");
    let saved_active_json = fs::read_to_string(&active_json_path).unwrap();
    fs::remove_file(&active_json_path).unwrap();
    fs::create_dir_all(active_json_path.join("blocker")).unwrap();
    fs::write(active_json_path.join("blocker/lock"), b"blocked").unwrap();

    // Case A: No previous/ existed before the failed activation
    let output = run_bridge_cli(
        &runtime_home,
        &user_home,
        &[
            "local",
            "install",
            "--activate",
            "--from-build",
            candidate_build.to_str().unwrap(),
        ],
    );
    assert!(
        !output.status.success(),
        "activation should have failed when active.json is a directory"
    );

    // Must not leave a previous/ directory behind
    assert!(
        !runtime_home.join("previous").exists(),
        "failed activation left behind a partial/uncommitted previous/ directory!"
    );
    // current pointer and bin must remain rolled back to old
    assert_eq!(
        fs::read_link(runtime_home.join("current")).unwrap(),
        std::path::Path::new("versions").join("old")
    );
    assert_eq!(
        compute_sha256(&runtime_home.join("bin").join(hook_name)).unwrap(),
        old_hook_sha
    );
    assert_eq!(
        compute_sha256(&runtime_home.join("bin").join(bridge_name)).unwrap(),
        old_bridge_sha
    );

    // Case B: An earlier previous/ backup ("v0") existed before failed activation;
    // it must be preserved intact on failure.
    let prev_dir = runtime_home.join("previous");
    fs::create_dir_all(&prev_dir).unwrap();
    fs::write(prev_dir.join(hook_name), b"prior-v0-hook").unwrap();
    fs::write(prev_dir.join(bridge_name), b"prior-v0-bridge").unwrap();
    fs::write(prev_dir.join("manifest.json"), r#"{"version_id":"v0"}"#).unwrap();
    let v0_hook_sha = compute_sha256(&prev_dir.join(hook_name)).unwrap();

    // Restore active.json file, but block the second binary in bin/ with a non-empty directory
    fs::remove_dir_all(&active_json_path).unwrap();
    fs::write(&active_json_path, &saved_active_json).unwrap();
    let bin_bridge = runtime_home.join("bin").join(bridge_name);
    fs::remove_file(&bin_bridge).unwrap();
    fs::create_dir_all(bin_bridge.join("blocker")).unwrap();
    fs::write(bin_bridge.join("blocker/file"), b"blocked").unwrap();

    let output2 = run_bridge_cli(
        &runtime_home,
        &user_home,
        &[
            "local",
            "install",
            "--activate",
            "--from-build",
            candidate_build.to_str().unwrap(),
        ],
    );
    assert!(
        !output2.status.success(),
        "activation should have failed when bin/<bridge> is a non-empty directory"
    );

    // previous/ must still contain v0, not overwritten with old ("v1")
    assert_eq!(
        compute_sha256(&prev_dir.join(hook_name)).unwrap(),
        v0_hook_sha,
        "failed activation overwrote existing previous/ backup!"
    );
    assert_eq!(
        fs::read_to_string(prev_dir.join("manifest.json")).unwrap(),
        r#"{"version_id":"v0"}"#
    );
    assert_eq!(
        fs::read_link(runtime_home.join("current")).unwrap(),
        std::path::Path::new("versions").join("old")
    );
    assert_eq!(
        compute_sha256(&runtime_home.join("bin").join(hook_name)).unwrap(),
        old_hook_sha
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_direct_version_symlinks_and_multiple_staged_builds_preserve_immutability_and_rollback() {
    let root = create_test_dir("direct_symlinks_and_rollback_test");
    let runtime_home = root.join("runtime");
    let user_home = root.join("home");
    let build_v1 = root.join("build_v1");
    let build_v2 = root.join("build_v2");
    fs::create_dir_all(&user_home).unwrap();

    let (hook_name, bridge_name) = exe_names();
    let (v1_hook_sha, v1_bridge_sha) =
        write_verified_build(&build_v1, b"v1-hook-content", b"v1-bridge-content");
    let (v2_hook_sha, v2_bridge_sha) =
        write_verified_build(&build_v2, b"v2-hook-content", b"v2-bridge-content");

    // 1. Install & activate v1
    let out1 = run_bridge_cli(
        &runtime_home,
        &user_home,
        &[
            "local",
            "install",
            "--activate",
            "--from-build",
            build_v1.to_str().unwrap(),
        ],
    );
    assert!(out1.status.success());
    let active1: LocalVersionManifest =
        serde_json::from_str(&fs::read_to_string(runtime_home.join("active.json")).unwrap())
            .unwrap();
    let v1_id = active1.version_id.clone();

    // Point bin/* directly at ../versions/<v1_id>/* (direct version symlinks)
    // and mark v1 files read-only to enforce immutability
    let bin_dir = runtime_home.join("bin");
    let _ = fs::remove_file(bin_dir.join(hook_name));
    let _ = fs::remove_file(bin_dir.join(bridge_name));
    if !try_make_symlink_file(
        &std::path::Path::new("..")
            .join("versions")
            .join(&v1_id)
            .join(hook_name),
        &bin_dir.join(hook_name),
    ) || !try_make_symlink_file(
        &std::path::Path::new("..")
            .join("versions")
            .join(&v1_id)
            .join(bridge_name),
        &bin_dir.join(bridge_name),
    ) {
        let _ = fs::remove_dir_all(&root);
        return;
    }

    let v1_dir = runtime_home.join("versions").join(&v1_id);
    for file in [v1_dir.join(hook_name), v1_dir.join(bridge_name)] {
        let mut perms = fs::metadata(&file).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&file, perms).unwrap();
    }

    // 2. Stage and activate v2 from the same git checkout; must not overwrite v1_dir
    let out2 = run_bridge_cli(
        &runtime_home,
        &user_home,
        &[
            "local",
            "install",
            "--activate",
            "--from-build",
            build_v2.to_str().unwrap(),
        ],
    );
    assert!(
        out2.status.success(),
        "activating v2 failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out2.stdout),
        String::from_utf8_lossy(&out2.stderr)
    );

    let active2: LocalVersionManifest =
        serde_json::from_str(&fs::read_to_string(runtime_home.join("active.json")).unwrap())
            .unwrap();
    assert_ne!(active2.version_id, v1_id);

    // v1_dir files must still have v1 hashes
    assert_eq!(
        compute_sha256(&v1_dir.join(hook_name)).unwrap(),
        v1_hook_sha
    );
    assert_eq!(
        compute_sha256(&v1_dir.join(bridge_name)).unwrap(),
        v1_bridge_sha
    );
    // bin/ must resolve to v2 hashes
    assert_eq!(
        compute_sha256(&bin_dir.join(hook_name)).unwrap(),
        v2_hook_sha
    );
    assert_eq!(
        compute_sha256(&bin_dir.join(bridge_name)).unwrap(),
        v2_bridge_sha
    );

    // 3. Rollback to v1 must restore v1 without corrupting v2's staged version
    let v2_dir = runtime_home.join("versions").join(&active2.version_id);
    let out_rb = run_bridge_cli(&runtime_home, &user_home, &["local", "rollback"]);
    assert!(
        out_rb.status.success(),
        "rollback failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out_rb.stdout),
        String::from_utf8_lossy(&out_rb.stderr)
    );
    assert_eq!(
        compute_sha256(&bin_dir.join(hook_name)).unwrap(),
        v1_hook_sha
    );
    assert_eq!(
        compute_sha256(&v2_dir.join(hook_name)).unwrap(),
        v2_hook_sha
    );

    // Restore write perms before cleanup
    for file in [v1_dir.join(hook_name), v1_dir.join(bridge_name)] {
        if let Ok(meta) = fs::metadata(&file) {
            let mut perms = meta.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            let _ = fs::set_permissions(&file, perms);
        }
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_symlink_helper_skips_on_os_error_1314() {
    let err_1314 = std::io::Error::from_raw_os_error(1314);
    assert!(
        !handle_symlink_result(Err(err_1314), std::path::Path::new("dummy_link")),
        "handle_symlink_result must return false (skip) on OS error 1314 instead of panicking"
    );
}

#[test]
fn test_copy_fallback_when_symlink_privilege_unavailable() {
    let root = create_test_dir("copy_fallback_test");
    let runtime_home = root.join("runtime");
    let user_home = root.join("home");
    let build_v1 = root.join("build_v1");
    let build_v2 = root.join("build_v2");
    fs::create_dir_all(&user_home).unwrap();

    let (hook_name, bridge_name) = exe_names();
    let (v1_hook_sha, v1_bridge_sha) = write_verified_build(
        &build_v1,
        b"copy-fallback-v1-hook",
        b"copy-fallback-v1-bridge",
    );
    let (v2_hook_sha, v2_bridge_sha) = write_verified_build(
        &build_v2,
        b"copy-fallback-v2-hook",
        b"copy-fallback-v2-bridge",
    );

    // 1. Install & activate v1 with forced copy fallback
    let out1 = run_bridge_cli_with_env(
        &runtime_home,
        &user_home,
        &[
            "local",
            "install",
            "--activate",
            "--from-build",
            build_v1.to_str().unwrap(),
        ],
        &[("AGENT_OTEL_TEST_FORCE_COPY_FALLBACK", "1")],
    );
    assert!(
        out1.status.success(),
        "copy fallback activate v1 failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out1.stdout),
        String::from_utf8_lossy(&out1.stderr)
    );

    // Under copy fallback, current symlink must not exist and bin/* must be regular files (not symlinks)
    assert!(
        fs::symlink_metadata(runtime_home.join("current")).is_err(),
        "current symlink should not exist when symlink creation fails with 1314"
    );
    let bin_hook = runtime_home.join("bin").join(hook_name);
    let bin_bridge = runtime_home.join("bin").join(bridge_name);
    let hook_meta = fs::symlink_metadata(&bin_hook).unwrap();
    let bridge_meta = fs::symlink_metadata(&bin_bridge).unwrap();
    assert!(!hook_meta.file_type().is_symlink() && hook_meta.is_file());
    assert!(!bridge_meta.file_type().is_symlink() && bridge_meta.is_file());
    assert_eq!(compute_sha256(&bin_hook).unwrap(), v1_hook_sha);
    assert_eq!(compute_sha256(&bin_bridge).unwrap(), v1_bridge_sha);

    // 2. Install & activate v2 with forced copy fallback
    let out2 = run_bridge_cli_with_env(
        &runtime_home,
        &user_home,
        &[
            "local",
            "install",
            "--activate",
            "--from-build",
            build_v2.to_str().unwrap(),
        ],
        &[("AGENT_OTEL_TEST_FORCE_COPY_FALLBACK", "1")],
    );
    assert!(
        out2.status.success(),
        "copy fallback activate v2 failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out2.stdout),
        String::from_utf8_lossy(&out2.stderr)
    );
    assert!(!fs::symlink_metadata(&bin_hook)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(compute_sha256(&bin_hook).unwrap(), v2_hook_sha);
    assert_eq!(compute_sha256(&bin_bridge).unwrap(), v2_bridge_sha);
    assert_eq!(
        compute_sha256(&runtime_home.join("previous").join(hook_name)).unwrap(),
        v1_hook_sha
    );
    assert_eq!(
        compute_sha256(&runtime_home.join("previous").join(bridge_name)).unwrap(),
        v1_bridge_sha
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_sweep_leftover_temp_entries_and_reactivate_prints_ondisk_manifest() {
    let root = create_test_dir("sweep_and_manifest_test");
    let runtime_home = root.join("runtime");
    let user_home = root.join("home");
    let build_v1 = root.join("build_v1");
    fs::create_dir_all(&user_home).unwrap();

    write_verified_build(&build_v1, b"sweep-v1-hook", b"sweep-v1-bridge");

    // Seed leftover temp entries to verify sweep at start of install and activate
    fs::create_dir_all(runtime_home.join(".rollback.stage.9999.1.0")).unwrap();
    fs::write(
        runtime_home.join(".rollback.stage.9999.1.0/stale"),
        b"stale",
    )
    .unwrap();
    fs::create_dir_all(runtime_home.join(".previous.stage.9999.1.0")).unwrap();
    fs::create_dir_all(runtime_home.join("bin")).unwrap();
    fs::write(
        runtime_home
            .join("bin")
            .join(".agent-hook.exe.old.9999.1.0"),
        b"old-exe",
    )
    .unwrap();

    // 1. Stage v1 first without activating
    let out_stage = run_bridge_cli(
        &runtime_home,
        &user_home,
        &[
            "local",
            "install",
            "--from-build",
            build_v1.to_str().unwrap(),
        ],
    );
    assert!(out_stage.status.success());
    assert!(!runtime_home.join(".rollback.stage.9999.1.0").exists());
    assert!(!runtime_home.join(".previous.stage.9999.1.0").exists());
    assert!(!runtime_home
        .join("bin")
        .join(".agent-hook.exe.old.9999.1.0")
        .exists());

    // Customize built_at in on-disk manifest of staged v1 to verify re-activation loads & prints it
    let staged_v1_dir = fs::read_dir(runtime_home.join("versions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut staged_manifest: LocalVersionManifest =
        serde_json::from_str(&fs::read_to_string(staged_v1_dir.join("manifest.json")).unwrap())
            .unwrap();
    staged_manifest.built_at = "1234567890".to_string();
    fs::write(
        staged_v1_dir.join("manifest.json"),
        serde_json::to_string_pretty(&staged_manifest).unwrap(),
    )
    .unwrap();

    // Seed leftover temp entries again before activate
    fs::create_dir_all(runtime_home.join(".rollback.stage.8888.1.0")).unwrap();
    fs::create_dir_all(runtime_home.join(".previous.stage.8888.1.0")).unwrap();
    fs::write(
        runtime_home
            .join("bin")
            .join(".agent-hook.exe.old.8888.1.0"),
        b"old-exe-2",
    )
    .unwrap();

    let out_act = run_bridge_cli(
        &runtime_home,
        &user_home,
        &[
            "local",
            "install",
            "--activate",
            "--from-build",
            build_v1.to_str().unwrap(),
        ],
    );
    assert!(out_act.status.success());
    assert!(!runtime_home.join(".rollback.stage.8888.1.0").exists());
    assert!(!runtime_home.join(".previous.stage.8888.1.0").exists());
    assert!(!runtime_home
        .join("bin")
        .join(".agent-hook.exe.old.8888.1.0")
        .exists());
    let stdout_act = String::from_utf8_lossy(&out_act.stdout);
    assert!(
        stdout_act.contains("1234567890"),
        "re-activating already-staged version must load and print on-disk manifest (expected built_at 1234567890 in stdout): {stdout_act}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_rollback_verifies_version_hashes_and_stages_suffixed_id_on_mismatch() {
    let root = create_test_dir("rollback_hash_mismatch_test");
    let runtime_home = root.join("runtime");
    let user_home = root.join("home");
    let build_v1 = root.join("build_v1");
    let build_v2 = root.join("build_v2");
    fs::create_dir_all(&user_home).unwrap();

    let (hook_name, bridge_name) = exe_names();
    let (v1_hook_sha, v1_bridge_sha) = write_verified_build(
        &build_v1,
        b"verified-v1-hook-bytes",
        b"verified-v1-bridge-bytes",
    );
    let (v2_hook_sha, v2_bridge_sha) = write_verified_build(
        &build_v2,
        b"verified-v2-hook-bytes",
        b"verified-v2-bridge-bytes",
    );

    let out1 = run_bridge_cli(
        &runtime_home,
        &user_home,
        &[
            "local",
            "install",
            "--activate",
            "--from-build",
            build_v1.to_str().unwrap(),
        ],
    );
    assert!(out1.status.success());
    let active1: LocalVersionManifest =
        serde_json::from_str(&fs::read_to_string(runtime_home.join("active.json")).unwrap())
            .unwrap();
    let v1_id = active1.version_id;

    let out2 = run_bridge_cli(
        &runtime_home,
        &user_home,
        &[
            "local",
            "install",
            "--activate",
            "--from-build",
            build_v2.to_str().unwrap(),
        ],
    );
    assert!(out2.status.success());

    // Tamper with versions/<v1_id>/agent-hook so its SHA-256 no longer matches previous/manifest.json
    let v1_dir = runtime_home.join("versions").join(&v1_id);
    fs::write(v1_dir.join(hook_name), b"tampered-corrupted-v1-hook-bytes").unwrap();

    // Rollback must detect the hash mismatch in versions/<v1_id>/, stage from previous/ under a
    // suffixed version_id (<v1_id>-<8hex>), and activate that verified version.
    let out_rb = run_bridge_cli(&runtime_home, &user_home, &["local", "rollback"]);
    assert!(
        out_rb.status.success(),
        "rollback with mismatched versions/<id> should succeed by staging from previous/: stdout={} stderr={}",
        String::from_utf8_lossy(&out_rb.stdout),
        String::from_utf8_lossy(&out_rb.stderr)
    );

    let bin_dir = runtime_home.join("bin");
    assert_eq!(
        compute_sha256(&bin_dir.join(hook_name)).unwrap(),
        v1_hook_sha,
        "rollback activated tampered binary instead of verified previous/ binary!"
    );
    assert_eq!(
        compute_sha256(&bin_dir.join(bridge_name)).unwrap(),
        v1_bridge_sha
    );

    let active_after_rb: LocalVersionManifest =
        serde_json::from_str(&fs::read_to_string(runtime_home.join("active.json")).unwrap())
            .unwrap();
    assert!(
        active_after_rb.version_id.starts_with(&format!("{v1_id}-"))
            && active_after_rb.version_id.len() == v1_id.len() + 1 + 8,
        "expected suffixed version_id '<v1_id>-<8hex>', got '{}'",
        active_after_rb.version_id
    );

    // Next: if both versions/<id> and previous/ fail hash verification, rollback must fail
    // WITHOUT leaving the installation without a usable current / bin.
    let out3 = run_bridge_cli(
        &runtime_home,
        &user_home,
        &[
            "local",
            "install",
            "--activate",
            "--from-build",
            build_v2.to_str().unwrap(),
        ],
    );
    assert!(out3.status.success());
    let active_v2: LocalVersionManifest =
        serde_json::from_str(&fs::read_to_string(runtime_home.join("active.json")).unwrap())
            .unwrap();

    // Corrupt both versions/<prev_id> and previous/<hook_name>
    let prev_manifest: LocalVersionManifest = serde_json::from_str(
        &fs::read_to_string(runtime_home.join("previous/manifest.json")).unwrap(),
    )
    .unwrap();
    fs::write(
        runtime_home
            .join("versions")
            .join(&prev_manifest.version_id)
            .join(hook_name),
        b"corrupted-in-versions",
    )
    .unwrap();
    fs::write(
        runtime_home.join("previous").join(hook_name),
        b"corrupted-in-previous",
    )
    .unwrap();

    let out_bad_rb = run_bridge_cli(&runtime_home, &user_home, &["local", "rollback"]);
    assert!(
        !out_bad_rb.status.success(),
        "rollback must fail when both versions/<id> and previous/ fail SHA-256 verification"
    );

    // Active v2 must remain completely usable
    assert_eq!(
        compute_sha256(&bin_dir.join(hook_name)).unwrap(),
        v2_hook_sha
    );
    assert_eq!(
        compute_sha256(&bin_dir.join(bridge_name)).unwrap(),
        v2_bridge_sha
    );
    let still_active: LocalVersionManifest =
        serde_json::from_str(&fs::read_to_string(runtime_home.join("active.json")).unwrap())
            .unwrap();
    assert_eq!(still_active.version_id, active_v2.version_id);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_version_id_traversal_and_invalid_characters_rejected() {
    let root = create_test_dir("version_id_traversal_test");
    let runtime_home = root.join("runtime");
    let user_home = root.join("home");
    let build_v1 = root.join("build_v1");
    fs::create_dir_all(&user_home).unwrap();

    let (hook_name, bridge_name) = exe_names();
    let (v1_hook_sha, v1_bridge_sha) =
        write_verified_build(&build_v1, b"safe-v1-hook", b"safe-v1-bridge");

    let out1 = run_bridge_cli(
        &runtime_home,
        &user_home,
        &[
            "local",
            "install",
            "--activate",
            "--from-build",
            build_v1.to_str().unwrap(),
        ],
    );
    assert!(out1.status.success());
    let active1: LocalVersionManifest =
        serde_json::from_str(&fs::read_to_string(runtime_home.join("active.json")).unwrap())
            .unwrap();

    let prev_dir = runtime_home.join("previous");
    write_verified_build(&prev_dir, b"evil-hook", b"evil-bridge");
    let evil_hook_sha = compute_sha256(&prev_dir.join(hook_name)).unwrap();
    let evil_bridge_sha = compute_sha256(&prev_dir.join(bridge_name)).unwrap();

    for bad_id in [
        "../escape",
        "..\\escape",
        "..",
        ".",
        "a..b",
        "sub/dir",
        "sub\\dir",
        "/abs/path",
        "C:\\abs\\path",
        "bad space",
        "bad$char",
        "",
    ] {
        let mut bad_manifest = active1.clone();
        bad_manifest.version_id = bad_id.to_string();
        bad_manifest.binaries[0].sha256 = evil_hook_sha.clone();
        bad_manifest.binaries[1].sha256 = evil_bridge_sha.clone();
        fs::write(
            prev_dir.join("manifest.json"),
            serde_json::to_string_pretty(&bad_manifest).unwrap(),
        )
        .unwrap();

        let out_rb = run_bridge_cli(&runtime_home, &user_home, &["local", "rollback"]);
        assert!(
            !out_rb.status.success(),
            "rollback must reject invalid version_id '{bad_id}'"
        );
        assert!(
            !runtime_home.join("escape").exists(),
            "path traversal created directory outside versions/ for '{bad_id}'!"
        );
        assert_eq!(
            compute_sha256(&runtime_home.join("bin").join(hook_name)).unwrap(),
            v1_hook_sha
        );
        assert_eq!(
            compute_sha256(&runtime_home.join("bin").join(bridge_name)).unwrap(),
            v1_bridge_sha
        );
    }

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_failed_activation_reports_restore_incomplete_and_keeps_rollback_backup() {
    let root = create_test_dir("restore_error_reporting_test");
    let runtime_home = root.join("runtime");
    let user_home = root.join("home");
    let candidate_build = root.join("candidate_build");
    fs::create_dir_all(&user_home).unwrap();

    let (hook_name, _bridge_name) = exe_names();
    let Some((_old_hook_sha, _old_bridge_sha)) = seed_active_symlink_runtime(
        &runtime_home,
        "old",
        b"old-hook-for-restore-err",
        b"old-bridge-for-restore-err",
    ) else {
        let _ = fs::remove_dir_all(&root);
        return;
    };
    write_verified_build(
        &candidate_build,
        b"new-hook-for-restore-err",
        b"new-bridge-for-restore-err",
    );

    // Block activation at active.json by making active.json a non-empty directory.
    // With AGENT_OTEL_TEST_FORCE_COPY_FALLBACK=1, bin/<hook> (a symlink) is replaced with a
    // regular file copy during activation, then active.json fails with IsADirectory, and then
    // restoring bin/<hook> back to PriorEntryState::Symlink fails with OS error 1314.
    let active_json_path = runtime_home.join("active.json");
    fs::remove_file(&active_json_path).unwrap();
    fs::create_dir_all(active_json_path.join("blocker")).unwrap();
    fs::write(active_json_path.join("blocker/lock"), b"blocked").unwrap();

    let output = run_bridge_cli_with_env(
        &runtime_home,
        &user_home,
        &[
            "local",
            "install",
            "--activate",
            "--from-build",
            candidate_build.to_str().unwrap(),
        ],
        &[("AGENT_OTEL_TEST_FORCE_COPY_FALLBACK", "1")],
    );
    assert!(
        !output.status.success(),
        "activation must fail when active.json is a directory"
    );

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("restore incomplete"),
        "expected 'restore incomplete' in error output, got: {combined}"
    );

    // The .rollback.stage.* backup directory must be preserved in runtime_home
    let rollback_backups: Vec<_> = fs::read_dir(&runtime_home)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(".rollback.stage.")
        })
        .collect();
    assert_eq!(
        rollback_backups.len(),
        1,
        "expected .rollback.stage.* directory to be preserved on restore error"
    );
    let _ = hook_name;

    let _ = fs::remove_dir_all(&root);
}
