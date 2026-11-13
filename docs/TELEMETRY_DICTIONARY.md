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

The table below describes bridge schema mappings for the five [built-in harness identities](../crates/agent-otel-core/src/model.rs#L142-L205), not verified native field availability for every harness version. Optional payload fields are emitted only when supplied or resolved by the bridge ([span builder](../crates/agent-otel-core/src/otlp.rs#L275-L480)). AOB emits `claude-code`; downstream dendro uses `claude` ([F12](BACKLOG.md#verified-aob-facts)). These names must not be assumed identical for joins.

| Taxonomy Dimension | Canonical OTel Attribute / Metric | OTel Signal | Type | Antigravity | Claude Code | OpenAI Codex | xAI Grok | Pi (pi.dev) | Source & Description |
|---|---|---|---|:---:|:---:|:---:|:---:|:---:|---|
| **Identity & Session** | `gen_ai.conversation.id` | Span Attr | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Conversation / session identifier when supplied; always a per-install keyed pseudonym or omitted in Minimal, including opaque IDs. Session-to-trace joins use `agent.session.link_id`; inbound W3C context retains precedence. |
| **Identity & Session** | `agent.session.link_id` | Span Attr | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | `link-id-v1` keyed session join: 64 lowercase hex characters, or `unlinked`. Present on every transformed span in every mode. Provisioned K_link and raw session inputs resolve before privacy projection; provider/model fields are excluded. |
| **Identity & Session** | `agent.session.unlinked_reason` | Span Attr | `string` | **Conditional** | **Conditional** | **Conditional** | **Conditional** | **Conditional** | Only when unlinked: `invalid_key_material`, `non_harness_origin`, `missing_org`, `missing_endpoint_scope`, `missing_harness`, `unknown_harness`, `missing_native_session`, `invalid_field_characters`. The core two-source resolver also defines `conflicting_harness`; A2 uses authoritative wire identity and does not supply both sources. No key uses the SPEC equivalent `invalid_key_material`. |
| **Identity & Session** | `user.email` | Span Attr | `string` | *Opt-in* | *Opt-in* | *Opt-in* | *Opt-in* | *Opt-in* | Omitted in default `minimal` capture mode. In `metadata_only` / `full` modes, resolved from the payload or `$USER_EMAIL` (and git config / `$GIT_AUTHOR_EMAIL` when `AGENT_OTEL_ENRICH_EMAIL=true\|1`). See [§5.1.1 privacy projection](#511-current-path-email-and-error-export-behaviour). |
| **Identity & Session** | `terminal.type` | Span Attr | `string` | *Enriched* | *Enriched* | **Yes** | *Enriched* | *Enriched* | Terminal emulator (closed vocabulary in `minimal` mode, e.g. `windows-terminal`, `xterm-256color`, `vscode`, `other`). |
| **Identity & Session** | `service.name` | Resource | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Standard OTel service name (`agent-otel-bridge` or custom via `OTEL_SERVICE_NAME`). |
| **Identity & Session** | `agent.capture.mode` | Resource | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Active daemon ingress privacy capture mode (`minimal` default, `metadata_only`, `full`) configured via `AGENT_OTEL_CAPTURE_MODE`. |
| **Identity & Session** | `host.name` | Resource | `string` | *Opt-in* | *Opt-in* | *Opt-in* | *Opt-in* | *Opt-in* | Omitted in default `minimal` capture mode; emitted in `metadata_only` and `full` modes. |
| **Identity & Session** | `os.type` | Resource | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Operating system type (`windows`, `linux`, `macos`). Enriched automatically. |
| **Agent Lifecycle** | `gen_ai.agent.name` | Span Attr | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Canonical agent identifier (`antigravity`, `claude-code`, `codex`, `grok`, `pi`). |
| **Agent Lifecycle** | `agent.hook.event` | Span Attr | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Universal lifecycle event (`PreToolUse`, `PostToolUse`, `PostInvocation`, `Stop`). |
| **Agent Lifecycle** | `agent.step.index` | Span Attr | `int64` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | 0-indexed turn step within the current session. |
| **Agent Lifecycle** | `agent.execution.num` | Span Attr | `int64` | **Yes** | *Optional* | *Optional* | *Optional* | *Optional* | Monotonic execution counter within an agent loop pass. |
| **Agent Lifecycle** | `agent.fully_idle` | Span Attr | `bool` | **Yes** | *Optional* | *Optional* | *Optional* | *Optional* | Flag indicating agent reached complete quiescence without pending background tasks. |
| **Turn Outcomes** | `agent.termination_reason` | Span Attr | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Completion reason: `NO_TOOL_CALL`, `model_stop`, `USER_INTERRUPT`, `TURN_LIMIT`. |
| **Turn Outcomes** | `gen_ai.response.finish_reasons` | Span Attr | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Harness termination string when present (same value as `agent.termination_reason`). Not mapped to the GenAI finish-reason enum, and not emitted as a string array. |
| **Turn Outcomes** | `agent.decision` | Span Attr | `string` | **Yes** | **Yes** | **Yes** | *Optional* | *Optional* | Policy/Human-in-the-loop decision: `allow`, `deny`, `ask`. |
| **Turn Outcomes** | `agent.success` | Span Attr | `bool` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Boolean turn success derived from span status and exit codes. |
| **Tool Invocations** | `gen_ai.operation.name` | Span Attr | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | `execute_tool`, `invoke_agent`, `agent.stop`, or `agent.unknown` ([operation mapping](../crates/agent-otel-core/src/otlp.rs#L305-L330)). |
| **Tool Invocations** | `gen_ai.tool.name` | Span Attr | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Specific tool name (`run_command`, `Bash`, `view_file`, `grep_search`, etc.). |
| **Tool Invocations** | `gen_ai.tool.call.id` | Span Attr | `string` | **Yes** | **Yes** | **Yes** | **Yes** | *Optional* | Unique call ID associated with the specific tool execution. |
| **Tool Invocations** | `durationNano` / `duration_ms` | Span Field | `int64` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Observed interval between paired hook deliveries, not a harness CPU timer. See §5.6. Unpaired spans still have equal start and end. |
| **Model & Provider** | `gen_ai.provider.name` | Span Attr | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Provider inferred from the model, else the harness default (legacy mode only; omitted in gen_ai_latest_experimental mode when the model is absent): `google`, `anthropic`, `openai`, `xai`, `pi`. |
| **Model & Provider** | `gen_ai.request.model` | Span Attr | `string` | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | Active model identifier (`gemini-2.5-pro`, `claude-3-7-sonnet`, `o3-mini`, etc.). |
| **Economics & Quota** | `gen_ai.usage.input_tokens` | Span Attr | `int64` | *Wrapper* | **Yes** | **Yes** | **Yes** | *Optional* | Input / prompt token consumption count per turn. |
| **Economics & Quota** | `gen_ai.usage.output_tokens` | Span Attr | `int64` | *Wrapper* | **Yes** | **Yes** | **Yes** | *Optional* | Output / generation token consumption count per turn. |
| **Economics & Quota** | `gen_ai.usage.cache_read_tokens` | Span Attr | `int64` | *Wrapper* | **Yes** | **Yes** | *Optional* | *Optional* | Cached prompt tokens read from context window cache. |
| **Economics & Quota** | `agent.quota.remaining_fraction` | Metric Gauge | `double` | **Yes** | *Optional* | *Optional* | *Optional* | *Optional* | Provider quota balance fraction (`0.0..=1.0`), by `bucket` and `group`. |
| **Economics & Quota** | `agent.quota.seconds_to_reset` | Metric Gauge | `double` | **Yes** | *Optional* | *Optional* | *Optional* | *Optional* | Countdown time (in seconds) until the quota reset window. |

> **Key to Matrix**:
> - **Yes**: Representable in the bridge schema; this does not prove the native harness supplies it. Resource attributes are resolved by the bridge, and quota metrics use [installed-provider detection](../crates/agent-otel-daemon/src/platforms/mod.rs#L21-L49).
> - *Enriched*: Automatically detected and populated by `agent-otel-bridge` from workstation environment if omitted by harness.
> - *Wrapper*: Antigravity does not emit per-turn token counts in its hook payload (it meters quota via rolling weekly buckets). Token counts can be supplied by an optional API wrapper or transcript watcher.
> - *Optional*: Emitted if the harness version supports that field.

---

## 3. Comparative Evaluation: OpenAI Codex Dashboard vs `agent-otel-bridge`

The OpenAI Codex dashboard (`codex-dashboard.json`) in SigNoz is recognized as one of the most comprehensive agent monitoring dashboards. Below is a query-to-schema mapping, not a native capture audit. “Conditional” means a query can use the named bridge fields when present; it does not certify that each harness supplies those fields. “Unmeasured” marks panels whose requested duration semantics are not measured by the bridge. Emission is conditional in the [span builder](../crates/agent-otel-core/src/otlp.rs#L275-L480):

| # | Codex Dashboard Panel | Codex Underlying Attribute | `agent-otel-bridge` Canonical Attribute | Coverage Status | Architectural Notes |
|---|---|---|---|:---:|---|
| 1 | **Conversations** | `count_distinct(conversation.id)` | `count_distinct(gen_ai.conversation.id)` | **Conditional** | OpenTelemetry GenAI standard convention. |
| 2 | **Conversations Over Time** | `conversation.id` over time | `gen_ai.conversation.id` over time | **Conditional** | Time series aggregation over `agent.stop` or `agent.hook.event`. |
| 3 | **Conversation Distribution By User** | `user.email` | `user.email` | **Conditional** | Emitted from the payload or `$USER_EMAIL`. Git-config enrichment is off unless `AGENT_OTEL_ENRICH_EMAIL` is set. |
| 4 | **Conversations by Terminal Type** | `terminal.type` | `terminal.type` | **Conditional** | Emitted from payload or enriched via `$WT_SESSION` / `$TERM_PROGRAM` / `$TERM`. |
| 5 | **Conversation Details** | `conversation.id`, `sum(duration_ms)`, `count_distinct(call_id)` | `gen_ai.conversation.id`, `sum(durationNano)`, `count_distinct(gen_ai.tool.call.id)` | **Unmeasured** | Paired duration is the daemon's observed interval between hook deliveries, not harness CPU time. See §5.6. |
| 6 | **Tool Call Distribution** | `tool_name` | `gen_ai.tool.name` | **Conditional** | Emitted when a tool name is present in the payload; schema support does not prove complete native harness coverage ([builder](../crates/agent-otel-core/src/otlp.rs#L287-L329)). |
| 7 | **Model Calls** | `count_distinct(call_id)` | `count_distinct(gen_ai.tool.call.id)` or span count | **Conditional** | Corresponds to `PostInvocation` or tool execution spans. |
| 8 | **Model Call Distribution** | `model` | `gen_ai.request.model` | **Conditional** | Grouped by canonical model identifier. |
| 9 | **Input Tokens** | `sum(input_token_count)` | `sum(gen_ai.usage.input_tokens)` | **Conditional** | Emitted when the corresponding token count is supplied in the payload ([builder](../crates/agent-otel-core/src/otlp.rs#L400-L409)). |
| 10 | **Output Tokens** | `sum(output_token_count)` | `sum(gen_ai.usage.output_tokens)` | **Conditional** | Emitted when the corresponding token count is supplied in the payload ([builder](../crates/agent-otel-core/src/otlp.rs#L400-L409)). |
| 11 | **Cached Tokens** | `sum(cached_token_count)` | `sum(gen_ai.usage.cache_read_tokens)` | **Conditional** | Emitted when the corresponding token count is supplied in the payload ([builder](../crates/agent-otel-core/src/otlp.rs#L400-L409)). |
| 12 | **Cache Utilization Rate** | `(cached / input) * 100` | `(cache_read_tokens / input_tokens) * 100` | **Conditional** | Supported via composite query formula `(B / A) * 100`. |
| 13 | **Token Usage Over Time** | `input + output` | `input_tokens + output_tokens` | **Conditional** | Composite time series formula `A + B`. |
| 14 | **Token Distribution By Model** | `input + output` by `model` | `input_tokens + output_tokens` by `gen_ai.request.model` | **Conditional** | Composite query grouped by model. |
| 15 | **Request Duration (P95)** | `P95(duration_ms)` | `quantile(0.95)(durationNano)` | **Unmeasured** | Paired duration is the daemon's observed interval between hook deliveries, not harness CPU time. See §5.6. |
| 16 | **Success Rate** | `count()` by `success` | `count()` by `agent.success` or `status.code` | **Conditional** | Captured directly in span status (`StatusCode::Ok` vs `StatusCode::Error`). |
| 17 | **User Decision** | `count()` by `decision` | `count()` by `agent.decision` | **Conditional** | Captured from `PreToolUse` hooks (`allow`, `deny`, `ask`). |

### Key Takeaways from Evaluation
1. **Conditional dashboard coverage**: The table maps dashboard queries to bridge fields; it does not establish complete native capture. Missing payload fields remain missing, and paired hook duration is not model request duration. See the [span builder](../crates/agent-otel-core/src/otlp.rs#L275-L480) and §5.6.
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
- When a termination reason is present, the bridge copies that harness string into `gen_ai.response.finish_reasons`. It does not map `NO_TOOL_CALL` to `["stop"]`, and the default export is a string, not a string array. The experimental opt-in below emits a one-element string array of the same raw value.

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
| `vcs.commit.sha` | `string` | Active commit: keyed `commit_<32hex>` pseudonym in Minimal (omitted without a key); raw SHA in opt-in modes | `commit_<32hex>` |
| `vcs.worktree.active` | `bool` | True if operating inside a secondary Git worktree | `true`, `false` |

### 5.1.1 Current path, email, and error export behaviour

**Historical F5 baseline before A1:** [Backlog fact F5](BACKLOG.md#verified-aob-facts) records the following code evidence, pinned to the planning baseline `96f9fdb`:

| Exported data | Current behaviour | Code evidence |
|---|---|---|
| Workspace paths | `workspace.path` and `workspace.project_root` are absolute by default. | [config.rs:36–47](https://github.com/smota/agent-otel-bridge/blob/96f9fdb/crates/agent-otel-daemon/src/config.rs#L36-L47) |
| Operator email | An explicit `USER_EMAIL` is eligible even with `AGENT_OTEL_ENRICH_EMAIL` off; a payload email takes precedence. | [daemon.rs:296–302](https://github.com/smota/agent-otel-bridge/blob/96f9fdb/crates/agent-otel-daemon/src/daemon.rs#L296-L302), [otlp.rs:417–419](https://github.com/smota/agent-otel-bridge/blob/96f9fdb/crates/agent-otel-core/src/otlp.rs#L417-L419) |
| Branch and repository names | Emitted in plaintext when present; changing the workspace path mode does not redact them. | [otlp.rs:459–469](https://github.com/smota/agent-otel-bridge/blob/96f9fdb/crates/agent-otel-core/src/otlp.rs#L459-L469) |
| Error status messages | Full raw error text is exported by default. Experimental truncation limits length, not sensitive content. | [otlp.rs:579–587](https://github.com/smota/agent-otel-bridge/blob/96f9fdb/crates/agent-otel-core/src/otlp.rs#L579-L587) |

[Work package A1](BACKLOG.md#aob-work-packages) (`AOB#17`) enforces an explicit daemon-side ingress privacy projection before OTLP export, controlled by `AGENT_OTEL_CAPTURE_MODE` and stamped on every exported `Resource` as `agent.capture.mode`:

- **`minimal` (default — content-free with per-install keyed pseudonyms):**
  - `agent.capture.mode` = `"minimal"` on every trace and metric `Resource`.
  - `host.name` and `user.email` are omitted on resources and spans (`ambient_user_email` is not harvested).
  - `workspace.path` and `workspace.project_root` are omitted entirely.
  - `workspace.project_name` and `vcs.repository.name` are replaced with per-install HMAC-SHA256 pseudonyms (`proj_<32hex>` and `repo_<32hex>`).
  - `vcs.branch.name` (and `vcs.ref.head.name` under `gen_ai_latest_experimental`) is mapped to a closed category set: `main`, `develop`, `feature`, `fix`, `release`, `docs`, `chore`, `detached`, or `other`.
  - `vcs.commit.sha` (or `vcs.ref.head.revision` under `gen_ai_latest_experimental`) is replaced with `commit_<32hex>` using the same HMAC scheme, domain `commit`, and lowercase full hexadecimal SHA as canonical input (40 or 64 hex digits). Invalid/abbreviated SHAs and values without an available key are omitted. Raw SHAs can identify a public or forge-hosted repository through global search and are never exported in Minimal.
  - `span.status.message` on error spans is set to the closed `agent.error.category` (`provider_quota_exhausted`, `user_interrupted`, `schema_validation_error`, or `tool_verification_failed`); raw error text and span events are never exported.
  - `agent.tool.binary` and `agent.tool.wrapped_binary` are restricted to a closed allowlist of standard developer binaries; unrecognized or custom script names are omitted.
  - Unrecognized custom `gen_ai.tool.name`, `capability.namespace`, `capability.name`, `gen_ai.agent.name`, and `gen_ai.agent.parent_name` values are pseudonymized (`tool_<32hex>`, `ns_<32hex>`, `cap_<32hex>`, `agent_<32hex>`, `parent_agent_<32hex>`), and `agent.termination_reason` / `agent.decision` / `terminal.type` are mapped to closed vocabularies.
  - All pseudonyms use the existing core HMAC-SHA256 implementation with a field-specific `aob.pseudonym.v1/<field>` domain, a NUL separator, and 16-byte truncation. They are stable only per install and do not guarantee anonymity. A2 retains this key separately from the jointly provisioned session-link K_link. Native conversation and root session IDs are always pseudonymized (`conv_<32hex>` / `root_<32hex>`) or omitted, even when opaque; no raw native session is exported in Minimal.
  - The daemon loads `pseudonym.key` once at startup under its runtime home (`AGENT_OTEL_HOME` overrides for tests; Windows `%LOCALAPPDATA%/agent-otel-bridge`; Unix `$XDG_DATA_HOME/agent-otel-bridge` or `$HOME/.local/share/agent-otel-bridge`). Missing keys are generated using the OS CSPRNG and created exclusively, with Unix mode 0600; Windows uses a protected owner-only file DACL before writing or reading key material. Windows warns once at startup, without paths or key material, if the home is outside the current user's profile; the runtime directory's inherited ACL still controls replacement/access. No key reads or creation occur on the event path. Malformed/unreadable/uncreatable keys fail closed: omit pseudonymized fields, continue span export, and count failed startups in `agent.bridge.privacy.key_unavailable.total` with the sole static label `reason=key_unavailable`. Key bytes never appear in Debug, logs or exports.
  - A final exact-key span allowlist drops future attributes until reviewed. The resource allowlist permits only bridge-owned `service.name=agent-otel-bridge`, `service.version`, telemetry SDK/distro identity, closed `service.namespace=agent-otel-bridge`, `os.type`, `agent.capture.mode`, and `deployment.environment` / `deployment.environment.name` (closed values `development`, `dev`, `test`, `staging`, `production`, `prod`, `local`, otherwise `other`). `OTEL_SERVICE_NAME` and version overrides are replaced with bridge identity/version. Other resource keys, including host/instance/enduser/team/Kubernetes identifiers and `os.description`, are dropped. Collector examples omit system resource detection so it cannot add `host.name` back.
- **`metadata_only` (`AGENT_OTEL_CAPTURE_MODE=metadata_only` or `metadata-only`):**
  - `agent.capture.mode` = `"metadata_only"`.
  - Exports operational metadata (`workspace.project_name`, credential-sanitized `vcs.repository.name`, `vcs.branch.name`, `host.name`, and `user.email` from payload/`$USER_EMAIL` or git config when `AGENT_OTEL_ENRICH_EMAIL=true|1`).
  - `AGENT_OTEL_WORKSPACE_PATH` controls `workspace.path` and `workspace.project_root` (`hash` default, `relative`, or `absolute`).
  - Content remains dropped (`span.status.message` uses the closed `agent.error.category` set; prompts, tool inputs/outputs, and span events are omitted), and credentials/tokens are scrubbed across all attributes.
- **`full` (`AGENT_OTEL_CAPTURE_MODE=full`):**
  - `agent.capture.mode` = `"full"`.
  - Exports operational metadata plus bounded `span.status.message` (truncated to 160 UTF-8 bytes), while still scrubbing credentials (URL userinfo, secret query parameters, bearer tokens, and `ghp_`/`github_pat_`/`glpat-`/`sk-`/`xox`/`AKIA`/`AIza`/JWT tokens) across all exported signals.

Tool archetype and capability kind are normalized to their closed enums in **all modes**; unrecognized values become `other`. Opt-in credential scrubbing is **best-effort**, including common colon/equal assignments, JSON token fields, `--token` arguments, `x-api-key` headers, and named AWS secret/session keys. Arbitrary unlabeled secrets and operational metadata can still be exposed in opt-in modes. The `hash` workspace mode uses the install key (and omits fields without it), rather than an unkeyed path digest.

A2 adds `agent.session.link_id` and `agent.session.unlinked_reason` to the Minimal span allowlist, including final batch projection. Provisioned administrative fields never become telemetry. With K_link, fallback trace IDs use the first 16 HMAC tag bytes for linked sessions; an unlinked session with a native ID uses HMAC-SHA256 over `LP("agent.session.trace-fallback/v1", org-or-empty, endpoint-or-empty, native_session)`. An all-zero 16-byte prefix is changed to `01` followed by 15 zero bytes. Without K_link the historical unkeyed fallback remains a dictionary-matching limitation; the raw conversation ID is still omitted or pseudonymized. W3C ingress order remains explicit payload, then transported hook environment, then fallback. Inherited trace ID, parent and flags never change when link attributes are added. See [provisioning](CONFIGURATION.md#keyed-session-links-a2--link-id-v1) and the [frozen contract](contracts/fact-export-v1/link-id-v1/SPEC.md).

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
Records inbound trace context; it does not implement spawn-time parent/child linking or produce a unified multi-agent flamegraph. See [lineage limits](WHY_AGENT_OTEL_BRIDGE.md#pillar-2-cross-agent-distributed-tracing--lineage) and the [resolver](../crates/agent-otel-core/src/trace_id.rs#L144-L166). A hook captures the source process's `TRACEPARENT` into the versioned IPC context envelope (`HookPayloadWithContext`, message type `0x04`); the legacy `HookPayload` (`0x01`) body remains unchanged. The transport carries only `TRACEPARENT`, with a local 512 UTF-8 byte limit. Missing, invalid, or oversized values are omitted without truncation. The daemon resolves context per event and does not use its own environment as a daemon-wide fallback.

| Attribute | Type | Description | Example Values |
| :--- | :--- | :--- | :--- |
| `gen_ai.agent.depth` | `int` | Explicitly recorded recursion hop depth ($0 = \text{root orchestrator}$); never inferred solely from TRACEPARENT presence | `0`, `1`, `2` |
| `gen_ai.agent.parent_name` | `string` | Name of the calling harness | `antigravity`, `claude-code` |
| `gen_ai.agent.root_id` | `string` | Root conversation ID binding the fleet turn | `conv-root-12345` |
| `gen_ai.agent.is_root` | `bool` | True if root orchestrator span | `true`, `false` |

#### Opt-in gen_ai_latest_experimental

Unset `OTEL_SEMCONV_STABILITY_OPT_IN` keeps the default names in the tables above. When the value contains the token `gen_ai_latest_experimental`, lineage and VCS attributes use the names below instead. The bridge does not emit both the old and new name for the same value. `agent.step.index` stays. `gen_ai.agent.step_index` is omitted. `gen_ai.response.finish_reasons` is a one-element string array of the raw harness termination string, including `NO_TOOL_CALL`. This opt-in does not control span pairing and does not change duration. Pairing is described in §5.6.

| Attribute | Type | Replaces |
| :--- | :--- | :--- |
| `agent.lineage.depth` | `int` | `gen_ai.agent.depth` |
| `agent.lineage.parent_name` | `string` | `gen_ai.agent.parent_name` |
| `agent.lineage.root_id` | `string` | `gen_ai.agent.root_id` |
| `agent.lineage.is_root` | `bool` | `gen_ai.agent.is_root` |
| `vcs.ref.head.name` | `string` | `vcs.branch.name` |
| `vcs.ref.head.revision` | `string` | `vcs.commit.sha` |
| `gen_ai.response.finish_reasons` | `string[]` | the default string copy of the harness termination reason |

### 5.5 Multi-Layer Error Categorization
Replaces ambiguous binary errors with actionable diagnostic categories:

| Attribute | Category Value | Operational Meaning | SRE Triage Action |
| :--- | :--- | :--- | :--- |
| `agent.error.category` | `tool_verification_failed` | Normal unit test or linter failure | Low; normal part of TDD feedback cycle. |
| `agent.error.category` | `provider_quota_exhausted` | Upstream rate limit or quota exceeded | High; triggers provider failover or wait window. |
| `agent.error.category` | `schema_validation_error` | Model emitted invalid JSON arguments | Medium; tune tool schema or system instructions. |
| `agent.error.category` | `user_interrupted` | User cancelled operation (Ctrl+C / Stop) | None; intentional human flow control. |

### 5.6 Bridge Pipeline Diagnostics

Bridge diagnostics are process-scoped operational facts. Counters are monotonic and cumulative until the daemon restarts; gauges are instantaneous snapshots. Labels use the finite values below. They never contain workspace paths, user identities, trace/span IDs, prompts, payloads, or error messages. Alert thresholds remain a backend policy.

A paired span's duration is the daemon's observed interval between the two hook deliveries (`PreToolUse` then `PostToolUse`, or `PreInvocation` then `PostInvocation` or `Stop`). It is not a harness CPU timer and must not be treated as a tool-latency SLA measurement. Every span the assembler emits sets `agent.span.paired`: `true` only when a Pre was joined with that later hook, `false` otherwise. Unpaired spans, including `Unknown` events and Pre hooks that expire, are evicted, or never see a match, still have equal start and end. The former synthetic 1 ms interval is no longer emitted. Internal hook execution is measured separately by the candidate observer; that interval is not tool execution time. Native spans with authoritative start/end timestamps retain their own duration semantics.

| Metric | OTLP aggregation | Unit | Labels | Meaning |
| :--- | :--- | :--- | :--- | :--- |
| `agent.bridge.events.total` | Cumulative monotonic sum | `{event}` | `stage`, `outcome` | Unique frames/events crossing `frame_received`, `admitted`, `transformed`, `export_queued`, and backend outcomes. Export retries do not increment event stages. |
| `agent.bridge.dropped.total` | Cumulative monotonic sum | `{event}` | `reason` | Events discarded for a finite reason such as invalid input, deadline, bounded-capacity rejection, span size, or shutdown. |
| `agent.bridge.queue.items` | Gauge | `{item}` | `queue` | Current item occupancy for the export and context-refresh queues. |
| `agent.bridge.queue.bytes` | Gauge | `By` | `queue`, `watermark` | Current or peak logical byte reservation for ingress, export, and the context cache. This is not process RSS. |
| `agent.bridge.context.refresh.total` | Cumulative monotonic sum | `{refresh}` | `result` | Context refreshes queued, completed, failed, rejected by queue/circuit capacity, or rejected for snapshot size. |
| `agent.bridge.export.attempts.total` | Cumulative monotonic sum | `{attempt}` | `result` | OTLP request attempts and batches whose delivery may have duplicated after a lost response. |
| `agent.bridge.connections` | Gauge | `{connection}` | `watermark` | Current or peak simultaneous ingress connections. |
| `agent.bridge.context.workers` | Gauge | `{worker}` | `state` | Fixed refresh workers currently active or considered stuck. |

For `agent.bridge.events.total{stage="backend"}`, `accepted`, `rejected`, and `unknown` are mutually exclusive accounting outcomes. A valid OTLP partial-success response contributes its accepted and rejected counts separately. A request sent without a conclusive response contributes to `unknown`; it must not also be counted as a confirmed drop. `agent.bridge.export.attempts.total` counts HTTP attempts, so it must not be summed with unique event counts. When `OTEL_SEMCONV_STABILITY_OPT_IN` contains `gen_ai_latest_experimental`, those four counters are emitted as `agent.bridge.events`, `agent.bridge.dropped`, `agent.bridge.context.refresh`, and `agent.bridge.export.attempts`, and the fleet token-rate gauge is `agent.fleet.token_rate` with unit `{token}/min` instead of `agent.fleet.token_rate_minute`; the bridge does not emit both names.
