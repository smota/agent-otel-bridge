use serde::{Deserialize, Serialize};

pub const CODEX_MODEL: &str = "gpt-5.6-luna";
pub const GROK_MODEL: &str = "grok-4.5";
pub const ANTIGRAVITY_MODEL: &str = "gemini-3.8-flash-low";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FleetPlan {
    pub format_version: u8,
    pub mode: String,
    pub scenarios: Vec<PlanScenario>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanScenario {
    pub id: String,
    pub purpose: String,
    pub requirements_ref: String,
    pub specification_ref: String,
    pub implementation_ref: String,
    pub tasks: Vec<PlanTask>,
    pub expectations: Expectations,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanTask {
    pub id: String,
    pub platform: String,
    pub model: String,
    pub reasoning: String,
    pub depends_on: Vec<String>,
    pub operation: String,
    pub requirement_ref: String,
    pub implementation_ref: String,
    pub expected_fault: Option<String>,
    pub expected_outcome: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Expectations {
    pub minimum_spans: usize,
    pub required_trace_continuity: bool,
    pub native_bridge_propagation: String,
    pub expected_fault: Option<String>,
    pub max_ready: usize,
}

pub fn seeded_plan() -> FleetPlan {
    FleetPlan {
        format_version: 1,
        mode: "synthetic-default".into(),
        scenarios: vec![
            scenario(
                "baseline",
                "A minimal cross-platform parent and child trace.",
                4,
                None,
            ),
            scenario(
                "mixed",
                "A three-platform fan-out with a deterministic dependency DAG.",
                7,
                None,
            ),
            scenario(
                "long-trace",
                "A bounded multi-hop lineage trace, without a live campaign.",
                13,
                None,
            ),
        ],
    }
}

pub fn selected_profile(plan: &FleetPlan, name: &str) -> Option<PlanScenario> {
    plan.scenarios
        .iter()
        .find(|scenario| scenario.id == name)
        .cloned()
}

pub fn seeded_profile(name: &str, seed: u64) -> Option<PlanScenario> {
    let mut scenario = selected_profile(&seeded_plan(), name)?;
    let operations = [
        ("valid-json", None),
        ("invalid-json", Some("invalid_json")),
        ("function-defect", Some("test_failure")),
        ("mcp-initialize", None),
        ("http-recovery", Some("http_500_recovered")),
        ("http-timeout", Some("timeout")),
    ];
    let fixtures: Vec<_> = operations
        .into_iter()
        .filter(|(_, fault)| name != "baseline" || fault.is_none())
        .collect();
    for (index, task) in scenario.tasks.iter_mut().enumerate() {
        let fixture = fixtures[(index + (seed % fixtures.len() as u64) as usize) % fixtures.len()];
        task.operation = fixture.0.into();
        task.requirement_ref = format!("dev/fleet-smoke/README.md#{}", fixture.0);
        task.implementation_ref = "dev/fleet-smoke/src/operations.rs::perform".into();
        task.expected_fault = fixture.1.map(str::to_owned);
        task.expected_outcome = if fixture.1.is_some() {
            "expected_fault".into()
        } else {
            "success".into()
        };
    }
    Some(scenario)
}

fn scenario(
    id: &str,
    purpose: &str,
    minimum_spans: usize,
    expected_fault: Option<&str>,
) -> PlanScenario {
    let task_prefix = format!("fleet-{id}");
    let tasks = match id {
        "baseline" => vec![
            task(&task_prefix, "codex", "codex", CODEX_MODEL, vec![]),
            task(
                &task_prefix,
                "grok",
                "grok",
                GROK_MODEL,
                vec!["codex".into()],
            ),
            task(
                &task_prefix,
                "antigravity",
                "antigravity",
                ANTIGRAVITY_MODEL,
                vec!["grok".into()],
            ),
        ],
        "mixed" => vec![
            task(&task_prefix, "root", "codex", CODEX_MODEL, vec![]),
            task(
                &task_prefix,
                "grok-branch",
                "grok",
                GROK_MODEL,
                vec!["root".into()],
            ),
            task(
                &task_prefix,
                "gemini-branch",
                "antigravity",
                ANTIGRAVITY_MODEL,
                vec!["root".into()],
            ),
            task(
                &task_prefix,
                "join",
                "codex",
                CODEX_MODEL,
                vec!["grok-branch".into(), "gemini-branch".into()],
            ),
            task(
                &task_prefix,
                "recovery",
                "grok",
                GROK_MODEL,
                vec!["join".into()],
            ),
            task(
                &task_prefix,
                "final",
                "antigravity",
                ANTIGRAVITY_MODEL,
                vec!["recovery".into()],
            ),
        ],
        _ => (0..12)
            .map(|n| {
                task(
                    &task_prefix,
                    &format!("hop-{n}"),
                    match n % 3 {
                        0 => "codex",
                        1 => "grok",
                        _ => "antigravity",
                    },
                    if n % 3 == 0 {
                        CODEX_MODEL
                    } else if n % 3 == 1 {
                        GROK_MODEL
                    } else {
                        ANTIGRAVITY_MODEL
                    },
                    if n == 0 {
                        vec![]
                    } else {
                        vec![format!("hop-{}", n - 1)]
                    },
                )
            })
            .collect(),
    };
    PlanScenario {
        id: id.into(),
        purpose: purpose.into(),
        requirements_ref: "AGENTS.md#3.4-cross-agent-w3c-tracing-propagation".into(),
        specification_ref: "docs/TELEMETRY_DICTIONARY.md".into(),
        implementation_ref: "dev/fleet-smoke/src/runner.rs".into(),
        tasks,
        expectations: Expectations {
            minimum_spans,
            required_trace_continuity: true,
            native_bridge_propagation: "INCONCLUSIVE until an actual bridge capture is supplied"
                .into(),
            expected_fault: expected_fault.map(str::to_owned),
            max_ready: 3,
        },
    }
}

fn task(
    prefix: &str,
    suffix: &str,
    platform: &str,
    model: &str,
    dependencies: Vec<String>,
) -> PlanTask {
    PlanTask {
        id: format!("{prefix}-{suffix}"),
        platform: platform.into(),
        model: model.into(),
        reasoning: "low".into(),
        depends_on: dependencies
            .into_iter()
            .map(|dep| format!("{prefix}-{dep}"))
            .collect(),
        operation: "unassigned".into(),
        requirement_ref: "dev/fleet-smoke/README.md#case-contracts".into(),
        implementation_ref: "dev/fleet-smoke/src/operations.rs::perform".into(),
        expected_fault: None,
        expected_outcome: "success".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plan_has_fixed_models_and_unique_task_ids() {
        let plan = seeded_plan();
        let mut ids = std::collections::BTreeSet::new();
        for task in plan.scenarios.iter().flat_map(|s| &s.tasks) {
            assert!(ids.insert(&task.id));
            assert_eq!(task.reasoning, "low");
        }
        assert!(plan
            .scenarios
            .iter()
            .flat_map(|s| &s.tasks)
            .all(|t| t.model != "claude"));
    }
}
