use agent_otel_bridge::hooks::{
    format_cmd_hook_command, format_hook_command, format_powershell_hook_command,
};

#[test]
fn renderers_keep_arguments_and_stdin_contract_visible() {
    let path = r#"C:\Program Files\Agent Bridge\agent-hook.exe"#;
    assert_eq!(
        format_hook_command(path, "PreToolUse", None),
        r#""C:\Program Files\Agent Bridge\agent-hook.exe" PreToolUse"#
    );
    assert_eq!(
        format_powershell_hook_command(path, "PreToolUse", Some("grok")),
        r#"& "C:\Program Files\Agent Bridge\agent-hook.exe" PreToolUse --client grok"#
    );
    assert_eq!(
        format_cmd_hook_command(path, "PreToolUse", None),
        r#"call "C:\Program Files\Agent Bridge\agent-hook.exe" PreToolUse"#
    );
}

#[cfg(windows)]
#[test]
fn powershell_executes_owned_fixture_with_json_stdin() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let dir = std::env::temp_dir().join(format!("adapter_fixture_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fixture = dir.join("hook fixture $&.exe");
    let source = dir.join("fixture.rs");
    std::fs::write(&source, "use std::io::{self,Read}; fn main(){let mut s=String::new();io::stdin().read_to_string(&mut s).unwrap(); print!(\"{} {}\",std::env::args().skip(1).collect::<Vec<_>>().join(\" \"),s);}").unwrap();
    assert!(Command::new("rustc")
        .args([source.to_str().unwrap(), "-o", fixture.to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    let command =
        format_powershell_hook_command(&fixture.to_string_lossy(), "PreToolUse", Some("grok"));
    let mut child = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &command])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"event":"PreToolUse"}"#)
        .unwrap();
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let output = child.wait_with_output().unwrap();
            assert_eq!(status, output.status);
            assert!(status.success());
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(stdout.contains("PreToolUse --client grok"));
            assert!(stdout.contains(r#"{"event":"PreToolUse"}"#));
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("PowerShell adapter fixture exceeded 3 second timeout");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(windows)]
#[test]
fn cmd_executes_owned_native_fixture_with_json_stdin() {
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let dir = std::env::temp_dir().join(format!("adapter_native_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("fixture.rs");
    let fixture = dir.join("hook fixture $&.exe");
    std::fs::write(
        &source,
        "use std::io::{self,Read}; fn main(){let mut s=String::new();io::stdin().read_to_string(&mut s).unwrap(); print!(\"{} {}\",std::env::args().skip(1).collect::<Vec<_>>().join(\" \"),s);}",
    )
    .unwrap();
    let status = Command::new("rustc")
        .args([source.to_str().unwrap(), "-o", fixture.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success());
    let command = format_cmd_hook_command(&fixture.to_string_lossy(), "PreToolUse", None);
    let mut child = Command::new("cmd.exe")
        .raw_arg(format!("/d /s /c {command}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"event":"PreToolUse"}"#)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let output = child.wait_with_output().unwrap();
            assert_eq!(status, output.status);
            assert!(status.success());
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(stdout.contains("PreToolUse"));
            assert!(stdout.contains(r#"{"event":"PreToolUse"}"#));
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("CMD adapter fixture exceeded 3 second timeout");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = std::fs::remove_dir_all(dir);
}
