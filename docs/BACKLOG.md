# AOB MVP backlog

Origin: cross-product council, 2026-11-11. **Status: recommendation pending maintainer decisions.**
This records the agent-otel-bridge (AOB) portion of the family MVP plan; it does not accept an ADR
or authorize implementation, captures, or merges.

## Verified AOB facts

The council verified these facts against AOB revision `96f9fdb`. Citations are pinned to that
revision and describe the planning baseline.

| # | Fact | Evidence |
|---|---|---|
| F5 | AOB is not zero-content by default. It emits absolute paths, an explicit `$USER_EMAIL` regardless of the enrichment setting, raw error status, and branch and repo names in plaintext | [crates/agent-otel-daemon/src/config.rs:36-47](https://github.com/smota/agent-otel-bridge/blob/96f9fdb/crates/agent-otel-daemon/src/config.rs#L36-L47); [crates/agent-otel-daemon/src/daemon.rs:296-302](https://github.com/smota/agent-otel-bridge/blob/96f9fdb/crates/agent-otel-daemon/src/daemon.rs#L296-L302); [crates/agent-otel-core/src/otlp.rs:417](https://github.com/smota/agent-otel-bridge/blob/96f9fdb/crates/agent-otel-core/src/otlp.rs#L417); [crates/agent-otel-core/src/otlp.rs:459-469](https://github.com/smota/agent-otel-bridge/blob/96f9fdb/crates/agent-otel-core/src/otlp.rs#L459-L469); [crates/agent-otel-core/src/otlp.rs:579](https://github.com/smota/agent-otel-bridge/blob/96f9fdb/crates/agent-otel-core/src/otlp.rs#L579) |
| F6 | AOB's fallback trace ID is an unkeyed SHA-256, and an inherited TRACEPARENT wins. The session→trace hash join cannot work | [crates/agent-otel-core/src/trace_id.rs:9](https://github.com/smota/agent-otel-bridge/blob/96f9fdb/crates/agent-otel-core/src/trace_id.rs#L9); [crates/agent-otel-core/src/trace_id.rs:146](https://github.com/smota/agent-otel-bridge/blob/96f9fdb/crates/agent-otel-core/src/trace_id.rs#L146) |
| F7 | AOB documents itself as "strictly in-memory" | [docs/LIMITATIONS.md:97](https://github.com/smota/agent-otel-bridge/blob/96f9fdb/docs/LIMITATIONS.md#L97) |
| F8 | AOB pairs Pre/Post on the bare call ID, and does not drain open spans on shutdown | [crates/agent-otel-daemon/src/span_assembler.rs:190](https://github.com/smota/agent-otel-bridge/blob/96f9fdb/crates/agent-otel-daemon/src/span_assembler.rs#L190); [crates/agent-otel-daemon/src/daemon.rs:243-267](https://github.com/smota/agent-otel-bridge/blob/96f9fdb/crates/agent-otel-daemon/src/daemon.rs#L243-L267) |
| F12 | Harness names differ: AOB `claude-code`, dendro `claude` | [crates/agent-otel-core/src/model.rs:188](https://github.com/smota/agent-otel-bridge/blob/96f9fdb/crates/agent-otel-core/src/model.rs#L188) |

## Shared identity and contract recommendations

The MVP has no local file loop: the AOB spool is deferred. The join uses a keyed attribute on
the backend side rather than a session-to-trace hash join.

The session link derivation is:

```text
agent.session.link_id = hex(HMAC-SHA256(K_link,
  LP("agent.session.link/v1", org, endpoint_scope, canonical_harness, native_session)))
K_link = HKDF(org master, purpose ‖ team ‖ epoch)
```

- `endpoint_scope` is a jointly provisioned, persistent endpoint namespace. It is never a
  daemon instance ID and never an ingestion generation.
- Harness names come from a closed canonical vocabulary.
- The link is set on every *identifiable* span; anything else is marked `unlinked`, never guessed.

The proposed contract home for the `fact-export-v1` and `link-id-v1` schemas and test vectors is
`docs/contracts/fact-export-v1/` in agent-otel-bridge, under Apache-2.0. Downstream consumers
keep digest-pinned copies. Contract crate extraction (#22) stays deferred.

## AOB work packages

The entries below retain the council table's issue mapping, size, dependencies, lane, scope,
acceptance evidence, and executor. `NEW` denotes an issue to be filed. S ≤2 days; M 3–5 days.
`*` marks the parallel or post-pilot P1 track. Cross-product dependencies are referenced by
package ID only; the shared issue mapping on G0a is retained.

Lanes give exclusive write ownership: packages in the same lane run serially. Different lanes
may run concurrently in separate worktrees once dependencies are met. AO owns AOB pipeline
and config; AN lanes own AOB native modules.

| WP | Repo / issue | Size | Depends on | Lane | Scope → acceptance evidence | Executor |
|---|---|---:|---|---|---|---|
| G0a | AOB#24, dendro#142 | M | S7 | captures-hook/log | Pinned identities and historical git provenance → sanitized matrix, receipt hashes | agy |
| G0b* | AOB#24 | M | S7 | captures-native | Claude Code native discovery → field, content and outage matrix | agy |
| C1a | AOB NEW | M | G0a | contract-identity | Identity and keys → independently checked collision and rotation vectors | codex + claude review |
| C1b | AOB NEW | M | C1a | contract-facts | Tables, snapshots, coverage → schema and replay vectors | codex |
| A0 | AOB#21 (split) | S | — | AOB-docs | Claims → evidence-linked corrections | agy |
| A1 | AOB#17 | M | C1a | AO | Privacy projection (F5) → forbidden-field sentinel audit | codex + claude review |
| A2 | AOB NEW | M | A1 | AO | Keyed session link → shared vectors, inherited-trace preservation | codex + claude review |
| A3 | AOB#16 (split) | M | A2 | AO | Qualified pairing, expiry, shutdown drain (F8) → collision and shutdown fixtures | codex |
| A4* | AOB#14 (split) | M | G0b, C1b | AN-map | Claude Code native mapping → captured-version equivalence | codex |
| A5a* | AOB#13 | M | C1b | AN-ingress | Bounded authenticated loopback ingress → flood, auth and limit tests | codex + claude review |
| A5b* | AOB#13, #15, #16 rest | M | A3, A4, A5a | AO | Integration and `enable`/`doctor` → privacy, source and outage receipts | codex |

Native discovery and integration are narrowed to Claude Code only (G0b, A4, A5a, A5b), on a
parallel P1 track outside the pilot's critical path. Hook signals are never held back waiting
for native ones. The council routes security review for C1a, A1, A2, and A5a to Claude;
standard-path review goes to Grok or Agy.

## Deferred AOB issues

Outside the MVP gate, subject to S6:

- #18: thin hook.
- #19: spool.
- #20: sandbox mode.
- #22: contract crate.
- #23: `coding_agent.*` projection.
- #14: the OTTL pack half only; A4 retains the native mapping half.

## AOB waves and exit evidence

This projects the family waves onto AOB packages only; it does not certify downstream gates.

| Wave | AOB packages | AOB exit evidence |
|---|---|---|
| 0 | G0a (after S7), A0; G0b optional (after S7) | Capture hashes and provenance; sanitized capture matrix; evidence-linked claim corrections. |
| 1 | C1a → C1b | Reviewed contract and vector digests; relevant maintainer decisions recorded. |
| 2 | A1 → A2 → A3 | Spans with a keyed link ID and no paths or emails; shared link vectors; inherited-trace preservation; collision and shutdown fixtures; independent security acceptance. |
| 3 | No AOB packages assigned | No separate AOB exit gate. |
| 4 | No AOB packages assigned | AOB evidence contributes to the pilot; G0b, A4, A5a, and A5b remain outside its P0 gate. |

The P1 track follows G0b and C1b → A4, C1b → A5a, then A3 + A4 + A5a → A5b,
subject to lane ownership. Its acceptance evidence remains the package table's field/content/outage
matrix, captured-version equivalence, ingress tests, and integration receipts.

## Maintainer decisions relevant to AOB

These decisions remain pending:

| # | Decision | Council recommendation | Blocks |
|---|---|---|---|
| S6 | MVP scope cut | Approve the deferrals above. | All MVP execution. |
| S7 | Authorize Gate-0 captures | Run Claude Code 2.1.322 and Codex 0.167.0 in disposable synthetic workspaces with a loopback fixture collector, using synthetic sentinels only. | G0a, G0b. |
| S8 | Human approval for high-assurance work | The sponsor is the human approver on PRs for C1a, A1, and A2. | Relevant merges. |
