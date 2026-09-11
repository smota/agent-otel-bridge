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
use crate::trace_id::{derive_span_id, resolve_trace_and_parent_id};

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
    let tool_name = input.resolved_tool_name();
    let tool_call_id = input.resolved_tool_call_id();
    let conv_id = input.conversation_id.as_deref();
    let step_idx = input.step_idx;

    let (trace_id, parent_span_id) =
        resolve_trace_and_parent_id(input.traceparent.as_deref(), conv_id);
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

    // Identity & Terminal context (payload or environment fallback)
    let email_opt = input
        .user_email
        .clone()
        .or_else(|| std::env::var("USER_EMAIL").ok())
        .or_else(|| std::env::var("GIT_AUTHOR_EMAIL").ok());
    if let Some(email) = email_opt {
        attributes.push(kv_string(USER_EMAIL, &email));
    }

    let terminal_opt = input
        .terminal_type
        .clone()
        .or_else(|| std::env::var("TERM_PROGRAM").ok())
        .or_else(|| std::env::var("WT_SESSION").ok().map(|_| "windows-terminal".to_string()))
        .or_else(|| std::env::var("TERM").ok());
    if let Some(term) = terminal_opt {
        attributes.push(kv_string(TERMINAL_TYPE, &term));
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

    let (status_code, status_msg) = match &input.error {
        Some(err) => (StatusCode::Error as i32, err.clone()),
        None => (StatusCode::Ok as i32, String::new()),
    };

    Span {
        trace_id: trace_id.to_vec(),
        span_id: span_id.to_vec(),
        parent_span_id: parent_span_id.map(|p| p.to_vec()).unwrap_or_default(),
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
