# Architectural Guide: Why agent-otel-bridge vs. Native Harness Telemetry

> **Audience**: Enterprise Architects, Principal Engineers, Platform Teams, and Observability Leads evaluating observability strategies for AI CLI agents.  
> **Status**: Enforced Architectural Standard  

---

## 1. Executive Summary: The Observability Gap in Modern AI Agents

As AI coding agents transition from experimental toys to critical engineering tools, teams encounter a common dilemma:

> *"Each AI harness (Claude Code, Antigravity, OpenAI Codex, Grok, Pi) already has some native hooks or local logs. Why do we need `agent-otel-bridge` instead of relying on each platform's built-in telemetry or custom ad-hoc scripts?"*

The short answer: **Native harness telemetry is siloed, raw, unstandardized, and blind to multi-agent distributed execution.**

Native client telemetry gives you fragmented data dumps (e.g. raw JSON strings in `.claude/projects/` or isolated Antigravity hook events) without semantic standardization, without token economics intelligence, without cross-agent distributed tracing, and with severe latency penalties if custom scripts are used.

`agent-otel-bridge` was engineered as a **zero-overhead, high-performance bridge** that transforms disparate agent hooks into standardized, actionable, enterprise-grade OpenTelemetry GenAI observability.

```mermaid
graph LR
    subgraph Siloed["Native Harnesses (Fragmented & Raw)"]
        A1["Antigravity (.gemini JSON)"]
        A2["Claude Code (.jsonl session logs)"]
        A3["Codex CLI (raw tool strings)"]
        A4["Custom Agents (extension required)"]
    end

    subgraph Bridge["agent-otel-bridge (Intelligence Layer)"]
        direction TB
        P1["Sub-ms Client (agent-hook < 1ms)"]
        P2["Behavioral Archetype Classifier"]
        P3["Capability & MCP Waste Engine"]
        P4["W3C Traceparent Propagation"]
        P5["Adaptive Quota & Fleet Bottleneck"]
    end

    subgraph Backend["Observability Platforms (Single Pane of Glass)"]
        O1["SigNoz"]
        O2["Datadog"]
        O3["Prometheus / Grafana"]
        O4["OpenTelemetry Collector"]
    end

    Siloed --> Bridge
    Bridge --> Backend
```

---

