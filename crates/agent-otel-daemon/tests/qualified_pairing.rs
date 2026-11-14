/* Copyright agent-otel-bridge contributors
 * SPDX-License-Identifier: Apache-2.0 */

use agent_otel_core::model::{ExecutionMode, HookEvent};
use agent_otel_core::otlp::ResolvedSpanMetadata;
use agent_otel_daemon::context_cache::ContextCache;
use agent_otel_daemon::privacy::CaptureMode;
use agent_otel_daemon::span_assembler::SpanAssembler;
use agent_otel_daemon::transform::production_transform;
use agent_otel_ipc::frame::{encode_context_payload, MsgType, WireHeader};
use opentelemetry_proto::tonic::common::v1::any_value;
use opentelemetry_proto::tonic::trace::v1::Span;
use serde_json::json;
use std::time::{Duration, Instant};

fn attr<'a>(span: &'a Span, key: &str) -> Option<&'a str> {
    span.attributes.iter().find_map(|kv| {
        if kv.key != key {
            return None;
        }
        match kv.value.as_ref()?.value.as_ref()? {
            any_value::Value::StringValue(value) => Some(value.as_str()),
            _ => None,
        }
    })
}

fn observe(
    assembler: &mut SpanAssembler,
    event: HookEvent,
    client: u16,
    session: &str,
    call: &str,
    now: Instant,
    nanos: u64,
) -> Vec<Span> {
    observe_body(
        assembler,
        event,
        client,
        &json!({"session_id":session,"tool_name":"Read", "tool_use_id":call, "tool_call":{"id":call,"name":"Read"}}),
        now,
        nanos,
    )
}

fn observe_body(
    assembler: &mut SpanAssembler,
    event: HookEvent,
    client: u16,
    body: &serde_json::Value,
    now: Instant,
    nanos: u64,
) -> Vec<Span> {
    let payload = encode_context_payload(
        WireHeader::new(client, event.to_wire()),
        None,
        &serde_json::to_vec(body).unwrap(),
    );
    let outcome = production_transform(
        MsgType::HookPayloadWithContext,
        &payload,
        &ContextCache::new(),
        nanos as u32,
        ExecutionMode::Interactive,
        false,
        false,
        ResolvedSpanMetadata::default(),
        now,
        nanos,
    )
    .unwrap();
    assembler.observe_qualified(outcome.span, outcome.pairing_identity, now)
}

fn assert_pair(span: &Span, start: u64, end: u64) {
    assert_eq!(span.start_time_unix_nano, start);
    assert_eq!(span.end_time_unix_nano, end);
    assert!(span.attributes.iter().any(|kv| {
        kv.key == "agent.span.paired"
            && kv.value.as_ref().and_then(|v| v.value.as_ref())
                == Some(&any_value::Value::BoolValue(true))
    }));
}

#[test]
fn identical_call_ids_in_two_sessions_are_two_pairs_without_a_privacy_key() {
    let mut assembler = SpanAssembler::default();
    let now = Instant::now();
    for (session, nanos) in [("SESSION_A_SENTINEL", 100), ("SESSION_B_SENTINEL", 200)] {
        assert!(observe(
            &mut assembler,
            HookEvent::PreToolUse,
            2,
            session,
            "CALL SENTINEL",
            now,
            nanos
        )
        .is_empty());
    }
    for (session, start, end) in [
        ("SESSION_B_SENTINEL", 200, 300),
        ("SESSION_A_SENTINEL", 100, 400),
    ] {
        let out = observe(
            &mut assembler,
            HookEvent::PostToolUse,
            2,
            session,
            "CALL SENTINEL",
            now,
            end,
        );
        assert_eq!(out.len(), 1);
        assert_pair(&out[0], start, end);
        let mut span = out[0].clone();
        agent_otel_daemon::privacy::project_span(&mut span, CaptureMode::Minimal);
        assert!(!format!("{span:?}").contains("SENTINEL"));
    }
}

#[test]
fn identical_call_and_session_ids_across_wire_harnesses_are_distinct() {
    let mut assembler = SpanAssembler::default();
    let now = Instant::now();
    for (client, nanos) in [(2, 100), (3, 200)] {
        assert!(observe(
            &mut assembler,
            HookEvent::PreToolUse,
            client,
            "SESSION_SENTINEL",
            "CALL SENTINEL",
            now,
            nanos
        )
        .is_empty());
    }
    for (client, start, end) in [(3, 200, 300), (2, 100, 400)] {
        let out = observe(
            &mut assembler,
            HookEvent::PostToolUse,
            client,
            "SESSION_SENTINEL",
            "CALL SENTINEL",
            now,
            end,
        );
        assert_eq!(out.len(), 1);
        assert_pair(&out[0], start, end);
    }
}

