/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_core::otlp::build_resource;
use opentelemetry_proto::tonic::resource::v1::Resource;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub otlp_endpoint: String,
    pub service_name: String,
    pub environment: String,
    pub service_version: String,
    pub pipe_name: String,
    pub batch_size: usize,
    pub batch_timeout: Duration,
    pub quota_interval: Duration,
    pub idle_timeout: Duration,
    pub emit_legacy_aliases: bool,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

impl DaemonConfig {
    pub fn from_env() -> Self {
        let otlp_endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
            .unwrap_or_else(|_| "http://127.0.0.1:4318".to_string());

        let service_name =
            std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "agent-otel-bridge".to_string());

        let environment = std::env::var("OTEL_RESOURCE_ATTRIBUTES")
            .ok()
            .and_then(|attrs| {
                for pair in attrs.split(',') {
                    let mut parts = pair.splitn(2, '=');
                    if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                        if k.trim() == "deployment.environment" {
                            return Some(v.trim().to_string());
                        }
                    }
                }
                None
            })
            .unwrap_or_else(|| "homelab".to_string());

        let service_version = env!("CARGO_PKG_VERSION").to_string();

        let pipe_name = std::env::var("AGENT_OTEL_PIPE")
            .or_else(|_| std::env::var("AGY_OTEL_PIPE"))
            .unwrap_or_else(|_| r"\\.\pipe\agent-otel".to_string());

        let batch_size = std::env::var("AGENT_OTEL_BATCH_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(50);

        let batch_timeout_ms = std::env::var("AGENT_OTEL_BATCH_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(200);

        let quota_interval_secs = std::env::var("AGENT_OTEL_QUOTA_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);

        let idle_timeout_secs = std::env::var("AGENT_OTEL_IDLE_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0); // 0 = disabled (persistent background service by default)
        let idle_timeout = if idle_timeout_secs == 0 {
            Duration::MAX
        } else {
            Duration::from_secs(idle_timeout_secs)
        };

        let emit_legacy_aliases = std::env::var("AGENT_OTEL_LEGACY_ATTRIBUTES")
            .or_else(|_| std::env::var("AGENT_OTEL_EMIT_AGY_ALIASES"))
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        Self {
            otlp_endpoint,
            service_name,
            environment,
            service_version,
            pipe_name,
            batch_size,
            batch_timeout: Duration::from_millis(batch_timeout_ms),
            quota_interval: Duration::from_secs(quota_interval_secs),
            idle_timeout,
            emit_legacy_aliases,
        }
    }

    pub fn resource(&self) -> Resource {
        build_resource(&self.service_name, &self.environment, &self.service_version)
    }

    pub fn traces_url(&self) -> String {
        format!("{}/v1/traces", self.otlp_endpoint.trim_end_matches('/'))
    }

    pub fn metrics_url(&self) -> String {
        format!("{}/v1/metrics", self.otlp_endpoint.trim_end_matches('/'))
    }
}
