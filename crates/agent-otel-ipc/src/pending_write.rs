/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

//! Ownership for one submitted Win32 overlapped write.
//!
//! Windows may continue to reference both the data buffer and `OVERLAPPED`
//! after `CancelIoEx` returns.  This module keeps every referenced allocation
//! and handle under one stable owner until `GetOverlappedResult` observes a
//! terminal result.

use std::fmt;
use std::ptr;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_OPERATION_ABORTED, FALSE, HANDLE, INVALID_HANDLE_VALUE, TRUE,
};
use windows_sys::Win32::Storage::FileSystem::WriteFile;
use windows_sys::Win32::System::Threading::CreateEventW;
use windows_sys::Win32::System::IO::{
    CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED,
};

use super::{SendAttempt, SendError, SendStage};

/// Completion observed while reclaiming a canceled or uncertain write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainStatus {
    Completed(usize),
    Cancelled,
    Failed(u32),
}

/// A Win32 handle with unique close ownership.
pub(crate) struct OwnedHandle(HANDLE);

impl OwnedHandle {
    pub(crate) fn from_raw(handle: HANDLE) -> Option<Self> {
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            None
        } else {
            Some(Self(handle))
        }
    }

    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: `OwnedHandle` is constructed only for a valid uniquely owned
        // handle and never exposes a close operation elsewhere.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OperationState {
    Submitted,
    CancelRequested,
    Completed,
}

impl OperationState {
    fn cancel_requested(self) -> Self {
        debug_assert_eq!(self, Self::Submitted);
        Self::CancelRequested
    }

    fn completed(self) -> Self {
        debug_assert_ne!(self, Self::Completed);
        Self::Completed
    }

    fn is_terminal(self) -> bool {
        self == Self::Completed
    }
}

struct WriteOperation {
    // The box containing this structure is allocated before submission.  Its
    // allocation, and therefore these two kernel-visible addresses, never move.
    overlapped: OVERLAPPED,
    frame: Box<[u8]>,
    pipe: OwnedHandle,
    _event: OwnedHandle,
    state: OperationState,
}

impl WriteOperation {
    fn request_cancel(&mut self) -> u32 {
        let pipe = self.pipe.raw();
        let overlapped = &mut self.overlapped;
        // SAFETY: the pipe and exact OVERLAPPED belong to this submitted
        // operation and remain alive after this call.
        let cancelled = unsafe { CancelIoEx(pipe, overlapped) };
        self.state = self.state.cancel_requested();
        if cancelled == 0 {
            // ERROR_NOT_FOUND is deliberately retained.  It is a race outcome,
            // not proof that the operation is already safe to free.
            unsafe { GetLastError() }
        } else {
            0
        }
    }

    fn observe_terminal(&mut self) -> DrainStatus {
        debug_assert!(matches!(
            self.state,
            OperationState::Submitted | OperationState::CancelRequested
        ));
        let mut transferred = 0u32;
        let pipe = self.pipe.raw();
        let overlapped = &mut self.overlapped;
        // SAFETY: both handles and the OVERLAPPED are private and valid.  TRUE
        // makes this the terminal observation required before freeing either
        // kernel-visible allocation.
        let completed = unsafe { GetOverlappedResult(pipe, overlapped, &mut transferred, TRUE) };
        self.state = self.state.completed();
        if completed != 0 {
            DrainStatus::Completed(transferred as usize)
        } else {
            let code = unsafe { GetLastError() };
            if code == ERROR_OPERATION_ABORTED {
                DrainStatus::Cancelled
            } else {
                DrainStatus::Failed(code)
            }
        }
    }
}

impl Drop for WriteOperation {
    fn drop(&mut self) {
        debug_assert!(
            self.state.is_terminal(),
            "submitted Win32 I/O must reach terminal completion before release"
        );
    }
}

/// Owns a submitted operation whose terminal completion is not yet observed.
///
/// Dropping this token can block: it must drain the operation before Windows
/// may lose access to the frame and `OVERLAPPED`.  Process-disposable callers
/// that require bounded user impact retain the token and terminate the process.
pub struct PendingWrite {
    operation: Option<Box<WriteOperation>>,
    cancel_code: u32,
}

impl fmt::Debug for PendingWrite {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PendingWrite")
            .field("cancel_code", &self.cancel_code)
            .field("owns_operation", &self.operation.is_some())
            .finish()
    }
}

impl PendingWrite {
    fn new(mut operation: Box<WriteOperation>) -> Self {
        let cancel_code = operation.request_cancel();
        Self {
            operation: Some(operation),
            cancel_code,
        }
    }

    /// The `CancelIoEx` result.  Zero means that cancellation was requested;
    /// any other value is the Win32 error code, including `ERROR_NOT_FOUND`.
    pub fn cancel_code(&self) -> u32 {
        self.cancel_code
    }

    /// Wait for terminal completion and release all operation resources.
    pub fn drain(mut self) -> DrainStatus {
        self.drain_inner()
    }

    fn drain_inner(&mut self) -> DrainStatus {
        let status = self
            .operation
            .as_mut()
            .expect("pending operation is drained exactly once")
            .observe_terminal();
        // Drop handles, frame, and OVERLAPPED only after terminal observation.
        self.operation.take();
        status
    }
}

