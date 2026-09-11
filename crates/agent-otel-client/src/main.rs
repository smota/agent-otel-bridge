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
const WATCHDOG_MS: u64 = 25;

fn main() {
    let tag = resolve_tag();
    let done = Arc::new(AtomicBool::new(false));
    spawn_watchdog(Arc::clone(&done), tag);

    let stdin_bytes = read_stdin_capped(MAX_STDIN_BYTES);

    let mut payload = Vec::with_capacity(1 + stdin_bytes.len());
    payload.push(tag);
    payload.extend_from_slice(&stdin_bytes);

    send_fire_and_forget(MsgType::HookPayload, &payload);

    done.store(true, Ordering::Release);
    finish_ok(tag);
}

fn resolve_tag() -> u8 {
    let mut event_id = 0u8;
    let mut client_id = 0u8;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--client" || arg == "--agent" {
            if i + 1 < args.len() {
                client_id = match args[i + 1].to_ascii_lowercase().as_str() {
                    "antigravity" | "agy" | "gemini" => 1,
                    "claude" | "claude-code" => 2,
                    "codex" | "openai" => 3,
                    "grok" | "xai" => 4,
                    "pi" | "inflection" => 5,
                    _ => 0,
                };
                i += 2;
                continue;
            }
        }

        match arg.as_str() {
            "PreInvocation" | "pre_invocation" => event_id = 1,
            "PostInvocation" | "post_invocation" => event_id = 2,
            "PreToolUse" | "pre_tool_use" => event_id = 3,
            "PostToolUse" | "post_tool_use" => event_id = 4,
            "Stop" | "stop" => event_id = 5,
            s => {
                if let Ok(num) = s.parse::<u8>() {
                    event_id = num;
                }
            }
        }
        i += 1;
    }

    if event_id == 0 {
        event_id = 255;
    }

    (client_id << 4) | (event_id & 0x0F)
}

fn read_stdin_capped(max: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4096);
    let mut handle = io::stdin().take(max as u64);
    let _ = handle.read_to_end(&mut buf);
    buf
}

fn spawn_watchdog(done: Arc<AtomicBool>, tag: u8) {
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(WATCHDOG_MS));
        if !done.load(Ordering::Acquire) {
            finish_ok(tag);
        }
    });
}

fn finish_ok(tag: u8) -> ! {
    let mut stdout = io::stdout();
    let client_id = tag >> 4;
    let event_id = tag & 0x0F;
    if event_id == 3 && client_id != 3 {
        let _ = stdout.write_all(b"{\"decision\":\"allow\"}");
    } else {
        let _ = stdout.write_all(b"{}");
    }
    let _ = stdout.flush();
    process::exit(0);
}
