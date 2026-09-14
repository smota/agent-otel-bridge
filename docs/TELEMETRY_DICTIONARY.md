# AI Agent Telemetry Taxonomy & Variable Dictionary

This document defines the comprehensive telemetry taxonomy, canonical variable dictionary, harness support matrix, and comparative evaluation against production dashboards (such as the OpenAI Codex dashboard) for `agent-otel-bridge`.

---

## 1. Architectural Taxonomy

Agent observability requires multi-dimensional telemetry spanning reasoning loops, tool interactions, quotas, and identity. We organize all captured telemetry into six foundational taxonomic dimensions:

```
                          ┌────────────────────────────────────────────────────────┐
                          │               AI Agent Telemetry Taxonomy              │
                          └────────────────────────────────────────────────────────┘
                                                       │
         ┌──────────────────┬──────────────────────────┼──────────────────────────┬──────────────────┐
         │                  │                          │                          │                  │
         ▼                  ▼                          ▼                          ▼                  ▼
┌─────────────────┐┌─────────────────┐  ┌────────────────────────┐  ┌──────────────────┐┌──────────────────┐
│   Dimension 1   ││   Dimension 2   │  │      Dimension 3       │  │   Dimension 4    ││   Dimension 5    │
│    Identity &   ││   Agent Loop    │  │      Turn Outcomes     │  │ Tool Invocations ││ Model & Provider │
│     Session     ││   & Lifecycle   │  │   & Safety Governance  │  │  & Performance   ││     Routing      │
└─────────────────┘└─────────────────┘  └────────────────────────┘  └──────────────────┘└──────────────────┘
         │                  │                          │                          │                  │
         └──────────────────┴──────────────────────────┼──────────────────────────┴──────────────────┘
                                                       ▼
                                        ┌──────────────────────────────┐
                                        │         Dimension 6          │
                                        │       Token Economics        │
                                        │      & Quota Management      │
                                        └──────────────────────────────┘
```

### Dimension 1: Session & Operator Identity
Identifies the human operator, execution environment, and interactive session boundaries across CLI invocations.
- Essential for multi-tenant developer fleets, attribution, terminal auditing, and user analytics.

### Dimension 2: Agent Loop & Lifecycle
Tracks the internal iterations of the autonomous agentic loop (`PreInvocation`, `PostInvocation`, `PreToolUse`, `PostToolUse`, `Stop`).
- Essential for detecting **runaway loops**, infinite tool-invocation cycles, recursion explosion, and turn-depth profiling.

### Dimension 3: Turn Outcomes & Safety Governance
Captures how and why an agent turn concluded, user approval/denial decisions, and operational health.
- Distinguishes healthy completion (`NO_TOOL_CALL`, `stop`) from user cancellation (`USER_INTERRUPT`), quota exhaustion, or agent errors.

### Dimension 4: Tool Invocations & Performance
Instruments every action the agent executes on the host system (`run_command`, `Bash`, `view_file`, `grep_search`, `write_to_file`, etc.).
- Measures granular P50/P95/P99 execution latency, tool argument payloads, and error status codes.

### Dimension 5: Model & AI Provider Routing
Attributes activity to specific underlying Foundation Models (`gemini-2.5-pro`, `claude-3-7-sonnet`, `gpt-4o`, `grok-2`, etc.) and vendor backends.
- Monitors multi-model routing, model migration, fallbacks, and provider performance.

### Dimension 6: Token Economics & Quota Management
Monitors input/output tokens, prompt caching efficiency, and provider quota window burn-down rates.
- Prevents billing surprises, tracks weekly quota depletion, and alerts on imminent rate limiting.

---

## 2. Canonical Telemetry Dictionary & Harness Support Matrix

The table below details all attributes captured, enriched, and emitted by `agent-otel-bridge`, mapped across the 5 primary AI CLI harnesses:

