#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Informative, independent stdlib-only implementation of SNAPSHOT.md.

Checkers do not define or freeze the contract. Production transport is not replayed.
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
    "git_evidence",
    "coverage",
)


HEX64_RE = re.compile(r"^[0-9a-f]{64}$")
KEY_ID_RE = re.compile(r"^[a-z0-9][a-z0-9._-]{1,63}$")
TENANT_RE = re.compile(r"^tn_[a-z0-9][a-z0-9_-]{1,60}$")
STREAM_RE = re.compile(r"^[a-z0-9][a-z0-9._-]{1,63}$")
SEMVER_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9._-]+)?$")

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
    "git_evidence": "git_evidence_key",
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
    """Validate the seven table schemas and return (schemas_by_table, schema_digest)."""
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
            if type(fid) is not int or fid < 1:
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


        for name, spec in props.items():
            types = spec["type"] if isinstance(spec["type"], list) else [spec["type"]]
            base_type = next(t for t in types if t != "null")
            physical, logical = {
                "string": ("BYTE_ARRAY", "STRING"),
                "integer": ("INT64", "TIMESTAMP(MICROS, true)" if name.endswith("_at_us") else "INT(64, isSigned=true)"),
                "boolean": ("BOOLEAN", "NONE"),
                "array": ("GROUP", "LIST<STRING>"),
            }[base_type]
            assert spec["x_parquet"]["physical_type"] == physical, (table, name, "physical")
            assert spec["x_parquet"]["logical_type"] == logical, (table, name, "logical")
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
    lower = val.translate(str.maketrans("ABCDEFGHIJKLMNOPQRSTUVWXYZ","abcdefghijklmnopqrstuvwxyz"))
    for forbidden in FORBIDDEN_SUBSTRINGS:
        if forbidden in lower:
            return False
    return True


def _field_valid(value,rule):
    if "enum" in rule and value not in rule["enum"]: return False
    if "anyOf" in rule and not any(_field_valid(value,r) for r in rule["anyOf"]): return False
    types=rule.get("type")
    if types is not None:
        types=types if isinstance(types,list) else [types]
        kind=("null" if value is None else "boolean" if isinstance(value,bool) else "integer" if is_int(value) else
              "string" if isinstance(value,str) else "array" if isinstance(value,list) else "invalid")
        if kind not in types: return False
    if value is None: return True
    if isinstance(value,str):
        if not check_zero_content_string(value): return False
        if not rule.get("minLength",0)<=len(value)<=rule.get("maxLength",sys.maxsize): return False
        if "pattern" in rule and re.fullmatch(rule["pattern"],value) is None: return False
    elif is_int(value):
        if not -(2**63)<=value<=2**63-1: return False
        if not rule.get("minimum",-(2**63))<=value<=rule.get("maximum",2**63-1): return False
    elif isinstance(value,list):
        if len(value)<rule.get("minItems",0): return False
        if rule.get("uniqueItems") and len({canonical_json(x) for x in value})!=len(value): return False
        if not all(_field_valid(x,rule.get("items",{})) for x in value): return False
    return True

def validate_row_against_schema(table,row,schema,expected_key_id):
    if not isinstance(row,dict) or set(row)!=set(schema["properties"]): return False
    if row["pseudonym_key_id"]!=expected_key_id: return False
    if not all(_field_valid(row[n],rule) for n,rule in schema["properties"].items()): return False
    validators={"sessions":_validate_sessions_invariants,"invocations":_validate_invocations_invariants,
        "tool_calls":_validate_tool_calls_invariants,"usage":_validate_usage_invariants,
        "outcomes":_validate_outcomes_invariants,"git_evidence":_validate_git_invariants,"coverage":_validate_coverage_invariants}
    return validators[table](row)

