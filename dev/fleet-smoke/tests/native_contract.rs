//! Captured-data fixtures exercise validation; they do not claim a live Bridge capture.
use agent_otel_fleet_smoke::{
    runner::cleanup_temp_artifact, telemetry::ObservedSpan, RunMode, Runner, Verifier,
};

#[test]
fn real_semantic_span_names_match_task_contexts_without_requiring_native_lab_root() {
    let run = Runner::new(42).run(RunMode::Synthetic).unwrap();
    let observed: Vec<_> = run
        .plan
        .tasks
        .iter()
        .enumerate()
        .map(|(index, task)| {
            let expected = run
                .spans
                .iter()
                .find(|span| span.task_id == task.id)
                .unwrap();
            ObservedSpan {
                trace_id: expected.trace_id.clone(),
                span_id: format!("{:016x}", 100 + index),
                parent_span_id: Some(expected.span_id.clone()),
                source: "external.otlp".into(),
                resource_service_name: Some("test-bridge".into()),
                scope_name: Some("agent-otel-bridge".into()),
                name: "execute_tool read_file".into(),
                start_unix_nanos: expected.start_unix_nanos,
                end_unix_nanos: expected.end_unix_nanos,
                origin: None,
                status_code: Some(1),
            }
        })
        .collect();
    let accepted = Verifier::verify_native(&run.plan, &run.spans, &observed);
    let mut missing = observed.clone();
    missing.pop();
    let rejected_missing = Verifier::verify_native(&run.plan, &run.spans, &missing);
    let mut relabeled = observed;
    relabeled[0].origin = Some("fleet-smoke-lab.live-orchestration".into());
    let rejected_lab = Verifier::verify_native(&run.plan, &run.spans, &relabeled);
    cleanup_temp_artifact(&run.artifact_path).unwrap();
    assert!(accepted.passed, "{:?}", accepted.failures);
    assert!(!rejected_missing.passed);
    assert!(!rejected_lab.passed);
}
