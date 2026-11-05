/*
 * Copyright agent-otel-bridge contributors
 * SPDX-License-Identifier: Apache-2.0
 */

#[cfg(not(target_os = "linux"))]
use std::borrow::Cow;
#[cfg(not(target_os = "linux"))]
use std::io::{self, Read, Write};
#[cfg(not(target_os = "linux"))]
use std::process;
#[cfg(not(target_os = "linux"))]
use std::thread;
use std::time::{Duration, Instant};

#[cfg(not(target_os = "linux"))]
use agent_otel_ipc::client::read_tracestate;
use agent_otel_ipc::client::{attempt_send_until, read_traceparent, SendAttempt};
use agent_otel_ipc::frame::{encode_context_payload, MsgType, WireHeader};

#[cfg(not(target_os = "linux"))]
use agent_otel_ipc::client::terminate_current_process;
#[cfg(not(target_os = "linux"))]
use std::process::exit as exit_process;

#[cfg(target_os = "linux")]
mod linux_runtime;
#[cfg(target_os = "linux")]
use linux_runtime::{exit_process, read_stdin_capped};

mod hook_observer;

use hook_observer::HookObserver;

const MAX_STDIN_BYTES: usize = 256 * 1024;
const DEFAULT_WATCHDOG_MS: u64 = 3;

fn main() {
    let main_entry = Instant::now();
    let watchdog_duration = watchdog_duration();
    #[cfg(not(target_os = "linux"))]
    let absolute_deadline = main_entry + watchdog_duration;
    #[cfg(target_os = "linux")]
    let absolute_deadline = match main_entry.checked_add(watchdog_duration) {
        Some(deadline) => deadline,
        None => exit_process(0),
    };
    #[cfg(not(target_os = "linux"))]
    spawn_watchdog(absolute_deadline);
    #[cfg(target_os = "linux")]
    if !linux_runtime::spawn_watchdog(absolute_deadline) {
        // Without an independent watchdog even a response could block stdout.
        exit_process(0);
    }

    let observer = HookObserver::from_environment(main_entry);
    let header = resolve_header();
    if !write_response(header) {
        exit_process(0);
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
                    result.err(),
                );
            }
            exit_process(0);
        }
        #[cfg(windows)]
        SendAttempt::CleanupRequired { error, pending } => {
            if let (Some(observer), Some(response_completed), Some(before_transport)) =
                (observer, response_completed, before_transport)
            {
                let work_completed = observer.elapsed();
                observer.emit(
                    response_completed,
                    before_transport,
                    work_completed,
                    Some(error),
                );
            }
            // Pending owns the overlapped operation through process exit. If
            // the bounded observer write stalls, the watchdog terminates the
            // process without releasing kernel-referenced memory early.
            finish_with_guard(pending)
        }
    }
}

struct ClientMapping {
    aliases: &'static [&'static str],
    wire_id: u16,
    #[cfg(not(target_os = "linux"))]
    allow_pre_tool: bool,
}

const CLIENT_MAPPINGS: &[ClientMapping] = &[
    ClientMapping {
        aliases: &["antigravity", "agy", "gemini"],
        wire_id: 1,
        #[cfg(not(target_os = "linux"))]
        allow_pre_tool: true,
    },
    ClientMapping {
        aliases: &["claude", "claude-code", "claudecode"],
        wire_id: 2,
        #[cfg(not(target_os = "linux"))]
        allow_pre_tool: true,
    },
    ClientMapping {
        aliases: &["codex", "openai", "codex-cli"],
        wire_id: 3,
        #[cfg(not(target_os = "linux"))]
        allow_pre_tool: false,
    },
    ClientMapping {
        aliases: &["grok", "xai", "grok-cli"],
        wire_id: 4,
        #[cfg(not(target_os = "linux"))]
        allow_pre_tool: true,
    },
    ClientMapping {
        aliases: &["pi", "pi-cli"],
        wire_id: 5,
        #[cfg(not(target_os = "linux"))]
        allow_pre_tool: true,
    },
];

