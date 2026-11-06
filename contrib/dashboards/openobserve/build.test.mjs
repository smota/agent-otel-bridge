import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

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
  buildDashboardLinkUrl,
  buildDashboard,
  buildDashboards,
  buildManifest,
} from "./build.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

test("artifact parity: disk manifest.json and dashboards/*.json match generator output", () => {
  const expectedManifest = buildManifest();
  const diskManifest = JSON.parse(
    fs.readFileSync(path.join(__dirname, "manifest.json"), "utf8")
  );
  assert.deepEqual(diskManifest, expectedManifest);

  const expectedDashboards = buildDashboards();
  for (const levelSpec of DASHBOARD_LEVELS) {
    const filePath = path.join(__dirname, "dashboards", levelSpec.fileName);
    assert.ok(fs.existsSync(filePath), `Missing dashboard file: ${filePath}`);
    const diskDashboard = JSON.parse(fs.readFileSync(filePath, "utf8"));
    assert.deepEqual(
      diskDashboard,
      expectedDashboards[levelSpec.key],
      `Parity mismatch for ${levelSpec.fileName}`
    );
  }
});

test("five levels: schema v5, persona descriptions, 3-5 panels, 192-column layout, and OpenAPI filter rules", () => {
  assert.equal(DASHBOARD_LEVELS.length, 5);
  const expectedKeys = [
    "fleet-portfolio",
    "workspace-project",
    "agent-workflow",
    "session-turn",
    "telemetry-operations",
  ];
  assert.deepEqual(
    DASHBOARD_LEVELS.map((l) => l.key),
    expectedKeys
  );

  const dashboards = buildDashboards();
  for (const levelSpec of DASHBOARD_LEVELS) {
    const dash = dashboards[levelSpec.key];
    assert.equal(dash.version, 5);
    assert.equal(dash.owner, "", "Dashboard must not copy private owner");
    assert.equal(dash.role, "");
    assert.equal(dash.dashboardId, "");

    const expectedMarker = formatDashboardMarker(levelSpec.key);
    assert.ok(
      dash.description.startsWith(expectedMarker),
      `Description must start with ${expectedMarker}`
    );
    assert.equal(extractDashboardKey(dash.description), levelSpec.key);
    assert.ok(
      dash.description.includes(levelSpec.persona),
      `Description must include persona ${levelSpec.persona}`
    );

    assert.equal(dash.tabs.length, 1);
    const panels = dash.tabs[0].panels;
    assert.ok(
      panels.length >= 3 && panels.length <= 5,
      `Dashboard ${levelSpec.key} must have 3-5 panels, found ${panels.length}`
    );

    for (const panel of panels) {
      assert.ok(panel.layout, `Panel ${panel.id} missing layout`);
      assert.ok(panel.layout.x >= 0 && panel.layout.x < 192);
      assert.ok(panel.layout.w > 0 && panel.layout.x + panel.layout.w <= 192);
      assert.ok(panel.layout.h > 0);
      assert.equal(panel.queryType, "sql");
      assert.ok(panel.queries.length >= 1);

      for (const q of panel.queries) {
        assert.equal(q.customQuery, true);
        assert.ok(Array.isArray(q.fields.x) && q.fields.x.length > 0, `Panel ${panel.id} must populate fields.x`);
        assert.ok(Array.isArray(q.fields.y) && q.fields.y.length > 0, `Panel ${panel.id} must populate fields.y`);
        assert.ok(Array.isArray(q.fields.z), `Panel ${panel.id} must populate fields.z array`);
        assert.deepEqual(q.fields.filter, {
          filterType: "group",
          logicalOperator: "AND",
          conditions: [],
        });
      }
    }
  }
});

