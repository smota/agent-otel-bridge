/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryArtifactInfo {
    pub filename: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalVersionManifest {
    pub version: String,
    pub version_id: String,
    pub git_commit: String,
    pub git_branch: String,
    pub is_dirty: bool,
    pub built_at: String,
    pub target_triple: String,
    pub binaries: Vec<BinaryArtifactInfo>,
}

pub fn get_canonical_local_dir() -> PathBuf {
    if let Ok(home) = std::env::var("AGENT_OTEL_HOME") {
        return PathBuf::from(home);
    }

    #[cfg(windows)]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            return PathBuf::from(local_app_data).join("agent-otel-bridge");
        }
        if let Ok(user_profile) = std::env::var("USERPROFILE") {
            return PathBuf::from(user_profile)
                .join("AppData")
                .join("Local")
                .join("agent-otel-bridge");
        }
    }

    #[cfg(unix)]
    {
        if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
            return PathBuf::from(xdg_data).join("agent-otel-bridge");
        }
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("agent-otel-bridge");
        }
    }

    PathBuf::from(".agent-otel-bridge")
}

pub fn get_canonical_bin_dir() -> PathBuf {
    get_canonical_local_dir().join("bin")
}

pub fn get_canonical_hook_path() -> PathBuf {
    let filename = if cfg!(windows) {
        "agent-hook.exe"
    } else {
        "agent-hook"
    };
    get_canonical_bin_dir().join(filename)
}

pub fn get_canonical_bridge_path() -> PathBuf {
    let filename = if cfg!(windows) {
        "agent-otel-bridge.exe"
    } else {
        "agent-otel-bridge"
    };
    get_canonical_bin_dir().join(filename)
}

pub fn get_versions_dir() -> PathBuf {
    get_canonical_local_dir().join("versions")
}

pub fn get_previous_dir() -> PathBuf {
    get_canonical_local_dir().join("previous")
}

pub fn get_active_manifest_path() -> PathBuf {
    get_canonical_local_dir().join("active.json")
}

pub fn compute_sha256(path: &Path) -> Result<String, io::Error> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];

    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

pub fn safe_copy_or_replace(src: &Path, dst: &Path) -> Result<(), io::Error> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }

    if dst.exists() {
        match fs::copy(src, dst) {
            Ok(_) => Ok(()),
            Err(_) => {
                // If direct overwrite fails (e.g. executable locked on Windows), rename and replace
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                let old_dst = dst.with_extension(format!("old.{}", timestamp));
                fs::rename(dst, &old_dst)?;
                fs::copy(src, dst)?;
                let _ = fs::remove_file(&old_dst);
                Ok(())
            }
        }
    } else {
        fs::copy(src, dst)?;
        Ok(())
    }
}

pub fn extract_git_metadata() -> (String, String, bool) {
    let commit = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let branch = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let is_dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false);

    (commit, branch, is_dirty)
}

