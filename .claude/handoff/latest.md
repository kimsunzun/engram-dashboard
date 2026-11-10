# Handoff: S25 — master merged + QA PASS, step-6 docs reviewed and closed (ADR-0357) — next = merge master again (S36 landed), then step 7

## Know first
- **Nothing is running** from this session. Tree clean except the 39 `src-tauri/bindings/*.ts` line-ending noise (never stage; `git checkout --` on them was blocked by the auto-mode classifier — leave them).
- **Push is now allowed** — user: 「푸쉬는 알아서 하고」 (replaces the earlier 「푸쉬없이」). Branch is pushed up to `9ee5db64`.
- **master moved again during this session**: origin/master is 16 commits ahead of this branch — `a40aaefe` "Merge v0.4.1/feat/connection-band — S36 connection band frontend" (brings ADR-0356, the connection band UI). `git merge-tree` predicts conflicts in `docs/decisions/README.md` (regenerate with the ADR index tool) and `src/api/protocolClient.test.ts`. The TRD §5-1 rule 「코드 짜기전에 최신업뎃 받아야된다」 applies before step 7 code.
- **Other sessions are active** in other worktrees (the user warned 「여러개 실행되서 제대로 안될수도 있음」): the main checkout's vite holds port 1420; before taking an ADR or T-number, scan all refs (the ADR script does; for T-numbers run the all-refs grep — highest used now T-100 here).
- Master-side open items carried from master's handoff (`history/20261110-131208-transport-done-merge-pending-user-gui.md`): the user's manual GUI check of `05cd2b1d` is pending; the untracked `.engram-dev.qa-bak-*` folder in the main checkout holds a daemon token — never stage it; parked branch `v0.4.1/feat/connection-ui-parked`; master's next topic = connection-UI simplification (S36 may have done part — check).
- Step 5 user confirmation is still deferred (material: commit `baf769ff` body + trd.md "Step 5 landed").
- Codex test threads in the user's real `~/.codex` (deletion is the user's): previous list in history `20261110-132008-s25-step6-landed.md` plus `01a1c43c-ae9a-73d0-8126-1edfb884923d` (this session's codex lane L1).
- Installed codex is now **0.167.0** (was 0.163.0); `lanes.tsv` codex line = 0.167.0 @ `bb062838`, claude line = 2.1.284 @ `bb062838 +uncommitted` (the c2 fix tree, later committed as `6c34aac9`).
- Style: plain Korean, no translationese; one question at a time; the user is terse and delegates — 「알아서 해. 좋은 방향으로.」 「문서 수정은 알아서해.」

## One-line state · next first action
S25 is at the end of step 6 (code + docs + ADRs all reviewed/QA'd and pushed). **Next:** merge origin/master (S36 connection band) into this branch (resolve the 2 conflicts, review + QA the merge per the same pattern as `bb062838`), then start step 7 (`asset-library` crate — parent TRD `trd.md` §5-2 step 7, hand-overs from steps 5 and 6 (a)–(f)); first decide whether step 7 needs a child TRD like steps 6 and 20 (likely yes — it is the largest step), and stop for user decisions before code.

## Repo state (branch v0.5.0/feat/preset-library · worktree wt1)
- HEAD `9ee5db64` + this handoff commit; upstream 0/0 before this commit (this commit is unpushed until the next push); origin/master 16 ahead / branch 146 ahead of master.
- This session's commits: `d5286b5f` renumber T-81..83→T-94..96 · `bb062838` merge master (S21 stage 3) · `6c34aac9` c2 test-race fix · `8dcb9dfb` T-97..T-99 + step-log merge entry · `bc93f49f` T-100 · `a9f3eb12` step-6 doc review fixes · `b56dd3c9` `54048607` ADR-0357 · `9ee5db64` ADR-0337 amend closed in docs.
- Engram repo (I:/Engram, not pushed): `dcd2dfc` engram-qa feedback row.

## Verification state
- Merge `bb062838`: `/engram-review code deep` (lead · lifecycle axis · codex gpt-6.1-sol xhigh) PASS. `/engram-qa` run 1 FAIL (claude lane c2 `pending` — pre-existing test race), fix `6c34aac9` reviewed (full, lead + codex PASS); QA run 2 on tree `a1fce8a6` (= `6c34aac9`'s tree) PASS: full regression (Rust 5824 · frontend 2762 · tsc · gates 31/31), claude lane, shell real-daemon lane incl. cross-version stop vs a real v0.3.2 daemon (RAN), mutation 3/3, GUI release build (spawn + output, `daemon_shutdown` closed in 716 ms, restart + reconnect). codex lane PASS on `bb062838` (0.167.0).
- Doc commits `8dcb9dfb` `bc93f49f` `9ee5db64`: not reviewed (tracking/step-log entries and a "closed" note — low claim density). `a9f3eb12` = reviewed (doc full, 3 rounds, final PASS). ADR-0357 body: worker-filled from the record, main fact-checked against `src/launch.rs` — not reviewer-reviewed.
- **CI not checked** for the pushed SHAs (last known: run `42782044193` for `8dcb9dfb` in progress). Check: `gh run list -b v0.5.0/feat/preset-library -L 5 --json headSha,status,conclusion`.
- Rerun: `cargo test --workspace -- --test-threads=4`; lanes per `.claude/skill-bindings/qa.md`.

## Failed approaches (do-not)
- Do not trust T-number facts from a handoff when merging master — both sides had used T-81..T-83 (master's handoff and ours each claimed only some). Scan both sides from the merge base before the merge.
- `python` on this PC is the Microsoft Store stub (prints "Python", does nothing) — use `py`.
- `git status --porcelain | awk | xargs git add` breaks on Korean ADR filenames (octal-quoted) — use `git add -u -- <dirs>`.
- Do not use `git -C /i/Engram add -- core` for a feedback row — add the one file by path.
- QA GUI with a release build + daemon restart: the app-started daemon ignores `ENGRAM_DATA_DIR` and uses `<exe dir>\data`; the QA runner bridged it with a junction (feedback filed, 5th occurrence).

## Decisions
User (2026-11-10):
- 업데이트 범위 · 상황: codex 0.163.0→0.167.0 available, master had S21 transport (35 commits, 8 conflicts) · 선택지 둘 다 ★ / master만 / codex만 · 고른 것 「둘 다 (권장)」 · 같음
- (묻지 않음) push · 고른 것 「푸쉬는 알아서 하고」 · — 
- (묻지 않음) merge · 고른 것 「병합은 알아서 하고」 · 근거: 「net crate를 지운걸로 알고있음」 · —
- (묻지 않음) doc decisions · 고른 것 「문서 수정은 알아서해.」 · — (main then put the same-name wording into PRD R57 + ADR-0355 "supplements R57"; ADR-0337 evidence kept with an appended update; ADR-0215 revival trigger kept size-only)
- 피드백 대상 · 상황: the CLAUDE.md 「최소한만 막기」 follow-up sentence was pending · 선택지 내 질문 방식 / CLAUDE.md 문구 자체 · 고른 것 「CLAUDE.md 문구 자체」 · — · 조건: record as feedback, do not edit CLAUDE.md now → `docs/tracking.md` T-100 (트리거 대기 — next CLAUDE.md revision)
- ADR-0357 push timing · 상황: other sessions active, ADR-0357 unpushed (number claim) · 선택지 push now ★ / wait · 고른 것 「알아서 해. 좋은 방향으로.」 · — (main pushed)
- (묻지 않음) codex version policy questions — explained only: no version blocks launch; minimum-version feature gates; no upper bound (risk noted, not tracked yet).
Main:
- Renumber ours (T-81..83 → T-94..96), not master's — master's numbers already landed.
- Review intensity of the merge raised light → deep (auto-merged daemon connection core where S21 stop meets S25 spawn = lifetime risk).
- c2 failure classified as a pre-existing test race (diagnostics 3/3 overlap on base and HEAD); fixed the harness (appearance-drain wait), product `pending` semantics unchanged (spec §6 ㉯).
- Pre-existing review findings recorded, not fixed: T-97 (per-connection `stopping` latch), T-98 (in-flight create outlives the instance guard), T-99 (spawn during stop leaves a failure mark — not refused per 「최소한만 막기」).
- ADR-0337 decision 3 → partial supersede ADR-0357 (three step-6 codes state facts only); ADR-0216 needs no impact note.
- Proposed but not done: track "codex has no upper version bound / update modal on terminal start" (user did not answer; not in tracking).
Open: step 7 child TRD yes/no; master re-merge; step 5 confirmation; ADR-0357 not reviewer-reviewed.

## Improvements
- Filed: engram-qa feedback — release-build daemon restart data-folder recipe gap, 5th occurrence (Engram `dcd2dfc`).
- Report only (project instruction, no feedback file convention): `docs/handbook/branch-commit.md` / parent TRD §5-1 step 0 could say "before merging master, diff T-numbers and ADR numbers added on both sides since the merge base" — this session's first merge run stopped on a T-81..83 collision.

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 fetch -q && git -C I:/Engram/apps/engram-dashboard-wt1 log --oneline -3 && git -C I:/Engram/apps/engram-dashboard-wt1 rev-list --left-right --count origin/master...HEAD`

## References
- Parent TRD `docs/process/S25-preset-library/trd.md` §5-2 step 7 (+ "Handed over by step 5/6"), §1-4 asset-library; child TRD `trd-codex-translation.md` §8-1
- `docs/decisions/0353-*` · `0354-*` · `0355-*` · `0357-*`
- `docs/tracking.md` T-87..T-103
- Previous handoff `history/20261110-132008-s25-step6-landed.md` (step-6 code ledger in `attachments/20261110-s25-step6-code/`)
