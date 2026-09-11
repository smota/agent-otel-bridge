/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_core::model::{AntigravityHookInput, HookEvent};
use agent_otel_core::otlp::{build_resource, build_span_from_hook};
use agent_otel_daemon::batch::SpanBatcher;
use agent_otel_daemon::config::DaemonConfig;
use agent_otel_daemon::quota::QuotaEngine;

#[test]
fn test_daemon_config_defaults() {
    let config = DaemonConfig::from_env();
    assert!(!config.otlp_endpoint.is_empty());
    assert!(!config.service_name.is_empty());
    assert_eq!(config.batch_size, 50);
    assert!(config.batch_timeout.as_millis() >= 100);
}

#[test]
fn test_span_batcher_capacity() {
    let res = build_resource("test-service", "test-env", "0.1.0");
    let mut batcher = SpanBatcher::new(res, 3);

    let input = AntigravityHookInput::default();
    let s1 = build_span_from_hook(HookEvent::PostInvocation, &input, 100, 200, 1);
    let s2 = build_span_from_hook(HookEvent::PostToolUse, &input, 200, 300, 2);
    let s3 = build_span_from_hook(HookEvent::Stop, &input, 300, 400, 3);

    assert!(!batcher.push(s1));
    assert_eq!(batcher.len(), 1);
    assert!(!batcher.push(s2));
    assert_eq!(batcher.len(), 2);
    assert!(batcher.push(s3)); // Reached batch size 3!
    assert_eq!(batcher.len(), 3);
}

#[test]
fn test_quota_engine_snapshot() {
    let engine = QuotaEngine::new();
    let snap = engine.snapshot();
    assert!(snap.remaining_fraction >= 0.0 && snap.remaining_fraction <= 1.0);
    assert!(snap.seconds_to_reset >= 0.0);
    assert_eq!(snap.bucket, "gemini-weekly");
    assert_eq!(snap.group, "gemini");
}
