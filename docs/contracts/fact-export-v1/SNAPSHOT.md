# `fact-export-v1` Snapshot Manifest, Atomic Publication & Replay Contract (`C1b`)

**Contract Identifier**: `fact-export-v1`  
**Contract Home**: `smota/agent-otel-bridge` (`docs/contracts/fact-export-v1/SNAPSHOT.md`)  
**Authoritative Inputs**: `2026-11-11-mvp-plan.md` (§2 Decisions 3–5; `D1`, `D4b`, `D5`, `EN1`, `EN2`, `EN3a–c`, `EN5c`), `DE-002`, `DE-004`, `DE-005`

---

## 1. Core Principles

1. **Flat Per-Table Snapshots**: Each snapshot publishes the six normative `fact-export-v1` tables (`sessions`, `invocations`, `tool_calls`, `usage`, `outcomes`, `coverage`) alongside a single root `MANIFEST.json`.
2. **Facts Are NOT Append-Only (`MVP plan §2 Decision 3`)**: Agent sessions evolve while in flight and across re-extractions:
   - Tool calls initially captured at `PreToolUse` (`outcome_status = "pending"`, `outcome_unknown_reason = "awaiting_result"`, `duration_status = "single_point_only"`) are later paired with `PostToolUse` (`outcome_status = "succeeded" | "failed"`, `duration_status = "observed"`).
   - Model attribution (`model_status = "unknown"` $\rightarrow$ `"observed"`), cumulative token normalization (`accounting_v1`), observed USD receipts, and historical git evidence (`outcomes`) update as later events in a session arrive.
   - Retention or redaction purges (`EN5c`) must deterministically retire superseded keys and block resurrection from stale snapshots.
3. **Atomic Publication via Root Manifest**: Table files are written to an immutable staging prefix first; the snapshot becomes visible to readers and importers **if and only if** `MANIFEST.json` is atomically committed and all six table files match the manifest's `sha256`, `byte_length`, and `row_count`.
4. **Idempotent, Monotonic Import (`EN3b`)**: Import state is keyed by `(producer.producer_id, stream, snapshot_sequence)`. Re-importing an already-committed snapshot with an identical `manifest_digest` is a deterministic no-op (`DUPLICATE_NOOP`). Conflicting manifests on an existing sequence, stale sequences (`< high_water_sequence`), out-of-order sequence gaps (`> high_water_sequence + 1` when `require_contiguous_sequence` is active, or `< high_water_sequence`), and resurrected purged generations are rejected without mutating target state.

---

## 2. Normative Manifest Schema (`MANIFEST.json`)

Every committed snapshot MUST include a UTF-8 JSON manifest with the following top-level fields:

```json
{
  "contract_id": "fact-export-v1",
  "contract_version": 1,
  "files": [
    {
      "byte_length": 1024,
      "format": "parquet",
      "path": "tables/sessions.parquet",
      "row_count": 2,
      "sha256": "<64-hex-sha256>",
      "table": "sessions"
    }
  ],
  "manifest_digest": "<64-hex-sha256-of-manifest-without-manifest_digest>",
  "producer": {
    "producer_id": "<64-hex-hmac-sha256>",
    "producer_kind": "dendro_cli",
    "producer_version": "0.5.0",
    "pseudonym_key_id": "kh_customer_v1"
  },
  "published_at_us": 1762900005000000,
  "replacement_scope": {
    "operation": "upsert_replace",
    "reason": "initial_export",
    "scope_mode": "source_generation",
    "session_keys": [],
    "source_generation": 1,
    "source_id": "<64-hex-hmac-sha256>",
    "supersedes_generations": []
  },
  "schema_digest": "<64-hex-sha256-of-canonical-table-schemas>",
  "snapshot_sequence": 1,
  "stream": "station-facts-default",
  "tenant_placeholder": "tn_acme_prod"
}
```

### 2.1 Field Requirements & Invariants

