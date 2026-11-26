# Handoff: S25 step 6 landed (code P1–P3 + ADR-0353..0355 + doc pass) — next = doc review of the pass, then step 7

## Know first
- **Nothing is running.** Tree clean except `src-tauri/bindings/*.ts` line-ending noise (never stage).
- No push (user: 「푸쉬없이」 — still in force). Ahead of origin by 237 + this handoff commit.
- One open question to the user, unanswered: append 「그 밖의 막기는 실제로 불편이 생기면 그때 더한다.」 to the new CLAUDE.md principle line 「최소한만 막기」 (user said 「일단 너무 막지 말고 추가로 막는건 실제로 불편하면 막자고」). Ask once; do not edit without a yes.
- Step 5 user confirmation is still deferred (from the previous handoff) — material: commit `baf769ff` body and the trd.md "Step 5 landed" block.
- Codex test threads left in the user's real `~/.codex` (deletion is the user's): `01a1c24d-f65a-75a0-8b6b-1cd27dcaefe4`, `01a1c264-a0cb-70c3-8b1d-88d843ea9c05`, `01a1c26f-b831-7262-aef4-f057b9b33710`, `01a1c367-b0d7-75f1-9caf-01b81b9f98b3`, `01a1c3a0-d86f-7831-b260-e0d20109f5a5`, plus `usage_probe_smoke codex_` threads (one per lane run) and the step-6 measurement thread from 11-09.
- This PC's codex 0.163.0 shows an update modal (→ 0.167.0) on terminal start; the live L4 test now disables the check by `-c`. Updating codex changes the codex lane version (lane re-runs).
- Style: no translationese / AI tone in chat or app wording; expand questions in plain terms, one at a time, short lists (user asked repeatedly for 「간단하게」).

## One-line state · next first action
S25 step 6 (codex launch-material translation) is landed and QA-passed. **Next:** run `/engram-review doc` (full) on the doc pass commit `2070ebf9` (child TRD as-landed + §11 record, parent TRD hand-over + "Step 6 landed", PRD R37 addendum, tracking T-84..T-93, step-log) — it was committed without review. Then the two leftover ADR items (below), then step 7 per parent TRD §5-2 (its hand-over list (a)–(f)).

## Repo state (branch v0.5.0/feat/preset-library · wt1)
- HEAD `9f7c9195` + this handoff commit; origin is 237 behind before this commit (nothing pushed). Working tree clean (bindings noise only).
- This session's commits: `7d1e0caa` qa lane paths · `9969ac60` CLAUDE.md principle · `090ed679` P1 · `a3891273` P2 · `abee6ebe` P3 (tree `f9cb17db` = QA tree) · `28b2bcf7` `b6223f08` `dc977a4d` ADR scaffolds · `45e06271` ADR bodies · `2070ebf9` doc pass · `9f7c9195` ledger/attachments.

## Verification state
- QA (`/engram-qa`) on the P3 tree `f9cb17db`: full regression (5737 Rust · 2762 frontend · tsc · 34 gates) PASS; codex lane (usage probe 2 · backend_contract 11 · `codex_launch_live` L1–L5) PASS; claude lane PASS (rerun 4, fresh); mutation 2/2; GUI PASS (codex JSON turn + resume `resume_instructions=true`, terminal up after Esc on the update modal). One claude-lane timeout in rerun 3 under machine load (disk cleanup + other worktree builds) passed fresh in rerun 4 — no label (ADR-0038).
- Intermediate commits P1 (`090ed679`) and P2 (`a3891273`) are the review-passed trees; QA ran only on the final tree.
- **Not reviewed:** doc pass `2070ebf9` (gate 미통과 for docs — trust the code, not yet the prose). ADR bodies 0353–0355 written by main, not reviewed.
- Rerun: `cargo test --workspace -- --test-threads=4`; lanes per `.claude/skill-bindings/qa.md` (codex live: `cargo test -p engram-dashboard-agent --test codex_launch_live -- --ignored --test-threads=1 --nocapture`, 2 real model turns).

## Failed approaches (do-not)
- Do not let the live L4 depend on the real `~/.codex` having no startup modal — it now passes `check_for_update_on_startup=false` as a preset raw line.
- Do not mark a timing failure PASS by rerunning (binding rule) — it was reported FAIL, then the user asked for a rerun.
- QA rerun 1 failed at CI platform gates 4/6: new files with test-only `#[cfg(windows)]` / `std::os` must be added to the `ci.yml` lists (feedback filed, Engram `8535305`).
- The P1 coder hit bash heredoc backslash folding — write code with Edit/Write tools.

## Decisions
User (2026-11-10):
- H1 codex-mirrored code · 상황: some code near-verbatim from openai/codex (Apache-2.0), same review finding two rounds · 선택지 A rewrite ★ / B keep + source pointer · 고른 것 「일단 다 만들어놓았으면 그냥 그대로 가던지」 · 갈림 · 조건: source + license pointer comments added; whether that is enough = user/legal, not main.
- (묻지 않음) scope principle · 고른 것 「일단 어지간한건 그냥 그쪽 클로드나 코덱스 룰로 가고 우리는 최소한만 막자 이것저것 다 하려고 하지 말고.」 → CLAUDE.md line (「ㅇㅇ 한줄 언헝」) `9969ac60`; follow-up 「일단 너무 막지 말고 추가로 막는건 실제로 불편하면 막자고」 (append not yet confirmed).
- Applied under that principle (presented, no objection): H2 Noop safety changes kept (space-less priming with `& ^ | < >` skipped; control URL with metachars refused); H3 real-terminal cwd guard minimal (2+ `%`, `"`, control, non-UTF-8, `& ^ | < >` without a space — single `%` measured intact); H4 `$<이름>(으)로`; H5 「지시서」, no count; no JSON size cap; retry rule unchanged.
- QA rerun · 고른 것 「다시 실행해봐. 뭔가 아까 이상했었을듯?」 · 근거: the timeout looked environmental.
Main: P1–P3 per TRD §9 (#1–#17, C1–C4); review fixes listed in the commit bodies; three commits built from the review-passed snapshot trees; ADRs: (a)+(b)+cwd → ADR-0353 (amends ADR-0215 d4/d5), (e)+(c) → ADR-0354 (amends ADR-0215 d7; ADR-0337/0226/0231 impact in its body), (d) → ADR-0355.
Open: ADR-0337 decision 3 still says the notice "states intended behaviour" — needs an amend (`/engram-adr link` or a short supersede) ; check whether ADR-0216 needs an impact note for the Noop control-URL refusal; optionally add the single-`%` measurement to `docs/research/codex-launch-materials-measurements-2026-11-09.md`.

## Improvements
- Filed: code-conventions feedback — CI platform gate lists for test-only OS cfg in new files (Engram `8535305`).

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 log --oneline -5 && git -C I:/Engram/apps/engram-dashboard-wt1 status --short | grep -v src-tauri/bindings`

## References
- `.claude/handoff/attachments/20261110-s25-step6-code/s25-step6-ledger.md` (every decision, hold, tracking item, QA run, thread id) · `p1/p2/p3-handover.md` (coder handovers) · `restore-points.txt`
- `docs/process/S25-preset-library/trd-codex-translation.md` ("As landed", §11) · `trd.md` §5-2 step 6 landed, step 7/8 hand-over
- `docs/decisions/0353-*.md`, `0354-*.md`, `0355-*.md`
