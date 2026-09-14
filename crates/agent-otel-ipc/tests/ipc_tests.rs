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
    let non_existent = format!(
        r"\\.\pipe\agy-otel-nonexistent-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let start = std::time::Instant::now();
    let res = client::attempt_send_until(
        Some(&non_existent),
        MsgType::HookPayload,
        b"test",
        start + std::time::Duration::from_millis(3),
    );
    let elapsed = start.elapsed();

    // Should return Err (fail open) swiftly without hanging
    assert!(matches!(res, client::SendAttempt::Complete(Err(_))));
    assert!(
        elapsed.as_millis() < 1500,
        "fail-open took too long: {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_ipc_roundtrip_named_pipe() {
    let pipe_name = format!(r"\\.\pipe\agy-otel-test-{}", std::process::id());

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
    let send_res = client::attempt_send_until(
        Some(&pipe_name),
        MsgType::HookPayload,
        payload,
        std::time::Instant::now() + std::time::Duration::from_millis(100),
    );
    assert!(
        matches!(&send_res, client::SendAttempt::Complete(Ok(_))),
        "client attempt failed on live server: {send_res:?}"
    );

    // Receive on server channel
    let (msg_type, received) =
        tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("timeout waiting for rx")
            .expect("channel closed");

    assert_eq!(msg_type, MsgType::HookPayload);
    assert_eq!(received, payload);

    // Send shutdown
    let shutdown_res = client::attempt_send_until(
        Some(&pipe_name),
        MsgType::Shutdown,
        b"",
        std::time::Instant::now() + std::time::Duration::from_millis(100),
    );
    assert!(matches!(shutdown_res, client::SendAttempt::Complete(Ok(_))));

    // Wait for server to shutdown cleanly
    let _ = tokio::time::timeout(std::time::Duration::from_millis(500), server_handle).await;
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_waiters_retry_busy_reopen_until_private_endpoint_accepts_all() {
    const CLIENTS: usize = 24;
    let pipe_name = format!(
        r"\\.\pipe\agent-otel-busy-reopen-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let (tx, mut rx) = tokio::sync::mpsc::channel(CLIENTS + 1);
    let shutdown = CancellationToken::new();
    let server_name = pipe_name.clone();
    let server_shutdown = shutdown.clone();
    let server_handle =
        tokio::spawn(
            async move { server::run_server(Some(&server_name), tx, server_shutdown).await },
        );
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(CLIENTS));
    let mut clients = Vec::with_capacity(CLIENTS);
    for index in 0..CLIENTS {
        let barrier = barrier.clone();
        let pipe_name = pipe_name.clone();
        clients.push(std::thread::spawn(move || {
            barrier.wait();
            let payload = index.to_le_bytes();
            match client::attempt_send_until(
                Some(&pipe_name),
                MsgType::HookPayload,
                &payload,
                std::time::Instant::now() + std::time::Duration::from_millis(250),
            ) {
                client::SendAttempt::Complete(Ok(_)) => Ok(()),
                client::SendAttempt::Complete(Err(error)) => Err(format!("{error:?}")),
                client::SendAttempt::CleanupRequired { error, pending } => {
                    let _ = pending.drain();
                    Err(format!("pending: {error:?}"))
                }
            }
        }));
    }
    let failures: Vec<_> = clients
        .into_iter()
        .enumerate()
        .filter_map(|(index, thread)| match thread.join() {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(format!("client {index}: {error}")),
            Err(_) => Some(format!("client {index}: thread panicked")),
        })
        .collect();
    assert!(
        failures.is_empty(),
        "concurrent delivery failures: {failures:?}"
    );

    for _ in 0..CLIENTS {
        let (msg_type, payload) =
            tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
                .await
                .expect("timeout waiting for concurrent frame")
                .expect("server channel closed");
        assert_eq!(msg_type, MsgType::HookPayload);
        assert_eq!(payload.len(), std::mem::size_of::<usize>());
    }

    let shutdown_result = client::attempt_send_until(
        Some(&pipe_name),
        MsgType::Shutdown,
        b"",
        std::time::Instant::now() + std::time::Duration::from_millis(250),
    );
    assert!(matches!(
        shutdown_result,
        client::SendAttempt::Complete(Ok(_))
    ));
    tokio::time::timeout(std::time::Duration::from_secs(1), server_handle)
        .await
        .expect("server shutdown timed out")
        .expect("server task panicked")
        .expect("server failed");
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_pending_writes_retain_ownership_and_release_handles() {
    use agent_otel_ipc::client::{attempt_send_until, DrainStatus, SendAttempt, SendStage};
    use tokio::net::windows::named_pipe::ServerOptions;

    const CHILD_ENV: &str = "AGENT_OTEL_PENDING_WRITE_TEST_CHILD";
    const TEST_NAME: &str = "repeated_pending_writes_retain_ownership_and_release_handles";
    if std::env::var_os(CHILD_ENV).is_none() {
        let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(CHILD_ENV, "1")
            .status()
            .expect("spawn isolated pending-write test");
        assert!(status.success(), "isolated pending-write test failed");
        return;
    }

    fn process_handle_count() -> u32 {
        let mut count = 0u32;
        let ok = unsafe {
            windows_sys::Win32::System::Threading::GetProcessHandleCount(
                windows_sys::Win32::System::Threading::GetCurrentProcess(),
                &mut count,
            )
        };
        assert_ne!(ok, 0, "GetProcessHandleCount failed");
        count
    }

    let mut steady_state_handles = None;
    for iteration in 0..4 {
        let pipe_name = format!(
            r"\\.\pipe\agent-otel-pending-{}-{}-{iteration}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let server = ServerOptions::new()
            .in_buffer_size(1)
            .out_buffer_size(1)
            .create(&pipe_name)
            .expect("create stalled-reader pipe");
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server.connect().await.expect("accept pending-write client");
            let _ = release_rx.await;
            drop(server);
        });

        // The tiny server buffer and large single write are intended to
        // exercise ERROR_IO_PENDING. The assertion never infers it from time.
        let payload = vec![0x5a; 8 * 1024 * 1024];
        let attempt = attempt_send_until(
            Some(&pipe_name),
            MsgType::HookPayload,
            &payload,
            std::time::Instant::now() + std::time::Duration::from_millis(50),
        );

        match attempt {
            SendAttempt::CleanupRequired { error, pending } => {
                assert!(matches!(
                    error.stage,
                    SendStage::Deadline | SendStage::AwaitCompletion
                ));
                // Release the deliberately stalled peer before Drop/drain so
                // failed kernel cancellation cannot hang the coordinator.
                let _ = release_tx.send(());
                tokio::time::timeout(std::time::Duration::from_secs(1), server_task)
                    .await
                    .expect("stalled server cleanup timed out")
                    .expect("stalled server task panicked");
                assert!(matches!(
                    pending.drain(),
                    DrainStatus::Cancelled | DrainStatus::Completed(_) | DrainStatus::Failed(_)
                ));
            }
            SendAttempt::Complete(result) => {
                let _ = release_tx.send(());
                let _ = server_task.await;
                panic!(
                    "native fixture did not exercise ERROR_IO_PENDING; completion was {result:?}"
                );
            }
        }
        tokio::task::yield_now().await;
        let current_handles = process_handle_count();
        if let Some(expected) = steady_state_handles {
            assert!(
                current_handles <= expected,
                "cancellation iteration {iteration} increased live Win32 handles: {expected} -> {current_handles}"
            );
            steady_state_handles = Some(current_handles);
        } else {
            // The first native I/O can initialize a runtime-owned completion
            // handle; later iterations must return to this settled baseline.
            steady_state_handles = Some(current_handles);
        }
    }
}
