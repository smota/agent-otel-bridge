use agent_otel_core::context::{HarvestLimits, WorkspaceContext};
use agent_otel_core::model::{
    AgentHookInput, HookEvent, MAX_DYNAMIC_JSON_NODES, MAX_HOOK_INPUT_BYTES,
};
use agent_otel_core::otlp::{build_span_from_resolved, ResolvedSpanMetadata};
use agent_otel_core::trace_id::ResolvedTraceContext;
use opentelemetry_proto::tonic::common::v1::any_value;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn string_attribute<'a>(
    span: &'a opentelemetry_proto::tonic::trace::v1::Span,
    key: &str,
) -> Option<&'a str> {
    span.attributes.iter().find_map(|attribute| {
        if attribute.key != key {
            return None;
        }
        match attribute.value.as_ref()?.value.as_ref()? {
            any_value::Value::StringValue(value) => Some(value.as_str()),
            _ => None,
        }
    })
}

fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "agent-otel-core-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("create isolated test directory");
    path
}

#[test]
fn parser_rejects_size_and_node_complexity_before_deserialization() {
    let oversized = vec![b' '; MAX_HOOK_INPUT_BYTES + 1];
    let size_error = AgentHookInput::parse_slice(&oversized).expect_err("oversized JSON accepted");
    assert!(size_error.to_string().contains("16 MiB"));

    let values = std::iter::repeat_n("null", MAX_DYNAMIC_JSON_NODES)
        .collect::<Vec<_>>()
        .join(",");
    let complex = format!(r#"{{"toolInput":[{values}]}}"#);
    let complexity_error =
        AgentHookInput::parse_slice(complex.as_bytes()).expect_err("complex JSON accepted");
    assert!(complexity_error.to_string().contains("node budget"));
}

#[test]
fn pure_normalization_fills_only_missing_fields() {
    let context = WorkspaceContext {
        current_dir: "C:/work/cache".to_string(),
        project_name: Some("cached-name".to_string()),
        project_root: Some("C:/work/cache".to_string()),
        project_type: Some("rust".to_string()),
        vcs_system: Some("git".to_string()),
        vcs_branch: Some("cached-branch".to_string()),
        ..Default::default()
    };
    let mut input = AgentHookInput {
        workspace_path: Some("C:/work/explicit".to_string()),
        project_name: Some("explicit-name".to_string()),
        tool_name: Some("run_command".to_string()),
        ..Default::default()
    };

    input.normalize_with_context(Some(&context));

    assert_eq!(input.workspace_path.as_deref(), Some("C:/work/explicit"));
    assert_eq!(input.project_name.as_deref(), Some("explicit-name"));
    assert_eq!(input.project_root.as_deref(), Some("C:/work/cache"));
    assert_eq!(input.vcs_branch.as_deref(), Some("cached-branch"));
    assert!(input.capability_kind.is_some());
    assert!(input.tool_archetype.is_some());
    assert_eq!(input.agent_depth, None);
}

#[test]
fn resolved_builder_uses_only_explicit_metadata_and_preserves_payload_precedence() {
    let input = AgentHookInput {
        conversation_id: Some("pure-builder".to_string()),
        user_email: Some("payload@example.test".to_string()),
        terminal_type: Some("payload-terminal".to_string()),
        ..Default::default()
    };
    let span = build_span_from_resolved(
        HookEvent::Stop,
        &input,
        10,
        20,
        1,
        ResolvedTraceContext {
            trace_id: [7; 16],
            parent_span_id: Some([8; 8]),
            trace_flags: 1,
        },
        ResolvedSpanMetadata {
            user_email: Some("fallback@example.test"),
            terminal_type: Some("fallback-terminal"),
        },
        false,
    );

    assert_eq!(
        string_attribute(&span, "user.email"),
        Some("payload@example.test")
    );
    assert_eq!(
        string_attribute(&span, "terminal.type"),
        Some("payload-terminal")
    );
    assert_eq!(span.trace_id, vec![7; 16]);
    assert_eq!(span.parent_span_id, vec![8; 8]);
}

#[test]
fn bounded_harvest_does_not_parse_oversized_project_file_or_fill_launch_dir() {
    let root = temporary_directory("bounded-harvest");
    let mut cargo_toml = b"[package]\nname = \"must-not-be-read\"\n".to_vec();
    cargo_toml.resize(65 * 1024, b' ');
    fs::write(root.join("Cargo.toml"), cargo_toml).expect("write oversized marker");

    let harvest = WorkspaceContext::harvest_from_dir_with_limits(
        &root,
        HarvestLimits {
            budget: Duration::from_secs(1),
            ..HarvestLimits::default()
        },
    );

    assert!(harvest.completed);
    assert_eq!(harvest.context.project_type.as_deref(), Some("rust"));
    assert_ne!(
        harvest.context.project_name.as_deref(),
        Some("must-not-be-read")
    );
    assert_eq!(harvest.context.launch_dir, None);

    fs::remove_dir_all(&root).expect("remove isolated test directory");
}

#[test]
fn bounded_harvest_resolves_worktree_common_dir_and_packed_ref() {
    let parent = temporary_directory("worktree");
    let repository = parent.join("repository");
    let worktree = parent.join("linked-worktree");
    let admin = repository.join(".git/worktrees/linked");
    fs::create_dir_all(&admin).expect("create worktree admin directory");
    fs::create_dir_all(&worktree).expect("create linked worktree");
    fs::write(
        worktree.join("Cargo.toml"),
        "[package]\nname = \"linked\"\n",
    )
    .expect("write worktree marker");
    fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", admin.to_string_lossy()),
    )
    .expect("write worktree pointer");
    fs::write(admin.join("HEAD"), "ref: refs/heads/packed-branch\n").expect("write HEAD");
    fs::write(admin.join("commondir"), "../..\n").expect("write commondir");
    fs::write(
        repository.join(".git/packed-refs"),
        "3333333333333333333333333333333333333333 refs/heads/packed-branch\n",
    )
    .expect("write packed refs");
    fs::write(
        repository.join(".git/config"),
        "[remote \"origin\"]\nurl = https://secret@example.test/owner/worktree-repo.git\n",
    )
    .expect("write common config");

    let harvest = WorkspaceContext::harvest_from_dir_with_limits(
        &worktree,
        HarvestLimits {
            budget: Duration::from_secs(1),
            ..HarvestLimits::default()
        },
    );

    assert!(harvest.completed);
    assert_eq!(harvest.context.vcs_worktree, Some(true));
    assert_eq!(harvest.context.vcs_branch.as_deref(), Some("packed-branch"));
    assert_eq!(
        harvest.context.vcs_commit.as_deref(),
        Some("3333333333333333333333333333333333333333")
    );
    assert_eq!(
        harvest.context.vcs_repository.as_deref(),
        Some("worktree-repo")
    );

    fs::remove_dir_all(parent).expect("remove isolated test directory");
}
