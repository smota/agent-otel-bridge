# Cross-Agent Telemetry Inventory & Dashboard Analysis

This document provides an exhaustive inventory of the variables, attributes, and metrics across the 7 reference AI Agent and LLM dashboards in SigNoz, mapped against the capabilities of `agent-otel-bridge`.

---

## 1. Reference Dashboards Catalog

| # | SigNoz Dashboard ID | Dashboard Title | Target Ecosystem | Panels | Dashboard Variables | Primary Signals |
|---|---|---|---|:---:|---|:---:|
| 1 | `01a08f6b-7e04-74ff-9c24-28d774057899` | **Grok Build (Grok CLI)** | xAI Grok CLI Harness | 13 | *(None)* | Metrics |
| 2 | `01a08f6b-a8d3-79b3-8e7f-c7994ee5773b` | **Grok** | xAI LLM API / Platform | 13 | `$service_name`, `$language`, `$llm_model` | Traces & Logs |
| 3 | `01a08f6b-d576-7bb2-bc1c-40859a18ff08` | **Claude Code Metrics** | Anthropic Claude Code CLI | 21 | *(None)* | Metrics & Logs |
| 4 | `01a08f6b-558b-7cc2-81f7-f9777c731f04` | **Codex** | OpenAI Codex CLI | 17 | `$service_name` | Traces & Logs |
| 5 | `01a08f6b-2c5c-715e-b891-ff98f19b4eee` | **OpenAI** | OpenAI API / Python SDK | 11 | `$service_name`, `$language`, `$llm_model` | Traces & Logs |
| 6 | `01a08f6b-0327-718d-98ad-6dce5b17fb3b` | **Claude Agent SDK** | Anthropic Agent SDK | 15 | `$service_name`, `$llm_model` | Traces & Metrics |
| 7 | `01a08f6b-fbeb-7439-a6a9-0809f9da72a0` | **Antigravity CLI** | Google Antigravity (AGY) | 11 | `$service_name` | Traces & Metrics |

---

## 2. Cross-Agent Common Telemetry Matrix

This table synthesizes all telemetry concepts across the 7 dashboards, groups equivalent vendor-specific fields, identifies the canonical OpenTelemetry standard, and details whether `agent-otel-bridge` captures it.

