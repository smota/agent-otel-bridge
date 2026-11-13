/*
 * Copyright agent-otel-bridge contributors
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::config::WorkspacePathMode;
use crate::context_cache::{ContextCache, ContextState};
use crate::privacy::{project_input_and_metadata, project_span, CaptureMode};
use agent_otel_core::model::{AgentHookInput, ClientKind, ExecutionMode, HookEvent};
use agent_otel_core::otlp::{build_span_from_resolved, kv_int, kv_string, ResolvedSpanMetadata};
use agent_otel_core::trace_id::resolve_trace_context;
use agent_otel_ipc::frame::{decode_context_payload, MsgType, WireHeader};
use opentelemetry_proto::tonic::trace::v1::Span;
use std::fmt;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformError {
    InvalidEnvelope,
    InvalidJson,
}

impl fmt::Display for TransformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransformError::InvalidEnvelope => write!(f, "invalid wire frame envelope"),
            TransformError::InvalidJson => write!(f, "failed to parse hook json payload"),
        }
    }
}

impl std::error::Error for TransformError {}

#[derive(Debug, Clone)]
pub struct ParsedEventMeta {
    pub agent_name: Option<String>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ProductionTransformOutcome {
    pub span: Span,
    pub meta: ParsedEventMeta,
    pub context_state: &'static str,
    pub context_source: &'static str,
    pub context_age_ms: Option<i64>,
}

/// Decode wire payload from MsgType::HookPayload (0x01) or MsgType::HookPayloadWithContext (0x04).
#[inline]
pub fn decode_wire_payload(
    msg_type: MsgType,
    payload: &[u8],
) -> Option<(WireHeader, Option<&str>, &[u8])> {
    match msg_type {
        MsgType::HookPayload => {
            WireHeader::decode(payload).map(|h| (h, None, &payload[WireHeader::LEN..]))
        }
        MsgType::HookPayloadWithContext => decode_context_payload(payload).ok(),
        _ => None,
    }
}

/// Shared production transformation that processes a wire frame into an OTLP Span
/// using ContextCache without filesystem/blocking side effects.
/// Returns the built Span, parsed token/identity metadata (for caller quota accounting),
/// and context diagnostics.
///
/// Boundary: excludes ambient clock, salt advancement, quota state mutations, span batching, queueing, and OTLP network export.
// Explicit clock/configuration inputs keep the shared benchmark boundary free
// of ambient reads and identical to the daemon's per-event transformation.
#[allow(clippy::too_many_arguments)]
pub fn production_transform(
    msg_type: MsgType,
    payload: &[u8],
    contexts: &ContextCache,
    salt: u32,
    mode: ExecutionMode,
    aliases: bool,
    gen_ai_latest_experimental: bool,
    metadata: ResolvedSpanMetadata<'_>,
    now_instant: Instant,
    now_unix_nanos: u64,
) -> Result<ProductionTransformOutcome, TransformError> {
    production_transform_with_capture_mode(
        msg_type,
        payload,
        contexts,
        salt,
        mode,
        aliases,
        gen_ai_latest_experimental,
        metadata,
        now_instant,
        now_unix_nanos,
        WorkspacePathMode::Hash,
        CaptureMode::Minimal,
        None,
    )
}

/// Shared production transformation with explicit [`WorkspacePathMode`] and [`CaptureMode`].
#[allow(clippy::too_many_arguments)]
pub fn production_transform_with_capture_mode(
    msg_type: MsgType,
    payload: &[u8],
    contexts: &ContextCache,
    salt: u32,
    mode: ExecutionMode,
    aliases: bool,
    gen_ai_latest_experimental: bool,
    mut metadata: ResolvedSpanMetadata<'_>,
    now_instant: Instant,
    now_unix_nanos: u64,
    workspace_path_mode: WorkspacePathMode,
    capture_mode: CaptureMode,
    pseudonym_key: Option<&crate::pseudonym_key::PseudonymKey>,
) -> Result<ProductionTransformOutcome, TransformError> {
    let (header, traceparent, bytes) =
        decode_wire_payload(msg_type, payload).ok_or(TransformError::InvalidEnvelope)?;

    let mut input = AgentHookInput::parse_slice(bytes).map_err(|_| TransformError::InvalidJson)?;

    let mut event = HookEvent::from_wire(header.event_id);
    if event == HookEvent::Unknown {
        if let Some(name) = &input.hook_event_name {
            event = HookEvent::from_str_name(name);
        }
    }
    let client_kind = ClientKind::from_wire(header.client_id);
    input.harness_provider = client_kind.default_provider();
    if input.agent_name.is_none() {
        input.agent_name = client_kind.as_str().map(str::to_owned);
    }
    input.execution_mode.get_or_insert(mode);

    let total_tokens = match (input.input_tokens, input.output_tokens) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0).saturating_add(b.unwrap_or(0)).max(0) as u64),
    };

    let context = resolve_trace_context(
        input.traceparent.as_deref(),
        traceparent,
        input.conversation_id.as_deref(),
    );
    let lookup = contexts.lookup(&input, now_instant);
    input.normalize_with_context(lookup.context.as_ref());

    let provided = lookup.context.is_none()
        && input
            .workspace_path
            .as_ref()
            .is_some_and(|p| std::path::Path::new(p).is_absolute());

    project_input_and_metadata(
        &mut input,
        &mut metadata,
        capture_mode,
        workspace_path_mode,
        pseudonym_key,
    );

    let meta = ParsedEventMeta {
        agent_name: input.agent_name.clone(),
        total_tokens,
    };

    let mut span = build_span_from_resolved(
        event,
        &input,
        now_unix_nanos,
        now_unix_nanos,
        salt,
        context,
        metadata,
        aliases,
        gen_ai_latest_experimental,
    );

    let state = if provided {
        "provided"
    } else {
        match lookup.state {
            ContextState::Fresh => "fresh",
            ContextState::Stale => "stale",
            ContextState::Missing => "missing",
        }
    };
    let source = if lookup.context.is_some() {
        "workspace_cache"
    } else if provided {
        "event"
    } else {
        "none"
    };
    let age_ms = if !provided && lookup.context.is_some() {
        lookup
            .age
            .map(|age| age.as_millis().min(i64::MAX as u128) as i64)
    } else {
        None
    };

    span.attributes
        .push(kv_string("agent.context.state", state));
    span.attributes
        .push(kv_string("agent.context.source", source));
    if let Some(age_val) = age_ms {
        span.attributes
            .push(kv_int("agent.context.age_ms", age_val));
    }

    if capture_mode == CaptureMode::Minimal && input.agent_name.is_none() {
        // Span construction has a display-name fallback; a failed custom-name
        // projection must omit the identity attribute rather than export a substitute.
        span.attributes.retain(|kv| kv.key != "gen_ai.agent.name");
    }
    project_span(&mut span, capture_mode);

    Ok(ProductionTransformOutcome {
        span,
        meta,
        context_state: state,
        context_source: source,
        context_age_ms: age_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy::apply_workspace_path_mode;
    use agent_otel_ipc::frame::{encode_context_payload, WireHeader};
    use opentelemetry_proto::tonic::common::v1::any_value;

    fn string_attr<'a>(
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

    #[test]
    fn relative_workspace_path_drops_unrelativizable_paths() {
        let mut input = AgentHookInput {
            workspace_path: Some("/work/repo/src".to_string()),
            project_root: Some("/work/repo".to_string()),
            project_name: Some("repo".to_string()),
            ..Default::default()
        };
        apply_workspace_path_mode(&mut input, WorkspacePathMode::Relative, None);
        assert_eq!(input.workspace_path.as_deref(), Some("src"));
        assert_eq!(input.project_root.as_deref(), Some("repo"));
        assert_eq!(input.project_name.as_deref(), Some("repo"));

        let mut outside = AgentHookInput {
            workspace_path: Some("/work/other".to_string()),
            project_root: Some("/work/repo".to_string()),
            project_name: Some("repo".to_string()),
            ..Default::default()
        };
        apply_workspace_path_mode(&mut outside, WorkspacePathMode::Relative, None);
        assert_eq!(outside.workspace_path, None);
        assert_eq!(outside.project_root.as_deref(), Some("repo"));
        assert_eq!(outside.project_name.as_deref(), Some("repo"));

        let mut same = AgentHookInput {
            workspace_path: Some("/work/repo".to_string()),
            project_root: Some("/work/repo".to_string()),
            ..Default::default()
        };
        apply_workspace_path_mode(&mut same, WorkspacePathMode::Relative, None);
        assert_eq!(same.workspace_path.as_deref(), Some("."));
        assert_eq!(same.project_root.as_deref(), Some("repo"));
    }

    #[test]
    fn hash_workspace_paths_fail_closed_without_key() {
        let mut input = AgentHookInput {
            workspace_path: Some("/work/repo/src".to_string()),
            project_root: Some("/work/repo".to_string()),
            ..Default::default()
        };
        apply_workspace_path_mode(&mut input, WorkspacePathMode::Hash, None);
        assert_eq!(input.workspace_path, None);
        assert_eq!(input.project_root, None);
    }

    #[test]
    fn production_transform_omits_workspace_paths_in_default_minimal_and_respects_opt_in_modes() {
        let contexts = ContextCache::new();
        let header = WireHeader::new(1, HookEvent::PostToolUse.to_wire());
        let json = br#"{"conversationId":"privacy","workspacePath":"/work/repo/src","projectRoot":"/work/repo","projectName":"repo"}"#;
        let frame = encode_context_payload(header, None, json);
        let metadata = ResolvedSpanMetadata::default();
        let default_minimal = production_transform(
            MsgType::HookPayloadWithContext,
            &frame,
            &contexts,
            1,
            ExecutionMode::Interactive,
            false,
            false,
            metadata,
            Instant::now(),
            1,
        )
        .expect("default minimal transform");
        assert_eq!(string_attr(&default_minimal.span, "workspace.path"), None);
        assert_eq!(
            string_attr(&default_minimal.span, "workspace.project_root"),
            None
        );
        assert_eq!(
            string_attr(&default_minimal.span, "workspace.project_name"),
            None
        );

        let metadata_hashed = production_transform_with_capture_mode(
            MsgType::HookPayloadWithContext,
            &frame,
            &contexts,
            1,
            ExecutionMode::Interactive,
            false,
            false,
            metadata,
            Instant::now(),
            1,
            WorkspacePathMode::Hash,
            CaptureMode::MetadataOnly,
            None,
        )
        .expect("metadata_only hash transform");
        assert_eq!(string_attr(&metadata_hashed.span, "workspace.path"), None);
        assert_eq!(
            string_attr(&metadata_hashed.span, "workspace.project_root"),
            None
        );
        assert_eq!(
            string_attr(&metadata_hashed.span, "workspace.project_name"),
            Some("repo")
        );

        let metadata_absolute = production_transform_with_capture_mode(
            MsgType::HookPayloadWithContext,
            &frame,
            &contexts,
            1,
            ExecutionMode::Interactive,
            false,
            false,
            metadata,
            Instant::now(),
            1,
            WorkspacePathMode::Absolute,
            CaptureMode::MetadataOnly,
            None,
        )
        .expect("metadata_only absolute transform");
        assert_eq!(
            string_attr(&metadata_absolute.span, "workspace.path"),
            Some("/work/repo/src")
        );
        assert_eq!(
            string_attr(&metadata_absolute.span, "workspace.project_root"),
            Some("/work/repo")
        );
        assert_eq!(
            string_attr(&metadata_absolute.span, "workspace.project_name"),
            Some("repo")
        );
    }

    #[test]
    fn production_transform_g0a_claude_2_1_322_wire_frame_attributes_anthropic_provider() {
        let contexts = ContextCache::new();
        // client_id = 2 (ClientKind::ClaudeCode), event_id = 3 (HookEvent::PreToolUse)
        let header = WireHeader::new(2, HookEvent::PreToolUse.to_wire());
        let json = br#"{"cwd":"PATH_e79ac6f0cf8bf601","hook_event_name":"PreToolUse","permission_mode":"<str:len=17>","session_id":"CONV_820267f0fb8c34dd","tool_input":{"command":"<str:len=8>","description":"<str:len=28>"},"tool_name":"Bash","tool_use_id":"CALL_aa5a5943c17ccbb4","transcript_path":"PATH_575cc059f0f9d4a9"}"#;
        let frame = encode_context_payload(header, None, json);
        let outcome = production_transform(
            MsgType::HookPayloadWithContext,
            &frame,
            &contexts,
            1,
            ExecutionMode::Interactive,
            false,
            false,
            ResolvedSpanMetadata::default(),
            Instant::now(),
            1,
        )
        .expect("transform G0a Claude 2.1.322 frame");

        assert_eq!(
            string_attr(&outcome.span, "gen_ai.agent.name"),
            Some("claude-code")
        );
        assert_eq!(
            string_attr(&outcome.span, "gen_ai.provider.name"),
            Some("anthropic")
        );
        assert_eq!(
            string_attr(&outcome.span, "gen_ai.system"),
            Some("anthropic")
        );
        assert_eq!(string_attr(&outcome.span, "gen_ai.request.model"), None);
    }

    #[test]
    fn production_transform_header_client_kind_attributes_provider_when_payload_sets_subagent_name()
    {
        let contexts = ContextCache::new();
        // client_id = 2 (ClientKind::ClaudeCode), payload supplies subagent agent_name = "planner" and no model
        let header = WireHeader::new(2, HookEvent::PreToolUse.to_wire());
        let json = br#"{"agent_name":"planner","hook_event_name":"PreToolUse","session_id":"CONV_subagent","tool_name":"Bash"}"#;
        let frame = encode_context_payload(header, None, json);
        let outcome = production_transform(
            MsgType::HookPayloadWithContext,
            &frame,
            &contexts,
            1,
            ExecutionMode::Interactive,
            false,
            false,
            ResolvedSpanMetadata::default(),
            Instant::now(),
            1,
        )
        .expect("transform subagent frame with client_id=2");

        assert_eq!(
            string_attr(&outcome.span, "gen_ai.agent.name"),
            Some("planner")
        );
        assert_eq!(outcome.meta.agent_name.as_deref(), Some("planner"));
        assert_eq!(
            string_attr(&outcome.span, "gen_ai.provider.name"),
            Some("anthropic")
        );
        assert_eq!(
            string_attr(&outcome.span, "gen_ai.system"),
            Some("anthropic")
        );
        assert_eq!(string_attr(&outcome.span, "gen_ai.request.model"), None);
    }
    #[test]
    fn unavailable_key_omits_pseudonymized_fields_but_preserves_span() {
        let json = br#"{"conversationId":"safe-session","agent_name":"private agent","toolName":"private tool","projectName":"private project","vcs.repository.name":"https://example.com/private-repo.git","capabilityNamespace":"private namespace","capabilityName":"private capability","workspacePath":"/private/path","projectRoot":"/private/root","toolArchetype":"private archetype","capabilityKind":"private kind"}"#;
        let frame = encode_context_payload(
            WireHeader::new(2, HookEvent::PreToolUse.to_wire()),
            None,
            json,
        );
        let result = production_transform_with_capture_mode(
            MsgType::HookPayloadWithContext,
            &frame,
            &ContextCache::new(),
            1,
            ExecutionMode::Interactive,
            false,
            false,
            ResolvedSpanMetadata::default(),
            Instant::now(),
            1,
            WorkspacePathMode::Hash,
            CaptureMode::Minimal,
            None,
        )
        .unwrap();
        for key in [
            "gen_ai.agent.name",
            "gen_ai.tool.name",
            "workspace.project_name",
            "vcs.repository.name",
            "capability.namespace",
            "capability.name",
            "workspace.path",
            "workspace.project_root",
        ] {
            assert_eq!(string_attr(&result.span, key), None, "{key}");
        }
        assert_eq!(
            string_attr(&result.span, "agent.tool.archetype"),
            Some("other")
        );
        assert_eq!(string_attr(&result.span, "capability.kind"), Some("other"));
        assert!(!format!("{:?}", result.span).contains("private"));
    }
}