| Field | Type / Constraint | Normative Rule |
| :--- | :--- | :--- |
| `contract_id` | `"fact-export-v1"` | Must equal `"fact-export-v1"`. |
| `contract_version` | `1` | Must equal integer `1`. |
| `schema_digest` | `^[0-9a-f]{64}$` | SHA-256 of the canonical schema bundle (`sessions`, `invocations`, `tool_calls`, `usage`, `outcomes`, `coverage` in fixed order). Any mismatch with the importer's pinned `schema_digest` MUST reject the snapshot with `REJECTED_SCHEMA_MISMATCH`. |
| `producer.producer_id` | `^[0-9a-f]{64}$` | Pseudonymized 64-hex HMAC-SHA256 identifier of the producer instance. |
| `producer.producer_kind` | Enum | One of `"dendro_cli"`, `"agent_otel_daemon"`, `"dendro_station_exporter"`. |
| `producer.producer_version` | String | Semantic version token (`^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9._-]+)?$`). |
| `producer.pseudonym_key_id` | `^[a-z0-9][a-z0-9._-]{1,63}$` | Active customer-held HMAC key generation ID (`DE-004`). Every row in the snapshot MUST carry a matching `pseudonym_key_id`. |
| `tenant_placeholder` | `^tn_[a-z0-9][a-z0-9_-]{1,60}$` | Tenant routing placeholder bound at upload/import time (`EN3a`). Importers MUST verify that `(tenant_placeholder, producer.producer_id)` matches the authenticated upload channel; mismatches reject with `REJECTED_TENANT_OR_PRODUCER_MISMATCH`. |
| `stream` | `^[a-z0-9][a-z0-9._-]{1,63}$` | Logical stream identifier within the producer (e.g. `"station-facts-default"`). |
| `snapshot_sequence` | Integer `>= 1` | Monotonic sequence number scoped to `(producer.producer_id, stream)`. |
| `published_at_us` | Integer `>= 1` | UTC epoch timestamp in microseconds when the manifest was finalized. |
| `replacement_scope` | Object | Defines the exact key set superseded or purged by this snapshot (see §3). |
| `files` | Array of 6 objects | Must contain exactly the 6 tables in canonical order: `["sessions", "invocations", "tool_calls", "usage", "outcomes", "coverage"]`. Each entry specifies `table`, `path`, `format` (`"parquet"` in production; `"canonical_json_rows"` in portable replay vectors), `row_count` (`>= 0`), `byte_length` (`>= 0`), and `sha256` (`^[0-9a-f]{64}$`). |
| `manifest_digest` | `^[0-9a-f]{64}$` | SHA-256 hex digest of the canonical JSON serialization (`sort_keys=True`, compact separators `,` and `:`) of the manifest object with the `manifest_digest` key removed. |

---

## 3. Replacement Scope & Non-Append-Only Correction Semantics

Facts are replaced deterministically according to `manifest.replacement_scope`:

### 3.1 `scope_mode` Variants

1. **`"source_generation"` (`source_id != null`, `source_generation != null`, `session_keys == []`)**:
   - Used when exporting or re-exporting an entire logical source root at `source_generation`.
   - When `operation == "upsert_replace"`:
     - All existing rows in all 6 tables belonging to `source_id` whose `source_generation` is `<= manifest.replacement_scope.source_generation` (or listed in `supersedes_generations`) are **deleted** from the active state and **replaced** by the rows in the incoming snapshot.
     - All rows in the incoming snapshot MUST have `row.source_id == replacement_scope.source_id` and `row.source_generation == replacement_scope.source_generation`.
   - When `operation == "purge_delete"` (`EN5c` retention/redaction purge):
     - All six incoming table files MUST have `row_count == 0`.
     - All existing rows in all 6 tables for `source_id` with `source_generation <= replacement_scope.source_generation` (and any generation in `supersedes_generations`) are permanently deleted, and `(source_id, replacement_scope.source_generation)` is recorded in the stream's `tombstoned_source_generations` watermark so no later snapshot can resurrect a generation `<=` the purged watermark.

