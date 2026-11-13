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
| `AGENT_OTEL_IDLE_TIMEOUT_SECS` | `0` | Idle timeout in seconds. `0` keeps the daemon running. A positive value exits after that many seconds without events. |
| `AGENT_OTEL_QUOTA_INTERVAL_SECS` | `60` | Ticker interval in seconds for broadcasting model quota gauges to the collector. |
| `AGENT_OTEL_QUOTA_FILE` | *(Auto-discovered)* | Custom file path to read quota state JSON. Defaults to auto-discovering in `~/.state/quota.json`, `~/.gemini/quota.json`, or `~/.agent-otel/quota.json`. |
| `AGENT_OTEL_LEGACY_ATTRIBUTES` | `false` | When set to `true`, emits legacy `agy.*` aliases alongside canonical OpenTelemetry `gen_ai.*` and `agent.*` conventions for backward compatibility. |
| `OTEL_SEMCONV_STABILITY_OPT_IN` | *(unset)* | Comma-separated tokens. Span export changes only when a token equals `gen_ai_latest_experimental` after trimming. The match is case-sensitive. Any other token is ignored. Unset leaves the default export unchanged. |
| `AGENT_OTEL_ENRICH_EMAIL` | `false` | `true` or `1` also reads `$GIT_AUTHOR_EMAIL` and git config for `user.email`. In opt-in capture modes, otherwise only the payload and `$USER_EMAIL` are used. Minimal omits email and skips ambient email harvesting. |
| `AGENT_OTEL_WORKSPACE_PATH` | `hash` | Applies only in opt-in capture modes: `absolute` emits full paths, `relative` emits a path under the root (or omits it), `hash` emits a per-install keyed HMAC pseudonym (or omits it if the key is unavailable). Minimal always omits paths. |
| `AGENT_OTEL_CAPTURE_MODE` | `minimal` | Unset, invalid and explicit `minimal` use the content-free projection. `metadata_only` / `metadata-only` exposes operational identifiers; `full` also retains bounded error text. Both opt-ins use best-effort credential scrubbing. |

### Exported content under current defaults

