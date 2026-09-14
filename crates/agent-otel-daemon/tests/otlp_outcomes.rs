use agent_otel_daemon::exporter::{ExportOutcome, OtlpExporter};
use opentelemetry_proto::tonic::collector::trace::v1::{
    ExportTracePartialSuccess, ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span};
use prost::Message;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{timeout, Instant};

fn request_with_span() -> ExportTraceServiceRequest {
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

fn response(partial: Option<ExportTracePartialSuccess>) -> Vec<u8> {
    ExportTraceServiceResponse {
        partial_success: partial,
    }
    .encode_to_vec()
}

async fn read_request(socket: &mut tokio::net::TcpStream) -> std::io::Result<()> {
    let mut buf = [0_u8; 8192];
    let _ = timeout(Duration::from_secs(1), socket.read(&mut buf)).await??;
    Ok(())
}

async fn write_response(
    socket: &mut tokio::net::TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let head = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    socket.write_all(head.as_bytes()).await?;
    socket.write_all(body).await
}

async fn run_status(
    statuses: Vec<&'static str>,
    body: Vec<u8>,
    content_type: &'static str,
) -> (agent_otel_daemon::exporter::ExportReport, usize) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&count);
    let server = tokio::spawn(async move {
        for status in statuses {
            let (mut socket, _) = listener.accept().await.unwrap();
            seen.fetch_add(1, Ordering::SeqCst);
            read_request(&mut socket).await.unwrap();
            let _ = write_response(&mut socket, status, content_type, &body).await;
        }
    });
    let exp = OtlpExporter::new(
        format!("http://{address}/v1/traces"),
        format!("http://{address}/v1/metrics"),
    )
    .unwrap();
    let report = timeout(
        Duration::from_secs(3),
        exp.export_traces_until(
            &request_with_span(),
            Instant::now() + Duration::from_secs(2),
        ),
    )
    .await
    .unwrap();
    server.abort();
    let _ = server.await;
    (report, count.load(Ordering::SeqCst))
}

#[tokio::test]
async fn http_500_is_rejected_with_exactly_one_request() {
    let (report, requests) = run_status(
        vec!["500 Internal Server Error"],
        vec![],
        "application/x-protobuf",
    )
    .await;
    assert_eq!(requests, 1);
    assert_eq!(report.attempts, 1);
    assert_eq!(
        report.outcome,
        ExportOutcome::Rejected {
            reason: "http_status"
        }
    );
}

#[tokio::test]
async fn http_503_then_200_retries_and_accepts() {
    let (report, requests) = run_status(
        vec!["503 Service Unavailable", "200 OK"],
        response(None),
        "application/x-protobuf",
    )
    .await;
    assert_eq!(requests, 2);
    assert_eq!(report.attempts, 2);
    assert_eq!(report.outcome, ExportOutcome::Accepted);
}

#[tokio::test]
async fn http_200_partial_success_is_reported_with_rejected_count() {
    let body = response(Some(ExportTracePartialSuccess {
        rejected_spans: 1,
        error_message: "one rejected".into(),
    }));
    let (report, requests) = run_status(vec!["200 OK"], body, "application/x-protobuf").await;
    assert_eq!(requests, 1);
    assert_eq!(
        report.outcome,
        ExportOutcome::PartiallyAccepted {
            rejected: 1,
            warning: true
        }
    );
}

#[tokio::test]
async fn malformed_or_wrong_content_type_200_is_unknown_without_retry() {
    let (malformed, malformed_requests) =
        run_status(vec!["200 OK"], vec![0xff], "application/x-protobuf").await;
    assert_eq!(malformed_requests, 1);
    assert_eq!(
        malformed.outcome,
        ExportOutcome::Unknown { reason: "protocol" }
    );
    let (wrong_type, wrong_requests) =
        run_status(vec!["200 OK"], response(None), "application/json").await;
    assert_eq!(wrong_requests, 1);
    assert_eq!(
        wrong_type.outcome,
        ExportOutcome::Unknown {
            reason: "content_type"
        }
    );
}

#[tokio::test]
async fn oversized_response_is_unknown() {
    let body = vec![0_u8; 4 * 1024 * 1024 + 1];
    let (report, requests) = run_status(vec!["200 OK"], body, "application/x-protobuf").await;
    assert_eq!(requests, 1);
    assert_eq!(
        report.outcome,
        ExportOutcome::Unknown {
            reason: "response_size"
        }
    );
}

#[tokio::test]
async fn stalled_server_hits_150ms_deadline_as_unknown() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let _ = read_request(&mut socket).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
    });
    let exp = OtlpExporter::new(
        format!("http://{address}/v1/traces"),
        format!("http://{address}/v1/metrics"),
    )
    .unwrap();
    let started = Instant::now();
    let report = timeout(
        Duration::from_secs(1),
        exp.export_traces_until(&request_with_span(), started + Duration::from_millis(150)),
    )
    .await
    .unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(matches!(report.outcome, ExportOutcome::Unknown { .. }));
    assert!(report.attempts <= 1);
    server.abort();
    let _ = server.await;
}
