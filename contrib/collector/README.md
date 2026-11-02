# Collector kit

These files replace the inline collector recipe that used to live in `docs/CONFIGURATION.md`. The daemon already exports OTLP/HTTP protobuf. Authentication, sampling, and vendor export belong in this collector configuration, not in the daemon and not in a custom receiver or distro.

| File | Role |
| --- | --- |
| `local.yaml` | Workstation `otelcol-contrib` config. OTLP/HTTP on `localhost:4318`, resource detection, `user.email` redaction, memory limiter, and batch. The `debug` exporter only prints signals. No upstream endpoint. |
| `team.yaml` | Same shape, plus tail sampling, redaction of `workspace.path` and `workspace.project_root`, and an `otlphttp` upstream. |
| `span-metrics.yaml` | `spanmetrics` fragment. Counts spans as `agent.span.calls` by `gen_ai.operation.name` and `agent.tool.archetype` only. |

A paired span's duration is the interval between the Pre and Post hook deliveries. Unpaired spans still have equal start and end. Do not chart that field as a harness CPU timer. The span-metrics fragment disables the connector histogram for that reason. Team tail sampling uses status code and a 5% probabilistic policy. It does not use a latency policy.

Quota gauges already emitted by the daemon stay named `agent.quota.*` and `agent.fleet.bottleneck_ratio`. This kit does not rename them.

## Run

Install `otelcol-contrib` yourself. From the repository root:

```bash
export OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
export OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf
export AGENT_OTEL_COLLECTOR_UPSTREAM=https://otlp.example.com
export OTEL_EXPORTER_OTLP_AUTH_HEADER='Bearer <token>'

otelcol-contrib validate --config contrib/collector/local.yaml
otelcol-contrib validate --config contrib/collector/team.yaml
otelcol-contrib --config contrib/collector/local.yaml
```

`OTEL_EXPORTER_OTLP_ENDPOINT` is only the daemon's address for this local collector (`http://localhost:4318`). `local.yaml` prints those signals with the `debug` exporter and has no upstream. `team.yaml` sends them to `AGENT_OTEL_COLLECTOR_UPSTREAM`, a base URL. The `otlphttp` exporter appends `/v1/traces` and `/v1/metrics`. Do not reuse the daemon variable as that upstream. Put the vendor credential in `OTEL_EXPORTER_OTLP_AUTH_HEADER`. Do not commit a token.

`span-metrics.yaml` is a fragment. Merge its `connectors` block and the commented pipeline wiring into `local.yaml` or `team.yaml` before validating that file. On `local.yaml` the extra exporter is `debug`. On `team.yaml` it is `otlphttp`.

The daemon points at the collector with `OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318` and `OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf`.

The Grafana board is `contrib/dashboards/grafana/agent-otel-bridge.json`. It reads `agent.span.calls` and the existing quota gauges. It is not a SigNoz board and it has no duration percentile panel.
