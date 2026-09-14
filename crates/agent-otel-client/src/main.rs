/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use std::io::{self, Read, Write};
use std::process;
use std::thread;
use std::time::{Duration, Instant};

use agent_otel_ipc::client::{
    attempt_send_until, read_traceparent, terminate_current_process, SendAttempt,
};
use agent_otel_ipc::frame::{encode_context_payload, MsgType, WireHeader};

mod hook_observer;

use hook_observer::HookObserver;

const MAX_STDIN_BYTES: usize = 256 * 1024;
const DEFAULT_WATCHDOG_MS: u64 = 3;

fn main() {
    let main_entry = Instant::now();
    let watchdog_duration = watchdog_duration();
    let absolute_deadline = main_entry + watchdog_duration;
    spawn_watchdog(absolute_deadline);

    let observer = HookObserver::from_environment(main_entry);
    let header = resolve_header();
    if !write_response(header) {
        process::exit(0);
    }
    let response_completed = observer.as_ref().map(HookObserver::elapsed);

    let stdin_bytes = read_stdin_capped(MAX_STDIN_BYTES);
    // The hook only reads its inherited environment; no thread mutates it.
    let traceparent = unsafe { read_traceparent() };

    let payload = encode_context_payload(header, traceparent.as_deref(), &stdin_bytes);
    let before_transport = observer.as_ref().map(HookObserver::elapsed);

    let attempt = attempt_send_until(
        None,
        MsgType::HookPayloadWithContext,
        &payload,
        absolute_deadline,
    );

    match attempt {
        SendAttempt::Complete(result) => {
            if let (Some(observer), Some(response_completed), Some(before_transport)) =
                (observer, response_completed, before_transport)
            {
                let work_completed = observer.elapsed();
                observer.emit(
                    response_completed,
                    before_transport,
                    work_completed,
                    result.is_ok(),
                );
            }
            process::exit(0);
        }
        #[cfg(windows)]
        SendAttempt::CleanupRequired { pending, .. } => finish_with_guard(pending),
    }
}

struct ClientMapping {
    aliases: &'static [&'static str],
    wire_id: u16,
    allow_pre_tool: bool,
}

const CLIENT_MAPPINGS: &[ClientMapping] = &[
    ClientMapping {
        aliases: &["antigravity", "agy", "gemini"],
        wire_id: 1,
        allow_pre_tool: true,
    },
    ClientMapping {
        aliases: &["claude", "claude-code", "claudecode"],
        wire_id: 2,
        allow_pre_tool: true,
    },
    ClientMapping {
        aliases: &["codex", "openai", "codex-cli"],
        wire_id: 3,
        allow_pre_tool: false,
    },
    ClientMapping {
        aliases: &["grok", "xai", "grok-cli"],
        wire_id: 4,
        allow_pre_tool: true,
    },
    ClientMapping {
        aliases: &["pi", "pi-cli"],
        wire_id: 5,
        allow_pre_tool: true,
    },
];

fn resolve_client_id(arg: &str) -> u16 {
    let lower = arg.to_ascii_lowercase();
    for m in CLIENT_MAPPINGS {
        if m.aliases.contains(&lower.as_str()) {
            return m.wire_id;
        }
    }
    0
}

fn resolve_header() -> WireHeader {
    let mut event_id = 0u8;
    let mut client_id = 0u16;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if (arg == "--client" || arg == "--agent") && i + 1 < args.len() {
            client_id = resolve_client_id(&args[i + 1]);
            i += 2;
            continue;
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

    WireHeader::new(client_id, event_id)
}

fn read_stdin_capped(max: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4096);
    let mut handle = io::stdin().take(max as u64);
    let _ = handle.read_to_end(&mut buf);
    buf
}

fn watchdog_duration() -> Duration {
    let watchdog_ms = std::env::var("AGENT_OTEL_WATCHDOG_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_WATCHDOG_MS);
    Duration::from_millis(watchdog_ms)
}

fn spawn_watchdog(absolute_deadline: Instant) {
    thread::spawn(move || {
        loop {
            let remaining = absolute_deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            thread::sleep(remaining);
        }
        terminate_current_process(0);
    });
}

fn write_response(header: WireHeader) -> bool {
    let mut stdout = io::stdout();
    let allow_pre_tool = CLIENT_MAPPINGS
        .iter()
        .find(|m| m.wire_id == header.client_id)
        .map(|m| m.allow_pre_tool)
        .unwrap_or(true);

    let write_result = if header.event_id == 3 && allow_pre_tool {
        stdout.write_all(b"{\"decision\":\"allow\"}")
    } else {
        stdout.write_all(b"{}")
    };
    write_result.is_ok() && stdout.flush().is_ok()
}

fn finish_with_guard<T>(_guard: T) -> ! {
    process::exit(0);
}
