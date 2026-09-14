/* Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0 */
use crate::{
    batch::SpanBatcher,
    config::DaemonConfig,
    context_cache::{ContextCache, ContextState},
    diagnostics::Diagnostics,
    exporter::{OtlpExporter, EXPORT_DEADLINE},
    pipeline::{export_worker, ExportQueue},
    quota_worker::QuotaWorker,
};
use agent_otel_core::model::{AntigravityHookInput, ExecutionMode, HookEvent, WireHeader};
use agent_otel_core::otlp::{build_span_from_resolved, kv_int, kv_string, ResolvedSpanMetadata};
use agent_otel_core::trace_id::resolve_trace_context;
use agent_otel_ipc::frame::{decode_context_payload, MsgType};
use agent_otel_ipc::server::{IngressFrame, IngressLimits, IngressStats};
use std::sync::{atomic::Ordering, Arc};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub struct Daemon {
    config: DaemonConfig,
    exporter: OtlpExporter,
    shutdown: CancellationToken,
}
impl Daemon {
    pub fn new(config: DaemonConfig, shutdown: CancellationToken) -> Result<Self, reqwest::Error> {
        let exporter = OtlpExporter::new(config.traces_url(), config.metrics_url())?;
        Ok(Self {
            config,
            exporter,
            shutdown,
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
        use prost::Message;
        if agent_otel_core::otlp::build_trace_request(self.config.resource(), Vec::new())
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
                );
                let _ = metrics_exporter
                    .export_metrics_until(&request, tokio::time::Instant::now() + EXPORT_DEADLINE)
                    .await;
            }
        });
        let mut batch = SpanBatcher::new(self.config.resource(), self.config.batch_size);
        let mut salt = 0u32;
        // Resolve ambient metadata once, never on the event-processing path.
        let user_email = std::env::var("USER_EMAIL")
            .or_else(|_| std::env::var("GIT_AUTHOR_EMAIL"))
            .ok();
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
                    process_frame(frame, &contexts, &quotas, &mut batch, &queue, &stats, &mut salt, execution_mode, self.config.emit_legacy_aliases, metadata);
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
                    metadata,
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
                "reserved_bytes":ingress.reserved_bytes.load(Ordering::Relaxed),"peak_bytes":ingress.peak_reserved_bytes.load(Ordering::Relaxed)}});
        eprintln!("{summary}");
        if let Some(error) = server_error {
            Err(error)
        } else {
            Ok(())
        }
    }
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
    metadata: ResolvedSpanMetadata<'_>,
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
        metadata,
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
    metadata: ResolvedSpanMetadata<'_>,
) {
    transform(
        frame, contexts, None, batch, queue, stats, salt, mode, aliases, metadata,
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
    metadata: ResolvedSpanMetadata<'_>,
) {
    let decoded = match frame.msg_type {
        MsgType::HookPayload => {
            WireHeader::decode(&frame.payload).map(|h| (h, None, &frame.payload[WireHeader::LEN..]))
        }
        MsgType::HookPayloadWithContext => decode_context_payload(&frame.payload).ok(),
        _ => None,
    };
    let Some((header, traceparent, bytes)) = decoded else {
        stats.invalid.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Ok(mut input) = AntigravityHookInput::parse_slice(bytes) else {
        stats.invalid.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let mut event = HookEvent::from_wire(header.event_id);
    if event == HookEvent::Unknown {
        if let Some(name) = &input.hook_event_name {
            event = HookEvent::from_str_name(name);
        }
    }
    if input.agent_name.is_none() {
        input.agent_name = agent_otel_core::model::ClientKind::from_wire(header.client_id)
            .as_str()
            .map(str::to_owned);
    }
    input.execution_mode.get_or_insert(mode);
    if let (Some(quotas), Some(name)) = (quotas, input.agent_name.as_deref()) {
        let tokens = match (input.input_tokens, input.output_tokens) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0).saturating_add(b.unwrap_or(0)).max(0) as u64),
        };
        if !quotas.activity(name, tokens) {
            stats.quota_activity_dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
    let context = resolve_trace_context(
        input.traceparent.as_deref(),
        traceparent,
        input.conversation_id.as_deref(),
    );
    let lookup = contexts.lookup(&input, Instant::now());
    input.normalize_with_context(lookup.context.as_ref());
    *salt = salt.wrapping_add(1);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let mut span =
        build_span_from_resolved(event, &input, now, now, *salt, context, metadata, aliases);
    let provided = lookup.context.is_none()
        && input
            .workspace_path
            .as_ref()
            .is_some_and(|p| std::path::Path::new(p).is_absolute());
    let state = if provided {
        "provided"
    } else {
        match lookup.state {
            ContextState::Fresh => "fresh",
            ContextState::Stale => "stale",
            ContextState::Missing => "missing",
        }
    };
    span.attributes
        .push(kv_string("agent.context.state", state));
    span.attributes.push(kv_string(
        "agent.context.source",
        if lookup.context.is_some() {
            "workspace_cache"
        } else if provided {
            "event"
        } else {
            "none"
        },
    ));
    if !provided && lookup.context.is_some() {
        if let Some(age) = lookup.age {
            span.attributes.push(kv_int(
                "agent.context.age_ms",
                age.as_millis().min(i64::MAX as u128) as i64,
            ));
        }
    }
    stats.transformed.fetch_add(1, Ordering::Relaxed);
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
