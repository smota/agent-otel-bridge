/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use clap::{Parser, Subcommand};
use tokio_util::sync::CancellationToken;

mod doctor;
mod emit_quota;
mod stop;

#[derive(Parser, Debug)]
#[command(
    name = "agent-otel-bridge",
    about = "High-Performance OpenTelemetry Bridge for AI CLI Agent Harnesses",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Dispatches a lifecycle hook event to the background daemon
    Hook {
        /// Hook event name (e.g. PostToolUse, PostInvocation, Stop)
        event: String,
    },
    /// Runs the agent-otel-bridge background daemon
    Daemon,
    /// Diagnoses OTLP collector, station configuration, and IPC connectivity
    Doctor,
    /// Emits quota metrics to OTLP collector or signals running daemon
    EmitQuota {
        /// If set, sends an IPC QuotaPing to the daemon instead of direct export
        #[arg(long)]
        ping: bool,
    },
    /// Gracefully stops the running background daemon
    Stop,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Fast-path bypass for hook invocation to minimize latency
    let mut args = std::env::args().skip(1);
    if let Some(first) = args.next() {
        if first == "hook" {
            let event = args.next().unwrap_or_else(|| "Unknown".to_string());
            run_fast_hook(&event);
            return Ok(());
        }
    }

    let cli = Cli::parse();

    match cli.command {
        Commands::Hook { event } => {
            run_fast_hook(&event);
            Ok(())
        }
        Commands::Daemon => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;

            rt.block_on(async {
                let config = agent_otel_daemon::DaemonConfig::from_env();
                let shutdown = CancellationToken::new();

                // Setup Ctrl-C handler
                let shutdown_ctrlc = shutdown.clone();
                tokio::spawn(async move {
                    if let Ok(()) = tokio::signal::ctrl_c().await {
                        println!("\n[agent-otel-daemon] Ctrl-C received, stopping gracefully...");
                        shutdown_ctrlc.cancel();
                    }
                });

                let daemon = agent_otel_daemon::Daemon::new(config, shutdown)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                daemon.run().await
            })?;
            Ok(())
        }
        Commands::Doctor => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            rt.block_on(doctor::run())
        }
        Commands::EmitQuota { ping } => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            rt.block_on(emit_quota::run(ping))
        }
        Commands::Stop => {
            stop::run()
        }
    }
}

fn run_fast_hook(event_str: &str) {
    use std::io::{self, Read, Write};

    let tag = match event_str {
        "PreInvocation" | "pre_invocation" => 1,
        "PostInvocation" | "post_invocation" => 2,
        "PreToolUse" | "pre_tool_use" => 3,
        "PostToolUse" | "post_tool_use" => 4,
        "Stop" | "stop" => 5,
        s => s.parse::<u8>().unwrap_or(255),
    };

    let mut buf = Vec::with_capacity(4096);
    let mut stdin = io::stdin().take(256 * 1024);
    let _ = stdin.read_to_end(&mut buf);

    let mut payload = Vec::with_capacity(1 + buf.len());
    payload.push(tag);
    payload.extend_from_slice(&buf);

    agent_otel_ipc::client::send_fire_and_forget(agent_otel_ipc::frame::MsgType::HookPayload, &payload);

    let mut stdout = io::stdout();
    let _ = stdout.write_all(b"{}");
    let _ = stdout.flush();
}
