/*
 * Copyright agent-otel-bridge contributors
 * SPDX-License-Identifier: Apache-2.0
 */

//! Normative reference implementation of the `fact-export-v1` table schema validator
//! and snapshot replay state machine (`C1b`).
//!
//! This module implements:
//! 1. Structural and Parquet logical-type mapping validation for the six normative
//!    `fact-export-v1` table schemas (`sessions`, `invocations`, `tool_calls`, `usage`,
//!    `outcomes`, `coverage`) and canonical schema bundle digest computation.
//! 2. Row-level validation enforcing zero-content (`DE-004`), customer-held key
//!    pseudonymization (`F13`), session-qualified `I01` invocation identity (`F9`),
//!    and bi-directional explicit missingness invariants (`F14`).
//! 3. The atomic publication, non-append-only replacement (`source_generation`,
//!    `session_keys`, `full_stream`, and `EN5c` `purge_delete`), idempotent import,
//!    and cross-table coverage denominator reconciliation state machine specified in
//!    `docs/contracts/fact-export-v1/SNAPSHOT.md`.

use crate::link_id::hex_encode;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Normative contract identifier for `fact-export-v1`.
pub const FACT_EXPORT_CONTRACT_ID: &str = "fact-export-v1";

/// Normative contract version for `fact-export-v1`.
pub const FACT_EXPORT_CONTRACT_VERSION: u64 = 1;

/// Canonical ordered list of the six `fact-export-v1` tables.
pub const CANONICAL_TABLES: [&str; 6] = [
    "sessions",
    "invocations",
    "tool_calls",
    "usage",
    "outcomes",
    "coverage",
];

/// Canonical ordered list of the seven `fact-export-v1` replay vector files.
pub const EXPECTED_VECTOR_FILES: [&str; 7] = [
    "clean_sequence.json",
    "outcome_replacement.json",
    "stale_out_of_order.json",
    "duplicate_idempotent.json",
    "partial_publication.json",
    "schema_mismatch.json",
    "coverage_missingness.json",
];

/// Forbidden substrings that violate the `DE-004` zero-content guarantee
/// (checked case-insensitively on every string field in every table row).
const FORBIDDEN_SUBSTRINGS_LOWER: [&str; 19] = [
    "g0a_",
    "_sentinel",
    "sk-",
    "ghp_",
    "gho_",
    "github_pat_",
    "akia",
    "aiza",
    "xoxb-",
    "xoxp-",
    "://",
    "http:",
    "https:",
    "ssh:",
    "git@",
    "\\",
    "@",
    "/users/",
    "/home/",
];

/// Deterministic outcome of evaluating a single snapshot publication step
/// in the `fact-export-v1` replay state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepStatus {
    /// Snapshot was valid, contiguous, and atomically applied to active state.
    #[serde(rename = "APPLIED")]
    Applied,
    /// Snapshot sequence and `manifest_digest` matched an already-committed sequence (idempotent no-op).
    #[serde(rename = "DUPLICATE_NOOP")]
    DuplicateNoop,
    /// Staged files without a committed `MANIFEST.json` are invisible to readers/importers.
    #[serde(rename = "IGNORED_UNCOMMITTED_NO_MANIFEST")]
    IgnoredUncommittedNoManifest,
    /// `tenant_placeholder`, `producer.producer_id`, or `stream` did not match the authenticated channel (`EN3a`).
    #[serde(rename = "REJECTED_TENANT_OR_PRODUCER_MISMATCH")]
    RejectedTenantOrProducerMismatch,
    /// `MANIFEST.json` failed structural or `manifest_digest` verification.
    #[serde(rename = "REJECTED_INVALID_MANIFEST")]
    RejectedInvalidManifest,
    /// `manifest.schema_digest` did not match the importer's pinned schema bundle digest (`EN3c`).
    #[serde(rename = "REJECTED_SCHEMA_MISMATCH")]
    RejectedSchemaMismatch,
    /// Same `snapshot_sequence` as `high_water_sequence` but with a conflicting `manifest_digest`.
    #[serde(rename = "REJECTED_CONFLICTING_DUPLICATE_SEQUENCE")]
    RejectedConflictingDuplicateSequence,
    /// `snapshot_sequence < high_water_sequence` (and not an exact committed duplicate).
    #[serde(rename = "REJECTED_STALE_SEQUENCE")]
    RejectedStaleSequence,
    /// `snapshot_sequence > high_water_sequence + 1` (gap in contiguous sequence).
    #[serde(rename = "REJECTED_OUT_OF_ORDER_SEQUENCE_GAP")]
    RejectedOutOfOrderSequenceGap,
    /// Missing table file, or `row_count`, `byte_length`, or `sha256` mismatch against `MANIFEST.json`.
    #[serde(rename = "REJECTED_FILE_INTEGRITY")]
    RejectedFileIntegrity,
    /// A row violated its table JSON Schema, `DE-004` zero-content rules, `F13` HMAC rules, or `F14` invariants.
    #[serde(rename = "REJECTED_ROW_SCHEMA_VIOLATION")]
    RejectedRowSchemaViolation,
    /// Snapshot attempted to regress `source_generation` or resurrect a purged generation/session (`EN5c`).
    #[serde(rename = "REJECTED_STALE_GENERATION_OR_TOMBSTONE")]
    RejectedStaleGenerationOrTombstone,
    /// Foreign-key integrity, `F9` invocation uniqueness, or `coverage` denominator reconciliation failed.
    #[serde(rename = "REJECTED_REFERENTIAL_OR_COVERAGE_MISMATCH")]
    RejectedReferentialOrCoverageMismatch,
}

impl StepStatus {
    /// Returns the normative wire string for this status.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "APPLIED",
            Self::DuplicateNoop => "DUPLICATE_NOOP",
            Self::IgnoredUncommittedNoManifest => "IGNORED_UNCOMMITTED_NO_MANIFEST",
            Self::RejectedTenantOrProducerMismatch => "REJECTED_TENANT_OR_PRODUCER_MISMATCH",
            Self::RejectedInvalidManifest => "REJECTED_INVALID_MANIFEST",
            Self::RejectedSchemaMismatch => "REJECTED_SCHEMA_MISMATCH",
            Self::RejectedConflictingDuplicateSequence => "REJECTED_CONFLICTING_DUPLICATE_SEQUENCE",
            Self::RejectedStaleSequence => "REJECTED_STALE_SEQUENCE",
            Self::RejectedOutOfOrderSequenceGap => "REJECTED_OUT_OF_ORDER_SEQUENCE_GAP",
            Self::RejectedFileIntegrity => "REJECTED_FILE_INTEGRITY",
            Self::RejectedRowSchemaViolation => "REJECTED_ROW_SCHEMA_VIOLATION",
            Self::RejectedStaleGenerationOrTombstone => "REJECTED_STALE_GENERATION_OR_TOMBSTONE",
            Self::RejectedReferentialOrCoverageMismatch => {
                "REJECTED_REFERENTIAL_OR_COVERAGE_MISMATCH"
            }
        }
    }
}

/// Returns the primary key column name for a canonical `fact-export-v1` table.
pub fn primary_key_for_table(table: &str) -> Option<&'static str> {
    match table {
        "sessions" => Some("session_key"),
        "invocations" => Some("invocation_key"),
        "tool_calls" => Some("tool_call_key"),
        "usage" => Some("usage_key"),
        "outcomes" => Some("outcome_key"),
        "coverage" => Some("coverage_key"),
        _ => None,
    }
}

/// Computes the 64-character lowercase hex SHA-256 digest of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_encode(&hasher.finalize())
}

/// Serializes a [`serde_json::Value`] into canonical compact sorted-key UTF-8 JSON.
///
/// Because `serde_json` uses `BTreeMap` by default when `preserve_order` is not enabled,
/// object keys are serialized in lexicographical UTF-8 code-point order with compact
/// `,` and `:` separators, matching Python's `json.dumps(v, sort_keys=True, separators=(",", ":"))`.
pub fn canonical_json_string(value: &Value) -> String {
    serde_json::to_string(value).expect("serde_json::Value serialization cannot fail")
}

