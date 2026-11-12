#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Stdlib-only normative validator for fact-export-v1 schemas, replay vectors, and digests (C1b).

Verifies:
1. All 6 table JSON Schemas under tables/*.schema.json (Parquet mapping, field_ids, justifications).
2. Canonical schema bundle digest (schema_digest).
3. All 7 replay vector files under vectors/*.json via the normative SNAPSHOT.md state machine.
4. Zero-content (DE-004), customer-held key pseudonymization (F13), qualified I01 identity (F9),
   and explicit missingness/coverage denominator invariants (F14, D5).
5. vectors/SHA256SUMS and CONTRACT-DIGEST bundle integrity.
"""

from __future__ import annotations

import copy
import hashlib
import json
import pathlib
import re
import sys
from typing import Any

CONTRACT_ID = "fact-export-v1"
CONTRACT_VERSION = 1

CANONICAL_TABLES: tuple[str, ...] = (
    "sessions",
    "invocations",
    "tool_calls",
    "usage",
    "outcomes",
    "coverage",
)

EXPECTED_VECTOR_FILES: tuple[str, ...] = (
    "clean_sequence.json",
    "outcome_replacement.json",
    "stale_out_of_order.json",
    "duplicate_idempotent.json",
    "partial_publication.json",
    "schema_mismatch.json",
    "coverage_missingness.json",
)

HEX64_RE = re.compile(r"^[0-9a-f]{64}$")
KEY_ID_RE = re.compile(r"^[a-z0-9][a-z0-9._-]{1,63}$")
TENANT_RE = re.compile(r"^tn_[a-z0-9][a-z0-9_-]{1,60}$")
STREAM_RE = re.compile(r"^[a-z0-9][a-z0-9._-]{1,63}$")
SEMVER_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9._-]+)?$")
USD_DECIMAL_RE = re.compile(r"^([0-9]+)\.([0-9]{6})$")

FORBIDDEN_SUBSTRINGS: tuple[str, ...] = (
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
    "c:/",
)

PRIMARY_KEY_BY_TABLE: dict[str, str] = {
    "sessions": "session_key",
    "invocations": "invocation_key",
    "tool_calls": "tool_call_key",
    "usage": "usage_key",
    "outcomes": "outcome_key",
    "coverage": "coverage_key",
}


def canonical_json(value: Any) -> str:
    """Serialize value to canonical compact sorted-key UTF-8 JSON."""
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def sha256_hex(data: bytes) -> str:
    """Return 64-character lowercase hex SHA-256 of bytes."""
    return hashlib.sha256(data).hexdigest()


def canonical_table_jsonl_bytes(rows: list[dict[str, Any]]) -> bytes:
    """Serialize table rows into canonical UTF-8 JSONL bytes."""
    if not rows:
        return b""
    lines = [canonical_json(row) for row in rows]
    return ("\n".join(lines) + "\n").encode("utf-8")


def compute_manifest_digest(manifest: dict[str, Any]) -> str:
    """Compute SHA-256 hex digest of manifest without the manifest_digest field."""
    without_digest = {k: v for k, v in manifest.items() if k != "manifest_digest"}
    return sha256_hex(canonical_json(without_digest).encode("utf-8"))


def verify_and_load_schemas(base_dir: pathlib.Path) -> tuple[dict[str, dict[str, Any]], str]:
    """Validate all 6 table schemas and return (schemas_by_table, schema_digest)."""
    tables_dir = base_dir / "tables"
    schemas: dict[str, dict[str, Any]] = {}
    digest_lines: list[str] = []

    for table in CANONICAL_TABLES:
        schema_path = tables_dir / f"{table}.schema.json"
        if not schema_path.is_file():
            raise AssertionError(f"Missing normative schema file: {schema_path}")
        schema = json.loads(schema_path.read_text(encoding="utf-8"))
        if schema.get("x_contract") != CONTRACT_ID:
            raise AssertionError(f"{table}: x_contract != {CONTRACT_ID}")
        if schema.get("x_version") != CONTRACT_VERSION:
            raise AssertionError(f"{table}: x_version != {CONTRACT_VERSION}")
        if schema.get("x_table") != table:
            raise AssertionError(f"{table}: x_table != {table}")
        if schema.get("additionalProperties") is not False:
            raise AssertionError(f"{table}: additionalProperties must be false")
        if schema.get("x_primary_key") != PRIMARY_KEY_BY_TABLE[table]:
            raise AssertionError(f"{table}: x_primary_key mismatch")

        props = schema.get("properties", {})
        required = schema.get("required", [])
        if set(props.keys()) != set(required) or len(props) != len(required):
            raise AssertionError(f"{table}: required list must match all properties exactly")

        seen_field_ids: set[int] = set()
        for col_name, col_spec in props.items():
            just = col_spec.get("x_justification")
            if not isinstance(just, dict):
                raise AssertionError(f"{table}.{col_name}: missing x_justification")
            for req_j in (
                "why_present",
                "consumers",
                "zero_content",
                "pseudonymization",
                "missingness_semantics",
            ):
                if not just.get(req_j):
                    raise AssertionError(f"{table}.{col_name}: empty x_justification.{req_j}")

            pq = col_spec.get("x_parquet")
            if not isinstance(pq, dict):
                raise AssertionError(f"{table}.{col_name}: missing x_parquet")
            fid = pq.get("field_id")
            if not isinstance(fid, int) or fid < 1 or fid > len(props):
                raise AssertionError(f"{table}.{col_name}: invalid field_id {fid}")
            if fid in seen_field_ids:
                raise AssertionError(f"{table}.{col_name}: duplicate field_id {fid}")
            seen_field_ids.add(fid)

            col_type = col_spec.get("type")
            is_nullable = isinstance(col_type, list) and "null" in col_type
            expected_rep = "OPTIONAL" if is_nullable else "REQUIRED"
            if pq.get("repetition") != expected_rep:
                raise AssertionError(
                    f"{table}.{col_name}: repetition {pq.get('repetition')} != {expected_rep}"
                )

        if seen_field_ids != set(range(1, len(props) + 1)):
            raise AssertionError(f"{table}: non-contiguous Parquet field_ids")

        schemas[table] = schema
        table_sha = sha256_hex(canonical_json(schema).encode("utf-8"))
        digest_lines.append(f"{table_sha}  tables/{table}.schema.json\n")

    schema_digest = sha256_hex("".join(digest_lines).encode("utf-8"))
    return schemas, schema_digest


def is_int(val: Any) -> bool:
    """Return True if val is an integer and not a boolean."""
    return isinstance(val, int) and not isinstance(val, bool)


def check_zero_content_string(val: str) -> bool:
    """Return True if string passes DE-004 zero-content checks."""
    lower = val.lower()
    for forbidden in FORBIDDEN_SUBSTRINGS:
        if forbidden in lower:
            return False
    return True


def parse_usd_micros(val: str) -> int | None:
    """Parse fixed 6-decimal USD string into integer micro-dollars."""
    m = USD_DECIMAL_RE.match(val)
    if not m:
        return None
    whole = int(m.group(1))
    frac = int(m.group(2))
    return whole * 1_000_000 + frac


def validate_row_against_schema(
    table: str,
    row: Any,
    schema: dict[str, Any],
    expected_key_id: str,
) -> bool:
    """Validate a single row against its JSON Schema, zero-content rules, and x_invariants."""
    if not isinstance(row, dict):
        return False
    props = schema["properties"]
    if set(row.keys()) != set(props.keys()):
        return False

    for col_name, col_spec in props.items():
        val = row[col_name]
        if isinstance(val, str) and not check_zero_content_string(val):
            return False

        col_type = col_spec["type"]
        allowed_types = col_type if isinstance(col_type, list) else [col_type]
        if val is None:
            if "null" not in allowed_types:
                return False
            continue

        if "string" in allowed_types and isinstance(val, str):
            if "enum" in col_spec and val not in col_spec["enum"]:
                return False
            if "minLength" in col_spec and len(val) < col_spec["minLength"]:
                return False
            if "maxLength" in col_spec and len(val) > col_spec["maxLength"]:
                return False
            if "pattern" in col_spec and not re.match(col_spec["pattern"], val):
                return False
        elif "integer" in allowed_types and is_int(val):
            if "minimum" in col_spec and val < col_spec["minimum"]:
                return False
        elif "boolean" in allowed_types and isinstance(val, bool):
            pass
        elif "array" in allowed_types and isinstance(val, list):
            if "minItems" in col_spec and len(val) < col_spec["minItems"]:
                return False
            if col_spec.get("uniqueItems") and len(set(val)) != len(val):
                return False
            item_spec = col_spec.get("items", {})
            for item in val:
                if not isinstance(item, str) or not check_zero_content_string(item):
                    return False
                if "enum" in item_spec and item not in item_spec["enum"]:
                    return False
        else:
            return False

    if row.get("pseudonym_key_id") != expected_key_id:
        return False

    if table == "sessions":
        return _validate_sessions_invariants(row)
    if table == "invocations":
        return _validate_invocations_invariants(row)
    if table == "tool_calls":
        return _validate_tool_calls_invariants(row)
    if table == "usage":
        return _validate_usage_invariants(row)
    if table == "outcomes":
        return _validate_outcomes_invariants(row)
    if table == "coverage":
        return _validate_coverage_invariants(row)
    return False


def _validate_sessions_invariants(r: dict[str, Any]) -> bool:
    link_id = r["agent.session.link_id"]
    unlinked_reason = r["unlinked_reason"]
    if (link_id == "unlinked") != (unlinked_reason is not None):
        return False
    if link_id != "unlinked":
        if (
            r["link_epoch"] is None
            or r["origin"] != "harness"
            or r["canonical_harness"] == "unknown"
        ):
            return False
    else:
        if unlinked_reason == "invalid_key_material" and r["link_epoch"] is not None:
            return False
    if r["origin"] != "harness":
        if link_id != "unlinked" or unlinked_reason != "non_harness_origin":
            return False
    if (r["harness_version_status"] == "observed") != (r["harness_version"] is not None):
        return False
    ts = r["timestamp_status"]
    s_us, e_us = r["started_at_us"], r["ended_at_us"]
    if ts == "complete":
        if s_us is None or e_us is None or e_us < s_us:
            return False
    elif ts == "start_only":
        if s_us is None or e_us is not None:
            return False
    elif ts == "not_observable":
        if s_us is not None or e_us is not None:
            return False
    else:
        return False

    if r["is_root"]:
        if (
            r["lineage_relation"] != "root"
            or r["lineage_status"] != "root"
            or r["parent_session_key"] is not None
        ):
            return False
    if r["lineage_status"] == "evidenced":
        if (
            r["is_root"]
            or r["parent_session_key"] is None
            or r["parent_session_key"] == r["session_key"]
            or r["lineage_relation"] == "root"
        ):
            return False
    else:
        if r["parent_session_key"] is not None:
            return False
    return True


def _validate_invocations_invariants(r: dict[str, Any]) -> bool:
    grain = r["invocation_grain"]
    if grain == "call":
        if r["message_key"] is None or r["call_id_missing_reason"] is not None:
            return False
    elif grain == "usage_only":
        if r["message_key"] is not None or r["call_id_missing_reason"] is None:
            return False
    else:
        return False

    if (r["occurred_at_status"] == "observed") != (r["occurred_at_us"] is not None):
        return False

    ms = r["model_status"]
    if ms == "observed":
        if r["model"] is None or r["model_unknown_reason"] is not None:
            return False
    elif ms == "unknown":
        if r["model"] is not None or r["model_unknown_reason"] is None:
            return False
    else:
        return False
    return True


def _validate_tool_calls_invariants(r: dict[str, Any]) -> bool:
    if (r["invocation_link_status"] == "linked") != (r["invocation_key"] is not None):
        return False
    if (r["call_id_status"] == "observed") != (r["call_id_hmac"] is not None):
        return False
    if (r["fingerprint_status"] == "observed") != (r["input_fingerprint_hmac"] is not None):
        return False
    os = r["outcome_status"]
    if os in ("pending", "unknown"):
        if r["outcome_unknown_reason"] is None:
            return False
    elif os in ("succeeded", "failed", "cancelled"):
        if r["outcome_unknown_reason"] is not None:
            return False
    if (r["occurred_at_status"] == "observed") != (r["occurred_at_us"] is not None):
        return False
    if (r["duration_status"] == "observed") != (r["duration_us"] is not None):
        return False
    return True


def _validate_usage_invariants(r: dict[str, Any]) -> bool:
    if (r["invocation_link_status"] == "linked") != (r["invocation_key"] is not None):
        return False
    if (r["occurred_at_status"] == "observed") != (r["occurred_at_us"] is not None):
        return False
    ms = r["model_status"]
    if ms in ("observed", "inferred_from_session"):
        if r["model"] is None or r["model_unknown_reason"] is not None:
            return False
    elif ms == "unknown":
        if r["model"] is not None or r["model_unknown_reason"] is None:
            return False
    cs = r["counter_semantics"]
    if (cs in ("cumulative_delta", "cumulative_monotonic_skip")) != (
        r["predecessor_usage_key"] is not None
    ):
        return False
    expected_total = (
        r["input_uncached_tokens"]
        + r["output_tokens"]
        + r["cache_read_tokens"]
        + r["cache_write_tokens"]
    )
    if r["total_tokens"] != expected_total:
        return False
    if (r["reasoning_status"] == "observed") != (r["reasoning_tokens"] is not None):
        return False
    cp = r["cost_provenance"]
    if cp in ("harness_observed_exact", "provider_receipt_exact"):
        if (
            r["observed_usd"] is None
            or r["observed_usd_micros"] is None
            or r["cost_unknown_reason"] is not None
        ):
            return False
        parsed = parse_usd_micros(r["observed_usd"])
        if parsed is None or parsed != r["observed_usd_micros"]:
            return False
    elif cp == "unknown":
        if (
            r["observed_usd"] is not None
            or r["observed_usd_micros"] is not None
            or r["cost_unknown_reason"] is None
        ):
            return False
    return True


def _validate_outcomes_invariants(r: dict[str, Any]) -> bool:
    if (r["remote_status"] == "observed") != (r["repo_remote_hmac"] is not None):
        return False
    if (r["branch_status"] == "named_branch") != (r["branch_name_hmac"] is not None):
        return False
    if (r["head_commit_status"] == "observed") != (r["head_commit_hmac"] is not None):
        return False
    if (r["base_commit_status"] == "observed") != (r["base_commit_hmac"] is not None):
        return False

    es = r["evidence_status"]
    all_unobserved = (
        r["remote_status"] == "not_observed"
        and r["branch_status"] == "not_observed"
        and r["head_commit_status"] == "not_observed"
        and r["base_commit_status"] == "not_observed"
    )
    all_full = (
        r["remote_status"] == "observed"
        and r["branch_status"] == "named_branch"
        and r["head_commit_status"] == "observed"
        and r["base_commit_status"] == "observed"
    )
    if es == "missing":
        if r["missing_reason"] is None or not all_unobserved:
            return False
    elif es == "observed_full":
        if r["missing_reason"] is not None or not all_full:
            return False
    elif es == "observed_partial":
        if r["missing_reason"] is not None or all_unobserved or all_full:
            return False
    if (r["observed_at_status"] == "observed") != (r["observed_at_us"] is not None):
        return False
    return True


def _validate_coverage_invariants(r: dict[str, Any]) -> bool:
    if (
        r["encountered_count"]
        != r["accepted_count"] + r["rejected_count"] + r["recognized_unprojected_count"]
    ):
        return False
    if r["expected_population_count"] != r["observable_count"] + r["unknown_or_missing_count"]:
        return False

    cs = r["coverage_status"]
    exp_pop = r["expected_population_count"]
    obs = r["observable_count"]
    unk = r["unknown_or_missing_count"]
    rej = r["rejected_count"]
    unproj = r["recognized_unprojected_count"]
    disc = r["discovery_status"]
    proj = r["projection_status"]

    is_complete = exp_pop > 0 and obs == exp_pop and unk == 0 and rej == 0 and unproj == 0
    is_partial = exp_pop > 0 and obs > 0 and (unk > 0 or rej > 0 or unproj > 0)
    is_empty = (
        exp_pop == 0
        and disc in ("discovered", "empty_root")
        and proj != "connector_unimplemented"
    )
    is_unavail = obs == 0 and (
        exp_pop > 0
        or proj == "connector_unimplemented"
        or disc in ("missing_root", "permission_denied", "not_configured")
    )

    if cs == "complete" and not is_complete:
        return False
    if cs == "partial" and not is_partial:
        return False
    if cs == "empty" and not is_empty:
        return False
    if cs == "unavailable" and not is_unavail:
        return False

    gaps = r["gap_reasons"]
    if cs in ("complete", "empty"):
        if gaps != ["none"]:
            return False
    else:
        if not gaps or "none" in gaps:
            return False
    return True


def validate_manifest_structure(manifest: Any) -> bool:
    """Validate structural and digest invariants of MANIFEST.json."""
    if not isinstance(manifest, dict):
        return False
    required_keys = {
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
    }
    if set(manifest.keys()) != required_keys:
        return False
    if manifest["contract_id"] != CONTRACT_ID or manifest["contract_version"] != CONTRACT_VERSION:
        return False
    if not isinstance(manifest["schema_digest"], str) or not HEX64_RE.match(
        manifest["schema_digest"]
    ):
        return False
    if not isinstance(manifest["tenant_placeholder"], str) or not TENANT_RE.match(
        manifest["tenant_placeholder"]
    ):
        return False
    if not isinstance(manifest["stream"], str) or not STREAM_RE.match(manifest["stream"]):
        return False
    if not is_int(manifest["snapshot_sequence"]) or manifest["snapshot_sequence"] < 1:
        return False
    if not is_int(manifest["published_at_us"]) or manifest["published_at_us"] < 1:
        return False

    prod = manifest["producer"]
    if not isinstance(prod, dict) or set(prod.keys()) != {
        "producer_id",
        "producer_kind",
        "producer_version",
        "pseudonym_key_id",
    }:
        return False
    if not isinstance(prod["producer_id"], str) or not HEX64_RE.match(prod["producer_id"]):
        return False
    if prod["producer_kind"] not in (
        "dendro_cli",
        "agent_otel_daemon",
        "dendro_station_exporter",
    ):
        return False
    if not isinstance(prod["producer_version"], str) or not SEMVER_RE.match(
        prod["producer_version"]
    ):
        return False
    if not isinstance(prod["pseudonym_key_id"], str) or not KEY_ID_RE.match(
        prod["pseudonym_key_id"]
    ):
        return False

    scope = manifest["replacement_scope"]
    if not isinstance(scope, dict) or set(scope.keys()) != {
        "scope_mode",
        "operation",
        "source_id",
        "source_generation",
        "supersedes_generations",
        "session_keys",
        "reason",
    }:
        return False
    mode = scope["scope_mode"]
    op = scope["operation"]
    if mode not in ("source_generation", "session_keys", "full_stream"):
        return False
    if op not in ("upsert_replace", "purge_delete"):
        return False
    if scope["reason"] not in (
        "initial_export",
        "in_generation_outcome_or_attribution_update",
        "source_generation_advance",
        "retention_or_redaction_purge",
        "full_stream_rebaseline",
    ):
        return False
    if not isinstance(scope["supersedes_generations"], list) or any(
        not is_int(g) or g < 1 for g in scope["supersedes_generations"]
    ):
        return False
    if not isinstance(scope["session_keys"], list) or any(
        not isinstance(sk, str) or not HEX64_RE.match(sk) for sk in scope["session_keys"]
    ):
        return False

    if mode in ("source_generation", "session_keys"):
        if not isinstance(scope["source_id"], str) or not HEX64_RE.match(scope["source_id"]):
            return False
        if not is_int(scope["source_generation"]) or scope["source_generation"] < 1:
            return False
    else:
        if scope["source_id"] is not None or scope["source_generation"] is not None:
            return False

    if mode == "session_keys":
        if len(scope["session_keys"]) < 1:
            return False
    else:
        if len(scope["session_keys"]) != 0:
            return False

    files = manifest["files"]
    if not isinstance(files, list) or len(files) != len(CANONICAL_TABLES):
        return False
    for idx, expected_table in enumerate(CANONICAL_TABLES):
        f_desc = files[idx]
        if not isinstance(f_desc, dict) or set(f_desc.keys()) != {
            "table",
            "path",
            "format",
            "row_count",
            "byte_length",
            "sha256",
        }:
            return False
        if f_desc["table"] != expected_table:
            return False
        if f_desc["format"] not in ("parquet", "canonical_json_rows"):
            return False
        if not is_int(f_desc["row_count"]) or f_desc["row_count"] < 0:
            return False
        if not is_int(f_desc["byte_length"]) or f_desc["byte_length"] < 0:
            return False
        if not isinstance(f_desc["sha256"], str) or not HEX64_RE.match(f_desc["sha256"]):
            return False

    if manifest["manifest_digest"] != compute_manifest_digest(manifest):
        return False
    return True


class ReplayStateMachine:
    """Deterministic implementation of the SNAPSHOT.md import state machine."""

    def __init__(
        self,
        schemas: dict[str, dict[str, Any]],
        expected_schema_digest: str,
        expected_tenant: str,
        expected_producer_id: str,
        expected_stream: str,
    ) -> None:
        self.schemas = schemas
        self.expected_schema_digest = expected_schema_digest
        self.expected_tenant = expected_tenant
        self.expected_producer_id = expected_producer_id
        self.expected_stream = expected_stream

        self.high_water_sequence: int = 0
        self.committed_manifests: dict[int, str] = {}
        self.active_source_generations: dict[str, int] = {}
        self.purged_source_generations: dict[str, int] = {}
        self.tombstoned_session_keys: dict[str, int] = {}
        self.tables: dict[str, dict[str, dict[str, Any]]] = {t: {} for t in CANONICAL_TABLES}

    def active_row_counts(self) -> dict[str, int]:
        return {t: len(self.tables[t]) for t in CANONICAL_TABLES}

    def export_sorted_tables(self) -> dict[str, list[dict[str, Any]]]:
        return {
            t: [self.tables[t][pk] for pk in sorted(self.tables[t].keys())]
            for t in CANONICAL_TABLES
        }

    def apply_step(self, step: dict[str, Any]) -> str:
        manifest = step.get("committed_manifest")
        staged = step.get("staged_tables", {})

        # Step 1: Atomic visibility (no manifest => uncommitted partial publication)
        if manifest is None:
            return "IGNORED_UNCOMMITTED_NO_MANIFEST"

        # Step 2: Producer & tenant authorization (EN3a)
        if (
            manifest.get("tenant_placeholder") != self.expected_tenant
            or manifest.get("producer", {}).get("producer_id") != self.expected_producer_id
            or manifest.get("stream") != self.expected_stream
        ):
            return "REJECTED_TENANT_OR_PRODUCER_MISMATCH"

        # Step 3: Manifest structure & digest verification
        if not validate_manifest_structure(manifest):
            return "REJECTED_INVALID_MANIFEST"

        # Step 4: Schema digest verification (EN3c)
        if manifest["schema_digest"] != self.expected_schema_digest:
            return "REJECTED_SCHEMA_MISMATCH"

        # Step 5: Sequence idempotency, conflict, stale & out-of-order checks
        seq = manifest["snapshot_sequence"]
        m_digest = manifest["manifest_digest"]
        if seq in self.committed_manifests:
            if self.committed_manifests[seq] == m_digest:
                return "DUPLICATE_NOOP"
            if seq < self.high_water_sequence:
                return "REJECTED_STALE_SEQUENCE"
            return "REJECTED_CONFLICTING_DUPLICATE_SEQUENCE"
        if seq < self.high_water_sequence:
            return "REJECTED_STALE_SEQUENCE"
        if seq > self.high_water_sequence + 1:
            return "REJECTED_OUT_OF_ORDER_SEQUENCE_GAP"

        # Step 6: File presence, byte_length, sha256, and row_count integrity
        if not isinstance(staged, dict):
            return "REJECTED_FILE_INTEGRITY"
        for f_desc in manifest["files"]:
            t_name = f_desc["table"]
            rows = staged.get(t_name)
            if not isinstance(rows, list):
                return "REJECTED_FILE_INTEGRITY"
            if len(rows) != f_desc["row_count"]:
                return "REJECTED_FILE_INTEGRITY"
            payload_bytes = canonical_table_jsonl_bytes(rows)
            if len(payload_bytes) != f_desc["byte_length"]:
                return "REJECTED_FILE_INTEGRITY"
            if sha256_hex(payload_bytes) != f_desc["sha256"]:
                return "REJECTED_FILE_INTEGRITY"

        # Step 7: Row schema, zero-content (DE-004), pseudonymization (F13) & missingness (F14)
        key_id = manifest["producer"]["pseudonym_key_id"]
        for t_name in CANONICAL_TABLES:
            pk_col = PRIMARY_KEY_BY_TABLE[t_name]
            seen_pks: set[str] = set()
            for row in staged[t_name]:
                if not validate_row_against_schema(t_name, row, self.schemas[t_name], key_id):
                    return "REJECTED_ROW_SCHEMA_VIOLATION"
                pk_val = row[pk_col]
                if pk_val in seen_pks:
                    return "REJECTED_ROW_SCHEMA_VIOLATION"
                seen_pks.add(pk_val)

        # Step 8: Replacement scope, anti-resurrection & cross-table / coverage reconciliation
        scope = manifest["replacement_scope"]
        mode = scope["scope_mode"]
        op = scope["operation"]
        src_id = scope["source_id"]
        src_gen = scope["source_generation"]
        sess_keys = set(scope["session_keys"])

        if mode in ("source_generation", "session_keys"):
            assert src_id is not None and src_gen is not None
            active_gen = self.active_source_generations.get(src_id, 0)
            purged_gen = self.purged_source_generations.get(src_id, 0)
            if src_gen < active_gen or src_gen <= purged_gen:
                return "REJECTED_STALE_GENERATION_OR_TOMBSTONE"
            if op == "purge_delete" and mode == "source_generation" and src_gen <= active_gen:
                return "REJECTED_STALE_GENERATION_OR_TOMBSTONE"

        candidate_tables = copy.deepcopy(self.tables)
        candidate_active_gens = dict(self.active_source_generations)
        candidate_purged_gens = dict(self.purged_source_generations)
        candidate_tombstones = dict(self.tombstoned_session_keys)

        if mode == "source_generation":
            assert src_id is not None and src_gen is not None
            if op == "purge_delete":
                if any(len(staged[t]) != 0 for t in CANONICAL_TABLES):
                    return "REJECTED_REFERENTIAL_OR_COVERAGE_MISMATCH"
                for t in CANONICAL_TABLES:
                    candidate_tables[t] = {
                        pk: r
                        for pk, r in candidate_tables[t].items()
                        if r["source_id"] != src_id or r["source_generation"] > src_gen
                    }
                candidate_purged_gens[src_id] = max(candidate_purged_gens.get(src_id, 0), src_gen)
                candidate_active_gens[src_id] = max(candidate_active_gens.get(src_id, 0), src_gen)
            else:
                for t in CANONICAL_TABLES:
                    for r in staged[t]:
                        if r["source_id"] != src_id or r["source_generation"] != src_gen:
                            return "REJECTED_REFERENTIAL_OR_COVERAGE_MISMATCH"
                        if t != "coverage":
                            tomb_key = f"{src_id}:{r['session_key']}"
                            if src_gen <= candidate_tombstones.get(tomb_key, 0):
                                return "REJECTED_STALE_GENERATION_OR_TOMBSTONE"
                    candidate_tables[t] = {
                        pk: r
                        for pk, r in candidate_tables[t].items()
                        if r["source_id"] != src_id
                    }
                    pk_col = PRIMARY_KEY_BY_TABLE[t]
                    for r in staged[t]:
                        candidate_tables[t][r[pk_col]] = r
                candidate_active_gens[src_id] = src_gen

        elif mode == "session_keys":
            assert src_id is not None and src_gen is not None
            if op == "purge_delete":
                for t in ("sessions", "invocations", "tool_calls", "usage", "outcomes"):
                    if len(staged[t]) != 0:
                        return "REJECTED_REFERENTIAL_OR_COVERAGE_MISMATCH"
                    candidate_tables[t] = {
                        pk: r
                        for pk, r in candidate_tables[t].items()
                        if not (r["source_id"] == src_id and r["session_key"] in sess_keys)
                    }
                for sk in sess_keys:
                    tomb_key = f"{src_id}:{sk}"
                    candidate_tombstones[tomb_key] = max(
                        candidate_tombstones.get(tomb_key, 0), src_gen
                    )
                for t in ("sessions", "invocations", "tool_calls", "usage", "outcomes"):
                    for r in candidate_tables[t].values():
                        if r["source_id"] == src_id:
                            r["source_generation"] = src_gen
                for r in staged["coverage"]:
                    if r["source_id"] != src_id or r["source_generation"] != src_gen:
                        return "REJECTED_REFERENTIAL_OR_COVERAGE_MISMATCH"
                candidate_tables["coverage"] = {
                    pk: r
                    for pk, r in candidate_tables["coverage"].items()
                    if r["source_id"] != src_id
                }
                for r in staged["coverage"]:
                    candidate_tables["coverage"][r["coverage_key"]] = r
                candidate_active_gens[src_id] = src_gen
            else:
                for sk in sess_keys:
                    tomb_key = f"{src_id}:{sk}"
                    if src_gen <= candidate_tombstones.get(tomb_key, 0):
                        return "REJECTED_STALE_GENERATION_OR_TOMBSTONE"
                for t in ("sessions", "invocations", "tool_calls", "usage", "outcomes"):
                    for r in staged[t]:
                        if (
                            r["source_id"] != src_id
                            or r["source_generation"] != src_gen
                            or r["session_key"] not in sess_keys
                        ):
                            return "REJECTED_REFERENTIAL_OR_COVERAGE_MISMATCH"
                    candidate_tables[t] = {
                        pk: r
                        for pk, r in candidate_tables[t].items()
                        if not (r["source_id"] == src_id and r["session_key"] in sess_keys)
                    }
                    pk_col = PRIMARY_KEY_BY_TABLE[t]
                    for r in staged[t]:
                        candidate_tables[t][r[pk_col]] = r
                for r in staged["coverage"]:
                    if r["source_id"] != src_id or r["source_generation"] != src_gen:
                        return "REJECTED_REFERENTIAL_OR_COVERAGE_MISMATCH"
                    candidate_tables["coverage"][r["coverage_key"]] = r
                candidate_active_gens[src_id] = src_gen

        elif mode == "full_stream":
            candidate_tables = {t: {} for t in CANONICAL_TABLES}
            for t in CANONICAL_TABLES:
                pk_col = PRIMARY_KEY_BY_TABLE[t]
                for r in staged[t]:
                    candidate_tables[t][r[pk_col]] = r
                    candidate_active_gens[r["source_id"]] = max(
                        candidate_active_gens.get(r["source_id"], 0),
                        r["source_generation"],
                    )

        if not _verify_cross_table_and_coverage(candidate_tables):
            return "REJECTED_REFERENTIAL_OR_COVERAGE_MISMATCH"

        # Step 9: Commit state atomically
        self.tables = candidate_tables
        self.active_source_generations = candidate_active_gens
        self.purged_source_generations = candidate_purged_gens
        self.tombstoned_session_keys = candidate_tombstones
        self.high_water_sequence = seq
        self.committed_manifests[seq] = m_digest
        return "APPLIED"


def _verify_cross_table_and_coverage(
    tables: dict[str, dict[str, dict[str, Any]]],
) -> bool:
    """Verify foreign keys, F9 invocation uniqueness, and coverage denominators."""
    sessions = tables["sessions"]
    invocations = tables["invocations"]
    tool_calls = tables["tool_calls"]
    usage = tables["usage"]
    outcomes = tables["outcomes"]
    coverage = tables["coverage"]

    seen_i01_tuples: set[tuple[str, int, str, str | None]] = set()
    for inv in invocations.values():
        sk = inv["session_key"]
        parent = sessions.get(sk)
        if parent is None:
            return False
        if (
            inv["source_id"] != parent["source_id"]
            or inv["source_generation"] != parent["source_generation"]
            or inv["canonical_harness"] != parent["canonical_harness"]
        ):
            return False
        i01_tuple = (
            sk,
            inv["sequence_no"],
            inv["invocation_grain"],
            inv["message_key"],
        )
        if i01_tuple in seen_i01_tuples:
            return False
        seen_i01_tuples.add(i01_tuple)

    for tc in tool_calls.values():
        sk = tc["session_key"]
        parent = sessions.get(sk)
        if parent is None:
            return False
        if (
            tc["source_id"] != parent["source_id"]
            or tc["source_generation"] != parent["source_generation"]
            or tc["canonical_harness"] != parent["canonical_harness"]
        ):
            return False
        if tc["invocation_link_status"] == "linked":
            inv_row = invocations.get(tc["invocation_key"])
            if inv_row is None or inv_row["session_key"] != sk:
                return False

    for usg in usage.values():
        sk = usg["session_key"]
        parent = sessions.get(sk)
        if parent is None:
            return False
        if (
            usg["source_id"] != parent["source_id"]
            or usg["source_generation"] != parent["source_generation"]
            or usg["canonical_harness"] != parent["canonical_harness"]
        ):
            return False
        if usg["invocation_link_status"] == "linked":
            inv_row = invocations.get(usg["invocation_key"])
            if inv_row is None or inv_row["session_key"] != sk:
                return False

    for out in outcomes.values():
        sk = out["session_key"]
        parent = sessions.get(sk)
        if parent is None:
            return False
        if (
            out["source_id"] != parent["source_id"]
            or out["source_generation"] != parent["source_generation"]
            or out["canonical_harness"] != parent["canonical_harness"]
        ):
            return False

    seen_cov_tuples: set[tuple[str, int, str, str, str]] = set()
    for cov in coverage.values():
        cov_tuple = (
            cov["source_id"],
            cov["source_generation"],
            cov["canonical_harness"],
            cov["target_table"],
            cov["coverage_dimension"],
        )
        if cov_tuple in seen_cov_tuples:
            return False
        seen_cov_tuples.add(cov_tuple)

        t_name = cov["target_table"]
        target_rows = [
            r
            for r in tables[t_name].values()
            if r["source_id"] == cov["source_id"]
            and r["source_generation"] == cov["source_generation"]
            and r["canonical_harness"] == cov["canonical_harness"]
        ]
        if cov["accepted_count"] != len(target_rows):
            return False

        dim = cov["coverage_dimension"]
        if dim == "row_ingestion":
            if cov["expected_population_count"] != cov["encountered_count"]:
                return False
            if cov["observable_count"] != cov["accepted_count"]:
                return False
            if (
                cov["unknown_or_missing_count"]
                != cov["rejected_count"] + cov["recognized_unprojected_count"]
            ):
                return False
        else:
            if cov["expected_population_count"] != len(target_rows):
                return False
            if dim == "session_link_id":
                if t_name != "sessions":
                    return False
                obs = sum(1 for r in target_rows if r["agent.session.link_id"] != "unlinked")
            elif dim == "timestamp_fidelity":
                if t_name != "sessions":
                    return False
                obs = sum(1 for r in target_rows if r["timestamp_status"] == "complete")
            elif dim == "model_attribution":
                if t_name not in ("invocations", "usage"):
                    return False
                obs = sum(
                    1
                    for r in target_rows
                    if r["model_status"] in ("observed", "inferred_from_session")
                )
            elif dim == "observed_cost_usd":
                if t_name != "usage":
                    return False
                obs = sum(
                    1
                    for r in target_rows
                    if r["cost_provenance"]
                    in ("harness_observed_exact", "provider_receipt_exact")
                )
            elif dim == "tool_outcome_pairing":
                if t_name != "tool_calls":
                    return False
                obs = sum(
                    1
                    for r in target_rows
                    if r["outcome_status"] in ("succeeded", "failed", "cancelled")
                )
            elif dim == "git_outcome_evidence":
                if t_name != "outcomes":
                    return False
                obs = sum(
                    1
                    for r in target_rows
                    if r["evidence_status"] in ("observed_full", "observed_partial")
                )
            else:
                return False

            if cov["observable_count"] != obs:
                return False
            if cov["unknown_or_missing_count"] != len(target_rows) - obs:
                return False

    return True


def verify_vector_files(
    base_dir: pathlib.Path,
    schemas: dict[str, dict[str, Any]],
    schema_digest: str,
) -> tuple[int, int]:
    """Verify all 7 replay vector files and return (num_files, total_steps)."""
    vectors_dir = base_dir / "vectors"
    total_steps = 0

    for filename in EXPECTED_VECTOR_FILES:
        vec_path = vectors_dir / filename
        if not vec_path.is_file():
            raise AssertionError(f"Missing vector file: {vec_path}")
        doc = json.loads(vec_path.read_text(encoding="utf-8"))
        if doc.get("contract_id") != CONTRACT_ID or doc.get("contract_version") != CONTRACT_VERSION:
            raise AssertionError(f"{filename}: invalid contract header")
        if doc.get("vector_file") != filename:
            raise AssertionError(f"{filename}: vector_file field mismatch")
        if doc.get("expected_schema_digest") != schema_digest:
            raise AssertionError(
                f"{filename}: expected_schema_digest {doc.get('expected_schema_digest')} != {schema_digest}"
            )

        sm = ReplayStateMachine(
            schemas=schemas,
            expected_schema_digest=schema_digest,
            expected_tenant=doc["expected_tenant_placeholder"],
            expected_producer_id=doc["expected_producer_id"],
            expected_stream=doc["expected_stream"],
        )

        steps = doc.get("steps", [])
        if not steps:
            raise AssertionError(f"{filename}: empty steps")

        for step in steps:
            total_steps += 1
            step_id = step["step_id"]
            actual_status = sm.apply_step(step)
            exp_res = step["expected_step_result"]
            if actual_status != exp_res["status"]:
                raise AssertionError(
                    f"{filename}:{step_id}: status {actual_status} != expected {exp_res['status']}"
                )
            if sm.high_water_sequence != exp_res["high_water_sequence"]:
                raise AssertionError(
                    f"{filename}:{step_id}: high_water_sequence {sm.high_water_sequence} != {exp_res['high_water_sequence']}"
                )
            if sm.active_row_counts() != exp_res["active_row_counts"]:
                raise AssertionError(
                    f"{filename}:{step_id}: active_row_counts {sm.active_row_counts()} != {exp_res['active_row_counts']}"
                )

        final = doc["expected_final_state"]
        if sm.high_water_sequence != final["high_water_sequence"]:
            raise AssertionError(f"{filename}: final high_water_sequence mismatch")
        if sorted(sm.committed_manifests.keys()) != final["committed_sequences"]:
            raise AssertionError(f"{filename}: final committed_sequences mismatch")
        if sm.active_source_generations != final["active_source_generations"]:
            raise AssertionError(f"{filename}: final active_source_generations mismatch")
        if sm.purged_source_generations != final["purged_source_generations"]:
            raise AssertionError(f"{filename}: final purged_source_generations mismatch")
        if sm.tombstoned_session_keys != final["tombstoned_session_keys"]:
            raise AssertionError(f"{filename}: final tombstoned_session_keys mismatch")
        if sm.export_sorted_tables() != final["active_tables"]:
            raise AssertionError(f"{filename}: final active_tables mismatch")

    return len(EXPECTED_VECTOR_FILES), total_steps


def verify_digests(base_dir: pathlib.Path) -> str:
    """Verify vectors/SHA256SUMS and CONTRACT-DIGEST bundle SHA-256."""
    vectors_sha_path = base_dir / "vectors" / "SHA256SUMS"
    seen_vector_files: list[str] = []
    for line in vectors_sha_path.read_text(encoding="utf-8").splitlines():
        s = line.strip()
        if not s:
            continue
        expected_sha, filename = s.split("  ", 1)
        actual_sha = sha256_hex((base_dir / "vectors" / filename).read_bytes())
        if actual_sha != expected_sha:
            raise AssertionError(
                f"vectors/SHA256SUMS mismatch for {filename}: {actual_sha} != {expected_sha}"
            )
        seen_vector_files.append(filename)

    if set(seen_vector_files) != set(EXPECTED_VECTOR_FILES) or len(seen_vector_files) != len(
        EXPECTED_VECTOR_FILES
    ):
        raise AssertionError("vectors/SHA256SUMS does not list the exact 7 expected vector files")

    digest_path = base_dir / "CONTRACT-DIGEST"
    file_entries: list[str] = []
    declared_bundle_sha: str | None = None
    for line in digest_path.read_text(encoding="utf-8").splitlines():
        s = line.strip()
        if not s:
            continue
        if s.startswith("# BUNDLE-SHA256:"):
            declared_bundle_sha = s.split(":", 1)[1].strip()
            continue
        if s.startswith("#"):
            continue
        expected_sha, rel_path = s.split("  ", 1)
        actual_sha = sha256_hex((base_dir / rel_path).read_bytes())
        if actual_sha != expected_sha:
            raise AssertionError(
                f"CONTRACT-DIGEST mismatch for {rel_path}: {actual_sha} != {expected_sha}"
            )
        file_entries.append(f"{expected_sha}  {rel_path}\n")

    if not file_entries or declared_bundle_sha is None:
        raise AssertionError("CONTRACT-DIGEST missing file entries or BUNDLE-SHA256 footer")

    computed_bundle_sha = sha256_hex("".join(file_entries).encode("utf-8"))
    if computed_bundle_sha != declared_bundle_sha:
        raise AssertionError(
            f"CONTRACT-DIGEST BUNDLE-SHA256 mismatch: {computed_bundle_sha} != {declared_bundle_sha}"
        )
    return computed_bundle_sha


def main() -> int:
    base_dir = pathlib.Path(__file__).resolve().parent
    schemas, schema_digest = verify_and_load_schemas(base_dir)
    num_files, total_steps = verify_vector_files(base_dir, schemas, schema_digest)
    bundle_sha = verify_digests(base_dir)
    print(
        f"OK: fact-export-v1 verified (6 table schemas, schema_digest={schema_digest[:16]}..., "
        f"{num_files} vector files, {total_steps} replay steps, SHA256SUMS, "
        f"CONTRACT-DIGEST bundle={bundle_sha})"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
