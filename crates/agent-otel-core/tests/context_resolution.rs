use agent_otel_core::model::{AgentHookInput, HookEvent};
use agent_otel_core::otlp::build_span_from_hook_with_context_opts;
use agent_otel_core::trace_id::{
    parse_w3c_traceparent, parse_w3c_traceparent_with_flags, resolve_trace_context,
};

const PAYLOAD: &str = "00-11111111111111111111111111111111-2222222222222222-00";
const ORIGIN: &str = "00-33333333333333333333333333333333-4444444444444444-01";

#[test]
fn resolution_is_pure_and_has_event_precedence() {
    let context = resolve_trace_context(Some(PAYLOAD), Some(ORIGIN), Some("conversation-a"));
    assert_eq!(context.trace_id, [0x11; 16]);
    assert_eq!(context.parent_span_id, Some([0x22; 8]));
    assert_eq!(context.trace_flags, 0);

    let origin = resolve_trace_context(Some("invalid"), Some(ORIGIN), Some("conversation-a"));
    assert_eq!(origin.trace_id, [0x33; 16]);
    assert_eq!(origin.parent_span_id, Some([0x44; 8]));
    assert_eq!(origin.trace_flags, 1);

    let fallback = resolve_trace_context(
        Some("invalid"),
        Some("also-invalid"),
        Some("conversation-a"),
    );
    assert_ne!(fallback.trace_id, [0; 16]);
    assert_eq!(fallback.parent_span_id, None);
}

#[test]
fn w3c_parser_rejects_malformed_and_unicode_without_panicking() {
    for malformed in [
        "00-11111111111111111111111111111111-2222222222222222-0g",
        "00-11111111111111111111111111111111-2222222222222222-01-extra",
        "ff-11111111111111111111111111111111-2222222222222222-01",
        "00-00000000000000000000000000000000-2222222222222222-01",
        "00-11111111111111111111111111111111-0000000000000000-01",
        "00-11111111111111111111111111111111-2222222222222222-01é",
        "00-11111111111111111111111111111111-2222222222222222-01-\u{1f642}",
        "00-1111111111111111111111111111111A-2222222222222222-01",
    ] {
        assert!(
            parse_w3c_traceparent_with_flags(malformed).is_none(),
            "{malformed}"
        );
    }

    assert!(parse_w3c_traceparent(
        "01-11111111111111111111111111111111-2222222222222222-01-vendor"
    )
    .is_some());
}

#[test]
fn resolved_context_preserves_unsampled_flag_without_creating_agent_lineage() {
    let mut input = AgentHookInput {
        conversation_id: Some("conversation-a".to_string()),
        ..Default::default()
    };
    input.auto_enrich_with_resolved_context();
    assert_eq!(input.agent_depth, None);
    assert_eq!(input.agent_is_root, None);

    let span = build_span_from_hook_with_context_opts(
        HookEvent::PreToolUse,
        &input,
        10,
        20,
        1,
        resolve_trace_context(Some(PAYLOAD), None, input.conversation_id.as_deref()),
        false,
    );
    assert_eq!(span.trace_id, vec![0x11; 16]);
    assert_eq!(span.parent_span_id, vec![0x22; 8]);
    assert_eq!(span.flags, 0);
}