/// Serializes a slice of canonical JSON row values into canonical UTF-8 JSONL bytes.
pub fn canonical_table_jsonl_bytes(rows: &[Value]) -> Vec<u8> {
    if rows.is_empty() {
        return Vec::new();
    }
    let mut out = String::new();
    for row in rows {
        out.push_str(&canonical_json_string(row));
        out.push('\n');
    }
    out.into_bytes()
}

/// Computes the normative `manifest_digest` for a manifest JSON object by removing
/// `manifest_digest`, serializing as canonical JSON, and hashing with SHA-256.
pub fn compute_manifest_digest(manifest: &Map<String, Value>) -> String {
    let mut cloned = manifest.clone();
    cloned.remove("manifest_digest");
    let canonical = canonical_json_string(&Value::Object(cloned));
    sha256_hex(canonical.as_bytes())
}

/// Validates all 6 table schema files in `base_dir/tables/*.schema.json` and returns
/// `(schemas_by_table, schema_digest)`.
pub fn verify_and_load_schemas(
    base_dir: &Path,
) -> Result<(BTreeMap<String, Value>, String), String> {
    let tables_dir = base_dir.join("tables");
    let mut schemas = BTreeMap::new();
    let mut digest_lines = String::new();

    for table in CANONICAL_TABLES {
        let schema_path = tables_dir.join(format!("{table}.schema.json"));
        let raw = std::fs::read_to_string(&schema_path)
            .map_err(|e| format!("failed to read {}: {e}", schema_path.display()))?;
        let schema: Value = serde_json::from_str(&raw)
            .map_err(|e| format!("invalid JSON in {}: {e}", schema_path.display()))?;

        validate_table_schema_definition(table, &schema)?;

        let table_sha = sha256_hex(canonical_json_string(&schema).as_bytes());
        digest_lines.push_str(&table_sha);
        digest_lines.push_str("  tables/");
        digest_lines.push_str(table);
        digest_lines.push_str(".schema.json\n");

        schemas.insert(table.to_string(), schema);
    }

    let schema_digest = sha256_hex(digest_lines.as_bytes());
    Ok((schemas, schema_digest))
}

/// Validates the structure, `x_parquet` mapping, and `x_justification` metadata of a table schema.
pub fn validate_table_schema_definition(table: &str, schema: &Value) -> Result<(), String> {
    let obj = schema
        .as_object()
        .ok_or_else(|| format!("{table}: schema root is not an object"))?;

    if obj.get("x_contract").and_then(Value::as_str) != Some(FACT_EXPORT_CONTRACT_ID) {
        return Err(format!("{table}: x_contract != {FACT_EXPORT_CONTRACT_ID}"));
    }
    if obj.get("x_version").and_then(Value::as_u64) != Some(FACT_EXPORT_CONTRACT_VERSION) {
        return Err(format!(
            "{table}: x_version != {FACT_EXPORT_CONTRACT_VERSION}"
        ));
    }
    if obj.get("x_table").and_then(Value::as_str) != Some(table) {
        return Err(format!("{table}: x_table mismatch"));
    }
    if obj.get("additionalProperties").and_then(Value::as_bool) != Some(false) {
        return Err(format!("{table}: additionalProperties must be false"));
    }
    let expected_pk =
        primary_key_for_table(table).ok_or_else(|| format!("unknown table {table}"))?;
    if obj.get("x_primary_key").and_then(Value::as_str) != Some(expected_pk) {
        return Err(format!("{table}: x_primary_key != {expected_pk}"));
    }

    let props = obj
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{table}: missing properties object"))?;
    let required = obj
        .get("required")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{table}: missing required array"))?;

    let prop_keys: BTreeSet<&str> = props.keys().map(String::as_str).collect();
    let req_keys: BTreeSet<&str> = required.iter().filter_map(Value::as_str).collect();
    if prop_keys != req_keys || props.len() != required.len() {
        return Err(format!(
            "{table}: required list must match all properties exactly"
        ));
    }

    let mut seen_field_ids = BTreeSet::new();
    for (col_name, col_spec_val) in props {
        let col_spec = col_spec_val
            .as_object()
            .ok_or_else(|| format!("{table}.{col_name}: spec is not an object"))?;

        let just = col_spec
            .get("x_justification")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("{table}.{col_name}: missing x_justification"))?;
        for req_key in [
            "why_present",
            "consumers",
            "zero_content",
            "pseudonymization",
            "missingness_semantics",
        ] {
            match just.get(req_key) {
                Some(Value::String(s)) if !s.trim().is_empty() => {}
                Some(Value::Array(arr)) if !arr.is_empty() => {}
                _ => {
                    return Err(format!(
                        "{table}.{col_name}: empty or missing x_justification.{req_key}"
                    ))
                }
            }
        }

        let pq = col_spec
            .get("x_parquet")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("{table}.{col_name}: missing x_parquet"))?;
        let fid = pq
            .get("field_id")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("{table}.{col_name}: missing field_id"))?;
        if fid == 0 || fid > props.len() as u64 || !seen_field_ids.insert(fid) {
            return Err(format!(
                "{table}.{col_name}: invalid or duplicate field_id {fid}"
            ));
        }

        let is_nullable = match col_spec.get("type") {
            Some(Value::Array(types)) => types.iter().any(|t| t.as_str() == Some("null")),
            _ => false,
        };
        let expected_rep = if is_nullable { "OPTIONAL" } else { "REQUIRED" };
        if pq.get("repetition").and_then(Value::as_str) != Some(expected_rep) {
            return Err(format!(
                "{table}.{col_name}: Parquet repetition does not match nullability"
            ));
        }
    }

    Ok(())
}

/// Checks whether a string value satisfies the `DE-004` zero-content guard.
pub fn check_zero_content_string(val: &str) -> bool {
    let lower = val.to_ascii_lowercase();
    for forbidden in FORBIDDEN_SUBSTRINGS_LOWER {
        if lower.contains(forbidden) {
            return false;
        }
    }
    if lower.contains("c:/") {
        return false;
    }
    true
}

#[inline]
fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[inline]
fn is_link_id_or_unlinked(s: &str) -> bool {
    s == "unlinked" || is_hex64(s)
}

#[inline]
fn is_printable_ascii_slug(s: &str, max_len: usize) -> bool {
    !s.is_empty() && s.len() <= max_len && s.bytes().all(|b| (0x21..=0x7E).contains(&b))
}

#[inline]
fn is_key_id_slug(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() >= 2
        && bytes.len() <= 64
        && matches!(bytes[0], b'a'..=b'z' | b'0'..=b'9')
        && bytes[1..]
            .iter()
            .all(|&b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
}

#[inline]
fn is_tenant_placeholder(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("tn_") else {
        return false;
    };
    let bytes = rest.as_bytes();
    bytes.len() >= 2
        && bytes.len() <= 61
        && matches!(bytes[0], b'a'..=b'z' | b'0'..=b'9')
        && bytes[1..]
            .iter()
            .all(|&b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'))
}

#[inline]
fn is_stream_slug(s: &str) -> bool {
    is_key_id_slug(s)
}

#[inline]
fn is_semver_token(s: &str) -> bool {
    let (core, suffix) = match s.split_once('-') {
        Some((c, suf)) => (c, Some(suf)),
        None => (s, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    if let Some(suf) = suffix {
        if suf.is_empty()
            || !suf
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        {
            return false;
        }
    }
    true
}

#[inline]
fn is_harness_version_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-'))
}

#[inline]
fn is_model_token(s: &str, max_len: usize) -> bool {
    !s.is_empty()
        && s.len() <= max_len
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'/' | b'-'))
}

#[inline]
fn is_tool_name_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
}

/// Parses a fixed 6-decimal USD string (`^[0-9]+\.[0-9]{6}$`) into integer micro-dollars.
pub fn parse_usd_micros(s: &str) -> Option<u64> {
    let (whole_str, frac_str) = s.split_once('.')?;
    if whole_str.is_empty()
        || frac_str.len() != 6
        || !whole_str.bytes().all(|b| b.is_ascii_digit())
        || !frac_str.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let whole: u64 = whole_str.parse().ok()?;
    let frac: u64 = frac_str.parse().ok()?;
    whole.checked_mul(1_000_000)?.checked_add(frac)
}

