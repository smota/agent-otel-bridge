/*
 * Copyright agent-otel-bridge contributors
 * SPDX-License-Identifier: Apache-2.0
 */

//! Startup-only per-install pseudonym key. A2 will use a customer K_link-derived key.

use agent_otel_core::link_id::{hex_encode, hmac_sha256};
use std::fmt;
use std::fs::{File, OpenOptions};
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
    /// Unix creation uses 0600; Windows inherits the per-user runtime home's ACL.
    pub fn load_or_create(home: &Path) -> io::Result<Self> {
        let path = home.join("pseudonym.key");
        match Self::load(&path) {
            Ok(key) => return Ok(key),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        std::fs::create_dir_all(home)?;
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
        match options.open(&path) {
            Ok(mut file) => {
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
        let mut file = File::open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() != 32 {
            return Err(io::Error::other("invalid pseudonym key file"));
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_stable_private_domain_separated_and_redacted() {
        let dir = std::env::temp_dir().join(format!("aob-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = PseudonymKey::load_or_create(&dir).unwrap();
        let again = PseudonymKey::load_or_create(&dir).unwrap();
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
