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
pub const GEN_AI_RESPONSE_FINISH_REASONS: &str = "gen_ai.response.finish_reasons";

// --- Standard Resource Attributes ---
pub const SERVICE_NAME: &str = "service.name";
pub const DEPLOYMENT_ENVIRONMENT: &str = "deployment.environment";
pub const SERVICE_VERSION: &str = "service.version";

// --- SigNoz & Antigravity Compatibility Attributes ---
pub const AGY_HOOK_EVENT: &str = "agy.hook.event";
pub const AGY_TERMINATION_REASON: &str = "agy.termination_reason";
pub const AGY_EXECUTION_NUM: &str = "agy.execution.num";
pub const AGY_FULLY_IDLE: &str = "agy.fully_idle";
pub const AGY_STEP_INDEX: &str = "agy.step.index";

// --- Quota Metrics & Attributes ---
pub const QUOTA_REMAINING_FRACTION: &str = "agy.quota.remaining_fraction";
pub const QUOTA_SECONDS_TO_RESET: &str = "agy.quota.seconds_to_reset";
pub const QUOTA_ATTR_BUCKET: &str = "bucket";
pub const QUOTA_ATTR_GROUP: &str = "group";
pub const QUOTA_DEFAULT_BUCKET: &str = "gemini-weekly";
pub const QUOTA_DEFAULT_GROUP: &str = "gemini";
