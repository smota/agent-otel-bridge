/*
 * Copyright agent-otel-bridge contributors
 * SPDX-License-Identifier: Apache-2.0
 */

//! Explicit Linux HOME transactions. Journals contain private configuration
//! bytes and are never printed. Station-wide activation serialization remains
//! the operator's responsibility; every native HOME writer shares this lock.
use super::{plan_home_hooks, HomeHookChange};
use clap::Args;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Args, Debug)]
pub struct HomeHookOptions {
    /// Explicit native harness HOME (ordinary user or isolated seat)
    #[arg(long)]
    pub home: PathBuf,
    /// Native reader: claude, codex, agy, gemini-cli, grok, pi (repeatable)
    #[arg(long = "client", required = true, value_delimiter = ',')]
    pub clients: Vec<String>,
    /// Absolute installed native agent-hook executable
    #[arg(long)]
    pub binary: PathBuf,
    /// Persist this socket in every hook; defaults to AGENT_OTEL_SOCKET
    #[arg(long)]
    pub socket: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    home: PathBuf,
    binary: PathBuf,
    socket: Option<PathBuf>,
    clients: Vec<String>,
    status: String,
    #[serde(default)]
    rollback_allow_before: bool,
    files: Vec<RecordedFile>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordedFile {
    path: PathBuf,
    before: Option<Vec<u8>>,
    after: Vec<u8>,
    mode: Option<u32>,
    #[serde(default)]
    restore_progress: RestoreProgress,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum RestoreProgress {
    #[default]
    Pending,
    Restoring,
    Restored,
}

const STATE_RELATIVE: &str = ".local/state/agent-otel-bridge/hooks";
const TARGETS: &[&str] = &[
    ".claude/settings.json",
    ".codex/hooks.json",
    ".gemini/config/hooks.json",
    ".gemini/settings.json",
    ".grok/hooks/agent-otel.json",
    ".pi/agent/extensions/agent-otel-bridge.ts",
];

fn uid() -> Result<u32, String> {
    fs::metadata("/proc/self")
        .map(|m| m.uid())
        .map_err(|_| "Cannot determine process owner".into())
}

fn absolute(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::Prefix(_)))
        || path.starts_with("/mnt")
    {
        return Err(
            "Requires an absolute native Linux path outside /mnt, without parent traversal".into(),
        );
    }
    Ok(())
}

/// Check every existing component before any state is created. Configuration
/// ancestors inside HOME must be owned and not writable by another account.
fn inspect_path(path: &Path, home: &Path) -> Result<(), String> {
    absolute(path)?;
    let owner = uid()?;
    let mut prefix = PathBuf::new();
    for component in path.components() {
        prefix.push(component.as_os_str());
        match fs::symlink_metadata(&prefix) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err("Symlink configuration paths are refused".into());
                }
                if prefix.starts_with(home)
                    && (metadata.uid() != owner || metadata.mode() & 0o022 != 0)
                {
                    return Err(
                        "HOME configuration paths must be owned and not writable by other accounts"
                            .into(),
                    );
                }
                if prefix != path && !metadata.is_dir() {
                    return Err("Configuration ancestor is not a directory".into());
                }
                if prefix == path
                    && !metadata.is_dir()
                    && (!metadata.is_file() || metadata.nlink() != 1)
                {
                    return Err("Configuration target must be an ordinary unlinked file".into());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => return Err("Cannot inspect configuration path".into()),
        }
    }
    Ok(())
}

fn check_home(home: &Path) -> Result<(), String> {
    inspect_path(home, home)?;
    let metadata = fs::metadata(home).map_err(|_| "Explicit HOME must already exist")?;
    if !metadata.is_dir() {
        return Err("Explicit HOME must be a directory".into());
    }
    Ok(())
}

fn read_current(path: &Path, home: &Path) -> Result<Option<Vec<u8>>, String> {
    inspect_path(path, home)?;
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("Cannot read configuration target".into()),
    }
}

