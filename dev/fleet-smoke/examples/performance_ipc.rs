//! Dev-only bounded in-process IPC latency benchmark.
//!
//! Validates normative SLA: IPC one-way send-through-receipt p99 < 3000 Âµs.
//! Covers MsgType::HookPayload (0x01) and MsgType::HookPayloadWithContext (0x04).

use agent_otel_core::model::{HookEvent, WireHeader};
use agent_otel_ipc::client::try_send;
use agent_otel_ipc::frame::{encode_context_payload, MsgType};
use agent_otel_ipc::server::run_server;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

const WARMUP_ROUNDS: usize = 20;
const TIMED_ROUNDS: usize = 1_000;
const RECV_TIMEOUT: Duration = Duration::from_millis(250);
const SLA_P99_US: f64 = 3000.0;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LatencyStats {
    pub count: usize,
    pub min_us: f64,
    pub max_us: f64,
    pub mean_us: f64,
    pub p50_us: f64,
    pub p95_us: f64,
    pub p99_us: f64,
}

impl LatencyStats {
    pub fn compute(mut samples: Vec<f64>) -> Result<Self, &'static str> {
        if samples.is_empty() {
            return Err("empty samples");
        }
        for s in &samples {
            if s.is_nan() || s.is_infinite() || *s < 0.0 {
                return Err("non-finite or negative sample");
            }
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let len = samples.len();
        let sum: f64 = samples.iter().sum();
        let p50_idx = (len * 50).div_ceil(100).saturating_sub(1).min(len - 1);
        let p95_idx = (len * 95).div_ceil(100).saturating_sub(1).min(len - 1);
        let p99_idx = (len * 99).div_ceil(100).saturating_sub(1).min(len - 1);
        Ok(Self {
            count: len,
            min_us: samples[0],
            max_us: samples[len - 1],
            mean_us: sum / len as f64,
            p50_us: samples[p50_idx],
            p95_us: samples[p95_idx],
            p99_us: samples[p99_idx],
        })
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IpcTypeReport {
    pub msg_type: String,
    pub expected_rounds: usize,
    pub warmup_rounds: usize,
    pub success_count: usize,
    pub failure_count: usize,
    pub latency_stats: Option<LatencyStats>,
    pub p99_sla_passed: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IpcBenchmarkReport {
    pub conversation_id: String,
    pub pipe_name: String,
    pub latency_label: String,
    pub normative_ref: String,
    pub impl_ref: String,
    pub results: Vec<IpcTypeReport>,
    pub overall_passed: bool,
    pub not_measured: Vec<String>,
}

fn run_ipc_loop(
    rt: &tokio::runtime::Runtime,
    rx: &mut tokio::sync::mpsc::Receiver<(MsgType, Vec<u8>)>,
    msg_type: MsgType,
    make_payload: impl Fn(usize) -> Vec<u8>,
) -> IpcTypeReport {
    let mut failures = 0;
    let deadline = Instant::now() + Duration::from_secs(20);
    for i in 0..WARMUP_ROUNDS {
        let p = make_payload(i);
        if try_send(msg_type, &p).is_err() {
            failures += 1;
        }
        let received = rt.block_on(async { tokio::time::timeout(RECV_TIMEOUT, rx.recv()).await });
        if !matches!(received, Ok(Some((kind, ref body))) if kind == msg_type && *body == p) {
            failures += 1;
        }
    }
    let mut samples = Vec::with_capacity(TIMED_ROUNDS);
    for i in 0..TIMED_ROUNDS {
        if Instant::now() >= deadline {
            failures += TIMED_ROUNDS - i;
            break;
        }
        let p = make_payload(WARMUP_ROUNDS + i);
        let start = Instant::now();
        let send_res = try_send(msg_type, &p);
        if send_res.is_err() {
            failures += 1;
            continue;
        }
        let recv_res = rt.block_on(async { tokio::time::timeout(RECV_TIMEOUT, rx.recv()).await });
        match recv_res {
            Ok(Some((rcv_type, rcv_p))) if rcv_type == msg_type && rcv_p == p => {
                samples.push(start.elapsed().as_secs_f64() * 1e6);
            }
            _ => failures += 1,
        }
    }
    let stats = LatencyStats::compute(samples).ok();
    let passed = failures == 0
        && stats
            .as_ref()
            .is_some_and(|s| s.count == TIMED_ROUNDS && s.p99_us < SLA_P99_US);
    IpcTypeReport {
        msg_type: format!("{:?}", msg_type),
        expected_rounds: TIMED_ROUNDS,
        warmup_rounds: WARMUP_ROUNDS,
        success_count: stats.as_ref().map_or(0, |s| s.count),
        failure_count: failures,
        latency_stats: stats,
        p99_sla_passed: passed,
    }
}

fn main() -> std::process::ExitCode {
    let pid = std::process::id();
    let tmp = TempDir::new().expect("tempdir");
    let token = format!(
        "{}-{}",
        pid,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let pipe_name = if cfg!(windows) {
        format!(r"\\.\pipe\agent-otel-bench-{}", token)
    } else {
        tmp.path()
            .join(format!("agent-otel-{}.sock", token))
            .to_str()
            .unwrap()
            .to_string()
    };
    std::env::set_var("AGENT_OTEL_PIPE", &pipe_name);
    std::env::set_var("AGY_OTEL_PIPE", &pipe_name);
    std::env::set_var("AGENT_OTEL_SOCKET", &pipe_name);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio rt");
    let (tx, mut rx) = tokio::sync::mpsc::channel(128);
    let cancel = CancellationToken::new();
    let srv_cancel = cancel.clone();
    let srv_pipe = pipe_name.clone();
    let mut srv_handle = rt.spawn(async move { run_server(Some(&srv_pipe), tx, srv_cancel).await });
    std::thread::sleep(Duration::from_millis(100));

    let header = WireHeader::new(3, HookEvent::PostToolUse.to_wire());
    let traceparent = "00-11223344556677889900aabbccddeeff-aabbccddeeff0011-01";
    let legacy_res = run_ipc_loop(&rt, &mut rx, MsgType::HookPayload, |i| {
        let json = format!(
            r#"{{"stepIdx":{},"toolCall":{{"name":"bench","id":"{}"}}}}"#,
            i, i
        );
        let mut body = header.encode().to_vec();
        body.extend_from_slice(json.as_bytes());
        body
    });
    let envelope_res = run_ipc_loop(&rt, &mut rx, MsgType::HookPayloadWithContext, |i| {
        let json = format!(r#"{{"stepIdx":{},"contextPayload":true}}"#, i).into_bytes();
        encode_context_payload(header, Some(traceparent), &json)
    });

    cancel.cancel();
    rt.block_on(async {
        if tokio::time::timeout(Duration::from_secs(1), &mut srv_handle)
            .await
            .is_err()
        {
            srv_handle.abort();
        }
    });

    let overall_pass = legacy_res.p99_sla_passed && envelope_res.p99_sla_passed;
    let report = IpcBenchmarkReport {
        conversation_id: "performance-fixture".to_string(),
        pipe_name,
        latency_label: "client_send_through_receipt_one_way_observer_micros".to_string(),
        normative_ref:
            "dev/fleet-smoke/performance-coordination.md#required-measurements-and-assertions"
                .to_string(),
        impl_ref: "agent_otel_ipc::client::try_send -> agent_otel_ipc::server::run_server"
            .to_string(),
        results: vec![legacy_res, envelope_res],
        overall_passed: overall_pass,
        not_measured: vec!["hook_internal_execution_duration_us".to_string()],
    };
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    if overall_pass {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_latency_stats_boundary_and_rejection() {
        assert!(LatencyStats::compute(vec![]).is_err());
        assert!(LatencyStats::compute(vec![10.0, f64::NAN]).is_err());
        assert!(LatencyStats::compute(vec![-1.0, 50.0]).is_err());
        let mut samples = Vec::new();
        for i in 1..=1000 {
            samples.push(i as f64);
        }
        let stats = LatencyStats::compute(samples).unwrap();
        assert_eq!(stats.count, 1000);
        assert_eq!(stats.min_us, 1.0);
        assert_eq!(stats.max_us, 1000.0);
        assert_eq!(stats.p50_us, 500.0);
        assert_eq!(stats.p95_us, 950.0);
        assert_eq!(stats.p99_us, 990.0);
    }
}
