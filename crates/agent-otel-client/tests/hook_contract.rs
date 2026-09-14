/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

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
        .arg(event)
        .arg("--client")
        .arg(client)
        .env("AGENT_OTEL_PIPE", pipe_name)
        .env("AGENT_OTEL_WATCHDOG_MS", watchdog_ms.to_string())
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
