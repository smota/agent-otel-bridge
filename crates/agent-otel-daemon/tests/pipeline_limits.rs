use agent_otel_core::model::{ClientKind, HookEvent, WireHeader};
use agent_otel_daemon::diagnostics::Diagnostics;
use agent_otel_daemon::{Daemon, DaemonConfig};
use agent_otel_ipc::frame::{encode_frame, MsgType};
use agent_otel_ipc::server::IngressStats;
use opentelemetry_proto::tonic::collector::{
    metrics::v1::ExportMetricsServiceRequest, trace::v1::ExportTraceServiceRequest,
};
use prost::Message;
use std::collections::HashSet;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

const EVENT_COUNT: u64 = 24;

#[derive(Default)]
struct CollectorStats {
    trace_requests: AtomicU64,
    metric_requests: AtomicU64,
    parsed_spans: AtomicU64,
    parse_failures: AtomicU64,
    transport_aborts: AtomicU64,
    unique_spans: Mutex<HashSet<(Vec<u8>, Vec<u8>)>>,
}

struct TestCollector {
    endpoint: String,
    stats: Arc<CollectorStats>,
    release_traces: CancellationToken,
    shutdown: CancellationToken,
    handle: tokio::task::JoinHandle<io::Result<()>>,
}

impl TestCollector {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind owned test collector");
        let endpoint = format!(
            "http://{}",
            listener.local_addr().expect("collector address")
        );
        let stats = Arc::new(CollectorStats::default());
        let release_traces = CancellationToken::new();
        let shutdown = CancellationToken::new();
        let handle = tokio::spawn(run_collector(
            listener,
            Arc::clone(&stats),
            release_traces.clone(),
            shutdown.clone(),
        ));
        Self {
            endpoint,
            stats,
            release_traces,
            shutdown,
            handle,
        }
    }

    async fn wait_for_trace(&self) {
        wait_for_counter(&self.stats.trace_requests, 1, Duration::from_secs(1)).await;
    }

    async fn wait_for_metric(&self) {
        wait_for_counter(&self.stats.metric_requests, 1, Duration::from_secs(1)).await;
    }

    async fn stop(self) {
        self.release_traces.cancel();
        self.shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(1), self.handle)
            .await
            .expect("collector shutdown exceeded bound")
            .expect("collector task panicked")
            .expect("collector failed");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn blocked_exporter_does_not_block_transform_and_shutdown_reconciles() {
    let collector = TestCollector::start().await;
    let endpoint = unique_ipc_endpoint();
    let shutdown = CancellationToken::new();
    let diagnostics = Arc::new(Diagnostics::default());
    let ingress = Arc::new(IngressStats::default());
    let config = DaemonConfig {
        otlp_endpoint: collector.endpoint.clone(),
        service_name: "agent-otel-pipeline-test".to_string(),
        environment: "isolated-test".to_string(),
        service_version: "test".to_string(),
        pipe_name: endpoint.clone(),
        batch_size: 1,
        batch_timeout: Duration::from_millis(200),
        quota_interval: Duration::from_secs(60),
        idle_timeout: Duration::MAX,
        emit_legacy_aliases: false,
    };
    let daemon = Daemon::new(config, shutdown.clone()).expect("construct isolated daemon");
    let daemon_diagnostics = Arc::clone(&diagnostics);
    let daemon_ingress = Arc::clone(&ingress);
    let daemon_handle = tokio::spawn(async move {
        daemon
            .run_with_diagnostics(daemon_diagnostics, daemon_ingress)
            .await
    });

    let workspace = unique_test_workspace();
    let frame = valid_one_kib_event(&workspace.to_string_lossy());
    let mut producers = JoinSet::new();
    for _ in 0..4 {
        let producer_endpoint = endpoint.clone();
        let producer_frame = frame.clone();
        producers.spawn(async move {
            for _ in 0..6 {
                send_native(&producer_endpoint, &producer_frame).await?;
            }
            io::Result::Ok(())
        });
    }
    while let Some(result) = producers.join_next().await {
        result.expect("producer task panicked").expect("send event");
    }

    wait_until(Duration::from_secs(1), || {
        diagnostics.transformed.load(Ordering::Relaxed) == EVENT_COUNT
    })
    .await;
    collector.wait_for_trace().await;

    // Control traffic and metric export remain independent of the blocked trace request.
    send_native(&endpoint, &encode_frame(MsgType::QuotaPing, &[]))
        .await
        .expect("send quota ping");
    collector.wait_for_metric().await;

    let mut malformed_payload = WireHeader::new(
        ClientKind::Codex.to_wire(),
        HookEvent::PostToolUse.to_wire(),
    )
    .encode()
    .to_vec();
    malformed_payload.extend_from_slice(br#"{"unterminated":"#);
    send_native(
        &endpoint,
        &encode_frame(MsgType::HookPayload, &malformed_payload),
    )
    .await
    .expect("send malformed event");
    wait_until(Duration::from_secs(1), || {
        diagnostics.invalid.load(Ordering::Relaxed) == 1
    })
    .await;
    assert_eq!(diagnostics.transformed.load(Ordering::Relaxed), EVENT_COUNT);

    let shutdown_started = tokio::time::Instant::now();
    shutdown.cancel();
    let daemon_result = tokio::time::timeout(Duration::from_millis(5_500), daemon_handle)
        .await
        .expect("daemon exceeded five-second shutdown contract")
        .expect("daemon task panicked");
    daemon_result.expect("daemon returned an IPC error");
    assert!(shutdown_started.elapsed() <= Duration::from_millis(5_500));

    let snapshot = diagnostics.snapshot();
    assert_eq!(ingress.admitted.load(Ordering::Relaxed), EVENT_COUNT + 1);
    assert_eq!(snapshot.transformed + snapshot.invalid, EVENT_COUNT + 1);
    assert_eq!(snapshot.export_queued, EVENT_COUNT);
    assert_eq!(
        snapshot.accepted + snapshot.rejected + snapshot.unknown + snapshot.shutdown_dropped,
        snapshot.export_queued
    );
    assert_eq!(snapshot.accepted, 0);
    assert_eq!(snapshot.rejected, 0);
    // The request deadline starts before the independent drain deadline. A
    // timed-out first request may therefore let another queued batch start,
    // so the exact unknown count is scheduler-dependent while total accounting
    // must remain exact.
    let parsed_spans = collector.stats.parsed_spans.load(Ordering::Relaxed);
    let unique_received = collector
        .stats
        .unique_spans
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .len() as u64;
    assert!((1..=EVENT_COUNT).contains(&snapshot.unknown));
    assert!((1..=snapshot.unknown).contains(&unique_received));
    assert_eq!(snapshot.unknown + snapshot.shutdown_dropped, EVENT_COUNT);
    assert_eq!(snapshot.queued_bytes, 0);
    assert_eq!(snapshot.queued_items, 0);
    assert_eq!(ingress.reserved_bytes.load(Ordering::Relaxed), 0);
    assert_eq!(ingress.active_connections.load(Ordering::Relaxed), 0);
    assert!(collector.stats.metric_requests.load(Ordering::Relaxed) >= 1);
    assert!(parsed_spans >= 1);
    assert_eq!(collector.stats.parse_failures.load(Ordering::Relaxed), 0);

    collector.stop().await;
    std::fs::remove_dir_all(workspace).expect("remove owned test workspace");
}

fn valid_one_kib_event(workspace: &str) -> Vec<u8> {
    let json = serde_json::to_vec(&serde_json::json!({
        "conversationId": "bounded-pipeline",
        "toolName": "test_tool",
        "workspacePath": workspace,
        "success": true
    }))
    .expect("serialize test event");
    assert!(json.len() <= 1024);
    let mut padded = json;
    padded.resize(1024, b' ');
    let mut payload = WireHeader::new(
        ClientKind::Codex.to_wire(),
        HookEvent::PostToolUse.to_wire(),
    )
    .encode()
    .to_vec();
    payload.extend_from_slice(&padded);
    encode_frame(MsgType::HookPayload, &payload)
}

async fn wait_until(timeout: Duration, predicate: impl Fn() -> bool) {
    let started = tokio::time::Instant::now();
    loop {
        if predicate() {
            return;
        }
        assert!(
            started.elapsed() < timeout,
            "condition missed its functional deadline"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn wait_for_counter(counter: &AtomicU64, minimum: u64, timeout: Duration) {
    tokio::time::timeout(timeout, async {
        loop {
            if counter.load(Ordering::Relaxed) >= minimum {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("collector did not observe expected request");
}

async fn run_collector(
    listener: TcpListener,
    stats: Arc<CollectorStats>,
    release_traces: CancellationToken,
    shutdown: CancellationToken,
) -> io::Result<()> {
    let permits = Arc::new(Semaphore::new(4));
    let mut handlers = JoinSet::new();
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            joined = handlers.join_next(), if !handlers.is_empty() => {
                if let Some(result) = joined {
                    result.map_err(io::Error::other)??;
                }
            }
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let Ok(permit) = Arc::clone(&permits).try_acquire_owned() else {
                    drop(stream);
                    continue;
                };
                let handler_stats = Arc::clone(&stats);
                let handler_release = release_traces.clone();
                handlers.spawn(async move {
                    let _permit = permit;
                    handle_collector_request(stream, handler_stats, handler_release).await
                });
            }
        }
    }
    release_traces.cancel();
    while let Some(result) = handlers.join_next().await {
        result.map_err(io::Error::other)??;
    }
    Ok(())
}

async fn handle_collector_request(
    mut stream: TcpStream,
    stats: Arc<CollectorStats>,
    release_traces: CancellationToken,
) -> io::Result<()> {
    let (path, body) = match read_http_request(&mut stream).await {
        Ok(request) => request,
        Err(error) if is_peer_transport_abort(&error) => {
            stats.transport_aborts.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    match path.as_str() {
        "/v1/traces" => {
            match ExportTraceServiceRequest::decode(body.as_slice()) {
                Ok(request) => {
                    let mut unique_spans = stats
                        .unique_spans
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    for span in request
                        .resource_spans
                        .iter()
                        .flat_map(|resource| &resource.scope_spans)
                        .flat_map(|scope| &scope.spans)
                    {
                        unique_spans.insert((span.trace_id.clone(), span.span_id.clone()));
                    }
                    let spans = request
                        .resource_spans
                        .iter()
                        .flat_map(|resource| &resource.scope_spans)
                        .map(|scope| scope.spans.len() as u64)
                        .sum::<u64>();
                    stats.parsed_spans.fetch_add(spans, Ordering::Relaxed);
                }
                Err(_) => {
                    stats.parse_failures.fetch_add(1, Ordering::Relaxed);
                }
            }
            stats.trace_requests.fetch_add(1, Ordering::Relaxed);
            release_traces.cancelled().await;
        }
        "/v1/metrics" => {
            if ExportMetricsServiceRequest::decode(body.as_slice()).is_err() {
                stats.parse_failures.fetch_add(1, Ordering::Relaxed);
            }
            stats.metric_requests.fetch_add(1, Ordering::Relaxed);
        }
        _ => {
            stats.parse_failures.fetch_add(1, Ordering::Relaxed);
        }
    }
    match stream
        .write_all(
            b"HTTP/1.1 200 OK\r\ncontent-type: application/x-protobuf\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
        )
        .await
    {
        Ok(()) => Ok(()),
        Err(error) if is_peer_transport_abort(&error) => {
            stats.transport_aborts.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn is_peer_transport_abort(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::UnexpectedEof
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::BrokenPipe
    )
}

async fn read_http_request(stream: &mut TcpStream) -> io::Result<(String, Vec<u8>)> {
    const HEADER_LIMIT: usize = 32 * 1024;
    const BODY_LIMIT: usize = 2 * 1024 * 1024;
    let mut bytes = Vec::new();
    let header_end = loop {
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
        if bytes.len() >= HEADER_LIMIT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP header too large",
            ));
        }
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "truncated HTTP header",
            ));
        }
        bytes.extend_from_slice(&chunk[..read]);
    };
    let header = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "non-UTF8 HTTP header"))?;
    let request_line = header
        .lines()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request line"))?;
    let path = request_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request path"))?
        .to_string();
    let content_length = header
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing content length"))?;
    if content_length > BODY_LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "HTTP body too large",
        ));
    }
    let required = header_end + content_length;
    while bytes.len() < required {
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "truncated HTTP body",
            ));
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    Ok((path, bytes[header_end..required].to_vec()))
}

