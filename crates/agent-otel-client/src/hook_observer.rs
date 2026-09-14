/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use std::time::{Duration, Instant};

const OBSERVER_ENV: &str = "AGENT_OTEL_BENCH_HANDLE";
#[cfg(windows)]
const RECORD_LEN: usize = 40;

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
        send_completed: bool,
    ) {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Storage::FileSystem::WriteFile;

            let mut record = [0u8; RECORD_LEN];
            record[0..4].copy_from_slice(b"AOBT");
            record[4..6].copy_from_slice(&1u16.to_le_bytes());
            record[6..8].copy_from_slice(&(RECORD_LEN as u16).to_le_bytes());
            record[8..12].copy_from_slice(&std::process::id().to_le_bytes());
            let flags = 1u32 | (u32::from(send_completed) << 1);
            record[12..16].copy_from_slice(&flags.to_le_bytes());
            record[16..24].copy_from_slice(&nanos(response_completed).to_le_bytes());
            record[24..32].copy_from_slice(&nanos(before_transport).to_le_bytes());
            record[32..40].copy_from_slice(&nanos(work_completed).to_le_bytes());

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
                send_completed,
            );
        }
    }
}

#[cfg(any(windows, test))]
fn nanos(duration: Duration) -> u64 {
    duration.as_nanos().min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::nanos;
    use std::time::Duration;

    #[test]
    fn nanoseconds_saturate_to_wire_width() {
        assert_eq!(nanos(Duration::from_nanos(17)), 17);
        assert_eq!(nanos(Duration::MAX), u64::MAX);
    }
}
