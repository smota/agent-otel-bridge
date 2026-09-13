<p align="center">
  <img src="assets/logo.svg" alt="agent-otel-bridge logo" width="800"/>
</p>

<p align="center">
  <strong>Ultra-Fast, Zero-Overhead OpenTelemetry Instrumentation Bridge for AI CLI Agent Harnesses</strong><br/>
  <em>Cross-Platform: Linux • macOS • Windows</em>
</p>

<p align="center">
  <a href="https://crates.io/crates/agent-otel-bridge"><img src="https://img.shields.io/crates/v/agent-otel-bridge?style=for-the-badge&logo=rust&color=blue" alt="Crates.io"/></a>
  <a href="https://docs.rs/agent-otel-core"><img src="https://img.shields.io/docsrs/agent-otel-core?style=for-the-badge&logo=docs.rs&label=docs.rs" alt="docs.rs"/></a>
  <a href="https://github.com/smota/agent-otel-bridge/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/smota/agent-otel-bridge/ci.yml?branch=main&style=for-the-badge&logo=github&label=CI" alt="CI Status"/></a>
  <a href="https://opentelemetry.io/"><img src="https://img.shields.io/badge/OpenTelemetry-OTel_GenAI_v1.28-4B5563?style=for-the-badge&logo=opentelemetry&logoColor=4B78E6" alt="OpenTelemetry"/></a>
  <a href="https://opentelemetry.io/"><img src="https://img.shields.io/badge/OTLP-Traces_%26_Metrics-0284C7?style=for-the-badge&logo=opentelemetry&logoColor=white" alt="OTLP Standards"/></a>
  <a href="#benchmarks"><img src="https://img.shields.io/badge/Hot--Path_Latency-%3C_1ms_(~150µs_IPC)-10B981?style=for-the-badge&logo=speedtest&logoColor=white" alt="Latency SLA"/></a>
  <a href="#benchmarks"><img src="https://img.shields.io/badge/Client_Binary-145_KB-7C3AED?style=for-the-badge&logo=rust&logoColor=white" alt="Binary Size SLA"/></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-Apache_2.0-0284C7?style=for-the-badge" alt="License"/></a>
</p>

<p align="center">
  <strong>Supported AI Agent Harnesses:</strong><br/>
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

## The Engineering Problem: Why agent-otel-bridge?

Modern autonomous AI CLI agent harnesses (**Google Antigravity**, **Claude Code**, **OpenAI Codex**, **xAI Grok**, **Pi [pi.dev]**) operate through rapid, iterative reasoning loops. An agent executing a single feature frequently triggers dozens to hundreds of tool executions (`PreToolUse`, `PostToolUse`, `PreInvocation`, `PostInvocation`, `Stop`).

These lifecycle hooks execute **synchronously on the agent's critical path**. This creates two fatal infrastructure bottlenecks:

### 1. The Hot-Path Latency Penalty
Traditional script-based telemetry hooks (written in Python, Node.js, or PowerShell) incur **150ms to 1,200ms of process cold-start and runtime initialization latency per tool call**.
* Across a typical 50-turn agent session, script-based hooks waste **25 to 60 seconds** of developer dead time staring at blank terminals.
* If a telemetry script hangs (e.g., waiting for an unreachable remote HTTP endpoint), it freezes the AI agent turn completely.

### 2. The Semantic & Distributed Observability Gap
* **Siloed & Incompatible Telemetry**: Native client logs are fragmented across disparate local formats (`.claude/projects/*.jsonl`, `.gemini/` event files, raw stdout) without semantic standardization.
* **Opaque Command Blobs**: Commands are dumped as raw strings (e.g., `bash: git diff | head -n 20`). Dashboards cannot distinguish whether the agent is compiling code, inspecting diffs, or performing destructive filesystem mutations.
* **Broken Cross-Agent Tracing**: When a parent agent spawns a child subagent or delegates to a secondary CLI tool, causal lineage is lost. There is zero distributed context propagation.

---

## The Architectural Solution: How It Works

`agent-otel-bridge` solves this with a **decoupled, two-tier architecture** designed around strict microsecond-level performance invariants:

