---
name: agent-otel
description: Inspects, troubleshoots, and validates OpenTelemetry instrumentation for AI CLI agent harnesses using agent-otel-bridge.
---

# Agent OpenTelemetry Observability Skill

This skill allows Antigravity agents to inspect, verify, and diagnose the health of the local OpenTelemetry pipeline and SigNoz dashboard using `agent-otel-bridge`.

## Quick Diagnosis

When the user asks whether telemetry is flowing or why dashboard panels are empty:

1. Run the doctor tool:
   ```powershell
   agent-otel-bridge doctor
   ```
2. Verify:
   - `OTEL_EXPORTER_OTLP_ENDPOINT`: should resolve to `http://127.0.0.1:4318`.
   - IPC named pipe (`\\.\pipe\agy-otel`): whether the daemon is actively listening.
   - OTLP collector reachability.
   - SigNoz UI reachability at `http://localhost:8080`.

## Emitting Synthetic Quota Probes

To trigger an immediate quota update on the SigNoz dashboard:
```powershell
agent-otel-bridge emit-quota --ping
```

## SigNoz Dashboard Reference

- **Dashboard URI**: `http://localhost:8080/dashboard/01a08f6b-fbeb-7439-a6a9-0809f9da72a0`
- **Tracked Service**: `antigravity-cli`
- **Key Metrics**: `agy.quota.remaining_fraction`, `agy.quota.seconds_to_reset`
- **Hot-Path Binary**: `agent-hook.exe` (sub-3ms execution, fail-open)
