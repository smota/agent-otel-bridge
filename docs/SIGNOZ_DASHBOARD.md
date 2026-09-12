# SigNoz Dashboard Integration & OpenTelemetry Standardization Guide

This document specifies the integration between `agent-otel-bridge` and the station SigNoz dashboard:
- **Dashboard URL**: `http://localhost:8080/dashboard/01a08f6b-fbeb-7439-a6a9-0809f9da72a0`
- **Dashboard Title**: AI CLI Agent Observability (Antigravity, Claude Code, Codex, Grok, Pi)
- **Collector Endpoint**: `http://127.0.0.1:4318` (OTLP HTTP/Protobuf)

---

## 1. OpenTelemetry Design Principles & Layer Separation

### A. Semantic Standardization at the Bridge Layer
In alignment with OpenTelemetry best practices, telemetry emitters (SDKs, hooks, and bridges) produce **canonical, vendor-neutral semantic conventions**:
- **GenAI SemConv (`gen_ai.*`)**: Standardized operations (`execute_tool`, `invoke_agent`), providers (`google`, `anthropic`, `openai`, `xai`, `pi`), and models.
- **Agent Lifecycle SemConv (`agent.*`)**: Universal agent hooks (`agent.hook.event`, `agent.step.index`, `agent.execution.num`, `agent.fully_idle`, `agent.termination_reason`, `agent.quota.*`).

Hardcoding client-specific prefixes (such as `agy.*`) in an agent bridge violates the *Single Responsibility Principle* and pollutes spans produced when instrumenting other agents (Claude Code, OpenAI Codex, xAI Grok, Pi).

### B. Transformation Belongs in the OpenTelemetry Collector
When backward compatibility or proprietary dialect translation is needed for legacy dashboards, OpenTelemetry architecture specifies that transformations belong in the **OpenTelemetry Collector** using the `transformprocessor` (OTTL):

```yaml
# otel-collector-config.yaml
processors:
  transform/legacy_agy:
    error_mode: ignore
    trace_statements:
      - context: span
        statements:
          - set(attributes["agy.hook.event"], attributes["agent.hook.event"])
          - set(attributes["agy.step.index"], attributes["agent.step.index"])
          - set(attributes["agy.execution.num"], attributes["agent.execution.num"])
          - set(attributes["agy.fully_idle"], attributes["agent.fully_idle"])
          - set(attributes["agy.termination_reason"], attributes["agent.termination_reason"])
    metric_statements:
      - context: datapoint
        statements:
          - set(metric.name, "agy.quota.remaining_fraction") where metric.name == "agent.quota.remaining_fraction"
          - set(metric.name, "agy.quota.seconds_to_reset") where metric.name == "agent.quota.seconds_to_reset"

service:
  pipelines:
    traces:
      receivers: [otlp]
      processors: [transform/legacy_agy, batch]
      exporters: [clickhouse]
    metrics:
      receivers: [otlp]
      processors: [transform/legacy_agy, batch]
      exporters: [clickhouse]
```

### C. Optional Bridge Compatibility Toggle
For environments without a custom Collector pipeline, `agent-otel-bridge` provides an opt-in environment variable:
```bash
AGENT_OTEL_LEGACY_ATTRIBUTES=true
```
- **Default**: `false` (emits exclusively canonical `gen_ai.*` and `agent.*` conventions).
- **When `true`**: Simultaneously emits `agy.*` aliases alongside canonical attributes.

---

## 2. Canonical Dashboard Panels & Query Specifications

Every panel in the dashboard queries standard conventions applicable to all supported AI harnesses:

| Panel # | Panel Name | Visualization | Data Source | Canonical Metric / Attribute | Aggregation & Grouping |
|:---:|---|---|---|---|---|
| **1** | **Active Sessions** | Stat / Value | Traces | `gen_ai.conversation.id` | `countDistinct(attributes['gen_ai.conversation.id'])` by `gen_ai.agent.name` |
| **2** | **Tool Execution Latency** | Time Series (ms) | Traces | `durationNano` | `p50`, `p90`, `p99` where `gen_ai.operation.name = 'execute_tool'` by `gen_ai.tool.name` |
| **3** | **Tool Call Distribution** | Donut / Bar | Traces | `gen_ai.tool.name` | `count()` where `gen_ai.operation.name = 'execute_tool'` by `gen_ai.tool.name` |
| **4** | **Hook Events Timeline** | Spans Table | Traces | `agent.hook.event` | List of spans where `agent.hook.event IN ('PostToolUse', 'PostInvocation', 'Stop')` |
| **5** | **Agent Quota Remaining** | Gauge (0–100%) | Metrics | `agent.quota.remaining_fraction` | `last_value` grouped by `bucket`, `group` |
| **6** | **Seconds to Quota Reset** | Stat / Time | Metrics | `agent.quota.seconds_to_reset` | `last_value` formatted as duration / HH:MM |
| **7** | **Error Rates by Tool** | Time Series | Traces | `status.code = 2` | `count()` where `status_code = 'STATUS_CODE_ERROR'` by `gen_ai.tool.name` |
| **8** | **AI Provider Breakdown** | Pie Chart | Traces | `gen_ai.provider.name` | `count()` by `gen_ai.provider.name` (`google`, `anthropic`, `openai`, `xai`, `pi`) |
| **9** | **Turn Step Distribution** | Histogram / Bar | Traces | `agent.step.index` | `max(attributes['agent.step.index'])` by `gen_ai.conversation.id` |
| **10** | **Quiescence & Stop Reasons** | Table / Pie | Traces | `agent.termination_reason` | `count()` where `agent.hook.event = 'Stop'` by `agent.termination_reason` |