def _validate_sessions_invariants(r):
    linked=r["agent.session.link_id"]!="unlinked"
    if linked!=(r["unlinked_reason"] is None) or linked!=(r["link_epoch"] is not None): return False
    if linked and (r["origin"]!="harness" or r["canonical_harness"]=="unknown"): return False
    for name in ("org","endpoint_scope","team"):
        if (r[name+"_status"]=="observed")!=(r[name] is not None): return False
    statuses=(r["org_status"],r["endpoint_scope_status"])
    reason=("invalid_field_characters" if "invalid_field_characters" in statuses else "missing_org" if statuses[0]=="missing_org"
            else "missing_endpoint_scope" if statuses[1]=="missing_endpoint_scope" else None)
    if reason and (linked or r["unlinked_reason"]!=reason): return False
    if (r["harness_version_status"]=="observed")!=(r["harness_version"] is not None): return False
    start,end=r["started_at_us"],r["ended_at_us"]
    if r["timestamp_status"]=="complete":
        if start is None or end is None or end<start: return False
    elif r["timestamp_status"]=="start_only":
        if start is None or end is not None: return False
    elif start is not None or end is not None: return False
    if r["is_root"] and (r["lineage_relation"]!="root" or r["lineage_status"]!="root" or r["parent_session_key"] is not None): return False
    if r["lineage_status"]=="evidenced":
        return not r["is_root"] and r["parent_session_key"] not in (None,r["session_key"]) and r["lineage_relation"]!="root"
    return r["parent_session_key"] is None

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
    if ms in ("observed", "inferred_from_session"):
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
    aggregate=r["usage_grain"]=="session_aggregate"
    if aggregate!=(r["counter_semantics"]=="session_aggregate"): return False
    if aggregate!=(r["invocation_link_status"]=="session_level_summary"): return False
    known=r["cost_provenance"]!="unknown"
    return known==(r["observed_usd_micros"] is not None) and known==(r["cost_unknown_reason"] is None)

def _validate_git_invariants(r):
    for status,value in (("merge_status","is_merge"),("default_branch_status","on_default_branch"),("observed_at_status","observed_at_us")):
        if (r[status]=="observed")!=(r[value] is not None): return False
    if r["is_merge"] is not None and r["is_merge"]!=(len(r["parent_hmacs"])>1): return False
    return (r["session_match_status"]=="matched")==(r["session_key"] is not None)


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



def lp(*parts):
    return b"".join(len(p.encode("utf-8")).to_bytes(4,"big")+p.encode("utf-8") for p in parts)

def coverage_key(r):
    return sha256_hex(lp("fact-export.v1/coverage",r["source_id"],str(r["source_generation"]),r["canonical_harness"],r["target_table"],r["coverage_dimension"]))

def round_usd_micros(value):
    """Exact binary64 rational scaling, followed by ties-to-even."""
    import math
    if not math.isfinite(value) or value<0: return None
    n,d=value.as_integer_ratio()
    q,rem=divmod(n*1_000_000,d)
    if rem*2>d or rem*2==d and q%2: q+=1
    return q if q<=2**63-1 else None

def validate_manifest_structure(m,*,vector_mode=False):
    if not isinstance(m,dict) or set(m)!={"contract_id","contract_version","schema_digest","producer","tenant_placeholder","stream","snapshot_sequence","published_at_us","replacement_scope","files","manifest_digest","skip_reason"}: return False
    if m["contract_id"]!=CONTRACT_ID or not is_int(m["contract_version"]) or m["contract_version"]!=1: return False
    def pattern(p,v): return isinstance(v,str) and re.fullmatch(p,v) is not None
    if not all(pattern(HEX64_RE,x) for x in (m["schema_digest"],m["manifest_digest"])): return False
    if not pattern(TENANT_RE,m["tenant_placeholder"]) or not pattern(STREAM_RE,m["stream"]): return False
    if not all(is_int(m[n]) and 1<=m[n]<=2**63-1 for n in ("snapshot_sequence","published_at_us")): return False
    p=m["producer"]
    if not isinstance(p,dict) or set(p)!={"producer_id","producer_kind","producer_version","pseudonym_key_id"}: return False
    if not pattern(HEX64_RE,p["producer_id"]) or not pattern(KEY_ID_RE,p["pseudonym_key_id"]) or not pattern(SEMVER_RE,p["producer_version"]): return False
    if p["producer_kind"] not in ("dendro_cli","agent_otel_daemon","dendro_station_exporter"): return False
    sc=m["replacement_scope"]
    if not isinstance(sc,dict) or set(sc)!={"scope_mode","operation","reason","source_id","source_generation","session_keys"}: return False
    if sc["scope_mode"] not in ("source_generation","session_keys") or sc["operation"] not in ("upsert_replace","purge_delete"): return False
    if sc["reason"] not in ("initial_export","in_generation_outcome_or_attribution_update","source_generation_advance","retention_or_redaction_purge","key_rotation"): return False
    if not pattern(HEX64_RE,sc["source_id"]) or not is_int(sc["source_generation"]) or not 1<=sc["source_generation"]<=2**63-1: return False
    keys=sc["session_keys"]
    if not isinstance(keys,list) or not all(pattern(HEX64_RE,k) for k in keys) or len(set(keys))!=len(keys): return False
    if (sc["scope_mode"]=="session_keys")!=bool(keys) or keys and sc["operation"]!="purge_delete": return False
    files=m["files"]
    if not isinstance(files,list): return False
    if m["skip_reason"] is not None:
        return m["skip_reason"] in ("abandoned_publication","source_unavailable","producer_recovery") and not files and not keys and sc["operation"]=="upsert_replace"
    if len(files)!=len(CANONICAL_TABLES): return False
    for t,f in zip(CANONICAL_TABLES,files):
        if not isinstance(f,dict) or set(f)!={"table","path","format","row_count","byte_length","sha256"}: return False
        if f["table"]!=t or f["format"] not in ("parquet","canonical_json_rows"): return False
        if f["format"]=="canonical_json_rows" and not vector_mode: return False
        ext="jsonl" if f["format"]=="canonical_json_rows" else "parquet"
        if f["path"]!=f"tables/{t}.{ext}" or not pattern(HEX64_RE,f["sha256"]): return False
        if not all(is_int(f[n]) and 0<=f[n]<=2**63-1 for n in ("row_count","byte_length")): return False
    return True

