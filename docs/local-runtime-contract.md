# Local Runtime & Station Integration Contract

> **Scope**: Local Windows host deployment, multi-agent lifecycle hooks, binary resolution, and atomic runtime upgrades.  
> **Repository**: `smota/agent-otel-bridge`  
> **Status**: Enforced & Canonical  

---

## 1. Objectives & Non-Negotiable Contract

`agent-otel-bridge` operates on the **hot execution path** of agent harnesses (Antigravity CLI / IDE, Claude Code, OpenAI Codex CLI / Desktop, xAI Grok, and Pi).

Any failure, unhandled crash, or unresolved path directly degrades or breaks the user's interactive agent turns. To ensure absolute station stability, all local installations and future development iterations MUST satisfy the following invariants:

1. **Independent Execution Context**: Binaries executed by agent hooks MUST NOT depend on:
   - Interactive shell `$env:PATH` modifications.
   - User `~/.cargo/bin` or repository `target/` directories.
   - The current working directory (`cd`) or existence of source checkouts.
   - Interactive vs non-interactive session variables (e.g. S4U logon sessions such as `agy.exe remote-control serve`).
2. **Absolute Canonical Path Projection**: Agent hook configurations (`hooks.json`, `settings.json`) MUST use the absolute, canonical path to `%LOCALAPPDATA%\agent-otel-bridge\bin\agent-hook.exe`. Relative commands (like bare `agent-hook`) are strictly prohibited because Windows `cmd.exe /c` will fail with `'agent-hook' is not recognized` before the client process can execute its internal 3.0ms fail-open watchdog.
3. **Safe Command Escaping**: All generated command strings containing whitespace or special characters (such as spaces in user directories) MUST be properly double-quoted when written into JSON configurations:
   ```json
   "command": "\"C:\\Users\\Username\\AppData\\Local\\agent-otel-bridge\\bin\\agent-hook.exe\" PreToolUse"
   ```
4. **Preservation of Third-Party Tools**: Installing or updating bridge hooks MUST NEVER erase, duplicate, or alter third-party hooks (such as `herdr`, `rtk`, custom scripts). Bridge hook registrations are updated in-place by matching the executable name (`agent-hook.exe`).
5. **Atomic Deployment & Instant Rollback**: Active binaries MUST be staged in immutable version directories and copied to the canonical runtime path. Locked Windows binaries must be replaced using atomic rename-on-replace semantics. A full backup of the previous runtime is preserved for single-command rollback.
6. **Strict Separation of Candidate vs Active Testing**: `cargo test` and local development builds MUST NEVER modify or lock the host's active installation in `%LOCALAPPDATA%`. Verification of candidate builds occurs strictly within `target/`, and promotion to active is a deliberate action via `agent-otel-bridge local install`.

---

## 2. Directory Layout & Manifest Specification

All local runtime state resides under the user's Local AppData directory:

```
%LOCALAPPDATA%\agent-otel-bridge\
├── bin/                          # Canonical active executables (hot path)
│   ├── agent-hook.exe            # Ultra-lean hook client (< 300 KB)
│   └── agent-otel-bridge.exe     # Unified daemon and CLI binary
├── versions/                     # Immutable staged versions
│   └── <version_id>/             # e.g., "0.4.0-3019074-dirty"
│       ├── agent-hook.exe
│       ├── agent-otel-bridge.exe
│       └── manifest.json         # Build metadata & SHA-256 hashes
├── previous/                     # Backup of immediately prior active version
│   ├── agent-hook.exe
│   ├── agent-otel-bridge.exe
│   └── manifest.json
├── logs/                         # Background daemon and client logs
│   ├── daemon.log
│   └── bridge.log
├── active.json                   # Manifest of current active runtime
└── previous.json                 # Manifest copy for verification
```

### Manifest Schema (`active.json`)

```json
{
  "version_id": "0.4.0-3019074-dirty",
  "semver": "0.4.0",
  "git_commit": "3019074",
  "git_branch": "feat/local-runtime-contract",
  "git_dirty": true,
  "installed_at": "2026-09-13T06:40:00.000000+02:00",
  "binaries": {
    "agent-hook.exe": {
      "sha256": "4f5539d48bfa9a2727181059ec216259ec05ee9653a988d55ffcaef6bba46c59",
      "size_bytes": 148480
    },
    "agent-otel-bridge.exe": {
      "sha256": "a3bb408b6f3a3d5ea715494f107936a72e8fa44821a733735165aaeb8c8cebf7",
      "size_bytes": 4820992
    }
  }
}
```

