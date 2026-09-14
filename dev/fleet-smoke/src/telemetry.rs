//! OTLP/HTTP JSON conversion and parsing for independently captured traces.
//!
//! Export uses the runner's observed wall-clock timestamps and measured duration.

use crate::{
    report::RunContext,
    runner::{RunArtifact, SpanRecord},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Read;

pub const SERVICE_NAME: &str = "agent-otel-fleet-smoke";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportReceipt {
    pub http_status: u16,
    pub transport_accepted: bool,
    pub backend_visibility: &'static str,
    pub rejected_spans: u64,
}

/// Result classification for a batch attempt. `Unknown` is used when the
/// transport outcome cannot establish whether the collector received it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BatchOutcome {
    Accepted,
    Rejected,
    Unknown,
}
pub type ReceiptClassification = BatchOutcome;
pub type ExportBatchReceipt = BatchReceipt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchReceipt {
    pub batch_id: String,
    pub run_id: String,
    pub trace_id: String,
    pub sequence: u64,
    pub span_ids: Vec<String>,
    pub span_count: usize,
    pub attempted_at_unix_nanos: u128,
    pub duration_micros: u64,
    pub http_status: Option<u16>,
    pub rejected_spans: u64,
    pub outcome: BatchOutcome,
    pub error: Option<String>,
}

impl BatchReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn accepted(
        batch_id: impl Into<String>,
        run_id: impl Into<String>,
        trace_id: impl Into<String>,
        sequence: u64,
        spans: &[SpanRecord],
        attempted_at_unix_nanos: u128,
        duration_micros: u64,
        receipt: &ExportReceipt,
    ) -> Self {
        Self {
            batch_id: batch_id.into(),
            run_id: run_id.into(),
            trace_id: trace_id.into(),
            sequence,
            span_ids: spans.iter().map(|s| s.span_id.clone()).collect(),
            span_count: spans.len(),
            attempted_at_unix_nanos,
            duration_micros,
            http_status: Some(receipt.http_status),
            rejected_spans: receipt.rejected_spans,
            outcome: if receipt.transport_accepted {
                BatchOutcome::Accepted
            } else {
                BatchOutcome::Rejected
            },
            error: None,
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn unknown(
        batch_id: impl Into<String>,
        run_id: impl Into<String>,
        trace_id: impl Into<String>,
        sequence: u64,
        spans: &[SpanRecord],
        attempted_at_unix_nanos: u128,
        duration_micros: u64,
        error: impl Into<String>,
    ) -> Self {
        Self {
            batch_id: batch_id.into(),
            run_id: run_id.into(),
            trace_id: trace_id.into(),
            sequence,
            span_ids: spans.iter().map(|s| s.span_id.clone()).collect(),
            span_count: spans.len(),
            attempted_at_unix_nanos,
            duration_micros,
            http_status: None,
            rejected_spans: 0,
            outcome: BatchOutcome::Unknown,
            error: Some(error.into()),
        }
    }
}

pub fn export_http(endpoint: &str, payload: &Value) -> Result<ExportReceipt, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.without_url().to_string())?;
    let response = client
        .post(endpoint)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(payload)
        .send()
        .map_err(|e| e.without_url().to_string())?;
    let status = response.status().as_u16();
    let mut bytes = Vec::new();
    response
        .take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 65_536 {
        return Err("OTLP response exceeds 64 KiB".into());
    }
    let body: Value = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&bytes).map_err(|_| "malformed OTLP response JSON")?
    };
    if !body.is_object() {
        return Err("OTLP response must be an object".into());
    }
    let rejected = match body.get("partialSuccess") {
        None | Some(Value::Null) => 0,
        Some(partial) if partial.is_object() => match partial.get("rejectedSpans") {
            None | Some(Value::Null) => 0,
            Some(value) => value
                .as_u64()
                .or_else(|| value.as_str().and_then(|value| value.parse::<u64>().ok()))
                .ok_or("invalid OTLP rejectedSpans count")?,
        },
        Some(_) => return Err("invalid OTLP partialSuccess".into()),
    };
    Ok(ExportReceipt {
        http_status: status,
        transport_accepted: status == 200 && rejected == 0,
        backend_visibility: "NOT_VERIFIED",
        rejected_spans: rejected,
    })
}