#[cfg(not(target_os = "linux"))]
fn resolve_client_id(arg: &str) -> u16 {
    let lower = arg.to_ascii_lowercase();
    for m in CLIENT_MAPPINGS {
        if m.aliases.contains(&lower.as_str()) {
            return m.wire_id;
        }
    }
    0
}

#[cfg(not(target_os = "linux"))]
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

#[cfg(target_os = "linux")]
fn resolve_header() -> WireHeader {
    let mut event_id = 255;
    let mut client_id = 0;
    let mut args = std::env::args_os().skip(1);
    while let Some(argument) = args.next() {
        // An unrecognized, non-UTF-8 OS argument must not panic on the hook's
        // fail-open path. Recognized event/client names are all ASCII.
        let Some(argument) = argument.to_str() else {
            continue;
        };
        if argument == "--client" || argument == "--agent" {
            if let Some(client) = args.next() {
                client_id = client
                    .to_str()
                    .and_then(|client| {
                        CLIENT_MAPPINGS.iter().find(|mapping| {
                            mapping
                                .aliases
                                .iter()
                                .any(|alias| alias.eq_ignore_ascii_case(client))
                        })
                    })
                    .map_or(0, |mapping| mapping.wire_id);
            }
            continue;
        }
        match argument {
            "PreInvocation" | "pre_invocation" => event_id = 1,
            "PostInvocation" | "post_invocation" => event_id = 2,
            "PreToolUse" | "pre_tool_use" => event_id = 3,
            "PostToolUse" | "post_tool_use" => event_id = 4,
            "Stop" | "stop" => event_id = 5,
            number => {
                if let Ok(number) = number.parse::<u8>() {
                    event_id = if number == 0 { 255 } else { number };
                }
            }
        }
    }
    WireHeader::new(client_id, event_id)
}

#[cfg(not(target_os = "linux"))]
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

#[cfg(not(target_os = "linux"))]
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

#[cfg(not(target_os = "linux"))]
fn write_response(header: WireHeader) -> bool {
    let mut stdout = io::stdout();
    let allow_pre_tool = CLIENT_MAPPINGS
        .iter()
        .find(|m| m.wire_id == header.client_id)
        .map(|m| m.allow_pre_tool)
        .unwrap_or(true);

    let write_result = if header.event_id == 3 && allow_pre_tool {
        let traceparent = unsafe { read_traceparent() };
        let tracestate = unsafe { read_tracestate() };
        let body = pre_tool_allow_json(traceparent.as_deref(), tracestate.as_deref());
        stdout.write_all(body.as_bytes())
    } else {
        stdout.write_all(b"{}")
    };
    write_result.is_ok() && stdout.flush().is_ok()
}

#[cfg(target_os = "linux")]
fn write_response(_header: WireHeader) -> bool {
    // Telemetry never approves, denies, or rewrites a tool invocation. Inherited
    // trace context travels only in IPC; stdout cannot set the harness's env.
    linux_runtime::write_stdout(b"{}")
}

#[cfg(windows)]
fn finish_with_guard<T>(_guard: T) -> ! {
    process::exit(0);
}

#[cfg(not(target_os = "linux"))]
fn traceparent_is_echo_safe(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
}

#[cfg(not(target_os = "linux"))]
fn tracestate_is_echo_safe(value: &str) -> bool {
    (1..=512).contains(&value.len())
        && value
            .bytes()
            .all(|byte| (byte.is_ascii_graphic() || byte == b' ') && byte != b'"' && byte != b'\\')
}

