# OpenObserve v5 Dashboard Package Specification (`contrib/dashboards/openobserve`)

> **Status**: User-Approved Implementation (Phase 1–3, AGY Native Argon)
> **Target Runtime**: OpenObserve OSS `v1.1.2` (`http://127.0.0.1:5080`, native Dashboard schema `version: 5`)
> **Scope**: `contrib/dashboards/openobserve/` only (`SPEC.md`, `build.mjs`, `manifest.json`, `dashboards/*.json`, `build.test.mjs`, `README.md`)

---

## 1. Live Environment & Schema Facts

Verified against live OpenObserve `v1.1.2` (`/api-doc/openapi.json`, `/api/default/dashboards/7524194371093135360`, `/api/default/streams/default/schema?type=traces`, and `/api/default/streams/<metric>/schema?type=metrics`):

1. **Dashboard Wrapper & Version**:
   - OpenObserve stores dashboards in versioned wrappers (`v1`–`v8`); the native v5 payload is a `Version5Dashboard` (`version: 5`) object.
   - Server assigns `dashboardId` on creation. Package artifacts emit `dashboardId: ""` (or runtime-resolved IDs when passed to the generator) and `owner: ""` (never copying private station owner metadata).
   - Every managed dashboard carries a deterministic identity marker in `description`: `agent-otel-bridge:openobserve:<key>`.

2. **Default Trace Stream (`default`, `stream_type: "traces"`)**:
   - Shared across multiple station applications; every trace SQL query MUST include `WHERE service_name = 'agent-otel-bridge'`.
   - Verified columns & types:
     - `_timestamp` (`Int64`, microseconds since epoch)
     - `service_name` (`Utf8`)
     - `gen_ai_agent_name` (`Utf8`)
     - `gen_ai_conversation_id` (`Utf8`)
     - `workspace_path` (`Utf8`)
     - `workspace_project_name` (`Utf8`)
     - `agent_tool_archetype` (`Utf8`)
     - `agent_hook_event` (`Utf8`)
     - `span_status` (`Utf8`)
     - `trace_id` (`Utf8`)
     - `span_id` (`Utf8`)
     - `duration` (`Int64`)
   - **Zero-Duration Hook Invariant**: Synchronous agent lifecycle hooks emit point-in-time / near-zero duration facts; dashboards MUST NOT use latency percentiles (`PERCENTILE_CONT`, `p50`, `p95`, `p99`) on hook spans.

3. **Metric Streams (`stream_type: "metrics"`) & PromQL / SQL**:
   - Verified streams and fields:
     - `agent_bridge_queue_items`: `_timestamp` (`Int64`), `service_name` (`Utf8`), `value` (`Float64`), `queue` (`Utf8`)
     - `agent_bridge_dropped_total`: `_timestamp` (`Int64`), `service_name` (`Utf8`), `value` (`Float64`), `reason` (`Utf8`)
     - `agent_bridge_export_attempts_total`: `_timestamp` (`Int64`), `service_name` (`Utf8`), `value` (`Float64`), `result` (`Utf8`)
     - `agent_bridge_events_total`: `_timestamp` (`Int64`), `service_name` (`Utf8`), `value` (`Float64`), `stage` (`Utf8`), `outcome` (`Utf8`)
     - `agent_quota_remaining_fraction`: `_timestamp` (`Int64`), `service_name` (`Utf8`), `value` (`Float64`), `gen_ai_provider_name` (`Utf8`), `bucket` (`Utf8`), `group` (`Utf8`)
     - `agent_quota_seconds_to_reset`: `_timestamp` (`Int64`), `service_name` (`Utf8`), `value` (`Float64`), `gen_ai_provider_name` (`Utf8`), `bucket` (`Utf8`), `group` (`Utf8`)
   - Verified read-only query endpoints: SQL (`/api/default/_search?type=metrics`) and PromQL (`/api/default/prometheus/api/v1/query`).

4. **OpenAPI Panel & Layout Constraints**:
   - 192-column grid layout (`layout.w` within `1..=192`, non-overlapping `x`, `y`, `w`, `h`, `i`).
   - `queries[].fields` MUST populate `x`, `y`, and `z` arrays even when `customQuery: true`.
   - `queries[].fields.filter` MUST be `{ "filterType": "group", "logicalOperator": "AND", "conditions": [] }` to match the actual accepted OpenObserve v5 panel filter structure.

