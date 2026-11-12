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
   Antigravity on Windows applies generic argv escaping before `cmd.exe`.
   Its adapter therefore uses OS-resolved PowerShell with `-EncodedCommand`:
   the UTF-16LE script contains `& "<absolute path>" <event>`, while the outer
   command has no quotes for the runner to corrupt. This adds PowerShell
   startup overhead outside the native client's execution budget. Other CMD
   consumers can use `call "<absolute path>" <event>` when they preserve quoting.
4. **Preservation of Third-Party Tools**: Installing or updating bridge hooks MUST NEVER erase, duplicate, or alter third-party hooks (such as `herdr`, `rtk`, custom scripts). Bridge hook registrations are updated in-place by matching the executable name (`agent-hook.exe`).
5. **Deployment Atomicity, Symlink Layout & Instant Rollback**: Active binaries MUST be staged in immutable version directories (`versions/<version_id>/`), re-verified by SHA-256 before final directory promotion, and activated without writing through existing symlinks or mutating staged version artifacts:
   - **Symlink Mode (POSIX, and Windows with Developer Mode or `SeCreateSymbolicLinkPrivilege`)**: `bin/agent-hook(.exe)` and `bin/agent-otel-bridge(.exe)` are file symlinks pointing directly to `../versions/<version_id>/<binary>` (replacing the earlier copy-based `bin/` layout and avoiding indirection through `current`). Replacing each file symlink stages a temporary sibling file symlink and renames it over the destination (`rename(2)` on Unix; `MoveFileExW(..., MOVEFILE_REPLACE_EXISTING)` on Windows). Because the file symlink reparse point is replaced rather than overwriting the mapped target executable in `versions/<old_id>/`, each `bin/*` switch is atomic with zero missing-entry window even while the daemon or hook executable is running. On Windows, `current -> versions/<version_id>` is a directory symlink (`FILE_ATTRIBUTE_DIRECTORY`), which `MoveFileExW` cannot overwrite in place; updating `current` renames the old directory symlink aside (`.current.old.*`) before moving the new link in. Because `bin/*` points directly to `../versions/<version_id>/*` and is updated before `current`, that brief gap on `current` never affects `bin/*` execution.
   - **Non-Atomic Fallback Boundaries (Copy Fallback & In-Use Regular Files on Windows)**:
     1. *Copy fallback without symlink privileges (Windows OS error 1314 `ERROR_PRIVILEGE_NOT_HELD`)*: When symlink creation is denied by the OS, `current` is omitted and `bin/*` uses regular file copies staged into temporary sibling files (re-hashed against the source SHA-256) and renamed into `bin/`. When the destination regular file in `bin/` is not running, `MoveFileExW(..., MOVEFILE_REPLACE_EXISTING)` replaces it atomically.
     2. *In-use regular-file executable on Windows (copy fallback and one-time migration from real `.exe` files in `bin/`)*: On the copy fallback path (and during the one-time migration from real `.exe` files in `bin/` to file symlinks), replacing a running `.exe` is **not gap-free**. When a destination entry in `bin/` is a regular file and that executable is currently running/mapped (such as `bin\agent-otel-bridge.exe` while the daemon is active, or `bin\agent-hook.exe` during a hook invocation), Windows rejects in-place overwrite (`ERROR_ACCESS_DENIED`). `replace_entry_with_staged` falls back to renaming the running file aside to `bin\.<name>.old.<pid>.<nanos>.<seq>` and then renaming the staged entry into `bin\`, creating a sub-millisecond window between the two renames during which a concurrent hook launch can fail. **Recommendation**: Stop the daemon (`agent-otel-bridge stop`) before activating on the copy fallback path (or before migrating a running legacy copy-based installation). While the old daemon process remains running, its old executable cannot be unlinked immediately on Windows and is left behind as `bin\.<name>.old.*` until a later `local install`, `local activate`, or `local rollback` sweep cleans it up after the process exits.
   - **Exclusive Runtime Lock & Scoped Temp Sweep**: `local install`, `local activate`, and `local rollback` acquire an exclusive lock file (`<runtime>/.runtime.lock` created via `create_new` and removed on drop; if a prior process crashed, the error message instructs the operator how to remove the stale lock file) before sweeping leftover temporary entries. Only the exact anchored temporary patterns generated by the runtime (`.rollback.stage.*`, `.previous.stage.*`, `.previous.bak.*`, `.<id>.stage.*`, `.<id>.rollback-stage.*`, `.*.old.*`, `.*.copy.*`, `.*.symlink.*`, `.*.write.*`, all ending in `.<pid>.<nanos>.<seq>`) are swept in `<runtime>` and `<runtime>/bin`, and the sweep never touches `<runtime>/versions/`.
   - **Transactional Backup & Rollback**: A full backup of the prior active runtime is staged before pointer updates and committed to `previous/` only when activation succeeds. Failed activations restore prior entries; if any restore step fails, the rollback backup directory is renamed to the non-swept prefix `.rollback.kept.<pid>.<nanos>.<seq>`, `"restore incomplete"` is reported alongside the activation error, and `agent-otel-bridge local status` lists all kept backups.
6. **Strict Separation of Candidate vs Active Testing**: `cargo test` and local development builds MUST NEVER modify or lock the host's active installation in `%LOCALAPPDATA%`. Verification of candidate builds occurs strictly within `target/`, staging via `agent-otel-bridge local install` leaves active state untouched (`--activate` defaults to `false`), and promotion to active is an explicit action via `agent-otel-bridge local install --activate`.

---

## 2. Directory Layout & Manifest Specification

All local runtime state resides under the user's Local AppData directory (changed from regular file copies in `bin/` to direct file symlinks `bin/<exe> -> ../versions/<version_id>/<exe>` wherever the OS permits symlink creation, falling back to staged regular file copies only on Windows OS error 1314):

```
%LOCALAPPDATA%\agent-otel-bridge\
├── bin/                          # Canonical active entrypoints (hot path)
│   ├── agent-hook.exe            # File symlink -> ..\versions\<version_id>\agent-hook.exe (or copy fallback on 1314)
│   └── agent-otel-bridge.exe     # File symlink -> ..\versions\<version_id>\agent-otel-bridge.exe (or copy fallback on 1314)
├── current -> versions/<id>      # Directory symlink to active version (omitted under 1314 copy fallback)
├── versions/                     # Immutable staged versions
│   └── <version_id>/             # Base "<semver>-<short_commit>[-dirty]" or digest-suffixed "<id>-<8hex>"
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
└── active.json                   # Manifest of current active runtime
```

### Version ID & `<id>-<8hex>` Digest Suffix Scheme

1. **Base Version ID**: Derived as `<semver>-<short_commit>` (or `<semver>-<short_commit>-dirty` when the working tree has uncommitted changes).
2. **Idempotent Re-Stage vs. `<id>-<8hex>` Disambiguation**:
   - If `versions/<base_id>/` does not exist, the build is staged at `versions/<base_id>/`.
   - If `versions/<base_id>/` already exists and its binaries match the candidate's SHA-256 hashes, staging is a no-op and re-activation loads and prints the existing on-disk `versions/<base_id>/manifest.json`.
   - If `versions/<base_id>/` already exists with different binary SHA-256 hashes (for example, subsequent dirty builds on the same commit, or when `local rollback` finds that `versions/<manifest.version_id>/` fails SHA-256 verification against `previous/manifest.json`), a deterministic suffixed version ID `<base_id>-<8hex>` is used, where `<8hex>` is the first 8 lowercase hex characters of `SHA256("<hook_sha256>:<bridge_sha256>")`. If `<base_id>-<8hex>` also already exists on disk with mismatched hashes, a fresh unused numbered suffix (`<base_id>-<8hex>-2`, `-3`, ...) is chosen so existing `versions/` directories are never deleted or modified.
3. **Manifest `version_id` Validation**: Every `version_id` read from `manifest.json` or passed to activation/rollback is strictly validated to be non-empty, contain only ASCII `[A-Za-z0-9._-]`, and never equal `.` or `..` or contain `..`, path separators, or absolute path prefixes.

### Manifest Schema (`active.json` and `versions/<version_id>/manifest.json`)

```json
{
  "version": "0.9.0",
  "version_id": "0.9.0-3019074-dirty-4f5539d4",
  "git_commit": "3019074abcdef1234567890abcdef1234567890",
  "git_branch": "feat/local-runtime-contract",
  "is_dirty": true,
  "built_at": "1726000000",
  "target_triple": "x86_64",
  "binaries": [
    {
      "filename": "agent-hook.exe",
      "sha256": "4f5539d48bfa9a2727181059ec216259ec05ee9653a988d55ffcaef6bba46c59",
      "size_bytes": 148480
    },
    {
      "filename": "agent-otel-bridge.exe",
      "sha256": "a3bb408b6f3a3d5ea715494f107936a72e8fa44821a733735165aaeb8c8cebf7",
      "size_bytes": 4820992
    }
  ]
}
```

---

## 3. Supported Client Integration Matrix

`agent-otel-bridge local install` stages candidate binaries into `versions/<version_id>/` by default (`--activate` defaults to `false`) and promotes them to the canonical active runtime only when `--activate` is passed. It does not rewrite harness configuration. `update_hooks` defaults to false for both `local install` and `local rollback`. Pass `--update-hooks` only to opt into the legacy JSON installer.

Enable and disable one application at a time:

```text
agent-otel-bridge local enable --app claude
agent-otel-bridge local enable --app codex
agent-otel-bridge local enable --app antigravity
agent-otel-bridge local enable --app grok
agent-otel-bridge local enable --app pi

