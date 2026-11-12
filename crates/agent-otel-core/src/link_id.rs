/*
 * Copyright agent-otel-bridge contributors
 * SPDX-License-Identifier: Apache-2.0
 */

//! Normative reference implementation of the `link-id-v1` session-to-trace link contract (`C1a`).
//!
//! Contract formula:
//! ```text
//! agent.session.link_id = hex(
//!     HMAC-SHA256(
//!         K_link,
//!         LP("agent.session.link/v1", org, endpoint_scope, canonical_harness, native_session)
//!     )
//! )
//! K_link = HKDF-SHA256(
//!     ikm  = K_org,
//!     salt = b"agent-otel-bridge:link-id-v1:salt",
//!     info = LP("agent.session.link.key/v1", team, epoch),
//!     L    = 32
//! )
//! ```
//!
//! Any event or session lacking a valid key, harness origin, organization, endpoint scope,
//! canonical harness from the closed vocabulary, or native session identifier resolves to the
//! sentinel string `"unlinked"` and is never guessed.

use crate::model::ClientKind;
use sha2::{Digest, Sha256};

/// Span and fact attribute key carrying the keyed session link identifier.
pub const AGENT_SESSION_LINK_ID: &str = "agent.session.link_id";

/// Sentinel attribute value emitted when a span or session cannot be deterministically linked.
pub const AGENT_SESSION_LINK_UNLINKED: &str = "unlinked";

/// Fixed domain tag (`f_0`) for the `LP(...)` session link message payload.
pub const LINK_ID_DOMAIN: &str = "agent.session.link/v1";

/// Fixed purpose tag (`f_0`) for the `LP(...)` HKDF `info` parameter.
pub const LINK_KEY_PURPOSE: &str = "agent.session.link.key/v1";

/// Fixed domain-separation salt for `HKDF-Extract` per RFC 5869 §3.1.
pub const LINK_KEY_HKDF_SALT: &[u8] = b"agent-otel-bridge:link-id-v1:salt";

/// Minimum length in bytes required for the organization master secret `K_org` (256 bits).
pub const MIN_ORG_MASTER_KEY_BYTES: usize = 32;

/// Output length in bytes of `K_link` and the `HMAC-SHA256` tag (256 bits).
pub const LINK_KEY_BYTES: usize = 32;

/// Maximum UTF-8 byte length permitted for any single contract identifier field.
pub const MAX_LINK_FIELD_BYTES: usize = 1024;

/// SHA-256 block size in bytes (512 bits).
const SHA256_BLOCK_BYTES: usize = 64;

/// SHA-256 digest size in bytes (256 bits).
const SHA256_DIGEST_BYTES: usize = 32;

/// Closed canonical harness vocabulary for `link-id-v1` (resolves Finding `F12`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CanonicalHarness {
    Antigravity,
    Claude,
    Codex,
    Grok,
    Pi,
}

impl CanonicalHarness {
    /// All five canonical harnesses in wire-tag order (`1..=5`).
    pub const ALL: [Self; 5] = [
        CanonicalHarness::Antigravity,
        CanonicalHarness::Claude,
        CanonicalHarness::Codex,
        CanonicalHarness::Grok,
        CanonicalHarness::Pi,
    ];

    /// Returns the canonical lowercase ASCII identifier used in `LP(...)`.
    #[inline]
    pub const fn as_str(&self) -> &'static str {
        match self {
            CanonicalHarness::Antigravity => "antigravity",
            CanonicalHarness::Claude => "claude",
            CanonicalHarness::Codex => "codex",
            CanonicalHarness::Grok => "grok",
            CanonicalHarness::Pi => "pi",
        }
    }

    /// Returns the IPC wire client ID (`1..=5`) associated with this canonical harness.
    #[inline]
    pub const fn wire_client_id(&self) -> u16 {
        match self {
            CanonicalHarness::Antigravity => 1,
            CanonicalHarness::Claude => 2,
            CanonicalHarness::Codex => 3,
            CanonicalHarness::Grok => 4,
            CanonicalHarness::Pi => 5,
        }
    }

    /// Returns the corresponding [`ClientKind`] for this canonical harness.
    #[inline]
    pub const fn client_kind(&self) -> ClientKind {
        match self {
            CanonicalHarness::Antigravity => ClientKind::Antigravity,
            CanonicalHarness::Claude => ClientKind::ClaudeCode,
            CanonicalHarness::Codex => ClientKind::Codex,
            CanonicalHarness::Grok => ClientKind::Grok,
            CanonicalHarness::Pi => ClientKind::Pi,
        }
    }

    /// Resolves a canonical harness from an IPC wire `ClientKind`.
    #[inline]
    pub const fn from_client_kind(kind: ClientKind) -> Option<Self> {
        match kind {
            ClientKind::Unspecified => None,
            ClientKind::Antigravity => Some(CanonicalHarness::Antigravity),
            ClientKind::ClaudeCode => Some(CanonicalHarness::Claude),
            ClientKind::Codex => Some(CanonicalHarness::Codex),
            ClientKind::Grok => Some(CanonicalHarness::Grok),
            ClientKind::Pi => Some(CanonicalHarness::Pi),
        }
    }

    /// Resolves a canonical harness from an IPC wire `client_id` (`1..=5`).
    #[inline]
    pub const fn from_wire_client_id(wire_id: u16) -> Option<Self> {
        match wire_id {
            1 => Some(CanonicalHarness::Antigravity),
            2 => Some(CanonicalHarness::Claude),
            3 => Some(CanonicalHarness::Codex),
            4 => Some(CanonicalHarness::Grok),
            5 => Some(CanonicalHarness::Pi),
            _ => None,
        }
    }

    /// Resolves a raw harness string against the closed `link-id-v1` alias table.
    ///
    /// Normalization trims leading and trailing ASCII whitespace and performs
    /// case-insensitive ASCII matching against the closed alias list.
    ///
    /// Provider/vendor names (`"google"`, `"anthropic"`, `"openai"`, `"xai"`, `"gemini"`)
    /// and fallback strings (`"ai-agent"`, `"unknown"`) are intentionally excluded
    /// per the `#29` attribution contract.
    pub fn from_alias(raw: &str) -> Option<Self> {
        let s = raw.trim();
        if s.eq_ignore_ascii_case("antigravity") || s.eq_ignore_ascii_case("agy") {
            Some(CanonicalHarness::Antigravity)
        } else if s.eq_ignore_ascii_case("claude")
            || s.eq_ignore_ascii_case("claude-code")
            || s.eq_ignore_ascii_case("claudecode")
        {
            Some(CanonicalHarness::Claude)
        } else if s.eq_ignore_ascii_case("codex") || s.eq_ignore_ascii_case("codex-cli") {
            Some(CanonicalHarness::Codex)
        } else if s.eq_ignore_ascii_case("grok") || s.eq_ignore_ascii_case("grok-cli") {
            Some(CanonicalHarness::Grok)
        } else if s.eq_ignore_ascii_case("pi") || s.eq_ignore_ascii_case("pi-cli") {
            Some(CanonicalHarness::Pi)
        } else {
            None
        }
    }
}

