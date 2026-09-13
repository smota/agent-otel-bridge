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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MsgType {
    HookPayload = 0x01,
    QuotaPing = 0x02,
    HealthPing = 0x03,
    Shutdown = 0xFF,
    Unknown = 0x00,
}

impl MsgType {
    pub fn from_u8(b: u8) -> Self {
        match b {
            0x01 => MsgType::HookPayload,
            0x02 => MsgType::QuotaPing,
            0x03 => MsgType::HealthPing,
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
