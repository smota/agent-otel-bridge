use crate::{
    adapters::{spawn_bounded, Platform},
    fixtures::SeededWorkspace,
    operations::perform,
    plan::{seeded_plan, seeded_profile, FleetPlan, PlanScenario, PlanTask},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub type WaveObserver = Arc<dyn Fn(&[SpanRecord]) -> io::Result<()> + Send + Sync>;
pub type EventObserver = Arc<dyn Fn(&str, &RunArtifact) -> io::Result<()> + Send + Sync>;
static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const MAX_CONCURRENT_TASKS: usize = 3;
const MAX_PROGRESS_SPANS: usize = 240;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    Synthetic,
    Live,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanRecord {
    pub span_id: String,
    pub trace_id: String,
    pub parent_span_id: Option<String>,
    pub dependency_links: Vec<String>,
    pub task_id: String,
    pub planned_platform: String,
    pub status: String,
    pub duration_micros: u64,
    pub origin: String,
    pub phase: String,
    pub expected_fault: Option<String>,
    pub observed_fault: Option<String>,
    pub start_unix_nanos: u128,
    pub end_unix_nanos: u128,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunArtifact {
    pub plan: PlanScenario,
    pub seed: u64,
    pub profile: String,
    pub run_id: String,
    pub mode: String,
    pub spans: Vec<SpanRecord>,
    pub artifact_path: PathBuf,
}
#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub artifact: RunArtifact,
    pub state: String,
    pub errors: Vec<String>,
    pub not_executed: Vec<String>,
}

/// Cooperative cancellation shared by an embedding caller and the runner.
#[derive(Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);
impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub struct Runner {
    seed: u64,
    max_tasks: usize,
    retries: u8,
    profile: String,
    native_programs: BTreeMap<String, PathBuf>,
    observer: Option<WaveObserver>,
    event_observer: Option<EventObserver>,
    progress_interval: Duration,
    cancellation: Option<CancellationToken>,
}
impl Runner {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            max_tasks: 32,
            retries: 1,
            profile: "baseline".into(),
            native_programs: BTreeMap::new(),
            observer: None,
            event_observer: None,
            progress_interval: Duration::from_secs(15),
            cancellation: None,
        }
    }
    pub fn with_limits(mut self, max_tasks: usize, retries: u8) -> Self {
        self.max_tasks = max_tasks;
        self.retries = retries;
        self
    }
    pub fn with_profile(mut self, profile: impl Into<String>) -> Self {
        self.profile = profile.into();
        self
    }
    pub fn with_native_program(mut self, platform: impl Into<String>, program: PathBuf) -> Self {
        self.native_programs.insert(platform.into(), program);
        self
    }
    pub fn with_observer(mut self, observer: WaveObserver) -> Self {
        self.observer = Some(observer);
        self
    }
    pub fn with_event_observer(mut self, observer: EventObserver) -> Self {
        self.event_observer = Some(observer);
        self
    }
    /// Test seam for the normal fifteen-second progress cadence.
    pub fn with_progress_interval(mut self, interval: Duration) -> Self {
        self.progress_interval = interval.max(Duration::from_millis(1));
        self
    }
    pub fn with_cancellation_token(mut self, token: CancellationToken) -> Self {
        self.cancellation = Some(token);
        self
    }
    /// Compatibility seam for process signal handlers that already share an atomic flag.
    pub fn with_cancellation(mut self, cancelled: Arc<AtomicBool>) -> Self {
        self.cancellation = Some(CancellationToken(cancelled));
        self
    }
    pub fn plan(&self) -> FleetPlan {
        seeded_plan()
    }
    pub fn retries(&self) -> u8 {
        self.retries
    }
    /// Legacy compatibility: this wrapper writes an artifact and returns an error for a failed outcome.
    pub fn run(&self, mode: RunMode) -> io::Result<RunArtifact> {
        let mut out = self.run_outcome(mode)?;
        if !out.errors.is_empty() {
            Err(io::Error::other(out.errors.join("; ")))
        } else {
            retain_artifact(&mut out.artifact)?;
            Ok(out.artifact)
        }
    }
    /// Runs in memory. Recoverable task, transport observer, and lifecycle observer failures stay in the outcome.
    pub fn run_outcome(&self, mode: RunMode) -> io::Result<RunOutcome> {
        let plan = seeded_profile(&self.profile, self.seed).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "unknown profile; choose baseline, mixed, or long-trace",
            )
        })?;
        if plan.tasks.len() > self.max_tasks {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "task bound exceeded",
            ));
        }
        let mode_name = if mode == RunMode::Live {
            "live"
        } else {
            "synthetic"
        };
        let run_id = next_run_id(mode_name, unix_nanos());
        let trace_id = hash128(&format!("{run_id}:{}", self.seed));
        let root_id = hash64(&format!("{run_id}:lab-root"));
        let wall = unix_nanos();
        let clock = Instant::now();
        let origin = if mode == RunMode::Live {
            "fleet-smoke-lab.live-orchestration"
        } else {
            "fleet-smoke-lab.synthetic-fixture"
        };
        let artifact = Arc::new(Mutex::new(RunArtifact {
            plan: plan.clone(),
            seed: self.seed,
            profile: self.profile.clone(),
            run_id: run_id.clone(),
            mode: mode_name.into(),
            spans: vec![root_span(
                &self.profile,
                &trace_id,
                &root_id,
                origin,
                wall,
                false,
            )],
            artifact_path: PathBuf::new(),
        }));
        let errors = Arc::new(Mutex::new(Vec::new()));
        self.event("run_started", &artifact, &errors);
        let active = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = start_progress_worker(
            Arc::clone(&artifact),
            Arc::clone(&errors),
            self.event_observer.clone(),
            Arc::clone(&active),
            Arc::clone(&stop),
            self.progress_interval,
        );
        let mut remaining: BTreeMap<String, PlanTask> = plan
            .tasks
            .iter()
            .cloned()
            .map(|t| (t.id.clone(), t))
            .collect();
        let mut completed = BTreeMap::new();
        let mut failed = false;
        while !remaining.is_empty() && !failed {
            failed |= !errors.lock().unwrap().is_empty();
            if failed {
                break;
            }
            if self
                .cancellation
                .as_ref()
                .is_some_and(CancellationToken::is_cancelled)
            {
                errors.lock().unwrap().push("run cancelled".into());
                break;
            }
            let ready: Vec<String> = remaining
                .values()
                .filter(|t| t.depends_on.iter().all(|d| completed.contains_key(d)))
                .take(MAX_CONCURRENT_TASKS)
                .map(|t| t.id.clone())
                .collect();
            if ready.is_empty() {
                errors
                    .lock()
                    .unwrap()
                    .push("plan DAG contains cycle or missing dependency".into());
                break;
            }
            let wave: Vec<PlanTask> = ready
                .into_iter()
                .map(|id| remaining.remove(&id).expect("ready task exists"))
                .collect();
            let deps = completed.clone();
            active.store(wave.len(), Ordering::Release);
            let results: Vec<TaskResult> = thread::scope(|scope| {
                let handles: Vec<_> = wave
                    .into_iter()
                    .map(|task| {
                        let deps = deps.clone();
                        let task_run_id = run_id.clone();
                        let task_trace_id = trace_id.clone();
                        let task_root_id = root_id.clone();
                        let panic_task = task.clone();
                        scope.spawn(move || {
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                execute_task(
                                    mode,
                                    task,
                                    deps.clone(),
                                    TaskContext {
                                        run: &task_run_id,
                                        trace: &task_trace_id,
                                        root: &task_root_id,
                                        origin,
                                    },
                                    &self.native_programs,
                                )
                            }))
                            .unwrap_or_else(|_| {
                                TaskResult::panicked(
                                    &panic_task,
                                    deps,
                                    &task_run_id,
                                    &task_trace_id,
                                    &task_root_id,
                                    origin,
                                )
                            })
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| {
                        h.join().unwrap_or_else(|_| {
                            TaskResult::panic_unknown(&trace_id, &root_id, origin)
                        })
                    })
                    .collect()
            });
            active.store(0, Ordering::Release);
            let spans: Vec<SpanRecord> = results.iter().map(|r| r.span.clone()).collect();
            for r in &results {
                completed.insert(r.task_id.clone(), r.span.span_id.clone());
                if let Some(e) = &r.error {
                    errors.lock().unwrap().push(e.clone());
                    failed = true;
                }
            }
            artifact.lock().unwrap().spans.extend(spans.clone());
            if let Some(observer) = &self.observer {
                if let Err(e) = observer(&spans) {
                    errors.lock().unwrap().push(format!("wave observer: {e}"));
                    failed = true;
                }
            }
            self.event("wave_completed", &artifact, &errors);
            failed |= !errors.lock().unwrap().is_empty();
        }
        let mut not_executed: Vec<String> = remaining.into_keys().collect();
        not_executed.sort();
        stop.store(true, Ordering::Release);
        if let Some(worker) = worker {
            worker.thread().unpark();
            let _ = worker.join();
        }
        let mut final_artifact = artifact.lock().unwrap().clone();
        let duration = clock.elapsed().as_micros().max(1) as u64;
        let no_errors = errors.lock().unwrap().is_empty();
        if let Some(root) = final_artifact
            .spans
            .iter_mut()
            .find(|s| s.span_id == root_id)
        {
            root.phase = "root-end".into();
            root.duration_micros = duration;
            root.end_unix_nanos = wall + u128::from(duration) * 1_000;
            root.status = if no_errors { "OK" } else { "ERROR" }.into();
        }
        if let Some(observer) = &self.observer {
            if let Some(root) = final_artifact.spans.iter().find(|s| s.span_id == root_id) {
                if let Err(e) = observer(std::slice::from_ref(root)) {
                    errors.lock().unwrap().push(format!("root observer: {e}"));
                }
            }
        }
        if !errors.lock().unwrap().is_empty() {
            if let Some(root) = final_artifact
                .spans
                .iter_mut()
                .find(|s| s.span_id == root_id)
            {
                root.status = "ERROR".into();
            }
        }
        *artifact.lock().unwrap() = final_artifact.clone();
        self.event("run_finished", &artifact, &errors);
        if !errors.lock().unwrap().is_empty() {
            let mut latest = artifact.lock().unwrap();
            if let Some(root) = latest.spans.iter_mut().find(|span| span.span_id == root_id) {
                root.status = "ERROR".into();
            }
            final_artifact = latest.clone();
        }
        let errors = errors.lock().unwrap().clone();
        let state = if self
            .cancellation
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
        {
            "cancelled"
        } else if errors.is_empty() {
            "completed"
        } else if completed.is_empty() {
            "incomplete"
        } else {
            "failed"
        }
        .into();
        Ok(RunOutcome {
            artifact: final_artifact,
            state,
            errors,
            not_executed,
        })
    }
    fn event(
        &self,
        name: &str,
        artifact: &Arc<Mutex<RunArtifact>>,
        errors: &Arc<Mutex<Vec<String>>>,
    ) {
        if let Some(observer) = &self.event_observer {
            let snapshot = artifact.lock().unwrap().clone();
            if let Err(e) = observer(name, &snapshot) {
                errors
                    .lock()
                    .unwrap()
                    .push(format!("event observer {name}: {e}"));
            }
        }
    }
    pub fn run_repeated(&self, mode: RunMode, repeat: u8) -> io::Result<Vec<RunArtifact>> {
        if !(1..=16).contains(&repeat) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "repeat must be in 1..=16",
            ));
        }
        let mut artifacts = Vec::new();
        for _ in 0..repeat {
            match self.run(mode) {
                Ok(a) => artifacts.push(a),
                Err(e) => {
                    for a in &artifacts {
                        let _ = cleanup_temp_artifact(&a.artifact_path);
                    }
                    return Err(e);
                }
            }
        }
        Ok(artifacts)
    }
}

