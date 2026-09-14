//! Dev-only performance validation example.
//!
//! Measures:
//! 1. Pure parser + span build (historical 0x01 body shape)
//! 2. Legacy frame decode (0x01) + parser + span build
//! 3. Context envelope decode (0x04) + parser + native context resolver + span build
//! 4. Filesystem workspace context harvesting (git repo, git worktree, no-git)
//! 5. Diagnostic-only pure JSON, resolved span build, and bounded real harvests
//!
//! Historical span metrics retain the existing user-email fallback, potentially filesystem I/O.
//! Diagnostic resolved-span metrics receive email and terminal metadata explicitly.
//! Does NOT claim native daemon propagation, hook internal execution, or native IPC delivery.

use std::fs;
use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

use agent_otel_core::context::WorkspaceContext;
use agent_otel_core::model::{AntigravityHookInput, HookEvent, WireHeader};
use agent_otel_core::otlp::{
    build_span_from_hook, build_span_from_hook_with_context_opts, build_span_from_resolved,
    ResolvedSpanMetadata,
};
use agent_otel_core::trace_id::resolve_trace_context;
use agent_otel_ipc::frame::{
    decode_context_payload, decode_header, encode_context_payload, encode_frame, MsgType,
    HEADER_LEN,
};
use tempfile::TempDir;

const WARMUP_ROUNDS: usize = 2_000;
const MEASURE_ROUNDS: usize = 50_000;
const HARVEST_WARMUP: usize = 100;
const HARVEST_ROUNDS: usize = 2_000;
const DIAGNOSTIC_HARVEST_WARMUP: usize = 25;
const DIAGNOSTIC_HARVEST_ROUNDS: usize = 500;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SampleStats {
    pub count: usize,
    pub min_us: f64,
    pub max_us: f64,
    pub mean_us: f64,
    pub p50_us: f64,
    pub p90_us: f64,
    pub p99_us: f64,
}

