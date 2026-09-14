/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_core::otlp::{instrumentation_scope, kv_string};
use agent_otel_ipc::server::IngressStats;
use opentelemetry_proto::tonic::{
    collector::metrics::v1::ExportMetricsServiceRequest,
    metrics::v1::{
        metric, number_data_point, AggregationTemporality, Gauge, Metric, NumberDataPoint,
        ResourceMetrics, ScopeMetrics, Sum,
    },
};
use std::sync::atomic::Ordering;

use crate::context_cache::ContextCacheStats;
use crate::diagnostics::DiagnosticSnapshot;

pub const METRIC_BRIDGE_EVENTS: &str = "agent.bridge.events.total";
pub const METRIC_BRIDGE_DROPPED: &str = "agent.bridge.dropped.total";
pub const METRIC_BRIDGE_QUEUE_ITEMS: &str = "agent.bridge.queue.items";
pub const METRIC_BRIDGE_QUEUE_BYTES: &str = "agent.bridge.queue.bytes";
pub const METRIC_BRIDGE_CONTEXT_REFRESH: &str = "agent.bridge.context.refresh.total";
pub const METRIC_BRIDGE_EXPORT_ATTEMPTS: &str = "agent.bridge.export.attempts.total";
pub const METRIC_BRIDGE_CONNECTIONS: &str = "agent.bridge.connections";
pub const METRIC_BRIDGE_CONTEXT_WORKERS: &str = "agent.bridge.context.workers";

/// Appends one finite snapshot of bridge diagnostics to an OTLP metrics request.
///
/// All counter points are cumulative for the current daemon process. Labels
/// are finite enums only; paths, identities, trace IDs, and payload data never
/// enter these metrics. The helper constructs no spans and performs no I/O.
pub fn append_bridge_metrics(
    request: &mut ExportMetricsServiceRequest,
    observed_at_unix_nano: u64,
    ingress: &IngressStats,
    pipeline: &DiagnosticSnapshot,
    context: &ContextCacheStats,
) -> usize {
    let metrics = bridge_metrics(observed_at_unix_nano, ingress, pipeline, context);
    let appended = metrics.len();
    scope_metrics(request).metrics.extend(metrics);
    appended
}

fn bridge_metrics(
    observed_ns: u64,
    ingress: &IngressStats,
    pipeline: &DiagnosticSnapshot,
    context: &ContextCacheStats,
) -> Vec<Metric> {
    let events = counter(
        METRIC_BRIDGE_EVENTS,
        "Unique bridge events by processing stage and outcome.",
        "{event}",
        observed_ns,
        vec![
            point2(
                "stage",
                "frame_received",
                "outcome",
                "completed",
                ingress.received.load(Ordering::Relaxed),
            ),
            point2(
                "stage",
                "admitted",
                "outcome",
                "completed",
                ingress.admitted.load(Ordering::Relaxed),
            ),
            point2(
                "stage",
                "transformed",
                "outcome",
                "completed",
                pipeline.transformed,
            ),
            point2(
                "stage",
                "export_queued",
                "outcome",
                "completed",
                pipeline.export_queued,
            ),
            point2("stage", "backend", "outcome", "accepted", pipeline.accepted),
            point2("stage", "backend", "outcome", "rejected", pipeline.rejected),
            point2("stage", "backend", "outcome", "unknown", pipeline.unknown),
        ],
    );

    let dropped = counter(
        METRIC_BRIDGE_DROPPED,
        "Bridge events discarded before confirmed backend acceptance.",
        "{event}",
        observed_ns,
        vec![
            point1(
                "reason",
                "invalid_frame",
                ingress.invalid.load(Ordering::Relaxed),
            ),
            point1(
                "reason",
                "read_failed",
                ingress.read_failed.load(Ordering::Relaxed),
            ),
            point1(
                "reason",
                "read_deadline",
                ingress.read_deadline.load(Ordering::Relaxed),
            ),
            point1(
                "reason",
                "ingress_capacity",
                ingress.capacity.load(Ordering::Relaxed),
            ),
            point1(
                "reason",
                "control_capacity",
                ingress.control_dropped.load(Ordering::Relaxed),
            ),
            point1("reason", "invalid_event", pipeline.invalid),
            point1("reason", "span_size", pipeline.span_size),
            point1("reason", "export_capacity", pipeline.export_capacity),
            point1("reason", "backend_rejected", pipeline.rejected),
            point1("reason", "shutdown", pipeline.shutdown_dropped),
            point1(
                "reason",
                "quota_activity_capacity",
                pipeline.quota_activity_dropped,
            ),
        ],
    );

    let queue_items = gauge(
        METRIC_BRIDGE_QUEUE_ITEMS,
        "Current in-memory queue occupancy.",
        "{item}",
        observed_ns,
        vec![
            point1("queue", "export", pipeline.queued_items as u64),
            point1("queue", "context_refresh", context.queued_keys as u64),
        ],
    );
    let queue_bytes = gauge(
        METRIC_BRIDGE_QUEUE_BYTES,
        "Current and peak byte reservations by bounded queue.",
        "By",
        observed_ns,
        vec![
            point2(
                "queue",
                "ingress",
                "watermark",
                "current",
                ingress.reserved_bytes.load(Ordering::Relaxed) as u64,
            ),
            point2(
                "queue",
                "ingress",
                "watermark",
                "peak",
                ingress.peak_reserved_bytes.load(Ordering::Relaxed) as u64,
            ),
            point2(
                "queue",
                "export",
                "watermark",
                "current",
                pipeline.queued_bytes as u64,
            ),
            point2(
                "queue",
                "export",
                "watermark",
                "peak",
                pipeline.peak_queued_bytes as u64,
            ),
            point2(
                "queue",
                "context_cache",
                "watermark",
                "current",
                context.bytes as u64,
            ),
        ],
    );

    let context_refresh = counter(
        METRIC_BRIDGE_CONTEXT_REFRESH,
        "Workspace context refresh operations by result.",
        "{refresh}",
        observed_ns,
        vec![
            point1("result", "queued", context.queued),
            point1("result", "completed", context.completed),
            point1("result", "failed", context.failed),
            point1("result", "queue_full", context.queue_full),
            point1("result", "circuit_open", context.circuit_open),
            point1("result", "snapshot_size", context.rejected_snapshot_size),
        ],
    );
    let export_attempts = counter(
        METRIC_BRIDGE_EXPORT_ATTEMPTS,
        "OTLP export attempts and batches with possible duplicate delivery.",
        "{attempt}",
        observed_ns,
        vec![
            point1("result", "attempted", pipeline.attempts),
            point1(
                "result",
                "possible_duplicate_batch",
                pipeline.possible_duplicate_batches,
            ),
        ],
    );
    let connections = gauge(
        METRIC_BRIDGE_CONNECTIONS,
        "Current and peak simultaneous ingress connections.",
        "{connection}",
        observed_ns,
        vec![
            point1(
                "watermark",
                "current",
                ingress.active_connections.load(Ordering::Relaxed) as u64,
            ),
            point1(
                "watermark",
                "peak",
                ingress.peak_connections.load(Ordering::Relaxed) as u64,
            ),
        ],
    );
    let context_workers = gauge(
        METRIC_BRIDGE_CONTEXT_WORKERS,
        "Current context refresh worker state.",
        "{worker}",
        observed_ns,
        vec![
            point1("state", "active", context.active_workers as u64),
            point1("state", "stuck", context.stuck_workers as u64),
        ],
    );

    vec![
        events,
        dropped,
        queue_items,
        queue_bytes,
        context_refresh,
        export_attempts,
        connections,
        context_workers,
    ]
}

