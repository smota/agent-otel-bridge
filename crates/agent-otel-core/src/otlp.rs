/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use opentelemetry_proto::tonic::{
    collector::trace::v1::ExportTraceServiceRequest,
    common::v1::{any_value, AnyValue, InstrumentationScope, KeyValue},
    resource::v1::Resource,
    trace::v1::{
        span::SpanKind, status::StatusCode, ResourceSpans, ScopeSpans, Span, Status,
    },
};

use crate::model::{AntigravityHookInput, HookEvent};
use crate::semconv::*;
use crate::trace_id::{derive_span_id, derive_trace_id};

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
    Resource {
        attributes: vec![
            kv_string(SERVICE_NAME, service_name),
            kv_string(DEPLOYMENT_ENVIRONMENT, environment),
            kv_string(SERVICE_VERSION, version),
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
    input: &AntigravityHookInput,
    start_time_unix_nano: u64,
    end_time_unix_nano: u64,
    salt: u32,
) -> Span {
    let tool_name = input.tool_call.as_ref().and_then(|tc| tc.name.as_deref());
    let tool_call_id = input.tool_call.as_ref().and_then(|tc| tc.id.as_deref());
    let conv_id = input.conversation_id.as_deref();
    let step_idx = input.step_idx;

    let trace_id = derive_trace_id(conv_id);
    let span_id = derive_span_id(conv_id, step_idx, event, tool_name, salt);

    // Canonical OTel GenAI operation name and span name
    let (op_name, span_name) = match event {
        HookEvent::PreInvocation => (
            GEN_AI_OPERATION_INVOKE_AGENT,
            "agy.pre_invocation".to_string(),
        ),
        HookEvent::PostInvocation => (
            GEN_AI_OPERATION_INVOKE_AGENT,
            "agy.post_invocation".to_string(),
        ),
        HookEvent::PreToolUse => {
            let name = if let Some(t) = tool_name {
                format!("agy.pre_tool_use {t}")
            } else {
                "agy.pre_tool_use".to_string()
            };
            (GEN_AI_OPERATION_EXECUTE_TOOL, name)
        }
        HookEvent::PostToolUse => {
            let name = if let Some(t) = tool_name {
                format!("agy.post_tool_use {t}")
            } else {
                "agy.post_tool_use".to_string()
            };
            (GEN_AI_OPERATION_EXECUTE_TOOL, name)
        }
        HookEvent::Stop => ("agent.stop", "agy.stop".to_string()),
        HookEvent::Unknown => ("agent.unknown", "agy.unknown".to_string()),
    };

    let mut attributes = vec![
        // Canonical GenAI SemConv
        kv_string(GEN_AI_OPERATION_NAME, op_name),
        kv_string(GEN_AI_PROVIDER_NAME, GEN_AI_PROVIDER_GOOGLE),
        kv_string(GEN_AI_SYSTEM, GEN_AI_SYSTEM_DEFAULT),
        kv_string(GEN_AI_AGENT_NAME, "antigravity"),
        // Exact dashboard match
        kv_string(AGY_HOOK_EVENT, event.as_str()),
    ];

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
        attributes.push(kv_string(AGY_TERMINATION_REASON, reason));
    }
    if let Some(step) = step_idx {
        attributes.push(kv_int(AGY_STEP_INDEX, step as i64));
    }
    if let Some(num) = input.execution_num {
        attributes.push(kv_int(AGY_EXECUTION_NUM, num));
    }
    if let Some(idle) = input.fully_idle {
        attributes.push(kv_bool(AGY_FULLY_IDLE, idle));
    }

    let (status_code, status_msg) = match &input.error {
        Some(err) => (StatusCode::Error as i32, err.clone()),
        None => (StatusCode::Ok as i32, String::new()),
    };

    Span {
        trace_id: trace_id.to_vec(),
        span_id: span_id.to_vec(),
        parent_span_id: vec![],
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