| Functional Category | Telemetry Concept | Grok CLI (`grok_code`) | Claude Code (`claude_code`) | OpenAI Codex | Antigravity (`agy`) | OpenAI / Grok SDK API | Canonical OTel Convention | `agent-otel-bridge` Status |
|---|---|---|---|---|---|---|---|:---:|
| **Identity & Session** | Interactive Session ID | `session.count` | `session.count` | `conversation.id` | `gen_ai.conversation.id` | *(None / Stateless)* | `gen_ai.conversation.id` | **Covered** (Direct) |
| **Identity & Session** | Operator Attribution | *(Not shown)* | *(Not shown)* | `user.email` | *(Enriched)* | *(Not shown)* | `user.email` | **Covered** (Enriched) |
| **Identity & Session** | Terminal Environment | *(Not shown)* | *(Not shown)* | `terminal.type` | *(Enriched)* | *(Not shown)* | `terminal.type` | **Covered** (Enriched) |
| **Identity & Session** | Workstation Hostname | *(Not shown)* | *(Not shown)* | `host.name` | `host.name` | `host.name` | `host.name` | **Covered** (Enriched) |
| **Identity & Session** | Service Name | *(Not shown)* | `service.name` | `service.name` | `service.name` | `service.name` | `service.name` | **Covered** (Direct) |
| **Reasoning Loop** | Turns / Iterations | `turn.count` | *(Inferred)* | `call_id` | `agent.step.index` / `PostInvocation` | *(Single Request)* | `agent.hook.event = 'PostInvocation'` | **Covered** (Direct) |
| **Reasoning Loop** | Step Counter | *(Not shown)* | *(Not shown)* | *(Not shown)* | `agent.step.index` | *(None)* | `agent.step.index` | **Covered** (Direct) |
| **Reasoning Loop** | Quiescence / Idle State | *(Not shown)* | *(Not shown)* | *(Not shown)* | `agent.fully_idle` | *(None)* | `agent.fully_idle` | **Covered** (Direct) |
| **Tools & Actions** | Tool Call Count | `tool.usage` | `code_edit_tool.decision` | `count(call_id)` | `count(gen_ai.tool.name)` | *(None)* | `gen_ai.operation.name = 'execute_tool'` | **Covered** (Direct) |
| **Tools & Actions** | Tool Name | `tool_name` | `tool_name` | `tool_name` | `gen_ai.tool.name` | *(None)* | `gen_ai.tool.name` | **Covered** (Direct) |
| **Tools & Actions** | Tool Execution Latency | *(Histograms)* | *(Inferred)* | `duration_ms` | `durationNano` | `duration_nano` | `durationNano` (Span) | **Covered** (Direct) |
| **Turn Outcomes** | Quiescence Reason | `outcome` | *(Exit code)* | `success` | `agent.termination_reason` | *(HTTP Status)* | `agent.termination_reason` | **Covered** (Direct) |
| **Turn Outcomes** | OTel Finish Reasons | *(Not shown)* | *(Not shown)* | `stop` | `NO_TOOL_CALL` -> `stop` | `finish_reasons` | `gen_ai.response.finish_reasons` | **Covered** (Direct) |
| **Turn Outcomes** | Policy / Human Approval | *(Not shown)* | `code_edit_tool.decision` | `decision` (`allow`/`deny`/`ask`) | `decision` | *(None)* | `agent.decision` | **Covered** (Direct) |
| **Turn Outcomes** | Turn Success / Failure | `outcome` | `success` | `success` | `status_code` | `has_error` | `agent.success` & `StatusCode` | **Covered** (Direct) |
| **Model & Routing** | Foundation Model Name | `model` | `model` | `model` | `gen_ai.request.model` | `gen_ai.request.model` | `gen_ai.request.model` | **Covered** (Direct) |
| **Model & Routing** | AI Provider Name | `xai` | `anthropic` | `openai` | `google` | `openai` / `xai` | `gen_ai.provider.name` | **Covered** (Direct) |
| **Token Economics** | Input Tokens | `token.usage (type=input)` | `token.usage (type=input)` | `input_token_count` | *(Meters via quota)* | `gen_ai.usage.input_tokens` | `gen_ai.usage.input_tokens` | **Covered** (Direct/Payload) |
| **Token Economics** | Output Tokens | `token.usage (type=output)` | `token.usage (type=output)` | `output_token_count` | *(Meters via quota)* | `gen_ai.usage.output_tokens` | `gen_ai.usage.output_tokens` | **Covered** (Direct/Payload) |
| **Token Economics** | Cached Read Tokens | `token.usage (type=cache)` | `token.usage (type=cache_read)` | `cached_token_count` | *(Meters via quota)* | *(Not shown)* | `gen_ai.usage.cache_read_tokens` | **Covered** (Direct/Payload) |
| **Token Economics** | Total Token Volume | `token.usage` | `token.usage` | `input + output` | *(Meters via quota)* | `gen_ai.usage.total_tokens` | `gen_ai.usage.total_tokens` | **Covered** (Formula) |
| **Token Economics** | Cache Utilization Rate | *(Not shown)* | `Cache Efficiency %` | `(cached / input) * 100` | *(Not shown)* | `Cache Utilization Rate` | Composite Formula `(C / I) * 100` | **Covered** (Formula) |
| **Token Economics** | Financial Cost (USD) | *(Not shown)* | `cost.usage` ($USD) | *(Not shown)* | *(Subscription)* | *(Not shown)* | `gen_ai.usage.cost_usd` | **Partially Covered** (See Notes) |
| **Token Economics** | Quota Fraction Remaining | *(Not shown)* | *(Not shown)* | *(Not shown)* | `agy.quota.remaining_fraction` | *(Not shown)* | `agent.quota.remaining_fraction` | **Covered** (Direct) |
| **Token Economics** | Quota Window Reset Time | *(Not shown)* | *(Not shown)* | *(Not shown)* | `agy.quota.seconds_to_reset` | *(Not shown)* | `agent.quota.seconds_to_reset` | **Covered** (Direct) |
| **Productivity** | Git Commits Created | *(Not shown)* | `commit.count` | *(Not shown)* | *(Not shown)* | *(Not shown)* | `agent.git.commit.count` | **Not Covered** (See Notes) |
| **Productivity** | Git PRs Created | *(Not shown)* | `pull_request.count` | *(Not shown)* | *(Not shown)* | *(Not shown)* | `agent.git.pr.count` | **Not Covered** (See Notes) |
| **Productivity** | Lines of Code Changed | *(Not shown)* | `lines_of_code.count` | *(Not shown)* | *(Not shown)* | *(Not shown)* | `agent.git.loc_changed` | **Not Covered** (See Notes) |
| **Productivity** | Active Time (CLI vs User) | *(Not shown)* | `active_time.total` | `sum(duration_ms)` | *(Duration)* | *(Duration)* | `agent.session.active_time` | **Partially Covered** (See Notes) |
| **Harness Internals**| Cold Startup Duration | `startup.total.bucket` | *(Not shown)* | *(Not shown)* | *(Not shown)* | *(Not shown)* | `agent.startup.duration` | **Not Covered** (See Notes) |
| **Harness Internals**| Startup Phase Latency | `startup.phase_duration` | *(Not shown)* | *(Not shown)* | *(Not shown)* | *(Not shown)* | `agent.startup.phase_duration` | **Not Covered** (See Notes) |
| **Harness Internals**| Error Categories | `error.count (category)` | *(Not shown)* | *(Not shown)* | *(Not shown)* | *(HTTP status)* | `error.type` | **Covered** (Span Status) |

---

## 3. Dashboard-by-Dashboard Granular Inventory

### 3.1 Grok Build (Grok CLI) — `01a08f6b-7e04-74ff-9c24-28d774057899`
- **Focus**: Performance, reliability, and startup overhead of the Grok CLI binary.
- **Metrics Discovered**:
  1. `grok_code.session.count`: Gauge/Counter of active Grok CLI sessions.
  2. `grok_code.turn.count`: Counter of turns taken, split by `outcome` (`success`, `failed`).
  3. `grok_code.tool.usage`: Tool executions grouped by `tool_name` and `outcome`.
  4. `grok_code.token.usage`: Token counter grouped by `type` (`input`, `output`, `cache`).
  5. `grok_code.error.count`: Counter of uncaught errors, categorized by `error_category` (`auth`, `network`, `timeout`, `parse`).
  6. `grok_code.startup.total.bucket`: Histogram of CLI bootstrap time (P95).
  7. `grok_code.startup.phase_duration.bucket`: Histogram breakdown by startup `phase` (`auth`, `plugin_discovery`, `workspace_scan`).