fn scope_metrics(request: &mut ExportMetricsServiceRequest) -> &mut ScopeMetrics {
    if request.resource_metrics.is_empty() {
        request.resource_metrics.push(ResourceMetrics::default());
    }
    let resource = &mut request.resource_metrics[0];
    let scope_name = instrumentation_scope().name;
    let index = resource.scope_metrics.iter().position(|scope| {
        scope
            .scope
            .as_ref()
            .is_some_and(|scope| scope.name == scope_name)
    });
    if let Some(index) = index {
        return &mut resource.scope_metrics[index];
    }
    resource.scope_metrics.push(ScopeMetrics {
        scope: Some(instrumentation_scope()),
        ..Default::default()
    });
    resource
        .scope_metrics
        .last_mut()
        .expect("scope was just appended")
}

fn counter(
    name: &str,
    description: &str,
    unit: &str,
    observed_ns: u64,
    points: Vec<Point>,
) -> Metric {
    Metric {
        name: name.to_string(),
        description: description.to_string(),
        unit: unit.to_string(),
        data: Some(metric::Data::Sum(Sum {
            data_points: number_points(points, observed_ns),
            aggregation_temporality: AggregationTemporality::Cumulative as i32,
            is_monotonic: true,
        })),
        ..Default::default()
    }
}

fn gauge(
    name: &str,
    description: &str,
    unit: &str,
    observed_ns: u64,
    points: Vec<Point>,
) -> Metric {
    Metric {
        name: name.to_string(),
        description: description.to_string(),
        unit: unit.to_string(),
        data: Some(metric::Data::Gauge(Gauge {
            data_points: number_points(points, observed_ns),
        })),
        ..Default::default()
    }
}

struct Point {
    labels: Vec<(&'static str, &'static str)>,
    value: u64,
}

fn point1(key: &'static str, value: &'static str, count: u64) -> Point {
    Point {
        labels: vec![(key, value)],
        value: count,
    }
}

fn point2(
    key1: &'static str,
    value1: &'static str,
    key2: &'static str,
    value2: &'static str,
    count: u64,
) -> Point {
    Point {
        labels: vec![(key1, value1), (key2, value2)],
        value: count,
    }
}

fn number_points(points: Vec<Point>, observed_ns: u64) -> Vec<NumberDataPoint> {
    points
        .into_iter()
        .map(|point| NumberDataPoint {
            attributes: point
                .labels
                .into_iter()
                .map(|(key, value)| kv_string(key, value))
                .collect(),
            start_time_unix_nano: 0,
            time_unix_nano: observed_ns,
            value: Some(number_data_point::Value::AsInt(
                i64::try_from(point.value).unwrap_or(i64::MAX),
            )),
            ..Default::default()
        })
        .collect()
}
