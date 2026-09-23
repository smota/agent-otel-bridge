/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use opentelemetry_proto::tonic::{
    collector::trace::v1::ExportTraceServiceRequest,
    common::v1::{any_value, AnyValue, InstrumentationScope, KeyValue},
    resource::v1::Resource,
    trace::v1::{span::SpanKind, status::StatusCode, ResourceSpans, ScopeSpans, Span, Status},
};

use crate::model::{AgentHookInput, HookEvent};
use crate::semconv::*;
use crate::trace_id::{
    derive_span_id, resolve_trace_context_with_environment, ResolvedTraceContext,
};

/// Ambient values resolved before span construction.
///
/// Keeping these values explicit makes [`build_span_from_resolved`] suitable
/// for the daemon hot path and deterministic benchmarks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResolvedSpanMetadata<'a> {
    pub user_email: Option<&'a str>,
    pub terminal_type: Option<&'a str>,
}

pub fn kv_string(key: &str, value: &str) -> KeyValue {
    KeyValue {
        key: key.to_string(),
        value: Some(AnyValue {
            value: Some(any_value::Value::StringValue(value.to_string())),
        }),
        ..Default::default()
    }
}

pub fn kv_int(key: &str, value: i64) -> KeyValue {
    KeyValue {
        key: key.to_string(),
        value: Some(AnyValue {
            value: Some(any_value::Value::IntValue(value)),
        }),
        ..Default::default()
    }
}

pub fn kv_bool(key: &str, value: bool) -> KeyValue {
    KeyValue {
        key: key.to_string(),
        value: Some(AnyValue {
            value: Some(any_value::Value::BoolValue(value)),
        }),
        ..Default::default()
    }
}

pub fn build_resource(service_name: &str, environment: &str, version: &str) -> Resource {
    let hostname = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "localhost".to_string());
    let os = std::env::consts::OS;

    Resource {
        attributes: vec![
            kv_string(SERVICE_NAME, service_name),
            kv_string(DEPLOYMENT_ENVIRONMENT, environment),
            kv_string(SERVICE_VERSION, version),
            kv_string(HOST_NAME, &hostname),
            kv_string(OS_TYPE, os),
        ],
        ..Default::default()
    }
}

pub fn instrumentation_scope() -> InstrumentationScope {
    InstrumentationScope {
        name: "agent-otel-bridge".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        attributes: vec![],
        ..Default::default()
    }
}

pub fn build_span_from_hook(
    event: HookEvent,
    input: &AgentHookInput,
    start_time_unix_nano: u64,
    end_time_unix_nano: u64,
    salt: u32,
) -> Span {
    build_span_from_hook_opts(
        event,
        input,
        start_time_unix_nano,
        end_time_unix_nano,
        salt,
        false,
    )
}

pub fn build_span_from_hook_opts(
    event: HookEvent,
    input: &AgentHookInput,
    start_time_unix_nano: u64,
    end_time_unix_nano: u64,
    salt: u32,
    emit_legacy_aliases: bool,
) -> Span {
    let context = resolve_trace_context_with_environment(
        input.traceparent.as_deref(),
        input.conversation_id.as_deref(),
    );
    build_span_from_hook_with_context_opts(
        event,
        input,
        start_time_unix_nano,
        end_time_unix_nano,
        salt,
        context,
        emit_legacy_aliases,
    )
}

/// Builds a span using a context already resolved for this event. This keeps
/// daemon IPC handling independent from the daemon process environment.
pub fn build_span_from_hook_with_context_opts(
    event: HookEvent,
    input: &AgentHookInput,
    start_time_unix_nano: u64,
    end_time_unix_nano: u64,
    salt: u32,
    context: ResolvedTraceContext,
    emit_legacy_aliases: bool,
) -> Span {
    let email = input
        .user_email
        .clone()
        .or_else(|| std::env::var("USER_EMAIL").ok())
        .or_else(|| std::env::var("GIT_AUTHOR_EMAIL").ok())
        .or_else(crate::context::harvest_user_email);
    let terminal = input
        .terminal_type
        .clone()
        .or_else(|| std::env::var("TERM_PROGRAM").ok())
        .or_else(|| {
            std::env::var("WT_SESSION")
                .ok()
                .map(|_| "windows-terminal".to_string())
        })
        .or_else(|| std::env::var("TERM").ok());

    build_span_from_resolved(
        event,
        input,
        start_time_unix_nano,
        end_time_unix_nano,
        salt,
        context,
        ResolvedSpanMetadata {
            user_email: email.as_deref(),
            terminal_type: terminal.as_deref(),
        },
        emit_legacy_aliases,
    )
}

