# ADR-0337: codex launch materials — JSON uses developerInstructions · extraRoots, the terminal only the 「기본」 priming and the settings `-c`

- 상태: 제안 (2026-10-31, basis: user decisions P4-FINAL (2026-10-20 「프리셋은 베이스때문에 무조건 들어가야겠네. 그럼 안에 스킬이나 그런 지시서가 있을때 경고 띄우면 될듯」) · P7AC · G28 + main M8 · M13 · M19 · M20 · M29 · M97 (reported)) · 부분 폐기 by ADR-0341 (Decision 3, the notice count) · 부분 폐기 by ADR-0357 (Decision 3, the intended-behavior sentence)
- 관련: ADR-0215 (codex instructions = developer_instructions) · ADR-0216 (override order) · ADR-0329 · ADR-0334 · ADR-0331 · PRD R37 · TRD §1-3 6 · §1-5 table · §7 5 · research `docs/research/codex-instruction-channels-2026-10-20.md` · Amended by ADR-0341 (Decision 3, the notice count) · ADR-0354 (impact note: `experimentalApi` only with `additionalContext`, extra roots on every process, junction layout confirmed, the applied hash recorded only on a delivered first turn) · Amended by ADR-0357 (Decision 3, the intended-behavior sentence)

## 맥락
For codex, the instruction and skill channels differ between JSON (app-server) and terminal mode. In the terminal there is no way to give a skill folder per process (PRD R37 · research).

## 결정
1. **JSON**: a new conversation = `thread/start.developerInstructions` · skills = the same junction folder passed as `skills/extraRoots/set` (per process · including skills in nested presets — M13) · on resume, only when the instruction hash differs from the last applied value, the first-turn `additionalContext` (an experimental API — T6) · a skill format that codex cannot load = blocked with 「오류」 (no new category — P7AC).
2. **Terminal**: only the priming of 「기본」 (today's path `-c developer_instructions=`) + the preset settings codex field `-c` lines are loaded. Preset instructions and skills (both common and codex-tagged) are not loaded.
3. The instructions and skills dropped in the terminal are listed by name in the notice list `notices` of the successful run reply — only to the caller (CLI output · screen), and nothing is sent to the started agent. The wording is facts only · it states that this is intended behavior and does not recommend a fix (G28 — the final wording is shown to the user at implementation).
4. Nothing is written to the working folder or `~/.codex`.
5. The settings `-c` lines = after the passthrough · before our overrides · blocked if they hit the command-line guard or the length budget (M8).

## 거부한 대안
- **Always generating an instruction file in the terminal working folder + a `.agents\skills` junction** (the decision of 2026-10-18) — contention among codex instances in the same folder · clone blocking · no cleanup (review #1 · withdrawn with P4).
- **Instructions only, through a per-agent profile `-p`** (P4) — left as research of 「codex 가 지원하면 나중」 (`codex-instruction-channels-2026-10-20.md`).
- **Not loading presets in the terminal at all** (P4-REVISED) — they must be included because of 「기본」 (P4-FINAL user words).
- **A fix button · 「고치고 실행」 modal for skills that cannot be loaded** (P7A) — user 「너무 요란한데?」 · go through the existing block path (P7AC).
- **Putting a remedy (「JSON 모드를 쓰세요」) in the notice wording** (the first wording of P4-FINAL) — because of the user's worry that bad wording could make the LLM read and insert that file by itself, only facts were kept (G28).
- **Changing `CODEX_HOME` per agent** — basis (record — research): `CODEX_HOME` moves settings, skills, global AGENTS.md, sessions and authentication as a whole, so changing it per agent breaks resume lookup and authentication (`docs/research/external-skill-dir-loading-claude-codex-2026-10-10.md` finding 11 — source certain · side effects likely · `docs/research/codex-instruction-channels-2026-10-20.md` 「앞선 보고서가 이미 막았다」).
- **Installing a local plugin** (`codex plugin marketplace add` → `codex plugin add`) — basis (record — research): installing changes the global settings, so it is not a per-agent switch (finding 15 of the same research — loading not measured).
- **If it is a user folder, ask whether to create it and remember** (the old R37 answer · memo 30) — the user withdrew it on 2026-10-18 (memo 202 — the question · the answer remembered per agent · the CLI · the bus default · the block reason 「codex 파일 대답 없음」, as a whole). Basis (main 2026-10-31 — reported): after P4-FINAL nothing is written to the working folder, so there is no question itself (M97). There is no question now — afterwards P4-FINAL also withdrew 「늘 작업 폴더에 깐다」, so the codex terminal writes nothing to the working folder, and there is no file to ask about creating.

## 근거
- Measured: `developerInstructions` = certain · `extraRoots` = likely · `additionalContext` = an experimental API (TRD §1-5 table).
  - ★Update (S25 step 6, 2026-11-10)★ — `extraRoots` = likely at 2026-10-31; certain on codex 0.163.0 since S25 step 6 (a junction under an extra root is followed — M1; the roots are per process, not restored by `thread/resume` and not shared between processes — M4 · M9; research `docs/research/codex-launch-materials-measurements-2026-11-09.md` · the ADR-0354 impact note). `additionalContext` is accepted only with the `experimentalApi` opt-in (M3).
- User decisions P4-FINAL · P7AC · G28.

## 영향 / 불변식
- The notice is not a warning category (ADR-0331). The codex terminal notice is a user exception of P4-FINAL.
- The 「지난 적용 지침 해시」 of JSON resume is written at the latch's first-turn event as a **separate incarnation-check registry mutate** — whether or not the session id compare-and-replace writes anything (on resume, expected = stored value · new value = stored value, so it writes nothing). The manager receives the hash from the launch materials and carries it through that closure. The session id persistence point (CLAUDE.md invariant) is unchanged (M29 · re-review A3).
- The path that loads instructions and skills in the terminal is reopened in that research when codex supports a per-process instruction channel.
