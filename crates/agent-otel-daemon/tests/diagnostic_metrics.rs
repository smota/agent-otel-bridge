use agent_otel_daemon::context_cache::ContextCacheStats;
use agent_otel_daemon::diagnostic_metrics::{
    append_bridge_metrics, METRIC_BRIDGE_CONTEXT_REFRESH, METRIC_BRIDGE_DROPPED,
    METRIC_BRIDGE_EVENTS, METRIC_BRIDGE_QUEUE_BYTES,
};
use agent_otel_daemon::diagnostics::DiagnosticSnapshot;
use agent_otel_ipc::server::IngressStats;
use opentelemetry_proto::tonic::{
    collector::metrics::v1::ExportMetricsServiceRequest,
    common::v1::any_value,
    metrics::v1::{metric, number_data_point, AggregationTemporality, Metric, NumberDataPoint},
};
use std::collections::HashMap;
use std::sync::atomic::Ordering;

fn diagnostic_snapshot() -> DiagnosticSnapshot {
    DiagnosticSnapshot {
        transformed: 30,
        invalid: 3,
        span_size: 4,
        export_queued: 27,
        export_capacity: 5,
        accepted: 20,
        rejected: 6,
        unknown: 1,
        shutdown_dropped: 2,
        attempts: 9,
        possible_duplicate_batches: 1,
        quota_activity_dropped: 7,
        queued_bytes: 1024,
        peak_queued_bytes: 4096,
        queued_items: 8,
    }
}

fn context_snapshot() -> ContextCacheStats {
    ContextCacheStats {
        queued: 11,
        queue_full: 2,
        circuit_open: 1,
        completed: 8,
        failed: 3,
        rejected_snapshot_size: 1,
        active_workers: 2,
        stuck_workers: 1,
        queued_keys: 4,
        entries: 5,
        bytes: 2048,
    }
}

fn all_metrics(request: &ExportMetricsServiceRequest) -> Vec<&Metric> {
    request
        .resource_metrics
        .iter()
        .flat_map(|resource| &resource.scope_metrics)
        .flat_map(|scope| &scope.metrics)
        .collect()
}

fn metric_named<'a>(request: &'a ExportMetricsServiceRequest, name: &str) -> &'a Metric {
    all_metrics(request)
        .into_iter()
        .find(|metric| metric.name == name)
        .unwrap_or_else(|| panic!("missing metric {name}"))
}

fn label<'a>(point: &'a NumberDataPoint, key: &str) -> Option<&'a str> {
    point.attributes.iter().find_map(|attribute| {
        if attribute.key != key {
            return None;
        }
        match attribute.value.as_ref()?.value.as_ref()? {
            any_value::Value::StringValue(value) => Some(value.as_str()),
            _ => None,
        }
    })
}

fn int_value(point: &NumberDataPoint) -> i64 {
    match point.value {
        Some(number_data_point::Value::AsInt(value)) => value,
        _ => panic!("diagnostic point was not a finite integer"),
    }
}

#[test]
fn appends_cumulative_event_outcomes_without_conflating_unknown() {
    let ingress = IngressStats::default();
    ingress.received.store(40, Ordering::Relaxed);
    ingress.admitted.store(30, Ordering::Relaxed);
    let mut request = ExportMetricsServiceRequest::default();

    let appended = append_bridge_metrics(
        &mut request,
        123_000,
        &ingress,
        &diagnostic_snapshot(),
        &context_snapshot(),
    );

    assert_eq!(appended, 8);
    let events = metric_named(&request, METRIC_BRIDGE_EVENTS);
    assert_eq!(events.unit, "{event}");
    let metric::Data::Sum(sum) = events.data.as_ref().expect("event metric data") else {
        panic!("event total must be a cumulative sum");
    };
    assert!(sum.is_monotonic);
    assert_eq!(
        sum.aggregation_temporality,
        AggregationTemporality::Cumulative as i32
    );
    let backend: HashMap<_, _> = sum
        .data_points
        .iter()
        .filter(|point| label(point, "stage") == Some("backend"))
        .map(|point| {
            (
                label(point, "outcome").expect("backend outcome"),
                int_value(point),
            )
        })
        .collect();
    assert_eq!(backend.get("accepted"), Some(&20));
    assert_eq!(backend.get("rejected"), Some(&6));
    assert_eq!(backend.get("unknown"), Some(&1));
    assert_eq!(sum.data_points[0].time_unix_nano, 123_000);
}

