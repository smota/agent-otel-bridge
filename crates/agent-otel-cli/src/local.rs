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
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

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

pub(crate) fn native_release_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target"))
        .join("release")
}

pub fn get_plugins_stage_dir() -> PathBuf {
    get_canonical_local_dir().join("plugins-stage")
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

pub fn get_current_link_path() -> PathBuf {
    get_canonical_local_dir().join("current")
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

fn unique_temp_path(target: &Path, tag: &str) -> PathBuf {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("artifact");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(
        ".{name}.{tag}.{}.{nanos}.{seq}",
        std::process::id()
    ))
}

fn remove_path_no_follow(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                #[cfg(windows)]
                {
                    if fs::remove_file(path).is_err() {
                        fs::remove_dir(path)?;
                    }
                    Ok(())
                }
                #[cfg(not(windows))]
                {
                    fs::remove_file(path)
                }
            } else if meta.is_file() {
                fs::remove_file(path)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::IsADirectory,
                    format!(
                        "Refusing to remove directory as file/symlink: {}",
                        path.display()
                    ),
                ))
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn replace_entry_with_staged(staged: &Path, dst: &Path) -> io::Result<()> {
    if let Ok(meta) = fs::symlink_metadata(dst) {
        if meta.is_dir() && !meta.file_type().is_symlink() {
            let _ = remove_path_no_follow(staged);
            return Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                format!("Destination is a directory: {}", dst.display()),
            ));
        }
    }

    if fs::rename(staged, dst).is_ok() {
        return Ok(());
    }

    let old_dst = unique_temp_path(dst, "old");
    if let Err(rename_err) = fs::rename(dst, &old_dst) {
        let _ = remove_path_no_follow(staged);
        return Err(rename_err);
    }
    if let Err(replace_err) = fs::rename(staged, dst) {
        let _ = fs::rename(&old_dst, dst);
        let _ = remove_path_no_follow(staged);
        return Err(replace_err);
    }
    let _ = remove_path_no_follow(&old_dst);
    Ok(())
}

pub fn safe_copy_or_replace(src: &Path, dst: &Path) -> Result<(), io::Error> {
    if let Ok(meta) = fs::symlink_metadata(dst) {
        if meta.is_dir() && !meta.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                format!("Destination is a directory: {}", dst.display()),
            ));
        }
    }

    if let Some(parent) = dst.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }

    // Stage into a sibling temporary file and atomically rename over dst so we
    // never open or truncate an existing symlink target in place.
    let tmp_dst = unique_temp_path(dst, "copy");
    if let Err(e) = fs::copy(src, &tmp_dst) {
        let _ = remove_path_no_follow(&tmp_dst);
        return Err(e);
    }

    replace_entry_with_staged(&tmp_dst, dst)
}

fn atomic_write_bytes(dst: &Path, content: &[u8]) -> io::Result<()> {
    if let Ok(meta) = fs::symlink_metadata(dst) {
        if meta.is_dir() && !meta.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                format!("Destination is a directory: {}", dst.display()),
            ));
        }
    }

    if let Some(parent) = dst.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }

    let tmp_dst = unique_temp_path(dst, "write");
    if let Err(e) = fs::write(&tmp_dst, content) {
        let _ = remove_path_no_follow(&tmp_dst);
        return Err(e);
    }

    replace_entry_with_staged(&tmp_dst, dst)
}

fn create_symlink_entry(target: &Path, link: &Path, is_dir_target: bool) -> io::Result<()> {
    #[cfg(unix)]
    {
        let _ = is_dir_target;
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        if is_dir_target {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        }
    }
}

fn atomic_replace_symlink(target: &Path, link: &Path, is_dir_target: bool) -> io::Result<()> {
    if let Ok(meta) = fs::symlink_metadata(link) {
        if meta.is_dir() && !meta.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                format!("Cannot replace directory with symlink: {}", link.display()),
            ));
        }
    }

    if let Some(parent) = link.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }

    let tmp_link = unique_temp_path(link, "symlink");
    create_symlink_entry(target, &tmp_link, is_dir_target)?;
    replace_entry_with_staged(&tmp_link, link)
}