| Taxonomy Dimension | Canonical OTel Attribute / Metric | OTel Signal | Type | Antigravity | Claude Code | OpenAI Codex | xAI Grok | Pi (pi.dev) | Source & Description |
|---|---|---|---|:---:|:---:|:---:|:---:|:---:|---|
| **Identity & Session** | `gen_ai.conversation.id` | Span Attr | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Conversation / session identifier. Deterministic trace correlation ID. |
| **Identity & Session** | `user.email` | Span Attr | `string` | *Enriched* | *Enriched* | **Yes** | *Enriched* | *Enriched* | Operator email. Captured from payload or enriched from `$USER_EMAIL` / `$GIT_AUTHOR_EMAIL`. |
| **Identity & Session** | `terminal.type` | Span Attr | `string` | *Enriched* | *Enriched* | **Yes** | *Enriched* | *Enriched* | Terminal emulator (e.g. `windows-terminal`, `xterm-256color`, `vscode`). Enriched from env. |
| **Identity & Session** | `service.name` | Resource | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Standard OTel service name (`agent-otel-bridge` or custom via `OTEL_SERVICE_NAME`). |
| **Identity & Session** | `host.name` | Resource | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Hostname of the developer workstation. Enriched automatically from OS. |
| **Identity & Session** | `os.type` | Resource | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Operating system type (`windows`, `linux`, `macos`). Enriched automatically. |
| **Agent Lifecycle** | `gen_ai.agent.name` | Span Attr | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Canonical agent identifier (`antigravity`, `claude-code`, `codex`, `grok`, `pi`). |
| **Agent Lifecycle** | `agent.hook.event` | Span Attr | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Universal lifecycle event (`PreToolUse`, `PostToolUse`, `PostInvocation`, `Stop`). |
| **Agent Lifecycle** | `agent.step.index` | Span Attr | `int64` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | 0-indexed turn step within the current session. |
| **Agent Lifecycle** | `agent.execution.num` | Span Attr | `int64` | **Yes** | *Optional* | *Optional* | *Optional* | *Optional* | Monotonic execution counter within an agent loop pass. |
| **Agent Lifecycle** | `agent.fully_idle` | Span Attr | `bool` | **Yes** | *Optional* | *Optional* | *Optional* | *Optional* | Flag indicating agent reached complete quiescence without pending background tasks. |
| **Turn Outcomes** | `agent.termination_reason` | Span Attr | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Completion reason: `NO_TOOL_CALL`, `model_stop`, `USER_INTERRUPT`, `TURN_LIMIT`. |
| **Turn Outcomes** | `gen_ai.response.finish_reasons` | Span Attr | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | OTel GenAI standard finish reasons (`stop`, `length`, `tool_calls`, `error`). |
| **Turn Outcomes** | `agent.decision` | Span Attr | `string` | **Yes** | **Yes** | **Yes** | *Optional* | *Optional* | Policy/Human-in-the-loop decision: `allow`, `deny`, `ask`. |
| **Turn Outcomes** | `agent.success` | Span Attr | `bool` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Boolean turn success derived from span status and exit codes. |
| **Tool Invocations** | `gen_ai.operation.name` | Span Attr | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | `execute_tool`, `invoke_agent`, or `chat`. |
| **Tool Invocations** | `gen_ai.tool.name` | Span Attr | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Specific tool name (`run_command`, `Bash`, `view_file`, `grep_search`, etc.). |
| **Tool Invocations** | `gen_ai.tool.call.id` | Span Attr | `string` | **Yes** | **Yes** | **Yes** | **Yes** | *Optional* | Unique call ID associated with the specific tool execution. |
| **Tool Invocations** | `durationNano` / `duration_ms` | Span Field | `int64` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Precise execution duration of the tool or invocation span. |
| **Model & Provider** | `gen_ai.provider.name` | Span Attr | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Provider inferred from model: `google`, `anthropic`, `openai`, `xai`, `pi`. |
| **Model & Provider** | `gen_ai.request.model` | Span Attr | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Active model identifier (`gemini-2.5-pro`, `claude-3-7-sonnet`, `o3-mini`, etc.). |
| **Economics & Quota** | `gen_ai.usage.input_tokens` | Span Attr | `int64` | *Wrapper* | **Yes** | **Yes** | **Yes** | *Optional* | Input / prompt token consumption count per turn. |
| **Economics & Quota** | `gen_ai.usage.output_tokens` | Span Attr | `int64` | *Wrapper* | **Yes** | **Yes** | **Yes** | *Optional* | Output / generation token consumption count per turn. |
| **Economics & Quota** | `gen_ai.usage.cache_read_tokens` | Span Attr | `int64` | *Wrapper* | **Yes** | **Yes** | *Optional* | *Optional* | Cached prompt tokens read from context window cache. |
| **Economics & Quota** | `agent.quota.remaining_fraction` | Metric Gauge | `double` | **Yes** | *Optional* | *Optional* | *Optional* | *Optional* | Provider quota balance fraction (`0.0..=1.0`), by `bucket` and `group`. |
| **Economics & Quota** | `agent.quota.seconds_to_reset` | Metric Gauge | `double` | **Yes** | *Optional* | *Optional* | *Optional* | *Optional* | Countdown time (in seconds) until the quota reset window. |

