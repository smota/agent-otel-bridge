//! Bounded, independent file oracle for the first live baseline.
use crate::plan::PlanTask;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::Path};

pub const MAX_EVIDENCE_BYTES: usize = 8 * 1024;
pub const MAX_FIXTURE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct LiveEvidence {
    pub task_id: String,
    pub operation: String,
    pub result: Value,
    pub fixture_sha256: String,
    /// Native agent-tool proof is not established by this file oracle.
    pub agent_tool_proof: &'static str,
}

/// Ask the agent to read the fixture and write independently retained evidence.
/// The expected event value is deliberately not supplied in the prompt.
pub fn prompt_for(task: &PlanTask) -> String {
    if task.operation != "valid-json" {
        return format!(
            "Task {} uses unsupported live evidence operation `{}`; do not claim success.",
            task.id, task.operation
        );
    }
    format!("For task {} execute the local valid-json fixture. Read event.json, parse and validate it, then write fleet-result.json containing task_id `{}`, operation `valid-json`, the parsed event object as result, and the SHA-256 digest of the exact event.json bytes as fixture_sha256. Do not use network or subagents. The file is the evidence; do not return a narrative result.", task.id, task.id)
}

/// Validate evidence against trusted bytes captured before the agent ran.
pub fn verify_file(
    task: &PlanTask,
    workspace: &Path,
    expected_fixture_bytes: &[u8],
) -> Result<LiveEvidence, String> {
    if task.operation != "valid-json" {
        return Err(format!(
            "unsupported baseline operation: {}",
            task.operation
        ));
    }
    if expected_fixture_bytes.len() > MAX_FIXTURE_BYTES {
        return Err("trusted fixture exceeds the bounded limit".into());
    }
    let root = workspace
        .canonicalize()
        .map_err(|e| format!("workspace: {e}"))?;
    let fixture_path = contained_path(&root, "event.json")?;
    let evidence_path = contained_path(&root, "fleet-result.json")?;
    let metadata =
        fs::metadata(&evidence_path).map_err(|_| "fleet-result.json is missing".to_owned())?;
    if metadata.len() == 0 || metadata.len() > MAX_EVIDENCE_BYTES as u64 {
        return Err("evidence is empty or exceeds the bounded limit".into());
    }
    let evidence_bytes =
        read_bounded(&evidence_path, MAX_EVIDENCE_BYTES).map_err(|e| format!("evidence: {e}"))?;
    let evidence: Value = serde_json::from_slice(&evidence_bytes)
        .map_err(|_| "malformed evidence JSON".to_owned())?;
    let task_id = evidence
        .get("task_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "evidence task_id is missing".to_owned())?;
    let operation = evidence
        .get("operation")
        .and_then(Value::as_str)
        .ok_or_else(|| "evidence operation is missing".to_owned())?;
    if task_id != task.id || operation != task.operation {
        return Err("evidence task identity does not match plan".into());
    }
    let result = evidence
        .get("result")
        .ok_or_else(|| "evidence result is missing".to_owned())?
        .clone();
    let digest = evidence
        .get("fixture_sha256")
        .and_then(Value::as_str)
        .ok_or_else(|| "concrete fixture digest is missing".to_owned())?;
    let expected_digest = sha256(expected_fixture_bytes);
    if digest != expected_digest {
        return Err("fixture digest does not match trusted bytes".into());
    }
    let actual_bytes = read_bounded(&fixture_path, expected_fixture_bytes.len())
        .map_err(|e| format!("fixture: {e}"))?;
    if actual_bytes != expected_fixture_bytes {
        return Err("event.json changed after the trusted snapshot".into());
    }
    let parsed: Value = serde_json::from_slice(expected_fixture_bytes)
        .map_err(|_| "trusted event.json is malformed".to_owned())?;
    if parsed.get("event").and_then(Value::as_str).is_none()
        || parsed.get("step").and_then(Value::as_u64).is_none()
    {
        return Err("event.json lacks event/step".into());
    }
    if result != parsed {
        return Err("result does not match validated event.json".into());
    }
    Ok(LiveEvidence {
        task_id: task.id.clone(),
        operation: task.operation.clone(),
        result,
        fixture_sha256: expected_digest,
        agent_tool_proof: "unknown",
    })
}

fn contained_path(root: &Path, name: &str) -> Result<std::path::PathBuf, String> {
    let canonical = root
        .join(name)
        .canonicalize()
        .map_err(|_| format!("{name} is missing"))?;
    if canonical.starts_with(root) {
        Ok(canonical)
    } else {
        Err(format!("{name} escapes workspace"))
    }
}

fn read_bounded(path: &Path, limit: usize) -> std::io::Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    let mut bytes = Vec::with_capacity(limit.min(8192));
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file exceeds the bounded limit",
        ));
    }
    Ok(bytes)
}
fn sha256(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}