impl SampleStats {
    pub fn from_micros(mut samples: Vec<f64>) -> Result<Self, &'static str> {
        if samples.is_empty() {
            return Err("samples collection must not be empty");
        }
        for s in &samples {
            if s.is_nan() || s.is_infinite() || *s < 0.0 {
                return Err("invalid non-finite or negative sample");
            }
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let len = samples.len();
        let sum: f64 = samples.iter().sum();
        let mean_us = sum / len as f64;

        let p50_idx = (len * 50).div_ceil(100).saturating_sub(1).min(len - 1);
        let p90_idx = (len * 90).div_ceil(100).saturating_sub(1).min(len - 1);
        let p99_idx = (len * 99).div_ceil(100).saturating_sub(1).min(len - 1);

        Ok(Self {
            count: len,
            min_us: samples[0],
            max_us: samples[len - 1],
            mean_us,
            p50_us: samples[p50_idx],
            p90_us: samples[p90_idx],
            p99_us: samples[p99_idx],
        })
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ThroughputMetric {
    pub iterations: usize,
    pub warmup_iterations: usize,
    pub elapsed_secs: f64,
    pub ops_per_sec: f64,
    pub latency_stats: SampleStats,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContextHarvestMetric {
    pub fixture: String,
    pub iterations: usize,
    pub warmup_iterations: usize,
    pub verified_vcs_system: Option<String>,
    pub verified_branch: Option<String>,
    pub is_worktree: bool,
    pub stats: SampleStats,
    pub warm_cache: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PerformanceReport {
    pub fixture_conversation_id: String,
    pub parser_pure_json_span: ThroughputMetric,
    pub legacy_frame_0x01_pipeline: ThroughputMetric,
    pub envelope_0x04_pipeline: ThroughputMetric,
    pub context_harvest: Vec<ContextHarvestMetric>,
    pub labeled_proxies: Vec<String>,
    pub not_measured: Vec<String>,
    pub diagnostic: DiagnosticMetrics,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DiagnosticMetrics {
    pub classification: String,
    pub pure_json_parse: ThroughputMetric,
    pub parse_and_build_span_from_resolved: ThroughputMetric,
    pub real_fixture_context_harvest: Vec<ContextHarvestMetric>,
    pub semantic_equivalence_verified: bool,
    pub acceptance_proxy: bool,
}

fn bench_json_only(payload: &[u8]) -> ThroughputMetric {
    for _ in 0..WARMUP_ROUNDS {
        black_box(AntigravityHookInput::parse_slice(payload).unwrap());
    }

    let mut micros = Vec::with_capacity(MEASURE_ROUNDS);
    let start_all = Instant::now();
    for _ in 0..MEASURE_ROUNDS {
        let op_start = Instant::now();
        black_box(AntigravityHookInput::parse_slice(payload).unwrap());
        micros.push(op_start.elapsed().as_secs_f64() * 1e6);
    }
    let total_elapsed = start_all.elapsed().as_secs_f64();

    ThroughputMetric {
        iterations: MEASURE_ROUNDS,
        warmup_iterations: WARMUP_ROUNDS,
        elapsed_secs: total_elapsed,
        ops_per_sec: MEASURE_ROUNDS as f64 / total_elapsed,
        latency_stats: SampleStats::from_micros(micros).unwrap(),
    }
}

fn bench_parse_and_build_resolved(
    payload: &[u8],
    context: agent_otel_core::trace_id::ResolvedTraceContext,
    metadata: ResolvedSpanMetadata<'_>,
) -> ThroughputMetric {
    for i in 0..WARMUP_ROUNDS {
        let input = AntigravityHookInput::parse_slice(payload).unwrap();
        let span = build_span_from_resolved(
            HookEvent::PostToolUse,
            &input,
            1_000_000,
            2_000_000,
            i as u32,
            context,
            metadata,
            false,
        );
        black_box(span);
    }

    let mut micros = Vec::with_capacity(MEASURE_ROUNDS);
    let start_all = Instant::now();
    for i in 0..MEASURE_ROUNDS {
        let op_start = Instant::now();
        let input = AntigravityHookInput::parse_slice(payload).unwrap();
        let span = build_span_from_resolved(
            HookEvent::PostToolUse,
            &input,
            1_000_000,
            2_000_000,
            i as u32,
            context,
            metadata,
            false,
        );
        black_box(span);
        micros.push(op_start.elapsed().as_secs_f64() * 1e6);
    }
    let total_elapsed = start_all.elapsed().as_secs_f64();

    ThroughputMetric {
        iterations: MEASURE_ROUNDS,
        warmup_iterations: WARMUP_ROUNDS,
        elapsed_secs: total_elapsed,
        ops_per_sec: MEASURE_ROUNDS as f64 / total_elapsed,
        latency_stats: SampleStats::from_micros(micros).unwrap(),
    }
}

fn verify_resolved_builder_equivalence(
    payload: &[u8],
    context: agent_otel_core::trace_id::ResolvedTraceContext,
    metadata: ResolvedSpanMetadata<'_>,
) -> bool {
    let input = AntigravityHookInput::parse_slice(payload).unwrap();
    assert_eq!(input.user_email.as_deref(), metadata.user_email);
    assert_eq!(input.terminal_type.as_deref(), metadata.terminal_type);
    let compatibility = build_span_from_hook_with_context_opts(
        HookEvent::PostToolUse,
        &input,
        1_000_000,
        2_000_000,
        7,
        context,
        false,
    );
    let resolved = build_span_from_resolved(
        HookEvent::PostToolUse,
        &input,
        1_000_000,
        2_000_000,
        7,
        context,
        metadata,
        false,
    );
    assert_eq!(compatibility, resolved);
    true
}

fn bench_pure_parser(payload: &[u8]) -> ThroughputMetric {
    for i in 0..WARMUP_ROUNDS {
        let input = AntigravityHookInput::parse_slice(payload).unwrap();
        let span = build_span_from_hook(
            HookEvent::PostToolUse,
            &input,
            1_000_000,
            2_000_000,
            i as u32,
        );
        black_box(&span);
    }

    let mut micros = Vec::with_capacity(MEASURE_ROUNDS);
    let start_all = Instant::now();
    for i in 0..MEASURE_ROUNDS {
        let op_start = Instant::now();
        let input = AntigravityHookInput::parse_slice(payload).unwrap();
        let span = build_span_from_hook(
            HookEvent::PostToolUse,
            &input,
            1_000_000,
            2_000_000,
            i as u32,
        );
        black_box(&span);
        micros.push(op_start.elapsed().as_secs_f64() * 1e6);
    }
    let total_elapsed = start_all.elapsed().as_secs_f64();
    let ops_per_sec = MEASURE_ROUNDS as f64 / total_elapsed;

    ThroughputMetric {
        iterations: MEASURE_ROUNDS,
        warmup_iterations: WARMUP_ROUNDS,
        elapsed_secs: total_elapsed,
        ops_per_sec,
        latency_stats: SampleStats::from_micros(micros).unwrap(),
    }
}

fn bench_legacy_frame_0x01(frame: &[u8]) -> ThroughputMetric {
    for i in 0..WARMUP_ROUNDS {
        let mut header_buf = [0u8; HEADER_LEN];
        header_buf.copy_from_slice(&frame[..HEADER_LEN]);
        let (msg_type, payload_len) = decode_header(&header_buf).unwrap();
        assert_eq!(msg_type, MsgType::HookPayload);
        let payload = &frame[HEADER_LEN..HEADER_LEN + payload_len as usize];
        let wire = WireHeader::decode(payload).unwrap();
        let input = AntigravityHookInput::parse_slice(&payload[WireHeader::LEN..]).unwrap();
        let span = build_span_from_hook(
            HookEvent::from_wire(wire.event_id),
            &input,
            1_000_000,
            2_000_000,
            i as u32,
        );
        black_box(&span);
    }

    let mut micros = Vec::with_capacity(MEASURE_ROUNDS);
    let start_all = Instant::now();
    for i in 0..MEASURE_ROUNDS {
        let op_start = Instant::now();
        let mut header_buf = [0u8; HEADER_LEN];
        header_buf.copy_from_slice(&frame[..HEADER_LEN]);
        let (msg_type, payload_len) = decode_header(&header_buf).unwrap();
        assert_eq!(msg_type, MsgType::HookPayload);
        let payload = &frame[HEADER_LEN..HEADER_LEN + payload_len as usize];
        let wire = WireHeader::decode(payload).unwrap();
        let input = AntigravityHookInput::parse_slice(&payload[WireHeader::LEN..]).unwrap();
        let span = build_span_from_hook(
            HookEvent::from_wire(wire.event_id),
            &input,
            1_000_000,
            2_000_000,
            i as u32,
        );
        black_box(&span);
        micros.push(op_start.elapsed().as_secs_f64() * 1e6);
    }
    let total_elapsed = start_all.elapsed().as_secs_f64();
    let ops_per_sec = MEASURE_ROUNDS as f64 / total_elapsed;

    ThroughputMetric {
        iterations: MEASURE_ROUNDS,
        warmup_iterations: WARMUP_ROUNDS,
        elapsed_secs: total_elapsed,
        ops_per_sec,
        latency_stats: SampleStats::from_micros(micros).unwrap(),
    }
}

fn bench_envelope_0x04_pipeline(frame: &[u8]) -> ThroughputMetric {
    for i in 0..WARMUP_ROUNDS {
        let mut header_buf = [0u8; HEADER_LEN];
        header_buf.copy_from_slice(&frame[..HEADER_LEN]);
        let (msg_type, payload_len) = decode_header(&header_buf).unwrap();
        assert_eq!(msg_type, MsgType::HookPayloadWithContext);
        let payload = &frame[HEADER_LEN..HEADER_LEN + payload_len as usize];
        let (hdr, ctx, raw_json) = decode_context_payload(payload).unwrap();
        let event = HookEvent::from_wire(hdr.event_id);
        let input = AntigravityHookInput::parse_slice(raw_json).unwrap();
        let resolved = resolve_trace_context(
            input.traceparent.as_deref(),
            ctx,
            input.conversation_id.as_deref(),
        );
        let span = build_span_from_hook_with_context_opts(
            event, &input, 1_000_000, 2_000_000, i as u32, resolved, false,
        );
        black_box((ctx, span));
    }

    let mut micros = Vec::with_capacity(MEASURE_ROUNDS);
    let start_all = Instant::now();
    for i in 0..MEASURE_ROUNDS {
        let op_start = Instant::now();
        let mut header_buf = [0u8; HEADER_LEN];
        header_buf.copy_from_slice(&frame[..HEADER_LEN]);
        let (msg_type, payload_len) = decode_header(&header_buf).unwrap();
        assert_eq!(msg_type, MsgType::HookPayloadWithContext);
        let payload = &frame[HEADER_LEN..HEADER_LEN + payload_len as usize];
        let (hdr, ctx, raw_json) = decode_context_payload(payload).unwrap();
        let event = HookEvent::from_wire(hdr.event_id);
        let input = AntigravityHookInput::parse_slice(raw_json).unwrap();
        let resolved = resolve_trace_context(
            input.traceparent.as_deref(),
            ctx,
            input.conversation_id.as_deref(),
        );
        let span = build_span_from_hook_with_context_opts(
            event, &input, 1_000_000, 2_000_000, i as u32, resolved, false,
        );
        black_box((ctx, span));
        micros.push(op_start.elapsed().as_secs_f64() * 1e6);
    }
    let total_elapsed = start_all.elapsed().as_secs_f64();
    let ops_per_sec = MEASURE_ROUNDS as f64 / total_elapsed;

    ThroughputMetric {
        iterations: MEASURE_ROUNDS,
        warmup_iterations: WARMUP_ROUNDS,
        elapsed_secs: total_elapsed,
        ops_per_sec,
        latency_stats: SampleStats::from_micros(micros).unwrap(),
    }
}

fn bench_harvest_dir(
    label: &str,
    dir: &Path,
    expected_vcs: Option<&str>,
    expected_branch: Option<&str>,
    expected_worktree: bool,
) -> ContextHarvestMetric {
    bench_harvest_dir_with_counts(
        label,
        dir,
        expected_vcs,
        expected_branch,
        expected_worktree,
        HARVEST_WARMUP,
        HARVEST_ROUNDS,
    )
}

#[allow(clippy::too_many_arguments)]
fn bench_harvest_dir_with_counts(
    label: &str,
    dir: &Path,
    expected_vcs: Option<&str>,
    expected_branch: Option<&str>,
    expected_worktree: bool,
    warmup_rounds: usize,
    measure_rounds: usize,
) -> ContextHarvestMetric {
    let initial = WorkspaceContext::harvest_from_dir(dir);
    let actual_vcs = initial.vcs_system.as_deref();
    let actual_branch = initial.vcs_branch.as_deref();
    assert_eq!(
        actual_vcs, expected_vcs,
        "fixture {} vcs mismatch: got {:?}, expected {:?}",
        label, actual_vcs, expected_vcs
    );
    if let Some(exp_b) = expected_branch {
        assert_eq!(
            actual_branch,
            Some(exp_b),
            "fixture {} branch mismatch",
            label
        );
    }
    if expected_worktree {
        assert!(
            initial.vcs_worktree == Some(true),
            "fixture {} expected worktree indicator",
            label
        );
    }

    for _ in 0..warmup_rounds {
        let ctx = WorkspaceContext::harvest_from_dir(dir);
        black_box(ctx);
    }

    let mut micros = Vec::with_capacity(measure_rounds);
    for _ in 0..measure_rounds {
        let op_start = Instant::now();
        let ctx = WorkspaceContext::harvest_from_dir(dir);
        black_box(ctx);
        micros.push(op_start.elapsed().as_secs_f64() * 1e6);
    }

    ContextHarvestMetric {
        fixture: label.to_string(),
        iterations: measure_rounds,
        warmup_iterations: warmup_rounds,
        verified_vcs_system: initial.vcs_system,
        verified_branch: initial.vcs_branch,
        is_worktree: initial.vcs_worktree == Some(true),
        stats: SampleStats::from_micros(micros).unwrap(),
        warm_cache: true,
    }
}

fn setup_fixtures(temp_root: &Path) -> std::io::Result<()> {
    let no_git = temp_root.join("no_git");
    fs::create_dir_all(&no_git)?;
    fs::write(
        no_git.join("Cargo.toml"),
        "[package]\nname = \"fixture-no-git\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )?;
    fs::write(no_git.join("README.md"), "plain project marker")?;

    let git_repo = temp_root.join("git_repo");
    let git_dir = git_repo.join(".git");
    fs::create_dir_all(&git_dir)?;
    fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n")?;
    fs::write(
        git_dir.join("config"),
        "[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n\tbare = false\n",
    )?;
    fs::create_dir_all(git_dir.join("worktrees").join("feature-wt"))?;
    fs::write(
        git_dir.join("worktrees").join("feature-wt").join("gitdir"),
        format!(
            "{}\n",
            temp_root.join("git_worktree").join(".git").display()
        ),
    )?;
    fs::write(
        git_dir.join("worktrees").join("feature-wt").join("HEAD"),
        "ref: refs/heads/feature\n",
    )?;
    fs::write(
        git_dir
            .join("worktrees")
            .join("feature-wt")
            .join("commondir"),
        "../../\n",
    )?;

    let git_worktree = temp_root.join("git_worktree");
    fs::create_dir_all(&git_worktree)?;
    let wt_gitdir = git_dir.join("worktrees").join("feature-wt");
    fs::write(
        git_worktree.join(".git"),
        format!("gitdir: {}\n", wt_gitdir.display()),
    )?;
    fs::write(git_worktree.join("file.txt"), "worktree content")?;

    Ok(())
}

fn main() {
    let payload = br#"{
        "conversationId": "performance-fixture",
        "stepIdx": 142,
        "toolCall": {
            "id": "call_12345",
            "name": "run_command",
            "arguments": {
                "CommandLine": "cargo build --release"
            }
        },
        "modelName": "gemini-2.5-pro",
        "executionNum": 4,
        "fullyIdle": false
    }"#;

    let pure_parser_res = bench_pure_parser(payload);

    let mut legacy_body = WireHeader::new(3, HookEvent::PostToolUse.to_wire())
        .encode()
        .to_vec();
    legacy_body.extend_from_slice(payload);
    let legacy_frame = encode_frame(MsgType::HookPayload, &legacy_body);
    let legacy_res = bench_legacy_frame_0x01(&legacy_frame);

    let header = WireHeader::new(3, HookEvent::PostToolUse.to_wire());
    let valid_traceparent = "00-11223344556677889900aabbccddeeff-aabbccddeeff0011-01";
    let context_payload = encode_context_payload(header, Some(valid_traceparent), payload);
    let envelope_frame = encode_frame(MsgType::HookPayloadWithContext, &context_payload);
    let envelope_res = bench_envelope_0x04_pipeline(&envelope_frame);

    // The diagnostic fixture supplies all ambient metadata explicitly.  It is
    // intentionally separate from the historical metrics above, whose payload
    // and timing counts remain unchanged.
    let diagnostic_payload = br#"{
        "conversationId": "performance-fixture-resolved",
        "stepIdx": 142,
        "toolCall": {
            "id": "call_12345",
            "name": "run_command",
            "arguments": {
                "CommandLine": "cargo build --release"
            }
        },
        "modelName": "gemini-2.5-pro",
        "executionNum": 4,
        "fullyIdle": false,
        "userEmail": "fixture@example.invalid",
        "terminalType": "fixture-terminal"
    }"#;
    let diagnostic_input = AntigravityHookInput::parse_slice(diagnostic_payload).unwrap();
    let diagnostic_context = resolve_trace_context(
        diagnostic_input.traceparent.as_deref(),
        Some(valid_traceparent),
        diagnostic_input.conversation_id.as_deref(),
    );
    let diagnostic_metadata = ResolvedSpanMetadata {
        user_email: Some("fixture@example.invalid"),
        terminal_type: Some("fixture-terminal"),
    };
    let diagnostic_json = bench_json_only(diagnostic_payload);
    let diagnostic_resolved =
        bench_parse_and_build_resolved(diagnostic_payload, diagnostic_context, diagnostic_metadata);
    let semantic_equivalence_verified = verify_resolved_builder_equivalence(
        diagnostic_payload,
        diagnostic_context,
        diagnostic_metadata,
    );

    let temp_dir = TempDir::new().expect("create isolated temp directory for fixtures");
    let temp_root = temp_dir.path();
    setup_fixtures(temp_root).expect("setup fixtures");

    let harvest_results = vec![
        bench_harvest_dir(
            "git_repo",
            &temp_root.join("git_repo"),
            Some("git"),
            Some("main"),
            false,
        ),
        bench_harvest_dir(
            "git_worktree",
            &temp_root.join("git_worktree"),
            Some("git"),
            Some("feature"),
            true,
        ),
        bench_harvest_dir(
            "no_git",
            &temp_root.join("no_git"),
            Some("none"),
            None,
            false,
        ),
    ];

    let diagnostic_harvest_results = vec![
        bench_harvest_dir_with_counts(
            "git_repo",
            &temp_root.join("git_repo"),
            Some("git"),
            Some("main"),
            false,
            DIAGNOSTIC_HARVEST_WARMUP,
            DIAGNOSTIC_HARVEST_ROUNDS,
        ),
        bench_harvest_dir_with_counts(
            "git_worktree",
            &temp_root.join("git_worktree"),
            Some("git"),
            Some("feature"),
            true,
            DIAGNOSTIC_HARVEST_WARMUP,
            DIAGNOSTIC_HARVEST_ROUNDS,
        ),
        bench_harvest_dir_with_counts(
            "no_git",
            &temp_root.join("no_git"),
            Some("none"),
            None,
            false,
            DIAGNOSTIC_HARVEST_WARMUP,
            DIAGNOSTIC_HARVEST_ROUNDS,
        ),
    ];

    let report = PerformanceReport {
        fixture_conversation_id: "performance-fixture".to_string(),
        parser_pure_json_span: pure_parser_res,
        legacy_frame_0x01_pipeline: legacy_res,
        envelope_0x04_pipeline: envelope_res,
        context_harvest: harvest_results,
        labeled_proxies: vec![
            "isolated decode/resolve/build excludes daemon queue, enrichment and export"
                .to_string(),
        ],
        not_measured: vec![
            "hook_internal_execution_duration_us".to_string(),
            "ipc_roundtrip_p99_us".to_string(),
            "concurrent_event_delivery_loss".to_string(),
        ],
        diagnostic: DiagnosticMetrics {
            classification: "diagnostic_only_not_an_acceptance_proxy".to_string(),
            pure_json_parse: diagnostic_json,
            parse_and_build_span_from_resolved: diagnostic_resolved,
            real_fixture_context_harvest: diagnostic_harvest_results,
            semantic_equivalence_verified,
            acceptance_proxy: false,
        },
    };

    let json_output = serde_json::to_string_pretty(&report).unwrap();
    println!("{json_output}");
}