fn is_symlink_privilege_error(err: &io::Error) -> bool {
    #[cfg(windows)]
    {
        err.raw_os_error() == Some(1314)
    }
    #[cfg(not(windows))]
    {
        let _ = err;
        false
    }
}

fn symlink_points_through_current(link_target: &Path, current_link: &Path, exe_name: &str) -> bool {
    if link_target == Path::new("..").join("current").join(exe_name) {
        return true;
    }
    let parts: Vec<String> = link_target
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if parts == ["..", "current", exe_name] {
        return true;
    }
    link_target == current_link.join(exe_name)
}

#[derive(Debug, Clone)]
enum PriorEntryState {
    Missing,
    Symlink {
        target: PathBuf,
        is_dir_target: bool,
    },
    RegularFile {
        backup_path: PathBuf,
    },
    Directory,
}

fn record_prior_entry(
    path: &Path,
    is_dir_symlink_hint: bool,
    rollback_stage: &Path,
    label: &str,
) -> io::Result<PriorEntryState> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                let target = fs::read_link(path)?;
                Ok(PriorEntryState::Symlink {
                    target,
                    is_dir_target: is_dir_symlink_hint || path.is_dir(),
                })
            } else if meta.is_file() {
                fs::create_dir_all(rollback_stage)?;
                let backup_path = rollback_stage.join(label);
                fs::copy(path, &backup_path)?;
                Ok(PriorEntryState::RegularFile { backup_path })
            } else {
                Ok(PriorEntryState::Directory)
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(PriorEntryState::Missing),
        Err(e) => Err(e),
    }
}

fn restore_prior_entry(path: &Path, state: &PriorEntryState) {
    match state {
        PriorEntryState::Missing => {
            let _ = remove_path_no_follow(path);
        }
        PriorEntryState::Symlink {
            target,
            is_dir_target,
        } => {
            let _ = atomic_replace_symlink(target, path, *is_dir_target);
        }
        PriorEntryState::RegularFile { backup_path } => {
            let _ = safe_copy_or_replace(backup_path, path);
        }
        PriorEntryState::Directory => {}
    }
}

fn version_dir_matches_hashes(
    version_dir: &Path,
    hook_name: &str,
    hook_sha: &str,
    bridge_name: &str,
    bridge_sha: &str,
) -> bool {
    let hook_path = version_dir.join(hook_name);
    let bridge_path = version_dir.join(bridge_name);
    let manifest_path = version_dir.join("manifest.json");
    let Ok(hook_meta) = fs::symlink_metadata(&hook_path) else {
        return false;
    };
    let Ok(bridge_meta) = fs::symlink_metadata(&bridge_path) else {
        return false;
    };
    if hook_meta.file_type().is_symlink()
        || bridge_meta.file_type().is_symlink()
        || !hook_meta.is_file()
        || !bridge_meta.is_file()
        || !manifest_path.is_file()
    {
        return false;
    }
    compute_sha256(&hook_path).ok().as_deref() == Some(hook_sha)
        && compute_sha256(&bridge_path).ok().as_deref() == Some(bridge_sha)
}

