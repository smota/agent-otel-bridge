/*
 * Copyright agent-otel-bridge contributors
 * SPDX-License-Identifier: Apache-2.0
 */

//! Startup-only per-install pseudonym key. A2 will use a customer K_link-derived key.

use agent_otel_core::link_id::{hex_encode, hmac_sha256};
use std::fmt;
#[cfg(windows)]
use std::fs::File;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::Path;

/// Resolve the same runtime home as CLI local installation, without touching it.
/// An explicit test home always wins; unresolved homes fail closed.
pub(crate) fn runtime_home() -> Option<std::path::PathBuf> {
    if let Some(home) = std::env::var_os("AGENT_OTEL_HOME") {
        return (!home.is_empty()).then(|| home.into());
    }
    #[cfg(windows)]
    {
        if let Some(home) = std::env::var_os("LOCALAPPDATA") {
            return Some(std::path::PathBuf::from(home).join("agent-otel-bridge"));
        }
        if let Some(home) = std::env::var_os("USERPROFILE") {
            return Some(std::path::PathBuf::from(home).join("AppData/Local/agent-otel-bridge"));
        }
    }
    #[cfg(unix)]
    {
        if let Some(home) = std::env::var_os("XDG_DATA_HOME") {
            return Some(std::path::PathBuf::from(home).join("agent-otel-bridge"));
        }
        if let Some(home) = std::env::var_os("HOME") {
            return Some(std::path::PathBuf::from(home).join(".local/share/agent-otel-bridge"));
        }
    }
    None
}

/// Secret key material never participates in derived Debug output.
pub struct PseudonymKey([u8; 32]);

impl fmt::Debug for PseudonymKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PseudonymKey(<redacted>)")
    }
}

impl PseudonymKey {
    /// Load once at daemon startup; never call from the event path.
    /// Unix creation uses 0600; Windows protects the file with an owner-only DACL.
    pub fn load_or_create(home: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(home)?;
        #[cfg(windows)]
        warn_if_home_outside_profile(home);
        let path = home.join("pseudonym.key");
        match Self::load(&path) {
            Ok(key) => return Ok(key),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes)
            .map_err(|_| io::Error::other("pseudonym key entropy unavailable"))?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_WRITE, WRITE_DAC};
            options.access_mode(FILE_GENERIC_WRITE | WRITE_DAC);
        }
        match options.open(&path) {
            Ok(mut file) => {
                // Restrict the empty file before any key material is written.
                #[cfg(windows)]
                restrict_key_file(&file)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
                Ok(Self(bytes))
            }
            // Another startup won exclusive creation. A partial/invalid file fails closed.
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Self::load(&path),
            Err(e) => Err(e),
        }
    }

    fn load(path: &Path) -> io::Result<Self> {
        if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
            return Err(io::Error::other("pseudonym key must be a regular file"));
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_READ, WRITE_DAC};
            options.access_mode(FILE_GENERIC_READ | WRITE_DAC);
        }
        let mut file = options.open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() != 32 {
            return Err(io::Error::other("invalid pseudonym key file"));
        }
        // Also repair inherited permissions on keys created by earlier versions.
        #[cfg(windows)]
        restrict_key_file(&file)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(io::Error::other(
                    "pseudonym key permissions are not private",
                ));
            }
        }
        let mut bytes = [0u8; 32];
        file.read_exact(&mut bytes)?;
        Ok(Self(bytes))
    }

    pub(crate) fn pseudonym(&self, domain: &str, raw: &str) -> String {
        let message = format!("aob.pseudonym.v1/{domain}\0{raw}");
        let digest = hmac_sha256(&self.0, message.as_bytes());
        format!("{domain}_{}", hex_encode(&digest[..16]))
    }
}

#[cfg(windows)]
fn profile_contains(home: &Path, profile: &Path) -> bool {
    let mut home = home.components();
    profile.components().all(|part| {
        home.next().is_some_and(|candidate| {
            candidate
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&part.as_os_str().to_string_lossy())
        })
    })
}