fn prepare(options: &HomeHookOptions) -> Result<(Vec<HomeHookChange>, Option<PathBuf>), String> {
    check_home(&options.home)?;
    absolute(&options.binary)?;
    if options.binary.file_name().and_then(|name| name.to_str()) != Some("agent-hook") {
        return Err("Native hook executable must retain the owned agent-hook filename".into());
    }
    absolute(
        &options
            .binary
            .canonicalize()
            .map_err(|_| "Native hook executable is unavailable")?,
    )?;
    let metadata =
        fs::metadata(&options.binary).map_err(|_| "Native hook executable is unavailable")?;
    if !metadata.is_file() || metadata.mode() & 0o111 == 0 {
        return Err("Hook executable is not an executable file".into());
    }
    let socket = options
        .socket
        .clone()
        .or_else(|| std::env::var_os("AGENT_OTEL_SOCKET").map(PathBuf::from));
    if let Some(socket) = &socket {
        absolute(socket)?;
    }
    for client in &options.clients {
        let target = match client.as_str() {
            "claude" => TARGETS[0],
            "codex" => TARGETS[1],
            "agy" | "antigravity" => TARGETS[2],
            "gemini-cli" => TARGETS[3],
            "grok" => TARGETS[4],
            "pi" => TARGETS[5],
            _ => return Err("Unsupported native hook reader".into()),
        };
        inspect_path(&options.home.join(target), &options.home)?;
    }
    let clients = options
        .clients
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let changes = plan_home_hooks(&options.home, &clients, &options.binary, socket.as_deref())
        .map_err(|_| "Native hook planning refused invalid or unowned configuration")?;
    Ok((changes, socket))
}

pub fn plan(options: &HomeHookOptions) -> Result<(), String> {
    let (changes, _) = prepare(options)?;
    let changed = changes
        .iter()
        .filter(|change| change.before.as_ref() != Some(&change.after))
        .count();
    println!(
        "Hook plan: {} target(s), {changed} changed, {} unchanged",
        changes.len(),
        changes.len() - changed
    );
    for change in changes {
        println!("  {}", change.path.display());
    }
    println!("No files changed. Native harness trust and hook review policies still apply.");
    Ok(())
}

fn private_dirs(path: &Path, home: &Path) -> Result<(), String> {
    inspect_path(path, home)?;
    let mut prefix = home.to_path_buf();
    for part in path
        .strip_prefix(home)
        .map_err(|_| "State path must remain inside HOME")?
        .components()
    {
        prefix.push(part);
        if !prefix.exists() {
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&prefix)
                .map_err(|_| "Cannot create private transaction directory")?;
        }
        inspect_path(&prefix, home)?;
    }
    Ok(())
}

fn profile_lock(home: &Path) -> Result<File, String> {
    let state = home.join(STATE_RELATIVE);
    private_dirs(&state, home)?;
    if fs::metadata(&state)
        .map_err(|_| "Cannot inspect private state")?
        .mode()
        & 0o077
        != 0
    {
        return Err("Hook transaction state must be private (0700)".into());
    }
    let path = state.join("mutation.lock");
    inspect_path(&path, home)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(0x20000)
        .open(path)
        .map_err(|_| "Cannot open private HOME mutation lock")?;
    if file
        .metadata()
        .map_err(|_| "Cannot inspect HOME lock")?
        .mode()
        & 0o077
        != 0
    {
        return Err("HOME mutation lock must be private (0600)".into());
    }
    file.try_lock()
        .map_err(|_| "Another hook transaction owns this HOME")?;
    Ok(file)
}

fn atomic_write(path: &Path, bytes: &[u8], mode: u32, home: &Path) -> Result<(), String> {
    inspect_path(path, home)?;
    let parent = path.parent().ok_or("Missing configuration directory")?;
    private_dirs(parent, home)?;
    let temporary = parent.join(format!(
        ".agent-otel-write-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "Clock unavailable")?
            .as_nanos()
    ));
    let operation = (|| -> Result<(), String> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(0x20000)
            .open(&temporary)
            .map_err(|_| "Cannot stage private configuration")?;
        file.write_all(bytes)
            .map_err(|_| "Cannot write staged configuration")?;
        file.set_permissions(fs::Permissions::from_mode(mode))
            .map_err(|_| "Cannot preserve configuration permissions")?;
        file.sync_all()
            .map_err(|_| "Cannot sync staged configuration")?;
        inspect_path(path, home)?;
        fs::rename(&temporary, path).map_err(|_| "Cannot atomically replace configuration")?;
        File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| "Cannot sync configuration directory")?;
        Ok(())
    })();
    if operation.is_err() {
        let _ = fs::remove_file(temporary);
    }
    operation
}