---

## 3. Supported Client Integration Matrix

The local bridge manager (`agent-otel-bridge local`) automatically configures and maintains hooks for all target harnesses on Windows:

| Client / Harness | Config Path | Scope | Hook Mechanism |
| :--- | :--- | :--- | :--- |
| **Antigravity CLI / IDE** | `~/.gemini/config/hooks.json` | CLI, S4U Daemon, IDE | `PreToolUse`, `PostToolUse`, `SessionStart`, `SessionEnd`, `PrePrompt` |
| **Claude Code CLI / Desktop** | `~/.claude/settings.json` | CLI, Desktop App | `commands.PreToolUse`, `commands.PostToolUse` |
| **OpenAI Codex CLI / Desktop** | `~/.codex/hooks.json` | CLI, Desktop App | `PreToolUse`, `PostToolUse`, `SessionStart`, `SessionEnd` |
| **xAI Grok CLI** | `~/.grok/hooks/agent-otel.json` | CLI | Standalone bridge config (cleans legacy duplicate in `hooks.json`) |
| **Pi CLI** | `~/.pi/hooks.json` | CLI | `PreToolUse`, `PostToolUse`, `SessionStart`, `SessionEnd` |

### Third-Party Coexistence Rules
- Existing hooks such as `powershell -ExecutionPolicy Bypass -File ... herdr-agent-state.ps1` or `rtk hook claude` MUST be preserved verbatim.
- The installer detects existing bridge hooks via executable filename matching (`agent-hook.exe`). If found, it updates the command path in-place. If absent, it appends the bridge entry.

---

## 4. Local Lifecycle Management Commands

The unified CLI provides end-to-end local lifecycle management via `agent-otel-bridge local`:

### 1. View Current Status
Inspect the active installation, manifest, running daemon status, and candidate version:
```powershell
agent-otel-bridge local status
```

### 2. Build & Stage a New Local Release
Compile optimized release binaries and atomically activate them in `%LOCALAPPDATA%\agent-otel-bridge`:
```powershell
# Automatically compiles release candidate and installs
agent-otel-bridge local install

# Or install from pre-built directory:
agent-otel-bridge local install --from-build target\release
```
During installation, the command:
1. Verifies executable sizes and computes SHA-256 hashes.
2. Stages files into `versions/<version_id>/`.
3. Backs up the current active version into `previous/`.
4. Atomically replaces executables in `bin/` (handling Windows file locks via temporary rename).
5. Projects canonical double-quoted paths to `%LOCALAPPDATA%\agent-otel-bridge\bin\agent-hook.exe` into all supported client config files.
6. Writes updated `active.json`.

### 3. Immediate Rollback
If a regression or issue is detected in the active runtime:
```powershell
agent-otel-bridge local rollback
```
This restores the executables and manifest from `previous/` and ensures hooks point to the verified working installation.

### 4. Station Diagnostics
Verify all IPC pipes, binary paths, and hook registrations:
```powershell
agent-otel-bridge doctor
```

---

## 5. Developer & AI Agent Contribution Workflow

When modifying any of the following subsystems:
- Client (`agent-hook`, `crates/agent-otel-client`)
- Daemon (`agent-otel-daemon`)
- IPC transport & protocols (`crates/agent-otel-ipc`)
- Hook generators (`crates/agent-otel-cli/src/hooks.rs`)
- Local runtime manager (`crates/agent-otel-cli/src/local.rs`)
- Client adapters (Antigravity, Claude, Codex, Grok, Pi)

Follow this strict cycle:

```
[1. Candidate Development]  --> cargo check / cargo test --workspace
                                  (all tests run in target/, active installation untouched)
                                      ↓
[2. Quality Verification]   --> cargo fmt --check
                            --> cargo clippy --workspace --all-targets -- -D warnings
                            --> cargo guardrails
                                      ↓
[3. Local Stage & Activate] --> cargo build --release
                            --> target\release\agent-otel-bridge.exe local install --from-build target\release
                                      ↓
[4. Active Verification]    --> agent-otel-bridge local status
                            --> agent-otel-bridge doctor
                            --> Run synthetic hook invocation from bare PATH (cmd.exe /c "...")
```

This workflow guarantees that experimental code never breaks background agent daemons until full verification is complete.