### 3.2 Grok (xAI API / SDK Platform) — `01a08f6b-a8d3-79b3-8e7f-c7994ee5773b`
- **Focus**: Raw LLM request latencies and HTTP-level telemetry for xAI foundation models.
- **Attributes Discovered**:
  - `gen_ai.request.model`: Model filter (`grok-2`, `grok-beta`).
  - `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`, `gen_ai.usage.total_tokens`: Standard GenAI token counts.
  - `duration_nano`: P95 duration of LLM inference calls.
  - `telemetry.sdk.language`, `http_method`, `response_status_code`, `has_error`: Standard OTel resource and HTTP attributes.

### 3.3 Claude Code Metrics — `01a08f6b-d576-7bb2-bc1c-40859a18ff08`
- **Focus**: Developer velocity, coding output, API cost economics, and tool acceptance.
- **Metrics & Logs Discovered**:
  1. `claude_code.session.count`: Number of interactive sessions.
  2. `claude_code.cost.usage`: Real-time financial spend in USD, grouped by `model` and time.
  3. `claude_code.token.usage`: Tokens grouped by `model` and `type`.
  4. `claude_code.lines_of_code.count`: Code modifications delivered (LOC added/removed).
  5. `claude_code.commit.count`: Number of Git commits created by the agent.
  6. `claude_code.pull_request.count`: Number of PRs opened by the agent.
  7. `claude_code.active_time.total`: Time spent in CLI execution vs waiting for user input.
  8. `claude_code.code_edit_tool.decision`: Human-in-the-loop decisions on file edits (`accept`, `reject`).
  9. `claude_code.tool_result`: Log stream recording `tool_name` and `success`.
  10. `claude_code.api_request`: Log stream recording `model` and `cost_usd`.

### 3.4 Codex — `01a08f6b-558b-7cc2-81f7-f9777c731f04`
- **Focus**: Enterprise CLI fleet observability, user attribution, and terminal behavior.
- **Attributes Discovered**:
  1. `conversation.id`: Unique conversation identifier.
  2. `user.email`: Attributed operator email address.
  3. `terminal.type`: Terminal emulator (`windows-terminal`, `vscode`, `xterm`).
  4. `tool_name`: Invoked CLI tool name.
  5. `call_id`: Distinct model / tool invocation ID.
  6. `model`: Model name (`o3-mini`, `gpt-4o`).
  7. `input_token_count`, `output_token_count`, `cached_token_count`: Token usage breakdown.
  8. `duration_ms`: Duration of interaction.
  9. `success`: Turn success boolean.
  10. `decision`: User authorization response (`allow`, `deny`, `ask`).

### 3.5 OpenAI (OpenAI Platform / SDK) — `01a08f6b-2c5c-715e-b891-ff98f19b4eee`
- **Focus**: Direct API instrumentation of OpenAI endpoints.
- **Attributes Discovered**:
  - Mirroring standard OpenTelemetry GenAI attributes (`gen_ai.request.model`, `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`, `duration_nano`, `telemetry.sdk.language`).

### 3.6 Claude Agent SDK — `01a08f6b-0327-718d-98ad-6dce5b17fb3b`
- **Focus**: Multi-step agent application traces built with Anthropic's Agent SDK.
- **Spans & Metrics Discovered**:
  - `span.type`: Categorizes spans into `'interaction'` (reasoning step) vs `'tool'` (execution step).
  - `tool_name`: Grouping for tool execution duration and count.
  - `claude_code.cost.usage` & `claude_code.token.usage`: Shared Anthropic metric counters.

### 3.7 Antigravity CLI — `01a08f6b-fbeb-7439-a6a9-0809f9da72a0`
- **Focus**: Autonomous loop stability, rate-limiting windows, and quiescence governance.
- **Spans & Metrics Discovered**:
  1. `agy.quota.remaining_fraction` & `agy.quota.seconds_to_reset`: Provider weekly rate-limiting bucket fractions and reset countdown.
  2. `gen_ai.conversation.id`: Interactive session identifier.
  3. `agy.hook.event`: Lifecycle transition (`PreToolUse`, `PostToolUse`, `PostInvocation`, `Stop`).
  4. `gen_ai.tool.name`: Granular tool calls over time.
  5. `agy.termination_reason`: Quiescence status (`NO_TOOL_CALL`, `USER_INTERRUPT`, `TURN_LIMIT`).
  6. `agy.execution.num` & `agy.fully_idle`: Loop recursion depth and full quiescence indicators.

---

## 4. Gap Analysis: What We Cover vs What We Do Not Cover

### 4.1 What `agent-otel-bridge` Fully Covers (100% Native or Canonical)
1. **Interactive Sessions**: Captured deterministically across all harnesses via `gen_ai.conversation.id`.
2. **Agent Reasoning Loops & Anti-Recursion**: Captured via `agent.hook.event` (`PostInvocation`) and `agent.step.index`.
3. **Tool Invocations & Latency**: Captured with nanosecond precision via OTel spans (`gen_ai.tool.name`, `durationNano`).
4. **Model Attribution**: Captured via `gen_ai.request.model` and `gen_ai.provider.name`.
5. **Token Usage (Input / Output / Cached)**: Fully represented in the bridge schema (`gen_ai.usage.*`). Provided natively by Codex and Claude Code hooks; supported for all agents via hook payload.
6. **User & Terminal Attribution**: Captured via `user.email` and `terminal.type`, with automatic environment fallback (`$USER_EMAIL`, `$GIT_AUTHOR_EMAIL`, `$WT_SESSION`, `$TERM_PROGRAM`).
7. **Turn Outcomes & Quiescence**: Captured via `agent.termination_reason`, `agent.decision`, and `agent.success`.
8. **Provider Quotas & Burn-down Rates**: Emitted as OTel metric gauges (`agent.quota.remaining_fraction`, `agent.quota.seconds_to_reset`). **This is an exclusive capability of `agent-otel-bridge` not present in any other CLI harness.**