fn save_journal(path: &Path, journal: &Journal) -> Result<(), String> {
    let bytes = serde_json::to_vec(journal).map_err(|_| "Cannot encode private hook journal")?;
    atomic_write(path, &bytes, 0o600, &journal.home)
}

pub fn apply(options: &HomeHookOptions) -> Result<(), String> {
    let journal = apply_inner(options, None)?;
    match journal {
        Some(path) => println!(
            "Hook changes applied. Private rollback journal: {}",
            path.display()
        ),
        None => println!("Hook configuration is unchanged; no transaction created."),
    }
    println!("Native harness trust and hook review policies still apply.");
    Ok(())
}

fn apply_inner(
    options: &HomeHookOptions,
    fail_after: Option<usize>,
) -> Result<Option<PathBuf>, String> {
    // Preview validates everything before creating the private lock/state.
    let _ = prepare(options)?;
    let _lock = profile_lock(&options.home)?;
    let (changes, socket) = prepare(options)?;
    let files = changes
        .into_iter()
        .filter(|change| change.before.as_ref() != Some(&change.after))
        .map(|change| {
            let mode = fs::metadata(&change.path)
                .ok()
                .map(|metadata| metadata.mode() & 0o777);
            RecordedFile {
                path: change.path,
                before: change.before,
                after: change.after,
                mode,
                restore_progress: RestoreProgress::Pending,
            }
        })
        .collect::<Vec<_>>();
    if files.is_empty() {
        return Ok(None);
    }
    let state = options.home.join(STATE_RELATIVE);
    let transaction = state.join(format!(
        "tx-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "Clock unavailable")?
            .as_nanos(),
        std::process::id()
    ));
    private_dirs(&transaction, &options.home)?;
    let path = transaction.join("journal.json");
    let mut journal = Journal {
        version: 1,
        home: options.home.clone(),
        binary: options.binary.clone(),
        socket,
        clients: options.clients.clone(),
        status: "prepared".into(),
        rollback_allow_before: false,
        files,
    };
    save_journal(&path, &journal)?;
    let mut applied = 0;
    let mut attempted = 0;
    let result = (|| {
        for file in &journal.files {
            if fail_after == Some(applied) {
                return Err("Injected transaction write failure".into());
            }
            if !matches_recorded_state(file, &journal.home, true)? {
                return Err("Configuration changed after planning; apply refused".into());
            }
            attempted += 1;
            atomic_write(
                &file.path,
                &file.after,
                file.mode.unwrap_or(0o600),
                &journal.home,
            )?;
            applied += 1;
        }
        journal.status = "applied".into();
        save_journal(&path, &journal)
    })();
    if let Err(error) = result {
        let restored = restore_files(&mut journal, &path, attempted, true);
        journal.status = if restored.is_ok() {
            "compensated"
        } else {
            "partial"
        }
        .into();
        let journal_saved = save_journal(&path, &journal).is_ok();
        return Err(format!(
            "{error}; compensation {}; journal {} at {}",
            if restored.is_ok() {
                "completed"
            } else {
                "incomplete"
            },
            if journal_saved {
                "saved"
            } else {
                "unavailable"
            },
            path.display()
        ));
    }
    Ok(Some(path))
}

