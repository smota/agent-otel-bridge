//! Acceptance checks derived from the laboratory contract, independent of its runner.
use agent_otel_fleet_smoke::{
    runner::cleanup_temp_artifact, RunArtifact, RunMode, Runner, Verifier,
};

struct Reports(Vec<RunArtifact>);
impl Drop for Reports {
    fn drop(&mut self) {
        for report in &self.0 {
            let _ = cleanup_temp_artifact(&report.artifact_path);
        }
    }
}

#[test]
fn seeded_profiles_preserve_fleet_and_fault_coverage() {
    use agent_otel_fleet_smoke::plan::seeded_profile;
    use std::collections::BTreeSet;
    for seed in [0, 1, 42, u64::MAX] {
        for profile in ["baseline", "mixed", "long-trace"] {
            let plan = seeded_profile(profile, seed).unwrap();
            assert_eq!(plan, seeded_profile(profile, seed).unwrap());
            let platforms: BTreeSet<_> = plan
                .tasks
                .iter()
                .map(|task| task.platform.as_str())
                .collect();
            assert_eq!(platforms, BTreeSet::from(["codex", "grok", "antigravity"]));
            let mut earlier = BTreeSet::new();
            for task in &plan.tasks {
                assert!(task
                    .depends_on
                    .iter()
                    .all(|id| earlier.contains(id.as_str())));
                assert!(earlier.insert(task.id.as_str()));
                assert!(!task.requirement_ref.is_empty());
            }
            if profile != "baseline" {
                let operations: BTreeSet<_> = plan
                    .tasks
                    .iter()
                    .map(|task| task.operation.as_str())
                    .collect();
                assert_eq!(
                    operations,
                    BTreeSet::from([
                        "valid-json",
                        "invalid-json",
                        "function-defect",
                        "mcp-initialize",
                        "http-recovery",
                        "http-timeout"
                    ])
                );
            }
        }
    }
}

#[test]
fn baseline_is_fault_free_and_repeats_the_same_stimuli() {
    let reports = Reports(Runner::new(42).run_repeated(RunMode::Synthetic, 2).unwrap());
    assert_eq!(reports.0.len(), 2);
    let first = &reports.0[0];
    let second = &reports.0[1];
    assert_eq!(
        first.seed, second.seed,
        "repeat must not silently become a seed sweep"
    );
    assert_eq!(first.plan, second.plan);
    assert_ne!(first.run_id, second.run_id);
    assert_ne!(first.spans[0].trace_id, second.spans[0].trace_id);
    assert!(
        first
            .plan
            .tasks
            .iter()
            .all(|task| task.expected_fault.is_none()),
        "baseline excludes injected failures"
    );
}

#[test]
fn completed_root_encloses_all_tasks_and_valid_reports_pass() {
    for profile in ["baseline", "mixed", "long-trace"] {
        let reports = Reports(vec![Runner::new(42)
            .with_profile(profile)
            .run(RunMode::Synthetic)
            .unwrap()]);
        let run = &reports.0[0];
        let roots: Vec<_> = run
            .spans
            .iter()
            .filter(|span| span.parent_span_id.is_none())
            .collect();
        assert_eq!(roots.len(), 1);
        let root = roots[0];
        for task in run
            .spans
            .iter()
            .filter(|span| span.parent_span_id.is_some())
        {
            assert_eq!(task.parent_span_id.as_deref(), Some(root.span_id.as_str()));
            assert!(root.start_unix_nanos <= task.start_unix_nanos);
            assert!(
                root.end_unix_nanos >= task.end_unix_nanos,
                "root ended before {}",
                task.task_id
            );
        }
        let result = Verifier::verify_plan(&run.plan, &run.spans);
        assert!(result.passed, "{}: {:?}", profile, result.failures);
    }
}
