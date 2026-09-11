/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use bytes::Bytes;
use opentelemetry_proto::tonic::collector::{
    metrics::v1::ExportMetricsServiceRequest, trace::v1::ExportTraceServiceRequest,
};
use prost::Message;
use reqwest::header::CONTENT_TYPE;
use std::fmt;
use std::time::Duration;

#[derive(Debug)]
pub enum ExportError {
    Transport(reqwest::Error),
    HttpStatus(reqwest::StatusCode),
    Encode(prost::EncodeError),
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExportError::Transport(e) => write!(f, "HTTP transport error: {e}"),
            ExportError::HttpStatus(s) => write!(f, "HTTP response error: status {s}"),
            ExportError::Encode(e) => write!(f, "Protobuf encode error: {e}"),
        }
    }
}

impl std::error::Error for ExportError {}

impl From<reqwest::Error> for ExportError {
    fn from(e: reqwest::Error) -> Self {
        ExportError::Transport(e)
    }
}

impl From<prost::EncodeError> for ExportError {
    fn from(e: prost::EncodeError) -> Self {
        ExportError::Encode(e)
    }
}

#[derive(Clone)]
pub struct OtlpExporter {
    client: reqwest::Client,
    traces_url: String,
    metrics_url: String,
}

impl OtlpExporter {
    pub fn new(traces_url: String, metrics_url: String) -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(5))
            .tcp_keepalive(Duration::from_secs(30))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(4)
            .user_agent(concat!("agent-otel-bridge/", env!("CARGO_PKG_VERSION")))
            .build()?;

        Ok(Self {
            client,
            traces_url,
            metrics_url,
        })
    }

    pub async fn export_traces(&self, req: ExportTraceServiceRequest) -> Result<(), ExportError> {
        let mut buf = Vec::with_capacity(req.encoded_len());
        req.encode(&mut buf)?;
        self.post_protobuf(&self.traces_url, Bytes::from(buf)).await
    }

    pub async fn export_metrics(
        &self,
        req: ExportMetricsServiceRequest,
    ) -> Result<(), ExportError> {
        let mut buf = Vec::with_capacity(req.encoded_len());
        req.encode(&mut buf)?;
        self.post_protobuf(&self.metrics_url, Bytes::from(buf))
            .await
    }

    async fn post_protobuf(&self, url: &str, body: Bytes) -> Result<(), ExportError> {
        const MAX_ATTEMPTS: u32 = 3;
        for attempt in 1..=MAX_ATTEMPTS {
            let res = self
                .client
                .post(url)
                .header(CONTENT_TYPE, "application/x-protobuf")
                .body(body.clone())
                .send()
                .await;

            match res {
                Ok(resp) if resp.status().is_success() => return Ok(()),
                Ok(resp) if resp.status().is_server_error() && attempt < MAX_ATTEMPTS => {
                    let backoff = Duration::from_millis(100 * 2u64.pow(attempt - 1));
                    tokio::time::sleep(backoff).await;
                }
                Ok(resp) => return Err(ExportError::HttpStatus(resp.status())),
                Err(e) if (e.is_timeout() || e.is_connect()) && attempt < MAX_ATTEMPTS => {
                    let backoff = Duration::from_millis(100 * 2u64.pow(attempt - 1));
                    tokio::time::sleep(backoff).await;
                }
                Err(e) => return Err(ExportError::Transport(e)),
            }
        }
        Ok(())
    }
}