fn matches_recorded_state(file: &RecordedFile, home: &Path, before: bool) -> Result<bool, String> {
    let bytes = read_current(&file.path, home)?;
    let mode = match fs::metadata(&file.path) {
        Ok(metadata) => Some(metadata.mode() & 0o777),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err("Cannot inspect configuration permissions".into()),
    };
    let expected_bytes = if before {
        file.before.as_ref()
    } else {
        Some(&file.after)
    };
    let expected_mode = if before {
        file.mode
    } else {
        Some(file.mode.unwrap_or(0o600))
    };
    Ok(bytes.as_ref() == expected_bytes && mode == expected_mode)
}

fn sync_parent(path: &Path) -> Result<(), String> {
    let mut parent = path.parent().ok_or("Missing configuration directory")?;
    loop {
        match File::open(parent) {
            Ok(directory) => {
                return directory
                    .sync_all()
                    .map_err(|_| "Cannot sync restored configuration directory".into())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // A prepared transaction may never have created this target's
                // parent. Sync the nearest existing ancestor without creating it.
                parent = parent.parent().ok_or("Missing configuration ancestor")?;
            }
            Err(_) => return Err("Cannot open restored configuration directory".into()),
        }
    }
}

fn restore_files(
    journal: &mut Journal,
    journal_path: &Path,
    count: usize,
    allow_before: bool,
) -> Result<(), String> {
    // Preflight bytes AND permissions for every target before any restoration.
    // Durable intent allows recovery if a crash happens between a filesystem
    // update and recording that update as complete in the private journal.
    for file in journal.files.iter().take(count) {
        let before = matches_recorded_state(file, &journal.home, true)?;
        let after = matches_recorded_state(file, &journal.home, false)?;
        let valid = match file.restore_progress {
            RestoreProgress::Restored => before,
            RestoreProgress::Restoring => before || after,
            RestoreProgress::Pending => after || (allow_before && before),
        };
        if !valid {
            return Err(
                "Configuration changed after apply (bytes or permissions); rollback refused".into(),
            );
        }
    }
    journal.rollback_allow_before = allow_before;
    journal.status = "rolling-back".into();
    save_journal(journal_path, journal)?;
    for index in (0..count).rev() {
        if journal.files[index].restore_progress == RestoreProgress::Restored {
            continue;
        }
        let file = &journal.files[index];
        let before = matches_recorded_state(file, &journal.home, true)?;
        if !matches_recorded_state(file, &journal.home, false)?
            && !((allow_before || file.restore_progress == RestoreProgress::Restoring) && before)
        {
            return Err(
                "Configuration changed during rollback; remaining targets preserved".into(),
            );
        }
        let recovered_before =
            before && (allow_before || file.restore_progress == RestoreProgress::Restoring);
        journal.files[index].restore_progress = RestoreProgress::Restoring;
        save_journal(journal_path, journal)?;
        let file = &journal.files[index];
        // Recheck after journaling intent so a mode tightening is never undone.
        let before = matches_recorded_state(file, &journal.home, true)?;
        if !(recovered_before && before) {
            if !matches_recorded_state(file, &journal.home, false)? {
                return Err(
                    "Configuration changed during rollback; remaining targets preserved".into(),
                );
            }
            if let Some(before) = &file.before {
                atomic_write(
                    &file.path,
                    before,
                    file.mode.unwrap_or(0o600),
                    &journal.home,
                )?;
            } else {
                fs::remove_file(&file.path)
                    .map_err(|_| "Cannot remove newly registered hook during rollback")?;
            }
        }
        // This also handles a crash after unlink/rename but before its sync:
        // persist the directory entry before durably marking this file restored.
        sync_parent(&file.path)?;
        journal.files[index].restore_progress = RestoreProgress::Restored;
        save_journal(journal_path, journal)?;
    }
    Ok(())
}