---

### 4.2 What `agent-otel-bridge` Does NOT Currently Cover & Why

| Telemetry Variable | Dashboard Found In | Why It Is Not Currently Covered | Feasibility / Implementation Pathway |
|---|---|---|---|
| **Git Productivity** (`lines_of_code.count`, `commit.count`, `pull_request.count`) | Claude Code Metrics | Antigravity and Codex hooks do not calculate git diff statistics at turn boundaries. They only emit the executed tool (e.g. `run_command` git commit). | **High Feasibility**: The bridge daemon or hook can execute `git status --porcelain` and `git diff --shortstat` on `Stop` events to emit `agent.git.loc_changed` and `agent.git.commit.count`. |
| **Financial Cost in USD** (`cost.usage`, `cost_usd`) | Claude Code Metrics, Claude Agent SDK | Claude Code embeds an Anthropic pricing lookup table into its runtime to multiply tokens by model rates. Antigravity and OpenAI Codex do not emit dollar costs (Antigravity operates on fixed/subscription quota buckets). | **Medium Feasibility**: A configurable pricing table in `agent-otel-daemon` (`config.toml`) could multiply `gen_ai.usage.input_tokens` and `output_tokens` by model rates and emit `gen_ai.usage.cost_usd`. |
| **CLI vs User Active Time** (`active_time.total`) | Claude Code Metrics | Measures wall-clock time waiting for human typing vs agent LLM streaming. Currently, `agent-otel-bridge` records span duration during tool execution and turn completion, but does not measure the prompt input idle gap. | **Medium Feasibility**: Can be calculated in the daemon by measuring time elapsed between `Stop` of turn N and `PreToolUse` of turn N+1. |
| **CLI Internal Startup Benchmarks** (`startup.total.bucket`, `startup.phase_duration.bucket`) | Grok Build (Grok CLI) | These metrics measure the Grok binary's internal C++/Rust initialization phases (loading shared libraries, parsing local config) *before* any hook or child process is spawned. | **Harness Internal**: Cannot be measured by an external hook unless the CLI binary exposes startup timestamps in its hook payload. |

---

## 5. Architectural Recommendation

To unify all 7 dashboards into a single universal operator experience:
# Cross-Agent Telemetry Inventory & Dashboard Analysis

This document provides an exhaustive inventory of the variables, attributes, and metrics across the 7 reference AI Agent and LLM dashboards in SigNoz, mapped against the capabilities of `agent-otel-bridge`.

---

## 1. Reference Dashboards Catalog

| # | SigNoz Dashboard ID | Dashboard Title | Target Ecosystem | Panels | Dashboard Variables | Primary Signals |
|---|---|---|---|:---:|---|:---:|
| 1 | `01a08f6b-7e04-74ff-9c24-28d774057899` | **Grok Build (Grok CLI)** | xAI Grok CLI Harness | 13 | *(None)* | Metrics |
| 2 | `01a08f6b-a8d3-79b3-8e7f-c7994ee5773b` | **Grok** | xAI LLM API / Platform | 13 | `$service_name`, `$language`, `$llm_model` | Traces & Logs |
| 3 | `01a08f6b-d576-7bb2-bc1c-40859a18ff08` | **Claude Code Metrics** | Anthropic Claude Code CLI | 21 | *(None)* | Metrics & Logs |
| 4 | `01a08f6b-558b-7cc2-81f7-f9777c731f04` | **Codex** | OpenAI Codex CLI | 17 | `$service_name` | Traces & Logs |
| 5 | `01a08f6b-2c5c-715e-b891-ff98f19b4eee` | **OpenAI** | OpenAI API / Python SDK | 11 | `$service_name`, `$language`, `$llm_model` | Traces & Logs |
| 6 | `01a08f6b-0327-718d-98ad-6dce5b17fb3b` | **Claude Agent SDK** | Anthropic Agent SDK | 15 | `$service_name`, `$llm_model` | Traces & Metrics |
| 7 | `01a08f6b-fbeb-7439-a6a9-0809f9da72a0` | **Antigravity CLI** | Google Antigravity (AGY) | 11 | `$service_name` | Traces & Metrics |

---

## 2. Cross-Agent Common Telemetry Matrix

This table synthesizes all telemetry concepts across the 7 dashboards, groups equivalent vendor-specific fields, identifies the canonical OpenTelemetry standard, and details whether `agent-otel-bridge` captures it.

