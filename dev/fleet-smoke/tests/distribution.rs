//! Ensure development tooling cannot become a runtime dependency of shipped crates.
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::process::Command;

#[test]
fn laboratory_is_unpublishable_and_outside_runtime_dependency_graph() {
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("Cargo metadata must be available during Cargo tests");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).unwrap();
    let packages: HashMap<&str, &Value> = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|package| (package["name"].as_str().unwrap(), package))
        .collect();
    let laboratory = packages["agent-otel-fleet-smoke"];
    assert_eq!(laboratory["publish"], serde_json::json!([]));

    let mut pending = vec!["agent-otel-bridge", "agent-otel-client"];
    let mut visited = HashSet::new();
    while let Some(name) = pending.pop() {
        assert_ne!(
            name, "agent-otel-fleet-smoke",
            "laboratory reached by shipped dependency graph"
        );
        if !visited.insert(name) {
            continue;
        }
        if let Some(package) = packages.get(name) {
            for dependency in package["dependencies"].as_array().unwrap() {
                if dependency["kind"].as_str() != Some("dev") {
                    pending.push(dependency["name"].as_str().unwrap());
                }
            }
        }
    }
}
