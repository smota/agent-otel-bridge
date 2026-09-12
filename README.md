<p align="center">
  <img src="assets/logo.svg" alt="agent-otel-bridge logo" width="800"/>
</p>

<p align="center">
  <strong>Ultra-Fast, Zero-Overhead OpenTelemetry Instrumentation Bridge for AI CLI Agent Harnesses</strong>
</p>

<p align="center">
  <a href="https://crates.io/crates/agent-otel-bridge"><img src="https://img.shields.io/crates/v/agent-otel-bridge?style=for-the-badge&logo=rust&color=blue" alt="Crates.io"/></a>
  <a href="https://docs.rs/agent-otel-core"><img src="https://img.shields.io/docsrs/agent-otel-core?style=for-the-badge&logo=docs.rs&label=docs.rs" alt="docs.rs"/></a>
  <a href="https://github.com/smota/agent-otel-bridge/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/smota/agent-otel-bridge/ci.yml?branch=main&style=for-the-badge&logo=github&label=CI" alt="CI Status"/></a>
  <a href="https://opentelemetry.io/"><img src="https://img.shields.io/badge/OpenTelemetry-OTel_GenAI_v1.28-4B5563?style=for-the-badge&logo=opentelemetry&logoColor=4B78E6" alt="OpenTelemetry"/></a>
  <a href="https://opentelemetry.io/"><img src="https://img.shields.io/badge/OTLP-Traces_%26_Metrics-0284C7?style=for-the-badge&logo=opentelemetry&logoColor=white" alt="OTLP Standards"/></a>
  <a href="docs/COMMUNITY_BENCHMARKS.md"><img src="https://img.shields.io/badge/Community_Benchmarks-Leaderboard-7C3AED?style=for-the-badge&logo=speedtest&logoColor=white" alt="Community Benchmarks"/></a>
  <a href="#benchmarks"><img src="https://img.shields.io/badge/Hot--Path_Latency-%3C_3ms-10B981?style=for-the-badge&logo=speedtest&logoColor=white" alt="Latency SLA"/></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-Apache_2.0-0284C7?style=for-the-badge" alt="License"/></a>
</p>

<p align="center">
  <strong>Supported Agent Harnesses:</strong><br/>
  <a href="https://deepmind.google/technologies/gemini/"><img src="https://img.shields.io/badge/Google_Antigravity-Supported-4285F4?style=flat-square&logo=google&logoColor=white" alt="Google Antigravity"/></a>
  <a href="https://claude.ai/"><img src="https://img.shields.io/badge/Claude_Code-Supported-D97706?style=flat-square&logo=anthropic&logoColor=white" alt="Claude Code"/></a>
  <a href="https://openai.com/"><img src="https://img.shields.io/badge/OpenAI_Codex-Supported-10A37F?style=flat-square&logo=openai&logoColor=white" alt="OpenAI Codex"/></a>
  <a href="https://x.ai/"><img src="https://img.shields.io/badge/xAI_Grok-Supported-1D9BF0?style=flat-square&logo=x&logoColor=white" alt="xAI Grok"/></a>
  <a href="https://pi.dev/"><img src="https://img.shields.io/badge/Pi-Supported-6366F1?style=flat-square&logo=sparkles&logoColor=white" alt="Pi"/></a>
</p>

<p align="center">
  <a href="https://www.movetheneedle.info">
    <img src="assets/move-the-needle-logo.png" alt="Sponsored by Move the Needle" width="260"/>
  </a>
  <br/>
  <em>Proudly sponsored and incubated by <a href="https://www.movetheneedle.info">Move the Needle</a></em>
</p>

---

## Overview

**`agent-otel-bridge`** is a high-performance, native OpenTelemetry sidecar bridge designed to solve the **hot-path lifecycle hook latency bottleneck** in autonomous AI CLI agent harnesses (**Google Antigravity**, **Claude Code**, **OpenAI Codex CLI**, **xAI Grok**, **Pi [pi.dev]**, and custom LLM developer agents).

