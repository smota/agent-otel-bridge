/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMetadata {
    pub os_name: String,
    pub os_version: String,
    pub os_build: String,
    pub os_arch: String,
    pub cpu_brand: String,
    pub cpu_cores: usize,
    pub ram_gb: f64,
    pub rustc_version: String,
    pub bridge_version: String,
    pub timestamp_utc: String,
}

impl SystemMetadata {
    pub fn collect() -> Self {
        let os_arch = std::env::consts::ARCH.to_string();
        let bridge_version = env!("CARGO_PKG_VERSION").to_string();
        let timestamp_utc = get_iso_timestamp();
        let rustc_version = get_rustc_version();

        #[cfg(windows)]
        {
            let (os_name, os_version, os_build) = get_windows_os_info();
            let cpu_brand = get_windows_cpu_brand();
            let cpu_cores = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or_else(|_| {
                    std::env::var("NUMBER_OF_PROCESSORS")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(1)
                });
            let ram_gb = get_windows_ram_gb();

            Self {
                os_name,
                os_version,
                os_build,
                os_arch,
                cpu_brand,
                cpu_cores,
                ram_gb,
                rustc_version,
                bridge_version,
                timestamp_utc,
            }
        }

        #[cfg(not(windows))]
        {
            let os_name = std::env::consts::OS.to_string();
            let os_version = "Unknown".to_string();
            let os_build = "Unknown".to_string();
            let cpu_brand = "Generic CPU".to_string();
            let cpu_cores = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1);
            let ram_gb = 0.0;

            Self {
                os_name,
                os_version,
                os_build,
                os_arch,
                cpu_brand,
                cpu_cores,
                ram_gb,
                rustc_version,
                bridge_version,
                timestamp_utc,
            }
        }
    }
}

#[cfg(windows)]
fn get_windows_ram_gb() -> f64 {
    unsafe {
        let mut status: windows_sys::Win32::System::SystemInformation::MEMORYSTATUSEX =
            std::mem::zeroed();
        status.dwLength = std::mem::size_of::<
            windows_sys::Win32::System::SystemInformation::MEMORYSTATUSEX,
        >() as u32;
        if windows_sys::Win32::System::SystemInformation::GlobalMemoryStatusEx(&mut status) != 0 {
            let total_bytes = status.ullTotalPhys;
            let gb = total_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
            (gb * 10.0).round() / 10.0
        } else {
            0.0
        }
    }
}

#[cfg(windows)]
fn get_windows_cpu_brand() -> String {
    if let Ok(output) = std::process::Command::new("reg")
        .args([
            "query",
            r"HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0",
            "/v",
            "ProcessorNameString",
        ])
        .output()
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                if line.contains("ProcessorNameString") {
                    if let Some(pos) = line.find("REG_SZ") {
                        let brand = line[pos + 6..].trim();
                        if !brand.is_empty() {
                            return brand.to_string();
                        }
                    }
                }
            }
        }
    }

    std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_else(|_| "Unknown x86_64 CPU".to_string())
}

#[cfg(windows)]
fn get_windows_os_info() -> (String, String, String) {
    let mut name = "Windows".to_string();
    let mut version = "Unknown".to_string();
    let mut build = "Unknown".to_string();

    if let Ok(output) = std::process::Command::new("reg")
        .args([
            "query",
            r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion",
        ])
        .output()
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("ProductName") {
                    if let Some(pos) = trimmed.find("REG_SZ") {
                        name = trimmed[pos + 6..].trim().to_string();
                    }
                } else if trimmed.starts_with("DisplayVersion") {
                    if let Some(pos) = trimmed.find("REG_SZ") {
                        version = trimmed[pos + 6..].trim().to_string();
                    }
                } else if trimmed.starts_with("CurrentBuildNumber") {
                    if let Some(pos) = trimmed.find("REG_SZ") {
                        build = trimmed[pos + 6..].trim().to_string();
                    }
                }
            }
        }
    }

    (name, version, build)
}

fn get_rustc_version() -> String {
    std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|out| {
            if out.status.success() {
                Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "rustc (unknown)".to_string())
}

fn get_iso_timestamp() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    let days = secs / 86400;
    let rem_secs = secs % 86400;
    let hours = rem_secs / 3600;
    let mins = (rem_secs % 3600) / 60;
    let s = rem_secs % 60;

    let z = days as i64 + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1024 + doe / 1461 - doe / 14244) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02}T{hours:02}:{mins:02}:{s:02}Z")
}
