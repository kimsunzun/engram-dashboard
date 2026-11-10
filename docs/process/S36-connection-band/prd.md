# PRD — S36 connection band (frontend)

- Status: draft 1 · 2026-11-10 · decisions = ADR-0350 (two bands, one stop path), ADR-0356 (version band only informs, 「다시 연결」 on both bands), ADR-0345 (「다시 연결」 on the band), ADR-0287 (band layer and ✕).

## 1. What · why

The shell already decides the connection band (S21 stage 3, merged to master): `not_connected { refused }` or `version { can_stop }`, sent with the connection state to every window. The frontend still draws only the old notice: a start-failure cause text with a ✕, no button. A daemon that dies while the app runs shows nothing, and recovering needs the tray.

This topic draws the two bands from the shell's band, puts one 「다시 연결」 button on both, and adds the LLM commands for the same actions (CLAUDE.md 「LLM-우선 제어」). Smallest version — the parked frontend (`4d25dfa7`) is the material, rebuilt on master without its rare-ordering guards.

## 2. Scope / non-goals

In scope: the band component, band texts, the 「다시 연결」 button and its busy state, ✕ memory keyed on the band, popout text-only, the three LLM commands, removal of the unused `daemon_stop` command.

Non-goals:
- A stop button on any band (ADR-0356). The stop stays on the tray and the LLM `daemon.stop`.
- Replacing a mismatched daemon — update system, T-84.
- Shell changes: the band rules, the stop path and their guards stay as merged.
- Boot start in the main window only (TRD S21 R17) and the listener single-flight that goes with it.
- Parked rare-ordering extras: closing an open confirm on a band change, focus management, attach after a failed start (R15), status re-pull for LLM replies, the special `connection.reconnect` error on ②.
- Multi-daemon.

## 3. Requirements

- **R1 — band ①** (`not_connected`): 「데몬에 연결하지 못했습니다.」 + a cause when known — 「데몬이 이 앱의 인증을 거절했습니다」 when `refused`, otherwise the start-failure text (`connectionError`) when there is one.
- **R2 — band ②** (`version`, older or newer daemon alike): 「데몬에 연결하지 못했습니다.」 + 「실행 중인 데몬이 이 앱과 버전이 다릅니다. 데몬을 끈 뒤 다시 연결하세요」. No start-failure text under ②. `can_stop` is not read.
- **R3 — no band** when the shell sends no band (connected, before the first failed reconnect, during the boot hold).
- **R4 — 「다시 연결」** on both bands, main window only: runs the existing start (`DaemonControl.start()` — starts the local daemon if none runs, then attaches). While it runs the button reads 「연결 중…」 and is disabled; when it ends the button returns and the band follows the shell.
- **R5 — ✕** hides the band; the same band kind with the same cause stays hidden until the next `connected`; a different band or cause shows.
- **R6 — popouts** show the band text only — no button, no commands.
- **R7 — `connectionError` lifetime:** set by a failed start (as today), cleared on `connected` (as today), when a new start begins, and when ② or ① `refused` arrives; not recorded while one of those holds (ADR-0350 decision 1). Otherwise the cause of a version-mismatch boot would show under the ① that follows the tray stop.
- **R8 — LLM commands** on `window.__engramCmd`, main window only, no `help`:
  - `connection.status` → `{ state, band, text }` (`text` = the band sentence, `null` with no band).
  - `connection.reconnect` → the same call as R4; success → the `connection.status` reply; failure → an error carrying the start's cause.
  - `daemon.stop` → the shell's stop (`daemon_shutdown`), no confirm; reply `{ outcome, reason }`.
- **R9 — remove `daemon_stop`** (the old pid-kill command; no frontend caller since S21 U5a), as ADR-0350 decision 6 records.

## 4. Acceptance criteria (measured on a release build)

- **AC1** — kill the daemon process while connected → no band at first, then ① after the first failed reconnect → 「다시 연결」 → 「연결 중…」 → the daemon starts and the band goes away.
- **AC2** — a `v0.3.2` daemon running → ② with the R2 text and 「다시 연결」; pressing it while that daemon runs leaves ②; tray 「데몬 끄기」 → ①; 「다시 연결」 → connected.
- **AC3** — token rewritten in an isolated `daemon.json` → ① with the refusal text; restore → 「다시 연결」 → connected.
- **AC4** — a start failure with a cause (e.g. unwritable data folder) → ① with that cause; fix → 「다시 연결」 → connected, cause gone.
- **AC5** — ✕ on ① → hidden; still hidden while the same ① holds; a different band shows; after a connect the memory is clear.
- **AC6** — a popout during ① shows the text, no button.
- **AC7** — LLM: `connection.status` returns the shape in R8 in each state; `connection.reconnect` connects when a daemon can start; `daemon.stop` → the daemon exits, reply `outcome: closed`, ① appears.
- **AC8** — vitest + tsc green; full regression green (except known failures).

## 5. Open decisions

None open. Settled by the user: two bands, texts (U2, ADR-0356), 「다시 연결」 on both (ADR-0356), no stop on the band (ADR-0356), ✕ behaviour unchanged (U1), popout text-only (R9), `daemon.stop` without confirm (D5).

Main defaults: rebuild from master instead of rebasing the parked branch (it predates ADR-0352 and ADR-0356); drop the extras listed under non-goals (each changes the screen only in a rare ordering).

## 6. Risks

- The parked code was written for the stop button and the stopping mode; copying it whole would bring both back. Take pieces, not the file.
- AC2 needs a `v0.3.2` daemon build; if it cannot be built, AC2 is covered by unit tests only and reported as such.
