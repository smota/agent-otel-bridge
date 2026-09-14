use agent_otel_fleet_smoke::adapters::{spawn_bounded, CommandSpec};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

const HELPER: &str = r#"
use std::{env, process::{Command, Stdio}, thread, time::Duration};
fn main() {
    let mode = env::args().nth(1).unwrap_or_default();
    match mode.as_str() {
        "success" => { println!("helper-stdout"); eprintln!("helper-stderr"); }
        "timeout" => thread::sleep(Duration::from_secs(30)),
        "flood" => { print!("{}", "x".repeat(1024 * 1024)); }
        "descendant" => { let exe = env::current_exe().unwrap(); let _ = Command::new(exe).arg("hold").stdin(Stdio::null()).stdout(Stdio::inherit()).stderr(Stdio::inherit()).spawn(); }
        "hold" => thread::sleep(Duration::from_secs(30)),
        _ => std::process::exit(2),
    }
}
"#;

fn helper() -> PathBuf {
    let dir = tempfile::Builder::new()
        .prefix("agent-otel-adapter-test-")
        .tempdir()
        .unwrap()
        .keep();
    let source = dir.join("helper.rs");
    let binary = dir.join(if cfg!(windows) {
        "helper.exe"
    } else {
        "helper"
    });
    fs::write(&source, HELPER).unwrap();
    let result = Command::new("rustc")
        .arg("--edition=2021")
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "helper compile failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    binary
}

fn spec(binary: &std::path::Path, mode: &str, timeout: Duration, cap: usize) -> CommandSpec {
    CommandSpec {
        program: binary.to_path_buf().into_os_string(),
        args: vec![OsString::from(mode)],
        env: BTreeMap::new(),
        workspace: binary.parent().unwrap().to_path_buf(),
        timeout,
        stdout_limit_bytes: cap,
        retry_limit: 0,
    }
}

fn cleanup(path: &std::path::Path) {
    for _ in 0..20 {
        if fs::remove_dir_all(path.parent().unwrap()).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("helper directory remained locked: {}", path.display());
}

#[test]
fn success_preserves_stdout_and_stderr() {
    let binary = helper();
    let output = spawn_bounded(&spec(&binary, "success", Duration::from_secs(2), 1024)).unwrap();
    assert_eq!(output.status, 0);
    assert!(String::from_utf8_lossy(&output.stdout).contains("helper-stdout"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("helper-stderr"));
    cleanup(&binary);
}

#[test]
fn timeout_returns_bounded_and_kills_group() {
    let binary = helper();
    let started = Instant::now();
    let output =
        spawn_bounded(&spec(&binary, "timeout", Duration::from_millis(100), 1024)).unwrap();
    assert!(output.timed_out);
    assert!(started.elapsed() < Duration::from_secs(2));
    cleanup(&binary);
}

#[test]
fn flood_is_capped_and_flagged() {
    let binary = helper();
    let output = spawn_bounded(&spec(&binary, "flood", Duration::from_secs(2), 4096)).unwrap();
    assert!(output.output_limited);
    assert!(output.stdout.len() <= 4096);
    cleanup(&binary);
}

#[test]
fn descendant_inherited_pipes_do_not_hold_runner() {
    let binary = helper();
    let started = Instant::now();
    let output = spawn_bounded(&spec(&binary, "descendant", Duration::from_secs(2), 1024)).unwrap();
    assert_eq!(output.status, 0);
    assert!(started.elapsed() < Duration::from_secs(2));
    cleanup(&binary);
}