> **Key to Matrix**:
> - **Yes**: Provided directly by the harness hook protocol and mapped into the span/metric.
> - *Enriched*: Automatically detected and populated by `agent-otel-bridge` from workstation environment if omitted by harness.
> - *Wrapper*: Antigravity does not emit per-turn token counts in its hook payload (it meters quota via rolling weekly buckets). Token counts can be supplied by an optional API wrapper or transcript watcher.
> - *Optional*: Emitted if the harness version supports that field.

---

## 3. Comparative Evaluation: OpenAI Codex Dashboard vs `agent-otel-bridge`

The OpenAI Codex dashboard (`codex-dashboard.json`) in SigNoz is recognized as one of the most comprehensive agent monitoring dashboards. Below is an item-by-item audit verifying how `agent-otel-bridge` covers every single variable and panel:

| # | Codex Dashboard Panel | Codex Underlying Attribute | `agent-otel-bridge` Canonical Attribute | Coverage Status | Architectural Notes |
|---|---|---|---|:---:|---|
| 1 | **Conversations** | `count_distinct(conversation.id)` | `count_distinct(gen_ai.conversation.id)` | **Full Coverage** | OpenTelemetry GenAI standard convention. |
| 2 | **Conversations Over Time** | `conversation.id` over time | `gen_ai.conversation.id` over time | **Full Coverage** | Time series aggregation over `agent.stop` or `agent.hook.event`. |
| 3 | **Conversation Distribution By User** | `user.email` | `user.email` | **Full Coverage** | Emitted from payload or enriched via `$USER_EMAIL` / Git config. |
| 4 | **Conversations by Terminal Type** | `terminal.type` | `terminal.type` | **Full Coverage** | Emitted from payload or enriched via `$WT_SESSION` / `$TERM_PROGRAM` / `$TERM`. |
| 5 | **Conversation Details** | `conversation.id`, `sum(duration_ms)`, `count_distinct(call_id)` | `gen_ai.conversation.id`, `sum(durationNano)`, `count_distinct(gen_ai.tool.call.id)` | **Full Coverage** | Trace-based duration calculation is higher precision than log-based duration. |
| 6 | **Tool Call Distribution** | `tool_name` | `gen_ai.tool.name` | **Full Coverage** | Captured for all tools across all 5 harnesses. |
| 7 | **Model Calls** | `count_distinct(call_id)` | `count_distinct(gen_ai.tool.call.id)` or span count | **Full Coverage** | Corresponds to `PostInvocation` or tool execution spans. |
| 8 | **Model Call Distribution** | `model` | `gen_ai.request.model` | **Full Coverage** | Grouped by canonical model identifier. |
| 9 | **Input Tokens** | `sum(input_token_count)` | `sum(gen_ai.usage.input_tokens)` | **Full Coverage** | Fully supported in schema; provided directly by Codex and Claude Code. |
| 10 | **Output Tokens** | `sum(output_token_count)` | `sum(gen_ai.usage.output_tokens)` | **Full Coverage** | Fully supported in schema; provided directly by Codex and Claude Code. |
| 11 | **Cached Tokens** | `sum(cached_token_count)` | `sum(gen_ai.usage.cache_read_tokens)` | **Full Coverage** | Fully supported in schema; provided directly by Codex and Claude Code. |
| 12 | **Cache Utilization Rate** | `(cached / input) * 100` | `(cache_read_tokens / input_tokens) * 100` | **Full Coverage** | Supported via composite query formula `(B / A) * 100`. |
| 13 | **Token Usage Over Time** | `input + output` | `input_tokens + output_tokens` | **Full Coverage** | Composite time series formula `A + B`. |
| 14 | **Token Distribution By Model** | `input + output` by `model` | `input_tokens + output_tokens` by `gen_ai.request.model` | **Full Coverage** | Composite query grouped by model. |
| 15 | **Request Duration (P95)** | `P95(duration_ms)` | `quantile(0.95)(durationNano)` | **Full Coverage** | Native OTel span duration calculation. |
| 16 | **Success Rate** | `count()` by `success` | `count()` by `agent.success` or `status.code` | **Full Coverage** | Captured directly in span status (`StatusCode::Ok` vs `StatusCode::Error`). |
| 17 | **User Decision** | `count()` by `decision` | `count()` by `agent.decision` | **Full Coverage** | Captured from `PreToolUse` hooks (`allow`, `deny`, `ask`). |