#[test]
fn emits_exact_drop_refresh_and_byte_values_with_fixed_labels() {
    let ingress = IngressStats::default();
    ingress.invalid.store(2, Ordering::Relaxed);
    ingress.read_deadline.store(3, Ordering::Relaxed);
    ingress.reserved_bytes.store(512, Ordering::Relaxed);
    ingress.peak_reserved_bytes.store(8192, Ordering::Relaxed);
    let mut request = ExportMetricsServiceRequest::default();
    append_bridge_metrics(
        &mut request,
        456_000,
        &ingress,
        &diagnostic_snapshot(),
        &context_snapshot(),
    );

    let dropped = metric_named(&request, METRIC_BRIDGE_DROPPED);
    let metric::Data::Sum(drop_sum) = dropped.data.as_ref().expect("drop data") else {
        panic!("drop total must be a sum");
    };
    let drops: HashMap<_, _> = drop_sum
        .data_points
        .iter()
        .map(|point| {
            (
                label(point, "reason").expect("drop reason"),
                int_value(point),
            )
        })
        .collect();
    assert_eq!(drops.get("invalid_frame"), Some(&2));
    assert_eq!(drops.get("read_deadline"), Some(&3));
    assert_eq!(drops.get("invalid_event"), Some(&3));
    assert_eq!(drops.get("backend_rejected"), Some(&6));
    assert_eq!(drops.get("shutdown"), Some(&2));

    let refresh = metric_named(&request, METRIC_BRIDGE_CONTEXT_REFRESH);
    let metric::Data::Sum(refresh_sum) = refresh.data.as_ref().expect("refresh data") else {
        panic!("refresh total must be a sum");
    };
    let refreshes: HashMap<_, _> = refresh_sum
        .data_points
        .iter()
        .map(|point| {
            (
                label(point, "result").expect("refresh result"),
                int_value(point),
            )
        })
        .collect();
    assert_eq!(refreshes.get("completed"), Some(&8));
    assert_eq!(refreshes.get("failed"), Some(&3));
    assert_eq!(refreshes.get("circuit_open"), Some(&1));

    let bytes = metric_named(&request, METRIC_BRIDGE_QUEUE_BYTES);
    assert_eq!(bytes.unit, "By");
    let metric::Data::Gauge(byte_gauge) = bytes.data.as_ref().expect("byte data") else {
        panic!("queue bytes must be a gauge");
    };
    let ingress_current = byte_gauge.data_points.iter().find(|point| {
        label(point, "queue") == Some("ingress") && label(point, "watermark") == Some("current")
    });
    assert_eq!(ingress_current.map(int_value), Some(512));
}

#[test]
fn diagnostic_labels_never_include_identity_or_payload_dimensions() {
    let mut request = ExportMetricsServiceRequest::default();
    append_bridge_metrics(
        &mut request,
        789_000,
        &IngressStats::default(),
        &diagnostic_snapshot(),
        &context_snapshot(),
    );

    let allowed = [
        "stage",
        "outcome",
        "reason",
        "queue",
        "watermark",
        "result",
        "state",
    ];
    for metric in all_metrics(&request) {
        let points = match metric.data.as_ref().expect("diagnostic metric data") {
            metric::Data::Sum(sum) => &sum.data_points,
            metric::Data::Gauge(gauge) => &gauge.data_points,
            _ => panic!("unexpected diagnostic aggregation"),
        };
        for point in points {
            assert!(point
                .attributes
                .iter()
                .all(|attribute| allowed.contains(&attribute.key.as_str())));
            assert!(point.attributes.iter().all(|attribute| {
                label(point, &attribute.key)
                    .is_some_and(|value| !value.chars().any(|ch| matches!(ch, '/' | '\\' | '@')))
            }));
        }
    }
}