fn matches_schema_pattern(pattern: &str, val: &str) -> bool {
    match pattern {
        "^[0-9a-f]{64}$" => is_hex64(val),
        "^([0-9a-f]{64}|unlinked)$" => is_link_id_or_unlinked(val),
        "^[\\x21-\\x7E]{1,128}$" => is_printable_ascii_slug(val, 128),
        "^[a-z0-9][a-z0-9._-]{1,63}$" => is_key_id_slug(val),
        "^[A-Za-z0-9._+-]{1,64}$" => is_harness_version_token(val),
        "^[A-Za-z0-9._:/-]{1,128}$" => is_model_token(val, 128),
        "^[A-Za-z0-9._:/-]{1,96}$" => is_model_token(val, 96),
        "^[A-Za-z0-9_.:-]{1,64}$" => is_tool_name_token(val),
        "^[0-9]+\\.[0-9]{6}$" => parse_usd_micros(val).is_some(),
        _ => false,
    }
}

/// Validates a single canonical JSON row against its table schema, `DE-004` zero-content guard,
/// `F13` pseudonymization rules, and table-specific `x_invariants`.
pub fn validate_row_against_schema(
    table: &str,
    row_val: &Value,
    schema_val: &Value,
    expected_key_id: &str,
) -> bool {
    let Some(row) = row_val.as_object() else {
        return false;
    };
    let Some(props) = schema_val.get("properties").and_then(Value::as_object) else {
        return false;
    };
    if row.len() != props.len() || row.keys().any(|k| !props.contains_key(k)) {
        return false;
    }

    for (col_name, col_spec_val) in props {
        let Some(col_spec) = col_spec_val.as_object() else {
            return false;
        };
        let Some(val) = row.get(col_name) else {
            return false;
        };
        if let Some(s) = val.as_str() {
            if !check_zero_content_string(s) {
                return false;
            }
        }

        let allowed_types: Vec<&str> = match col_spec.get("type") {
            Some(Value::String(t)) => vec![t.as_str()],
            Some(Value::Array(arr)) => arr.iter().filter_map(Value::as_str).collect(),
            _ => return false,
        };

        if val.is_null() {
            if !allowed_types.contains(&"null") {
                return false;
            }
            continue;
        }

        if allowed_types.contains(&"string") && val.is_string() {
            let s = val.as_str().unwrap();
            if let Some(Value::Array(enums)) = col_spec.get("enum") {
                if !enums.iter().any(|e| e.as_str() == Some(s)) {
                    return false;
                }
            }
            if let Some(min_len) = col_spec.get("minLength").and_then(Value::as_u64) {
                if (s.len() as u64) < min_len {
                    return false;
                }
            }
            if let Some(max_len) = col_spec.get("maxLength").and_then(Value::as_u64) {
                if (s.len() as u64) > max_len {
                    return false;
                }
            }
            if let Some(pat) = col_spec.get("pattern").and_then(Value::as_str) {
                if !matches_schema_pattern(pat, s) {
                    return false;
                }
            }
        } else if allowed_types.contains(&"integer") && val.is_i64() || val.is_u64() {
            let Some(n) = val.as_i64() else {
                return false;
            };
            if let Some(min_val) = col_spec.get("minimum").and_then(Value::as_i64) {
                if n < min_val {
                    return false;
                }
            }
        } else if allowed_types.contains(&"boolean") && val.is_boolean() {
            // valid boolean
        } else if allowed_types.contains(&"array") && val.is_array() {
            let arr = val.as_array().unwrap();
            if let Some(min_items) = col_spec.get("minItems").and_then(Value::as_u64) {
                if (arr.len() as u64) < min_items {
                    return false;
                }
            }
            if col_spec.get("uniqueItems").and_then(Value::as_bool) == Some(true) {
                let unique: BTreeSet<&str> = arr.iter().filter_map(Value::as_str).collect();
                if unique.len() != arr.len() {
                    return false;
                }
            }
            let item_enums = col_spec
                .get("items")
                .and_then(|i| i.get("enum"))
                .and_then(Value::as_array);
            for item in arr {
                let Some(s) = item.as_str() else {
                    return false;
                };
                if !check_zero_content_string(s) {
                    return false;
                }
                if let Some(enums) = item_enums {
                    if !enums.iter().any(|e| e.as_str() == Some(s)) {
                        return false;
                    }
                }
            }
        } else {
            return false;
        }
    }

    if row.get("pseudonym_key_id").and_then(Value::as_str) != Some(expected_key_id) {
        return false;
    }

    match table {
        "sessions" => validate_sessions_invariants(row),
        "invocations" => validate_invocations_invariants(row),
        "tool_calls" => validate_tool_calls_invariants(row),
        "usage" => validate_usage_invariants(row),
        "outcomes" => validate_outcomes_invariants(row),
        "coverage" => validate_coverage_invariants(row),
        _ => false,
    }
}

fn validate_sessions_invariants(r: &Map<String, Value>) -> bool {
    let link_id = r["agent.session.link_id"].as_str().unwrap_or("");
    let unlinked_reason = r["unlinked_reason"].as_str();
    if (link_id == "unlinked") != unlinked_reason.is_some() {
        return false;
    }
    let origin = r["origin"].as_str().unwrap_or("");
    let harness = r["canonical_harness"].as_str().unwrap_or("");
    if link_id != "unlinked" {
        if r["link_epoch"].is_null() || origin != "harness" || harness == "unknown" {
            return false;
        }
    } else if unlinked_reason == Some("invalid_key_material") && !r["link_epoch"].is_null() {
        return false;
    }
    if origin != "harness"
        && (link_id != "unlinked" || unlinked_reason != Some("non_harness_origin"))
    {
        return false;
    }
    if (r["harness_version_status"].as_str() == Some("observed")) != !r["harness_version"].is_null()
    {
        return false;
    }
    let s_us = r["started_at_us"].as_i64();
    let e_us = r["ended_at_us"].as_i64();
    match r["timestamp_status"].as_str() {
        Some("complete") => {
            let (Some(s), Some(e)) = (s_us, e_us) else {
                return false;
            };
            if e < s {
                return false;
            }
        }
        Some("start_only") => {
            if s_us.is_none() || e_us.is_some() {
                return false;
            }
        }
        Some("not_observable") => {
            if s_us.is_some() || e_us.is_some() {
                return false;
            }
        }
        _ => return false,
    }

    let is_root = r["is_root"].as_bool().unwrap_or(false);
    let lin_rel = r["lineage_relation"].as_str().unwrap_or("");
    let lin_stat = r["lineage_status"].as_str().unwrap_or("");
    let parent_sk = r["parent_session_key"].as_str();
    let self_sk = r["session_key"].as_str().unwrap_or("");

    if is_root && (lin_rel != "root" || lin_stat != "root" || parent_sk.is_some()) {
        return false;
    }
    if lin_stat == "evidenced" {
        if is_root || parent_sk.is_none() || parent_sk == Some(self_sk) || lin_rel == "root" {
            return false;
        }
    } else if parent_sk.is_some() {
        return false;
    }
    true
}

fn validate_invocations_invariants(r: &Map<String, Value>) -> bool {
    match r["invocation_grain"].as_str() {
        Some("call") => {
            if r["message_key"].is_null() || !r["call_id_missing_reason"].is_null() {
                return false;
            }
        }
        Some("usage_only") => {
            if !r["message_key"].is_null() || r["call_id_missing_reason"].is_null() {
                return false;
            }
        }
        _ => return false,
    }
    if (r["occurred_at_status"].as_str() == Some("observed")) != !r["occurred_at_us"].is_null() {
        return false;
    }
    match r["model_status"].as_str() {
        Some("observed") => {
            if r["model"].is_null() || !r["model_unknown_reason"].is_null() {
                return false;
            }
        }
        Some("unknown") => {
            if !r["model"].is_null() || r["model_unknown_reason"].is_null() {
                return false;
            }
        }
        _ => return false,
    }
    true
}

