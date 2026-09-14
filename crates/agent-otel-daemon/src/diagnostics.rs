/* Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0 */
use crate::exporter::{ExportOutcome, ExportReport};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

#[derive(Default)]
pub struct Diagnostics {
    pub transformed: AtomicU64,
    pub invalid: AtomicU64,
    pub span_size: AtomicU64,
    pub export_queued: AtomicU64,
    pub export_capacity: AtomicU64,
    pub accepted: AtomicU64,
    pub rejected: AtomicU64,
    pub unknown: AtomicU64,
    pub shutdown_dropped: AtomicU64,
    pub attempts: AtomicU64,
    pub possible_duplicate_batches: AtomicU64,
    pub quota_activity_dropped: AtomicU64,
    pub queued_bytes: AtomicUsize,
    pub peak_queued_bytes: AtomicUsize,
    pub queued_items: AtomicUsize,
}
#[derive(Debug, Serialize)]
pub struct DiagnosticSnapshot {
    pub transformed: u64,
    pub invalid: u64,
    pub span_size: u64,
    pub export_queued: u64,
    pub export_capacity: u64,
    pub accepted: u64,
    pub rejected: u64,
    pub unknown: u64,
    pub shutdown_dropped: u64,
    pub attempts: u64,
    pub possible_duplicate_batches: u64,
    pub quota_activity_dropped: u64,
    pub queued_bytes: usize,
    pub peak_queued_bytes: usize,
    pub queued_items: usize,
}
impl Diagnostics {
    pub fn record_export(&self, count: usize, report: &ExportReport) {
        self.attempts
            .fetch_add(report.attempts as u64, Ordering::Relaxed);
        if report.may_duplicate {
            self.possible_duplicate_batches
                .fetch_add(1, Ordering::Relaxed);
        }
        match report.outcome {
            ExportOutcome::Accepted => {
                self.accepted.fetch_add(count as u64, Ordering::Relaxed);
            }
            ExportOutcome::PartiallyAccepted { rejected, .. } => {
                self.accepted
                    .fetch_add(count.saturating_sub(rejected) as u64, Ordering::Relaxed);
                self.rejected.fetch_add(rejected as u64, Ordering::Relaxed);
            }
            ExportOutcome::Rejected { .. } => {
                self.rejected.fetch_add(count as u64, Ordering::Relaxed);
            }
            ExportOutcome::Unknown { .. } => {
                self.unknown.fetch_add(count as u64, Ordering::Relaxed);
            }
        }
    }
    pub fn snapshot(&self) -> DiagnosticSnapshot {
        macro_rules! load {
            ($x:ident) => {
                self.$x.load(Ordering::Relaxed)
            };
        }
        DiagnosticSnapshot {
            transformed: load!(transformed),
            invalid: load!(invalid),
            span_size: load!(span_size),
            export_queued: load!(export_queued),
            export_capacity: load!(export_capacity),
            accepted: load!(accepted),
            rejected: load!(rejected),
            unknown: load!(unknown),
            shutdown_dropped: load!(shutdown_dropped),
            attempts: load!(attempts),
            possible_duplicate_batches: load!(possible_duplicate_batches),
            quota_activity_dropped: load!(quota_activity_dropped),
            queued_bytes: load!(queued_bytes),
            peak_queued_bytes: load!(peak_queued_bytes),
            queued_items: load!(queued_items),
        }
    }
}
