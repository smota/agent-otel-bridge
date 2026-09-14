//! Dev-only production-path benchmark example.
//!
//! Validates work item A1 under schema agent-otel-production-path/v1:
//! 1. Pure parser only: AgentHookInput::parse_slice (strictly > 50000/s gate)
//! 2. Production transform: actual shared production_transform function using ContextCache
//!    excluding quota/clock/batch/export.
//! 3. Diagnostics: lookup_hit, lookup_miss, and lookup_stale latency statistics with disjoint disposition accounting.

use std::hint::black_box;
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use agent_otel_core::model::{AgentHookInput, ExecutionMode, HookEvent};
use agent_otel_core::otlp::ResolvedSpanMetadata;
use agent_otel_core::semconv::AGENT_HOOK_EVENT;
use agent_otel_daemon::context_cache::{
    ContextCache, ContextState, RefreshDisposition, CONTEXT_FRESH_TTL,
};
use agent_otel_daemon::transform::{production_transform, TransformError};
use agent_otel_ipc::frame::{encode_context_payload, MsgType, WireHeader};
use serde::Serialize;

const LOOKUP_BENCH_ROUNDS: usize = 10_000;

#[derive(Debug, Clone, Serialize)]
pub struct LatencyDistribution {
    pub count: usize,
    pub p50_us: f64,
    pub p95_us: f64,
    pub p99_us: f64,
    pub max_us: f64,
}

impl LatencyDistribution {
    pub fn from_nanos(mut nanos: Vec<u64>) -> Result<Self, &'static str> {
        if nanos.is_empty() {
            return Err("empty samples");
        }
        nanos.sort_unstable();
        let len = nanos.len();
        let p50_idx = (len * 50).div_ceil(100).saturating_sub(1).min(len - 1);
        let p95_idx = (len * 95).div_ceil(100).saturating_sub(1).min(len - 1);
        let p99_idx = (len * 99).div_ceil(100).saturating_sub(1).min(len - 1);
        let max_idx = len - 1;

