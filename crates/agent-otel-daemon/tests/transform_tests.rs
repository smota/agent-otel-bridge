/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_core::model::{ExecutionMode, HookEvent};
use agent_otel_core::otlp::ResolvedSpanMetadata;
use agent_otel_daemon::context_cache::ContextCache;
use agent_otel_daemon::transform::{production_transform, TransformError};
use agent_otel_ipc::frame::{encode_context_payload, MsgType, WireHeader};
use std::time::Instant;

#[test]
fn test_production_transform_0x04_envelope() {
    let contexts = ContextCache::new();
    let header = WireHeader::new(1, HookEvent::PostToolUse.to_wire()); // client_id 1 = Antigravity
    let traceparent = "00-11223344556677889900aabbccddeeff-aabbccddeeff0011-01";
    let json = br#"{"conversationId":"sess-test-01","stepIdx":42,"toolCall":{"name":"read_file","id":"call_1"},"inputTokens":200,"outputTokens":80}"#;
    let frame = encode_context_payload(header, Some(traceparent), json);

    let metadata = ResolvedSpanMetadata {
        user_email: Some("tester@example.invalid"),
        terminal_type: Some("test-terminal"),
    };

    let outcome = production_transform(
        MsgType::HookPayloadWithContext,
        &frame,
        &contexts,
        1,
        ExecutionMode::Interactive,
        false,
        metadata,
        Instant::now(),
        1_700_000_000_000_000_000,
    )
    .expect("production transform must succeed");

    assert_eq!(outcome.meta.agent_name.as_deref(), Some("antigravity"));
    assert_eq!(outcome.meta.total_tokens, Some(280));
    assert_eq!(outcome.context_state, "missing");
    assert_eq!(outcome.context_source, "none");
    assert_eq!(outcome.span.name, "execute_tool read_file");
    assert_eq!(
        outcome.span.trace_id,
        [
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0x00, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ]
    );
    assert_eq!(
        outcome.span.parent_span_id,
        [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00, 0x11]
    );
    assert_eq!(
        outcome.span.span_id,
        [0x40, 0x56, 0xc8, 0xe7, 0xde, 0x79, 0xce, 0x7c]
    );
    assert!(outcome
        .span
        .attributes
        .iter()
        .any(|kv| kv.key == "agent.context.state"));
    assert!(outcome
        .span
        .attributes
        .iter()
        .any(|kv| kv.key == "gen_ai.agent.name"
            && kv.value.as_ref().unwrap().value.as_ref().is_some()));
}

#[test]
fn test_production_transform_0x01_legacy_frame() {
    let contexts = ContextCache::new();
    let header = WireHeader::new(1, HookEvent::PreInvocation.to_wire());
    let json = br#"{"conversationId":"sess-test-02","stepIdx":1}"#;
    let mut body = header.encode().to_vec();
    body.extend_from_slice(json);

    let metadata = ResolvedSpanMetadata {
        user_email: None,
        terminal_type: None,
    };

    let outcome = production_transform(
        MsgType::HookPayload,
        &body,
        &contexts,
        2,
        ExecutionMode::Automation,
        false,
        metadata,
        Instant::now(),
        1_700_000_000_000_000_000,
    )
    .expect("production transform must succeed for 0x01");

    assert_eq!(outcome.span.name, "invoke_agent antigravity");
    assert_eq!(outcome.meta.total_tokens, None);
}

#[test]
fn test_production_transform_invalid_inputs() {
    let contexts = ContextCache::new();
    let metadata = ResolvedSpanMetadata::default();

    let res_env = production_transform(
        MsgType::HookPayloadWithContext,
        &[0x00],
        &contexts,
        1,
        ExecutionMode::Interactive,
        false,
        metadata,
        Instant::now(),
        1_000,
    );
    assert_eq!(res_env.err(), Some(TransformError::InvalidEnvelope));

    let bad_json_frame = encode_context_payload(
        WireHeader::new(1, HookEvent::PostToolUse.to_wire()),
        None,
        b"not a json",
    );
    let res_json = production_transform(
        MsgType::HookPayloadWithContext,
        &bad_json_frame,
        &contexts,
        2,
        ExecutionMode::Interactive,
        false,
        metadata,
        Instant::now(),
        1_000,
    );
    assert_eq!(res_json.err(), Some(TransformError::InvalidJson));
}
