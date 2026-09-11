/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_core::model::{AntigravityHookInput, HookEvent};
use agent_otel_core::otlp::{build_resource, build_span_from_hook, build_trace_request};
use agent_otel_core::quota::{build_quota_metrics_request, QuotaSnapshot};
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

    let sid1 = derive_span_id(Some(conv), Some(1), HookEvent::PostToolUse, Some("grep_search"), 0);
    let sid2 = derive_span_id(Some(conv), Some(1), HookEvent::PostToolUse, Some("grep_search"), 0);
    let sid3 = derive_span_id(Some(conv), Some(2), HookEvent::PostToolUse, Some("grep_search"), 0);
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

    assert_eq!(span.name, "agy.post_tool_use run_command");
    assert_eq!(span.start_time_unix_nano, 1_000_000);
    assert_eq!(span.end_time_unix_nano, 2_000_000);

    let has_system = span.attributes.iter().any(|kv| kv.key == GEN_AI_SYSTEM);
    let has_model = span.attributes.iter().any(|kv| kv.key == GEN_AI_REQUEST_MODEL);
    let has_tool = span.attributes.iter().any(|kv| kv.key == GEN_AI_TOOL_NAME);
    assert!(has_system);
    assert!(has_model);
    assert!(has_tool);

    let res = build_resource("antigravity-cli", "homelab", "0.1.0");
    let req = build_trace_request(res, vec![span]);
    assert_eq!(req.resource_spans.len(), 1);
}

#[test]
fn test_quota_metrics_request() {
    let res = build_resource("antigravity-cli", "homelab", "0.1.0");
    let snapshot = QuotaSnapshot {
        remaining_fraction: 0.85,
        seconds_to_reset: 3600.0,
        observed_at_unix_nano: 1_700_000_000_000_000_000,
        bucket: "gemini-weekly".to_string(),
        group: "gemini".to_string(),
    };

    let req = build_quota_metrics_request(res, &snapshot);
    let metrics = &req.resource_metrics[0].scope_metrics[0].metrics;
    assert_eq!(metrics.len(), 2);
    assert_eq!(metrics[0].name, QUOTA_REMAINING_FRACTION);
    assert_eq!(metrics[1].name, QUOTA_SECONDS_TO_RESET);
}