fn validate_tool_calls_invariants(r: &Map<String, Value>) -> bool {
    if (r["invocation_link_status"].as_str() == Some("linked")) != !r["invocation_key"].is_null() {
        return false;
    }
    if (r["call_id_status"].as_str() == Some("observed")) != !r["call_id_hmac"].is_null() {
        return false;
    }
    if (r["fingerprint_status"].as_str() == Some("observed"))
        != !r["input_fingerprint_hmac"].is_null()
    {
        return false;
    }
    match r["outcome_status"].as_str() {
        Some("pending" | "unknown") => {
            if r["outcome_unknown_reason"].is_null() {
                return false;
            }
        }
        Some("succeeded" | "failed" | "cancelled") => {
            if !r["outcome_unknown_reason"].is_null() {
                return false;
            }
        }
        _ => return false,
    }
    if (r["occurred_at_status"].as_str() == Some("observed")) != !r["occurred_at_us"].is_null() {
        return false;
    }
    if (r["duration_status"].as_str() == Some("observed")) != !r["duration_us"].is_null() {
        return false;
    }
    true
}

fn validate_usage_invariants(r: &Map<String, Value>) -> bool {
    if (r["invocation_link_status"].as_str() == Some("linked")) != !r["invocation_key"].is_null() {
        return false;
    }
    if (r["occurred_at_status"].as_str() == Some("observed")) != !r["occurred_at_us"].is_null() {
        return false;
    }
    match r["model_status"].as_str() {
        Some("observed" | "inferred_from_session") => {
            if r["model"].is_null() || !r["model_unknown_reason"].is_null() {
                return false;
            }
        }
        Some("unknown") => {
            if !r["model"].is_null() || r["model_unknown_reason"].is_null() {
                return false;
            }
        }
        _ => return false,
    }
    let cs = r["counter_semantics"].as_str().unwrap_or("");
    let needs_pred = matches!(cs, "cumulative_delta" | "cumulative_monotonic_skip");
    if needs_pred != !r["predecessor_usage_key"].is_null() {
        return false;
    }
    let in_tok = r["input_uncached_tokens"].as_u64().unwrap_or(0);
    let out_tok = r["output_tokens"].as_u64().unwrap_or(0);
    let cr_tok = r["cache_read_tokens"].as_u64().unwrap_or(0);
    let cw_tok = r["cache_write_tokens"].as_u64().unwrap_or(0);
    let tot_tok = r["total_tokens"].as_u64().unwrap_or(u64::MAX);
    if tot_tok != in_tok + out_tok + cr_tok + cw_tok {
        return false;
    }
    if (r["reasoning_status"].as_str() == Some("observed")) != !r["reasoning_tokens"].is_null() {
        return false;
    }
    match r["cost_provenance"].as_str() {
        Some("harness_observed_exact" | "provider_receipt_exact") => {
            let Some(usd_str) = r["observed_usd"].as_str() else {
                return false;
            };
            let Some(micros) = r["observed_usd_micros"].as_u64() else {
                return false;
            };
            if !r["cost_unknown_reason"].is_null() {
                return false;
            }
            if parse_usd_micros(usd_str) != Some(micros) {
                return false;
            }
        }
        Some("unknown") => {
            if !r["observed_usd"].is_null()
                || !r["observed_usd_micros"].is_null()
                || r["cost_unknown_reason"].is_null()
            {
                return false;
            }
        }
        _ => return false,
    }
    true
}

fn validate_outcomes_invariants(r: &Map<String, Value>) -> bool {
    if (r["remote_status"].as_str() == Some("observed")) != !r["repo_remote_hmac"].is_null() {
        return false;
    }
    if (r["branch_status"].as_str() == Some("named_branch")) != !r["branch_name_hmac"].is_null() {
        return false;
    }
    if (r["head_commit_status"].as_str() == Some("observed")) != !r["head_commit_hmac"].is_null() {
        return false;
    }
    if (r["base_commit_status"].as_str() == Some("observed")) != !r["base_commit_hmac"].is_null() {
        return false;
    }

    let all_unobserved = r["remote_status"].as_str() == Some("not_observed")
        && r["branch_status"].as_str() == Some("not_observed")
        && r["head_commit_status"].as_str() == Some("not_observed")
        && r["base_commit_status"].as_str() == Some("not_observed");
    let all_full = r["remote_status"].as_str() == Some("observed")
        && r["branch_status"].as_str() == Some("named_branch")
        && r["head_commit_status"].as_str() == Some("observed")
        && r["base_commit_status"].as_str() == Some("observed");

    match r["evidence_status"].as_str() {
        Some("missing") => {
            if r["missing_reason"].is_null() || !all_unobserved {
                return false;
            }
        }
        Some("observed_full") => {
            if !r["missing_reason"].is_null() || !all_full {
                return false;
            }
        }
        Some("observed_partial") => {
            if !r["missing_reason"].is_null() || all_unobserved || all_full {
                return false;
            }
        }
        _ => return false,
    }
    if (r["observed_at_status"].as_str() == Some("observed")) != !r["observed_at_us"].is_null() {
        return false;
    }
    true
}

fn validate_coverage_invariants(r: &Map<String, Value>) -> bool {
    let acc = r["accepted_count"].as_u64().unwrap_or(0);
    let rej = r["rejected_count"].as_u64().unwrap_or(0);
    let unproj = r["recognized_unprojected_count"].as_u64().unwrap_or(0);
    let enc = r["encountered_count"].as_u64().unwrap_or(u64::MAX);
    if enc != acc + rej + unproj {
        return false;
    }

    let exp_pop = r["expected_population_count"].as_u64().unwrap_or(u64::MAX);
    let obs = r["observable_count"].as_u64().unwrap_or(0);
    let unk = r["unknown_or_missing_count"].as_u64().unwrap_or(0);
    if exp_pop != obs + unk {
        return false;
    }

    let disc = r["discovery_status"].as_str().unwrap_or("");
    let proj = r["projection_status"].as_str().unwrap_or("");
    let cs = r["coverage_status"].as_str().unwrap_or("");

    let is_complete = exp_pop > 0 && obs == exp_pop && unk == 0 && rej == 0 && unproj == 0;
    let is_partial = exp_pop > 0 && obs > 0 && (unk > 0 || rej > 0 || unproj > 0);
    let is_empty = exp_pop == 0
        && matches!(disc, "discovered" | "empty_root")
        && proj != "connector_unimplemented";
    let is_unavail = obs == 0
        && (exp_pop > 0
            || proj == "connector_unimplemented"
            || matches!(
                disc,
                "missing_root" | "permission_denied" | "not_configured"
            ));

    match cs {
        "complete" if !is_complete => return false,
        "partial" if !is_partial => return false,
        "empty" if !is_empty => return false,
        "unavailable" if !is_unavail => return false,
        "complete" | "partial" | "empty" | "unavailable" => {}
        _ => return false,
    }

    let Some(gaps) = r["gap_reasons"].as_array() else {
        return false;
    };
    if matches!(cs, "complete" | "empty") {
        if gaps.len() != 1 || gaps[0].as_str() != Some("none") {
            return false;
        }
    } else if gaps.is_empty() || gaps.iter().any(|g| g.as_str() == Some("none")) {
        return false;
    }
    true
}

