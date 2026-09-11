# Architecture Design: agent-otel-bridge

`agent-otel-bridge` is a native, high-performance OpenTelemetry instrumentation bridge engineered for autonomous AI CLI agent harnesses (Google Antigravity / `agy`, Claude Code, OpenAI Codex CLI, xAI Grok CLI, Inflection Pi CLI, and custom LLM developer tools).

---

## 1. Problem Statement & Invariants

AI coding agents execute dozens to hundreds of tool calls per session (`read_file`, `grep_search`, `run_command`, `replace_file_content`). Modern agent architectures invoke lifecycle hooks synchronously on hot paths:
- `PreToolUse`
- `PostToolUse`
- `PreInvocation`
- `PostInvocation`
- `Stop`

### Traditional Approaches vs Invariants

Traditional telemetry scripts (Python, Node.js, PowerShell) impose massive latency penalties on Windows:
- **PowerShell cold start**: 400ms – 1,200ms per tool invocation.
- **Python cold start**: 150ms – 280ms per tool invocation.
- **Direct HTTP/TCP POST**: 10ms – 40ms TCP socket setup + HTTP handshake. If the collector hangs, the hook blocks the developer's agent loop.

In a session with 200 tool calls, a script hook wastes **30 to 120 seconds of pure idle latency**, violating workstation invariants:

> **Workstation Invariant**: *"A hook command is a declared dependency: it must resolve, and a hook on a per-tool-call event must be a native binary."* ([`AGENTS.md`](file:///C:/Users/samue/code/environment-contract/AGENTS.md))

---

## 2. Core Architecture

`agent-otel-bridge` decomposes telemetry into an ultra-fast, zero-overhead client and a persistent background daemon communicating over high-speed Win32 Named Pipes:

```mermaid
flowchart TD
    subgraph AgentHarness ["AI CLI Agent Harnesses"]
        AGY["Google Antigravity (~/.gemini)"]
        Claude["Claude Code (~/.claude)"]
        Codex["OpenAI Codex (~/.codex)"]
        Grok["xAI Grok (~/.grok)"]
        Pi["Inflection Pi (~/.pi)"]
        Custom["Custom LLM CLI / Harness"]
    end

    subgraph HotPath ["Ultra-Fast Client (< 1ms execution)"]
        AgentHook["agent-hook.exe (242 KB Native Binary)"]
        Watchdog["Watchdog Thread (3ms deadline)"]
        PipeClient["Win32 Overlapped Client (\\.\\pipe\\agent-otel)"]
    end

    subgraph DaemonProcess ["agent-otel-bridge daemon (Tokio Background Process)"]
        PipeServer["Named Pipe Server (64KB Ring Buffer)"]
        Parser["ProtoJSON Parser & SemConv Mapper"]
        Batcher["Micro-Batcher (Configurable Buffer)"]
        QuotaEngine["Quota & Metrics Engine"]
        OtlpExporter["OTLP HTTP/Protobuf Exporter (reqwest keep-alive)"]
    end

    subgraph OTelCollectorPipeline ["OpenTelemetry Collector Pipeline"]
        Collector["OpenTelemetry Collector (:4318 / :4317)"]
        Processor["Batch / Transform Processor (OTTL)"]
    end

    subgraph ObservabilityBackends ["Pluggable Observability Ecosystem"]
        Jaeger["Jaeger (Distributed Traces)"]
        Tempo["Grafana Tempo / Mimir"]
        Prom["Prometheus (Gauges)"]
        SigNoz["SigNoz (Traces + Metrics)"]
        Cloud["Datadog / Honeycomb / Cloud Trace"]
    end

    AGY -->|"stdin (ProtoJSON)"| AgentHook
    Claude -->|"stdin (JSON)"| AgentHook
    Codex -->|"stdin (JSON)"| AgentHook
    Grok -->|"stdin (JSON)"| AgentHook
    Pi -->|"stdin (JSON)"| AgentHook
    Custom -->|"stdin (JSON)"| AgentHook
    AgentHook -->|"Spawn watchdog"| Watchdog
    AgentHook -->|"Overlapped Write"| PipeClient
    AgentHook -->|"stdout: {} & exit 0"| AgentHarness
    PipeClient -->|"Binary Wire Frame [AG:v1]"| PipeServer

    PipeServer -->|"mpsc channel"| Parser
    Parser --> Batcher
    QuotaEngine -->|"Metrics"| OtlpExporter
    Batcher -->|"Traces"| OtlpExporter
    OtlpExporter -->|"POST /v1/traces (Protobuf)"| Collector
    OtlpExporter -->|"POST /v1/metrics (Protobuf)"| Collector
    Collector --> Processor
    Processor --> Jaeger
    Processor --> Tempo
    Processor --> Prom
    Processor --> SigNoz
    Processor --> Cloud
```

---

## 3. IPC Protocol & Framing

The transport layer uses a zero-copy binary wire framing protocol:

| Offset | Field | Type | Description |
|---|---|---|---|
| `0..2` | `MAGIC` | `[u8; 2]` | Magic bytes `['A', 'G']` (`0x41`, `0x47`) |
| `2` | `VERSION` | `u8` | Protocol version (`0x01`) |
| `3` | `MSG_TYPE` | `u8` | `0x01`: HookPayload, `0x02`: QuotaPing, `0x03`: HealthPing, `0xFF`: Shutdown |
| `4..8` | `LEN` | `u32` (LE) | Little-endian payload length |
| `8..` | `PAYLOAD` | `[u8]` | Event tag (byte 0) + raw JSON payload |

### Fail-Open Guarantees

1. **Win32 Overlapped I/O**: `agent-hook.exe` issues asynchronous writes using Windows `CreateFileW(..., FILE_FLAG_OVERLAPPED, ...)`.
2. **Primary & Fallback Pipe**: Resolves `AGENT_OTEL_PIPE` (default `\\.\pipe\agent-otel`), with automated fallback to `AGY_OTEL_PIPE` (`\\.\pipe\agy-otel`).
3. **2ms Wait Budget**: `GetOverlappedResultEx` waits at most 2ms. If the daemon is busy or offline, `CancelIoEx` is triggered, and the handle is closed.
4. **Hard Watchdog Thread**: A secondary thread sleeps for 3ms. If the main thread hangs on an OS call, the watchdog outputs `{}` to `stdout` and exits with code 0.

---

## 4. OpenTelemetry Semantic Conventions & Architecture Layering

`agent-otel-bridge` complies with canonical CNCF OpenTelemetry GenAI standards (`gen_ai.*`) and universal Agent Lifecycle standards (`agent.*`):

### Architecture Layer Separation Principle
1. **Bridge Layer (Standardized Instrumentation)**:
   The bridge emits **purely canonical attributes**. Vendor-specific prefixes like `agy.*` are disabled by default to avoid semantic pollution when instrumenting other agents (Claude Code, OpenAI Codex, xAI Grok, Inflection Pi) and to avoid cardinality/payload bloat.
2. **Collector Layer (Transformation & Aliasing)**:
   Per OpenTelemetry design, dialect transformations or vendor-specific backward compatibility belong in the **OpenTelemetry Collector** pipeline using the `transformprocessor` (OTTL).
3. **Bridge Compatibility Toggle**:
   For deployments without a custom Collector, setting `AGENT_OTEL_LEGACY_ATTRIBUTES=true` enables dual-emission of `agy.*` aliases.

### Spans & Attributes

| OTel Canonical Attribute | Value / Format | Opt-in Legacy Alias (`AGENT_OTEL_LEGACY_ATTRIBUTES=true`) |
|---|---|---|
| **Span Name (Tool Use)** | `execute_tool {tool_name}` | — |
| **Span Name (Invocation)** | `invoke_agent {agent_name}` | — |
| **Span Name (Stop)** | `agent.stop` | — |
| `gen_ai.operation.name` | `execute_tool`, `invoke_agent` | — |
| `gen_ai.provider.name` | `"anthropic"`, `"google"`, `"openai"`, `"xai"`, `"inflection"` | Inferred from model / client |
| `gen_ai.agent.name` | `"antigravity"`, `"claude-code"`, `"codex"`, `"grok"`, `"pi"` | Inferred from payload / client |
| `gen_ai.conversation.id` | Session UUID | Mapped from `conversationId` or `sessionId` |
| `gen_ai.tool.name` | `run_command`, `Bash`, `grep_search`, etc. | `gen_ai.tool.name` |
| `gen_ai.tool.call.id` | Tool call identifier | `gen_ai.tool.call.id` |
| `gen_ai.request.model` | Model name (e.g. `claude-3-5-sonnet`, `gemini-2.5-pro`, `gpt-4o`, `grok-2`) | `gen_ai.request.model` |
| `agent.hook.event` | `PostToolUse`, `PreToolUse`, `PostInvocation`, `Stop` | `agy.hook.event` |
| `agent.step.index` | Integer turn index | `agy.step.index` |
| `agent.execution.num` | Execution sequence number | `agy.execution.num` |
| `agent.fully_idle` | Boolean quiescence indicator | `agy.fully_idle` |
| `agent.termination_reason` | Completion / stop reason | `agy.termination_reason` |

### Deterministic Trace & Span ID Correlation

- **`trace_id`**: Deterministic SHA-256 hash of `conversationId` (or `sessionId`) truncated to 16 bytes. All hook events within the same agent session automatically group into a single trace in your observability backend (Jaeger, Tempo, SigNoz, Datadog) without cross-process context propagation.
- **`span_id`**: Deterministic SHA-256 hash of `(conversationId, stepIdx, event, toolName, salt)` truncated to 8 bytes.

### Agent Quota Gauges

- `agent.quota.remaining_fraction` (optional legacy alias: `agy.quota.remaining_fraction`): Gauge, unit `"1"`, value `0.0..=1.0`.
- `agent.quota.seconds_to_reset` (optional legacy alias: `agy.quota.seconds_to_reset`): Gauge, unit `"s"`, value `>= 0.0`.
- Standard attributes: `bucket`, `group`.