#[test]
fn true_pre_post_pair_exports_exactly_one_span() {
    let mut assembler = SpanAssembler::default();
    let now = Instant::now();
    assert!(observe(
        &mut assembler,
        HookEvent::PreToolUse,
        2,
        "session",
        "call",
        now,
        100
    )
    .is_empty());
    let out = observe(
        &mut assembler,
        HookEvent::PostToolUse,
        2,
        "session",
        "call",
        now,
        200,
    );
    assert_eq!(out.len(), 1);
    assert_pair(&out[0], 100, 200);
}

#[test]
fn ttl_expiry_has_an_explicit_minimal_outcome() {
    let ttl = Duration::from_secs(1);
    let mut assembler = SpanAssembler::new(2, ttl);
    let now = Instant::now();
    assert!(observe(
        &mut assembler,
        HookEvent::PreToolUse,
        2,
        "session",
        "call",
        now,
        100
    )
    .is_empty());
    let out = observe(
        &mut assembler,
        HookEvent::Unknown,
        2,
        "session",
        "other",
        now + ttl,
        200,
    );
    assert_eq!(out.len(), 2);
    let mut expired = out[0].clone();
    agent_otel_daemon::privacy::project_span(&mut expired, CaptureMode::Minimal);
    assert_eq!(
        attr(&expired, "agent.span.outcome"),
        Some("expired_unpaired")
    );
}

#[test]
fn capacity_overflow_evicts_oldest_with_explicit_outcome() {
    let mut assembler = SpanAssembler::new(1, Duration::from_secs(60));
    let now = Instant::now();
    assert!(observe(
        &mut assembler,
        HookEvent::PreToolUse,
        2,
        "session",
        "oldest",
        now,
        100
    )
    .is_empty());
    let out = observe(
        &mut assembler,
        HookEvent::PreToolUse,
        2,
        "session",
        "newest",
        now + Duration::from_millis(1),
        200,
    );
    assert_eq!(out.len(), 1);
    let mut projected = out[0].clone();
    agent_otel_daemon::privacy::project_span(&mut projected, CaptureMode::Minimal);
    assert_eq!(
        attr(&projected, "agent.span.outcome"),
        Some("evicted_unpaired")
    );
    assert_eq!(out[0].start_time_unix_nano, 100);
    assert_eq!(
        attr(&out[0], "agent.span.outcome"),
        Some("evicted_unpaired")
    );
}

#[test]
fn g0a_codex_s2_collision_frames_preserve_wire_identity_and_provenance() {
    // Read-only sanitized capture: codex-0.167.0/S2/hook-sanitized.jsonl.
    // These are synthetic collision probes with client_id=2, not native Codex events.
    let rows: Vec<serde_json::Value> = include_str!("fixtures/g0a-codex-s2-collision.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 4);
    assert_eq!(
        rows.iter().filter(|row| row["origin"] == "harness").count(),
        0
    );
    let now = Instant::now();
    let mut assembler = SpanAssembler::default();
    let mut pairs = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row["header"]["client_id"], 2);
        assert_eq!(row["origin"], "synthetic");
        let mut body = row["stdin"].clone();
        body["origin"] = row["origin"].clone();
        let payload = encode_context_payload(
            WireHeader::new(2, row["header"]["event_id"].as_u64().unwrap() as u8),
            None,
            &serde_json::to_vec(&body).unwrap(),
        );
        let result = production_transform(
            MsgType::HookPayloadWithContext,
            &payload,
            &ContextCache::new(),
            index as u32,
            ExecutionMode::Interactive,
            false,
            false,
            ResolvedSpanMetadata::default(),
            now,
            row["timestamp_ns"].as_u64().unwrap(),
        )
        .unwrap();
        let out = assembler.observe_qualified(result.span, result.pairing_identity, now);
        if index < 2 {
            assert!(out.is_empty());
        }
        pairs.extend(out);
    }
    assert_eq!(pairs.len(), 2);
    for (index, pair) in pairs.iter().enumerate() {
        assert_pair(
            pair,
            rows[index]["timestamp_ns"].as_u64().unwrap(),
            rows[index + 2]["timestamp_ns"].as_u64().unwrap(),
        );
        assert_eq!(attr(pair, "agent.session.link_id"), Some("unlinked"));
        assert!(!format!("{pair:?}").contains("SENTINEL"));
    }
}