fn unique_ipc_endpoint() -> String {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    #[cfg(windows)]
    {
        format!(
            r"\\.\pipe\agent-otel-pipeline-{}-{nonce}",
            std::process::id()
        )
    }
    #[cfg(unix)]
    {
        std::env::temp_dir()
            .join(format!(
                "agent-otel-pipeline-{}-{nonce}.sock",
                std::process::id()
            ))
            .to_string_lossy()
            .to_string()
    }
}

fn unique_test_workspace() -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "agent-otel-pipeline-workspace-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).expect("create owned test workspace");
    path
}

#[tokio::test]
async fn truncated_header_is_an_aborted_attempt_not_a_parsed_request() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let client = tokio::spawn(async move {
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream
            .write_all(b"POST /v1/traces HTTP/1.1\r\n")
            .await
            .unwrap();
        stream.shutdown().await.unwrap();
    });
    let (stream, _) = listener.accept().await.unwrap();
    client.await.unwrap();
    let stats = Arc::new(CollectorStats::default());

    handle_collector_request(stream, Arc::clone(&stats), CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(stats.transport_aborts.load(Ordering::Relaxed), 1);
    assert_eq!(stats.trace_requests.load(Ordering::Relaxed), 0);
    assert_eq!(stats.parsed_spans.load(Ordering::Relaxed), 0);
    assert!(stats
        .unique_spans
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_empty());
}