/// Build an OTLP JSON trace request. `start_unix_nanos` must come from the
/// caller's clock at observation time; end time is derived from duration.
pub fn export_request(spans: &[SpanRecord]) -> Value {
    let resource = json!({"attributes":[{"key":"service.name","value":{"stringValue":SERVICE_NAME}},{"key":"agent.smoke.origin","value":{"stringValue":"fleet-smoke-lab"}}]});
    let exported = spans.iter().map(|s| {
        let start = s.start_unix_nanos;
        let end = s.end_unix_nanos;
        let mut span = json!({"traceId":s.trace_id,"spanId":s.span_id,"name":s.task_id,"startTimeUnixNano":start.to_string(),"endTimeUnixNano":end.to_string(),"attributes":[{"key":"agent.smoke.origin","value":{"stringValue":s.origin}},{"key":"agent.smoke.platform","value":{"stringValue":s.planned_platform}},{"key":"agent.smoke.phase","value":{"stringValue":s.phase}}],"status":{"code":match s.status.as_str() { "OK" => 1, "ERROR" => 2, _ => 0 }}});
        if let Some(parent) = &s.parent_span_id { span["parentSpanId"] = json!(parent); }
        span["links"] = Value::Array(s.dependency_links.iter().map(|id| json!({"traceId":s.trace_id,"spanId":id})).collect());
        span
    }).collect::<Vec<_>>();
    json!({"resourceSpans":[{"resource":resource,"scopeSpans":[{"scope":{"name":"agent-otel-fleet-smoke"},"spans":exported}]}]})
}

