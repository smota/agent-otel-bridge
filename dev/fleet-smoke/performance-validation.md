# Performance Validation Methodology & Execution Guide

For the next candidate architecture, use the [implementation specification](performance-implementation-spec.md) and [execution plan](performance-implementation-plan.md). The commands below describe the existing suite; new controller/schema/timing capabilities are planned until their work items pass. Benchmark `--repeats` counts statistical repetitions and is distinct from the controller's maximum five attempts. No local collector result certifies SigNoz visibility.

## Overview

This document describes the Windows performance validation methodology, measurement boundaries, active-versus-candidate isolation, and coordinator relay protocol coordinated by Antigravity and reviewed by Codex.

## Active Installation vs Candidate Isolation

- **Zero Active Mutation**: Active installations in `%LOCALAPPDATA%\agent-otel-bridge` or `%USERPROFILE%\.gemini\antigravity-cli` remain completely untouched.
- **Read-Only Manifest Verification**: The active installation manifest (`%LOCALAPPDATA%\agent-otel-bridge\active.json`) is inspected strictly read-only. It verifies the active binaries in `bin/` against recorded SHA256 hashes and byte counts without modifying any active file.
- **Candidate Scope**: All benchmark builds, tests, and examples are compiled in the source tree under `target/`.

## Measurement Boundaries & Labels

1. **JSON + span construction baseline (`parser_pure_json_span_throughput`, retained report key)**:
   - Measures isolated JSON parse (`AntigravityHookInput::parse_slice`) and span construction (`build_span_from_hook`).
   - The span builder may call `harvest_user_email`, including environment/filesystem fallback. This is not a pure in-memory parser measurement. Environment, filesystem cache and background load may affect results; a lower rate alone cannot isolate the cause or establish envelope overhead. No cache eviction is performed.
   - Fixed historical payload shape (`stepIdx: 142`, command: `cargo build --release`, model: `gemini-2.5-pro`, fixture conversation: `performance-fixture`).
   - Normative Ref: `dev/fleet-smoke/performance-coordination.md#required-measurements-and-assertions`.
   - Strict SLA: `> 50,000` spans/s.
2. **Legacy Frame Decode (`legacy_frame_0x01_throughput`)**:
   - Measures 8-byte header decode (`decode_header`) + 3-byte `WireHeader` decoding + JSON parse + span construction.
   - Strict SLA: `> 50,000` spans/s.
3. **Context Envelope Decode (`envelope_0x04_throughput`)**:
   - Measures 8-byte header decode + context envelope decode (`decode_context_payload`) with valid W3C traceparent (`00-11223344556677889900aabbccddeeff-aabbccddeeff0011-01`) + context resolution via `resolve_trace_context` + span construction via `build_span_from_hook_with_context_opts`.
   - Strict SLA: `> 50,000` spans/s.
4. **Workspace Context Harvesting (`context_harvest_*_p99_us`)**:
   - Measured in isolated temporary directories using RAII `TempDir` across three explicit fixtures:
     - `git_repo`: `.git` directory with `HEAD` and `config`.
     - `git_worktree`: `.git` file pointing to worktree gitdir with `HEAD`, `commondir`, and `config`.
     - `no_git`: non-git directory containing a `Cargo.toml` project marker to prevent ancestor traversal.
   - Assertions verify `vcs_system` and `vcs_worktree`/`vcs_branch` before timing.
   - Strict SLA: p99 latency `< 150` Âµs.
5. **Candidate Hook Binary Size (`hook_binary_size`)**:
   - Candidate `agent-hook.exe` measured in bytes (daemon size is not subject to the hook size SLA).
   - Strict SLA: `< 307,200` bytes (300 KB).
6. **Independent native observations**:
   - `hook_internal_execution_duration_us`: Process startup/stdin/exit timings cannot substitute for internal SLA (< 1000 Âµs); requires dedicated internal stopwatch telemetry.
   - `performance_ipc` measures client send through receipt at a private receiver for both envelope types: 20 warmups and 1,000 timed samples each, with bounded receive and overall deadlines. It is a one-way application observation, matching the existing benchmark boundary; it is not a two-way acknowledgement. Any send/receive/identity failure prevents PASS.
   - `perf_delivery.py` compares isolated debug and release daemons with 24 events, concurrency 4, and three planned repetitions by default. An ephemeral local HTTP collector decodes real OTLP protobuf and checks expected trace IDs against received trace/span IDs, missing and duplicate IDs, collector errors, and hook responses. External process duration cannot satisfy the hook internal SLA. This capture does not assert SigNoz visibility.
   - `perf_campaign.py` preserves raw reports and composes effective assertions. Native results replace only matching missing observations from the microbenchmark report; hook internal timing remains not measured. Any failed required assertion makes the campaign failed; missing required observations prevent approval. A later successful repetition does not erase an earlier failure.
7. **Out-of-Scope Platforms**:
   - Linux and macOS native performance are out of scope for this Windows machine and tracked as `outside_required_scope: true`.

## Exact Commands for Codex Relay

```bash
# 1. Run Python regression test suite
python -m unittest dev/fleet-smoke/tests/test_perf_driver.py

# 2. Build release candidate packages and example
cargo build --release -p agent-otel-client -p agent-otel-bridge
cargo build --release -p agent-otel-fleet-smoke --example performance
cargo build --release -p agent-otel-fleet-smoke --example performance_ipc

# 3. Build debug daemon for delivery loss comparison
cargo build -p agent-otel-bridge

# 4. Execute bounded performance driver (repeats 3, timeout 60s, structured JSON stdout)
python dev/fleet-smoke/perf_driver.py \
  --example-bin target/release/examples/performance.exe \
  --hook-bin target/release/agent-hook.exe \
  --debug-daemon-bin target/debug/agent-otel-bridge.exe \
  --release-daemon-bin target/release/agent-otel-bridge.exe \
  --repeats 3 \
  --timeout 60
```

The repeatable combined entrypoint, from the repository root after these builds, is:

```text
python dev/fleet-smoke/perf_campaign.py --repeats 3
```

The commands above perform Windows measurements when run on Windows. The CI matrix executes only deterministic code/contract tests; it does not certify native performance on unavailable machines. `--compose-only` explicitly evaluates previously collected reports and does not constitute a new measurement. The component commands and composition were validated independently in this round; the newly combined full entrypoint was not rerun to consume additional measurement attempts.

Coordinator tools may fail before execution because of installed hook configuration. In that case Antigravity continues in the same conversation through structured proposals, with Codex relaying source/evidence and applying reviewed changes. Do not disable or edit global hooks to bypass that problem. Coordinator assertions about causality require independent evidence: a serial PASS following a concurrent loss suggests a hypothesis but does not prove where the event was lost.
