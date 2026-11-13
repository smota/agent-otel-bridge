/* Copyright agent-otel-bridge contributors
 * SPDX-License-Identifier: Apache-2.0 */
use crate::{
    batch::SpanBatcher,
    config::{CaptureMode, DaemonConfig, WorkspacePathMode},
    context_cache::ContextCache,
    diagnostics::Diagnostics,
    exporter::{ExporterBuildError, OtlpExporter},
    pipeline::{export_worker, ExportQueue},
    quota_worker::QuotaWorker,
    span_assembler::SpanAssembler,
};
use agent_otel_core::model::ExecutionMode;
use agent_otel_core::otlp::ResolvedSpanMetadata;
use agent_otel_ipc::frame::MsgType;
use agent_otel_ipc::server::{IngressFrame, IngressLimits, IngressStats};
use std::sync::{atomic::Ordering, Arc};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub struct Daemon {
    config: DaemonConfig,
    exporter: OtlpExporter,
    shutdown: CancellationToken,
    pseudonym_key: Option<crate::pseudonym_key::PseudonymKey>,
}
impl Daemon {
    pub fn new(
        config: DaemonConfig,
        shutdown: CancellationToken,
    ) -> Result<Self, ExporterBuildError> {
        let exporter = OtlpExporter::from_config(&config)?;
        // Startup only. Errors contain no key, path or raw identifiers and fail closed.
        let pseudonym_key = crate::pseudonym_key::runtime_home()
            .and_then(|home| crate::pseudonym_key::PseudonymKey::load_or_create(&home).ok());
        Ok(Self {
            config,
            exporter,
            shutdown,
            pseudonym_key,
        })
    }
    pub async fn run(self) -> std::io::Result<()> {
        self.run_with_diagnostics(
            Arc::new(Diagnostics::default()),
            Arc::new(IngressStats::default()),
        )
        .await
    }
    /// Run with externally inspectable fact counters (also used by isolated conformance probes).
    pub async fn run_with_diagnostics(
        self,
        stats: Arc<Diagnostics>,
        ingress: Arc<IngressStats>,
    ) -> std::io::Result<()> {
        let session_link = crate::session_link::SessionLinkConfig::from_env()
            .map_err(|reason| std::io::Error::new(std::io::ErrorKind::InvalidInput, reason))?;
        if self.config.batch_size == 0
            || self.config.batch_size > 4096
            || self.config.batch_timeout.is_zero()
            || self.config.quota_interval.is_zero()
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid daemon pipeline configuration",
            ));
        }
        if self.pseudonym_key.is_none() {
            stats
                .pseudonym_key_unavailable
                .fetch_add(1, Ordering::Relaxed);
        }
        use prost::Message;
        let schema_url = if self.config.gen_ai_latest_experimental {
            agent_otel_core::semconv::OTEL_SCHEMA_1_41_0
        } else {
            ""
        };
        if agent_otel_core::otlp::build_trace_request_with_schema(
            self.config.resource(),
            Vec::new(),
            schema_url,
        )
        .encoded_len()
            >= crate::exporter::MAX_REQUEST_BYTES
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "resource exceeds request limit",
            ));
        }
        let (tx, mut rx) = mpsc::channel(4096);
        let (ctrl, mut controls) = mpsc::channel(8);
        let name = self.config.pipe_name.clone();
        let stop_server = self.shutdown.clone();
        let ingress_copy = ingress.clone();
        let mut server = tokio::spawn(async move {
            agent_otel_ipc::server::run_server_bounded(
                Some(&name),
                tx,
                ctrl,
                stop_server,
                IngressLimits::default(),
                ingress_copy,
            )
            .await
        });
        let (queue, export_rx) = ExportQueue::new(stats.clone());
        let mut export = tokio::spawn(export_worker(
            export_rx,
            self.exporter.clone(),
            stats.clone(),
        ));
        let (quotas, mut metrics_rx) = match QuotaWorker::start(self.config.clone()) {
            Ok(worker) => worker,
            Err(error) => {
                self.shutdown.cancel();
                server.abort();
                export.abort();
                let _ = server.await;
                let _ = export.await;
                return Err(error);
            }
        };
        let contexts = Arc::new(ContextCache::new());
        let metrics_exporter = self.exporter.clone();
        let metrics_stats = stats.clone();
        let metrics_ingress = ingress.clone();
        let metrics_contexts = contexts.clone();
        let metrics_resource = self.config.resource();
        let metrics_semconv_opt_in = self.config.gen_ai_latest_experimental;
        let mut metrics = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    changed = metrics_rx.changed() => { if changed.is_err() { break; } },
                    _ = interval.tick() => {},
                }
                let mut request = metrics_rx.borrow_and_update().clone().unwrap_or_else(|| {
                    agent_otel_core::quota::build_multi_quota_metrics_request_opts(
                        metrics_resource.clone(),
                        &[],
                        false,
                    )
                });
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64;
                crate::diagnostic_metrics::append_bridge_metrics(
                    &mut request,
                    now,
                    &metrics_ingress,
                    &metrics_stats.snapshot(),
                    &metrics_contexts.stats(Instant::now()),
                    metrics_semconv_opt_in,
                );
                let _ = metrics_exporter
                    .export_metrics_until(
                        &request,
                        tokio::time::Instant::now() + metrics_exporter.metrics_deadline(),
                    )
                    .await;
            }
        });
        let mut batch =
            SpanBatcher::with_schema(self.config.resource(), self.config.batch_size, schema_url);
        let mut span_assembler = SpanAssembler::default();
        let mut salt = 0u32;
        // Resolve ambient metadata once, never on the event-processing path.
        let user_email = ambient_user_email(&self.config);
        let terminal_type = std::env::var("TERM_PROGRAM")
            .ok()
            .or_else(|| std::env::var_os("WT_SESSION").map(|_| "windows-terminal".to_string()))
            .or_else(|| std::env::var("TERM").ok());
        let metadata = ResolvedSpanMetadata {
            user_email: user_email.as_deref(),
            terminal_type: terminal_type.as_deref(),
        };
        let execution_mode = if ["CI", "GITHUB_ACTIONS", "AUTOMATION"]
            .iter()
            .any(|k| std::env::var_os(k).is_some())
        {
            ExecutionMode::Automation
        } else {
            ExecutionMode::Interactive
        };
        let mut last_activity = Instant::now();
        let mut tick =
            tokio::time::interval(self.config.batch_timeout.min(Duration::from_millis(200)));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut server_finished = false;
        let mut server_error = None;
        let mut processed = 0;
        loop {
            if self.shutdown.is_cancelled() {
                break;
            }
            if processed >= 64 {
                if let Some(ready) = batch.take_due(Instant::now(), self.config.batch_timeout) {
                    queue.enqueue(ready);
                }
                processed = 0;
                tokio::task::yield_now().await;
            }
            tokio::select! {
                _ = self.shutdown.cancelled() => break,
                result = &mut server => {
                    server_finished = true;
                    server_error = match result {
                        Ok(Ok(())) if self.shutdown.is_cancelled() => None,
                        Ok(Ok(())) => Some(std::io::Error::other("IPC server stopped")),
                        Ok(Err(error)) => Some(error),
                        Err(error) => Some(std::io::Error::other(error.to_string())),
                    };
                    break;
                }
                control = controls.recv() => {
                    match control {
                        Some(MsgType::QuotaPing) => { quotas.refresh(); last_activity = Instant::now(); },
                        Some(MsgType::Shutdown) => break,
                        Some(_) => last_activity = Instant::now(),
                        None => break,
                    }
                }
                frame = rx.recv() => {
                    let Some(frame) = frame else { break; };
                    last_activity = Instant::now();
                    process_frame(frame, &contexts, &quotas, &mut batch, &queue, &stats, &mut salt, execution_mode, self.config.emit_legacy_aliases, self.config.gen_ai_latest_experimental, metadata, self.config.workspace_path_mode, self.config.capture_mode, self.pseudonym_key.as_ref(), &session_link, &mut span_assembler);
                    processed += 1;
                }
                _ = tick.tick() => {
                    if let Some(ready) = batch.take_due(Instant::now(), self.config.batch_timeout) { queue.enqueue(ready); }
                    if last_activity.elapsed() >= self.config.idle_timeout { break; }
                }
            }
        }
        // One absolute shutdown budget. Queue guards reconcile all unexported work.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        self.shutdown.cancel();
        drop(quotas);
        metrics.abort();
        let _ = (&mut metrics).await;
        if !server_finished
            && tokio::time::timeout_at(deadline, &mut server)
                .await
                .is_err()
        {
            server.abort();
            let _ = (&mut server).await;
        }
        rx.close();
        while let Ok(frame) = rx.try_recv() {
            if tokio::time::Instant::now() >= deadline {
                stats.shutdown_dropped.fetch_add(1, Ordering::Relaxed);
            } else {
                // Quota collection is stopped during drain; event transformation remains pure.
                process_frame_without_quota(
                    frame,
                    &contexts,
                    &mut batch,
                    &queue,
                    &stats,
                    &mut salt,
                    execution_mode,
                    self.config.emit_legacy_aliases,
                    self.config.gen_ai_latest_experimental,
                    metadata,
                    self.config.workspace_path_mode,
                    self.config.capture_mode,
                    self.pseudonym_key.as_ref(),
                    &session_link,
                    &mut span_assembler,
                );
            }
        }
        if let Some(ready) = batch.take() {
            queue.enqueue(ready);
        }
        drop(queue);
        if tokio::time::timeout_at(deadline, &mut export)
            .await
            .is_err()
        {
            export.abort();
            let _ = (&mut export).await;
        }
        // Bounded fact-only summary, useful to an owned test collector even if OTLP is down.
        let summary = serde_json::json!({"kind":"bridge_diagnostics", "pipeline":stats.snapshot(),
            "ingress":{"received":ingress.received.load(Ordering::Relaxed),"admitted":ingress.admitted.load(Ordering::Relaxed),
                "invalid":ingress.invalid.load(Ordering::Relaxed),"capacity":ingress.capacity.load(Ordering::Relaxed),
                "read_failed":ingress.read_failed.load(Ordering::Relaxed),"read_deadline":ingress.read_deadline.load(Ordering::Relaxed),
                "reserved_bytes":ingress.reserved_bytes.load(Ordering::Relaxed),"peak_bytes":ingress.peak_reserved_bytes.load(Ordering::Relaxed),
                "accept_created":ingress.accept_created.load(Ordering::Relaxed),"accept_create_failed":ingress.accept_create_failed.load(Ordering::Relaxed),
                "accept_connected":ingress.accept_connected.load(Ordering::Relaxed),"accept_connect_failed":ingress.accept_connect_failed.load(Ordering::Relaxed),
                "accept_spawned":ingress.accept_spawned.load(Ordering::Relaxed),"accept_polled":ingress.accept_polled.load(Ordering::Relaxed),
                "replacement_create_count":ingress.replacement_create_count.load(Ordering::Relaxed),"replacement_create_max_micros":ingress.replacement_create_max_micros.load(Ordering::Relaxed),
                "dispatch_to_spawn_count":ingress.dispatch_to_spawn_count.load(Ordering::Relaxed),"dispatch_to_spawn_max_micros":ingress.dispatch_to_spawn_max_micros.load(Ordering::Relaxed),
                "completion_to_dispatch_count":ingress.completion_to_dispatch_count.load(Ordering::Relaxed),"completion_to_dispatch_max_micros":ingress.completion_to_dispatch_max_micros.load(Ordering::Relaxed),
                "accept_spawn_to_poll_count":ingress.accept_spawn_to_poll_count.load(Ordering::Relaxed),"accept_spawn_to_poll_max_micros":ingress.accept_spawn_to_poll_max_micros.load(Ordering::Relaxed)}});
        eprintln!("{summary}");
        if let Some(error) = server_error {
            Err(error)
        } else {
            Ok(())
        }
    }
}

