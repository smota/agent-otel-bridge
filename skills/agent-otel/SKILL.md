---
name: agent-otel
description: Inspects, troubleshoots, and validates OpenTelemetry instrumentation for AI CLI agent harnesses using agent-otel-bridge.
---

# Agent OpenTelemetry Observability Skill

This skill allows agents to inspect, verify, and diagnose the health of the local OpenTelemetry pipeline and connected observability dashboards using `agent-otel-bridge`.

## Quick Diagnosis & Hook Management

When the user asks whether telemetry is flowing or why dashboard panels are empty:

1. Check hook installation status across clients (Antigravity, Claude Code, Codex, Grok, Pi):
   ```powershell
   agent-otel-bridge hooks status
   ```
2. If hooks are missing, install them automatically:
   ```powershell
   agent-otel-bridge install-hooks
   ```
3. Run the doctor tool:
   ```powershell
   agent-otel-bridge doctor
   ```
4. Verify:
   - `OTEL_EXPORTER_OTLP_ENDPOINT`: resolves to `http://127.0.0.1:4318`.
   - IPC named pipe (`\\.\pipe\agent-otel`): whether the daemon is actively listening.
   - OTLP collector reachability (`/v1/traces`).
   - Observability UI reachability (e.g. `http://localhost:8080` or `OTEL_UI_URL`).

## Emitting Synthetic Quota Probes

To trigger an immediate quota update on the dashboard:
```powershell
agent-otel-bridge emit-quota --ping
```

## OpenTelemetry Dashboard Reference

- **Visualization Guide**: [docs/DASHBOARDS.md](../../docs/DASHBOARDS.md)
- **Supported Backends**: Grafana (Tempo + Mimir), SigNoz, Jaeger, Prometheus, Datadog, Honeycomb
- **Supported Clients**: Google Antigravity, Claude Code, OpenAI Codex, xAI Grok, Pi (pi.dev), and custom AI CLI agents
- **Key Metrics**: `agent.quota.remaining_fraction`, `agent.quota.seconds_to_reset` (canonical OpenTelemetry gauges)
- **Hot-Path Binary**: `agent-hook.exe` (sub-1ms execution, 3ms fail-open watchdog)
