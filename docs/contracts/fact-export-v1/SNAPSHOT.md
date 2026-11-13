# fact-export-v1: snapshot and import specification (C1b)

This is the single normative specification. The table JSON schemas and
tables/README.md are normative only as incorporated below. Checkers, generators,
and other documents are informative and cannot introduce rules. This candidate
is **not frozen**. The independent link-id-v1 contract remains unchanged.

## 1. Encoding and schema bundle

Canonical table order: sessions, invocations, tool_calls, usage, outcomes,
git_evidence, coverage. Primary keys: session_key, invocation_key, tool_call_key,
usage_key, outcome_key, git_evidence_key, coverage_key respectively.

Canonical JSON is UTF-8 without BOM, sorted object keys, compact separators,
unescaped Unicode, standard JSON escaping, no floats, no whitespace. Integers
fit signed INT64; booleans are not integers. Table bytes are each canonical JSON
row in supplied order followed by LF; an empty table is zero bytes.
For each table in canonical order hash its entire canonical JSON schema, including
metadata. Concatenate `<sha256>  tables/<table>.schema.json\n` in that order;
schema_digest is SHA256 of that UTF-8 text. manifest_digest is SHA256 of canonical
JSON after removing only manifest_digest. All digests are lowercase hexadecimal.

Schemas MUST declare x_contract=fact-export-v1, x_version=1, x_table,
x_primary_key, additionalProperties=false. required contains every property
exactly once, including nullable fields. Each column has x_justification with
nonempty why_present, consumers, zero_content, pseudonymization,
missingness_semantics. x_parquet has unique positive field_id (gaps allowed),
repetition agreeing with nullability; strings map to BYTE_ARRAY/STRING, integers
to INT64, timestamps to TIMESTAMP(MICROS,true), booleans to BOOLEAN/NONE, string
arrays to GROUP/LIST<STRING>. Exact mappings are in the incorporated schemas.

## 2. Manifest and publication

A manifest has exactly these properties:

| Property | Constraint |
| --- | --- |
| contract_id, contract_version | fact-export-v1, integer 1 |
| schema_digest, manifest_digest | 64 lowercase hex |
| tenant_placeholder | `^tn_[a-z0-9][a-z0-9_-]{1,60}$` |
| stream | `^[a-z0-9][a-z0-9._-]{1,63}$` |
| snapshot_sequence, published_at_us | positive INT64 |
| producer | exactly producer_id (64 hex), producer_kind (dendro_cli, agent_otel_daemon, dendro_station_exporter), producer_version (`^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9._-]+)?$`), pseudonym_key_id |
| replacement_scope | exactly scope_mode, operation, reason, source_id, source_generation, session_keys |
| skip_reason | null normally; otherwise abandoned_publication, source_unavailable, producer_recovery |
| files | seven descriptors in canonical order normally; zero for explicit skip |

Scope source_id is 64 hex, source_generation positive INT64, session_keys a unique
array of 64-hex pseudonyms. Exactly two scope modes exist: source_generation
(empty session_keys) and session_keys (nonempty session_keys, purge_delete only).
Operations: upsert_replace, purge_delete. Reasons: initial_export,
in_generation_outcome_or_attribution_update, source_generation_advance,
retention_or_redaction_purge, key_rotation. Neither full_stream nor
supersedes_generations exists in v1. D4b emits source_generation only.

Each file descriptor has exactly table, path, format, row_count, byte_length,
sha256. Table matches its canonical position. Path is exactly
tables/<table>.parquet for parquet, tables/<table>.jsonl for canonical_json_rows.
Counts are nonnegative INT64, sha256 is 64 hex. Production importers MUST reject
canonical_json_rows with REJECTED_INVALID_MANIFEST; only explicitly selected
vector replay mode accepts it. Parquet integrity uses actual file bytes and
decoded row_count, then decoded rows undergo the same row checks.

Write payloads first; atomically publish MANIFEST.json last (rename or conditional
create). Manifests and sequence slots are write-once. Absent manifests are invisible
and never imported. An unused slot may be completed or filled by an explicit skip
before the next sequence. A visible rejected manifest MUST NOT be repaired or
republished: publish the next sequence. Producers SHOULD run the same checker
before publishing.

