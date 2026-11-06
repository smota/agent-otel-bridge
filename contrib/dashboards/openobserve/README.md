# OpenObserve v5 Dashboards (`contrib/dashboards/openobserve`)

Deterministic generator and static artifacts for the five-level `agent-otel-bridge` observability hierarchy in **OpenObserve OSS v1.1.2** (native Dashboard schema `version: 5`).

## Generator API (`build.mjs`)

Provisioners and tests can import the generator directly:

```js
import {
  MARKER_PREFIX,
  SERVICE_NAME,
  TRACE_STREAM,
  TRACE_FIELDS,
  METRIC_STREAMS,
  DASHBOARD_VARIABLE_SPECS,
  DASHBOARD_LEVELS,
  formatDashboardMarker,
  extractDashboardKey,
  resolveDashboardId,
  buildDashboardLinkUrl,
  buildDashboard,
  buildDashboards,
  buildManifest,
  writeArtifacts,
} from "./build.mjs";
```

### Runtime Dashboard ID Resolution

OpenObserve assigns `dashboardId` on the server upon dashboard creation.
- **Static Artifacts**: `node contrib/dashboards/openobserve/build.mjs` emits `dashboardId: ""` and placeholder link tokens `${dashboard_id:<key>}`.
- **Runtime Provisioner Resolution**: Pass `{ dashboardIds: { "fleet-portfolio": "7524...", ... }, orgIdentifier: "default", folder: "default" }` to `buildDashboards(options)` or `buildDashboard(key, options)` to stamp server-assigned IDs into both `dashboardId` and all cross-dashboard `config.drilldown` / URL links.
- **Managed Identity Marker**: Every dashboard `description` begins with `agent-otel-bridge:openobserve:<key>` (parsed via `extractDashboardKey(description)`).

## Dashboard Variables & Safe Substitution Contract

Each dashboard declares three `query_values` variables scoped to `stream: "default"` (`stream_type: "traces"`) with `filter: [{ name: "service_name", operator: "=", value: "agent-otel-bridge" }]` and `escapeSingleQuotes: true`:

1. `workspace_project_name` (`'$workspace_project_name'`)
2. `gen_ai_agent_name` (`'$gen_ai_agent_name'`)
3. `gen_ai_conversation_id` (`'$gen_ai_conversation_id'`)

Cross-dashboard links carry over time range (`from=$__from&to=$__to&period=$__period`) and variable selections (`var-workspace_project_name=$workspace_project_name&var-gen_ai_agent_name=$gen_ai_agent_name&var-gen_ai_conversation_id=$gen_ai_conversation_id`).

> **Constraint Note**: OpenObserve custom SQL queries perform template interpolation rather than server-side prepared-statement binding. Using `query_values` variables with `escapeSingleQuotes: true` constrains UI selection to indexed telemetry values; filters do not provide general SQL sanitization for untrusted free-text SQL injection.

## Provisioning and rollback

Requires Node.js 22 or newer; uses only the standard library. Set `OPENOBSERVE_URL`
to an origin such as `http://127.0.0.1:5080`, `OPENOBSERVE_ORG` to `default`, and
inject `OPENOBSERVE_AUTH` from your secret manager as the **complete** `Basic …`
or `Bearer …` Authorization header. Never put the header in command arguments,
tracked files, shell history, or diagnostic output. HTTPS is required except for
literal loopback addresses and localhost. URL credentials, paths, queries,
fragments and HTTP redirects are rejected. Each request has a 10-second deadline
including response-body consumption, and a 16 MiB response limit. Errors omit
server bodies, SQL, variable values and credentials.

Run from the repository root:

```powershell
node contrib/dashboards/openobserve/provision.mjs
node contrib/dashboards/openobserve/provision.mjs --apply
node --test contrib/dashboards/openobserve/build.test.mjs contrib/dashboards/openobserve/provision.test.mjs
```