Built-in identities are `antigravity`, `claude-code`, `codex`, `grok`, and `pi` ([registry](../crates/agent-otel-core/src/platform.rs#L155-L161), [mapping](../crates/agent-otel-core/src/model.rs#L184-L205)); Hermes is an [extension example](CLIENTS.md#6-adding-a-new-ai-agent-harness-step-by-step-blueprint-hermes). These mappings do not certify every native harness version or desktop/IDE session.

**Current defaults are not zero-content or privacy-safe**: workspace paths are absolute, an explicit `USER_EMAIL` is eligible with enrichment off (payload email takes precedence), error status messages are raw, and branch and repository names are plaintext. See the [F5 source evidence](TELEMETRY_DICTIONARY.md#511-current-path-email-and-error-export-behaviour). [Work package A1](BACKLOG.md#aob-work-packages) addresses privacy projection; it is planned work.

## 2. Comparative Matrix: Native Telemetry vs. `agent-otel-bridge`

| Capability Dimension | Native Harness Telemetry (Ad-Hoc) | `agent-otel-bridge` | Architectural Impact |
|:---|:---|:---|:---|
| **Multi-Agent Unification** | ❌ **Siloed**: Each tool emits incompatible formats to different locations. | ✅ **Unified**: Single OTLP collector endpoint for configured built-in harnesses; custom harnesses require integration ([registry](../crates/agent-otel-core/src/platform.rs#L155-L161)). | Eliminates tool-specific dashboard sprawl; unifies developer observability into a single pane of glass. |
| **Semantic Standards** | ❌ **Proprietary**: Vendor-specific schemas, inconsistent attribute naming. | ✅ **Bridge span names**: Emits `execute_tool` and `invoke_agent` spans and its own capability, archetype, and quota attributes. Not a certified GenAI v1.28 conformance claim. | OTLP export still targets SigNoz, Datadog, and Honeycomb. Attribute names are the bridge's, not a certified GenAI profile. |
| **Tool Intelligence & Archetypes** | ❌ **Opaque**: Tool execution is labeled as generic `bash` or `exec` with a raw string. | ✅ **Behavioral Archetypes**: Classified as `FilterCompressor`, `StructuredParser`, `InspectorDiff`, `BuildTestVerify`. | Identifies what the agent is *actually doing* (testing vs. filtering vs. diffing) without human inspection. |
| **Tokenomics & Prompt Waste** | ❌ **Absent**: Only logs total input/output tokens per session. | ✅ **Token Waste Metrics**: Emits `agent.tool.tokens_saved`, `agent.tool.compression_ratio`, and schema bloat waste. | Reveals whether tools like `rtk` or `jq` are saving context tokens or if verbose commands are bloating LLM costs. |
| **Capability Decomposition** | ❌ **Flat**: MCP tools, skills, and subagents are treated as identical flat tool names. | ✅ **Decomposed**: Hierarchical extraction into `capability.kind` (`mcp`, `skill`, `subagent`, `native`) and namespace. | Enables precise monitoring of MCP server reliability, subagent recursion depth, and skill failure rates. |
| **Cross-Agent Lineage** | ❌ **Zero**: When Agent A invokes CLI Agent B, distributed trace context is lost. | ✅ **Inbound W3C context**: Honors `TRACEPARENT` when the harness passes it. A conversation id hashes to a fallback trace id. Parent/child span linking across process spawn is not implemented. | A shared trace id arrives only if the harness already set it. The bridge does not attach child processes under the current span. |
| **Quota & Headroom Visibility** | ❌ **Reactive**: Developer only finds out about rate limits after receiving HTTP 429 errors. | ✅ **Proactive Fleet Intelligence**: Real-time remaining fractions, reset countdowns, and fleet bottleneck gauge. | Prevents pipeline halts by forecasting quota exhaustion across OpenAI, Anthropic, Google, and xAI. |
| **Execution Hot-Path SLA** | ⚠️ **150ms – 1,200ms**: Custom Python/Node.js hook scripts cause noticeable agent hesitation. | ⚡ **< 1.0 ms**: Static native PE32 binary (`agent-hook.exe`, 145 KB) with 3ms hardware watchdog. | Zero developer perceptible latency. Complete fail-open safety guarantee. |

---

## 3. The 5 Architectural Pillars of `agent-otel-bridge`

### Pillar 1: From Raw Strings to Semantic Intelligence
When an AI agent executes a command, native harnesses record only the literal command line:
```json
// Native Claude Code or Antigravity raw payload
{ "tool": "bash", "command": "git diff | grep -E '^[+-]' | wc -l" }
```
In traditional dashboards, this appears as an unindexable string. `agent-otel-bridge` classifies this in **< 10 µs** using its compiled archetype engine into:
- `agent.tool.archetype`: `"InspectorDiff"`
- `agent.tool.binary`: `"git"`
- `agent.tool.wrapped_binary`: `"grep"`
- `agent.tool.pipeline_depth`: `3`
- `agent.tool.tokens_saved`: `1450`

Architects can instantly query: *"What percentage of our agents' tool executions are verifying builds vs. searching code?"*

### Pillar 2: Cross-Agent Distributed Tracing & Lineage
In modern workflows, agents invoke other agents:
1. Google Antigravity starts a feature task.
2. It invokes a subagent to search documentation.
3. The subagent executes a CLI command that triggers Claude Code or Codex CLI.

Without `agent-otel-bridge`, this produces **three disconnected traces** with no causal relationship.  
With `agent-otel-bridge`, inbound context is recorded; spawn-time linking is not:
- Inbound W3C `TRACEPARENT` is honored when the harness passes it.
- A conversation ID uses an unkeyed SHA-256 fallback; valid inbound context wins, so hashing a session ID cannot reliably join it to an exported trace ([F6 and resolver evidence](BACKLOG.md#verified-aob-facts)).
- Parent/child span linking across process spawn is not implemented ([context resolver](../crates/agent-otel-core/src/trace_id.rs#L144-L166)).
- `gen_ai.agent.depth`, `gen_ai.agent.parent_name`, and `gen_ai.agent.root_id` are project extensions on the span. They do not produce a unified collector flamegraph.

### Pillar 3: Machine-Adaptive Quota Intelligence
Traditional monitoring either tracks nothing or emits hardcoded synthetic numbers.  
`agent-otel-bridge` introduces **Machine-Adaptive Quota Monitoring**:
- The daemon detects which AI harnesses actually exist on the developer's machine (`is_installed`).
- If you only use Claude Code and Antigravity, **only Claude and Gemini metrics are emitted**. You will never see phantom gauges for absent tools.
- Emits `agent.fleet.bottleneck_ratio`, giving team leads an immediate gauge of the most constrained provider in their developer fleet.

### Pillar 4: Universal Capability Decomposition & Waste Detection
With the explosion of Model Context Protocol (MCP) servers and reusable agent skills, prompt schemas are growing exponentially. `agent-otel-bridge` decomposes tool calls:
- Identifies whether a tool is `mcp`, `skill`, `subagent`, or `native`.
- Measures schema token consumption (`capability.schema_tokens`).
- Detects retry waste (`capability.consecutive_retries`, `capability.is_waste`), flagging when an agent is stuck in an expensive failure loop calling faulty MCP servers.

### Pillar 5: Sub-Millisecond, Fail-Open Engine
Observability must never degrade the user experience or crash an agent session.
- **Microsecond Cold-Start**: `agent-hook.exe` is a tiny 145 KB static PE32 binary. It performs zero disk reads and zero external network calls on the hot path.
- **Overlapped Named Pipes**: IPC writes directly to the Win32 Named Pipe in $< 150\ \mu\text{s}$.
- **Hardware Watchdog**: An OS background thread enforces a strict **3.0 ms** deadline. If the daemon is busy or the pipe stalls, the client exits immediately with code `0` and `{}`.
- **Fail-Open**: An agent turn will **never** fail or freeze due to telemetry.

---

## 4. Architectural Decision Framework: When to Adopt

```mermaid
flowchart TD
    Q1{"Are you running AI CLI agents in production or team workflows?"}
    Q1 -->|No, personal toy scripts| Native["Native harness logs are sufficient"]
    Q1 -->|Yes| Q2{"Do developers use multiple agent harnesses<br/>(Antigravity, Claude Code, Codex, Grok, Pi)?"}
    
    Q2 -->|Yes| Adopt["Adopt agent-otel-bridge:<br/>Routes configured built-in harnesses into one OTLP pipeline"]
    Q2 -->|No, single agent| Q3{"Do you need enterprise observability<br/>(SigNoz, Datadog, Grafana) with OTLP export?"}
    
    Q3 -->|Yes| Adopt
    Q3 -->|No| Q4{"Do you need tokenomics, archetype intelligence,<br/>or rate-limit quota visibility?"}
    
    Q4 -->|Yes| Adopt
    Q4 -->|No| Native
```

---

## 5. Summary

| Architectural Need | With Native Telemetry | With `agent-otel-bridge` |
|:---|:---|:---|
| **Fleet Standardization** | High custom maintenance | `execute_tool` / `invoke_agent` spans and bridge-owned attributes (not certified GenAI v1.28) |
| **Developer Turn Latency** | Degraded by slow custom scripts | Sub-millisecond (< 1.0 ms), zero impact |
| **Distributed Lineage** | Broken at process boundaries | Inbound `TRACEPARENT` when the harness passes it; conversation-id fallback trace id. Spawn-time parent/child links are not implemented. |
| **Token Optimization** | Invisible | Precise tokenomics & compression metrics |
| **Rate Limit Prevention** | Reactive 429 errors | Proactive multi-provider headroom gauges |
