use agent_otel_fleet_smoke::{
    plan::seeded_profile,
    report::{plan_hash, read_artifact, serialize_envelope, summarize, trace_url, RunContext},
    runner::{cleanup_temp_artifact, RunMode, Runner},
};

#[test]
fn v2_report_roundtrips_and_retains_legacy_artifact_fields() {
    let run = Runner::new(7).run(RunMode::Synthetic).unwrap();
    let value = serialize_envelope(&run, 0, 1);
    assert_eq!(value["schema_version"], 2);
    assert!(value.get("plan").is_some());
    assert!(value.get("summary").is_some());
    assert!(value["summary"]["state"].is_string());
    assert!(value["summary"]["task_counts"]["control_spans"].is_number());
    let decoded = read_artifact(&serde_json::to_string(&value).unwrap()).unwrap();
    assert_eq!(decoded.run_id, run.run_id);
    assert_eq!(decoded.plan, run.plan);
    let mut changed = value.clone();
    changed["summary"]["trace_id"] = serde_json::json!("ffffffffffffffffffffffffffffffff");
    assert!(read_artifact(&changed.to_string()).is_err());
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../report-v2.schema.json")).unwrap();
    assert!(jsonschema::validator_for(&schema).unwrap().is_valid(&value));
    let _ = cleanup_temp_artifact(&run.artifact_path);
}

#[test]
fn reader_accepts_v1_and_rejects_unknown_versions() {
    let run = Runner::new(8).run(RunMode::Synthetic).unwrap();
    let v1 = serde_json::to_string(&run).unwrap();
    assert_eq!(read_artifact(&v1).unwrap().run_id, run.run_id);
    let mut unknown = serde_json::to_value(&run).unwrap();
    unknown["schema_version"] = serde_json::json!(99);
    let error = read_artifact(&serde_json::to_string(&unknown).unwrap()).unwrap_err();
    assert!(error.contains("unknown schema_version"));
    for bad in [
        serde_json::Value::Null,
        serde_json::json!("2"),
        serde_json::json!(-1),
    ] {
        unknown["schema_version"] = bad;
        assert!(read_artifact(&unknown.to_string()).is_err());
    }
    assert!(read_artifact(&serde_json::json!([run, run]).to_string()).is_err());
    let _ = cleanup_temp_artifact(&run.artifact_path);
}

#[test]
fn plan_hash_and_context_are_reproducible() {
    let plan = seeded_profile("baseline", 42).unwrap();
    assert_eq!(plan_hash(&plan), plan_hash(&plan));
    let run = Runner::new(42).run(RunMode::Synthetic).unwrap();
    let context = RunContext::from_artifact(&run);
    assert_eq!(context.seed, 42);
    assert_eq!(context.trace_id, run.spans[0].trace_id);
    assert_eq!(summarize(&run, 1, 2)["repetition"], 1);
    let _ = cleanup_temp_artifact(&run.artifact_path);
}

#[test]
fn trace_links_encode_values_and_reject_bad_templates() {
    let run = Runner::new(9).run(RunMode::Synthetic).unwrap();
    let url = trace_url(
        "https://collector.example/search?trace={trace_id}&from={start_unix_ms}&to={end_unix_ms}",
        &run,
    )
    .unwrap();
    assert!(url.starts_with("https://collector.example/search?trace="));
    assert!(trace_url("https://user:secret@example.test/{trace_id}", &run).is_err());
    assert!(trace_url("https://example.test/{unknown}", &run).is_err());
    assert!(trace_url("file:///tmp/{trace_id}", &run).is_err());
    assert!(trace_url("https://example.test/{trace_id}}", &run).is_err());
    assert!(trace_url("https://example.test/search?token={trace_id}", &run).is_err());
    let _ = cleanup_temp_artifact(&run.artifact_path);
}