/// Validates the structure and `manifest_digest` of a `MANIFEST.json` object.
pub fn validate_manifest_structure(manifest_val: &Value) -> bool {
    let Some(m) = manifest_val.as_object() else {
        return false;
    };
    let required = [
        "contract_id",
        "contract_version",
        "schema_digest",
        "producer",
        "tenant_placeholder",
        "stream",
        "snapshot_sequence",
        "published_at_us",
        "replacement_scope",
        "files",
        "manifest_digest",
    ];
    if m.len() != required.len() || required.iter().any(|&k| !m.contains_key(k)) {
        return false;
    }
    if m["contract_id"].as_str() != Some(FACT_EXPORT_CONTRACT_ID)
        || m["contract_version"].as_u64() != Some(FACT_EXPORT_CONTRACT_VERSION)
    {
        return false;
    }
    if !m["schema_digest"].as_str().is_some_and(is_hex64) {
        return false;
    }
    if !m["tenant_placeholder"]
        .as_str()
        .is_some_and(is_tenant_placeholder)
    {
        return false;
    }
    if !m["stream"].as_str().is_some_and(is_stream_slug) {
        return false;
    }
    if m["snapshot_sequence"].as_u64().unwrap_or(0) < 1
        || m["published_at_us"].as_u64().unwrap_or(0) < 1
    {
        return false;
    }

    let Some(prod) = m["producer"].as_object() else {
        return false;
    };
    if prod.len() != 4
        || !prod
            .get("producer_id")
            .and_then(Value::as_str)
            .is_some_and(is_hex64)
        || !matches!(
            prod.get("producer_kind").and_then(Value::as_str),
            Some("dendro_cli" | "agent_otel_daemon" | "dendro_station_exporter")
        )
        || !prod
            .get("producer_version")
            .and_then(Value::as_str)
            .is_some_and(is_semver_token)
        || !prod
            .get("pseudonym_key_id")
            .and_then(Value::as_str)
            .is_some_and(is_key_id_slug)
    {
        return false;
    }

    let Some(scope) = m["replacement_scope"].as_object() else {
        return false;
    };
    if scope.len() != 7 {
        return false;
    }
    let Some(mode) = scope.get("scope_mode").and_then(Value::as_str) else {
        return false;
    };
    let Some(op) = scope.get("operation").and_then(Value::as_str) else {
        return false;
    };
    if !matches!(mode, "source_generation" | "session_keys" | "full_stream")
        || !matches!(op, "upsert_replace" | "purge_delete")
        || !matches!(
            scope.get("reason").and_then(Value::as_str),
            Some(
                "initial_export"
                    | "in_generation_outcome_or_attribution_update"
                    | "source_generation_advance"
                    | "retention_or_redaction_purge"
                    | "full_stream_rebaseline"
            )
        )
    {
        return false;
    }
    let Some(sup_gens) = scope
        .get("supersedes_generations")
        .and_then(Value::as_array)
    else {
        return false;
    };
    if sup_gens.iter().any(|g| g.as_u64().unwrap_or(0) < 1) {
        return false;
    }
    let Some(sess_keys) = scope.get("session_keys").and_then(Value::as_array) else {
        return false;
    };
    if sess_keys
        .iter()
        .any(|sk| !sk.as_str().is_some_and(is_hex64))
    {
        return false;
    }
    if matches!(mode, "source_generation" | "session_keys") {
        if !scope
            .get("source_id")
            .and_then(Value::as_str)
            .is_some_and(is_hex64)
            || scope
                .get("source_generation")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                < 1
        {
            return false;
        }
    } else if !scope.get("source_id").is_some_and(Value::is_null)
        || !scope.get("source_generation").is_some_and(Value::is_null)
    {
        return false;
    }
    if (mode == "session_keys") != (!sess_keys.is_empty()) {
        return false;
    }

    let Some(files) = m["files"].as_array() else {
        return false;
    };
    if files.len() != CANONICAL_TABLES.len() {
        return false;
    }
    for (idx, &expected_table) in CANONICAL_TABLES.iter().enumerate() {
        let Some(f_obj) = files[idx].as_object() else {
            return false;
        };
        if f_obj.len() != 6
            || f_obj.get("table").and_then(Value::as_str) != Some(expected_table)
            || !matches!(
                f_obj.get("format").and_then(Value::as_str),
                Some("parquet" | "canonical_json_rows")
            )
            || f_obj.get("path").and_then(Value::as_str).is_none()
            || f_obj.get("row_count").and_then(Value::as_u64).is_none()
            || f_obj.get("byte_length").and_then(Value::as_u64).is_none()
            || !f_obj
                .get("sha256")
                .and_then(Value::as_str)
                .is_some_and(is_hex64)
        {
            return false;
        }
    }

    let Some(declared_digest) = m["manifest_digest"].as_str() else {
        return false;
    };
    declared_digest == compute_manifest_digest(m)
}

/// Deterministic implementation of the `SNAPSHOT.md` replay and import state machine.
#[derive(Debug, Clone)]
pub struct ReplayStateMachine {
    schemas: BTreeMap<String, Value>,
    expected_schema_digest: String,
    expected_tenant: String,
    expected_producer_id: String,
    expected_stream: String,
    /// Highest contiguous `snapshot_sequence` committed so far.
    pub high_water_sequence: u64,
    /// Map of committed `snapshot_sequence -> manifest_digest`.
    pub committed_manifests: BTreeMap<u64, String>,
    /// Map of `source_id -> active source_generation`.
    pub active_source_generations: BTreeMap<String, u64>,
    /// Map of `source_id -> highest purged source_generation`.
    pub purged_source_generations: BTreeMap<String, u64>,
    /// Map of `"{source_id}:{session_key}" -> tombstoned source_generation`.
    pub tombstoned_session_keys: BTreeMap<String, u64>,
    /// Active rows per table keyed by primary key.
    pub tables: BTreeMap<String, BTreeMap<String, Value>>,
}

impl ReplayStateMachine {
    /// Creates a new replay state machine bound to the expected schema digest, tenant, producer, and stream.
    pub fn new(
        schemas: BTreeMap<String, Value>,
        expected_schema_digest: String,
        expected_tenant: String,
        expected_producer_id: String,
        expected_stream: String,
    ) -> Self {
        let mut tables = BTreeMap::new();
        for t in CANONICAL_TABLES {
            tables.insert(t.to_string(), BTreeMap::new());
        }
        Self {
            schemas,
            expected_schema_digest,
            expected_tenant,
            expected_producer_id,
            expected_stream,
            high_water_sequence: 0,
            committed_manifests: BTreeMap::new(),
            active_source_generations: BTreeMap::new(),
            purged_source_generations: BTreeMap::new(),
            tombstoned_session_keys: BTreeMap::new(),
            tables,
        }
    }

    /// Returns a map of table name to active row count.
    pub fn active_row_counts(&self) -> BTreeMap<String, usize> {
        CANONICAL_TABLES
            .iter()
            .map(|&t| (t.to_string(), self.tables[t].len()))
            .collect()
    }

    /// Exports all active tables with rows sorted deterministically by primary key.
    pub fn export_sorted_tables(&self) -> BTreeMap<String, Vec<Value>> {
        CANONICAL_TABLES
            .iter()
            .map(|&t| {
                let rows: Vec<Value> = self.tables[t].values().cloned().collect();
                (t.to_string(), rows)
            })
            .collect()
    }