fn resolve_version_id(
    versions_root: &Path,
    base_version_id: &str,
    hook_name: &str,
    hook_sha: &str,
    bridge_name: &str,
    bridge_sha: &str,
) -> (String, bool) {
    let base_dir = versions_root.join(base_version_id);
    if !base_dir.exists() && fs::symlink_metadata(&base_dir).is_err() {
        return (base_version_id.to_string(), false);
    }
    if version_dir_matches_hashes(&base_dir, hook_name, hook_sha, bridge_name, bridge_sha) {
        return (base_version_id.to_string(), true);
    }

    let mut hasher = Sha256::new();
    hasher.update(hook_sha.as_bytes());
    hasher.update(b":");
    hasher.update(bridge_sha.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    let candidate_id = format!("{}-{}", base_version_id, &digest[..8]);
    let candidate_dir = versions_root.join(&candidate_id);
    let already_staged =
        version_dir_matches_hashes(&candidate_dir, hook_name, hook_sha, bridge_name, bridge_sha);
    (candidate_id, already_staged)
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
        let release_dir = if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            native_release_dir()
        } else {
            Path::new("target").join("release")
        };
        let hook_path = release_dir.join(hook_name);
        let bridge_path = release_dir.join(bridge_name);

        if !hook_path.exists() || !bridge_path.exists() {
            println!("  [build] Compiling production release binaries...");
            let mut build = Command::new("cargo");
            if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
                build.args([
                    "build",
                    "--release",
                    "--locked",
                    "--target",
                    "x86_64-unknown-linux-gnu",
                    "-p",
                    "agent-otel-bridge",
                ]);
            } else {
                build.args([
                    "build",
                    "--release",
                    "-p",
                    "agent-otel-client",
                    "-p",
                    "agent-otel-cli",
                ]);
            }
            let status = build.status()?;
            if !status.success() {
                return Err("Failed to compile candidate binaries with cargo".into());
            }
            if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
                let native_bridge = release_dir
                    .parent()
                    .ok_or("Missing native build output directory")?
                    .join("x86_64-unknown-linux-gnu/release")
                    .join(bridge_name);
                safe_copy_or_replace(&native_bridge, &bridge_path)?;
            }
        }
        if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            // Ordinary Cargo builds can replace agent-hook. Build the reviewed
            // production-only native profile last; test harnesses keep unwind.
            let status = Command::new("bash")
                .arg("scripts/build-linux-hook.sh")
                .status()?;
            if !status.success() {
                return Err("Native Linux hook packaging failed; nothing installed".into());
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
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) && hook_size >= 300_000 {
        return Err(format!(
            "Native Linux hook candidate is {hook_size} bytes; strict limit is below 300000 bytes"
        )
        .into());
    }
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
    let base_version_id = format!("{}-{}{}", version, short_commit, dirty_suffix);

    let versions_root = get_versions_dir();
    let (version_id, already_staged) = resolve_version_id(
        &versions_root,
        &base_version_id,
        hook_name,
        &hook_sha,
        bridge_name,
        &bridge_sha,
    );

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

    let versions_dir = versions_root.join(&version_id);
    if !already_staged {
        if !versions_root.exists() {
            fs::create_dir_all(&versions_root)?;
        }
        let stage_dir = unique_temp_path(&versions_dir, "stage");
        let stage_result = (|| -> Result<(), Box<dyn std::error::Error>> {
            fs::create_dir_all(&stage_dir)?;
            fs::copy(&src_hook, stage_dir.join(hook_name))?;
            fs::copy(&src_bridge, stage_dir.join(bridge_name))?;
            let manifest_json = serde_json::to_string_pretty(&manifest)?;
            fs::write(stage_dir.join("manifest.json"), manifest_json)?;
            fs::rename(&stage_dir, &versions_dir)?;
            Ok(())
        })();
        if let Err(err) = stage_result {
            let _ = fs::remove_dir_all(&stage_dir);
            return Err(err);
        }
    }

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

