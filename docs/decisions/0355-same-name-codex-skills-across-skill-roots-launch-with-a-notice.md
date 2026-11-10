# ADR-0355: Same-name codex skills across skill roots launch with a notice

- 상태: 확정 (2026-11-10, 근거: user decision U2 2026-11-09 — TRD `docs/process/S25-preset-library/trd-codex-translation.md` §9-3 · measurement M2)
- 관련: ADR-0341 (folder entries — codex gets only what it lacks) · ADR-0337 (codex materials) · answers the PRD §6 same-name row

## 맥락
In codex JSON mode a preset can bring skills from several roots: library skills linked into the volatile folder, folder entries' `.agents/skills` / `.codex/skills`, and codex's own skill folders (`$CODEX_HOME/skills`, `<home>/.agents/skills`, project folders). M2 measured what codex does with two skills of the same name in different roots: it lists both, reports no error, and `$name` then selects neither. The user would not know why a skill call does nothing.

## 결정
Same-name skills across roots do not block the launch. The launch check (step 7) produces the notice `codex-same-name-skills` naming the skill and the places it was found, with the wording in TRD §4-9 (as landed: 「알림: 같은 이름의 스킬이 여러 곳에 있습니다. codex에서 $<이름>(으)로 부르면 어느 쪽도 선택되지 않습니다. <이름>: <경로>, <경로>」). Two library skills of the same name stay the existing library conflict block.

## 거부한 대안
- **Block the launch** — rejected by the user (U2): the duplicate may be harmless to the task, and blocking would stop agents over a skill nobody calls.
- **Launch silently** — rejected by the user (U2): `$name` silently selecting neither is the failure this notice exists for (M2).

## 근거
- User 2026-11-09: 「같은 이름이 여러곳있으면 띄우되 알려.」; wording particle `(으)로` — user 2026-11-10.
- M2 (codex 0.163.0): both listed, no error, `$name` selects neither.

## 영향 / 불변식
- Step 7 builds the comparison from library names + folder `skill_names` + `root_skills` over codex's own roots (P1 `backend/codex/settings.rs`), canonicalizing every `SKILL.md` path and dropping identical ones before counting places.
- Codex's plugin namespace (`plugin:name`) and its walk caps are not predicted; for such layouts the notice can differ from codex (documented on `FolderLoadSet::skill_names` and `walk_entry`).
- Notice wording arm: `launch.rs` `notice_text` (`CODEX_SAME_NAME_SKILLS`); malformed `args` fall back to the generic wording.
