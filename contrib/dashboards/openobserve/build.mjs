import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const MARKER_PREFIX = "agent-otel-bridge:openobserve:";
export const SERVICE_NAME = "agent-otel-bridge";
export const TRACE_STREAM = "default";
export const DEFAULT_CREATED_AT = "2026-11-05T00:00:00Z";
export const GRID_COLUMNS = 192;

export const TRACE_FIELDS = Object.freeze([
  "_timestamp",
  "service_name",
  "gen_ai_agent_name",
  "gen_ai_conversation_id",
  "workspace_path",
  "workspace_project_name",
  "agent_tool_archetype",
  "agent_hook_event",
  "span_status",
  "trace_id",
  "span_id",
  "duration",
]);

export const METRIC_STREAMS = Object.freeze({
  agent_bridge_queue_items: {
    stream: "agent_bridge_queue_items",
    stream_type: "metrics",
    fields: ["_timestamp", "service_name", "host_name", "service_instance_id", "value", "queue"],
    description: "Current items queued in the background daemon micro-batcher.",
  },
  agent_bridge_dropped_total: {
    stream: "agent_bridge_dropped_total",
    stream_type: "metrics",
    fields: ["_timestamp", "service_name", "host_name", "service_instance_id", "value", "reason"],
    description: "Total telemetry items dropped by backpressure or framing guardrails.",
  },
  agent_bridge_export_attempts_total: {
    stream: "agent_bridge_export_attempts_total",
    stream_type: "metrics",
    fields: ["_timestamp", "service_name", "host_name", "service_instance_id", "value", "result"],
    description: "Total OTLP export attempts grouped by result.",
  },
  agent_bridge_events_total: {
    stream: "agent_bridge_events_total",
    stream_type: "metrics",
    fields: ["_timestamp", "service_name", "host_name", "service_instance_id", "value", "stage", "outcome"],
    description: "Total bridge events processed across daemon pipeline stages and outcomes.",
  },
  agent_quota_remaining_fraction: {
    stream: "agent_quota_remaining_fraction",
    stream_type: "metrics",
    fields: ["_timestamp", "service_name", "host_name", "service_instance_id", "value", "gen_ai_provider_name", "bucket", "group"],
    description: "Remaining provider quota headroom fraction (0.0 to 1.0).",
  },
  agent_quota_seconds_to_reset: {
    stream: "agent_quota_seconds_to_reset",
    stream_type: "metrics",
    fields: ["_timestamp", "service_name", "host_name", "service_instance_id", "value", "gen_ai_provider_name", "bucket", "group"],
    description: "Seconds remaining until provider quota window reset.",
  },
});

export const DASHBOARD_VARIABLE_SPECS = Object.freeze([
  {
    name: "workspace_project_name",
    label: "Workspace Project",
    field: "workspace_project_name",
  },
  {
    name: "gen_ai_agent_name",
    label: "Agent Name",
    field: "gen_ai_agent_name",
  },
  {
    name: "gen_ai_conversation_id",
    label: "Conversation ID",
    field: "gen_ai_conversation_id",
  },
]);

export const DASHBOARD_LEVELS = Object.freeze([
  {
    level: 1,
    key: "fleet-portfolio",
    fileName: "01-fleet-portfolio.json",
    title: "Agent Telemetry L1: Fleet Portfolio",
    persona: "Platform / Engineering Lead",
    summary:
      "Fleet-wide portfolio view of active workspaces, AI agent harnesses, provider quota headroom, and bridge event volume.",
    navigationTargets: ["workspace-project", "agent-workflow", "telemetry-operations"],
    variables: ["workspace_project_name", "gen_ai_agent_name"],
  },
  {
    level: 2,
    key: "workspace-project",
    fileName: "02-workspace-project.json",
    title: "Agent Telemetry L2: Workspace Project",
    persona: "Tech Lead / Repository Owner",
    summary:
      "Workspace-scoped execution breakdown across project roots, active agents, behavioral tool archetypes, and span statuses.",
    navigationTargets: ["fleet-portfolio", "agent-workflow", "session-turn"],
    variables: ["workspace_project_name", "gen_ai_agent_name", "gen_ai_conversation_id"],
  },
  {
    level: 3,
    key: "agent-workflow",
    fileName: "03-agent-workflow.json",
    title: "Agent Telemetry L3: Agent Workflow",
    persona: "AI Workflow Engineer",
    summary:
      "Agent harness lifecycle analysis across hook events, behavioral tool archetypes, and conversation activity.",
    navigationTargets: ["workspace-project", "session-turn", "telemetry-operations"],
    variables: ["workspace_project_name", "gen_ai_agent_name", "gen_ai_conversation_id"],
  },
  {
    level: 4,
    key: "session-turn",
    fileName: "04-session-turn.json",
    title: "Agent Telemetry L4: Session Turn",
    persona: "Developer / Session Debugger",
    summary:
      "Single-conversation turn timeline inspecting ordered trace/span IDs, hook events, tool archetypes, and recorded span duration facts.",
    navigationTargets: ["workspace-project", "agent-workflow", "telemetry-operations"],
    variables: ["workspace_project_name", "gen_ai_agent_name", "gen_ai_conversation_id"],
  },
  {
    level: 5,
    key: "telemetry-operations",
    fileName: "05-telemetry-operations.json",
    title: "Agent Telemetry L5: Telemetry Operations",
    persona: "SRE / Bridge Operator",
    summary:
      "Daemon pipeline health covering queue depth, dropped items, OTLP export attempts, stage/outcome event counters, and quota reset clocks.",
    navigationTargets: ["fleet-portfolio", "agent-workflow", "session-turn"],
    variables: ["workspace_project_name", "gen_ai_agent_name"],
  },
]);