### Key Takeaways from Evaluation
1. **100% Variable Coverage**: Every metric, dimension, and formula in the Codex dashboard is fully represented in `agent-otel-bridge` using canonical OpenTelemetry standards.
2. **Multi-Agent Superset**: The `agent-otel-bridge` specification goes beyond the Codex dashboard by also tracking:
   - **Provider Quota Balances** (`agent.quota.remaining_fraction` & `agent.quota.seconds_to_reset`), which Codex does not track.
   - **Runaway Loop Detection** (`agent.step.index` and `PostInvocation` frequency per session).
   - **Quiescence / Termination Reason breakdown** (`agent.termination_reason`).
   - **Multi-Provider attribution** (`gen_ai.provider.name`).

---

## 4. Understanding Termination Reasons: Why `NO_TOOL_CALL` Appears

A common question from operators monitoring Antigravity (AGY) sessions:
> *"Why do I see only `NO_TOOL_CALL` as the termination reason in my dashboard? Is this an error or missing data?"*

### The Lifecycle Explanation
In the Antigravity agent architecture:
1. **Interactive Prompt**: The user enters a request.
2. **Loop Iteration**: The LLM analyzes the context. If it decides to execute an action (e.g. read a file or run a command), it emits a tool call. The lifecycle transitions through `PreToolUse` -> tool execution -> `PostToolUse` -> `PostInvocation`.
3. **Quiescence**: When the model finishes all requested actions, it composes the final conversational markdown response for the user and **does not emit any tool call**.
4. **Natural Stop**: The Antigravity agent harness evaluates the turn state: because no further tool was requested, the turn loop concludes with the internal state: **`NO_TOOL_CALL`**.

### What This Means for Operators
- **`NO_TOOL_CALL` is the normal, healthy success signal** for Antigravity. It indicates that the agent successfully completed its planned actions and delivered a response to the user.
- **Other termination reasons** occur during abnormal or explicit state transitions:
  - `USER_INTERRUPT`: The developer canceled execution (Ctrl+C).
  - `TURN_LIMIT` / `MAX_STEPS`: The loop exceeded the configured safety recursion depth.
  - `ERROR` / `API_ERROR`: An unrecoverable upstream provider error occurred.
- In other harnesses:
  - Claude Code emits `model_stop` or `end_turn`.
  - OpenAI Codex emits `stop` or `success`.
- Under canonical OpenTelemetry GenAI semantic conventions, `NO_TOOL_CALL` maps directly to `gen_ai.response.finish_reasons = ["stop"]`.

---

## 5. Universal Agent Intelligence & v0.3 Semantic Extensions

`agent-otel-bridge` v0.3 introduces universal abstractions for execution context, preference-agnostic tool archetypes, MCP and skill waste quantification, and cross-agent distributed tracing lineage:

### 5.1 Execution Context & Non-Blocking VCS Attributes
Extracted synchronously in $< 150\mu\text{s}$ via zero-subprocess direct file inspection:

| Attribute | Type | Description | Example Values |
| :--- | :--- | :--- | :--- |
| `workspace.path` | `string` | Canonical absolute path of the active working directory | `C:\Users\samue\code\agent-otel-bridge` |
| `workspace.project_name` | `string` | Project name inferred from manifest or directory basename | `agent-otel-bridge` |
| `workspace.project_root` | `string` | Inferred top-level root directory containing project marker | `C:\Users\samue\code\agent-otel-bridge` |
| `workspace.project_type` | `string` | Detected ecosystem type | `rust`, `node`, `python`, `go`, `antigravity_workspace` |
| `vcs.system` | `string` | Detected Version Control System | `git`, `none` |
| `vcs.repository.name` | `string` | Extracted repository identifier | `agent-otel-bridge` |
| `vcs.branch.name` | `string` | Active branch name extracted directly from `.git/HEAD` | `main`, `feat/tokenomics` |
| `vcs.commit.sha` | `string` | Active commit SHA hash | `6b755a6...` |
| `vcs.worktree.active` | `bool` | True if operating inside a secondary Git worktree | `true`, `false` |

