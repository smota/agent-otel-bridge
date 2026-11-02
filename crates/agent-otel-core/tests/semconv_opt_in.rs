use agent_otel_core::model::{AgentHookInput, HookEvent};
use agent_otel_core::otlp::{
    build_span_from_resolved, build_trace_request, build_trace_request_with_schema,
    ResolvedSpanMetadata,
};
use agent_otel_core::semconv::{
    AGENT_LINEAGE_DEPTH, AGENT_LINEAGE_IS_ROOT, AGENT_LINEAGE_PARENT_NAME, AGENT_LINEAGE_ROOT_ID,
    AGENT_STEP_INDEX, AGENT_TERMINATION_REASON, ERROR_TYPE, GEN_AI_AGENT_DEPTH,
    GEN_AI_AGENT_IS_ROOT, GEN_AI_AGENT_PARENT_NAME, GEN_AI_AGENT_ROOT_ID, GEN_AI_AGENT_STEP_INDEX,
    GEN_AI_OPERATION_NAME, GEN_AI_PROVIDER_ANTHROPIC, GEN_AI_PROVIDER_GOOGLE, GEN_AI_PROVIDER_NAME,
    GEN_AI_PROVIDER_UNKNOWN, GEN_AI_RESPONSE_FINISH_REASONS, GEN_AI_SYSTEM, OTEL_SCHEMA_1_41_0,
    VCS_BRANCH_NAME, VCS_COMMIT_SHA, VCS_REF_HEAD_NAME, VCS_REF_HEAD_REVISION,
};
use agent_otel_core::trace_id::ResolvedTraceContext;
use opentelemetry_proto::tonic::common::v1::any_value;
use opentelemetry_proto::tonic::resource::v1::Resource;
use opentelemetry_proto::tonic::trace::v1::status::StatusCode;
use opentelemetry_proto::tonic::trace::v1::Span;

fn build(input: &AgentHookInput, opt_in: bool) -> Span {
    build_span_from_resolved(
        HookEvent::PostToolUse,
        input,
        1,
        2,
        0,
        ResolvedTraceContext {
            trace_id: [3; 16],
            parent_span_id: None,
            trace_flags: 0,
        },
        ResolvedSpanMetadata::default(),
        false,
        opt_in,
    )
}

fn string_attr<'a>(span: &'a Span, key: &str) -> Option<&'a str> {
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

fn int_attr(span: &Span, key: &str) -> Option<i64> {
    span.attributes.iter().find_map(|attribute| {
        if attribute.key != key {
            return None;
        }
        match attribute.value.as_ref()?.value.as_ref()? {
            any_value::Value::IntValue(value) => Some(*value),
            _ => None,
        }
    })
}

fn bool_attr(span: &Span, key: &str) -> Option<bool> {
    span.attributes.iter().find_map(|attribute| {
        if attribute.key != key {
            return None;
        }
        match attribute.value.as_ref()?.value.as_ref()? {
            any_value::Value::BoolValue(value) => Some(*value),
            _ => None,
        }
    })
}

fn string_array(span: &Span, key: &str) -> Option<Vec<String>> {
    span.attributes.iter().find_map(|attribute| {
        if attribute.key != key {
            return None;
        }
        match attribute.value.as_ref()?.value.as_ref()? {
            any_value::Value::ArrayValue(values) => Some(
                values
                    .values
                    .iter()
                    .map(|value| match value.value.as_ref() {
                        Some(any_value::Value::StringValue(text)) => text.clone(),
                        other => panic!("expected string array element, got {other:?}"),
                    })
                    .collect(),
            ),
            _ => None,
        }
    })
}

fn has_key(span: &Span, key: &str) -> bool {
    span.attributes.iter().any(|attribute| attribute.key == key)
}

#[test]
fn missing_model_defaults_to_google_and_opt_in_omits_provider() {
    let input = AgentHookInput::default();
    let off = build(&input, false);
    assert_eq!(
        string_attr(&off, GEN_AI_PROVIDER_NAME),
        Some(GEN_AI_PROVIDER_GOOGLE)
    );
    assert_eq!(
        string_attr(&off, GEN_AI_SYSTEM),
        Some(GEN_AI_PROVIDER_GOOGLE)
    );

    let on = build(&input, true);
    assert!(!has_key(&on, GEN_AI_PROVIDER_NAME));
    assert!(!has_key(&on, GEN_AI_SYSTEM));
    assert_ne!(string_attr(&on, GEN_AI_OPERATION_NAME), Some("chat"));
}

#[test]
fn unrecognized_model_stays_unknown_only_on_the_default_path() {
    let input = AgentHookInput {
        model: Some("not-a-real-model".to_string()),
        ..Default::default()
    };
    let off = build(&input, false);
    assert_eq!(
        string_attr(&off, GEN_AI_PROVIDER_NAME),
        Some(GEN_AI_PROVIDER_UNKNOWN)
    );
    assert_eq!(
        string_attr(&off, GEN_AI_SYSTEM),
        Some(GEN_AI_PROVIDER_UNKNOWN)
    );

    let on = build(&input, true);
    assert!(!has_key(&on, GEN_AI_PROVIDER_NAME));
    assert!(!has_key(&on, GEN_AI_SYSTEM));
}