agent-otel-bridge local disable --app <client>
```

`install-hooks` and `hooks install` without `--legacy-config` print `agent-otel-bridge local enable --app <client>` and exit 2. They write nothing. With `--legacy-config`, they run the previous JSON merger and preserve third-party hooks.

If `local enable` cannot reach the daemon, it starts the existing daemon. If `claude`, `codex`, or `agy` cannot be spawned, enable returns an error and does not write under that user's `.claude`, `.codex`, or `.gemini` directory. Plugin files may be staged under the bridge runtime directory `<runtime>/plugins-stage`.

Unknown application names are an error and write nothing.

| Surface | Enablement | Detected when | Registered when |
| :--- | :--- | :--- | :--- |
| **claude** | `claude plugin marketplace add` of a staged marketplace, then `claude plugin install agent-otel-bridge@agent-otel-bridge --scope user`. Hook commands come from `format_claude_hook_command`. | `~/.claude` exists | `settings.json` has a bridge command, or `enabledPlugins` mentions `agent-otel-bridge` |
| **codex** | Staged `.agents/plugins/marketplace.json` plus `.codex-plugin/plugin.json` and `hooks/hooks.json`, then `codex plugin marketplace add` and `codex plugin add agent-otel-bridge@agent-otel-bridge`. Disable uses `codex plugin remove agent-otel-bridge@agent-otel-bridge`. This path does not write `~/.codex/hooks.json` or Codex trust files. | `~/.codex` exists | `hooks.json` contains a bridge command, or `config.toml` mentions `agent-otel-bridge`. Doctor then says trust review remains with Codex. |
| **antigravity-cli** | `agy plugin install` of a staged plugin whose `hooks.json` uses the same object `install_antigravity_hooks` stores under `agent-otel-bridge`. Disable is `agy plugin uninstall agent-otel-bridge`. | `~/.gemini` exists | `~/.gemini/antigravity-cli/plugins/agent-otel-bridge/plugin.json` or `~/.gemini/config/plugins/agent-otel-bridge/plugin.json` exists |
| **antigravity-ide** | Not enabled by the CLI plugin command above. | `~/.gemini` exists | `~/.gemini/config/hooks.json` contains `agent-hook` or `agent-otel-bridge` |
| **grok** | `install_grok_hooks` / `uninstall_standard_hooks` on the owned `~/.grok/hooks/agent-otel.json` file only. No other Grok file is merged. | `~/.grok` exists | That owned file contains `agent-hook` |
| **pi** | `install_pi_hooks` writes only `~/.pi/agent/extensions/agent-otel-bridge.ts`. It does not create `hooks.json`. Uninstall deletes the owned extension and may still clean a legacy `hooks.json`. | `~/.pi` exists | The managed extension starts with `// Managed by agent-otel-bridge.` |

