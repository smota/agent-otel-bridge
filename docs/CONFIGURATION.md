# Configuration & Environment Reference

This guide details all environment variables, daemon parameters, and OpenTelemetry Collector configurations supported by `agent-otel-bridge`.

---

## 1. Environment Variables Catalog

All environment variables follow standard OpenTelemetry conventions with project-specific overrides:

| Environment Variable | Default Value | Description |
|---|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | `http://127.0.0.1:4318` | Base URL for OTLP/HTTP protobuf. One trailing slash is removed, then `/v1/traces` and `/v1/metrics` are appended. A base that already contains those paths is not special-cased. |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` | *(unset)* | Exact trace URL. When set and non-empty, `/v1/traces` is not appended. |
| `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` | *(unset)* | Exact metrics URL. When set and non-empty, `/v1/metrics` is not appended. |
| `OTEL_EXPORTER_OTLP_PROTOCOL` | `http/protobuf` | Only `http/protobuf` is exported. `grpc` or any other value disables that signal. |
| `OTEL_EXPORTER_OTLP_TRACES_PROTOCOL` / `OTEL_EXPORTER_OTLP_METRICS_PROTOCOL` | *(inherit)* | Signal-specific protocol. Overrides the general protocol for that signal only. |
| `OTEL_EXPORTER_OTLP_HEADERS` | *(empty)* | Comma-separated `key=value` headers. Values are percent-decoded. Invalid names or values are skipped. |
| `OTEL_EXPORTER_OTLP_TRACES_HEADERS` / `OTEL_EXPORTER_OTLP_METRICS_HEADERS` | *(inherit)* | Merged over the general headers. The same key, ignoring case, uses the signal value. |
| `OTEL_EXPORTER_OTLP_COMPRESSION` | `none` | `none` sends the protobuf body unchanged. `gzip` compresses it with `Content-Encoding: gzip`. Any other value disables that signal. |
| `OTEL_EXPORTER_OTLP_TRACES_COMPRESSION` / `OTEL_EXPORTER_OTLP_METRICS_COMPRESSION` | *(inherit)* | Signal-specific compression. |
| `OTEL_EXPORTER_OTLP_TIMEOUT` | `5000` | Request timeout in milliseconds. A value that does not parse is treated as 5000. |
| `OTEL_EXPORTER_OTLP_TRACES_TIMEOUT` / `OTEL_EXPORTER_OTLP_METRICS_TIMEOUT` | *(inherit)* | Signal-specific timeout. The export deadline is at least this timeout. |
| `OTEL_EXPORTER_OTLP_CERTIFICATE` | *(unset)* | PEM file of extra root CAs. A missing or invalid file prevents the daemon from starting. |
| `OTEL_EXPORTER_OTLP_TRACES_CERTIFICATE` / `OTEL_EXPORTER_OTLP_METRICS_CERTIFICATE` | *(inherit)* | Signal-specific CA file. Both are loaded when they differ. |
| `OTEL_SDK_DISABLED` | `false` | `true` (any case) accepts exports locally and performs no HTTP. The process still starts. |
| `OTEL_SERVICE_NAME` | `agent-otel-bridge` | `service.name`. Wins over the same key in `OTEL_RESOURCE_ATTRIBUTES`. |
| `OTEL_SERVICE_VERSION` | crate version | `service.version` and the telemetry SDK/distro version when set and non-empty. |
| `OTEL_RESOURCE_ATTRIBUTES` | *(empty)* | Full baggage list (`key=value`, percent-decoded). Merged onto detected resource attributes. Empty does not invent `deployment.environment`. |
| `AGENT_OTEL_PIPE` | `\\.\pipe\agent-otel` | Primary Win32 Named Pipe path used for IPC between `agent-hook` and the daemon. |
| `AGY_OTEL_PIPE` | `\\.\pipe\agy-otel` | Backward-compatibility fallback Named Pipe path. |
| `AGENT_OTEL_BATCH_SIZE` | `50` | Maximum number of spans held in memory before forcing an immediate OTLP export flush. |
| `AGENT_OTEL_BATCH_TIMEOUT_MS` | `200` | Maximum time in milliseconds to wait before flushing buffered spans if batch size is not reached. |
| `AGENT_OTEL_IDLE_TIMEOUT_SECS` | `1800` | Idle timeout in seconds (30 minutes). If no events or IPC connections occur, the daemon exits cleanly to conserve memory. Set to `0` to disable idle timeout. |
| `AGENT_OTEL_QUOTA_INTERVAL_SECS` | `60` | Ticker interval in seconds for broadcasting model quota gauges to the collector. |
| `AGENT_OTEL_QUOTA_FILE` | *(Auto-discovered)* | Custom file path to read quota state JSON. Defaults to auto-discovering in `~/.state/quota.json`, `~/.gemini/quota.json`, or `~/.agent-otel/quota.json`. |
| `AGENT_OTEL_LEGACY_ATTRIBUTES` | `false` | When set to `true`, emits legacy `agy.*` aliases alongside canonical OpenTelemetry `gen_ai.*` and `agent.*` conventions for backward compatibility. |

### OTLP/HTTP export

The daemon exports OTLP protobuf over HTTP. `Content-Type` is always `application/x-protobuf`; user headers cannot replace it. `Content-Encoding: gzip` is set only when that signal's compression is `gzip`. Retry, partial success, the 1 MiB request limit, and the 4 MiB response limit are unchanged. A disabled signal returns without opening a connection. `OTEL_SDK_DISABLED=true` reports the batch accepted with zero attempts.

Signal-specific variables override the general `OTEL_EXPORTER_OTLP_*` variable for that signal. The daemon and direct `emit-quota` export both use this contract. `OtlpExporter::new(traces_url, metrics_url)` remains a compatibility client and does not read environment variables.

### Resource identity

Detected attributes are `service.name`, `service.version`, `host.name`, `os.type`, `telemetry.sdk.name=agent-otel-bridge`, `telemetry.sdk.language=rust`, `telemetry.sdk.version`, `telemetry.distro.name=agent-otel-bridge`, `telemetry.distro.version`, and a process-stable `service.instance.id`. `os.type` is `darwin` when the Rust OS name is `macos`.

`deployment.environment` is emitted only when that key is present in `OTEL_RESOURCE_ATTRIBUTES`. `deployment.environment.name` is emitted only when that key is present. Neither is copied from the other, and neither defaults to `homelab`.

`OTEL_RESOURCE_ATTRIBUTES` overrides detected attributes except `service.name` and the `telemetry.sdk.*` / `telemetry.distro.*` identity. An explicit `service.instance.id` in the list replaces the generated id.

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
