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
    #[serde(rename = "interactive", alias = "iterativo")]
    Interactive,
    #[serde(rename = "automation", alias = "automacao")]
    Automation,
}

impl ExecutionMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExecutionMode::Interactive => "interactive",
            ExecutionMode::Automation => "automation",
        }
    }

    pub fn from_str_name(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "interactive" | "iterativo" | "repl" | "tui" => ExecutionMode::Interactive,
            _ => ExecutionMode::Automation,
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

    #[serde(
        alias = "execution_mode",
        alias = "executionMode",
        alias = "mode",
        default
    )]
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

    // --- Execution Context & Workspace (v0.3) ---
    #[serde(alias = "workspace_path", alias = "workspacePath", default)]
    pub workspace_path: Option<String>,

    #[serde(alias = "project_name", alias = "projectName", default)]
    pub project_name: Option<String>,

    #[serde(alias = "project_root", alias = "projectRoot", default)]
    pub project_root: Option<String>,

    #[serde(alias = "project_type", alias = "projectType", default)]
    pub project_type: Option<String>,

    #[serde(alias = "vcs_system", alias = "vcsSystem", default)]
    pub vcs_system: Option<String>,

    #[serde(alias = "vcs_repository", alias = "vcsRepository", default)]
    pub vcs_repository: Option<String>,

    #[serde(alias = "vcs_branch", alias = "vcsBranch", default)]
    pub vcs_branch: Option<String>,

    #[serde(alias = "vcs_commit", alias = "vcsCommit", default)]
    pub vcs_commit: Option<String>,

    #[serde(alias = "vcs_worktree", alias = "vcsWorktree", default)]
    pub vcs_worktree: Option<bool>,

    // --- Tool Archetypes & I/O Economics (v0.3) ---
    #[serde(alias = "tool_archetype", alias = "toolArchetype", default)]
    pub tool_archetype: Option<String>,

    #[serde(alias = "tool_binary", alias = "toolBinary", default)]
    pub tool_binary: Option<String>,

    #[serde(alias = "tool_wrapped_binary", alias = "toolWrappedBinary", default)]
    pub tool_wrapped_binary: Option<String>,

    #[serde(alias = "tool_pipeline_depth", alias = "toolPipelineDepth", default)]
    pub tool_pipeline_depth: Option<u32>,

    #[serde(
        alias = "tool_compression_ratio",
        alias = "toolCompressionRatio",
        default
    )]
    pub tool_compression_ratio: Option<f64>,

    #[serde(alias = "tool_tokens_saved", alias = "toolTokensSaved", default)]
    pub tool_tokens_saved: Option<i64>,

    // --- Universal Capabilities (MCP & Skills) & Waste (v0.3) ---
    #[serde(alias = "capability_kind", alias = "capabilityKind", default)]
    pub capability_kind: Option<String>,

    #[serde(alias = "capability_namespace", alias = "capabilityNamespace", default)]
    pub capability_namespace: Option<String>,

    #[serde(alias = "capability_name", alias = "capabilityName", default)]
    pub capability_name: Option<String>,

    #[serde(
        alias = "capability_schema_tokens",
        alias = "capabilitySchemaTokens",
        default
    )]
    pub capability_schema_tokens: Option<i64>,

    #[serde(
        alias = "capability_response_bytes",
        alias = "capabilityResponseBytes",
        default
    )]
    pub capability_response_bytes: Option<u64>,

    #[serde(
        alias = "capability_response_tokens",
        alias = "capabilityResponseTokens",
        default
    )]
    pub capability_response_tokens: Option<i64>,

    #[serde(
        alias = "capability_consecutive_retries",
        alias = "capabilityConsecutiveRetries",
        default
    )]
    pub capability_consecutive_retries: Option<u32>,

    #[serde(alias = "capability_is_waste", alias = "capabilityIsWaste", default)]
    pub capability_is_waste: Option<bool>,

    // --- Cross-Agent Lineage & Subagent Parenting (v0.3) ---
    #[serde(alias = "agent_depth", alias = "agentDepth", default)]
    pub agent_depth: Option<u32>,

    #[serde(alias = "agent_parent_name", alias = "agentParentName", default)]
    pub agent_parent_name: Option<String>,

    #[serde(alias = "agent_root_id", alias = "agentRootId", default)]
    pub agent_root_id: Option<String>,

    #[serde(alias = "agent_is_root", alias = "agentIsRoot", default)]
    pub agent_is_root: Option<bool>,

    // --- Multi-Layer Error Categorization (v0.3) ---
    #[serde(alias = "error_category", alias = "errorCategory", default)]
    pub error_category: Option<String>,
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

    /// Auto-enriches the input with execution context, tool archetypes,
    /// capabilities, lineage, and error categories if not already specified.
    pub fn auto_enrich(&mut self) {
        // 1. Workspace and Project Context
        if self.workspace_path.is_none() || self.project_root.is_none() {
            let ctx = crate::context::WorkspaceContext::harvest_current();
            if self.workspace_path.is_none() {
                self.workspace_path = Some(ctx.current_dir);
            }
            if self.project_name.is_none() {
                self.project_name = ctx.project_name;
            }
            if self.project_root.is_none() {
                self.project_root = ctx.project_root;
            }
            if self.project_type.is_none() {
                self.project_type = ctx.project_type;
            }
            if self.vcs_system.is_none() {
                self.vcs_system = ctx.vcs_system;
            }
            if self.vcs_repository.is_none() {
                self.vcs_repository = ctx.vcs_repository;
            }
            if self.vcs_branch.is_none() {
                self.vcs_branch = ctx.vcs_branch;
            }
            if self.vcs_commit.is_none() {
                self.vcs_commit = ctx.vcs_commit;
            }
            if self.vcs_worktree.is_none() {
                self.vcs_worktree = ctx.vcs_worktree;
            }
        }

        // 2. Capabilities (MCP, Skills, Subagents)
        let tool_opt = self.resolved_tool_name().map(|s| s.to_string());
        let cmd_str_opt = self.resolved_tool_arguments().and_then(|args| {
            args.get("command")
                .or_else(|| args.get("CommandLine"))
                .or_else(|| args.get("cmd"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        });

        if let Some(ref tool) = tool_opt {
            if self.capability_kind.is_none() {
                let cap = crate::capability::ClassifiedCapability::classify(tool);
                self.capability_kind = Some(cap.kind.as_str().to_string());
                self.capability_namespace = Some(cap.namespace);
                self.capability_name = Some(cap.operation);
            }

            // 3. Tool Archetypes
            if self.tool_archetype.is_none() {
                let cmd_to_classify = cmd_str_opt.as_deref().unwrap_or(tool);
                let classified = crate::archetype::ClassifiedCommand::classify(cmd_to_classify);
                self.tool_archetype = Some(classified.archetype.as_str().to_string());
                self.tool_binary = Some(classified.binary);
                self.tool_wrapped_binary = classified.wrapped_binary;
                self.tool_pipeline_depth = Some(classified.pipeline_depth as u32);
            }
        }

        // 4. Lineage and Hierarchy
        if self.agent_depth.is_none() {
            let has_parent = self.traceparent.is_some()
                || self.agent_parent_name.is_some()
                || std::env::var("TRACEPARENT").is_ok();
            if has_parent {
                self.agent_depth = Some(1);
                self.agent_is_root = Some(false);
            } else {
                self.agent_depth = Some(0);
                self.agent_is_root = Some(true);
            }
        }

        // 5. Multi-layer Error Categorization
        if self.error.is_some() && self.error_category.is_none() {
            let err_msg = self.error.as_deref().unwrap_or("").to_ascii_lowercase();
            if err_msg.contains("rate limit")
                || err_msg.contains("429")
                || err_msg.contains("quota")
            {
                self.error_category = Some("provider_quota_exhausted".to_string());
            } else if err_msg.contains("interrupted")
                || err_msg.contains("sigint")
                || err_msg.contains("user cancelled")
            {
                self.error_category = Some("user_interrupted".to_string());
            } else if err_msg.contains("validation")
                || err_msg.contains("schema")
                || err_msg.contains("invalid argument")
            {
                self.error_category = Some("schema_validation_error".to_string());
            } else {
                self.error_category = Some("tool_verification_failed".to_string());
            }
        }
    }
}
