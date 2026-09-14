/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

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
    metadata: ResolvedSpanMetadata<'_>,
    now_instant: Instant,
    now_unix_nanos: u64,
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

    let mut span = build_span_from_resolved(
        event,
        &input,
        now_unix_nanos,
        now_unix_nanos,
        salt,
        context,
        metadata,
        aliases,
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
