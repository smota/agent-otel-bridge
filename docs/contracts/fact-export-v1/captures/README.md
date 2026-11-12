# G0a sanitized captures

Refs #24. This evidence supports the issue; it does not close it.

The `g0a-20261112/` package covers the captured versions
Claude Code 2.1.322 and Codex 0.167.0: ten final scenarios, S1-S5 for each
harness. S6/S7 were not captured and remain `UNVERIFIED` in the capture matrix.
These observations apply only to the captured versions. Source captures were
collected in Linux/WSL, and published resources report `os.type = linux`.
Only offline sanitization happened on native Windows.

The public files retain structure only, with redaction and per-run keyed
pseudonyms; they do not retain content. Codex `hook_stdin` was not captured
because hooks were disabled. Retained injected collision and shutdown frames
are synthetic fixtures, not harness observations. The JSON matrix is
authoritative for `artifact_available` and `capture_status`; empty Codex
`hook_stdin` rows in the Markdown matrix mean `not captured: hooks disabled`,
not evidence of observation. S6/S7 remain uncaptured.

Field presence alone does not prove attribution or linkage. The coordinator
independently verified that real Claude hook `session_id` values appear as
session-log `sessionId` values in each S1-S5 capture, including both sessions
in S2. This is captured-version evidence only. All nine Codex `turn_context`
sandbox policies were `workspace-write`, including the two S4 resume turns.

There is no packet-level proof of zero traffic. Socket metadata sampling does
not prove absence of traffic. Scoped log evacuation does not cover every
harness side file. Any backend check lacking credentials remains `UNVERIFIED`.

Raw data, receipts, and HMAC keys remain private. `RAW-PRIVATE-SHA256SUMS`
records hashes of private material; the corresponding files are not published
or available in this package. `SHA256SUMS` verifies published package files
only. Generated package bytes and checksums must remain unchanged; the local
Git attributes disable text normalization for `g0a-20261112/**`.