#[derive(Clone)]
struct TaskResult {
    task_id: String,
    span: SpanRecord,
    error: Option<String>,
}
impl TaskResult {
    fn panic_unknown(trace: &str, root: &str, origin: &str) -> Self {
        let now = unix_nanos();
        Self {
            task_id: "thread-panic".into(),
            span: error_span(
                "thread-panic",
                "lab",
                trace,
                root,
                origin,
                now,
                "task thread panicked",
            ),
            error: Some("fleet task thread panicked".into()),
        }
    }
    fn panicked(
        task: &PlanTask,
        done: BTreeMap<String, String>,
        run: &str,
        trace: &str,
        root: &str,
        origin: &str,
    ) -> Self {
        let now = unix_nanos();
        let mut span = error_span(
            &task.id,
            &task.platform,
            trace,
            root,
            origin,
            now,
            "task thread panicked",
        );
        span.span_id = hash64(&format!("{run}:{}", task.id));
        span.dependency_links = task
            .depends_on
            .iter()
            .filter_map(|dependency| done.get(dependency).cloned())
            .collect();
        Self {
            task_id: task.id.clone(),
            span,
            error: Some(format!("task {} panicked", task.id)),
        }
    }
}
struct TaskContext<'a> {
    run: &'a str,
    trace: &'a str,
    root: &'a str,
    origin: &'a str,
}
fn execute_task(
    mode: RunMode,
    task: PlanTask,
    done: BTreeMap<String, String>,
    context: TaskContext<'_>,
    programs: &BTreeMap<String, PathBuf>,
) -> TaskResult {
    let TaskContext {
        run,
        trace,
        root,
        origin,
    } = context;
    let wall = unix_nanos();
    let started = Instant::now();
    let span_id = hash64(&format!("{run}:{}", task.id));
    let finish = |observed: Option<String>, error: Option<String>| {
        let duration = started.elapsed().as_micros().max(1) as u64;
        TaskResult {
            task_id: task.id.clone(),
            span: SpanRecord {
                span_id: span_id.clone(),
                trace_id: trace.into(),
                parent_span_id: Some(root.into()),
                dependency_links: task
                    .depends_on
                    .iter()
                    .filter_map(|d| done.get(d).cloned())
                    .collect(),
                task_id: task.id.clone(),
                planned_platform: task.platform.clone(),
                status: if observed.is_some() || error.is_some() {
                    "ERROR"
                } else if mode == RunMode::Live {
                    "UNSET"
                } else {
                    "OK"
                }
                .into(),
                duration_micros: duration,
                origin: origin.into(),
                phase: "progress-child".into(),
                expected_fault: task.expected_fault.clone(),
                observed_fault: observed,
                start_unix_nanos: wall,
                end_unix_nanos: wall + u128::from(duration) * 1_000,
            },
            error,
        }
    };
    let result: io::Result<Option<String>> = match mode {
        RunMode::Synthetic => SeededWorkspace::create().and_then(|ws| {
            perform(&ws, &task.operation, task.expected_fault.clone()).map(|(_, seen)| seen)
        }),
        RunMode::Live => {
            let platform = match task.platform.as_str() {
                "codex" => Platform::Codex,
                "grok" => Platform::Grok,
                "antigravity" => Platform::Antigravity,
                _ => {
                    return finish(
                        Some("invalid_platform".into()),
                        Some(format!("unknown task platform {}", task.platform)),
                    )
                }
            };
            let ws = match SeededWorkspace::create() {
                Ok(ws) => ws,
                Err(e) => return finish(Some("workspace".into()), Some(format!("workspace: {e}"))),
            };
            let prompt=format!("Fleet smoke task {}. Execute local fixture operation `{}`. Return one JSON object describing observed result; expected outcome is `{}`. Do not start subagents or use network.",task.id,task.operation,task.expected_outcome);
            platform
                .command_spec_at(
                    &prompt,
                    &child_traceparent(trace, &span_id),
                    ws.root(),
                    programs.get(&task.platform).map(PathBuf::as_path),
                )
                .map_err(|e| io::Error::other(format!("adapter: {e:?}")))
                .and_then(|spec| {
                    spawn_bounded(&spec).map_err(|e| io::Error::other(format!("spawn: {e:?}")))
                })
                .map(|out| {
                    if out.timed_out {
                        Some("timeout".into())
                    } else if out.output_limited {
                        Some("output_limited".into())
                    } else if out.status != 0 {
                        Some("process_failed".into())
                    } else {
                        None
                    }
                })
        }
    };
    match result {
        Ok(observed) => finish(observed, None),
        Err(e) => finish(
            Some("execution_error".into()),
            Some(format!("task {}: {e}", task.id)),
        ),
    }
}
fn start_progress_worker(
    artifact: Arc<Mutex<RunArtifact>>,
    errors: Arc<Mutex<Vec<String>>>,
    observer: Option<EventObserver>,
    active: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    interval: Duration,
) -> Option<thread::JoinHandle<()>> {
    observer.map(|observer| {
        thread::spawn(move || {
            let (run, trace, root, origin) = {
                let snapshot = artifact.lock().unwrap();
                let root = &snapshot.spans[0];
                (
                    snapshot.run_id.clone(),
                    root.trace_id.clone(),
                    root.span_id.clone(),
                    root.origin.clone(),
                )
            };
            let mut seq = 0usize;
            while !stop.load(Ordering::Acquire) {
                thread::park_timeout(interval);
                if stop.load(Ordering::Acquire) {
                    break;
                }
                if active.load(Ordering::Acquire) == 0 {
                    continue;
                }
                if seq >= MAX_PROGRESS_SPANS {
                    break;
                }
                seq += 1;
                let now = unix_nanos();
                let span = SpanRecord {
                    span_id: hash64(&format!("{run}:progress:{seq}")),
                    trace_id: trace.clone(),
                    parent_span_id: Some(root.clone()),
                    dependency_links: Vec::new(),
                    task_id: format!("fleet-progress-{seq}"),
                    planned_platform: "lab".into(),
                    status: "OK".into(),
                    duration_micros: 1,
                    origin: origin.clone(),
                    phase: "control-progress".into(),
                    expected_fault: None,
                    observed_fault: None,
                    start_unix_nanos: now,
                    end_unix_nanos: now + 1_000,
                };
                let snapshot = {
                    let mut a = artifact.lock().unwrap();
                    a.spans.push(span);
                    a.clone()
                };
                if let Err(e) = observer("progress", &snapshot) {
                    errors
                        .lock()
                        .unwrap()
                        .push(format!("event observer progress: {e}"));
                }
            }
        })
    })
}
fn root_span(
    profile: &str,
    trace: &str,
    id: &str,
    origin: &str,
    now: u128,
    ended: bool,
) -> SpanRecord {
    SpanRecord {
        span_id: id.into(),
        trace_id: trace.into(),
        parent_span_id: None,
        dependency_links: Vec::new(),
        task_id: format!("fleet-{profile}-lab-root"),
        planned_platform: "lab".into(),
        status: "UNSET".into(),
        duration_micros: 1,
        origin: origin.into(),
        phase: if ended { "root-end" } else { "root-start" }.into(),
        expected_fault: None,
        observed_fault: None,
        start_unix_nanos: now,
        end_unix_nanos: now + 1,
    }
}
fn error_span(
    task: &str,
    platform: &str,
    trace: &str,
    root: &str,
    origin: &str,
    now: u128,
    fault: &str,
) -> SpanRecord {
    SpanRecord {
        span_id: hash64(&format!("{trace}:{task}:{now}")),
        trace_id: trace.into(),
        parent_span_id: Some(root.into()),
        dependency_links: Vec::new(),
        task_id: task.into(),
        planned_platform: platform.into(),
        status: "ERROR".into(),
        duration_micros: 1,
        origin: origin.into(),
        phase: "progress-child".into(),
        expected_fault: None,
        observed_fault: Some(fault.into()),
        start_unix_nanos: now,
        end_unix_nanos: now + 1_000,
    }
}
/// Explicit retention boundary: `run_outcome` does not write a report by default.
pub fn retain_artifact(artifact: &mut RunArtifact) -> io::Result<()> {
    let path = owned_artifact_path(&artifact.run_id)?;
    artifact.artifact_path = path;
    let written = fs::write(
        &artifact.artifact_path,
        serde_json::to_vec_pretty(&*artifact).map_err(io::Error::other)?,
    );
    if let Err(error) = written {
        // Only this helper-created path can be cleaned here.
        let _ = fs::remove_file(&artifact.artifact_path);
        if let Some(dir) = artifact.artifact_path.parent() {
            let _ = fs::remove_dir(dir);
        }
        artifact.artifact_path = PathBuf::new();
        return Err(error);
    }
    Ok(())
}
pub fn child_traceparent(trace: &str, parent: &str) -> String {
    format!("00-{trace}-{parent}-01")
}
fn owned_artifact_path(run: &str) -> io::Result<PathBuf> {
    if run.is_empty()
        || run.len() > 160
        || !run
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid report run ID",
        ));
    }
    let dir = std::env::temp_dir().join(format!("agent-otel-fleet-report-{run}"));
    fs::create_dir(&dir)?;
    Ok(dir.join("spans.json"))
}
pub fn cleanup_temp_artifact(path: &Path) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "artifact no parent"))?;
    if path.file_name().and_then(|n| n.to_str()) != Some("spans.json")
        || !dir
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("agent-otel-fleet-report-"))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "refusing non-owned cleanup",
        ));
    }
    let resolved = fs::canonicalize(dir)?;
    let tmp = fs::canonicalize(std::env::temp_dir())?;
    if resolved.parent() != Some(tmp.as_path()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "artifact directory is outside the temporary root",
        ));
    }
    fs::remove_file(resolved.join("spans.json"))?;
    fs::remove_dir(resolved)
}
fn hash64(s: &str) -> String {
    let mut h = 0xcbf29ce484222325u64;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}
fn hash128(s: &str) -> String {
    format!("{}{}", hash64(s), hash64(&(s.to_owned() + ":trace")))
}
fn next_run_id(mode: &str, nonce: u128) -> String {
    let seq = RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{mode}-{:x}-{nonce:x}-{seq:x}", std::process::id())
}
fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}
