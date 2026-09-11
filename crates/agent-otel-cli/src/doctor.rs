/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use std::time::Duration;
use agent_otel_ipc::client::try_send;
use agent_otel_ipc::frame::MsgType;

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== agent-otel-bridge doctor ===");
    println!("Diagnosing station telemetry pipeline & OpenTelemetry invariants\n");

    // 1. Environment variables check
    println!("[1/4] Checking environment contract variables...");
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://127.0.0.1:4318".to_string());
    println!("  [ok] OTEL_EXPORTER_OTLP_ENDPOINT = {}", endpoint);

    match std::env::var("OTEL_SERVICE_NAME") {
        Ok(v) => println!("  [ok] OTEL_SERVICE_NAME = {} (per-process)", v),
        Err(_) => println!("  [info] OTEL_SERVICE_NAME is unset (using default 'antigravity-cli')"),
    }

    match std::env::var("OTEL_RESOURCE_ATTRIBUTES") {
        Ok(v) => println!("  [ok] OTEL_RESOURCE_ATTRIBUTES = {}", v),
        Err(_) => println!("  [info] OTEL_RESOURCE_ATTRIBUTES unset (defaulting to 'deployment.environment=homelab')"),
    }

    // 2. Named Pipe IPC check
    println!("\n[2/4] Checking named pipe IPC (\\\\.\\pipe\\agy-otel)...");
    let ping_res = try_send(MsgType::HealthPing, b"doctor-probe");
    match ping_res {
        Ok(()) => println!("  [ok] Daemon is RUNNING and responding on named pipe!"),
        Err(()) => println!("  [warn] Daemon is NOT currently running (pipe not found or busy)."),
    }

    // 3. OTLP Collector connectivity check
    println!("\n[3/4] Checking OTLP Collector HTTP endpoint...");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;

    let traces_url = format!("{}/v1/traces", endpoint.trim_end_matches('/'));
    match client.post(&traces_url).body(vec![]).send().await {
        Ok(resp) => {
            println!(
                "  [ok] OTLP Collector reachable at {} (HTTP status: {})",
                traces_url,
                resp.status()
            );
        }
        Err(e) => {
            println!(
                "  [fail] OTLP Collector UNREACHABLE at {}: {}",
                traces_url, e
            );
        }
    }

    // 4. SigNoz Dashboard reachability check
    println!("\n[4/4] Checking SigNoz UI reachability (http://localhost:8080)...");
    match client.get("http://localhost:8080").send().await {
        Ok(resp) => {
            println!(
                "  [ok] SigNoz UI reachable at http://localhost:8080 (HTTP status: {})",
                resp.status()
            );
            println!("  [dashboard] http://localhost:8080/dashboard/01a08f6b-fbeb-7439-a6a9-0809f9da72a0");
        }
        Err(e) => {
            println!("  [warn] SigNoz UI not reachable at http://localhost:8080: {}", e);
        }
    }

    println!("\nDoctor check completed.\n");
    Ok(())
}
