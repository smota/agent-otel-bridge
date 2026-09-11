/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_core::otlp::build_trace_request;
use opentelemetry_proto::tonic::resource::v1::Resource;
use opentelemetry_proto::tonic::trace::v1::Span;

use crate::exporter::OtlpExporter;

pub struct SpanBatcher {
    resource: Resource,
    batch_size: usize,
    spans: Vec<Span>,
}

impl SpanBatcher {
    pub fn new(resource: Resource, batch_size: usize) -> Self {
        Self {
            resource,
            batch_size,
            spans: Vec::with_capacity(batch_size),
        }
    }

    pub fn push(&mut self, span: Span) -> bool {
        self.spans.push(span);
        self.spans.len() >= self.batch_size
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    pub fn len(&self) -> usize {
        self.spans.len()
    }

    pub async fn flush(&mut self, exporter: &OtlpExporter) {
        if self.spans.is_empty() {
            return;
        }

        let to_export = std::mem::replace(&mut self.spans, Vec::with_capacity(self.batch_size));
        let count = to_export.len();
        let request = build_trace_request(self.resource.clone(), to_export);

        if let Err(e) = exporter.export_traces(request).await {
            eprintln!("[agent-otel-daemon] Failed to export {count} spans: {e}");
        }
    }
}
