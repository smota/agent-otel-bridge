use agent_otel_fleet_smoke::{runner::retain_artifact, RunMode, Runner};
use std::{
    fs, io,
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::Duration,
};

#[test]
fn lifecycle_failure_finishes_current_wave_and_marks_later_tasks_unexecuted() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&events);
    let outcome = Runner::new(7)
        .with_profile("baseline")
        .with_event_observer(Arc::new(move |event, artifact| {
            seen.lock()
                .unwrap()
                .push((event.to_owned(), artifact.spans.len()));
            if event == "wave_completed" {
                Err(io::Error::other("collector unavailable"))
            } else {
                Ok(())
            }
        }))
        .run_outcome(RunMode::Synthetic)
        .unwrap();

    assert_eq!(outcome.state, "failed");
    assert!(outcome
        .errors
        .iter()
        .any(|error| error.contains("collector unavailable")));
    assert!(!outcome.not_executed.is_empty());
    assert!(outcome
        .artifact
        .spans
        .iter()
        .any(|span| span.phase == "progress-child"));
    assert_eq!(
        outcome
            .artifact
            .spans
            .iter()
            .filter(|span| span.phase == "root-end")
            .count(),
        1
    );
    assert!(outcome.artifact.artifact_path.as_os_str().is_empty());
    assert_eq!(events.lock().unwrap().last().unwrap().0, "run_finished");
}

#[test]
fn outcome_is_memory_only_until_explicitly_retained() {
    let mut outcome = Runner::new(2).run_outcome(RunMode::Synthetic).unwrap();
    assert_eq!(outcome.state, "completed");
    assert!(outcome.artifact.artifact_path.as_os_str().is_empty());
    retain_artifact(&mut outcome.artifact).unwrap();
    assert!(outcome.artifact.artifact_path.is_file());
    let stored = fs::read_to_string(&outcome.artifact.artifact_path).unwrap();
    assert!(stored.contains(&outcome.artifact.run_id));
    agent_otel_fleet_smoke::runner::cleanup_temp_artifact(&outcome.artifact.artifact_path).unwrap();
}

#[test]
fn progress_control_records_are_distinct_from_task_records() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let records = Arc::clone(&seen);
    let outcome = Runner::new(3)
        .with_profile("long-trace")
        .with_progress_interval(Duration::from_millis(1))
        .with_event_observer(Arc::new(move |event, artifact| {
            records
                .lock()
                .unwrap()
                .push((event.to_owned(), artifact.spans.clone()));
            Ok(())
        }))
        .run_outcome(RunMode::Synthetic)
        .unwrap();
    let progress: Vec<_> = outcome
        .artifact
        .spans
        .iter()
        .filter(|span| span.phase == "control-progress")
        .collect();
    assert!(
        !progress.is_empty(),
        "long trace should emit progress with the injected clock interval"
    );
    assert!(progress.len() <= 240);
    for span in progress {
        assert!(span.task_id.starts_with("fleet-progress-"));
        assert_eq!(span.planned_platform, "lab");
    }
    assert_eq!(
        outcome
            .artifact
            .spans
            .iter()
            .filter(|span| span.phase == "root-end")
            .count(),
        1
    );
    assert_eq!(
        outcome
            .artifact
            .spans
            .iter()
            .filter(|span| span.phase == "root-start")
            .count(),
        0
    );
}

#[test]
fn same_seed_concurrent_runs_reserve_distinct_trace_identities() {
    let (first, second) = std::thread::scope(|scope| {
        let first = scope.spawn(|| Runner::new(9).run_outcome(RunMode::Synthetic).unwrap());
        let second = scope.spawn(|| Runner::new(9).run_outcome(RunMode::Synthetic).unwrap());
        (first.join().unwrap(), second.join().unwrap())
    });
    assert_ne!(first.artifact.run_id, second.artifact.run_id);
    assert_ne!(
        first.artifact.spans[0].trace_id,
        second.artifact.spans[0].trace_id
    );
}

