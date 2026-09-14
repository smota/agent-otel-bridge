/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_ipc::client::try_send;
use agent_otel_ipc::frame::MsgType;
use reqwest::{StatusCode, Url};
use std::time::Duration;

const MAX_OTLP_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
enum OtlpProbeResult {
    Accepted,
    PartiallyRejected(u64),
    TransportOnly,
    ProtocolUnknown(&'static str),
}

fn sanitize_url(raw: &str) -> String {
    let Ok(mut url) = Url::parse(raw) else {
        return "<invalid URL redacted>".to_string();
    };

    let _ = url.set_username("");
    let _ = url.set_password(None);
    if url.query().is_some() {
        url.set_query(Some("redacted"));
    }
    if url.fragment().is_some() {
        url.set_fragment(Some("<redacted>"));
    }
    url.to_string()
}

fn sanitize_resource_attributes(raw: &str) -> String {
    raw.split(',')
        .map(|attribute| {
            attribute
                .split_once('=')
                .map(|(key, _)| format!("{key}=<redacted>"))
                .unwrap_or_else(|| "<redacted>".to_string())
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn format_http_probe(level: &str, service: &str, url: &str, status: StatusCode) -> String {
    if status.is_success() {
        format!("  [{level}] {service} reachable at {url} (HTTP status: {status})")
    } else {
        format!(
            "  [warn] {service} responded at {url} (HTTP status: {status}; request was not accepted)"
        )
    }
}

fn is_protobuf_content_type(value: Option<&str>) -> bool {
    value
        .and_then(|v| v.split(';').next())
        .map(|v| v.trim().eq_ignore_ascii_case("application/x-protobuf"))
        .unwrap_or(false)
}

// Validate the small response envelope without adding a protobuf runtime to
// the CLI binary. Empty ExportTraceServiceResponse is the valid default.
fn parse_otlp_response(body: &[u8]) -> Result<Option<u64>, &'static str> {
    let mut i = 0;
    let mut rejected = None;
    while i < body.len() {
        let (key, used) = read_varint(&body[i..])?;
        i += used;
        let field = key >> 3;
        if field == 0 {
            return Err("invalid protobuf field number");
        }
        match key & 7 {
            wire if field == 1 && wire != 2 => return Err("invalid partialSuccess wire type"),
            0 => {
                let (_, used) = read_varint(&body[i..])?;
                i += used;
            }
            2 => {
                let (len, used) = read_varint(&body[i..])?;
                i += used;
                let length = usize::try_from(len).map_err(|_| "length overflow")?;
                let end = i.checked_add(length).ok_or("length overflow")?;
                if end > body.len() {
                    return Err("truncated protobuf response");
                }
                if field == 1 {
                    rejected = parse_partial_success(&body[i..end])?;
                }
                i = end;
            }
            1 => i = i.checked_add(8).ok_or("length overflow")?,
            5 => i = i.checked_add(4).ok_or("length overflow")?,
            _ => return Err("unsupported protobuf wire type"),
        }
        if i > body.len() {
            return Err("truncated protobuf response");
        }
    }
    Ok(rejected)
}

fn parse_partial_success(body: &[u8]) -> Result<Option<u64>, &'static str> {
    let mut i = 0;
    let mut rejected = None;
    while i < body.len() {
        let (key, used) = read_varint(&body[i..])?;
        i += used;
        if key >> 3 == 0 {
            return Err("invalid partialSuccess field number");
        }
        match (key >> 3, key & 7) {
            (1, wire) if wire != 0 => return Err("invalid rejectedSpans wire type"),
            (2, wire) if wire != 2 => return Err("invalid errorMessage wire type"),
            (1, 0) => {
                let (value, used) = read_varint(&body[i..])?;
                rejected = Some(value);
                i += used;
            }
            (_, 0) => {
                let (_, used) = read_varint(&body[i..])?;
                i += used;
            }
            (2, 2) => {
                let (len, used) = read_varint(&body[i..])?;
                i += used;
                let length = usize::try_from(len).map_err(|_| "length overflow")?;
                let end = i.checked_add(length).ok_or("length overflow")?;
                if end > body.len() {
                    return Err("truncated partialSuccess protobuf");
                }
                i = end;
            }
            (field, 2) if field != 1 && field != 2 => {
                let (len, used) = read_varint(&body[i..])?;
                i += used;
                let length = usize::try_from(len).map_err(|_| "length overflow")?;
                let end = i.checked_add(length).ok_or("length overflow")?;
                if end > body.len() {
                    return Err("truncated partialSuccess protobuf");
                }
                i = end;
            }
            _ => return Err("invalid partialSuccess protobuf"),
        }
    }
    Ok(rejected)
}

fn read_varint(bytes: &[u8]) -> Result<(u64, usize), &'static str> {
    let mut value = 0u64;
    for (idx, byte) in bytes.iter().copied().enumerate().take(10) {
        let shift = idx * 7;
        if idx == 9 && byte > 1 {
            return Err("invalid protobuf varint");
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, idx + 1));
        }
    }
    Err("truncated protobuf varint")
}

