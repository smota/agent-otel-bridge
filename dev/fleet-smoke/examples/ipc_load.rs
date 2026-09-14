//! Dev-only deterministic bounded Rust IPC load emitter.
//!
//! Validates work item A2 under schema agent-otel-ipc-load/v1:
//! 1. Bounded workers with Barrier start, monotonic scheduling t0 + i/rate without silently skipping jobs.
//! 2. Precomputed deterministic non-zero trace_id (32-hex) and parent_span_id (16-hex) per event.
//! 3. Payloads framed via MsgType::HookPayloadWithContext (0x04) with valid traceparent header.
//! 4. Transport execution via agent_otel_ipc::client::try_send_detailed (safely drains Win32/Unix pending I/O).
//! 5. Output JSON reporting planned/offered/attempted/send_completed/send_failed counts and per-event latencies.
//!    Boundary: acknowledges send completion is not receiver receipt.

use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use agent_otel_core::model::HookEvent;
use agent_otel_ipc::client::{try_send_detailed, SendError, SendStage};
use agent_otel_ipc::frame::{encode_context_payload, MsgType, WireHeader};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ConfigReport {
    pub endpoint: String,
    pub events: usize,
    pub concurrency: usize,
    pub target_rate_eps: f64,
    pub seed: u64,
    pub period_us: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventTimingRecord {
    pub index: usize,
    pub trace_id: String,
    pub parent_span_id: String,
    pub send_completed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_code: Option<u32>,
    pub scheduled_offset_us: u64,
    pub start_offset_us: u64,
    pub end_offset_us: u64,
    pub lateness_us: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct IpcLoadReport {
    pub schema: &'static str,
    pub configuration: ConfigReport,
    pub planned_events: usize,
    pub offered_events: usize,
    pub attempted_events: usize,
    pub send_completed_events: usize,
    pub send_failed_events: usize,
    pub schedule_late_events: usize,
    pub elapsed_seconds: f64,
    pub achieved_attempt_rate: f64,
    pub generator_limited: bool,
    pub boundary: &'static str,
    pub warnings: Vec<&'static str>,
    pub per_event: Vec<EventTimingRecord>,
}

struct PrecomputedItem {
    trace_id_hex: String,
    parent_span_id_hex: String,
    frame_bytes: Vec<u8>,
}

fn parse_args(args: &[String]) -> Result<(String, usize, usize, f64, u64), String> {
    let mut endpoint = None;
    let mut events = 400usize;
    let mut concurrency = 4usize;
    let mut rate = 1000.0f64;
    let mut seed = 42u64;

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--endpoint" {
            i += 1;
            endpoint = Some(
                args.get(i)
                    .ok_or_else(|| "missing value for --endpoint".to_string())?
                    .clone(),
            );
        } else if let Some(val) = arg.strip_prefix("--endpoint=") {
            endpoint = Some(val.to_string());
        } else if arg == "--events" {
            i += 1;
            let v = args
                .get(i)
                .ok_or_else(|| "missing value for --events".to_string())?;
            events = v
                .parse::<usize>()
                .map_err(|e| format!("invalid --events: {e}"))?;
        } else if let Some(val) = arg.strip_prefix("--events=") {
            events = val
                .parse::<usize>()
                .map_err(|e| format!("invalid --events: {e}"))?;
        } else if arg == "--concurrency" {
            i += 1;
            let v = args
                .get(i)
                .ok_or_else(|| "missing value for --concurrency".to_string())?;
            concurrency = v
                .parse::<usize>()
                .map_err(|e| format!("invalid --concurrency: {e}"))?;
        } else if let Some(val) = arg.strip_prefix("--concurrency=") {
            concurrency = val
                .parse::<usize>()
                .map_err(|e| format!("invalid --concurrency: {e}"))?;
        } else if arg == "--rate" {
            i += 1;
            let v = args
                .get(i)
                .ok_or_else(|| "missing value for --rate".to_string())?;
            rate = v
                .parse::<f64>()
                .map_err(|e| format!("invalid --rate: {e}"))?;
        } else if let Some(val) = arg.strip_prefix("--rate=") {
            rate = val
                .parse::<f64>()
                .map_err(|e| format!("invalid --rate: {e}"))?;
        } else if arg == "--seed" {
            i += 1;
            let v = args
                .get(i)
                .ok_or_else(|| "missing value for --seed".to_string())?;
            seed = v
                .parse::<u64>()
                .map_err(|e| format!("invalid --seed: {e}"))?;
        } else if let Some(val) = arg.strip_prefix("--seed=") {
            seed = val
                .parse::<u64>()
                .map_err(|e| format!("invalid --seed: {e}"))?;
        } else if arg.starts_with('-') {
            return Err(format!("unknown option: {arg}"));
        }
        i += 1;
    }

    let endpoint = endpoint.ok_or_else(|| "--endpoint is mandatory".to_string())?;
    if !endpoint.contains("aob-round-") {
        return Err(format!(
            "endpoint must contain 'aob-round-' owned prefix, got {endpoint}"
        ));
    }
    if !(1..=100_000).contains(&events) {
        return Err(format!("--events must be in 1..=100000, got {events}"));
    }
    if !(1..=32).contains(&concurrency) {
        return Err(format!(
            "--concurrency must be in 1..=32, got {concurrency}"
        ));
    }
    if !(1.0..=50_000.0).contains(&rate) {
        return Err(format!("--rate must be in 1..=50000, got {rate}"));
    }

    let planned_duration_secs = events as f64 / rate;
    if planned_duration_secs > 60.0 {
        return Err(format!(
            "planned schedule duration (events/rate) exceeds 60s: {planned_duration_secs:.2}s"
        ));
    }

    Ok((endpoint, events, concurrency, rate, seed))
}