An explicit skip has zero files, non-null skip_reason, and source_generation /
upsert_replace scope with empty session_keys. The scope is descriptive only.
A valid skip consumes its sequence without changing tables, keys, generations
or tombstones; it emits importer coverage status skipped, reason=skip_reason.

## 3. Import state and ordered algorithm

State key K=(tenant_placeholder, producer_id, stream); idempotency key=(K,sequence).
Each K has cursor (initial 0), all consumed digests/outcomes, active generation and
last key ID by source, purged-generation watermarks, permanent session tombstones,
active tables, and import coverage rows. A checker instance is bound to one K;
separate instances MUST NOT share state.

Evaluate in this exact order. Steps 1-5 never consume or mutate state.

1. Absent/null committed manifest: IGNORED_UNCOMMITTED_NO_MANIFEST.
2. Require tenant_placeholder, producer.producer_id AND stream to equal the
   authenticated expected K; otherwise REJECTED_TENANT_OR_PRODUCER_MISMATCH.
3. Validate section 2 shape, types, enums, bounds, paths and format mode, including
   digest syntax but NOT digest equality. Failure: REJECTED_INVALID_MANIFEST.
4. For any consumed sequence, same declared digest -> DUPLICATE_IDEMPOTENT;
   different declared digest -> CONFLICTING_DUPLICATE. This includes ALL old,
   rejected and skipped sequences. Do not revalidate payloads or schema pins.
5. Sequence >cursor+1 -> GAP_PENDING. Retry after the missing slot arrives or
   the producer publishes an explicit skip there. No implicit skips.
6. Verify manifest digest equality (REJECTED_INVALID_MANIFEST), then schema digest
   (REJECTED_SCHEMA_MISMATCH), then payload existence/byte_length/SHA256/row_count
   (REJECTED_FILE_INTEGRITY), in this priority order.
7. Validate section 4 rows and unique primary keys within each table. Failure:
   REJECTED_ROW_SCHEMA_VIOLATION.
8. Check section 5 freshness, tombstones, key rotation (failure:
   REJECTED_STALE_GENERATION_OR_TOMBSTONE). Build candidate state; incorrect row
   scope, nonempty purge payloads, primary-key collisions with other sources,
   section 6 references/coverage/usage grains fail with
   REJECTED_REFERENTIAL_OR_COVERAGE_MISMATCH.
9. Atomically commit, store digest/outcome, cursor=sequence: APPLIED (SKIPPED for
   explicit skip). Step-6-valid skips bypass steps 7-8.

ANY rejection in steps 6-8 is consumed: preserve tables, keys, generations and
tombstones, atomically record REJECTED(sequence,reason), digest and cursor=sequence,
append import coverage. The next sequence proceeds. Earlier rejections do not
consume. Import coverage is importer-owned and separate from producer coverage.
Exactly one row per consumed sequence contains tenant_placeholder, producer_id,
stream, snapshot_sequence, manifest_digest, status (accepted, rejected, skipped),
reason (null for accepted, rejection code or skip_reason otherwise). Gaps and
duplicate retries append no rows.

## 4. Row rules

