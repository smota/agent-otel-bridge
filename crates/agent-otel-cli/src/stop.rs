/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use agent_otel_ipc::client::try_send;
use agent_otel_ipc::frame::MsgType;

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    println!("[stop] Sending graceful shutdown signal to daemon via IPC...");
    match try_send(MsgType::Shutdown, b"") {
        Ok(()) => {
            println!("[ok] Shutdown signal delivered. Daemon will flush pending spans and exit.");
            Ok(())
        }
        Err(()) => {
            println!("[warn] Could not deliver shutdown signal (daemon is not running).");
            Ok(())
        }
    }
}
