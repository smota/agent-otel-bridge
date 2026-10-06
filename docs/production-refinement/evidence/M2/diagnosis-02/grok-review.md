# grok-review

Modelo observado: grok-4.7-build-fast. Sessão: 01a10e9c-cf16-7360-be66-27fac35ff772

**Verdict: changes required.** This is a valid one-run probe. A pass shows that process’s pipes and export destinations. It does not show the binary is build `d206a0c`.

**Provenance.** Isolation is observable without a matching hash. Embedded names show those strings were compiled in. Proof is live behavior: pipes created, server PID, and connect destinations. Record full path, size, and hash `24C738…` as the artifact identity. Leave the `d206a0c-dirty` mismatch open. Do not install hooks or treat a pass as trust.

**Split the result.**
- **Containment:** this child owns no default pipe; its TCP peers are only `127.0.0.1:<mock_port>`; the job is gone within the deadline.
- **Function:** the test-pipe server PID equals the child; the loopback sink received the export; the `0xFF` frame yields exit 0.

Containment can pass when function fails. That is a finished diagnostic. Quota stays unmeasured: 15s with a 60s interval never exercises quota reads. Scratch `USERPROFILE` / `LOCALAPPDATA` affect env-based lookups only. `SHGetKnownFolderPath` and HKCU follow the user token.

**Safeguards.**
1. Allowlist the child environment: `SystemRoot`, `WinDir`, and a System32-only `PATH`. Point `USERPROFILE`, `HOME`, `APPDATA`, `LOCALAPPDATA`, `TEMP`, and `TMP` at empty scratch dirs outside the real profile. Set cwd there. Drop proxy, token, and `OTEL_*` variables, then set the pipe names and `OTEL_EXPORTER_OTLP_ENDPOINT`. Set `AGENT_OTEL_IDLE_TIMEOUT_SECS=120` so idle exit does not collide with the probe.
2. Create the process suspended, place it in a job with kill-on-job-close, then resume. Record supervisor PID, child PID, and start time. If `GetNamedPipeServerProcessId` is not the child, stop. Do not kill that other PID. Do not send a payload.
3. Bind `127.0.0.1:0` before spawn. After the pipe check, require every child TCP peer to be that port. Keep raw bytes if protobuf decode fails. Decode failure leaves containment intact.
4. Open the test pipe only long enough to read the server PID, close it, then write a minimal payload on a new connection. No prompts, paths, or credentials. Check pipes before spawn, after startup, after the payload, and after exit. A pre-existing default pipe aborts before spawn. Containment fails when this child owns a default pipe.
5. Accept either injected name (`agent-otel-test-<uuid>` or `agy-otel-test-<uuid>`) when this child owns it. Forbid every other `agent-otel*` / `agy-otel*` pipe owned by this child.
6. Skip any trial that omits the isolation keys. That is the path that may create the default pipe and touch real quota state.
7. If shutdown stalls, terminate the job within 5s. Confirm the PID and both test pipes are gone before the supervisor exits.

The earlier AGY failure was a tool-policy denial. It is separate from this daemon’s credentials and from hash identity.