```mermaid
flowchart TD
    subgraph AgentHarness ["AI CLI Agent Harnesses (Antigravity / Claude / Codex / Grok / Pi)"]
        HookEvent["Synchronous Lifecycle Hook (e.g. PostToolUse)"]
    end

    subgraph ClientPath ["Tier 1: Ultra-Fast Native Client (< 1ms execution)"]
        HookBin["agent-hook (145 KB Native Binary — ELF / Mach-O / PE32)"]
        Watchdog["Hard OS Watchdog Thread (3ms Fail-Open Deadline)"]
        IPCClient["Zero-Copy IPC Client\n• Unix Domain Socket: /tmp/agent_otel_bridge.sock\n• Win32 Named Pipe: \\\\.\\pipe\\agent-otel"]
    end

    subgraph DaemonProcess ["Tier 2: Asynchronous Micro-Batcher (Tokio Background Daemon)"]
        IPCServer["IPC Server (Tokio Async Ring Buffer)"]
        ArchetypeEngine["Behavioral Archetype Engine (< 5µs)\n10 Execution Archetypes + Flag-Aware Subcommands"]
        ContextHarvester["Zero-Subprocess Context Harvester (< 150µs)\n.git/HEAD direct read + Credential Sanitization"]
        Batcher["Micro-Batcher (50 spans / 200ms window)"]
        QuotaEngine["Machine-Adaptive Quota Engine"]
        OtlpExporter["OTLP HTTP/Protobuf Exporter (Keep-Alive Pool)"]
    end

    subgraph OTelPipeline ["OpenTelemetry Collector Pipeline"]
        Collector["OTel Collector (:4318 HTTP / :4317 gRPC)"]
        Processor["Batch / Transform Processor (OTTL)"]
    end

    subgraph ObservabilityBackends ["Pluggable Observability Ecosystem"]
        SigNoz["SigNoz (Turnkey FDE Dashboards)"]
        Grafana["Grafana / Tempo / Mimir"]
        Datadog["Datadog / Honeycomb"]
    end

    HookEvent -->|"stdin (JSON)"| HookBin
    HookBin -->|"Spawn watchdog"| Watchdog
    HookBin -->|"Non-blocking write"| IPCClient
    HookBin -->|"stdout: {} & exit 0"| HookEvent
    IPCClient -->|"3-Byte Wire Protocol [event_id, client_id]"| IPCServer

    IPCServer -->|"Tokio mpsc channel"| ArchetypeEngine
    ArchetypeEngine --> ContextHarvester
    ContextHarvester --> Batcher
    QuotaEngine -->|"Metrics"| OtlpExporter
    Batcher -->|"Traces"| OtlpExporter
    OtlpExporter -->|"POST /v1/traces (Protobuf)"| Collector
    OtlpExporter -->|"POST /v1/metrics (Protobuf)"| Collector
    Collector --> Processor
    Processor --> SigNoz
    Processor --> Grafana
    Processor --> Datadog
```

### 1. Tier 1: Microscopic Native Client (`agent-hook`)
* **Sub-Millisecond Execution**: A compiled native binary (**145 KB**, stripped, LTO enabled) with zero runtime dependencies (no tokio, no reqwest, no regex engines).
* **3-Byte Binary Wire Framing (`WireHeader`)**: Encodes `[u8 event_id, u16 client_id]` via zero-allocation array slicing, providing headroom for **65,535 AI platforms** and 256 lifecycle events.
* **Cross-Platform Native IPC**:
  * **Linux & macOS**: POSIX Non-blocking Unix Domain Sockets (`/tmp/agent_otel_bridge.sock`).
  * **Windows**: Win32 Overlapped Named Pipes (`\\.\pipe\agent-otel`).
  * **Measured Round-Trip Time**: **~150 µs** (p50: 22 µs).
* **Hard Fail-Open Watchdog**: A dedicated background OS thread enforces a **3.0 ms deadline**. If IPC stalls or the daemon is unavailable, the client immediately terminates with `{}` and `exit 0`. **The AI agent turn is never blocked or failed.**

### 2. Tier 2: Tokio Async Micro-Batcher (`agent-otel-bridge daemon`)
* Operates entirely off the agent's critical path.
* **Micro-Batching**: Buffers spans into 50-item batches or flushes every 200ms.
* **Persistent Connection Pooling**: Maintains persistent HTTP keep-alive connections to local or remote OTLP endpoints (`http://localhost:4318/v1/traces`).
* **Zero-Dependency Protobuf Serialization**: Hand-optimized OTLP Protobuf encoders via `prost` without requiring `protoc` installed on developer machines.
* **30-Minute Idle Quiescence**: Automatically shuts down after 30 minutes of agent inactivity to preserve workstation battery and memory.

