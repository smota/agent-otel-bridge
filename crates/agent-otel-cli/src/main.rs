/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use clap::{Parser, Subcommand};
use tokio_util::sync::CancellationToken;

mod doctor;
mod emit_quota;
mod guardrails;
mod hooks;
mod local;
mod scanner;
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
    /// Starts the agent-otel-bridge background daemon as a detached process
    Start,
    /// Gracefully stops the running background daemon
    Stop,
    /// Manage the isolated local runtime installation and development lifecycle
    Local {
        #[command(subcommand)]
        action: LocalAction,
    },
    /// Manage agent lifecycle hooks for supported clients (Google Antigravity, Claude Code, OpenAI Codex, xAI Grok, Pi [pi.dev])
    Hooks {
        #[command(subcommand)]
        action: HookAction,
    },
    /// Automated shortcut to install lifecycle hooks into supported agent clients
    InstallHooks {
        /// Target client: antigravity, claude, codex, grok, pi, or all (default: all)
        #[arg(long, default_value = "all")]
        client: String,
        /// If set, installs hooks at project level rather than user global level
        #[arg(long)]
        project: bool,
        /// Custom binary name or path for hook command (default: agent-hook)
        #[arg(long)]
        binary: Option<String>,
    },
    /// Runs performance benchmarks and evaluates hot-path latency SLAs
    Benchmark {
        /// Output full benchmark report as JSON
        #[arg(long)]
        json: bool,

        /// Output benchmark report as Markdown
        #[arg(long)]
        markdown: bool,

        /// Export benchmark report to file (format inferred from .json or .md)
        #[arg(long)]
        export: Option<std::path::PathBuf>,

        /// Interactively submit benchmark results to community repository
        #[arg(long)]
        submit: bool,

        /// Automatically open web browser when submitting benchmark
        #[arg(long)]
        open_browser: bool,
    },
    /// Verifies all architectural guardrails, SLAs, code formatting, clippy, and test suite
    CheckGuardrails {
        /// If set, automatically fixes formatting violations
        #[arg(long)]
        fix: bool,
    },
}

#[derive(Subcommand, Debug)]
enum LocalAction {
    /// Builds and installs candidate build into isolated local runtime
    Install {
        /// If set, activates this build as the current running version in bin/
        #[arg(long, default_value_t = true)]
        activate: bool,

        /// If set, updates agent lifecycle hooks to point to canonical bin/
        #[arg(long, default_value_t = true)]
        update_hooks: bool,

        /// Optional path to pre-built target directory containing release binaries
        #[arg(long)]
        from_build: Option<std::path::PathBuf>,
    },
    /// Inspects the active local installation, binaries integrity, and hook registrations
    Status,
    /// Rolls back immediately to the previous installed version
    Rollback {
        /// If set, updates agent lifecycle hooks after rollback
        #[arg(long, default_value_t = true)]
        update_hooks: bool,
    },
    /// Uninstalls local runtime and optionally removes hooks
    Uninstall {
        /// If set, removes entire versions history and data directory
        #[arg(long)]
        purge: bool,

        /// If set, removes hooks from supported agents
        #[arg(long, default_value_t = true)]
        remove_hooks: bool,
    },
}

#[derive(Subcommand, Debug)]
enum HookAction {
    /// Installs hooks for specified client (antigravity, claude, codex, grok, pi, or all)
    Install {
        #[arg(long, default_value = "all")]
        client: String,
        #[arg(long)]
        project: bool,
        #[arg(long)]
        binary: Option<String>,
    },
    /// Uninstalls hooks for specified client (antigravity, claude, codex, grok, pi, or all)
    Uninstall {
        #[arg(long, default_value = "all")]
        client: String,
        #[arg(long)]
        project: bool,
        #[arg(long)]
        binary: Option<String>,
    },
    /// Checks hook installation status across supported clients
    Status,
    /// Synchronizes project-level hooks in the specified or current workspace
    Sync {
        /// Optional path to project workspace directory (default: current directory)
        #[arg(long)]
        path: Option<std::path::PathBuf>,
        /// If set, scans workstation directories and synchronizes all projects found
        #[arg(long)]
        scan: bool,
        /// Custom binary name or path for hook command (default: agent-hook)
        #[arg(long)]
        binary: Option<String>,
    },
    /// Scans workstation directories for projects and synchronizes hooks across all of them
    #[command(alias = "scan")]
    ScanAll {
        /// Optional root directories to scan (defaults to standard developer folders in user profile)
        #[arg(long, num_args = 0..)]
        roots: Vec<std::path::PathBuf>,
        /// If set, scans root of available local drives (with strict system folder exclusions)
        #[arg(long)]
        all_drives: bool,
        /// Maximum directory traversal depth (default: 5)
        #[arg(long, default_value_t = 5)]
        max_depth: usize,
        /// Preview actions without writing changes to files
        #[arg(long)]
        dry_run: bool,
        /// Custom binary name or path for hook command (default: agent-hook)
        #[arg(long)]
        binary: Option<String>,
    },
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
                    .map_err(std::io::Error::other)?;
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
        Commands::Start => run_start(),
        Commands::Stop => stop::run(),
        Commands::Local { action } => match action {
            LocalAction::Install {
                activate,
                update_hooks,
                from_build,
            } => {
                local::install_candidate(activate, update_hooks, from_build.as_deref())?;
                Ok(())
            }
            LocalAction::Status => local::run_status(),
            LocalAction::Rollback { update_hooks } => local::rollback_candidate(update_hooks),
            LocalAction::Uninstall {
                purge,
                remove_hooks,
            } => local::uninstall_local(purge, remove_hooks),
        },
        Commands::Hooks { action } => match action {
            HookAction::Install {
                client,
                project,
                binary,
            } => hooks::run_install(&client, project, binary.as_deref()),
            HookAction::Uninstall {
                client,
                project,
                binary,
            } => hooks::run_uninstall(&client, project, binary.as_deref()),
            HookAction::Status => hooks::run_status(),
            HookAction::Sync { path, scan, binary } => {
                if scan {
                    scanner::run_scan_all(scanner::ScanOptions {
                        roots: path.into_iter().collect(),
                        all_drives: false,
                        max_depth: 5,
                        dry_run: false,
                        binary,
                    })
                } else {
                    hooks::run_sync(path.as_deref(), binary.as_deref())
                }
            }
            HookAction::ScanAll {
                roots,
                all_drives,
                max_depth,
                dry_run,
                binary,
            } => scanner::run_scan_all(scanner::ScanOptions {
                roots,
                all_drives,
                max_depth,
                dry_run,
                binary,
            }),
        },
        Commands::InstallHooks {
            client,
            project,
            binary,
        } => hooks::run_install(&client, project, binary.as_deref()),
        Commands::Benchmark {
            json,
            markdown,
            export,
            submit,
            open_browser,
        } => run_benchmark(json, markdown, export, submit, open_browser),
        Commands::CheckGuardrails { fix } => guardrails::run_check(fix),
    }
}