const MAX_TEXT: usize = 2048;
fn bounded(value: &str) -> String {
    if value.len() <= MAX_TEXT {
        return value.to_owned();
    }
    let mut end = MAX_TEXT.saturating_sub("…[truncated]".len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…[truncated]", &value[..end])
}
fn attr(key: &str, value: impl AsRef<str>) -> Value {
    json!({"key": key, "value": {"stringValue": bounded(value.as_ref())}})
}

/// The single mapper used by progressive and terminal exports.
pub fn export_context(context: &RunContext, spans: &[SpanRecord]) -> Value {
    export_snapshot_inner(context, spans, spans, false)
}

pub fn export_snapshot(
    context: &RunContext,
    batch: &[SpanRecord],
    all_spans: &[SpanRecord],
) -> Value {
    export_snapshot_inner(context, batch, all_spans, true)
}

fn export_snapshot_inner(
    context: &RunContext,
    spans: &[SpanRecord],
    all_spans: &[SpanRecord],
    include_counts: bool,
) -> Value {
    let plan_hash = crate::report::plan_hash(&context.plan);
    let mut resource_attrs = vec![
        attr("service.name", SERVICE_NAME),
        attr("agent.smoke.origin", "fleet-smoke-lab"),
        attr("agent.smoke.run_id", &context.run_id),
        attr("agent.smoke.trace_id", &context.trace_id),
        attr("agent.smoke.root_span_id", &context.root_span_id),
        attr("agent.smoke.seed", context.seed.to_string()),
        attr("agent.smoke.profile", &context.profile),
        attr("agent.smoke.scenario_version", "1"),
        attr("agent.smoke.plan_hash", plan_hash),
        attr(
            "agent.smoke.plan_hash_algorithm",
            crate::report::PLAN_HASH_ALGORITHM,
        ),
        attr("agent.smoke.contract_version", "2"),
        attr("service.version", crate::report::LAB_VERSION),
        attr("agent.smoke.mode", &context.mode),
    ];
    let mut spans_out = Vec::with_capacity(spans.len());
    for s in spans {
        let task = context.plan.tasks.iter().find(|t| t.id == s.task_id);
        let expected_fault = task.and_then(|t| t.expected_fault.as_deref());
        let expected_status = if expected_fault.is_some() {
            "ERROR"
        } else {
            "OK"
        };
        let evaluation = if task.is_none() || s.status == "UNSET" {
            "inconclusive"
        } else if s.expected_fault.as_deref() != expected_fault {
            "fail"
        } else if s.status == expected_status && s.observed_fault.as_deref() == expected_fault {
            "pass"
        } else {
            "fail"
        };
        let mut attrs = vec![
            attr("agent.smoke.origin", &s.origin),
            attr("agent.smoke.phase", &s.phase),
            attr("agent.smoke.task_id", &s.task_id),
            attr("agent.smoke.platform", &s.planned_platform),
            attr(
                "agent.smoke.expected_fault",
                expected_fault.unwrap_or("none"),
            ),
            attr(
                "agent.smoke.observed_fault",
                s.observed_fault.as_deref().unwrap_or("none"),
            ),
            attr("agent.smoke.evaluation", evaluation),
        ];
        if s.status == "UNSET" && s.observed_fault.is_none() {
            attrs.retain(|attribute| attribute["key"] != "agent.smoke.observed_fault");
        }
        if let Some(task) = task {
            attrs.extend([
                attr("agent.smoke.operation", &task.operation),
                attr("agent.smoke.model", &task.model),
                attr("agent.smoke.reasoning", &task.reasoning),
                attr("agent.smoke.model.planned", &task.model),
                attr("agent.smoke.reasoning.planned", &task.reasoning),
                attr("agent.smoke.expected_outcome", &task.expected_outcome),
                attr("agent.smoke.requirement_ref", &task.requirement_ref),
                attr("agent.smoke.implementation_ref", &task.implementation_ref),
                attr(
                    "agent.smoke.specification_ref",
                    &context.plan.specification_ref,
                ),
            ]);
        }
        if include_counts && s.span_id == context.root_span_id {
            let planned = context.plan.tasks.len();
            let started = all_spans
                .iter()
                .filter(|span| context.plan.tasks.iter().any(|t| t.id == span.task_id))
                .count();
            attrs.extend([
                attr("agent.smoke.cases.planned", planned.to_string()),
                attr("agent.smoke.cases.started", started.to_string()),
                attr("agent.smoke.cases.ended", started.to_string()),
                attr(
                    "agent.smoke.cases.not_executed",
                    planned.saturating_sub(started).to_string(),
                ),
            ]);
            let assertions = crate::verifier::Verifier::assertions(&context.plan, all_spans);
            let missing: Vec<_> = context
                .plan
                .tasks
                .iter()
                .filter(|task| !all_spans.iter().any(|span| span.task_id == task.id))
                .map(|task| task.id.as_str())
                .collect();
            attrs.extend([
                attr(
                    "agent.smoke.cases.not_executed_ids",
                    serde_json::to_string(&missing).expect("IDs serializable"),
                ),
                attr(
                    "agent.smoke.cases.passed",
                    assertions
                        .iter()
                        .filter(|a| a["result"] == "PASS")
                        .count()
                        .to_string(),
                ),
                attr(
                    "agent.smoke.cases.failed",
                    assertions
                        .iter()
                        .filter(|a| a["result"] == "FAIL")
                        .count()
                        .to_string(),
                ),
                attr(
                    "agent.smoke.cases.inconclusive",
                    assertions
                        .iter()
                        .filter(|a| a["result"] == "INCONCLUSIVE")
                        .count()
                        .to_string(),
                ),
            ]);
        }
        let mut span = json!({"traceId":s.trace_id,"spanId":s.span_id,"name":s.task_id,"startTimeUnixNano":s.start_unix_nanos.to_string(),"endTimeUnixNano":s.end_unix_nanos.to_string(),"attributes":attrs,"status":{"code":match s.status.as_str(){"OK"=>1,"ERROR"=>2,_=>0}}});
        if let Some(parent) = &s.parent_span_id {
            span["parentSpanId"] = json!(parent);
        }
        span["links"] = Value::Array(
            s.dependency_links
                .iter()
                .map(|id| json!({"traceId":s.trace_id,"spanId":id}))
                .collect(),
        );
        spans_out.push(span);
    }
    resource_attrs.shrink_to_fit();
    json!({"resourceSpans":[{"resource":{"attributes":resource_attrs},"scopeSpans":[{"scope":{"name":"agent-otel-fleet-smoke"},"spans":spans_out}]}]})
}

/// Export a complete run with reproducibility metadata at resource scope.
pub fn export_run(run: &RunArtifact) -> Value {
    export_snapshot(&RunContext::from_artifact(run), &run.spans, &run.spans)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObservedSpan {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub source: String,
    pub resource_service_name: Option<String>,
    pub scope_name: Option<String>,
    pub name: String,
    pub start_unix_nanos: u128,
    pub end_unix_nanos: u128,
    pub origin: Option<String>,
    pub status_code: Option<i64>,
}

/// Parse standard OTLP JSON and retain only valid span identity fields.
/// Backend/collector provenance is preserved as `external.otlp`; receiving an
/// HTTP response alone is not treated as backend verification.
pub fn parse_observation(input: &str) -> Result<Vec<ObservedSpan>, String> {
    let root: Value = serde_json::from_str(input).map_err(|e| format!("invalid OTLP JSON: {e}"))?;
    let resources = root
        .get("resourceSpans")
        .and_then(Value::as_array)
        .ok_or("missing resourceSpans")?;
    let mut out = Vec::new();
    for resource in resources {
        let service = resource
            .get("resource")
            .and_then(|r| r.get("attributes"))
            .and_then(Value::as_array)
            .and_then(|attrs| {
                attrs
                    .iter()
                    .find(|a| a.get("key").and_then(Value::as_str) == Some("service.name"))
            })
            .and_then(|a| {
                a.get("value")
                    .and_then(|v| v.get("stringValue"))
                    .and_then(Value::as_str)
            })
            .map(str::to_owned);
        let scopes = resource
            .get("scopeSpans")
            .and_then(Value::as_array)
            .ok_or("missing scopeSpans")?;
        for scope in scopes {
            let scope_name = scope
                .get("scope")
                .and_then(|s| s.get("name"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            for span in scope
                .get("spans")
                .and_then(Value::as_array)
                .ok_or("missing spans")?
            {
                let trace_id = span
                    .get("traceId")
                    .and_then(Value::as_str)
                    .ok_or("missing traceId")?;
                let span_id = span
                    .get("spanId")
                    .and_then(Value::as_str)
                    .ok_or("missing spanId")?;
                if !valid_hex(trace_id, 32) || !valid_hex(span_id, 16) {
                    return Err("invalid OTLP span identity".into());
                }
                let parent = span
                    .get("parentSpanId")
                    .and_then(Value::as_str)
                    .filter(|p| !p.is_empty())
                    .map(str::to_owned);
                if let Some(p) = &parent {
                    if !valid_hex(p, 16) {
                        return Err("invalid OTLP parentSpanId".into());
                    }
                }
                let start = decimal(span, "startTimeUnixNano")?;
                let end = decimal(span, "endTimeUnixNano")?;
                if end < start {
                    return Err("span end precedes start".into());
                }
                let origin = span
                    .get("attributes")
                    .and_then(Value::as_array)
                    .and_then(|attrs| {
                        attrs.iter().find(|a| {
                            a.get("key").and_then(Value::as_str) == Some("agent.smoke.origin")
                        })
                    })
                    .and_then(|a| {
                        a.get("value")
                            .and_then(|v| v.get("stringValue"))
                            .and_then(Value::as_str)
                    })
                    .map(str::to_owned);
                let status_code = span
                    .get("status")
                    .and_then(|s| s.get("code"))
                    .and_then(Value::as_i64);
                out.push(ObservedSpan {
                    trace_id: trace_id.into(),
                    span_id: span_id.into(),
                    parent_span_id: parent,
                    source: "external.otlp".into(),
                    resource_service_name: service.clone(),
                    scope_name: scope_name.clone(),
                    name: span
                        .get("name")
                        .and_then(Value::as_str)
                        .ok_or("missing span name")?
                        .into(),
                    start_unix_nanos: start,
                    end_unix_nanos: end,
                    origin,
                    status_code,
                });
            }
        }
    }
    Ok(out)
}

fn valid_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value.bytes().all(|b| b.is_ascii_hexdigit())
        && value.bytes().any(|b| b != b'0')
}
fn decimal(span: &Value, key: &str) -> Result<u128, String> {
    span.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing {key}"))
        .and_then(|v| v.parse().map_err(|_| format!("invalid {key}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn doc(parent: &str) -> String {
        format!(
            r#"{{"resourceSpans":[{{"resource":{{"attributes":[{{"key":"service.name","value":{{"stringValue":"agent-otel-fleet-smoke"}}}}]}},"scopeSpans":[{{"scope":{{"name":"lab"}},"spans":[{{"traceId":"0123456789abcdef0123456789abcdef","spanId":"0123456789abcdef","parentSpanId":"{parent}","name":"task","startTimeUnixNano":"10","endTimeUnixNano":"20","attributes":[{{"key":"agent.smoke.origin","value":{{"stringValue":"fleet-smoke-lab.synthetic-fixture"}}}}],"status":{{"code":1}}}}]}}]}}]}}"#
        )
    }
    #[test]
    fn preserves_scope_and_lab_provenance() {
        let o = parse_observation(&doc("1111111111111111")).unwrap();
        assert_eq!(o[0].scope_name.as_deref(), Some("lab"));
        assert_eq!(
            o[0].origin.as_deref(),
            Some("fleet-smoke-lab.synthetic-fixture")
        );
        assert_eq!(o[0].start_unix_nanos, 10);
    }
    #[test]
    fn rejects_bad_parent_and_missing_id() {
        assert!(parse_observation(&doc("bad")).is_err());
        let missing = doc("").replace("\"traceId\":\"0123456789abcdef0123456789abcdef\",", "");
        assert!(parse_observation(&missing).is_err());
    }
    #[test]
    fn rejects_malformed_envelope() {
        assert!(parse_observation("{}").is_err());
        assert!(parse_observation("{\"resourceSpans\":[null]}").is_err());
    }
}