### 3. The Semantic Intelligence Layer
* **10 Behavioral Execution Archetypes**: Automatically classifies arbitrary commands into 10 preference-agnostic behavioral archetypes (`FilterCompressor`, `StructuredParser`, `InspectorDiff`, `SearchRetrieval`, `BuildTestVerify`, `StateMutation`, `EnvPkgManager`, `VcsLifecycle`, `NetworkTransfer`, `GenericExec`) with option-aware subcommand parsing (`git -C dir commit`, `cargo +nightly test`) in **$< 5\ \mu\text{s}$**.
* **Zero-Subprocess Context Harvesting**: Reads `.git/HEAD`, detached worktrees, and sanitized remote URLs directly from the filesystem in **$< 150\ \mu\text{s}$**. External processes like `git.exe` are **never spawned** on the telemetry path.
* **Cross-Agent Distributed Tracing**: Propagates W3C `traceparent` environment variables (`00-{trace_id}-{span_id}-01`). Heterogeneous multi-agent chains (e.g. Antigravity $\to$ Claude Code $\to$ Codex CLI) appear as unified flamegraphs in your APM.
* **Machine-Adaptive Quota Intelligence**: Detects installed harnesses (`is_installed`) and emits headroom metrics *only* for tools present on the machine, preventing phantom dashboard clutter.

> 📚 **Deep Dive Guides**:
> - [Why agent-otel-bridge vs. Native Harness Telemetry](docs/WHY_AGENT_OTEL_BRIDGE.md) — Comprehensive architectural evaluation.
> - [System Limitations & Mitigations Catalog](docs/LIMITATIONS.md) — Design constraints, fail-open trade-offs, and mitigations.
> - [Local Runtime & Station Contract](docs/local-runtime-contract.md) — Binary staging, atomic symlinking, and hook idempotency.

---

## Hardware Benchmarks & Performance SLAs

Continuously validated across Linux and Windows via `agent-otel-bench` and hardware guardrails:

| Boundary / Subsystem | Hard Target SLA | Measured Performance | Verification Tool |
|---|:---:|:---:|---|
| **`agent-hook` Binary Size** | **< 300 KB** | **145.5 KB** (opt-level "s", strip, lto) | `cargo guardrails` |
| **Local IPC Round-Trip (p99)** | **< 3,000 µs** | **101 µs** (Named Pipe) / **~80 µs** (Unix Socket) | `agent-otel-bench` |
| **Archetype Classifier Latency** | **< 15 µs** | **~2.8 µs** (Zero disk I/O, compiled matchers) | `core_tests` |
| **Context Harvester Latency** | **< 150 µs** | **~28 µs** (Direct filesystem read, zero subprocesses) | `test_harvest_current` |
| **ProtoJSON Parser Throughput** | **> 50,000 spans/s** | **62,943 spans/s** (Mean: 14.4 µs) | `agent-otel-bench --submit` |
| **Watchdog Fail-Open Deadline** | **3.0 ms** | **Guaranteed exit 0** | Dedicated OS Watchdog Thread |
| **Process Cold-Start Impact** | **< 1.0 ms** | **~150 µs** (accumulates < 0.7s per 200 turns) | Hardware benchmarks |

*Full methodology, raw data, and multi-platform results are documented in [`docs/BENCHMARKS.md`](docs/BENCHMARKS.md).*

---

## OpenTelemetry Semantic Conventions

Fully compliant with **CNCF OpenTelemetry GenAI & Agent Semantic Conventions (v1.28+ / v1.36)**:

