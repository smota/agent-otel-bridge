/*
 * Copyright agent-otel-bridge contributors
 * SPDX-License-Identifier: Apache-2.0
 */

//! Informative, independently implemented SPEC checker for fact-export-v1.
//! SNAPSHOT.md is normative; this replay harness cannot freeze the contract.

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

/// Canonical ordered list of the seven `fact-export-v1` tables.
pub const CANONICAL_TABLES: [&str; 7] = [
    "sessions",
    "invocations",
    "tool_calls",
    "usage",
    "outcomes",
    "git_evidence",
    "coverage",
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

/// Outcome strings specified by SNAPSHOT.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepStatus {
    /// SPEC result APPLIED.
    #[serde(rename = "APPLIED")]
    Applied,
    /// SPEC result SKIPPED.
    #[serde(rename = "SKIPPED")]
    Skipped,
    /// SPEC result DUPLICATE_IDEMPOTENT.
    #[serde(rename = "DUPLICATE_IDEMPOTENT")]
    DuplicateIdempotent,
    /// SPEC result IGNORED_UNCOMMITTED_NO_MANIFEST.
    #[serde(rename = "IGNORED_UNCOMMITTED_NO_MANIFEST")]
    IgnoredUncommittedNoManifest,
    /// SPEC result REJECTED_TENANT_OR_PRODUCER_MISMATCH.
    #[serde(rename = "REJECTED_TENANT_OR_PRODUCER_MISMATCH")]
    RejectedTenantOrProducerMismatch,
    /// SPEC result REJECTED_INVALID_MANIFEST.
    #[serde(rename = "REJECTED_INVALID_MANIFEST")]
    RejectedInvalidManifest,
    /// SPEC result REJECTED_SCHEMA_MISMATCH.
    #[serde(rename = "REJECTED_SCHEMA_MISMATCH")]
    RejectedSchemaMismatch,
    /// SPEC result CONFLICTING_DUPLICATE.
    #[serde(rename = "CONFLICTING_DUPLICATE")]
    ConflictingDuplicate,
    /// SPEC result GAP_PENDING.
    #[serde(rename = "GAP_PENDING")]
    GapPending,
    /// SPEC result REJECTED_FILE_INTEGRITY.
    #[serde(rename = "REJECTED_FILE_INTEGRITY")]
    RejectedFileIntegrity,
    /// SPEC result REJECTED_ROW_SCHEMA_VIOLATION.
    #[serde(rename = "REJECTED_ROW_SCHEMA_VIOLATION")]
    RejectedRowSchemaViolation,
    /// SPEC result REJECTED_STALE_GENERATION_OR_TOMBSTONE.
    #[serde(rename = "REJECTED_STALE_GENERATION_OR_TOMBSTONE")]
    RejectedStaleGenerationOrTombstone,
    /// SPEC result REJECTED_REFERENTIAL_OR_COVERAGE_MISMATCH.
    #[serde(rename = "REJECTED_REFERENTIAL_OR_COVERAGE_MISMATCH")]
    RejectedReferentialOrCoverageMismatch,
}
impl StepStatus {
    /// Returns the SPEC result string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "APPLIED",
            Self::Skipped => "SKIPPED",
            Self::DuplicateIdempotent => "DUPLICATE_IDEMPOTENT",
            Self::IgnoredUncommittedNoManifest => "IGNORED_UNCOMMITTED_NO_MANIFEST",
            Self::RejectedTenantOrProducerMismatch => "REJECTED_TENANT_OR_PRODUCER_MISMATCH",
            Self::RejectedInvalidManifest => "REJECTED_INVALID_MANIFEST",
            Self::RejectedSchemaMismatch => "REJECTED_SCHEMA_MISMATCH",
            Self::ConflictingDuplicate => "CONFLICTING_DUPLICATE",
            Self::GapPending => "GAP_PENDING",
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
        "git_evidence" => Some("git_evidence_key"),
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

/// Validates all seven table schema files in `base_dir/tables/*.schema.json` and returns
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
        if fid == 0 || !seen_field_ids.insert(fid) {
            return Err(format!(
                "{table}.{col_name}: invalid or duplicate field_id {fid}"
            ));
        }