2. **`"session_keys"` (`source_id != null`, `source_generation != null`, `len(session_keys) >= 1`)**:
   - Used for targeted in-generation replacement when one or more sessions receive late tool outcomes (`PostToolUse`), late model attribution, updated cumulative token deltas, or updated historical git evidence (`outcomes`), or when specific sessions are purged (`EN5c`).
   - When `operation == "upsert_replace"`:
     - In `sessions`, `invocations`, `tool_calls`, `usage`, and `outcomes`, all existing rows with `row.source_id == replacement_scope.source_id` and `row.session_key in replacement_scope.session_keys` (at `row.source_generation <= replacement_scope.source_generation`) are **deleted** and **replaced** by the incoming rows for those `session_keys`.
     - Incoming rows in `sessions`, `invocations`, `tool_calls`, `usage`, and `outcomes` MUST belong exclusively to `(replacement_scope.source_id, replacement_scope.source_generation)` and have `row.session_key in replacement_scope.session_keys`.
     - In `coverage`, because coverage denominators are scoped to `(source_id, source_generation, canonical_harness, target_table, coverage_dimension)`, any incoming `coverage` row replaces the existing `coverage` row with the same `coverage_key` (or `(source_id, source_generation, canonical_harness, target_table, coverage_dimension)`).
   - When `operation == "purge_delete"` (`EN5c` targeted session purge):
     - Incoming `sessions`, `invocations`, `tool_calls`, `usage`, and `outcomes` files MUST have `row_count == 0`.
     - All existing rows in those 5 tables for `replacement_scope.source_id` and `row.session_key in replacement_scope.session_keys` are deleted, and those `(source_id, session_key)` pairs are recorded in `tombstoned_session_keys` up to `replacement_scope.source_generation`.
     - Incoming `coverage` rows update the remaining active denominators for `(source_id, source_generation)`.

3. **`"full_stream"` (`source_id == null`, `source_generation == null`, `session_keys == []`)**:
   - Replaces the entire active state of `(producer.producer_id, stream)` across all sources with the incoming snapshot's rows.

### 3.2 Generation Monotonicity & Anti-Resurrection Rules
For every `source_id` tracked within `(producer.producer_id, stream)`:
- Let `active_source_generation[source_id]` be the highest `source_generation` accepted so far for `source_id`.
- Let `purged_source_generation[source_id]` be the highest `source_generation` purged via `scope_mode == "source_generation", operation == "purge_delete"`.
- If an incoming snapshot specifies a `source_generation < active_source_generation[source_id]`, or `source_generation <= purged_source_generation[source_id]`, or attempts to insert a `(source_id, session_key)` at a generation `<= tombstoned_session_keys[(source_id, session_key)]`, the snapshot MUST be rejected with `REJECTED_STALE_GENERATION_OR_TOMBSTONE`.

---

## 4. Atomic Publication Protocol

1. **Stage Table Payloads First**: Producer writes all 6 table files under an uncommitted staging location `snapshots/staging/<uuid>/tables/<table_name>.parquet`, flushing file descriptors and computing `(row_count, byte_length, sha256)` for each file.
2. **Construct & Digest `MANIFEST.json`**: Producer populates `MANIFEST.json` with the 6 file descriptors, `schema_digest`, `replacement_scope`, and `manifest_digest`.
3. **Atomic Commit**: Producer atomically renames/publishes the staged directory and writes `snapshots/seq-<snapshot_sequence>/MANIFEST.json` last (or uses an object-store conditional put `If-None-Match: *` on `MANIFEST.json`).
4. **Reader / Importer Visibility Rule**:
   - A staging directory or sequence prefix **without** a committed `MANIFEST.json` is an uncommitted partial publication (`IGNORED_UNCOMMITTED_NO_MANIFEST`). Readers and importers MUST ignore it without advancing watermarks or mutating state.
   - If `MANIFEST.json` is present, the importer MUST verify all 6 table files exist and match their declared `byte_length`, `sha256`, and `row_count` **before** applying any state transition. Any missing file, checksum mismatch, or row count mismatch aborts the import atomically with `REJECTED_FILE_INTEGRITY`.

---

## 5. Deterministic Replay & Import State Machine (`EN3b`)

For each stream key `K = (producer_id, stream)`, the importer maintains:
- `high_water_sequence`: `u64` (initially `0`)
- `committed_manifests`: `Map<u64, String>` mapping accepted `snapshot_sequence` $\rightarrow$ `manifest_digest`
- `active_source_generation`: `Map<String, u64>`
- `purged_source_generation`: `Map<String, u64>`
- `tombstoned_session_keys`: `Map<(String, String), u64>`
- `tables`: active row maps keyed by primary key for `sessions`, `invocations`, `tool_calls`, `usage`, `outcomes`, and `coverage`.

When evaluating a publication event `E`:

1. **Step 1 — Manifest Presence Check (Atomic Visibility)**:
   - If `E.committed_manifest` is `null`: return `IGNORED_UNCOMMITTED_NO_MANIFEST`. State is unchanged.
