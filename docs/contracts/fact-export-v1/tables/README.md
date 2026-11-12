# `fact-export-v1` Normative Table Schemas & Parquet Logical-Type Mapping (`C1b`)

**Contract Identifier**: `fact-export-v1`  
**Contract Home**: `smota/agent-otel-bridge` (`docs/contracts/fact-export-v1/tables/`)  
**Join-Key Dependency**: [`link-id-v1/SPEC.md`](../link-id-v1/SPEC.md) (`07dff9562958ec286c1b096f0eb15c91deddb284c26d99904afeba2b882d0ab5`)  
**Authoritative Inputs**: `2026-11-11-mvp-plan.md` (§2 Decisions 3–5; Findings `F1`–`F4`, `F8`, `F9`, `F12`–`F14`), ADRs `DE-002`, `DE-004`, `DE-005`, `DE-006`, `DE-008`, and `dendro` `DATA_DICTIONARY.md` (`I01`–`I25`)  
**Consumers**: `D1`, `D4b`, `D4c`, `D5`, `EN1`, `EN2`, `EN3a–c`, `EN5c`, `EN6a`, `EN6b`, `A4`, `A5a`

---

## 1. Purpose & Replacement of the Legacy Aggregate Export (`F3`)

Prior to `fact-export-v1`, the prototype DuckLake exporter (`F3`) emitted a 4-column aggregate summary (`canonical_harness`, `session_count`, `event_count`, `total_tokens`), which could not support session-to-trace joining (`DE-006`), qualified `I01` invocation counting (`F9`), tool archetype/failure-loop analysis (`D1`), Enterprise contract-price estimation (`EN6a`), historical git outcome proxies (`EN6b`), or explicit missingness denominators (`F14`, `D5`).

`fact-export-v1` defines **six flat per-table schemas** exported as atomic Parquet snapshots (see [`../SNAPSHOT.md`](../SNAPSHOT.md)) and represented in replay vectors as canonical JSON:

| Table | Normative JSON Schema | Primary Key | Row Granularity | Primary Producers / Consumers |
| :--- | :--- | :--- | :--- | :--- |
| **`sessions`** | [`sessions.schema.json`](./sessions.schema.json) | `session_key` | One row per discovered agent session | `D1`, `D3`, `D4b`, `EN3c`, `EN6a`, `EN6b`, `A4`, `A5a` |
| **`invocations`** | [`invocations.schema.json`](./invocations.schema.json) | `invocation_key` | One row per session-qualified `I01` model/agent invocation (`F9`) | `D1`, `D2`, `EN3c`, `EN6a`, `A4` |
| **`tool_calls`** | [`tool_calls.schema.json`](./tool_calls.schema.json) | `tool_call_key` | One row per tool call execution with replaceable outcome status (`F2`, `F8`) | `D1`, `A3`, `EN3c`, `EN6a` |
| **`usage`** | [`usage.schema.json`](./usage.schema.json) | `usage_key` | One row per normalized token/cost accounting delta (`I02`, `F14`) | `D1`, `EN3c`, `EN6a`, `A4` |
| **`outcomes`** | [`outcomes.schema.json`](./outcomes.schema.json) | `outcome_key` | One row per historical git evidence checkpoint extracted from session records (`D4c`, `F4`) | `D4b`, `D4c`, `EN6b` |
| **`coverage`** | [`coverage.schema.json`](./coverage.schema.json) | `coverage_key` | One row per `(source_id, source_generation, canonical_harness, target_table, coverage_dimension)` (`D5`, `F14`, `F1`) | `D5`, `EN3c`, `EN6a`, `EN6b`, `A5a` |

---

## 2. Cross-Table Invariants

