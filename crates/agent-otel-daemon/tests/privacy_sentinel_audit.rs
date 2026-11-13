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
use agent_otel_daemon::pseudonym_key::PseudonymKey;
use agent_otel_daemon::quota::QuotaEngine;
use agent_otel_daemon::span_assembler::SpanAssembler;
use agent_otel_daemon::transform::production_transform_with_capture_mode;
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
    "abcdef0123456789abcdef0123456789abcdef01",
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
    "G0A_PROMPT_SENTINEL",
    "PATH_1bc2e6a412d846c2",
    "PATH_50e028c78d6e3f1b",
    "PATH_f7391ca0f3631869",
    "PATH_72be49c261597ac5",
    "EMAIL_a59b02022923c70d",
    "HOST_644f019937e80a7b",
];

#[test]
fn commit_aliases_are_keyed_in_minimal_and_raw_only_in_opt_in_modes() {
    let base = create_sentinel_workspace();
    let key = PseudonymKey::load_or_create(&base.join("privacy-home")).unwrap();
    let raw = "ABCDEF0123456789ABCDEF0123456789ABCDEF01";
    let payload = serde_json::to_vec(&serde_json::json!({"vcsCommit": raw})).unwrap();
    let frame = encode_context_payload(
        WireHeader::new(
            ClientKind::Antigravity.to_wire(),
            HookEvent::PreToolUse.to_wire(),
        ),
        None,
        &payload,
    );
    let cache = ContextCache::new();
    for latest in [false, true] {
        for mode in [
            CaptureMode::Minimal,
            CaptureMode::MetadataOnly,
            CaptureMode::Full,
        ] {
            for available_key in [Some(&key), None] {
                let outcome = production_transform_with_capture_mode(
                    MsgType::HookPayloadWithContext,
                    &frame,
                    &cache,
                    1,
                    ExecutionMode::Interactive,
                    false,
                    latest,
                    ResolvedSpanMetadata::default(),
                    Instant::now(),
                    1,
                    WorkspacePathMode::Hash,
                    mode,
                    available_key,
                )
                .unwrap();
                let (name, absent_alias) = if latest {
                    ("vcs.ref.head.revision", "vcs.commit.sha")
                } else {
                    ("vcs.commit.sha", "vcs.ref.head.revision")
                };
                let value = outcome
                    .span
                    .attributes
                    .iter()
                    .find(|kv| kv.key == name)
                    .and_then(|kv| match kv.value.as_ref()?.value.as_ref()? {
                        any_value::Value::StringValue(s) => Some(s.as_str()),
                        _ => None,
                    });
                assert!(!outcome
                    .span
                    .attributes
                    .iter()
                    .any(|kv| kv.key == absent_alias));
                if mode == CaptureMode::Minimal {
                    if available_key.is_some() {
                        let value = value.expect("keyed commit must be exported");
                        assert!(value.starts_with("commit_"));
                        assert_eq!(value.len(), 39);
                    } else {
                        assert_eq!(value, None);
                    }
                    let bytes =
                        String::from_utf8_lossy(&outcome.span.encode_to_vec()).to_ascii_lowercase();
                    assert!(!bytes.contains(&raw.to_ascii_lowercase()));
                } else {
                    assert_eq!(value, Some(raw));
                }
            }
        }
    }
    fs::remove_dir_all(base).unwrap();
}

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
    // Fixture warm-up can encounter cold filesystem/AV latency on Windows.
    // Wait for a real snapshot; this is setup, not an event-path latency gate.
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let lookup = cache.lookup(&probe, Instant::now());
        if lookup.context.is_some() {
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

fn unkeyed_sha256_hex(raw: &str) -> String {
    use sha2::{Digest, Sha256};
    agent_otel_core::link_id::hex_encode(&Sha256::digest(raw.as_bytes()))
}

fn assert_no_sentinels_in_bytes_or_strings(
    label: &str,
    proto_bytes: &[u8],
    extracted_strings: &[String],
    sentinels: &[&str],
) {
    let lossy_proto = String::from_utf8_lossy(proto_bytes);
    for &sentinel in sentinels {
        let old_digest = unkeyed_sha256_hex(sentinel);
        for hex in [&old_digest[..12], &old_digest[..16], old_digest.as_str()] {
            assert!(
                !lossy_proto.contains(hex),
                "[{label}] leaked old unkeyed digest"
            );
            assert!(
                extracted_strings.iter().all(|s| !s.contains(hex)),
                "[{label}] leaked old unkeyed digest"
            );
        }
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

    let key = PseudonymKey::load_or_create(&base_dir.join("privacy-home")).unwrap();
    let other_key = PseudonymKey::load_or_create(&base_dir.join("other-privacy-home")).unwrap();
    for configured_mode in [None, Some("invalid-capture-mode"), Some("minimal")] {
        // Default DaemonConfig with hostile ambient resource attributes and environment
        let config = DaemonConfig::from_vars(&|key| {
            match key {
        "AGENT_OTEL_CAPTURE_MODE" => configured_mode.map(str::to_string),
        "AGENT_OTEL_ENRICH_EMAIL" => Some("true".to_string()),
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

        assert_eq!(config.capture_mode, CaptureMode::Minimal);
        assert_eq!(agent_otel_daemon::daemon::ambient_user_email(&config), None);
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
            "toolArchetype": "SENTINEL_USER_77",
            "capabilityKind": "SENTINEL_USER_77",
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
            let outcome = production_transform_with_capture_mode(
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
                config.workspace_path_mode,
                config.capture_mode,
                Some(&key),
            )
            .expect("transform sentinel frame");

            assert_eq!(outcome.context_source, "workspace_cache");
            let other = production_transform_with_capture_mode(
                MsgType::HookPayloadWithContext,
                &frame,
                &cache,
                (idx + 1) as u32,
                ExecutionMode::Interactive,
                true,
                true,
                metadata,
                now,
                1,
                config.workspace_path_mode,
                config.capture_mode,
                Some(&other_key),
            )
            .unwrap();
            let project = |span: &Span| {
                span.attributes
                    .iter()
                    .find(|kv| kv.key == "workspace.project_name")
                    .cloned()
            };
            assert!(project(&outcome.span).is_some());
            assert_ne!(project(&outcome.span), project(&other.span));
            for span in
                assembler.observe(outcome.span, now + Duration::from_millis(idx as u64 * 10))
            {
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
        // Activity accepts the identity already projected by production transform.
        let projected_agent = trace_batch.request.resource_spans[0].scope_spans[0].spans[0]
            .attributes
            .iter()
            .find(|kv| kv.key == "gen_ai.agent.name")
            .and_then(|kv| match kv.value.as_ref()?.value.as_ref()? {
                any_value::Value::StringValue(s) => Some(s.as_str()),
                _ => None,
            })
            .unwrap();
        quota_engine.record_activity(projected_agent, Some(165));
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

        assert_no_sentinels_in_bytes_or_strings(
            "paths_and_names",
            &trace_bytes,
            &trace_strings,
            &[
                &repo_path_str,
                "SENTINEL_REPO_99",
                "SENTINEL_USER_77_custom_agent",
            ],
        );
    }
    let _ = fs::remove_dir_all(base_dir);
}

#[test]
fn g0a_captures_replay_in_default_mode_emits_zero_sentinels() {
    use std::collections::BTreeMap;
    let captures_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join("docs/contracts/fact-export-v1/captures/g0a-20261112");
    let matrix: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(captures_root.join("capture-matrix.json")).unwrap(),
    )
    .unwrap();
    let mut sources = BTreeMap::new();
    for row in matrix["rows"].as_array().unwrap() {
        for relative in row["sanitized_derivative_sha256"]
            .as_object()
            .unwrap()
            .keys()
        {
            if !relative.ends_with(".jsonl") {
                continue;
            }
            let directory = Path::new(relative)
                .parent()
                .unwrap()
                .to_string_lossy()
                .to_string();
            let client = match row["harness"].as_str().unwrap() {
                "claude" | "claude-code" => ClientKind::ClaudeCode.to_wire(),
                "codex" => ClientKind::Codex.to_wire(),
                other => panic!("unhandled capture harness {other}"),
            };
            let entry = sources
                .entry(directory)
                .or_insert((client, relative.clone()));
            if relative.ends_with("hook-sanitized.jsonl") {
                *entry = (client, relative.clone());
            }
        }
    }
    assert!(sources.keys().any(|s| s.starts_with("codex-0.167.0/")));
    let base = create_sentinel_workspace();
    let workspace = base.join("SENTINEL_USER_77/SENTINEL_HOME_PATH_11/SENTINEL_REPO_99");
    let path = workspace.to_string_lossy().to_string();
    let key = PseudonymKey::load_or_create(&base.join("privacy-home")).unwrap();
    let cache = ContextCache::new();
    wait_for_cache_ready(&cache, &path);
    let config = DaemonConfig::from_vars(&|_| None);
    let mut seq = 0u32;
    for (directory, (client, relative)) in sources {
        let content = fs::read_to_string(captures_root.join(relative)).unwrap();
        let mut spans = Vec::new();
        for line in content.lines() {
            let record: serde_json::Value = serde_json::from_str(line).unwrap();
            // Codex has no captured hooks: replay its session payload through the same
            // production ingress as a synthetic envelope, without claiming native-hook conformance.
            let mut input = record
                .get("stdin")
                .or_else(|| record.get("payload"))
                .unwrap_or(&record)
                .clone();
            let Some(obj) = input.as_object_mut() else {
                continue;
            };
            if obj.contains_key("conversation_id") {
                obj.remove("session_id");
            }
            obj.insert("workspacePath".into(), path.clone().into());
            let header = record.get("header");
            let client_id = header
                .and_then(|h| h["client_id"].as_u64())
                .unwrap_or(client as u64) as u16;
            let event_id = header
                .and_then(|h| h["event_id"].as_u64())
                .unwrap_or(HookEvent::PreToolUse.to_wire() as u64) as u8;
            let frame = encode_context_payload(
                WireHeader::new(client_id, event_id),
                None,
                &serde_json::to_vec(&input).unwrap(),
            );
            seq += 1;
            let outcome = production_transform_with_capture_mode(
                MsgType::HookPayloadWithContext,
                &frame,
                &cache,
                seq,
                ExecutionMode::Interactive,
                true,
                true,
                ResolvedSpanMetadata::default(),
                Instant::now(),
                1,
                config.workspace_path_mode,
                config.capture_mode,
                Some(&key),
            )
            .unwrap_or_else(|e| panic!("{directory} record {seq}: {e}"));
            assert_eq!(outcome.context_source, "workspace_cache", "{directory}");
            spans.push(outcome.span);
        }
        assert!(!spans.is_empty(), "no replayed records in {directory}");
        let request = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: Some(config.resource()),
                scope_spans: vec![ScopeSpans {
                    spans,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let bytes = request.encode_to_vec();
        let strings = collect_all_strings_from_trace_request(&request);
        assert_no_sentinels_in_bytes_or_strings(&directory, &bytes, &strings, G0A_SENTINELS);
        assert_no_sentinels_in_bytes_or_strings(&directory, &bytes, &strings, SENTINELS);
        assert_no_sentinels_in_bytes_or_strings(&directory, &bytes, &strings, &[&path]);
    }
    fs::remove_dir_all(base).unwrap();
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
        None,
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
        None,
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
