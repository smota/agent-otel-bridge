/*
 * Copyright agent-otel-bridge contributors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_core::model::{ClientKind, ExecutionMode, HookEvent};
use agent_otel_core::otlp::ResolvedSpanMetadata;
use agent_otel_core::quota::build_multi_quota_metrics_request_opts;
use agent_otel_daemon::batch::SpanBatcher;
use agent_otel_daemon::config::DaemonConfig;
use agent_otel_daemon::config::{CaptureMode, WorkspacePathMode};
use agent_otel_daemon::context_cache::{ContextCache, ContextCacheStats};
use agent_otel_daemon::diagnostic_metrics::append_bridge_metrics;
use agent_otel_daemon::diagnostics::Diagnostics;
use agent_otel_daemon::quota::QuotaEngine;
use agent_otel_daemon::span_assembler::SpanAssembler;
use agent_otel_daemon::transform::{production_transform, production_transform_with_capture_mode};
use agent_otel_ipc::frame::{encode_context_payload, MsgType, WireHeader};
use agent_otel_ipc::server::IngressStats;
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::any_value;
use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span};
use prost::Message;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SENTINELS: &[&str] = &[
    "sentinel.user.f5@secret-corp.example",
    "secret-corp.example",
    "SENTINEL_HOME_PATH_11",
    "SENTINEL-HOST-F5",
    "sentinel-host.internal",
    "SENTINEL_USER_77",
    "SENTINEL_BRANCH_SECRET_42",
    "ghp_SENTINEL_TOKEN_88",
    "SENTINEL_TOKEN_88",
    "SENTINEL_REPO_99",
    "SENTINEL_PROMPT_TEXT_TOP_SECRET",
    "SENTINEL_CMD_BINARY",
    "SENTINEL_TOOL_OUTPUT_STDOUT_DATA",
    "SENTINEL_TOOL_OUTPUT_STDERR_DATA",
    "SENTINEL_ERROR_RAW_MESSAGE_555",
];

const G0A_SENTINELS: &[&str] = &[
    "G0A_ARG_SENTINEL",
    "G0A_ERR_SENTINEL",
    "G0A_ARG_SENTINEL_S2",
    "G0A_ERR_SENTINEL_S2",
    "G0A_ARG_SENTINEL_S3",
    "G0A_SIDECHAIN_SENTINEL_S3",
    "G0A_RUN_",
    "PATH_1bc2e6a412d846c2",
    "PATH_50e028c78d6e3f1b",
    "PATH_f7391ca0f3631869",
    "PATH_72be49c261597ac5",
    "EMAIL_a59b02022923c70d",
    "HOST_644f019937e80a7b",
];

fn create_sentinel_workspace() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let base = std::env::temp_dir().join(format!("aob-f5-{}-{nonce}", std::process::id()));
    let root = base
        .join("SENTINEL_USER_77")
        .join("SENTINEL_HOME_PATH_11")
        .join("SENTINEL_REPO_99");
    fs::create_dir_all(root.join(".git/refs/heads/feat")).expect("create git dirs");
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"SENTINEL_REPO_99\"\nversion = \"0.1.0\"\n",
    )
    .expect("write Cargo.toml");
    fs::write(
        root.join(".git/HEAD"),
        "ref: refs/heads/feat/SENTINEL_BRANCH_SECRET_42\n",
    )
    .expect("write .git/HEAD");
    fs::write(
        root.join(".git/refs/heads/feat/SENTINEL_BRANCH_SECRET_42"),
        "abcdef0123456789abcdef0123456789abcdef01\n",
    )
    .expect("write branch ref");
    fs::write(
        root.join(".git/config"),
        "[remote \"origin\"]\nurl = https://SENTINEL_USER_77:ghp_SENTINEL_TOKEN_88@git.sentinel-host.internal/SENTINEL_USER_77/SENTINEL_REPO_99.git\n[user]\nemail = sentinel.user.f5@secret-corp.example\n",
    )
    .expect("write .git/config");
    base
}

fn wait_for_cache_ready(cache: &ContextCache, workspace_path: &str) {
    let probe = agent_otel_core::model::AgentHookInput {
        workspace_path: Some(workspace_path.to_string()),
        ..Default::default()
    };
    let _ = cache.lookup(&probe, Instant::now());
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        let stats = cache.stats(Instant::now());
        if stats.completed >= 1 {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("context cache did not complete refresh within deadline");
}

fn collect_all_strings_from_trace_request(req: &ExportTraceServiceRequest) -> Vec<String> {
    let mut out = Vec::new();
    for rs in &req.resource_spans {
        out.push(rs.schema_url.clone());
        if let Some(res) = &rs.resource {
            for kv in &res.attributes {
                out.push(kv.key.clone());
                if let Some(val) = &kv.value {
                    if let Some(any_value::Value::StringValue(s)) = &val.value {
                        out.push(s.clone());
                    }
                }
            }
        }
        for ss in &rs.scope_spans {
            out.push(ss.schema_url.clone());
            if let Some(scope) = &ss.scope {
                out.push(scope.name.clone());
                out.push(scope.version.clone());
            }
            for span in &ss.spans {
                out.push(span.name.clone());
                out.push(span.trace_state.clone());
                if let Some(status) = &span.status {
                    out.push(status.message.clone());
                }
                for kv in &span.attributes {
                    out.push(kv.key.clone());
                    if let Some(val) = &kv.value {
                        match &val.value {
                            Some(any_value::Value::StringValue(s)) => out.push(s.clone()),
                            Some(any_value::Value::ArrayValue(arr)) => {
                                for item in &arr.values {
                                    if let Some(any_value::Value::StringValue(s)) = &item.value {
                                        out.push(s.clone());
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                for event in &span.events {
                    out.push(event.name.clone());
                    for kv in &event.attributes {
                        out.push(kv.key.clone());
                        if let Some(val) = &kv.value {
                            if let Some(any_value::Value::StringValue(s)) = &val.value {
                                out.push(s.clone());
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

fn collect_all_strings_from_metrics_request(req: &ExportMetricsServiceRequest) -> Vec<String> {
    let mut out = Vec::new();
    for rm in &req.resource_metrics {
        out.push(rm.schema_url.clone());
        if let Some(res) = &rm.resource {
            for kv in &res.attributes {
                out.push(kv.key.clone());
                if let Some(val) = &kv.value {
                    if let Some(any_value::Value::StringValue(s)) = &val.value {
                        out.push(s.clone());
                    }
                }
            }
        }
        for sm in &rm.scope_metrics {
            for metric in &sm.metrics {
                out.push(metric.name.clone());
                out.push(metric.description.clone());
                if let Some(data) = &metric.data {
                    use opentelemetry_proto::tonic::metrics::v1::metric::Data;
                    let points = match data {
                        Data::Gauge(g) => &g.data_points,
                        Data::Sum(s) => &s.data_points,
                        _ => continue,
                    };
                    for pt in points {
                        for kv in &pt.attributes {
                            out.push(kv.key.clone());
                            if let Some(val) = &kv.value {
                                if let Some(any_value::Value::StringValue(s)) = &val.value {
                                    out.push(s.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

fn assert_no_sentinels_in_bytes_or_strings(
    label: &str,
    proto_bytes: &[u8],
    extracted_strings: &[String],
    sentinels: &[&str],
) {
    let lossy_proto = String::from_utf8_lossy(proto_bytes);
    for &sentinel in sentinels {
        assert!(
            !lossy_proto.contains(sentinel),
            "[{label}] serialized OTLP protobuf bytes leaked sentinel substring {sentinel:?}"
        );
        for s in extracted_strings {
            assert!(
                !s.contains(sentinel),
                "[{label}] OTLP field {s:?} leaked sentinel substring {sentinel:?}"
            );
        }
    }
}

#[test]
fn default_mode_rejects_all_forbidden_field_sentinels_across_spans_events_resources_and_metrics() {
    let base_dir = create_sentinel_workspace();
    let repo_dir = base_dir
        .join("SENTINEL_USER_77")
        .join("SENTINEL_HOME_PATH_11")
        .join("SENTINEL_REPO_99");
    let repo_path_str = repo_dir.to_string_lossy().to_string();

    let cache = ContextCache::new();
    wait_for_cache_ready(&cache, &repo_path_str);

    // Default DaemonConfig with hostile ambient resource attributes and environment
    let config = DaemonConfig::from_vars(&|key| {
        match key {
        "USER_EMAIL" => Some("sentinel.user.f5@secret-corp.example".to_string()),
        "GIT_AUTHOR_EMAIL" => Some("sentinel.user.f5@secret-corp.example".to_string()),
        "HOSTNAME" => Some("SENTINEL-HOST-F5.corp.internal".to_string()),
        "COMPUTERNAME" => Some("SENTINEL-HOST-F5.corp.internal".to_string()),
        "OTEL_RESOURCE_ATTRIBUTES" => Some(
            "host.name=SENTINEL-HOST-F5.corp.internal,user.email=sentinel.user.f5@secret-corp.example,user.name=SENTINEL_USER_77,vcs.repository.url=https://SENTINEL_USER_77:ghp_SENTINEL_TOKEN_88@git.sentinel-host.internal/SENTINEL_USER_77/SENTINEL_REPO_99.git".to_string(),
        ),
        _ => None,
    }
    });

    let resource = config.resource();

    let metadata = ResolvedSpanMetadata {
        user_email: Some("sentinel.user.f5@secret-corp.example"),
        terminal_type: Some("/Users/SENTINEL_USER_77/SENTINEL-HOST-F5"),
    };

    let payload_json = serde_json::json!({
        "conversationId": "conv-sentinel-audit",
        "stepIdx": 7,
        "agent_name": "SENTINEL_USER_77_custom_agent",
        "user_email": "sentinel.user.f5@secret-corp.example",
        "workspacePath": repo_path_str,
        "projectRoot": repo_path_str,
        "projectName": "SENTINEL_REPO_99",
        "vcs.repository.name": "https://SENTINEL_USER_77:ghp_SENTINEL_TOKEN_88@git.sentinel-host.internal/SENTINEL_USER_77/SENTINEL_REPO_99.git",
        "vcs.branch.name": "feat/SENTINEL_BRANCH_SECRET_42",
        "prompt": "SENTINEL_PROMPT_TEXT_TOP_SECRET",
        "toolName": "run_command",
        "toolCall": {
            "id": "call-sentinel-1",
            "name": "run_command",
            "arguments": {
                "CommandLine": "/Users/SENTINEL_USER_77/bin/SENTINEL_CMD_BINARY --token ghp_SENTINEL_TOKEN_88 SENTINEL_PROMPT_TEXT_TOP_SECRET",
                "prompt": "SENTINEL_PROMPT_TEXT_TOP_SECRET"
            }
        },
        "tool_input": {
            "command": "/Users/SENTINEL_USER_77/bin/SENTINEL_CMD_BINARY --token ghp_SENTINEL_TOKEN_88",
            "prompt": "SENTINEL_PROMPT_TEXT_TOP_SECRET"
        },
        "tool_response": {
            "stdout": "SENTINEL_TOOL_OUTPUT_STDOUT_DATA",
            "stderr": "SENTINEL_TOOL_OUTPUT_STDERR_DATA"
        },
        "error": "Fatal error: SENTINEL_ERROR_RAW_MESSAGE_555 for SENTINEL_USER_77 at /Users/SENTINEL_USER_77/SENTINEL_HOME_PATH_11",
        "terminationReason": "Fatal error: SENTINEL_ERROR_RAW_MESSAGE_555",
        "transcript_path": "/Users/SENTINEL_USER_77/SENTINEL_HOME_PATH_11/transcript.jsonl",
        "inputTokens": 120,
        "outputTokens": 45
    });
    let payload_bytes = serde_json::to_vec(&payload_json).expect("serialize sentinel fixture");

    let mut assembler = SpanAssembler::default();
    let mut exported_spans: Vec<Span> = Vec::new();
    let now = Instant::now();

    for (idx, event) in [HookEvent::PreToolUse, HookEvent::PostToolUse]
        .into_iter()
        .enumerate()
    {
        let header = WireHeader::new(ClientKind::Antigravity.to_wire(), event.to_wire());
        let frame = encode_context_payload(header, None, &payload_bytes);
        let outcome = production_transform(
            MsgType::HookPayloadWithContext,
            &frame,
            &cache,
            (idx + 1) as u32,
            ExecutionMode::Interactive,
            true,
            true,
            metadata,
            now + Duration::from_millis(idx as u64 * 10),
            1_700_000_000_000_000_000 + (idx as u64 * 10_000_000),
        )
        .expect("transform sentinel frame");

        for span in assembler.observe(outcome.span, now + Duration::from_millis(idx as u64 * 10)) {
            exported_spans.push(span);
        }
    }
    assert_eq!(exported_spans.len(), 1, "expected paired tool span");

    let mut batcher = SpanBatcher::new(resource.clone(), 1);
    batcher
        .try_push(exported_spans.remove(0), Instant::now())
        .expect("push span");
    let trace_batch = batcher.take().expect("batch ready");

    let trace_bytes = trace_batch.request.encode_to_vec();
    let trace_strings = collect_all_strings_from_trace_request(&trace_batch.request);
    assert_no_sentinels_in_bytes_or_strings(
        "synthetic_sentinel_traces",
        &trace_bytes,
        &trace_strings,
        SENTINELS,
    );

    // Also test metrics export path (QuotaEngine + bridge diagnostic metrics) with the same resource & activity
    let quota_engine = QuotaEngine::new();
    quota_engine.record_activity("SENTINEL_USER_77_custom_agent", Some(165));
    let snapshots = quota_engine.snapshots();
    let mut metrics_req = build_multi_quota_metrics_request_opts(
        resource.clone(),
        &snapshots,
        config.emit_legacy_aliases,
    );
    append_bridge_metrics(
        &mut metrics_req,
        1_700_000_000_000_000_000,
        &IngressStats::default(),
        &Diagnostics::default().snapshot(),
        &ContextCacheStats::default(),
        config.gen_ai_latest_experimental,
    );
    let metrics_bytes = metrics_req.encode_to_vec();
    let metrics_strings = collect_all_strings_from_metrics_request(&metrics_req);
    assert_no_sentinels_in_bytes_or_strings(
        "synthetic_sentinel_metrics",
        &metrics_bytes,
        &metrics_strings,
        SENTINELS,
    );

    // Resource must carry agent.capture.mode = "minimal" in default mode
    let capture_mode_attr = resource.attributes.iter().find_map(|kv| {
        if kv.key == "agent.capture.mode" {
            match kv.value.as_ref()?.value.as_ref()? {
                any_value::Value::StringValue(s) => Some(s.as_str()),
                _ => None,
            }
        } else {
            None
        }
    });
    assert_eq!(
        capture_mode_attr,
        Some("minimal"),
        "default resource must stamp agent.capture.mode=minimal"
    );

    let _ = fs::remove_dir_all(base_dir);
}

#[test]
fn g0a_captures_replay_in_default_mode_emits_zero_sentinels() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let captures_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("docs/contracts/fact-export-v1/captures/g0a-20261112/claude-2.1.322");

    let cache = ContextCache::new();
    let config = DaemonConfig::from_vars(&|_| None);
    let resource = config.resource();
    let metadata = ResolvedSpanMetadata::default();
    let mut assembler = SpanAssembler::default();
    let mut spans = Vec::new();
    let now = Instant::now();
    let mut seq = 0u32;

    for scenario in ["S1", "S2", "S3"] {
        let jsonl_path = captures_root.join(scenario).join("hook-sanitized.jsonl");
        let content = fs::read_to_string(&jsonl_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", jsonl_path.display()));
        for line in content.lines() {
            let record: serde_json::Value =
                serde_json::from_str(line).expect("parse G0a capture JSONL line");
            let Some(stdin_obj) = record.get("stdin") else {
                continue;
            };
            let mut stdin_patched = stdin_obj.clone();
            if let Some(obj) = stdin_patched.as_object_mut() {
                if obj.contains_key("conversation_id") && obj.contains_key("session_id") {
                    obj.remove("session_id");
                }
                if let Some(cwd) = obj.get("cwd").cloned() {
                    obj.entry("workspacePath").or_insert(cwd);
                }
            }
            let header_obj = record.get("header");
            let client_id = header_obj
                .and_then(|h| h.get("client_id"))
                .and_then(|v| v.as_u64())
                .unwrap_or(2) as u16;
            let event_id = header_obj
                .and_then(|h| h.get("event_id"))
                .and_then(|v| v.as_u64())
                .unwrap_or(3) as u8;
            let payload_bytes = serde_json::to_vec(&stdin_patched).expect("serialize G0a stdin");
            let header = WireHeader::new(client_id, event_id);
            let frame = encode_context_payload(header, None, &payload_bytes);
            seq = seq.wrapping_add(1);
            let Ok(outcome) = production_transform(
                MsgType::HookPayloadWithContext,
                &frame,
                &cache,
                seq,
                ExecutionMode::Interactive,
                true,
                true,
                metadata,
                now + Duration::from_millis(seq as u64),
                1_794_355_200_000_000_000 + (seq as u64 * 1_000_000),
            ) else {
                continue;
            };
            spans.push(outcome.span.clone());
            for paired in assembler.observe(outcome.span, now + Duration::from_millis(seq as u64)) {
                spans.push(paired);
            }
        }
    }

    assert!(!spans.is_empty(), "expected spans from G0a captures");
    let trace_req = ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            resource: Some(resource),
            scope_spans: vec![ScopeSpans {
                scope: None,
                spans,
                schema_url: String::new(),
            }],
            schema_url: String::new(),
        }],
    };
    let trace_bytes = trace_req.encode_to_vec();
    let trace_strings = collect_all_strings_from_trace_request(&trace_req);
    assert_no_sentinels_in_bytes_or_strings(
        "g0a_captures_replay",
        &trace_bytes,
        &trace_strings,
        G0A_SENTINELS,
    );
}

#[test]
fn opt_in_modes_stamp_resource_and_never_export_credentials_or_prompt_content() {
    let cache = ContextCache::new();
    let metadata = ResolvedSpanMetadata {
        user_email: Some("dev@example.com"),
        terminal_type: Some("vscode"),
    };
    let payload_json = serde_json::json!({
        "conversationId": "conv-opt-in",
        "stepIdx": 1,
        "workspacePath": "/work/repo",
        "projectRoot": "/work/repo",
        "projectName": "my-project",
        "vcs.repository.name": "https://user:ghp_SECRET_CREDENTIAL_TOKEN_999@github.com/org/my-project.git?token=sk-SECRET_QUERY_TOKEN_888",
        "vcs.branch.name": "feat/custom-branch-name",
        "prompt": "SENTINEL_PROMPT_NEVER_EXPORTED",
        "toolName": "run_command",
        "toolCall": {
            "id": "call-opt-1",
            "name": "run_command",
            "arguments": {
                "CommandLine": "curl -H 'Authorization: Bearer ghp_SECRET_CREDENTIAL_TOKEN_999' https://example.com"
            }
        },
        "tool_response": {
            "stdout": "SENTINEL_TOOL_STDOUT_NEVER_EXPORTED"
        },
        "error": "Auth failed with token=ghp_SECRET_CREDENTIAL_TOKEN_999 and Bearer sk-SECRET_BEARER_777: SENTINEL_RAW_ERROR_DETAIL"
    });
    let payload_bytes = serde_json::to_vec(&payload_json).expect("serialize opt-in fixture");
    let header = WireHeader::new(
        ClientKind::Antigravity.to_wire(),
        HookEvent::PostToolUse.to_wire(),
    );
    let frame = encode_context_payload(header, None, &payload_bytes);

    let credential_sentinels = &[
        "ghp_SECRET_CREDENTIAL_TOKEN_999",
        "SECRET_CREDENTIAL_TOKEN_999",
        "sk-SECRET_QUERY_TOKEN_888",
        "SECRET_QUERY_TOKEN_888",
        "sk-SECRET_BEARER_777",
        "SECRET_BEARER_777",
        "SENTINEL_PROMPT_NEVER_EXPORTED",
        "SENTINEL_TOOL_STDOUT_NEVER_EXPORTED",
    ];

    // 1. MetadataOnly mode: exports operational metadata, drops raw error content, scrubs credentials
    let meta_cfg = DaemonConfig::from_vars(&|key| {
        match key {
        "AGENT_OTEL_CAPTURE_MODE" => Some("metadata_only".to_string()),
        "OTEL_RESOURCE_ATTRIBUTES" => Some(
            "custom.url=https://admin:ghp_SECRET_CREDENTIAL_TOKEN_999@internal.example/api?token=sk-SECRET_QUERY_TOKEN_888".to_string(),
        ),
        _ => None,
    }
    });
    let meta_res = meta_cfg.resource();
    let meta_outcome = production_transform_with_capture_mode(
        MsgType::HookPayloadWithContext,
        &frame,
        &cache,
        1,
        ExecutionMode::Interactive,
        false,
        false,
        metadata,
        Instant::now(),
        1_700_000_000_000_000_000,
        WorkspacePathMode::Hash,
        CaptureMode::MetadataOnly,
    )
    .expect("metadata_only transform");

    // In MetadataOnly mode, raw error detail is replaced with closed category
    assert_eq!(
        meta_outcome
            .span
            .status
            .as_ref()
            .map(|s| s.message.as_str()),
        Some("tool_verification_failed")
    );
    let meta_req = ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            resource: Some(meta_res),
            scope_spans: vec![ScopeSpans {
                scope: None,
                spans: vec![meta_outcome.span],
                schema_url: String::new(),
            }],
            schema_url: String::new(),
        }],
    };
    assert_no_sentinels_in_bytes_or_strings(
        "metadata_only_mode",
        &meta_req.encode_to_vec(),
        &collect_all_strings_from_trace_request(&meta_req),
        &[
            "ghp_SECRET_CREDENTIAL_TOKEN_999",
            "SECRET_CREDENTIAL_TOKEN_999",
            "sk-SECRET_QUERY_TOKEN_888",
            "SECRET_QUERY_TOKEN_888",
            "sk-SECRET_BEARER_777",
            "SECRET_BEARER_777",
            "SENTINEL_PROMPT_NEVER_EXPORTED",
            "SENTINEL_TOOL_STDOUT_NEVER_EXPORTED",
            "SENTINEL_RAW_ERROR_DETAIL",
        ],
    );

    // 2. Full mode: keeps bounded error text, but still scrubs all credentials and never exports prompts/tool output
    let full_cfg = DaemonConfig::from_vars(&|key| {
        match key {
        "AGENT_OTEL_CAPTURE_MODE" => Some("full".to_string()),
        "OTEL_RESOURCE_ATTRIBUTES" => Some(
            "custom.url=https://admin:ghp_SECRET_CREDENTIAL_TOKEN_999@internal.example/api?token=sk-SECRET_QUERY_TOKEN_888".to_string(),
        ),
        _ => None,
    }
    });
    let full_res = full_cfg.resource();
    let full_outcome = production_transform_with_capture_mode(
        MsgType::HookPayloadWithContext,
        &frame,
        &cache,
        2,
        ExecutionMode::Interactive,
        false,
        false,
        metadata,
        Instant::now(),
        1_700_000_000_000_000_000,
        WorkspacePathMode::Absolute,
        CaptureMode::Full,
    )
    .expect("full transform");

    let full_req = ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            resource: Some(full_res),
            scope_spans: vec![ScopeSpans {
                scope: None,
                spans: vec![full_outcome.span],
                schema_url: String::new(),
            }],
            schema_url: String::new(),
        }],
    };
    assert_no_sentinels_in_bytes_or_strings(
        "full_mode",
        &full_req.encode_to_vec(),
        &collect_all_strings_from_trace_request(&full_req),
        credential_sentinels,
    );
}