/// Errors returned when `K_link` key derivation or `LP(...)` framing fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKeyError {
    /// `K_org` is shorter than [`MIN_ORG_MASTER_KEY_BYTES`] (32 bytes).
    MasterKeyTooShort,
    /// `team` is empty, whitespace-only, exceeds [`MAX_LINK_FIELD_BYTES`], has outer whitespace,
    /// or contains ASCII control characters.
    InvalidTeam,
    /// `epoch` is empty, whitespace-only, exceeds [`MAX_LINK_FIELD_BYTES`], has outer whitespace,
    /// or contains ASCII control characters.
    InvalidEpoch,
    /// Requested HKDF output length exceeds `255 * 32` bytes (RFC 5869 §2.3).
    InvalidOkmLength,
    /// A field passed to `lp_encode` exceeds `u32::MAX` bytes.
    FieldTooLarge,
}

/// Deterministic reason code explaining why a span or session resolved to `"unlinked"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnlinkedReason {
    /// Key derivation inputs (`K_org`, `team`, or `epoch`) were missing or invalid.
    InvalidKeyMaterial,
    /// Event `origin` is not `"harness"` (for example, `"synthetic"` test/collision fixtures).
    NonHarnessOrigin,
    /// `org` is absent, empty, or whitespace-only.
    MissingOrg,
    /// `endpoint_scope` is absent, empty, or whitespace-only.
    MissingEndpointScope,
    /// Neither a valid wire client ID nor a non-empty harness name was supplied.
    MissingHarness,
    /// Supplied wire client ID or harness name is not in the closed vocabulary.
    UnknownHarness,
    /// Both `wire_client_id` and `raw_harness` were supplied and resolved to different harnesses.
    ConflictingHarness,
    /// `native_session` is absent, empty, or whitespace-only.
    MissingNativeSession,
    /// `org`, `endpoint_scope`, or `native_session` contains ASCII control characters, NUL bytes,
    /// leading/trailing ASCII whitespace, or exceeds [`MAX_LINK_FIELD_BYTES`].
    InvalidFieldCharacters,
}

impl UnlinkedReason {
    /// Returns the normative snake_case code used in contract test vectors.
    #[inline]
    pub const fn as_str(&self) -> &'static str {
        match self {
            UnlinkedReason::InvalidKeyMaterial => "invalid_key_material",
            UnlinkedReason::NonHarnessOrigin => "non_harness_origin",
            UnlinkedReason::MissingOrg => "missing_org",
            UnlinkedReason::MissingEndpointScope => "missing_endpoint_scope",
            UnlinkedReason::MissingHarness => "missing_harness",
            UnlinkedReason::UnknownHarness => "unknown_harness",
            UnlinkedReason::ConflictingHarness => "conflicting_harness",
            UnlinkedReason::MissingNativeSession => "missing_native_session",
            UnlinkedReason::InvalidFieldCharacters => "invalid_field_characters",
        }
    }
}

/// Intermediate and final outputs of `K_link` derivation via RFC 5869 `HKDF-SHA256`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedLinkKey {
    /// Length-prefixed `info = LP("agent.session.link.key/v1", team, epoch)`.
    pub hkdf_info: Vec<u8>,
    /// Pseudorandom key `PRK = HMAC-SHA256(salt, K_org)`.
    pub prk: [u8; 32],
    /// Derived 32-byte link key `K_link = HKDF-Expand(PRK, info, 32)`.
    pub k_link: [u8; 32],
}

/// Input parameters for deriving `agent.session.link_id` on an event or session record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkIdInput<'a> {
    /// Canonical organization identifier.
    pub org: Option<&'a str>,
    /// Jointly provisioned, persistent endpoint namespace (never a daemon instance ID).
    pub endpoint_scope: Option<&'a str>,
    /// Optional IPC wire header `client_id` (`0` = `Unspecified`, `1..=5` = known harnesses).
    pub wire_client_id: Option<u16>,
    /// Optional harness string (e.g. `gen_ai.agent.name` or dendro `harness`).
    pub raw_harness: Option<&'a str>,
    /// Harness-native session / conversation identifier.
    pub native_session: Option<&'a str>,
    /// Event origin classification (`None` or `Some("harness")` for real harness events).
    pub origin: Option<&'a str>,
}

