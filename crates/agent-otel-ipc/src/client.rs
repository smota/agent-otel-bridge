/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

#[cfg(windows)]
use std::ptr;
use std::time::Duration;
#[cfg(windows)]
use std::time::Instant;

#[cfg(windows)]
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_IO_PENDING, ERROR_PIPE_BUSY, FALSE, HANDLE,
    INVALID_HANDLE_VALUE, TRUE,
};
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, WriteFile, FILE_FLAG_OVERLAPPED, FILE_GENERIC_WRITE, OPEN_EXISTING,
};
#[cfg(windows)]
use windows_sys::Win32::System::Pipes::WaitNamedPipeW;
#[cfg(windows)]
use windows_sys::Win32::System::Threading::CreateEventW;
#[cfg(windows)]
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResultEx, OVERLAPPED};

#[cfg(windows)]
use crate::frame::DEFAULT_PIPE_NAME;
use crate::frame::{encode_frame, MsgType};

const TOTAL_BUDGET: Duration = Duration::from_millis(3);
#[cfg(windows)]
const CONNECT_BUDGET_CAP: Duration = Duration::from_millis(2);

#[cfg(windows)]
struct HandleGuard(HANDLE);

#[cfg(windows)]
impl Drop for HandleGuard {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

#[cfg(windows)]
fn to_wide(name: &str) -> Vec<u16> {
    name.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn wide_pipe_name() -> Vec<u16> {
    let name = std::env::var("AGENT_OTEL_PIPE")
        .or_else(|_| std::env::var("AGY_OTEL_PIPE"))
        .unwrap_or_else(|_| DEFAULT_PIPE_NAME.to_string());
    to_wide(&name)
}

#[cfg(windows)]
fn open_pipe(pipe_wide: &[u16]) -> Option<HANDLE> {
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
    if h == INVALID_HANDLE_VALUE || h.is_null() {
        None
    } else {
        Some(h)
    }
}

#[cfg(windows)]
pub fn send_fire_and_forget(msg_type: MsgType, payload: &[u8]) {
    let _ = try_send(msg_type, payload);
}

#[cfg(windows)]
#[allow(clippy::result_unit_err)]
pub fn try_send(msg_type: MsgType, payload: &[u8]) -> Result<(), ()> {
    let deadline = Instant::now() + TOTAL_BUDGET;
    let pipe_wide = wide_pipe_name();

    let handle = match open_pipe(&pipe_wide) {
        Some(h) => h,
        None => {
            let err = unsafe { GetLastError() };
            if err == ERROR_PIPE_BUSY {
                let remaining = deadline
                    .saturating_duration_since(Instant::now())
                    .min(CONNECT_BUDGET_CAP);
                if remaining.is_zero() {
                    return Err(());
                }
                let ok =
                    unsafe { WaitNamedPipeW(pipe_wide.as_ptr(), remaining.as_millis() as u32) };
                if ok == 0 {
                    return Err(());
                }
                open_pipe(&pipe_wide).ok_or(())?
            } else if std::env::var("AGENT_OTEL_PIPE").is_err()
                && std::env::var("AGY_OTEL_PIPE").is_err()
            {
                // Fallback to legacy pipe name if default pipe is not listening
                let legacy_wide = to_wide(crate::frame::LEGACY_PIPE_NAME);
                if let Some(h) = open_pipe(&legacy_wide) {
                    h
                } else {
                    return Err(());
                }
            } else {
                return Err(());
            }
        }
    };
    let handle = HandleGuard(handle);

    let event = unsafe { CreateEventW(ptr::null(), TRUE, FALSE, ptr::null()) };
    if event.is_null() || event == INVALID_HANDLE_VALUE {
        return Err(());
    }
    let _event_guard = HandleGuard(event);

    let mut overlapped: OVERLAPPED = unsafe { core::mem::zeroed() };
    overlapped.hEvent = event;

    let frame = encode_frame(msg_type, payload);

    let write_ok = unsafe {
        WriteFile(
            handle.0,
            frame.as_ptr(),
            frame.len() as u32,
            ptr::null_mut(),
            &mut overlapped,
        )
    };

    if write_ok == 0 {
        if unsafe { GetLastError() } != ERROR_IO_PENDING {
            return Err(());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            unsafe {
                CancelIoEx(handle.0, &overlapped);
            }
            return Err(());
        }
        let mut transferred: u32 = 0;
        let done = unsafe {
            GetOverlappedResultEx(
                handle.0,
                &overlapped,
                &mut transferred,
                remaining.as_millis() as u32,
                FALSE,
            )
        };
        if done == 0 {
            unsafe {
                CancelIoEx(handle.0, &overlapped);
            }
            return Err(());
        }
    }

    Ok(())
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
#[allow(clippy::result_unit_err)]
pub fn try_send(msg_type: MsgType, payload: &[u8]) -> Result<(), ()> {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    let socket_path = std::env::var("AGENT_OTEL_SOCKET")
        .unwrap_or_else(|_| "/tmp/agent_otel_bridge.sock".to_string());
    let mut stream = UnixStream::connect(socket_path).map_err(|_| ())?;
    let _ = stream.set_write_timeout(Some(TOTAL_BUDGET));
    let frame = encode_frame(msg_type, payload);
    stream.write_all(&frame).map_err(|_| ())?;
    Ok(())
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
#[allow(clippy::result_unit_err)]
pub fn try_send(_msg_type: MsgType, _payload: &[u8]) -> Result<(), ()> {
    Ok(())
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
