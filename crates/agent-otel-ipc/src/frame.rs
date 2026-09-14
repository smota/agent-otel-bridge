/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

pub use agent_otel_core::model::WireHeader;

pub const MAGIC: [u8; 2] = [0x41, 0x47]; // "AG"
pub const PROTOCOL_VERSION: u8 = 1;
pub const HEADER_LEN: usize = 8;
pub const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\agent-otel";
pub const LEGACY_PIPE_NAME: &str = r"\\.\pipe\agy-otel";

/// Version of the payload envelope used by [`encode_context_payload`].
pub const CONTEXT_ENVELOPE_VERSION: u8 = 1;
/// Maximum number of bytes reserved for the optional UTF-8 context.
pub const MAX_CONTEXT_LEN: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MsgType {
    HookPayload = 0x01,
    QuotaPing = 0x02,
    HealthPing = 0x03,
    HookPayloadWithContext = 0x04,
    Shutdown = 0xFF,
    Unknown = 0x00,
}

impl MsgType {
    pub fn from_u8(b: u8) -> Self {
        match b {
            0x01 => MsgType::HookPayload,
            0x02 => MsgType::QuotaPing,
            0x03 => MsgType::HealthPing,
            0x04 => MsgType::HookPayloadWithContext,
            0xFF => MsgType::Shutdown,
            _ => MsgType::Unknown,
        }
    }

    pub fn to_u8(self) -> u8 {
        self as u8
    }
}

pub fn encode_frame(msg_type: MsgType, payload: &[u8]) -> Vec<u8> {
    let payload_len = payload.len() as u32;
    let mut frame = Vec::with_capacity(HEADER_LEN + payload.len());
    frame.push(MAGIC[0]);
    frame.push(MAGIC[1]);
    frame.push(PROTOCOL_VERSION);
    frame.push(msg_type.to_u8());
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(payload);
    frame
}

pub fn decode_header(header: &[u8; HEADER_LEN]) -> Result<(MsgType, u32), &'static str> {
    if header[0] != MAGIC[0] || header[1] != MAGIC[1] {
        return Err("invalid magic bytes");
    }
    if header[2] != PROTOCOL_VERSION {
        return Err("unsupported protocol version");
    }
    let msg_type = MsgType::from_u8(header[3]);
    if msg_type == MsgType::Unknown {
        return Err("unknown message type");
    }
    let mut len_bytes = [0u8; 4];
    len_bytes.copy_from_slice(&header[4..8]);
    let len = u32::from_le_bytes(len_bytes);
    Ok((msg_type, len))
}

/// Encodes the body for a [`MsgType::HookPayloadWithContext`] frame.
///
/// The optional context is omitted when it is empty or exceeds the wire
/// limit. The JSON bytes are copied verbatim and are never parsed or changed.
pub fn encode_context_payload(header: WireHeader, context: Option<&str>, json: &[u8]) -> Vec<u8> {
    let context_bytes = context
        .filter(|value| !value.is_empty())
        .map(str::as_bytes)
        .filter(|value| value.len() <= MAX_CONTEXT_LEN);
    let context_len = context_bytes.map_or(0, |value| value.len());

    let mut payload = Vec::with_capacity(WireHeader::LEN + 1 + 2 + context_len + json.len());
    payload.extend_from_slice(&header.encode());
    payload.push(CONTEXT_ENVELOPE_VERSION);
    payload.extend_from_slice(&(context_len as u16).to_le_bytes());
    if let Some(context_bytes) = context_bytes {
        payload.extend_from_slice(context_bytes);
    }
    payload.extend_from_slice(json);
    payload
}

/// Decodes a body produced by [`encode_context_payload`].
///
/// Invalid UTF-8 is treated as an absent context while preserving the JSON
/// slice. Structural envelope errors are reported without attempting to parse
/// the JSON payload.
pub fn decode_context_payload(
    payload: &[u8],
) -> Result<(WireHeader, Option<&str>, &[u8]), &'static str> {
    let envelope_len = WireHeader::LEN + 1 + 2;
    if payload.len() < envelope_len {
        return Err("truncated context payload");
    }

    let header = WireHeader::decode(payload).ok_or("truncated wire header")?;
    if payload[WireHeader::LEN] != CONTEXT_ENVELOPE_VERSION {
        return Err("unsupported context envelope version");
    }

    let context_len_offset = WireHeader::LEN + 1;
    let context_len =
        u16::from_le_bytes([payload[context_len_offset], payload[context_len_offset + 1]]) as usize;
    if context_len > MAX_CONTEXT_LEN {
        return Err("context exceeds maximum length");
    }

    let context_start = envelope_len;
    let context_end = context_start
        .checked_add(context_len)
        .ok_or("invalid context length")?;
    if context_end > payload.len() {
        return Err("truncated context payload");
    }

    let context = if context_len == 0 {
        None
    } else {
        std::str::from_utf8(&payload[context_start..context_end]).ok()
    };
    Ok((header, context, &payload[context_end..]))
}
