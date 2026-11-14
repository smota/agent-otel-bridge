# G0a S2 collision fixture

Read-only source: dendro-enterprise/.agent-runs/g0a/sanitized-final-20261112/codex-0.167.0/S2/hook-sanitized.jsonl.

Source SHA-256: bf0a30ceab9baf32791fc6c86e8f08657fc0db573fc54761169c3a9f74ccb31d.

Only the four synthetic_kind=collision rows were extracted, retaining their recorded header, origin and timestamps. JSON is compacted and key-sorted; values are unchanged. The source was read only and rechecked. All four rows carry client_id=2 and origin=synthetic. The source contains zero origin=harness rows; these probes must not be described as native Codex harness events or relabeled client_id=3. Replay transfers the envelope origin into the hook input to preserve provenance. Real Claude/Codex isolation is tested separately with canonical wire IDs 2 and 3.
