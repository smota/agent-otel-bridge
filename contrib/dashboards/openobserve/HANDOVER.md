# OpenObserve package handover — 2026-11-05

The package generates five native v5 dashboards for local OpenObserve OSS 1.1.2
in the `default` organization. Provisioning uses exact description-marker
ownership, runtime ID resolution, preview-first planning, optimistic updates,
external immutable backups and guarded dashboard rollback. Receipts bind to the
PUT response hash and canonical full-definition digest; verification GET must
match both, including unmanaged fields. An intervening edit blocks receipt
creation and automated rollback.

Final parent-verified deployment evidence:

| Dashboard | Deployed ID |
| --- | --- |
| Fleet Portfolio | `7524223890374000640` |
| Workspace Project | `7524223890533384192` |
| Agent Workflow | `7524223890663407616` |
| Session Turn | `7524223890801819648` |
| Telemetry Operations | `7524223890931843072` |

All five dashboards were deployed. Hashes of all 12 existing dashboards remained
unchanged. After deploying the null-dimension correction, all **21/21 live SQL
and output-field checks passed**. Another apply performed zero writes across
all five dashboards. All 27 drilldown targets resolve to owned dashboards;
all three variables persist on every dashboard, with no unresolved ID placeholders.

This bounded leaf fix projects `host_name` and `service_instance_id` as
`COALESCE(..., 'unknown') AS` the original alias in every metric SELECT. Quota
provider, quoted `group` (alias `quota_group`), and bucket are likewise coalesced.
Raw GROUP BY dimensions remain unchanged. Regression coverage checks all seven
metric queries and both quota queries; generated artifacts match the generator.
Final live revalidation and deployment of this fix are complete.

Earlier parent validation passed: Cargo formatting, Clippy, workspace tests,
documentation, and guardrails; Collector configuration validation also passed.
The fixture contained **3 spans and 1 error**. These results were not rerun by this
leaf. Browser access was disabled and the user chose API-only validation; no
browser/UI acceptance is claimed.

Stream settings changed through expected distinct-value registrations. The
pre-change snapshot is external to the repository:
`/home/sam/.local/state/agent-otel-bridge/openobserve/stream-settings-before-20261105T213345Z.json`.
Dashboard backups and rollback exclude stream settings; stream recovery requires
separate operator action using that snapshot.

agy builders were prioritized. Codex handled provisioning, security review, and
fallback after the agy 120-second timeout. This leaf runs inline without subagents.
The package retains the Collector edge-only integration and OSS persona-only
boundary documented in [ADR-001.md](ADR-001.md); `--require-rbac` fails closed.
Use one provisioner across machines because creation has no distributed unique-
marker constraint; a missing receipt blocks automated rollback.

Leaf validation: `node --test contrib/dashboards/openobserve/build.test.mjs
contrib/dashboards/openobserve/provision.test.mjs` and package whitespace checks
**26 passed, 0 failed**; package whitespace checks passed. This leaf changes only package files, with no Rust
changes, live actions, commit, push, or merge.
