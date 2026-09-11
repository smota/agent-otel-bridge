# Troubleshooting & Diagnostic Runbook

This guide helps diagnose and resolve common issues with `agent-otel-bridge`, Named Pipe IPC, OTLP exports, and AI client hook integrations.

---

## 1. Quick Diagnostic Checklist

Always start by running `doctor`:
```powershell
agent-otel-bridge doctor
```

Output inspection:
- **[1/5] Environment Contract**: Verifies `OTEL_EXPORTER_OTLP_ENDPOINT` and `OTEL_SERVICE_NAME`.
- **[2/5] Named Pipe IPC**: Verifies that the background daemon is listening on `\\.\pipe\agent-otel`.
- **[3/5] OTLP Collector HTTP**: Sends a probe to `http://127.0.0.1:4318/v1/traces`.
- **[4/5] Observability UI**: Checks if your dashboard/UI is reachable (default `http://localhost:8080`, or configured via `OTEL_UI_URL`).
- **[5/5] Hook Registrations**: Verifies if `agent-hook` is in PATH and registered for Antigravity, Claude Code, Codex, Grok, and Pi.

---

## 2. Common Issues and Resolutions

### Issue 1: `Daemon is NOT currently running`
**Symptom**: Step [2/5] in `doctor` outputs:
```text
[2/5] Checking named pipe IPC (\\.\pipe\agent-otel)...
  [warn] Daemon is NOT currently running (pipe not found or busy).
```
**Cause**: The background daemon has not been started, or exited after the 30-minute idle timeout.
**Fix**:
1. Start the daemon in a background shell or terminal:
   ```powershell
   agent-otel-bridge daemon
   ```
2. Or configure it as a Windows scheduled task or system service to run at user login.

---

### Issue 2: `agent-hook in PATH: [warn] not found`
**Symptom**: Step [5/5] shows `agent-hook` is missing from system PATH.
**Cause**: Cargo's bin directory is not in your environment PATH variable.
**Fix**:
1. Verify where `agent-hook.exe` is located:
   ```powershell
   Test-Path "$env:USERPROFILE\.cargo\bin\agent-hook.exe"
   ```
2. If missing, install it:
   ```powershell
   cargo install --path crates/agent-otel-client --force
   ```
3. Add Cargo bin to PATH permanently in PowerShell:
   ```powershell
   [Environment]::SetEnvironmentVariable("PATH", $env:PATH + ";$env:USERPROFILE\.cargo\bin", "User")
   ```

---

### Issue 3: `OTLP Collector unreachable`
**Symptom**: Step [3/5] reports:
```text
[3/5] Checking OTLP Collector HTTP endpoint...
  [fail] Failed to connect to OTLP Collector: Connection refused
```
**Cause**: The OpenTelemetry Collector is not running on port `4318`.
**Resolution**:
1. Check if the collector container or process is running:
   ```powershell
   docker ps | grep -E "otel|collector"
   ```
2. If running via Docker Compose, start the observability cluster:
   ```powershell
   docker compose up -d
   ```
3. If running a standalone collector binary:
   ```powershell
   otelcol.exe --config config.yaml
   ```

---

### Issue 4: Client Hooks Not Configured
**Symptom**: Step [5/5] shows `[info] not registered` for your desired agent client.
**Resolution**:
Run the automated installation shortcut:
```powershell
agent-otel-bridge install-hooks
```
To force installation for a specific client:
```powershell
agent-otel-bridge install-hooks --client claude
```

---

### Issue 5: Dashboard Displays Empty Panels
**Symptom**: Traces do not appear in the dashboard after tool executions.
**Diagnosis Steps**:
1. Emit a synthetic quota metric probe directly to the collector:
   ```powershell
   agent-otel-bridge emit-quota --ping
   ```
2. If the probe succeeds, the collector and backend ingestion pipelines are functioning.
3. Check that your AI agent is actually invoking the registered hook by inspecting the agent session output.
4. Verify the active trace queries in [docs/DASHBOARDS.md](DASHBOARDS.md) to ensure panel filters match canonical conventions (`agent.hook.event`, `gen_ai.operation.name = "execute_tool"`).

---

## 3. Verifying the Fail-Open Invariant

`agent-otel-bridge` is built with a hard invariant: **telemetry must never crash, block, or delay the autonomous agent harness.**

You can independently verify this fail-open guarantee:

1. **Stop the background daemon and collector**:
   ```powershell
   agent-otel-bridge stop
   ```
2. **Execute `agent-hook.exe` directly**:
   ```powershell
   echo '{"conversationId":"test","stepIdx":1}' | agent-hook PostToolUse
   ```
3. **Verify the output**:
   - The command outputs `{}` immediately.
   - The exit code is `0`.
   - Execution finishes in **< 3ms** due to the hard fail-open watchdog thread.
   - Your agent's workflow continues seamlessly even when the entire observability infrastructure is offline.