#[test]
fn native_tool_use_id_is_available_for_pairing_before_projection() {
    let mut assembler = SpanAssembler::default();
    let now = Instant::now();
    let body: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/codex-native-hook.json")).unwrap();
    for event in [HookEvent::PreToolUse, HookEvent::PostToolUse] {
        let payload = encode_context_payload(
            WireHeader::new(3, event.to_wire()),
            None,
            &serde_json::to_vec(&body).unwrap(),
        );
        let result = production_transform(
            MsgType::HookPayloadWithContext,
            &payload,
            &ContextCache::new(),
            1,
            ExecutionMode::Interactive,
            false,
            false,
            ResolvedSpanMetadata::default(),
            now,
            100,
        )
        .unwrap();
        let out = assembler.observe_qualified(result.span, result.pairing_identity, now);
        if event == HookEvent::PreToolUse {
            assert!(out.is_empty(), "native tool_use_id must open a span");
        } else {
            assert_eq!(out.len(), 1);
            assert_pair(&out[0], 100, 100);
        }
    }
}

#[test]
fn idle_expiry_at_exact_ttl_is_counted_once() {
    let stats = std::sync::Arc::new(agent_otel_daemon::diagnostics::Diagnostics::default());
    let ttl = Duration::from_secs(1);
    let mut assembler = SpanAssembler::with_diagnostics(2, ttl, stats.clone());
    let now = Instant::now();
    assert!(observe(
        &mut assembler,
        HookEvent::PreToolUse,
        2,
        "session",
        "call",
        now,
        100
    )
    .is_empty());
    assert!(assembler
        .expire(now + ttl - Duration::from_nanos(1))
        .is_empty());
    let out = assembler.expire(now + ttl);
    assert_eq!(out.len(), 1);
    assert_eq!(
        attr(&out[0], "agent.span.outcome"),
        Some("expired_unpaired")
    );
    assert_eq!(attr(&out[0], "agent.span.incomplete_reason"), Some("ttl"));
    assert!(assembler.expire(now + ttl + ttl).is_empty());
    assert_eq!(stats.snapshot().span_expired_ttl, 1);
    assert_eq!(stats.snapshot().span_expired_capacity, 0);
}

#[test]
fn equal_clock_capacity_overflow_evicts_first_insertion_and_counts() {
    let stats = std::sync::Arc::new(agent_otel_daemon::diagnostics::Diagnostics::default());
    let mut assembler = SpanAssembler::with_diagnostics(2, Duration::from_secs(60), stats.clone());
    let now = Instant::now();
    for (call, nanos) in [("first", 100), ("second", 200)] {
        assert!(observe(
            &mut assembler,
            HookEvent::PreToolUse,
            2,
            "session",
            call,
            now,
            nanos
        )
        .is_empty());
    }
    let out = observe(
        &mut assembler,
        HookEvent::PreToolUse,
        2,
        "session",
        "third",
        now,
        300,
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].start_time_unix_nano, 100);
    assert_eq!(
        attr(&out[0], "agent.span.incomplete_reason"),
        Some("capacity")
    );
    assert_eq!(stats.snapshot().span_expired_capacity, 1);
    let out = observe(
        &mut assembler,
        HookEvent::PostToolUse,
        2,
        "session",
        "second",
        now,
        400,
    );
    assert_eq!(out.len(), 1);
    assert_pair(&out[0], 200, 400);
}

#[test]
fn shutdown_drains_all_open_spans_once_and_accounts_expired_deadline() {
    for deadline_passed in [false, true] {
        let stats = std::sync::Arc::new(agent_otel_daemon::diagnostics::Diagnostics::default());
        let mut assembler =
            SpanAssembler::with_diagnostics(2, Duration::from_secs(60), stats.clone());
        let now = Instant::now();
        for call in ["first", "second"] {
            assert!(observe(
                &mut assembler,
                HookEvent::PreToolUse,
                2,
                "session",
                call,
                now,
                100
            )
            .is_empty());
        }
        let deadline = if deadline_passed {
            now - Duration::from_secs(1)
        } else {
            now + Duration::from_secs(5)
        };
        let out = assembler.drain_shutdown(deadline);
        assert_eq!(out.len(), if deadline_passed { 0 } else { 2 });
        for span in out {
            assert_eq!(attr(&span, "agent.span.outcome"), Some("drained_unpaired"));
            assert_eq!(
                attr(&span, "agent.span.incomplete_reason"),
                Some("shutdown")
            );
            assert_eq!(span.start_time_unix_nano, span.end_time_unix_nano);
        }
        assert!(assembler.drain_shutdown(deadline).is_empty());
        assert_eq!(
            stats.snapshot().span_shutdown_drained,
            if deadline_passed { 0 } else { 2 }
        );
        assert_eq!(
            stats.snapshot().shutdown_dropped,
            if deadline_passed { 2 } else { 0 }
        );
    }
}

