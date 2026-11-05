/* Copyright agent-otel-bridge contributors
 * SPDX-License-Identifier: Apache-2.0 */

//! Linux's independent native watchdog and unbuffered inherited streams.
//! No hook configuration is read and no inherited descriptor flags are changed.

use std::ffi::c_void;
use std::mem::MaybeUninit;
use std::time::{Duration, Instant};

pub fn spawn_watchdog(deadline: Instant) -> bool {
    let mut now = MaybeUninit::<libc::timespec>::uninit();
    // SAFETY: clock_gettime initializes the supplied timespec on success.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, now.as_mut_ptr()) } != 0 {
        return false;
    }
    // Reading Instant after clock_gettime makes the native deadline no later
    // than the caller's deadline, even if the main thread is descheduled here.
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return false;
    }
    // SAFETY: clock_gettime returned success above.
    let Some(absolute) = absolute_time(unsafe { now.assume_init() }, remaining) else {
        return false;
    };
    let argument = Box::into_raw(Box::new(absolute));
    let mut thread = MaybeUninit::<libc::pthread_t>::uninit();
    // SAFETY: pthread_create writes thread on success and transfers the unique
    // Box allocation to watchdog_main. On failure no callback is created, so
    // this thread reclaims the allocation. No pointer refers to stack storage.
    let result = unsafe {
        libc::pthread_create(
            thread.as_mut_ptr(),
            std::ptr::null(),
            watchdog_main,
            argument.cast(),
        )
    };
    if result != 0 {
        // SAFETY: failed creation leaves sole ownership with this thread.
        drop(unsafe { Box::from_raw(argument) });
        return false;
    }
    // SAFETY: successful creation initialized the pthread identifier. A failed
    // detach only retains join bookkeeping until this short-lived process exits.
    unsafe { libc::pthread_detach(thread.assume_init()) };
    true
}

extern "C" fn watchdog_main(argument: *mut c_void) -> *mut c_void {
    // SAFETY: only spawn_watchdog passes a successful creation's unique Box.
    let absolute = unsafe { Box::from_raw(argument.cast::<libc::timespec>()) };
    loop {
        // SAFETY: absolute is live, normalized and belongs to this thread.
        // Absolute sleep retries EINTR without extending the original deadline.
        let result = unsafe {
            libc::clock_nanosleep(
                libc::CLOCK_MONOTONIC,
                libc::TIMER_ABSTIME,
                &*absolute,
                std::ptr::null_mut(),
            )
        };
        if result != libc::EINTR {
            // A clock failure must also fail open; never leave main unguarded.
            exit_process(0);
        }
    }
}

fn absolute_time(now: libc::timespec, remaining: Duration) -> Option<libc::timespec> {
    if now.tv_sec < 0 || !(0..1_000_000_000).contains(&now.tv_nsec) {
        return None;
    }
    let seconds = libc::time_t::try_from(remaining.as_secs()).ok()?;
    let nanoseconds = now.tv_nsec as u64 + u64::from(remaining.subsec_nanos());
    let carry = libc::time_t::try_from(nanoseconds / 1_000_000_000).ok()?;
    Some(libc::timespec {
        tv_sec: now.tv_sec.checked_add(seconds)?.checked_add(carry)?,
        tv_nsec: (nanoseconds % 1_000_000_000) as libc::c_long,
    })
}

fn interrupted() -> bool {
    // SAFETY: Linux libc provides this thread's valid errno pointer.
    unsafe { *libc::__errno_location() == libc::EINTR }
}

pub fn read_stdin_capped(max: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(4096.min(max));
    let mut buffer = [0u8; 4096];
    while bytes.len() < max {
        let capacity = buffer.len().min(max - bytes.len());
        // SAFETY: buffer is writable for capacity bytes. The independent
        // watchdog bounds blocked inherited input without changing its flags.
        let count = unsafe { libc::read(libc::STDIN_FILENO, buffer.as_mut_ptr().cast(), capacity) };
        if count > 0 {
            bytes.extend_from_slice(&buffer[..count as usize]);
        } else if count == 0 || !interrupted() {
            break;
        }
    }
    bytes
}

pub fn write_stdout(bytes: &[u8]) -> bool {
    let mut offset = 0;
    while offset < bytes.len() {
        // SAFETY: the remaining byte slice stays live for this write. Rust's
        // normal Linux entrypoint ignores SIGPIPE; errors return fail-open.
        // The independent watchdog also covers a full pipe or blocked device.
        let count = unsafe {
            libc::write(
                libc::STDOUT_FILENO,
                bytes[offset..].as_ptr().cast(),
                bytes.len() - offset,
            )
        };
        if count > 0 {
            offset += count as usize;
        } else if count == 0 || !interrupted() {
            return false;
        }
    }
    true
}

pub fn exit_process(code: i32) -> ! {
    // SAFETY: the complete response has already been written, or a fail-open
    // deadline/error forbids further output. _exit never waits for Rust's
    // buffered stream locks, including when another thread is blocked on one.
    unsafe { libc::_exit(code) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_deadline_normalizes_nanosecond_carry() {
        let value = absolute_time(
            libc::timespec {
                tv_sec: 5,
                tv_nsec: 999_999_999,
            },
            Duration::from_nanos(2),
        )
        .unwrap();
        assert_eq!((value.tv_sec, value.tv_nsec), (6, 1));
    }

    #[test]
    fn invalid_or_unrepresentable_deadlines_fail_closed_to_startup() {
        assert!(absolute_time(
            libc::timespec {
                tv_sec: -1,
                tv_nsec: 0
            },
            Duration::ZERO
        )
        .is_none());
        assert!(absolute_time(
            libc::timespec {
                tv_sec: 0,
                tv_nsec: 1_000_000_000
            },
            Duration::ZERO
        )
        .is_none());
        assert!(absolute_time(
            libc::timespec {
                tv_sec: libc::time_t::MAX,
                tv_nsec: 0
            },
            Duration::from_secs(1)
        )
        .is_none());
        assert!(absolute_time(
            libc::timespec {
                tv_sec: 1,
                tv_nsec: 0
            },
            Duration::MAX
        )
        .is_none());
    }
}
