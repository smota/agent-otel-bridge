/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_core::context::{HarvestLimits, WorkspaceContext};
use agent_otel_core::model::AgentHookInput;
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const CONTEXT_CACHE_MAX_ENTRIES: usize = 128;
pub const CONTEXT_CACHE_MAX_BYTES: usize = 4 * 1024 * 1024;
pub const CONTEXT_SNAPSHOT_MAX_BYTES: usize = 64 * 1024;
pub const CONTEXT_REFRESH_QUEUE_CAPACITY: usize = 32;
pub const CONTEXT_REFRESH_WORKERS: usize = 2;
pub const CONTEXT_FRESH_TTL: Duration = Duration::from_secs(1);
pub const CONTEXT_STALE_TTL: Duration = Duration::from_secs(5);
pub const CONTEXT_NEGATIVE_TTL: Duration = Duration::from_secs(1);
pub const CONTEXT_WORKER_STUCK_AFTER: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextState {
    Fresh,
    Stale,
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshDisposition {
    NotNeeded,
    Queued,
    AlreadyPending,
    QueueFull,
    CircuitOpen,
    InvalidWorkspace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextLookup {
    pub context: Option<WorkspaceContext>,
    pub state: ContextState,
    pub age: Option<Duration>,
    pub refresh: RefreshDisposition,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContextCacheStats {
    pub queued: u64,
    pub queue_full: u64,
    pub circuit_open: u64,
    pub completed: u64,
    pub failed: u64,
    pub rejected_snapshot_size: u64,
    pub active_workers: usize,
    pub stuck_workers: usize,
    pub queued_keys: usize,
    pub entries: usize,
    pub bytes: usize,
}

struct RefreshJob {
    key: String,
    path: PathBuf,
}

struct CacheEntry {
    snapshot: Option<WorkspaceContext>,
    captured_at: Instant,
    bytes: usize,
    last_used: u64,
}

#[derive(Default)]
struct CacheState {
    entries: HashMap<String, CacheEntry>,
    aliases: HashMap<String, String>,
    pending: HashSet<String>,
    worker_started: [Option<Instant>; CONTEXT_REFRESH_WORKERS],
    bytes: usize,
    use_counter: u64,
}

#[derive(Default)]
struct Counters {
    queued: AtomicU64,
    queue_full: AtomicU64,
    circuit_open: AtomicU64,
    completed: AtomicU64,
    failed: AtomicU64,
    rejected_snapshot_size: AtomicU64,
    queued_keys: AtomicUsize,
}

struct Shared {
    state: Mutex<CacheState>,
    counters: Counters,
}

/// Workspace-scoped cache with a fixed filesystem concurrency boundary.
///
/// Lookups never touch the filesystem. Exactly two dedicated threads perform
/// bounded direct reads; no Tokio blocking task or replacement thread is
/// spawned when a syscall stalls.
pub struct ContextCache {
    shared: Arc<Shared>,
    tx: mpsc::SyncSender<RefreshJob>,
}

impl ContextCache {
    pub fn new() -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(CacheState::default()),
            counters: Counters::default(),
        });
        let (tx, rx) = mpsc::sync_channel(CONTEXT_REFRESH_QUEUE_CAPACITY);
        let rx = Arc::new(Mutex::new(rx));

        for worker_id in 0..CONTEXT_REFRESH_WORKERS {
            let shared_worker = Arc::clone(&shared);
            let rx_worker = Arc::clone(&rx);
            thread::Builder::new()
                .name(format!("agent-otel-context-{worker_id}"))
                .spawn(move || worker_loop(worker_id, shared_worker, rx_worker))
                .expect("failed to create fixed context refresh worker");
        }

        Self { shared, tx }
    }

    /// Returns context for an event and schedules refresh without waiting.
    /// Relative or absent workspace paths never fall back to the daemon cwd.
    pub fn lookup(&self, input: &AgentHookInput, now: Instant) -> ContextLookup {
        let Some((key, path)) = select_workspace(input) else {
            return ContextLookup {
                context: None,
                state: ContextState::Missing,
                age: None,
                refresh: RefreshDisposition::InvalidWorkspace,
            };
        };

        let cached = {
            let mut state = lock_unpoisoned(&self.shared.state);
            state.use_counter = state.use_counter.wrapping_add(1);
            let use_counter = state.use_counter;
            let identity = state
                .aliases
                .get(&key)
                .cloned()
                .unwrap_or_else(|| key.clone());
            state.entries.get_mut(&identity).map(|entry| {
                entry.last_used = use_counter;
                let age = now
                    .checked_duration_since(entry.captured_at)
                    .unwrap_or_default();
                (entry.snapshot.clone(), age)
            })
        };

        match cached {
            Some((Some(context), age)) if age <= CONTEXT_FRESH_TTL => ContextLookup {
                context: Some(context),
                state: ContextState::Fresh,
                age: Some(age),
                refresh: RefreshDisposition::NotNeeded,
            },
            Some((Some(context), age)) if age <= CONTEXT_STALE_TTL => ContextLookup {
                context: Some(context),
                state: ContextState::Stale,
                age: Some(age),
                refresh: self.schedule(key, path, now),
            },
            Some((None, age)) if age <= CONTEXT_NEGATIVE_TTL => ContextLookup {
                context: None,
                state: ContextState::Missing,
                age: None,
                refresh: RefreshDisposition::NotNeeded,
            },
            _ => ContextLookup {
                context: None,
                state: ContextState::Missing,
                age: None,
                refresh: self.schedule(key, path, now),
            },
        }
    }

    pub fn stats(&self, now: Instant) -> ContextCacheStats {
        let state = lock_unpoisoned(&self.shared.state);
        let active_workers = state
            .worker_started
            .iter()
            .filter(|start| start.is_some())
            .count();
        let stuck_workers = state
            .worker_started
            .iter()
            .flatten()
            .filter(|start| now.saturating_duration_since(**start) > CONTEXT_WORKER_STUCK_AFTER)
            .count();
        ContextCacheStats {
            queued: self.shared.counters.queued.load(Ordering::Relaxed),
            queue_full: self.shared.counters.queue_full.load(Ordering::Relaxed),
            circuit_open: self.shared.counters.circuit_open.load(Ordering::Relaxed),
            completed: self.shared.counters.completed.load(Ordering::Relaxed),
            failed: self.shared.counters.failed.load(Ordering::Relaxed),
            rejected_snapshot_size: self
                .shared
                .counters
                .rejected_snapshot_size
                .load(Ordering::Relaxed),
            active_workers,
            stuck_workers,
            queued_keys: self.shared.counters.queued_keys.load(Ordering::Relaxed),
            entries: state.entries.len(),
            bytes: state.bytes,
        }
    }

    fn schedule(&self, key: String, path: PathBuf, now: Instant) -> RefreshDisposition {
        {
            let mut state = lock_unpoisoned(&self.shared.state);
            let stuck = state
                .worker_started
                .iter()
                .flatten()
                .filter(|start| now.saturating_duration_since(**start) > CONTEXT_WORKER_STUCK_AFTER)
                .count();
            if stuck == CONTEXT_REFRESH_WORKERS {
                self.shared
                    .counters
                    .circuit_open
                    .fetch_add(1, Ordering::Relaxed);
                return RefreshDisposition::CircuitOpen;
            }
            if !state.pending.insert(key.clone()) {
                return RefreshDisposition::AlreadyPending;
            }
        }

        match self.tx.try_send(RefreshJob {
            key: key.clone(),
            path,
        }) {
            Ok(()) => {
                self.shared.counters.queued.fetch_add(1, Ordering::Relaxed);
                self.shared
                    .counters
                    .queued_keys
                    .fetch_add(1, Ordering::Relaxed);
                RefreshDisposition::Queued
            }
            Err(_) => {
                lock_unpoisoned(&self.shared.state).pending.remove(&key);
                self.shared
                    .counters
                    .queue_full
                    .fetch_add(1, Ordering::Relaxed);
                RefreshDisposition::QueueFull
            }
        }
    }
}