* **Spans**: `execute_tool {gen_ai.tool.name}` (`SpanKind::Internal`), `invoke_agent {gen_ai.agent.name}`, `agent.stop`.
* **GenAI Attributes**: `gen_ai.operation.name`, `gen_ai.provider.name`, `gen_ai.agent.name`, `gen_ai.conversation.id`, `gen_ai.tool.name`, `gen_ai.request.model`.
* **Behavioral Archetypes (v0.5.1)**: `agent.tool.archetype` (`filter_compressor`, `structured_parser`, `inspector_diff`, `search_retrieval`, `build_test_verify`, `state_mutation`, `env_pkg_manager`, `vcs_lifecycle`, `network_transfer`, `generic_exec`), `agent.tool.binary`, `agent.tool.pipeline_depth`.
* **Workspace & VCS Context**: `workspace.project_name`, `workspace.project_type`, `workspace.project_root`, `vcs.system`, `vcs.branch.name`, `vcs.origin.url`.
* **Tokenomics & Waste**: `tool.tokens_saved_estimate`, `tool.compression_ratio`, `capability.kind` (`mcp`, `skill`, `subagent`, `native`), `capability.schema_tokens`.
* **Adaptive Quota Metrics**: `agent.quota.remaining_fraction`, `agent.quota.seconds_to_reset`, `gen_ai.client.quota.fleet_bottleneck_ratio`.

*Complete attribute dictionary available in [`docs/TELEMETRY_DICTIONARY.md`](docs/TELEMETRY_DICTIONARY.md).*

---

## Workspace Crates

```
crates/
├── agent-otel-core      # Domain models, SemConv constants, context harvester, archetype classifier, W3C trace_id.
├── agent-otel-ipc       # Wire framing, Unix Domain Socket client/server, Win32 Overlapped client, Tokio Named Pipe server.
├── agent-otel-client    # Ultra-lean agent-hook binary (< 150 KB, zero external runtime dependencies).
├── agent-otel-daemon    # Background Tokio micro-batcher, OTLP Protobuf exporter, machine-adaptive quota engine.
├── agent-otel-cli       # Unified developer CLI: start, stop, doctor, local, hooks, install-hooks, scan-all, benchmark.
└── agent-otel-bench     # High-precision hardware microsecond benchmark suite with JSON/Markdown reporting.
```

---

## Installation & Setup

### 1. Install via Cargo or Pre-Built Binaries

```bash
# Install CLI and hook binary from crates.io
cargo install agent-otel-bridge
cargo install agent-otel-client
```