Doctor prints these rows in addition to the existing adapter lines. A missing home child is `not detected`, not an error. Desktop and IDE live sessions are not certified.

### Third-Party Coexistence Rules
- On Windows, Claude bridge commands use encoded PowerShell and return `{}`
  when imported by Grok (`GROK_WORKSPACE_ROOT` is present). Grok's dedicated
  bridge hooks emit its telemetry; imported third-party hooks remain enabled.
- Pi refuses to overwrite an unowned extension at the managed path. Uninstall removes only that owned extension, and may remove bridge entries from a legacy `hooks.json` without deleting unrelated extensions.
- `--legacy-config`, and the Grok owned file, preserve existing third-party hooks such as `powershell -ExecutionPolicy Bypass -File ... herdr-agent-state.ps1` or `rtk hook claude`.
- The legacy installer detects existing bridge hooks via executable filename matching (`agent-hook.exe`). If found, it updates the command path in-place. If absent, it appends the bridge entry.

---

## 4. Local Lifecycle Management Commands

The unified CLI provides end-to-end local lifecycle management via `agent-otel-bridge local`:

### 1. View Current Status
Inspect the active installation, manifest, `current` pointer consistency against `active.json`, any preserved `.rollback.kept.*` backups, running daemon status, and candidate version:
```powershell
agent-otel-bridge local status
```