export function formatDashboardMarker(key) {
  if (!key || typeof key !== "string") {
    throw new Error("Dashboard logical key must be a non-empty string.");
  }
  return `${MARKER_PREFIX}${key}`;
}

export function extractDashboardKey(description) {
  if (typeof description !== "string") return null;
  const match = description.match(/agent-otel-bridge:openobserve:([a-z0-9-]+)/);
  return match ? match[1] : null;
}

export function resolveDashboardId(targetKey, dashboardIds = {}) {
  if (dashboardIds && typeof dashboardIds[targetKey] === "string" && dashboardIds[targetKey].trim() !== "") {
    return dashboardIds[targetKey].trim();
  }
  return `\${dashboard_id:${targetKey}}`;
}

export function buildDashboardLinkUrl(targetKey, options = {}) {
  const orgIdentifier = options.orgIdentifier || "default";
  const folder = options.folder || "default";
  const tab = options.tab || "default";
  const resolvedId = resolveDashboardId(targetKey, options.dashboardIds);
  const queryParams = [
    `org_identifier=${encodeURIComponent(orgIdentifier)}`,
    `dashboard=${encodeURIComponent(resolvedId)}`,
    `folder=${encodeURIComponent(folder)}`,
    `tab=${encodeURIComponent(tab)}`,
    "from=$__from",
    "to=$__to",
    "period=$__period",
    "var-workspace_project_name=$workspace_project_name",
    "var-gen_ai_agent_name=$gen_ai_agent_name",
    "var-gen_ai_conversation_id=$gen_ai_conversation_id",
  ];
  return `/web/dashboards/view?${queryParams.join("&")}`;
}

function makeAxisItem(column, label = column, alias = column) {
  return {
    label,
    alias,
    column,
    color: null,
  };
}

function makePanelFilter() {
  return {
    filterType: "group",
    logicalOperator: "AND",
    conditions: [],
  };
}

function makeVariable(spec) {
  return {
    name: spec.name,
    label: spec.label,
    type: "query_values",
    query_data: {
      stream: TRACE_STREAM,
      stream_type: "traces",
      field: spec.field,
      max_record_size: 1000,
      filter: [
        {
          name: "service_name",
          operator: "=",
          value: SERVICE_NAME,
        },
      ],
    },
    value: "",
    options: [],
    multiSelect: false,
    customMultiSelectValue: [],
    escapeSingleQuotes: true,
    hideOnDashboard: false,
  };
}

function makeDrilldown(targetKey, options = {}) {
  const levelSpec = DASHBOARD_LEVELS.find((l) => l.key === targetKey);
  const resolvedId = resolveDashboardId(targetKey, options.dashboardIds);
  const folder = options.folder || "default";
  return {
    name: levelSpec ? `Navigate to ${levelSpec.title}` : `Navigate to ${targetKey}`,
    type: "dashboard",
    targetBlank: false,
    findBy: "",
    data: {
      dashboard: resolvedId,
      folder,
      tab: "default",
      url: buildDashboardLinkUrl(targetKey, options),
      passAllVariables: true,
      variables: [
        { name: "workspace_project_name", value: "$workspace_project_name" },
        { name: "gen_ai_agent_name", value: "$gen_ai_agent_name" },
        { name: "gen_ai_conversation_id", value: "$gen_ai_conversation_id" },
      ],
    },
  };
}

