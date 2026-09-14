use agent_otel_core::model::AgentHookInput;
use agent_otel_daemon::context_cache::{
    ContextCache, ContextState, RefreshDisposition, CONTEXT_CACHE_MAX_BYTES,
    CONTEXT_CACHE_MAX_ENTRIES, CONTEXT_REFRESH_WORKERS,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "agent-otel-cache-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("create isolated test directory");
    path
}

fn create_workspace(root: &Path, name: &str, branch: &str) {
    fs::create_dir_all(root.join(".git/refs/heads")).expect("create git metadata");
    fs::write(
        root.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.0.0\"\n"),
    )
    .expect("write Cargo marker");
    fs::write(
        root.join(".git/HEAD"),
        format!("ref: refs/heads/{branch}\n"),
    )
    .expect("write HEAD");
    fs::write(
        root.join(".git/refs/heads").join(branch),
        "1111111111111111111111111111111111111111\n",
    )
    .expect("write branch ref");
    fs::write(
        root.join(".git/config"),
        "[remote \"origin\"]\nurl = https://token@example.test/owner/safe-repo.git\n",
    )
    .expect("write config");
}

fn create_git_only_workspace(root: &Path, branch: &str) {
    fs::create_dir_all(root.join(".git/refs/heads")).expect("create git metadata");
    fs::write(
        root.join(".git/HEAD"),
        format!("ref: refs/heads/{branch}\n"),
    )
    .expect("write HEAD");
    fs::write(
        root.join(".git/refs/heads").join(branch),
        "1111111111111111111111111111111111111111\n",
    )
    .expect("write branch ref");
}

fn input_for(path: &Path) -> AgentHookInput {
    AgentHookInput {
        workspace_path: Some(path.to_string_lossy().to_string()),
        ..Default::default()
    }
}

fn wait_for_completions(cache: &ContextCache, minimum: u64) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        let stats = cache.stats(Instant::now());
        if stats.completed + stats.failed >= minimum {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("context refresh did not finish before test deadline");
}

#[test]
fn miss_is_non_blocking_then_becomes_workspace_scoped_fresh_context() {
    let parent = temporary_directory("workspace-scope");
    let first = parent.join("first");
    let second = parent.join("second");
    create_workspace(&first, "first-project", "first-branch");
    create_workspace(&second, "second-project", "second-branch");
    let cache = ContextCache::new();

    let first_miss = cache.lookup(&input_for(&first), Instant::now());
    let second_miss = cache.lookup(&input_for(&second), Instant::now());
    assert_eq!(first_miss.state, ContextState::Missing);
    assert_eq!(first_miss.refresh, RefreshDisposition::Queued);
    assert_eq!(second_miss.refresh, RefreshDisposition::Queued);

    wait_for_completions(&cache, 2);
    let first_hit = cache.lookup(&input_for(&first), Instant::now());
    let second_hit = cache.lookup(&input_for(&second), Instant::now());
    assert_eq!(first_hit.state, ContextState::Fresh);
    assert_eq!(second_hit.state, ContextState::Fresh);
    assert_eq!(
        first_hit
            .context
            .as_ref()
            .and_then(|ctx| ctx.vcs_branch.as_deref()),
        Some("first-branch")
    );
    assert_eq!(
        second_hit
            .context
            .as_ref()
            .and_then(|ctx| ctx.vcs_branch.as_deref()),
        Some("second-branch")
    );
    assert_eq!(
        first_hit
            .context
            .as_ref()
            .and_then(|ctx| ctx.vcs_repository.as_deref()),
        Some("safe-repo")
    );

    fs::remove_dir_all(parent).expect("remove isolated test directory");
}