pub fn install_candidate(
    activate: bool,
    update_hooks: bool,
    from_build: Option<&Path>,
) -> Result<LocalVersionManifest, Box<dyn std::error::Error>> {
    println!("\n=== Installing agent-otel-bridge to isolated local runtime ===");

    let hook_name = if cfg!(windows) {
        "agent-hook.exe"
    } else {
        "agent-hook"
    };
    let bridge_name = if cfg!(windows) {
        "agent-otel-bridge.exe"
    } else {
        "agent-otel-bridge"
    };

    let (src_hook, src_bridge) = if let Some(build_dir) = from_build {
        (build_dir.join(hook_name), build_dir.join(bridge_name))
    } else {
        let release_dir = Path::new("target").join("release");
        let hook_path = release_dir.join(hook_name);
        let bridge_path = release_dir.join(bridge_name);

        if !hook_path.exists() || !bridge_path.exists() {
            println!("  [build] Compiling production release binaries...");
            let status = Command::new("cargo")
                .args([
                    "build",
                    "--release",
                    "-p",
                    "agent-otel-client",
                    "-p",
                    "agent-otel-cli",
                ])
                .status()?;
            if !status.success() {
                return Err("Failed to compile candidate binaries with cargo".into());
            }
        }
        (hook_path, bridge_path)
    };

    if !src_hook.is_file() {
        return Err(format!("Source hook binary not found: {}", src_hook.display()).into());
    }
    if !src_bridge.is_file() {
        return Err(format!("Source bridge binary not found: {}", src_bridge.display()).into());
    }

    let hook_sha = compute_sha256(&src_hook)?;
    let hook_size = fs::metadata(&src_hook)?.len();
    let bridge_sha = compute_sha256(&src_bridge)?;
    let bridge_size = fs::metadata(&src_bridge)?.len();

    let (commit, branch, is_dirty) = extract_git_metadata();
    let short_commit = if commit.len() >= 7 {
        &commit[..7]
    } else {
        &commit
    };
    let dirty_suffix = if is_dirty { "-dirty" } else { "" };
    let version = env!("CARGO_PKG_VERSION");
    let version_id = format!("{}-{}{}", version, short_commit, dirty_suffix);

    let built_at = {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        format!("{now}")
    };

    let manifest = LocalVersionManifest {
        version: version.to_string(),
        version_id: version_id.clone(),
        git_commit: commit,
        git_branch: branch,
        is_dirty,
        built_at,
        target_triple: std::env::consts::ARCH.to_string(),
        binaries: vec![
            BinaryArtifactInfo {
                filename: hook_name.to_string(),
                sha256: hook_sha,
                size_bytes: hook_size,
            },
            BinaryArtifactInfo {
                filename: bridge_name.to_string(),
                sha256: bridge_sha,
                size_bytes: bridge_size,
            },
        ],
    };

    let versions_dir = get_versions_dir().join(&version_id);
    fs::create_dir_all(&versions_dir)?;

    let target_hook = versions_dir.join(hook_name);
    let target_bridge = versions_dir.join(bridge_name);
    safe_copy_or_replace(&src_hook, &target_hook)?;
    safe_copy_or_replace(&src_bridge, &target_bridge)?;

    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    fs::write(versions_dir.join("manifest.json"), &manifest_json)?;

    println!("  [staged] Build staged at: {}", versions_dir.display());
    println!("           Version ID: {}", version_id);
    println!("           Hook SHA256:   {}", manifest.binaries[0].sha256);
    println!("           Bridge SHA256: {}", manifest.binaries[1].sha256);

    if activate {
        activate_version(&version_id)?;

        if update_hooks {
            println!("\n  [hooks] Updating agent lifecycle hooks with canonical absolute paths...");
            let canonical_hook = get_canonical_hook_path();
            let hook_str = canonical_hook.to_string_lossy();
            crate::hooks::run_install("all", false, Some(&hook_str))?;
        }
    }

    Ok(manifest)
}

pub fn activate_version(version_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let version_dir = get_versions_dir().join(version_id);
    if !version_dir.is_dir() {
        return Err(format!(
            "Version '{}' does not exist in versions directory",
            version_id
        )
        .into());
    }

    let bin_dir = get_canonical_bin_dir();
    let previous_dir = get_previous_dir();

    // Preserve previous version for rollback
    if bin_dir.is_dir() {
        let _ = fs::remove_dir_all(&previous_dir);
        fs::create_dir_all(&previous_dir)?;
        if let Ok(entries) = fs::read_dir(&bin_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() {
                    let _ = safe_copy_or_replace(&p, &previous_dir.join(entry.file_name()));
                }
            }
        }
        let active_manifest = get_active_manifest_path();
        if active_manifest.is_file() {
            let _ = safe_copy_or_replace(&active_manifest, &previous_dir.join("manifest.json"));
        }
    }

    fs::create_dir_all(&bin_dir)?;
    let hook_name = if cfg!(windows) {
        "agent-hook.exe"
    } else {
        "agent-hook"
    };
    let bridge_name = if cfg!(windows) {
        "agent-otel-bridge.exe"
    } else {
        "agent-otel-bridge"
    };

    safe_copy_or_replace(&version_dir.join(hook_name), &bin_dir.join(hook_name))?;
    safe_copy_or_replace(&version_dir.join(bridge_name), &bin_dir.join(bridge_name))?;

    let manifest_content = fs::read_to_string(version_dir.join("manifest.json"))?;
    fs::write(get_active_manifest_path(), manifest_content)?;

    println!(
        "  [active] Version '{}' is now active at: {}",
        version_id,
        bin_dir.display()
    );
    Ok(())
}

