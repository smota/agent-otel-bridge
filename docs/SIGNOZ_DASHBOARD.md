# SigNoz Dashboard Integration Guide

This document specifies the integration between `agent-otel-bridge` and the station SigNoz dashboard:
- **Dashboard URL**: `http://localhost:8080/dashboard/01a08f6b-fbeb-7439-a6a9-0809f9da72a0`
- **Dashboard Title**: Antigravity CLI Observability
- **Collector Endpoint**: `http://127.0.0.1:4318` (OTLP HTTP/Protobuf)

---

## 1. Dashboard Panels & Data Sources

| Panel Name | Type | Query / Metric | Target Attribute / Filter |
|---|---|---|---|
| **Active Sessions** | Value / Count | Count distinct `gen_ai.conversation.id` | `service.name = "antigravity-cli"` |
| **Tool Execution Latency** | Time Series / Heatmap | Duration of spans where `gen_ai.operation.name = "execute_tool"` | Grouped by `gen_ai.tool.name` |
| **Tool Call Distribution** | Pie / Bar Chart | Count of spans | Grouped by `gen_ai.tool.name` |
| **Hook Events Timeline** | Spans List | All spans | Filtered by `agy.hook.event IN ("PostToolUse", "PostInvocation", "Stop")` |
| **Gemini Weekly Quota Remaining** | Gauge | `agy.quota.remaining_fraction` | `bucket = "gemini-weekly"`, `group = "gemini"` |
| **Seconds to Quota Reset** | Gauge / Time | `agy.quota.seconds_to_reset` | `bucket = "gemini-weekly"`, `group = "gemini"` |
| **Error Rates** | Time Series | Spans with `status.code = ERROR` | Filtered by `error.type` or `status_message` |

---

## 2. Telemetry Ingestion Verification

To verify that telemetry is reaching the dashboard:

1. Run the diagnosis command:
   ```powershell
   agent-otel-bridge doctor
   ```
2. Send a manual quota ping:
   ```powershell
   agent-otel-bridge emit-quota --ping
   ```
3. Open the SigNoz dashboard:
   [`http://localhost:8080/dashboard/01a08f6b-fbeb-7439-a6a9-0809f9da72a0`](http://localhost:8080/dashboard/01a08f6b-fbeb-7439-a6a9-0809f9da72a0)
4. Verify that the **Gemini Weekly Quota Remaining** panel renders a gauge and the **Hook Events Timeline** displays recent tool calls.