The default `minimal` mode omits workspace paths, hostnames, email, content and raw errors. It projects custom names into per-install keyed pseudonyms, maps categories to closed vocabularies, and applies exact-key span/resource allowlists, including a final span check at the export batch boundary. Both `vcs.commit.sha` and experimental `vcs.ref.head.revision` contain `commit_<32hex>` pseudonyms (domain `commit`, canonical input: lowercase full hexadecimal SHA); without a key they are omitted. Raw commit SHAs can identify a public or forge-hosted repository through global search, so Minimal never exports them. Pseudonymization is not anonymity; permitted opaque session IDs still allow linkage. See [privacy projection](TELEMETRY_DICTIONARY.md#511-current-path-email-and-error-export-behaviour).

A2 follow-up: a non-opaque conversation ID is currently hashed **without a key** into `trace_id` before privacy projection. This can allow dictionary matching of predictable IDs; A2 must revisit that derivation. A1 leaves trace ID behavior unchanged.

The daemon loads `pseudonym.key` once at startup in its runtime home (`AGENT_OTEL_HOME` overrides; Windows `%LOCALAPPDATA%/agent-otel-bridge`; Unix `$XDG_DATA_HOME/agent-otel-bridge` or `$HOME/.local/share/agent-otel-bridge`). A missing key is generated with the OS CSPRNG and created exclusively (Unix 0600; Windows protected owner-only DACL, applied before writing key bytes and also on existing keys before reading). On Windows, a home outside the current user's profile triggers one static startup warning without paths or key material. The runtime directory still inherits its parent's ACL; protect that directory against access/replacement by other users even though the key file itself has a restricted DACL. Failure omits fields needing pseudonyms and still exports spans, recording `agent.bridge.privacy.key_unavailable.total{reason="key_unavailable"}`. Keys never appear in Debug, logs or exports. Pseudonyms use field-domain-separated HMAC-SHA256 truncated to 16 bytes and are stable only per install; A2 will replace this key with a customer K_link-derived key.

### OTLP/HTTP export

The daemon exports OTLP protobuf over HTTP. `Content-Type` is always `application/x-protobuf`; user headers cannot replace it. `Content-Encoding: gzip` is set only when that signal's compression is `gzip`. Retry, partial success, the 1 MiB request limit, and the 4 MiB response limit are unchanged. A disabled signal returns without opening a connection. `OTEL_SDK_DISABLED=true` reports the batch accepted with zero attempts.

Signal-specific variables override the general `OTEL_EXPORTER_OTLP_*` variable for that signal. The daemon and direct `emit-quota` export both use this contract. `OtlpExporter::new(traces_url, metrics_url)` remains a compatibility client and does not read environment variables.

### Resource identity

Before privacy projection, resource detection supplies service, host, OS, telemetry SDK/distro and process instance identity. Export applies the capture mode:

- Minimal allows only `service.name` (fixed to `agent-otel-bridge`, replacing `OTEL_SERVICE_NAME`), bridge `service.version`, closed `service.namespace=agent-otel-bridge`, the bridge's telemetry SDK/distro identity, `os.type`, `agent.capture.mode`, and the two deployment environment keys. Version overrides are replaced with the bridge version. Deployment values are closed (`development`, `dev`, `test`, `staging`, `production`, `prod`, `local`, otherwise `other`). Other `OTEL_RESOURCE_ATTRIBUTES` keys are dropped, including `host.name`, `service.instance.id`, `enduser.id`, `team`, `host.id`, `k8s.pod.name`, and `os.description`.
- Opt-in modes preserve operational resource overrides and apply best-effort credential scrubbing. Neither deployment environment key is invented or copied from the other. `os.type` is `darwin` for macOS.

The collector examples do not run the system resource detector, preventing it from reintroducing `host.name` after daemon projection.

### Experimental semantic conventions

The default trace export is unchanged when `OTEL_SEMCONV_STABILITY_OPT_IN` is unset or does not contain the exact token `gen_ai_latest_experimental`. When that token is present, trace spans differ as follows:

- `gen_ai.system` is omitted. `gen_ai.provider.name` is emitted only when the event model is present and recognized. A missing or unrecognized model does not substitute `google`.
- `gen_ai.response.finish_reasons` is a one-element string array of the raw harness string. `agent.termination_reason` stays a string copy. `NO_TOOL_CALL` is not mapped to `stop`.
- Successful spans use status `Unset`. Errors stay status `Error` with the same message. `error.type` is the event error category when that is non-empty, otherwise `_OTHER`.
- Lineage is `agent.lineage.depth`, `agent.lineage.parent_name`, `agent.lineage.root_id`, and `agent.lineage.is_root` instead of `gen_ai.agent.depth`, `gen_ai.agent.parent_name`, `gen_ai.agent.root_id`, and `gen_ai.agent.is_root`. `gen_ai.agent.step_index` is omitted. `agent.step.index` stays. No new `gen_ai.*` attribute is added, and `gen_ai.operation.name` is not set to `chat`.
- `vcs.branch.name` and `vcs.commit.sha` are emitted as `vcs.ref.head.name` and `vcs.ref.head.revision`. The same value is not emitted under both names.
- Trace `schema_url` on resource spans and scope spans is `https://opentelemetry.io/schemas/1.41.0`. Metrics requests are unchanged in this slice.

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

## 3. OpenTelemetry Collector Recipe

The daemon exports OTLP/HTTP protobuf. Authentication, sampling, and vendor export belong in the collector, not in the daemon. The maintained configs replace the old inline recipe:

- [`contrib/collector/local.yaml`](../contrib/collector/local.yaml) — workstation config. The daemon points at `http://localhost:4318`. This file only prints signals with the `debug` exporter. No upstream endpoint.
- [`contrib/collector/team.yaml`](../contrib/collector/team.yaml) — the same shape, plus tail sampling, path redaction, and an upstream `otlphttp` exporter. The upstream base URL is `AGENT_OTEL_COLLECTOR_UPSTREAM`. Do not reuse the daemon's `OTEL_EXPORTER_OTLP_ENDPOINT`.
- [`contrib/collector/README.md`](../contrib/collector/README.md) — validate command, upstream variable, and the span-metrics fragment.

A paired span's duration is the interval between the Pre and Post hook deliveries. Unpaired spans still have equal start and end. That interval is not a harness CPU timer, so the span-metrics fragment does not publish a duration histogram. The collector listens for the daemon on OTLP/HTTP. No gRPC receiver is required:

```yaml
receivers:
  otlp:
    protocols:
      http:
        endpoint: localhost:4318
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
