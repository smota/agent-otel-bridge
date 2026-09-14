use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_agent-otel-fleet-smoke"))
        .args(args)
        .output()
        .expect("fleet-smoke binary runs")
}

#[test]
fn plan_selects_seeded_profile_and_accepts_global_args_after_subcommand() {
    let output = cli(&["plan", "--profile", "mixed", "--seed", "42"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["id"], "mixed");
    assert_eq!(value["tasks"].as_array().unwrap().len(), 6);
    assert_eq!(value["tasks"][0]["operation"], "valid-json");
}

#[test]
fn offline_run_repeat_two_returns_two_fresh_runs() {
    let output = cli(&[
        "run",
        "--profile",
        "baseline",
        "--seed",
        "42",
        "--repeat",
        "2",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let reports: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let reports = reports.as_array().unwrap();
    assert_eq!(reports.len(), 2);
    assert!(reports.iter().all(|r| r["verification"]["passed"] == true));
    assert_ne!(reports[0]["run"]["run_id"], reports[1]["run"]["run_id"]);
}

#[test]
fn invalid_profile_and_repeat_are_rejected() {
    assert!(!cli(&["plan", "--profile", "missing"]).status.success());
    assert!(!cli(&["run", "--repeat", "0"]).status.success());
}

#[test]
fn artifact_tamper_is_rejected() {
    let output = cli(&[
        "run",
        "--profile",
        "baseline",
        "--seed",
        "7",
        "--keep-artifact",
    ]);
    assert!(output.status.success());
    let reports: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let artifact = reports[0]["run"]["artifact_path"].as_str().unwrap();
    let raw = fs::read_to_string(artifact).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    value["plan"]["id"] = serde_json::Value::String("tampered".into());
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let tampered = std::env::temp_dir().join(format!("fleet-smoke-tampered-{stamp}.json"));
    fs::write(&tampered, serde_json::to_vec(&value).unwrap()).unwrap();
    let verify = cli(&["verify", tampered.to_str().unwrap()]);
    assert!(!verify.status.success());
    let _ = fs::remove_file(tampered);
    let _ = fs::remove_dir_all(std::path::Path::new(artifact).parent().unwrap());
}