#[cfg(not(target_os = "linux"))]
fn pre_tool_allow_json(traceparent: Option<&str>, tracestate: Option<&str>) -> Cow<'static, str> {
    let traceparent = traceparent.filter(|value| traceparent_is_echo_safe(value));
    let tracestate = tracestate.filter(|value| tracestate_is_echo_safe(value));
    match (traceparent, tracestate) {
        (Some(traceparent), Some(tracestate)) => Cow::Owned(format!(
            "{{\"decision\":\"allow\",\"hookSpecificOutput\":{{\"env\":{{\"TRACEPARENT\":\"{traceparent}\",\"TRACESTATE\":\"{tracestate}\"}}}}}}"
        )),
        (Some(traceparent), None) => Cow::Owned(format!(
            "{{\"decision\":\"allow\",\"hookSpecificOutput\":{{\"env\":{{\"TRACEPARENT\":\"{traceparent}\"}}}}}}"
        )),
        (None, Some(tracestate)) => Cow::Owned(format!(
            "{{\"decision\":\"allow\",\"hookSpecificOutput\":{{\"env\":{{\"TRACESTATE\":\"{tracestate}\"}}}}}}"
        )),
        (None, None) => Cow::Borrowed("{\"decision\":\"allow\"}"),
    }
}

#[cfg(test)]
mod response_tests {
    #[cfg(not(target_os = "linux"))]
    use super::pre_tool_allow_json;

    #[test]
    fn ordinary_cargo_tests_retain_panic_unwinding() {
        assert!(std::panic::catch_unwind(|| panic!("synthetic unwind fixture")).is_err());
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn echoes_traceparent_and_safe_tracestate() {
        let traceparent = "00-11111111111111111111111111111111-2222222222222222-01";
        let tracestate = "vendor=a,other=b";
        assert_eq!(
            pre_tool_allow_json(Some(traceparent), Some(tracestate)),
            format!(
                "{{\"decision\":\"allow\",\"hookSpecificOutput\":{{\"env\":{{\"TRACEPARENT\":\"{traceparent}\",\"TRACESTATE\":\"{tracestate}\"}}}}}}"
            )
        );
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn keeps_traceparent_only_json() {
        let traceparent = "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01";
        assert_eq!(
            pre_tool_allow_json(Some(traceparent), Some("bad\"value")).as_ref(),
            format!(
                "{{\"decision\":\"allow\",\"hookSpecificOutput\":{{\"env\":{{\"TRACEPARENT\":\"{traceparent}\"}}}}}}"
            )
        );
        assert_eq!(
            pre_tool_allow_json(Some(traceparent), None).as_ref(),
            format!(
                "{{\"decision\":\"allow\",\"hookSpecificOutput\":{{\"env\":{{\"TRACEPARENT\":\"{traceparent}\"}}}}}}"
            )
        );
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn echoes_tracestate_when_traceparent_is_unsafe() {
        let tracestate = "rojo=00f067aa0ba902b7";
        assert_eq!(
            pre_tool_allow_json(Some("bad\"value\n"), Some(tracestate)).as_ref(),
            format!(
                "{{\"decision\":\"allow\",\"hookSpecificOutput\":{{\"env\":{{\"TRACESTATE\":\"{tracestate}\"}}}}}}"
            )
        );
        assert_eq!(
            pre_tool_allow_json(None, Some(tracestate)).as_ref(),
            format!(
                "{{\"decision\":\"allow\",\"hookSpecificOutput\":{{\"env\":{{\"TRACESTATE\":\"{tracestate}\"}}}}}}"
            )
        );
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn rejects_empty_control_quote_backslash_and_overlong_tracestate() {
        assert_eq!(
            pre_tool_allow_json(None, Some("")).as_ref(),
            "{\"decision\":\"allow\"}"
        );
        assert_eq!(
            pre_tool_allow_json(None, Some("has\\slash")).as_ref(),
            "{\"decision\":\"allow\"}"
        );
        assert_eq!(
            pre_tool_allow_json(None, Some("line\nbreak")).as_ref(),
            "{\"decision\":\"allow\"}"
        );
        assert_eq!(
            pre_tool_allow_json(None, Some("del\u{7f}char")).as_ref(),
            "{\"decision\":\"allow\"}"
        );
        assert_eq!(
            pre_tool_allow_json(None, Some(&"a".repeat(513))).as_ref(),
            "{\"decision\":\"allow\"}"
        );
        assert!(pre_tool_allow_json(None, Some(&"a".repeat(512))).contains("TRACESTATE"));
    }
}
