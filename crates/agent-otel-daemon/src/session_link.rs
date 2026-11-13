/* Copyright agent-otel-bridge contributors
 * SPDX-License-Identifier: Apache-2.0 */

//! Startup-only endpoint provisioning for the frozen `link-id-v1` contract.
//! Endpoints hold only K_link, never an organization root key or PRK.

use agent_otel_core::link_id::{
    derive_session_link_with_key, hmac_sha256, is_valid_ascii_slug, lp_encode_str, LinkIdInput,
    LinkIdOutcome, UnlinkedReason, AGENT_SESSION_LINK_ID, AGENT_SESSION_LINK_UNLINKED,
    AGENT_SESSION_UNLINKED_REASON,
};
use agent_otel_core::model::AgentHookInput;
use agent_otel_core::otlp::kv_string;
use opentelemetry_proto::tonic::trace::v1::Span;
use std::io::Read;
use std::path::PathBuf;
use std::process::Command;
use zeroize::Zeroizing;

/// Immutable startup snapshot. Secret buffers are zeroized on drop and never cloned.
/// Team/epoch validate the provisioned scope; the key is already derived for it.
/// Neither administrative identifiers nor key material participate in Debug.
pub struct SessionLinkConfig {
    org: Option<String>,
    endpoint_scope: Option<String>,
    key: Option<Zeroizing<[u8; 32]>>,
}

impl std::fmt::Debug for SessionLinkConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionLinkConfig(<redacted>)")
    }
}

impl SessionLinkConfig {
    /// Read once before admitting events. No event-path environment or file access.
    pub fn from_env() -> Result<Self, &'static str> {
        Self::from_vars(&|name| std::env::var(name).ok())
    }

    /// Build from a provisioning source without logging any supplied values.
    /// Raw root keys are always refused: this endpoint has no admin mode.
    pub fn from_vars(get: &dyn Fn(&str) -> Option<String>) -> Result<Self, &'static str> {
        for name in ["AGENT_OTEL_LINK_K_ORG_HEX", "AGENT_OTEL_K_ORG_HEX"] {
            if get(name).map(Zeroizing::new).is_some() {
                return Err("raw K_org refused: provision a derived K_link outside the daemon");
            }
        }
        let org = get("AGENT_OTEL_LINK_ORG");
        let endpoint_scope = get("AGENT_OTEL_LINK_ENDPOINT_SCOPE");
        let team = get("AGENT_OTEL_LINK_TEAM");
        let epoch = get("AGENT_OTEL_LINK_EPOCH");
        for field in [&org, &endpoint_scope, &team, &epoch].into_iter().flatten() {
            if !is_valid_ascii_slug(field) {
                return Err("invalid session-link provisioning slug");
            }
        }
        let key = match provisioned_key(get)? {
            None => None,
            Some(raw) => {
                if raw.len() != 64 || team.is_none() || epoch.is_none() {
                    return Err("invalid session-link key material or missing team/epoch");
                }
                // Decode directly into a zeroizing buffer so even a partial decode
                // is wiped when a later non-hex byte rejects the configuration.
                let mut key = Zeroizing::new([0u8; 32]);
                for (pair, output) in raw.as_bytes().as_chunks::<2>().0.iter().zip(key.iter_mut()) {
                    let nibble = |byte| match byte {
                        b'0'..=b'9' => Ok(byte - b'0'),
                        b'a'..=b'f' => Ok(byte - b'a' + 10),
                        b'A'..=b'F' => Ok(byte - b'A' + 10),
                        _ => Err("invalid session-link key encoding"),
                    };
                    *output = (nibble(pair[0])? << 4) | nibble(pair[1])?;
                }
                Some(key)
            }
        };
        Ok(Self {
            org,
            endpoint_scope,
            key,
        })
    }
}

/// Load only explicitly provisioned keys. A configured file must be valid;
/// only an absent default file falls through to the process environment.
fn provisioned_key(
    get: &dyn Fn(&str) -> Option<String>,
) -> Result<Option<Zeroizing<String>>, &'static str> {
    let explicit = get("AGENT_OTEL_LINK_KEY_FILE");
    let path = explicit.as_ref().map(PathBuf::from).or_else(|| {
        crate::pseudonym_key::runtime_home_from_vars(get).map(|home| home.join("link.key"))
    });
    if let Some(path) = path {
        match crate::pseudonym_key::open_private_key_file(&path, 64, false) {
            Ok(mut file) => {
                let mut bytes = Zeroizing::new([0u8; 64]);
                file.read_exact(&mut bytes[..])
                    .map_err(|_| "unable to read session-link key file")?;
                let raw = std::str::from_utf8(&bytes[..])
                    .map_err(|_| "invalid session-link key encoding")?;
                return Ok(Some(Zeroizing::new(raw.to_owned())));
            }
            Err(error) if explicit.is_none() && error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("invalid or non-private session-link key file"),
        }
    }
    Ok(get("AGENT_OTEL_LINK_KEY_HEX").map(Zeroizing::new))
}