| Functional Category | Telemetry Concept | Grok CLI (`grok_code`) | Claude Code (`claude_code`) | OpenAI Codex | Antigravity (`agy`) | OpenAI / Grok SDK API | Canonical OTel Convention | `agent-otel-bridge` Status |
|---|---|---|---|---|---|---|---|:---:|
| **Identity & Session** | Interactive Session ID | `session.count` | `session.count` | `conversation.id` | `gen_ai.conversation.id` | *(None / Stateless)* | `gen_ai.conversation.id` | **Covered** (Direct) |
| **Identity & Session** | Operator Attribution | *(Not shown)* | *(Not shown)* | `user.email` | *(Enriched)* | *(Not shown)* | `user.email` | **Covered** (Enriched) |
| **Identity & Session** | Terminal Environment | *(Not shown)* | *(Not shown)* | `terminal.type` | *(Enriched)* | *(Not shown)* | `terminal.type` | **Covered** (Enriched) |
| **Identity & Session** | Workstation Hostname | *(Not shown)* | *(Not shown)* | `host.name` | `host.name` | `host.name` | `host.name` | **Covered** (Enriched) |
| **Identity & Session** | Service Name | *(Not shown)* | `service.name` | `service.name` | `service.name` | `service.name` | `service.name` | **Covered** (Direct) |
| **Reasoning Loop** | Turns / Iterations | `turn.count` | *(Inferred)* | `call_id` | `agent.step.index` / `PostInvocation` | *(Single Request)* | `agent.hook.event = 'PostInvocation'` | **Covered** (Direct) |
| **Reasoning Loop** | Step Counter | *(Not shown)* | *(Not shown)* | *(Not shown)* | `agent.step.index` | *(None)* | `agent.step.index` | **Covered** (Direct) |
| **Reasoning Loop** | Quiescence / Idle State | *(Not shown)* | *(Not shown)* | *(Not shown)* | `agent.fully_idle` | *(None)* | `agent.fully_idle` | **Covered** (Direct) |
| **Tools & Actions** | Tool Call Count | `tool.usage` | `code_edit_tool.decision` | `count(call_id)` | `count(gen_ai.tool.name)` | *(None)* | `gen_ai.operation.name = 'execute_tool'` | **Covered** (Direct) |
| **Tools & Actions** | Tool Name | `tool_name` | `tool_name` | `tool_name` | `gen_ai.tool.name` | *(None)* | `gen_ai.tool.name` | **Covered** (Direct) |
| **Tools & Actions** | Tool Execution Latency | *(Histograms)* | *(Inferred)* | `duration_ms` | `durationNano` | `duration_nano` | `durationNano` (Span) | **Covered** (Direct) |
| **Turn Outcomes** | Quiescence Reason | `outcome` | *(Exit code)* | `success` | `agent.termination_reason` | *(HTTP Status)* | `agent.termination_reason` | **Covered** (Direct) |
| **Turn Outcomes** | OTel Finish Reasons | *(Not shown)* | *(Not shown)* | `stop` | `NO_TOOL_CALL` -> `stop` | `finish_reasons` | `gen_ai.response.finish_reasons` | **Covered** (Direct) |
| **Turn Outcomes** | Policy / Human Approval | *(Not shown)* | `code_edit_tool.decision` | `decision` (`allow`/`deny`/`ask`) | `decision` | *(None)* | `agent.decision` | **Covered** (Direct) |
| **Turn Outcomes** | Turn Success / Failure | `outcome` | `success` | `success` | `status_code` | `has_error` | `agent.success` & `StatusCode` | **Covered** (Direct) |
| **Model & Routing** | Foundation Model Name | `model` | `model` | `model` | `gen_ai.request.model` | `gen_ai.request.model` | `gen_ai.request.model` | **Covered** (Direct) |
| **Model & Routing** | AI Provider Name | `xai` | `anthropic` | `openai` | `google` | `openai` / `xai` | `gen_ai.provider.name` | **Covered** (Direct) |
| **Token Economics** | Input Tokens | `token.usage (type=input)` | `token.usage (type=input)` | `input_token_count` | *(Meters via quota)* | `gen_ai.usage.input_tokens` | `gen_ai.usage.input_tokens` | **Covered** (Direct/Payload) |
| **Token Economics** | Output Tokens | `token.usage (type=output)` | `token.usage (type=output)` | `output_token_count` | *(Meters via quota)* | `gen_ai.usage.output_tokens` | `gen_ai.usage.output_tokens` | **Covered** (Direct/Payload) |
| **Token Economics** | Cached Read Tokens | `token.usage (type=cache)` | `token.usage (type=cache_read)` | `cached_token_count` | *(Meters via quota)* | *(Not shown)* | `gen_ai.usage.cache_read_tokens` | **Covered** (Direct/Payload) |
| **Token Economics** | Total Token Volume | `token.usage` | `token.usage` | `input + output` | *(Meters via quota)* | `gen_ai.usage.total_tokens` | `gen_ai.usage.total_tokens` | **Covered** (Formula) |
| **Token Economics** | Cache Utilization Rate | *(Not shown)* | `Cache Efficiency %` | `(cached / input) * 100` | *(Not shown)* | `Cache Utilization Rate` | Composite Formula `(C / I) * 100` | **Covered** (Formula) |
| **Token Economics** | Financial Cost (USD) | *(Not shown)* | `cost.usage` ($USD) | *(Not shown)* | *(Subscription)* | *(Not shown)* | `gen_ai.usage.cost_usd` | **Partially Covered** (See Notes) |
| **Token Economics** | Quota Fraction Remaining | *(Not shown)* | *(Not shown)* | *(Not shown)* | `agy.quota.remaining_fraction` | *(Not shown)* | `agent.quota.remaining_fraction` | **Covered** (Direct) |
| **Token Economics** | Quota Window Reset Time | *(Not shown)* | *(Not shown)* | *(Not shown)* | `agy.quota.seconds_to_reset` | *(Not shown)* | `agent.quota.seconds_to_reset` | **Covered** (Direct) |
| **Productivity** | Git Commits Created | *(Not shown)* | `commit.count` | *(Not shown)* | *(Not shown)* | *(Not shown)* | `agent.git.commit.count` | **Not Covered** (See Notes) |
| **Productivity** | Git PRs Created | *(Not shown)* | `pull_request.count` | *(Not shown)* | *(Not shown)* | *(Not shown)* | `agent.git.pr.count` | **Not Covered** (See Notes) |
| **Productivity** | Lines of Code Changed | *(Not shown)* | `lines_of_code.count` | *(Not shown)* | *(Not shown)* | *(Not shown)* | `agent.git.loc_changed` | **Not Covered** (See Notes) |
| **Productivity** | Active Time (CLI vs User) | *(Not shown)* | `active_time.total` | `sum(duration_ms)` | *(Duration)* | *(Duration)* | `agent.session.active_time` | **Partially Covered** (See Notes) |
| **Harness Internals**| Cold Startup Duration | `startup.total.bucket` | *(Not shown)* | *(Not shown)* | *(Not shown)* | *(Not shown)* | `agent.startup.duration` | **Not Covered** (See Notes) |
| **Harness Internals**| Startup Phase Latency | `startup.phase_duration` | *(Not shown)* | *(Not shown)* | *(Not shown)* | *(Not shown)* | `agent.startup.phase_duration` | **Not Covered** (See Notes) |
| **Harness Internals**| Error Categories | `error.count (category)` | *(Not shown)* | *(Not shown)* | *(Not shown)* | *(HTTP status)* | `error.type` | **Covered** (Span Status) |