The default is preview: GET requests only, with no server or local state writes.
Provisioning is scoped to the configured organization's `default` folder. It
adopts only a v5 dashboard with exactly one complete identity marker at the start
of its description, followed by the description separator. The logical key is
stable even if its title is edited. Duplicate markers, malformed markers,
unsupported owned versions and unmanaged title collisions fail closed before
updates. Other dashboards, folders, users and permissions are outside this tool's
write scope. **Stream side effects:** in OpenObserve v1.1.2, dashboard create/update
processes `query_values` variables, registers distinct values and may persist
stream settings before dashboard persistence, even when the dashboard request
later fails. See the [v1.1.2 implementation](https://github.com/openobserve/openobserve/blob/v1.1.2/src/core/src/dashboards/mod.rs#L192).
Review and capture affected stream settings separately before apply. Dashboard
backups and rollback restore dashboard definitions only; they do not capture or
automatically roll back stream indexes/settings or distinct-value registrations.
An administrator must inspect these side effects and restore stream settings
separately if recovery is required, including after a failed apply.

Apply first creates missing identities, then regenerates all five dashboards with
the assigned IDs in navigation URLs and drilldowns. Preview shows planned creates
and updates; links to missing identities remain placeholders until apply. Server
owner/created metadata and additional server default fields do not force repeated
updates. A successful rerun is a no-op when managed fields match. PUT uses the
hash obtained from GET, so a concurrent edit causes a conflict rather than an
overwrite. Stop other provisioners during apply: the local exclusive lock prevents
same-state-directory writers, but the native create API has no distributed unique
marker constraint. An external simultaneous create may need manual collision
resolution; the tool never deletes or silently adopts duplicates.

There is no automatic POST retry. If create succeeds but its response is lost,
rerun: exact marker discovery reconciles the identity before creating anything
else. Partial applies can be completed by rerunning. Every update writes an
exclusive-create, flushed backup **before** PUT, followed by a separate receipt
bound to the returned PUT wrapper's hash and canonical definition digest. A
verification GET must match both before the receipt is written, including all
unmanaged fields; an intervening edit leaves a backup without a receipt. Files
are never overwritten by this tool. They contain
owned dashboard definitions and target identifiers, never Authorization headers.
Protect them as configuration data. State lives outside the repository at:

- Windows: `%LOCALAPPDATA%\agent-otel-bridge\openobserve\`
- Other platforms: `~/.local/state/agent-otel-bridge/openobserve/`

The apply summary reports backup UUID tokens. Restore a prior owned definition:

```powershell
node contrib/dashboards/openobserve/provision.mjs rollback --backup <UUID>
node contrib/dashboards/openobserve/provision.mjs rollback --backup <UUID> --apply
```

Rollback defaults to preview. It requires the backup plus completed receipt,
matching URL, organization, folder, logical key and dashboard ID, and unchanged
post-update hash and content. It records another backup before restoring. It
cannot delete dashboards, restore arbitrary files, cross targets or overwrite
later edits. A crash between PUT and receipt creation deliberately blocks that
backup's automated rollback; inspect it manually or reconcile with a normal apply.
Creation itself has no prior definition and is never rolled back by deletion.
After a crashed worker, inspect process activity before removing a stale
`apply.lock` from the external state directory.

## Read-only SQL validation

Prepare a local JSON file containing exactly the three variable names below,
using actual selections or the documented empty/`*` bypass values:

```json
{"workspace_project_name":"*","gen_ai_agent_name":"*","gen_ai_conversation_id":"*"}
```

```powershell
node contrib/dashboards/openobserve/provision.mjs validate --variables C:\private\o2-variables.json --start 1793836800000000 --end 1793840400000000
```

Use your own positive microsecond timestamps, with a range of at most 24 hours.
This runs all 21 generated SQL panels through the read-only `_search` POST API
with `validate=true`, quoted variable substitution and a one-row result size.
It checks API/query/function/partial errors and the fields needed by panel axes.
Only panel identifiers and pass/fail counts are printed; raw telemetry, SQL,
field values and server errors are never printed or saved. Empty results fail
field validation because they cannot establish output columns; choose an active
window and selections. Query execution can scan data and populate server search
history/cache; it performs no dashboard or ingestion mutation. The command exits
nonzero for any failed panel. Mock validation does not certify live SQL execution.

## Personas and permission prerequisites

These dashboards are **persona views**, not access controls. The current OSS
target has no validated dashboard RBAC automation. Filters and separate dashboards
do not isolate underlying telemetry. `--require-rbac` fails closed on every
command, including Enterprise/Cloud, until edition-specific permission support
has been independently validated; there is no permission-writing code here.

Suggested least-privilege matrix for an administrator to implement and test on a
licensed edition with supported organization/folder/dashboard and stream grants:

| Principal | Dashboard access | Data scope | Mutation rights |
| --- | --- | --- | --- |
| Platform lead | Fleet and approved summaries | Approved fleet aggregates | None |
| Project lead | Workspace and workflow | Authorized project traces | None |
| Developer | Session and workflow | Authorized conversations/projects | None |
| SRE | Telemetry operations | Bridge metrics and approved diagnostics | None |
| Provisioning identity | Five owned definitions in target folder | Read only for validation | Create/update owned dashboards; server may also register distinct values and update stream settings |
| Administrator | Permission administration | Explicit operational need | Grants, audit and emergency recovery |

Enterprise prerequisites include a verified license/edition, documented permission
API support, distinct identities, organization/folder grants, stream/query data
restrictions, denied-access tests across projects, audit evidence and an approved
administrator change. Dashboard view permission alone does not establish row-level
data isolation. Restrict sensitive projects through supported data controls or
separate organizations/streams rather than relying on variable selections.

## Collector example and interpretation

[`collector/otel-collector-openobserve.yaml`](collector/otel-collector-openobserve.yaml)
is an illustrative **Collector Contrib** configuration using OTLP HTTP traces and
Prometheus remote-write metrics. It references secret environment variables and
separate receiver ports; it is not an active station configuration. Validate it
with your installed Collector Contrib version before use. The native OpenAPI
advertises `/api/{org}/v1/traces` and `/api/{org}/prometheus/api/v1/write`.

To add routing to an existing Collector, merge the named exporters and append
them to the existing traces/metrics pipelines, preserving every existing exporter,
processor and receiver. Do not replace the active configuration or send the same
input through both old and new duplicate OpenObserve pipelines. Resource-to-label
conversion preserves `service_name` for the generated metric queries. Verify label
and stream mappings against actual ingestion. This package makes no daemon, hook,
Collector-service or station runtime changes; the user's existing live routing is
outside this template's validation.

Activity counts and latest timestamps show observed activity, not process liveness.
Counter panels display the latest cumulative snapshot for each grouped label set,
not totals accumulated within the selected interval; resets and multiple independent
emitters require separate treatment. Gauge/quota panels are observations at their
latest timestamp. No duration percentiles are inferred from zero-duration hook
spans, and no financial savings or ROI are fabricated. See [SPEC.md](SPEC.md) for
the original generator/schema provenance and [HANDOVER.md](HANDOVER.md) for the
bounded builder's validation and remaining limits.

The integration and ownership decisions are recorded in [ADR-001.md](ADR-001.md).
