/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use std::ptr;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_IO_PENDING, ERROR_PIPE_BUSY, FALSE, HANDLE,
    INVALID_HANDLE_VALUE, TRUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_GENERIC_WRITE, OPEN_EXISTING, WriteFile,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResultEx, OVERLAPPED};
use windows_sys::Win32::System::Pipes::WaitNamedPipeW;
use windows_sys::Win32::System::Threading::CreateEventW;

use crate::frame::{encode_frame, MsgType, DEFAULT_PIPE_NAME};

const TOTAL_BUDGET: Duration = Duration::from_millis(3);
const CONNECT_BUDGET_CAP: Duration = Duration::from_millis(2);

struct HandleGuard(HANDLE);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

fn wide_pipe_name() -> Vec<u16> {
    let name = std::env::var("AGY_OTEL_PIPE").unwrap_or_else(|_| DEFAULT_PIPE_NAME.to_string());
    name.encode_utf16().chain(std::iter::once(0)).collect()
}

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

pub fn send_fire_and_forget(msg_type: MsgType, payload: &[u8]) {
    let _ = try_send(msg_type, payload);
}

pub fn try_send(msg_type: MsgType, payload: &[u8]) -> Result<(), ()> {
    let deadline = Instant::now() + TOTAL_BUDGET;
    let pipe_wide = wide_pipe_name();

    let handle = match open_pipe(&pipe_wide) {
        Some(h) => h,
        None => {
            let err = unsafe { GetLastError() };
            if err != ERROR_PIPE_BUSY {
                return Err(());
            }
            let remaining = deadline
                .saturating_duration_since(Instant::now())
                .min(CONNECT_BUDGET_CAP);
            if remaining.is_zero() {
                return Err(());
            }
            let ok = unsafe { WaitNamedPipeW(pipe_wide.as_ptr(), remaining.as_millis() as u32) };
            if ok == 0 {
                return Err(());
            }
            open_pipe(&pipe_wide).ok_or(())?
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
                CancelIoEx(handle.0, &mut overlapped);
            }
            return Err(());
        }
        let mut transferred: u32 = 0;
        let done = unsafe {
            GetOverlappedResultEx(
                handle.0,
                &mut overlapped,
                &mut transferred,
                remaining.as_millis() as u32,
                FALSE,
            )
        };
        if done == 0 {
            unsafe {
                CancelIoEx(handle.0, &mut overlapped);
            }
            return Err(());
        }
    }

    Ok(())
}
