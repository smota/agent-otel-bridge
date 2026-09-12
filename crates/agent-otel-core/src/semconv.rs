/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

// --- Canonical OpenTelemetry GenAI & Agent Semantic Conventions ---
pub const GEN_AI_OPERATION_NAME: &str = "gen_ai.operation.name";
pub const GEN_AI_OPERATION_EXECUTE_TOOL: &str = "execute_tool";
pub const GEN_AI_OPERATION_INVOKE_AGENT: &str = "invoke_agent";
pub const GEN_AI_OPERATION_CHAT: &str = "chat";

pub const GEN_AI_PROVIDER_NAME: &str = "gen_ai.provider.name";
pub const GEN_AI_PROVIDER_GOOGLE: &str = "google";
pub const GEN_AI_PROVIDER_ANTHROPIC: &str = "anthropic";
pub const GEN_AI_PROVIDER_OPENAI: &str = "openai";
pub const GEN_AI_PROVIDER_XAI: &str = "xai";
pub const GEN_AI_PROVIDER_PI: &str = "pi";
pub const GEN_AI_PROVIDER_INFLECTION: &str = "pi";

pub const GEN_AI_SYSTEM: &str = "gen_ai.system";
pub const GEN_AI_SYSTEM_DEFAULT: &str = "antigravity";

pub const GEN_AI_AGENT_NAME: &str = "gen_ai.agent.name";
pub const GEN_AI_AGENT_ID: &str = "gen_ai.agent.id";
pub const GEN_AI_CONVERSATION_ID: &str = "gen_ai.conversation.id";
pub const GEN_AI_REQUEST_MODEL: &str = "gen_ai.request.model";
pub const GEN_AI_RESPONSE_MODEL: &str = "gen_ai.response.model";
pub const GEN_AI_TOOL_NAME: &str = "gen_ai.tool.name";
pub const GEN_AI_TOOL_CALL_ID: &str = "gen_ai.tool.call.id";
pub const GEN_AI_USAGE_INPUT_TOKENS: &str = "gen_ai.usage.input_tokens";
pub const GEN_AI_USAGE_OUTPUT_TOKENS: &str = "gen_ai.usage.output_tokens";
pub const GEN_AI_USAGE_CACHE_READ_TOKENS: &str = "gen_ai.usage.cache_read_tokens";
pub const GEN_AI_RESPONSE_FINISH_REASONS: &str = "gen_ai.response.finish_reasons";

// --- Standard Resource Attributes ---
pub const SERVICE_NAME: &str = "service.name";
pub const DEPLOYMENT_ENVIRONMENT: &str = "deployment.environment";
pub const SERVICE_VERSION: &str = "service.version";
pub const HOST_NAME: &str = "host.name";
pub const OS_TYPE: &str = "os.type";

// --- Identity & Environment Attributes ---
pub const USER_EMAIL: &str = "user.email";
pub const TERMINAL_TYPE: &str = "terminal.type";
pub const AGENT_DECISION: &str = "agent.decision";
pub const AGENT_SUCCESS: &str = "agent.success";
pub const AGENT_EXECUTION_MODE: &str = "agent.execution.mode";

// --- Git Workspace Intelligence ---
pub const AGENT_GIT_LINES_ADDED: &str = "agent.git.lines_added";
pub const AGENT_GIT_LINES_DELETED: &str = "agent.git.lines_deleted";
pub const AGENT_GIT_FILES_CHANGED: &str = "agent.git.files_changed";
pub const AGENT_GIT_SELF_REVERT: &str = "agent.git.self_revert";

// --- Execution Context & Workspace SemConv ---
pub const WORKSPACE_PATH: &str = "workspace.path";
pub const WORKSPACE_PROJECT_NAME: &str = "workspace.project_name";
pub const WORKSPACE_PROJECT_ROOT: &str = "workspace.project_root";
pub const WORKSPACE_PROJECT_TYPE: &str = "workspace.project_type";

// --- VCS / Git Attributes (Opportunistic) ---
pub const VCS_SYSTEM: &str = "vcs.system";
pub const VCS_REPOSITORY_NAME: &str = "vcs.repository.name";
pub const VCS_BRANCH_NAME: &str = "vcs.branch.name";
pub const VCS_COMMIT_SHA: &str = "vcs.commit.sha";
pub const VCS_WORKTREE_ACTIVE: &str = "vcs.worktree.active";

// --- Tool Archetypes & I/O Economics ---
pub const AGENT_TOOL_ARCHETYPE: &str = "agent.tool.archetype";
pub const AGENT_TOOL_BINARY: &str = "agent.tool.binary";
pub const AGENT_TOOL_WRAPPED_BINARY: &str = "agent.tool.wrapped_binary";
pub const AGENT_TOOL_PIPELINE_DEPTH: &str = "agent.tool.pipeline_depth";
pub const AGENT_TOOL_COMPRESSION_RATIO: &str = "agent.tool.compression_ratio";
pub const AGENT_TOOL_TOKENS_SAVED: &str = "agent.tool.tokens_saved";