function makePanel({
  id,
  type = "table",
  title,
  description = "",
  queryType = "sql",
  query,
  stream,
  streamType,
  xColumns,
  yColumns,
  zColumns = [],
  layout,
  drilldownTargets = [],
  promqlLegend = "",
  options = {},
}) {
  return {
    id,
    type,
    title,
    description,
    config: {
      show_legends: true,
      legends_position: null,
      base_map: null,
      map_view: null,
      no_value_replacement: "N/A",
      drilldown: drilldownTargets.map((targetKey) => makeDrilldown(targetKey, options)),
    },
    queryType,
    queries: [
      {
        query,
        vrlFunctionQuery: "",
        customQuery: true,
        fields: {
          stream,
          stream_type: streamType,
          x: xColumns.map((col) =>
            typeof col === "string" ? makeAxisItem(col) : makeAxisItem(col.column, col.label, col.alias)
          ),
          y: yColumns.map((col) =>
            typeof col === "string" ? makeAxisItem(col) : makeAxisItem(col.column, col.label, col.alias)
          ),
          z: zColumns.map((col) =>
            typeof col === "string" ? makeAxisItem(col) : makeAxisItem(col.column, col.label, col.alias)
          ),
          breakdown: [],
          filter: makePanelFilter(),
        },
        config: {
          promql_legend: promqlLegend,
        },
      },
    ],
    layout,
    htmlContent: "",
    markdownContent: "",
    customChartContent: "",
  };
}

