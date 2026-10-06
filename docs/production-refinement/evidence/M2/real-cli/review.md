# Grok review

Model: grok-4.7-build-fast; session: 01a10ebf-af82-7433-ba6f-0b2bf9a446e3

**Verdict: conditionally sound.** The pinned foreground daemon, empty-home env, private pipes, loopback sink, hook-hash check, and fail-closed rejects can stay. Three corrections are required.

1. **Enforcement.** `--dangerously-skip-permissions` removes the control that would limit the run to one `ReadFile`. The prompt cannot replace it. Allow only `ReadFile` if the CLI supports that. Otherwise use a read-only view of the repo so config still loads, with the marker in temp. Fail unless the only call is `ReadFile` of that marker.

2. **Containment.** Create `agy.exe` suspended, assign it to the existing kill-on-close job, then resume, so hook children inherit the job. After shutdown, confirm absence by the daemon PID, the agy PID, and the private pipe names. Leave every other `agy` process alone.

3. **Environment.** Keep the user environment for subscription auth and global hooks. Override both pipe variables. Remove paid-fallback credentials and OTEL/collector endpoints. Before spawn, read repo-local config; if it sets an API key or an exporter, stop. Pass only when auth is the subscription and the OTLP conversation and tool-call ids match the stream session and tool-call ids. Keep `stream-json` outside the repo, extract those ids, and delete it.

No other change is required.