#[test]
fn finalized_root_encloses_tasks_and_keeps_dependency_links() {
    let outcome = Runner::new(42)
        .with_profile("mixed")
        .run_outcome(RunMode::Synthetic)
        .unwrap();
    let root = outcome
        .artifact
        .spans
        .iter()
        .find(|span| span.phase == "root-end")
        .unwrap();
    let tasks: std::collections::BTreeMap<_, _> = outcome
        .artifact
        .spans
        .iter()
        .filter(|span| span.phase == "progress-child")
        .map(|span| (span.task_id.as_str(), span))
        .collect();
    assert_eq!(tasks.len(), outcome.artifact.plan.tasks.len());
    for task in &outcome.artifact.plan.tasks {
        let span = tasks[task.id.as_str()];
        assert_eq!(span.parent_span_id.as_deref(), Some(root.span_id.as_str()));
        assert!(
            root.start_unix_nanos <= span.start_unix_nanos
                && span.end_unix_nanos <= root.end_unix_nanos
        );
        let expected: Vec<_> = task
            .depends_on
            .iter()
            .map(|dependency| tasks[dependency.as_str()].span_id.clone())
            .collect();
        assert_eq!(span.dependency_links, expected);
    }
}

#[test]
fn legacy_observer_sees_task_waves_then_exactly_one_final_root() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let records = Arc::clone(&observed);
    let artifact = Runner::new(11)
        .with_profile("mixed")
        .with_observer(Arc::new(move |spans| {
            records.lock().unwrap().push(spans.to_vec());
            Ok(())
        }))
        .run(RunMode::Synthetic)
        .unwrap();
    let records = observed.lock().unwrap();
    assert_eq!(records.last().unwrap().len(), 1);
    assert_eq!(records.last().unwrap()[0].phase, "root-end");
    assert_eq!(
        records
            .iter()
            .flatten()
            .filter(|span| span.phase == "root-end")
            .count(),
        1
    );
    assert_eq!(
        records
            .iter()
            .take(records.len() - 1)
            .flatten()
            .filter(|span| span.phase == "progress-child")
            .count(),
        artifact.plan.tasks.len()
    );
    drop(records);
    agent_otel_fleet_smoke::runner::cleanup_temp_artifact(&artifact.artifact_path).unwrap();
}

#[test]
fn cancelled_before_start_has_no_task_span_and_is_terminal() {
    let cancelled = Arc::new(AtomicBool::new(true));
    let outcome = Runner::new(1)
        .with_cancellation(cancelled)
        .run_outcome(RunMode::Synthetic)
        .unwrap();
    assert_eq!(outcome.state, "cancelled");
    assert!(outcome
        .artifact
        .spans
        .iter()
        .all(|span| span.phase != "progress-child"));
    assert_eq!(
        outcome.not_executed.len(),
        outcome.artifact.plan.tasks.len()
    );
    assert_eq!(
        outcome
            .artifact
            .spans
            .iter()
            .filter(|span| span.phase == "root-end")
            .count(),
        1
    );
}

#[test]
fn start_event_failure_stops_before_any_task() {
    let outcome = Runner::new(1)
        .with_event_observer(Arc::new(|event, _| {
            if event == "run_started" {
                Err(io::Error::other("start export failed"))
            } else {
                Ok(())
            }
        }))
        .run_outcome(RunMode::Synthetic)
        .unwrap();
    assert_eq!(outcome.state, "incomplete");
    assert!(outcome
        .artifact
        .spans
        .iter()
        .all(|span| span.phase != "progress-child"));
    assert_eq!(
        outcome.not_executed.len(),
        outcome.artifact.plan.tasks.len()
    );
}

#[test]
fn terminal_observer_failure_marks_local_root_error() {
    let outcome = Runner::new(1)
        .with_observer(Arc::new(|spans| {
            if spans[0].phase == "root-end" {
                Err(io::Error::other("terminal export failed"))
            } else {
                Ok(())
            }
        }))
        .run_outcome(RunMode::Synthetic)
        .unwrap();
    assert_eq!(outcome.state, "failed");
    let root = outcome
        .artifact
        .spans
        .iter()
        .find(|span| span.phase == "root-end")
        .unwrap();
    assert_eq!(root.status, "ERROR");
}

#[test]
fn missing_live_executable_preserves_started_task_identity() {
    let absent = std::env::temp_dir().join(format!("fleet-smoke-absent-{}", std::process::id()));
    let outcome = Runner::new(5)
        .with_native_program("codex", absent.clone())
        .with_native_program("grok", absent.clone())
        .with_native_program("antigravity", absent)
        .run_outcome(RunMode::Live)
        .unwrap();
    assert_eq!(outcome.state, "failed");
    assert!(outcome
        .artifact
        .spans
        .iter()
        .any(|span| span.phase == "progress-child"
            && span.status == "ERROR"
            && span.task_id.starts_with("fleet-baseline-")));
    assert!(!outcome.not_executed.is_empty());
}
