//! Independent checks of the emitted contract against the planned cases.
use crate::{
    report::{plan_hash, RunContext},
    runner::SpanRecord,
};
use serde_json::Value;

pub fn validate_export(context: &RunContext, batch: &[SpanRecord], payload: &Value) -> Vec<String> {
    let mut failures = Vec::new();
    let resource = &payload["resourceSpans"][0];
    let attrs = &resource["resource"]["attributes"];
    for (key, expected) in [
        ("agent.smoke.run_id", context.run_id.clone()),
        ("agent.smoke.seed", context.seed.to_string()),
        ("agent.smoke.profile", context.profile.clone()),
        ("agent.smoke.plan_hash", plan_hash(&context.plan)),
        ("agent.smoke.contract_version", "2".into()),
    ] {
        check(attrs, key, &expected, &mut failures);
    }
    let Some(spans) = resource["scopeSpans"][0]["spans"].as_array() else {
        return vec!["R2: missing OTLP spans".into()];
    };
    if spans.len() != batch.len() {
        failures.push("R2: batch coverage mismatch".into());
    }
    for expected in batch {
        let matches: Vec<_> = spans
            .iter()
            .filter(|span| span["spanId"] == expected.span_id)
            .collect();
        if matches.len() != 1 {
            failures.push(format!("R2: span coverage {}", expected.span_id));
            continue;
        }
        let span = matches[0];
        if span["traceId"] != context.trace_id
            || span.get("parentSpanId").and_then(Value::as_str)
                != expected.parent_span_id.as_deref()
        {
            failures.push(format!("R2: span identity {}", expected.span_id));
        }
        if let Some(task) = context
            .plan
            .tasks
            .iter()
            .find(|task| task.id == expected.task_id)
        {
            for (key, value) in [
                ("agent.smoke.task_id", task.id.as_str()),
                ("agent.smoke.operation", task.operation.as_str()),
                ("agent.smoke.model.planned", task.model.as_str()),
                ("agent.smoke.reasoning.planned", task.reasoning.as_str()),
                (
                    "agent.smoke.expected_fault",
                    task.expected_fault.as_deref().unwrap_or("none"),
                ),
                (
                    "agent.smoke.expected_outcome",
                    task.expected_outcome.as_str(),
                ),
                ("agent.smoke.requirement_ref", task.requirement_ref.as_str()),
                (
                    "agent.smoke.implementation_ref",
                    task.implementation_ref.as_str(),
                ),
            ] {
                check(&span["attributes"], key, value, &mut failures);
            }
        }
    }
    failures
}
fn check(attrs: &Value, key: &str, expected: &str, failures: &mut Vec<String>) {
    let values: Vec<_> = attrs
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a["key"] == key)
        .collect();
    if values.len() != 1 || values[0]["value"]["stringValue"] != expected {
        failures.push(format!("R2: missing or divergent {key}"));
    }
}