/// Keep provisioning out of every daemon child, including future LINK_* names.
/// Apply after any caller-supplied environment overrides and before spawning.
pub(crate) fn remove_link_environment(command: &mut Command) -> &mut Command {
    let explicit: Vec<_> = command.get_envs().map(|(key, _)| key.to_owned()).collect();
    for name in std::env::vars_os().map(|(name, _)| name).chain(explicit) {
        if name
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("AGENT_OTEL_LINK_")
        {
            command.env_remove(name);
        }
    }
    // Also clear the separately named, forbidden organization root key.
    command.env_remove("AGENT_OTEL_K_ORG_HEX")
}

/// Contains only exportable values and an optional keyed fallback trace ID.
/// Deliberately has no Debug implementation.
pub(crate) struct SessionLink {
    link_id: String,
    reason: Option<UnlinkedReason>,
    pub(crate) fallback_trace_id: Option<[u8; 16]>,
}

impl SessionLink {
    pub(crate) fn stamp(self, span: &mut Span) {
        span.attributes
            .push(kv_string(AGENT_SESSION_LINK_ID, &self.link_id));
        if let Some(reason) = self.reason {
            span.attributes
                .push(kv_string(AGENT_SESSION_UNLINKED_REASON, reason.as_str()));
        }
    }
}

fn trace_prefix(tag: &[u8]) -> [u8; 16] {
    let mut trace = [0u8; 16];
    trace.copy_from_slice(&tag[..16]);
    if trace == [0u8; 16] {
        trace[0] = 1;
    }
    trace
}

/// Resolve against raw harness session data before privacy projection or normalization.
pub(crate) fn resolve(
    config: Option<&SessionLinkConfig>,
    wire_client_id: u16,
    input: &AgentHookInput,
) -> SessionLink {
    let Some((config, key)) =
        config.and_then(|config| config.key.as_ref().map(|key| (config, key)))
    else {
        return SessionLink {
            link_id: AGENT_SESSION_LINK_UNLINKED.into(),
            reason: Some(UnlinkedReason::InvalidKeyMaterial),
            fallback_trace_id: None,
        };
    };
    let link_input = LinkIdInput {
        org: config.org.as_deref(),
        endpoint_scope: config.endpoint_scope.as_deref(),
        wire_client_id: Some(wire_client_id),
        // Nonzero wire IDs are authoritative, including unknown IDs (fail closed).
        // A payload name alongside a known wire ID may be a subagent role.
        raw_harness: (wire_client_id == 0)
            .then_some(input.agent_name.as_deref())
            .flatten(),
        native_session: input.conversation_id.as_deref(),
        origin: input.origin.as_deref(),
    };
    match derive_session_link_with_key(key, &link_input) {
        LinkIdOutcome::Linked { tag, link_id, .. } => SessionLink {
            link_id,
            reason: None,
            fallback_trace_id: Some(trace_prefix(&tag)),
        },
        LinkIdOutcome::Unlinked { reason } => {
            // Even an unidentifiable harness must not expose a dictionary-matchable
            // conversation hash when a valid endpoint key exists. Distinct domain
            // prevents this fallback from becoming a contract link ID.
            // Origin governs whether a link may be exported, not trace identity.
            // Reuse the link domain for a known native session across origins.
            let native_trace = derive_session_link_with_key(
                key,
                &LinkIdInput {
                    origin: None,
                    ..link_input
                },
            );
            let fallback_trace_id = if let LinkIdOutcome::Linked { tag, .. } = native_trace {
                Some(trace_prefix(&tag))
            } else {
                let nonce;
                let session = match input.conversation_id.as_deref() {
                    Some(session) if !session.trim().is_empty() => session,
                    _ => {
                        nonce = agent_otel_core::link_id::hex_encode(
                            &agent_otel_core::trace_id::derive_trace_id(None),
                        );
                        &nonce
                    }
                };
                lp_encode_str(&[
                    "agent.session.trace-fallback/v1",
                    config.org.as_deref().unwrap_or(""),
                    config.endpoint_scope.as_deref().unwrap_or(""),
                    session,
                ])
                .ok()
                .map(|message| {
                    let message = Zeroizing::new(message);
                    trace_prefix(&hmac_sha256(&key[..], &message))
                })
            };
            SessionLink {
                link_id: AGENT_SESSION_LINK_UNLINKED.into(),
                reason: Some(reason),
                fallback_trace_id,
            }
        }
    }
}

