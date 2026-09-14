//! Versioned local run reports and navigation helpers.
//!
//! This module deliberately has no I/O.  The runner owns retention and the
//! CLI owns transport; report construction remains deterministic and useful
//! when either of those surfaces fails.

use crate::{plan::PlanScenario, runner::RunArtifact};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

pub const SCHEMA_VERSION: u8 = 2;
pub const LAB_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const PLAN_HASH_ALGORITHM: &str = "sha256-canonical-json-v1";
pub const SEARCH_MARGIN_SECONDS: u64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunContext {
    pub run_id: String,
    pub trace_id: String,
    pub root_span_id: String,
    pub seed: u64,
    pub profile: String,
    pub mode: String,
    pub plan: PlanScenario,
}

impl RunContext {
    pub fn new(
        plan: PlanScenario,
        seed: u64,
        profile: impl Into<String>,
        mode: impl Into<String>,
        run_id: impl Into<String>,
        trace_id: impl Into<String>,
        root_span_id: impl Into<String>,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            trace_id: trace_id.into(),
            root_span_id: root_span_id.into(),
            seed,
            profile: profile.into(),
            mode: mode.into(),
            plan,
        }
    }

    pub fn from_artifact(run: &RunArtifact) -> Self {
        let root = run.spans.iter().find(|span| span.parent_span_id.is_none());
        Self::new(
            run.plan.clone(),
            run.seed,
            run.profile.clone(),
            run.mode.clone(),
            run.run_id.clone(),
            root.map(|span| span.trace_id.clone()).unwrap_or_default(),
            root.map(|span| span.span_id.clone()).unwrap_or_default(),
        )
    }
}

