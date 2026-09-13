# Contributing to agent-otel-bridge

Thank you for your interest in contributing to **`agent-otel-bridge`**! We welcome contributions from human engineers and autonomous AI agents alike.

Because `agent-otel-bridge` operates on the synchronous lifecycle hot-path of AI coding agents, our community maintains strict design, architectural, and performance guardrails.

---

## 1. Branching Policy & Release Cycles

To ensure production stability, predictable releases, and smooth community contributions, this repository enforces a structured Git branching strategy:

```mermaid
gitGraph
    commit id: "v0.2.0" tag: "v0.2.0"
    branch feat/context-harvester
    checkout feat/context-harvester
    commit id: "add context.rs"
    commit id: "add tests"
    checkout main
    merge feat/context-harvester id: "PR #12 Merge"
    branch fix/worktree-read
    checkout fix/worktree-read
    commit id: "fix gitdir read"
    merge fix/worktree-read id: "PR #13 Merge"
    commit id: "v0.3.0" tag: "v0.3.0"
    branch feat/v0.4-cross-platform
    checkout feat/v0.4-cross-platform
    commit id: "unix domain sockets"
    commit id: "native cli telemetry"
    checkout main
    merge feat/v0.4-cross-platform id: "PR #14 Merge"
    commit id: "v0.4.0" tag: "v0.4.0"
```

### Branch Roles
* **`main`**: The production trunk. Always stable, always releasable.
  - Direct commits to `main` are restricted.
  - Releases are tagged directly from `main` using Semantic Versioning (`vMAJOR.MINOR.PATCH`).
* **Feature Branches (`feat/<feature-name>`)**:
  - Used for developing new capabilities (e.g. `feat/tokenomics-dashboard`, `feat/archetype-parsers`).
* **Bugfix Branches (`fix/<issue-name>`)**:
  - Used for patching defects (e.g. `fix/pipe-timeout-windows`, `fix/url-sanitizer`).
* **Documentation & Benchmark Branches (`docs/<topic>`)**:
  - Used for doc improvements, benchmark submissions, and dashboard templates.

