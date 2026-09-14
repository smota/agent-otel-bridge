use crate::{plan::PlanScenario, runner::SpanRecord, telemetry::ObservedSpan};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub passed: bool,
    pub native_bridge_propagation: String,
    pub failures: Vec<String>,
}
pub struct Verifier;

impl Verifier {
    /// Legacy structural check. Plan-bound verification should use `verify_plan`.
    pub fn verify(spans: &[SpanRecord]) -> VerificationReport {
        let mut failures = Vec::new();
        if spans.is_empty() {
            failures.push("no spans captured".into());
        }
        let traces: BTreeSet<_> = spans.iter().map(|s| &s.trace_id).collect();
        if traces.len() != 1 {
            failures.push("trace continuity failed: spans have different trace ids".into());
        }
        let ids: BTreeMap<_, _> = spans.iter().map(|s| (s.span_id.as_str(), s)).collect();
        if ids.len() != spans.len() {
            failures.push("duplicate span id".into());
        }
        for span in spans {
            validate_span_identity(span, &mut failures);
            if span.status == "ERROR" && span.expected_fault.is_none() {
                failures.push(format!("ERROR without expected fault for {}", span.task_id));
            }
            if span.expected_fault != span.observed_fault {
                failures.push(format!("fault mismatch for {}", span.task_id));
            }
            if span.expected_fault.is_some() && span.status != "ERROR" {
                failures.push(format!(
                    "expected fault did not produce ERROR for {}",
                    span.task_id
                ));
            }
            if span.expected_fault.is_none() && span.status != "OK" {
                failures.push(format!(
                    "unexpected status for {}: {}",
                    span.task_id, span.status
                ));
            }
            if let Some(parent) = &span.parent_span_id {
                if !ids.contains_key(parent.as_str()) {
                    failures.push(format!("missing parent {} for {}", parent, span.task_id));
                }
            }
        }
        report(
            failures,
            "INCONCLUSIVE: synthetic spans do not prove native bridge propagation",
        )
    }

    pub fn verify_plan(plan: &PlanScenario, spans: &[SpanRecord]) -> VerificationReport {
        let mut report = Self::verify(spans);
        let expected: BTreeMap<_, _> = plan.tasks.iter().map(|t| (t.id.as_str(), t)).collect();
        let lab_roots: Vec<_> = spans
            .iter()
            .filter(|s| {
                s.planned_platform == "lab" && s.phase == "root-end" && s.parent_span_id.is_none()
            })
            .collect();
        if lab_roots.len() != 1 {
            report.failures.push(format!(
                "expected exactly one lab root, found {}",
                lab_roots.len()
            ));
        }
        let lab_root = lab_roots.first().copied();
        let actual: BTreeMap<_, _> = spans
            .iter()
            .filter(|s| {
                s.phase != "control-progress" && !lab_roots.iter().any(|r| std::ptr::eq(*r, *s))
            })
            .map(|s| (s.task_id.as_str(), s))
            .collect();
        if spans
            .iter()
            .filter(|s| {
                s.phase != "control-progress" && !lab_roots.iter().any(|r| std::ptr::eq(*r, *s))
            })
            .count()
            != actual.len()
        {
            report.failures.push("duplicate task id".into());
        }
        if actual.len() != expected.len() {
            report.failures.push(format!(
                "task count mismatch: expected {}, observed {}",
                expected.len(),
                actual.len()
            ));
        }
        for id in expected.keys() {
            if !actual.contains_key(id) {
                report.failures.push(format!("missing task {id}"));
            }
        }
        for id in actual.keys() {
            if !expected.contains_key(id) {
                report.failures.push(format!("unexpected task {id}"));
            }
        }
        for task in &plan.tasks {
            let Some(span) = actual.get(task.id.as_str()) else {
                continue;
            };
            if !valid_hex(&span.span_id, 16) || span.span_id.bytes().all(|b| b == b'0') {
                report
                    .failures
                    .push(format!("invalid span id for {}", task.id));
            }
            if span.end_unix_nanos < span.start_unix_nanos
                || span.end_unix_nanos - span.start_unix_nanos
                    > u128::from(span.duration_micros) * 1_000 + 999
            {
                report
                    .failures
                    .push(format!("duration bounds inconsistent for {}", task.id));
            }
            if let Some(root) = lab_root {
                if span.parent_span_id.as_deref() != Some(root.span_id.as_str()) {
                    report
                        .failures
                        .push(format!("{} is not linked to lab root", task.id));
                }
                if span.start_unix_nanos < root.start_unix_nanos
                    || span.end_unix_nanos > root.end_unix_nanos
                {
                    report
                        .failures
                        .push(format!("{} lies outside lab root", task.id));
                }
            }
            let mut links = span.dependency_links.clone();
            links.sort();
            let mut wanted: Vec<_> = task
                .depends_on
                .iter()
                .filter_map(|dep| actual.get(dep.as_str()).map(|s| s.span_id.clone()))
                .collect();
            wanted.sort();
            if links != wanted {
                report
                    .failures
                    .push(format!("dependency links mismatch for {}", task.id));
            }
            if task.expected_fault != span.expected_fault
                || task.expected_fault != span.observed_fault
            {
                report
                    .failures
                    .push(format!("plan fault mismatch for {}", task.id));
            }
            if task.expected_fault.is_some()
                && !(span.status == "ERROR" && span.observed_fault.is_some())
            {
                report
                    .failures
                    .push(format!("injected fault was not observed for {}", task.id));
            }
            if task.expected_fault.is_none()
                && (span.status != "OK" || span.observed_fault.is_some())
            {
                report
                    .failures
                    .push(format!("unexpected fault status for {}", task.id));
            }
        }
        report.passed = report.failures.is_empty();
        report
    }

