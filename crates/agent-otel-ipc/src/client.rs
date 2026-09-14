/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

#[cfg(windows)]
use std::ptr;
use std::time::Duration;
use std::time::Instant;

#[cfg(windows)]
use windows_sys::Win32::Foundation::{GetLastError, ERROR_PIPE_BUSY, ERROR_SEM_TIMEOUT};
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_GENERIC_WRITE, OPEN_EXISTING,
};
#[cfg(windows)]
use windows_sys::Win32::System::Pipes::WaitNamedPipeW;
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};

#[cfg(windows)]
use crate::frame::DEFAULT_PIPE_NAME;
use crate::frame::{encode_frame, MsgType};

#[cfg(windows)]
#[path = "pending_write.rs"]
mod pending_write;
#[cfg(windows)]
use pending_write::{create_event, write_until, OwnedHandle};
#[cfg(windows)]
pub use pending_write::{DrainStatus, PendingWrite};

const TOTAL_BUDGET: Duration = Duration::from_millis(3);
const MAX_TRACEPARENT_BYTES: usize = 512;
#[cfg(windows)]
const CONNECT_BUDGET_CAP: Duration = Duration::from_millis(2);

/// Stage at which a synchronous client attempt failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SendStage {
    Connect,
    WaitForPipe,
    Reopen,
    CreateEvent,
    FrameTooLarge,
    Submit,
    AwaitCompletion,
    ObserveCompletion,
    Deadline,
    ShortWrite,
}

/// Compact transport error; callers decide whether and where to report it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendError {
    pub stage: SendStage,
    pub os_code: u32,
    pub cancel_code: u32,
    pub expected_bytes: u32,
    pub transferred_bytes: u32,
}

impl SendError {
    pub(crate) const fn new(stage: SendStage, os_code: u32) -> Self {
        Self {
            stage,
            os_code,
            cancel_code: 0,
            expected_bytes: 0,
            transferred_bytes: 0,
        }
    }

    pub(crate) const fn short_write(expected_bytes: u32, transferred_bytes: u32) -> Self {
        Self {
            stage: SendStage::ShortWrite,
            os_code: 0,
            cancel_code: 0,
            expected_bytes,
            transferred_bytes,
        }
    }
}

/// Result of an attempt whose delivery deadline is distinct from cleanup.
#[derive(Debug)]
pub enum SendAttempt {
    Complete(Result<usize, SendError>),
    #[cfg(windows)]
    CleanupRequired {
        error: SendError,
        pending: PendingWrite,
    },
}

/// Compatibility wrapper retained for existing CLI and benchmark callers.
///
/// Detailed transport diagnostics are available through [`try_send_detailed`].
/// This legacy boundary intentionally erases them because its established
/// contract exposes only success or failure.
#[allow(clippy::result_unit_err)]
pub fn try_send(msg_type: MsgType, payload: &[u8]) -> Result<(), ()> {
    try_send_detailed(msg_type, payload).map_err(|_| ())
}

/// Read the hook process's TRACEPARENT without an unbounded environment
/// allocation. Values over 512 UTF-8 bytes or invalid UTF-8 are omitted.
///
/// # Safety
///
/// On Unix, the caller must ensure no thread concurrently mutates the process
/// environment while this function reads the pointer returned by `getenv`.
pub unsafe fn read_traceparent() -> Option<String> {
    #[cfg(windows)]
    {
        const CAP: u32 = (MAX_TRACEPARENT_BYTES + 1) as u32;
        let name: [u16; 12] = [
            'T' as u16, 'R' as u16, 'A' as u16, 'C' as u16, 'E' as u16, 'P' as u16, 'A' as u16,
            'R' as u16, 'E' as u16, 'N' as u16, 'T' as u16, 0,
        ];
        let mut buffer = [0u16; MAX_TRACEPARENT_BYTES + 1];
        let len = unsafe { GetEnvironmentVariableW(name.as_ptr(), buffer.as_mut_ptr(), CAP) };
        if len == 0 || len >= CAP {
            return None;
        }
        let value = String::from_utf16(&buffer[..len as usize]).ok()?;
        accept_traceparent_bytes(value.as_bytes())
    }

    #[cfg(unix)]
    {
        let ptr = unsafe { getenv(c"TRACEPARENT".as_ptr()) };
        if ptr.is_null() {
            return None;
        }
        let mut bytes = [0u8; MAX_TRACEPARENT_BYTES + 1];
        let mut len = 0;
        while len < bytes.len() {
            let byte = unsafe { ptr.cast::<u8>().add(len).read() };
            if byte == 0 {
                return accept_traceparent_bytes(&bytes[..len]);
            }
            bytes[len] = byte;
            len += 1;
        }
        None
    }

    #[cfg(not(any(windows, unix)))]
    None
}