fn run_benchmark(
    json: bool,
    markdown: bool,
    export: Option<std::path::PathBuf>,
    submit: bool,
    open_browser: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut bin = std::path::PathBuf::from("agent-otel-bench");
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let neighbor = parent.join(if cfg!(windows) {
                "agent-otel-bench.exe"
            } else {
                "agent-otel-bench"
            });
            if neighbor.exists() {
                bin = neighbor;
            }
        }
    }

    let mut cmd = std::process::Command::new(&bin);
    if json {
        cmd.arg("--json");
    }
    if markdown {
        cmd.arg("--markdown");
    }
    if let Some(p) = export {
        cmd.arg("--export").arg(p);
    }
    if submit {
        cmd.arg("--submit");
    }
    if open_browser {
        cmd.arg("--open-browser");
    }

    match cmd.status() {
        Ok(status) => {
            if !status.success() {
                std::process::exit(status.code().unwrap_or(1));
            }
            Ok(())
        }
        Err(_) => {
            println!("  [info] agent-otel-bench binary not found in PATH or adjacent folder.");
            println!("  Run benchmark directly with Cargo:\n");
            println!("    cargo run --release -p agent-otel-bench -- --submit\n");
            Ok(())
        }
    }
}

fn run_start() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Starting agent-otel-bridge daemon ===");

    if agent_otel_ipc::client::try_send(agent_otel_ipc::frame::MsgType::HealthPing, &[]).is_ok() {
        println!("  [ok] Daemon is already running and listening on named pipe.\n");
        return Ok(());
    }

    agent_otel_ipc::client::spawn_daemon_detached();

    let start = std::time::Instant::now();
    let mut started = false;
    while start.elapsed() < std::time::Duration::from_millis(1500) {
        std::thread::sleep(std::time::Duration::from_millis(50));
        if agent_otel_ipc::client::try_send(agent_otel_ipc::frame::MsgType::HealthPing, &[]).is_ok()
        {
            started = true;
            break;
        }
    }

    if started {
        println!("  [ok] Background daemon successfully launched and listening on named pipe!\n");
    } else {
        println!("  [warn] Spawn signal sent, but daemon has not yet responded on named pipe.");
        println!("         Run 'agent-otel-bridge doctor' for pipeline diagnostics.\n");
    }

    Ok(())
}

fn run_fast_hook(event_str: &str) {
    use agent_otel_ipc::frame::WireHeader;
    use std::io::{self, Read, Write};

    let event_id = match event_str {
        "PreInvocation" | "pre_invocation" => 1,
        "PostInvocation" | "post_invocation" => 2,
        "PreToolUse" | "pre_tool_use" => 3,
        "PostToolUse" | "post_tool_use" => 4,
        "Stop" | "stop" => 5,
        s => s.parse::<u8>().unwrap_or(255),
    };

    let header = WireHeader::new(1, event_id);

    let mut buf = Vec::with_capacity(4096);
    let mut stdin = io::stdin().take(256 * 1024);
    let _ = stdin.read_to_end(&mut buf);

    let mut payload = Vec::with_capacity(WireHeader::LEN + buf.len());
    payload.extend_from_slice(&header.encode());
    payload.extend_from_slice(&buf);

    agent_otel_ipc::client::send_fire_and_forget(
        agent_otel_ipc::frame::MsgType::HookPayload,
        &payload,
    );

    let mut stdout = io::stdout();
    if event_id == 3 {
        let _ = stdout.write_all(b"{\"decision\":\"allow\"}");
    } else {
        let _ = stdout.write_all(b"{}");
    }
    let _ = stdout.flush();
}
