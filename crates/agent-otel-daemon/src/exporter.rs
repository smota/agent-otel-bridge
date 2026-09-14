/* Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0 */
use bytes::Bytes;
use opentelemetry_proto::tonic::collector::{
    metrics::v1::{ExportMetricsServiceRequest, ExportMetricsServiceResponse},
    trace::v1::{ExportTraceServiceRequest, ExportTraceServiceResponse},
};
use prost::Message;
use reqwest::header::{CONTENT_TYPE, RETRY_AFTER};
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

#[derive(Clone)]
pub struct OtlpExporter {
    client: reqwest::Client,
    traces_url: String,
    metrics_url: String,
}
#[derive(Clone, Copy)]
enum Signal {
    Traces,
    Metrics,
}

impl OtlpExporter {
    pub fn new(traces_url: String, metrics_url: String) -> Result<Self, reqwest::Error> {
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(5))
                .tcp_keepalive(Duration::from_secs(30))
                .pool_idle_timeout(Duration::from_secs(90))
                .pool_max_idle_per_host(4)
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(concat!("agent-otel-bridge/", env!("CARGO_PKG_VERSION")))
                .build()?,
            traces_url,
            metrics_url,
        })
    }
    /// Compatibility API: partial rejection must not appear as success.
    pub async fn export_traces(&self, req: ExportTraceServiceRequest) -> Result<(), ExportError> {
        outcome_result(
            self.export_traces_until(&req, Instant::now() + EXPORT_DEADLINE)
                .await,
        )
    }
    pub async fn export_metrics(
        &self,
        req: ExportMetricsServiceRequest,
    ) -> Result<(), ExportError> {
        outcome_result(
            self.export_metrics_until(&req, Instant::now() + EXPORT_DEADLINE)
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
        self.export(req, &self.traces_url, Signal::Traces, count, deadline)
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
        self.export(req, &self.metrics_url, Signal::Metrics, count, deadline)
            .await
    }
    async fn export<M: Message>(
        &self,
        req: &M,
        url: &str,
        signal: Signal,
        count: usize,
        deadline: Instant,
    ) -> ExportReport {
        if req.encoded_len() > MAX_REQUEST_BYTES {
            return report(
                ExportOutcome::Rejected {
                    reason: "request_size",
                },
                0,
                false,
            );
        }
        let body = Bytes::from(req.encode_to_vec());
        let deadline = deadline.min(Instant::now() + EXPORT_DEADLINE);
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
            let response = tokio::time::timeout_at(
                deadline,
                self.client
                    .post(url)
                    .header(CONTENT_TYPE, "application/x-protobuf")
                    .body(body.clone())
                    .send(),
            )
            .await;
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
}