function buildPanelsForLevel(key, options = {}) {
  const wsFilter = "('$workspace_project_name' IN ('', 'All', '*') OR workspace_project_name = '$workspace_project_name')";
  const agentFilter = "('$gen_ai_agent_name' IN ('', 'All', '*') OR gen_ai_agent_name = '$gen_ai_agent_name')";
  const convFilter = "('$gen_ai_conversation_id' IN ('', 'All', '*') OR gen_ai_conversation_id = '$gen_ai_conversation_id')";

  switch (key) {
    case "fleet-portfolio":
      return [
        makePanel({
          id: "fleet-portfolio-workspaces",
          type: "table",
          title: "Active Workspaces & Agent Sessions",
          description:
            "Fleet summary of workspace projects, active agent harnesses, conversation count, and latest event timestamp.",
          stream: TRACE_STREAM,
          streamType: "traces",
          query: `SELECT COALESCE(workspace_project_name, 'unscoped') AS workspace_project_name, COUNT(DISTINCT gen_ai_agent_name) AS active_agents, COUNT(DISTINCT gen_ai_conversation_id) AS conversations, COUNT(span_id) AS hook_events, MAX(_timestamp) AS latest_timestamp FROM "default" WHERE service_name = 'agent-otel-bridge' AND ${wsFilter} AND ${agentFilter} GROUP BY workspace_project_name ORDER BY hook_events DESC`,
          xColumns: ["workspace_project_name"],
          yColumns: ["active_agents", "conversations", "hook_events", "latest_timestamp"],
          layout: { x: 0, y: 0, w: 96, h: 8, i: 1 },
          drilldownTargets: ["workspace-project", "agent-workflow"],
          options,
        }),
        makePanel({
          id: "fleet-portfolio-agents",
          type: "bar",
          title: "Fleet Event Volume by Agent Harness",
          description:
            "Distribution of synchronous hook events and distinct traces across installed AI agent harnesses.",
          stream: TRACE_STREAM,
          streamType: "traces",
          query: `SELECT COALESCE(gen_ai_agent_name, 'unknown') AS gen_ai_agent_name, COUNT(DISTINCT trace_id) AS traces, COUNT(span_id) AS span_events FROM "default" WHERE service_name = 'agent-otel-bridge' AND ${wsFilter} AND ${agentFilter} GROUP BY gen_ai_agent_name ORDER BY span_events DESC`,
          xColumns: ["gen_ai_agent_name"],
          yColumns: ["traces", "span_events"],
          layout: { x: 96, y: 0, w: 96, h: 8, i: 2 },
          drilldownTargets: ["agent-workflow"],
          options,
        }),
        makePanel({
          id: "fleet-portfolio-quota",
          type: "table",
          title: "Provider Quota Remaining Fraction",
          description: "Latest remaining quota fraction per AI provider and bucket.",
          stream: "agent_quota_remaining_fraction",
          streamType: "metrics",
          query: `SELECT COALESCE(host_name, 'unknown') AS host_name, COALESCE(service_instance_id, 'unknown') AS service_instance_id, COALESCE(gen_ai_provider_name, 'unknown') AS gen_ai_provider_name, COALESCE("group", 'unknown') AS quota_group, COALESCE(bucket, 'unknown') AS bucket, last_value(value ORDER BY _timestamp) AS value, MAX(_timestamp) AS latest_timestamp FROM "agent_quota_remaining_fraction" WHERE service_name = 'agent-otel-bridge' GROUP BY host_name, service_instance_id, gen_ai_provider_name, "group", bucket ORDER BY gen_ai_provider_name`,
          xColumns: ["host_name", "service_instance_id", "gen_ai_provider_name", "quota_group", "bucket"],
          yColumns: ["value", "latest_timestamp"],
          layout: { x: 0, y: 8, w: 96, h: 8, i: 3 },
          drilldownTargets: ["telemetry-operations"],
          options,
        }),
        makePanel({
          id: "fleet-portfolio-pipeline",
          type: "table",
          title: "Bridge Pipeline Events by Stage & Outcome",
          description: "Fleet-wide daemon event throughput segmented by processing stage and outcome.",
          stream: "agent_bridge_events_total",
          streamType: "metrics",
          query: `SELECT COALESCE(host_name, 'unknown') AS host_name, COALESCE(service_instance_id, 'unknown') AS service_instance_id, stage, outcome, last_value(value ORDER BY _timestamp) AS value, MAX(_timestamp) AS latest_timestamp FROM "agent_bridge_events_total" WHERE service_name = 'agent-otel-bridge' GROUP BY host_name, service_instance_id, stage, outcome ORDER BY stage, outcome`,
          xColumns: ["host_name", "service_instance_id", "stage", "outcome"],
          yColumns: ["value", "latest_timestamp"],
          layout: { x: 96, y: 8, w: 96, h: 8, i: 4 },
          drilldownTargets: ["telemetry-operations"],
          options,
        }),
      ];

    case "workspace-project":
      return [
        makePanel({
          id: "workspace-project-paths",
          type: "table",
          title: "Workspace Paths & Project Activity",
          description: "Activity across workspace_project_name and workspace_path roots.",
          stream: TRACE_STREAM,
          streamType: "traces",
          query: `SELECT COALESCE(workspace_project_name, 'unscoped') AS workspace_project_name, COALESCE(workspace_path, 'unknown') AS workspace_path, COUNT(DISTINCT gen_ai_conversation_id) AS conversations, COUNT(span_id) AS span_events, MAX(_timestamp) AS latest_timestamp FROM "default" WHERE service_name = 'agent-otel-bridge' AND ${wsFilter} AND ${agentFilter} AND ${convFilter} GROUP BY workspace_project_name, workspace_path ORDER BY span_events DESC`,
          xColumns: ["workspace_project_name", "workspace_path"],
          yColumns: ["conversations", "span_events", "latest_timestamp"],
          layout: { x: 0, y: 0, w: 192, h: 8, i: 1 },
          drilldownTargets: ["agent-workflow", "session-turn"],
          options,
        }),
        makePanel({
          id: "workspace-project-agents",
          type: "bar",
          title: "Workspace Turns by Agent Harness",
          description: "Agent harness invocation count and active conversations inside the selected workspace.",
          stream: TRACE_STREAM,
          streamType: "traces",
          query: `SELECT COALESCE(gen_ai_agent_name, 'unknown') AS gen_ai_agent_name, COUNT(DISTINCT gen_ai_conversation_id) AS conversations, COUNT(span_id) AS span_events FROM "default" WHERE service_name = 'agent-otel-bridge' AND ${wsFilter} AND ${agentFilter} AND ${convFilter} GROUP BY gen_ai_agent_name ORDER BY span_events DESC`,
          xColumns: ["gen_ai_agent_name"],
          yColumns: ["conversations", "span_events"],
          layout: { x: 0, y: 8, w: 64, h: 8, i: 2 },
          drilldownTargets: ["agent-workflow"],
          options,
        }),
        makePanel({
          id: "workspace-project-archetypes",
          type: "bar",
          title: "Workspace Tool Archetype Observed Spans",
          description: "Observed spans/events across preference-agnostic behavioral archetypes in workspace tool hooks.",
          stream: TRACE_STREAM,
          streamType: "traces",
          query: `SELECT COALESCE(agent_tool_archetype, 'non_tool') AS agent_tool_archetype, COUNT(span_id) AS span_events FROM "default" WHERE service_name = 'agent-otel-bridge' AND ${wsFilter} AND ${agentFilter} AND ${convFilter} GROUP BY agent_tool_archetype ORDER BY span_events DESC`,
          xColumns: ["agent_tool_archetype"],
          yColumns: ["span_events"],
          layout: { x: 64, y: 8, w: 64, h: 8, i: 3 },
          drilldownTargets: ["agent-workflow"],
          options,
        }),
        makePanel({
          id: "workspace-project-status",
          type: "table",
          title: "Workspace Span Status Breakdown",
          description: "Hook span status distribution for quick anomaly detection in the workspace.",
          stream: TRACE_STREAM,
          streamType: "traces",
          query: `SELECT COALESCE(span_status, 'UNSET') AS span_status, COUNT(span_id) AS span_count, COUNT(DISTINCT trace_id) AS trace_count FROM "default" WHERE service_name = 'agent-otel-bridge' AND ${wsFilter} AND ${agentFilter} AND ${convFilter} GROUP BY span_status ORDER BY span_count DESC`,
          xColumns: ["span_status"],
          yColumns: ["span_count", "trace_count"],
          layout: { x: 128, y: 8, w: 64, h: 8, i: 4 },
          drilldownTargets: ["session-turn", "fleet-portfolio"],
          options,
        }),
      ];

    case "agent-workflow":
      return [
        makePanel({
          id: "agent-workflow-hooks",
          type: "bar",
          title: "Lifecycle Hook Events by Agent",
          description: "Frequency of synchronous lifecycle hook events across AI agent harnesses.",
          stream: TRACE_STREAM,
          streamType: "traces",
          query: `SELECT COALESCE(gen_ai_agent_name, 'unknown') AS gen_ai_agent_name, COALESCE(agent_hook_event, 'unspecified') AS agent_hook_event, COUNT(span_id) AS event_count FROM "default" WHERE service_name = 'agent-otel-bridge' AND ${wsFilter} AND ${agentFilter} AND ${convFilter} GROUP BY gen_ai_agent_name, agent_hook_event ORDER BY event_count DESC`,
          xColumns: ["gen_ai_agent_name", "agent_hook_event"],
          yColumns: ["event_count"],
          layout: { x: 0, y: 0, w: 96, h: 8, i: 1 },
          drilldownTargets: ["session-turn"],
          options,
        }),
        makePanel({
          id: "agent-workflow-archetypes",
          type: "table",
          title: "Behavioral Tool Archetype Matrix",
          description:
            "Observed spans/events across FilterCompressor, StructuredParser, InspectorDiff, SearchRetrieval, BuildTestVerify, and GenericExec.",
          stream: TRACE_STREAM,
          streamType: "traces",
          query: `SELECT COALESCE(gen_ai_agent_name, 'unknown') AS gen_ai_agent_name, COALESCE(agent_tool_archetype, 'non_tool') AS agent_tool_archetype, COUNT(span_id) AS span_events, COUNT(DISTINCT gen_ai_conversation_id) AS conversations FROM "default" WHERE service_name = 'agent-otel-bridge' AND ${wsFilter} AND ${agentFilter} AND ${convFilter} GROUP BY gen_ai_agent_name, agent_tool_archetype ORDER BY span_events DESC`,
          xColumns: ["gen_ai_agent_name", "agent_tool_archetype"],
          yColumns: ["span_events", "conversations"],
          layout: { x: 96, y: 0, w: 96, h: 8, i: 2 },
          drilldownTargets: ["session-turn"],
          options,
        }),
        makePanel({
          id: "agent-workflow-conversations",
          type: "table",
          title: "Active Agent Conversations",
          description:
            "Recent agent conversations with workspace project context, span counts, and latest activity timestamp.",
          stream: TRACE_STREAM,
          streamType: "traces",
          query: `SELECT COALESCE(gen_ai_conversation_id, 'untracked') AS gen_ai_conversation_id, COALESCE(gen_ai_agent_name, 'unknown') AS gen_ai_agent_name, COALESCE(workspace_project_name, 'unscoped') AS workspace_project_name, COUNT(span_id) AS span_events, MAX(_timestamp) AS latest_timestamp FROM "default" WHERE service_name = 'agent-otel-bridge' AND ${wsFilter} AND ${agentFilter} AND ${convFilter} GROUP BY gen_ai_conversation_id, gen_ai_agent_name, workspace_project_name ORDER BY latest_timestamp DESC LIMIT 50`,
          xColumns: ["gen_ai_conversation_id", "gen_ai_agent_name", "workspace_project_name"],
          yColumns: ["span_events", "latest_timestamp"],
          layout: { x: 0, y: 8, w: 128, h: 8, i: 3 },
          drilldownTargets: ["session-turn", "workspace-project"],
          options,
        }),
        makePanel({
          id: "agent-workflow-status",
          type: "table",
          title: "Workflow Hook Status by Event Type",
          description: "Status outcome counts per agent_hook_event and span_status.",
          stream: TRACE_STREAM,
          streamType: "traces",
          query: `SELECT COALESCE(agent_hook_event, 'unspecified') AS agent_hook_event, COALESCE(span_status, 'UNSET') AS span_status, COUNT(span_id) AS span_count FROM "default" WHERE service_name = 'agent-otel-bridge' AND ${wsFilter} AND ${agentFilter} AND ${convFilter} GROUP BY agent_hook_event, span_status ORDER BY span_count DESC`,
          xColumns: ["agent_hook_event", "span_status"],
          yColumns: ["span_count"],
          layout: { x: 128, y: 8, w: 64, h: 8, i: 4 },
          drilldownTargets: ["telemetry-operations"],
          options,
        }),
      ];

    case "session-turn":
      return [
        makePanel({
          id: "session-turn-timeline",
          type: "table",
          title: "Session Turn Span Timeline",
          description:
            "Chronological trace and span timeline for the selected conversation (duration recorded as raw Int64 fact without percentile aggregation).",
          stream: TRACE_STREAM,
          streamType: "traces",
          query: `SELECT _timestamp, trace_id, span_id, COALESCE(gen_ai_conversation_id, 'untracked') AS gen_ai_conversation_id, COALESCE(gen_ai_agent_name, 'unknown') AS gen_ai_agent_name, COALESCE(workspace_project_name, 'unscoped') AS workspace_project_name, COALESCE(workspace_path, 'unknown') AS workspace_path, COALESCE(agent_hook_event, 'unspecified') AS agent_hook_event, COALESCE(agent_tool_archetype, 'non_tool') AS agent_tool_archetype, COALESCE(span_status, 'UNSET') AS span_status, duration FROM "default" WHERE service_name = 'agent-otel-bridge' AND ${wsFilter} AND ${agentFilter} AND ${convFilter} ORDER BY _timestamp DESC LIMIT 200`,
          xColumns: [
            "_timestamp",
            "trace_id",
            "span_id",
            "gen_ai_conversation_id",
            "gen_ai_agent_name",
            "workspace_project_name",
            "workspace_path",
            "agent_hook_event",
            "agent_tool_archetype",
            "span_status",
          ],
          yColumns: ["duration"],
          layout: { x: 0, y: 0, w: 192, h: 9, i: 1 },
          drilldownTargets: ["agent-workflow", "workspace-project"],
          options,
        }),
        makePanel({
          id: "session-turn-hooks",
          type: "bar",
          title: "Turn Hook Event Sequence Counts",
          description: "Count of hook lifecycle events within the filtered session scope.",
          stream: TRACE_STREAM,
          streamType: "traces",
          query: `SELECT COALESCE(agent_hook_event, 'unspecified') AS agent_hook_event, COUNT(span_id) AS span_count FROM "default" WHERE service_name = 'agent-otel-bridge' AND ${wsFilter} AND ${agentFilter} AND ${convFilter} GROUP BY agent_hook_event ORDER BY span_count DESC`,
          xColumns: ["agent_hook_event"],
          yColumns: ["span_count"],
          layout: { x: 0, y: 9, w: 64, h: 7, i: 2 },
          drilldownTargets: ["agent-workflow"],
          options,
        }),
        makePanel({
          id: "session-turn-archetypes",
          type: "bar",
          title: "Turn Tool Archetype Mix",
          description: "Observed spans/events by tool archetype during the session.",
          stream: TRACE_STREAM,
          streamType: "traces",
          query: `SELECT COALESCE(agent_tool_archetype, 'non_tool') AS agent_tool_archetype, COUNT(span_id) AS span_count FROM "default" WHERE service_name = 'agent-otel-bridge' AND ${wsFilter} AND ${agentFilter} AND ${convFilter} GROUP BY agent_tool_archetype ORDER BY span_count DESC`,
          xColumns: ["agent_tool_archetype"],
          yColumns: ["span_count"],
          layout: { x: 64, y: 9, w: 64, h: 7, i: 3 },
          drilldownTargets: ["agent-workflow"],
          options,
        }),
        makePanel({
          id: "session-turn-traces",
          type: "table",
          title: "Session W3C Trace Summary",
          description: "Distinct W3C trace_id roots, span counts, and status summary in the session.",
          stream: TRACE_STREAM,
          streamType: "traces",
          query: `SELECT trace_id, COALESCE(span_status, 'UNSET') AS span_status, COUNT(span_id) AS spans, MIN(_timestamp) AS start_timestamp, MAX(_timestamp) AS end_timestamp FROM "default" WHERE service_name = 'agent-otel-bridge' AND ${wsFilter} AND ${agentFilter} AND ${convFilter} GROUP BY trace_id, span_status ORDER BY end_timestamp DESC LIMIT 50`,
          xColumns: ["trace_id", "span_status"],
          yColumns: ["spans", "start_timestamp", "end_timestamp"],
          layout: { x: 128, y: 9, w: 64, h: 7, i: 4 },
          drilldownTargets: ["telemetry-operations"],
          options,
        }),
      ];

    case "telemetry-operations":
      return [
        makePanel({
          id: "telemetry-ops-queue",
          type: "table",
          title: "Daemon Micro-Batcher Queue Depth",
          description: "Current queued items reported by agent_bridge_queue_items.",
          stream: "agent_bridge_queue_items",
          streamType: "metrics",
          query: `SELECT COALESCE(host_name, 'unknown') AS host_name, COALESCE(service_instance_id, 'unknown') AS service_instance_id, COALESCE(queue, 'default') AS queue, last_value(value ORDER BY _timestamp) AS value, MAX(_timestamp) AS latest_timestamp FROM "agent_bridge_queue_items" WHERE service_name = 'agent-otel-bridge' GROUP BY host_name, service_instance_id, queue ORDER BY queue`,
          xColumns: ["host_name", "service_instance_id", "queue"],
          yColumns: ["value", "latest_timestamp"],
          layout: { x: 0, y: 0, w: 64, h: 7, i: 1 },
          drilldownTargets: ["fleet-portfolio"],
          options,
        }),
        makePanel({
          id: "telemetry-ops-dropped",
          type: "table",
          title: "Dropped Telemetry Items by Reason",
          description: "Cumulative dropped telemetry items from agent_bridge_dropped_total.",
          stream: "agent_bridge_dropped_total",
          streamType: "metrics",
          query: `SELECT COALESCE(host_name, 'unknown') AS host_name, COALESCE(service_instance_id, 'unknown') AS service_instance_id, COALESCE(reason, 'none') AS reason, last_value(value ORDER BY _timestamp) AS value, MAX(_timestamp) AS latest_timestamp FROM "agent_bridge_dropped_total" WHERE service_name = 'agent-otel-bridge' GROUP BY host_name, service_instance_id, reason ORDER BY value DESC`,
          xColumns: ["host_name", "service_instance_id", "reason"],
          yColumns: ["value", "latest_timestamp"],
          layout: { x: 64, y: 0, w: 64, h: 7, i: 2 },
          drilldownTargets: ["fleet-portfolio"],
          options,
        }),
        makePanel({
          id: "telemetry-ops-exports",
          type: "table",
          title: "OTLP Export Attempts by Result",
          description: "Cumulative export attempts from agent_bridge_export_attempts_total.",
          stream: "agent_bridge_export_attempts_total",
          streamType: "metrics",
          query: `SELECT COALESCE(host_name, 'unknown') AS host_name, COALESCE(service_instance_id, 'unknown') AS service_instance_id, COALESCE(result, 'unknown') AS result, last_value(value ORDER BY _timestamp) AS value, MAX(_timestamp) AS latest_timestamp FROM "agent_bridge_export_attempts_total" WHERE service_name = 'agent-otel-bridge' GROUP BY host_name, service_instance_id, result ORDER BY value DESC`,
          xColumns: ["host_name", "service_instance_id", "result"],
          yColumns: ["value", "latest_timestamp"],
          layout: { x: 128, y: 0, w: 64, h: 7, i: 3 },
          drilldownTargets: ["fleet-portfolio"],
          options,
        }),
        makePanel({
          id: "telemetry-ops-events",
          type: "table",
          title: "Pipeline Events Total by Stage & Outcome",
          description: "Daemon event counters across stage and outcome dimensions from agent_bridge_events_total.",
          stream: "agent_bridge_events_total",
          streamType: "metrics",
          query: `SELECT COALESCE(host_name, 'unknown') AS host_name, COALESCE(service_instance_id, 'unknown') AS service_instance_id, stage, outcome, last_value(value ORDER BY _timestamp) AS value, MAX(_timestamp) AS latest_timestamp FROM "agent_bridge_events_total" WHERE service_name = 'agent-otel-bridge' GROUP BY host_name, service_instance_id, stage, outcome ORDER BY stage, outcome`,
          xColumns: ["host_name", "service_instance_id", "stage", "outcome"],
          yColumns: ["value", "latest_timestamp"],
          layout: { x: 0, y: 7, w: 96, h: 8, i: 4 },
          drilldownTargets: ["agent-workflow", "session-turn"],
          options,
        }),
        makePanel({
          id: "telemetry-ops-quota-reset",
          type: "table",
          title: "Provider Quota Headroom & Seconds to Reset",
          description:
            "Provider quota remaining fraction alongside quota reset countdown from agent_quota_seconds_to_reset.",
          stream: "agent_quota_seconds_to_reset",
          streamType: "metrics",
          query: `SELECT COALESCE(host_name, 'unknown') AS host_name, COALESCE(service_instance_id, 'unknown') AS service_instance_id, COALESCE(gen_ai_provider_name, 'unknown') AS gen_ai_provider_name, COALESCE("group", 'unknown') AS quota_group, COALESCE(bucket, 'unknown') AS bucket, last_value(value ORDER BY _timestamp) AS value, MAX(_timestamp) AS latest_timestamp FROM "agent_quota_seconds_to_reset" WHERE service_name = 'agent-otel-bridge' GROUP BY host_name, service_instance_id, gen_ai_provider_name, "group", bucket ORDER BY gen_ai_provider_name, bucket`,
          xColumns: ["host_name", "service_instance_id", "gen_ai_provider_name", "quota_group", "bucket"],
          yColumns: ["value", "latest_timestamp"],
          layout: { x: 96, y: 7, w: 96, h: 8, i: 5 },
          drilldownTargets: ["fleet-portfolio"],
          options,
        }),
      ];

    default:
      throw new Error(`Unknown dashboard level key: ${key}`);
  }
}