fn stage_str(stage: SendStage) -> &'static str {
    match stage {
        SendStage::Connect => "Connect",
        SendStage::WaitForPipe => "WaitForPipe",
        SendStage::Reopen => "Reopen",
        SendStage::CreateEvent => "CreateEvent",
        SendStage::FrameTooLarge => "FrameTooLarge",
        SendStage::Submit => "Submit",
        SendStage::AwaitCompletion => "AwaitCompletion",
        SendStage::ObserveCompletion => "ObserveCompletion",
        SendStage::Deadline => "Deadline",
        SendStage::ShortWrite => "ShortWrite",
    }
}

fn precompute_identities_and_payloads(count: usize, seed: u64) -> Vec<PrecomputedItem> {
    let mut items = Vec::with_capacity(count);
    let header = WireHeader::new(1, HookEvent::PostToolUse.to_wire()); // client_id 1 = Antigravity

    let mut state = seed;
    let mut next_prng = || -> u64 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        state
    };

    for i in 0..count {
        let r1 = next_prng().max(1);
        let r2 = next_prng().max(1);
        let r3 = next_prng().max(1);

        let trace_id_hex = format!("{:016x}{:016x}", r1, r2);
        let parent_span_id_hex = format!("{:016x}", r3);
        let traceparent = format!("00-{}-{}-01", trace_id_hex, parent_span_id_hex);

        let json_body = serde_json::to_vec(&serde_json::json!({
            "conversationId": format!("ipc-load-{}", i),
            "stepIdx": i as u64,
            "toolCall": {
                "id": format!("call_{}", i),
                "name": "run_command",
                "arguments": {"command": "status"}
            },
            "inputTokens": 10,
            "outputTokens": 5
        }))
        .expect("serialize json");

        let frame_bytes = encode_context_payload(header, Some(&traceparent), &json_body);
        items.push(PrecomputedItem {
            trace_id_hex,
            parent_span_id_hex,
            frame_bytes,
        });
    }
    items
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let (endpoint, total_events, concurrency, target_rate, seed) = match parse_args(&args) {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(2);
        }
    };

    // Set pipe/socket env once in parent process before worker threads spawn
    if cfg!(windows) {
        std::env::set_var("AGENT_OTEL_PIPE", &endpoint);
        std::env::set_var("AGY_OTEL_PIPE", &endpoint);
    } else {
        std::env::set_var("AGENT_OTEL_SOCKET", &endpoint);
    }

    let period_us = (1_000_000.0 / target_rate).round() as u64;
    let items = Arc::new(precompute_identities_and_payloads(total_events, seed));
    let next_index = Arc::new(AtomicUsize::new(0));
    let start_barrier = Arc::new(Barrier::new(concurrency + 1));
    let start_time: Arc<OnceLock<Instant>> = Arc::new(OnceLock::new());

    let mut handles = Vec::with_capacity(concurrency);

    for _ in 0..concurrency {
        let barrier = Arc::clone(&start_barrier);
        let next_idx = Arc::clone(&next_index);
        let items_ref = Arc::clone(&items);
        let start_time_ref = Arc::clone(&start_time);

        let h = thread::spawn(move || -> Vec<EventTimingRecord> {
            let mut thread_records = Vec::new();
            barrier.wait();
            let t0 = *start_time_ref.get().expect("parent sets shared start time");

            loop {
                let i = next_idx.fetch_add(1, Ordering::Relaxed);
                if i >= items_ref.len() {
                    break;
                }
                let scheduled_offset_us = ((i as f64 * 1_000_000.0) / target_rate) as u64;
                let scheduled_instant = t0 + Duration::from_micros(scheduled_offset_us);

                let now = Instant::now();
                if now < scheduled_instant {
                    spin_sleep_until(scheduled_instant);
                }

                let start_offset_us = t0.elapsed().as_micros() as u64;
                let lateness_us = start_offset_us.saturating_sub(scheduled_offset_us);

                let send_res: Result<(), SendError> =
                    try_send_detailed(MsgType::HookPayloadWithContext, &items_ref[i].frame_bytes);

                let end_offset_us = t0.elapsed().as_micros() as u64;
                let (send_completed, stage, os_code) = match send_res {
                    Ok(()) => (true, None, None),
                    Err(err) => (
                        false,
                        Some(stage_str(err.stage).to_string()),
                        Some(err.os_code),
                    ),
                };

                thread_records.push(EventTimingRecord {
                    index: i,
                    trace_id: items_ref[i].trace_id_hex.clone(),
                    parent_span_id: items_ref[i].parent_span_id_hex.clone(),
                    send_completed,
                    stage,
                    os_code,
                    scheduled_offset_us,
                    start_offset_us,
                    end_offset_us,
                    lateness_us,
                });
            }
            thread_records
        });
        handles.push(h);
    }

    let t_run_start = Instant::now();
    start_time.set(t_run_start).expect("start time set once");
    start_barrier.wait();

    let mut all_records = Vec::with_capacity(total_events);
    let mut worker_panics = 0usize;
    for h in handles {
        if let Ok(records) = h.join() {
            all_records.extend(records);
        } else {
            worker_panics += 1;
        }
    }
    let total_elapsed = t_run_start.elapsed().as_secs_f64();

    all_records.sort_by_key(|r| r.index);

    let offered_events = total_events;
    let attempted_events = all_records.len();
    let send_completed_events = all_records.iter().filter(|r| r.send_completed).count();
    let send_failed_events = attempted_events.saturating_sub(send_completed_events);
    let schedule_late_events = all_records
        .iter()
        .filter(|r| r.lateness_us > period_us)
        .count();

    let achieved_attempt_rate = if total_elapsed > 0.0 {
        attempted_events as f64 / total_elapsed
    } else {
        0.0
    };

    let generator_limited =
        schedule_late_events > 0 || (attempted_events < offered_events) || worker_panics > 0;
    let mut warnings = vec![
        "Pending write cleanup in try_send_detailed can safely drain canceled Win32 IO and exceed the initial 3ms attempt budget under severe receiver pressure",
    ];
    if worker_panics > 0 {
        warnings.push(
            "worker panic: one or more workers terminated before recording all offered events",
        );
    }

    let report = IpcLoadReport {
        schema: "agent-otel-ipc-load/v1",
        configuration: ConfigReport {
            endpoint,
            events: total_events,
            concurrency,
            target_rate_eps: target_rate,
            seed,
            period_us,
        },
        planned_events: total_events,
        offered_events,
        attempted_events,
        send_completed_events,
        send_failed_events,
        schedule_late_events,
        elapsed_seconds: total_elapsed,
        achieved_attempt_rate,
        generator_limited,
        boundary: "IPC send completion (try_send_detailed exit) confirms client socket/pipe write completion; does not confirm daemon ingestion, batching or OTLP export",
        warnings,
        per_event: all_records,
    };

    if let Ok(json_str) = serde_json::to_string_pretty(&report) {
        println!("{json_str}");
    }

    if send_failed_events > 0 || attempted_events < offered_events || worker_panics > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn spin_sleep_until(target: Instant) {
    let now = Instant::now();
    if let Some(remaining) = target.checked_duration_since(now) {
        if remaining > Duration::from_millis(2) {
            thread::sleep(remaining - Duration::from_millis(1));
        }
        while Instant::now() < target {
            std::hint::spin_loop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_precomputed_identities_uniqueness_and_length() {
        let count = 100;
        let items = precompute_identities_and_payloads(count, 42);
        assert_eq!(items.len(), count);

        let mut traces = std::collections::HashSet::new();
        let mut parents = std::collections::HashSet::new();
        for item in &items {
            assert_eq!(item.trace_id_hex.len(), 32);
            assert_eq!(item.parent_span_id_hex.len(), 16);
            assert_ne!(item.trace_id_hex, "00000000000000000000000000000000");
            assert_ne!(item.parent_span_id_hex, "0000000000000000");
            assert!(
                traces.insert(item.trace_id_hex.clone()),
                "trace_ids must be unique"
            );
            assert!(
                parents.insert(item.parent_span_id_hex.clone()),
                "parent_span_ids must be unique"
            );
            assert!(!item.frame_bytes.is_empty());
        }
    }

    #[test]
    fn parse_args_rejects_missing_or_unowned_endpoint() {
        let missing = vec!["ipc_load".to_string()];
        assert!(parse_args(&missing).unwrap_err().contains("mandatory"));
        let unowned = vec![
            "ipc_load".into(),
            "--endpoint".into(),
            "\\\\.\\pipe\\other".into(),
        ];
        assert!(parse_args(&unowned).unwrap_err().contains("owned prefix"));
    }

    #[test]
    fn parse_args_rejects_invalid_bounds_and_nan() {
        let base = vec![
            "ipc_load".into(),
            "--endpoint".into(),
            "aob-round-test".into(),
        ];
        for extra in [
            vec!["--events".into(), "0".into()],
            vec!["--events".into(), "100001".into()],
            vec!["--concurrency".into(), "33".into()],
            vec!["--rate".into(), "NaN".into()],
            vec!["--rate".into(), "0.5".into()],
        ] {
            let mut args = base.clone();
            args.extend(extra);
            assert!(parse_args(&args).is_err());
        }
    }

    #[test]
    fn parse_args_defaults_are_stable_and_identities_repeat() {
        let args = vec![
            "ipc_load".into(),
            "--endpoint".into(),
            "aob-round-test".into(),
        ];
        assert_eq!(
            parse_args(&args).unwrap(),
            ("aob-round-test".into(), 400, 4, 1000.0, 42)
        );
        let first = precompute_identities_and_payloads(4, 42);
        let second = precompute_identities_and_payloads(4, 42);
        for (a, b) in first.iter().zip(second.iter()) {
            assert_eq!(a.trace_id_hex, b.trace_id_hex);
            assert_eq!(a.parent_span_id_hex, b.parent_span_id_hex);
            assert_eq!(a.frame_bytes, b.frame_bytes);
        }
    }
}
