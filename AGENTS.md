# AGENTS.md: Architectural Invariants, Design Constraints, & Guidelines for AI Agents

> **Audience**: Autonomous AI coding agents (Google Antigravity, Claude Code, OpenAI Codex CLI, xAI Grok, Pi [pi.dev]), human core contributors, and automated reviewers.  
> **Repository**: `smota/agent-otel-bridge`  
> **Status**: Active & Enforced  

---

## 1. System Mission & Core Invariants

`agent-otel-bridge` is the high-performance OpenTelemetry observability bridge for AI CLI agent harnesses. Because it executes on the **synchronous hot-path of agent lifecycle hooks**, strict engineering constraints apply to every code modification.

### The Non-Negotiable Invariants
1. **Sub-Millisecond Client Execution**: `agent-hook.exe` MUST execute in **< 1.0 ms**. It is a tiny, static PE32 binary whose sole job is to grab `stdin`, write to the Win32 Named Pipe via Overlapped I/O, and exit with `code 0`.
2. **Fail-Open Backstop**: The client MUST NEVER block, freeze, or fail an AI agent turn. A dedicated 3ms OS watchdog thread terminates the client with `{}` and `exit 0` if pipe I/O stalls.
3. **Zero LLM Prompt Pollution**: Distributed tracing context MUST propagate via process environment variables (`$env:TRACEPARENT`). Agents and LLMs must **never** be instructed to append `--traceparent` CLI flags or pollute tool schemas with tracing plumbing.
4. **Non-Blocking Context Harvesting**: Workspace, project, and VCS context must be harvested in **< 150 µs** using direct filesystem stat/reads. **NEVER spawn external processes (like `git.exe` or `bash`) on the telemetry path.**
5. **Preference-Agnostic Tool Modeling**: Never hardcode developer-specific tool choices (`rtk`, `jq`, `bat`, `delta`) in the core engine. All commands are modeled as **Behavioral Execution Archetypes** (`FilterCompressor`, `StructuredParser`, `InspectorDiff`, `SearchRetrieval`, `BuildTestVerify`, `GenericExec`).
6. **Objective Telemetry Only in Core**: The Rust core engine measures and emits raw facts (`capability.schema_tokens`, `tool.tokens_saved_estimate`). Alert thresholds and policy decisions belong in downstream observability platforms (SigNoz, Prometheus).
7. **Zero Hot-Path Disk I/O**: `agent-hook` MUST NOT read configuration files from disk. Common tool mappings are compiled directly into the binary; custom overrides are cached in-memory by the long-running daemon at startup.

---

## 2. Performance SLAs & Hard Boundaries

All pull requests, features, and refactors MUST respect the following SLAs, continuously verified by `agent-otel-bench`:

| Boundary / Subsystem | Hard SLA Target | Benchmark Reference | Failure Consequence |
| :--- | :--- | :--- | :--- |
| **`agent-hook.exe` Binary Size** | **< 300 KB** | `profile.release.package.agent-otel-client` (opt-level "s", strip, lto) | Bloated process creation time on Windows |
| **Named Pipe Client RTT (p99)** | **< 3,000 µs** | Observed: `150 µs` (Win32 Overlapped) | Test failure in `ipc_tests` / `agent-otel-bench` |
| **ProtoJSON Parser Throughput** | **> 50,000 spans/s** | Observed: `62,943 spans/s` (Mean: `14.4 µs`) | Dropped spans under heavy agent loops |
| **Context Harvester Latency** | **< 150 µs** | Direct file read of `.git/HEAD` | Degraded agent turn latency |
| **Watchdog Deadline** | **3.0 ms** | Dedicated background thread | Watchdog fires, fails open cleanly |

---

## 3. Architecture & Code Guidelines

### 3.1 Crate Architecture & Separation of Concerns

```
crates/
├── agent-otel-core      # Domain models, SemConv constants, context harvester, archetype classifier, W3C trace_id.
├── agent-otel-ipc       # Wire framing, Win32 Overlapped client, Tokio Named Pipe server.
├── agent-otel-client    # Ultra-lean agent-hook binary (< 300 KB, zero external runtime deps).
├── agent-otel-daemon    # Background Tokio micro-batcher, OTLP Protobuf exporter, quota monitor.
├── agent-otel-cli       # Unified developer CLI: start, stop, doctor, hooks, install-hooks, emit-quota.
└── agent-otel-bench     # Microsecond hardware benchmark suite with JSON/Markdown reporting.
```

