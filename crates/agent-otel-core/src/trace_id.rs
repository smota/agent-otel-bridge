/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use sha2::{Digest, Sha256};
use crate::model::HookEvent;

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
            hasher.update(&now.to_le_bytes());
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
        hasher.update(&step.to_le_bytes());
    }
    hasher.update(b":");
    hasher.update(event.as_str().as_bytes());
    hasher.update(b":");
    if let Some(tool) = tool_name {
        hasher.update(tool.as_bytes());
    }
    if salt > 0 {
        hasher.update(b":salt:");
        hasher.update(&salt.to_le_bytes());
    }
    let result = hasher.finalize();
    let mut span_id = [0u8; 8];
    span_id.copy_from_slice(&result[0..8]);
    if span_id == [0u8; 8] {
        span_id[0] = 0x01;
    }
    span_id
}