#[test]
fn claude_model_emits_anthropic_without_gen_ai_system_when_opted_in() {
    let input = AgentHookInput {
        model: Some("claude-3-5-sonnet".to_string()),
        ..Default::default()
    };
    let off = build(&input, false);
    assert_eq!(
        string_attr(&off, GEN_AI_PROVIDER_NAME),
        Some(GEN_AI_PROVIDER_ANTHROPIC)
    );
    assert_eq!(
        string_attr(&off, GEN_AI_SYSTEM),
        Some(GEN_AI_PROVIDER_ANTHROPIC)
    );

    let on = build(&input, true);
    assert_eq!(
        string_attr(&on, GEN_AI_PROVIDER_NAME),
        Some(GEN_AI_PROVIDER_ANTHROPIC)
    );
    assert!(!has_key(&on, GEN_AI_SYSTEM));
    assert_ne!(string_attr(&on, GEN_AI_OPERATION_NAME), Some("chat"));
    assert_eq!(string_attr(&on, "gen_ai.agent.name"), Some("claude-code"));
}

#[test]
fn finish_reasons_stay_a_string_until_opt_in_makes_a_one_element_array() {
    let input = AgentHookInput {
        termination_reason: Some("NO_TOOL_CALL".to_string()),
        ..Default::default()
    };
    let off = build(&input, false);
    assert_eq!(
        string_attr(&off, AGENT_TERMINATION_REASON),
        Some("NO_TOOL_CALL")
    );
    assert_eq!(
        string_attr(&off, GEN_AI_RESPONSE_FINISH_REASONS),
        Some("NO_TOOL_CALL")
    );
    assert!(string_array(&off, GEN_AI_RESPONSE_FINISH_REASONS).is_none());

    let on = build(&input, true);
    assert_eq!(
        string_attr(&on, AGENT_TERMINATION_REASON),
        Some("NO_TOOL_CALL")
    );
    assert!(string_attr(&on, GEN_AI_RESPONSE_FINISH_REASONS).is_none());
    assert_eq!(
        string_array(&on, GEN_AI_RESPONSE_FINISH_REASONS),
        Some(vec!["NO_TOOL_CALL".to_string()])
    );
}

#[test]
fn success_status_is_ok_by_default_and_unset_when_opted_in() {
    let input = AgentHookInput::default();
    let off = build(&input, false);
    assert_eq!(off.status.as_ref().unwrap().code, StatusCode::Ok as i32);
    assert!(!has_key(&off, ERROR_TYPE));

    let on = build(&input, true);
    assert_eq!(on.status.as_ref().unwrap().code, StatusCode::Unset as i32);
    assert!(on.status.as_ref().unwrap().message.is_empty());
    assert!(!has_key(&on, ERROR_TYPE));
}

#[test]
fn error_type_uses_category_or_other_and_keeps_the_status_message() {
    let categorized = AgentHookInput {
        error: Some(" upstream failed ".to_string()),
        error_category: Some("provider_quota_exhausted".to_string()),
        ..Default::default()
    };
    let off = build(&categorized, false);
    assert_eq!(off.status.as_ref().unwrap().code, StatusCode::Error as i32);
    assert_eq!(off.status.as_ref().unwrap().message, " upstream failed ");
    assert!(!has_key(&off, ERROR_TYPE));

    let on = build(&categorized, true);
    assert_eq!(on.status.as_ref().unwrap().code, StatusCode::Error as i32);
    assert_eq!(on.status.as_ref().unwrap().message, " upstream failed ");
    assert_eq!(
        string_attr(&on, ERROR_TYPE),
        Some("provider_quota_exhausted")
    );

    let bare = AgentHookInput {
        error: Some("boom".to_string()),
        error_category: Some(String::new()),
        ..Default::default()
    };
    let fallback = build(&bare, true);
    assert_eq!(string_attr(&fallback, ERROR_TYPE), Some("_OTHER"));
    assert_eq!(fallback.status.as_ref().unwrap().message, "boom");
}