def verify_cross_table_and_coverage(tables):
    """SPEC sections 4/6, evaluated for the complete affected-source candidate."""
    sessions=tables["sessions"]
    inv=tables["invocations"]
    identities=set()
    grains={}
    for t in CANONICAL_TABLES[1:-1]:
        for r in tables[t].values():
            sk=r["session_key"]
            if sk is not None:
                parent=sessions.get(sk)
                if parent is None or any(r[n]!=parent[n] for n in ("source_id","source_generation","canonical_harness","pseudonym_key_id")): return False
            if t in ("usage","tool_calls") and r["invocation_key"] is not None:
                target=inv.get(r["invocation_key"])
                if target is None or target["session_key"]!=sk: return False
            if t=="invocations":
                ident=tuple(r[n] for n in ("session_key","sequence_no","invocation_grain","message_key"))
                if ident in identities: return False
                identities.add(ident)
            if t=="usage":
                grain=r["usage_grain"]
                prior=grains.get(sk)
                if prior and (prior!=grain or grain=="session_aggregate"): return False
                grains[sk]=grain
    populations={}
    for t in CANONICAL_TABLES[:-1]:
        for r in tables[t].values():
            key=(r["source_id"],r["source_generation"],r["canonical_harness"],t)
            populations.setdefault(key,[]).append(r)
    seen=set()
    ingestion=set()
    for c in tables["coverage"].values():
        key=(c["source_id"],c["source_generation"],c["canonical_harness"],c["target_table"])
        dimension=c["coverage_dimension"]
        if key+(dimension,) in seen or c["coverage_key"]!=coverage_key(c): return False
        seen.add(key+(dimension,))
        rows=populations.get(key,[])
        total=len(rows)
        t=c["target_table"]
        if c["accepted_count"]!=total: return False
        if dimension=="row_ingestion":
            ingestion.add(key)
            if c["expected_population_count"]!=c["encountered_count"] or c["observable_count"]!=total or c["unknown_or_missing_count"]!=c["rejected_count"]+c["recognized_unprojected_count"]: return False
            continue
        if dimension=="session_link_id" and t=="sessions": observed=sum(r["agent.session.link_id"]!="unlinked" for r in rows)
        elif dimension=="model_attribution" and t in ("invocations","usage"): observed=sum(r["model_status"]!="unknown" for r in rows)
        elif dimension=="observed_cost_usd" and t=="usage": observed=sum(r["cost_provenance"]!="unknown" for r in rows)
        elif dimension=="tool_outcome_pairing" and t=="tool_calls": observed=sum(r["outcome_status"] not in ("pending","unknown") for r in rows)
        elif dimension=="git_outcome_evidence" and t in ("outcomes","git_evidence"):
            observed=sum(r["evidence_status"]!="missing" if t=="outcomes" else r["session_match_status"]=="matched" for r in rows)
        elif dimension=="timestamp_fidelity":
            field={"sessions":"timestamp_status","invocations":"occurred_at_status","tool_calls":"occurred_at_status","usage":"occurred_at_status","outcomes":"observed_at_status","git_evidence":"observed_at_status"}[t]
            observed=sum(r[field]==("complete" if t=="sessions" else "observed") for r in rows)
        else: return False
        if c["expected_population_count"]!=total or c["observable_count"]!=observed or c["unknown_or_missing_count"]!=total-observed: return False
    return set(populations)<=ingestion