### 5.2 Behavioral Tool Execution Archetypes
Decouples observability from specific CLI tools (`rtk`, `jq`, `bat`, `delta`) by modeling functional intent:

| Attribute | Type | Description | Example Values |
| :--- | :--- | :--- | :--- |
| `agent.tool.archetype` | `string` | Functional archetype classification | `filter_compressor`, `structured_parser`, `inspector_diff`, `search_retrieval`, `build_test_verify`, `state_mutation`, `env_pkg_manager`, `vcs_lifecycle`, `network_transfer`, `generic_exec` |
| `agent.tool.binary` | `string` | Base executable name | `rtk`, `jq`, `rg`, `bat`, `cargo` |
| `agent.tool.wrapped_binary` | `string` | Wrapped binary if using an execution proxy | `cargo` (from `rtk cargo test`) |
| `agent.tool.pipeline_depth` | `int` | Number of piped command segments | `1`, `3` (from `cat f.json \| jq .items \| head -n 5`) |
| `agent.tool.compression_ratio` | `double` | Output token reduction ratio: $1 - (\text{bytes\_out} / \text{bytes\_in})$ | `0.85` (85% reduction) |
| `agent.tool.tokens_saved` | `int` | Estimated prompt tokens saved by compression | `3450` |

### 5.3 Universal Capabilities (MCP & Skills) & Waste Metrics
Quantifies context tax, payload overhead, and retry thrashing:

| Attribute | Type | Description | Example Values |
| :--- | :--- | :--- | :--- |
| `capability.kind` | `string` | Capability tier | `mcp`, `skill`, `subagent`, `native` |
| `capability.namespace` | `string` | Server or skill bundle identifier | `github`, `postgres`, `agy-customizations`, `core` |
| `capability.name` | `string` | Specific operation invoked | `create_issue`, `explain_rule`, `view_file` |
| `capability.schema_tokens` | `int` | Context tokens consumed by tool definitions in prompt | `4200` |
| `capability.response_bytes` | `int` | Raw byte size of tool response payload | `154000` |
| `capability.response_tokens` | `int` | Estimated tokens returned by tool execution | `38500` |
| `capability.consecutive_retries` | `int` | Failure retry streak on identical/similar arguments | `0`, `3` |
| `capability.is_waste` | `bool` | True if invocation is classified as unproductive loop waste | `true`, `false` |

### 5.4 Cross-Agent Distributed Tracing & Lineage
Links heterogeneous subagent sessions into a single distributed trace tree. A hook captures the source process's `TRACEPARENT` into the versioned IPC context envelope (`HookPayloadWithContext`, message type `0x04`); the legacy `HookPayload` (`0x01`) body remains unchanged. The transport carries only `TRACEPARENT`, with a local 512 UTF-8 byte limit. Missing, invalid, or oversized values are omitted without truncation. The daemon resolves context per event and does not use its own environment as a daemon-wide fallback.

| Attribute | Type | Description | Example Values |
| :--- | :--- | :--- | :--- |
| `gen_ai.agent.depth` | `int` | Explicitly recorded recursion hop depth ($0 = \text{root orchestrator}$); never inferred solely from TRACEPARENT presence | `0`, `1`, `2` |
| `gen_ai.agent.parent_name` | `string` | Name of the calling harness | `antigravity`, `claude-code` |
| `gen_ai.agent.root_id` | `string` | Root conversation ID binding the fleet turn | `conv-root-12345` |
| `gen_ai.agent.is_root` | `bool` | True if root orchestrator span | `true`, `false` |

### 5.5 Multi-Layer Error Categorization
Replaces ambiguous binary errors with actionable diagnostic categories:

| Attribute | Category Value | Operational Meaning | SRE Triage Action |
| :--- | :--- | :--- | :--- |
| `agent.error.category` | `tool_verification_failed` | Normal unit test or linter failure | Low; normal part of TDD feedback cycle. |
| `agent.error.category` | `provider_quota_exhausted` | Upstream rate limit or quota exceeded | High; triggers provider failover or wait window. |
| `agent.error.category` | `schema_validation_error` | Model emitted invalid JSON arguments | Medium; tune tool schema or system instructions. |
| `agent.error.category` | `user_interrupted` | User cancelled operation (Ctrl+C / Stop) | None; intentional human flow control. |
