# G0a Provider Attribution Investigation (`MTN-51` / `agent-otel-bridge#29`)

- **Issue**: [`smota/agent-otel-bridge#29`](https://github.com/smota/agent-otel-bridge/issues/29) (`MTN-51`)
- **Evidence Corpus Under Investigation**: Draft PR `#28` (`docs/g0a-sanitized-captures`, commit `aa71997ed1365179b43c1ef23a176a27b5d8f4d0`, 52 files under [`docs/contracts/fact-export-v1/captures/`](../contracts/fact-export-v1/captures/README.md))
- **Downstream Consumer Unblocked**: Contract package **`C1a` (`contract-identity`: identity and keys)** under `#24`
- **Root-Cause Verdict**: **`(b) Bridge attribution bug`** in `agent-otel-core` (`build_span_from_resolved`, [`crates/agent-otel-core/src/otlp.rs`](../../crates/agent-otel-core/src/otlp.rs#L296-L358))

---

## 1. Executive Summary & Classification

In all five Claude Code `2.1.322` capture scenarios (`S1`–`S5`) under [`docs/contracts/fact-export-v1/captures/g0a-20261112/claude-2.1.322/`](../contracts/fact-export-v1/captures/g0a-20261112/capture-matrix.md), every exported OTLP trace span in `otlp-traces-sanitized.json` (14 spans total) carries:

- `gen_ai.agent.name = "claude-code"`
- `gen_ai.provider.name = "google"`
- `gen_ai.system = "google"`
- No `gen_ai.request.model` attribute

### Classification Among Candidate Hypotheses

| Candidate | Verdict | Evidence Summary |
| :--- | :--- | :--- |
| **`(a)` Capture / labelling mistake** | **Ruled out** | Binary provenance (`claude` `2.1.322`, SHA-256 `bfb3703a...`), IPC wire headers (`client_id: 2` = `ClientKind::ClaudeCode`), span agent attribution (`gen_ai.agent.name = "claude-code"`), Claude-specific hook fields (`transcript_path`, `permission_mode`, `tool_use_id`), and coordinator-verified joins (`session_id` $\leftrightarrow$ `sessionId`, `tool_use_id` `CALL_...` across hook and session JSONL) confirm the captures are genuine Claude Code `2.1.322` runs. |
| **`(b)` Bridge attribution bug** | **CONFIRMED** | Real Claude Code `2.1.322` hook `stdin` JSON (`PreToolUse`, `PostToolUse`, `Stop`) does not include a `"model"` field. In [`crates/agent-otel-daemon/src/transform.rs`](../../crates/agent-otel-daemon/src/transform.rs#L124-L128), the daemon previously populated `input.agent_name = Some("claude-code")` from `WireHeader.client_id == 2`, while `input.model` remained `None`. Then [`crates/agent-otel-core/src/otlp.rs`](../../crates/agent-otel-core/src/otlp.rs#L296-L358) computed `inferred_provider = input.model.as_deref().map(infer_provider)` (`None`) and unconditionally fell back to `inferred_provider.unwrap_or(GEN_AI_PROVIDER_GOOGLE)` (`"google"`), ignoring the wire header `ClientKind` and `input.agent_name`. |
| **`(c)` Legitimate Google-hosted Claude route** | **Ruled out** | `agent-otel-bridge` does not inspect HTTP endpoints, Vertex AI / Antigravity routing headers, or session logs when building hook spans. In `agent-otel-core`, `"google"` was produced solely by `infer_provider(model)` when `model` contained `"gemini"` or by `.unwrap_or(GEN_AI_PROVIDER_GOOGLE)` when `input.model` was `None`. None of the 14 spans has `gen_ai.request.model` (which is always emitted at [`otlp.rs:367-369`](../../crates/agent-otel-core/src/otlp.rs#L367-L369) whenever `input.model.is_some()`), proving `input.model` was `None`. |
| **`(d)` Undetermined** | **Ruled out** | The mechanism is 100% deterministic and reproducible from the sanitized `hook-sanitized.jsonl` frames and `agent-otel-core` / `agent-otel-daemon` source code. |

---

## 2. Complete Attribution Inventory Across the 52 G0a Files

The G0a corpus in commit `aa71997` consists of **52 files**:
- 7 top-level/manifest files (`README.md`, `.gitattributes`, `g0a-20261112/LICENSE`, `g0a-20261112/RAW-PRIVATE-SHA256SUMS`, `g0a-20261112/SHA256SUMS`, `g0a-20261112/capture-matrix.md`, `g0a-20261112/capture-matrix.json`)
- 25 files under `g0a-20261112/claude-2.1.322/S1..S5/` (5 files per scenario)
- 20 files under `g0a-20261112/codex-0.167.0/S1..S5/` (4 files per scenario; no `otlp-traces-sanitized.json`)

### 2.1 Surface-by-Surface Attribution Matrix

| File / Surface Pattern | Files | Records / Spans | Harness / Client Attribution | Provider / System Attribution | Model Attribution | SDK / Scope Attribution |
| :--- | :---: | :---: | :--- | :--- | :--- | :--- |
| [`capture-matrix.json`](../contracts/fact-export-v1/captures/g0a-20261112/capture-matrix.json) & [`capture-matrix.md`](../contracts/fact-export-v1/captures/g0a-20261112/capture-matrix.md) | 2 | 2 harness entries, 10 scenario entries | `harnesses.claude-code`: `version = "2.1.322"`, `binary_sha256 = "bfb3703a..."`<br>`harnesses.codex`: `version = "0.167.0"`, `binary_sha256 = "cdcb45aa..."`<br>Finding `F12` notes `gen_ai.agent.name = "claude-code"` vs canonical `"claude"`. | None in matrix metadata | `"model"` listed in `session_jsonl.fields_present` for both harnesses; absent from `hook_stdin.fields_present` | `agent-otel-bridge` (`0.5.0`, binary SHA-256 `f4cf2ec8...`) |
| `claude-2.1.322/S1..S5/otlp-traces-sanitized.json` | 5 | 14 spans (`S1`: 4, `S2`: 4, `S3`: 2, `S4`: 2, `S5`: 2) | Span attr `gen_ai.agent.name = "claude-code"` on all 14 spans (`S5` stop span also has `name = "agent.stop"`). | Span attrs **`gen_ai.provider.name = "google"`** and **`gen_ai.system = "google"`** on all 14 spans (**BUG**). | **Absent** on all 14 spans (no `gen_ai.request.model` or `gen_ai.response.model`). | Resource: `telemetry.sdk.name = "agent-otel-bridge"`, `telemetry.distro.name = "agent-otel-bridge"`, `telemetry.sdk.language = "rust"`, `os.type = "linux"`.<br>Scope: `name = "agent-otel-bridge"`. |
| `claude-2.1.322/S1..S5/hook-sanitized.jsonl` | 5 | 43 lines (`34` `"origin": "harness"`, `4` `"origin": "synthetic"` collision in `S2`, `5` `"origin": "synthetic"` shutdown) | `header.client_id = 2` (`ClientKind::ClaudeCode`) on all 34 harness frames and all 4 `S2` collision frames (`client_id = 0` on the 5 `MsgType = 2` shutdown frames). | **Absent** in all hook payloads. | **Absent** in all hook payloads. | Captured by G0a IPC wire tee (`msg_type = 1` `HookPayload` / `msg_type = 3` `HookPayloadWithContext`). |
| `codex-0.167.0/S1..S5/hook-sanitized.jsonl` | 5 | 9 lines (`0` `"origin": "harness"`, `4` `"origin": "synthetic"` collision in `S2`, `5` `"origin": "synthetic"` shutdown) | **Secondary fixture finding**: `S2` lines 1–4 have `"origin": "synthetic"`, `"synthetic_kind": "collision"`, and **`header.client_id = 2`** (synthetic collision injector hardcoded `ClientKind::ClaudeCode` wire ID `2`). Real Codex hooks were disabled (`0` harness frames). | **Absent** in all hook payloads. | **Absent** in all hook payloads. | Captured by G0a IPC wire tee. |
| `claude-2.1.322/S1..S5/otlp-metrics-sanitized.json` & `codex-0.167.0/S1..S5/otlp-metrics-sanitized.json` | 10 | 10 metric batches (31 `resourceMetrics` entries in `claude`, 36 in `codex`) | **None** (no `gen_ai.agent.name` or harness attribute). | **None** (no `gen_ai.provider.name` or `gen_ai.system`). | **None** | **Secondary attribution finding**: Although mapped under `surface: "native_otlp"` in `capture-matrix.json`, all 10 files contain exclusively `agent-otel-bridge` daemon self-metrics (`telemetry.sdk.name = "agent-otel-bridge"`, `scope.name = "agent-otel-bridge"`, metrics `agent.bridge.{connections,context.refresh.total,context.workers,dropped.total,events.total,export.attempts.total,queue.bytes,queue.items}`). |
| `claude-2.1.322/S1..S5/session-log-sanitized.jsonl` | 5 | 87 lines (`S1`: 26, `S2`: 23, `S3`: 16, `S4`: 11, `S5`: 11) | Claude Code native JSONL schema (`type` in `session-owner`, `user`, `assistant`, `api-request`, `api-request-shape`, `rate-limit-event`, `last-prompt`, `queue-operation`; `version = "<str:len=7>"` = `"2.1.322"`). | Implied by Claude Code `api-request` / `assistant` schema; string values redacted by sanitizer. | Present on all 19 `type == "assistant"` lines at `message.model` (pseudonymized token `KEY_...`, e.g. `KEY_5c7c71301e3f2cba` in `S1`, `KEY_4ad64c3efa0825e5` in `S5`). | Native Claude Code session transcript under `~/.claude/projects/...`. |
| `codex-0.167.0/S1..S5/session-log-sanitized.jsonl` | 5 | 105 lines (`S1`: 26, `S2`: 26, `S3`: 17, `S4`: 20, `S5`: 16) | Codex native rollout JSONL schema (`type` in `session_meta`, `turn_context`, `response_item`, `event_msg`, `token_usage_record`, `world_state`). | Implied by Codex rollout schema and model family. | Present unredacted as **`"model": "gpt-6.1-sol"`** on `turn_context.payload.model` and nested `payload.model` across `S1`–`S5`. | Native Codex rollout log under `~/.codex/sessions/...`. |
| `{claude-2.1.322,codex-0.167.0}/S1..S5/git-provenance-sanitized.json` | 10 | 10 JSON objects | **None** | **None** | **None** | Workspace git status/branch/commit snapshot only. |

### 2.2 Per-Scenario Span Breakdown (`claude-2.1.322/S1..S5/otlp-traces-sanitized.json`)

Every span exported across `claude-2.1.322/S1..S5/otlp-traces-sanitized.json` exhibits the exact same attribution discrepancy:

| File | Span # | `name` | `agent.hook.event` | `gen_ai.operation.name` | `gen_ai.agent.name` | `gen_ai.provider.name` | `gen_ai.system` | `gen_ai.request.model` | `gen_ai.tool.call.id` |
| :--- | :---: | :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| [`claude-2.1.322/S1/otlp-traces-sanitized.json`](../contracts/fact-export-v1/captures/g0a-20261112/claude-2.1.322/S1/otlp-traces-sanitized.json) | 1 | `execute_tool Bash` | `PreToolUse` | `execute_tool` | `"claude-code"` | `"google"` | `"google"` | *(absent)* | `CALL_df62ccdb7d1b495f` |
| [`claude-2.1.322/S1/otlp-traces-sanitized.json`](../contracts/fact-export-v1/captures/g0a-20261112/claude-2.1.322/S1/otlp-traces-sanitized.json) | 2 | `execute_tool Bash` | `PostToolUse` | `execute_tool` | `"claude-code"` | `"google"` | `"google"` | *(absent)* | `CALL_df62ccdb7d1b495f` |
| [`claude-2.1.322/S1/otlp-traces-sanitized.json`](../contracts/fact-export-v1/captures/g0a-20261112/claude-2.1.322/S1/otlp-traces-sanitized.json) | 3 | `execute_tool Bash` | `PreToolUse` | `execute_tool` | `"claude-code"` | `"google"` | `"google"` | *(absent)* | `CALL_d7b89c8a885873e1` |
| [`claude-2.1.322/S1/otlp-traces-sanitized.json`](../contracts/fact-export-v1/captures/g0a-20261112/claude-2.1.322/S1/otlp-traces-sanitized.json) | 4 | `execute_tool Bash` | `PostToolUse` | `execute_tool` | `"claude-code"` | `"google"` | `"google"` | *(absent)* | `CALL_d7b89c8a885873e1` |
| [`claude-2.1.322/S2/otlp-traces-sanitized.json`](../contracts/fact-export-v1/captures/g0a-20261112/claude-2.1.322/S2/otlp-traces-sanitized.json) | 1 | `execute_tool Bash` | `PreToolUse` | `execute_tool` | `"claude-code"` | `"google"` | `"google"` | *(absent)* | `CALL_07e2be8f7825a75f` |
| [`claude-2.1.322/S2/otlp-traces-sanitized.json`](../contracts/fact-export-v1/captures/g0a-20261112/claude-2.1.322/S2/otlp-traces-sanitized.json) | 2 | `execute_tool Bash` | `PostToolUse` | `execute_tool` | `"claude-code"` | `"google"` | `"google"` | *(absent)* | `CALL_07e2be8f7825a75f` |
| [`claude-2.1.322/S2/otlp-traces-sanitized.json`](../contracts/fact-export-v1/captures/g0a-20261112/claude-2.1.322/S2/otlp-traces-sanitized.json) | 3 | `execute_tool Bash` | `PreToolUse` | `execute_tool` | `"claude-code"` | `"google"` | `"google"` | *(absent)* | `CALL_c4ed59d704a78202` |
| [`claude-2.1.322/S2/otlp-traces-sanitized.json`](../contracts/fact-export-v1/captures/g0a-20261112/claude-2.1.322/S2/otlp-traces-sanitized.json) | 4 | `execute_tool Bash` | `PostToolUse` | `execute_tool` | `"claude-code"` | `"google"` | `"google"` | *(absent)* | `CALL_c4ed59d704a78202` |
| [`claude-2.1.322/S3/otlp-traces-sanitized.json`](../contracts/fact-export-v1/captures/g0a-20261112/claude-2.1.322/S3/otlp-traces-sanitized.json) | 1 | `execute_tool Bash` | `PreToolUse` | `execute_tool` | `"claude-code"` | `"google"` | `"google"` | *(absent)* | `CALL_83dddf1641fd28f1` |
| [`claude-2.1.322/S3/otlp-traces-sanitized.json`](../contracts/fact-export-v1/captures/g0a-20261112/claude-2.1.322/S3/otlp-traces-sanitized.json) | 2 | `execute_tool Bash` | `PostToolUse` | `execute_tool` | `"claude-code"` | `"google"` | `"google"` | *(absent)* | `CALL_83dddf1641fd28f1` |
| [`claude-2.1.322/S4/otlp-traces-sanitized.json`](../contracts/fact-export-v1/captures/g0a-20261112/claude-2.1.322/S4/otlp-traces-sanitized.json) | 1 | `execute_tool Bash` | `PreToolUse` | `execute_tool` | `"claude-code"` | `"google"` | `"google"` | *(absent)* | `CALL_f4d197b1e7b8d7e7` |
| [`claude-2.1.322/S4/otlp-traces-sanitized.json`](../contracts/fact-export-v1/captures/g0a-20261112/claude-2.1.322/S4/otlp-traces-sanitized.json) | 2 | `execute_tool Bash` | `PostToolUse` | `execute_tool` | `"claude-code"` | `"google"` | `"google"` | *(absent)* | `CALL_f4d197b1e7b8d7e7` |
| [`claude-2.1.322/S5/otlp-traces-sanitized.json`](../contracts/fact-export-v1/captures/g0a-20261112/claude-2.1.322/S5/otlp-traces-sanitized.json) | 1 | `execute_tool Bash` | `PreToolUse` | `execute_tool` | `"claude-code"` | `"google"` | `"google"` | *(absent)* | `CALL_aa5a5943c17ccbb4` |
| [`claude-2.1.322/S5/otlp-traces-sanitized.json`](../contracts/fact-export-v1/captures/g0a-20261112/claude-2.1.322/S5/otlp-traces-sanitized.json) | 2 | `agent.stop` | `Stop` | `agent.stop` | `"claude-code"` | `"google"` | `"google"` | *(absent)* | *(absent)* |

---

## 3. End-to-End Mechanism & Code Trace

### Step 1: Hook Registration Sets `--client claude`
When `agent-otel-bridge install-hooks` configures Claude Code (`~/.claude/settings.json`), [`crates/agent-otel-cli/src/hooks/command.rs`](../../crates/agent-otel-cli/src/hooks/command.rs#L252) emits hook commands of the form:
```text
"/path/to/agent-hook" PreToolUse --client claude
```

### Step 2: `agent-hook` Encodes `WireHeader.client_id = 2`
In [`crates/agent-otel-client/src/main.rs`](../../crates/agent-otel-client/src/main.rs#L127-L132), `resolve_client_id` parses `--client claude` via `ClientKind::from_str_name("claude")` ([`crates/agent-otel-core/src/model.rs`](../../crates/agent-otel-core/src/model.rs#L211-L238)) to `ClientKind::ClaudeCode` (`to_wire() == 2`).
`agent-hook` sends the 3-byte `WireHeader { version: 1, client_id: 2, event_id }` followed by the raw `stdin` JSON bytes without parsing or modifying the JSON payload.

Every harness-originated frame in [`claude-2.1.322/S1..S5/hook-sanitized.jsonl`](../contracts/fact-export-v1/captures/g0a-20261112/claude-2.1.322/S5/hook-sanitized.jsonl) confirms this wire header:
```json
{
  "header": {"client_id": 2, "event_id": 3, "version": 1},
  "msg_type": 3,
  "origin": "harness",
  "payload": {
    "cwd": "PATH_e79ac6f0cf8bf601",
    "hook_event_name": "PreToolUse",
    "permission_mode": "<str:len=17>",
    "session_id": "CONV_820267f0fb8c34dd",
    "tool_input": {"command": "<str:len=8>", "description": "<str:len=28>"},
    "tool_name": "Bash",
    "tool_use_id": "CALL_aa5a5943c17ccbb4",
    "transcript_path": "PATH_575cc059f0f9d4a9"
  }
}
```

### Step 3: Claude Code `2.1.322` Hook `stdin` Omits `model`
Real Claude Code `2.1.322` hook `stdin` payloads for `PreToolUse`, `PostToolUse`, and `Stop` contain only `{cwd, hook_event_name, permission_mode, session_id, tool_input, tool_name, tool_response, tool_use_id, transcript_path, stop_hook_active, last_assistant_message}`.
Unlike the native session transcript (`session-log-sanitized.jsonl`, where `assistant` records include `message.model`), Claude Code's hook `stdin` payload **never includes `"model"`, `"model_name"`, or `"modelName"`**.
- Note: `"model"` is an allowlisted key name in the G0a sanitizer (it passes through unredacted in `codex-0.167.0/S1..S5/session-log-sanitized.jsonl` and `claude-2.1.322/S1..S5/session-log-sanitized.jsonl`), and `capture-matrix.json` records `fields_present` before value redaction. `"model"` is genuinely absent from Claude Code `2.1.322` hook `stdin`.

Consequently, `AgentHookInput::parse_slice(bytes)` deserializes:
- `input.model = None`
- `input.agent_name = None`

### Step 4: Daemon Wire Transform Previously Populated `input.agent_name` Only
In [`crates/agent-otel-daemon/src/transform.rs`](../../crates/agent-otel-daemon/src/transform.rs#L124-L128), the daemon previously ran:
```rust
if input.agent_name.is_none() {
    input.agent_name = ClientKind::from_wire(header.client_id)
        .as_str()
        .map(str::to_owned);
}
```
For `header.client_id == 2`, `ClientKind::ClaudeCode.as_str()` set `input.agent_name = Some("claude-code".to_string())` when `agent_name` was absent (and discarded `header.client_id` altogether when the payload supplied a subagent `agent_name` such as `"planner"`). `input.model` remained `None`.

### Step 5: `build_span_from_resolved` Unconditionally Defaulted Missing `model` to `"google"`
In [`crates/agent-otel-core/src/otlp.rs`](../../crates/agent-otel-core/src/otlp.rs#L296-L358), `build_span_from_resolved` previously ran:
```rust
let inferred_provider = input.model.as_deref().map(infer_provider);

let agent_name = input
    .agent_name
    .as_deref()
    .unwrap_or_else(|| infer_agent_name(input.model.as_deref(), None));
// ...
if gen_ai_latest_experimental {
    if let Some(provider) =
        inferred_provider.filter(|provider| *provider != GEN_AI_PROVIDER_UNKNOWN)
    {
        attributes.push(kv_string(GEN_AI_PROVIDER_NAME, provider));
    }
} else {
    let provider = inferred_provider.unwrap_or(GEN_AI_PROVIDER_GOOGLE);
    attributes.push(kv_string(GEN_AI_PROVIDER_NAME, provider));
    attributes.push(kv_string(GEN_AI_SYSTEM, provider));
}
```
Because `inferred_provider` only inspected `input.model` and never consulted the wire header `ClientKind` or `input.agent_name`:
1. When `input.model` was `None`, `inferred_provider` was `None`.
2. In default stability mode (`gen_ai_latest_experimental == false`), `inferred_provider.unwrap_or(GEN_AI_PROVIDER_GOOGLE)` evaluated to `"google"`.
3. The span was emitted with `gen_ai.agent.name = "claude-code"`, `gen_ai.provider.name = "google"`, and `gen_ai.system = "google"`.

### Historical Provenance & Why Existing Tests Missed It
- This fallback dates back to `v0.1` (`26b8ce0` / `f9f456a`), when `agent-otel-bridge` only supported Antigravity (`agy`).
- The existing unit test `test_claude_code_payload_parsing` ([`crates/agent-otel-core/tests/core_tests.rs`](../../crates/agent-otel-core/tests/core_tests.rs#L153-L197)) did not catch this bug because its synthetic JSON fixture manually included `"model": "claude-3-5-sonnet-20241022"`.
- The exact same defect is visible in earlier M2 captures in this repository whenever hook `stdin` lacked `"model"`:
  - [`docs/production-refinement/evidence/M2/real-cli/claude-spans.json`](../production-refinement/evidence/M2/real-cli/claude-spans.json#L25-L45) (`gen_ai.agent.name = "claude-code"`, `gen_ai.provider.name = "google"`)
  - [`docs/production-refinement/evidence/M2/real-cli/grok-spans.json`](../production-refinement/evidence/M2/real-cli/grok-spans.json#L25-L45) (`gen_ai.agent.name = "grok"`, `gen_ai.provider.name = "google"`)

---

## 4. Minimal Fix & Deterministic Fixture-Backed Reproduction

### 4.1 Fix Summary
1. Add `ClientKind::default_provider(&self) -> Option<&'static str>` in [`crates/agent-otel-core/src/model.rs`](../../crates/agent-otel-core/src/model.rs#L200-L209) and implement `infer_provider_from_agent(agent_name: &str) -> Option<&'static str>` in [`crates/agent-otel-core/src/semconv.rs`](../../crates/agent-otel-core/src/semconv.rs#L186-L193) via `ClientKind::from_str_name(agent_name).default_provider()` (using zero-allocation `eq_ignore_ascii_case`):
   - `ClientKind::Antigravity` (`"antigravity" | "agy" | "gemini"`) $\rightarrow$ `Some(GEN_AI_PROVIDER_GOOGLE)` (`"google"`)
   - `ClientKind::ClaudeCode` (`"claude" | "claude-code" | "claudecode"`) $\rightarrow$ `Some(GEN_AI_PROVIDER_ANTHROPIC)` (`"anthropic"`)
   - `ClientKind::Codex` (`"codex" | "codex-cli" | "openai"`) $\rightarrow$ `Some(GEN_AI_PROVIDER_OPENAI)` (`"openai"`)
   - `ClientKind::Grok` (`"grok" | "grok-cli" | "xai"`) $\rightarrow$ `Some(GEN_AI_PROVIDER_XAI)` (`"xai"`)
   - `ClientKind::Pi` (`"pi" | "pi-cli"`) $\rightarrow$ `Some(GEN_AI_PROVIDER_PI)` (`"pi"`, documented as a harness default provider rather than a verified model vendor)
   - `ClientKind::Unspecified` / any unknown agent $\rightarrow$ `None`
2. Preserve wire-header harness identity independently of payload `agent_name` via `#[serde(skip)] pub harness_provider: Option<&'static str>` on `AgentHookInput` ([`crates/agent-otel-core/src/model.rs`](../../crates/agent-otel-core/src/model.rs#L501-L504)), populated from `ClientKind::from_wire(header.client_id).default_provider()` in [`crates/agent-otel-daemon/src/transform.rs`](../../crates/agent-otel-daemon/src/transform.rs#L124-L128).
3. In [`crates/agent-otel-core/src/otlp.rs`](../../crates/agent-otel-core/src/otlp.rs#L347-L355) (`build_span_from_resolved`, default path `gen_ai_latest_experimental == false`), resolve provider with precedence **explicit `model` $\rightarrow$ header `ClientKind` default provider $\rightarrow$ payload `agent_name` inference $\rightarrow$ `"google"` fallback**:
   ```rust
   let provider = inferred_provider
       .or(input.harness_provider)
       .or_else(|| {
           input
               .agent_name
               .as_deref()
               .and_then(infer_provider_from_agent)
       })
       .unwrap_or(GEN_AI_PROVIDER_GOOGLE);
   ```
4. **Preserved Contracts**:
   - **Explicit `input.model` takes precedence**: If a harness supplies `input.model` (for example, `antigravity` hosting a Claude model with `"model": "claude-..."`), `inferred_provider` is `Some("anthropic")` and takes precedence over `input.harness_provider` and `input.agent_name`.
   - **Subagent payload `agent_name` does not clobber header harness provider**: When `header.client_id == 2` (`ClaudeCode`) and the payload sets `agent_name = "planner"` without `model`, `gen_ai.agent.name` remains `"planner"` while `input.harness_provider` attributes `gen_ai.provider.name = "anthropic"` and `gen_ai.system = "anthropic"`.
   - **Experimental 1.41.0 opt-in unchanged**: When `OTEL_SEMCONV_STABILITY_OPT_IN=gen_ai_latest_experimental` is enabled (`gen_ai_latest_experimental == true`), `gen_ai.provider.name` continues to be emitted **only** when `input.model` is present and recognized, preserving the documented contract in [`docs/CONFIGURATION.md`](../CONFIGURATION.md#L64) and [`crates/agent-otel-core/tests/semconv_opt_in.rs`](../../crates/agent-otel-core/tests/semconv_opt_in.rs#L100-L116).
   - **Legacy unknown/empty input fallback unchanged**: When `input.model`, `input.harness_provider`, and `infer_provider_from_agent` all resolve to `None`, the default path still falls back to `GEN_AI_PROVIDER_GOOGLE`.

### 4.2 Deterministic Fixture Tests
The regression is locked in by deterministic tests in:
- [`crates/agent-otel-core/tests/semconv_opt_in.rs`](../../crates/agent-otel-core/tests/semconv_opt_in.rs#L363-L462) (`g0a_claude_code_2_1_322_hook_without_model_infers_anthropic_provider`)
- [`crates/agent-otel-daemon/src/transform.rs`](../../crates/agent-otel-daemon/src/transform.rs#L418-L489) (`production_transform_g0a_claude_2_1_322_wire_frame_attributes_anthropic_provider` and `production_transform_header_client_kind_attributes_provider_when_payload_sets_subagent_name`)

These tests replay the exact sanitized `claude-2.1.322/S5/hook-sanitized.jsonl` frame (`WireHeader::new(2, HookEvent::PreToolUse.to_wire())` with the 8-key Claude Code `2.1.322` `PreToolUse` JSON payload lacking `"model"`) as well as subagent payload overrides (`agent_name: "planner"` with `client_id = 2`):
- **Before fix**: `gen_ai.provider.name = "google"`, `gen_ai.system = "google"`.
- **After fix**: `gen_ai.provider.name = "anthropic"`, `gen_ai.system = "anthropic"`.

---

## 5. Decision for Contract Package `C1a` (`contract-identity`)

### Can `C1a` proceed on the existing G0a evidence?
**YES.** `C1a` (`contract-identity`: identity and keys) is **unblocked** and can proceed immediately on the existing 52 sanitized capture files in `docs/g0a-sanitized-captures` (`aa71997`) without re-running raw harness captures, subject to the following four explicit rules:

1. **Never Derive `canonical_harness` from `gen_ai.provider.name` or `gen_ai.system`**:
   - In `C1a`, `agent.session.link_id = hex(HMAC-SHA256(K_link, LP("agent.session.link/v1", org, endpoint_scope, canonical_harness, native_session)))` and `agent.tool_call.join_key` MUST derive `canonical_harness` strictly from the harness identity surface (`WireHeader.client_id` / `ClientKind` on the IPC wire, `gen_ai.agent.name` on OTLP spans, or the native session log source), normalized through the closed canonical harness vocabulary reconciling Finding `F12` (`"claude-code"` $\leftrightarrow$ `"claude"`, `"codex"` $\leftrightarrow$ `"codex"`).
   - `gen_ai.provider.name` / `gen_ai.system` represent the model vendor/provider (which may differ from the harness when a harness routes to a foreign model, or when pre-fix spans hit the missing-model fallback) and MUST NOT participate in session link ID or tool-call join key derivation.
2. **Treat `"google"` in G0a `claude-2.1.322/S1..S5/otlp-traces-sanitized.json` as a Known Pre-Fix Bridge Artifact**:
   - Contract tests and fixture validators consuming `g0a-20261112/claude-2.1.322/S1..S5/otlp-traces-sanitized.json` must treat `gen_ai.provider.name = "google"` and `gen_ai.system = "google"` on those 14 historical spans as the documented pre-`#29` fallback caused by `input.model == None` in `build_span_from_resolved`.
   - Authoritative per-turn model attribution for Claude Code `2.1.322` resides in `session-log-sanitized.jsonl` (`assistant.message.model`), joinable to hook/OTLP spans via `session_id` (`CONV_...`) and `tool_use_id` (`CALL_...`).
3. **Filter `"origin": "synthetic"` Frames When Validating Harness Identity**:
   - In `codex-0.167.0/S2/hook-sanitized.jsonl` lines 1–4, the synthetic collision frames (`"origin": "synthetic"`, `"synthetic_kind": "collision"`) carry `header.client_id = 2` (`ClaudeCode`) because the collision test injector reused client ID `2`. Contract consumers must filter on `"origin": "harness"` when asserting native harness wire headers.
4. **Scope `otlp-metrics-sanitized.json` as Bridge Daemon Self-Metrics**:
   - All 10 `{claude-2.1.322,codex-0.167.0}/S1..S5/otlp-metrics-sanitized.json` files contain `agent-otel-bridge` daemon internal health metrics (`agent.bridge.*`, `telemetry.sdk.name = "agent-otel-bridge"`), not harness-native OTLP metric exports. `C1a` must not expect harness or provider identity attributes on those metric data points.
