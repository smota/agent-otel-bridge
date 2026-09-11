/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use tokio::io::AsyncReadExt;
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::frame::{decode_header, MsgType, DEFAULT_PIPE_NAME, HEADER_LEN};

pub async fn run_server(
    pipe_name: Option<&str>,
    tx: mpsc::Sender<(MsgType, Vec<u8>)>,
    shutdown: CancellationToken,
) -> std::io::Result<()> {
    let name = pipe_name
        .map(|s| s.to_string())
        .or_else(|| std::env::var("AGY_OTEL_PIPE").ok())
        .unwrap_or_else(|| DEFAULT_PIPE_NAME.to_string());

    let mut server = ServerOptions::new()
        .first_pipe_instance(true)
        .in_buffer_size(64 * 1024)
        .out_buffer_size(4 * 1024)
        .create(&name)?;

    loop {
        tokio::select! {
            res = server.connect() => {
                match res {
                    Ok(()) => {
                        let mut next_server = None;
                        for _attempt in 0..10 {
                            match ServerOptions::new()
                                .in_buffer_size(64 * 1024)
                                .out_buffer_size(4 * 1024)
                                .create(&name)
                            {
                                Ok(s) => {
                                    next_server = Some(s);
                                    break;
                                }
                                Err(_) => {
                                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                                }
                            }
                        }

                        let next = match next_server {
                            Some(s) => s,
                            None => {
                                eprintln!("[agent-otel-ipc] Warning: unable to allocate next pipe instance, retrying in 500ms");
                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                continue;
                            }
                        };

                        let connected = std::mem::replace(&mut server, next);
                        let tx = tx.clone();
                        let shutdown_child = shutdown.clone();
                        tokio::spawn(async move {
                            let _ = handle_client(connected, tx, shutdown_child).await;
                        });
                    }
                    Err(_e) => {
                        // Transient connection error, retry
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                }
            }
            _ = shutdown.cancelled() => {
                return Ok(());
            }
        }
    }
}

async fn handle_client(
    mut pipe: NamedPipeServer,
    tx: mpsc::Sender<(MsgType, Vec<u8>)>,
    shutdown: CancellationToken,
) -> std::io::Result<()> {
    let mut header = [0u8; HEADER_LEN];
    if pipe.read_exact(&mut header).await.is_err() {
        return Ok(()); // client disconnected or sent nothing
    }

    let (msg_type, payload_len) = decode_header(&header)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    // Protect against unbounded allocation (> 16MB)
    if payload_len > 16 * 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "payload exceeds 16MB limit",
        ));
    }

    let mut payload = vec![0u8; payload_len as usize];
    pipe.read_exact(&mut payload).await?;

    let is_shutdown = msg_type == MsgType::Shutdown;
    let _ = tx.send((msg_type, payload)).await;

    if is_shutdown {
        shutdown.cancel();
    }

    Ok(())
}