        Ok(Self {
            count: len,
            p50_us: nanos[p50_idx] as f64 / 1000.0,
            p95_us: nanos[p95_idx] as f64 / 1000.0,
            p99_us: nanos[p99_idx] as f64 / 1000.0,
            max_us: nanos[max_idx] as f64 / 1000.0,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ThroughputResult {
    pub iterations: usize,
    pub elapsed_secs: f64,
    pub ops_per_sec: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boundary: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LookupDiagnosticSummary {
    pub distribution: LatencyDistribution,
    pub queued_count: usize,
    pub already_pending_count: usize,
    pub queue_full_count: usize,
    pub not_needed_count: usize,
    pub circuit_open_count: usize,
    pub invalid_workspace_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProductionPathReport {
    pub schema: &'static str,
    pub profile: &'static str,
    pub seed: u64,
    pub warmup_iterations: usize,
    pub corpus_items: usize,
    pub corpus_identity: &'static str,
    pub parser_only: ThroughputResult,
    pub production_transform: ThroughputResult,
    pub lookup_hit: LatencyDistribution,
    pub lookup_miss: LookupDiagnosticSummary,
    pub lookup_stale: LookupDiagnosticSummary,
    pub boundary_notes: &'static str,
    pub semantic_assert_passed: bool,
    pub gate_parser_gt_50k_passed: bool,
}

struct CorpusItem {
    msg_type: MsgType,
    frame_bytes: Vec<u8>,
    raw_json_bytes: Vec<u8>,
}

fn build_deterministic_corpus(temp_root: &Path, seed: u64) -> (Vec<CorpusItem>, Vec<u8>, Vec<u8>) {
    let ws_valid = temp_root.join("ws_valid");
    std::fs::create_dir_all(&ws_valid).expect("create valid ws dir");
    let ws_valid_str = ws_valid.to_str().expect("valid utf8 path");

    let header_01 = WireHeader::new(1, HookEvent::PostToolUse.to_wire());
    let header_04 = WireHeader::new(1, HookEvent::PostToolUse.to_wire());
    let traceparent = "00-11223344556677889900aabbccddeeff-aabbccddeeff0011-01";

    let mut base_items = Vec::new();

    // Item 0: 0x04 frame with valid workspace (cache fresh/stale lookup candidate)
    let json_valid = serde_json::to_vec(&serde_json::json!({
        "conversationId": "bench-c0",
        "stepIdx": 10,
        "workspacePath": ws_valid_str,
        "toolCall": {"id": "c0", "name": "run_cmd"},
        "inputTokens": 100,
        "outputTokens": 50
    }))
    .unwrap();
    let frame_valid = encode_context_payload(header_04, Some(traceparent), &json_valid);
    base_items.push(CorpusItem {
        msg_type: MsgType::HookPayloadWithContext,
        frame_bytes: frame_valid,
        raw_json_bytes: json_valid,
    });

    // Item 1: 0x04 frame with missing workspace
    let json_missing = serde_json::to_vec(&serde_json::json!({
        "conversationId": "bench-c1",
        "stepIdx": 11,
        "toolCall": {"id": "c1", "name": "query_db"},
        "inputTokens": 20
    }))
    .unwrap();
    let frame_missing = encode_context_payload(header_04, Some(traceparent), &json_missing);
    base_items.push(CorpusItem {
        msg_type: MsgType::HookPayloadWithContext,
        frame_bytes: frame_missing,
        raw_json_bytes: json_missing,
    });

    // Item 2: 0x01 frame with provided absolute path
    let json_provided = serde_json::to_vec(&serde_json::json!({
        "conversationId": "bench-c2",
        "stepIdx": 12,
        "workspacePath": ws_valid_str,
        "toolCall": {"id": "c2", "name": "build_all"}
    }))
    .unwrap();
    let mut body_01 = header_01.encode().to_vec();
    body_01.extend_from_slice(&json_provided);
    base_items.push(CorpusItem {
        msg_type: MsgType::HookPayload,
        frame_bytes: body_01,
        raw_json_bytes: json_provided,
    });

    // Item 3: Agent event with hookEventName and clientKind resolution
    let json_event = serde_json::to_vec(&serde_json::json!({
        "conversationId": "bench-c3",
        "stepIdx": 13,
        "hookEventName": "PreToolUse",
        "inputTokens": 500,
        "outputTokens": 250
    }))
    .unwrap();
    let frame_event = encode_context_payload(WireHeader::new(1, 255), None, &json_event);
    base_items.push(CorpusItem {
        msg_type: MsgType::HookPayloadWithContext,
        frame_bytes: frame_event,
        raw_json_bytes: json_event,
    });

    // Seed-directed deterministic permutation
    let mut corpus = Vec::with_capacity(base_items.len());
    let mut indices: Vec<usize> = (0..base_items.len()).collect();
    let mut s = seed;
    for i in (1..indices.len()).rev() {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
        let j = (s as usize) % (i + 1);
        indices.swap(i, j);
    }
    let mut base_map: Vec<Option<CorpusItem>> = base_items.into_iter().map(Some).collect();
    for idx in indices {
        if let Some(item) = base_map[idx].take() {
            corpus.push(item);
        }
    }

    let invalid_json = b"{\"conversationId\": \"unclosed".to_vec();
    let invalid_envelope = vec![0x00, 0x01];

    (corpus, invalid_json, invalid_envelope)
}

fn parse_bounded_iterations() -> Result<usize, String> {
    let args: Vec<String> = std::env::args().collect();
    let mut iterations = 50_000usize;
    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--iterations" {
            i += 1;
            let val_str = args
                .get(i)
                .ok_or_else(|| "missing value for --iterations".to_string())?;
            iterations = val_str
                .parse::<usize>()
                .map_err(|e| format!("invalid --iterations: {e}"))?;
        } else if let Some(stripped) = arg.strip_prefix("--iterations=") {
            iterations = stripped
                .parse::<usize>()
                .map_err(|e| format!("invalid --iterations: {e}"))?;
        } else {
            return Err(format!("unknown option: {arg}"));
        }
        i += 1;
    }
    if !(1..=1_000_000).contains(&iterations) {
        return Err(format!(
            "--iterations must be bounded in 1..=1000000, got {iterations}"
        ));
    }
    Ok(iterations)
}

fn wait_until_fresh(
    contexts: &ContextCache,
    input: &AgentHookInput,
    deadline: Instant,
) -> Result<(), &'static str> {
    while Instant::now() < deadline {
        let lookup = contexts.lookup(input, Instant::now());
        if lookup.state == ContextState::Fresh && lookup.context.is_some() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Err("context cache entry failed to transition to Fresh within deadline")
}

fn main() -> ExitCode {
    let iterations = match parse_bounded_iterations() {
        Ok(it) => it,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(2);
        }
    };

    let warmup = 2_000usize.min(iterations / 2).max(10);
    let seed = 0x5EED_2026_0914_u64;

    let temp_dir = match tempfile::TempDir::new() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("failed to create temp dir: {e}");
            return ExitCode::from(1);
        }
    };
    let (corpus, invalid_json, invalid_envelope) =
        build_deterministic_corpus(temp_dir.path(), seed);