class ReplayStateMachine:
    """One isolated tenant/producer/stream state, using vector transport only."""
    def __init__(self,schemas,expected_schema_digest,expected_tenant,expected_producer_id,expected_stream):
        self.schemas=schemas
        self.expected_schema_digest=expected_schema_digest
        self.key=(expected_tenant,expected_producer_id,expected_stream)
        self.high_water_sequence=0
        self.committed_manifests={}
        self.active_source_generations={}
        self.purged_source_generations={}
        self.source_key_ids={}
        self.tombstoned_session_keys=set()
        self.tables={t:{} for t in CANONICAL_TABLES}
        self.import_coverage=[]

    def active_row_counts(self):
        return {t:len(self.tables[t]) for t in CANONICAL_TABLES}

    def export_sorted_tables(self):
        return {t:[self.tables[t][k] for k in sorted(self.tables[t])] for t in CANONICAL_TABLES}

    def _consume(self,m,status,recorded_digest=None):
        q=m["snapshot_sequence"]
        digest=recorded_digest if recorded_digest is not None else m["manifest_digest"]
        self.high_water_sequence=q
        self.committed_manifests[q]=digest
        kind="accepted" if status=="APPLIED" else "skipped" if status=="SKIPPED" else "rejected"
        self.import_coverage.append(dict(tenant_placeholder=self.key[0],producer_id=self.key[1],stream=self.key[2],
            snapshot_sequence=q,manifest_digest=digest,status=kind,
            reason=None if kind=="accepted" else m["skip_reason"] if kind=="skipped" else status))
        return status

    def apply_step(self,step):
        m=step.get("committed_manifest")
        # Replay transport: explicit UTF-8 text is authoritative when supplied;
        # otherwise the embedded object's canonical JSON represents the raw bytes.
        raw=step.get("raw_manifest_utf8")
        if raw is not None:
            try: m=json.loads(raw)
            except (ValueError,TypeError): return "IGNORED_UNPARSEABLE_MANIFEST_KEY_OR_SEQUENCE"
        elif m is None: return "IGNORED_UNCOMMITTED_NO_MANIFEST"
        else: raw=canonical_json(m)
        if not isinstance(m,dict): return "IGNORED_UNPARSEABLE_MANIFEST_KEY_OR_SEQUENCE"
        p=m.get("producer")
        p=p if isinstance(p,dict) else {}
        q=m.get("snapshot_sequence")
        if not (isinstance(m.get("tenant_placeholder"),str) and TENANT_RE.fullmatch(m["tenant_placeholder"])
                and isinstance(p.get("producer_id"),str) and HEX64_RE.fullmatch(p["producer_id"])
                and isinstance(m.get("stream"),str) and STREAM_RE.fullmatch(m["stream"])
                and is_int(q) and 1<=q<=2**63-1):
            return "IGNORED_UNPARSEABLE_MANIFEST_KEY_OR_SEQUENCE"
        if (m.get("tenant_placeholder"),p.get("producer_id"),m.get("stream"))!=self.key: return "REJECTED_TENANT_OR_PRODUCER_MISMATCH"
        shape_valid=validate_manifest_structure(m,vector_mode=True)
        declared=m.get("manifest_digest")
        if q in self.committed_manifests:
            return "DUPLICATE_IDEMPOTENT" if self.committed_manifests[q]==declared else "CONFLICTING_DUPLICATE"
        if q>self.high_water_sequence+1: return "GAP_PENDING"
        if not shape_valid:
            digest=declared if isinstance(declared,str) and HEX64_RE.fullmatch(declared) else sha256_hex(raw.encode("utf-8"))
            return self._consume(m,"REJECTED_INVALID_MANIFEST",digest)
        def reject(reason): return self._consume(m,reason)
        if compute_manifest_digest(m)!=m["manifest_digest"]: return reject("REJECTED_INVALID_MANIFEST")
        if m["schema_digest"]!=self.expected_schema_digest: return reject("REJECTED_SCHEMA_MISMATCH")
        if m["skip_reason"] is not None: return self._consume(m,"SKIPPED")
        staged=step.get("staged_tables")
        if not isinstance(staged,dict): return reject("REJECTED_FILE_INTEGRITY")
        for f in m["files"]:
            rows=staged.get(f["table"])
            if not isinstance(rows,list): return reject("REJECTED_FILE_INTEGRITY")
            data=canonical_table_jsonl_bytes(rows)
            if len(rows)!=f["row_count"] or len(data)!=f["byte_length"] or sha256_hex(data)!=f["sha256"]: return reject("REJECTED_FILE_INTEGRITY")
        key_id=m["producer"]["pseudonym_key_id"]
        for table in CANONICAL_TABLES:
            rows=staged[table]
            pk=PRIMARY_KEY_BY_TABLE[table]
            if not all(validate_row_against_schema(table,r,self.schemas[table],key_id) for r in rows): return reject("REJECTED_ROW_SCHEMA_VIOLATION")
            if len({r[pk] for r in rows})!=len(rows): return reject("REJECTED_ROW_SCHEMA_VIOLATION")
        scope=m["replacement_scope"]
        source=scope["source_id"]
        generation=scope["source_generation"]
        active=self.active_source_generations.get(source,0)
        purged=self.purged_source_generations.get(source,0)
        session_purge=scope["scope_mode"]=="session_keys"
        purge=scope["operation"]=="purge_delete"
        if (session_purge and (generation!=active or self.source_key_ids.get(source)!=key_id)
                or not session_purge and (generation<active or generation<=purged)):
            return reject("REJECTED_STALE_GENERATION_OR_TOMBSTONE")
        if not purge:
            old_key=self.source_key_ids.get(source)
            if old_key and old_key!=key_id and purged<active: return reject("REJECTED_STALE_GENERATION_OR_TOMBSTONE")
            if any(r.get("session_key") in self.tombstoned_session_keys for t in CANONICAL_TABLES for r in staged[t]):
                return reject("REJECTED_STALE_GENERATION_OR_TOMBSTONE")
        if purge and any(staged[t] for t in CANONICAL_TABLES): return reject("REJECTED_REFERENTIAL_OR_COVERAGE_MISMATCH")
        candidate=copy.deepcopy(self.tables)
        if session_purge:
            selected=set(scope["session_keys"])
            for table in CANONICAL_TABLES[:-1]:
                candidate[table]={k:r for k,r in candidate[table].items() if not (r["source_id"]==source and r.get("session_key") in selected)}
        else:
            for table in CANONICAL_TABLES:
                candidate[table]={k:r for k,r in candidate[table].items() if r["source_id"]!=source}
                if not purge:
                    pk=PRIMARY_KEY_BY_TABLE[table]
                    for row in staged[table]:
                        if row["source_id"]!=source or row["source_generation"]!=generation or row[pk] in candidate[table]:
                            return reject("REJECTED_REFERENTIAL_OR_COVERAGE_MISMATCH")
                        candidate[table][row[pk]]=copy.deepcopy(row)
            if not purge:
                focused={t:{k:r for k,r in candidate[t].items() if r["source_id"]==source} for t in CANONICAL_TABLES}
                if not verify_cross_table_and_coverage(focused): return reject("REJECTED_REFERENTIAL_OR_COVERAGE_MISMATCH")
        self.tables=candidate
        if session_purge: self.tombstoned_session_keys.update(scope["session_keys"])
        else:
            self.active_source_generations[source]=generation
            if purge: self.purged_source_generations[source]=generation
            else: self.source_key_ids[source]=key_id
        return self._consume(m,"APPLIED")

