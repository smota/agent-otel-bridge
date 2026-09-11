/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum HookEvent {
    PreInvocation = 1,
    PostInvocation = 2,
    PreToolUse = 3,
    PostToolUse = 4,
    Stop = 5,
    Unknown = 255,
}

impl HookEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
            HookEvent::PreInvocation => "PreInvocation",
            HookEvent::PostInvocation => "PostInvocation",
            HookEvent::PreToolUse => "PreToolUse",
            HookEvent::PostToolUse => "PostToolUse",
            HookEvent::Stop => "Stop",
            HookEvent::Unknown => "Unknown",
        }
    }

    pub fn from_tag(tag: u8) -> Self {
        match tag {
            1 => HookEvent::PreInvocation,
            2 => HookEvent::PostInvocation,
            3 => HookEvent::PreToolUse,
            4 => HookEvent::PostToolUse,
            5 => HookEvent::Stop,
            _ => HookEvent::Unknown,
        }
    }

    pub fn to_tag(&self) -> u8 {
        *self as u8
    }

    pub fn from_str_name(s: &str) -> Self {
        match s.trim() {
            "PreInvocation" | "pre_invocation" => HookEvent::PreInvocation,
            "PostInvocation" | "post_invocation" => HookEvent::PostInvocation,
            "PreToolUse" | "pre_tool_use" => HookEvent::PreToolUse,
            "PostToolUse" | "post_tool_use" => HookEvent::PostToolUse,
            "Stop" | "stop" => HookEvent::Stop,
            _ => HookEvent::Unknown,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolCallInfo {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityHookInput {
    #[serde(alias = "conversation_id", default)]
    pub conversation_id: Option<String>,

    #[serde(alias = "step_idx", default)]
    pub step_idx: Option<u64>,

    #[serde(alias = "tool_call", default)]
    pub tool_call: Option<ToolCallInfo>,

    #[serde(default)]
    pub error: Option<String>,

    #[serde(alias = "termination_reason", default)]
    pub termination_reason: Option<String>,

    #[serde(alias = "model_name", alias = "modelName", default)]
    pub model: Option<String>,

    #[serde(alias = "execution_num", default)]
    pub execution_num: Option<i64>,

    #[serde(alias = "fully_idle", default)]
    pub fully_idle: Option<bool>,

    #[serde(alias = "workspace_paths", default)]
    pub workspace_paths: Option<Vec<String>>,

    #[serde(alias = "transcript_path", default)]
    pub transcript_path: Option<String>,
}

impl AntigravityHookInput {
    pub fn parse_slice(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        if bytes.is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_slice(bytes)
    }
}
