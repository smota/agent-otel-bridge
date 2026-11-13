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
        let key = match get("AGENT_OTEL_LINK_KEY_HEX").map(Zeroizing::new) {
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
            let fallback_trace_id = input.conversation_id.as_deref().and_then(|session| {
                let message = Zeroizing::new(
                    lp_encode_str(&[
                        "agent.session.trace-fallback/v1",
                        config.org.as_deref().unwrap_or(""),
                        config.endpoint_scope.as_deref().unwrap_or(""),
                        session,
                    ])
                    .ok()?,
                );
                Some(trace_prefix(&hmac_sha256(&key[..], &message)))
            });
            SessionLink {
                link_id: AGENT_SESSION_LINK_UNLINKED.into(),
                reason: Some(reason),
                fallback_trace_id,
            }
        }
    }
}