#[test]
fn origins_are_isolated_and_wire_aliases_use_a2_precedence() {
    let now = Instant::now();
    let mut assembler = SpanAssembler::default();
    let mut body =
        json!({"session_id":"session", "tool_use_id":"call", "agent_name":"claude-code"});
    assert!(observe_body(&mut assembler, HookEvent::PreToolUse, 0, &body, now, 100).is_empty());
    body["origin"] = json!("synthetic");
    assert!(observe_body(&mut assembler, HookEvent::PreToolUse, 2, &body, now, 200).is_empty());
    body["agent_name"] = json!("planner");
    let out = observe_body(&mut assembler, HookEvent::PostToolUse, 2, &body, now, 300);
    assert_eq!(out.len(), 1);
    assert_pair(&out[0], 200, 300);
    body["origin"] = json!("harness");
    let out = observe_body(&mut assembler, HookEvent::PostToolUse, 2, &body, now, 400);
    assert_eq!(out.len(), 1);
    assert_pair(&out[0], 100, 400);
}

#[test]
fn missing_sessions_unknown_wire_ids_and_conflicting_calls_fail_open() {
    let now = Instant::now();
    for (client, body) in [
        (2, json!({"tool_use_id":"call"})),
        (2, json!({"session_id":" ","tool_use_id":"call"})),
        (
            99,
            json!({"agent_name":"codex","session_id":"session","tool_use_id":"call"}),
        ),
        (
            2,
            json!({"session_id":"session","tool_use_id":"one","tool_call":{"id":"two"}}),
        ),
    ] {
        let mut assembler = SpanAssembler::default();
        for event in [HookEvent::PreToolUse, HookEvent::PostToolUse] {
            let out = observe_body(&mut assembler, event, client, &body, now, 100);
            assert_eq!(out.len(), 1);
            assert!(out[0]
                .attributes
                .iter()
                .any(|kv| kv == &agent_otel_core::otlp::kv_bool("agent.span.paired", false)));
        }
    }
}

#[test]
fn invocation_and_tool_fallback_keys_include_harness_and_session() {
    let now = Instant::now();
    let scopes = [(2, "a"), (2, "b"), (3, "a"), (3, "b")];
    for (pre, post) in [
        (HookEvent::PreInvocation, HookEvent::PostInvocation),
        (HookEvent::PreToolUse, HookEvent::PostToolUse),
    ] {
        let mut assembler = SpanAssembler::default();
        for (index, (client, session)) in scopes.iter().enumerate() {
            let body =
                json!({"session_id":session,"step_idx":7,"tool_name":"PRIVATE TOOL SENTINEL"});
            assert!(
                observe_body(&mut assembler, pre, *client, &body, now, 100 + index as u64)
                    .is_empty()
            );
        }
        for (index, (client, session)) in scopes.iter().enumerate().rev() {
            let body =
                json!({"session_id":session,"step_idx":7,"tool_name":"PRIVATE TOOL SENTINEL"});
            let out = observe_body(
                &mut assembler,
                post,
                *client,
                &body,
                now,
                200 + index as u64,
            );
            assert_eq!(out.len(), 1);
            assert_pair(&out[0], 100 + index as u64, 200 + index as u64);
            assert!(!format!("{:?}", out[0]).contains("SENTINEL"));
        }
    }
}

#[test]
fn private_native_identity_is_never_serialized_or_stamped() {
    let body = json!({"session_id":"SESSION SENTINEL", "tool_use_id":"CALL SENTINEL"});
    let input =
        agent_otel_core::model::AgentHookInput::parse_slice(&serde_json::to_vec(&body).unwrap())
            .unwrap();
    assert!(!serde_json::to_string(&input)
        .unwrap()
        .contains("CALL SENTINEL"));
    for mode in [
        CaptureMode::Minimal,
        CaptureMode::MetadataOnly,
        CaptureMode::Full,
    ] {
        let mut input = input.clone();
        let identity = agent_otel_daemon::span_assembler::PairingIdentity::from_input(
            2,
            HookEvent::PreToolUse,
            &input,
        );
        assert!(!format!("{identity:?}").contains("SENTINEL"));
        agent_otel_daemon::privacy::project_input_and_metadata(
            &mut input,
            &mut ResolvedSpanMetadata::default(),
            mode,
            agent_otel_daemon::config::WorkspacePathMode::Hash,
            None,
        );
        assert!(input.native_tool_call_id.is_none());
    }
}

