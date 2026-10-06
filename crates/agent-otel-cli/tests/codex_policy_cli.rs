#[cfg(windows)]
#[test]
fn invalid_codex_policy_returns_failure_without_changing_hooks() {
    use std::{fs, process::Command};
    let dir = std::env::temp_dir().join(format!("codex_policy_cli_{}", std::process::id()));
    let config = dir.join(".codex");
    fs::create_dir_all(&config).unwrap();
    let hooks = config.join("hooks.json");
    let policy = config.join("agent-otel-bridge-policy.json");
    let original = br#"{"hooks":{},"custom":"preserve"}"#;
    fs::write(&hooks, original).unwrap();
    fs::write(
        &policy,
        br#"{"version":99,"windows_hook_shell":"powershell"}"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_agent-otel-bridge"))
        .args([
            "install-hooks",
            "--client",
            "codex",
            "--binary",
            "C:/Bridge/agent-hook.exe",
        ])
        .env("USERPROFILE", &dir)
        .env("HOME", &dir)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(fs::read(&hooks).unwrap(), original);
    assert!(String::from_utf8_lossy(&output.stderr).contains("Hook installation incomplete"));
    fs::remove_file(hooks).unwrap();
    fs::remove_file(policy).unwrap();
    fs::remove_dir(config).unwrap();
    fs::remove_dir(dir).unwrap();
}
