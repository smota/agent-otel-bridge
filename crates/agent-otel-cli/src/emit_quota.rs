/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_core::quota::build_quota_metrics_request_opts;
use agent_otel_daemon::config::DaemonConfig;
use agent_otel_daemon::exporter::OtlpExporter;
use agent_otel_daemon::quota::QuotaEngine;
use agent_otel_ipc::client::try_send;
use agent_otel_ipc::frame::MsgType;

pub async fn run(ping: bool) -> Result<(), Box<dyn std::error::Error>> {
    if ping {
        println!("[emit-quota] Sending QuotaPing to running daemon via IPC...");
        match try_send(MsgType::QuotaPing, b"") {
            Ok(()) => {
                println!("[emit-quota] QuotaPing successfully delivered to daemon.");
                return Ok(());
            }
            Err(()) => {
                println!(
                    "[emit-quota] Daemon is not running on pipe. Falling back to direct export..."
                );
            }
        }
    }

    println!("[emit-quota] Emitting quota metrics directly to OTLP collector...");
    let config = DaemonConfig::from_env();
    let engine = QuotaEngine::new();
    let snapshot = engine.snapshot();

    println!(
        "  [quota] remaining_fraction: {:.2}, seconds_to_reset: {:.0}s, bucket: {}, group: {}",
        snapshot.remaining_fraction, snapshot.seconds_to_reset, snapshot.bucket, snapshot.group
    );

    let exporter = OtlpExporter::new(config.traces_url(), config.metrics_url())?;
    let request = build_quota_metrics_request_opts(config.resource(), &snapshot, config.emit_legacy_aliases);

    exporter.export_metrics(request).await?;
    println!(
        "  [ok] Quota metrics successfully exported to {}",
        config.metrics_url()
    );

    Ok(())
}
