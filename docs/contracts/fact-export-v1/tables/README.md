# fact-export-v1 tables and pseudonym derivations

SNAPSHOT.md is the single normative source. It incorporates the derivations and
Parquet mappings here and the seven schemas' field constraints.

| Table | Primary key | Grain |
| --- | --- | --- |
| sessions | session_key | Discovered session |
| invocations | invocation_key | Qualified session/ordinal/grain/message |
| tool_calls | tool_call_key | Tool execution |
| usage | usage_key | Per-row delta; aggregate/per-turn exclusive |
| outcomes | outcome_key | Historical session git checkpoint |
| git_evidence | git_evidence_key | Local historical commit and ordered parents |
| coverage | coverage_key | Source/generation/harness/table/dimension |

## Incorporated normative derivations

LP(s1,...,sn) concatenates each UTF-8 string preceded by its byte length as an
unsigned u32 big-endian integer, exactly as C1a link-id-v1. No separator or
terminator. Lengths exceeding u32 fail. Decimal integers have no leading zeros.
Null optional derivation components use the empty string. Except coverage_key,
pseudonyms are lowercase hex HMAC-SHA256 under the org-wide customer-held key
identified by pseudonym_key_id. Message=LP(domain_prefix, canonical inputs below).

| Column | Domain prefix | Canonical inputs, in order |
| --- | --- | --- |
| producer.producer_id | fact-export.v1/producer | Provisioned stable random producer instance ID |
| source_id | fact-export.v1/source | Provisioned stable random logical source ID, not a path |
| session_key | fact-export.v1/session | source_id, canonical_harness, exact raw session ID |
| endpoint_scope | fact-export.v1/endpoint_scope | Raw C1a printable ASCII endpoint slug, case preserved |
| invocation_key | fact-export.v1/invocation | session_key, sequence_no, invocation_grain, message_key or empty |
| message_key | fact-export.v1/message | session_key, exact raw message ID |
| tool_call_key | fact-export.v1/tool_call | session_key, sequence_no, call_id_hmac or empty |
| call_id_hmac | fact-export.v1/tool_call_id | session_key, exact raw tool-call ID |
| input_fingerprint_hmac | fact-export.v1/tool_input_fingerprint | Canonical JSON of tool input under SNAPSHOT.md encoding rules |
| tool_name custom | fact-export.v1/tool_name | Exact raw tool name, no normalization |
| usage_key | fact-export.v1/usage | session_key, sequence_no, usage_grain |
| outcome_key | fact-export.v1/outcome | session_key, sequence_no |
| repo_remote_hmac | fact-export.v1/repo_remote | Historical URL with userinfo and query/fragment removed, otherwise exact |
| branch_name_hmac | fact-export.v1/branch | Exact recorded historical branch name |
| commit_hmac, parent_hmacs elements, head_commit_hmac, base_commit_hmac | fact-export.v1/commit | Canonical full commit ID |
| git_evidence_key | fact-export.v1/git_evidence | source_id, commit_hmac, observed_at_us or empty, session_key or empty, session_match_status |

Canonical commit input is lowercase full 40-hex SHA-1, or lowercase full 64-hex
SHA-256 for SHA-256 repositories. Validate the repository hash algorithm; reject
abbreviations/nonhex input. Commit-bearing columns deliberately share one identity
domain so Enterprise can match them. Parent order is local git's recorded order.
Enterprise matches pseudonyms with the same pseudonym_key_id, never raw SHAs.
Dendro D4c uses LOCAL historical evidence as recorded, never current HEAD at export
time or a network service. Default-branch evidence is historical or null/unknown.
Session match statuses are matched, unmatched, ambiguous, unknown; ambiguous
evidence never receives an arbitrary session link.

producer_id and source_id are provisioned once under the initial org key and
retained as routing identities through fact-key rotation. The provisioning key
is retained separately; pseudonym_key_id identifies the rotating key for every
other keyed column. Rotation purges ALL old-key source rows and re-exports at a
higher generation. The producer purge ledger prevents reminting purged logical
sessions under a new key.

Built-ins are exactly tool_name's schema enum, case sensitive. Every other name
is mcp_hmac_ followed by its full 64-hex keyed tool_name digest above. Raw MCP
names, arbitrary safe-looking slugs and truncated digests are invalid.

coverage_key=SHA256(LP("fact-export.v1/coverage", source_id,
decimal(source_generation), canonical_harness, target_table, coverage_dimension)).
This public digest contains only pseudonymized scope metadata and is recomputable
without a secret key. Exactly one row_ingestion row covers each populated tuple,
including canonical_harness=unknown when applicable.

org and team are operator-provided slugs, never inferred user names. Raw org,
endpoint_scope and team follow C1a printable ASCII 0x21..0x7e, 1..1024, plus the
zero-content denylist. Endpoint exports only its HMAC. Nulls require the status
in SNAPSHOT.md; optional team's missing_team denotes absent org-provided
attribution and does not itself prevent linking. agent.session.link_id and
link_epoch follow the unchanged link-id-v1/SPEC.md.

## Incorporated Parquet mapping

Schema x_parquet annotations assign stable field_id values, retained across this
rework. Removing observed_usd leaves an intentional ID gap. All properties must
be present in JSON; nullable values map to OPTIONAL. Strings are BYTE_ARRAY/STRING,
integers signed INT64, epoch micros TIMESTAMP(MICROS,true), booleans BOOLEAN/NONE,
string arrays GROUP/LIST<STRING> with STRING elements. Never zero-fill unknowns.
Only observed_usd_micros INT64 deltas are exported. DOUBLE conversion is exact
binary64 multiplication followed by round-half-even as SNAPSHOT.md specifies.

x_justification records each column's consumer, content restrictions, derivation
and missingness. x_invariants are explanatory mirrors; SNAPSHOT.md defines the
complete cross-field rules.