impl Default for ContextCache {
    fn default() -> Self {
        Self::new()
    }
}

fn worker_loop(worker_id: usize, shared: Arc<Shared>, rx: Arc<Mutex<mpsc::Receiver<RefreshJob>>>) {
    loop {
        let job = {
            let receiver = lock_unpoisoned(&rx);
            receiver.recv()
        };
        let Ok(job) = job else {
            return;
        };

        {
            let mut state = lock_unpoisoned(&shared.state);
            state.worker_started[worker_id] = Some(Instant::now());
        }
        shared.counters.queued_keys.fetch_sub(1, Ordering::Relaxed);

        let result = if job.path.is_dir() {
            let harvest =
                WorkspaceContext::harvest_from_dir_with_limits(&job.path, HarvestLimits::default());
            harvest.completed.then_some(harvest.context)
        } else {
            None
        };
        let captured_at = Instant::now();

        let mut state = lock_unpoisoned(&shared.state);
        state.worker_started[worker_id] = None;
        state.pending.remove(&job.key);

        match result {
            Some(snapshot) => {
                let snapshot_bytes = workspace_context_bytes(&snapshot);
                if snapshot_bytes > CONTEXT_SNAPSHOT_MAX_BYTES {
                    shared
                        .counters
                        .rejected_snapshot_size
                        .fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                insert_snapshot(
                    &mut state,
                    job.key,
                    Some(snapshot),
                    captured_at,
                    snapshot_bytes,
                );
                shared.counters.completed.fetch_add(1, Ordering::Relaxed);
            }
            None => {
                insert_snapshot(&mut state, job.key, None, captured_at, 0);
                shared.counters.failed.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn insert_snapshot(
    state: &mut CacheState,
    lookup_key: String,
    snapshot: Option<WorkspaceContext>,
    captured_at: Instant,
    snapshot_bytes: usize,
) {
    state.use_counter = state.use_counter.wrapping_add(1);
    // Use the requested absolute workspace conservatively. A generic ancestor
    // project marker is not VCS identity and must not collapse sibling repos.
    let identity = lookup_key.clone();
    let alias_bytes = lookup_key.len() + identity.len();
    let entry_bytes = identity.len() + snapshot_bytes;

    if let Some(old_identity) = state.aliases.remove(&lookup_key) {
        state.bytes = state
            .bytes
            .saturating_sub(lookup_key.len().saturating_add(old_identity.len()));
    }
    if lookup_key != identity {
        state.aliases.insert(lookup_key, identity.clone());
        state.bytes = state.bytes.saturating_add(alias_bytes);
    }
    if let Some(old) = state.entries.remove(&identity) {
        state.bytes = state.bytes.saturating_sub(old.bytes);
    }
    state.bytes = state.bytes.saturating_add(entry_bytes);
    state.entries.insert(
        identity,
        CacheEntry {
            snapshot,
            captured_at,
            bytes: entry_bytes,
            last_used: state.use_counter,
        },
    );
    evict_to_limits(state);
}

fn evict_to_limits(state: &mut CacheState) {
    while state.entries.len() > CONTEXT_CACHE_MAX_ENTRIES || state.bytes > CONTEXT_CACHE_MAX_BYTES {
        let Some(oldest) = state
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        if let Some(entry) = state.entries.remove(&oldest) {
            state.bytes = state.bytes.saturating_sub(entry.bytes);
        }
        let removed_alias_bytes = state
            .aliases
            .iter()
            .filter(|(_, identity)| *identity == &oldest)
            .map(|(alias, identity)| alias.len().saturating_add(identity.len()))
            .sum::<usize>();
        state.aliases.retain(|_, identity| identity != &oldest);
        state.bytes = state.bytes.saturating_sub(removed_alias_bytes);
    }
}

fn select_workspace(input: &AgentHookInput) -> Option<(String, PathBuf)> {
    let raw = input.workspace_path.as_deref().or_else(|| {
        input
            .workspace_paths
            .as_deref()?
            .first()
            .map(String::as_str)
    })?;
    let path = Path::new(raw);
    if !path.is_absolute() {
        return None;
    }
    let normalized = normalize_lexical(path);
    Some((normalized.to_string_lossy().to_string(), normalized))
}

fn normalize_lexical(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn workspace_context_bytes(context: &WorkspaceContext) -> usize {
    context.current_dir.len()
        + context.launch_dir.as_ref().map_or(0, String::len)
        + context.project_name.as_ref().map_or(0, String::len)
        + context.project_root.as_ref().map_or(0, String::len)
        + context.project_type.as_ref().map_or(0, String::len)
        + context.vcs_system.as_ref().map_or(0, String::len)
        + context.vcs_repository.as_ref().map_or(0, String::len)
        + context.vcs_branch.as_ref().map_or(0, String::len)
        + context.vcs_commit.as_ref().map_or(0, String::len)
        + std::mem::size_of::<WorkspaceContext>()
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