/// Detailed result of `agent.session.link_id` derivation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkIdOutcome {
    /// All inputs were valid; carries the derived 64-character hex link ID and intermediates.
    Linked {
        canonical_harness: CanonicalHarness,
        lp_payload: Vec<u8>,
        tag: [u8; 32],
        link_id: String,
    },
    /// One or more inputs were missing, synthetic, or invalid; resolves to `"unlinked"`.
    Unlinked { reason: UnlinkedReason },
}

impl LinkIdOutcome {
    /// Returns the string value to emit for `agent.session.link_id` (64-char hex or `"unlinked"`).
    #[inline]
    pub fn as_attribute_value(&self) -> &str {
        match self {
            LinkIdOutcome::Linked { link_id, .. } => link_id.as_str(),
            LinkIdOutcome::Unlinked { .. } => AGENT_SESSION_LINK_UNLINKED,
        }
    }

    /// Returns `true` if a keyed link ID was produced.
    #[inline]
    pub const fn is_linked(&self) -> bool {
        matches!(self, LinkIdOutcome::Linked { .. })
    }
}

/// Computes `HMAC-SHA256(key, message)` per RFC 2104 over `sha2::Sha256`.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; SHA256_DIGEST_BYTES] {
    let mut k_block = [0u8; SHA256_BLOCK_BYTES];
    if key.len() > SHA256_BLOCK_BYTES {
        let hashed = Sha256::digest(key);
        k_block[..SHA256_DIGEST_BYTES].copy_from_slice(&hashed);
    } else {
        k_block[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; SHA256_BLOCK_BYTES];
    let mut opad = [0x5cu8; SHA256_BLOCK_BYTES];
    for i in 0..SHA256_BLOCK_BYTES {
        ipad[i] ^= k_block[i];
        opad[i] ^= k_block[i];
    }

    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(message);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_digest);
    let outer_digest = outer.finalize();

    let mut out = [0u8; SHA256_DIGEST_BYTES];
    out.copy_from_slice(&outer_digest);
    out
}

/// Computes `HKDF-Extract(salt, IKM)` using `HMAC-SHA256` per RFC 5869 §2.2.
///
/// If `salt` is empty, a 32-byte zero-filled salt is used per RFC 5869 §2.2.
pub fn hkdf_extract_sha256(salt: &[u8], ikm: &[u8]) -> [u8; SHA256_DIGEST_BYTES] {
    let zero_salt = [0u8; SHA256_DIGEST_BYTES];
    let effective_salt = if salt.is_empty() { &zero_salt } else { salt };
    hmac_sha256(effective_salt, ikm)
}

/// Computes `HKDF-Expand(PRK, info, L)` using `HMAC-SHA256` per RFC 5869 §2.3.
pub fn hkdf_expand_sha256(
    prk: &[u8; SHA256_DIGEST_BYTES],
    info: &[u8],
    okm_len: usize,
) -> Result<Vec<u8>, LinkKeyError> {
    if okm_len > 255 * SHA256_DIGEST_BYTES {
        return Err(LinkKeyError::InvalidOkmLength);
    }
    if okm_len == 0 {
        return Ok(Vec::new());
    }

    let n = okm_len.div_ceil(SHA256_DIGEST_BYTES);
    let mut okm = Vec::with_capacity(n * SHA256_DIGEST_BYTES);
    let mut t_prev = [0u8; SHA256_DIGEST_BYTES];
    let mut has_prev = false;

    for i in 1..=n {
        let mut msg =
            Vec::with_capacity((if has_prev { SHA256_DIGEST_BYTES } else { 0 }) + info.len() + 1);
        if has_prev {
            msg.extend_from_slice(&t_prev);
        }
        msg.extend_from_slice(info);
        msg.push(i as u8);

        t_prev = hmac_sha256(prk, &msg);
        has_prev = true;
        okm.extend_from_slice(&t_prev);
    }

    okm.truncate(okm_len);
    Ok(okm)
}

/// Computes `HKDF-SHA256(ikm, salt, info, okm_len)` per RFC 5869.
pub fn hkdf_sha256(
    ikm: &[u8],
    salt: &[u8],
    info: &[u8],
    okm_len: usize,
) -> Result<Vec<u8>, LinkKeyError> {
    let prk = hkdf_extract_sha256(salt, ikm);
    hkdf_expand_sha256(&prk, info, okm_len)
}

/// Encodes a sequence of byte slices using unambiguous 32-bit big-endian length prefixing (`LP`).
///
/// Each field `f_i` is serialized as `u32::to_be_bytes(f_i.len()) || f_i`.
pub fn lp_encode(fields: &[&[u8]]) -> Result<Vec<u8>, LinkKeyError> {
    let mut total_len = 0usize;
    for field in fields {
        if u32::try_from(field.len()).is_err() {
            return Err(LinkKeyError::FieldTooLarge);
        }
        total_len = total_len
            .checked_add(4)
            .and_then(|v| v.checked_add(field.len()))
            .ok_or(LinkKeyError::FieldTooLarge)?;
    }

    let mut out = Vec::with_capacity(total_len);
    for field in fields {
        let len_u32 = field.len() as u32;
        out.extend_from_slice(&len_u32.to_be_bytes());
        out.extend_from_slice(field);
    }
    Ok(out)
}

/// Encodes a sequence of UTF-8 string slices using `LP` (`u32` big-endian byte length + UTF-8 bytes).
pub fn lp_encode_str(fields: &[&str]) -> Result<Vec<u8>, LinkKeyError> {
    let mut total_len = 0usize;
    for field in fields {
        if u32::try_from(field.len()).is_err() {
            return Err(LinkKeyError::FieldTooLarge);
        }
        total_len = total_len
            .checked_add(4)
            .and_then(|v| v.checked_add(field.len()))
            .ok_or(LinkKeyError::FieldTooLarge)?;
    }

    let mut out = Vec::with_capacity(total_len);
    for field in fields {
        let bytes = field.as_bytes();
        let len_u32 = bytes.len() as u32;
        out.extend_from_slice(&len_u32.to_be_bytes());
        out.extend_from_slice(bytes);
    }
    Ok(out)
}

