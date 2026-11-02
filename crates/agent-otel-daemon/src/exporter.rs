/* Copyright agent-otel-bridge contributors
 * SPDX-License-Identifier: Apache-2.0 */
use crate::config::{DaemonConfig, SignalExportConfig};
use bytes::Bytes;
use opentelemetry_proto::tonic::collector::{
    metrics::v1::{ExportMetricsServiceRequest, ExportMetricsServiceResponse},
    trace::v1::{ExportTraceServiceRequest, ExportTraceServiceResponse},
};
use prost::Message;
use reqwest::header::{CONTENT_ENCODING, CONTENT_TYPE, RETRY_AFTER};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};
use tokio::time::Instant;

pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
pub const EXPORT_DEADLINE: Duration = Duration::from_secs(6);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportOutcome {
    Accepted,
    PartiallyAccepted { rejected: usize, warning: bool },
    Rejected { reason: &'static str },
    Unknown { reason: &'static str },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportReport {
    pub outcome: ExportOutcome,
    pub attempts: u32,
    pub may_duplicate: bool,
}
#[derive(Debug)]
pub enum ExportError {
    Transport(reqwest::Error),
    HttpStatus(reqwest::StatusCode),
    Encode(prost::EncodeError),
    Outcome(ExportOutcome),
}
impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(e) => write!(f, "HTTP transport error: {e}"),
            Self::HttpStatus(s) => write!(f, "HTTP status: {s}"),
            Self::Encode(e) => write!(f, "Protobuf encode error: {e}"),
            Self::Outcome(o) => write!(f, "OTLP outcome: {o:?}"),
        }
    }
}
impl std::error::Error for ExportError {}
impl From<reqwest::Error> for ExportError {
    fn from(e: reqwest::Error) -> Self {
        Self::Transport(e)
    }
}
impl From<prost::EncodeError> for ExportError {
    fn from(e: prost::EncodeError) -> Self {
        Self::Encode(e)
    }
}

#[derive(Debug)]
pub enum ExporterBuildError {
    Client(reqwest::Error),
    Certificate(String),
}
impl fmt::Display for ExporterBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Client(error) => write!(f, "OTLP client build failed: {error}"),
            Self::Certificate(error) => write!(f, "OTLP certificate error: {error}"),
        }
    }
}
impl std::error::Error for ExporterBuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Client(error) => Some(error),
            Self::Certificate(_) => None,
        }
    }
}

#[derive(Clone)]
pub struct OtlpExporter {
    client: reqwest::Client,
    traces: SignalExportConfig,
    metrics: SignalExportConfig,
    sdk_disabled: bool,
}
#[derive(Clone, Copy)]
enum Signal {
    Traces,
    Metrics,
}

