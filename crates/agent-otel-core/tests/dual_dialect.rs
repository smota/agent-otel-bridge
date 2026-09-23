use agent_otel_core::model::AgentHookInput;

#[test]
fn empty_protojson_error_does_not_mark_successful_tools_as_failed() {
    use agent_otel_core::{model::HookEvent, otlp::build_span_from_hook};
    for error in ["", " ", "actual failure"] {
        let mut input = AgentHookInput {
            error: Some(error.to_string()),
            ..Default::default()
        };
        input.normalize_with_context(None);
        let span = build_span_from_hook(HookEvent::PostToolUse, &input, 100, 200, 1);
        assert_eq!(
            span.status.unwrap().code,
            if error.trim().is_empty() { 1 } else { 2 }
        );
        if error.trim().is_empty() {
            assert!(input.error_category.is_none());
        }
    }
}

#[test]
fn equivalent_grok_aliases_preserve_event_and_tool_input() {
    let input = AgentHookInput::parse_slice(br#"{"sessionId":"s","session_id":"s","hookEventName":"pre_tool_use","hook_event_name":"PreToolUse","toolName":"read_file","tool_name":"read_file","toolInput":{"path":"fixture"},"tool_input":{"path":"fixture"},"transcriptPath":"t","transcript_path":"t"}"#).unwrap();
    assert_eq!(input.conversation_id.as_deref(), Some("s"));
    assert_eq!(input.hook_event_name.as_deref(), Some("pre_tool_use"));
    assert_eq!(input.resolved_tool_name(), Some("read_file"));
    assert_eq!(input.tool_input.unwrap()["path"], "fixture");
}

#[test]
fn conflicting_aliases_and_repeated_keys_remain_invalid() {
    for payload in [
        r#"{"sessionId":"a","session_id":"b"}"#,
        r#"{"sessionId":"a","sessionId":"b"}"#,
        r#"{"sessionId":"a","session_id":"a","toolName":"one","toolName":"two"}"#,
    ] {
        assert!(AgentHookInput::parse_slice(payload.as_bytes()).is_err());
    }
}