/// Validates that a non-empty contract field has no leading/trailing ASCII whitespace,
/// contains no ASCII control characters (`0x00..=0x1F` or `0x7F`), and is at most
/// [`MAX_LINK_FIELD_BYTES`] bytes long.
#[inline]
pub fn is_valid_contract_field(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_LINK_FIELD_BYTES {
        return false;
    }
    if value.trim() != value {
        return false;
    }
    !value.bytes().any(|b| b <= 0x1f || b == 0x7f)
}

/// Derives `K_link` and its intermediate `hkdf_info` and `prk` values per `link-id-v1`.
pub fn derive_link_key_detailed(
    k_org: &[u8],
    team: &str,
    epoch: &str,
) -> Result<DerivedLinkKey, LinkKeyError> {
    if k_org.len() < MIN_ORG_MASTER_KEY_BYTES {
        return Err(LinkKeyError::MasterKeyTooShort);
    }
    if !is_valid_contract_field(team) {
        return Err(LinkKeyError::InvalidTeam);
    }
    if !is_valid_contract_field(epoch) {
        return Err(LinkKeyError::InvalidEpoch);
    }

    let hkdf_info = lp_encode_str(&[LINK_KEY_PURPOSE, team, epoch])?;
    let prk = hkdf_extract_sha256(LINK_KEY_HKDF_SALT, k_org);
    let okm = hkdf_expand_sha256(&prk, &hkdf_info, LINK_KEY_BYTES)?;
    let mut k_link = [0u8; LINK_KEY_BYTES];
    k_link.copy_from_slice(&okm);

    Ok(DerivedLinkKey {
        hkdf_info,
        prk,
        k_link,
    })
}

/// Derives the 32-byte scoped key `K_link = HKDF-SHA256(K_org, salt, LP(purpose, team, epoch))`.
pub fn derive_link_key(
    k_org: &[u8],
    team: &str,
    epoch: &str,
) -> Result<[u8; LINK_KEY_BYTES], LinkKeyError> {
    Ok(derive_link_key_detailed(k_org, team, epoch)?.k_link)
}

/// Resolves canonical harness identity from optional IPC `wire_client_id` and/or `raw_harness`.
///
/// - `wire_client_id == Some(0)` (`ClientKind::Unspecified`) is treated as unasserted on the wire.
/// - Any non-zero `wire_client_id` outside `1..=5` fails closed with [`UnlinkedReason::UnknownHarness`].
/// - If both a non-zero `wire_client_id` and `raw_harness` are provided, both must resolve to the
///   same [`CanonicalHarness`] or derivation fails closed with [`UnlinkedReason::ConflictingHarness`]
///   (or [`UnlinkedReason::UnknownHarness`] if `raw_harness` is unrecognized).
pub fn resolve_harness_identity(
    wire_client_id: Option<u16>,
    raw_harness: Option<&str>,
) -> Result<CanonicalHarness, UnlinkedReason> {
    let wire_resolved = match wire_client_id {
        None | Some(0) => None,
        Some(id) => {
            Some(CanonicalHarness::from_wire_client_id(id).ok_or(UnlinkedReason::UnknownHarness)?)
        }
    };

    let str_resolved = match raw_harness {
        None => None,
        Some(s) if s.trim().is_empty() => {
            if wire_resolved.is_none() {
                return Err(UnlinkedReason::MissingHarness);
            }
            None
        }
        Some(s) => Some(CanonicalHarness::from_alias(s).ok_or(UnlinkedReason::UnknownHarness)?),
    };

    match (wire_resolved, str_resolved) {
        (Some(w), Some(s)) => {
            if w == s {
                Ok(w)
            } else {
                Err(UnlinkedReason::ConflictingHarness)
            }
        }
        (Some(w), None) => Ok(w),
        (None, Some(s)) => Ok(s),
        (None, None) => Err(UnlinkedReason::MissingHarness),
    }
}

/// Derives `agent.session.link_id` from a pre-derived 32-byte `K_link` and [`LinkIdInput`].
pub fn derive_session_link_with_key(
    k_link: &[u8; LINK_KEY_BYTES],
    input: &LinkIdInput<'_>,
) -> LinkIdOutcome {
    if let Some(origin) = input.origin {
        if origin != "harness" {
            return LinkIdOutcome::Unlinked {
                reason: UnlinkedReason::NonHarnessOrigin,
            };
        }
    }

    let org = match input.org {
        Some(s) if !s.trim().is_empty() => s,
        _ => {
            return LinkIdOutcome::Unlinked {
                reason: UnlinkedReason::MissingOrg,
            }
        }
    };
    if !is_valid_contract_field(org) {
        return LinkIdOutcome::Unlinked {
            reason: UnlinkedReason::InvalidFieldCharacters,
        };
    }

    let endpoint_scope = match input.endpoint_scope {
        Some(s) if !s.trim().is_empty() => s,
        _ => {
            return LinkIdOutcome::Unlinked {
                reason: UnlinkedReason::MissingEndpointScope,
            }
        }
    };
    if !is_valid_contract_field(endpoint_scope) {
        return LinkIdOutcome::Unlinked {
            reason: UnlinkedReason::InvalidFieldCharacters,
        };
    }

    let canonical_harness = match resolve_harness_identity(input.wire_client_id, input.raw_harness)
    {
        Ok(h) => h,
        Err(reason) => return LinkIdOutcome::Unlinked { reason },
    };

    let native_session = match input.native_session {
        Some(s) if !s.trim().is_empty() => s,
        _ => {
            return LinkIdOutcome::Unlinked {
                reason: UnlinkedReason::MissingNativeSession,
            }
        }
    };
    if !is_valid_contract_field(native_session) {
        return LinkIdOutcome::Unlinked {
            reason: UnlinkedReason::InvalidFieldCharacters,
        };
    }

    let lp_payload = match lp_encode_str(&[
        LINK_ID_DOMAIN,
        org,
        endpoint_scope,
        canonical_harness.as_str(),
        native_session,
    ]) {
        Ok(bytes) => bytes,
        Err(_) => {
            return LinkIdOutcome::Unlinked {
                reason: UnlinkedReason::InvalidFieldCharacters,
            }
        }
    };

    let tag = hmac_sha256(k_link, &lp_payload);
    let link_id = hex_encode(&tag);

    LinkIdOutcome::Linked {
        canonical_harness,
        lp_payload,
        tag,
        link_id,
    }
}