#[test]
fn sibling_git_workspaces_under_shared_project_marker_keep_distinct_snapshots() {
    let parent = temporary_directory("shared-ancestor-marker");
    fs::write(parent.join("Cargo.toml"), "[workspace]\nmembers = []\n")
        .expect("write shared Cargo marker");
    let first = parent.join("first");
    let second = parent.join("second");
    create_git_only_workspace(&first, "first-branch");
    create_git_only_workspace(&second, "second-branch");
    let cache = ContextCache::new();

    assert_eq!(
        cache.lookup(&input_for(&first), Instant::now()).refresh,
        RefreshDisposition::Queued
    );
    assert_eq!(
        cache.lookup(&input_for(&second), Instant::now()).refresh,
        RefreshDisposition::Queued
    );
    wait_for_completions(&cache, 2);

    let first_hit = cache.lookup(&input_for(&first), Instant::now());
    let second_hit = cache.lookup(&input_for(&second), Instant::now());
    assert_eq!(first_hit.state, ContextState::Fresh);
    assert_eq!(second_hit.state, ContextState::Fresh);
    assert_eq!(
        first_hit.context.and_then(|ctx| ctx.vcs_branch),
        Some("first-branch".to_string())
    );
    assert_eq!(
        second_hit.context.and_then(|ctx| ctx.vcs_branch),
        Some("second-branch".to_string())
    );

    fs::remove_dir_all(parent).expect("remove isolated test directory");
}

#[test]
fn stale_snapshot_is_served_while_branch_refresh_runs() {
    let root = temporary_directory("stale-refresh");
    create_workspace(&root, "branch-project", "before");
    let cache = ContextCache::new();
    let input = input_for(&root);
    let started = Instant::now();
    assert_eq!(
        cache.lookup(&input, started).refresh,
        RefreshDisposition::Queued
    );
    wait_for_completions(&cache, 1);

    fs::write(root.join(".git/HEAD"), "ref: refs/heads/after\n").expect("switch HEAD");
    fs::write(
        root.join(".git/refs/heads/after"),
        "2222222222222222222222222222222222222222\n",
    )
    .expect("write new ref");
    let stale = cache.lookup(&input, Instant::now() + Duration::from_secs(2));
    assert_eq!(stale.state, ContextState::Stale);
    assert_eq!(
        stale
            .context
            .as_ref()
            .and_then(|ctx| ctx.vcs_branch.as_deref()),
        Some("before")
    );
    assert_eq!(stale.refresh, RefreshDisposition::Queued);

    wait_for_completions(&cache, 2);
    let refreshed = cache.lookup(&input, Instant::now());
    assert_eq!(refreshed.state, ContextState::Fresh);
    assert_eq!(
        refreshed
            .context
            .as_ref()
            .and_then(|ctx| ctx.vcs_branch.as_deref()),
        Some("after")
    );

    fs::remove_dir_all(root).expect("remove isolated test directory");
}

#[test]
fn relative_or_absent_workspace_never_uses_daemon_cwd() {
    let cache = ContextCache::new();
    for input in [
        AgentHookInput::default(),
        AgentHookInput {
            workspace_path: Some("relative/workspace".to_string()),
            ..Default::default()
        },
    ] {
        let lookup = cache.lookup(&input, Instant::now());
        assert_eq!(lookup.state, ContextState::Missing);
        assert_eq!(lookup.context, None);
        assert_eq!(lookup.refresh, RefreshDisposition::InvalidWorkspace);
    }
    assert_eq!(cache.stats(Instant::now()).queued, 0);
}

#[test]
fn cache_and_worker_counts_remain_within_fixed_limits() {
    let parent = temporary_directory("fixed-limits");
    let cache = ContextCache::new();
    let total = CONTEXT_CACHE_MAX_ENTRIES + 12;
    for index in 0..total {
        let workspace = parent.join(format!("workspace-{index}"));
        create_workspace(&workspace, &format!("project-{index}"), "main");
        let _ = cache.lookup(&input_for(&workspace), Instant::now());
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let stats = cache.stats(Instant::now());
        assert!(stats.active_workers <= CONTEXT_REFRESH_WORKERS);
        assert!(stats.entries <= CONTEXT_CACHE_MAX_ENTRIES);
        assert!(stats.bytes <= CONTEXT_CACHE_MAX_BYTES);
        if stats.completed + stats.failed + stats.queue_full >= total as u64 {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    let stats = cache.stats(Instant::now());
    assert!(stats.active_workers <= CONTEXT_REFRESH_WORKERS);
    assert!(stats.entries <= CONTEXT_CACHE_MAX_ENTRIES);
    assert!(stats.bytes <= CONTEXT_CACHE_MAX_BYTES);
    assert_eq!(stats.queued_keys, 0);

    fs::remove_dir_all(parent).expect("remove isolated test directory");
}
