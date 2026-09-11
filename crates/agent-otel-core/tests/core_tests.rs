/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_core::model::{AntigravityHookInput, HookEvent};
use agent_otel_core::otlp::{
    build_resource, build_span_from_hook, build_span_from_hook_opts, build_trace_request,
};
use agent_otel_core::quota::{
    build_quota_metrics_request, build_quota_metrics_request_opts, QuotaSnapshot,
};
use agent_otel_core::semconv::*;
use agent_otel_core::trace_id::{derive_span_id, derive_trace_id};

#[test]
fn test_hook_input_deserialization_camel_case() {
    let json = r#"{
        "conversationId": "test-conv-12345",
        "stepIdx": 42,
        "toolCall": {
            "id": "call-99",
            "name": "run_command",
            "arguments": { "CommandLine": "cargo check" }
        },
        "modelName": "gemini-2.5-pro",
        "executionNum": 3,
        "fullyIdle": false
    }"#;

    let parsed = AntigravityHookInput::parse_slice(json.as_bytes()).expect("failed to parse");
    assert_eq!(parsed.conversation_id.as_deref(), Some("test-conv-12345"));
    assert_eq!(parsed.step_idx, Some(42));
    assert_eq!(parsed.model.as_deref(), Some("gemini-2.5-pro"));
    assert_eq!(parsed.execution_num, Some(3));
    assert_eq!(parsed.fully_idle, Some(false));

    let tc = parsed.tool_call.expect("tool call missing");
    assert_eq!(tc.name.as_deref(), Some("run_command"));
}

#[test]
fn test_deterministic_trace_and_span_ids() {
    let conv = "session-abc-xyz";
    let tid1 = derive_trace_id(Some(conv));
    let tid2 = derive_trace_id(Some(conv));
    assert_eq!(tid1, tid2);
    assert_ne!(tid1, [0u8; 16]);

    let sid1 = derive_span_id(
        Some(conv),
        Some(1),
        HookEvent::PostToolUse,
        Some("grep_search"),
        0,
    );
    let sid2 = derive_span_id(
        Some(conv),
        Some(1),
        HookEvent::PostToolUse,
        Some("grep_search"),
        0,
    );
    let sid3 = derive_span_id(
        Some(conv),
        Some(2),
        HookEvent::PostToolUse,
        Some("grep_search"),
        0,
    );
    assert_eq!(sid1, sid2);
    assert_ne!(sid1, sid3);
    assert_ne!(sid1, [0u8; 8]);
}

#[test]
fn test_otlp_span_generation() {
    let json = r#"{
        "conversationId": "session-123",
        "stepIdx": 10,
        "toolCall": { "name": "run_command" },
        "modelName": "gemini-flash"
    }"#;
    let input = AntigravityHookInput::parse_slice(json.as_bytes()).unwrap();
    let span = build_span_from_hook(HookEvent::PostToolUse, &input, 1_000_000, 2_000_000, 0);

    // Canonical OTel GenAI span naming: execute_tool {tool_name}
    assert_eq!(span.name, "execute_tool run_command");
    assert_eq!(span.start_time_unix_nano, 1_000_000);
    assert_eq!(span.end_time_unix_nano, 2_000_000);

    let has_system = span.attributes.iter().any(|kv| kv.key == GEN_AI_SYSTEM);
    let has_model = span
        .attributes
        .iter()
        .any(|kv| kv.key == GEN_AI_REQUEST_MODEL);
    let has_tool = span.attributes.iter().any(|kv| kv.key == GEN_AI_TOOL_NAME);
    let has_agent_event = span.attributes.iter().any(|kv| kv.key == AGENT_HOOK_EVENT);
    let has_agy_event = span.attributes.iter().any(|kv| kv.key == AGY_HOOK_EVENT);
    let has_agent_step = span.attributes.iter().any(|kv| kv.key == AGENT_STEP_INDEX);
    let has_agy_step = span.attributes.iter().any(|kv| kv.key == AGY_STEP_INDEX);

    assert!(has_system);
    assert!(has_model);
    assert!(has_tool);
    assert!(has_agent_event);
    assert!(!has_agy_event); // Canonical by default: no agy pollution
    assert!(has_agent_step);
    assert!(!has_agy_step); // Canonical by default: no agy pollution

    let res = build_resource("agent-otel-bridge", "homelab", "0.1.0");
    let req = build_trace_request(res, vec![span]);
    assert_eq!(req.resource_spans.len(), 1);
}

