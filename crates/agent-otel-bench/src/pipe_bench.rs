/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_ipc::client::try_send;
use agent_otel_ipc::frame::MsgType;
use agent_otel_ipc::server::run_server;
use std::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::stats::Stats;

pub async fn run(iterations: usize) -> Result<Stats, Box<dyn std::error::Error>> {
    let pipe_name = format!(r"\\.\pipe\agy-otel-bench-{}", std::process::id());
    std::env::set_var("AGY_OTEL_PIPE", &pipe_name);

    let (tx, mut rx) = tokio::sync::mpsc::channel(4096);
    let shutdown = CancellationToken::new();
    let shutdown_server = shutdown.clone();
    let pipe_name_server = pipe_name.clone();

    let server_handle =
        tokio::spawn(async move { run_server(Some(&pipe_name_server), tx, shutdown_server).await });

    // Warm-up and wait for pipe server
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let payload = b"{\"event\":\"PostToolUse\",\"tool\":\"run_command\",\"duration\":1500}";
    let mut samples = Vec::with_capacity(iterations);

    // Warm-up round
    for _ in 0..10 {
        let _ = try_send(MsgType::HookPayload, payload);
        let _ = rx.recv().await;
    }

    for _ in 0..iterations {
        let start = Instant::now();
        let send_res = try_send(MsgType::HookPayload, payload);
        if send_res.is_err() {
            continue;
        }
        let _ = rx.recv().await;
        let elapsed = start.elapsed();
        samples.push(elapsed.as_micros() as u64);
    }

    // Clean shutdown of test server
    shutdown.cancel();
    let _ = try_send(MsgType::Shutdown, b"");
    let _ = server_handle.await;

    Ok(Stats::new(samples))
}