    /// Evaluates and applies a single replay step object.
    pub fn apply_step(&mut self, step: &Value) -> StepStatus {
        let Some(step_obj) = step.as_object() else {
            return StepStatus::RejectedInvalidManifest;
        };
        let Some(manifest_val) = step_obj.get("committed_manifest") else {
            return StepStatus::IgnoredUncommittedNoManifest;
        };
        // Step 1: Atomic visibility check
        if manifest_val.is_null() {
            return StepStatus::IgnoredUncommittedNoManifest;
        }
        let Some(manifest) = manifest_val.as_object() else {
            return StepStatus::RejectedInvalidManifest;
        };

        // Step 2: Producer & tenant authorization (EN3a)
        let tenant = manifest.get("tenant_placeholder").and_then(Value::as_str);
        let prod_id = manifest
            .get("producer")
            .and_then(|p| p.get("producer_id"))
            .and_then(Value::as_str);
        let stream = manifest.get("stream").and_then(Value::as_str);
        if tenant != Some(self.expected_tenant.as_str())
            || prod_id != Some(self.expected_producer_id.as_str())
            || stream != Some(self.expected_stream.as_str())
        {
            return StepStatus::RejectedTenantOrProducerMismatch;
        }

        // Step 3: Manifest structure & digest verification
        if !validate_manifest_structure(manifest_val) {
            return StepStatus::RejectedInvalidManifest;
        }

        // Step 4: Schema digest verification (EN3c)
        if manifest["schema_digest"].as_str() != Some(self.expected_schema_digest.as_str()) {
            return StepStatus::RejectedSchemaMismatch;
        }

        // Step 5: Sequence idempotency, conflict, stale & out-of-order checks
        let seq = manifest["snapshot_sequence"].as_u64().unwrap();
        let m_digest = manifest["manifest_digest"].as_str().unwrap();
        if let Some(existing_digest) = self.committed_manifests.get(&seq) {
            if existing_digest == m_digest {
                return StepStatus::DuplicateNoop;
            }
            if seq < self.high_water_sequence {
                return StepStatus::RejectedStaleSequence;
            }
            return StepStatus::RejectedConflictingDuplicateSequence;
        }
        if seq < self.high_water_sequence {
            return StepStatus::RejectedStaleSequence;
        }
        if seq > self.high_water_sequence + 1 {
            return StepStatus::RejectedOutOfOrderSequenceGap;
        }

        // Step 6: File presence, byte_length, sha256, and row_count integrity
        let Some(staged) = step_obj.get("staged_tables").and_then(Value::as_object) else {
            return StepStatus::RejectedFileIntegrity;
        };
        let files = manifest["files"].as_array().unwrap();
        for f_val in files {
            let f_desc = f_val.as_object().unwrap();
            let t_name = f_desc["table"].as_str().unwrap();
            let Some(rows) = staged.get(t_name).and_then(Value::as_array) else {
                return StepStatus::RejectedFileIntegrity;
            };
            if rows.len() as u64 != f_desc["row_count"].as_u64().unwrap() {
                return StepStatus::RejectedFileIntegrity;
            }
            let payload_bytes = canonical_table_jsonl_bytes(rows);
            if payload_bytes.len() as u64 != f_desc["byte_length"].as_u64().unwrap() {
                return StepStatus::RejectedFileIntegrity;
            }
            if sha256_hex(&payload_bytes) != f_desc["sha256"].as_str().unwrap() {
                return StepStatus::RejectedFileIntegrity;
            }
        }

        // Step 7: Row schema, zero-content (DE-004), pseudonymization (F13) & missingness (F14)
        let key_id = manifest["producer"]["pseudonym_key_id"].as_str().unwrap();
        for t_name in CANONICAL_TABLES {
            let pk_col = primary_key_for_table(t_name).unwrap();
            let rows = staged[t_name].as_array().unwrap();
            let mut seen_pks = BTreeSet::new();
            for row in rows {
                if !validate_row_against_schema(t_name, row, &self.schemas[t_name], key_id) {
                    return StepStatus::RejectedRowSchemaViolation;
                }
                let pk_val = row[pk_col].as_str().unwrap();
                if !seen_pks.insert(pk_val.to_string()) {
                    return StepStatus::RejectedRowSchemaViolation;
                }
            }
        }

        // Step 8: Replacement scope, anti-resurrection & cross-table / coverage reconciliation
        let scope = manifest["replacement_scope"].as_object().unwrap();
        let mode = scope["scope_mode"].as_str().unwrap();
        let op = scope["operation"].as_str().unwrap();
        let src_id_opt = scope["source_id"].as_str();
        let src_gen_opt = scope["source_generation"].as_u64();
        let sess_keys: BTreeSet<String> = scope["session_keys"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();

        if matches!(mode, "source_generation" | "session_keys") {
            let src_id = src_id_opt.unwrap();
            let src_gen = src_gen_opt.unwrap();
            let active_gen = self
                .active_source_generations
                .get(src_id)
                .copied()
                .unwrap_or(0);
            let purged_gen = self
                .purged_source_generations
                .get(src_id)
                .copied()
                .unwrap_or(0);
            if src_gen < active_gen || src_gen <= purged_gen {
                return StepStatus::RejectedStaleGenerationOrTombstone;
            }
            if op == "purge_delete" && mode == "source_generation" && src_gen <= active_gen {
                return StepStatus::RejectedStaleGenerationOrTombstone;
            }
        }

        let mut candidate_tables = self.tables.clone();
        let mut candidate_active_gens = self.active_source_generations.clone();
        let mut candidate_purged_gens = self.purged_source_generations.clone();
        let mut candidate_tombstones = self.tombstoned_session_keys.clone();

        match mode {
            "source_generation" => {
                let src_id = src_id_opt.unwrap();
                let src_gen = src_gen_opt.unwrap();
                if op == "purge_delete" {
                    if CANONICAL_TABLES
                        .iter()
                        .any(|&t| !staged[t].as_array().unwrap().is_empty())
                    {
                        return StepStatus::RejectedReferentialOrCoverageMismatch;
                    }
                    for t in CANONICAL_TABLES {
                        candidate_tables.get_mut(t).unwrap().retain(|_, r| {
                            r["source_id"].as_str() != Some(src_id)
                                || r["source_generation"].as_u64().unwrap_or(0) > src_gen
                        });
                    }
                    let prev_purged = candidate_purged_gens.get(src_id).copied().unwrap_or(0);
                    candidate_purged_gens.insert(src_id.to_string(), prev_purged.max(src_gen));
                    let prev_active = candidate_active_gens.get(src_id).copied().unwrap_or(0);
                    candidate_active_gens.insert(src_id.to_string(), prev_active.max(src_gen));
                } else {
                    for t in CANONICAL_TABLES {
                        let rows = staged[t].as_array().unwrap();
                        for r in rows {
                            if r["source_id"].as_str() != Some(src_id)
                                || r["source_generation"].as_u64() != Some(src_gen)
                            {
                                return StepStatus::RejectedReferentialOrCoverageMismatch;
                            }
                            if t != "coverage" {
                                let sk = r["session_key"].as_str().unwrap();
                                let tomb_key = format!("{src_id}:{sk}");
                                if src_gen
                                    <= candidate_tombstones.get(&tomb_key).copied().unwrap_or(0)
                                {
                                    return StepStatus::RejectedStaleGenerationOrTombstone;
                                }
                            }
                        }
                        let map = candidate_tables.get_mut(t).unwrap();
                        map.retain(|_, r| r["source_id"].as_str() != Some(src_id));
                        let pk_col = primary_key_for_table(t).unwrap();
                        for r in rows {
                            map.insert(r[pk_col].as_str().unwrap().to_string(), r.clone());
                        }
                    }
                    candidate_active_gens.insert(src_id.to_string(), src_gen);
                }
            }
            "session_keys" => {
                let src_id = src_id_opt.unwrap();
                let src_gen = src_gen_opt.unwrap();
                if op == "purge_delete" {
                    for t in ["sessions", "invocations", "tool_calls", "usage", "outcomes"] {
                        if !staged[t].as_array().unwrap().is_empty() {
                            return StepStatus::RejectedReferentialOrCoverageMismatch;
                        }
                        candidate_tables.get_mut(t).unwrap().retain(|_, r| {
                            !(r["source_id"].as_str() == Some(src_id)
                                && r["session_key"]
                                    .as_str()
                                    .is_some_and(|sk| sess_keys.contains(sk)))
                        });
                    }
                    for sk in &sess_keys {
                        let tomb_key = format!("{src_id}:{sk}");
                        let prev = candidate_tombstones.get(&tomb_key).copied().unwrap_or(0);
                        candidate_tombstones.insert(tomb_key, prev.max(src_gen));
                    }
                    for t in ["sessions", "invocations", "tool_calls", "usage", "outcomes"] {
                        for r in candidate_tables.get_mut(t).unwrap().values_mut() {
                            if r["source_id"].as_str() == Some(src_id) {
                                r.as_object_mut()
                                    .unwrap()
                                    .insert("source_generation".to_string(), Value::from(src_gen));
                            }
                        }
                    }
                    let cov_rows = staged["coverage"].as_array().unwrap();
                    for r in cov_rows {
                        if r["source_id"].as_str() != Some(src_id)
                            || r["source_generation"].as_u64() != Some(src_gen)
                        {
                            return StepStatus::RejectedReferentialOrCoverageMismatch;
                        }
                    }
                    let cov_map = candidate_tables.get_mut("coverage").unwrap();
                    cov_map.retain(|_, r| r["source_id"].as_str() != Some(src_id));
                    for r in cov_rows {
                        cov_map.insert(r["coverage_key"].as_str().unwrap().to_string(), r.clone());
                    }
                    candidate_active_gens.insert(src_id.to_string(), src_gen);
                } else {
                    for sk in &sess_keys {
                        let tomb_key = format!("{src_id}:{sk}");
                        if src_gen <= candidate_tombstones.get(&tomb_key).copied().unwrap_or(0) {
                            return StepStatus::RejectedStaleGenerationOrTombstone;
                        }
                    }
                    for t in ["sessions", "invocations", "tool_calls", "usage", "outcomes"] {
                        let rows = staged[t].as_array().unwrap();
                        for r in rows {
                            let sk = r["session_key"].as_str().unwrap_or("");
                            if r["source_id"].as_str() != Some(src_id)
                                || r["source_generation"].as_u64() != Some(src_gen)
                                || !sess_keys.contains(sk)
                            {
                                return StepStatus::RejectedReferentialOrCoverageMismatch;
                            }
                        }
                        let map = candidate_tables.get_mut(t).unwrap();
                        map.retain(|_, r| {
                            !(r["source_id"].as_str() == Some(src_id)
                                && r["session_key"]
                                    .as_str()
                                    .is_some_and(|sk| sess_keys.contains(sk)))
                        });
                        let pk_col = primary_key_for_table(t).unwrap();
                        for r in rows {
                            map.insert(r[pk_col].as_str().unwrap().to_string(), r.clone());
                        }
                    }
                    let cov_rows = staged["coverage"].as_array().unwrap();
                    let cov_map = candidate_tables.get_mut("coverage").unwrap();
                    for r in cov_rows {
                        if r["source_id"].as_str() != Some(src_id)
                            || r["source_generation"].as_u64() != Some(src_gen)
                        {
                            return StepStatus::RejectedReferentialOrCoverageMismatch;
                        }
                        cov_map.insert(r["coverage_key"].as_str().unwrap().to_string(), r.clone());
                    }
                    candidate_active_gens.insert(src_id.to_string(), src_gen);
                }
            }
            "full_stream" => {
                for t in CANONICAL_TABLES {
                    let map = candidate_tables.get_mut(t).unwrap();
                    map.clear();
                    let pk_col = primary_key_for_table(t).unwrap();
                    for r in staged[t].as_array().unwrap() {
                        map.insert(r[pk_col].as_str().unwrap().to_string(), r.clone());
                        let s_id = r["source_id"].as_str().unwrap();
                        let s_gen = r["source_generation"].as_u64().unwrap();
                        let prev = candidate_active_gens.get(s_id).copied().unwrap_or(0);
                        candidate_active_gens.insert(s_id.to_string(), prev.max(s_gen));
                    }
                }
            }
            _ => return StepStatus::RejectedInvalidManifest,
        }

        if !verify_cross_table_and_coverage(&candidate_tables) {
            return StepStatus::RejectedReferentialOrCoverageMismatch;
        }

        // Step 9: Commit state atomically
        self.tables = candidate_tables;
        self.active_source_generations = candidate_active_gens;
        self.purged_source_generations = candidate_purged_gens;
        self.tombstoned_session_keys = candidate_tombstones;
        self.high_water_sequence = seq;
        self.committed_manifests.insert(seq, m_digest.to_string());
        StepStatus::Applied
    }
}

/// Verifies foreign-key integrity, `F9` qualified invocation uniqueness, and `coverage`
/// denominator reconciliation across all active tables.
pub fn verify_cross_table_and_coverage(tables: &BTreeMap<String, BTreeMap<String, Value>>) -> bool {
    let sessions = &tables["sessions"];
    let invocations = &tables["invocations"];
    let tool_calls = &tables["tool_calls"];
    let usage = &tables["usage"];
    let outcomes = &tables["outcomes"];
    let coverage = &tables["coverage"];

    let mut seen_i01_tuples = BTreeSet::new();
    for inv in invocations.values() {
        let sk = inv["session_key"].as_str().unwrap_or("");
        let Some(parent) = sessions.get(sk) else {
            return false;
        };
        if inv["source_id"] != parent["source_id"]
            || inv["source_generation"] != parent["source_generation"]
            || inv["canonical_harness"] != parent["canonical_harness"]
        {
            return false;
        }
        let tuple = (
            sk.to_string(),
            inv["sequence_no"].as_u64().unwrap_or(0),
            inv["invocation_grain"].as_str().unwrap_or("").to_string(),
            inv["message_key"].as_str().unwrap_or("").to_string(),
        );
        if !seen_i01_tuples.insert(tuple) {
            return false;
        }
    }

    for tc in tool_calls.values() {
        let sk = tc["session_key"].as_str().unwrap_or("");
        let Some(parent) = sessions.get(sk) else {
            return false;
        };
        if tc["source_id"] != parent["source_id"]
            || tc["source_generation"] != parent["source_generation"]
            || tc["canonical_harness"] != parent["canonical_harness"]
        {
            return false;
        }
        if tc["invocation_link_status"].as_str() == Some("linked") {
            let ik = tc["invocation_key"].as_str().unwrap_or("");
            let Some(inv_row) = invocations.get(ik) else {
                return false;
            };
            if inv_row["session_key"].as_str() != Some(sk) {
                return false;
            }
        }
    }

    for usg in usage.values() {
        let sk = usg["session_key"].as_str().unwrap_or("");
        let Some(parent) = sessions.get(sk) else {
            return false;
        };
        if usg["source_id"] != parent["source_id"]
            || usg["source_generation"] != parent["source_generation"]
            || usg["canonical_harness"] != parent["canonical_harness"]
        {
            return false;
        }
        if usg["invocation_link_status"].as_str() == Some("linked") {
            let ik = usg["invocation_key"].as_str().unwrap_or("");
            let Some(inv_row) = invocations.get(ik) else {
                return false;
            };
            if inv_row["session_key"].as_str() != Some(sk) {
                return false;
            }
        }
    }

    for out in outcomes.values() {
        let sk = out["session_key"].as_str().unwrap_or("");
        let Some(parent) = sessions.get(sk) else {
            return false;
        };
        if out["source_id"] != parent["source_id"]
            || out["source_generation"] != parent["source_generation"]
            || out["canonical_harness"] != parent["canonical_harness"]
        {
            return false;
        }
    }

    let mut seen_cov_tuples = BTreeSet::new();
    for cov in coverage.values() {
        let cov_tuple = (
            cov["source_id"].as_str().unwrap_or("").to_string(),
            cov["source_generation"].as_u64().unwrap_or(0),
            cov["canonical_harness"].as_str().unwrap_or("").to_string(),
            cov["target_table"].as_str().unwrap_or("").to_string(),
            cov["coverage_dimension"].as_str().unwrap_or("").to_string(),
        );
        if !seen_cov_tuples.insert(cov_tuple) {
            return false;
        }

        let t_name = cov["target_table"].as_str().unwrap_or("");
        let Some(target_map) = tables.get(t_name) else {
            return false;
        };
        let target_rows: Vec<&Value> = target_map
            .values()
            .filter(|r| {
                r["source_id"] == cov["source_id"]
                    && r["source_generation"] == cov["source_generation"]
                    && r["canonical_harness"] == cov["canonical_harness"]
            })
            .collect();

        let total_target = target_rows.len() as u64;
        if cov["accepted_count"].as_u64() != Some(total_target) {
            return false;
        }

        let dim = cov["coverage_dimension"].as_str().unwrap_or("");
        if dim == "row_ingestion" {
            if cov["expected_population_count"] != cov["encountered_count"]
                || cov["observable_count"] != cov["accepted_count"]
                || cov["unknown_or_missing_count"].as_u64()
                    != Some(
                        cov["rejected_count"].as_u64().unwrap_or(0)
                            + cov["recognized_unprojected_count"].as_u64().unwrap_or(0),
                    )
            {
                return false;
            }
        } else {
            if cov["expected_population_count"].as_u64() != Some(total_target) {
                return false;
            }
            let obs_count = match dim {
                "session_link_id" => {
                    if t_name != "sessions" {
                        return false;
                    }
                    target_rows
                        .iter()
                        .filter(|r| r["agent.session.link_id"].as_str() != Some("unlinked"))
                        .count() as u64
                }
                "timestamp_fidelity" => {
                    if t_name != "sessions" {
                        return false;
                    }
                    target_rows
                        .iter()
                        .filter(|r| r["timestamp_status"].as_str() == Some("complete"))
                        .count() as u64
                }
                "model_attribution" => {
                    if !matches!(t_name, "invocations" | "usage") {
                        return false;
                    }
                    target_rows
                        .iter()
                        .filter(|r| {
                            matches!(
                                r["model_status"].as_str(),
                                Some("observed" | "inferred_from_session")
                            )
                        })
                        .count() as u64
                }
                "observed_cost_usd" => {
                    if t_name != "usage" {
                        return false;
                    }
                    target_rows
                        .iter()
                        .filter(|r| {
                            matches!(
                                r["cost_provenance"].as_str(),
                                Some("harness_observed_exact" | "provider_receipt_exact")
                            )
                        })
                        .count() as u64
                }
                "tool_outcome_pairing" => {
                    if t_name != "tool_calls" {
                        return false;
                    }
                    target_rows
                        .iter()
                        .filter(|r| {
                            matches!(
                                r["outcome_status"].as_str(),
                                Some("succeeded" | "failed" | "cancelled")
                            )
                        })
                        .count() as u64
                }
                "git_outcome_evidence" => {
                    if t_name != "outcomes" {
                        return false;
                    }
                    target_rows
                        .iter()
                        .filter(|r| {
                            matches!(
                                r["evidence_status"].as_str(),
                                Some("observed_full" | "observed_partial")
                            )
                        })
                        .count() as u64
                }
                _ => return false,
            };

            if cov["observable_count"].as_u64() != Some(obs_count)
                || cov["unknown_or_missing_count"].as_u64() != Some(total_target - obs_count)
            {
                return false;
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn contract_base_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("docs")
            .join("contracts")
            .join("fact-export-v1")
    }

    #[test]
    fn verify_fact_export_v1_schemas_vectors_and_contract_digest() {
        let base = contract_base_dir();
        let (schemas, schema_digest) =
            verify_and_load_schemas(&base).expect("all 6 table schemas must be valid");
        assert_eq!(schemas.len(), 6);
        assert!(is_hex64(&schema_digest));

        let mut total_steps = 0usize;
        for fname in EXPECTED_VECTOR_FILES {
            let v_path = base.join("vectors").join(fname);
            let raw = std::fs::read_to_string(&v_path)
                .unwrap_or_else(|e| panic!("failed to read {}: {e}", v_path.display()));
            let doc: Value = serde_json::from_str(&raw)
                .unwrap_or_else(|e| panic!("invalid JSON in {}: {e}", v_path.display()));

            assert_eq!(
                doc["contract_id"].as_str(),
                Some(FACT_EXPORT_CONTRACT_ID),
                "{fname}: contract_id mismatch"
            );
            assert_eq!(
                doc["contract_version"].as_u64(),
                Some(FACT_EXPORT_CONTRACT_VERSION),
                "{fname}: contract_version mismatch"
            );
            assert_eq!(
                doc["vector_file"].as_str(),
                Some(fname),
                "{fname}: vector_file mismatch"
            );
            assert_eq!(
                doc["expected_schema_digest"].as_str(),
                Some(schema_digest.as_str()),
                "{fname}: expected_schema_digest mismatch"
            );

            let mut sm = ReplayStateMachine::new(
                schemas.clone(),
                schema_digest.clone(),
                doc["expected_tenant_placeholder"]
                    .as_str()
                    .unwrap()
                    .to_string(),
                doc["expected_producer_id"].as_str().unwrap().to_string(),
                doc["expected_stream"].as_str().unwrap().to_string(),
            );

            let steps = doc["steps"].as_array().expect("steps must be array");
            assert!(!steps.is_empty(), "{fname}: empty steps");

            for step in steps {
                total_steps += 1;
                let step_id = step["step_id"].as_str().unwrap();
                let status = sm.apply_step(step);
                let exp = &step["expected_step_result"];
                assert_eq!(
                    status.as_str(),
                    exp["status"].as_str().unwrap(),
                    "{fname}:{step_id}: status mismatch"
                );
                assert_eq!(
                    sm.high_water_sequence,
                    exp["high_water_sequence"].as_u64().unwrap(),
                    "{fname}:{step_id}: high_water_sequence mismatch"
                );
                let actual_counts_val = serde_json::to_value(sm.active_row_counts()).unwrap();
                assert_eq!(
                    &actual_counts_val, &exp["active_row_counts"],
                    "{fname}:{step_id}: active_row_counts mismatch"
                );
            }

            let final_state = &doc["expected_final_state"];
            assert_eq!(
                sm.high_water_sequence,
                final_state["high_water_sequence"].as_u64().unwrap(),
                "{fname}: final high_water_sequence mismatch"
            );
            let committed_seqs: Vec<u64> = sm.committed_manifests.keys().copied().collect();
            assert_eq!(
                serde_json::to_value(&committed_seqs).unwrap(),
                final_state["committed_sequences"],
                "{fname}: final committed_sequences mismatch"
            );
            assert_eq!(
                serde_json::to_value(&sm.active_source_generations).unwrap(),
                final_state["active_source_generations"],
                "{fname}: final active_source_generations mismatch"
            );
            assert_eq!(
                serde_json::to_value(&sm.purged_source_generations).unwrap(),
                final_state["purged_source_generations"],
                "{fname}: final purged_source_generations mismatch"
            );
            assert_eq!(
                serde_json::to_value(&sm.tombstoned_session_keys).unwrap(),
                final_state["tombstoned_session_keys"],
                "{fname}: final tombstoned_session_keys mismatch"
            );
            assert_eq!(
                serde_json::to_value(sm.export_sorted_tables()).unwrap(),
                final_state["active_tables"],
                "{fname}: final active_tables mismatch"
            );
        }

        assert!(total_steps >= 25, "expected at least 25 replay steps");

        // Verify vectors/SHA256SUMS
        let sha_sums = std::fs::read_to_string(base.join("vectors").join("SHA256SUMS")).unwrap();
        let mut seen_files = Vec::new();
        for line in sha_sums.lines() {
            let s = line.trim();
            if s.is_empty() {
                continue;
            }
            let (expected_sha, fname) = s.split_once("  ").unwrap();
            let bytes = std::fs::read(base.join("vectors").join(fname)).unwrap();
            assert_eq!(
                sha256_hex(&bytes),
                expected_sha,
                "vectors/SHA256SUMS mismatch for {fname}"
            );
            seen_files.push(fname.to_string());
        }
        assert_eq!(seen_files.len(), EXPECTED_VECTOR_FILES.len());

        // Verify CONTRACT-DIGEST
        let digest_manifest = std::fs::read_to_string(base.join("CONTRACT-DIGEST")).unwrap();
        let mut entry_lines = String::new();
        let mut declared_bundle_sha: Option<String> = None;
        let mut declared_schema_sha: Option<String> = None;
        for line in digest_manifest.lines() {
            let s = line.trim();
            if s.is_empty() {
                continue;
            }
            if let Some(rest) = s.strip_prefix("# SCHEMA-BUNDLE-SHA256:") {
                declared_schema_sha = Some(rest.trim().to_string());
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
            let bytes = std::fs::read(base.join(rel_path)).unwrap();
            assert_eq!(
                sha256_hex(&bytes),
                expected_sha,
                "CONTRACT-DIGEST mismatch for {rel_path}"
            );
            entry_lines.push_str(expected_sha);
            entry_lines.push_str("  ");
            entry_lines.push_str(rel_path);
            entry_lines.push('\n');
        }
        assert_eq!(
            declared_schema_sha.as_deref(),
            Some(schema_digest.as_str()),
            "CONTRACT-DIGEST SCHEMA-BUNDLE-SHA256 mismatch"
        );
        let computed_bundle = sha256_hex(entry_lines.as_bytes());
        assert_eq!(
            declared_bundle_sha.as_deref(),
            Some(computed_bundle.as_str()),
            "CONTRACT-DIGEST BUNDLE-SHA256 mismatch"
        );
    }

    #[test]
    fn zero_content_and_usd_micros_helpers_reject_leaks() {
        assert!(check_zero_content_string("claude-opus-4-6"));
        assert!(!check_zero_content_string("G0A_PROMPT_SENTINEL_01"));
        assert!(!check_zero_content_string("C:\\Users\\samue\\file.rs"));
        assert!(!check_zero_content_string("https://github.com/org/repo"));
        assert!(!check_zero_content_string("user@example.com"));
        assert!(!check_zero_content_string("sk-live-12345"));

        assert_eq!(parse_usd_micros("0.018500"), Some(18_500));
        assert_eq!(parse_usd_micros("12.345678"), Some(12_345_678));
        assert_eq!(parse_usd_micros("0.0185"), None);
        assert_eq!(parse_usd_micros("-0.018500"), None);
    }
}
