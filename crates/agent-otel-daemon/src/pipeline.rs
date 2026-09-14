/* Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0 */
use crate::{
    batch::ExportBatch,
    diagnostics::Diagnostics,
    exporter::{OtlpExporter, EXPORT_DEADLINE},
};
use std::sync::{atomic::Ordering, Arc};
use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};

pub const EXPORT_QUEUE_BYTES: usize = 16 * 1024 * 1024;
pub const EXPORT_QUEUE_ITEMS: usize = 64;
pub struct QueuedBatch {
    pub batch: ExportBatch,
    _permit: OwnedSemaphorePermit,
    stats: Arc<Diagnostics>,
    attempted: bool,
    accounted: bool,
}
impl Drop for QueuedBatch {
    fn drop(&mut self) {
        self.stats
            .queued_bytes
            .fetch_sub(self.batch.encoded_bytes, Ordering::Relaxed);
        self.stats.queued_items.fetch_sub(1, Ordering::Relaxed);
        if !self.accounted {
            let counter = if self.attempted {
                &self.stats.unknown
            } else {
                &self.stats.shutdown_dropped
            };
            counter.fetch_add(self.batch.count as u64, Ordering::Relaxed);
        }
    }
}
pub struct ExportQueue {
    tx: mpsc::Sender<QueuedBatch>,
    bytes: Arc<Semaphore>,
    stats: Arc<Diagnostics>,
}
impl ExportQueue {
    pub fn new(stats: Arc<Diagnostics>) -> (Self, mpsc::Receiver<QueuedBatch>) {
        let (tx, rx) = mpsc::channel(EXPORT_QUEUE_ITEMS);
        (
            Self {
                tx,
                bytes: Arc::new(Semaphore::new(EXPORT_QUEUE_BYTES)),
                stats,
            },
            rx,
        )
    }
    pub fn enqueue(&self, batch: ExportBatch) {
        let count = batch.count;
        let Ok(permit) = self
            .bytes
            .clone()
            .try_acquire_many_owned(batch.encoded_bytes as u32)
        else {
            self.stats
                .export_capacity
                .fetch_add(count as u64, Ordering::Relaxed);
            return;
        };
        let bytes = self
            .stats
            .queued_bytes
            .fetch_add(batch.encoded_bytes, Ordering::Relaxed)
            + batch.encoded_bytes;
        self.stats
            .peak_queued_bytes
            .fetch_max(bytes, Ordering::Relaxed);
        self.stats.queued_items.fetch_add(1, Ordering::Relaxed);
        let queued = QueuedBatch {
            batch,
            _permit: permit,
            stats: self.stats.clone(),
            attempted: false,
            accounted: false,
        };
        match self.tx.try_send(queued) {
            Ok(()) => {
                self.stats
                    .export_queued
                    .fetch_add(count as u64, Ordering::Relaxed);
            }
            Err(error) => {
                let mut rejected = error.into_inner();
                rejected.accounted = true;
                self.stats
                    .export_capacity
                    .fetch_add(count as u64, Ordering::Relaxed);
            }
        }
    }
}
pub async fn export_worker(
    mut rx: mpsc::Receiver<QueuedBatch>,
    exporter: OtlpExporter,
    stats: Arc<Diagnostics>,
) {
    while let Some(mut item) = rx.recv().await {
        // Cancellation during an attempted request is conservatively unknown, never confirmed loss.
        item.attempted = true;
        let report = exporter
            .export_traces_until(
                &item.batch.request,
                tokio::time::Instant::now() + EXPORT_DEADLINE,
            )
            .await;
        stats.record_export(item.batch.count, &report);
        item.accounted = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::SpanBatcher;
    use opentelemetry_proto::tonic::{resource::v1::Resource, trace::v1::Span};
    #[test]
    fn queue_admission_is_nonblocking_and_drop_reconciles() {
        let stats = Arc::new(Diagnostics::default());
        let (queue, rx) = ExportQueue::new(stats.clone());
        for _ in 0..100 {
            let mut batch = SpanBatcher::new(Resource::default(), 1);
            batch.push(Span::default());
            queue.enqueue(batch.take().unwrap());
        }
        assert_eq!(stats.export_queued.load(Ordering::Relaxed), 64);
        assert_eq!(stats.export_capacity.load(Ordering::Relaxed), 36);
        drop(rx);
        drop(queue);
        assert_eq!(stats.shutdown_dropped.load(Ordering::Relaxed), 64);
        assert_eq!(stats.queued_bytes.load(Ordering::Relaxed), 0);
        assert_eq!(stats.queued_items.load(Ordering::Relaxed), 0);
    }
}