Modern AI coding agents invoke synchronous lifecycle hooks (`PostToolUse`, `PreInvocation`, `PostInvocation`, `Stop`) dozens to hundreds of times per session. Traditional script-based hooks (Python, Node.js, PowerShell) impose **150ms to 1,200ms of cold-start latency per tool call**, accumulating up to several minutes of wasted developer time in a single pairing session.

`agent-otel-bridge` decouples hot-path event emission into:
1. **`agent-hook.exe`**: An ultra-fast, microscopic static PE32 binary (**242 KB**) that executes in **< 1ms**, drops the event into a local Win32 Named Pipe via Overlapped I/O, and enforces a **3ms watchdog fail-open guarantee** (never blocks or breaks the agent's workflow).
2. **`agent-otel-bridge daemon`**: A Tokio async background daemon maintaining connection pooling with the OpenTelemetry Collector (`localhost:4318`), performing micro-batching (50 spans / 200ms), and serializing canonical OTLP Protobuf traces and quota metrics without requiring `protoc` build-time dependencies.

---

## Architecture

```mermaid
flowchart TD
    subgraph AgentHarness ["AI CLI Agent Harnesses (Antigravity / Claude / Codex / Grok / Pi)"]
        HookEvent["Synchronous Lifecycle Hook (e.g. PostToolUse)"]
    end

    subgraph ClientPath ["Ultra-Fast Client (< 1ms execution)"]
        HookBin["agent-hook.exe (242 KB Native PE32)"]
        Watchdog["Hard Watchdog Thread (3ms Deadline)"]
        PipeClient["Win32 Overlapped Client (\\.\\pipe\\agent-otel)"]
    end

    subgraph DaemonProcess ["agent-otel-bridge daemon (Tokio Background Process)"]
        PipeServer["Named Pipe Server (64KB Ring Buffer)"]
        Parser["ProtoJSON Parser & SemConv Mapper"]
        Batcher["Micro-Batcher (Configurable Buffer)"]
        QuotaEngine["Quota & Heartbeat Engine"]
        OtlpExporter["OTLP HTTP/Protobuf Exporter (Keep-Alive Pool)"]
    end

    subgraph OTelPipeline ["OpenTelemetry Collector Pipeline"]
        Collector["OTel Collector (:4318 / :4317)"]
        Processor["Batch / Transform Processor (OTTL)"]
    end

    subgraph ObservabilityBackends ["Pluggable Observability Ecosystem"]
        Traces["Distributed Traces (Jaeger / Tempo / SigNoz / Datadog)"]
        Metrics["Metrics & Gauges (Prometheus / Mimir / VictoriaMetrics)"]
        Dashboards["Dashboards (Grafana / SigNoz / Honeycomb)"]
    end

    HookEvent -->|"stdin (ProtoJSON)"| HookBin
    HookBin -->|"Spawn watchdog"| Watchdog
    HookBin -->|"Non-blocking write"| PipeClient
    HookBin -->|"stdout: {} & exit 0"| HookEvent
    PipeClient -->|"Binary Wire Framing [AG:v1]"| PipeServer

    PipeServer -->|"Tokio mpsc channel"| Parser
    Parser --> Batcher
    QuotaEngine -->|"Metrics"| OtlpExporter
    Batcher -->|"Traces"| OtlpExporter
    OtlpExporter -->|"POST /v1/traces (Protobuf)"| Collector
    OtlpExporter -->|"POST /v1/metrics (Protobuf)"| Collector
    Collector --> Processor
    Processor --> Traces
    Processor --> Metrics
    Processor --> Dashboards
```

---

## Benchmarks

Measured on Windows 11 (AMD Ryzen / NVMe) via the dedicated benchmark suite (`agent-otel-bench`):

| Measurement Stage | p50 (median) | p90 | p95 | p99 | p99.9 | SLA Target | Status |
|---|---:|---:|---:|---:|---:|---:|:---:|
| **Win32 Named Pipe RTT** | **22 µs** | **38 µs** | **57 µs** | **101 µs** | **277 µs** | < 3,000 µs | ✅ **PASS** |
| **ProtoJSON Parse + Span Build** | **1 µs** | **1 µs** | **2 µs** | **2 µs** | **5 µs** | < 3,000 µs | ✅ **PASS** |
| **Real OS Process Lifecycle** | **3.56 ms** | **4.25 ms** | **4.70 ms** | **5.59 ms** | **8.36 ms** | < 10.0 ms | ✅ **PASS** |

- **OTLP Span Processing Throughput**: **> 546,000 spans/second**.
- **Hook Cost across 200 Tool Invocations**:
  - PowerShell Hook Script: **90s – 240s** lost.
  - Python Hook Script: **30s – 56s** lost.
  - `agent-hook.exe` (Rust): **0.7s total accumulated time**.
- Full methodology and data in [`docs/BENCHMARKS.md`](docs/BENCHMARKS.md).

---

## OpenTelemetry Semantic Conventions

Fully aligned with canonical **CNCF OpenTelemetry GenAI & Agent Semantic Conventions (v1.28+ / v1.36)**:

### Spans & Canonical Attributes
- **Span Names**:
  - `execute_tool {gen_ai.tool.name}` (`SpanKind::Internal`)
  - `invoke_agent {gen_ai.agent.name}` (`SpanKind::Internal`)
  - `agent.stop` (`SpanKind::Internal`)
- **GenAI Attributes**:
  - `gen_ai.operation.name`: `execute_tool`, `invoke_agent`
  - `gen_ai.provider.name`: dynamically inferred (`"anthropic"`, `"google"`, `"openai"`, `"xai"`, `"pi"`)
  - `gen_ai.agent.name`: dynamically inferred (`"antigravity"`, `"claude-code"`, `"codex"`, `"grok"`, `"pi"`, `"ai-agent"`)
  - `gen_ai.conversation.id`: correlated session identifier (mapped from `conversationId` or Claude Code `session_id`)
  - `gen_ai.tool.name` & `gen_ai.tool.call.id`
  - `gen_ai.request.model`: e.g. `claude-3-5-sonnet`, `gemini-2.5-pro`, `gpt-4o`, `grok-2`
- **Canonical Agent Attributes**:
  - `agent.hook.event`: `PostToolUse`, `PreToolUse`, `PostInvocation`, `PreInvocation`, `Stop`
  - `agent.step.index` & `gen_ai.agent.step_index`: integer turn index
  - `agent.execution.num`: execution sequence number
  - `agent.fully_idle`: boolean quiescence indicator
  - `agent.termination_reason`: stop / completion reason (e.g. `NO_TOOL_CALL`, `model_stop`)
- **Agent Quota Gauges**:
  - `agent.quota.remaining_fraction`: Gauge (unit `"1"`, `0.0..=1.0`).
  - `agent.quota.seconds_to_reset`: Gauge (unit `"s"`, `>= 0.0`).
  - Attributes: `bucket`, `group`.
- **Universal Agent Intelligence & Context (v0.3)**:
  - **Behavioral CLI Archetypes (Preference-Agnostic)**: Commands are categorized into 6 functional archetypes (`filter_compressor`, `structured_parser`, `inspector_diff`, `search_retrieval`, `build_test_verify`, `generic_exec`) instead of hardcoding developer-specific tools (`rtk`, `jq`, `bat`). Tracks compression ratio, tokens saved, and pipeline depth.
  - **Universal MCP & Skills Taxonomy + Waste Tracking**: Standardizes tool calls across `mcp` servers, `skill` bundles, and `native` tools. Quantifies **Schema Tax** (dead-weight prompt tokens spent on inactive tools), retry thrashing, and response payload bloat.
  - **Multi-Tier Zero-Subprocess Context Harvester (< 150 µs)**: Extracts workspace path, project root, project type (`rust`, `node`, `python`, `go`), and direct file-based Git metadata (`.git/HEAD`, sanitized remote origin, worktrees) with zero external process calls (`git.exe` is never spawned).
  - **Cross-Agent Distributed Tracing (W3C `traceparent`)**: Injects and propagates W3C trace context via process environment variables (`$env:TRACEPARENT`). Heterogeneous subagents (e.g. Antigravity spawning Claude Code, which delegates to Codex) are unified into a single distributed trace DAG in SigNoz.
- **OpenTelemetry Layering & Opt-In Compatibility**:
  - Emits purely canonical, vendor-neutral attributes by default so all AI agents (Antigravity, Claude Code, Codex, Grok, Pi) share a clean data model without vendor pollution.
  - Optional `AGENT_OTEL_LEGACY_ATTRIBUTES=true` enables legacy `agy.*` aliases for older dashboards, or use an OpenTelemetry Collector `transformprocessor` (OTTL) to alias attributes in the collection tier.

---

## Crates in the Workspace

| Crate | Binary / Role | Description |
|---|---|---|
| [`agent-otel-core`](crates/agent-otel-core) | Library | Domain models, SemConv constants, deterministic 16-byte `trace_id` / 8-byte `span_id` generators, OTLP Protobuf builders. |
| [`agent-otel-ipc`](crates/agent-otel-ipc) | Library | Binary wire protocol (`MAGIC: AG`), Win32 Overlapped client (`windows-sys`), Tokio Named Pipe server (`\\.\pipe\agent-otel`). |
| [`agent-otel-client`](crates/agent-otel-client) | `agent-hook.exe` | 242 KB static PE32 binary with 3ms watchdog fail-open backstop. |
| [`agent-otel-daemon`](crates/agent-otel-daemon) | Library | Tokio micro-batcher, `reqwest` keep-alive exporter, quota ticker, 30-min idle shutdown. |
| [`agent-otel-bridge`](crates/agent-otel-cli) | `agent-otel-bridge.exe` | Unified CLI (`hook`, `daemon`, `doctor`, `hooks`, `install-hooks`, `emit-quota`, `stop`). |
| [`agent-otel-bench`](crates/agent-otel-bench) | `agent-otel-bench.exe` | High-precision microsecond benchmark suite. |

---

## Installation & Setup

### 1. Install via Cargo or Pre-built Binaries

From [crates.io](https://crates.io/crates/agent-otel-bridge):
```powershell
cargo install agent-otel-bridge
cargo install agent-otel-client
```

Or download pre-compiled release archives directly from [GitHub Releases](https://github.com/smota/agent-otel-bridge/releases).

Or build locally from repository:
```powershell
cargo install --path crates/agent-otel-client --force
cargo install --path crates/agent-otel-cli --force
```

### 2. Automated Hook Installation (`install-hooks`)

`agent-otel-bridge` automatically detects and configures lifecycle hooks for your AI agents:

```powershell
# Automatically detects installed agents (Antigravity, Claude Code, Codex, Grok, Pi) and registers hooks
agent-otel-bridge install-hooks

# Or target a specific client
agent-otel-bridge install-hooks --client antigravity
agent-otel-bridge install-hooks --client claude
agent-otel-bridge install-hooks --client codex
agent-otel-bridge install-hooks --client grok
agent-otel-bridge install-hooks --client pi

# Or register hooks for all supported clients
agent-otel-bridge install-hooks --client all

# Or install at project level (.claude/settings.json or .gemini/hooks.json, etc.)
agent-otel-bridge install-hooks --client claude --project
```

Check hook status anytime:
```powershell
agent-otel-bridge hooks status
```

### 3. Verify Installation (`doctor`)
```powershell
agent-otel-bridge doctor
```
Output:
```text
=== agent-otel-bridge doctor ===
Diagnosing station telemetry pipeline & OpenTelemetry invariants

[1/5] Checking environment contract variables...
  [ok] OTEL_EXPORTER_OTLP_ENDPOINT = http://127.0.0.1:4318
  [info] OTEL_SERVICE_NAME is unset (using default 'agent-otel-bridge')
  [ok] OTEL_RESOURCE_ATTRIBUTES = deployment.environment=homelab

[2/5] Checking named pipe IPC (\\.\pipe\agent-otel)...
  [ok] Daemon is RUNNING and responding on named pipe!

[3/5] Checking OTLP Collector HTTP endpoint...
  [ok] OTLP Collector reachable at http://127.0.0.1:4318/v1/traces

[4/5] Checking Observability UI reachability (http://localhost:8080)...
  [ok] Observability UI reachable at http://localhost:8080

[5/5] Checking Client Hook Registrations...
  agent-hook in PATH: [ok] present
  Google Antigravity: [ok] registered (C:\Users\samue\.gemini\config\hooks.json)
  Claude Code:        [ok] registered (C:\Users\samue\.claude\settings.json)
  OpenAI Codex:       [info] not registered (C:\Users\samue\.codex\hooks.json)
  xAI Grok:           [info] not registered (C:\Users\samue\.grok\hooks.json)
  Pi (pi.dev):        [info] not registered (C:\Users\samue\.pi\hooks.json)
```

### 4. Run CLI Commands
```powershell
# Start background daemon
agent-otel-bridge start

# Send manual quota probe to collector
agent-otel-bridge emit-quota --ping

# Gracefully stop daemon
agent-otel-bridge stop

# Run benchmarks and submit results to community leaderboard
cargo run --release -p agent-otel-bench -- --submit --open-browser
# Or via unified CLI:
agent-otel-bridge benchmark --submit --open-browser
```

---

## Documentation

### 🏛️ Architecture & Core Principles
| Document | Description |
|---|---|
| [Native CLI Instrumentation Design](docs/NATIVE_CLI_INSTRUMENTATION.md) | Sub-millisecond architecture, 1-byte bitwise protocol, watchdog fail-open, and zero prompt pollution. |
| [Architecture & Sequence Diagrams](docs/ARCHITECTURE.md) | Zero-copy IPC wire protocol and Win32 Named Pipe / Unix Domain Socket architecture. |
| [AI Agent Architectural Invariants & SLAs](AGENTS.md) | Enforced design constraints, sub-millisecond SLAs, and AI agent operating instructions. |

### 🤖 Harness Integrations & Client Setup
| Document | Description |
|---|---|
| [AI Agent Harness Integration](docs/CLIENTS.md) | Setup & hook configs for Google Antigravity, Claude Code, OpenAI Codex, xAI Grok, Pi, and Custom Agents. |
| [Agent Observability Skill](skills/agent-otel/SKILL.md) | Built-in pair programming skill for inspecting and diagnosing telemetry pipelines. |

### 📊 Telemetry Standards & Dashboards
| Document | Description |
|---|---|
| [Telemetry Taxonomy & Variable Dictionary](docs/TELEMETRY_DICTIONARY.md) | Complete captured attribute dictionary, 6-dimension taxonomy, and GenAI conventions. |
| [OpenTelemetry Dashboard & Visualization Guide](docs/DASHBOARDS.md) | Panel queries and visualization recipes across Grafana, SigNoz, Jaeger, and Prometheus. |
| [SigNoz Contrib Dashboard Templates](contrib/dashboards/signoz/README.md) | Ready-to-import production JSON templates (Tokenomics, Archetypes, Fleet Governance, SRE Loops). |

### ⚙️ Configuration & Diagnostics
| Document | Description |
|---|---|
| [Configuration & Environment](docs/CONFIGURATION.md) | Complete environment variable catalog and OTel Collector recipes (`config.yaml`). |
| [Troubleshooting & Diagnostic Runbook](docs/TROUBLESHOOTING.md) | Step-by-step resolution for common issues and fail-open verification. |

### ⚡ Benchmarks & Performance SLAs
| Document | Description |
|---|---|
| [Performance Benchmark Report](docs/BENCHMARKS.md) | Detailed microsecond measurement methodology and hardware SLA evaluation. |
| [Community Benchmark Matrix](docs/COMMUNITY_BENCHMARKS.md) | Living leaderboard comparing benchmark results across community hardware. |
| [Benchmark Submission Guide](docs/BENCHMARK_SUBMISSION.md) | Instructions on running automated benchmarks and contributing hardware results. |

### 🤝 Contributing & Quality Guardrails
| Document | Description |
|---|---|
| [Contributing & Branching Policy](CONTRIBUTING.md) | Branching strategy (`feat/*`, `fix/*`), PR workflow, and automated guardrail verification. |

---

## License

Copyright (c) 2026 Samuel Mota. Licensed under the [Apache License, Version 2.0](LICENSE).