#[tokio::test]
async fn complete_malformed_http_remains_a_fatal_collector_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let client = tokio::spawn(async move {
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream
            .write_all(b"POST /v1/traces HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        stream.shutdown().await.unwrap();
    });
    let (mut stream, _) = listener.accept().await.unwrap();
    client.await.unwrap();

    let error = read_http_request(&mut stream).await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(error.to_string(), "missing content length");
}

#[test]
fn only_expected_peer_disconnects_are_transport_aborts() {
    for kind in [
        io::ErrorKind::UnexpectedEof,
        io::ErrorKind::ConnectionAborted,
        io::ErrorKind::ConnectionReset,
        io::ErrorKind::BrokenPipe,
    ] {
        assert!(is_peer_transport_abort(&io::Error::from(kind)));
    }
    assert!(!is_peer_transport_abort(&io::Error::from(
        io::ErrorKind::InvalidData
    )));
}

#[cfg(windows)]
async fn send_native(endpoint: &str, frame: &[u8]) -> io::Result<()> {
    use tokio::net::windows::named_pipe::ClientOptions;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    loop {
        match ClientOptions::new().open(endpoint) {
            Ok(mut client) => return client.write_all(frame).await,
            Err(error) if tokio::time::Instant::now() < deadline => {
                let _ = error;
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(unix)]
async fn send_native(endpoint: &str, frame: &[u8]) -> io::Result<()> {
    use tokio::net::UnixStream;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    loop {
        match UnixStream::connect(endpoint).await {
            Ok(mut stream) => return stream.write_all(frame).await,
            Err(error) if tokio::time::Instant::now() < deadline => {
                let _ = error;
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            Err(error) => return Err(error),
        }
    }
}
