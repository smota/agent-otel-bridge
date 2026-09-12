# Configuration & Environment Reference

This guide details all environment variables, daemon parameters, and OpenTelemetry Collector configurations supported by `agent-otel-bridge`.

---

## 1. Environment Variables Catalog

All environment variables follow standard OpenTelemetry conventions with project-specific overrides:

| Environment Variable | Default Value | Description |
|---|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | `http://127.0.0.1:4318` | Base URL for OTLP HTTP/Protobuf exports (`/v1/traces` and `/v1/metrics`). |
| `OTEL_SERVICE_NAME` | `agent-otel-bridge` | Service name stamped on all traces and resource metrics. |
| `OTEL_RESOURCE_ATTRIBUTES` | *(Empty)* | Comma-separated key-value resource attributes (e.g., `deployment.environment=homelab,host.id=ws-01`). |
| `AGENT_OTEL_PIPE` | `\\.\pipe\agent-otel` | Primary Win32 Named Pipe path used for IPC between `agent-hook` and the daemon. |
| `AGY_OTEL_PIPE` | `\\.\pipe\agy-otel` | Backward-compatibility fallback Named Pipe path. |
| `AGENT_OTEL_BATCH_SIZE` | `50` | Maximum number of spans held in memory before forcing an immediate OTLP export flush. |
| `AGENT_OTEL_BATCH_TIMEOUT_MS` | `200` | Maximum time in milliseconds to wait before flushing buffered spans if batch size is not reached. |
| `AGENT_OTEL_IDLE_TIMEOUT_SECS` | `1800` | Idle timeout in seconds (30 minutes). If no events or IPC connections occur, the daemon exits cleanly to conserve memory. Set to `0` to disable idle timeout. |
| `AGENT_OTEL_QUOTA_INTERVAL_SECS` | `60` | Ticker interval in seconds for broadcasting model quota gauges to the collector. |
| `AGENT_OTEL_QUOTA_FILE` | *(Auto-discovered)* | Custom file path to read quota state JSON. Defaults to auto-discovering in `~/.state/quota.json`, `~/.gemini/quota.json`, or `~/.agent-otel/quota.json`. |
| `AGENT_OTEL_LEGACY_ATTRIBUTES` | `false` | When set to `true`, emits legacy `agy.*` aliases alongside canonical OpenTelemetry `gen_ai.*` and `agent.*` conventions for backward compatibility. |

---

## 2. Cross-Platform Environment Variable Configuration

### PowerShell (Windows, macOS, Linux)

#### Permanent Configuration (User Profile)
```powershell
[System.Environment]::SetEnvironmentVariable("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:4318", "User")
[System.Environment]::SetEnvironmentVariable("OTEL_SERVICE_NAME", "agent-otel-bridge", "User")
[System.Environment]::SetEnvironmentVariable("OTEL_RESOURCE_ATTRIBUTES", "deployment.environment=production", "User")
```

#### Session-Specific Configuration
```powershell
$env:OTEL_EXPORTER_OTLP_ENDPOINT = "http://127.0.0.1:4318"
$env:OTEL_SERVICE_NAME = "my-custom-workstation"
$env:TRACEPARENT = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
```

### Bash / Zsh (Linux, macOS)

#### Permanent Configuration (`~/.bashrc` or `~/.zshrc`)
```bash
export OTEL_EXPORTER_OTLP_ENDPOINT="http://127.0.0.1:4318"
export OTEL_SERVICE_NAME="agent-otel-bridge"
export OTEL_RESOURCE_ATTRIBUTES="deployment.environment=production"
```

#### Session-Specific Configuration
```bash
export OTEL_EXPORTER_OTLP_ENDPOINT="http://127.0.0.1:4318"
export OTEL_SERVICE_NAME="my-custom-workstation"
export TRACEPARENT="00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
```

---

## 3. OpenTelemetry Collector Recipe (`config.yaml`)

Below is a production-tested OpenTelemetry Collector configuration supporting traces and metrics export to **SigNoz**, **Jaeger**, and **Prometheus**:

```yaml
receivers:
  otlp:
    protocols:
      grpc:
        endpoint: 0.0.0.0:4317
      http:
        endpoint: 0.0.0.0:4318

processors:
  batch:
    send_batch_size: 100
    timeout: 1s
  memory_limiter:
    check_interval: 1s
    limit_percentage: 75
    spike_limit_percentage: 20

exporters:
  # Export traces & metrics to local SigNoz instance
  otlp/signoz:
    endpoint: localhost:4317
    tls:
      insecure: true

  # Export metrics to Prometheus scraper endpoint
  prometheus:
    endpoint: 0.0.0.0:8889
    namespace: agent_otel

  # Debug exporter (useful for testing)
  debug:
    verbosity: basic

service:
  pipelines:
    traces:
      receivers: [otlp]
      processors: [memory_limiter, batch]
      exporters: [otlp/signoz, debug]
    metrics:
      receivers: [otlp]
      processors: [memory_limiter, batch]
      exporters: [otlp/signoz, prometheus]
```

---

## 4. Validating Collector Ingestion

You can verify that your collector is receiving and routing spans correctly:

1. Run the diagnosis tool:
   ```powershell
   agent-otel-bridge doctor
   ```
2. Trigger a synthetic quota metric ping:
   ```powershell
   agent-otel-bridge emit-quota --ping
   ```
3. Open your collector logs or SigNoz dashboard (`http://localhost:8080`) to inspect the newly ingested traces and gauges.