pub fn rollback(path: &Path) -> Result<(), String> {
    absolute(path)?;
    // Read only after checking that the journal itself and every ancestor are
    // regular non-symlink paths. Then validate its declared HOME and target list.
    inspect_path(path, path)?;
    let metadata = fs::metadata(path).map_err(|_| "Private rollback journal unavailable")?;
    if !metadata.is_file() || metadata.uid() != uid()? || metadata.mode() & 0o077 != 0 {
        return Err("Rollback journal must be a private owned file".into());
    }
    let bytes = fs::read(path).map_err(|_| "Cannot read private rollback journal")?;
    let mut journal: Journal =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid private rollback journal")?;
    check_home(&journal.home)?;
    inspect_path(path, &journal.home)?;
    let expected = journal.home.join(STATE_RELATIVE);
    if journal.version != 1
        || path.file_name().and_then(|name| name.to_str()) != Some("journal.json")
        || path.parent().and_then(Path::parent) != Some(expected.as_path())
        || journal.files.iter().any(|file| {
            !TARGETS
                .iter()
                .any(|target| file.path == journal.home.join(target))
        })
    {
        return Err("Rollback journal ownership or target scope is invalid".into());
    }
    let _lock = profile_lock(&journal.home)?;
    // Prevent a concurrent journal replacement between the first read and lock.
    if fs::read(path).map_err(|_| "Cannot recheck private journal")? != bytes {
        return Err("Rollback journal changed; operation refused".into());
    }
    if journal.status == "rolled-back" {
        println!("Hook transaction was already rolled back.");
        return Ok(());
    }
    if ![
        "applied",
        "prepared",
        "partial",
        "rolling-back",
        "rollback-partial",
    ]
    .contains(&journal.status.as_str())
    {
        return Err("Hook transaction is not eligible for rollback".into());
    }
    let count = journal.files.len();
    let allow_before =
        journal.rollback_allow_before || matches!(journal.status.as_str(), "prepared" | "partial");
    if let Err(error) = restore_files(&mut journal, path, count, allow_before) {
        // A preflight conflict has not changed any target; an interrupted
        // restoration retains per-file intent/progress for a later retry.
        if journal.status == "rolling-back" {
            journal.status = "rollback-partial".into();
            if save_journal(path, &journal).is_err() {
                return Err(format!(
                    "{error}; rollback progress journal unavailable at {}",
                    path.display()
                ));
            }
        }
        return Err(error);
    }
    journal.status = "rolled-back".into();
    save_journal(path, &journal)?;
    println!("Hook transaction rolled back: {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn options(label: &str) -> HomeHookOptions {
        let home = std::env::temp_dir().join(format!(
            "bridge-transaction-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::DirBuilder::new().mode(0o700).create(&home).unwrap();
        let binary = home.join("agent-hook");
        fs::copy("/bin/true", &binary).unwrap();
        HomeHookOptions {
            home,
            clients: vec![
                "claude".into(),
                "codex".into(),
                "agy".into(),
                "gemini-cli".into(),
                "grok".into(),
                "pi".into(),
            ],
            binary,
            socket: Some(PathBuf::from("/tmp/owner's socket")),
        }
    }

    #[test]
    fn preview_is_read_only_apply_is_private_and_rollback_restores_exact_bytes() {
        let options = options("roundtrip");
        let original = b"{\"foreign\":\"private test credential\", \"hooks\":{\"PreToolUse\":[{\"hooks\":[{\"type\":\"command\",\"command\":\"rtk hook claude\"}]}]}}\n";
        fs::DirBuilder::new()
            .mode(0o700)
            .create(options.home.join(".claude"))
            .unwrap();
        fs::write(options.home.join(".claude/settings.json"), original).unwrap();
        plan(&options).unwrap();
        assert!(!options.home.join(".local").exists());
        let path = apply_inner(&options, None).unwrap().unwrap();
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        assert_eq!(
            fs::metadata(path.parent().unwrap()).unwrap().mode() & 0o777,
            0o700
        );
        let journal: Journal = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(journal.status, "applied");
        assert_eq!(
            journal.files[0].before.as_deref(),
            Some(original.as_slice())
        );
        assert!(apply_inner(&options, None).unwrap().is_none());
        rollback(&path).unwrap();
        assert_eq!(
            fs::read(options.home.join(".claude/settings.json")).unwrap(),
            original
        );
        assert!(!options.home.join(".codex/hooks.json").exists());
        assert!(!options
            .home
            .join(".pi/agent/extensions/agent-otel-bridge.ts")
            .exists());
        rollback(&path).unwrap();
        let _ = fs::remove_dir_all(options.home);
    }

    #[test]
    fn rollback_refuses_intervening_edits_before_restoring_any_file() {
        let options = options("intervening");
        let path = apply_inner(&options, None).unwrap().unwrap();
        let claude_before = fs::read(options.home.join(".claude/settings.json")).unwrap();
        fs::write(options.home.join(".codex/hooks.json"), "changed externally").unwrap();
        assert!(rollback(&path).unwrap_err().contains("changed after apply"));
        assert_eq!(
            fs::read(options.home.join(".claude/settings.json")).unwrap(),
            claude_before
        );
        assert_eq!(
            fs::read_to_string(options.home.join(".codex/hooks.json")).unwrap(),
            "changed externally"
        );
        let _ = fs::remove_dir_all(options.home);
    }

    #[test]
    fn partial_apply_is_compensated_and_keeps_private_rollback_evidence() {
        let options = options("partial");
        let error = apply_inner(&options, Some(1)).unwrap_err();
        assert!(error.contains("compensation completed"));
        assert!(!options.home.join(".claude/settings.json").exists());
        assert!(!options.home.join(".codex/hooks.json").exists());
        let state = options.home.join(STATE_RELATIVE);
        let tx = fs::read_dir(&state)
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| entry.file_name().to_str().unwrap().starts_with("tx-"))
            .unwrap();
        let journal: Journal =
            serde_json::from_slice(&fs::read(tx.path().join("journal.json")).unwrap()).unwrap();
        assert_eq!(journal.status, "compensated");
        let _ = fs::remove_dir_all(options.home);
    }

    #[test]
    fn lock_contention_symlinks_and_unsafe_paths_are_refused() {
        let mut options = options("unsafe");
        let lock = profile_lock(&options.home).unwrap();
        assert!(apply_inner(&options, None)
            .unwrap_err()
            .contains("Another hook transaction"));
        drop(lock);
        let other = options.home.join("elsewhere");
        fs::DirBuilder::new().mode(0o700).create(&other).unwrap();
        symlink(&other, options.home.join(".codex")).unwrap();
        assert!(plan(&options).unwrap_err().contains("Symlink"));
        assert!(!other.join("hooks.json").exists());
        fs::remove_file(options.home.join(".codex")).unwrap();
        options.home = PathBuf::from("/mnt/c/Users/test");
        assert!(plan(&options).is_err());
        options.home = PathBuf::from("/tmp/../home/test");
        assert!(plan(&options).is_err());
    }

    #[test]
    fn rollback_recovers_durable_intent_after_crash_before_progress_record() {
        let mut options = options("crash-after-unlink");
        options.clients = vec!["claude".into(), "codex".into()];
        let path = apply_inner(&options, None).unwrap().unwrap();
        let mut journal: Journal = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        journal.status = "rolling-back".into();
        journal.files[1].restore_progress = RestoreProgress::Restoring;
        save_journal(&path, &journal).unwrap();
        // Simulate a crash after the filesystem change, before its directory
        // sync and completed progress record. Recovery must sync and finish it.
        fs::remove_file(&journal.files[1].path).unwrap();
        rollback(&path).unwrap();
        let journal: Journal = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(journal.status, "rolled-back");
        assert!(journal
            .files
            .iter()
            .all(|file| file.restore_progress == RestoreProgress::Restored));
        assert!(!options.home.join(".claude/settings.json").exists());
        assert!(!options.home.join(".codex/hooks.json").exists());
        let _ = fs::remove_dir_all(options.home);
    }

    #[test]
    fn prepared_legacy_journal_recovers_without_creating_target_directories() {
        let mut options = options("legacy-prepared");
        options.clients = vec!["claude".into(), "codex".into()];
        let (changes, _) = prepare(&options).unwrap();
        let transaction = options
            .home
            .join(STATE_RELATIVE)
            .join("tx-prepared-fixture");
        private_dirs(&transaction, &options.home).unwrap();
        let path = transaction.join("journal.json");
        // Version-1 journals from before progress tracking have no new fields.
        let legacy = serde_json::json!({"version":1,"home":options.home,"binary":options.binary,"socket":options.socket,"clients":options.clients,"status":"prepared","files":changes.into_iter().map(|change| serde_json::json!({"path":change.path,"before":change.before,"after":change.after,"mode":null})).collect::<Vec<_>>()});
        fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        rollback(&path).unwrap();
        assert!(!options.home.join(".claude").exists());
        assert!(!options.home.join(".codex").exists());
        let _ = fs::remove_dir_all(options.home);
    }

    #[test]
    fn rollback_refuses_permission_tightening_before_restoring_any_file() {
        let mut options = options("tightened-mode");
        options.clients = vec!["claude".into(), "codex".into()];
        fs::DirBuilder::new()
            .mode(0o700)
            .create(options.home.join(".claude"))
            .unwrap();
        let claude = options.home.join(".claude/settings.json");
        fs::write(&claude, b"{}").unwrap();
        fs::set_permissions(&claude, fs::Permissions::from_mode(0o644)).unwrap();
        let path = apply_inner(&options, None).unwrap().unwrap();
        let after = fs::read(&claude).unwrap();
        fs::set_permissions(&claude, fs::Permissions::from_mode(0o600)).unwrap();
        let result = rollback(&path);
        assert!(
            result.is_err(),
            "rollback silently undid a confidentiality tightening"
        );
        assert_eq!(fs::metadata(&claude).unwrap().mode() & 0o777, 0o600);
        assert_eq!(fs::read(&claude).unwrap(), after);
        assert!(options.home.join(".codex/hooks.json").exists());
        let _ = fs::remove_dir_all(options.home);
    }

    #[test]
    fn partial_rollback_can_resume_after_restoring_directory_permissions() {
        const CHILD: &str = "AGENT_OTEL_PERMISSION_FIXTURE_CHILD";
        if std::env::var_os(CHILD).as_deref() != Some(std::ffi::OsStr::new("1")) {
            // Other parallel tests spawn children. A fork can retain this
            // test's flock until exec even after apply drops its descriptor.
            // Isolate the permission fault so lock contention cannot replace it.
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "hooks::home_apply::tests::partial_rollback_can_resume_after_restoring_directory_permissions", "--test-threads=1"])
                .env(CHILD, "1")
                .output()
                .expect("start isolated permission-failure fixture");
            assert!(
                output.status.success(),
                "isolated permission-failure fixture failed: {} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let mut options = options("retry-rollback");
        options.clients = vec!["claude".into(), "codex".into()];
        let claude_dir = options.home.join(".claude");
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&claude_dir)
            .unwrap();
        let claude = claude_dir.join("settings.json");
        fs::write(&claude, b"{}").unwrap();
        let path = apply_inner(&options, None).unwrap().unwrap();
        fs::set_permissions(&claude_dir, fs::Permissions::from_mode(0o500)).unwrap();
        let first = rollback(&path);
        fs::set_permissions(&claude_dir, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            first.unwrap_err(),
            "Cannot stage private configuration",
            "rollback must reach the intended permission fault after restoring Codex"
        );
        assert!(!options.home.join(".codex/hooks.json").exists());
        let journal: Journal = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_ne!(
            journal.status, "applied",
            "partial rollback falsely remained applied"
        );
        rollback(&path).expect("durable partial rollback must be retryable");
        assert_eq!(fs::read(&claude).unwrap(), b"{}");
        let _ = fs::remove_dir_all(options.home);
    }

    #[test]
    fn journal_cannot_restore_an_unrelated_owned_file() {
        let options = options("scope");
        let path = apply_inner(&options, None).unwrap().unwrap();
        let mut journal: Journal = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let unrelated = options.home.join("unrelated");
        fs::write(&unrelated, "preserve").unwrap();
        journal.files[0].path = unrelated.clone();
        save_journal(&path, &journal).unwrap();
        assert!(rollback(&path).unwrap_err().contains("target scope"));
        assert_eq!(fs::read_to_string(unrelated).unwrap(), "preserve");
        let _ = fs::remove_dir_all(options.home);
    }
}