### 2. Build, Stage, & Activate a New Local Release
Stage optimized release binaries in `versions/<version_id>/`, and pass `--activate` to atomically activate them in `%LOCALAPPDATA%\agent-otel-bridge`:
```powershell
# Compiles release candidate and stages only (leaves current, bin/, active.json, previous/, hooks, and daemon untouched):
agent-otel-bridge local install

# Stage from a pre-built directory without activating:
agent-otel-bridge local install --from-build target\release

# Explicitly stage and atomically activate as the current runtime:
agent-otel-bridge local install --activate --from-build target\release
```
During staging (`local install` without `--activate`), the command:
1. Acquires `<runtime>/.runtime.lock` and sweeps anchored leftover temporary entries in `<runtime>` and `<runtime>/bin` (never sweeping inside `versions/` or deleting `.rollback.kept.*` backups, and skipping any in-use `bin\.<name>.old.*` files).
2. Verifies executable sizes and computes SHA-256 hashes.
3. Copies files and `manifest.json` into a temporary staging directory, re-verifies SHA-256 hashes of the staged binaries, and renames the staging directory into immutable `versions/<version_id>/` (or loads the existing on-disk `manifest.json` when an identical build is already staged).
4. Leaves `current`, `bin/`, `active.json`, `previous/`, hooks, and running daemons untouched.

When `--activate` is specified, activation additionally:
1. Verifies the staged `versions/<version_id>/` binaries against `manifest.binaries[].sha256` and stages the prior active version into a temporary backup before mutating canonical pointers, committing `previous/` only after activation succeeds (and restoring prior entries on failure while renaming the backup to `.rollback.kept.*` if any restore step fails).
2. Updates `bin/` file symlinks directly to `../versions/<version_id>/<binary>` (atomic file-symlink rename on POSIX and Windows) and updates `current -> versions/<version_id>`, or falls back to re-hashed temporary sibling copies + rename when symlink privileges are unavailable (OS error 1314), never writing through existing symlinks or modifying `versions/`.
3. Does not rewrite harness configuration. Enable each application with `agent-otel-bridge local enable --app <client>`.
4. Atomically writes updated `active.json`.

