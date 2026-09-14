use agent_otel_fleet_smoke::{
    fixtures::SeededWorkspace,
    live_evidence::{prompt_for, verify_file},
    plan::PlanTask,
};
use serde_json::json;
use sha2::{Digest, Sha256};

fn task(operation: &str) -> PlanTask {
    PlanTask {
        id: "fleet-baseline-codex".into(),
        platform: "codex".into(),
        model: "gpt-5.6-luna".into(),
        reasoning: "low".into(),
        depends_on: vec![],
        operation: operation.into(),
        requirement_ref: "fixture".into(),
        implementation_ref: "fixture".into(),
        expected_fault: None,
        expected_outcome: "success".into(),
    }
}
fn digest(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

#[test]
fn valid_file_evidence_is_independent_and_native_proof_stays_unknown() {
    let workspace = SeededWorkspace::create().unwrap();
    let trusted = std::fs::read(workspace.root().join("event.json")).unwrap();
    std::fs::write(workspace.root().join("fleet-result.json"), serde_json::to_vec(&json!({"task_id":"fleet-baseline-codex","operation":"valid-json","result":{"event":"tool.start","step":1},"fixture_sha256":digest(&trusted)})).unwrap()).unwrap();
    let result = verify_file(&task("valid-json"), workspace.root(), &trusted).unwrap();
    assert_eq!(result.agent_tool_proof, "unknown");
    assert!(prompt_for(&task("valid-json")).contains("fleet-result.json"));
}

#[test]
fn missing_wrong_hash_and_altered_fixture_are_rejected() {
    let workspace = SeededWorkspace::create().unwrap();
    let trusted = std::fs::read(workspace.root().join("event.json")).unwrap();
    assert!(verify_file(&task("valid-json"), workspace.root(), &trusted).is_err());
    std::fs::write(workspace.root().join("fleet-result.json"), br#"{"task_id":"fleet-baseline-codex","operation":"valid-json","result":{"event":"tool.start","step":1},"fixture_sha256":"bad"}"#).unwrap();
    assert!(verify_file(&task("valid-json"), workspace.root(), &trusted).is_err());
    std::fs::write(workspace.root().join("fleet-result.json"), serde_json::to_vec(&json!({"task_id":"fleet-baseline-codex","operation":"valid-json","result":{"event":"wrong","step":1},"fixture_sha256":digest(&trusted)})).unwrap()).unwrap();
    assert!(verify_file(&task("valid-json"), workspace.root(), &trusted).is_err());
    std::fs::write(workspace.root().join("fleet-result.json"), serde_json::to_vec(&json!({"task_id":"fleet-baseline-codex","operation":"valid-json","result":{"event":"changed","step":1},"fixture_sha256":digest(br#"{"event":"changed","step":1}"#)})).unwrap()).unwrap();
    std::fs::write(
        workspace.root().join("event.json"),
        br#"{"event":"changed","step":1}"#,
    )
    .unwrap();
    assert!(verify_file(&task("valid-json"), workspace.root(), &trusted).is_err());
}

#[test]
fn oversized_evidence_is_rejected_before_full_read() {
    let workspace = SeededWorkspace::create().unwrap();
    let trusted = std::fs::read(workspace.root().join("event.json")).unwrap();
    std::fs::write(
        workspace.root().join("fleet-result.json"),
        vec![b'{'; agent_otel_fleet_smoke::live_evidence::MAX_EVIDENCE_BYTES + 1],
    )
    .unwrap();
    assert!(verify_file(&task("valid-json"), workspace.root(), &trusted).is_err());
}

#[test]
fn unsupported_operation_is_explicitly_inconclusive() {
    let workspace = SeededWorkspace::create().unwrap();
    assert!(verify_file(&task("mcp-initialize"), workspace.root(), b"{}").is_err());
    assert!(prompt_for(&task("mcp-initialize")).contains("unsupported"));
}
