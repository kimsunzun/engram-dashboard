# ADR-0354: Every codex app-server thread resume carries developerInstructions

- 상태: 확정 (2026-11-10, 근거: measurement M8 — `docs/research/codex-launch-materials-measurements-2026-11-09.md` · main decision #15, TRD `docs/process/S25-preset-library/trd-codex-translation.md` §9-1 · landed `abee6ebe`)
- 관련: Amends ADR-0215 (결정 7) · ADR-0337 (codex materials — impact note below) · ADR-0226 and ADR-0231 (own the `FirstTurnSink` port — impact note below)

## 맥락
ADR-0215 decision 7 recorded that codex takes the app-server instructions only at `thread/start`, ignores a different value on resume, and so we do not send the field on `thread/resume` ("it has no effect"). M8 (codex 0.163.0, mock provider, manual compaction) showed the field is not without effect: codex holds the resume's `developerInstructions` in the resumed process and rebuilds the initial context from it at a compaction there. Without it, a resumed JSON thread lost its instructions entirely at the first compaction in the resumed process — a pre-existing defect in every resumed codex JSON agent. The value still does not reach the next turn of a resume; a change reaches the model only through the first turn's `additionalContext`.

## 결정
1. Every codex app-server `thread/resume` that has instructions carries `developerInstructions` = the current full instructions text (Noop: the priming file; real materials: the merged text), changed or not, when the `initialize` `userAgent` reads codex 0.155.0 or later (the version whose stable schema has the field); below that or unreadable, the field is dropped (insurance, not a known break). With no instructions the field is absent: Noop — no priming file, or one that is blank or cannot be read (`priming_text` → `None`); real materials — no merged instructions file at all (`LaunchKit.instructions_file` = `None` — the kit has no instructions to carry, by `prepare`'s contract: parent TRD §5-2 step 7 hand-over (e)) or a blank merged text (`backend/codex/launch.rs` `instructions_text` → `Ok(None)` in both cases; an unreadable merged file fails the spawn — ADR-0353).
2. A changed resume also sends the change as the first turn's `additionalContext` (`engram-instructions`), with `experimentalApi` opted in only on that spawn. It is attached to every `turn/start` of that process (never to a steer) until the echo of a turn that carried it arrives; after two consecutive rejected carrying turns it is no longer attached in that process.

## 거부한 대안
- **Keep ADR-0215 decision 7 (no field on resume)** — rejected by measurement M8: the resumed process rebuilds its context without instructions at a compaction.
- **Send it on resume only when the instructions changed** — rejected: the compaction loss happens whether or not they changed.
- **Send it with no version gate** — rejected as a precaution: the field enters the stable schema at 0.155.0; the gate costs nothing on current codex (`약함` — no measured break on an older codex; insurance only).

## 근거
- M8 (resume + compaction, mock provider): with the field the rebuilt context holds the instructions; without it, none.
- Code: `backend/codex/transport.rs` `RESUME_INSTRUCTIONS_FLOOR`, `backend/codex/mod.rs` `thread_open` / `app_server_open`; QA 2026-11-10 GUI: the Noop resume logs `resume_instructions=true`.

## 영향 / 불변식
- The Noop resume handshake changes (its pin row carries the field) — deliberate, named as a pre-existing fix in the P3 commit.
- Accepted consequence — removing every instruction does not reach a resumed conversation: with no text there is neither `additionalContext` nor `developerInstructions`, so the old instructions stay in its history until a compaction in that process rebuilds the context without them (M8). The same holds for a Noop resume whose priming file cannot be read (TRD R6 · parent TRD `docs/process/S25-preset-library/trd.md` §6-3).
- ADR-0337 impact (not a new decision): `experimentalApi` only with `additionalContext`; extra skill roots are set on every process including resume (M4, M9); the junction layout `<volatile>/engram/skills/<name>` confirmed (M1); step 8 records the applied hash only when the first-turn report says delivered.
- ADR-0226 / ADR-0231 impact: `FirstTurnSink` now carries one bool (whether the instructions reached the model); session-id persistence does not depend on it and is unchanged.