fn switch_active_pointers(
    local_dir: &Path,
    version_id: &str,
    preserve_previous: bool,
) -> Result<(), Box<dyn std::error::Error>> {
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

    let version_dir = local_dir.join("versions").join(version_id);
    if !version_dir.is_dir() {
        return Err(format!(
            "Version '{}' does not exist in versions directory",
            version_id
        )
        .into());
    }

    let version_hook = version_dir.join(hook_name);
    let version_bridge = version_dir.join(bridge_name);
    let version_manifest_path = version_dir.join("manifest.json");
    if !version_hook.is_file() || !version_bridge.is_file() || !version_manifest_path.is_file() {
        return Err(format!(
            "Version '{}' is incomplete in {}",
            version_id,
            version_dir.display()
        )
        .into());
    }
    let manifest_content = fs::read_to_string(&version_manifest_path)?;
    let _parsed_manifest: LocalVersionManifest = serde_json::from_str(&manifest_content)?;

    let current_link = local_dir.join("current");
    let bin_dir = local_dir.join("bin");
    let bin_hook = bin_dir.join(hook_name);
    let bin_bridge = bin_dir.join(bridge_name);
    let active_manifest = local_dir.join("active.json");
    let previous_dir = local_dir.join("previous");

    let rollback_stage = unique_temp_path(&local_dir.join("rollback"), "stage");
    let prior_current = record_prior_entry(&current_link, true, &rollback_stage, "current")?;
    let prior_hook = record_prior_entry(&bin_hook, false, &rollback_stage, hook_name)?;
    let prior_bridge = record_prior_entry(&bin_bridge, false, &rollback_stage, bridge_name)?;
    let prior_active = record_prior_entry(&active_manifest, false, &rollback_stage, "active.json")?;

    // Stage previous/ into a temporary directory before mutating any canonical pointer.
    let same_active_version = fs::read_to_string(&active_manifest)
        .ok()
        .and_then(|s| serde_json::from_str::<LocalVersionManifest>(&s).ok())
        .map(|m| m.version_id == version_id)
        .unwrap_or(false);

    let mut previous_stage: Option<PathBuf> = None;
    if preserve_previous && !same_active_version {
        let prior_hook_src = if bin_hook.is_file() {
            Some(bin_hook.clone())
        } else if current_link.join(hook_name).is_file() {
            Some(current_link.join(hook_name))
        } else {
            None
        };
        let prior_bridge_src = if bin_bridge.is_file() {
            Some(bin_bridge.clone())
        } else if current_link.join(bridge_name).is_file() {
            Some(current_link.join(bridge_name))
        } else {
            None
        };
        let prior_manifest_src = if active_manifest.is_file() {
            Some(active_manifest.clone())
        } else if current_link.join("manifest.json").is_file() {
            Some(current_link.join("manifest.json"))
        } else {
            None
        };

        if prior_hook_src.is_some() || prior_bridge_src.is_some() || prior_manifest_src.is_some() {
            let stage = unique_temp_path(&previous_dir, "stage");
            let prep_res = (|| -> io::Result<()> {
                fs::create_dir_all(&stage)?;
                if let Some(src) = &prior_hook_src {
                    fs::copy(src, stage.join(hook_name))?;
                }
                if let Some(src) = &prior_bridge_src {
                    fs::copy(src, stage.join(bridge_name))?;
                }
                if let Some(src) = &prior_manifest_src {
                    fs::copy(src, stage.join("manifest.json"))?;
                }
                Ok(())
            })();
            if let Err(err) = prep_res {
                let _ = fs::remove_dir_all(&stage);
                let _ = fs::remove_dir_all(&rollback_stage);
                return Err(err.into());
            }
            previous_stage = Some(stage);
        }
    }

    let apply_res = (|| -> Result<(), Box<dyn std::error::Error>> {
        let rel_version = Path::new("versions").join(version_id);
        let current_is_symlink = match atomic_replace_symlink(&rel_version, &current_link, true) {
            Ok(()) => true,
            Err(err)
                if is_symlink_privilege_error(&err)
                    && matches!(prior_current, PriorEntryState::Missing) =>
            {
                false
            }
            Err(err) => return Err(err.into()),
        };

        for (exe_name, bin_exe, version_exe) in [
            (hook_name, &bin_hook, &version_hook),
            (bridge_name, &bin_bridge, &version_bridge),
        ] {
            let already_points_through_current = current_is_symlink
                && fs::symlink_metadata(bin_exe)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false)
                && fs::read_link(bin_exe)
                    .map(|target| symlink_points_through_current(&target, &current_link, exe_name))
                    .unwrap_or(false);

            if !already_points_through_current {
                if !bin_dir.exists() {
                    fs::create_dir_all(&bin_dir)?;
                }
                if current_is_symlink {
                    let rel_current_exe = Path::new("..").join("current").join(exe_name);
                    match atomic_replace_symlink(&rel_current_exe, bin_exe, false) {
                        Ok(()) => {}
                        Err(err) if is_symlink_privilege_error(&err) => {
                            safe_copy_or_replace(version_exe, bin_exe)?;
                        }
                        Err(err) => return Err(err.into()),
                    }
                } else {
                    safe_copy_or_replace(version_exe, bin_exe)?;
                }
            }
        }

        if !bin_hook.is_file() || !bin_bridge.is_file() {
            return Err("Activated canonical binaries do not resolve to valid files".into());
        }

        atomic_write_bytes(&active_manifest, manifest_content.as_bytes())?;

        if let Some(stage) = previous_stage.take() {
            if previous_dir.exists() || fs::symlink_metadata(&previous_dir).is_ok() {
                let prev_bak = unique_temp_path(&previous_dir, "bak");
                if let Err(err) = fs::rename(&previous_dir, &prev_bak) {
                    let _ = fs::remove_dir_all(&stage);
                    return Err(err.into());
                }
                if let Err(err) = fs::rename(&stage, &previous_dir) {
                    let _ = fs::rename(&prev_bak, &previous_dir);
                    let _ = fs::remove_dir_all(&stage);
                    return Err(err.into());
                }
                let _ = fs::remove_dir_all(&prev_bak);
            } else if let Err(err) = fs::rename(&stage, &previous_dir) {
                let _ = fs::remove_dir_all(&stage);
                return Err(err.into());
            }
        }

        Ok(())
    })();

    if let Err(err) = apply_res {
        restore_prior_entry(&active_manifest, &prior_active);
        restore_prior_entry(&bin_bridge, &prior_bridge);
        restore_prior_entry(&bin_hook, &prior_hook);
        restore_prior_entry(&current_link, &prior_current);
        if let Some(stage) = previous_stage {
            let _ = fs::remove_dir_all(&stage);
        }
        let _ = fs::remove_dir_all(&rollback_stage);
        return Err(err);
    }

    let _ = fs::remove_dir_all(&rollback_stage);
    Ok(())
}

