/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_ipc::client::try_send;
use agent_otel_ipc::frame::MsgType;
use std::time::Duration;

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== agent-otel-bridge doctor ===");
    println!("Diagnosing station telemetry pipeline & OpenTelemetry invariants\n");

    // 1. Environment variables check
    println!("[1/5] Checking environment contract variables...");
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://127.0.0.1:4318".to_string());
    println!("  [ok] OTEL_EXPORTER_OTLP_ENDPOINT = {}", endpoint);

    match std::env::var("OTEL_SERVICE_NAME") {
        Ok(v) => println!("  [ok] OTEL_SERVICE_NAME = {} (per-process)", v),
        Err(_) => {
            println!("  [info] OTEL_SERVICE_NAME is unset (using default 'agent-otel-bridge')")
        }
    }

    match std::env::var("OTEL_RESOURCE_ATTRIBUTES") {
        Ok(v) => println!("  [ok] OTEL_RESOURCE_ATTRIBUTES = {}", v),
        Err(_) => println!("  [info] OTEL_RESOURCE_ATTRIBUTES unset (defaulting to 'deployment.environment=homelab')"),
    }

    // 2. Named Pipe IPC check
    let pipe_name = std::env::var("AGENT_OTEL_PIPE")
        .or_else(|_| std::env::var("AGY_OTEL_PIPE"))
        .unwrap_or_else(|_| agent_otel_ipc::frame::DEFAULT_PIPE_NAME.to_string());
    println!("\n[2/5] Checking named pipe IPC ({})...", pipe_name);
    let ping_res = try_send(MsgType::HealthPing, b"doctor-probe");
    match ping_res {
        Ok(()) => println!("  [ok] Daemon is RUNNING and responding on named pipe!"),
        Err(()) => {
            println!("  [warn] Daemon is NOT currently running (pipe not found or busy).");
            println!(
                "         Run 'agent-otel-bridge start' or invoke an agent hook to auto-launch it."
            );
        }
    }

    // 3. OTLP Collector connectivity check
    println!("\n[3/5] Checking OTLP Collector HTTP endpoint...");
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

    // 4. Observability Dashboard / UI reachability check
    let ui_url = std::env::var("OTEL_UI_URL")
        .or_else(|_| std::env::var("SIGNOZ_UI_URL"))
        .unwrap_or_else(|_| "http://localhost:8080".to_string());

    println!("\n[4/5] Checking Observability UI reachability ({})...", ui_url);
    match client.get(&ui_url).send().await {
        Ok(resp) => {
            println!(
                "  [ok] Observability UI reachable at {} (HTTP status: {})",
                ui_url,
                resp.status()
            );
            if ui_url.contains("localhost:8080") {
                println!(
                    "  [dashboard] {}/dashboard/01a091c1-c5c7-7105-9963-436f893fa832 (AI Agent Observability)",
                    ui_url.trim_end_matches('/')
                );
            }
        }
        Err(e) => {
            println!(
                "  [info] Observability UI not reachable at {} (optional): {}",
                ui_url, e
            );
        }
    }

    // 5. Client Hook Registrations check
    println!("\n[5/5] Checking Client Hook Registrations...");
    let hook_in_path = crate::hooks::check_binary_in_path("agent-hook");
    println!(
        "  agent-hook in PATH: {}",
        if hook_in_path {
            "[ok] present"
        } else {
            "[warn] not found (run cargo install or check PATH)"
        }
    );

    if let Some(p) = crate::hooks::get_antigravity_config_path(false) {
        let configured = p.exists()
            && std::fs::read_to_string(&p)
                .map(|s| s.contains("agent-otel-bridge"))
                .unwrap_or(false);
        println!(
            "  Google Antigravity: {}",
            if configured {
                format!("[ok] registered ({})", p.display())
            } else {
                format!("[info] not registered ({})", p.display())
            }
        );
    }

    if let Some(p) = crate::hooks::get_claude_config_path(false) {
        let configured = p.exists()
            && std::fs::read_to_string(&p)
                .map(|s| s.contains("agent-hook"))
                .unwrap_or(false);
        println!(
            "  Claude Code:        {}",
            if configured {
                format!("[ok] registered ({})", p.display())
            } else {
                format!("[info] not registered ({})", p.display())
            }
        );
    }

    if let Some(p) = crate::hooks::get_codex_config_path(false) {
        let configured = p.exists()
            && std::fs::read_to_string(&p)
                .map(|s| s.contains("agent-hook") || s.contains("agent-otel-bridge"))
                .unwrap_or(false);
        println!(
            "  OpenAI Codex:       {}",
            if configured {
                format!("[ok] registered ({})", p.display())
            } else {
                format!("[info] not registered ({})", p.display())
            }
        );
    }

    if let Some(p) = crate::hooks::get_grok_config_path(false) {
        let configured = p.exists()
            && std::fs::read_to_string(&p)
                .map(|s| s.contains("agent-hook") || s.contains("agent-otel-bridge"))
                .unwrap_or(false);
        println!(
            "  xAI Grok:           {}",
            if configured {
                format!("[ok] registered ({})", p.display())
            } else {
                format!("[info] not registered ({})", p.display())
            }
        );
    }

    if let Some(p) = crate::hooks::get_pi_config_path(false) {
        let configured = p.exists()
            && std::fs::read_to_string(&p)
                .map(|s| s.contains("agent-otel-bridge"))
                .unwrap_or(false);
        println!(
            "  Inflection Pi:      {}",
            if configured {
                format!("[ok] registered ({})", p.display())
            } else {
                format!("[info] not registered ({})", p.display())
            }
        );
    }

    println!("\nDoctor check completed.\n");
    Ok(())
}
