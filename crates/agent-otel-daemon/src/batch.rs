/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_core::otlp::build_trace_request;
use opentelemetry_proto::tonic::resource::v1::Resource;
use opentelemetry_proto::tonic::trace::v1::Span;

use crate::exporter::OtlpExporter;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use prost::Message;
use std::time::{Duration, Instant};

pub struct ExportBatch {
    pub request: ExportTraceServiceRequest,
    pub count: usize,
    pub encoded_bytes: usize,
    pub oldest: Instant,
}

pub struct SpanBatcher {
    resource: Resource,
    batch_size: usize,
    spans: Vec<Span>,
    wire_bytes: usize,
    resource_bytes: usize,
    scope_bytes: usize,
    oldest: Option<Instant>,
}

impl SpanBatcher {
    pub fn new(resource: Resource, batch_size: usize) -> Self {
        let resource_bytes = field_bytes(resource.encoded_len());
        let scope_bytes = field_bytes(agent_otel_core::otlp::instrumentation_scope().encoded_len());
        Self {
            resource,
            batch_size: batch_size.clamp(1, 4096),
            spans: Vec::with_capacity(batch_size.clamp(1, 4096)),
            wire_bytes: 0,
            resource_bytes,
            scope_bytes,
            oldest: None,
        }
    }

    pub fn push(&mut self, span: Span) -> bool {
        self.oldest.get_or_insert_with(Instant::now);
        self.wire_bytes += field_bytes(span.encoded_len());
        self.spans.push(span);
        self.spans.len() >= self.batch_size
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    pub fn len(&self) -> usize {
        self.spans.len()
    }

    fn request_bytes(&self, spans_wire_bytes: usize) -> usize {
        field_bytes(self.resource_bytes + field_bytes(self.scope_bytes + spans_wire_bytes))
    }

    /// Bounded synchronous path. A returned batch precedes the newly buffered span.
    pub fn try_push(
        &mut self,
        span: Span,
        now: Instant,
    ) -> Result<Option<ExportBatch>, &'static str> {
        let bytes = field_bytes(span.encoded_len());
        if self.request_bytes(bytes) > crate::exporter::MAX_REQUEST_BYTES {
            return Err("span_size");
        }
        let ready = if !self.spans.is_empty()
            && self.request_bytes(self.wire_bytes + bytes) > crate::exporter::MAX_REQUEST_BYTES
        {
            self.take()
        } else {
            None
        };
        self.oldest.get_or_insert(now);
        self.wire_bytes += bytes;
        self.spans.push(span);
        Ok(ready)
    }

    pub fn is_full(&self) -> bool {
        self.spans.len() >= self.batch_size
    }

    pub fn take_due(&mut self, now: Instant, max_age: Duration) -> Option<ExportBatch> {
        if self
            .oldest
            .is_some_and(|start| now.saturating_duration_since(start) >= max_age)
        {
            self.take()
        } else {
            None
        }
    }

    pub fn take(&mut self) -> Option<ExportBatch> {
        if self.spans.is_empty() {
            return None;
        }
        let count = self.spans.len();
        let predicted = self.request_bytes(self.wire_bytes);
        let spans = std::mem::replace(&mut self.spans, Vec::with_capacity(self.batch_size));
        let request = build_trace_request(self.resource.clone(), spans);
        let encoded_bytes = request.encoded_len();
        debug_assert_eq!(predicted, encoded_bytes);
        self.wire_bytes = 0;
        Some(ExportBatch {
            request,
            count,
            encoded_bytes,
            oldest: self.oldest.take().expect("nonempty batch has age"),
        })
    }

    pub async fn flush(&mut self, exporter: &OtlpExporter) {
        if self.spans.is_empty() {
            return;
        }

        let batch = self.take().expect("nonempty batch");
        let count = batch.count;
        let request = batch.request;

        if let Err(e) = exporter.export_traces(request).await {
            eprintln!("[agent-otel-daemon] Failed to export {count} spans: {e}");
        }
    }
}

fn field_bytes(len: usize) -> usize {
    let mut value = len;
    let mut varint = 1;
    while value >= 128 {
        value >>= 7;
        varint += 1;
    }
    1 + varint + len
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_request_accounting_and_oldest_age() {
        let mut batch = SpanBatcher::new(Resource::default(), 50);
        let now = Instant::now();
        batch
            .try_push(
                Span {
                    name: "x".repeat(130),
                    ..Default::default()
                },
                now,
            )
            .unwrap();
        batch
            .try_push(Span::default(), now + Duration::from_millis(100))
            .unwrap();
        assert!(batch
            .take_due(now + Duration::from_millis(199), Duration::from_millis(200))
            .is_none());
        let ready = batch
            .take_due(now + Duration::from_millis(200), Duration::from_millis(200))
            .unwrap();
        assert_eq!(ready.count, 2);
        assert_eq!(ready.encoded_bytes, ready.request.encode_to_vec().len());
        assert!(batch.is_empty());
        assert!(batch
            .try_push(
                Span {
                    name: "x".repeat(crate::exporter::MAX_REQUEST_BYTES),
                    ..Default::default()
                },
                now
            )
            .is_err());
    }
}