#[cfg(windows)]
fn warn_if_home_outside_profile(home: &Path) {
    static WARNED: std::sync::Once = std::sync::Once::new();
    let inside_profile = std::env::var_os("USERPROFILE")
        .and_then(|profile| {
            Some(profile_contains(
                &std::fs::canonicalize(home).ok()?,
                &std::fs::canonicalize(profile).ok()?,
            ))
        })
        .unwrap_or(false);
    if !inside_profile {
        WARNED.call_once(|| {
            eprintln!("[agent-otel-daemon] AGENT_OTEL_HOME is outside the current user's profile; protect the runtime directory ACL.");
        });
    }
}

#[cfg(windows)]
fn restrict_key_file(file: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        SetKernelObjectSecurity, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    };

    // Protected DACL: full access for the file owner (OWNER RIGHTS SID), no
    // inherited or other ACEs. Parent-directory replacement rights still matter.
    let sddl: Vec<u16> = "D:P(A;;FA;;;OW)".encode_utf16().chain(Some(0)).collect();
    let mut descriptor = std::ptr::null_mut();
    // SAFETY: the NUL-terminated input and descriptor out-pointer are valid.
    // The descriptor remains allocated until SetKernelObjectSecurity returns.
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let result = SetKernelObjectSecurity(
            file.as_raw_handle(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        );
        let error = (result == 0).then(io::Error::last_os_error);
        LocalFree(descriptor);
        error.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn profile_check_uses_case_insensitive_components() {
        let profile = Path::new(r"C:\Users\Alice");
        assert!(profile_contains(
            Path::new(r"c:\users\alice\runtime"),
            profile
        ));
        assert!(profile_contains(profile, profile));
        assert!(!profile_contains(
            Path::new(r"C:\Users\Alice-other"),
            profile
        ));
        assert!(!profile_contains(Path::new(r"D:\Shared\runtime"), profile));
    }

    #[cfg(windows)]
    fn assert_owner_only_dacl(path: &Path) {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
            SE_FILE_OBJECT,
        };
        use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;

        let file = File::open(path).unwrap();
        let mut descriptor = std::ptr::null_mut();
        let mut sddl = std::ptr::null_mut();
        // SAFETY: valid open handle and out-pointers; both Windows allocations
        // are freed after inspecting the returned NUL-terminated SDDL.
        unsafe {
            assert_eq!(
                GetSecurityInfo(
                    file.as_raw_handle(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut descriptor,
                ),
                0
            );
            assert_ne!(
                ConvertSecurityDescriptorToStringSecurityDescriptorW(
                    descriptor,
                    SDDL_REVISION_1,
                    DACL_SECURITY_INFORMATION,
                    &mut sddl,
                    std::ptr::null_mut(),
                ),
                0
            );
            let mut len = 0;
            while *sddl.add(len) != 0 {
                len += 1;
            }
            let actual = String::from_utf16_lossy(std::slice::from_raw_parts(sddl, len));
            LocalFree(sddl.cast());
            LocalFree(descriptor);
            assert_eq!(actual, "D:P(A;;FA;;;OW)");
        }
    }

    #[test]
    fn keys_are_stable_private_domain_separated_and_redacted() {
        let dir = std::env::temp_dir().join(format!("aob-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = PseudonymKey::load_or_create(&dir).unwrap();
        let again = PseudonymKey::load_or_create(&dir).unwrap();
        #[cfg(windows)]
        assert_owner_only_dacl(&dir.join("pseudonym.key"));
        assert_eq!(
            key.pseudonym("proj", "sentinel"),
            again.pseudonym("proj", "sentinel")
        );
        assert_eq!(key.pseudonym("proj", "sentinel").len(), 37);
        assert_ne!(
            key.pseudonym("proj", "sentinel"),
            key.pseudonym("repo", "sentinel")
        );
        assert_ne!(
            key.pseudonym("proj", "sentinel"),
            PseudonymKey([42; 32]).pseudonym("proj", "sentinel")
        );
        assert_eq!(format!("{key:?}"), "PseudonymKey(<redacted>)");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(dir.join("pseudonym.key"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        std::fs::write(dir.join("pseudonym.key"), b"bad").unwrap();
        assert!(PseudonymKey::load_or_create(&dir).is_err());
        assert_eq!(std::fs::read(dir.join("pseudonym.key")).unwrap(), b"bad");
        let blocked_home = dir.join("blocked-home");
        std::fs::write(&blocked_home, b"not a directory").unwrap();
        assert!(PseudonymKey::load_or_create(&blocked_home).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
