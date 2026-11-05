# ADR-0074: Enabling resume in json (stream-json) mode — completing the ADR-0044 follow-up (reusing the controlled sid / ADR-0008)

- 상태: 확정 (2026-07-13)
- 관련: ADR-0044 (json mode wiring — scoped resume out as a follow-up) · ADR-0008 (session restore controlled sid) · ADR-0030 (capability) · `crates/engram-dashboard-core/src/agent/backend/claude.rs` (build_spec StreamJson·capabilities) · `session.rs`

## 맥락

ADR-0044 left json (stream-json) resume outside the MVP (as a follow-up), fixed build_spec to always use `--session-id` (fresh) regardless of SpawnMode, and reported caps as resume=false. The user asked: "attach it so it resumes the same as the terminal".

## 결정

In json mode too, build_spec splits the session flag by mode, the same as the terminal branch (Fresh→`--session-id`, Resume→`--resume <sid>`), and caps are resume=true for both modes. It reuses the controlled-sid (ADR-0008) infrastructure as is — no new json-specific machinery. The default output_format of the tree's "에이전트 생성" (create agent) is also switched to StreamJson along with this (safe since resume is guaranteed).

## 거부한 대안

- **Keep the conservative resume=false (status quo)** — the user explicitly asked, safety was confirmed by a spike, and without resume the conversation is silently lost when resuming after a stop, so there is no reason to keep it.
- **Mechanism B (restart `claude -p --resume` every turn)** — already rejected by ADR-0044 (per-turn startup cost + no mid-turn intervention). Only a startup `--resume` is added to the current Mechanism A (persistent process).

## 근거

Spike measurement (2026-07-13, claude 2.1.170) — `claude -p --input-format stream-json --output-format stream-json --replay-user-messages --resume <sid>` coexists with `-p`/stream-json and resumes the past conversation without loss and without "session already in use" (the secret-code recall succeeded + `cache_read_input_tokens` proved the earlier context was loaded). That is, it was not a missing capability but unwired. The restore path is backend-agnostic (`needs_session` unconditionally true + `claude_session_id` persisted), so json takes the same rail as the terminal.

## 영향 / 불변식

- On resume failure, not a silent fresh but a visual `FreshFallback` (loud).
- `capabilities` is now resume=true in both modes (regardless of mode).
- The ADR-0044 §6 item "resume restore consistency follow-up" is completed by this ADR.
- **Unverified remainder:** the app-level round-trip measurement (needs a daemon rebuild) is not done — consistency was confirmed by the CLI spike + code review (deep), and app E2E is a follow-up.