---

## 3. Dashboard-by-Dashboard Granular Inventory

### 3.1 Grok Build (Grok CLI) — `01a08f6b-7e04-74ff-9c24-28d774057899`
- **Focus**: Performance, reliability, and startup overhead of the Grok CLI binary.
- **Metrics Discovered**:
  1. `grok_code.session.count`: Gauge/Counter of active Grok CLI sessions.
  2. `grok_code.turn.count`: Counter of turns taken, split by `outcome` (`success`, `failed`).
  3. `grok_code.tool.usage`: Tool executions grouped by `tool_name` and `outcome`.
  4. `grok_code.token.usage`: Token counter grouped by `type` (`input`, `output`, `cache`).
  5. `grok_code.error.count`: Counter of uncaught errors, categorized by `error_category` (`auth`, `network`, `timeout`, `parse`).
  6. `grok_code.startup.total.bucket`: Histogram of CLI bootstrap time (P95).
  7. `grok_code.startup.phase_duration.bucket`: Histogram breakdown by startup `phase` (`auth`, `plugin_discovery`, `workspace_scan`).

### 3.2 Grok (xAI API / SDK Platform) — `01a08f6b-a8d3-79b3-8e7f-c7994ee5773b`
- **Focus**: Raw LLM request latencies and HTTP-level telemetry for xAI foundation models.
- **Attributes Discovered**:
  - `gen_ai.request.model`: Model filter (`grok-2`, `grok-beta`).
  - `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`, `gen_ai.usage.total_tokens`: Standard GenAI token counts.
  - `duration_nano`: P95 duration of LLM inference calls.
  - `telemetry.sdk.language`, `http_method`, `response_status_code`, `has_error`: Standard OTel resource and HTTP attributes.

### 3.3 Claude Code Metrics — `01a08f6b-d576-7bb2-bc1c-40859a18ff08`
- **Focus**: Developer velocity, coding output, API cost economics, and tool acceptance.
- **Metrics & Logs Discovered**:
  1. `claude_code.session.count`: Number of interactive sessions.
  2. `claude_code.cost.usage`: Real-time financial spend in USD, grouped by `model` and time.
  3. `claude_code.token.usage`: Tokens grouped by `model` and `type`.
  4. `claude_code.lines_of_code.count`: Code modifications delivered (LOC added/removed).
  5. `claude_code.commit.count`: Number of Git commits created by the agent.
  6. `claude_code.pull_request.count`: Number of PRs opened by the agent.
  7. `claude_code.active_time.total`: Time spent in CLI execution vs waiting for user input.
  8. `claude_code.code_edit_tool.decision`: Human-in-the-loop decisions on file edits (`accept`, `reject`).
  9. `claude_code.tool_result`: Log stream recording `tool_name` and `success`.
  10. `claude_code.api_request`: Log stream recording `model` and `cost_usd`.

### 3.4 Codex — `01a08f6b-558b-7cc2-81f7-f9777c731f04`
- **Focus**: Enterprise CLI fleet observability, user attribution, and terminal behavior.
- **Attributes Discovered**:
  1. `conversation.id`: Unique conversation identifier.
  2. `user.email`: Attributed operator email address.
  3. `terminal.type`: Terminal emulator (`windows-terminal`, `vscode`, `xterm`).
  4. `tool_name`: Invoked CLI tool name.
  5. `call_id`: Distinct model / tool invocation ID.
  6. `model`: Model name (`o3-mini`, `gpt-4o`).
  7. `input_token_count`, `output_token_count`, `cached_token_count`: Token usage breakdown.
  8. `duration_ms`: Duration of interaction.
  9. `success`: Turn success boolean.
  10. `decision`: User authorization response (`allow`, `deny`, `ask`).

