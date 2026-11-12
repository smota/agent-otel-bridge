# Gate-0 captures (G0a): sanitized, structure-only

These derivatives come from real, isolated sessions of **Claude Code 2.1.322** and **Codex 0.167.0**
captured on 2026-11-12. They are the evidence base for the `fact-export-v1` / `link-id-v1` contract
(work packages C1a and C1b in [docs/BACKLOG.md](../../../BACKLOG.md)).

## What is here

| Path | Content |
|---|---|
| `capture-matrix.md`, `capture-matrix.json` | Per harness, scenario and surface: which identity, parent-edge, call, tool, message, usage, trace-context and git fields exist, and where synthetic sentinels appeared |
| `<harness>-<version>/<scenario>/` | Structure-only derivatives of hook payloads, AOB OTLP spans, native OTLP, session logs and git provenance |
| `SHA256SUMS` | Digests of every derivative file, plus the hashes of the private raw receipts they were derived from |

All free text (prompts, instructions, responses, tool output, commands, file contents) is replaced by
typed placeholders such as `<str:len=N>`. Keys, closed-vocabulary values, pseudonymized identifiers,
numbers, normalized timestamps and the synthetic `G0A_*` sentinels are kept. Raw captures stay private.

## Scenarios

| ID | Exercises |
|---|---|
| S1 | Tool success and failure, file edit, commit (git provenance) |
| S2 | Two concurrent sessions, plus synthetic frames with colliding tool-call IDs across sessions |
| S3 | Subagent / sidechain spawn |
| S4 | Session resume |
| S5 | Inherited `TRACEPARENT` |

## Isolation

Captures ran in disposable `/tmp` git repositories with synthetic content only. Hooks pointed at a
capture proxy and an isolated AOB daemon that exported only to a loopback fixture collector. For
every accepted run, the operator's real AOB socket, hook configuration, Claude settings and dendro
database were verified unchanged, and every synthetic session log was moved out of the harness's
home directory.

## Findings and limits

- **Identity link (Claude Code):** in every real session the hook payload `session_id` equals the
  session log `sessionId`. The extra IDs in S2's hook frames are the synthetic collision frames.
- **Codex hooks:** the operator's global Codex hooks embed a fixed AOB socket and also call other
  local relays, so Codex ran with `--disable hooks` for each capture. Codex hook payloads are
  therefore **not** captured here; session logs, native OTLP and git evidence are.
- **Codex side effect:** Codex records a `projects."<workspace>".trust_level` entry in its own
  `config.toml` for each workspace it runs in. This is Codex behaviour, recorded as a key-level diff
  in the private receipts.
- S6 (truncated tail) and S7 (collector outage) are harness-free replays and are not included yet.