/// Ambient email is eligible only in opt-in modes. Git config and
/// `$GIT_AUTHOR_EMAIL` run only with enrichment enabled, and only at startup.
pub fn ambient_user_email(config: &DaemonConfig) -> Option<String> {
    ambient_user_email_with(
        config,
        &|key| std::env::var(key).ok(),
        agent_otel_core::context::harvest_user_email,
    )
}

fn ambient_user_email_with(
    config: &DaemonConfig,
    get: &dyn Fn(&str) -> Option<String>,
    harvest: impl FnOnce() -> Option<String>,
) -> Option<String> {
    if config.capture_mode == CaptureMode::Minimal {
        return None;
    }
    let explicit = get("USER_EMAIL");
    if !config.enrich_email {
        return explicit;
    }
    explicit
        .or_else(|| get("GIT_AUTHOR_EMAIL"))
        .or_else(harvest)
}

#[allow(clippy::too_many_arguments)]
fn process_frame(
    frame: IngressFrame,
    contexts: &ContextCache,
    quotas: &QuotaWorker,
    batch: &mut SpanBatcher,
    queue: &ExportQueue,
    stats: &Diagnostics,
    salt: &mut u32,
    mode: ExecutionMode,
    aliases: bool,
    gen_ai_latest_experimental: bool,
    metadata: ResolvedSpanMetadata<'_>,
    workspace_path_mode: WorkspacePathMode,
    capture_mode: CaptureMode,
    pseudonym_key: Option<&crate::pseudonym_key::PseudonymKey>,
    session_link: &crate::session_link::SessionLinkConfig,
    assembler: &mut SpanAssembler,
) {
    transform(
        frame,
        contexts,
        Some(quotas),
        batch,
        queue,
        stats,
        salt,
        mode,
        aliases,
        gen_ai_latest_experimental,
        metadata,
        workspace_path_mode,
        capture_mode,
        pseudonym_key,
        session_link,
        assembler,
    );
}
#[allow(clippy::too_many_arguments)]
fn process_frame_without_quota(
    frame: IngressFrame,
    contexts: &ContextCache,
    batch: &mut SpanBatcher,
    queue: &ExportQueue,
    stats: &Diagnostics,
    salt: &mut u32,
    mode: ExecutionMode,
    aliases: bool,
    gen_ai_latest_experimental: bool,
    metadata: ResolvedSpanMetadata<'_>,
    workspace_path_mode: WorkspacePathMode,
    capture_mode: CaptureMode,
    pseudonym_key: Option<&crate::pseudonym_key::PseudonymKey>,
    session_link: &crate::session_link::SessionLinkConfig,
    assembler: &mut SpanAssembler,
) {
    transform(
        frame,
        contexts,
        None,
        batch,
        queue,
        stats,
        salt,
        mode,
        aliases,
        gen_ai_latest_experimental,
        metadata,
        workspace_path_mode,
        capture_mode,
        pseudonym_key,
        session_link,
        assembler,
    );
}
#[allow(clippy::too_many_arguments)]
fn transform(
    frame: IngressFrame,
    contexts: &ContextCache,
    quotas: Option<&QuotaWorker>,
    batch: &mut SpanBatcher,
    queue: &ExportQueue,
    stats: &Diagnostics,
    salt: &mut u32,
    mode: ExecutionMode,
    aliases: bool,
    gen_ai_latest_experimental: bool,
    metadata: ResolvedSpanMetadata<'_>,
    workspace_path_mode: WorkspacePathMode,
    capture_mode: CaptureMode,
    pseudonym_key: Option<&crate::pseudonym_key::PseudonymKey>,
    session_link: &crate::session_link::SessionLinkConfig,
    assembler: &mut SpanAssembler,
) {
    let now_instant = Instant::now();
    let now_nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let next_salt = salt.wrapping_add(1);

    let outcome = match crate::transform::production_transform_with_session_link(
        frame.msg_type,
        &frame.payload,
        contexts,
        next_salt,
        mode,
        aliases,
        gen_ai_latest_experimental,
        metadata,
        now_instant,
        now_nanos,
        workspace_path_mode,
        capture_mode,
        pseudonym_key,
        Some(session_link),
    ) {
        Ok(res) => {
            *salt = next_salt;
            res
        }
        Err(_) => {
            stats.invalid.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };

    if let (Some(quotas), Some(name)) = (quotas, outcome.meta.agent_name.as_deref()) {
        if !quotas.activity(name, outcome.meta.total_tokens) {
            stats.quota_activity_dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    stats.record_trace_fallback(outcome.trace_fallback_unkeyed);
    stats.transformed.fetch_add(1, Ordering::Relaxed);
    for span in assembler.observe(outcome.span, now_instant) {
        match batch.try_push(span, Instant::now()) {
            Ok(ready) => {
                if let Some(ready) = ready {
                    queue.enqueue(ready);
                }
                if batch.is_full() {
                    if let Some(ready) = batch.take() {
                        queue.enqueue(ready);
                    }
                }
            }
            Err(_) => {
                stats.span_size.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

#[cfg(test)]
mod ambient_email_tests {
    use super::*;

    #[test]
    fn default_invalid_and_minimal_skip_all_ambient_reads_even_with_enrichment() {
        for configured in [None, Some("invalid"), Some("minimal")] {
            let config = DaemonConfig::from_vars(&|key| match key {
                "AGENT_OTEL_CAPTURE_MODE" => configured.map(str::to_string),
                "AGENT_OTEL_ENRICH_EMAIL" => Some("true".to_string()),
                _ => None,
            });
            assert_eq!(
                ambient_user_email_with(
                    &config,
                    &|_| panic!("Minimal must not read ambient email"),
                    || panic!("Minimal must not harvest git email")
                ),
                None
            );
        }
    }
}
