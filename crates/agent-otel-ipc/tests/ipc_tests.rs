/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_ipc::frame::{decode_header, encode_frame, MsgType, HEADER_LEN};
#[cfg(windows)]
use agent_otel_ipc::{client, server};
use tokio_util::sync::CancellationToken;

#[test]
fn test_frame_encoding_and_decoding() {
    let payload = b"{\"event\":\"PostToolUse\",\"tool\":\"run_command\"}";
    let encoded = encode_frame(MsgType::HookPayload, payload);

    assert_eq!(encoded.len(), HEADER_LEN + payload.len());
    let mut header = [0u8; HEADER_LEN];
    header.copy_from_slice(&encoded[0..HEADER_LEN]);

    let (msg_type, len) = decode_header(&header).expect("failed to decode header");
    assert_eq!(msg_type, MsgType::HookPayload);
    assert_eq!(len as usize, payload.len());
    assert_eq!(&encoded[HEADER_LEN..], payload);
}

#[test]
fn test_client_fails_open_when_no_server() {
    std::env::set_var("AGY_OTEL_PIPE", r"\\.\pipe\agy-otel-nonexistent-pipe");
    let start = std::time::Instant::now();
    let res = client::try_send(MsgType::HookPayload, b"test");
    let elapsed = start.elapsed();

    // Should return Err (fail open) in < 10ms
    assert!(res.is_err());
    assert!(
        elapsed.as_millis() < 50,
        "fail-open took too long: {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_ipc_roundtrip_named_pipe() {
    let pipe_name = format!(r"\\.\pipe\agy-otel-test-{}", std::process::id());
    std::env::set_var("AGY_OTEL_PIPE", &pipe_name);

    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let shutdown = CancellationToken::new();
    let shutdown_server = shutdown.clone();
    let pipe_name_clone = pipe_name.clone();

    let server_handle = tokio::spawn(async move {
        server::run_server(Some(&pipe_name_clone), tx, shutdown_server).await
    });

    // Allow server to initialize pipe
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Send payload from client
    let payload = b"hello from client hook";
    let send_res = client::try_send(MsgType::HookPayload, payload);
    assert!(send_res.is_ok(), "try_send failed on live server");

    // Receive on server channel
    let (msg_type, received) =
        tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("timeout waiting for rx")
            .expect("channel closed");

    assert_eq!(msg_type, MsgType::HookPayload);
    assert_eq!(received, payload);

    // Send shutdown
    let shutdown_res = client::try_send(MsgType::Shutdown, b"");
    assert!(shutdown_res.is_ok());

    // Wait for server to shutdown cleanly
    let _ = tokio::time::timeout(std::time::Duration::from_millis(500), server_handle).await;
}
