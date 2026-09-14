# Performance validation coordination

## Next implementation phase

The [implementation specification](performance-implementation-spec.md) and [execution plan](performance-implementation-plan.md) govern the next candidate changes. Per the latest user request, Codex coordinates specification and implementation subagents, using fixed Luna/Sol assignments. Antigravity remains coordinator of native fleet execution, with Codex QA/relay. The original campaign protocol below records the completed suite-update phase; it does not authorize active installation or automatically start another campaign. Planned APIs and flags are not yet implemented.

Antigravity is the primary implementation and execution coordinator for this campaign; Codex independently reviews methodology, diffs, and evidence and sends correction requests. Use Gemini 3.8 Flash Low initially; escalate to Flash Medium for concrete unresolved review defects. No Claude or silent model substitution. Deterministic measurements need no model. Coordinator turns are distinct from the fleet's maximum five live scenario attempts.

## Authorized scope

Update the development test suite and execute performance validation on the current Windows machine. Do not install, promote, restart the active daemon, change global hooks/settings, commit, push, or publish. Preserve unrelated work. Read AGENTS.md and docs/local-runtime-contract.md. Runtime output and captures are transient; persist test code, methodology and general lessons only. Use portable Rust/Cargo or Python tooling, no .ps1 scripts. Transport adapters may isolate OS-specific APIs. Linux and macOS native measurements are explicitly out of scope for this machine and must be reported as not_measured, never passed by inference.

The user supplied reference revision a27470c, installed as 0.5.1-a27470c, and historical parser result 37231.6 spans/s. Verify actual revision, installed manifest and binary hashes without changing the installation. Do not describe a seven-character Git revision as a W3C trace ID. Record the exact candidate source revision/dirty state, binary hashes, build profile and environment for each new measurement; distinguish candidate vs active installation evidence.

## Required measurements and assertions

- Parser: strictly greater than 50000 spans/s. Measure reproducibly with warmup, sample counts and multiple runs; state exactly what parsing/span construction includes. Cover legacy 0x01 and new context envelope 0x04, including decode cost in a separately named measurement. Investigate the historical 37231.6 result without inventing a cause.
- Hook internal execution: strictly below 1000 microseconds. External process creation/stdin/exit timing must be separately labeled and cannot satisfy this internal SLA. If exact internal execution cannot be observed without production instrumentation, return not_measured with the missing observation, never substitute an in-process proxy as a PASS.
- Context harvesting: strictly below 150 microseconds; define percentile and workspace fixtures (Git directory/worktree/no Git), include real filesystem harvesting and report warm/cold cache limitations.
- Hook size: strictly below 300 KB, document bytes convention and report exact bytes. IPC p99: strictly below 3000 microseconds. Include legacy and 0x04 paths where meaningful. Do not weaken strict thresholds or allow absent/empty samples to pass.
- Concurrent event delivery: investigate the previously observed one-event loss with debug daemon vs successful release. Use owned isolated daemon/pipe/collector, stable unique event IDs, expected/received/missing/duplicate IDs, concurrency, repeat count, deadlines, exit/fail-open evidence. Compare debug and release; do not count exit 0 as delivery. Clean up only processes/files owned by the probe. No active runtime mutation.
- Environment: OS/version/build, architecture, CPU, cores, Rust/toolchain, timestamp, source revision, profile, sample/warmup counts, binary hashes, transport and concurrent system-load caveats. All results have passed/failed/not_measured and normative requirement + implementation reference. Overall cannot pass with a failed or required unmeasured assertion.

## Execution protocol

1. Inspect source and tests; report the intended implementation, then implement bounded dev-only validation and regression tests. Avoid hot-path product modifications in this round; report causal product defects for review.
2. Own all Cargo execution while working; Codex will not concurrently invoke Cargo. Run meaningful regression tests, then fmt, workspace clippy with -D warnings, workspace tests/docs and guardrails. Existing guardrail size verdict is insufficient for the stricter 300 KB requirement.
3. Measure native Windows candidate binaries with reproducible commands and bounded repetition (at most five diagnostic attempts per scenario, no blind retries). Do not perform paid fleet calls yet. Do not submit benchmark results externally or invoke --submit.
4. Output concise structured results with commands, metrics, counts, classifications, exact binary/source/environment identity, and any blocker. Keep large raw outputs outside Git. For telemetry visibility use SigNoz MCP/direct API only, never the screen; Codex can perform MCP QA if unavailable in the coordinator.
5. Stop after this implementation/measurement batch for Codex quality review. Return the conversation ID for follow-up. Codex will request focused corrections in the same conversation, and validate the final state independently.

If installed hooks prevent every coordinator tool invocation, do not mutate or disable those hooks. Resume the same Antigravity conversation without tools, relay the needed source and evidence, and ask for structured files/commands/analysis. Codex reviews proposals before applying them, executes deterministic commands as the coordinator's relay, and sends findings back. Direct autonomous tool execution remains unavailable until the hook problem is resolved in a separately authorized installation round. Report this execution mode explicitly.