pub fn rollback_candidate(update_hooks: bool) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Rolling back agent-otel-bridge to previous installation ===");
    let previous_dir = get_previous_dir();
    let previous_manifest = previous_dir.join("manifest.json");

    if !previous_manifest.is_file() {
        return Err("No previous installation found for rollback.".into());
    }

    let manifest_content = fs::read_to_string(&previous_manifest)?;
    let manifest: LocalVersionManifest = serde_json::from_str(&manifest_content)?;

    let bin_dir = get_canonical_bin_dir();
    fs::create_dir_all(&bin_dir)?;

    let hook_name = if cfg!(windows) {
        "agent-hook.exe"
    } else {
        "agent-hook"
    };
    let bridge_name = if cfg!(windows) {
        "agent-otel-bridge.exe"
    } else {
        "agent-otel-bridge"
    };

    safe_copy_or_replace(&previous_dir.join(hook_name), &bin_dir.join(hook_name))?;
    safe_copy_or_replace(&previous_dir.join(bridge_name), &bin_dir.join(bridge_name))?;
    fs::write(get_active_manifest_path(), &manifest_content)?;

    println!(
        "  [ok] Successfully rolled back to: {}",
        manifest.version_id
    );

    if update_hooks {
        let canonical_hook = get_canonical_hook_path();
        let hook_str = canonical_hook.to_string_lossy();
        crate::hooks::run_install("all", false, Some(&hook_str))?;
    }

    Ok(())
}

pub fn run_status() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== agent-otel-bridge Local Runtime Status ===");
    let local_dir = get_canonical_local_dir();
    println!("Local Runtime Home: {}", local_dir.display());

    let active_manifest_path = get_active_manifest_path();
    if active_manifest_path.is_file() {
        let content = fs::read_to_string(&active_manifest_path)?;
        if let Ok(manifest) = serde_json::from_str::<LocalVersionManifest>(&content) {
            println!("Active Version:     {}", manifest.version_id);
            println!("  Package Version:  {}", manifest.version);
            println!("  Git Commit:       {}", manifest.git_commit);
            println!("  Git Branch:       {}", manifest.git_branch);
            println!(
                "  Working Tree:     {}",
                if manifest.is_dirty { "DIRTY" } else { "CLEAN" }
            );
            println!("  Built At Epoch:   {}", manifest.built_at);

            println!("\nInstalled Binaries:");
            let bin_dir = get_canonical_bin_dir();
            for bin in &manifest.binaries {
                let bin_path = bin_dir.join(&bin.filename);
                if bin_path.is_file() {
                    let current_sha = compute_sha256(&bin_path).unwrap_or_default();
                    let matches = current_sha == bin.sha256;
                    println!(
                        "  {} ({} bytes) -> {}",
                        bin.filename,
                        bin.size_bytes,
                        if matches {
                            "[ok] verified SHA-256"
                        } else {
                            "[fail] SHA-256 mismatch"
                        }
                    );
                } else {
                    println!("  {} -> [fail] missing from bin/", bin.filename);
                }
            }
        } else {
            println!("Active Manifest:    [warn] corrupted active.json");
        }
    } else {
        println!("Active Version:     [none] Not installed locally");
    }

    let prev_dir = get_previous_dir();
    if prev_dir.join("manifest.json").is_file() {
        if let Ok(content) = fs::read_to_string(prev_dir.join("manifest.json")) {
            if let Ok(prev) = serde_json::from_str::<LocalVersionManifest>(&content) {
                println!(
                    "\nPrevious Version (Rollback available): {}",
                    prev.version_id
                );
            }
        }
    }

    // Daemon IPC test
    println!("\nDaemon Status:");
    match agent_otel_ipc::client::try_send(
        agent_otel_ipc::frame::MsgType::HealthPing,
        b"status-probe",
    ) {
        Ok(()) => println!("  Daemon named pipe: [ok] RUNNING and listening"),
        Err(()) => println!("  Daemon named pipe: [stopped] Not currently running"),
    }

    // Hook registrations status
    println!();
    crate::hooks::run_status()?;

    Ok(())
}

pub fn uninstall_local(purge: bool, remove_hooks: bool) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Uninstalling agent-otel-bridge local runtime ===");

    if remove_hooks {
        crate::hooks::run_uninstall("all", false, None)?;
    }

    let bin_dir = get_canonical_bin_dir();
    if bin_dir.is_dir() {
        let _ = fs::remove_dir_all(&bin_dir);
        println!("  [ok] Removed bin directory: {}", bin_dir.display());
    }

    let active_manifest = get_active_manifest_path();
    if active_manifest.is_file() {
        let _ = fs::remove_file(&active_manifest);
    }

    let prev_dir = get_previous_dir();
    if prev_dir.is_dir() {
        let _ = fs::remove_dir_all(&prev_dir);
        println!("  [ok] Removed previous rollback version");
    }

    if purge {
        let local_dir = get_canonical_local_dir();
        let _ = fs::remove_dir_all(&local_dir);
        println!(
            "  [ok] Purged entire local runtime home: {}",
            local_dir.display()
        );
    }

    println!("\nLocal runtime uninstalled.\n");
    Ok(())
}
