/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use std::time::{Duration, Instant};

use agent_otel_ipc::client::SendError;

const OBSERVER_ENV: &str = "AGENT_OTEL_BENCH_HANDLE";
#[cfg(any(windows, test))]
const RECORD_LEN: usize = 48;

pub struct HookObserver {
    started: Instant,
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

impl HookObserver {
    pub fn from_environment(started: Instant) -> Option<Self> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Storage::FileSystem::{GetFileType, FILE_TYPE_PIPE};
            use windows_sys::Win32::System::Pipes::{SetNamedPipeHandleState, PIPE_NOWAIT};

            let raw = std::env::var(OBSERVER_ENV).ok()?.parse::<u64>().ok()?;
            let raw = usize::try_from(raw).ok()?;
            if raw == 0 || raw == usize::MAX {
                return None;
            }
            let handle = raw as windows_sys::Win32::Foundation::HANDLE;
            // SAFETY: querying the type does not consume or mutate the inherited
            // handle.  Only pipe handles are accepted by the observer protocol.
            if unsafe { GetFileType(handle) } != FILE_TYPE_PIPE {
                return None;
            }
            let mode = PIPE_NOWAIT;
            // SAFETY: the validated inherited pipe handle remains owned by the
            // controller. PIPE_NOWAIT makes the single observer write return
            // immediately when its buffer cannot accept the whole record.
            if unsafe { SetNamedPipeHandleState(handle, &mode, std::ptr::null(), std::ptr::null()) }
                == 0
            {
                // Never enable instrumentation on a handle whose write could
                // block. Observer absence is explicitly an unknown result.
                return None;
            }
            Some(Self { started, handle })
        }

        #[cfg(not(windows))]
        {
            let _ = (started, OBSERVER_ENV);
            None
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn emit(
        self,
        response_completed: Duration,
        before_transport: Duration,
        work_completed: Duration,
        send_error: Option<SendError>,
    ) {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Storage::FileSystem::WriteFile;

            let record = encode_record(
                std::process::id(),
                response_completed,
                before_transport,
                work_completed,
                send_error,
            );

            let mut written = 0u32;
            // SAFETY: validation accepted this inherited pipe handle and the
            // fixed record stays alive until the synchronous write returns.
            let _ = unsafe {
                WriteFile(
                    self.handle,
                    record.as_ptr(),
                    RECORD_LEN as u32,
                    &mut written,
                    std::ptr::null_mut(),
                )
            };
        }

        #[cfg(not(windows))]
        {
            let _ = (
                self,
                response_completed,
                before_transport,
                work_completed,
                send_error,
            );
        }
    }
}

#[cfg(any(windows, test))]
fn encode_record(
    pid: u32,
    response_completed: Duration,
    before_transport: Duration,
    work_completed: Duration,
    send_error: Option<SendError>,
) -> [u8; RECORD_LEN] {
    let mut record = [0u8; RECORD_LEN];
    record[0..4].copy_from_slice(b"AOBT");
    record[4..6].copy_from_slice(&2u16.to_le_bytes());
    record[6..8].copy_from_slice(&(RECORD_LEN as u16).to_le_bytes());
    record[8..12].copy_from_slice(&pid.to_le_bytes());
    let flags = 1u32 | (u32::from(send_error.is_none()) << 1);
    record[12..16].copy_from_slice(&flags.to_le_bytes());
    if let Some(error) = send_error {
        // Zero is reserved for successful transport. SendStage has a stable
        // repr(u8), so adding one preserves every stage without conflating
        // Connect with success.
        record[16] = error.stage as u8 + 1;
        record[20..24].copy_from_slice(&error.os_code.to_le_bytes());
    }
    record[24..32].copy_from_slice(&nanos(response_completed).to_le_bytes());
    record[32..40].copy_from_slice(&nanos(before_transport).to_le_bytes());
    record[40..48].copy_from_slice(&nanos(work_completed).to_le_bytes());
    record
}

#[cfg(any(windows, test))]
fn nanos(duration: Duration) -> u64 {
    duration.as_nanos().min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::{encode_record, nanos};
    use agent_otel_ipc::client::{SendError, SendStage};
    use std::time::Duration;

    #[test]
    fn nanoseconds_saturate_to_wire_width() {
        assert_eq!(nanos(Duration::from_nanos(17)), 17);
        assert_eq!(nanos(Duration::MAX), u64::MAX);
    }

    #[test]
    fn v2_record_carries_transport_stage_and_os_code() {
        let record = encode_record(
            1234,
            Duration::from_nanos(10),
            Duration::from_nanos(20),
            Duration::from_nanos(30),
            Some(SendError {
                stage: SendStage::Connect,
                os_code: 2,
                cancel_code: 0,
                expected_bytes: 0,
                transferred_bytes: 0,
            }),
        );
        assert_eq!(&record[0..4], b"AOBT");
        assert_eq!(u16::from_le_bytes(record[4..6].try_into().unwrap()), 2);
        assert_eq!(u16::from_le_bytes(record[6..8].try_into().unwrap()), 48);
        assert_eq!(u32::from_le_bytes(record[8..12].try_into().unwrap()), 1234);
        assert_eq!(u32::from_le_bytes(record[12..16].try_into().unwrap()), 1);
        assert_eq!(record[16], 1);
        assert_eq!(u32::from_le_bytes(record[20..24].try_into().unwrap()), 2);
    }

    #[test]
    fn v2_success_reserves_zero_stage_and_code() {
        let record = encode_record(
            1234,
            Duration::from_nanos(10),
            Duration::from_nanos(20),
            Duration::from_nanos(30),
            None,
        );
        assert_eq!(u32::from_le_bytes(record[12..16].try_into().unwrap()), 3);
        assert_eq!(record[16], 0);
        assert_eq!(u32::from_le_bytes(record[20..24].try_into().unwrap()), 0);
    }
}