### 3.5 OpenAI (OpenAI Platform / SDK) — `01a08f6b-2c5c-715e-b891-ff98f19b4eee`
- **Focus**: Direct API instrumentation of OpenAI endpoints.
- **Attributes Discovered**:
  - Mirroring standard OpenTelemetry GenAI attributes (`gen_ai.request.model`, `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`, `duration_nano`, `telemetry.sdk.language`).

### 3.6 Claude Agent SDK — `01a08f6b-0327-718d-98ad-6dce5b17fb3b`
- **Focus**: Multi-step agent application traces built with Anthropic's Agent SDK.
- **Spans & Metrics Discovered**:
  - `span.type`: Categorizes spans into `'interaction'` (reasoning step) vs `'tool'` (execution step).
  - `tool_name`: Grouping for tool execution duration and count.
  - `claude_code.cost.usage` & `claude_code.token.usage`: Shared Anthropic metric counters.

### 3.7 Antigravity CLI — `01a08f6b-fbeb-7439-a6a9-0809f9da72a0`
- **Focus**: Autonomous loop stability, rate-limiting windows, and quiescence governance.
- **Spans & Metrics Discovered**:
  1. `agy.quota.remaining_fraction` & `agy.quota.seconds_to_reset`: Provider weekly rate-limiting bucket fractions and reset countdown.
  2. `gen_ai.conversation.id`: Interactive session identifier.
  3. `agy.hook.event`: Lifecycle transition (`PreToolUse`, `PostToolUse`, `PostInvocation`, `Stop`).
  4. `gen_ai.tool.name`: Granular tool calls over time.
  5. `agy.termination_reason`: Quiescence status (`NO_TOOL_CALL`, `USER_INTERRUPT`, `TURN_LIMIT`).
  6. `agy.execution.num` & `agy.fully_idle`: Loop recursion depth and full quiescence indicators.

---

## 4. Gap Analysis: What We Cover vs What We Do Not Cover

### 4.1 What `agent-otel-bridge` Fully Covers (100% Native or Canonical)
1. **Interactive Sessions**: Captured deterministically across all harnesses via `gen_ai.conversation.id`.
2. **Agent Reasoning Loops & Anti-Recursion**: Captured via `agent.hook.event` (`PostInvocation`) and `agent.step.index`.
3. **Tool Invocations & Latency**: Captured with nanosecond precision via OTel spans (`gen_ai.tool.name`, `durationNano`).
4. **Model Attribution**: Captured via `gen_ai.request.model` and `gen_ai.provider.name`.
5. **Token Usage (Input / Output / Cached)**: Fully represented in the bridge schema (`gen_ai.usage.*`). Provided natively by Codex and Claude Code hooks; supported for all agents via hook payload.
6. **User & Terminal Attribution**: Captured via `user.email` and `terminal.type`, with automatic environment fallback (`$USER_EMAIL`, `$GIT_AUTHOR_EMAIL`, `$WT_SESSION`, `$TERM_PROGRAM`).
7. **Turn Outcomes & Quiescence**: Captured via `agent.termination_reason`, `agent.decision`, and `agent.success`.
8. **Provider Quotas & Burn-down Rates**: Emitted as OTel metric gauges (`agent.quota.remaining_fraction`, `agent.quota.seconds_to_reset`). **This is an exclusive capability of `agent-otel-bridge` not present in any other CLI harness.**

---

### 4.2 What `agent-otel-bridge` Does NOT Currently Cover & Why

| Telemetry Variable | Dashboard Found In | Why It Is Not Currently Covered | Feasibility / Implementation Pathway |
|---|---|---|---|
| **Git Productivity** (`lines_of_code.count`, `commit.count`, `pull_request.count`) | Claude Code Metrics | Antigravity and Codex hooks do not calculate git diff statistics at turn boundaries. They only emit the executed tool (e.g. `run_command` git commit). | **High Feasibility**: The bridge daemon or hook can execute `git status --porcelain` and `git diff --shortstat` on `Stop` events to emit `agent.git.loc_changed` and `agent.git.commit.count`. |
| **Financial Cost in USD** (`cost.usage`, `cost_usd`) | Claude Code Metrics, Claude Agent SDK | Claude Code embeds an Anthropic pricing lookup table into its runtime to multiply tokens by model rates. Antigravity and OpenAI Codex do not emit dollar costs (Antigravity operates on fixed/subscription quota buckets). | **Medium Feasibility**: A configurable pricing table in `agent-otel-daemon` (`config.toml`) could multiply `gen_ai.usage.input_tokens` and `output_tokens` by model rates and emit `gen_ai.usage.cost_usd`. |
| **CLI vs User Active Time** (`active_time.total`) | Claude Code Metrics | Measures wall-clock time waiting for human typing vs agent LLM streaming. Currently, `agent-otel-bridge` records span duration during tool execution and turn completion, but does not measure the prompt input idle gap. | **Medium Feasibility**: Can be calculated in the daemon by measuring time elapsed between `Stop` of turn N and `PreToolUse` of turn N+1. |
| **CLI Internal Startup Benchmarks** (`startup.total.bucket`, `startup.phase_duration.bucket`) | Grok Build (Grok CLI) | These metrics measure the Grok binary's internal C++/Rust initialization phases (loading shared libraries, parsing local config) *before* any hook or child process is spawned. | **Harness Internal**: Cannot be measured by an external hook unless the CLI binary exposes startup timestamps in its hook payload. |