test("trace scope & zero-duration hook invariant: all trace queries scope service_name and avoid percentiles", () => {
  const dashboards = buildDashboards();
  const observedTraceFieldsInQueries = new Set();
  let tracePanelCount = 0;

  for (const dash of Object.values(dashboards)) {
    for (const panel of dash.tabs[0].panels) {
      for (const q of panel.queries) {
        const sql = q.query;
        assert.ok(
          !/percentile|approx_percentile|p50|p90|p95|p99/i.test(sql),
          `Panel ${panel.id} must not use latency percentiles on zero-duration hook spans: ${sql}`
        );

        if (q.fields.stream_type === "traces") {
          tracePanelCount += 1;
          assert.equal(q.fields.stream, TRACE_STREAM);
          assert.ok(
            sql.includes(`WHERE service_name = '${SERVICE_NAME}'`),
            `Trace query in ${panel.id} must filter WHERE service_name = '${SERVICE_NAME}'`
          );

          for (const field of TRACE_FIELDS) {
            if (sql.includes(field)) {
              observedTraceFieldsInQueries.add(field);
            }
          }
        }
      }
    }
  }

  assert.ok(tracePanelCount >= 10, "Expected at least 10 trace panels across levels 1-4");
  for (const requiredField of TRACE_FIELDS) {
    assert.ok(
      observedTraceFieldsInQueries.has(requiredField),
      `Required trace field ${requiredField} was not referenced in any trace query`
    );
  }
});

test("synthetic query & variable contracts: query_values variables and safe quoted SQL substitution", () => {
  const dash = buildDashboard("session-turn");
  const varList = dash.variables.list;
  assert.equal(varList.length, 3);

  for (const spec of DASHBOARD_VARIABLE_SPECS) {
    const v = varList.find((item) => item.name === spec.name);
    assert.ok(v, `Missing variable ${spec.name}`);
    assert.equal(v.type, "query_values");
    assert.equal(v.escapeSingleQuotes, true);
    assert.equal(v.query_data.stream, "default");
    assert.equal(v.query_data.stream_type, "traces");
    assert.equal(v.query_data.field, spec.field);
    assert.deepEqual(v.query_data.filter, [
      { name: "service_name", operator: "=", value: SERVICE_NAME },
    ]);
  }

  const timelineSql = dash.tabs[0].panels[0].queries[0].query;
  assert.ok(timelineSql.includes("'$workspace_project_name'"));
  assert.ok(timelineSql.includes("'$gen_ai_agent_name'"));
  assert.ok(timelineSql.includes("'$gen_ai_conversation_id'"));
});

test("navigation targets & runtime dashboardId resolution carry over time and filter variables", () => {
  const runtimeIds = {
    "fleet-portfolio": "7524000000000000001",
    "workspace-project": "7524000000000000002",
    "agent-workflow": "7524000000000000003",
    "session-turn": "7524000000000000004",
    "telemetry-operations": "7524000000000000005",
  };

  const resolvedDashboards = buildDashboards({
    dashboardIds: runtimeIds,
    orgIdentifier: "default",
    folder: "default",
  });

  for (const levelSpec of DASHBOARD_LEVELS) {
    const dash = resolvedDashboards[levelSpec.key];
    assert.equal(dash.dashboardId, runtimeIds[levelSpec.key]);

    for (const targetKey of levelSpec.navigationTargets) {
      const expectedUrl = buildDashboardLinkUrl(targetKey, {
        dashboardIds: runtimeIds,
        orgIdentifier: "default",
        folder: "default",
      });
      assert.ok(expectedUrl.includes(`dashboard=${runtimeIds[targetKey]}`));
      assert.ok(expectedUrl.includes("from=$__from&to=$__to&period=$__period"));
      assert.ok(expectedUrl.includes("var-workspace_project_name=$workspace_project_name"));
      assert.ok(expectedUrl.includes("var-gen_ai_agent_name=$gen_ai_agent_name"));
      assert.ok(expectedUrl.includes("var-gen_ai_conversation_id=$gen_ai_conversation_id"));
      assert.ok(
        dash.description.includes(expectedUrl),
        `Dashboard ${levelSpec.key} description should include resolved URL for ${targetKey}`
      );
    }

    const drilldowns = dash.tabs[0].panels.flatMap((p) => p.config.drilldown || []);
    assert.ok(drilldowns.length > 0);
    for (const d of drilldowns) {
      assert.equal(d.data.passAllVariables, true);
      assert.ok(
        Object.values(runtimeIds).includes(d.data.dashboard),
        `Drilldown target dashboard ID ${d.data.dashboard} was not resolved`
      );
    }
  }
});

