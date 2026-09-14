# Stall round measurement plan

This controller repeats the existing deterministic `ipc_load` offer through a freshly owned candidate daemon and OTLP receiver. It reads the active installation and hook configuration only for before/after integrity snapshots. It does not activate, install, stop, or reconfigure the active bridge.

The default contractual workload is ten repetitions of four concurrency profiles (`1,4,8,16`). Each profile offers 15,000 events over 15 seconds at 1,000 events/s. At most three campaign attempts may be reserved, in order. A reserved attempt stays consumed after interruption. The optional final real-hook confirmation invokes `load_probe.py` ten times at each of 5, 20, and 50 events/s for 15 seconds when `--hook-bin` is supplied. A targeted causal experiment may narrow `--concurrency` or `--repetitions`; the report preserves this reduced coverage explicitly.

```text
python dev/fleet-smoke/stall_round.py \
  --output-dir <absolute-output-directory> --attempt 1 \
  --daemon-bin <absolute-candidate-daemon> \
  --emitter <absolute-ipc-load-emitter> \
  --hook-bin <absolute-candidate-hook> \
  --daemon-label <baseline-or-intervention-label>
```

The controller sets the owned endpoint through the emitter arguments; the emitter projects it to `AGENT_OTEL_PIPE` and `AGY_OTEL_PIPE` on Windows or `AGENT_OTEL_SOCKET` on Unix. It records exact argv, endpoint, source state, active-state snapshots, and SHA-256 hashes. Candidate binaries must be built separately before the attempt. The controller never runs Cargo.

Use `--daemon-label` to distinguish a frozen baseline binary from an instrumented intervention binary. Each invocation accepts one daemon/hash identity; comparisons across invocations use the label, immutable hashes, and reserved attempt number rather than mixing daemon identities inside a profile loop.

Each child is bounded to 180 seconds by default and the campaign has a 30-minute monotonic deadline. Child process trees and captured output are bounded by `perf_campaign.run_subprocess_json` at 32 MiB; serialized raw artifacts are independently rejected above 128 MiB, while 15,000 current emitter records are expected to occupy roughly 4 MiB. The owned collector is set to the exact declared event count for each profile and restored to the suite default afterward. Full emitter output, including `per_event`, plus daemon/collector cleanup and bounded stdout/stderr metadata is stored in a per-profile raw JSON artifact with its hash and byte count. The controller report and stdout contain only reconciled counts, achieved rate, generator-limit state, raw failure durations, consecutive failure clusters, cleanup status, provenance, and artifact references.

`send_failed_events / attempted_events` measures transport send failures. Unique received identities intersected with offered identities, divided by offered events, measures end-to-end delivery from the declared offer. Unique received identities intersected with send-completed identities, divided by `(attempted_events - send_failed_events)`, is separately reported so duplicates and failed sends cannot inflate the conditional ratio. Raw received span count and identities delivered despite a recorded send failure remain explicit. Consecutive failures cluster only when their indices and `(stage, os_code)` match; this describes temporal adjacency in the generator record and does not establish a daemon cause.

The emitter accepts up to 100,000 events, so the 15,000-event sustained profile fits without segmentation. The controller preflight defaults to a 15,000-event declared capability; passing a smaller `--emitter-max-events` rejects the run before reservation rather than substituting a shorter workload.

The real-hook phase is a separate boundary: external hook process execution through an owned candidate daemon and collector. Its process-creation cost is intentionally not compared with IPC-only send durations. Backend visibility and active-install behavior remain separate acceptance steps.

## Approved execution and evidence gates

Antigravity leads ingress instrumentation and experiment decisions, by code/evidence relay when its installed tool hook is unavailable. Codex reviews and applies the relay, owns final acceptance and serializes Cargo. Sol handles the bounded Python controller; mechanical local review continues if a delegate's usage is exhausted. No installed hook configuration changes are part of this workflow.

The current round uses attempt 1 for ten repetitions each of c1/c16, at 1,000 offers/s for 15 seconds; attempt 2 repeats those profiles with disjoint logical CPU affinity and identical product binaries. Attempt 3 completes c4/c8 under ordinary affinity, then ten repetitions each of real process-per-hook at 5/20/50 offers/s for 15 seconds, plus a separate 60-second trace whose exact identities and ancestry are checked via backend MCP/API. This avoids repeating unchanged c1/c16 again while preserving coverage of all four profiles across the round. The proposed normal rates remain an explicit workload assumption, not a measurement of actual user activity.

The first two attempts did not justify a scheduling, pool-size or transport rewrite: affinity changed 217 failures/300,000 offers to 227/300,000. These are observations, not a new acceptable-loss threshold or statistical proof of equivalence. No new numerical SLO is introduced. All failures and generator-limit flags remain visible. Mandatory fmt, Clippy, workspace tests, documentation and guardrails precede final acceptance.

### Diagnostic semantics

Ingress counters are bounded atomics emitted only in the daemon's final diagnostic summary. `accept_polled` means the task reached its first poll, not that a kernel instance is free. `accept_spawn_to_poll_max_micros` measures task scheduling delay; `completion_to_dispatch_max_micros` starts when the accept task observes connection completion and ends when the main loop receives it. It excludes any kernel-completion-to-task-resumption delay. `replacement_create_max_micros` covers the complete Tokio listener-construction call, not a separately profiled Win32 syscall. `dispatch_to_spawn_max_micros` ends at task submission, not at kernel readiness. Independent maxima cannot be subtracted to obtain per-event overhead or correlated with a particular failed event.

These diagnostics do not certify kernel queue occupancy, actual concurrent connections, watchdog firings, timer resolution or a causal source of loss. Their overhead has not been independently isolated against an uninstrumented control. Existing client latency/size/deadline contracts still apply.

### CPU-affinity experiment

`--isolate-cpus` partitions the coordinator's already-allowed logical CPUs into two disjoint sets. Only the owned daemon and emitter are changed. The Windows emitter is pinned while suspended and contained, before execution; the daemon is pinned after readiness and before offers. Requested, inherited and observed masks are retained. Invalid masks fail before the suspended child executes; process-tree cleanup is verified. POSIX support is best effort after spawn; unavailable affinity and Windows processor groups above 64 CPUs are explicit limitations.

The controller, collector, installed daemon and unrelated processes remain untouched. This is logical affinity, not exclusive physical-core reservation: SMT, background OS activity and reduced parallelism per process are confounders. Real-hook affinity is not implemented, so combining `--isolate-cpus` with `--hook-bin` is rejected rather than misreported.