#[cfg(test)]
mod traceparent_tests {
    use super::accept_traceparent_bytes;

    #[test]
    fn accepts_at_byte_limit() {
        assert!(accept_traceparent_bytes(&[b'a'; 512]).is_some());
    }

    #[test]
    fn omits_oversized_without_truncating() {
        assert!(accept_traceparent_bytes(&[b'a'; 513]).is_none());
    }

    #[test]
    fn omits_invalid_utf8() {
        assert!(accept_traceparent_bytes(&[0xff]).is_none());
    }
}

fn accept_traceparent_bytes(bytes: &[u8]) -> Option<String> {
    if bytes.len() > MAX_TRACEPARENT_BYTES {
        return None;
    }
    std::str::from_utf8(bytes).ok().map(str::to_owned)
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetEnvironmentVariableW(name: *const u16, value: *mut u16, size: u32) -> u32;
}

#[cfg(unix)]
unsafe extern "C" {
    fn getenv(name: *const std::ffi::c_char) -> *const std::ffi::c_char;
}

#[cfg(windows)]
fn to_wide(name: &str) -> Vec<u16> {
    name.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn resolve_pipe_name(endpoint: Option<&str>) -> (Vec<u16>, bool) {
    if let Some(endpoint) = endpoint {
        return (to_wide(endpoint), false);
    }
    if let Ok(name) = std::env::var("AGENT_OTEL_PIPE") {
        return (to_wide(&name), false);
    }
    if let Ok(name) = std::env::var("AGY_OTEL_PIPE") {
        return (to_wide(&name), false);
    }
    (to_wide(DEFAULT_PIPE_NAME), true)
}

#[cfg(windows)]
fn open_pipe(pipe_wide: &[u16]) -> Result<OwnedHandle, u32> {
    let h = unsafe {
        CreateFileW(
            pipe_wide.as_ptr(),
            FILE_GENERIC_WRITE,
            0,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            std::ptr::null_mut(),
        )
    };
    OwnedHandle::from_raw(h).ok_or_else(|| unsafe { GetLastError() })
}

#[cfg(windows)]
fn open_pipe_until(
    pipe_wide: &[u16],
    absolute_deadline: Instant,
) -> Result<OwnedHandle, SendError> {
    let mut reopening = false;
    loop {
        if Instant::now() >= absolute_deadline {
            return Err(SendError::new(SendStage::Deadline, 0));
        }
        match open_pipe(pipe_wide) {
            Ok(handle) => return Ok(handle),
            Err(ERROR_PIPE_BUSY) => reopening = true,
            Err(code) => {
                return Err(SendError::new(
                    if reopening {
                        SendStage::Reopen
                    } else {
                        SendStage::Connect
                    },
                    code,
                ));
            }
        }

        let remaining = absolute_deadline
            .saturating_duration_since(Instant::now())
            .min(CONNECT_BUDGET_CAP);
        let Some(wait_ms) = wait_named_pipe_millis(remaining) else {
            if remaining.is_zero() {
                return Err(SendError::new(SendStage::Deadline, 0));
            }
            // WaitNamedPipeW interprets zero as the server's default timeout.
            // For the final sub-millisecond remainder, yield and retry CreateFileW.
            std::thread::yield_now();
            continue;
        };
        let ok = unsafe { WaitNamedPipeW(pipe_wide.as_ptr(), wait_ms) };
        if ok == 0 {
            let code = unsafe { GetLastError() };
            if code == ERROR_SEM_TIMEOUT && Instant::now() < absolute_deadline {
                continue;
            }
            return Err(SendError::new(SendStage::WaitForPipe, code));
        }
        // Multiple waiters may wake for one available instance. Reopening in
        // the loop lets losers observe BUSY and wait again on the same budget.
    }
}

#[cfg(windows)]
pub fn send_fire_and_forget(msg_type: MsgType, payload: &[u8]) {
    let _ = try_send(msg_type, payload);
}

#[cfg(windows)]
pub fn attempt_send_until(
    endpoint: Option<&str>,
    msg_type: MsgType,
    payload: &[u8],
    absolute_deadline: Instant,
) -> SendAttempt {
    let (pipe_wide, allow_legacy_fallback) = resolve_pipe_name(endpoint);
    if Instant::now() >= absolute_deadline {
        return SendAttempt::Complete(Err(SendError::new(SendStage::Deadline, 0)));
    }

    let pipe = match open_pipe_until(&pipe_wide, absolute_deadline) {
        Ok(handle) => handle,
        Err(error) => {
            if allow_legacy_fallback && error.stage == SendStage::Connect {
                // Fallback to legacy pipe name if default pipe is not listening
                let legacy_wide = to_wide(crate::frame::LEGACY_PIPE_NAME);
                match open_pipe_until(&legacy_wide, absolute_deadline) {
                    Ok(handle) => handle,
                    Err(legacy_error) => {
                        return SendAttempt::Complete(Err(legacy_error));
                    }
                }
            } else {
                return SendAttempt::Complete(Err(error));
            }
        }
    };

    if Instant::now() >= absolute_deadline {
        return SendAttempt::Complete(Err(SendError::new(SendStage::Deadline, 0)));
    }
    let frame = encode_frame(msg_type, payload);
    if Instant::now() >= absolute_deadline {
        return SendAttempt::Complete(Err(SendError::new(SendStage::Deadline, 0)));
    }
    let event = match create_event() {
        Ok(event) => event,
        Err(error) => return SendAttempt::Complete(Err(error)),
    };
    write_until(pipe, event, frame, absolute_deadline)
}

/// Attempt delivery for up to three milliseconds, then safely drain any
/// canceled Win32 operation before returning.
///
/// The budget limits the delivery attempt, not return latency: Windows does not
/// make `CancelIoEx` synchronous.  Process-disposable hooks use
/// [`attempt_send_until`] so they can retain pending ownership through exit.
#[cfg(windows)]
pub fn try_send_detailed(msg_type: MsgType, payload: &[u8]) -> Result<(), SendError> {
    match attempt_send_until(None, msg_type, payload, Instant::now() + TOTAL_BUDGET) {
        SendAttempt::Complete(result) => result.map(|_| ()),
        SendAttempt::CleanupRequired { error, pending } => {
            let _ = pending.drain();
            Err(error)
        }
    }
}

#[cfg(windows)]
fn wait_named_pipe_millis(remaining: Duration) -> Option<u32> {
    let millis = remaining.as_millis().min(u32::MAX as u128) as u32;
    (millis != 0).then_some(millis)
}

#[cfg(all(test, windows))]
mod deadline_tests {
    use super::wait_named_pipe_millis;
    use std::time::Duration;

    #[test]
    fn named_pipe_wait_never_reinterprets_submillisecond_as_default() {
        assert_eq!(wait_named_pipe_millis(Duration::ZERO), None);
        assert_eq!(wait_named_pipe_millis(Duration::from_nanos(999_999)), None);
        assert_eq!(wait_named_pipe_millis(Duration::from_millis(1)), Some(1));
        assert_eq!(
            wait_named_pipe_millis(Duration::from_micros(2_900)),
            Some(2)
        );
    }
}

/// Terminate the disposable hook process without taking Rust stdout or logging
/// locks.  The function does not unwind, so a live `PendingWrite` remains owned
/// until Windows tears down the process.
#[cfg(windows)]
pub fn terminate_current_process(exit_code: u32) -> ! {
    let process = unsafe { GetCurrentProcess() };
    let terminated = unsafe { TerminateProcess(process, exit_code) };
    if terminated == 0 {
        std::process::exit(exit_code as i32);
    }
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(windows)]
pub fn spawn_daemon_detached() {
    static LAST_SPAWN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let prev = LAST_SPAWN.load(std::sync::atomic::Ordering::Relaxed);
    if now.saturating_sub(prev) < 5 {
        return;
    }
    LAST_SPAWN.store(now, std::sync::atomic::Ordering::Relaxed);

    if let Some(exe) = find_bridge_binary() {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        let _ = std::process::Command::new(exe)
            .arg("daemon")
            .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

#[cfg(unix)]
pub fn send_fire_and_forget(msg_type: MsgType, payload: &[u8]) {
    let _ = try_send(msg_type, payload);
}

#[cfg(unix)]
pub fn attempt_send_until(
    endpoint: Option<&str>,
    msg_type: MsgType,
    payload: &[u8],
    absolute_deadline: Instant,
) -> SendAttempt {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    let socket_path = endpoint.map(str::to_owned).unwrap_or_else(|| {
        std::env::var("AGENT_OTEL_SOCKET")
            .unwrap_or_else(|_| "/tmp/agent_otel_bridge.sock".to_string())
    });
    let mut stream = match UnixStream::connect(socket_path) {
        Ok(stream) => stream,
        Err(error) => {
            return SendAttempt::Complete(Err(SendError::new(
                SendStage::Connect,
                error.raw_os_error().unwrap_or_default() as u32,
            )));
        }
    };
    let remaining = absolute_deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return SendAttempt::Complete(Err(SendError::new(SendStage::Deadline, 0)));
    }
    let _ = stream.set_write_timeout(Some(remaining));
    let frame = encode_frame(msg_type, payload);
    match stream.write_all(&frame) {
        Ok(()) => SendAttempt::Complete(Ok(frame.len())),
        Err(error) => SendAttempt::Complete(Err(SendError::new(
            SendStage::Submit,
            error.raw_os_error().unwrap_or_default() as u32,
        ))),
    }
}

#[cfg(unix)]
pub fn try_send_detailed(msg_type: MsgType, payload: &[u8]) -> Result<(), SendError> {
    match attempt_send_until(None, msg_type, payload, Instant::now() + TOTAL_BUDGET) {
        SendAttempt::Complete(result) => result.map(|_| ()),
    }
}

#[cfg(unix)]
pub fn terminate_current_process(exit_code: u32) -> ! {
    std::process::exit(exit_code as i32)
}

#[cfg(unix)]
pub fn spawn_daemon_detached() {
    static LAST_SPAWN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let prev = LAST_SPAWN.load(std::sync::atomic::Ordering::Relaxed);
    if now.saturating_sub(prev) < 5 {
        return;
    }
    LAST_SPAWN.store(now, std::sync::atomic::Ordering::Relaxed);

    if let Some(exe) = find_bridge_binary() {
        use std::os::unix::process::CommandExt;
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("daemon")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        cmd.process_group(0);
        let _ = cmd.spawn();
    }
}

#[cfg(not(any(windows, unix)))]
pub fn send_fire_and_forget(_msg_type: MsgType, _payload: &[u8]) {}

#[cfg(not(any(windows, unix)))]
pub fn attempt_send_until(
    _endpoint: Option<&str>,
    _msg_type: MsgType,
    payload: &[u8],
    _absolute_deadline: Instant,
) -> SendAttempt {
    SendAttempt::Complete(Ok(payload.len()))
}

#[cfg(not(any(windows, unix)))]
pub fn try_send_detailed(msg_type: MsgType, payload: &[u8]) -> Result<(), SendError> {
    match attempt_send_until(None, msg_type, payload, Instant::now() + TOTAL_BUDGET) {
        SendAttempt::Complete(result) => result.map(|_| ()),
    }
}

#[cfg(not(any(windows, unix)))]
pub fn terminate_current_process(exit_code: u32) -> ! {
    std::process::exit(exit_code as i32)
}

#[cfg(not(any(windows, unix)))]
pub fn spawn_daemon_detached() {}

pub fn find_bridge_binary() -> Option<std::path::PathBuf> {
    // 1. Explicit override via AGENT_OTEL_BRIDGE_BIN
    if let Ok(explicit) = std::env::var("AGENT_OTEL_BRIDGE_BIN") {
        let p = std::path::PathBuf::from(explicit);
        if p.is_file() {
            return Some(p);
        }
    }

    // 2. Sibling in the same directory as current running executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let neighbor = dir.join(if cfg!(windows) {
                "agent-otel-bridge.exe"
            } else {
                "agent-otel-bridge"
            });
            if neighbor.is_file() {
                return Some(neighbor);
            }
        }
    }

    // 3. Isolated canonical local runtime installation directory
    let bin_name = if cfg!(windows) {
        "agent-otel-bridge.exe"
    } else {
        "agent-otel-bridge"
    };

    if let Ok(home) = std::env::var("AGENT_OTEL_HOME") {
        let candidate = std::path::PathBuf::from(home).join("bin").join(bin_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    #[cfg(windows)]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let candidate = std::path::PathBuf::from(local_app_data)
                .join("agent-otel-bridge")
                .join("bin")
                .join(bin_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        } else if let Ok(user_profile) = std::env::var("USERPROFILE") {
            let candidate = std::path::PathBuf::from(user_profile)
                .join("AppData")
                .join("Local")
                .join("agent-otel-bridge")
                .join("bin")
                .join(bin_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    #[cfg(unix)]
    {
        if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
            let candidate = std::path::PathBuf::from(xdg_data)
                .join("agent-otel-bridge")
                .join("bin")
                .join(bin_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        } else if let Ok(home) = std::env::var("HOME") {
            let candidate = std::path::PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("agent-otel-bridge")
                .join("bin")
                .join(bin_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    // 4. System PATH search
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(bin_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    // 5. Explicit developer mode ONLY (never in production)
    let dev_mode = std::env::var("AGENT_OTEL_DEV_MODE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if dev_mode {
        if let Ok(dev_dir) = std::env::var("AGENT_OTEL_DEV_DIR") {
            for rel in &["target/release", "target/debug"] {
                let candidate = std::path::PathBuf::from(&dev_dir).join(rel).join(bin_name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        for rel in &["target/release", "target/debug"] {
            let candidate = std::path::PathBuf::from(rel).join(bin_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}
