/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use std::io::{self, Read, Write};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use agent_otel_ipc::client::send_fire_and_forget;
use agent_otel_ipc::frame::MsgType;

const MAX_STDIN_BYTES: usize = 256 * 1024;
const WATCHDOG_MS: u64 = 3;

fn main() {
    let done = Arc::new(AtomicBool::new(false));
    spawn_watchdog(Arc::clone(&done));

    let tag = resolve_tag();
    let stdin_bytes = read_stdin_capped(MAX_STDIN_BYTES);

    let mut payload = Vec::with_capacity(1 + stdin_bytes.len());
    payload.push(tag);
    payload.extend_from_slice(&stdin_bytes);

    send_fire_and_forget(MsgType::HookPayload, &payload);

    done.store(true, Ordering::Release);
    finish_ok();
}

fn resolve_tag() -> u8 {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        Some("PreInvocation") | Some("pre_invocation") => 1,
        Some("PostInvocation") | Some("post_invocation") => 2,
        Some("PreToolUse") | Some("pre_tool_use") => 3,
        Some("PostToolUse") | Some("post_tool_use") => 4,
        Some("Stop") | Some("stop") => 5,
        Some(s) => s.parse::<u8>().unwrap_or(255),
        None => 255,
    }
}

fn read_stdin_capped(max: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4096);
    let mut handle = io::stdin().take(max as u64);
    let _ = handle.read_to_end(&mut buf);
    buf
}

fn spawn_watchdog(done: Arc<AtomicBool>) {
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(WATCHDOG_MS));
        if !done.load(Ordering::Acquire) {
            finish_ok();
        }
    });
}

fn finish_ok() -> ! {
    let mut stdout = io::stdout();
    let _ = stdout.write_all(b"{}");
    let _ = stdout.flush();
    process::exit(0);
}
