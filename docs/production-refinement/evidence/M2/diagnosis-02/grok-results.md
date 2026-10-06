# grok-results

Modelo observado: grok-4.7-build-fast. Sessão: 01a10ea1-fe77-74f3-879d-055a9e1bef41

M2 stays partial. This unit authorizes no installation and no code change.

**Immediate cause of the missing bridge series:** no daemon and no pipes, before and after the tests. The collector is live. Last-hour ClickHouse holds only `codex-app-server` (76626) and `codex-exec-server` (24). With the bridge process absent, it has nothing to export. That accounts for the gap in the hour that was queried.

**Disappearance is a separate, open question.** HKCU Run points at the canonical exe `start` command. There is no StartupApproved entry. Boot was 2 Oct. These facts do not show whether the process never started, started and exited, or was stopped later. Provenance stays unknown. Active 0.5.2 hashes are unchanged across the tests and do not match the manifest. That is an integrity question beside the export gap.

**What the probes show:** under a minimal empty profile, unique pipe, loopback receiver, Windows job, 20s timeout, and checked PID ownership, four direct synthetic frames became four decoded spans (`antigravity`, `claude-code`, `codex`, `grok`): accepted 4, rejected 0, unknown 0, `shutdown_dropped` 0, exit 0. The second probe invoked `start`: launcher 42932 spawned pipe owner 57224 inside the job; canonical `agent-hook.exe` ran `PreToolUse --client` for those four clients, each exit 0; four trace spans reached the local sink; shutdown was graceful; pipes were gone and the job closed.

**What they leave open:** launcher exit 0 does not establish the child exit code; that code was not captured. There was no real harness session and no production-backend export. An empty TCP snapshot does not support a universal network-containment claim.

**Separate labeling finding:** both probes omitted `model` and recorded `gen_ai.provider.name` and `gen_ai.system` as `google` for every client. `otlp.rs` 194–198, `input.model.map(infer_provider).unwrap_or(GEN_AI_PROVIDER_GOOGLE)`, matches that default. Provider stays derived from the model. Harness or client identity is the wrong key, because one harness can run more than one provider.

**Severity:** the coverage gap is real, and daemon absence is the immediate mechanism. Onset is unknown. The Google default is a confirmed omitted-model labeling defect and is not why ClickHouse lacks bridge spans. The manifest mismatch is unresolved and was not shown to block decode or local emit.

**Next smallest diagnostic:** read-only history from boot 2 Oct — event log, Run versus StartupApproved state, and any recorded lifetime of the canonical exe — so a never-start is separated from an exit after start. M2 moves only after one real harness session is counted at the production backend, with the pipe-owner exit code captured. Until that session exists, M2 remains partial.
