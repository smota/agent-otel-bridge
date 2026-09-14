/* Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0 */
use crate::{config::DaemonConfig, quota::QuotaEngine};
use agent_otel_core::quota::build_multi_quota_metrics_request_opts;
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use prost::Message;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::{Duration, Instant};
use tokio::sync::watch;

pub struct QuotaWorker {
    tx: mpsc::SyncSender<(String, Option<u64>)>,
    refresh: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}
impl QuotaWorker {
    pub fn start(
        config: DaemonConfig,
    ) -> std::io::Result<(Self, watch::Receiver<Option<ExportMetricsServiceRequest>>)> {
        let (tx, rx) = mpsc::sync_channel::<(String, Option<u64>)>(128);
        let (snap_tx, snap_rx) = watch::channel(None);
        let refresh = Arc::new(AtomicBool::new(true));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_refresh = refresh.clone();
        let thread_stop = stop.clone();
        std::thread::Builder::new()
            .name("agent-otel-quota".into())
            .spawn(move || {
                // Provider discovery and filesystem reads never hold up the ingress consumer.
                let engine = QuotaEngine::new();
                let mut last = Instant::now();
                while !thread_stop.load(Ordering::Acquire) {
                    match rx.recv_timeout(Duration::from_millis(20)) {
                        Ok((name, tokens)) => engine.record_activity(&name, tokens),
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    if thread_refresh.swap(false, Ordering::AcqRel)
                        || last.elapsed() >= config.quota_interval
                    {
                        let snapshots = engine.snapshots();
                        let request = build_multi_quota_metrics_request_opts(
                            config.resource(),
                            &snapshots,
                            config.emit_legacy_aliases,
                        );
                        if request.encoded_len() <= crate::exporter::MAX_REQUEST_BYTES {
                            snap_tx.send_replace(Some(request));
                        }
                        last = Instant::now();
                    }
                }
            })?;
        Ok((Self { tx, refresh, stop }, snap_rx))
    }
    pub fn activity(&self, name: &str, tokens: Option<u64>) -> bool {
        name.len() <= 256 && self.tx.try_send((name.to_owned(), tokens)).is_ok()
    }
    pub fn refresh(&self) {
        self.refresh.store(true, Ordering::Release);
    }
}
impl Drop for QuotaWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
