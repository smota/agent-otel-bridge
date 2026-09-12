/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[cfg(windows)]
use crate::frame::DEFAULT_PIPE_NAME;
use crate::frame::{decode_header, MsgType, HEADER_LEN};

#[cfg(windows)]
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

#[cfg(windows)]
pub async fn run_server(
    pipe_name: Option<&str>,
    tx: mpsc::Sender<(MsgType, Vec<u8>)>,
    shutdown: CancellationToken,
) -> std::io::Result<()> {
    let name = pipe_name
        .map(|s| s.to_string())
        .or_else(|| std::env::var("AGENT_OTEL_PIPE").ok())
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

#[cfg(windows)]
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

#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};

#[cfg(unix)]
pub async fn run_server(
    socket_path: Option<&str>,
    tx: mpsc::Sender<(MsgType, Vec<u8>)>,
    shutdown: CancellationToken,
) -> std::io::Result<()> {
    let path = socket_path
        .map(|s| s.to_string())
        .or_else(|| std::env::var("AGENT_OTEL_SOCKET").ok())
        .unwrap_or_else(|| "/tmp/agent_otel_bridge.sock".to_string());

    let _ = std::fs::remove_file(&path);
    if let Some(parent) = std::path::Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let listener = UnixListener::bind(&path)?;

    loop {
        tokio::select! {
            res = listener.accept() => {
                match res {
                    Ok((stream, _)) => {
                        let tx = tx.clone();
                        let shutdown_child = shutdown.clone();
                        tokio::spawn(async move {
                            let _ = handle_unix_client(stream, tx, shutdown_child).await;
                        });
                    }
                    Err(_) => {
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                }
            }
            _ = shutdown.cancelled() => {
                let _ = std::fs::remove_file(&path);
                return Ok(());
            }
        }
    }
}

#[cfg(unix)]
async fn handle_unix_client(
    mut stream: UnixStream,
    tx: mpsc::Sender<(MsgType, Vec<u8>)>,
    shutdown: CancellationToken,
) -> std::io::Result<()> {
    let mut header = [0u8; HEADER_LEN];
    if stream.read_exact(&mut header).await.is_err() {
        return Ok(());
    }

    let (msg_type, payload_len) = decode_header(&header)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    if payload_len > 16 * 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "payload exceeds 16MB limit",
        ));
    }

    let mut payload = vec![0u8; payload_len as usize];
    stream.read_exact(&mut payload).await?;

    let is_shutdown = msg_type == MsgType::Shutdown;
    let _ = tx.send((msg_type, payload)).await;

    if is_shutdown {
        shutdown.cancel();
    }

    Ok(())
}

#[cfg(not(any(windows, unix)))]
pub async fn run_server(
    _name: Option<&str>,
    _tx: mpsc::Sender<(MsgType, Vec<u8>)>,
    shutdown: CancellationToken,
) -> std::io::Result<()> {
    shutdown.cancelled().await;
    Ok(())
}