2. **Step 2 — Producer & Tenant Authorization (`EN3a`)**:
   - Verify `manifest.tenant_placeholder == expected_tenant_placeholder` and `manifest.producer.producer_id == expected_producer_id`. If mismatched, return `REJECTED_TENANT_OR_PRODUCER_MISMATCH`.
3. **Step 3 — Manifest Structural & Digest Check**:
   - Verify `contract_id == "fact-export-v1"`, `contract_version == 1`, valid regexes on identifiers, canonical table list order, and recomputed `manifest_digest`. If invalid, return `REJECTED_INVALID_MANIFEST`.
4. **Step 4 — Schema Digest Check (`EN3c`)**:
   - Verify `manifest.schema_digest == expected_schema_digest`. If mismatched, return `REJECTED_SCHEMA_MISMATCH`.
5. **Step 5 — Sequence Idempotency, Conflict, Stale & Out-of-Order Check**:
   - If `manifest.snapshot_sequence` exists in `committed_manifests`:
     - If `committed_manifests[seq] == manifest.manifest_digest`: return `DUPLICATE_NOOP` (idempotent re-import).
     - Else: return `REJECTED_CONFLICTING_DUPLICATE_SEQUENCE` (two different manifests claiming the same sequence).
   - If `manifest.snapshot_sequence < high_water_sequence`:
     - Return `REJECTED_STALE_SEQUENCE`.
   - If `manifest.snapshot_sequence > high_water_sequence + 1`:
     - Return `REJECTED_OUT_OF_ORDER_SEQUENCE_GAP` (snapshots within a `(producer_id, stream)` must be applied in contiguous monotonic sequence order `1, 2, 3, ...`).
6. **Step 6 — File Presence & SHA-256 / Byte-Length / Row-Count Integrity Check**:
   - For each of the 6 table descriptors in `manifest.files`, verify the staged table payload exists and its canonical byte length, SHA-256 digest, and row count equal `file.byte_length`, `file.sha256`, and `file.row_count`. If any check fails, return `REJECTED_FILE_INTEGRITY`.
7. **Step 7 — Row Schema, Zero-Content (`DE-004`), Pseudonymization (`F13`) & Missingness (`F14`) Validation**:
   - Validate every row in all 6 tables against its table JSON Schema, bi-directional `x_invariants`, `pseudonym_key_id == manifest.producer.pseudonym_key_id`, and zero-content sentinel checks. If any row fails, return `REJECTED_ROW_SCHEMA_VIOLATION`.
8. **Step 8 — Replacement Scope, Anti-Resurrection & Cross-Table Referential / Denominator Validation**:
   - Validate `replacement_scope` invariants and check against `active_source_generation`, `purged_source_generation`, and `tombstoned_session_keys`. If violated, return `REJECTED_STALE_GENERATION_OR_TOMBSTONE`.
   - Simulate applying the replacement scope to a clone of `tables`, then verify cross-table invariants on the resulting active state for `(source_id, source_generation)`:
     - Every `invocations.session_key`, `tool_calls.session_key`, `usage.session_key`, and `outcomes.session_key` references an active `sessions.session_key` in the same `(source_id, source_generation)` and `canonical_harness`.
     - Every non-null `tool_calls.invocation_key` and `usage.invocation_key` references an active `invocations.invocation_key` in the same `session_key`.
     - Every active `invocations` row is uniquely qualified by `(session_key, sequence_no, invocation_grain, message_key)` (`F9`).
     - Every active `coverage` row for `(source_id, source_generation, canonical_harness, target_table, coverage_dimension)` has `accepted_count` matching the active row count in `target_table` for `(source_id, source_generation, canonical_harness)`, and for field-level dimensions (`session_link_id`, `model_attribution`, `observed_cost_usd`, `tool_outcome_pairing`, `git_outcome_evidence`, `timestamp_fidelity`), `expected_population_count`, `observable_count`, and `unknown_or_missing_count` match the actual active rows in `target_table`!
     - If any cross-table or coverage denominator check fails, return `REJECTED_REFERENTIAL_OR_COVERAGE_MISMATCH`.
9. **Step 9 — Atomic State Commit**:
   - Commit the updated `tables`, update `high_water_sequence = manifest.snapshot_sequence`, record `committed_manifests[seq] = manifest.manifest_digest`, update generation/tombstone watermarks, and return `APPLIED`.