export function buildDashboard(key, options = {}) {
  const levelSpec = DASHBOARD_LEVELS.find((l) => l.key === key);
  if (!levelSpec) {
    throw new Error(`Unsupported dashboard level key: ${key}`);
  }

  const dashboardId =
    options.dashboardIds && typeof options.dashboardIds[key] === "string"
      ? options.dashboardIds[key]
      : "";
  const created = options.created || DEFAULT_CREATED_AT;
  const marker = formatDashboardMarker(key);
  const navLinks = levelSpec.navigationTargets.map((targetKey) => ({
    targetKey,
    url: buildDashboardLinkUrl(targetKey, options),
  }));

  const description = `${marker} | Persona: ${levelSpec.persona}. ${levelSpec.summary} Links: ${navLinks
    .map((l) => `${l.targetKey}=${l.url}`)
    .join(" ")}`;

  const variableSpecs = DASHBOARD_VARIABLE_SPECS;

  return {
    version: 5,
    dashboardId,
    title: levelSpec.title,
    description,
    role: "",
    owner: "",
    created,
    tabs: [
      {
        tabId: "default",
        name: "Default",
        panels: buildPanelsForLevel(key, options),
      },
    ],
    variables: {
      list: variableSpecs.map(makeVariable),
      showDynamicFilters: true,
    },
    defaultDatetimeDuration: {
      type: "relative",
      relativeTimePeriod: "1h",
    },
  };
}