---

## 3. Concrete ClickHouse / SigNoz Query Reference

### Panel 1: Active Sessions
```sql
SELECT
    attributes_string_value['gen_ai.agent.name'] AS agent_name,
    count(DISTINCT attributes_string_value['gen_ai.conversation.id']) AS active_sessions
FROM signoz_traces.signoz_index_v2
WHERE timestamp >= now() - INTERVAL 1 HOUR
GROUP BY agent_name;
```

### Panel 2: Tool Execution Latency (P50 / P95)
```sql
SELECT
    toStartOfInterval(timestamp, INTERVAL 1 MINUTE) AS time,
    attributes_string_value['gen_ai.tool.name'] AS tool_name,
    quantile(0.50)(durationNano) / 1000000 AS p50_latency_ms,
    quantile(0.95)(durationNano) / 1000000 AS p95_latency_ms
FROM signoz_traces.signoz_index_v2
WHERE attributes_string_value['gen_ai.operation.name'] = 'execute_tool'
  AND timestamp >= now() - INTERVAL 1 HOUR
GROUP BY time, tool_name
ORDER BY time ASC;
```

### Panel 3: Tool Call Distribution
```sql
SELECT
    attributes_string_value['gen_ai.tool.name'] AS tool_name,
    count() AS total_calls
FROM signoz_traces.signoz_index_v2
WHERE attributes_string_value['gen_ai.operation.name'] = 'execute_tool'
  AND timestamp >= now() - INTERVAL 24 HOUR
GROUP BY tool_name
ORDER BY total_calls DESC;
```

### Panel 4: Hook Events Timeline
```sql
SELECT
    timestamp,
    trace_id,
    attributes_string_value['gen_ai.agent.name'] AS agent,
    attributes_string_value['agent.hook.event'] AS hook_event,
    attributes_string_value['gen_ai.tool.name'] AS tool,
    attributes_string_value['agent.termination_reason'] AS termination_reason,
    durationNano / 1000000 AS duration_ms
FROM signoz_traces.signoz_index_v2
WHERE has(attributes_string_key, 'agent.hook.event')
  AND timestamp >= now() - INTERVAL 1 HOUR
ORDER BY timestamp DESC
LIMIT 100;
```

### Panel 5: Agent Quota Remaining
```sql
SELECT
    toStartOfInterval(timestamp, INTERVAL 1 MINUTE) AS time,
    attributes_string_value['bucket'] AS bucket,
    avg(value) * 100 AS remaining_percent
FROM signoz_metrics.samples_v4
WHERE metric_name = 'agent.quota.remaining_fraction'
  AND timestamp >= now() - INTERVAL 6 HOUR
GROUP BY time, bucket
ORDER BY time ASC;
```

### Panel 6: Seconds to Quota Reset
```sql
SELECT
    attributes_string_value['bucket'] AS bucket,
    last_value(value) AS seconds_remaining
FROM signoz_metrics.samples_v4
WHERE metric_name = 'agent.quota.seconds_to_reset'
  AND timestamp >= now() - INTERVAL 15 MINUTE
GROUP BY bucket;
```

### Panel 7: AI Provider Breakdown
```sql
SELECT
    attributes_string_value['gen_ai.provider.name'] AS provider,
    count() AS span_count
FROM signoz_traces.signoz_index_v2
WHERE has(attributes_string_key, 'gen_ai.provider.name')
  AND timestamp >= now() - INTERVAL 24 HOUR
GROUP BY provider;
```

---

## 4. Verification & Testing

To verify end-to-end telemetry flow to the updated dashboard:

1. **Verify pipeline health**:
   ```powershell
   agent-otel-bridge doctor
   ```
2. **Emit a real-time quota data point**:
   ```powershell
   agent-otel-bridge emit-quota --ping
   ```
3. **Open the SigNoz Dashboard**:
   [`http://localhost:8080/dashboard/01a08f6b-fbeb-7439-a6a9-0809f9da72a0`](http://localhost:8080/dashboard/01a08f6b-fbeb-7439-a6a9-0809f9da72a0)
4. Confirm that:
   - **Active Sessions** groups distinct sessions by agent (`antigravity`, `claude-code`, etc.).
   - **Agent Quota Remaining** displays the gauge for metric `agent.quota.remaining_fraction`.
   - **Hook Events Timeline** displays spans with `agent.hook.event = 'PostToolUse'`.