#[test]
fn native_session_aliases_accept_equal_values_and_reject_conflicts() {
    use agent_otel_core::model::AgentHookInput;
    let accepted = br#"{"conversation_id":"SESSION SENTINEL","session_id":"SESSION SENTINEL","sessionId":"SESSION SENTINEL"}"#;
    assert_eq!(
        AgentHookInput::parse_slice(accepted)
            .unwrap()
            .conversation_id
            .as_deref(),
        Some("SESSION SENTINEL")
    );
    for rejected in [
        br#"{"conversation_id":"one","session_id":"two"}"#.as_slice(),
        br#"{"conversationId":"one","session_id":"two"}"#.as_slice(),
        br#"{"session_id":"one","session_id":"one"}"#.as_slice(),
    ] {
        assert!(AgentHookInput::parse_slice(rejected).is_err());
    }
}

#[test]
fn stop_without_identity_or_open_invocation_remains_silent() {
    let now = Instant::now();
    let mut assembler = SpanAssembler::default();
    for client in [0, 2, 99] {
        for body in [json!({}), json!({"session_id":"session"})] {
            assert!(
                observe_body(&mut assembler, HookEvent::Stop, client, &body, now, 100).is_empty()
            );
        }
    }
}

#[test]
fn equal_native_call_aliases_pair_including_all_three() {
    for aliases in [
        json!({"tool_use_id":"call", "call_id":"call"}),
        json!({"tool_use_id":"call", "tool_call_id":"call"}),
        json!({"tool_call_id":"call", "call_id":"call"}),
        json!({"tool_use_id":"call", "tool_call_id":"call", "call_id":"call"}),
    ] {
        let mut body = aliases;
        body["session_id"] = json!("session");
        // Exercise both compatibility alias groups in the same retry.
        body["conversation_id"] = json!("session");
        let mut assembler = SpanAssembler::default();
        let now = Instant::now();
        assert!(observe_body(&mut assembler, HookEvent::PreToolUse, 3, &body, now, 100).is_empty());
        let out = observe_body(&mut assembler, HookEvent::PostToolUse, 3, &body, now, 200);
        assert_eq!(out.len(), 1);
        assert_pair(&out[0], 100, 200);
    }
}

#[test]
fn conflicting_native_call_aliases_export_unpaired_even_with_fallback_fields() {
    for aliases in [
        json!({"tool_use_id":"one", "call_id":"two"}),
        json!({"tool_use_id":"one", "tool_call_id":"two"}),
        json!({"tool_call_id":"one", "call_id":"two"}),
        json!({"tool_use_id":"one", "tool_call_id":"one", "call_id":"two"}),
    ] {
        for fallback in [false, true] {
            let mut body = aliases.clone();
            body["session_id"] = json!("session");
            if fallback {
                body["step_idx"] = json!(7);
                body["tool_name"] = json!("Read");
                body["tool_call"] = json!({"id":"one"});
            }
            let input = agent_otel_core::model::AgentHookInput::parse_slice(
                &serde_json::to_vec(&body).unwrap(),
            )
            .unwrap();
            assert!(input.native_tool_call_id.is_none());
            let mut assembler = SpanAssembler::default();
            let now = Instant::now();
            for event in [HookEvent::PreToolUse, HookEvent::PostToolUse] {
                let out = observe_body(&mut assembler, event, 3, &body, now, 100);
                assert_eq!(out.len(), 1, "ambiguous IDs must retain every event");
                assert!(out[0]
                    .attributes
                    .iter()
                    .any(|kv| kv == &agent_otel_core::otlp::kv_bool("agent.span.paired", false)));
            }
            assert!(assembler
                .drain_shutdown(now + Duration::from_secs(1))
                .is_empty());
        }
    }
}

#[test]
fn shutdown_drains_by_arrival_then_insertion_order() {
    let mut assembler = SpanAssembler::default();
    let now = Instant::now();
    for (call, opened, nanos) in [
        ("late", now + Duration::from_millis(1), 300),
        ("first", now, 100),
        ("second", now, 200),
    ] {
        assert!(observe(
            &mut assembler,
            HookEvent::PreToolUse,
            2,
            "session",
            call,
            opened,
            nanos
        )
        .is_empty());
    }
    let out = assembler.drain_shutdown(now + Duration::from_secs(5));
    assert_eq!(
        out.iter()
            .map(|span| span.start_time_unix_nano)
            .collect::<Vec<_>>(),
        vec![100, 200, 300]
    );
}