export function buildDashboards(options = {}) {
  const result = {};
  for (const levelSpec of DASHBOARD_LEVELS) {
    result[levelSpec.key] = buildDashboard(levelSpec.key, options);
  }
  return result;
}

export function buildManifest(options = {}) {
  const dashboards = buildDashboards(options);
  return {
    package: "agent-otel-bridge-openobserve-dashboards",
    schemaVersion: 1,
    openobserveDashboardVersion: 5,
    targetRuntime: "openobserve-v1.1.2",
    serviceName: SERVICE_NAME,
    markerPrefix: MARKER_PREFIX,
    gridColumns: GRID_COLUMNS,
    traceStream: {
      name: TRACE_STREAM,
      type: "traces",
      requiredFilter: `service_name = '${SERVICE_NAME}'`,
      fields: TRACE_FIELDS,
      zeroDurationHookInvariant: true,
    },
    metricStreams: METRIC_STREAMS,
    variables: DASHBOARD_VARIABLE_SPECS.map((v) => ({
      ...v,
      type: "query_values",
      escapeSingleQuotes: true,
      substitutionPattern: `'$${v.name}'`,
    })),
    dashboards: DASHBOARD_LEVELS.map((levelSpec) => {
      const dash = dashboards[levelSpec.key];
      const panels = dash.tabs[0].panels;
      return {
        level: levelSpec.level,
        key: levelSpec.key,
        marker: formatDashboardMarker(levelSpec.key),
        fileName: levelSpec.fileName,
        path: `dashboards/${levelSpec.fileName}`,
        title: levelSpec.title,
        persona: levelSpec.persona,
        summary: levelSpec.summary,
        panelCount: panels.length,
        panelIds: panels.map((p) => p.id),
        navigationTargets: levelSpec.navigationTargets,
        resolvedLinks: Object.fromEntries(
          levelSpec.navigationTargets.map((targetKey) => [
            targetKey,
            buildDashboardLinkUrl(targetKey, options),
          ])
        ),
      };
    }),
  };
}

export function writeArtifacts(
  baseDir = path.dirname(fileURLToPath(import.meta.url)),
  options = {}
) {
  const dashboardsDir = path.join(baseDir, "dashboards");
  fs.mkdirSync(dashboardsDir, { recursive: true });

  const dashboards = buildDashboards(options);
  for (const levelSpec of DASHBOARD_LEVELS) {
    const filePath = path.join(dashboardsDir, levelSpec.fileName);
    const content = JSON.stringify(dashboards[levelSpec.key], null, 2) + "\n";
    fs.writeFileSync(filePath, content, "utf8");
  }

  const manifest = buildManifest(options);
  const manifestPath = path.join(baseDir, "manifest.json");
  fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + "\n", "utf8");

  return {
    baseDir,
    manifestPath,
    dashboardFiles: DASHBOARD_LEVELS.map((l) => path.join(dashboardsDir, l.fileName)),
  };
}

const isMain =
  process.argv[1] &&
  path.resolve(process.argv[1]) === path.resolve(fileURLToPath(import.meta.url));

if (isMain) {
  writeArtifacts();
}