    /// Evidence-bound case assertions suitable for a report or terminal telemetry.
    pub fn verify_structure(plan: &PlanScenario, spans: &[SpanRecord]) -> VerificationReport {
        // Normalize only outcomes in a copy so topology is checked independently
        // from an intentionally failing operation or inconclusive live result.
        let mut structural = spans.to_vec();
        for span in &mut structural {
            let fault = plan
                .tasks
                .iter()
                .find(|task| task.id == span.task_id)
                .and_then(|task| task.expected_fault.clone());
            span.status = if fault.is_some() { "ERROR" } else { "OK" }.into();
            span.expected_fault = fault.clone();
            span.observed_fault = fault;
        }
        Self::verify_plan(plan, &structural)
    }

    /// Evidence-bound case assertions suitable for a report or terminal telemetry.
    pub fn assertions(plan: &PlanScenario, spans: &[SpanRecord]) -> Vec<serde_json::Value> {
        plan.tasks.iter().map(|task| {
            let observed = spans.iter().find(|span| span.task_id == task.id);
            let passed = observed.is_some_and(|span| {
                span.expected_fault == task.expected_fault
                    && span.observed_fault == task.expected_fault
                    && span.planned_platform == task.platform
                    && span.status == if task.expected_fault.is_some() { "ERROR" } else { "OK" }
            });
            let insufficient = observed.is_none_or(|span| span.status == "UNSET");
            serde_json::json!({
                "requirement_id": task.id,
                "requirement_ref": task.requirement_ref,
                "implementation_ref": task.implementation_ref,
                "expected": {"fault": task.expected_fault, "outcome": task.expected_outcome, "platform":task.platform},
                "observed": observed.map(|span| serde_json::json!({"fault":span.observed_fault,"status":span.status,"platform":span.planned_platform,"span_id":span.span_id})),
                "result": if passed {"PASS"} else if insufficient {"INCONCLUSIVE"} else {"FAIL"},
                "classification": if passed {"none"} else if insufficient {"insufficient_evidence"} else {"implementation_defect"},
                "evidence_origin": observed.map(|span| span.origin.as_str())
            })
        }).collect()
    }

