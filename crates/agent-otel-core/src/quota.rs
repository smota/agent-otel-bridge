/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use opentelemetry_proto::tonic::{
    collector::metrics::v1::ExportMetricsServiceRequest,
    metrics::v1::{
        metric, number_data_point, Gauge, Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics,
    },
    resource::v1::Resource,
};

use crate::otlp::{instrumentation_scope, kv_string};
use crate::semconv::*;

#[derive(Debug, Clone, PartialEq)]
pub struct QuotaSnapshot {
    pub remaining_fraction: f64,
    pub seconds_to_reset: f64,
    pub observed_at_unix_nano: u64,
    pub bucket: String,
    pub group: String,
}

impl Default for QuotaSnapshot {
    fn default() -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        Self {
            remaining_fraction: 1.0,
            seconds_to_reset: 0.0,
            observed_at_unix_nano: now,
            bucket: QUOTA_DEFAULT_BUCKET.to_string(),
            group: QUOTA_DEFAULT_GROUP.to_string(),
        }
    }
}

pub fn quota_gauge(
    name: &str,
    unit: &str,
    value: f64,
    observed_ns: u64,
    bucket: &str,
    group: &str,
) -> Metric {
    Metric {
        name: name.to_string(),
        description: String::new(),
        unit: unit.to_string(),
        data: Some(metric::Data::Gauge(Gauge {
            data_points: vec![NumberDataPoint {
                attributes: vec![
                    kv_string(QUOTA_ATTR_BUCKET, bucket),
                    kv_string(QUOTA_ATTR_GROUP, group),
                    kv_string("gen_ai.provider.name", group),
                ],
                start_time_unix_nano: 0,
                time_unix_nano: observed_ns,
                value: Some(number_data_point::Value::AsDouble(value)),
                ..Default::default()
            }],
        })),
        ..Default::default()
    }
}

pub fn build_quota_metrics_request(
    resource: Resource,
    snapshot: &QuotaSnapshot,
) -> ExportMetricsServiceRequest {
    build_quota_metrics_request_opts(resource, snapshot, false)
}

pub fn build_quota_metrics_request_opts(
    resource: Resource,
    snapshot: &QuotaSnapshot,
    emit_legacy_aliases: bool,
) -> ExportMetricsServiceRequest {
    build_multi_quota_metrics_request_opts(
        resource,
        std::slice::from_ref(snapshot),
        emit_legacy_aliases,
    )
}

pub fn build_multi_quota_metrics_request(
    resource: Resource,
    snapshots: &[QuotaSnapshot],
) -> ExportMetricsServiceRequest {
    build_multi_quota_metrics_request_opts(resource, snapshots, false)
}

pub fn build_multi_quota_metrics_request_opts(
    resource: Resource,
    snapshots: &[QuotaSnapshot],
    emit_legacy_aliases: bool,
) -> ExportMetricsServiceRequest {
    let mut metrics = Vec::new();
    let mut min_remaining = 1.0f64;
    let mut has_finite_remaining = false;
    let mut latest_observed_ns = 0u64;

    for snapshot in snapshots {
        if snapshot.remaining_fraction.is_finite()
            && (0.0..=1.0).contains(&snapshot.remaining_fraction)
        {
            has_finite_remaining = true;
            if snapshot.remaining_fraction < min_remaining {
                min_remaining = snapshot.remaining_fraction;
            }
            if snapshot.observed_at_unix_nano > latest_observed_ns {
                latest_observed_ns = snapshot.observed_at_unix_nano;
            }

            // Canonical generic agent metric for this provider/bucket
            metrics.push(quota_gauge(
                METRIC_AGENT_QUOTA_REMAINING,
                "1",
                snapshot.remaining_fraction,
                snapshot.observed_at_unix_nano,
                &snapshot.bucket,
                &snapshot.group,
            ));
        }
    }

    // Fleet-wide normalized bottleneck gauge (0.0..=1.0) computed across all active providers
    if has_finite_remaining && latest_observed_ns > 0 {
        metrics.push(quota_gauge(
            METRIC_FLEET_BOTTLENECK_RATIO,
            "1",
            min_remaining,
            latest_observed_ns,
            "fleet-bottleneck",
            "all",
        ));
    }

    // Optional legacy remaining alias for backward compatibility with older v0.1 dashboards
    if emit_legacy_aliases {
        for snapshot in snapshots {
            if snapshot.remaining_fraction.is_finite()
                && (0.0..=1.0).contains(&snapshot.remaining_fraction)
                && (snapshot.group == "gemini" || snapshot.bucket.contains("gemini"))
            {
                metrics.push(quota_gauge(
                    METRIC_AGY_QUOTA_REMAINING,
                    "1",
                    snapshot.remaining_fraction,
                    snapshot.observed_at_unix_nano,
                    &snapshot.bucket,
                    &snapshot.group,
                ));
            }
        }
    }

    // Canonical reset seconds for each provider/bucket
    for snapshot in snapshots {
        if snapshot.seconds_to_reset.is_finite() && snapshot.seconds_to_reset >= 0.0 {
            if snapshot.observed_at_unix_nano > latest_observed_ns {
                latest_observed_ns = snapshot.observed_at_unix_nano;
            }

            metrics.push(quota_gauge(
                METRIC_AGENT_QUOTA_RESET,
                "s",
                snapshot.seconds_to_reset,
                snapshot.observed_at_unix_nano,
                &snapshot.bucket,
                &snapshot.group,
            ));
        }
    }

    // Optional legacy reset alias for backward compatibility with older v0.1 dashboards
    if emit_legacy_aliases {
        for snapshot in snapshots {
            if snapshot.seconds_to_reset.is_finite()
                && snapshot.seconds_to_reset >= 0.0
                && (snapshot.group == "gemini" || snapshot.bucket.contains("gemini"))
            {
                metrics.push(quota_gauge(
                    METRIC_AGY_QUOTA_RESET,
                    "s",
                    snapshot.seconds_to_reset,
                    snapshot.observed_at_unix_nano,
                    &snapshot.bucket,
                    &snapshot.group,
                ));
            }
        }
    }

    ExportMetricsServiceRequest {
        resource_metrics: vec![ResourceMetrics {
            resource: Some(resource),
            scope_metrics: vec![ScopeMetrics {
                scope: Some(instrumentation_scope()),
                metrics,
                schema_url: String::new(),
            }],
            schema_url: String::new(),
        }],
    }
}
