/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_core::model::{AntigravityHookInput, ExecutionMode, HookEvent};
use agent_otel_core::otlp::build_span_from_hook_opts;
use agent_otel_core::quota::build_quota_metrics_request_opts;
use agent_otel_ipc::frame::MsgType;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::batch::SpanBatcher;
use crate::config::DaemonConfig;
use crate::exporter::OtlpExporter;
use crate::quota::QuotaEngine;

pub struct Daemon {
    config: DaemonConfig,
    exporter: OtlpExporter,
    quota_engine: QuotaEngine,
    shutdown: CancellationToken,
}

impl Daemon {
    pub fn new(config: DaemonConfig, shutdown: CancellationToken) -> Result<Self, reqwest::Error> {
        let exporter = OtlpExporter::new(config.traces_url(), config.metrics_url())?;
        let quota_engine = QuotaEngine::new();

        Ok(Self {
            config,
            exporter,
            quota_engine,
            shutdown,
        })
    }

    pub async fn run(self) -> std::io::Result<()> {
        let (tx, mut rx) = mpsc::channel::<(MsgType, Vec<u8>)>(4096);
        let pipe_name = self.config.pipe_name.clone();
        let shutdown_server = self.shutdown.clone();

        // Spawn IPC Named Pipe server
        let server_handle = tokio::spawn(async move {
            agent_otel_ipc::server::run_server(Some(&pipe_name), tx, shutdown_server).await
        });

        println!(
            "[agent-otel-daemon] Listening on named pipe: {}",
            self.config.pipe_name
        );
        println!(
            "[agent-otel-daemon] OTLP traces endpoint: {}",
            self.config.traces_url()
        );
        println!(
            "[agent-otel-daemon] OTLP metrics endpoint: {}",
            self.config.metrics_url()
        );

        let mut batcher = SpanBatcher::new(self.config.resource(), self.config.batch_size);
        let mut last_activity = Instant::now();
        let mut salt_counter: u32 = 0;

        let mut flush_interval = tokio::time::interval(self.config.batch_timeout);
        let mut quota_interval = tokio::time::interval(self.config.quota_interval);
        let mut idle_interval = tokio::time::interval(std::time::Duration::from_secs(10));

        // Initial quota probe on start
        self.emit_quota_metrics().await;

        loop {
            tokio::select! {
                biased;

                maybe_msg = rx.recv() => {
                    match maybe_msg {
                        Some((MsgType::HookPayload, payload)) => {
                            last_activity = Instant::now();
                            if !payload.is_empty() {
                                let tag = payload[0];
                                let json_bytes = &payload[1..];
                                let mut event = HookEvent::from_tag(tag);
                                let client_kind = agent_otel_core::model::ClientKind::from_tag(tag);

                                let now_nano = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_nanos() as u64;

                                salt_counter = salt_counter.wrapping_add(1);

                                let mut input = AntigravityHookInput::parse_slice(json_bytes)
                                    .unwrap_or_default();

                                if event == HookEvent::Unknown {
                                    if let Some(ref name) = input.hook_event_name {
                                        event = HookEvent::from_str_name(name);
                                    }
                                }

                                if input.agent_name.is_none() {
                                    if let Some(c) = client_kind.as_str() {
                                        input.agent_name = Some(c.to_string());
                                    }
                                }

                                // Populate execution_mode if missing
                                if input.execution_mode.is_none() {
                                    let mode = if std::env::var("CI").is_ok()
                                        || std::env::var("GITHUB_ACTIONS").is_ok()
                                        || std::env::var("AUTOMATION").is_ok()
                                    {
                                        ExecutionMode::Automacao
                                    } else {
                                        ExecutionMode::Iterativo
                                    };
                                    input.execution_mode = Some(mode);
                                }

                                // On Stop event, collect Git stats from workspace
                                if event == HookEvent::Stop {
                                    let ws = input
                                        .workspace_paths
                                        .as_ref()
                                        .and_then(|v| v.first().cloned())
                                        .or_else(|| {
                                            std::env::current_dir()
                                                .ok()
                                                .map(|p| p.to_string_lossy().to_string())
                                        });
                                    if let Some(ws_path) = ws {
                                        let stats = crate::git::collect_git_stats(&ws_path);
                                        if stats.lines_added.is_some() || stats.files_changed.is_some() {
                                            input.git_lines_added = stats.lines_added;
                                            input.git_lines_deleted = stats.lines_deleted;
                                            input.git_files_changed = stats.files_changed;
                                        }
                                        if stats.self_revert.is_some() {
                                            input.git_self_revert = stats.self_revert;
                                        }
                                    }
                                }

                                let span = build_span_from_hook_opts(
                                    event,
                                    &input,
                                    now_nano.saturating_sub(1_000_000), // ~1ms approximate duration if not given
                                    now_nano,
                                    salt_counter,
                                    self.config.emit_legacy_aliases,
                                );

                                if batcher.push(span) {
                                    batcher.flush(&self.exporter).await;
                                }
                            }
                        }
                        Some((MsgType::QuotaPing, _)) => {
                            last_activity = Instant::now();
                            self.emit_quota_metrics().await;
                        }
                        Some((MsgType::HealthPing, _)) => {
                            last_activity = Instant::now();
                        }
                        Some((MsgType::Shutdown, _)) => {
                            println!("[agent-otel-daemon] Shutdown requested via IPC");
                            break;
                        }
                        Some((MsgType::Unknown, _)) => {}
                        None => {
                            // Channel closed
                            break;
                        }
                    }
                }

                _ = flush_interval.tick() => {
                    if !batcher.is_empty() {
                        batcher.flush(&self.exporter).await;
                    }
                }

                _ = quota_interval.tick() => {
                    last_activity = Instant::now();
                    self.emit_quota_metrics().await;
                }

                _ = idle_interval.tick() => {
                    if last_activity.elapsed() >= self.config.idle_timeout {
                        println!(
                            "[agent-otel-daemon] Idle timeout reached ({:?}), shutting down automatically",
                            self.config.idle_timeout
                        );
                        break;
                    }
                }

                _ = self.shutdown.cancelled() => {
                    println!("[agent-otel-daemon] Shutdown cancellation signal received");
                    break;
                }
            }
        }

        // Final flush of remaining spans
        batcher.flush(&self.exporter).await;
        self.shutdown.cancel();

        // Wait briefly for server handle
        let _ = tokio::time::timeout(std::time::Duration::from_millis(500), server_handle).await;

        println!("[agent-otel-daemon] Daemon cleanly stopped");
        Ok(())
    }

    async fn emit_quota_metrics(&self) {
        let snapshot = self.quota_engine.snapshot();
        let request = build_quota_metrics_request_opts(
            self.config.resource(),
            &snapshot,
            self.config.emit_legacy_aliases,
        );
        if let Err(e) = self.exporter.export_metrics(request).await {
            eprintln!("[agent-otel-daemon] Failed to export quota metrics: {e}");
        }
    }
}