---

## 5. Architectural Recommendation

To unify all 7 dashboards into a single universal operator experience:
1. **Standardize on the Universal Dashboard**:
   The newly published dashboard `ai-agent-observability.json` already merges the essential telemetry from all 7 dashboards (Sessions, Loops, Quotas, Tools, Models, Outcomes, and Latencies).
2. **Phase 2 Expansion (Optional Git & Cost Enrichment)**:
   Adding an opt-in Git statistics collector (`git diff --shortstat` on `Stop`) and a model cost calculator will achieve **100% parity with Claude Code Metrics**.

---

## 6. Approved Harnessing Extensions & Zero-Duplicate Architecture

Based on architecture alignment for the v0.2 milestone:

### 6.1 Operating Modes (`agent.execution.mode`)
We classify harness execution into two unambiguous operational paradigms:
1. **`interactive` (Human-in-the-Loop / REPL / TUI)**:
   - Developer interacts live via terminal TUI/REPL.
   - Detected via active console TTY (`isatty()`) and absence of batch/prompt flags.
2. **`automation` (Headless / Scripted / Pipelines)**:
   - Automated invocation via `-p`, `--prompt`, stdin pipes, CI/CD scripts, or background subagent daemons.
   - Detected via non-TTY stdin, CLI inspection of parent process (`-p`), or automation environment variables.

### 6.2 Git Intelligence Strategy
- **Sampling Point**: Exclusively on the **Stop** event (turn quiescence).
- **Execution Overhead**: < 5ms via lightweight `git diff --shortstat` and `git status --porcelain`.
- **Metrics Emitted**:
  - `agent.git.lines_added` & `agent.git.lines_deleted`
  - `agent.git.files_changed`
  - `agent.git.self_revert` (boolean flag detecting circular reasoning / rollback loops)

### 6.3 Dual-Track Quota Architecture
- **Individual Provider Track**:
  - Preserves exact individual gauges per provider and bucket (`agent.quota.remaining_fraction`, `agent.quota.seconds_to_reset`, `bucket`, `group`).
  - Allows operators to filter by `$agent_name` and inspect vendor-native quota behavior.
- **Fleet Aggregate Track (Normalized)**:
  - **Bottleneck Gauge**: `min(agent.quota.remaining_fraction)` across all active providers, alerting instantly when any provider nears rate exhaustion (`agent.fleet.bottleneck_ratio`).
  - **Fleet Token Burn**: Rolling sum of tokens consumed across all token-metered agents on the workstation.

### 6.4 Zero-Duplicate Ingestion Strategy
To prevent duplicate collection when a platform already has native OpenTelemetry:
1. **TraceContext Propagation (W3C `traceparent`)**:
   - If an upstream agent already creates a trace and injects `traceparent`, `agent-hook` attaches as an enriched child span instead of spawning an isolated trace root.
2. **Local Daemon as Ingest & Enrichment Hub (Port 4318 Proxy)**:
   - Platforms with native OTel (e.g. Claude Code or OpenAI SDK) can emit directly to `http://127.0.0.1:4318`.
   - The daemon intercepts native spans, enriches them with workstation Git stats and quota telemetry, and forwards a single batch to SigNoz.
3. **Smart Hook De-duplication**:
   - When `agent-hook` detects that the parent process is already configured with an active direct OTLP exporter, it suppresses duplicate execution spans and emits only the unique harnessing signals (Git churn, quota snapshot, operating mode).

---

## 7. Published Persona Dashboards (SigNoz v6)

Four specialized persona dashboards have been published to SigNoz, addressing distinct operator needs:

| Dashboard Name | SigNoz UUID | Target Persona | Key Signals & Panels |
|---|---|---|---|
| **AI Fleet Executive & AI Governance** | `01a091ef-5319-7646-8fdd-efabd14f5708` | VP of Eng, Head of AI Platform | Fleet Quota Headroom (`agent.fleet.bottleneck_ratio`), Active Agent Sessions, Execution Mode (`interactive` vs `automation`), Ecosystem Share, Terminal Environments, Developer Attribution (`user.email`). |
| **AI Agent SRE & Loop Protection** | `01a091ef-533b-768a-917f-151c1c991f16` | AI Platform SRE, Reliability Engineers | Runaway Loop Invocations (`PostInvocation` spikes), Provider Quotas Remaining, Window Reset Countdown, Tool Latency P95, Tool Failures & Error Breakdown. |
| **AI Developer Productivity & Git Churn** | `01a091ef-5360-74d2-98d2-841f093849f6` | Engineering Managers, Tech Leads | Lines of Code Added/Deleted, Self-Revert Anomaly Count (`agent.git.self_revert`), Files Changed Distribution, Human Decisions (`allow`/`deny`/`ask`), Turns Timeline. |
| **AI Agent Turn Inspector & Debugger** | `01a091ee-025a-7219-9695-eee9131b561b` | AI Agent Developers, Prompt Engineers | Why Turns Ended (`NO_TOOL_CALL`, `USER_INTERRUPT`, `TURN_LIMIT`), Model Request Distribution, Real-Time Turn Audit Feed with mode, reason, and Git churn. |

The source JSON definitions are stored in [`contrib/dashboards/signoz/`](file:///C:/Users/samue/code/agent-otel-bridge/contrib/dashboards/signoz/).