Or download pre-compiled release archives directly from [GitHub Releases](https://github.com/smota/agent-otel-bridge/releases).

Or build from source repository:
```bash
git clone https://github.com/smota/agent-otel-bridge.git
cd agent-otel-bridge
cargo build --release --workspace
```

### 2. Deploy to Isolated Local Runtime (`local install`)

Deploy the compiled binaries into your operating system's isolated canonical runtime path:

```bash
# On Linux / macOS (installs to ~/.local/share/agent-otel-bridge/bin)
agent-otel-bridge local install

# On Windows (installs to %LOCALAPPDATA%\agent-otel-bridge\bin)
agent-otel-bridge local install
```

`local install` stages versioned builds, computes SHA-256 integrity hashes, updates the active symlink, and idempotently registers canonical hook paths.

### 3. Automated Hook Registration (`install-hooks`)

Automatically detect installed AI agent harnesses and configure their lifecycle hooks:

```bash
# Automatically detects Antigravity, Claude Code, Codex CLI, Grok, Pi and configures hooks
agent-otel-bridge install-hooks

# Or register hooks for a specific platform
agent-otel-bridge install-hooks --client claude
agent-otel-bridge install-hooks --client antigravity
agent-otel-bridge install-hooks --client codex

# Check status of all hook registrations
agent-otel-bridge hooks status
```

### 4. Verify Telemetry Health (`doctor`)

Run the 5-point automated station diagnostic:

```bash
agent-otel-bridge doctor
```

```text
=== agent-otel-bridge doctor ===
Diagnosing station telemetry pipeline & OpenTelemetry invariants

[1/5] Checking environment contract variables...
  [ok] OTEL_EXPORTER_OTLP_ENDPOINT = http://127.0.0.1:4318
  [info] OTEL_SERVICE_NAME is unset (using default 'agent-otel-bridge')
  [ok] OTEL_RESOURCE_ATTRIBUTES = deployment.environment=homelab

[2/5] Checking IPC pipeline...
  [ok] Daemon is RUNNING and responding on local IPC!

[3/5] Checking OTLP Collector HTTP endpoint...
  [ok] OTLP Collector reachable at http://127.0.0.1:4318/v1/traces (HTTP status: 415)

[4/5] Checking Observability UI reachability (http://localhost:8080)...
  [ok] Observability UI reachable at http://localhost:8080 (HTTP status: 200 OK)

[5/5] Checking Client Hook Registrations & Local Runtime...
  Local runtime installation: [ok] present
  agent-hook in PATH:         [ok] present
  Google Antigravity:         [ok] registered
  Claude Code:                [ok] registered
  OpenAI Codex:               [ok] registered
```

### 5. Multi-Project Repository Discovery (`scan-all`)

Discover repositories across your developer workstation and align hooks in bulk:

```bash
# Linux / macOS
agent-otel-bridge hooks scan-all --path ~/code

# Windows
agent-otel-bridge hooks scan-all --path C:\code
```

---

## Turnkey SigNoz Dashboards

Production-ready dashboard templates are available in [`contrib/dashboards/signoz/`](contrib/dashboards/signoz/):

| Dashboard | File | Primary Use Case |
|---|---|---|
| **Fleet Operations (FDE)** | [`fleet-operations-fde.json`](contrib/dashboards/signoz/fleet-operations-fde.json) | Executive visibility into human approval latency, tool velocity, token burn rates, self-reverts, and behavioral archetype distributions. |
| **Tool Archetypes & Waste** | [`tool-archetypes-and-waste.json`](contrib/dashboards/signoz/tool-archetypes-and-waste.json) | Analyzes compression ratios, prompt tokens saved by filters (`rtk`, `grep`), and dead-weight MCP schema bloat. |
| **Tokenomics & Cost** | [`ai-agent-observability.json`](contrib/dashboards/signoz/ai-agent-observability.json) | Token consumption by model, prompt vs completion costs, and KV cache efficiency. |

Import directly into SigNoz via the web UI or API:
```bash
curl -X POST http://localhost:8080/api/v2/dashboards \
  -H "SIGNOZ-API-KEY: $SIGNOZ_API_KEY" \
  -H "Content-Type: application/json" \
  -d @contrib/dashboards/signoz/fleet-operations-fde.json
```

---

## Documentation Index

### 🏛️ Architecture & Core Principles
* [Why agent-otel-bridge vs. Native Telemetry](docs/WHY_AGENT_OTEL_BRIDGE.md) — Architectural comparison, hot-path SLAs, and semantic unification.
* [AI Agent Architectural Invariants & SLAs](AGENTS.md) — Enforced performance invariants, binary size SLAs, and agent operating rules.
* [Local Runtime & Station Installation Contract](docs/local-runtime-contract.md) — Isolated versioning, atomic activation, and safe hook updates.
* [System Limitations & Mitigations](docs/LIMITATIONS.md) — Design trade-offs, fail-open guarantees, and operational edge cases.

### 🤖 Harness Integrations & Hook Configurations
* [AI Agent Harness Integration Guide](docs/CLIENTS.md) — Step-by-step hook setup for Antigravity, Claude Code, Codex, Grok, Pi, and custom agents.
* [Agent Observability Skill](skills/agent-otel/SKILL.md) — Built-in pair programming skill for inspecting telemetry pipelines.

### 📊 Telemetry Standards & Dashboards
* [Telemetry Taxonomy & Variable Dictionary](docs/TELEMETRY_DICTIONARY.md) — Complete captured attribute dictionary and GenAI SemConv mappings.
* [SigNoz Dashboard Catalog](contrib/dashboards/signoz/README.md) — Ready-to-import production templates and ClickHouse SQL queries.

### ⚡ Benchmarks & Verification
* [Performance Benchmark Report](docs/BENCHMARKS.md) — Microsecond hardware measurements and profiling methodology.
* [Community Benchmark Leaderboard](docs/COMMUNITY_BENCHMARKS.md) — Hardware leaderboard comparing cross-platform performance.
* [Contributing & Quality Guardrails](CONTRIBUTING.md) — Branching policy, testing invariants, and automated guardrail commands.

---

## Quality Guardrails

Every contribution must pass the automated guardrail suite:

```bash
# Run all linters, conformance tests, doc tests, and binary size checks
cargo guardrails

# Or via unified CLI binary
agent-otel-bridge check-guardrails
```

---

## License

Copyright (c) 2026 Samuel Mota. Licensed under the [Apache License, Version 2.0](LICENSE).