/// Derives `agent.session.link_id` end-to-end from `(K_org, team, epoch)` and [`LinkIdInput`].
///
/// If key derivation fails, returns `LinkIdOutcome::Unlinked { reason: UnlinkedReason::InvalidKeyMaterial }`.
pub fn derive_session_link_id(
    k_org: &[u8],
    team: &str,
    epoch: &str,
    input: &LinkIdInput<'_>,
) -> LinkIdOutcome {
    let k_link = match derive_link_key(k_org, team, epoch) {
        Ok(k) => k,
        Err(_) => {
            return LinkIdOutcome::Unlinked {
                reason: UnlinkedReason::InvalidKeyMaterial,
            }
        }
    };
    derive_session_link_with_key(&k_link, input)
}

/// Computes candidate `(epoch, link_id)` pairs across an ordered slice of epochs
/// (for example `[epoch_curr, epoch_prev]` during a bounded epoch-rotation overlap window).
pub fn derive_session_link_candidates_across_epochs(
    k_org: &[u8],
    team: &str,
    epochs: &[&str],
    input: &LinkIdInput<'_>,
) -> Result<Vec<(String, String)>, UnlinkedReason> {
    if epochs.is_empty() {
        return Err(UnlinkedReason::InvalidKeyMaterial);
    }
    let mut candidates = Vec::with_capacity(epochs.len());
    for &epoch in epochs {
        match derive_session_link_id(k_org, team, epoch, input) {
            LinkIdOutcome::Linked { link_id, .. } => {
                candidates.push((epoch.to_string(), link_id));
            }
            LinkIdOutcome::Unlinked { reason } => return Err(reason),
        }
    }
    Ok(candidates)
}

/// Encodes a byte slice as lowercase ASCII hexadecimal.
pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX_CHARS[(b >> 4) as usize] as char);
        out.push(HEX_CHARS[(b & 0x0f) as usize] as char);
    }
    out
}