/// Builds one span entirely from caller-supplied values.
///
/// This function performs no filesystem, environment, clock, random, or
/// tracing-context reads. The daemon should resolve all ambient data before
/// calling it; the historical builders above remain compatibility wrappers.
#[allow(clippy::too_many_arguments)]
pub fn build_span_from_resolved(
    event: HookEvent,
    input: &AgentHookInput,
    start_time_unix_nano: u64,
    end_time_unix_nano: u64,
    salt: u32,
    context: ResolvedTraceContext,
    metadata: ResolvedSpanMetadata<'_>,
    emit_legacy_aliases: bool,
) -> Span {
    let tool_name = input.resolved_tool_name();
    let tool_call_id = input.resolved_tool_call_id();
    let conv_id = input.conversation_id.as_deref();
    let step_idx = input.step_idx;

    let trace_id = context.trace_id;
    let parent_span_id = context.parent_span_id;
    let span_id = derive_span_id(conv_id, step_idx, event, tool_name, salt);

    let provider = input
        .model
        .as_deref()
        .map(infer_provider)
        .unwrap_or(GEN_AI_PROVIDER_GOOGLE);

    let agent_name = input
        .agent_name
        .as_deref()
        .unwrap_or_else(|| infer_agent_name(input.model.as_deref(), None));

    // Canonical OTel GenAI operation name and span name
    let (op_name, span_name) = match event {
        HookEvent::PreInvocation => (
            GEN_AI_OPERATION_INVOKE_AGENT,
            format!("invoke_agent {agent_name}"),
        ),
        HookEvent::PostInvocation => (
            GEN_AI_OPERATION_INVOKE_AGENT,
            format!("invoke_agent {agent_name}"),
        ),
        HookEvent::PreToolUse => {
            let name = if let Some(t) = tool_name {
                format!("execute_tool {t}")
            } else {
                "execute_tool".to_string()
            };
            (GEN_AI_OPERATION_EXECUTE_TOOL, name)
        }
        HookEvent::PostToolUse => {
            let name = if let Some(t) = tool_name {
                format!("execute_tool {t}")
            } else {
                "execute_tool".to_string()
            };
            (GEN_AI_OPERATION_EXECUTE_TOOL, name)
        }
        HookEvent::Stop => ("agent.stop", "agent.stop".to_string()),
        HookEvent::Unknown => ("agent.unknown", "agent.unknown".to_string()),
    };

    let mut attributes = vec![
        // Canonical GenAI SemConv
        kv_string(GEN_AI_OPERATION_NAME, op_name),
        kv_string(GEN_AI_PROVIDER_NAME, provider),
        kv_string(GEN_AI_SYSTEM, provider),
        kv_string(GEN_AI_AGENT_NAME, agent_name),
        // Canonical Agent attribute
        kv_string(AGENT_HOOK_EVENT, event.as_str()),
    ];

    if emit_legacy_aliases {
        attributes.push(kv_string(AGY_HOOK_EVENT, event.as_str()));
    }

    if let Some(cid) = conv_id {
        attributes.push(kv_string(GEN_AI_CONVERSATION_ID, cid));
    }
    if let Some(model) = input.model.as_deref() {
        attributes.push(kv_string(GEN_AI_REQUEST_MODEL, model));
    }
    if let Some(tool) = tool_name {
        attributes.push(kv_string(GEN_AI_TOOL_NAME, tool));
    }
    if let Some(t_id) = tool_call_id {
        attributes.push(kv_string(GEN_AI_TOOL_CALL_ID, t_id));
    }
    if let Some(reason) = input.termination_reason.as_deref() {
        attributes.push(kv_string(AGENT_TERMINATION_REASON, reason));
        attributes.push(kv_string(GEN_AI_RESPONSE_FINISH_REASONS, reason));
        if emit_legacy_aliases {
            attributes.push(kv_string(AGY_TERMINATION_REASON, reason));
        }
    }
    if let Some(step) = step_idx {
        attributes.push(kv_int(AGENT_STEP_INDEX, step as i64));
        attributes.push(kv_int(GEN_AI_AGENT_STEP_INDEX, step as i64));
        if emit_legacy_aliases {
            attributes.push(kv_int(AGY_STEP_INDEX, step as i64));
        }
    }
    if let Some(num) = input.execution_num {
        attributes.push(kv_int(AGENT_EXECUTION_NUM, num));
        if emit_legacy_aliases {
            attributes.push(kv_int(AGY_EXECUTION_NUM, num));
        }
    }
    if let Some(idle) = input.fully_idle {
        attributes.push(kv_bool(AGENT_FULLY_IDLE, idle));
        if emit_legacy_aliases {
            attributes.push(kv_bool(AGY_FULLY_IDLE, idle));
        }
    }
    if let Some(it) = input.input_tokens {
        attributes.push(kv_int(GEN_AI_USAGE_INPUT_TOKENS, it));
    }
    if let Some(ot) = input.output_tokens {
        attributes.push(kv_int(GEN_AI_USAGE_OUTPUT_TOKENS, ot));
    }
    if let Some(ct) = input.cached_tokens {
        attributes.push(kv_int(GEN_AI_USAGE_CACHE_READ_TOKENS, ct));
    }
    if let Some(decision) = input.decision.as_deref() {
        attributes.push(kv_string(AGENT_DECISION, decision));
    }
    if let Some(success) = input.success {
        attributes.push(kv_bool(AGENT_SUCCESS, success));
    }

    // Identity and terminal values were resolved outside this pure builder.
    if let Some(email) = input.user_email.as_deref().or(metadata.user_email) {
        attributes.push(kv_string(USER_EMAIL, email));
    }

    if let Some(term) = input.terminal_type.as_deref().or(metadata.terminal_type) {
        attributes.push(kv_string(TERMINAL_TYPE, term));
    }

    if let Some(mode) = input.execution_mode {
        attributes.push(kv_string(AGENT_EXECUTION_MODE, mode.as_str()));
    }
    if let Some(added) = input.git_lines_added {
        attributes.push(kv_int(AGENT_GIT_LINES_ADDED, added as i64));
    }
    if let Some(deleted) = input.git_lines_deleted {
        attributes.push(kv_int(AGENT_GIT_LINES_DELETED, deleted as i64));
    }
    if let Some(changed) = input.git_files_changed {
        attributes.push(kv_int(AGENT_GIT_FILES_CHANGED, changed as i64));
    }
    if let Some(revert) = input.git_self_revert {
        attributes.push(kv_bool(AGENT_GIT_SELF_REVERT, revert));
    }

    // Execution Context & Workspace (v0.3)
    if let Some(path) = input.workspace_path.as_deref() {
        attributes.push(kv_string(WORKSPACE_PATH, path));
    }
    if let Some(pname) = input.project_name.as_deref() {
        attributes.push(kv_string(WORKSPACE_PROJECT_NAME, pname));
    }
    if let Some(proot) = input.project_root.as_deref() {
        attributes.push(kv_string(WORKSPACE_PROJECT_ROOT, proot));
    }
    if let Some(ptype) = input.project_type.as_deref() {
        attributes.push(kv_string(WORKSPACE_PROJECT_TYPE, ptype));
    }

    // VCS / Git (v0.3)
    if let Some(vcs) = input.vcs_system.as_deref() {
        attributes.push(kv_string(VCS_SYSTEM, vcs));
    }
    if let Some(repo) = input.vcs_repository.as_deref() {
        attributes.push(kv_string(VCS_REPOSITORY_NAME, repo));
    }
    if let Some(branch) = input.vcs_branch.as_deref() {
        attributes.push(kv_string(VCS_BRANCH_NAME, branch));
    }
    if let Some(commit) = input.vcs_commit.as_deref() {
        attributes.push(kv_string(VCS_COMMIT_SHA, commit));
    }
    if let Some(worktree) = input.vcs_worktree {
        attributes.push(kv_bool(VCS_WORKTREE_ACTIVE, worktree));
    }

    // Tool Archetypes & I/O Economics (v0.3)
    if let Some(arch) = input.tool_archetype.as_deref() {
        attributes.push(kv_string(AGENT_TOOL_ARCHETYPE, arch));
    }
    if let Some(bin) = input.tool_binary.as_deref() {
        attributes.push(kv_string(AGENT_TOOL_BINARY, bin));
    }
    if let Some(wbin) = input.tool_wrapped_binary.as_deref() {
        attributes.push(kv_string(AGENT_TOOL_WRAPPED_BINARY, wbin));
    }
    if let Some(pdepth) = input.tool_pipeline_depth {
        attributes.push(kv_int(AGENT_TOOL_PIPELINE_DEPTH, pdepth as i64));
    }
    if let Some(cratio) = input.tool_compression_ratio {
        attributes.push(KeyValue {
            key: AGENT_TOOL_COMPRESSION_RATIO.to_string(),
            value: Some(AnyValue {
                value: Some(any_value::Value::DoubleValue(cratio)),
            }),
            ..Default::default()
        });
    }
    if let Some(tsaved) = input.tool_tokens_saved {
        attributes.push(kv_int(AGENT_TOOL_TOKENS_SAVED, tsaved));
    }

    // Universal Capabilities & Waste (v0.3)
    if let Some(ckind) = input.capability_kind.as_deref() {
        attributes.push(kv_string(CAPABILITY_KIND, ckind));
    }
    if let Some(cns) = input.capability_namespace.as_deref() {
        attributes.push(kv_string(CAPABILITY_NAMESPACE, cns));
    }
    if let Some(cname) = input.capability_name.as_deref() {
        attributes.push(kv_string(CAPABILITY_NAME, cname));
    }
    if let Some(st) = input.capability_schema_tokens {
        attributes.push(kv_int(CAPABILITY_SCHEMA_TOKENS, st));
    }
    if let Some(rb) = input.capability_response_bytes {
        attributes.push(kv_int(CAPABILITY_RESPONSE_BYTES, rb as i64));
    }
    if let Some(rt) = input.capability_response_tokens {
        attributes.push(kv_int(CAPABILITY_RESPONSE_TOKENS, rt));
    }
    if let Some(retries) = input.capability_consecutive_retries {
        attributes.push(kv_int(CAPABILITY_CONSECUTIVE_RETRIES, retries as i64));
    }
    if let Some(is_waste) = input.capability_is_waste {
        attributes.push(kv_bool(CAPABILITY_IS_WASTE, is_waste));
    }

    // Cross-Agent Lineage (v0.3)
    if let Some(depth) = input.agent_depth {
        attributes.push(kv_int(GEN_AI_AGENT_DEPTH, depth as i64));
    }
    if let Some(parent_name) = input.agent_parent_name.as_deref() {
        attributes.push(kv_string(GEN_AI_AGENT_PARENT_NAME, parent_name));
    }
    if let Some(root_id) = input.agent_root_id.as_deref() {
        attributes.push(kv_string(GEN_AI_AGENT_ROOT_ID, root_id));
    }
    if let Some(is_root) = input.agent_is_root {
        attributes.push(kv_bool(GEN_AI_AGENT_IS_ROOT, is_root));
    }

    // Multi-Layer Error Categorization (v0.3)
    if let Some(ecat) = input.error_category.as_deref() {
        attributes.push(kv_string(AGENT_ERROR_CATEGORY, ecat));
    }

    let (status_code, status_msg) = match &input.error {
        Some(err) if !err.trim().is_empty() => (StatusCode::Error as i32, err.clone()),
        _ => (StatusCode::Ok as i32, String::new()),
    };

    Span {
        trace_id: trace_id.to_vec(),
        span_id: span_id.to_vec(),
        parent_span_id: parent_span_id.map(|p| p.to_vec()).unwrap_or_default(),
        flags: context.trace_flags as u32,
        name: span_name,
        kind: SpanKind::Internal as i32,
        start_time_unix_nano,
        end_time_unix_nano: if end_time_unix_nano >= start_time_unix_nano {
            end_time_unix_nano
        } else {
            start_time_unix_nano
        },
        attributes,
        status: Some(Status {
            code: status_code,
            message: status_msg,
        }),
        ..Default::default()
    }
}

pub fn build_trace_request(resource: Resource, spans: Vec<Span>) -> ExportTraceServiceRequest {
    ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            resource: Some(resource),
            scope_spans: vec![ScopeSpans {
                scope: Some(instrumentation_scope()),
                spans,
                schema_url: String::new(),
            }],
            schema_url: String::new(),
        }],
    }
}