#[test]
fn test_otlp_span_generation_with_legacy_aliases() {
    let json = r#"{
        "conversationId": "38d58ff9-8e43-43cf-bf24-e9188be08a1c",
        "stepIdx": 42,
        "toolCall": {
            "name": "run_command",
            "arguments": { "CommandLine": "cargo test" }
        },
        "modelName": "gemini-2.5-pro",
        "executionNum": 1,
        "fullyIdle": false
    }"#;
    let input = AntigravityHookInput::parse_slice(json.as_bytes()).unwrap();
    let span = build_span_from_hook_opts(HookEvent::PostToolUse, &input, 1_000_000, 2_000_000, 0, true);

    let has_agent_event = span.attributes.iter().any(|kv| kv.key == AGENT_HOOK_EVENT);
    let has_agy_event = span.attributes.iter().any(|kv| kv.key == AGY_HOOK_EVENT);
    let has_agent_step = span.attributes.iter().any(|kv| kv.key == AGENT_STEP_INDEX);
    let has_agy_step = span.attributes.iter().any(|kv| kv.key == AGY_STEP_INDEX);

    assert!(has_agent_event);
    assert!(has_agy_event);
    assert!(has_agent_step);
    assert!(has_agy_step);
}

#[test]
fn test_claude_code_payload_parsing() {
    let json = r#"{
        "session_id": "claude-session-777",
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": { "command": "cargo test" },
        "model": "claude-3-5-sonnet-20241022"
    }"#;
    let input =
        AntigravityHookInput::parse_slice(json.as_bytes()).expect("failed to parse Claude payload");
    assert_eq!(input.conversation_id.as_deref(), Some("claude-session-777"));
    assert_eq!(input.resolved_tool_name(), Some("Bash"));
    assert!(input.resolved_tool_arguments().is_some());

    let span = build_span_from_hook(HookEvent::PostToolUse, &input, 1_000_000, 2_000_000, 1);
    assert_eq!(span.name, "execute_tool Bash");

    let provider = span
        .attributes
        .iter()
        .find(|kv| kv.key == GEN_AI_PROVIDER_NAME)
        .unwrap();
    assert_eq!(
        provider.value.as_ref().and_then(|v| match &v.value {
            Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(s)) =>
                Some(s.as_str()),
            _ => None,
        }),
        Some("anthropic")
    );

    let agent = span
        .attributes
        .iter()
        .find(|kv| kv.key == GEN_AI_AGENT_NAME)
        .unwrap();
    assert_eq!(
        agent.value.as_ref().and_then(|v| match &v.value {
            Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(s)) =>
                Some(s.as_str()),
            _ => None,
        }),
        Some("claude-code")
    );
}

#[test]
fn test_quota_metrics_request() {
    let res = build_resource("agent-otel-bridge", "homelab", "0.1.0");
    let snapshot = QuotaSnapshot {
        remaining_fraction: 0.85,
        seconds_to_reset: 3600.0,
        observed_at_unix_nano: 1_700_000_000_000_000_000,
        bucket: "gemini-weekly".to_string(),
        group: "gemini".to_string(),
    };

    // Default: canonical only (individual quota + fleet bottleneck + reset seconds)
    let req = build_quota_metrics_request(res.clone(), &snapshot);
    let metrics = &req.resource_metrics[0].scope_metrics[0].metrics;
    assert_eq!(metrics.len(), 3);
    assert_eq!(metrics[0].name, METRIC_AGENT_QUOTA_REMAINING);
    assert_eq!(metrics[1].name, METRIC_FLEET_BOTTLENECK_RATIO);
    assert_eq!(metrics[2].name, METRIC_AGENT_QUOTA_RESET);

    // Opt-in legacy compatibility: 3 canonical + 2 legacy aliases
    let req_legacy = build_quota_metrics_request_opts(res, &snapshot, true);
    let metrics_legacy = &req_legacy.resource_metrics[0].scope_metrics[0].metrics;
    assert_eq!(metrics_legacy.len(), 5);
    assert_eq!(metrics_legacy[0].name, METRIC_AGENT_QUOTA_REMAINING);
    assert_eq!(metrics_legacy[1].name, METRIC_FLEET_BOTTLENECK_RATIO);
    assert_eq!(metrics_legacy[2].name, METRIC_AGY_QUOTA_REMAINING);
    assert_eq!(metrics_legacy[3].name, METRIC_AGENT_QUOTA_RESET);
    assert_eq!(metrics_legacy[4].name, METRIC_AGY_QUOTA_RESET);
}