    let contexts = ContextCache::new();
    let metadata = ResolvedSpanMetadata {
        user_email: Some("bench@example.invalid"),
        terminal_type: Some("bench-term"),
    };
    let mode = ExecutionMode::Interactive;

    // Ensure real background workers harvest and make sample_ws_path Fresh
    let sample_ws_path = temp_dir.path().join("ws_valid");
    let hit_input = AgentHookInput {
        workspace_path: Some(sample_ws_path.to_str().unwrap().to_string()),
        ..Default::default()
    };
    let _ = contexts.lookup(&hit_input, Instant::now());
    let fresh_deadline = Instant::now() + Duration::from_secs(5);
    if let Err(e) = wait_until_fresh(&contexts, &hit_input, fresh_deadline) {
        eprintln!("Context cache readiness failure: {e}");
        return ExitCode::from(1);
    }

    // 1. Semantic Assertions with Expected Span Attributes & Client Ingress Matching
    for item in &corpus {
        let outcome = match production_transform(
            item.msg_type,
            &item.frame_bytes,
            &contexts,
            100,
            mode,
            false,
            metadata,
            Instant::now(),
            1_700_000_000_000_000_000,
        ) {
            Ok(res) => res,
            Err(err) => {
                eprintln!("semantic assertion error on corpus item: {err}");
                return ExitCode::from(1);
            }
        };
        assert!(!outcome.span.name.is_empty(), "span name must not be empty");
        assert_eq!(outcome.meta.agent_name.as_deref(), Some("antigravity"));
        assert!(outcome
            .span
            .attributes
            .iter()
            .any(|kv| kv.key == AGENT_HOOK_EVENT));
        assert!(outcome
            .span
            .attributes
            .iter()
            .any(|kv| kv.key == "gen_ai.agent.name"));
    }

    let err_json = production_transform(
        MsgType::HookPayloadWithContext,
        &encode_context_payload(
            WireHeader::new(1, HookEvent::PostToolUse.to_wire()),
            None,
            &invalid_json,
        ),
        &contexts,
        200,
        mode,
        false,
        metadata,
        Instant::now(),
        1_700_000_000_000_000_000,
    );
    assert_eq!(err_json.err(), Some(TransformError::InvalidJson));

    let err_env = production_transform(
        MsgType::HookPayloadWithContext,
        &invalid_envelope,
        &contexts,
        300,
        mode,
        false,
        metadata,
        Instant::now(),
        1_700_000_000_000_000_000,
    );
    assert_eq!(err_env.err(), Some(TransformError::InvalidEnvelope));
    let semantic_assert_passed = true;

    // 2. Pure Parser Benchmark (AgentHookInput::parse_slice only)
    let corpus_json_slices: Vec<&[u8]> =
        corpus.iter().map(|c| c.raw_json_bytes.as_slice()).collect();
    let json_count = corpus_json_slices.len();
    for i in 0..warmup {
        let slice = corpus_json_slices[i % json_count];
        black_box(AgentHookInput::parse_slice(slice).unwrap());
    }
    let parser_start = Instant::now();
    for i in 0..iterations {
        let slice = corpus_json_slices[i % json_count];
        black_box(AgentHookInput::parse_slice(slice).unwrap());
    }
    let parser_elapsed = parser_start.elapsed().as_secs_f64();
    if parser_elapsed <= 0.0 {
        eprintln!("parser elapsed duration is non-positive: {parser_elapsed}");
        return ExitCode::from(1);
    }
    let parser_ops = iterations as f64 / parser_elapsed;
    let gate_parser_gt_50k_passed = parser_ops > 50_000.0;