impl OtlpExporter {
    pub fn new(traces_url: String, metrics_url: String) -> Result<Self, reqwest::Error> {
        Ok(Self {
            client: client_builder().build()?,
            traces: SignalExportConfig::http_protobuf(traces_url),
            metrics: SignalExportConfig::http_protobuf(metrics_url),
            sdk_disabled: false,
        })
    }
    pub fn from_config(config: &DaemonConfig) -> Result<Self, ExporterBuildError> {
        let client = install_certificates(
            client_builder(),
            config.traces.certificate.as_deref(),
            config.metrics.certificate.as_deref(),
        )?
        .build()
        .map_err(ExporterBuildError::Client)?;
        Ok(Self {
            client,
            traces: config.traces.clone(),
            metrics: config.metrics.clone(),
            sdk_disabled: config.sdk_disabled,
        })
    }
    pub fn traces_deadline(&self) -> Duration {
        self.traces.timeout.max(EXPORT_DEADLINE)
    }
    pub fn metrics_deadline(&self) -> Duration {
        self.metrics.timeout.max(EXPORT_DEADLINE)
    }
    /// Compatibility API: partial rejection must not appear as success.
    pub async fn export_traces(&self, req: ExportTraceServiceRequest) -> Result<(), ExportError> {
        outcome_result(
            self.export_traces_until(&req, Instant::now() + self.traces_deadline())
                .await,
        )
    }
    pub async fn export_metrics(
        &self,
        req: ExportMetricsServiceRequest,
    ) -> Result<(), ExportError> {
        outcome_result(
            self.export_metrics_until(&req, Instant::now() + self.metrics_deadline())
                .await,
        )
    }
    pub async fn export_traces_until(
        &self,
        req: &ExportTraceServiceRequest,
        deadline: Instant,
    ) -> ExportReport {
        let count = req
            .resource_spans
            .iter()
            .flat_map(|r| &r.scope_spans)
            .map(|s| s.spans.len())
            .sum();
        self.export(req, &self.traces, Signal::Traces, count, deadline)
            .await
    }
    pub async fn export_metrics_until(
        &self,
        req: &ExportMetricsServiceRequest,
        deadline: Instant,
    ) -> ExportReport {
        use opentelemetry_proto::tonic::metrics::v1::metric::Data;
        let count = req
            .resource_metrics
            .iter()
            .flat_map(|r| &r.scope_metrics)
            .flat_map(|s| &s.metrics)
            .map(|m| match m.data.as_ref() {
                Some(Data::Gauge(x)) => x.data_points.len(),
                Some(Data::Sum(x)) => x.data_points.len(),
                Some(Data::Histogram(x)) => x.data_points.len(),
                Some(Data::ExponentialHistogram(x)) => x.data_points.len(),
                Some(Data::Summary(x)) => x.data_points.len(),
                None => 0,
            })
            .sum();
        self.export(req, &self.metrics, Signal::Metrics, count, deadline)
            .await
    }
    async fn export<M: Message>(
        &self,
        req: &M,
        target: &SignalExportConfig,
        signal: Signal,
        count: usize,
        deadline: Instant,
    ) -> ExportReport {
        if self.sdk_disabled {
            return report(ExportOutcome::Accepted, 0, false);
        }
        if let Some(reason) = target.disabled_reason {
            return report(ExportOutcome::Rejected { reason }, 0, false);
        }
        if req.encoded_len() > MAX_REQUEST_BYTES {
            return report(
                ExportOutcome::Rejected {
                    reason: "request_size",
                },
                0,
                false,
            );
        }
        let encoded = req.encode_to_vec();
        let body = if target.gzip {
            match gzip_encode(&encoded) {
                Ok(compressed) => Bytes::from(compressed),
                Err(_) => {
                    return report(ExportOutcome::Rejected { reason: "encode" }, 0, false);
                }
            }
        } else {
            Bytes::from(encoded)
        };
        let deadline = deadline.min(Instant::now() + target.timeout.max(EXPORT_DEADLINE));
        let mut uncertain = false;
        let mut last = ExportOutcome::Rejected {
            reason: "deadline_before_send",
        };
        let mut attempts = 0;
        for attempt in 1..=3 {
            if Instant::now() >= deadline {
                break;
            }
            attempts = attempt;
            let response =
                tokio::time::timeout_at(deadline, self.request(target, body.clone()).send()).await;
            let retry_after = match response {
                Ok(Ok(mut response)) => {
                    let status = response.status();
                    if status == reqwest::StatusCode::OK {
                        let ct = response
                            .headers()
                            .get(CONTENT_TYPE)
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("");
                        if ct.split(';').next().unwrap_or("").trim() != "application/x-protobuf" {
                            return report(
                                ExportOutcome::Unknown {
                                    reason: "content_type",
                                },
                                attempts,
                                uncertain,
                            );
                        }
                        let read = async {
                            let mut data = Vec::new();
                            while let Some(chunk) =
                                response.chunk().await.map_err(|_| "response_transport")?
                            {
                                if chunk.len() > MAX_RESPONSE_BYTES.saturating_sub(data.len()) {
                                    return Err("response_size");
                                }
                                data.extend_from_slice(&chunk);
                            }
                            Ok(data)
                        };
                        let outcome = match tokio::time::timeout_at(deadline, read).await {
                            Ok(Ok(data)) => decode_response(signal, &data, count),
                            Ok(Err(reason)) => ExportOutcome::Unknown { reason },
                            Err(_) => ExportOutcome::Unknown {
                                reason: "response_deadline",
                            },
                        };
                        return report(outcome, attempts, uncertain);
                    }
                    last = if status.is_success() {
                        ExportOutcome::Unknown {
                            reason: "unexpected_success_status",
                        }
                    } else {
                        ExportOutcome::Rejected {
                            reason: "http_status",
                        }
                    };
                    if !retryable_status(status.as_u16()) {
                        return report(
                            if uncertain {
                                ExportOutcome::Unknown {
                                    reason: "earlier_attempt_unknown",
                                }
                            } else {
                                last
                            },
                            attempts,
                            uncertain,
                        );
                    }
                    response
                        .headers()
                        .get(RETRY_AFTER)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| retry_after_duration(s, SystemTime::now()))
                }
                Ok(Err(e)) if e.is_connect() => {
                    last = ExportOutcome::Rejected { reason: "connect" };
                    None
                }
                Ok(Err(_)) => {
                    uncertain = true;
                    last = ExportOutcome::Unknown {
                        reason: "transport",
                    };
                    None
                }
                Err(_) => {
                    return report(
                        ExportOutcome::Unknown {
                            reason: "request_deadline",
                        },
                        attempts,
                        true,
                    )
                }
            };
            if attempt == 3 {
                break;
            }
            let delay =
                retry_after.unwrap_or_else(|| jitter(Duration::from_millis(100 << (attempt - 1))));
            if delay >= deadline.saturating_duration_since(Instant::now()) {
                break;
            }
            tokio::time::sleep(delay).await;
        }
        if uncertain && matches!(last, ExportOutcome::Rejected { .. }) {
            last = ExportOutcome::Unknown {
                reason: "earlier_attempt_unknown",
            };
        }
        report(last, attempts, uncertain)
    }

    fn request(&self, target: &SignalExportConfig, body: Bytes) -> reqwest::RequestBuilder {
        let mut builder = self.client.post(&target.url).timeout(target.timeout);
        for (name, value) in &target.headers {
            if name.eq_ignore_ascii_case("content-type")
                || name.eq_ignore_ascii_case("content-encoding")
            {
                continue;
            }
            builder = builder.header(name, value);
        }
        builder = builder.header(CONTENT_TYPE, "application/x-protobuf");
        if target.gzip {
            builder = builder.header(CONTENT_ENCODING, "gzip");
        }
        builder.body(body)
    }
}
fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(5))
        .tcp_keepalive(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(4)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("agent-otel-bridge/", env!("CARGO_PKG_VERSION")))
}
fn install_certificates(
    mut builder: reqwest::ClientBuilder,
    traces: Option<&str>,
    metrics: Option<&str>,
) -> Result<reqwest::ClientBuilder, ExporterBuildError> {
    let mut seen = std::collections::HashSet::new();
    for path in [traces, metrics].into_iter().flatten() {
        if !seen.insert(path.to_string()) {
            continue;
        }
        let bytes = std::fs::read(path).map_err(|error| {
            ExporterBuildError::Certificate(format!("failed to read {path}: {error}"))
        })?;
        for certificate in parse_pem_certificates(&bytes).map_err(|error| {
            ExporterBuildError::Certificate(format!("invalid certificate {path}: {error}"))
        })? {
            builder = builder.add_root_certificate(certificate);
        }
    }
    Ok(builder)
}
fn parse_pem_certificates(bytes: &[u8]) -> Result<Vec<reqwest::Certificate>, String> {
    let text = String::from_utf8_lossy(bytes);
    let marker = "-----END CERTIFICATE-----";
    let mut rest = text.as_ref();
    let mut certificates = Vec::new();
    while let Some(start) = rest.find("-----BEGIN CERTIFICATE-----") {
        let after = &rest[start..];
        let Some(end) = after.find(marker) else {
            return Err("truncated PEM certificate".to_string());
        };
        let pem = &after[..end + marker.len()];
        let certificate =
            reqwest::Certificate::from_pem(pem.as_bytes()).map_err(|error| error.to_string())?;
        certificates.push(certificate);
        rest = &after[end + marker.len()..];
    }
    if certificates.is_empty() {
        return Err("no PEM certificates found".to_string());
    }
    Ok(certificates)
}
fn gzip_encode(raw: &[u8]) -> std::io::Result<Vec<u8>> {
    use flate2::write::GzEncoder;
    use std::io::Write;
    let mut encoder = GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(raw)?;
    encoder.finish()
}
fn report(outcome: ExportOutcome, attempts: u32, may_duplicate: bool) -> ExportReport {
    ExportReport {
        outcome,
        attempts,
        may_duplicate,
    }
}
fn outcome_result(report: ExportReport) -> Result<(), ExportError> {
    match report.outcome {
        ExportOutcome::Accepted | ExportOutcome::PartiallyAccepted { rejected: 0, .. } => Ok(()),
        other => Err(ExportError::Outcome(other)),
    }
}
pub fn retryable_status(status: u16) -> bool {
    matches!(status, 429 | 502 | 503 | 504)
}
pub fn retry_after_duration(value: &str, now: SystemTime) -> Option<Duration> {
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    httpdate::parse_http_date(value)
        .ok()
        .map(|date| date.duration_since(now).unwrap_or_default())
}
fn jitter(max: Duration) -> Duration {
    static SEQUENCE: AtomicU64 = AtomicU64::new(1);
    let time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let mut x = time ^ SEQUENCE.fetch_add(0x9e3779b97f4a7c15, Ordering::Relaxed);
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58476d1ce4e5b9);
    x ^= x >> 27;
    Duration::from_nanos(x % (max.as_nanos() as u64 + 1))
}
fn decode_response(signal: Signal, bytes: &[u8], sent: usize) -> ExportOutcome {
    let partial = match signal {
        Signal::Traces => ExportTraceServiceResponse::decode(bytes).map(|r| {
            r.partial_success
                .map(|p| (p.rejected_spans, !p.error_message.is_empty()))
        }),
        Signal::Metrics => ExportMetricsServiceResponse::decode(bytes).map(|r| {
            r.partial_success
                .map(|p| (p.rejected_data_points, !p.error_message.is_empty()))
        }),
    };
    match partial {
        Ok(None) => ExportOutcome::Accepted,
        Ok(Some((rejected, warning))) if rejected >= 0 && rejected as u64 <= sent as u64 => {
            ExportOutcome::PartiallyAccepted {
                rejected: rejected as usize,
                warning,
            }
        }
        _ => ExportOutcome::Unknown { reason: "protocol" },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_proto::tonic::collector::trace::v1::ExportTracePartialSuccess;
    use prost::Message;
    #[test]
    fn partial_is_never_full_success() {
        let body = ExportTraceServiceResponse {
            partial_success: Some(ExportTracePartialSuccess {
                rejected_spans: 2,
                error_message: "warning".into(),
            }),
        }
        .encode_to_vec();
        assert_eq!(
            decode_response(Signal::Traces, &body, 3),
            ExportOutcome::PartiallyAccepted {
                rejected: 2,
                warning: true
            }
        );
        assert!(matches!(
            decode_response(Signal::Traces, &body, 1),
            ExportOutcome::Unknown { .. }
        ));
        assert_eq!(
            decode_response(Signal::Traces, &[], 3),
            ExportOutcome::Accepted
        );
        assert!(matches!(
            decode_response(Signal::Traces, &[0xff], 3),
            ExportOutcome::Unknown { .. }
        ));
    }
    #[test]
    fn retry_policy_uses_otlp_statuses_and_dates() {
        for status in [429, 502, 503, 504] {
            assert!(retryable_status(status));
        }
        for status in [200, 400, 401, 403, 500] {
            assert!(!retryable_status(status));
        }
        assert_eq!(
            retry_after_duration("5", SystemTime::UNIX_EPOCH),
            Some(Duration::from_secs(5))
        );
        assert_eq!(
            retry_after_duration("Thu, 01 Jan 1970 00:00:10 GMT", SystemTime::UNIX_EPOCH),
            Some(Duration::from_secs(10))
        );
        assert_eq!(
            retry_after_duration("invalid", SystemTime::UNIX_EPOCH),
            None
        );
    }

    fn vars<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| {
            pairs
                .iter()
                .rev()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_string())
        }
    }

    fn span_request() -> ExportTraceServiceRequest {
        use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span};
        ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: None,
                scope_spans: vec![ScopeSpans {
                    scope: None,
                    spans: vec![Span::default()],
                    ..Default::default()
                }],
                schema_url: String::new(),
            }],
        }
    }

    async fn read_http(socket: &mut tokio::net::TcpStream) -> (String, Vec<u8>) {
        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        let mut tmp = [0_u8; 2048];
        loop {
            let n = tokio::time::timeout(Duration::from_secs(1), socket.read(&mut tmp))
                .await
                .expect("request header timeout")
                .expect("request read");
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if let Some(split) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&buf[..split]).to_lowercase();
                let mut body = buf[split + 4..].to_vec();
                let length = header
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(body.len());
                while body.len() < length {
                    let n = socket.read(&mut tmp).await.expect("body read");
                    if n == 0 {
                        break;
                    }
                    body.extend_from_slice(&tmp[..n]);
                }
                body.truncate(length);
                return (header, body);
            }
        }
        (String::from_utf8_lossy(&buf).to_lowercase(), Vec::new())
    }

    #[tokio::test]
    async fn gzip_sets_content_encoding_and_compresses_body() {
        use flate2::read::GzDecoder;
        use std::io::Read;
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let (header, body) = read_http(&mut socket).await;
            let response = b"HTTP/1.1 200 OK\r\nContent-Type: application/x-protobuf\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = socket.write_all(response).await;
            (header, body)
        });
        let config = crate::config::DaemonConfig::from_vars(&vars(&[
            (
                "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
                &format!("http://{address}/v1/traces"),
            ),
            (
                "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
                &format!("http://{address}/v1/metrics"),
            ),
            ("OTEL_EXPORTER_OTLP_TRACES_COMPRESSION", "gzip"),
            ("OTEL_EXPORTER_OTLP_TRACES_HEADERS", "X-Test=hello%20world"),
            (
                "OTEL_EXPORTER_OTLP_HEADERS",
                "Content-Type=application/json",
            ),
        ]));
        let exporter = OtlpExporter::from_config(&config).unwrap();
        let report = exporter
            .export_traces_until(&span_request(), Instant::now() + Duration::from_secs(2))
            .await;
        let (header, body) = server.await.unwrap();
        assert_eq!(report.outcome, ExportOutcome::Accepted);
        assert!(header.contains("content-encoding: gzip"));
        assert!(header.contains("content-type: application/x-protobuf"));
        assert!(header.contains("x-test: hello world"));
        assert!(!header.contains("application/json"));
        assert_eq!(&body[..2], &[0x1f, 0x8b]);
        let mut decoded = Vec::new();
        GzDecoder::new(body.as_slice())
            .read_to_end(&mut decoded)
            .unwrap();
        let parsed = ExportTraceServiceRequest::decode(decoded.as_slice()).unwrap();
        assert_eq!(parsed.resource_spans[0].scope_spans[0].spans.len(), 1);
    }

    #[tokio::test]
    async fn sdk_disabled_performs_zero_http_attempts() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&hits);
        let server = tokio::spawn(async move {
            loop {
                if let Ok((socket, _)) = listener.accept().await {
                    seen.fetch_add(1, Ordering::SeqCst);
                    drop(socket);
                }
            }
        });
        let config = crate::config::DaemonConfig::from_vars(&vars(&[
            ("OTEL_SDK_DISABLED", "TRUE"),
            ("OTEL_EXPORTER_OTLP_ENDPOINT", &format!("http://{address}")),
        ]));
        let exporter = OtlpExporter::from_config(&config).unwrap();
        let report = exporter
            .export_traces_until(&span_request(), Instant::now() + Duration::from_secs(2))
            .await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        server.abort();
        assert_eq!(report.outcome, ExportOutcome::Accepted);
        assert_eq!(report.attempts, 0);
        assert!(!report.may_duplicate);
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn unsupported_protocol_does_not_connect() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&hits);
        let server = tokio::spawn(async move {
            loop {
                if let Ok((socket, _)) = listener.accept().await {
                    seen.fetch_add(1, Ordering::SeqCst);
                    drop(socket);
                }
            }
        });
        let config = crate::config::DaemonConfig::from_vars(&vars(&[
            ("OTEL_EXPORTER_OTLP_TRACES_PROTOCOL", "grpc"),
            ("OTEL_EXPORTER_OTLP_ENDPOINT", &format!("http://{address}")),
        ]));
        let exporter = OtlpExporter::from_config(&config).unwrap();
        let report = exporter
            .export_traces_until(&span_request(), Instant::now() + Duration::from_secs(2))
            .await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        server.abort();
        assert_eq!(
            report.outcome,
            ExportOutcome::Rejected {
                reason: "unsupported_protocol"
            }
        );
        assert_eq!(report.attempts, 0);
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        assert!(exporter.metrics.disabled_reason.is_none());
    }

    #[test]
    fn invalid_certificate_path_fails_construction() {
        let config = crate::config::DaemonConfig::from_vars(&vars(&[(
            "OTEL_EXPORTER_OTLP_CERTIFICATE",
            "/no/such/agent-otel-bridge-root.pem",
        )]));
        match OtlpExporter::from_config(&config) {
            Err(ExporterBuildError::Certificate(_)) => {}
            Err(error) => panic!("expected certificate error, got {error}"),
            Ok(_) => panic!("expected certificate error"),
        }
    }
}
