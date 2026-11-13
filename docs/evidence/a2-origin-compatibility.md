# A2 native origin compatibility review

Reviewed 2026-11-13. No existing top-level `origin` field was found in the native hook schemas below. Keep the current field; treat it as an untrusted classification hint, never proof of provenance. Missing or explicit `harness` allows linking; other values refuse linking. Origin classification does not split a known native session's keyed fallback trace.

| Harness | Native schema checked | Result |
| --- | --- | --- |
| Claude Code | [Official hook input reference](https://code.claude.com/docs/en/hooks#common-input-fields) and G0a Claude 2.1.322 hook stdin | Common/session/tool inputs use `session_id`; no top-level `origin`. |
| Codex | [Official Codex hook input reference](https://learn.chatgpt.com/docs/hooks#common-input-fields) | Common/session/tool inputs use `session_id`; no top-level `origin`. |
| Antigravity | [Official hook reference](https://antigravity.google/docs/hooks), common and event-specific input tables | Common inputs use `conversationId`; no top-level `origin`. |
| Grok | [Native Grok hook reference](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-pager/docs/user-guide/10-hooks.md), Input and event-specific fields | Common inputs use `sessionId` and snake_case compatibility aliases; no top-level `origin`. |
| Pi | [Native extension event types](https://raw.githubusercontent.com/badlogic/pi-mono/main/packages/coding-agent/src/core/extensions/types.ts) and the bridge's `pi-extension.ts` serialization | Native events and the bridge's explicit hook payload have no top-level `origin`; the adapter writes `session_id` from the session manager. |

G0a's outer `origin` classifies capture records. It is outside `stdin` and must not be injected into the native hook payload. The captured Claude stdin records have no `origin`. Codex G0a hooks were disabled: its hook files contain only synthetic collisions/shutdown, not real Codex hook frames. Its captured native session payloads also contain no `origin`.

`g0a_native_claude_hook_and_codex_session_payloads_stay_linked` replays the real Claude hook stdin and real Codex session-meta payload through the production transform, checks absence of the native field, and compares links with their captured session identity. It deliberately excludes synthetic G0a frames. This proves compatibility of the retained native payloads, not live Codex hook conformance. The native Codex hook schema fixture separately verifies the documented input shape; no new harness or live-daemon run is claimed.