    // 3. Shared Production Transform Benchmark
    let corpus_len = corpus.len();
    let mut salt = 1000u32;
    for i in 0..warmup {
        salt = salt.wrapping_add(1);
        let item = &corpus[i % corpus_len];
        let out = production_transform(
            item.msg_type,
            &item.frame_bytes,
            &contexts,
            salt,
            mode,
            false,
            metadata,
            Instant::now(),
            1_700_000_000_000_000_000,
        )
        .unwrap();
        black_box(out);
    }

    let transform_start = Instant::now();
    for i in 0..iterations {
        salt = salt.wrapping_add(1);
        let item = &corpus[i % corpus_len];
        let out = production_transform(
            item.msg_type,
            &item.frame_bytes,
            &contexts,
            salt,
            mode,
            false,
            metadata,
            Instant::now(),
            1_700_000_000_000_000_000,
        )
        .unwrap();
        black_box(out);
    }
    let transform_elapsed = transform_start.elapsed().as_secs_f64();
    if transform_elapsed <= 0.0 {
        eprintln!("transform elapsed duration is non-positive: {transform_elapsed}");
        return ExitCode::from(1);
    }
    let transform_ops = iterations as f64 / transform_elapsed;

    // Semantic and warmup transforms may consume the original fresh window.
    // Re-establish it immediately before timing, then freeze the lookup clock.
    let _ = contexts.lookup(&hit_input, Instant::now());
    if let Err(e) = wait_until_fresh(
        &contexts,
        &hit_input,
        Instant::now() + Duration::from_secs(5),
    ) {
        eprintln!("Context cache hit readiness failure: {e}");
        return ExitCode::from(1);
    }

