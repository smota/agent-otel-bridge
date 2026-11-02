/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use std::fs;
use std::process::Command;

fn isolated_home() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "install_hooks_gate_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_agent-otel-bridge"))
        .args(args)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .output()
        .unwrap()
}

#[test]
fn install_hooks_without_legacy_config_exits_2_and_writes_nothing() {
    let home = isolated_home();
    for args in [
        &["install-hooks", "--client", "pi", "--binary", "agent-hook"][..],
        &["hooks", "install", "--client", "claude"][..],
    ] {
        let output = run(&home, args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("Run `agent-otel-bridge local enable --app <client>`"),
            "{stdout}"
        );
        assert!(
            !home.join(".pi").exists() && !home.join(".claude").exists(),
            "home was modified: {}",
            home.display()
        );
    }
    fs::remove_dir_all(home).unwrap();
}