/// Hash the plan after recursively sorting JSON object keys.
pub fn plan_hash(plan: &PlanScenario) -> String {
    let value = serde_json::to_value(plan).expect("PlanScenario is serializable");
    let canonical = canonical_json(&value);
    let digest = Sha256::digest(canonical.as_bytes());
    format!("{digest:x}")
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => serde_json::to_string(v).expect("string is serializable"),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(values) => {
            let ordered: BTreeMap<_, _> = values.iter().collect();
            format!(
                "{{{}}}",
                ordered
                    .into_iter()
                    .map(|(key, value)| format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap(),
                        canonical_json(value)
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn rfc3339(nanos: u128) -> Option<String> {
    let nanos = i128::try_from(nanos).ok()?;
    OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .ok()?
        .format(&Rfc3339)
        .ok()
}

/// Produce the v2 summary retained alongside the legacy run/verifier/export fields.
pub fn summarize(run: &RunArtifact, repetition: usize, total: usize) -> Value {
    let context = RunContext::from_artifact(run);
    let root = run
        .spans
        .iter()
        .find(|span| span.span_id == context.root_span_id);
    let task_ids: std::collections::BTreeSet<_> =
        run.plan.tasks.iter().map(|t| t.id.as_str()).collect();
    let started_ids: std::collections::BTreeSet<_> = run
        .spans
        .iter()
        .filter(|span| task_ids.contains(span.task_id.as_str()))
        .map(|span| span.task_id.as_str())
        .collect();
    let ended = started_ids.len();
    let planned = task_ids.len();
    let start = run.spans.iter().map(|s| s.start_unix_nanos).min();
    let end = run.spans.iter().map(|s| s.end_unix_nanos).max();
    let terminal = root.is_some_and(|span| span.phase == "root-end");
    let state = if ended == planned && terminal {
        "completed"
    } else {
        "incomplete"
    };
    json!({
        "schema_version": SCHEMA_VERSION,
        "run_id": context.run_id,
        "trace_id": context.trace_id,
        "root_span_id": context.root_span_id,
        "repetition": repetition,
        "total": total,
        "reproduction": {
            "seed": context.seed,
            "profile": context.profile,
            "scenario_version": "1",
            "plan_hash": plan_hash(&context.plan),
            "plan_hash_algorithm": PLAN_HASH_ALGORITHM,
            "laboratory_version": LAB_VERSION,
            "source_revision": Value::Null,
        },
        "state": state,
        "progress_limit_reached": run.spans.iter().filter(|span|span.phase == "control-progress").count() >= 240,
        "task_counts": {
            "planned": planned,
            "started": started_ids.len(),
            "ended": ended,
            "not_executed": planned.saturating_sub(started_ids.len()),
            "control_spans": run.spans.len().saturating_sub(started_ids.len()),
        },
        "time": {
            "start_utc": start.and_then(rfc3339),
            "end_utc": if terminal { end.and_then(rfc3339) } else { None },
            "start_unix_nanos": start,
            "end_unix_nanos": if terminal { end } else { None },
            "duration_monotonic_micros": root.map(|s| s.duration_micros),
            "search_window_margin_seconds": SEARCH_MARGIN_SECONDS,
            "search_window": {
                "start_unix_ms": start.map(|value| value / 1_000_000).map(|value| value.saturating_sub(u128::from(SEARCH_MARGIN_SECONDS) * 1_000)),
                "end_unix_ms": end.map(|value| value / 1_000_000).map(|value| value.saturating_add(u128::from(SEARCH_MARGIN_SECONDS) * 1_000)),
            },
        },
        "evaluation": {
            "cases": Value::Null,
            "trace_structure": Value::Null,
            "native_propagation": "inconclusive",
            "transport": Value::Null,
            "backend_visibility": "not_checked",
        },
        "navigation": {
            "service": "agent-otel-fleet-smoke",
            "trace_id": context.trace_id,
            "links": [],
            "artifact_retention": "not_created",
            "artifact_path": if run.artifact_path.as_os_str().is_empty() { Value::Null } else { json!(run.artifact_path) },
        }
    })
}

/// Serialize one report item.  The legacy fields remain available for callers
/// that already consume the CLI array format.
pub fn serialize_envelope(run: &RunArtifact, repetition: usize, total: usize) -> Value {
    let mut item = serde_json::to_value(run).expect("RunArtifact is serializable");
    let object = item
        .as_object_mut()
        .expect("RunArtifact serializes as object");
    object.insert("schema_version".into(), json!(SCHEMA_VERSION));
    object.insert("summary".into(), summarize(run, repetition, total));
    object.insert("verification".into(), Value::Null);
    object.insert("export".into(), json!([]));
    item
}

pub fn serialize(run: &RunArtifact, repetition: usize, total: usize) -> Value {
    serialize_envelope(run, repetition, total)
}

/// Read a raw v1 artifact or a v2 report item/envelope.
pub fn read_artifact(raw: &str) -> Result<RunArtifact, String> {
    let value: Value =
        serde_json::from_str(raw).map_err(|e| format!("invalid report JSON: {e}"))?;
    let item = if let Some(items) = value.as_array() {
        if items.len() != 1 {
            return Err("report array must contain exactly one item".into());
        }
        &items[0]
    } else {
        &value
    };
    let version = match item.get("schema_version") {
        None => 1,
        Some(Value::Number(number)) => {
            number.as_u64().ok_or("schema_version must be an integer")?
        }
        Some(_) => return Err("schema_version must be an integer".into()),
    };
    if version != 1 && version != 2 {
        return Err(format!("unknown schema_version: {version}"));
    }
    let run_value = if version == 2 {
        item.get("run").unwrap_or(item)
    } else {
        item
    };
    let run: RunArtifact = serde_json::from_value(run_value.clone())
        .map_err(|e| format!("invalid v{version} artifact: {e}"))?;
    if version == 2 {
        {
            let summary = item.get("summary").ok_or("v2 report requires summary")?;
            let context = RunContext::from_artifact(&run);
            if summary.get("run_id").and_then(Value::as_str) != Some(run.run_id.as_str())
                || summary.get("trace_id").and_then(Value::as_str)
                    != Some(context.trace_id.as_str())
                || summary.get("root_span_id").and_then(Value::as_str)
                    != Some(context.root_span_id.as_str())
                || summary
                    .pointer("/reproduction/plan_hash")
                    .and_then(Value::as_str)
                    != Some(plan_hash(&run.plan).as_str())
            {
                return Err("summary does not match artifact identity or plan hash".into());
            }
        }
    }
    Ok(run)
}

pub fn trace_url(template: &str, run: &RunArtifact) -> Result<String, String> {
    let context = RunContext::from_artifact(run);
    let start = run
        .spans
        .iter()
        .map(|s| s.start_unix_nanos)
        .min()
        .unwrap_or_default()
        / 1_000_000;
    let end = run
        .spans
        .iter()
        .map(|s| s.end_unix_nanos)
        .max()
        .unwrap_or_default()
        / 1_000_000;
    let values = [
        ("trace_id", context.trace_id),
        ("start_unix_ms", start.to_string()),
        ("end_unix_ms", end.to_string()),
    ];
    let mut output = template.to_owned();
    let mut cursor = 0;
    while let Some(open) = output[cursor..].find('{') {
        let open = cursor + open;
        let close = output[open..]
            .find('}')
            .ok_or("unterminated URL placeholder")?
            + open;
        let key = &output[open + 1..close];
        let (_, value) = values
            .iter()
            .find(|(name, _)| *name == key)
            .ok_or_else(|| format!("unknown URL placeholder: {{{key}}}"))?;
        let encoded = percent_encode(value);
        output.replace_range(open..=close, &encoded);
        cursor = open + encoded.len();
    }
    if output.contains('}') {
        return Err("invalid trace URL template".into());
    }
    let parsed = reqwest::Url::parse(&output).map_err(|e| format!("invalid trace URL: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("trace URL must use http or https".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("trace URL must not contain credentials".into());
    }
    for (key, _) in parsed.query_pairs() {
        if matches!(
            key.to_ascii_lowercase().as_str(),
            "token" | "key" | "password" | "secret" | "signature"
        ) {
            return Err("trace URL contains a prohibited secret query key".into());
        }
    }
    Ok(parsed.to_string())
}

fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .flat_map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                vec![b as char]
            } else {
                format!("%{b:02X}").chars().collect()
            }
        })
        .collect()
}