        let is_nullable = match col_spec.get("type") {
            Some(Value::Array(types)) => types.iter().any(|t| t.as_str() == Some("null")),
            _ => false,
        };
        let types = col_spec.get("type").unwrap();
        let base_type = types.as_str().or_else(|| {
            types
                .as_array()
                .and_then(|a| a.iter().filter_map(Value::as_str).find(|t| *t != "null"))
        });
        let (physical, logical) = match base_type {
            Some("string") => ("BYTE_ARRAY", "STRING"),
            Some("integer") if col_name.ends_with("_at_us") => ("INT64", "TIMESTAMP(MICROS, true)"),
            Some("integer") => ("INT64", "INT(64, isSigned=true)"),
            Some("boolean") => ("BOOLEAN", "NONE"),
            Some("array") => ("GROUP", "LIST<STRING>"),
            _ => return Err(format!("{table}.{col_name}: unsupported column type")),
        };
        if pq.get("physical_type").and_then(Value::as_str) != Some(physical)
            || pq.get("logical_type").and_then(Value::as_str) != Some(logical)
        {
            return Err(format!("{table}.{col_name}: Parquet type mapping mismatch"));
        }
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

fn matches_schema_pattern(pattern: &str, val: &str) -> bool {
    match pattern {
        "^[0-9a-f]{64}$" => is_hex64(val),
        "^[\\x21-\\x7E]{1,1024}$" => is_printable_ascii_slug(val, 1024),
        "^mcp_hmac_[0-9a-f]{64}$" => val.strip_prefix("mcp_hmac_").is_some_and(is_hex64),
        "^([0-9a-f]{64}|unlinked)$" => is_link_id_or_unlinked(val),
        "^[\\x21-\\x7E]{1,128}$" => is_printable_ascii_slug(val, 128),
        "^[a-z0-9][a-z0-9._-]{1,63}$" => is_key_id_slug(val),
        "^[A-Za-z0-9._+-]{1,64}$" => is_harness_version_token(val),
        "^[A-Za-z0-9._:/-]{1,128}$" => is_model_token(val, 128),
        "^[A-Za-z0-9._:/-]{1,96}$" => is_model_token(val, 96),
        "^[A-Za-z0-9_.:-]{1,64}$" => is_tool_name_token(val),
        _ => false,
    }
}

fn field_matches(value: &Value, rule: &Value) -> bool {
    if let Some(items) = rule.get("enum").and_then(Value::as_array) {
        if !items.contains(value) {
            return false;
        }
    }
    if let Some(choices) = rule.get("anyOf").and_then(Value::as_array) {
        if !choices.iter().any(|r| field_matches(value, r)) {
            return false;
        }
    }
    if let Some(types) = rule.get("type") {
        let kind = match value {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) if value.as_i64().is_some() => "integer",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            _ => return false,
        };
        let accepted = types.as_str() == Some(kind)
            || types
                .as_array()
                .is_some_and(|a| a.iter().any(|t| t.as_str() == Some(kind)));
        if !accepted {
            return false;
        }
    }
    match value {
        Value::String(s) => {
            if !check_zero_content_string(s) {
                return false;
            }
            let n = s.chars().count() as u64;
            if rule
                .get("minLength")
                .and_then(Value::as_u64)
                .is_some_and(|min| n < min)
                || rule
                    .get("maxLength")
                    .and_then(Value::as_u64)
                    .is_some_and(|max| n > max)
            {
                return false;
            }
            if let Some(pattern) = rule.get("pattern").and_then(Value::as_str) {
                if !matches_schema_pattern(pattern, s) {
                    return false;
                }
            }
        }
        Value::Number(_) => {
            let Some(n) = value.as_i64() else {
                return false;
            };
            if rule
                .get("minimum")
                .and_then(Value::as_i64)
                .is_some_and(|min| n < min)
                || rule
                    .get("maximum")
                    .and_then(Value::as_i64)
                    .is_some_and(|max| n > max)
            {
                return false;
            }
        }
        Value::Array(a) => {
            if rule
                .get("minItems")
                .and_then(Value::as_u64)
                .is_some_and(|min| (a.len() as u64) < min)
            {
                return false;
            }
            if rule.get("uniqueItems").and_then(Value::as_bool) == Some(true) {
                let unique: BTreeSet<String> = a.iter().map(canonical_json_string).collect();
                if unique.len() != a.len() {
                    return false;
                }
            }
            if let Some(items) = rule.get("items") {
                if !a.iter().all(|v| field_matches(v, items)) {
                    return false;
                }
            }
        }
        Value::Object(_) => return false,
        _ => {}
    }
    true
}

/// Checks incorporated field constraints and the explicit SPEC row invariants.
pub fn validate_row_against_schema(table: &str, row: &Value, schema: &Value, key_id: &str) -> bool {
    let (Some(r), Some(props)) = (row.as_object(), schema["properties"].as_object()) else {
        return false;
    };
    if r.len() != props.len()
        || r.keys().any(|k| !props.contains_key(k))
        || r.get("pseudonym_key_id").and_then(Value::as_str) != Some(key_id)
    {
        return false;
    }
    if !props
        .iter()
        .all(|(name, spec)| r.get(name).is_some_and(|v| field_matches(v, spec)))
    {
        return false;
    }
    match table {
        "sessions" => validate_sessions_invariants(r),
        "invocations" => validate_invocations_invariants(r),
        "tool_calls" => validate_tool_calls_invariants(r),
        "usage" => validate_usage_invariants(r),
        "outcomes" => validate_outcomes_invariants(r),
        "git_evidence" => validate_git_invariants(r),
        "coverage" => validate_coverage_invariants(r),
        _ => false,
    }
}

fn validate_sessions_invariants(r: &Map<String, Value>) -> bool {
    let linked = r["agent.session.link_id"].as_str() != Some("unlinked");
    if linked != r["unlinked_reason"].is_null() || linked == r["link_epoch"].is_null() {
        return false;
    }
    if linked
        && (r["origin"].as_str() != Some("harness")
            || r["canonical_harness"].as_str() == Some("unknown"))
    {
        return false;
    }
    for field in ["org", "endpoint_scope", "team"] {
        if (r[&format!("{field}_status")].as_str() == Some("observed")) == r[field].is_null() {
            return false;
        }
    }
    let org = r["org_status"].as_str();
    let endpoint = r["endpoint_scope_status"].as_str();
    let reason = if [org, endpoint].contains(&Some("invalid_field_characters")) {
        Some("invalid_field_characters")
    } else if org == Some("missing_org") {
        Some("missing_org")
    } else if endpoint == Some("missing_endpoint_scope") {
        Some("missing_endpoint_scope")
    } else {
        None
    };
    if let Some(reason) = reason {
        if linked || r["unlinked_reason"].as_str() != Some(reason) {
            return false;
        }
    }
    if (r["harness_version_status"].as_str() == Some("observed")) == r["harness_version"].is_null()
    {
        return false;
    }
    let start = r["started_at_us"].as_i64();
    let end = r["ended_at_us"].as_i64();
    match r["timestamp_status"].as_str() {
        Some("complete") if start.is_some() && end.is_some() && end >= start => {}
        Some("start_only") if start.is_some() && end.is_none() => {}
        Some("not_observable") if start.is_none() && end.is_none() => {}
        _ => return false,
    }
    if r["is_root"].as_bool() == Some(true)
        && (r["lineage_relation"].as_str() != Some("root")
            || r["lineage_status"].as_str() != Some("root")
            || !r["parent_session_key"].is_null())
    {
        return false;
    }
    if r["lineage_status"].as_str() == Some("evidenced") {
        r["is_root"].as_bool() == Some(false)
            && !r["parent_session_key"].is_null()
            && r["parent_session_key"] != r["session_key"]
            && r["lineage_relation"].as_str() != Some("root")
    } else {
        r["parent_session_key"].is_null()
    }
}