#[test]
fn test_enriched_attributes_and_tokens() {
    let json = r#"{
        "conversationId": "test-enrich-123",
        "stepIdx": 5,
        "toolName": "Bash",
        "inputTokens": 1500,
        "outputTokens": 350,
        "cachedTokens": 800,
        "decision": "allow",
        "success": true,
        "userEmail": "engineer@example.com",
        "terminalType": "xterm-256color"
    }"#;

    let input = AntigravityHookInput::parse_slice(json.as_bytes()).expect("parse failed");
    assert_eq!(input.input_tokens, Some(1500));
    assert_eq!(input.output_tokens, Some(350));
    assert_eq!(input.cached_tokens, Some(800));
    assert_eq!(input.decision.as_deref(), Some("allow"));
    assert_eq!(input.success, Some(true));

    let span = build_span_from_hook(HookEvent::PostToolUse, &input, 1000, 2000, 5);

    let find_attr = |key: &str| -> Option<String> {
        span.attributes.iter().find(|kv| kv.key == key).and_then(|kv| {
            kv.value.as_ref().and_then(|v| match &v.value {
                Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(s)) => {
                    Some(s.clone())
                }
                Some(opentelemetry_proto::tonic::common::v1::any_value::Value::IntValue(i)) => {
                    Some(i.to_string())
                }
                Some(opentelemetry_proto::tonic::common::v1::any_value::Value::BoolValue(b)) => {
                    Some(b.to_string())
                }
                _ => None,
            })
        })
    };

    assert_eq!(find_attr(GEN_AI_USAGE_INPUT_TOKENS), Some("1500".to_string()));
    assert_eq!(find_attr(GEN_AI_USAGE_OUTPUT_TOKENS), Some("350".to_string()));
    assert_eq!(find_attr(GEN_AI_USAGE_CACHE_READ_TOKENS), Some("800".to_string()));
    assert_eq!(find_attr(AGENT_DECISION), Some("allow".to_string()));
    assert_eq!(find_attr(AGENT_SUCCESS), Some("true".to_string()));
    assert_eq!(find_attr(USER_EMAIL), Some("engineer@example.com".to_string()));
    assert_eq!(find_attr(TERMINAL_TYPE), Some("xterm-256color".to_string()));
}

#[test]
fn test_w3c_traceparent_parsing() {
    use agent_otel_core::trace_id::{parse_w3c_traceparent, resolve_trace_and_parent_id};

    let valid_header = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
    let parsed = parse_w3c_traceparent(valid_header);
    assert!(parsed.is_some());
    let (trace_id, parent_id) = parsed.unwrap();
    assert_eq!(trace_id[0], 0x4b);
    assert_eq!(trace_id[15], 0x36);
    assert_eq!(parent_id[0], 0x00);
    assert_eq!(parent_id[7], 0xb7);

    // Invalid length
    assert!(parse_w3c_traceparent("00-short-01").is_none());
    // All zeros invalid
    assert!(parse_w3c_traceparent("00-00000000000000000000000000000000-0000000000000000-01").is_none());

    // Resolve traceparent from explicit parameter
    let (resolved_trace, resolved_parent) = resolve_trace_and_parent_id(Some(valid_header), Some("conv-123"));
    assert_eq!(resolved_trace, trace_id);
    assert_eq!(resolved_parent, Some(parent_id));

    // Fallback when no traceparent
    let (fallback_trace, fallback_parent) = resolve_trace_and_parent_id(None, Some("conv-123"));
    assert_ne!(fallback_trace, [0u8; 16]);
    assert_eq!(fallback_parent, None);
}

#[test]
fn test_v02_git_and_execution_mode_attributes() {
    use agent_otel_core::model::ExecutionMode;

    let json = r#"{
        "conversationId": "test-v02-git",
        "stepIdx": 10,
        "mode": "automacao",
        "linesAdded": 42,
        "linesDeleted": 7,
        "filesChanged": 3,
        "selfRevert": true,
        "traceparent": "00-11223344556677889900aabbccddeeff-aabbccddeeff0011-01"
    }"#;

    let input = AntigravityHookInput::parse_slice(json.as_bytes()).expect("parse failed");
    assert_eq!(input.execution_mode, Some(ExecutionMode::Automacao));
    assert_eq!(input.git_lines_added, Some(42));
    assert_eq!(input.git_lines_deleted, Some(7));
    assert_eq!(input.git_files_changed, Some(3));
    assert_eq!(input.git_self_revert, Some(true));

    let span = build_span_from_hook(HookEvent::Stop, &input, 1000, 2000, 1);

    // Span parent_span_id should be adopted from traceparent
    assert_eq!(span.parent_span_id.len(), 8);
    assert_eq!(span.trace_id[0], 0x11);

    let find_attr = |key: &str| -> Option<String> {
        span.attributes.iter().find(|kv| kv.key == key).and_then(|kv| {
            kv.value.as_ref().and_then(|v| match &v.value {
                Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(s)) => {
                    Some(s.clone())
                }
                Some(opentelemetry_proto::tonic::common::v1::any_value::Value::IntValue(i)) => {
                    Some(i.to_string())
                }
                Some(opentelemetry_proto::tonic::common::v1::any_value::Value::BoolValue(b)) => {
                    Some(b.to_string())
                }
                _ => None,
            })
        })
    };

    assert_eq!(find_attr(AGENT_EXECUTION_MODE), Some("automacao".to_string()));
    assert_eq!(find_attr(AGENT_GIT_LINES_ADDED), Some("42".to_string()));
    assert_eq!(find_attr(AGENT_GIT_LINES_DELETED), Some("7".to_string()));
    assert_eq!(find_attr(AGENT_GIT_FILES_CHANGED), Some("3".to_string()));
    assert_eq!(find_attr(AGENT_GIT_SELF_REVERT), Some("true".to_string()));
}