* **No Heavy Dependencies in Core or Client**: `agent-otel-core` and `agent-otel-client` MUST NOT depend on `tokio`, `reqwest`, `clap`, or regex engines. Keep dependencies minimal to guarantee instant compile times and sub-millisecond cold starts.
* **No `protoc` Requirement**: Protobuf serialization is handled via `prost` with pre-generated structures or minimal handwritten encoders. Contributors must not need `protoc` installed.

### 3.2 Semantic Convention Rules
* Adhere strictly to **CNCF OpenTelemetry GenAI & Agent Semantic Conventions v1.28+**:
  - `execute_tool {tool_name}` for tool executions.
  - `invoke_agent {agent_name}` for agent orchestrations.
  - `agent.hook.event`, `agent.step.index`, `agent.execution.mode`.
* Extended v0.3 attributes:
  - Context: `workspace.path`, `workspace.project_name`, `workspace.project_root`, `workspace.project_type`, `vcs.system`, `vcs.branch.name`.
  - Archetypes: `agent.tool.archetype`, `agent.tool.binary`, `agent.tool.pipeline_depth`.
  - Capabilities: `capability.kind`, `capability.namespace`, `capability.name`, `capability.schema_tokens`.
  - Lineage: `gen_ai.agent.depth`, `gen_ai.agent.parent_name`, `gen_ai.agent.root_id`, `gen_ai.agent.is_root`.
  - Error categories: `agent.error.category`.
* **Backward Compatibility**: Never rename or delete an existing attribute constant without providing a deprecated fallback alias.

### 3.3 Multi-Tier Execution Context & Safe VCS Probe
When harvesting context:
1. Probe `.git` as a directory or file (worktree pointer).
2. Read `.git/HEAD` directly:
   - If `ref: refs/heads/<name>`, extract branch name.
   - If 40-character hex, treat as detached HEAD commit.
3. Read `.git/config` for origin URL, **always sanitizing embedded credentials/tokens** (`https://token@...` $\rightarrow$ `https://...`).
4. If `.git` does not exist, set `vcs.system = "none"` with zero errors or logs.

### 3.4 Cross-Agent W3C Tracing Propagation
* Ingress: Evaluate in order:
  1. Explicit `traceparent` payload string.
  2. `$env:TRACEPARENT`.
  3. Deterministic fallback hash of conversation ID.
* Egress:
  - When spawning a subagent or running child commands, inject `$env:TRACEPARENT` with format:
    `00-{trace_id:32hex}-{current_span_id:16hex}-01`.
  - Child processes automatically adopt the parent's `trace_id` and attach under `parent_span_id`.

---

## 4. Quality & Verification Guardrails

Every pull request and modification MUST satisfy:

1. **Zero Clippy Warnings**:
   ```powershell
   cargo clippy --workspace --all-targets -- -D warnings
   ```
2. **Strict Code Formatting**:
   ```powershell
   cargo fmt --check
   ```
3. **100% Test Coverage**:
   ```powershell
   cargo test --workspace
   ```
4. **Documentation Invariants**:
   ```powershell
   cargo doc --workspace --no-deps
   ```
5. **Automated Guardrail Verification**:
   Run the dedicated cross-platform project guardrail check before committing:
   ```powershell
   # Cross-platform Cargo alias (Windows, Linux, macOS)
   cargo guardrails

   # Or via unified CLI binary
   agent-otel-bridge check-guardrails
   ```

---

## 5. Git Branching & Contribution Workflow

* **`main`**: Production trunk. Protected branch. Releases are tagged from `main` (`v0.1.0`, `v0.2.0`, `v0.3.0`). Direct pushes to `main` are restricted to maintainers for release merges.
* **Topic Branches**: All changes must be developed on descriptive feature branches:
  - `feat/<feature-name>`: New capabilities (e.g. `feat/tokenomics-dashboard`).
  - `fix/<issue-name>`: Bug fixes (e.g. `fix/worktree-detection`).
  - `docs/<topic>`: Documentation, benchmarks, or guides.
* **Pull Request Checklist**:
  - [ ] Code compiles cleanly with 0 warnings on Rust stable.
  - [ ] `cargo guardrails` passes with all green checks.
  - [ ] New semantic conventions are documented in `docs/TELEMETRY_DICTIONARY.md`.
  - [ ] Backward compatibility with existing dashboards is preserved.