// --- Universal Capabilities (MCP & Skills) & Waste ---
pub const CAPABILITY_KIND: &str = "capability.kind";
pub const CAPABILITY_NAMESPACE: &str = "capability.namespace";
pub const CAPABILITY_NAME: &str = "capability.name";
pub const CAPABILITY_SCHEMA_TOKENS: &str = "capability.schema_tokens";
pub const CAPABILITY_RESPONSE_BYTES: &str = "capability.response_bytes";
pub const CAPABILITY_RESPONSE_TOKENS: &str = "capability.response_tokens";
pub const CAPABILITY_CONSECUTIVE_RETRIES: &str = "capability.consecutive_retries";
pub const CAPABILITY_IS_WASTE: &str = "capability.is_waste";

// --- Cross-Agent Lineage & Hierarchy ---
pub const GEN_AI_AGENT_DEPTH: &str = "gen_ai.agent.depth";
pub const GEN_AI_AGENT_PARENT_NAME: &str = "gen_ai.agent.parent_name";
pub const GEN_AI_AGENT_ROOT_ID: &str = "gen_ai.agent.root_id";
pub const GEN_AI_AGENT_IS_ROOT: &str = "gen_ai.agent.is_root";

// --- Multi-Layer Error Categorization ---
pub const AGENT_ERROR_CATEGORY: &str = "agent.error.category";

// --- Generic Agent Semantic Conventions ---
pub const AGENT_HOOK_EVENT: &str = "agent.hook.event";
pub const AGENT_STEP_INDEX: &str = "agent.step.index";
pub const GEN_AI_AGENT_STEP_INDEX: &str = "gen_ai.agent.step_index";
pub const AGENT_EXECUTION_NUM: &str = "agent.execution.num";
pub const AGENT_FULLY_IDLE: &str = "agent.fully_idle";
pub const AGENT_TERMINATION_REASON: &str = "agent.termination_reason";

// --- SigNoz & Antigravity Compatibility Aliases (v0.1) ---
pub const AGY_HOOK_EVENT: &str = "agy.hook.event";
pub const AGY_TERMINATION_REASON: &str = "agy.termination_reason";
pub const AGY_EXECUTION_NUM: &str = "agy.execution.num";
pub const AGY_FULLY_IDLE: &str = "agy.fully_idle";
pub const AGY_STEP_INDEX: &str = "agy.step.index";

// --- Canonical Agent Quota Metrics ---
pub const METRIC_AGENT_QUOTA_REMAINING: &str = "agent.quota.remaining_fraction";
pub const METRIC_AGENT_QUOTA_RESET: &str = "agent.quota.seconds_to_reset";
pub const METRIC_FLEET_BOTTLENECK_RATIO: &str = "agent.fleet.bottleneck_ratio";
pub const METRIC_FLEET_TOKEN_RATE: &str = "agent.fleet.token_rate_minute";

// --- Legacy Quota Metrics Aliases ---
pub const METRIC_AGY_QUOTA_REMAINING: &str = "agy.quota.remaining_fraction";
pub const METRIC_AGY_QUOTA_RESET: &str = "agy.quota.seconds_to_reset";
pub const QUOTA_REMAINING_FRACTION: &str = METRIC_AGY_QUOTA_REMAINING;
pub const QUOTA_SECONDS_TO_RESET: &str = METRIC_AGY_QUOTA_RESET;

pub const QUOTA_ATTR_BUCKET: &str = "bucket";
pub const QUOTA_ATTR_GROUP: &str = "group";
pub const QUOTA_DEFAULT_BUCKET: &str = "gemini-weekly";
pub const QUOTA_DEFAULT_GROUP: &str = "gemini";

/// Infers the AI provider name from the model identifier.
pub fn infer_provider(model: &str) -> &'static str {
    let lower = model.to_ascii_lowercase();
    if lower.contains("claude") {
        GEN_AI_PROVIDER_ANTHROPIC
    } else if lower.contains("gemini") {
        GEN_AI_PROVIDER_GOOGLE
    } else if lower.contains("grok") {
        GEN_AI_PROVIDER_XAI
    } else if lower.contains("pi") || lower.contains("inflection") {
        GEN_AI_PROVIDER_PI
    } else if lower.contains("gpt")
        || lower.contains("o1")
        || lower.contains("o3")
        || lower.contains("o4")
        || lower.contains("codex")
    {
        GEN_AI_PROVIDER_OPENAI
    } else {
        "unknown"
    }
}

/// Infers the agent name from model or client hints.
pub fn infer_agent_name(model: Option<&str>, client_hint: Option<&str>) -> &'static str {
    if let Some(hint) = client_hint {
        let h = hint.to_ascii_lowercase();
        if h == "claude" || h == "claude-code" || h == "claudecode" {
            return "claude-code";
        }
        if h == "antigravity" || h == "agy" || h == "gemini" {
            return "antigravity";
        }
        if h == "grok" || h == "xai" || h == "grok-cli" {
            return "grok";
        }
        if h == "codex" || h == "codex-cli" || h == "openai" {
            return "codex";
        }
        if h == "pi" || h == "pi-cli" || h == "inflection" {
            return "pi";
        }
    }
    if let Some(m) = model {
        let lower = m.to_ascii_lowercase();
        if lower.contains("claude") {
            return "claude-code";
        }
        if lower.contains("gemini") {
            return "antigravity";
        }
        if lower.contains("grok") {
            return "grok";
        }
        if lower.contains("codex")
            || lower.contains("gpt")
            || lower.contains("o1")
            || lower.contains("o3")
            || lower.contains("o4")
        {
            return "codex";
        }
        if lower.contains("pi") || lower.contains("inflection") {
            return "pi";
        }
    }
    "ai-agent"
}