pub fn activate_version(version_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let local_dir = get_canonical_local_dir();
    let bin_dir = get_canonical_bin_dir();
    switch_active_pointers(&local_dir, version_id, true)?;

    println!(
        "  [active] Version '{}' is now active at: {}",
        version_id,
        bin_dir.display()
    );
    Ok(())
}

pub fn rollback_candidate(update_hooks: bool) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Rolling back agent-otel-bridge to previous installation ===");
    let local_dir = get_canonical_local_dir();
    let previous_dir = get_previous_dir();
    let previous_manifest = previous_dir.join("manifest.json");

    if !previous_manifest.is_file() {
        return Err("No previous installation found for rollback.".into());
    }

    let manifest_content = fs::read_to_string(&previous_manifest)?;
    let manifest: LocalVersionManifest = serde_json::from_str(&manifest_content)?;

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

    let version_dir = get_versions_dir().join(&manifest.version_id);
    if !version_dir.is_dir() {
        let versions_root = get_versions_dir();
        if !versions_root.exists() {
            fs::create_dir_all(&versions_root)?;
        }
        let stage_dir = unique_temp_path(&version_dir, "rollback-stage");
        let stage_res = (|| -> io::Result<()> {
            fs::create_dir_all(&stage_dir)?;
            fs::copy(previous_dir.join(hook_name), stage_dir.join(hook_name))?;
            fs::copy(previous_dir.join(bridge_name), stage_dir.join(bridge_name))?;
            fs::write(stage_dir.join("manifest.json"), &manifest_content)?;
            fs::rename(&stage_dir, &version_dir)?;
            Ok(())
        })();
        if let Err(err) = stage_res {
            let _ = fs::remove_dir_all(&stage_dir);
            return Err(err.into());
        }
    }

    switch_active_pointers(&local_dir, &manifest.version_id, false)?;

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

    let current_link = get_current_link_path();
    if fs::symlink_metadata(&current_link).is_ok() {
        let _ = remove_path_no_follow(&current_link);
    }

    let bin_dir = get_canonical_bin_dir();
    if let Ok(meta) = fs::symlink_metadata(&bin_dir) {
        if meta.file_type().is_symlink() {
            let _ = remove_path_no_follow(&bin_dir);
        } else if meta.is_dir() {
            let _ = fs::remove_dir_all(&bin_dir);
        }
        println!("  [ok] Removed bin directory: {}", bin_dir.display());
    }

    let active_manifest = get_active_manifest_path();
    if fs::symlink_metadata(&active_manifest).is_ok() {
        let _ = remove_path_no_follow(&active_manifest);
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