/// Decodes a hexadecimal string (case-insensitive) into bytes.
pub fn hex_decode(hex: &str) -> Option<Vec<u8>> {
    let bytes = hex.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for &[hi_byte, lo_byte] in bytes.as_chunks::<2>().0 {
        let hi = decode_nibble(hi_byte)?;
        let lo = decode_nibble(lo_byte)?;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

#[inline]
fn decode_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies all 7 official HMAC-SHA256 test vectors from RFC 4231 §4.
    #[test]
    fn rfc4231_hmac_sha256_official_vectors() {
        // Test Case 1
        let key1 = [0x0bu8; 20];
        let data1 = b"Hi There";
        assert_eq!(
            hex_encode(&hmac_sha256(&key1, data1)),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );

        // Test Case 2
        let key2 = b"Jefe";
        let data2 = b"what do ya want for nothing?";
        assert_eq!(
            hex_encode(&hmac_sha256(key2, data2)),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );

        // Test Case 3
        let key3 = [0xaau8; 20];
        let data3 = [0xddu8; 50];
        assert_eq!(
            hex_encode(&hmac_sha256(&key3, &data3)),
            "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe"
        );

        // Test Case 4
        let key4 = hex_decode("0102030405060708090a0b0c0d0e0f10111213141516171819").unwrap();
        let data4 = [0xcdu8; 50];
        assert_eq!(
            hex_encode(&hmac_sha256(&key4, &data4)),
            "82558a389a443c0ea4cc819899f2083a85f0faa3e578f8077a2e3ff46729665b"
        );

        // Test Case 5 (truncation to 128 bits = first 32 hex chars)
        let key5 = [0x0cu8; 20];
        let data5 = b"Test With Truncation";
        let mac5 = hex_encode(&hmac_sha256(&key5, data5));
        assert_eq!(&mac5[..32], "a3b6167473100ee06e0c796c2955552b");

        // Test Case 6 (key larger than 64-byte block size: 131 bytes)
        let key6 = [0xaau8; 131];
        let data6 = b"Test Using Larger Than Block-Size Key - Hash Key First";
        assert_eq!(
            hex_encode(&hmac_sha256(&key6, data6)),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );

        // Test Case 7 (key larger than block size: 131 bytes, with multi-block data)
        let key7 = [0xaau8; 131];
        let data7 = b"This is a test using a larger than block-size key and a larger than block-size data. The key needs to be hashed before being used by the HMAC algorithm.";
        assert_eq!(
            hex_encode(&hmac_sha256(&key7, data7)),
            "9b09ffa71b942fcb27635fbcd5b0e944bfdc63644f0713938a7f51535c3a35e2"
        );
    }

    /// Verifies all 3 official HKDF-SHA256 test vectors from RFC 5869 Appendix A.1–A.3.
    #[test]
    fn rfc5869_hkdf_sha256_official_vectors() {
        // Test Case 1: Basic test case with SHA-256
        let ikm1 = [0x0bu8; 22];
        let salt1 = hex_decode("000102030405060708090a0b0c").unwrap();
        let info1 = hex_decode("f0f1f2f3f4f5f6f7f8f9").unwrap();
        let prk1 = hkdf_extract_sha256(&salt1, &ikm1);
        assert_eq!(
            hex_encode(&prk1),
            "077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5"
        );
        let okm1 = hkdf_expand_sha256(&prk1, &info1, 42).unwrap();
        assert_eq!(
            hex_encode(&okm1),
            "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
        );

        // Test Case 2: Test with SHA-256 and longer inputs/outputs
        let ikm2: Vec<u8> = (0x00u8..=0x4fu8).collect();
        let salt2: Vec<u8> = (0x60u8..=0xafu8).collect();
        let info2: Vec<u8> = (0xb0u8..=0xffu8).collect();
        let prk2 = hkdf_extract_sha256(&salt2, &ikm2);
        assert_eq!(
            hex_encode(&prk2),
            "06a6b88c5853361a06104c9ceb35b45cef760014904671014a193f40c15fc244"
        );
        let okm2 = hkdf_expand_sha256(&prk2, &info2, 82).unwrap();
        assert_eq!(
            hex_encode(&okm2),
            "b11e398dc80327a1c8e7f78c596a49344f012eda2d4efad8a050cc4c19afa97c59045a99cac7827271cb41c65e590e09da3275600c2f09b8367793a9aca3db71cc30c58179ec3e87c14c01d5c1f3434f1d87"
        );

        // Test Case 3: Test with SHA-256 and zero-length salt/info
        let ikm3 = [0x0bu8; 22];
        let prk3 = hkdf_extract_sha256(&[], &ikm3);
        assert_eq!(
            hex_encode(&prk3),
            "19ef24a32c717b167f33a91d6f648bdf96596776afdb6377ac434c1c293ccb04"
        );
        let okm3 = hkdf_expand_sha256(&prk3, &[], 42).unwrap();
        assert_eq!(
            hex_encode(&okm3),
            "8da4e775a563c18f715f802a063c5a31b8a11f5c5ee1879ec3454e5f3c738d2d9d201395faa4b61a96c8"
        );
    }

    #[test]
    fn lp_encoding_prevents_concatenation_ambiguity() {
        let a = lp_encode_str(&["ab", "c"]).unwrap();
        let b = lp_encode_str(&["a", "bc"]).unwrap();
        assert_ne!(a, b);
        assert_eq!(hex_encode(&a), "0000000261620000000163");
        assert_eq!(hex_encode(&b), "0000000161000000026263");
    }

    #[test]
    fn canonical_harness_resolves_f12_and_rejects_providers() {
        assert_eq!(
            CanonicalHarness::from_alias("claude"),
            Some(CanonicalHarness::Claude)
        );
        assert_eq!(
            CanonicalHarness::from_alias("claude-code"),
            Some(CanonicalHarness::Claude)
        );
        assert_eq!(
            CanonicalHarness::from_alias("claudecode"),
            Some(CanonicalHarness::Claude)
        );
        assert_eq!(
            CanonicalHarness::from_alias("antigravity"),
            Some(CanonicalHarness::Antigravity)
        );
        assert_eq!(
            CanonicalHarness::from_alias("agy"),
            Some(CanonicalHarness::Antigravity)
        );
        assert_eq!(
            CanonicalHarness::from_alias("codex"),
            Some(CanonicalHarness::Codex)
        );
        assert_eq!(
            CanonicalHarness::from_alias("codex-cli"),
            Some(CanonicalHarness::Codex)
        );
        assert_eq!(
            CanonicalHarness::from_alias("grok"),
            Some(CanonicalHarness::Grok)
        );
        assert_eq!(
            CanonicalHarness::from_alias("grok-cli"),
            Some(CanonicalHarness::Grok)
        );
        assert_eq!(
            CanonicalHarness::from_alias("pi"),
            Some(CanonicalHarness::Pi)
        );
        assert_eq!(
            CanonicalHarness::from_alias("pi-cli"),
            Some(CanonicalHarness::Pi)
        );

        // Provider/vendor names and fallbacks MUST NOT be accepted as harness aliases.
        for forbidden in [
            "google",
            "anthropic",
            "openai",
            "xai",
            "gemini",
            "inflection",
            "ai-agent",
            "unknown",
            "cursor",
            "",
        ] {
            assert_eq!(
                CanonicalHarness::from_alias(forbidden),
                None,
                "forbidden string '{forbidden}' must not resolve to a canonical harness"
            );
        }
    }

    fn contract_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("docs")
            .join("contracts")
            .join("fact-export-v1")
            .join("link-id-v1")
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        let digest = Sha256::digest(bytes);
        hex_encode(digest.as_slice())
    }

    #[test]
    fn vocabulary_json_matches_rust_canonical_harness() {
        let vocab_bytes = std::fs::read(contract_dir().join("vocabulary.json")).unwrap();
        let vocab: serde_json::Value = serde_json::from_slice(&vocab_bytes).unwrap();
        assert_eq!(vocab["contract"].as_str(), Some("link-id-v1"));
        assert_eq!(
            vocab["unknown_behavior"].as_str(),
            Some(AGENT_SESSION_LINK_UNLINKED)
        );

        let entries = vocab["canonical_harnesses"].as_array().unwrap();
        assert_eq!(entries.len(), CanonicalHarness::ALL.len());

        let aliases_map = vocab["aliases"].as_object().unwrap();
        for (alias, canon_val) in aliases_map {
            let expected = CanonicalHarness::from_alias(canon_val.as_str().unwrap()).unwrap();
            let resolved = CanonicalHarness::from_alias(alias)
                .unwrap_or_else(|| panic!("alias '{alias}' failed to resolve in Rust"));
            assert_eq!(resolved, expected);
            assert_eq!(resolved.as_str(), canon_val.as_str().unwrap());
        }

        let wire_map = vocab["wire_client_ids"].as_object().unwrap();
        for (wire_str, canon_val) in wire_map {
            let id: u16 = wire_str.parse().unwrap();
            let resolved = CanonicalHarness::from_wire_client_id(id).unwrap();
            assert_eq!(resolved.as_str(), canon_val.as_str().unwrap());
            assert_eq!(
                CanonicalHarness::from_client_kind(resolved.client_kind()),
                Some(resolved)
            );
        }
    }

    #[test]
    fn contract_vectors_and_digests_match_rust_reference() {
        use std::collections::{HashMap, HashSet};

        let base = contract_dir();
        let repo_root = base
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap();

        let suites = [
            "basic.json",
            "collisions.json",
            "rotation.json",
            "negative.json",
            "g0a.json",
        ];

        let mut total_vectors = 0usize;

        for suite_file in suites {
            let raw = std::fs::read(base.join("vectors").join(suite_file)).unwrap();
            let doc: serde_json::Value = serde_json::from_slice(&raw).unwrap();
            assert_eq!(doc["contract"].as_str(), Some("link-id-v1"));

            let vectors = doc["vectors"].as_array().unwrap();
            let mut by_id: HashMap<String, &serde_json::Value> = HashMap::new();

            for vec in vectors {
                let vid = vec["id"].as_str().unwrap().to_string();
                assert!(
                    by_id.insert(vid.clone(), vec).is_none(),
                    "duplicate vector id {vid} in {suite_file}"
                );

                let inp = &vec["inputs"];
                assert_eq!(
                    inp["salt_hex"].as_str().unwrap(),
                    hex_encode(LINK_KEY_HKDF_SALT)
                );
                let k_org = hex_decode(inp["k_org_hex"].as_str().unwrap()).unwrap();
                let team = inp["team"].as_str();
                let epoch = inp["epoch"].as_str();
                let org = inp["org"].as_str();
                let endpoint_scope = inp["endpoint_scope"].as_str();
                let wire_client_id = inp["wire_client_id"].as_u64().map(|v| v as u16);
                let raw_harness = inp["raw_harness"].as_str();
                let native_session = inp["native_session"].as_str();
                let origin = inp["origin"].as_str();

                let link_input = LinkIdInput {
                    org,
                    endpoint_scope,
                    wire_client_id,
                    raw_harness,
                    native_session,
                    origin,
                };

                let inter = &vec["intermediates"];
                let exp = &vec["expected"];

                match (team, epoch) {
                    (Some(t), Some(e)) => match derive_link_key_detailed(&k_org, t, e) {
                        Ok(derived_key) => {
                            assert_eq!(
                                Some(hex_encode(&derived_key.hkdf_info).as_str()),
                                inter["hkdf_info_hex"].as_str(),
                                "{suite_file}:{vid} hkdf_info_hex mismatch"
                            );
                            assert_eq!(
                                Some(hex_encode(&derived_key.prk).as_str()),
                                inter["prk_hex"].as_str(),
                                "{suite_file}:{vid} prk_hex mismatch"
                            );
                            assert_eq!(
                                Some(hex_encode(&derived_key.k_link).as_str()),
                                inter["k_link_hex"].as_str(),
                                "{suite_file}:{vid} k_link_hex mismatch"
                            );

                            let outcome = derive_session_link_id(&k_org, t, e, &link_input);
                            match outcome {
                                LinkIdOutcome::Linked {
                                    canonical_harness,
                                    lp_payload,
                                    link_id,
                                    ..
                                } => {
                                    assert_eq!(
                                        Some(canonical_harness.as_str()),
                                        inter["canonical_harness"].as_str(),
                                        "{suite_file}:{vid} canonical_harness mismatch"
                                    );
                                    assert_eq!(
                                        Some(hex_encode(&lp_payload).as_str()),
                                        inter["lp_payload_hex"].as_str(),
                                        "{suite_file}:{vid} lp_payload_hex mismatch"
                                    );
                                    assert_eq!(
                                        link_id,
                                        exp["link_id"].as_str().unwrap(),
                                        "{suite_file}:{vid} link_id mismatch"
                                    );
                                    assert!(
                                        exp["unlinked_reason"].is_null(),
                                        "{suite_file}:{vid} expected null unlinked_reason"
                                    );
                                }
                                LinkIdOutcome::Unlinked { reason } => {
                                    assert_eq!(
                                        exp["link_id"].as_str().unwrap(),
                                        AGENT_SESSION_LINK_UNLINKED,
                                        "{suite_file}:{vid} expected unlinked"
                                    );
                                    assert_eq!(
                                        Some(reason.as_str()),
                                        exp["unlinked_reason"].as_str(),
                                        "{suite_file}:{vid} unlinked_reason mismatch"
                                    );
                                }
                            }
                        }
                        Err(_) => {
                            assert!(inter["k_link_hex"].is_null());
                            assert_eq!(exp["link_id"].as_str(), Some(AGENT_SESSION_LINK_UNLINKED));
                            assert_eq!(
                                exp["unlinked_reason"].as_str(),
                                Some(UnlinkedReason::InvalidKeyMaterial.as_str())
                            );
                        }
                    },
                    _ => {
                        assert!(inter["k_link_hex"].is_null());
                        assert_eq!(exp["link_id"].as_str(), Some(AGENT_SESSION_LINK_UNLINKED));
                        assert_eq!(
                            exp["unlinked_reason"].as_str(),
                            Some(UnlinkedReason::InvalidKeyMaterial.as_str())
                        );
                    }
                }

                // Verify G0a capture file provenance if present
                if let Some(prov) = vec.get("g0a_provenance") {
                    for (file_key, sha_key) in [
                        ("hook_file", "hook_file_sha256"),
                        ("traces_file", "traces_file_sha256"),
                        ("session_log_file", "session_log_file_sha256"),
                    ] {
                        if let Some(rel_path) = prov.get(file_key).and_then(|v| v.as_str()) {
                            let expected_sha = prov[sha_key].as_str().unwrap();
                            let full_path = repo_root.join(rel_path);
                            let file_bytes = std::fs::read(&full_path).unwrap_or_else(|e| {
                                panic!("failed to read G0a capture {}: {e}", full_path.display())
                            });
                            assert_eq!(
                                sha256_hex(&file_bytes),
                                expected_sha,
                                "{suite_file}:{vid} capture SHA-256 mismatch for {rel_path}"
                            );
                            let file_str = std::str::from_utf8(&file_bytes).unwrap();
                            assert!(
                                file_str.contains(native_session.unwrap()),
                                "{suite_file}:{vid} native_session not found in {rel_path}"
                            );
                        }
                    }
                }

                total_vectors += 1;
            }

            if let Some(groups) = doc
                .get("assert_equal_link_id_groups")
                .and_then(|v| v.as_array())
            {
                for group in groups {
                    let ids: HashSet<&str> = group
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|id| {
                            by_id[id.as_str().unwrap()]["expected"]["link_id"]
                                .as_str()
                                .unwrap()
                        })
                        .collect();
                    assert_eq!(ids.len(), 1, "{suite_file} equal group failed: {group:?}");
                    assert!(!ids.contains(AGENT_SESSION_LINK_UNLINKED));
                }
            }

            if let Some(groups) = doc
                .get("assert_distinct_link_id_groups")
                .and_then(|v| v.as_array())
            {
                for group in groups {
                    let arr = group.as_array().unwrap();
                    let ids: HashSet<&str> = arr
                        .iter()
                        .map(|id| {
                            by_id[id.as_str().unwrap()]["expected"]["link_id"]
                                .as_str()
                                .unwrap()
                        })
                        .collect();
                    assert_eq!(
                        ids.len(),
                        arr.len(),
                        "{suite_file} distinct link_id group failed: {group:?}"
                    );
                }
            }

            if let Some(groups) = doc
                .get("assert_distinct_k_link_groups")
                .and_then(|v| v.as_array())
            {
                for group in groups {
                    let arr = group.as_array().unwrap();
                    let keys: HashSet<&str> = arr
                        .iter()
                        .map(|id| {
                            by_id[id.as_str().unwrap()]["intermediates"]["k_link_hex"]
                                .as_str()
                                .unwrap()
                        })
                        .collect();
                    assert_eq!(
                        keys.len(),
                        arr.len(),
                        "{suite_file} distinct k_link group failed: {group:?}"
                    );
                }
            }

            if let Some(ov) = doc.get("overlap_window_example") {
                let ref_inp = &by_id["rotation_epoch_2026_q4"]["inputs"];
                let k_org = hex_decode(ref_inp["k_org_hex"].as_str().unwrap()).unwrap();
                let team = ref_inp["team"].as_str().unwrap();
                let active_epochs: Vec<&str> = ov["active_epochs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap())
                    .collect();
                let link_input = LinkIdInput {
                    org: ref_inp["org"].as_str(),
                    endpoint_scope: ref_inp["endpoint_scope"].as_str(),
                    wire_client_id: None,
                    raw_harness: ref_inp["raw_harness"].as_str(),
                    native_session: ref_inp["native_session"].as_str(),
                    origin: ref_inp["origin"].as_str(),
                };
                let candidates = derive_session_link_candidates_across_epochs(
                    &k_org,
                    team,
                    &active_epochs,
                    &link_input,
                )
                .unwrap();
                let expected_arr = ov["expected_candidates"].as_array().unwrap();
                assert_eq!(candidates.len(), expected_arr.len());
                for ((ep, lid), exp_item) in candidates.iter().zip(expected_arr.iter()) {
                    assert_eq!(ep, exp_item["epoch"].as_str().unwrap());
                    assert_eq!(lid, exp_item["link_id"].as_str().unwrap());
                }
            }
        }

        assert!(total_vectors >= 50, "expected at least 50 contract vectors");

        // Verify vectors/SHA256SUMS
        let sha_sums = std::fs::read_to_string(base.join("vectors").join("SHA256SUMS")).unwrap();
        for line in sha_sums.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let (expected_sha, fname) = line.split_once("  ").unwrap();
            let file_bytes = std::fs::read(base.join("vectors").join(fname)).unwrap();
            assert_eq!(
                sha256_hex(&file_bytes),
                expected_sha,
                "vectors/SHA256SUMS mismatch for {fname}"
            );
        }

        // Verify CONTRACT-DIGEST
        let digest_manifest = std::fs::read_to_string(base.join("CONTRACT-DIGEST")).unwrap();
        let mut entry_lines = String::new();
        let mut declared_bundle_sha: Option<String> = None;
        for line in digest_manifest.lines() {
            let s = line.trim();
            if s.is_empty() {
                continue;
            }
            if let Some(rest) = s.strip_prefix("# BUNDLE-SHA256:") {
                declared_bundle_sha = Some(rest.trim().to_string());
                continue;
            }
            if s.starts_with('#') {
                continue;
            }
            let (expected_sha, rel_path) = s.split_once("  ").unwrap();
            let file_bytes = std::fs::read(base.join(rel_path)).unwrap();
            assert_eq!(
                sha256_hex(&file_bytes),
                expected_sha,
                "CONTRACT-DIGEST mismatch for {rel_path}"
            );
            entry_lines.push_str(expected_sha);
            entry_lines.push_str("  ");
            entry_lines.push_str(rel_path);
            entry_lines.push('\n');
        }
        let computed_bundle = sha256_hex(entry_lines.as_bytes());
        assert_eq!(
            Some(computed_bundle),
            declared_bundle_sha,
            "CONTRACT-DIGEST BUNDLE-SHA256 mismatch"
        );
    }
}
