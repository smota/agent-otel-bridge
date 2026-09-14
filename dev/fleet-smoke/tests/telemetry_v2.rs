use agent_otel_fleet_smoke::{
    plan::seeded_profile,
    report::RunContext,
    runner::SpanRecord,
    telemetry::{export_context, export_snapshot, BatchOutcome, BatchReceipt},
    telemetry_validation::validate_export,
};

fn span(task_id: String, trace_id: &str) -> SpanRecord {
    SpanRecord {
        span_id: "0123456789abcdef".into(),
        trace_id: trace_id.into(),
        parent_span_id: None,
        dependency_links: vec![],
        task_id,
        planned_platform: "codex".into(),
        status: "OK".into(),
        duration_micros: 4,
        origin: "test".into(),
        phase: "case".into(),
        expected_fault: None,
        observed_fault: None,
        start_unix_nanos: 1,
        end_unix_nanos: 5,
    }
}

#[test]
fn context_export_contains_plan_identity_and_case_expectations() {
    let plan = seeded_profile("baseline", 1).unwrap();
    let context = RunContext::new(
        plan.clone(),
        1,
        "baseline",
        "synthetic",
        "run-1",
        "0123456789abcdef0123456789abcdef",
        "0123456789abcdef",
    );
    let payload = export_context(
        &context,
        &[span(plan.tasks[0].id.clone(), &context.trace_id)],
    );
    let attrs = payload["resourceSpans"][0]["resource"]["attributes"]
        .as_array()
        .unwrap();
    assert!(attrs.iter().any(|a| a["key"] == "agent.smoke.plan_hash"));
    let case_attrs = payload["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["attributes"]
        .as_array()
        .unwrap();
    assert!(case_attrs
        .iter()
        .any(|a| a["key"] == "agent.smoke.expected_fault" && a["value"]["stringValue"] == "none"));
    assert!(case_attrs.iter().any(|a| a["key"] == "agent.smoke.model"));
}

#[test]
fn long_fault_values_are_bounded_and_marked() {
    let plan = seeded_profile("baseline", 1).unwrap();
    let context = RunContext::new(
        plan.clone(),
        1,
        "baseline",
        "synthetic",
        "run-1",
        "t",
        "0123456789abcdef",
    );
    let mut value = span(plan.tasks[0].id.clone(), "0123456789abcdef0123456789abcdef");
    value.observed_fault = Some("x".repeat(5000));
    let payload = export_context(&context, &[value]);
    let attrs = payload["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["attributes"]
        .as_array()
        .unwrap();
    let fault = attrs
        .iter()
        .find(|a| a["key"] == "agent.smoke.observed_fault")
        .unwrap()["value"]["stringValue"]
        .as_str()
        .unwrap();
    assert!(fault.len() <= 2048);
    assert!(fault.ends_with("…[truncated]"));
}

#[test]
fn unknown_batch_receipt_keeps_network_uncertainty() {
    let spans = vec![span("case".into(), "0123456789abcdef0123456789abcdef")];
    let receipt = BatchReceipt::unknown("b", "r", "t", 1, &spans, 10, 20, "connection reset");
    assert_eq!(receipt.outcome, BatchOutcome::Unknown);
    assert_eq!(receipt.span_ids, vec!["0123456789abcdef"]);
}

#[test]
fn injected_expected_fault_passes_only_with_error_status() {
    let mut plan = seeded_profile("baseline", 1).unwrap();
    plan.tasks[0].expected_fault = Some("injected".into());
    let context = RunContext::new(
        plan.clone(),
        1,
        "baseline",
        "synthetic",
        "run-1",
        "0123456789abcdef0123456789abcdef",
        "0123456789abcdef",
    );
    let mut value = span(plan.tasks[0].id.clone(), &context.trace_id);
    value.expected_fault = Some("injected".into());
    value.observed_fault = Some("injected".into());
    value.status = "ERROR".into();
    let payload = export_context(&context, &[value]);
    let attrs = payload["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["attributes"]
        .as_array()
        .unwrap();
    assert!(attrs
        .iter()
        .any(|a| a["key"] == "agent.smoke.evaluation" && a["value"]["stringValue"] == "pass"));
}

#[test]
fn unset_is_inconclusive_and_observed_model_is_absent() {
    let plan = seeded_profile("baseline", 1).unwrap();
    let context = RunContext::new(
        plan.clone(),
        1,
        "baseline",
        "synthetic",
        "run-1",
        "0123456789abcdef0123456789abcdef",
        "0123456789abcdef",
    );
    let mut value = span(plan.tasks[0].id.clone(), &context.trace_id);
    value.status = "UNSET".into();
    let payload = export_context(&context, &[value]);
    let attrs = payload["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["attributes"]
        .as_array()
        .unwrap();
    assert!(attrs.iter().any(
        |a| a["key"] == "agent.smoke.evaluation" && a["value"]["stringValue"] == "inconclusive"
    ));
    assert!(!attrs
        .iter()
        .any(|a| a["key"].as_str().unwrap_or("").contains("model.observed")));
}

#[test]
fn root_only_batch_uses_full_snapshot_counts_and_preserves_identity() {
    let plan = seeded_profile("baseline", 1).unwrap();
    let context = RunContext::new(
        plan.clone(),
        1,
        "baseline",
        "synthetic",
        "run-1",
        "0123456789abcdef0123456789abcdef",
        "0123456789abcdef",
    );
    let mut all = Vec::new();
    for task in &plan.tasks {
        all.push(span(task.id.clone(), &context.trace_id));
    }
    all[0].span_id = context.root_span_id.clone();
    let progressive = export_context(&context, &[all[0].clone()]);
    let terminal = export_snapshot(&context, &[all[0].clone()], &all);
    assert_eq!(
        progressive["resourceSpans"][0]["resource"],
        terminal["resourceSpans"][0]["resource"]
    );
    let attrs = terminal["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["attributes"]
        .as_array()
        .unwrap();
    assert!(attrs.iter().any(|a| a["key"] == "agent.smoke.cases.started"
        && a["value"]["stringValue"]
            .as_str()
            .and_then(|value| value.parse::<usize>().ok())
            == Some(plan.tasks.len())));
}

#[test]
fn validation_rejects_missing_plan_bound_attributes() {
    let plan = seeded_profile("baseline", 1).unwrap();
    let context = RunContext::new(
        plan.clone(),
        1,
        "baseline",
        "synthetic",
        "run-1",
        "0123456789abcdef0123456789abcdef",
        "0123456789abcdef",
    );
    let value = span(plan.tasks[0].id.clone(), &context.trace_id);
    let mut payload = export_context(&context, std::slice::from_ref(&value));
    let attrs = payload["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["attributes"]
        .as_array_mut()
        .unwrap();
    attrs.retain(|a| {
        a["key"] != "agent.smoke.model.planned" && a["key"] != "agent.smoke.requirement_ref"
    });
    let failures = validate_export(&context, &[value], &payload);
    assert!(failures.iter().any(|f| f.contains("model.planned")));
    assert!(failures.iter().any(|f| f.contains("requirement_ref")));
}