### 2.1 Zero-Content Guarantee (`DE-004`)
Every column in all six tables is restricted to:
- **64-character lowercase hexadecimal HMAC-SHA256 digests** (`^[0-9a-f]{64}$`);
- **64-character lowercase hexadecimal `agent.session.link_id` tags or literal `"unlinked"`** (`^([0-9a-f]{64}|unlinked)$`, per `link-id-v1`);
- **Closed-vocabulary enum strings** (harnesses, archetypes, statuses, reason codes);
- **Bounded safe identifier slugs/tokens** (`pseudonym_key_id`, `org`, `endpoint_scope`, `team`, `link_epoch`, `harness_version`, `model`, `tool_name`) validated against strict character classes and prohibited from containing whitespace, filesystem slashes (`/` in paths, `\`), `@` email markers, URLs (`://`), secret prefixes (`sk-`, `ghp_`, `AKIA`, `AIza`, `xoxb-`), or G0a leak sentinels (`G0A_*_SENTINEL*`);
- **Fixed-scale decimal strings** (`observed_usd`: `^[0-9]+\.[0-9]{6}$`); or
- **Non-negative 64-bit integers and booleans**.

Free-form prompts, completions, tool arguments, tool stdout/stderr, file paths, repository URLs, raw git branch names, raw 40-character git commit SHAs, email addresses, hostnames, and usernames are **structurally unrepresentable** and rejected by both the Rust (`agent_otel_core::fact_export`) and Python (`check_fact_vectors.py`) validators.

### 2.2 Customer-Held Key Pseudonymization (`F13`, `DE-004`)
Finding `F13` identified that `dendro` previously used unkeyed SHA-256 hashes (which are deterministic content hashes rather than pseudonymization) and passed raw git commit SHAs. In `fact-export-v1`:
1. Every pseudonymized digest column is computed as `HMAC-SHA256(customer_key, domain_prefix || canonical_input)` and formatted as 64 lowercase hex characters (`^[0-9a-f]{64}$`).
2. Every row carries `pseudonym_key_id` identifying the customer-held key generation used to produce its digests, enabling key rotation and `EN5c` retention purges without exposing secret key material.
3. Git evidence columns in `outcomes` (`repo_remote_hmac`, `branch_name_hmac`, `head_commit_hmac`, `base_commit_hmac`) are 64-character HMAC-SHA256 digests. Raw 40-character git SHAs are rejected.

### 2.3 Session-Qualified `I01` Invocation Identity (`F9`)
Finding `F9` identified that `I01` in `dendro` previously deduplicated invocations by bare `message_id` or `(turn_index, call_id)` without `session_key`, causing cross-session collisions whenever two sessions shared identical raw message identifiers or lacked call identifiers. In `fact-export-v1`:
- Every `invocations` row includes required `session_key`, `sequence_no`, `invocation_grain` (`"call"` vs `"usage_only"`), `message_key` (session-qualified 64-hex HMAC when `invocation_grain == "call"`), and `call_id_missing_reason` (when `invocation_grain == "usage_only"`).
- `invocation_key` and `message_key` are derived with `session_key` bound into the HMAC message. Two sessions with identical raw `message_id` or `sequence_no = 0` always produce distinct `invocation_key` and `message_key` values because their `session_key` values differ.

### 2.4 Explicit Unknownness & Missingness (`F14`)
Finding `F14` established that cost is `NULL` in 90–99.9% of local rows and timestamps are `NULL` in ~38% of Antigravity rows. Across all six tables:
- **`null` never means `0`, and `0` never means `unknown`.**
- Every nullable column has a required, non-null companion status or reason column enforced by bi-directional `x_invariants` in the JSON Schema:
  - `sessions`: `agent.session.link_id` (`"unlinked"`) $\iff$ `unlinked_reason`; `harness_version` $\iff$ `harness_version_status`; `started_at_us`/`ended_at_us` $\iff$ `timestamp_status` (`"complete"`, `"start_only"`, `"not_observable"`); `parent_session_key` $\iff$ `lineage_status` & `lineage_relation`.
  - `invocations`: `message_key` & `call_id_missing_reason` $\iff$ `invocation_grain` (`"call"` vs `"usage_only"`); `model` $\iff$ `model_status` & `model_unknown_reason`; `occurred_at_us` $\iff$ `occurred_at_status`.
  - `tool_calls`: `invocation_key` $\iff$ `invocation_link_status`; `call_id_hmac` $\iff$ `call_id_status`; `input_fingerprint_hmac` $\iff$ `fingerprint_status`; `outcome_status` $\iff$ `outcome_unknown_reason`; `occurred_at_us` $\iff$ `occurred_at_status`; `duration_us` $\iff$ `duration_status`.
  - `usage`: `invocation_key` $\iff$ `invocation_link_status`; `occurred_at_us` $\iff$ `occurred_at_status`; `model` $\iff$ `model_status` & `model_unknown_reason`; `predecessor_usage_key` $\iff$ `counter_semantics`; `reasoning_tokens` $\iff$ `reasoning_status`; `observed_usd` & `observed_usd_micros` $\iff$ `cost_provenance` & `cost_unknown_reason`.
  - `outcomes`: `missing_reason` $\iff$ `evidence_status`; `repo_remote_hmac` $\iff$ `remote_status`; `branch_name_hmac` $\iff$ `branch_status`; `head_commit_hmac` $\iff$ `head_commit_status`; `base_commit_hmac` $\iff$ `base_commit_status`; `observed_at_us` $\iff$ `observed_at_status`.
  - `coverage`: zero nullable columns; all denominators (`encountered_count`, `expected_population_count`) and numerators (`accepted_count`, `observable_count`, `unknown_or_missing_count`) are required non-negative integers with strict additive invariants.

---

## 3. Normative Parquet Logical-Type Mapping

Each property in `tables/*.schema.json` carries an `x_parquet` annotation specifying its Parquet `field_id` (1-based sequential per table), `repetition` (`REQUIRED` or `OPTIONAL`), `physical_type`, and `logical_type`:

| JSON Schema Type / Pattern | Parquet `physical_type` | Parquet `logical_type` | Canonical JSON Representation |
| :--- | :--- | :--- | :--- |
| `string` (HMAC hex, enum, token) | `BYTE_ARRAY` | `STRING` (UTF-8) | JSON string (`"..."`) or `null` if `OPTIONAL` |
| `integer` (`minimum: 0` or `1`, counts/ordinals/durations) | `INT64` | `INT(64, false)` / `INT(64, isSigned=true)` | JSON integer (`0`, `1`, ...) or `null` if `OPTIONAL` |
| `integer` (`*_at_us` UTC epoch microseconds) | `INT64` | `TIMESTAMP(MICROS, true)` | JSON integer (`1762900000000000`) or `null` if `OPTIONAL` |
| `boolean` | `BOOLEAN` | `NONE` | JSON boolean (`true` / `false`) |
| `string` (`observed_usd`: `^[0-9]+\.[0-9]{6}$`) | `INT64` | `DECIMAL(18, 6)` | Fixed 6-decimal string (`"0.014250"`) paired with `observed_usd_micros` (`14250`) |
| `array` of `string` (`coverage.gap_reasons`) | `GROUP` | `LIST<STRING>` | Non-empty JSON array of unique enum strings (`["none"]`) |

Because canonical JSON rows use only strings, 64-bit integers, booleans, arrays, and `null` (never IEEE-754 floats), canonical JSON serialization (`sort_keys=True`, compact separators `,` and `:`) is 100% deterministic and byte-identical across Rust and Python without requiring a Parquet library to validate replay vectors.

---

## 4. Column-by-Column Justification Summary

Every column in every schema file includes an embedded `x_justification` object with five required keys:
1. `why_present`: Analytical or operational requirement from the MVP plan.
2. `consumers`: Downstream work items (`D1`, `D2`, `D3`, `D4b`, `D5`, `EN3a–c`, `EN5c`, `EN6a`, `EN6b`, `A4`, `A5a`).
3. `zero_content`: How the column satisfies `DE-004`.
4. `pseudonymization`: How the column satisfies `F13` and `DE-004`.
5. `missingness_semantics`: How the column satisfies `F14` without null-means-zero ambiguity.

### 4.1 `sessions` (`tables/sessions.schema.json` — 23 columns)
- **Identity & Scope**: `session_key` (PK, 64-hex HMAC), `source_id` (64-hex HMAC), `source_generation` (`>= 1`), `pseudonym_key_id`, `org`, `endpoint_scope`, `team`.
- **Trace Join (`DE-006`, `C1a`)**: `agent.session.link_id` (64-hex lowercase HMAC-SHA256 tag or literal `"unlinked"`), `link_epoch` (key epoch slug or `null`), `unlinked_reason` (closed 9-reason vocabulary from `link-id-v1` or `null`), `origin` (`"harness"`, `"synthetic"`, `"unknown"`).
- **Platform & Surface**: `canonical_harness` (`antigravity`, `claude`, `codex`, `grok`, `pi`, `unknown`), `harness_version` + `harness_version_status` (`observed`, `not_reported`), `signal_source` (`session_log`, `hook_ipc`, `native_otlp`).
- **Lifecycle & Timestamps (`F14`)**: `session_status` (`active`, `completed`, `failed`, `cancelled`, `interrupted`, `unknown`), `started_at_us`, `ended_at_us`, `timestamp_status` (`complete`, `start_only`, `not_observable`).
- **Lineage (`I03`, `I09`)**: `is_root` (`boolean`), `parent_session_key` (64-hex HMAC or `null`), `lineage_relation` (`root`, `spawn`, `delegate`, `handoff`, `return`, `join`, `retry`, `unknown`), `lineage_status` (`root`, `evidenced`, `absent`, `truncated`, `conflict`).

### 4.2 `invocations` (`tables/invocations.schema.json` — 18 columns)
- **Qualified `I01` Identity (`F9`)**: `invocation_key` (PK, 64-hex HMAC), `session_key`, `sequence_no` (`>= 0`), `invocation_grain` (`"call"` vs `"usage_only"`), `message_key` (session-qualified 64-hex HMAC when `invocation_grain == "call"`, else `null`), `call_id_missing_reason` (`"usage_row_without_message_id"`, `"unreported_by_harness"`, or `null`).
- **Source & Platform**: `source_id`, `source_generation`, `pseudonym_key_id`, `canonical_harness`, `signal_source` (`session_log`, `hook_ipc`, `native_otlp`).
- **Replaceable Model, Effort & Outcome Attribution (`MVP plan §2 Decision 3`)**: `model`, `model_status` (`observed`, `unknown`), `model_unknown_reason` (`not_reported_in_event`, `pending_turn_attribution`, `redacted_unallowlisted`, or `null`), `effort` (`minimal`, `low`, `medium`, `high`, `max`, `not_applicable`, `unknown`), `invocation_status` (`succeeded`, `failed`, `cancelled`, `pending`, `unknown`), `occurred_at_us`, `occurred_at_status` (`observed`, `not_observable`).

### 4.3 `tool_calls` (`tables/tool_calls.schema.json` — 24 columns)
- **Identity & Pairing (`F8`, `D1`)**: `tool_call_key` (PK, 64-hex HMAC), `source_id`, `source_generation`, `pseudonym_key_id`, `session_key`, `invocation_key` + `invocation_link_status` (`linked`, `unassociated_hook_event`, `missing_parent_turn`), `canonical_harness`, `sequence_no`, `call_id_hmac` + `call_id_status` (`observed`, `not_reported`).
- **Capability & Preference-Agnostic Archetype (`F2`, `AGENTS.md §1 Invariant 5`)**: `tool_name` (safe identifier token or `mcp_custom_hmac_<16hex>`), `tool_category` (`coord_queue`, `coord_status`, `governance_phase`, `eng_build_test`, `eng_code_edit`, `eng_vcs`, `inspect_read`, `other`), `tool_archetype` (`filter_compressor`, `structured_parser`, `inspector_diff`, `search_retrieval`, `build_test_verify`, `generic_exec`, `file_read`, `file_write`, `agent_delegate`, `unknown`), `structured_write` (`boolean`).
- **Loop Detection (`I08`)**: `input_fingerprint_hmac` (64-hex HMAC) + `fingerprint_status` (`observed`, `input_unavailable`).
- **Replaceable Outcome & Latency (`D1`, `F14`)**: `outcome_status` (`succeeded`, `failed`, `cancelled`, `pending`, `unknown`) + `outcome_unknown_reason` (`awaiting_tool_result`, `result_not_recorded`, `stream_truncated`, or `null`), `occurred_at_us` + `occurred_at_status` (`observed`, `not_observable`), `duration_us` + `duration_status` (`observed`, `single_point_only`, `not_observable`), `signal_source` (`session_log`, `hook_ipc`, `native_otlp`).

### 4.4 `usage` (`tables/usage.schema.json` — 29 columns)
- **Identity & Attribution**: `usage_key` (PK, 64-hex HMAC), `source_id`, `source_generation`, `pseudonym_key_id`, `session_key`, `invocation_key` + `invocation_link_status` (`linked`, `session_level_summary`, `unattributed_turn`), `canonical_harness`, `sequence_no`, `occurred_at_us` + `occurred_at_status` (`observed`, `not_observable`).
- **Model & Effort (`MVP plan §2 Decision 4`)**: `model` + `model_status` (`observed`, `inferred_from_session`, `unknown`) + `model_unknown_reason` (`not_reported_by_harness`, `redacted_unrecognized_format`, `awaiting_turn_completion`, or `null`), `effort` (`minimal`, `low`, `medium`, `high`, `max`, `not_applicable`, `unknown`).
- **Normalized Token Deltas (`accounting_v1`, `I02`)**: `counter_semantics` (`per_turn_delta`, `cumulative_delta`, `cumulative_monotonic_skip`, `cumulative_reset_rebaseline`, `session_aggregate`), `predecessor_usage_key`, `input_uncached_tokens`, `output_tokens`, `cache_read_tokens`, `cache_write_tokens`, `total_tokens` (exact sum of the 4 components), `reasoning_tokens` + `reasoning_status` (`observed`, `included_in_output_only`, `not_supported`, `not_reported`).
- **Observed Cost & Provenance (`F14`, `EN6a`)**: `observed_usd` (fixed 6-decimal string or `null`), `observed_usd_micros` (integer micro-dollars or `null`), `cost_provenance` (`harness_observed_exact`, `provider_receipt_exact`, `unknown`), `cost_unknown_reason` (`not_reported_by_harness`, `subscription_unmetered`, `deferred_to_enterprise_rate_card`, `unpriced_model`, or `null`), `signal_source` (`session_log`, `hook_ipc`, `native_otlp`).

### 4.5 `outcomes` (`tables/outcomes.schema.json` — 21 columns)
- **Historical Git Evidence Only (`MVP plan §2 Decision 5`, `D4c`, `F4`, `EN6b`)**: `outcome_key` (PK, 64-hex HMAC), `source_id`, `source_generation`, `pseudonym_key_id`, `session_key`, `canonical_harness`, `sequence_no`, `evidence_source` (`session_record_header`, `session_record_event`, `session_tool_output_metadata`, `otel_workspace_snapshot` — never a live `git` subprocess), `evidence_status` (`observed_full`, `observed_partial`, `missing`), `missing_reason`.
- **Pseudonymized Git State (`F13`, `AGENTS.md §3.3`)**: `repo_remote_hmac` + `remote_status` (`observed`, `no_remote_configured`, `not_observed`), `branch_name_hmac` + `branch_status` (`named_branch`, `detached_head`, `not_observed`), `head_commit_hmac` + `head_commit_status` (`observed`, `not_observed`), `base_commit_hmac` + `base_commit_status` (`observed`, `single_point_only`, `not_observed`), `worktree_state` (`clean`, `dirty`, `unknown`), `observed_at_us` + `observed_at_status` (`observed`, `not_observable`).

### 4.6 `coverage` (`tables/coverage.schema.json` — 21 columns)
- **Dimension Key (`D5`, `F1`, `F14`, `A5a`)**: `coverage_key` (PK, 64-hex HMAC), `source_id`, `source_generation`, `pseudonym_key_id`, `canonical_harness`, `target_table` (`sessions`, `invocations`, `tool_calls`, `usage`, `outcomes`), `coverage_dimension` (`row_ingestion`, `session_link_id`, `model_attribution`, `observed_cost_usd`, `tool_outcome_pairing`, `git_outcome_evidence`, `timestamp_fidelity`).
- **Pipeline Statuses (`retrieval_v1`, `F1`)**: `discovery_status`, `read_status`, `projection_status` (includes `connector_unimplemented` for `pi` per `F1`), `field_fidelity` (`exact`, `inferred_or_partial`, `unavailable`).
- **Explicit Denominators & Numerators (`F14`)**: `denominator_basis`, `accepted_count`, `rejected_count`, `recognized_unprojected_count`, `encountered_count` (`== accepted + rejected + recognized_unprojected`), `expected_population_count` (`== observable + unknown_or_missing`), `observable_count`, `unknown_or_missing_count`, `coverage_status` (`complete`, `partial`, `empty`, `unavailable`), `gap_reasons` (non-empty array of closed reason codes).