    pub fn verify_native(
        plan: &PlanScenario,
        expected: &[SpanRecord],
        observed: &[ObservedSpan],
    ) -> VerificationReport {
        let mut failures = Vec::new();
        let expected_by_task: BTreeMap<_, _> = expected
            .iter()
            .filter(|span| plan.tasks.iter().any(|task| task.id == span.task_id))
            .map(|span| (span.task_id.as_str(), span))
            .collect();
        for task in &plan.tasks {
            if !expected_by_task.contains_key(task.id.as_str()) {
                failures.push(format!("missing expected context for {}", task.id));
            }
        }
        if observed.is_empty() {
            failures.push("no independently observed bridge spans".into());
        }
        let mut seen_ids = BTreeSet::new();
        let mut matched = BTreeSet::new();
        for span in observed {
            if !seen_ids.insert(span.span_id.clone()) {
                failures.push(format!("duplicate native span id {}", span.span_id));
            }
            if span.source != "external.otlp"
                || span
                    .origin
                    .as_deref()
                    .is_some_and(|origin| origin.starts_with("fleet-smoke-lab"))
            {
                failures.push(format!(
                    "native span {} is lab-authored or lacks external source",
                    span.name
                ));
            }
            if span.scope_name.as_deref() != Some("agent-otel-bridge") {
                failures.push(format!("native scope mismatch for {}", span.name));
            }
            if span.resource_service_name.as_deref() == Some(crate::telemetry::SERVICE_NAME) {
                failures.push(format!("native service is lab service for {}", span.name));
            }
            if !span.name.starts_with("execute_tool ") && !span.name.starts_with("invoke_agent ") {
                failures.push(format!("unexpected native span name {}", span.name));
            }
            let Some((task_id, expected_span)) =
                expected_by_task.iter().find(|(_, expected_span)| {
                    span.parent_span_id.as_deref() == Some(expected_span.span_id.as_str())
                })
            else {
                failures.push(format!(
                    "native parent does not match an expected task for {}",
                    span.name
                ));
                continue;
            };
            matched.insert(*task_id);
            if span.trace_id != expected_span.trace_id {
                failures.push(format!("native trace mismatch for {}", task_id));
            }
        }
        for task_id in expected_by_task.keys() {
            if !matched.contains(task_id) {
                failures.push(format!("missing native observation for {}", task_id));
            }
        }
        let native = if failures.is_empty() {
            "CAPTURE_ASSERTIONS_PASSED: backend visibility is not verified by an imported file"
        } else {
            "INCONCLUSIVE: imported bridge spans failed capture assertions"
        };
        report(failures, native)
    }

    pub fn verify_native_observation(spans: &[SpanRecord]) -> VerificationReport {
        let mut report = Self::verify(spans);
        report.failures.push("native propagation requires independently observed bridge spans, not lab-authored spans".into());
        report.passed = false;
        report.native_bridge_propagation =
            "INCONCLUSIVE: independent bridge evidence was not supplied".into();
        report
    }
}

fn validate_span_identity(span: &SpanRecord, failures: &mut Vec<String>) {
    if !valid_hex(&span.span_id, 16) || !valid_hex(&span.trace_id, 32) {
        failures.push(format!("invalid span identity for {}", span.task_id));
    }
    if span.duration_micros == 0 {
        failures.push(format!("non-positive duration for {}", span.task_id));
    }
    if span.origin != "fleet-smoke-lab.synthetic-fixture" {
        failures.push(format!("unexpected origin for {}", span.task_id));
    }
}
fn valid_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value.bytes().all(|b| b.is_ascii_hexdigit())
        && value.bytes().any(|b| b != b'0')
}
fn report(failures: Vec<String>, native: &str) -> VerificationReport {
    VerificationReport {
        passed: failures.is_empty(),
        native_bridge_propagation: native.into(),
        failures,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        plan::seeded_profile,
        runner::{RunMode, Runner},
    };

    #[test]
    fn plan_bound_verification_rejects_missing_task() {
        let run = Runner::new(41)
            .with_profile("baseline")
            .run(RunMode::Synthetic)
            .unwrap();
        let mut spans = run.spans.clone();
        spans.pop();
        let report = Verifier::verify_plan(&run.plan, &spans);
        assert!(!report.passed);
        assert!(report
            .failures
            .iter()
            .any(|f| f.contains("task count mismatch") || f.contains("missing task")));
        crate::runner::cleanup_temp_artifact(&run.artifact_path).unwrap();
    }

    #[test]
    fn plan_bound_verification_rejects_missed_injected_fault() {
        let plan = seeded_profile("baseline", 1).unwrap();
        let run = Runner::new(1)
            .with_profile("baseline")
            .run(RunMode::Synthetic)
            .unwrap();
        let mut spans = run.spans.clone();
        spans[0].expected_fault = Some("invalid_json".into());
        spans[0].observed_fault = None;
        let report = Verifier::verify_plan(&plan, &spans);
        assert!(!report.passed);
        assert!(report
            .failures
            .iter()
            .any(|f| f.contains("fault") || f.contains("injected")));
        crate::runner::cleanup_temp_artifact(&run.artifact_path).unwrap();
    }
}