#[cfg(test)]
mod provisioning_tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs::{self, OpenOptions};
    use std::io::Write;

    fn source(home: &std::path::Path) -> HashMap<String, String> {
        HashMap::from([
            ("AGENT_OTEL_HOME".into(), home.to_str().unwrap().into()),
            ("AGENT_OTEL_LINK_TEAM".into(), "team".into()),
            ("AGENT_OTEL_LINK_EPOCH".into(), "epoch".into()),
            ("AGENT_OTEL_LINK_KEY_HEX".into(), "11".repeat(32)),
        ])
    }

    fn load(vars: &HashMap<String, String>) -> Result<SessionLinkConfig, &'static str> {
        SessionLinkConfig::from_vars(&|name| vars.get(name).cloned())
    }

    fn private_file(path: &std::path::Path, contents: &[u8]) {
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
        let mut file = options.open(path).unwrap();
        #[cfg(windows)]
        crate::pseudonym_key::restrict_key_file(&file).unwrap();
        file.write_all(contents).unwrap();
    }

    #[test]
    fn provisioned_files_precede_env_and_missing_default_never_creates_key() {
        let home = std::env::temp_dir().join(format!("aob-link-files-{}", std::process::id()));
        assert!(!home.exists());
        let mut vars = source(&home);
        assert_eq!(load(&vars).unwrap().key.unwrap().as_ref(), &[0x11; 32]);
        assert!(
            !home.exists(),
            "provisioning must not create runtime or key files"
        );
        fs::create_dir(&home).unwrap();
        let default = home.join("link.key");
        private_file(&default, "22".repeat(32).as_bytes());
        vars.insert(
            "AGENT_OTEL_LINK_KEY_HEX".into(),
            "invalid-env-ignored".into(),
        );
        assert_eq!(load(&vars).unwrap().key.unwrap().as_ref(), &[0x22; 32]);
        let custom = home.join("custom.key");
        private_file(&custom, "33".repeat(32).as_bytes());
        vars.insert(
            "AGENT_OTEL_LINK_KEY_FILE".into(),
            custom.to_str().unwrap().into(),
        );
        assert_eq!(load(&vars).unwrap().key.unwrap().as_ref(), &[0x33; 32]);
        for contents in ["bad".to_owned(), "g".repeat(64), "44".repeat(32) + "\n"] {
            fs::write(&custom, contents).unwrap();
            assert!(
                load(&vars).is_err(),
                "bad file must not fall through to env/default"
            );
        }
        fs::remove_file(&custom).unwrap();
        assert!(
            load(&vars).is_err(),
            "explicit missing file must fail closed"
        );
        vars.remove("AGENT_OTEL_LINK_KEY_FILE");
        vars.remove("AGENT_OTEL_LINK_KEY_HEX");
        fs::remove_file(default).unwrap();
        assert!(load(&vars).unwrap().key.is_none());
        assert!(!home.join("link.key").exists());
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn wider_key_permissions_are_rejected_without_repair() {
        let home = std::env::temp_dir().join(format!("aob-link-perms-{}", std::process::id()));
        fs::create_dir(&home).unwrap();
        let path = home.join("link.key");
        // Inherited Windows ACLs are deliberately not the protected owner-only DACL.
        fs::write(&path, "44".repeat(32)).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        }
        let before = fs::metadata(&path).unwrap().permissions();
        assert!(load(&source(&home)).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "44".repeat(32));
        assert_eq!(before, fs::metadata(&path).unwrap().permissions());
        #[cfg(windows)]
        {
            // Still rejected a second time: the first attempt did not repair it.
            assert!(crate::pseudonym_key::open_private_key_file(&path, 64, false).is_err());
        }
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn child_environment_removes_current_and_future_link_names() {
        let mut command = Command::new("unused-test-program");
        for key in [
            "AGENT_OTEL_LINK_KEY_HEX",
            "AGENT_OTEL_LINK_KEY_FILE",
            "AGENT_OTEL_LINK_FUTURE",
            "agent_otel_link_lower",
            "AGENT_OTEL_K_ORG_HEX",
        ] {
            command.env(key, "sentinel");
        }
        command.env("UNRELATED_SENTINEL", "preserved");
        remove_link_environment(&mut command);
        for (key, value) in command.get_envs() {
            if key
                .to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("AGENT_OTEL_LINK_")
                || key == "AGENT_OTEL_K_ORG_HEX"
            {
                assert!(value.is_none());
            } else if key == "UNRELATED_SENTINEL" {
                assert_eq!(value.unwrap(), "preserved");
            }
        }
    }
}
