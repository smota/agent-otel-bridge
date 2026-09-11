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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExecutionMode {
    #[serde(rename = "iterativo")]
    Iterativo,
    #[serde(rename = "automacao")]
    Automacao,
}

impl ExecutionMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExecutionMode::Iterativo => "iterativo",
            ExecutionMode::Automacao => "automacao",
        }
    }

    pub fn from_str_name(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "iterativo" | "interactive" | "repl" | "tui" => ExecutionMode::Iterativo,
            _ => ExecutionMode::Automacao,
        }
    }
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
        match tag & 0x0F {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClientKind {
    Unspecified,
    Antigravity,
    ClaudeCode,
    Codex,
    Grok,
    Pi,
}

impl ClientKind {
    pub fn from_tag(tag: u8) -> Self {
        match tag >> 4 {
            1 => ClientKind::Antigravity,
            2 => ClientKind::ClaudeCode,
            3 => ClientKind::Codex,
            4 => ClientKind::Grok,
            5 => ClientKind::Pi,
            _ => ClientKind::Unspecified,
        }
    }

    pub fn to_tag(&self) -> u8 {
        match self {
            ClientKind::Unspecified => 0,
            ClientKind::Antigravity => 1,
            ClientKind::ClaudeCode => 2,
            ClientKind::Codex => 3,
            ClientKind::Grok => 4,
            ClientKind::Pi => 5,
        }
    }

    pub fn as_str(&self) -> Option<&'static str> {
        match self {
            ClientKind::Unspecified => None,
            ClientKind::Antigravity => Some("antigravity"),
            ClientKind::ClaudeCode => Some("claude-code"),
            ClientKind::Codex => Some("codex"),
            ClientKind::Grok => Some("grok"),
            ClientKind::Pi => Some("pi"),
        }
    }

    pub fn from_str_name(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "antigravity" | "agy" | "gemini" => ClientKind::Antigravity,
            "claude" | "claude-code" | "claudecode" => ClientKind::ClaudeCode,
            "codex" | "codex-cli" | "openai" => ClientKind::Codex,
            "grok" | "grok-cli" | "xai" => ClientKind::Grok,
            "pi" | "pi-cli" | "inflection" => ClientKind::Pi,
            _ => ClientKind::Unspecified,
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
pub struct AgentHookInput {
    #[serde(
        alias = "conversation_id",
        alias = "session_id",
        alias = "sessionId",
        default
    )]
    pub conversation_id: Option<String>,

    #[serde(alias = "step_idx", alias = "step_index", alias = "stepIndex", default)]
    pub step_idx: Option<u64>,

    #[serde(alias = "tool_call", alias = "toolCall", default)]
    pub tool_call: Option<ToolCallInfo>,

    #[serde(alias = "tool_name", alias = "toolName", default)]
    pub tool_name: Option<String>,

    #[serde(alias = "tool_input", alias = "toolInput", default)]
    pub tool_input: Option<serde_json::Value>,

    #[serde(default)]
    pub error: Option<String>,

    #[serde(alias = "termination_reason", alias = "terminationReason", default)]
    pub termination_reason: Option<String>,

    #[serde(alias = "model_name", alias = "modelName", default)]
    pub model: Option<String>,

    #[serde(alias = "execution_num", alias = "executionNum", default)]
    pub execution_num: Option<i64>,

    #[serde(alias = "fully_idle", alias = "fullyIdle", default)]
    pub fully_idle: Option<bool>,

    #[serde(alias = "workspace_paths", alias = "workspacePaths", default)]
    pub workspace_paths: Option<Vec<String>>,

    #[serde(alias = "transcript_path", alias = "transcriptPath", default)]
    pub transcript_path: Option<String>,

    #[serde(alias = "agent_name", alias = "agentName", default)]
    pub agent_name: Option<String>,

    #[serde(alias = "hook_event_name", alias = "hookEventName", default)]
    pub hook_event_name: Option<String>,

    #[serde(
        alias = "input_tokens",
        alias = "inputTokens",
        alias = "input_token_count",
        alias = "inputTokenCount",
        alias = "prompt_tokens",
        alias = "promptTokens",
        default
    )]
    pub input_tokens: Option<i64>,

    #[serde(
        alias = "output_tokens",
        alias = "outputTokens",
        alias = "output_token_count",
        alias = "outputTokenCount",
        alias = "completion_tokens",
        alias = "completionTokens",
        default
    )]
    pub output_tokens: Option<i64>,

    #[serde(
        alias = "cached_tokens",
        alias = "cachedTokens",
        alias = "cached_token_count",
        alias = "cachedTokenCount",
        alias = "cache_read_input_tokens",
        default
    )]
    pub cached_tokens: Option<i64>,

    #[serde(alias = "user_email", alias = "userEmail", alias = "email", default)]
    pub user_email: Option<String>,

    #[serde(alias = "terminal_type", alias = "terminalType", default)]
    pub terminal_type: Option<String>,

    #[serde(default)]
    pub decision: Option<String>,

    #[serde(default)]
    pub success: Option<bool>,

    #[serde(alias = "execution_mode", alias = "executionMode", alias = "mode", default)]
    pub execution_mode: Option<ExecutionMode>,

    #[serde(
        alias = "git_lines_added",
        alias = "gitLinesAdded",
        alias = "lines_added",
        alias = "linesAdded",
        default
    )]
    pub git_lines_added: Option<u64>,

    #[serde(
        alias = "git_lines_deleted",
        alias = "gitLinesDeleted",
        alias = "lines_deleted",
        alias = "linesDeleted",
        default
    )]
    pub git_lines_deleted: Option<u64>,

    #[serde(
        alias = "git_files_changed",
        alias = "gitFilesChanged",
        alias = "files_changed",
        alias = "filesChanged",
        default
    )]
    pub git_files_changed: Option<u64>,

    #[serde(
        alias = "git_self_revert",
        alias = "gitSelfRevert",
        alias = "self_revert",
        alias = "selfRevert",
        default
    )]
    pub git_self_revert: Option<bool>,

    #[serde(alias = "traceparent", alias = "trace_parent", default)]
    pub traceparent: Option<String>,
}

/// Backward compatibility alias for Antigravity-specific integrations
pub type AntigravityHookInput = AgentHookInput;

impl AgentHookInput {
    pub fn resolved_tool_name(&self) -> Option<&str> {
        self.tool_call
            .as_ref()
            .and_then(|tc| tc.name.as_deref())
            .or(self.tool_name.as_deref())
    }

    pub fn resolved_tool_call_id(&self) -> Option<&str> {
        self.tool_call.as_ref().and_then(|tc| tc.id.as_deref())
    }

    pub fn resolved_tool_arguments(&self) -> Option<&serde_json::Value> {
        self.tool_call
            .as_ref()
            .and_then(|tc| tc.arguments.as_ref())
            .or(self.tool_input.as_ref())
    }

    pub fn parse_slice(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        if bytes.is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_slice(bytes)
    }
}
