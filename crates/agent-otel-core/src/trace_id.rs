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

/// A validated trace context carried by a hook event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedTraceContext {
    pub trace_id: [u8; 16],
    pub parent_span_id: Option<[u8; 8]>,
    /// The W3C trace-flags byte. Bit 0 is the sampled flag.
    pub trace_flags: u8,
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn parse_hex<const N: usize>(bytes: &[u8]) -> Option<[u8; N]> {
    if bytes.len() != N * 2 {
        return None;
    }
    let mut parsed = [0u8; N];
    for (index, output) in parsed.iter_mut().enumerate() {
        *output = (hex_nibble(bytes[index * 2])? << 4) | hex_nibble(bytes[index * 2 + 1])?;
    }
    Some(parsed)
}

/// Parses a W3C traceparent header without indexing UTF-8 string boundaries.
///
/// Version `00` has the exact four-field form. Future non-`ff` versions may
/// carry extension fields after the required fields, as permitted by W3C.
pub fn parse_w3c_traceparent_with_flags(raw: &str) -> Option<ResolvedTraceContext> {
    let bytes = raw.as_bytes();
    if bytes.len() < 55 || bytes.len() > 512 {
        return None;
    }
    if bytes.get(2) != Some(&b'-') || bytes.get(35) != Some(&b'-') || bytes.get(52) != Some(&b'-') {
        return None;
    }

    let version = parse_hex::<1>(&bytes[..2])?[0];
    if version == 0xff {
        return None;
    }
    if version == 0 && bytes.len() != 55 {
        return None;
    }
    if version != 0 && bytes.len() > 55 {
        // Future-version extensions must be explicitly separated and ASCII.
        if bytes[55] != b'-' || bytes[56..].is_empty() || !bytes[56..].iter().all(u8::is_ascii) {
            return None;
        }
    }

    let trace_id = parse_hex::<16>(&bytes[3..35])?;
    let parent_span_id = parse_hex::<8>(&bytes[36..52])?;
    let trace_flags = parse_hex::<1>(&bytes[53..55])?[0];
    if trace_id == [0u8; 16] || parent_span_id == [0u8; 8] {
        return None;
    }
    Some(ResolvedTraceContext {
        trace_id,
        parent_span_id: Some(parent_span_id),
        trace_flags,
    })
}

/// Parses a W3C Traceparent header string into (trace_id, parent_span_id).
/// This compatibility wrapper omits the trace-flags byte.
pub fn parse_w3c_traceparent(raw: &str) -> Option<([u8; 16], [u8; 8])> {
    let context = parse_w3c_traceparent_with_flags(raw)?;
    Some((context.trace_id, context.parent_span_id?))
}

/// Resolves context without consulting process environment. The precedence is
/// payload JSON, then the transported hook-origin context, then conversation.
pub fn resolve_trace_context(
    payload_traceparent: Option<&str>,
    origin_traceparent: Option<&str>,
    conversation_id: Option<&str>,
) -> ResolvedTraceContext {
    for candidate in [payload_traceparent, origin_traceparent]
        .into_iter()
        .flatten()
    {
        if let Some(context) = parse_w3c_traceparent_with_flags(candidate) {
            return context;
        }
    }
    ResolvedTraceContext {
        trace_id: derive_trace_id(conversation_id),
        parent_span_id: None,
        trace_flags: 1,
    }
}

/// Compatibility resolver that retains the historical environment fallback.
/// Daemon IPC handling must use [`resolve_trace_context`] directly.
pub fn resolve_trace_context_with_environment(
    explicit_traceparent: Option<&str>,
    conversation_id: Option<&str>,
) -> ResolvedTraceContext {
    let environment = std::env::var("TRACEPARENT").ok();
    resolve_trace_context(
        explicit_traceparent,
        environment.as_deref(),
        conversation_id,
    )
}

/// Resolves trace_id and optional parent_span_id by prioritizing:
/// 1. Explicit traceparent string (from payload)
/// 2. Process environment variable `TRACEPARENT`
/// 3. Deterministic fallback from conversation_id
pub fn resolve_trace_and_parent_id(
    explicit_traceparent: Option<&str>,
    conversation_id: Option<&str>,
) -> ([u8; 16], Option<[u8; 8]>) {
    let context = resolve_trace_context_with_environment(explicit_traceparent, conversation_id);
    (context.trace_id, context.parent_span_id)
}

/// Formats a trace_id and span_id into a W3C traceparent header string.
/// Format: 00-{trace_id:32hex}-{span_id:16hex}-01
pub fn format_w3c_traceparent(trace_id: &[u8; 16], span_id: &[u8; 8], sampled: bool) -> String {
    let trace_hex: String = trace_id.iter().map(|b| format!("{b:02x}")).collect();
    let span_hex: String = span_id.iter().map(|b| format!("{b:02x}")).collect();
    let flags = if sampled { "01" } else { "00" };
    format!("00-{trace_hex}-{span_hex}-{flags}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_traceparent_roundtrip() {
        let trace_id = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let span_id = [10u8, 20, 30, 40, 50, 60, 70, 80];
        let formatted = format_w3c_traceparent(&trace_id, &span_id, true);
        assert!(formatted.starts_with("00-"));
        assert!(formatted.ends_with("-01"));

        let parsed = parse_w3c_traceparent(&formatted);
        assert!(parsed.is_some());
        let (t, p) = parsed.unwrap();
        assert_eq!(t, trace_id);
        assert_eq!(p, span_id);
    }
}