test("metric streams & fields: all 6 metric streams and stage/outcome/value/_timestamp/service_name fields covered", () => {
  const dashboards = buildDashboards();
  const observedMetricStreams = new Set();
  const allMetricQueries = [];

  for (const dash of Object.values(dashboards)) {
    for (const panel of dash.tabs[0].panels) {
      for (const q of panel.queries) {
        if (q.fields.stream_type === "metrics") {
          observedMetricStreams.add(q.fields.stream);
          allMetricQueries.push({ panelId: panel.id, stream: q.fields.stream, sql: q.query });
          assert.ok(
            q.query.includes(`WHERE service_name = '${SERVICE_NAME}'`),
            `Metric query in ${panel.id} must scope service_name = '${SERVICE_NAME}'`
          );
          assert.ok(q.query.includes("value"), `Metric query in ${panel.id} must reference value`);
          assert.ok(
            q.query.includes("_timestamp"),
            `Metric query in ${panel.id} must reference _timestamp`
          );
        }
      }
    }
  }

  for (const streamName of Object.keys(METRIC_STREAMS)) {
    assert.ok(
      observedMetricStreams.has(streamName),
      `Expected metric stream ${streamName} to be queried in dashboards`
    );
  }

  const eventsQueries = allMetricQueries.filter(
    (q) => q.stream === "agent_bridge_events_total"
  );
  assert.ok(eventsQueries.length >= 1);
  for (const eq of eventsQueries) {
    assert.ok(eq.sql.includes("stage"), "agent_bridge_events_total query must include stage");
    assert.ok(eq.sql.includes("outcome"), "agent_bridge_events_total query must include outcome");
  }
});

test("regression: per-instance grouping across all metric queries and stream field lists", () => {
  const dashboards = buildDashboards();

  for (const [streamName, meta] of Object.entries(METRIC_STREAMS)) {
    assert.ok(
      meta.fields.includes("host_name"),
      `METRIC_STREAMS.${streamName} must declare host_name field`
    );
    assert.ok(
      meta.fields.includes("service_instance_id"),
      `METRIC_STREAMS.${streamName} must declare service_instance_id field`
    );
  }

  let metricPanelCount = 0;
  for (const dash of Object.values(dashboards)) {
    for (const panel of dash.tabs[0].panels) {
      for (const q of panel.queries) {
        if (q.fields.stream_type === "metrics") {
          metricPanelCount += 1;
          const sql = q.query;
          assert.ok(
            /\bhost_name\b/i.test(sql.split(/\bFROM\b/i)[0]),
            `Panel ${panel.id} query must SELECT host_name: ${sql}`
          );
          assert.ok(
            /\bservice_instance_id\b/i.test(sql.split(/\bFROM\b/i)[0]),
            `Panel ${panel.id} query must SELECT service_instance_id: ${sql}`
          );
          assert.ok(
            /GROUP\s+BY\s+.*?\bhost_name\b/i.test(sql),
            `Panel ${panel.id} query must GROUP BY host_name: ${sql}`
          );
          assert.ok(
            /GROUP\s+BY\s+.*?\bservice_instance_id\b/i.test(sql),
            `Panel ${panel.id} query must GROUP BY service_instance_id: ${sql}`
          );

          const xColNames = q.fields.x.map((x) => x.column);
          assert.ok(
            xColNames.includes("host_name"),
            `Panel ${panel.id} fields.x must include host_name`
          );
          assert.ok(
            xColNames.includes("service_instance_id"),
            `Panel ${panel.id} fields.x must include service_instance_id`
          );
        }
      }
    }
  }
  assert.equal(metricPanelCount, 7, "Expected all 7 metric panels across dashboards");
});

test("regression: nullable metric dimensions retain aliases and raw grouping", () => {
  let metricQueries = 0;
  let quotaQueries = 0;
  for (const dash of Object.values(buildDashboards())) {
    for (const panel of dash.tabs[0].panels) {
      for (const q of panel.queries) {
        if (q.fields.stream_type !== "metrics") continue;
        metricQueries += 1;
        const select = q.query.split(/\bFROM\b/i)[0];
        const groupBy = q.query.match(/\bGROUP BY (.*?) ORDER BY/i)?.[1];
        assert.ok(groupBy, `${panel.id}: missing raw grouping`);
        const dimensions = [
          ["host_name", "host_name"],
          ["service_instance_id", "service_instance_id"],
        ];
        if (q.fields.stream.startsWith("agent_quota_")) {
          quotaQueries += 1;
          dimensions.push(
            ["gen_ai_provider_name", "gen_ai_provider_name"],
            ['"group"', "quota_group"],
            ["bucket", "bucket"]
          );
        }
        for (const [raw, alias] of dimensions) {
          assert.ok(
            select.includes(`COALESCE(${raw}, 'unknown') AS ${alias}`),
            `${panel.id}: nullable ${raw} must emit a stable ${alias} field`
          );
          assert.ok(groupBy.split(", ").includes(raw), `${panel.id}: group by raw ${raw}`);
          assert.ok(q.fields.x.some((field) => field.column === alias));
        }
        assert.ok(!groupBy.includes("COALESCE"), `${panel.id}: preserve raw groups`);
      }
    }
  }
  assert.equal(metricQueries, 7);
  assert.equal(quotaQueries, 2);
});