impl Drop for PendingWrite {
    fn drop(&mut self) {
        if self.operation.is_some() {
            let _ = self.drain_inner();
        }
    }
}

pub(crate) fn create_event() -> Result<OwnedHandle, SendError> {
    // A manual-reset event is dedicated to exactly one operation.
    let raw = unsafe { CreateEventW(ptr::null(), TRUE, FALSE, ptr::null()) };
    OwnedHandle::from_raw(raw).ok_or_else(|| {
        SendError::new(
            SendStage::CreateEvent,
            // SAFETY: this immediately follows the failed Win32 call.
            unsafe { GetLastError() },
        )
    })
}

pub(crate) fn write_until(
    pipe: OwnedHandle,
    event: OwnedHandle,
    frame: Vec<u8>,
    deadline: Instant,
) -> SendAttempt {
    let frame_len = match u32::try_from(frame.len()) {
        Ok(length) => length,
        Err(_) => {
            return SendAttempt::Complete(Err(SendError::new(SendStage::FrameTooLarge, 0)));
        }
    };

    let mut overlapped: OVERLAPPED = unsafe { core::mem::zeroed() };
    overlapped.hEvent = event.raw();
    let mut operation = Box::new(WriteOperation {
        overlapped,
        frame: frame.into_boxed_slice(),
        pipe,
        _event: event,
        state: OperationState::Submitted,
    });

    if Instant::now() >= deadline {
        // No kernel request exists yet; mark terminal solely so the ownership
        // assertion records that releasing these resources is safe.
        operation.state = operation.state.completed();
        return SendAttempt::Complete(Err(SendError::new(SendStage::Deadline, 0)));
    }

    // SAFETY: the operation was boxed before these addresses were exposed and
    // the box remains owned until completion is observed or the process exits.
    let pipe = operation.pipe.raw();
    let frame_ptr = operation.frame.as_ptr();
    let overlapped = &mut operation.overlapped;
    let write_ok = unsafe { WriteFile(pipe, frame_ptr, frame_len, ptr::null_mut(), overlapped) };

    if write_ok != 0 {
        return completed_result(operation, frame_len);
    }

    let submit_code = unsafe { GetLastError() };
    if submit_code != windows_sys::Win32::Foundation::ERROR_IO_PENDING {
        // WriteFile did not accept an asynchronous request, so no kernel-owned
        // buffer lifetime remains.
        operation.state = operation.state.completed();
        return SendAttempt::Complete(Err(SendError::new(SendStage::Submit, submit_code)));
    }

    let remaining = deadline.saturating_duration_since(Instant::now());
    let wait_ms = duration_to_wait_ms(remaining);
    let mut transferred = 0u32;
    let pipe = operation.pipe.raw();
    let overlapped = &mut operation.overlapped;
    let done = unsafe { GetOverlappedResultEx(pipe, overlapped, &mut transferred, wait_ms, FALSE) };
    if done != 0 {
        operation.state = operation.state.completed();
        return validate_transferred(operation, frame_len, transferred);
    }

    let wait_code = unsafe { GetLastError() };
    let stage = if remaining.is_zero() || wait_code == windows_sys::Win32::Foundation::WAIT_TIMEOUT
    {
        SendStage::Deadline
    } else {
        SendStage::AwaitCompletion
    };
    let mut error = SendError::new(stage, wait_code);
    let pending = PendingWrite::new(operation);
    error.cancel_code = pending.cancel_code();
    SendAttempt::CleanupRequired { error, pending }
}

fn completed_result(mut operation: Box<WriteOperation>, frame_len: u32) -> SendAttempt {
    let mut transferred = 0u32;
    let pipe = operation.pipe.raw();
    let overlapped = &mut operation.overlapped;
    let completed = unsafe { GetOverlappedResult(pipe, overlapped, &mut transferred, FALSE) };
    operation.state = operation.state.completed();
    if completed == 0 {
        SendAttempt::Complete(Err(SendError::new(SendStage::ObserveCompletion, unsafe {
            GetLastError()
        })))
    } else {
        validate_transferred(operation, frame_len, transferred)
    }
}

// Keep the operation's address stable until the terminal-result path releases it.
#[allow(clippy::boxed_local)]
fn validate_transferred(
    _operation: Box<WriteOperation>,
    frame_len: u32,
    transferred: u32,
) -> SendAttempt {
    if transferred == frame_len {
        SendAttempt::Complete(Ok(transferred as usize))
    } else {
        SendAttempt::Complete(Err(SendError::short_write(frame_len, transferred)))
    }
}

fn duration_to_wait_ms(duration: Duration) -> u32 {
    duration.as_millis().min(u32::MAX as u128) as u32
}

#[cfg(test)]
mod tests {
    use super::{duration_to_wait_ms, OperationState};
    use std::time::Duration;

    #[test]
    fn overlapped_poll_may_use_zero_milliseconds() {
        assert_eq!(duration_to_wait_ms(Duration::from_nanos(999_999)), 0);
        assert_eq!(duration_to_wait_ms(Duration::from_millis(1)), 1);
    }

    #[test]
    fn only_completed_state_authorizes_release() {
        let submitted = OperationState::Submitted;
        assert!(!submitted.is_terminal());
        let cancel_requested = submitted.cancel_requested();
        assert!(!cancel_requested.is_terminal());
        assert!(cancel_requested.completed().is_terminal());
    }
}
