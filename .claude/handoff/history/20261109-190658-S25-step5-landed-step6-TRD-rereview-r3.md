# Handoff: S25 step 5 landed (user confirmation deferred); step 6 TRD Draft 4 in re-review round 3 — then code P1→P2→P3

## Know first
- **Re-review round 3 of the step-6 TRD is RUNNING at save time** (lead · risk-axis · codex, delta `cc546b14..f444dd36`, brief `.review-s25-6trd/rereview-r3.md`). Results land after this document. If the session ended, re-run them (same brief; codex = new thread is fine).
- **Step 5 user confirmation is deferred** — the user said 「내 확인을 하면 되긴하는데 나중에 확인해야될것같아서 지금은 사정상 안됨」 and then 「진행해」 for step 6. Step 5 is committed; ask for the confirmation when the user is available (material: commit `baf769ff` body + QA result below).
- User style correction this session: no translationese / AI tone in chat or app wording (「싣다/실리다」 etc.) — 「싣지 않습니다? 뭐 자꾸 ai 어체 쓰는데 너 이거 몇번째야?」. Filed (Engram `de1734d`).
- User wants questions expanded in plain terms (「뭔소리야 펼쳐봐」) and work to continue on independent branches while one item waits (「막힐때 펼치고 하던건 계속하면」). Filed for engram-review (Engram `8ee61b2`).
- A test thread from the step-6 P0 measurement remains in the user's real `~/.codex` (sessions/2026/11/09/rollout-…01a1bf6b….jsonl, preview "Use the $dup skill now …") — told the user; deletion is theirs.
- No push (user: 「푸쉬없이」). Line-ending noise in `src-tauri/bindings/*.ts` — never stage.

## One-line state · next first action
Step 6 TRD is at Draft 4 (`f444dd36`) after review rounds 1–2 (round 2: lead PASS, axis PASS, codex FIX → fixed in Draft 4). **Next:** collect round-3 results → if PASS, add a review-pass line to TRD §11 and commit → code P1 (port codex impl + `-c` guard/budget + `toml` dep) via /engram-implement §2 with a fresh coder (brief from `docs/process/S25-preset-library/trd-codex-translation.md` §8 P1 + §4-1..4-3 + §7-1 P1), review deep, then P2, P3, QA, commit, doc pass.

## Repo state (branch v0.5.0/feat/preset-library · wt1)
- HEAD `f444dd36` + this handoff commit; ahead of origin by 222 (nothing pushed). Working tree clean except bindings noise and the review folders `.review-s25-5p1/` `.review-s25-6trd/` (excluded via `.git/info/exclude`, untracked, disposable).
- Commits this session: `0badd007` step 20 code · `a3a25be6` step 20 docs · `b631515e` engram-help English · `baf769ff` step 5 code · `829f920a` step 5 docs (T-82..84) · `e77d4f68`+`1e36176d` ADR-0351 (D7) · `21bc0063`/`a96a8b46`/`cc546b14`/`f444dd36` step 6 TRD drafts 1–4 + research.

## Verification state
- Step 5 QA PASS on tree `ee694de7` (= `baf769ff`): `cargo test --workspace` 5629 · frontend 2762 · tsc · 34 gates · bindings sync · claude lane (control_send 5, usage_probe 5) · codex lane (usage_probe 2, backend_contract 11) · mutation 5/5 red · GUI smoke on a release build (claude agent spawned with Noop argv, answered `QA_PONG_5`, killed). Real-material path (settings/`--add-dir`/`--plugin-dir`) is test-only until step 8.
- Step 6: no code yet. Measurements M1–M9 in `docs/research/codex-launch-materials-measurements-2026-11-09.md` (codex-cli 0.163.0).
- Rerun: `cargo test --workspace -- --test-threads=4`; QA procedure `.claude/skill-bindings/qa.md` + `qa-verify.md`.

## Failed approaches (do-not)
- Do not stop the whole pipeline when the review rule 「같은 부류 두 라운드 연속 → 사용자 선택지」 fires — pause only that class; keep independent pieces running.
- Do not chase claude's exact folder-loading rules case by case — policy is "clear cases only, the rest `unsure`" (step 5).
- Do not edit an ADR body to record a new decision (README rule 3) — use /engram-adr (ADR-0351 was made that way after reverting an in-place edit).

## Decisions
User (this session):
- U2 (step 20) codex `agent.spawnInto` deadline · 상황: healthy creates 0.71–1.19 s, 9.7 s not reproduced · 선택지 keep 7 s ★ / longer spawn deadline · 고른 것 「7초 유지」 · 같음.
- engram-help translation · 상황: runtime text agents read, S33 kept string literals Korean · 선택지 keep Korean ★ / translate · 고른 것 「영어로 번역」 · 갈림.
- Front-matter types placement (step 5 P1 hold) · 선택지 neutral port file ★ / per backend · 고른 것 「중립으로 하고 구현 진행해」 · 같음 · 근거: 「어느정도 타협 + 해당 내부 구현은 각 전용」.
- D7 settings write failure · 선택지 fail spawn ★ / warn+continue · 고른 것 「실행 실패로 막기」 · 같음 → ADR-0351.
- Step 6 before step 5 confirmation · 선택지 proceed ★ / wait · 고른 것 「진행해」 · 같음.
- U1 (step 6) priming that cannot go on the codex terminal command line · 선택지 block ★ / warn and launch without · 고른 것 「띄우지 말아」 · 같음.
- U2 (step 6) same-name skills across roots · 선택지 block / launch + notice ★ / silent · 고른 것 「같은 이름이 여러곳있으면 띄우되 알려.」 · 같음.
- U3 (step 6) notice wording · 고른 것 「그렇게 하던지」 (after rewording without 「싣다」) · wording in TRD §4-9.
Main: step 5 decisions D1–D11 (TRD "Step 5 landed"); preview policy A (clear cases only, `unsure` otherwise — user informed, no objection); step 6 decisions #1–#17 and rulings C1–C4 (TRD §9-1/§9-2), incl. #15 resume `developerInstructions` on every resume (≥ 0.155.0) fixing a pre-existing defect (resumed codex JSON threads lose start instructions at compaction — M8).
Open (user): QA binding — add `crates/engram-dashboard-platform/src/shell.rs` to the claude lane paths (recommended; asked twice, explained, no answer yet); run the codex lane on codex CLI version change (axis suggestion; not yet asked as a choice).

## Improvements
- Filed: engram-review — pause only the repeating class (`8ee61b2`); global — no AI tone in chat/app wording (`de1734d`).

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 log --oneline -3 && git -C I:/Engram/apps/engram-dashboard-wt1 status --short | grep -v src-tauri/bindings`

## References
- `docs/process/S25-preset-library/trd-codex-translation.md` (step 6 TRD, Draft 4) · `docs/research/codex-launch-materials-measurements-2026-11-09.md`
- `docs/process/S25-preset-library/trd.md` §5-2 step 5 「Step 5 landed」, step 6 entry + hand-over
- `.claude/handoff/attachments/20261109-s25-step6-material.md` (scout material for step 6 — spec excerpts with TRD line numbers, code state)