test("regression: quota group separation in SELECT, GROUP BY, and displayed x columns", () => {
  const dashboards = buildDashboards();
  const quotaStreams = ["agent_quota_remaining_fraction", "agent_quota_seconds_to_reset"];

  for (const streamName of quotaStreams) {
    assert.ok(
      METRIC_STREAMS[streamName].fields.includes("group"),
      `METRIC_STREAMS.${streamName} must include group field`
    );
  }

  let quotaPanelCount = 0;
  for (const dash of Object.values(dashboards)) {
    for (const panel of dash.tabs[0].panels) {
      for (const q of panel.queries) {
        if (quotaStreams.includes(q.fields.stream)) {
          quotaPanelCount += 1;
          const sql = q.query;
          assert.ok(
            /COALESCE\("group", 'unknown'\)\s+AS\s+quota_group\b/i.test(sql.split(/\bFROM\b/i)[0]),
            `Quota query in ${panel.id} must SELECT quoted "group" AS quota_group: ${sql}`
          );
          assert.ok(
            /GROUP\s+BY\s+.*?"group"/i.test(sql),
            `Quota query in ${panel.id} must GROUP BY "group": ${sql}`
          );

          const xColNames = q.fields.x.map((x) => x.column);
          assert.ok(
            xColNames.includes("quota_group"),
            `Quota panel ${panel.id} fields.x must include quota_group`
          );
        }
      }
    }
  }
  assert.equal(quotaPanelCount, 2, "Expected both quota panels across dashboards");
});

test("regression: no false execution labels; archetype panels report observed spans/events", () => {
  const dashboards = buildDashboards();

  for (const dash of Object.values(dashboards)) {
    for (const panel of dash.tabs[0].panels) {
      assert.ok(
        !/tool-execution\s+count/i.test(panel.title) &&
          !/tool-execution\s+count/i.test(panel.description),
        `Panel ${panel.id} must never use tool-execution counts: title="${panel.title}" desc="${panel.description}"`
      );

      for (const q of panel.queries) {
        assert.ok(
          !/\bAS\s+executions\b/i.test(q.query),
          `Panel ${panel.id} query must not alias AS executions: ${q.query}`
        );
        const yCols = q.fields.y.map((y) => y.column);
        assert.ok(
          !yCols.includes("executions"),
          `Panel ${panel.id} fields.y must not contain executions`
        );
      }

      if (panel.id.includes("archetype")) {
        const text = `${panel.title} ${panel.description}`.toLowerCase();
        assert.ok(
          text.includes("observed spans") || text.includes("observed spans/events"),
          `Archetype panel ${panel.id} title/description must describe observed spans/events: ${text}`
        );
      }
    }
  }
});

test("regression: missing-data display defaults to N/A, not 0", () => {
  const dashboards = buildDashboards();

  for (const dash of Object.values(dashboards)) {
    for (const panel of dash.tabs[0].panels) {
      assert.equal(
        panel.config.no_value_replacement,
        "N/A",
        `Panel ${panel.id} must default no_value_replacement to "N/A"`
      );
    }
  }
});

test("regression: v5 variables omit selectAllValueForMultiSelect, scope, panels, and tabs", () => {
  const dashboards = buildDashboards();

  for (const [key, dash] of Object.entries(dashboards)) {
    assert.ok(dash.variables.list.length > 0, `Dashboard ${key} has variables`);
    for (const v of dash.variables.list) {
      assert.equal(Object.prototype.hasOwnProperty.call(v, "selectAllValueForMultiSelect"), false);
      assert.equal(Object.prototype.hasOwnProperty.call(v, "scope"), false);
      assert.equal(Object.prototype.hasOwnProperty.call(v, "panels"), false);
      assert.equal(Object.prototype.hasOwnProperty.call(v, "tabs"), false);
    }
  }
});