5. **Variables, Substitution & Safe Constraints**:
   - Dashboard variables use OpenObserve `type: "query_values"` against `stream: "default"`, `stream_type: "traces"` scoped by `filter: [{ name: "service_name", operator: "=", value: "agent-otel-bridge" }]` and `escapeSingleQuotes: true`.
   - Variables: `workspace_project_name`, `gen_ai_agent_name`, `gen_ai_conversation_id`.
   - SQL queries use quoted variable substitution (`'$workspace_project_name'`, `'$gen_ai_agent_name'`, `'$gen_ai_conversation_id'`) with empty/wildcard bypass (`('$workspace_project_name' IN ('', 'All', '*') OR workspace_project_name = '$workspace_project_name')`).
   - Security note: OpenObserve dashboard variables perform client/template substitution rather than parameterized prepared-statement binding. Using `query_values` variables with `escapeSingleQuotes: true` constrains UI selection to indexed field values, NOT server-side SQL sanitization of arbitrary free-text input.

---

## 2. Five-Level Dashboard Hierarchy & Personas

| Level | Logical Key | File Name | Persona & Purpose | Panels | Primary Navigation Targets |
| :---: | :--- | :--- | :--- | :---: | :--- |
| 1 | `fleet-portfolio` | `01-fleet-portfolio.json` | **Platform / Engineering Lead**: Fleet-wide active workspaces, agent harness distribution, quota headroom, and bridge event volume. | 4 | `workspace-project`, `agent-workflow`, `telemetry-operations` |
| 2 | `workspace-project` | `02-workspace-project.json` | **Tech Lead / Repo Owner**: Workspace-scoped activity by `workspace_project_name` and `workspace_path`, agent turns, tool archetypes, and error statuses. | 4 | `fleet-portfolio`, `agent-workflow`, `session-turn` |
| 3 | `agent-workflow` | `03-agent-workflow.json` | **AI Workflow Engineer**: Harness execution behavior by `gen_ai_agent_name`, `agent_hook_event` lifecycle stages, `agent_tool_archetype` mix, and active conversations. | 4 | `workspace-project`, `session-turn`, `telemetry-operations` |
| 4 | `session-turn` | `04-session-turn.json` | **Developer / Debugger**: Deep dive into a single `gen_ai_conversation_id` turn timeline, ordered spans (`trace_id`, `span_id`, `duration`), hook events, and `span_status`. | 4 | `workspace-project`, `agent-workflow`, `telemetry-operations` |
| 5 | `telemetry-operations` | `05-telemetry-operations.json` | **SRE / Bridge Operator**: Daemon pipeline health (`agent_bridge_queue_items`, `agent_bridge_dropped_total`, `agent_bridge_export_attempts_total`, `agent_bridge_events_total` by `stage`/`outcome`) and provider quota reset telemetry (`agent_quota_remaining_fraction`, `agent_quota_seconds_to_reset`). | 5 | `fleet-portfolio`, `agent-workflow`, `session-turn` |

---

## 3. Acceptance Criteria

1. `build.mjs` exports deterministic generator functions (`buildDashboards`, `buildDashboard`, `buildManifest`, `DASHBOARD_LEVELS`, `TRACE_FIELDS`, `METRIC_STREAMS`, `formatDashboardMarker`, `buildDashboardLinkUrl`) and writes `manifest.json` + `dashboards/*.json` when executed as CLI.
2. All 5 dashboards conform to OpenObserve v5 structure (`version: 5`, 3–5 panels, 192-column layout, populated `x`/`y`/`z`, `fields.filter` `{filterType:"group",logicalOperator:"AND",conditions:[]}`, no private `owner`, description marker `agent-otel-bridge:openobserve:<key>`).
3. All trace SQL queries enforce `service_name = 'agent-otel-bridge'` and avoid latency percentiles.
4. Runtime dashboard ID map (`dashboardIds`) resolves cross-dashboard drilldown targets and navigation URLs while carrying over time (`from`, `to`, `period`) and filter variables (`workspace_project_name`, `gen_ai_agent_name`, `gen_ai_conversation_id`).
5. `build.test.mjs` verifies artifact parity, all 5 levels, trace scoping, synthetic query contracts, navigation/drilldown resolution, and metric fields via `node --test`.