    // 4. Hit Lookup Diagnostic Latency (fixed reference instant prevents TTL drift)
    let fixed_hit_instant = Instant::now();
    let mut hit_nanos = Vec::with_capacity(LOOKUP_BENCH_ROUNDS);
    for _ in 0..LOOKUP_BENCH_ROUNDS {
        let t0 = Instant::now();
        let res = contexts.lookup(&hit_input, fixed_hit_instant);
        let dt = t0.elapsed().as_nanos() as u64;
        assert_eq!(
            res.state,
            ContextState::Fresh,
            "every measured hit lookup must be fresh"
        );
        black_box(res);
        hit_nanos.push(dt);
    }
    let lookup_hit = match LatencyDistribution::from_nanos(hit_nanos) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("failed to compute lookup_hit stats: {e}");
            return ExitCode::from(1);
        }
    };

    // 5. Miss + Enqueue Diagnostic Latency (distinct precomputed absolute paths)
    let missing_base = temp_dir.path().join("nonexistent_workspaces");
    let mut precomputed_miss_inputs = Vec::with_capacity(LOOKUP_BENCH_ROUNDS);
    for i in 0..LOOKUP_BENCH_ROUNDS {
        let p = missing_base.join(format!("sub_{i}"));
        precomputed_miss_inputs.push(AgentHookInput {
            workspace_path: Some(p.to_str().unwrap().to_string()),
            ..Default::default()
        });
    }

    let mut miss_nanos = Vec::with_capacity(LOOKUP_BENCH_ROUNDS);
    let mut miss_queued = 0;
    let mut miss_already_pending = 0;
    let mut miss_queue_full = 0;
    let mut miss_not_needed = 0;
    let mut miss_circuit_open = 0;
    let mut miss_invalid_ws = 0;

    for input in &precomputed_miss_inputs {
        let t0 = Instant::now();
        let res = contexts.lookup(input, t0);
        let dt = t0.elapsed().as_nanos() as u64;
        match res.refresh {
            RefreshDisposition::Queued => miss_queued += 1,
            RefreshDisposition::AlreadyPending => miss_already_pending += 1,
            RefreshDisposition::QueueFull => miss_queue_full += 1,
            RefreshDisposition::NotNeeded => miss_not_needed += 1,
            RefreshDisposition::CircuitOpen => miss_circuit_open += 1,
            RefreshDisposition::InvalidWorkspace => miss_invalid_ws += 1,
        }
        black_box(res);
        miss_nanos.push(dt);
    }
    let lookup_miss_dist = match LatencyDistribution::from_nanos(miss_nanos) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("failed to compute lookup_miss stats: {e}");
            return ExitCode::from(1);
        }
    };
    let lookup_miss = LookupDiagnosticSummary {
        distribution: lookup_miss_dist,
        queued_count: miss_queued,
        already_pending_count: miss_already_pending,
        queue_full_count: miss_queue_full,
        not_needed_count: miss_not_needed,
        circuit_open_count: miss_circuit_open,
        invalid_workspace_count: miss_invalid_ws,
    };

    // The unique miss corpus exceeds the cache entry bound and may evict the
    // hit key. Restore it before constructing a controlled stale lookup.
    let _ = contexts.lookup(&hit_input, Instant::now());
    if let Err(e) = wait_until_fresh(
        &contexts,
        &hit_input,
        Instant::now() + Duration::from_secs(5),
    ) {
        eprintln!("Context cache stale readiness failure: {e}");
        return ExitCode::from(1);
    }

    // 6. Explicitly Controlled Stale Lookup Diagnostic Latency
    // Using fixed reference instant past FRESH_TTL (1s) but within STALE_TTL (5s)
    let stale_instant = Instant::now() + CONTEXT_FRESH_TTL + Duration::from_millis(500);
    let mut stale_nanos = Vec::with_capacity(LOOKUP_BENCH_ROUNDS);
    let mut stale_queued = 0;
    let mut stale_already_pending = 0;
    let mut stale_queue_full = 0;
    let mut stale_not_needed = 0;
    let mut stale_circuit_open = 0;
    let mut stale_invalid_ws = 0;

    for _ in 0..LOOKUP_BENCH_ROUNDS {
        let t0 = Instant::now();
        let res = contexts.lookup(&hit_input, stale_instant);
        let dt = t0.elapsed().as_nanos() as u64;
        assert_eq!(
            res.state,
            ContextState::Stale,
            "controlled stale lookup must report Stale"
        );
        match res.refresh {
            RefreshDisposition::Queued => stale_queued += 1,
            RefreshDisposition::AlreadyPending => stale_already_pending += 1,
            RefreshDisposition::QueueFull => stale_queue_full += 1,
            RefreshDisposition::NotNeeded => stale_not_needed += 1,
            RefreshDisposition::CircuitOpen => stale_circuit_open += 1,
            RefreshDisposition::InvalidWorkspace => stale_invalid_ws += 1,
        }
        black_box(res);
        stale_nanos.push(dt);
    }
    let lookup_stale_dist = match LatencyDistribution::from_nanos(stale_nanos) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("failed to compute lookup_stale stats: {e}");
            return ExitCode::from(1);
        }
    };
    let lookup_stale = LookupDiagnosticSummary {
        distribution: lookup_stale_dist,
        queued_count: stale_queued,
        already_pending_count: stale_already_pending,
        queue_full_count: stale_queue_full,
        not_needed_count: stale_not_needed,
        circuit_open_count: stale_circuit_open,
        invalid_workspace_count: stale_invalid_ws,
    };

    let report = ProductionPathReport {
        schema: "agent-otel-production-path/v1",
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        seed,
        warmup_iterations: warmup,
        corpus_items: corpus.len(),
        corpus_identity: "deterministic_mixed_v1_0x01_0x04_tokens_ws",
        parser_only: ThroughputResult {
            iterations,
            elapsed_secs: parser_elapsed,
            ops_per_sec: parser_ops,
            boundary: None,
        },
        production_transform: ThroughputResult {
            iterations,
            elapsed_secs: transform_elapsed,
            ops_per_sec: transform_ops,
            boundary: Some("decode_frame + parse_slice + context_cache_lookup + normalize + build_span_from_resolved; excludes quota, clock, batch, export"),
        },
        lookup_hit,
        lookup_miss,
        lookup_stale,
        boundary_notes: "Lookup hit benchmark uses fixed reference instant at captured age to prevent TTL drift during 10k timed rounds; stale benchmark explicitly samples at fresh_ttl+500ms; miss benchmark samples 10k unique absolute paths to record disjoint queued vs queue_full/already_pending distributions; transform excludes quota, batching and export.",
        semantic_assert_passed,
        gate_parser_gt_50k_passed,
    };

    let json_str = match serde_json::to_string_pretty(&report) {
        Ok(json) => json,
        Err(error) => {
            eprintln!("failed to serialize production-path report: {error}");
            return ExitCode::from(1);
        }
    };
    println!("{json_str}");

    if gate_parser_gt_50k_passed && semantic_assert_passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