#[test]
fn lineage_and_vcs_keys_move_without_keeping_the_old_names() {
    let input = AgentHookInput {
        step_idx: Some(4),
        agent_depth: Some(2),
        agent_parent_name: Some("claude-code".to_string()),
        agent_root_id: Some("root-1".to_string()),
        agent_is_root: Some(false),
        vcs_branch: Some("main".to_string()),
        vcs_commit: Some("abc123".to_string()),
        ..Default::default()
    };
    let off = build(&input, false);
    assert_eq!(int_attr(&off, GEN_AI_AGENT_DEPTH), Some(2));
    assert_eq!(
        string_attr(&off, GEN_AI_AGENT_PARENT_NAME),
        Some("claude-code")
    );
    assert_eq!(string_attr(&off, GEN_AI_AGENT_ROOT_ID), Some("root-1"));
    assert_eq!(bool_attr(&off, GEN_AI_AGENT_IS_ROOT), Some(false));
    assert_eq!(int_attr(&off, GEN_AI_AGENT_STEP_INDEX), Some(4));
    assert_eq!(int_attr(&off, AGENT_STEP_INDEX), Some(4));
    assert_eq!(string_attr(&off, VCS_BRANCH_NAME), Some("main"));
    assert_eq!(string_attr(&off, VCS_COMMIT_SHA), Some("abc123"));
    assert!(!has_key(&off, AGENT_LINEAGE_DEPTH));
    assert!(!has_key(&off, VCS_REF_HEAD_NAME));
    assert!(!has_key(&off, VCS_REF_HEAD_REVISION));

    let on = build(&input, true);
    assert_eq!(int_attr(&on, AGENT_LINEAGE_DEPTH), Some(2));
    assert_eq!(
        string_attr(&on, AGENT_LINEAGE_PARENT_NAME),
        Some("claude-code")
    );
    assert_eq!(string_attr(&on, AGENT_LINEAGE_ROOT_ID), Some("root-1"));
    assert_eq!(bool_attr(&on, AGENT_LINEAGE_IS_ROOT), Some(false));
    assert_eq!(int_attr(&on, AGENT_STEP_INDEX), Some(4));
    assert!(!has_key(&on, GEN_AI_AGENT_DEPTH));
    assert!(!has_key(&on, GEN_AI_AGENT_PARENT_NAME));
    assert!(!has_key(&on, GEN_AI_AGENT_ROOT_ID));
    assert!(!has_key(&on, GEN_AI_AGENT_IS_ROOT));
    assert!(!has_key(&on, GEN_AI_AGENT_STEP_INDEX));
    assert!(!has_key(&on, VCS_BRANCH_NAME));
    assert!(!has_key(&on, VCS_COMMIT_SHA));
    assert_eq!(string_attr(&on, VCS_REF_HEAD_NAME), Some("main"));
    assert_eq!(string_attr(&on, VCS_REF_HEAD_REVISION), Some("abc123"));
}

#[test]
fn trace_schema_url_is_empty_unless_the_opt_in_schema_is_requested() {
    let off = build_trace_request(
        Resource::default(),
        vec![build(&AgentHookInput::default(), false)],
    );
    assert_eq!(off.resource_spans[0].schema_url, "");
    assert_eq!(off.resource_spans[0].scope_spans[0].schema_url, "");

    let on = build_trace_request_with_schema(
        Resource::default(),
        vec![build(&AgentHookInput::default(), true)],
        OTEL_SCHEMA_1_41_0,
    );
    assert_eq!(on.resource_spans[0].schema_url, OTEL_SCHEMA_1_41_0);
    assert_eq!(
        on.resource_spans[0].scope_spans[0].schema_url,
        OTEL_SCHEMA_1_41_0
    );
    assert_eq!(
        OTEL_SCHEMA_1_41_0,
        "https://opentelemetry.io/schemas/1.41.0"
    );
}

#[test]
fn resolved_builder_ignores_git_author_email_without_metadata() {
    let previous = std::env::var("GIT_AUTHOR_EMAIL").ok();
    // SAFETY: this test binary does not otherwise write GIT_AUTHOR_EMAIL.
    // build_span_from_resolved must not read it. The previous value is restored
    // before the test returns.
    unsafe {
        std::env::set_var(
            "GIT_AUTHOR_EMAIL",
            "git-author-must-not-leak@example.invalid",
        );
    }
    let span = build(&AgentHookInput::default(), false);
    assert!(
        !has_key(&span, "user.email"),
        "user.email leaked from the environment"
    );
    unsafe {
        match previous {
            Some(value) => std::env::set_var("GIT_AUTHOR_EMAIL", value),
            None => std::env::remove_var("GIT_AUTHOR_EMAIL"),
        }
    }
}

#[test]
fn status_message_truncates_only_when_experimental_opt_in_is_on() {
    let long = format!("{}{}", "e".repeat(159), "é");
    assert!(long.len() > 160);
    let input = AgentHookInput {
        error: Some(long.clone()),
        error_category: Some("provider_quota_exhausted".to_string()),
        ..Default::default()
    };

    let off = build(&input, false);
    assert_eq!(off.status.as_ref().unwrap().message, long);
    assert!(!has_key(&off, ERROR_TYPE));

    let on = build(&input, true);
    let message = &on.status.as_ref().unwrap().message;
    assert_eq!(message, &format!("{}…", "e".repeat(159)));
    assert_ne!(message, &long);
    assert!(!message.contains('é'));
    assert_eq!(
        string_attr(&on, ERROR_TYPE),
        Some("provider_quota_exhausted")
    );
    assert_ne!(string_attr(&on, ERROR_TYPE), Some(message.as_str()));
}
