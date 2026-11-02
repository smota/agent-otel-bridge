/*
 * Copyright agent-otel-bridge contributors
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::config::WorkspacePathMode;
use crate::context_cache::{ContextCache, ContextState};
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
    production_transform_with_workspace(
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
        WorkspacePathMode::Absolute,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn production_transform_with_workspace(
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
    workspace_path_mode: WorkspacePathMode,
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
    if input.agent_name.is_none() {
        input.agent_name = ClientKind::from_wire(header.client_id)
            .as_str()
            .map(str::to_owned);
    }
    input.execution_mode.get_or_insert(mode);

    let total_tokens = match (input.input_tokens, input.output_tokens) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0).saturating_add(b.unwrap_or(0)).max(0) as u64),
    };
    let meta = ParsedEventMeta {
        agent_name: input.agent_name.clone(),
        total_tokens,
    };

    let context = resolve_trace_context(
        input.traceparent.as_deref(),
        traceparent,
        input.conversation_id.as_deref(),
    );
    let lookup = contexts.lookup(&input, now_instant);
    input.normalize_with_context(lookup.context.as_ref());
    let mut redacted = None;
    if workspace_path_mode != WorkspacePathMode::Absolute {
        let mut clone = input.clone();
        apply_workspace_path_mode(&mut clone, workspace_path_mode);
        redacted = Some(clone);
    }
    let span_input = redacted.as_ref().unwrap_or(&input);

    let mut span = build_span_from_resolved(
        event,
        span_input,
        now_unix_nanos,
        now_unix_nanos,
        salt,
        context,
        metadata,
        aliases,
        gen_ai_latest_experimental,
    );

    let provided = lookup.context.is_none()
        && input
            .workspace_path
            .as_ref()
            .is_some_and(|p| std::path::Path::new(p).is_absolute());
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

    Ok(ProductionTransformOutcome {
        span,
        meta,
        context_state: state,
        context_source: source,
        context_age_ms: age_ms,
    })
}

fn apply_workspace_path_mode(input: &mut AgentHookInput, mode: WorkspacePathMode) {
    match mode {
        WorkspacePathMode::Absolute => {}
        WorkspacePathMode::Relative => apply_relative_workspace(input),
        WorkspacePathMode::Hash => {
            if let Some(path) = input.workspace_path.clone() {
                input.workspace_path = Some(sha256_hex(&path));
            }
            if let Some(root) = input.project_root.clone() {
                input.project_root = Some(sha256_hex(&root));
            }
        }
    }
}

fn apply_relative_workspace(input: &mut AgentHookInput) {
    let root = input.project_root.clone();
    input.workspace_path = match (input.workspace_path.as_deref(), root.as_deref()) {
        (Some(path), Some(root)) => relative_workspace_path(path, root),
        (Some(_), None) => None,
        (None, _) => None,
    };
    if let Some(root) = root.as_deref() {
        input.project_root = final_path_component(root);
    }
}

fn relative_workspace_path(path: &str, root: &str) -> Option<String> {
    let relative = std::path::Path::new(path)
        .strip_prefix(std::path::Path::new(root))
        .ok()?;
    if relative.as_os_str().is_empty() {
        Some(".".to_string())
    } else {
        relative.to_str().map(str::to_owned)
    }
}

fn final_path_component(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

fn sha256_hex(value: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(value.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push(char::from(b"0123456789abcdef"[usize::from(byte >> 4)]));
        hex.push(char::from(b"0123456789abcdef"[usize::from(byte & 0x0f)]));
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;
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
        apply_workspace_path_mode(&mut input, WorkspacePathMode::Relative);
        assert_eq!(input.workspace_path.as_deref(), Some("src"));
        assert_eq!(input.project_root.as_deref(), Some("repo"));
        assert_eq!(input.project_name.as_deref(), Some("repo"));

        let mut outside = AgentHookInput {
            workspace_path: Some("/work/other".to_string()),
            project_root: Some("/work/repo".to_string()),
            project_name: Some("repo".to_string()),
            ..Default::default()
        };
        apply_workspace_path_mode(&mut outside, WorkspacePathMode::Relative);
        assert_eq!(outside.workspace_path, None);
        assert_eq!(outside.project_root.as_deref(), Some("repo"));
        assert_eq!(outside.project_name.as_deref(), Some("repo"));

        let mut same = AgentHookInput {
            workspace_path: Some("/work/repo".to_string()),
            project_root: Some("/work/repo".to_string()),
            ..Default::default()
        };
        apply_workspace_path_mode(&mut same, WorkspacePathMode::Relative);
        assert_eq!(same.workspace_path.as_deref(), Some("."));
        assert_eq!(same.project_root.as_deref(), Some("repo"));
    }

    #[test]
    fn workspace_hash_is_stable_sha256_and_not_the_raw_path() {
        const PATH: &str = "/work/repo/src";
        const ROOT: &str = "/work/repo";
        const PATH_HASH: &str = "cc324a191f87ae3da5bbdb78642ab3bc058214e6341c6480b674884778ebe947";
        const ROOT_HASH: &str = "ddc5e473a09bd156b4eb7c426367aa59c37909a35241b49fbfa0631b9b532a39";

        let mut input = AgentHookInput {
            workspace_path: Some(PATH.to_string()),
            project_root: Some(ROOT.to_string()),
            project_name: Some("repo".to_string()),
            ..Default::default()
        };
        apply_workspace_path_mode(&mut input, WorkspacePathMode::Hash);
        assert_eq!(input.workspace_path.as_deref(), Some(PATH_HASH));
        assert_eq!(input.project_root.as_deref(), Some(ROOT_HASH));
        assert_ne!(input.workspace_path.as_deref(), Some(PATH));
        assert_eq!(input.project_name.as_deref(), Some("repo"));
        assert_eq!(sha256_hex(PATH), sha256_hex(PATH));
        assert!(PATH_HASH
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(PATH_HASH.len(), 64);

        let mut absolute = AgentHookInput {
            workspace_path: Some(PATH.to_string()),
            project_root: Some(ROOT.to_string()),
            ..Default::default()
        };
        apply_workspace_path_mode(&mut absolute, WorkspacePathMode::Absolute);
        assert_eq!(absolute.workspace_path.as_deref(), Some(PATH));
        assert_eq!(absolute.project_root.as_deref(), Some(ROOT));
    }

    #[test]
    fn production_transform_hashes_workspace_paths_only_in_hash_mode() {
        let contexts = ContextCache::new();
        let header = WireHeader::new(1, HookEvent::PostToolUse.to_wire());
        let json = br#"{"conversationId":"privacy","workspacePath":"/work/repo/src","projectRoot":"/work/repo","projectName":"repo"}"#;
        let frame = encode_context_payload(header, None, json);
        let metadata = ResolvedSpanMetadata::default();
        let hashed = production_transform_with_workspace(
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
        )
        .expect("hash transform");
        assert_eq!(
            string_attr(&hashed.span, "workspace.path"),
            Some("cc324a191f87ae3da5bbdb78642ab3bc058214e6341c6480b674884778ebe947")
        );
        assert_eq!(
            string_attr(&hashed.span, "workspace.project_root"),
            Some("ddc5e473a09bd156b4eb7c426367aa59c37909a35241b49fbfa0631b9b532a39")
        );
        assert_eq!(
            string_attr(&hashed.span, "workspace.project_name"),
            Some("repo")
        );

        let absolute = production_transform(
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
        .expect("absolute transform");
        assert_eq!(
            string_attr(&absolute.span, "workspace.path"),
            Some("/work/repo/src")
        );
        assert_eq!(
            string_attr(&absolute.span, "workspace.project_root"),
            Some("/work/repo")
        );
    }
}
