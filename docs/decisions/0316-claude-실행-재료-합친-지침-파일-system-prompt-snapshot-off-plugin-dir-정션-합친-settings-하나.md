# ADR-0316: claude launch materials = one merged instruction file + `--system-prompt-snapshot off` + `--plugin-dir` junction + one merged `--settings`

- 상태: 제안 (2026-10-31, basis: TRD decision T7 (confirmed 2026-10-18 — recommendation as is) · T10 (user 2026-10-18) · R35 · G42 (2026-10-31) · measured (TRD §1-5 table · §6-1))
- 관련: ADR-0334 · ADR-0327 · ADR-0328 · ADR-0109 Decision 9 · TRD `docs/process/S25-preset-library/trd.md` §1-5 「backend 번역」 · §7 4

## 맥락
With which flags do we load the preset material into the two claude modes (JSON and terminal)?

## 결정
1. Instructions: one file merged in loading order → `--append-system-prompt-file` (takes the place of today's priming). Only for real material, `--system-prompt-snapshot off` (new text also on resume).
2. Skills: the volatile folder `engram\skills\<이름>` = a directory junction pointing to the library skill folder → `--plugin-dir <휘발>\engram` → name `engram:<스킬>`.
3. Settings: the claude raw text merged in attached order → one volatile `settings.json` → one `--settings` (ADR-0328).
4. The model is not loaded separately — the `model` key in the raw text goes through `--settings` (R35).

## 거부한 대안
- **Skills through `--add-dir`** — it has no prefix, so if the name collides with a native skill we do not know the winner (T7). The chosen way does not mix with the user's `~/.claude/skills` and its loading was measured (T7).
- **Several `--settings`** — the behavior of a second `--settings` is not in the documentation and was not measured (§7 19).

## 근거
- claude 2.1.284 `--help`: `--plugin-dir` (repeatable) · `--settings <file-or-json>` (single-use form) · `--system-prompt-snapshot <on|off>` exists (TRD §6-1 — certain).
- Plugin folder loading = measured, certain · following the junction = not measured (updated 2026-11-09, S25 step 5 — measured certain with a fake API: the 2026-10-20 research ran this exact shape, `--plugin-dir <휘발>\engram` + per-skill junctions, on claude 2.1.284 against a fake local API with no real model turn — `docs/research/skill-md-recognition-claude-codex-2026-10-20.md` row 6; the step-5 code review also found static evidence in the binary — the plugin skill loader accepts an entry that `isDirectory()||isSymbolicLink()`; the live confirmation is in the step-8 real CLI lane) · new text also on resume with snapshot off = measured, certain · no cache loss when the text is the same = measured (TRD §1-5 table).

## 영향 / 불변식
- Following the junction is not measured — it is measured at the relevant step (updated 2026-11-09, S25 step 5 `baf769ff` — measured certain with a fake API, see 근거; only the live turn is left, in the step-8 real CLI lane; copying the skill folder instead is needed only if that run fails). Noop material emits the argv byte for byte as today (ADR-0328).
- A running agent sees the library skill folder through the junction — if that folder is moved, renamed or deleted mid-session, that session breaks or fails with 「파일 열림」. This is the same behavior as today's CLI skill folder and is not handled separately (G42 · user 「그건 기존에도 똑같은거 아냐?」 · TRD §1-11a finding 14 closed).
- The parts for creating and turning off the junction = ADR-0336 (`junction` 2.1.0 `create` · turning off = delete through the very handle that confirmed it is a link) (implemented 2026-11-01 — delete through the very handle that confirmed it · finding the path again and deleting it could delete a place that was swapped for something else between the check and the delete — S25 step 2 code review).