### Pull Request Workflow
1. Fork or branch from `main`.
2. Create your topic branch: `git checkout -b feat/my-new-feature`.
3. Implement changes following the [Design Constraints](#2-architectural-and-performance-guardrails).
4. Run the automated guardrails checker:
   ```powershell
   cargo guardrails
   ```
5. Commit with conventional commit messages (`feat: ...`, `fix: ...`, `docs: ...`, `perf: ...`).
6. Submit a Pull Request targeting `main`.

---

## 2. Architectural & Performance Guardrails

Before submitting a PR, ensure your contribution satisfies our core architectural invariants:

1. **Sub-Millisecond Client Execution SLA**:
   - `agent-hook` must execute in **< 1.0 ms** on standard hardware.
   - Hard watchdog fail-open backstop: 3.0 ms maximum duration.
2. **Zero LLM Prompt Pollution**:
   - Tracing context must propagate exclusively through the OS transport layer (`$env:TRACEPARENT` / `export TRACEPARENT`).
   - Never instruct agents or LLMs to pass `--traceparent` CLI flags.
3. **Preference-Agnostic Behavioral Archetypes**:
   - Do not hardcode specific CLI tools (`rtk`, `jq`, `bat`).
   - Map tools into the 6 standardized archetypes (`FilterCompressor`, `StructuredParser`, `InspectorDiff`, `SearchRetrieval`, `BuildTestVerify`, `GenericExec`).
4. **Non-Blocking Context Harvester**:
   - Direct filesystem read only. Never spawn `git.exe` or subprocesses inside the telemetry collection path.
5. **Zero Hot-Path Disk I/O**:
   - Defaults must remain compiled-in constants. Never read YAML/JSON files on the client hot path.

---

## 3. Automated Guardrails Verification

We provide an automated, cross-platform verification command built directly into the Rust workspace:

- [x] Code formatting (`cargo fmt --check`)
- [x] Linter purity with zero warnings (`cargo clippy --workspace --all-targets -- -D warnings`)
- [x] Full workspace test suite (`cargo test --workspace`)
- [x] Client binary size threshold (< 350 KB SLA)
- [x] Documentation generation (`cargo doc --workspace --no-deps`)
- [x] Git branch naming policy (`feat/*`, `fix/*`, `docs/*`, `perf/*`, `release/*`, `main`)

Run it before pushing:
```powershell
cargo guardrails
```

Or run via the unified CLI tool:
```powershell
agent-otel-bridge check-guardrails
```

---

## 4. How to Add Support for a New Agent Client Harness

Adding telemetry support for a new AI coding agent harness (e.g., Cursor CLI, Aider, OpenCodeInterpreter, Continue) is designed to be declarative, modular, and fast.

The entire harness integration lifecycle is managed through the `ClientAdapter` registry. Once registered, your new harness automatically inherits:
- `agent-otel-bridge hooks install --client <name>`
- `agent-otel-bridge hooks uninstall --client <name>`
- `agent-otel-bridge hooks status`
- `agent-otel-bridge hooks sync` (project-level hook alignment)
- `agent-otel-bridge hooks scan-all` (workstation-wide discovery & sync)
- `agent-otel-bridge doctor` (pipeline health checks)

### Step 1: Determine the Harness Configuration Scope
In `crates/agent-otel-cli/src/hooks.rs`, check how your harness resolves configuration:
- **`HookScope::GlobalOnly`**: The harness only reads from a global user config file (e.g. `~/.myagent/hooks.json`). Projects never shadow global telemetry.
- **`HookScope::NamespaceMerged`**: The harness natively merges namespaces between global and workspace configs (e.g. Google Antigravity).
- **`HookScope::ProjectShadowsGlobal`**: A project-level configuration file (e.g. `.myagent/settings.json`) completely replaces the global hooks list unless the bridge hook is present (e.g. Claude Code).

### Step 2: Implement Config Paths & Serialization
In [`crates/agent-otel-cli/src/hooks.rs`](crates/agent-otel-cli/src/hooks.rs):
1. Add configuration path resolvers:
   ```rust
   pub fn get_myagent_config_path(project: bool) -> Option<PathBuf> {
       if project {
           get_myagent_project_path(None)
       } else {
           std::env::var("USERPROFILE")
               .or_else(|_| std::env::var("HOME"))
               .ok()
               .map(|home| PathBuf::from(home).join(".myagent").join("hooks.json"))
       }
   }

   pub fn get_myagent_project_path(base: Option<&Path>) -> Option<PathBuf> {
       let root = base.unwrap_or_else(|| Path::new("."));
       Some(root.join(".myagent").join("hooks.json"))
   }
   ```
2. Implement install & uninstall functions (or reuse existing JSON helpers if your harness uses standard `{ "hooks": [...] }` or namespace formats):
   - **Crucial Invariant**: You MUST preserve all existing third-party hooks and formatting (non-destructive guarantee).

### Step 3: Register in the `CLIENT_ADAPTERS` Table
Add your adapter entry to `CLIENT_ADAPTERS` in [`crates/agent-otel-cli/src/hooks.rs`](crates/agent-otel-cli/src/hooks.rs):
```rust
ClientAdapter {
    id: "myagent",
    display_name: "MyAgent (myagent.ai)",
    aliases: &["myagent-cli", "my-agent"],
    client_tag: Some("myagent"),
    scope: HookScope::GlobalOnly, // or ProjectShadowsGlobal
    global_config_fn: || get_myagent_config_path(false),
    project_config_fn: get_myagent_project_path,
    install_fn: install_myagent_hooks,
    uninstall_fn: uninstall_myagent_hooks,
    is_registered_fn: is_myagent_hook_registered,
},
```
Also add your client identifier to `ClientTarget` enum in `hooks.rs` for CLI argument parsing.

### Step 4: Map the Fast-Path in `agent-hook`
In [`crates/agent-otel-client/src/main.rs`](crates/agent-otel-client/src/main.rs), map the `--client` CLI argument to a numeric client ID:
```rust
"myagent" | "myagent-cli" => 6,
```

### Step 5: Map OpenTelemetry Semantic Conventions
1. In [`crates/agent-otel-core/src/semconv.rs`](crates/agent-otel-core/src/semconv.rs):
   - Add provider constant (e.g. `pub const GEN_AI_PROVIDER_MYAGENT: &str = "myagent";`).
   - Update `infer_provider()` and `infer_agent_name()`.
2. In [`crates/agent-otel-core/src/model.rs`](crates/agent-otel-core/src/model.rs):
   - Add enum variant to `ClientKind::MyAgent`.
   - Update `ClientKind::from_str_name()`.

### Step 6: Add Tests & Verify Guardrails
1. Add unit tests in `hooks.rs` asserting:
   - Installation idempotency (running install twice does not duplicate hooks).
   - Third-party preservation (pre-existing hooks remain intact).
   - Clean uninstallation.
2. Run the automated guardrails checker:
   ```powershell
   cargo guardrails
   ```
   Ensure 100% pass: zero clippy warnings, code formatted, and binary size `< 350 KB`.

---

## 5. Enabling Local Git Hooks

To automatically prevent broken commits and branch policy violations, enable our repository Git hooks:

```powershell
git config core.hooksPath .githooks
```

Once enabled, `.githooks/pre-commit` and `.githooks/pre-push` will automatically enforce code formatting and branch naming conventions before any git commit or push.

---

## 6. Community Hardware Benchmarks

We actively track microsecond performance across diverse developer hardware. If you are testing on a new CPU/OS environment:

```powershell
# Run hardware benchmark and submit results
agent-otel-bridge benchmark --submit --open-browser
```

See [docs/COMMUNITY_BENCHMARKS.md](docs/COMMUNITY_BENCHMARKS.md) for current leaderboard results.

---

## 7. Sponsoring & Incubation

`agent-otel-bridge` is proudly sponsored and incubated by [Move the Needle](https://www.movetheneedle.info).