### 3. Immediate Rollback
If a regression or issue is detected in the active runtime:
```powershell
agent-otel-bridge local rollback
```
This acquires `.runtime.lock`, validates `previous/manifest.json` (`version_id` restricted to `[A-Za-z0-9._-]` without `..`), and verifies `versions/<manifest.version_id>/` against `manifest.binaries[].sha256` before switching pointers. If `versions/<manifest.version_id>/` is missing, it stages and re-hashes from `previous/` into `versions/<manifest.version_id>/`; if `versions/<manifest.version_id>/` (or `<id>-<8hex>`) exists with mismatched hashes, it stages and re-hashes from `previous/` under a fresh unused suffix (`<id>-<8hex>`, `<id>-<8hex>-2`, ...) without deleting or modifying any existing `versions/` entry. Verification failure in `previous/` aborts before mutating `current` or `bin/`. It does not rewrite harness configuration unless `--update-hooks` is passed, which runs the legacy JSON installer.

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
                            --> target\release\agent-otel-bridge.exe local install --activate --from-build target\release
                                      ↓
[4. Active Verification]    --> agent-otel-bridge local status
                            --> agent-otel-bridge doctor
                            --> Run synthetic hook invocation from bare PATH (cmd.exe /c "...")
```

This workflow guarantees that experimental code never breaks background agent daemons until full verification is complete.

## 6. Codex Windows shell policy

Codex evaluates hook commands through the session shell, or a CMD fallback. The installer reads a bridge-owned file beside Codex's hooks.json: `agent-otel-bridge-policy.json`. It is not a Codex configuration key and the native client never reads it. The installer preserves this file across updates.

```json
{"version": 1, "windows_hook_shell": "powershell"}
```

- `powershell`: direct call operator and double-quoted canonical executable, using the existing PowerShell escaping. Choose only for Codex sessions whose hook runner uses PowerShell (Windows PowerShell or pwsh). This command is not compatible with a CMD fallback. It adds no nested shell process.
- `portable`: encoded Windows PowerShell launcher, compatible with the tested CMD and PowerShell outer shells. Also used when the policy file is absent. Its extra shell startup cost is unsuitable for the station's preferred hot path. The absolute SystemRoot must be a safe unquoted token; roots with spaces or metacharacters are rejected before writing.

Unknown versions, modes, fields, malformed JSON and read errors are rejected. The installer does not infer shell policy from its own environment or old command text. Non-Windows projection ignores this Windows-only policy. Changing the Codex session shell requires reviewing this choice; neither mode forces Codex to use a particular interpreter.

Codex projection detects encoded bridge registrations, updates owned commandWindows overrides, and refuses ambiguous ownership or malformed hooks without replacing the file. Failed hook registrations produce a nonzero CLI result; successful registrations for other clients remain applied and are reported as partial work. Changed definitions require Codex's normal trust review; installers must not write trust approval.

[Candidate measurements](production-refinement/evidence/M2/codex-candidate-02.md) separate shell startup from native runtime. Generate previews in an isolated home, review the diff against the active file, and use a scoped projection that preserves third-party hooks. Never replace the global file with a bridge-only sample. Active station validation and Desktop/IDE coverage remain separate acceptance steps.

## 7. Native Linux / WSL release profile (x86_64 GNU)

The production Linux hook retains the strict **below 300000 bytes** gate. Build it
with `bash scripts/build-linux-hook.sh`, after any ordinary Cargo release build.
The script checks the native target, runs `cargo rustc --release --locked` for
`agent-hook` only, explicitly sets `panic=abort`, and links the reviewed augment
script. It publishes the checked executable to `target/release/agent-hook` (or the
same release directory under `CARGO_TARGET_DIR`). No post-link binary editing is
used. The daemon, dependencies, Windows builds, and ordinary Cargo test harnesses
do not receive these linker arguments.

The linker script is copied into an immutable SHA-256-addressed build-cache path;
changing only its contents therefore changes Cargo's final link argument and
forces a relink. Linux local builds also select the native target explicitly and
copy its bridge output into the same conventional staging directory, including
when `CARGO_BUILD_TARGET` selects that native target.

This profile removes `.eh_frame` and `.eh_frame_hdr` at the final client link,
retains `.gcc_except_table`, RELRO, a non-executable stack, and separate writable
and executable segments. It requires no Rust/C exception unwinding,
`pthread_exit`, or `pthread_cancel` in the hook. Diagnostic Rust backtraces may
truncate; this is an explicit diagnostic limitation, not a claim that prebuilt
standard-library unwinder imports disappear. Linux uses an independent native
pthread watchdog and unbuffered inherited streams; thread/clock creation failure
exits zero without attempting a potentially blocked response.

Linux telemetry hooks write only `{}` and exit zero for every client and event.
They never emit approval or denial decisions, replacement tool inputs, blocking
instructions, or environment-injection output. If the watchdog cannot start or
its deadline expires, fail-open termination can omit the response. Incoming
`TRACEPARENT`, when supplied by the parent process, is still carried unchanged in
the native IPC context alongside the original event JSON. Linux stdout does not
set the harness's environment or propagate context into its future child tools;
that requires an independently supported parent/harness integration. This Linux
response contract does not change the existing non-Linux response path.

Native AGY rejected tools with the Linux bridge-owned `PreToolUse` registration
active, despite the client's exit-zero `{}` response; removing only that
registration restored native tool execution. An observational continuation
response contract has not been established. Linux AGY projections omit the
bridge-owned `PreToolUse` registration, including when replacing older registrations. AGY
retains `PostToolUse`, `PreInvocation`, `PostInvocation`, and `Stop` telemetry;
pre-tool telemetry is unavailable through this integration. Foreign policy hooks
remain intact and the bridge emits no approval or permission override. This
exception applies to native AGY's `.gemini/config/hooks.json` and staged plugin
hooks, not the separate Gemini CLI settings reader. Packaged ELF response checks
alone do not establish that a harness interprets a response as continuation;
native tool-execution acceptance is a separate check.

The versioned [Linux user-service template](../deploy/systemd/agent-otel-bridge.service)
consumes the required public `~/.config/observability/consumer.env` contract and
persists the shared runtime/socket paths under `~/.local/bin/.agent-otel-bridge`.
The public consumer contract must not redefine `AGENT_OTEL_HOME`,
`AGENT_OTEL_SOCKET`, or `OTEL_SERVICE_NAME`; those values belong to this service.
It clears only signal-specific endpoint/protocol overrides and exporter auth
headers; the consumer file's base endpoint/protocol remain authoritative. It
stores no backend credentials. Socket setup waits at most five seconds, refuses
symlinks and foreign ownership, and sets mode 0600 under a private mode-0700 run
directory. This lifecycle setup is separate from the native hook hot path.
The service restarts after unexpected exits, and SIGINT allows the native daemon
up to ten seconds to drain on stop. Installing artifacts, enabling this unit,
and applying per-HOME hooks are separate activation steps; source checkout
changes never implicitly activate them.

Linux listener startup and cleanup hold a nonblocking private `socket.lock`
lease for the listener's lifetime. Lease files persist and must not be removed
while a listener can run. After a crash, only an owned non-symlink socket whose
bounded connect probe returns connection-refused and whose inode still matches
may be removed and rebound. Live or ambiguous endpoints are preserved, and
shutdown will not unlink a replacement socket. This recovery is startup work;
it does not change the hook path, wire protocol, or non-Linux listener behavior.

Build verification uses Cargo, Bash, GNU core utilities/binutils (`readelf`), and
Python 3 from the station runtime. Run
`python3 scripts/check-linux-hook.py target/release/agent-hook` to exercise the
actual packaged ELF against isolated Unix receivers and blocked-stream fixtures.
Every subprocess has its own socket override; no live daemon is contacted.
`cargo guardrails` invokes both the native packager and these artifact checks on
Linux x86_64. `local install` builds this profile last, and `--from-build` refuses
oversized Linux hooks before creating installation state. Source checks,
packaged-artifact checks, and active installation checks remain separate.

Existing process-lifecycle benchmark timings include loader/process overhead;
they do not establish the internal main-entry-to-exit latency. Preserve this
measurement distinction in acceptance evidence. No Windows artifact equivalence
is claimed by Linux validation.