fn classify_otlp_response(
    status: StatusCode,
    content_type: Option<&str>,
    body: &[u8],
) -> OtlpProbeResult {
    if !status.is_success() {
        return OtlpProbeResult::TransportOnly;
    }
    if body.len() > MAX_OTLP_RESPONSE_BYTES {
        return OtlpProbeResult::ProtocolUnknown("response exceeds bounded limit");
    }
    if !is_protobuf_content_type(content_type) {
        return OtlpProbeResult::ProtocolUnknown("unexpected response content type");
    }
    match parse_otlp_response(body) {
        Ok(Some(count)) if count > 0 => OtlpProbeResult::PartiallyRejected(count),
        Ok(_) => OtlpProbeResult::Accepted,
        Err(reason) => OtlpProbeResult::ProtocolUnknown(reason),
    }
}

async fn send_otlp_probe(
    client: &reqwest::Client,
    traces_url: &str,
) -> Result<(StatusCode, Option<String>, Vec<u8>), String> {
    let mut response = client
        .post(traces_url)
        .header(reqwest::header::CONTENT_TYPE, "application/x-protobuf")
        .body(Vec::<u8>::new())
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| error.without_url().to_string())?
    {
        if body.len().saturating_add(chunk.len()) > MAX_OTLP_RESPONSE_BYTES {
            body.resize(MAX_OTLP_RESPONSE_BYTES + 1, 0);
            break;
        }
        body.extend_from_slice(&chunk);
    }
    Ok((status, content_type, body))
}

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== agent-otel-bridge doctor ===");
    println!("Diagnosing station telemetry pipeline & OpenTelemetry invariants\n");

    // 1. Environment variables check
    println!("[1/5] Checking environment contract variables...");
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://127.0.0.1:4318".to_string());
    println!(
        "  [ok] OTEL_EXPORTER_OTLP_ENDPOINT = {}",
        sanitize_url(&endpoint)
    );

    match std::env::var("OTEL_SERVICE_NAME") {
        Ok(v) => println!("  [ok] OTEL_SERVICE_NAME = {} (per-process)", v),
        Err(_) => {
            println!("  [info] OTEL_SERVICE_NAME is unset (using default 'agent-otel-bridge')")
        }
    }

    match std::env::var("OTEL_RESOURCE_ATTRIBUTES") {
        Ok(v) => println!(
            "  [ok] OTEL_RESOURCE_ATTRIBUTES = {}",
            sanitize_resource_attributes(&v)
        ),
        Err(_) => println!("  [info] OTEL_RESOURCE_ATTRIBUTES unset (defaulting to 'deployment.environment=homelab')"),
    }

    // 2. Named Pipe IPC check
    let pipe_name = std::env::var("AGENT_OTEL_PIPE")
        .or_else(|_| std::env::var("AGY_OTEL_PIPE"))
        .unwrap_or_else(|_| agent_otel_ipc::frame::DEFAULT_PIPE_NAME.to_string());
    println!("\n[2/5] Checking named pipe IPC ({})...", pipe_name);
    let ping_res = try_send(MsgType::HealthPing, b"doctor-probe");
    match ping_res {
        Ok(()) => println!("  [ok] Daemon is RUNNING and responding on named pipe!"),
        Err(()) => {
            println!("  [warn] Daemon is NOT currently running (pipe not found or busy).");
            println!(
                "         Run 'agent-otel-bridge start' or invoke an agent hook to auto-launch it."
            );
        }
    }

    // 3. OTLP Collector connectivity check
    println!("\n[3/5] Checking OTLP Collector HTTP endpoint...");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;

    let traces_url = format!("{}/v1/traces", endpoint.trim_end_matches('/'));
    match send_otlp_probe(&client, &traces_url).await {
        Ok((status, content_type, body)) => {
            println!(
                "{}",
                format_http_probe("ok", "OTLP Collector", &sanitize_url(&traces_url), status)
            );
            match classify_otlp_response(status, content_type.as_deref(), &body) {
                OtlpProbeResult::Accepted =>
                    println!("  [ok] OTLP protocol accepted the probe (backend visibility not checked)"),
                OtlpProbeResult::PartiallyRejected(count) => println!(
                    "  [warn] OTLP protocol partially rejected the probe ({count} spans; backend visibility not checked)"
                ),
                OtlpProbeResult::TransportOnly => println!(
                    "  [warn] HTTP endpoint is reachable, but the OTLP request was not accepted"
                ),
                OtlpProbeResult::ProtocolUnknown(reason) => println!(
                    "  [warn] HTTP endpoint responded, but OTLP protocol acceptance is unknown: {reason}"
                ),
            }
        }
        Err(e) => {
            println!(
                "  [fail] OTLP Collector UNREACHABLE at {}: {}",
                sanitize_url(&traces_url),
                e
            );
        }
    }

    // 4. Observability Dashboard / UI reachability check
    let ui_url = std::env::var("OTEL_UI_URL")
        .or_else(|_| std::env::var("SIGNOZ_UI_URL"))
        .unwrap_or_else(|_| "http://localhost:8080".to_string());

    println!(
        "\n[4/5] Checking Observability UI reachability ({})...",
        sanitize_url(&ui_url)
    );
    match client.get(&ui_url).send().await {
        Ok(resp) => {
            println!(
                "{}",
                format_http_probe(
                    "ok",
                    "Observability UI",
                    &sanitize_url(&ui_url),
                    resp.status()
                )
            );
            if ui_url.contains("localhost:8080") {
                println!(
                    "  [dashboard] {}/dashboard/01a091c1-c5c7-7105-9963-436f893fa832 (AI Agent Observability)",
                    sanitize_url(ui_url.trim_end_matches('/'))
                );
            }
        }
        Err(e) => {
            println!(
                "  [info] Observability UI not reachable at {} (optional): {}",
                sanitize_url(&ui_url),
                e.without_url()
            );
        }
    }

    // 5. Client Hook Registrations & Local Runtime check
    println!("\n[5/5] Checking Client Hook Registrations & Local Runtime...");
    let canonical_hook = crate::local::get_canonical_hook_path();
    let canonical_bridge = crate::local::get_canonical_bridge_path();
    let local_ok = canonical_hook.is_file() && canonical_bridge.is_file();
    println!(
        "  Local runtime installation: {}",
        if local_ok {
            format!(
                "[ok] present at {}",
                crate::local::get_canonical_bin_dir().display()
            )
        } else {
            "[info] not installed (run 'agent-otel-bridge local install')".to_string()
        }
    );

    let hook_in_path = crate::hooks::check_binary_in_path("agent-hook");
    println!(
        "  agent-hook in PATH:         {}",
        if hook_in_path {
            "[ok] present"
        } else {
            "[info] not in PATH (not required when absolute path is configured)"
        }
    );

    for adapter in crate::hooks::CLIENT_ADAPTERS {
        if let Some(p) = (adapter.global_config_fn)() {
            let configured = (adapter.is_registered_fn)(&p);
            println!(
                "  {:<20} {}",
                format!("{}:", adapter.display_name),
                if configured {
                    format!("[ok] registered ({})", p.display())
                } else {
                    format!("[info] not registered ({})", p.display())
                }
            );
        }
    }

    // Workspace shadowing diagnostic for current directory
    if let Ok(current_dir) = std::env::current_dir() {
        for adapter in crate::hooks::CLIENT_ADAPTERS {
            if adapter.scope == crate::hooks::HookScope::ProjectShadowsGlobal {
                if let Some(p) = (adapter.project_config_fn)(Some(&current_dir)) {
                    if p.exists() && !(adapter.is_registered_fn)(&p) {
                        println!(
                            "\n  [warn] Workspace has local {} without bridge hook (shadows global telemetry!)",
                            p.display()
                        );
                        println!(
                            "         Run 'agent-otel-bridge hooks sync' to activate telemetry in this workspace."
                        );
                    }
                }
            }
        }
    }

    println!("\nDoctor check completed.\n");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn collector_http_errors_are_not_reported_as_ok() {
        let output = format_http_probe(
            "ok",
            "OTLP Collector",
            "http://collector/v1/traces",
            StatusCode::INTERNAL_SERVER_ERROR,
        );
        assert!(output.starts_with("  [warn]"));
        assert!(output.contains("request was not accepted"));

        let output = format_http_probe(
            "ok",
            "OTLP Collector",
            "http://collector/v1/traces",
            StatusCode::UNAUTHORIZED,
        );
        assert!(output.starts_with("  [warn]"));
        assert!(!output.contains("[ok]"));
    }

    #[test]
    fn sensitive_url_parts_are_redacted() {
        let output =
            sanitize_url("https://user:password@example.test/v1/traces?api_key=secret&token=abc");
        assert_eq!(output, "https://example.test/v1/traces?redacted");
        assert!(!output.contains("password"));
        assert!(!output.contains("secret"));
        assert_eq!(
            sanitize_url("not a URL user:password@example.test"),
            "<invalid URL redacted>"
        );
    }

    #[test]
    fn resource_attribute_values_are_redacted() {
        let output =
            sanitize_resource_attributes("deployment.environment=production,api.key=secret");
        assert_eq!(
            output,
            "deployment.environment=<redacted>,api.key=<redacted>"
        );
        assert!(!output.contains("production"));
        assert!(!output.contains("secret"));
    }

    #[test]
    fn otlp_probe_requires_matching_content_type_and_valid_body() {
        assert_eq!(
            classify_otlp_response(StatusCode::OK, Some("application/x-protobuf"), &[]),
            OtlpProbeResult::Accepted
        );
        assert_eq!(
            classify_otlp_response(StatusCode::OK, Some("application/json"), &[]),
            OtlpProbeResult::ProtocolUnknown("unexpected response content type")
        );
        assert_eq!(
            classify_otlp_response(
                StatusCode::OK,
                Some("application/x-protobuf"),
                &[0x0a, 0x05]
            ),
            OtlpProbeResult::ProtocolUnknown("truncated protobuf response")
        );
    }

    #[test]
    fn otlp_probe_distinguishes_transport_and_partial_rejection() {
        assert_eq!(
            classify_otlp_response(StatusCode::BAD_REQUEST, Some("application/x-protobuf"), &[]),
            OtlpProbeResult::TransportOnly
        );
        // partial_success { rejected_spans: 2 }
        assert_eq!(
            classify_otlp_response(
                StatusCode::OK,
                Some("application/x-protobuf; charset=binary"),
                // partial_success { rejected_spans: 2, error_message: "x" }
                &[0x0a, 0x05, 0x08, 0x02, 0x12, 0x01, b'x']
            ),
            OtlpProbeResult::PartiallyRejected(2)
        );
        assert_eq!(
            classify_otlp_response(StatusCode::OK, Some("application/x-protobuf"), &[0x00]),
            OtlpProbeResult::ProtocolUnknown("invalid protobuf field number")
        );
        assert_eq!(
            classify_otlp_response(
                StatusCode::OK,
                Some("application/x-protobuf"),
                &[0x0a, 0x02, 0x0a, 0x00]
            ),
            OtlpProbeResult::ProtocolUnknown("invalid rejectedSpans wire type")
        );
    }

    #[test]
    fn otlp_probe_response_is_bounded() {
        let body = vec![0; MAX_OTLP_RESPONSE_BYTES + 1];
        assert_eq!(
            classify_otlp_response(StatusCode::OK, Some("application/x-protobuf"), &body),
            OtlpProbeResult::ProtocolUnknown("response exceeds bounded limit")
        );
    }

    #[tokio::test]
    async fn otlp_probe_sends_matching_header_and_valid_empty_body() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let receiver = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 1024];
            loop {
                let count = stream.read(&mut buffer).await.unwrap();
                request.extend_from_slice(&buffer[..count]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request_text = String::from_utf8_lossy(&request).to_ascii_lowercase();
            assert!(request_text.starts_with("post /v1/traces "));
            assert!(request_text.contains("content-type: application/x-protobuf"));
            assert!(request_text.ends_with("\r\n\r\n"));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-protobuf\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
        });
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let (status, content_type, body) =
            send_otlp_probe(&client, &format!("http://{address}/v1/traces"))
                .await
                .unwrap();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type.as_deref(), Some("application/x-protobuf"));
        assert!(body.is_empty());
        assert_eq!(
            classify_otlp_response(status, content_type.as_deref(), &body),
            OtlpProbeResult::Accepted
        );
        receiver.await.unwrap();
    }
}
