/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::stats::Stats;

const HOOK_PAYLOAD: &str =
    r#"{"conversationId":"bench-session","stepIdx":10,"toolCall":{"name":"run_command"}}"#;

pub fn find_agent_hook_binary() -> PathBuf {
    // Check release dir, then debug dir, then PATH
    let release_path = PathBuf::from("target/release/agent-hook.exe");
    if release_path.exists() {
        return release_path;
    }
    let debug_path = PathBuf::from("target/debug/agent-hook.exe");
    if debug_path.exists() {
        return debug_path;
    }
    PathBuf::from("agent-hook.exe")
}

pub fn run(iterations: usize, quiet: bool) -> Result<Stats, Box<dyn std::error::Error>> {
    let bin_path = find_agent_hook_binary();
    if !quiet {
        println!("  [spawn-bench] Using binary: {:?}", bin_path);
    }

    let mut samples = Vec::with_capacity(iterations);

    // Warm-up iteration
    let _ = Command::new(&bin_path)
        .arg("PostToolUse")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .and_then(|mut c| {
            if let Some(mut sin) = c.stdin.take() {
                let _ = sin.write_all(HOOK_PAYLOAD.as_bytes());
            }
            c.wait()
        });

    for _ in 0..iterations {
        let start = Instant::now();

        let mut child = Command::new(&bin_path)
            .arg("PostToolUse")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        if let Some(mut sin) = child.stdin.take() {
            let _ = sin.write_all(HOOK_PAYLOAD.as_bytes());
        }

        let _ = child.wait()?;
        let elapsed = start.elapsed();

        samples.push(elapsed.as_micros() as u64);
    }

    Ok(Stats::new(samples))
}
