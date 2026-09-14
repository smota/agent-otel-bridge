use agent_otel_fleet_smoke::telemetry::export_http;
use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

fn serve(response: impl Into<String>) -> (String, thread::JoinHandle<String>) {
    let response = response.into();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let endpoint = format!("http://{}/v1/traces", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = stream.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            request.extend_from_slice(&buf[..n]);
            if let Some(split) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..split]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("Content-Length:")
                            .or_else(|| line.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if request.len() >= split + 4 + length {
                    break;
                }
            }
        }
        stream.write_all(response.as_bytes()).unwrap();
        String::from_utf8(request).unwrap()
    });
    (endpoint, handle)
}

#[test]
fn malformed_or_oversized_acceptance_is_never_reported_as_success() {
    for body in [
        "not-json".to_owned(),
        r#"{"partialSuccess":{"rejectedSpans":"invalid"}}"#.to_owned(),
        " ".repeat(65_537),
    ] {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let (endpoint, server) = serve(response);
        let result = export_http(&endpoint, &json!({"resourceSpans":[]}));
        server.join().unwrap();
        assert!(result.is_err());
    }
}

#[test]
fn exports_json_post_and_reports_transport_only() {
    let (endpoint, server) = serve("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}");
    let payload = json!({"resourceSpans":[]});
    let receipt = export_http(&endpoint, &payload).unwrap();
    let request = server.join().unwrap();
    assert!(request.starts_with("POST /v1/traces HTTP/1.1"));
    assert!(request
        .to_ascii_lowercase()
        .contains("content-type: application/json"));
    assert!(request.contains(r#"{"resourceSpans":[]}"#));
    assert_eq!(receipt.http_status, 200);
    assert!(receipt.transport_accepted);
    assert_eq!(receipt.backend_visibility, "NOT_VERIFIED");
}

#[test]
fn partial_success_is_not_full_acceptance() {
    let (endpoint, server) = serve("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 40\r\n\r\n{\"partialSuccess\":{\"rejectedSpans\":\"1\"}}");
    let receipt = export_http(&endpoint, &json!({"resourceSpans":[]})).unwrap();
    server.join().unwrap();
    assert_eq!(receipt.rejected_spans, 1);
    assert!(!receipt.transport_accepted);
}

#[test]
fn server_error_is_reported_without_false_acceptance() {
    let (endpoint, server) =
        serve("HTTP/1.1 500 Internal Server Error\r\nContent-Length: 2\r\n\r\n{}");
    let receipt = export_http(&endpoint, &json!({"resourceSpans":[]})).unwrap();
    server.join().unwrap();
    assert_eq!(receipt.http_status, 500);
    assert!(!receipt.transport_accepted);
}