def verify_vector_files(base,schemas,schema_digest):
    import struct
    files=sorted((base/"vectors").glob("*.json"))
    assert files,"No vectors"
    count=0
    for path in files:
        doc=json.loads(path.read_text(encoding="utf-8"))
        assert doc["expected_schema_digest"]==schema_digest,path.name
        assert doc["vector_file"]==path.name and doc["contract_id"]==CONTRACT_ID and doc["contract_version"]==1
        sm=ReplayStateMachine(schemas,schema_digest,doc["expected_tenant_placeholder"],doc["expected_producer_id"],doc["expected_stream"])
        for step in doc["steps"]:
            count+=1
            got=sm.apply_step(step)
            exp=step["expected_step_result"]
            assert got==exp["status"],(path.name,step["step_id"],got,exp["status"])
            assert sm.high_water_sequence==exp["high_water_sequence"],(path.name,step["step_id"],"cursor")
            assert sm.active_row_counts()==exp["active_row_counts"],(path.name,step["step_id"],"counts")
            assert sm.import_coverage==exp["import_coverage"],(path.name,step["step_id"],"import coverage")
            assert {t:sha256_hex(canonical_table_jsonl_bytes(rows)) for t,rows in sm.export_sorted_tables().items()}==exp["active_table_digests"],(path.name,step["step_id"],"row bytes")
        final=doc["expected_final_state"]
        assert sm.export_sorted_tables()==final["active_tables"],(path.name,"active rows")
        assert sm.high_water_sequence==final["high_water_sequence"],(path.name,"final cursor")
        for name in ("active_source_generations","purged_source_generations","source_key_ids"):
            assert getattr(sm,name)==final[name],(path.name,name)
        assert sorted(sm.tombstoned_session_keys)==final["tombstoned_session_keys"],(path.name,"tombstones")
        assert sorted(sm.committed_manifests)==final["committed_sequences"],(path.name,"sequences")
        for case in doc.get("rounding_cases",[]):
            value=struct.unpack(">d",bytes.fromhex(case["binary64_hex"]))[0]
            assert round_usd_micros(value)==case["expected_micros"],case
        for step in doc["steps"]:
            manifest=step.get("committed_manifest")
            if isinstance(manifest,dict) and manifest.get("skip_reason") is None and validate_manifest_structure(manifest,vector_mode=True):
                assert not validate_manifest_structure(manifest),"Production must reject vector transport"
    # Verify actual separate states for tenant in K, not merely authorization.
    doc=json.loads((base/"vectors/clean_sequence.json").read_text())
    event=doc["steps"][0]
    first=ReplayStateMachine(schemas,schema_digest,doc["expected_tenant_placeholder"],doc["expected_producer_id"],doc["expected_stream"])
    second=ReplayStateMachine(schemas,schema_digest,"tn_other_tenant",doc["expected_producer_id"],doc["expected_stream"])
    changed=copy.deepcopy(event)
    changed["committed_manifest"]["tenant_placeholder"]="tn_other_tenant"
    changed["committed_manifest"]["manifest_digest"]=compute_manifest_digest(changed["committed_manifest"])
    assert first.apply_step(changed)=="REJECTED_TENANT_OR_PRODUCER_MISMATCH" and first.high_water_sequence==0
    assert second.apply_step(changed)=="APPLIED" and first.high_water_sequence==0
    assert first.apply_step(event)=="APPLIED"
    return len(files),count