The seven tables/*.schema.json files' type, enum, pattern, bounds, lengths, anyOf,
array items/minItems/uniqueItems constraints are incorporated. Unknown/missing
columns fail. Null must satisfy both type and enum. Pattern matching is full-match.
All integers fit INT64. Every row key ID equals the manifest pseudonym_key_id.
Shared key-ID pattern: `^[a-z0-9][a-z0-9._-]{1,63}$`. Shared effort enum:
minimal, low, medium, high, xhigh, max, not_applicable, unknown. Shared model
pattern: `^[A-Za-z0-9._:/-]{1,128}$`. Every non-null epoch timestamp is >=1.
Harness enum everywhere: antigravity, claude, codex, grok, pi, unknown.

Every string value, recursively including array items, is scanned with ASCII
case-insensitive substring matching against exactly this zero-content denylist:
`g0a_`, `_sentinel`, `sk-`, `ghp_`, `gho_`, `github_pat_`, `akia`, `aiza`,
`xoxb-`, `xoxp-`, `://`, `http:`, `https:`, `ssh:`, `git@`, `\`, `@`,
`/users/`, `/home/`, `c:/`. No undocumented substring rules. These restrictions
supplement field grammars; they never authorize free-form content.

Additional invariants (schema x_invariants are descriptive mirrors):

* sessions: linked ID iff reason null and link_epoch non-null; unlinked means
  reason present and epoch null. Non-harness origin or unknown harness cannot link.
  org/team are operator-provided C1a printable ASCII slugs (0x21-0x7e, 1..1024), or
  null. endpoint_scope is HMAC of a raw slug satisfying the same grammar. For each,
  status observed iff value present; null status is missing_org (org/team: missing
  org-provided attribution), missing_endpoint_scope (endpoint), or
  invalid_field_characters (grammar/denylist failure). Missing org/endpoint requires
  unlinked with its reason; invalid org/endpoint requires unlinked with
  invalid_field_characters (invalid has priority, then missing_org, then missing_endpoint_scope). harness_version_status observed
  iff version present. timestamp_status complete means both timestamps and end>=start;
  start_only means start only; not_observable means both null. is_root implies
  relation=status=root and parent null. evidenced means nonroot, nonself parent,
  nonroot relation. Other lineage statuses require parent null.
* invocations: call grain has message_key and null call_id_missing_reason;
  usage_only has null message and non-null reason. model_status observed requires
  model present/reason null; unknown requires model null/reason present.
  occurred_at_status observed iff timestamp present.
* tool_calls: linked iff invocation_key present; call_id_status observed iff
  call_id_hmac present; fingerprint_status observed iff input_fingerprint_hmac
  present. pending/unknown outcome requires reason; other outcomes forbid reason.
  occurred_at_status and duration_status observed iff their values present.
  tool_name is ONLY its schema's closed built-in list or mcp_hmac_<64hex>.
* usage: linked iff invocation_key present. usage_grain=session_aggregate iff
  counter_semantics=session_aggregate and invocation_link_status=session_level_summary;
  per_turn forbids both aggregate indicators. At most one aggregate per session;
  aggregate and per-turn rows are mutually exclusive per session. Observed or
  inferred model requires model present/reason null; unknown is the reverse.
  occurred_at_status observed iff timestamp present. predecessor_usage_key present
  iff cumulative_delta or cumulative_monotonic_skip. total_tokens equals checked
  sum of input_uncached_tokens, output_tokens, cache_read_tokens, cache_write_tokens.
  reasoning_status observed iff reasoning_tokens present. Cost provenance is
  harness_reported, provider_receipt_exact or unknown. Known costs require micros
  present/reason null; unknown requires micros null/reason present. Export ONLY
  observed_usd_micros INT64, always per-row delta, never running total. For finite
  nonnegative DOUBLE USD, multiply the exact binary64 value mathematically by
  1,000,000, then round-half-even to INT64 (reject overflow); no intermediate
  binary64 rounding. Difference cumulative receipts before exporting.
* outcomes: remote/branch/head/base value present iff status is observed/
  named_branch/observed/observed respectively. Missing evidence means all four
  statuses not_observed and reason present; observed_full means all four values
  present/reason null; observed_partial means neither extreme/reason null.
  observed_at_status observed iff timestamp present. Historical as recorded,
  never sampled from current HEAD at export time.
* git_evidence: dendro D4c produces LOCAL historical git records only, never
  network lookups/current HEAD sampling. commit_hmac and ordered parent_hmacs
  are keyed pseudonyms; empty parents means root commit. merge_status observed iff
  is_merge present; if observed is_merge=(parent count>1). default_branch_status
  observed iff on_default_branch present. observed_at_status observed iff timestamp
  present. session_match_status matched iff session_key present; unknown, unmatched,
  ambiguous require null. Never choose an arbitrary ambiguous session. Enterprise
  joins commit pseudonyms under the same key ID, never raw SHAs.
* coverage: encountered=accepted+rejected+recognized_unprojected;
  expected_population=observable+unknown_or_missing (checked INT64 sums).
  complete: expected>0, observable=expected, unknown/rejected/unprojected=0;
  partial: expected>0, observable>0, at least one of those three>0;
  empty: expected=0, discovery discovered/empty_root, projection not
  connector_unimplemented; unavailable: observable=0 and (expected>0 or projection
  connector_unimplemented or discovery missing_root/permission_denied/not_configured).
  complete/empty gap_reasons=[none]; others nonempty without none.
  coverage_key is recomputed by the derivation in tables/README.md.

Pseudonym derivations and Parquet mapping in tables/README.md and the schemas
are incorporated. Raw commits, endpoints and custom tool names are forbidden.
Importers validate representation, not possession of the secret pseudonym key.

## 5. Replacement, purge and key rotation

All incoming normal rows belong to scope source_id and source_generation.
A source scope spans ALL harnesses. Upsert replaces every source row in all seven
tables and updates generation/key ID. Reject generation<active or <=purged.
Reject any incoming non-null session_key in the permanent tombstone set, regardless
of generation or source within K. Never overwrite another source's primary keys.

Source-generation purge requires all payloads empty, generation>=active and
>purged, deletes source rows through that generation, sets active and purged
watermarks to at least that generation, preserves permanent session tombstones
and last key ID. Later exports must exceed the purged watermark.

Session purge is reserved for EN5c: generation MUST equal active, key ID MUST equal
active key, ALL payloads empty. Delete only rows with that source and selected
non-null session_key. Preserve all surviving rows byte-identically, including
source_generation and ALL coverage rows. No generation bump or coverage swap.
Tombstone selected keys permanently within K even if absent. Skip coverage
reconciliation for this deletion: preserved coverage describes the pre-purge export.
Subsequent full source upserts require accurate complete coverage again.
Deletion authority in v1 is producer-issued purge manifests only; an
Enterprise-to-producer retention channel is deferred to EN5c. A session purge
may also record permanent tombstones after a source-generation purge, when
active equals the purged watermark; it still uses that active generation and key.

Changing pseudonym_key_id requires source-generation purge of ALL old-key rows,
then COMPLETE re-export at a strictly greater generation under the new key.
Without that purge, reject at step 8. source_id and producer_id are stable
provisioned routing pseudonyms, computed once and retained through fact-key
rotation; other keyed pseudonyms are recalculated. Producers must retain their
purge ledger and exclude purged logical sessions when rotating pseudonyms.
No raw SHA, translation table or key material is sent to Enterprise.

## 6. Snapshot references and coverage

Validate complete candidate state for the affected source after upsert; other sources historical post-purge coverage is not revalidated. Every non-null session_key in
invocations, tools, usage, outcomes and git_evidence references a session with equal
source_id, source_generation, canonical_harness and pseudonym_key_id. Non-null
invocation keys reference the same session. Invocation identity tuple
(session_key,sequence_no,invocation_grain,message_key) is unique. Usage grains obey
section 4. Parent-session existence is not required (retained population may differ).

For every (source_id,generation,canonical_harness,table) with at least one fact row
in the snapshot, require EXACTLY one coverage row_ingestion row. Coverage tuples
(source_id,generation,harness,target_table,dimension) are unique for ALL dimensions.
Zero-row coverage is allowed; target_table excludes coverage, includes git_evidence.
accepted_count always equals the corresponding fact population count. row_ingestion
expected_population=encountered_count, observable=accepted_count, and
unknown_or_missing=rejected_count+recognized_unprojected_count. Other dimensions
have expected_population=count; observable counts the predicate below and unknown
is its complement. Inapplicable table/dimension combinations fail.

| Dimension | Tables | Observable predicate |
| --- | --- | --- |
| session_link_id | sessions | link ID != unlinked |
| model_attribution | invocations, usage | model_status != unknown |
| observed_cost_usd | usage | cost_provenance != unknown |
| tool_outcome_pairing | tool_calls | outcome not pending/unknown |
| git_outcome_evidence | outcomes, git_evidence | evidence_status != missing; session_match_status == matched respectively |
| timestamp_fidelity | all six fact tables | sessions timestamp_status=complete; invocations/tools/usage occurred_at_status=observed; outcomes/git_evidence observed_at_status=observed |

Recompute coverage_key from its tuple. After targeted purge only, producer coverage
remains historical as section 5 states. Importer rejection coverage is separate.

## 7. Evidence

SHA256SUMS hashes exact bytes of SNAPSHOT.md, tables/README.md, seven schemas and
vectors in lexicographic relative-path order. CONTRACT-DIGEST is SHA256 of exact
SHA256SUMS bytes. Checkers, generator, reports, captures and link-id-v1 are excluded.
Both independent checkers verify digests, replay vectors and compare statuses,
cursors, import coverage, per-step active-table digests and final active tables. Passing is informative evidence,
not authority to freeze or production-importer certification.
