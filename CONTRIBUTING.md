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
    branch feat/v0.5-platform-architecture
    checkout feat/v0.5-platform-architecture
    commit id: "platform contracts"
    commit id: "3-byte wire protocol"
    checkout main
    merge feat/v0.5-platform-architecture id: "PR #15 Merge"
    commit id: "v0.5.0" tag: "v0.5.0"
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
- [x] Platform contract conformance (static & dynamic) (`cargo test -p agent-otel-core --test platform_conformance`)
- [x] Documentation generation (`cargo doc --workspace --no-deps`)
- [x] Client binary size threshold (< 350 KB SLA, observed ~145 KB)
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

## 4. How to Add Support for a New Agent Client Harness (e.g. "Hermes")

Adding telemetry support for a new AI coding agent harness (e.g., Hermes, Cursor CLI, Aider, OpenCode) is designed to be declarative, modular, and contract-driven via the **Platform Provider & Descriptor Pattern**.

Once implemented, your new harness automatically inherits:
- Machine-adaptive quota monitoring (tracks your agent *only* when installed on the workstation)
- `agent-otel-bridge hooks install --client <name>` and `hooks uninstall`
- `agent-otel-bridge hooks status`
- `agent-otel-bridge hooks sync` (workspace hook alignment)
- `agent-otel-bridge hooks scan-all` (automatic workspace discovery via `workspace_markers`)
- Automated Static & Dynamic Conformance Testing

---

### Step 1: Core Descriptor (`crates/agent-otel-core/src/platform.rs`)
Implement `PlatformDescriptor` and register it in `BUILTIN_PLATFORMS`:

```rust
pub struct HermesDescriptor;

impl PlatformDescriptor for HermesDescriptor {
    fn id(&self) -> &'static str { "hermes" }
    fn display_name(&self) -> &'static str { "Hermes AI Agent" }
    fn aliases(&self) -> &'static [&'static str] { &["hermes-cli", "nous-hermes"] }
    fn wire_client_id(&self) -> u8 { 6 } // Must be unique in 1..=15
    fn pre_tool_response(&self) -> HookResponse { HookResponse::AllowJson }
}
```

*Invariants Enforced by `PlatformStaticValidator`:*
- `id` must be non-empty, lowercase ASCII alphanumeric (`[a-z0-9_-]`).
- `wire_client_id` must be strictly between `1` and `15` (occupies upper 4 bits of binary wire tag; `0` is reserved for Unspecified).
- Primary IDs, wire IDs, and aliases must have zero collisions across platforms.

---

### Step 2: Quota Provider (`crates/agent-otel-daemon/src/platforms/`)
Create `crates/agent-otel-daemon/src/platforms/hermes.rs` and register it in `BUILTIN_PROVIDERS`:

```rust
use std::path::Path;
use agent_otel_core::platform::PlatformDescriptor;
use agent_otel_core::quota::QuotaSnapshot;
use crate::platform::PlatformQuotaProvider;

pub struct HermesQuotaProvider;

impl PlatformDescriptor for HermesQuotaProvider {
    fn id(&self) -> &'static str { "hermes" }
    fn display_name(&self) -> &'static str { "Hermes AI Agent" }
    fn aliases(&self) -> &'static [&'static str] { &["hermes-cli", "nous-hermes"] }
    fn wire_client_id(&self) -> u8 { 6 }
}

impl PlatformQuotaProvider for HermesQuotaProvider {
    fn is_installed(&self, home: &Path) -> bool {
        home.join(".hermes").exists() || which::which("hermes").is_ok()
    }

    fn harvest_quota(&self, home: &Path) -> Option<QuotaSnapshot> {
        let p = home.join(".hermes").join("quota.json");
        if p.exists() {
            crate::quota::read_quota_file(&p)
        } else {
            None
        }
    }

    fn fallback_baseline(&self) -> QuotaSnapshot {
        QuotaSnapshot {
            remaining_fraction: 1.0,
            seconds_to_reset: 3600.0,
            observed_at_unix_nano: crate::quota::current_unix_nano(),
            bucket: "hermes".to_string(),
            group: "nous".to_string(),
        }
    }

    fn burn_per_token(&self) -> f64 {
        1.0 / 10_000_000.0
    }
}
```

*Invariants Enforced by `PlatformDynamicValidator`:*
- `remaining_fraction` must be a finite float in `[0.0, 1.0]`.
- `seconds_to_reset` must be $\ge 0.0$.
- Probes must execute in $< 150\ \mu\text{s}$ without spawning external child processes.

---

### Step 3: Compile-Time Client Dispatch (`crates/agent-otel-client/src/main.rs`)
Add your platform to `CLIENT_MAPPINGS` in `agent-hook`:

```rust
ClientMapping {
    aliases: &["hermes", "hermes-cli", "nous-hermes"],
    wire_id: 6,
    allow_pre_tool: true,
},
```

---

### Step 4: CLI Adapter & Scanner Registration (`crates/agent-otel-cli/src/hooks.rs`)
Add your adapter to `CLIENT_ADAPTERS`:

```rust
ClientAdapter {
    id: "hermes",
    display_name: "Hermes AI Agent",
    aliases: &["hermes-cli", "nous-hermes"],
    client_tag: Some("hermes"),
    scope: HookScope::GlobalOnly,
    workspace_markers: &[".hermes", "hermes.json"],
    global_config_fn: || get_hermes_config_path(false),
    project_config_fn: get_hermes_project_path,
    install_fn: install_standard_hooks,
    uninstall_fn: uninstall_standard_hooks,
    is_registered_fn: is_hermes_registered,
},
```

---

### Step 5: Verify Conformance & Guardrails
Run the automated conformance test suite:
```powershell
# Validates all static and dynamic platform invariants
cargo test -p agent-otel-core --test platform_conformance

# Runs the complete automated guardrail verification
cargo guardrails
```

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
