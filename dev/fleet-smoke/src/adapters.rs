//! Structured native adapter commands. Specs are never shell strings.
use crate::plan::{ANTIGRAVITY_MODEL, CODEX_MODEL, GROK_MODEL};
use command_group::CommandGroup;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub env: BTreeMap<OsString, OsString>,
    pub workspace: PathBuf,
    pub timeout: Duration,
    pub stdout_limit_bytes: usize,
    pub retry_limit: u8,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Codex,
    Grok,
    Antigravity,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterError {
    GrokShimRejected(PathBuf),
    Spawn(String),
}

impl Platform {
    pub fn model(self) -> &'static str {
        match self {
            Self::Codex => CODEX_MODEL,
            Self::Grok => GROK_MODEL,
            Self::Antigravity => ANTIGRAVITY_MODEL,
        }
    }
    pub fn program(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Grok => "grok",
            Self::Antigravity => "agy",
        }
    }
    pub fn command_spec(self, prompt: &str, traceparent: &str) -> CommandSpec {
        self.command_spec_at(prompt, traceparent, Path::new("<workspace>"), None)
            .expect("built-in program")
    }
    pub fn command_spec_at(
        self,
        prompt: &str,
        traceparent: &str,
        workspace: &Path,
        native_program: Option<&Path>,
    ) -> Result<CommandSpec, AdapterError> {
        if let Some(path) = native_program {
            if matches!(
                path.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_ascii_lowercase())
                    .as_deref(),
                Some("ps1" | "cmd" | "bat" | "sh")
            ) {
                return Err(AdapterError::GrokShimRejected(path.to_path_buf()));
            }
        }
        let program = native_program
            .unwrap_or_else(|| Path::new(self.program()))
            .to_path_buf();
        let strings: Vec<String> = match self {
            Self::Codex => vec![
                "exec",
                "-m",
                self.model(),
                "-c",
                "model_reasoning_effort=\"low\"",
                "--ephemeral",
                "--json",
                "--skip-git-repo-check",
                "-C",
                workspace.to_str().unwrap_or_default(),
                prompt,
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            Self::Grok => vec![
                "-m",
                self.model(),
                "--reasoning-effort",
                "low",
                "--output-format",
                "streaming-json",
                "--max-turns",
                "6",
                "--no-subagents",
                "--disable-web-search",
                "--cwd",
                workspace.to_str().unwrap_or_default(),
                "-p",
                prompt,
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            Self::Antigravity => vec![
                "--model",
                self.model(),
                "--effort",
                "low",
                "--output-format",
                "stream-json",
                "--print-timeout",
                "90s",
                "--print",
                prompt,
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        };
        Ok(CommandSpec {
            program: program.into_os_string(),
            args: strings.into_iter().map(OsString::from).collect(),
            env: [(OsString::from("TRACEPARENT"), OsString::from(traceparent))]
                .into_iter()
                .collect(),
            workspace: workspace.to_path_buf(),
            timeout: Duration::from_secs(90),
            stdout_limit_bytes: 256 * 1024,
            retry_limit: 0,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnOutput {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
    pub output_limited: bool,
}

/// Explicit bounded invocation. Provider calls are never made implicitly.
pub fn spawn_bounded(spec: &CommandSpec) -> Result<SpawnOutput, AdapterError> {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .current_dir(&spec.workspace)
        .envs(spec.env.clone())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .group_spawn()
        .map_err(|e| AdapterError::Spawn(e.to_string()))?;
    let stdout = child
        .inner()
        .stdout
        .take()
        .ok_or_else(|| AdapterError::Spawn("stdout pipe unavailable".into()))?;
    let stderr = child
        .inner()
        .stderr
        .take()
        .ok_or_else(|| AdapterError::Spawn("stderr pipe unavailable".into()))?;
    let limit = spec.stdout_limit_bytes;
    let (output_tx, output_rx) = std::sync::mpsc::sync_channel(2);
    let tx = output_tx.clone();
    std::thread::spawn(move || {
        let _ = tx.send((true, read_capped(stdout, limit)));
    });
    let err_limit = limit;
    std::thread::spawn(move || {
        let _ = output_tx.send((false, read_capped(stderr, err_limit)));
    });
    let deadline = Instant::now() + spec.timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let ((stdout, limited), (stderr, err_limited)) = collect_output(&output_rx);
                let _ = child.kill();
                let _ = child.wait();
                return Ok(SpawnOutput {
                    status: status.code().unwrap_or(-1),
                    stdout,
                    stderr,
                    timed_out: false,
                    output_limited: limited || err_limited,
                });
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let status = child.wait().ok();
                let ((stdout, limited), (stderr, err_limited)) = collect_output(&output_rx);
                return Ok(SpawnOutput {
                    status: status.and_then(|s| s.code()).unwrap_or(-1),
                    stdout,
                    stderr,
                    timed_out: true,
                    output_limited: limited || err_limited,
                });
            }
        }
    }
}

fn collect_output(
    rx: &std::sync::mpsc::Receiver<(bool, (Vec<u8>, bool))>,
) -> ((Vec<u8>, bool), (Vec<u8>, bool)) {
    let mut stdout = (Vec::new(), false);
    let mut stderr = (Vec::new(), false);
    for _ in 0..2 {
        if let Ok((is_stdout, output)) = rx.recv_timeout(Duration::from_millis(250)) {
            if is_stdout {
                stdout = output;
            } else {
                stderr = output;
            }
        }
    }
    (stdout, stderr)
}
fn read_capped<R: Read>(mut reader: R, limit: usize) -> (Vec<u8>, bool) {
    let mut bytes = Vec::new();
    let mut limited = false;
    let mut chunk = [0; 8192];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => return (bytes, limited),
            Ok(n) => {
                let remaining = limit.saturating_sub(bytes.len());
                bytes.extend_from_slice(&chunk[..n.min(remaining)]);
                if n > remaining {
                    limited = true;
                }
            }
            Err(_) => return (bytes, limited),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_flags() {
        let root = Path::new("C:/fixture workspace");
        let c = Platform::Codex
            .command_spec_at("p", "tp", root, None)
            .unwrap();
        assert_eq!(c.args[0], "exec");
        assert!(c.args.iter().any(|x| x == "model_reasoning_effort=\"low\""));
        assert!(c.args.iter().any(|x| x == "C:/fixture workspace"));
        let g = Platform::Grok
            .command_spec_at("p", "tp", root, None)
            .unwrap();
        assert!(g.args.iter().any(|x| x == "6"));
        let a = Platform::Antigravity
            .command_spec_at("p", "tp", root, None)
            .unwrap();
        assert!(a.args.iter().any(|x| x == "90s"));
    }
    #[test]
    fn grok_shim_rejected() {
        assert!(matches!(
            Platform::Grok.command_spec_at("p", "tp", Path::new("."), Some(Path::new("grok.ps1"))),
            Err(AdapterError::GrokShimRejected(_))
        ));
    }
    #[test]
    fn traceparent_is_per_child_env() {
        let s = Platform::Codex
            .command_spec_at("p", "tp", Path::new("."), None)
            .unwrap();
        assert_eq!(s.env.get(&OsString::from("TRACEPARENT")).unwrap(), "tp");
    }
}
