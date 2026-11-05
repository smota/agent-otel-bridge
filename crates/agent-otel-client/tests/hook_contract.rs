/*
 * Copyright agent-otel-bridge contributors
 * SPDX-License-Identifier: Apache-2.0
 */

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn isolated_socket() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "aob-contract-{}-{}.sock",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos()
    ))
}

fn spawn_hook(event: &str, client: &str, watchdog_ms: u64) -> Child {
    let pipe_name = format!(
        r"\\.\pipe\agent-otel-hook-contract-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos()
    );
    Command::new(env!("CARGO_BIN_EXE_agent-hook"))
        .env("AGENT_OTEL_SOCKET", isolated_socket())
        .arg(event)
        .arg("--client")
        .arg(client)
        .env("AGENT_OTEL_PIPE", pipe_name)
        .env("AGENT_OTEL_WATCHDOG_MS", watchdog_ms.to_string())
        .env_remove("TRACEPARENT")
        .env_remove("TRACESTATE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn candidate agent-hook")
}

fn read_exact_response(child: &mut Child, expected: &[u8]) {
    let mut actual = vec![0u8; expected.len()];
    child
        .stdout
        .as_mut()
        .expect("hook stdout")
        .read_exact(&mut actual)
        .expect("response must be available while stdin remains open");
    assert_eq!(actual, expected);
}

#[test]
fn traceparent_response_is_safe_and_preserves_context() {
    let traceparent = "00-11111111111111111111111111111111-2222222222222222-01";
    for (client, value, expected) in [
        ("claude", traceparent, format!("{{\"decision\":\"allow\",\"hookSpecificOutput\":{{\"env\":{{\"TRACEPARENT\":\"{traceparent}\"}}}}}}")),
        ("claude", "bad\"value\n", "{\"decision\":\"allow\"}".to_string()),
        ("codex", traceparent, "{}".to_string()),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_agent-hook"))
        .env("AGENT_OTEL_SOCKET", isolated_socket())
            .args(["PreToolUse", "--client", client])
            .env("TRACEPARENT", value)
            .env_remove("TRACESTATE")
            .env("AGENT_OTEL_WATCHDOG_MS", "500")
            .env("AGENT_OTEL_PIPE", format!(r"\\.\pipe\aob-response-test-{}", std::process::id()))
            .stdin(Stdio::null())
            .output()
            .expect("run candidate hook");
        assert!(output.status.success());
        assert_eq!(output.stdout, expected.as_bytes());
    }
}

#[test]
fn response_is_published_before_stdin_and_only_once() {
    let cases: &[(&str, &[u8])] = &[
        ("antigravity", b"{\"decision\":\"allow\"}"),
        ("claude", b"{\"decision\":\"allow\"}"),
        ("grok", b"{\"decision\":\"allow\"}"),
        ("pi", b"{\"decision\":\"allow\"}"),
        ("codex", b"{}"),
    ];

    for (client, expected) in cases {
        let mut child = spawn_hook("PreToolUse", client, 500);
        let started = Instant::now();
        read_exact_response(&mut child, expected);
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "{client} response was not published ahead of stdin"
        );

        child
            .stdin
            .as_mut()
            .expect("hook stdin")
            .write_all(b"{}")
            .expect("write hook payload");
        drop(child.stdin.take());
        let status = child.wait().expect("wait for hook");
        assert!(status.success(), "{client} hook did not fail open");

        let mut trailing = Vec::new();
        child
            .stdout
            .take()
            .expect("hook stdout")
            .read_to_end(&mut trailing)
            .expect("read trailing output");
        assert!(trailing.is_empty(), "{client} wrote a second response");
    }
}

#[test]
fn watchdog_terminates_without_becoming_a_second_writer() {
    let mut child = spawn_hook("PostToolUse", "codex", 50);
    read_exact_response(&mut child, b"{}");

    // Keep stdin open so the main thread cannot reach transport or normal exit.
    let status = child.wait().expect("wait for watchdog termination");
    assert!(status.success());
    drop(child.stdin.take());

    let mut trailing = Vec::new();
    child
        .stdout
        .take()
        .expect("hook stdout")
        .read_to_end(&mut trailing)
        .expect("read trailing output");
    assert!(trailing.is_empty(), "watchdog wrote duplicate JSON");
}

#[cfg(target_os = "linux")]
mod linux_deadlines {
    use super::*;
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;

    fn wait_bounded(child: &mut Child) -> std::process::ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(status) = child.try_wait().expect("poll candidate hook") {
                return status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("hook did not fail open within subprocess cleanup deadline");
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn default_watchdog_exits_zero_while_stdin_is_open() {
        let mut child = Command::new(env!("CARGO_BIN_EXE_agent-hook"))
            .env("AGENT_OTEL_SOCKET", isolated_socket())
            .args(["PostToolUse", "--client", "codex"])
            .env_remove("AGENT_OTEL_WATCHDOG_MS")
            .env_remove("TRACEPARENT")
            .env_remove("TRACESTATE")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn default-deadline hook");
        assert!(wait_bounded(&mut child).success());
    }

    #[test]
    fn watchdog_exits_zero_when_stdout_has_no_capacity() {
        let (_reader, mut writer) = UnixStream::pair().expect("owned stdout fixture");
        writer.set_nonblocking(true).unwrap();
        let bytes = [0u8; 4096];
        while writer.write(&bytes).is_ok() {}
        while writer.write(&[0u8]).is_ok() {}
        writer.set_nonblocking(false).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_agent-hook"))
            .env("AGENT_OTEL_SOCKET", isolated_socket())
            .args(["PostToolUse", "--client", "codex"])
            .env("AGENT_OTEL_WATCHDOG_MS", "25")
            .stdin(Stdio::null())
            .stdout(Stdio::from(OwnedFd::from(writer)))
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn blocked-stdout hook");
        assert!(wait_bounded(&mut child).success());
    }

    #[test]
    fn failed_watchdog_creation_exits_zero_without_a_response() {
        use std::os::unix::process::CommandExt;
        // Root is exempt from RLIMIT_NPROC; this fixture needs a normal user.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-hook"));
        command.env("AGENT_OTEL_SOCKET", isolated_socket());
        command
            .args(["PostToolUse", "--client", "codex"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // SAFETY: setrlimit is async-signal-safe and changes only this child's
        // process limit before exec. No parent or sibling limit is changed.
        unsafe {
            command.pre_exec(|| {
                let limit = libc::rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                if libc::setrlimit(libc::RLIMIT_NPROC, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().expect("spawn restricted candidate");
        let status = wait_bounded(&mut child);
        let mut response = Vec::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_end(&mut response)
            .unwrap();
        assert!(status.success(), "watchdog creation failure must fail open");
        assert!(
            response.is_empty(),
            "do not risk blocking stdout without a watchdog"
        );
    }

    #[test]
    fn invalid_utf8_argument_keeps_valid_event_and_fails_open() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let output = Command::new(env!("CARGO_BIN_EXE_agent-hook"))
            .env("AGENT_OTEL_SOCKET", isolated_socket())
            .arg(OsStr::from_bytes(b"invalid-\xff"))
            .args(["PostToolUse", "--client", "codex"])
            .env("AGENT_OTEL_WATCHDOG_MS", "500")
            .stdin(Stdio::null())
            .output()
            .expect("run hook with an invalid UTF-8 argument");
        assert!(output.status.success(), "invalid argv must not panic");
        assert_eq!(output.stdout, b"{}");
    }
}