def verify_digests(base):
    paths=sorted(["SNAPSHOT.md","tables/README.md","vectors/SHA256SUMS"]+[f"tables/{t}.schema.json" for t in CANONICAL_TABLES]+
                 [p.relative_to(base).as_posix() for p in (base/"vectors").glob("*.json")])
    raw=(base/"SHA256SUMS").read_bytes()
    lines=raw.decode("utf-8").splitlines()
    assert [line.split("  ",1)[1] for line in lines]==paths,"Exact contract inventory"
    for line in lines:
        digest,name=line.split("  ",1)
        assert sha256_hex((base/name).read_bytes())==digest,name
    result=sha256_hex(raw)
    assert (base/"CONTRACT-DIGEST").read_text().strip()==result,"Contract digest"
    expected="".join(f"{sha256_hex(p.read_bytes())}  {p.name}\n" for p in sorted((base/"vectors").glob("*.json")))
    assert (base/"vectors/SHA256SUMS").read_text()==expected
    return result

def main():
    base=pathlib.Path(__file__).resolve().parent
    schemas,schema_digest=verify_and_load_schemas(base)
    files,steps=verify_vector_files(base,schemas,schema_digest)
    digest=verify_digests(base)
    print(f"OK: 7 schemas; {files} vectors; {steps} steps; schema_digest={schema_digest}; CONTRACT-DIGEST={digest}")
    return 0

if __name__=="__main__":
    sys.exit(main())
