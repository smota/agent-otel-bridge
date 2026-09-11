/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::model::HookEvent;
use sha2::{Digest, Sha256};

pub fn derive_trace_id(conversation_id: Option<&str>) -> [u8; 16] {
    let mut hasher = Sha256::new();
    match conversation_id {
        Some(cid) if !cid.trim().is_empty() => {
            hasher.update(b"agy-otel:trace_id:");
            hasher.update(cid.trim().as_bytes());
        }
        _ => {
            hasher.update(b"agy-otel:trace_id:fallback:");
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            hasher.update(now.to_le_bytes());
        }
    }
    let result = hasher.finalize();
    let mut trace_id = [0u8; 16];
    trace_id.copy_from_slice(&result[0..16]);
    if trace_id == [0u8; 16] {
        trace_id[0] = 0x01;
    }
    trace_id
}

pub fn derive_span_id(
    conversation_id: Option<&str>,
    step_idx: Option<u64>,
    event: HookEvent,
    tool_name: Option<&str>,
    salt: u32,
) -> [u8; 8] {
    let mut hasher = Sha256::new();
    hasher.update(b"agy-otel:span_id:");
    if let Some(cid) = conversation_id {
        hasher.update(cid.as_bytes());
    }
    hasher.update(b":");
    if let Some(step) = step_idx {
        hasher.update(step.to_le_bytes());
    }
    hasher.update(b":");
    hasher.update(event.as_str().as_bytes());
    hasher.update(b":");
    if let Some(tool) = tool_name {
        hasher.update(tool.as_bytes());
    }
    if salt > 0 {
        hasher.update(b":salt:");
        hasher.update(salt.to_le_bytes());
    }
    let result = hasher.finalize();
    let mut span_id = [0u8; 8];
    span_id.copy_from_slice(&result[0..8]);
    if span_id == [0u8; 8] {
        span_id[0] = 0x01;
    }
    span_id
}

/// Parses a W3C Traceparent header string into (trace_id, parent_span_id).
/// Format: {version:2}-{trace_id:32}-{parent_id:16}-{trace_flags:2}
/// Example: "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
pub fn parse_w3c_traceparent(raw: &str) -> Option<([u8; 16], [u8; 8])> {
    let parts: Vec<&str> = raw.trim().split('-').collect();
    if parts.len() != 4 {
        return None;
    }
    let (version, trace_hex, parent_hex, _flags) = (parts[0], parts[1], parts[2], parts[3]);
    if version.len() != 2 {
        return None;
    }
    if trace_hex.len() != 32 || parent_hex.len() != 16 {
        return None;
    }
    let mut trace_id = [0u8; 16];
    for i in 0..16 {
        trace_id[i] = u8::from_str_radix(&trace_hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    let mut parent_id = [0u8; 8];
    for i in 0..8 {
        parent_id[i] = u8::from_str_radix(&parent_hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    if trace_id == [0u8; 16] || parent_id == [0u8; 8] {
        return None;
    }
    Some((trace_id, parent_id))
}

/// Resolves trace_id and optional parent_span_id by prioritizing:
/// 1. Explicit traceparent string (from payload)
/// 2. Process environment variable `TRACEPARENT`
/// 3. Deterministic fallback from conversation_id
pub fn resolve_trace_and_parent_id(
    explicit_traceparent: Option<&str>,
    conversation_id: Option<&str>,
) -> ([u8; 16], Option<[u8; 8]>) {
    if let Some(tp) = explicit_traceparent {
        if let Some((t, p)) = parse_w3c_traceparent(tp) {
            return (t, Some(p));
        }
    }
    if let Ok(env_tp) = std::env::var("TRACEPARENT") {
        if let Some((t, p)) = parse_w3c_traceparent(&env_tp) {
            return (t, Some(p));
        }
    }
    (derive_trace_id(conversation_id), None)
}