fn validate_git_invariants(r: &Map<String, Value>) -> bool {
    for (status, field) in [
        ("merge_status", "is_merge"),
        ("default_branch_status", "on_default_branch"),
        ("observed_at_status", "observed_at_us"),
    ] {
        if (r[status].as_str() == Some("observed")) == r[field].is_null() {
            return false;
        }
    }
    if let Some(is_merge) = r["is_merge"].as_bool() {
        if is_merge != (r["parent_hmacs"].as_array().unwrap().len() > 1) {
            return false;
        }
    }
    (r["session_match_status"].as_str() == Some("matched")) != r["session_key"].is_null()
}

/// Exact binary64 USD scaling with round-half-even, without intermediate f64 rounding.
pub fn round_usd_micros(usd: f64) -> Option<i64> {
    if !usd.is_finite() || usd < 0.0 {
        return None;
    }
    let bits = usd.to_bits();
    let encoded_exponent = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1u64 << 52) - 1);
    let (mantissa, exponent) = if encoded_exponent == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1u64 << 52), encoded_exponent - 1023 - 52)
    };
    let scaled = u128::from(mantissa) * 1_000_000;
    let rounded = if exponent >= 0 {
        scaled.checked_mul(1u128.checked_shl(exponent as u32)?)?
    } else {
        let shift = (-exponent) as u32;
        if shift >= 128 {
            0
        } else {
            let whole = scaled >> shift;
            let rem = scaled & ((1u128 << shift) - 1);
            let half = 1u128 << (shift - 1);
            whole + u128::from(rem > half || (rem == half && whole & 1 == 1))
        }
    };
    i64::try_from(rounded).ok()
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
    if Some(tot_tok)
        != in_tok
            .checked_add(out_tok)
            .and_then(|v| v.checked_add(cr_tok))
            .and_then(|v| v.checked_add(cw_tok))
    {
        return false;
    }
    if (r["reasoning_status"].as_str() == Some("observed")) != !r["reasoning_tokens"].is_null() {
        return false;
    }
    let aggregate = r["usage_grain"].as_str() == Some("session_aggregate");
    if aggregate != (r["counter_semantics"].as_str() == Some("session_aggregate"))
        || aggregate != (r["invocation_link_status"].as_str() == Some("session_level_summary"))
    {
        return false;
    }
    let known = r["cost_provenance"].as_str() != Some("unknown");
    known != r["observed_usd_micros"].is_null() && known == r["cost_unknown_reason"].is_null()
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
    if Some(enc) != acc.checked_add(rej).and_then(|v| v.checked_add(unproj)) {
        return false;
    }

    let exp_pop = r["expected_population_count"].as_u64().unwrap_or(u64::MAX);
    let obs = r["observable_count"].as_u64().unwrap_or(0);
    let unk = r["unknown_or_missing_count"].as_u64().unwrap_or(0);
    if Some(exp_pop) != obs.checked_add(unk) {
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

fn exact_keys(object: &Map<String, Value>, keys: &[&str]) -> bool {
    object.len() == keys.len() && keys.iter().all(|k| object.contains_key(*k))
}
fn positive_i64(v: &Value) -> bool {
    v.as_i64().is_some_and(|n| n >= 1)
}
fn nonnegative_i64(v: &Value) -> bool {
    v.as_i64().is_some_and(|n| n >= 0)
}

/// Validates production manifest structure; vector JSON transport is rejected.
pub fn validate_manifest_structure(manifest: &Value) -> bool {
    manifest_shape(manifest, false)
}
fn manifest_shape(value: &Value, vector_mode: bool) -> bool {
    let Some(m) = value.as_object() else {
        return false;
    };
    if !exact_keys(
        m,
        &[
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
            "skip_reason",
        ],
    ) {
        return false;
    }
    if m["contract_id"].as_str() != Some(FACT_EXPORT_CONTRACT_ID)
        || m["contract_version"].as_i64() != Some(1)
        || !m["schema_digest"].as_str().is_some_and(is_hex64)
        || !m["manifest_digest"].as_str().is_some_and(is_hex64)
        || !m["tenant_placeholder"]
            .as_str()
            .is_some_and(is_tenant_placeholder)
        || !m["stream"].as_str().is_some_and(is_stream_slug)
        || !positive_i64(&m["snapshot_sequence"])
        || !positive_i64(&m["published_at_us"])
    {
        return false;
    }
    let Some(p) = m["producer"].as_object() else {
        return false;
    };
    if !exact_keys(
        p,
        &[
            "producer_id",
            "producer_kind",
            "producer_version",
            "pseudonym_key_id",
        ],
    ) || !p["producer_id"].as_str().is_some_and(is_hex64)
        || !p["pseudonym_key_id"].as_str().is_some_and(is_key_id_slug)
        || !p["producer_version"].as_str().is_some_and(is_semver_token)
        || !matches!(
            p["producer_kind"].as_str(),
            Some("dendro_cli" | "agent_otel_daemon" | "dendro_station_exporter")
        )
    {
        return false;
    }
    let Some(s) = m["replacement_scope"].as_object() else {
        return false;
    };
    if !exact_keys(
        s,
        &[
            "scope_mode",
            "operation",
            "reason",
            "source_id",
            "source_generation",
            "session_keys",
        ],
    ) || !s["source_id"].as_str().is_some_and(is_hex64)
        || !positive_i64(&s["source_generation"])
        || !matches!(
            s["scope_mode"].as_str(),
            Some("source_generation" | "session_keys")
        )
        || !matches!(
            s["operation"].as_str(),
            Some("upsert_replace" | "purge_delete")
        )
        || !matches!(
            s["reason"].as_str(),
            Some(
                "initial_export"
                    | "in_generation_outcome_or_attribution_update"
                    | "source_generation_advance"
                    | "retention_or_redaction_purge"
                    | "key_rotation"
            )
        )
    {
        return false;
    }
    let Some(keys) = s["session_keys"].as_array() else {
        return false;
    };
    if keys.iter().any(|k| !k.as_str().is_some_and(is_hex64)) {
        return false;
    }
    let unique: BTreeSet<&str> = keys.iter().filter_map(Value::as_str).collect();
    if unique.len() != keys.len()
        || (s["scope_mode"].as_str() == Some("session_keys")) == keys.is_empty()
        || (!keys.is_empty() && s["operation"].as_str() != Some("purge_delete"))
    {
        return false;
    }
    let Some(files) = m["files"].as_array() else {
        return false;
    };
    if !m["skip_reason"].is_null() {
        return matches!(
            m["skip_reason"].as_str(),
            Some("abandoned_publication" | "source_unavailable" | "producer_recovery")
        ) && files.is_empty()
            && keys.is_empty()
            && s["operation"].as_str() == Some("upsert_replace");
    }
    if files.len() != CANONICAL_TABLES.len() {
        return false;
    }
    for (table, file) in CANONICAL_TABLES.iter().zip(files) {
        let Some(f) = file.as_object() else {
            return false;
        };
        if !exact_keys(
            f,
            &[
                "table",
                "path",
                "format",
                "row_count",
                "byte_length",
                "sha256",
            ],
        ) || f["table"].as_str() != Some(*table)
            || !nonnegative_i64(&f["row_count"])
            || !nonnegative_i64(&f["byte_length"])
            || !f["sha256"].as_str().is_some_and(is_hex64)
        {
            return false;
        }
        let extension = match f["format"].as_str() {
            Some("parquet") => "parquet",
            Some("canonical_json_rows") if vector_mode => "jsonl",
            _ => return false,
        };
        if f["path"].as_str() != Some(format!("tables/{table}.{extension}").as_str()) {
            return false;
        }
    }
    true
}

/// Informative replay state, isolated to its authenticated tenant/producer/stream.
/// This harness simulates decoded vector payloads, not production Parquet IO.
#[derive(Debug, Clone)]
pub struct ReplayStateMachine {
    schemas: BTreeMap<String, Value>,
    expected_schema_digest: String,
    expected_tenant: String,
    expected_producer_id: String,
    expected_stream: String,
    /// Highest consumed sequence, including rejections and explicit skips.
    pub high_water_sequence: u64,
    /// Digests of every consumed sequence (historical name retained).
    pub committed_manifests: BTreeMap<u64, String>,
    /// Source generation watermarks.
    pub active_source_generations: BTreeMap<String, u64>,
    /// Purged generation watermarks.
    pub purged_source_generations: BTreeMap<String, u64>,
    /// Last accepted fact key per stable source.
    pub source_key_ids: BTreeMap<String, String>,
    /// Permanent session keys across all sources in this import key.
    pub tombstoned_session_keys: BTreeSet<String>,
    /// Active rows, kept byte-identical to producer rows.
    pub tables: BTreeMap<String, BTreeMap<String, Value>>,
    /// Importer-owned operational coverage, independent of producer coverage.
    pub import_coverage: Vec<Value>,
}
impl ReplayStateMachine {
    /// Creates a vector replay instance bound to exactly one import key.
    pub fn new(
        schemas: BTreeMap<String, Value>,
        expected_schema_digest: String,
        expected_tenant: String,
        expected_producer_id: String,
        expected_stream: String,
    ) -> Self {
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
            source_key_ids: BTreeMap::new(),
            tombstoned_session_keys: BTreeSet::new(),
            tables: CANONICAL_TABLES
                .iter()
                .map(|t| (t.to_string(), BTreeMap::new()))
                .collect(),
            import_coverage: Vec::new(),
        }
    }
    /// Counts active rows per table.
    pub fn active_row_counts(&self) -> BTreeMap<String, usize> {
        self.tables
            .iter()
            .map(|(t, rows)| (t.clone(), rows.len()))
            .collect()
    }
    /// Returns rows in primary-key order, preserving every value.
    pub fn export_sorted_tables(&self) -> BTreeMap<String, Vec<Value>> {
        self.tables
            .iter()
            .map(|(t, rows)| (t.clone(), rows.values().cloned().collect()))
            .collect()
    }
    /// Evaluates SPEC steps 1-9 in order; only steps 6-9 consume a slot.
    pub fn apply_step(&mut self, step: &Value) -> StepStatus {
        let Some(value) = step.get("committed_manifest").filter(|v| !v.is_null()) else {
            return StepStatus::IgnoredUncommittedNoManifest;
        };
        let Some(m) = value.as_object() else {
            return StepStatus::RejectedInvalidManifest;
        };
        if m.get("tenant_placeholder").and_then(Value::as_str)
            != Some(self.expected_tenant.as_str())
            || m.get("producer")
                .and_then(|p| p.get("producer_id"))
                .and_then(Value::as_str)
                != Some(self.expected_producer_id.as_str())
            || m.get("stream").and_then(Value::as_str) != Some(self.expected_stream.as_str())
        {
            return StepStatus::RejectedTenantOrProducerMismatch;
        }
        if !manifest_shape(value, true) {
            return StepStatus::RejectedInvalidManifest;
        }
        let sequence = m["snapshot_sequence"].as_u64().unwrap();
        let digest = m["manifest_digest"].as_str().unwrap();
        if let Some(prior) = self.committed_manifests.get(&sequence) {
            return if prior == digest {
                StepStatus::DuplicateIdempotent
            } else {
                StepStatus::ConflictingDuplicate
            };
        }
        if sequence > self.high_water_sequence + 1 {
            return StepStatus::GapPending;
        }
        let status = self.evaluate_next(m, step);
        self.high_water_sequence = sequence;
        self.committed_manifests
            .insert(sequence, digest.to_string());
        let (kind, reason) = match status {
            StepStatus::Applied => ("accepted", Value::Null),
            StepStatus::Skipped => ("skipped", m["skip_reason"].clone()),
            other => ("rejected", Value::String(other.as_str().to_string())),
        };
        self.import_coverage.push(serde_json::json!({
            "tenant_placeholder": self.expected_tenant, "producer_id": self.expected_producer_id,
            "stream": self.expected_stream, "snapshot_sequence": sequence, "manifest_digest": digest,
            "status": kind, "reason": reason
        }));
        status
    }
    fn evaluate_next(&mut self, m: &Map<String, Value>, step: &Value) -> StepStatus {
        if m["manifest_digest"].as_str() != Some(compute_manifest_digest(m).as_str()) {
            return StepStatus::RejectedInvalidManifest;
        }
        if m["schema_digest"].as_str() != Some(self.expected_schema_digest.as_str()) {
            return StepStatus::RejectedSchemaMismatch;
        }
        if !m["skip_reason"].is_null() {
            return StepStatus::Skipped;
        }
        let Some(staged) = step.get("staged_tables").and_then(Value::as_object) else {
            return StepStatus::RejectedFileIntegrity;
        };
        for file in m["files"].as_array().unwrap() {
            let table = file["table"].as_str().unwrap();
            let Some(rows) = staged.get(table).and_then(Value::as_array) else {
                return StepStatus::RejectedFileIntegrity;
            };
            let bytes = canonical_table_jsonl_bytes(rows);
            if file["row_count"].as_u64() != Some(rows.len() as u64)
                || file["byte_length"].as_u64() != Some(bytes.len() as u64)
                || file["sha256"].as_str() != Some(sha256_hex(&bytes).as_str())
            {
                return StepStatus::RejectedFileIntegrity;
            }
        }
        let key_id = m["producer"]["pseudonym_key_id"].as_str().unwrap();
        for table in CANONICAL_TABLES {
            let pk = primary_key_for_table(table).unwrap();
            let mut seen = BTreeSet::new();
            for row in staged[table].as_array().unwrap() {
                if !validate_row_against_schema(table, row, &self.schemas[table], key_id) {
                    return StepStatus::RejectedRowSchemaViolation;
                }
                if !seen.insert(row[pk].as_str().unwrap()) {
                    return StepStatus::RejectedRowSchemaViolation;
                }
            }
        }
        let scope = &m["replacement_scope"];
        let source = scope["source_id"].as_str().unwrap();
        let generation = scope["source_generation"].as_u64().unwrap();
        let active = self
            .active_source_generations
            .get(source)
            .copied()
            .unwrap_or(0);
        let purged = self
            .purged_source_generations
            .get(source)
            .copied()
            .unwrap_or(0);
        let session_purge = scope["scope_mode"].as_str() == Some("session_keys");
        let purge = scope["operation"].as_str() == Some("purge_delete");
        let stale = if session_purge {
            generation != active
                || self.source_key_ids.get(source).map(String::as_str) != Some(key_id)
        } else {
            generation < active || generation <= purged
        };
        if stale {
            return StepStatus::RejectedStaleGenerationOrTombstone;
        }
        if !purge {
            if self
                .source_key_ids
                .get(source)
                .is_some_and(|old| old != key_id)
                && purged < active
            {
                return StepStatus::RejectedStaleGenerationOrTombstone;
            }
            for table in CANONICAL_TABLES {
                for row in staged[table].as_array().unwrap() {
                    if row
                        .get("session_key")
                        .and_then(Value::as_str)
                        .is_some_and(|sk| self.tombstoned_session_keys.contains(sk))
                    {
                        return StepStatus::RejectedStaleGenerationOrTombstone;
                    }
                }
            }
        }
        if purge
            && CANONICAL_TABLES
                .iter()
                .any(|t| !staged[*t].as_array().unwrap().is_empty())
        {
            return StepStatus::RejectedReferentialOrCoverageMismatch;
        }
        let mut candidate = self.tables.clone();
        let selected: BTreeSet<&str> = scope["session_keys"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        if session_purge {
            for table in CANONICAL_TABLES.iter().filter(|t| **t != "coverage") {
                candidate.get_mut(*table).unwrap().retain(|_, r| {
                    !(r["source_id"].as_str() == Some(source)
                        && r.get("session_key")
                            .and_then(Value::as_str)
                            .is_some_and(|sk| selected.contains(sk)))
                });
            }
        } else {
            for table in CANONICAL_TABLES {
                let rows = candidate.get_mut(table).unwrap();
                rows.retain(|_, r| r["source_id"].as_str() != Some(source));
                if !purge {
                    let pk = primary_key_for_table(table).unwrap();
                    for row in staged[table].as_array().unwrap() {
                        let key = row[pk].as_str().unwrap();
                        if row["source_id"].as_str() != Some(source)
                            || row["source_generation"].as_u64() != Some(generation)
                            || rows.contains_key(key)
                        {
                            return StepStatus::RejectedReferentialOrCoverageMismatch;
                        }
                        rows.insert(key.to_string(), row.clone());
                    }
                }
            }
            if !purge {
                let focused = candidate
                    .iter()
                    .map(|(t, rows)| {
                        (
                            t.clone(),
                            rows.iter()
                                .filter(|(_, r)| r["source_id"].as_str() == Some(source))
                                .map(|(k, r)| (k.clone(), r.clone()))
                                .collect(),
                        )
                    })
                    .collect();
                if !verify_cross_table_and_coverage(&focused) {
                    return StepStatus::RejectedReferentialOrCoverageMismatch;
                }
            }
        }
        self.tables = candidate;
        if session_purge {
            self.tombstoned_session_keys
                .extend(selected.into_iter().map(String::from));
        } else {
            self.active_source_generations
                .insert(source.to_string(), generation);
            if purge {
                self.purged_source_generations
                    .insert(source.to_string(), generation);
            } else {
                self.source_key_ids
                    .insert(source.to_string(), key_id.to_string());
            }
        }
        StepStatus::Applied
    }
}

/// Public coverage tuple digest. No secret key is required.
pub fn coverage_key(row: &Value) -> String {
    let generation = row["source_generation"].as_u64().unwrap_or(0).to_string();
    let parts = [
        "fact-export.v1/coverage",
        row["source_id"].as_str().unwrap_or(""),
        generation.as_str(),
        row["canonical_harness"].as_str().unwrap_or(""),
        row["target_table"].as_str().unwrap_or(""),
        row["coverage_dimension"].as_str().unwrap_or(""),
    ];
    let mut encoded = Vec::new();
    for part in parts {
        encoded.extend_from_slice(&(part.len() as u32).to_be_bytes());
        encoded.extend_from_slice(part.as_bytes());
    }
    sha256_hex(&encoded)
}

fn coverage_observable(table: &str, dimension: &str, row: &Value) -> Option<bool> {
    match (dimension, table) {
        ("session_link_id", "sessions") => {
            Some(row["agent.session.link_id"].as_str() != Some("unlinked"))
        }
        ("model_attribution", "invocations" | "usage") => {
            Some(row["model_status"].as_str() != Some("unknown"))
        }
        ("observed_cost_usd", "usage") => Some(row["cost_provenance"].as_str() != Some("unknown")),
        ("tool_outcome_pairing", "tool_calls") => Some(!matches!(
            row["outcome_status"].as_str(),
            Some("pending" | "unknown")
        )),
        ("git_outcome_evidence", "outcomes") => {
            Some(row["evidence_status"].as_str() != Some("missing"))
        }
        ("git_outcome_evidence", "git_evidence") => {
            Some(row["session_match_status"].as_str() == Some("matched"))
        }
        ("timestamp_fidelity", "sessions") => {
            Some(row["timestamp_status"].as_str() == Some("complete"))
        }
        ("timestamp_fidelity", "invocations" | "tool_calls" | "usage") => {
            Some(row["occurred_at_status"].as_str() == Some("observed"))
        }
        ("timestamp_fidelity", "outcomes" | "git_evidence") => {
            Some(row["observed_at_status"].as_str() == Some("observed"))
        }
        _ => None,
    }
}
fn population_tuple(row: &Value, table: &str) -> (String, u64, String, String) {
    (
        row["source_id"].as_str().unwrap_or("").to_string(),
        row["source_generation"].as_u64().unwrap_or(0),
        row["canonical_harness"].as_str().unwrap_or("").to_string(),
        table.to_string(),
    )
}

/// Checks the affected-source candidate against the explicit relational SPEC.
pub fn verify_cross_table_and_coverage(tables: &BTreeMap<String, BTreeMap<String, Value>>) -> bool {
    let sessions = &tables["sessions"];
    let invocations = &tables["invocations"];
    let mut identities = BTreeSet::new();
    let mut usage_grains = BTreeMap::new();
    for table in CANONICAL_TABLES
        .iter()
        .filter(|t| !matches!(**t, "sessions" | "coverage"))
    {
        for row in tables[*table].values() {
            if let Some(sk) = row["session_key"].as_str() {
                let Some(parent) = sessions.get(sk) else {
                    return false;
                };
                if [
                    "source_id",
                    "source_generation",
                    "canonical_harness",
                    "pseudonym_key_id",
                ]
                .iter()
                .any(|k| row[*k] != parent[*k])
                {
                    return false;
                }
            }
            if matches!(*table, "tool_calls" | "usage") {
                if let Some(ik) = row["invocation_key"].as_str() {
                    let Some(invocation) = invocations.get(ik) else {
                        return false;
                    };
                    if invocation["session_key"] != row["session_key"] {
                        return false;
                    }
                }
            }
            if *table == "invocations" {
                let tuple = canonical_json_string(&serde_json::json!([
                    row["session_key"],
                    row["sequence_no"],
                    row["invocation_grain"],
                    row["message_key"]
                ]));
                if !identities.insert(tuple) {
                    return false;
                }
            }
            if *table == "usage" {
                let sk = row["session_key"].as_str().unwrap_or("");
                let grain = row["usage_grain"].as_str().unwrap_or("");
                if let Some(prior) = usage_grains.insert(sk, grain) {
                    if prior != grain || grain == "session_aggregate" {
                        return false;
                    }
                }
            }
        }
    }
    let mut populations: BTreeMap<_, Vec<&Value>> = BTreeMap::new();
    for table in CANONICAL_TABLES.iter().filter(|t| **t != "coverage") {
        for row in tables[*table].values() {
            populations
                .entry(population_tuple(row, table))
                .or_default()
                .push(row);
        }
    }
    let mut seen = BTreeSet::new();
    let mut ingestion = BTreeSet::new();
    for cov in tables["coverage"].values() {
        let table = cov["target_table"].as_str().unwrap_or("");
        let dimension = cov["coverage_dimension"].as_str().unwrap_or("");
        if table == "coverage" || !CANONICAL_TABLES.contains(&table) {
            return false;
        }
        let tuple = population_tuple(cov, table);
        if !seen.insert((tuple.clone(), dimension.to_string()))
            || cov["coverage_key"].as_str() != Some(coverage_key(cov).as_str())
        {
            return false;
        }
        let empty = Vec::new();
        let rows = populations.get(&tuple).unwrap_or(&empty);
        let total = rows.len() as u64;
        if cov["accepted_count"].as_u64() != Some(total) {
            return false;
        }
        if dimension == "row_ingestion" {
            ingestion.insert(tuple);
            let missing = cov["rejected_count"]
                .as_u64()
                .unwrap_or(0)
                .checked_add(cov["recognized_unprojected_count"].as_u64().unwrap_or(0));
            if cov["expected_population_count"] != cov["encountered_count"]
                || cov["observable_count"].as_u64() != Some(total)
                || cov["unknown_or_missing_count"].as_u64() != missing
            {
                return false;
            }
        } else {
            if coverage_observable(table, dimension, &Value::Null).is_none() {
                return false;
            }
            let observed = rows
                .iter()
                .filter(|r| coverage_observable(table, dimension, r) == Some(true))
                .count() as u64;
            if cov["expected_population_count"].as_u64() != Some(total)
                || cov["observable_count"].as_u64() != Some(observed)
                || cov["unknown_or_missing_count"].as_u64() != Some(total - observed)
            {
                return false;
            }
        }
    }
    populations.keys().all(|k| ingestion.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    fn base() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("docs/contracts/fact-export-v1")
    }
    fn read(path: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }
    fn machine(schemas: &BTreeMap<String, Value>, digest: &str, doc: &Value) -> ReplayStateMachine {
        ReplayStateMachine::new(
            schemas.clone(),
            digest.to_string(),
            doc["expected_tenant_placeholder"]
                .as_str()
                .unwrap()
                .to_string(),
            doc["expected_producer_id"].as_str().unwrap().to_string(),
            doc["expected_stream"].as_str().unwrap().to_string(),
        )
    }
    #[test]
    fn verify_fact_export_v1_schemas_vectors_and_contract_digest() {
        let base = base();
        let (schemas, digest) = verify_and_load_schemas(&base).unwrap();
        assert_eq!(schemas.len(), 7);
        let mut paths: Vec<_> = std::fs::read_dir(base.join("vectors"))
            .unwrap()
            .map(|p| p.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        paths.sort();
        assert!(!paths.is_empty());
        let mut steps = 0;
        for path in &paths {
            let doc = read(path);
            let name = path.file_name().unwrap().to_str().unwrap();
            assert_eq!(doc["vector_file"], name);
            assert_eq!(doc["contract_id"], FACT_EXPORT_CONTRACT_ID);
            assert_eq!(doc["contract_version"], 1);
            assert_eq!(doc["expected_schema_digest"], digest);
            let mut sm = machine(&schemas, &digest, &doc);
            for step in doc["steps"].as_array().unwrap() {
                steps += 1;
                let result = sm.apply_step(step);
                let expected = &step["expected_step_result"];
                assert_eq!(
                    result.as_str(),
                    expected["status"].as_str().unwrap(),
                    "{name}:{}",
                    step["step_id"]
                );
                assert_eq!(
                    sm.high_water_sequence,
                    expected["high_water_sequence"].as_u64().unwrap(),
                    "{name}: cursor"
                );
                assert_eq!(
                    serde_json::to_value(sm.active_row_counts()).unwrap(),
                    expected["active_row_counts"],
                    "{name}: counts"
                );
                assert_eq!(
                    serde_json::to_value(&sm.import_coverage).unwrap(),
                    expected["import_coverage"],
                    "{name}: import coverage"
                );
                let actual_digests: BTreeMap<String, String> = sm
                    .export_sorted_tables()
                    .iter()
                    .map(|(t, rows)| (t.clone(), sha256_hex(&canonical_table_jsonl_bytes(rows))))
                    .collect();
                assert_eq!(
                    serde_json::to_value(actual_digests).unwrap(),
                    expected["active_table_digests"],
                    "{name}: row bytes"
                );
                let manifest = &step["committed_manifest"];
                if manifest["skip_reason"].is_null() && manifest_shape(manifest, true) {
                    assert!(!validate_manifest_structure(manifest));
                }
            }
            let f = &doc["expected_final_state"];
            assert_eq!(
                serde_json::to_value(sm.export_sorted_tables()).unwrap(),
                f["active_tables"],
                "{name}: rows"
            );
            assert_eq!(
                sm.high_water_sequence,
                f["high_water_sequence"].as_u64().unwrap()
            );
            assert_eq!(
                serde_json::to_value(&sm.active_source_generations).unwrap(),
                f["active_source_generations"],
                "{name}: generations"
            );
            assert_eq!(
                serde_json::to_value(&sm.purged_source_generations).unwrap(),
                f["purged_source_generations"],
                "{name}: purges"
            );
            assert_eq!(
                serde_json::to_value(&sm.tombstoned_session_keys).unwrap(),
                f["tombstoned_session_keys"],
                "{name}: tombstones"
            );
            assert_eq!(
                serde_json::to_value(&sm.source_key_ids).unwrap(),
                f["source_key_ids"],
                "{name}: keys"
            );
            assert_eq!(
                serde_json::to_value(sm.committed_manifests.keys().collect::<Vec<_>>()).unwrap(),
                f["committed_sequences"]
            );
            if let Some(cases) = doc["rounding_cases"].as_array() {
                for case in cases {
                    let bits =
                        u64::from_str_radix(case["binary64_hex"].as_str().unwrap(), 16).unwrap();
                    assert_eq!(
                        serde_json::to_value(round_usd_micros(f64::from_bits(bits))).unwrap(),
                        case["expected_micros"]
                    );
                }
            }
        }
        assert!(steps > 0);
        let mut inventory = vec!["SNAPSHOT.md".to_string(), "tables/README.md".to_string()];
        inventory.extend(
            CANONICAL_TABLES
                .iter()
                .map(|t| format!("tables/{t}.schema.json")),
        );
        inventory.extend(
            paths
                .iter()
                .map(|p| format!("vectors/{}", p.file_name().unwrap().to_str().unwrap())),
        );
        inventory.sort();
        let sums = std::fs::read(base.join("SHA256SUMS")).unwrap();
        let lines = std::str::from_utf8(&sums).unwrap();
        let mut listed = Vec::new();
        for line in lines.lines() {
            let (sha, name) = line.split_once("  ").unwrap();
            assert_eq!(
                sha256_hex(&std::fs::read(base.join(name)).unwrap()),
                sha,
                "{name}"
            );
            listed.push(name.to_string());
        }
        assert_eq!(listed, inventory);
        assert_eq!(
            std::fs::read_to_string(base.join("CONTRACT-DIGEST"))
                .unwrap()
                .trim(),
            sha256_hex(&sums)
        );
        let expected_vector_sums: String = paths
            .iter()
            .map(|p| {
                format!(
                    "{}  {}\n",
                    sha256_hex(&std::fs::read(p).unwrap()),
                    p.file_name().unwrap().to_str().unwrap()
                )
            })
            .collect();
        assert_eq!(
            std::fs::read_to_string(base.join("vectors/SHA256SUMS")).unwrap(),
            expected_vector_sums
        );
    }
    #[test]
    fn tenant_is_part_of_isolated_import_state() {
        let base = base();
        let (schemas, digest) = verify_and_load_schemas(&base).unwrap();
        let doc = read(&base.join("vectors/clean_sequence.json"));
        let event = &doc["steps"][0];
        let mut first = machine(&schemas, &digest, &doc);
        let mut second = machine(&schemas, &digest, &doc);
        second.expected_tenant = "tn_other_tenant".to_string();
        let mut changed = event.clone();
        changed["committed_manifest"]["tenant_placeholder"] =
            Value::String(second.expected_tenant.clone());
        let hash = compute_manifest_digest(changed["committed_manifest"].as_object().unwrap());
        changed["committed_manifest"]["manifest_digest"] = Value::String(hash);
        assert_eq!(
            first.apply_step(&changed),
            StepStatus::RejectedTenantOrProducerMismatch
        );
        assert_eq!(first.high_water_sequence, 0);
        assert_eq!(second.apply_step(&changed), StepStatus::Applied);
        assert_eq!(first.high_water_sequence, 0);
        assert_eq!(first.apply_step(event), StepStatus::Applied);
        assert_eq!(first.high_water_sequence, 1);
    }
    #[test]
    fn zero_content_and_rounding_boundaries() {
        assert!(check_zero_content_string("model-v1"));
        for s in [
            "G0A_PROMPT_SENTINEL",
            "C:\\Users\\file",
            "https://example.com",
            "user@example.com",
            "sk-live-123",
        ] {
            assert!(!check_zero_content_string(s));
        }
        assert_eq!(round_usd_micros(0.0078125), Some(7812));
        assert_eq!(round_usd_micros(0.0234375), Some(23438));
        assert_eq!(round_usd_micros(f64::INFINITY), None);
        assert_eq!(round_usd_micros(-1.0), None);
        assert_eq!(round_usd_micros(1.0e20), None);
    }
}
