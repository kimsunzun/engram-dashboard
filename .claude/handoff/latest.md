# Handoff: S25 step 7 — P4 (rescoped, ADR-0359) reviewed + QA'd, UNCOMMITTED; waiting on one user decision (pre-existing codex lane failure); then commit P4 → P5

## STOP — ask the user first (do not commit P4 before the answer)
**Pre-existing failure decision (implement flow §4 — must be asked even in autonomous mode):** QA's codex lane fails `backend_contract` q4 / q11 / q12 ("codex: 시작 화면을 45s 안에 못 지났다 … the startup-done signal never came"; q11 once showed codex's own `Error: app server did not become ready … timed out probing app-server control socket … rerun with --no-daemon`). Triage: with all 11 tests under `--test-threads=4` the **baseline `70c35894` fails the same 3 at the same asserts** (`backend_contract.rs:1720:9`, `:2632:13`) → labelled `FAIL(선재)` (caveat: the `모달=` field was `Passed` on baseline vs `NotShown` on P4 — treated as a run-varying value; strict reading = unlabelled, but it reproduces on baseline either way). Which tests fail varies run to run (3, 3, 1). Run alone (3 tests) they pass. Related records: tracking **T-56** (same file, codex stuck on "loading", pre-existing, cause unknown — but T-56 says they fail even one at a time) and step-log `:2201` (S31: same app-server socket mechanism, `SUN_LEN`). The changed P4 code is not on that path (argv has no `-c` lines; the delta was `launch.rs` notice text).
Question to put to the user (one line each, recommend ★):
- ★ Skip for P4 and record it: append to T-56 (or a new T-entry) "fails when 11 run under 4 threads; passes when 3 run alone; app-server readiness timeout seen (`--no-daemon` hint)"; commit P4 with the label in the body.
- Fix now: harness change (longer start wait or `--no-daemon` for these tests) — a separate small unit before P4's commit.
After "skip": commit P4 (see Resume), push, then continue to P5.

## Know first
- **Working tree = P4, uncommitted, gated** (code tree `7bd62bc7` = HEAD `8c93c881` + 12 files). Backup of exactly this tree: local branch `v0.5.0/wip/s25-p4-final-pending` (`27543d0e`, unpushed). The old schema P4 is parked on local branch `v0.5.0/wip/s25-p4-schema-validate-parked` (`002cd9f9`, unpushed). Neither is to be merged.
- `src-tauri/bindings/*.ts` (~45) line-ending noise — never stage.
- Codex threads left in the user's real `~/.codex` by this session's QA (deleting is the user's): `01a1ca2a-9a73-7263-9784-3631b2d761a6`, `01a1cbe5-124a-7993-b288-facabe32db77` (plus older ones listed in previous handoffs).
- The user went to sleep with 「쭉 진행해. 그냥」 (autonomous). Report at wake-up: the decision above, and today's decisions below.
- Deviations to report (already told the user, keep for the record): the step-7 TRD amendment rounds raised a class of timing findings two rounds running → raised to the user as a class choice (they chose "requirement + ordering rule; per-entrance values in step 8"); a P4 coder re-ran a classifier-denied `git checkout` after self-verifying the backup (no harm; recorded in global feedback).

## One-line state · next first action
Step 7 = new crate `engram-dashboard-asset-library` in 9 units. P1–P3 committed (`42cede81`, `d8bd75db`, `1246253d`). **P4 rescoped by ADR-0359 is done and gated except the codex-lane pre-existing failure decision.** Next: ask the user (above) → commit P4 → push → P5 store/scan/catalog/refs → P6 folder IO (deep; now 60 s / 2 s per ADR-0360) → P7 service+gate (deep) → P8 builtin+resolve → P9 launch (deep) → step-7 doc pass. Step 7 is *Stop (after)*.

## Repo state (branch v0.5.0/feat/preset-library · worktree wt1)
- HEAD `8c93c881` + this handoff commit; pushed up to `8c93c881` (upstream 0/0 before this commit — this commit is unpushed until the next push); vs origin/master: 0 behind / 193 ahead.
- Uncommitted (P4, gated, NOT committed): `Cargo.lock`; agent `Cargo.toml`, `src/backend/settings_check.rs`, `src/backend/claude/settings.rs`, `src/backend/codex/settings.rs`, `src/backend/codex/cmdline.rs`, `src/launch.rs`; asset-library `Cargo.toml`, `src/lib.rs`, `src/backends.rs`, `src/format/frontmatter.rs`, `src/validate.rs` (new).
- This session's commits (since `b53ef8ba`): ADR-0359 (`ead83a79` scaffold, `062eb4dc` body, `a8906754`…`c305a7a2` partial links to 0334/0336/0323/0331/0324/0311/0314, `38b8413f`, `02022f7a` 0334 clause widened); TRD draft 9 `4a173811` + review fixes `2dc83bbe` `7f3dd8f6` `30491ace`; HTML clarifications; ADR-0360 (`69e931c4` scaffold, `483a14bc` body, `30c8be6d` confirmed); TRD draft 10 `fa386bbf`, draft 11 `ac99be15` `7bfa9f8f` `16241706` `70c35894`; draft 12 `8c93c881` (B29 notice withdrawn).
- `.review-s25-d9/` (excluded via `.git/info/exclude`) holds briefs, diffs, the P4 TRD extract and QA/review briefs — reusable patterns.

## Verification state
- **P4 code review (deep):** lead PASS (r1 PASS w/ 권장 → r2 PASS → r3 PASS on the notice removal), risk-axis PASS (no kept launch behaviour changed: claude merged settings byte-identical, no codex `-c` refusal dropped, notice texts unchanged, argv goldens untouched), codex FIX (must: `check_settings` kept a port-given line) → fixed and confirmed by lead (both raised it → Claude-only confirmation per rule). Unfixed batched: test name `check_skill_runs_the_port_with_the_library_parser` promises more than it proves (doc says so) — rename in a later unit.
- **P4 QA:** run 1 on tree `06680519` (with the notice): PASS — Rust 5943, frontend 2807, tsc, gates 33/33 incl. license notice (`toml` dropped from agent), claude lane 10/10, codex lane 16/16, mutation 4/4. Run 2 on tree `7bd62bc7` (notice removed): build/backend 5942/gates 33/33/frontend 2807/claude lane PASS; codex lane `backend_contract` 8/11 → **FAIL(선재)** per triage above; mutation 해당 없음 (no tests added in that delta). lanes.tsv: claude = `8c93c881 +uncommitted`; codex left at `70c35894 +uncommitted` (lane failed).
- TRD: draft 9 reviewed PASS (full: lead + codex, 3 rounds); drafts 10–11 reviewed (full) — FIX rounds then no must; draft 12 (B29 revert) doc-only, not reviewed.
- CI: last checked green for P1–P3 commits; not checked for the doc pushes of this session (`gh run list -b v0.5.0/feat/preset-library -L 5 --json headSha,status,conclusion`).
- Rerun: `cargo test --workspace --locked -- --test-threads=4`; lanes per `.claude/skill-bindings/qa.md`.

## Failed approaches (do-not)
- Do not state CLI behaviour from web issue posts before grepping `docs/research/` — the 10-17 spike (`docs/research/settings-asset-validation-spike-2026-10-17.md`) measured claude silently discarding a bad `--settings` object; an issue-based claim ("claude warns") reached the user and ADR-0359 and had to be corrected.
- Do not let a worker run destructive working-tree resets (`git checkout HEAD -- …`); do such prep in main after confirmation.
- Do not design per-entrance launch timeout numbers in the step-7 TRD — step 8 owns them (ordering rule only).
- Heredocs mangle backslashes; `python` is the Store stub — use `py` and the Write tool.

## Decisions
User (2026-11-11/12):
- (묻지 않음) 스키마 검사 · 상황: P4 schema validate built (9 review rounds); claude silently drops a whole bad `--settings` object (measured 10-17), merged file ⇒ one bad asset drops all settings · 선택지 block (ADR-0332) / warn only / block permission files only / claude only / drop all ★(after "claude rules") · 고른 것 「우리는 잘 조합해서 넘기는 것에 집중하자. 문제 생기면 codex claude가 알아서 그쪽룰로 처리하게.」 and, after the correction, 「셋팅 잘못해서 넘어가는건 기존 클로드에도 있던 버그잖아. 그런거 말고 조합을 잘못해서 문제라던가 그런것 위주로 봐야지」 · 「클로드나 코덱스 독립적으로 하는거 말고 우리가 해서 문제가 되는부분만 체크로하면 될것같음. 진행해.」 · 갈림 (Claude had recommended C after the correction) → ADR-0359 (supersedes 0332)
- 8 B25 참고 1초 · 상황: advisory wait only bites on a dead share · 선택지 keep ★ · 고른 것 「어쩔 수 없는거 아님?」 · 같음 (later 2 s by ADR-0360)
- 9 B29 · 상황: cwd canonicalize timeout · 선택지 proceed ★ / block · 고른 것 proceed + 「실행을 시킨쪽에 알려야되는거 아님?」 → later withdrawn: 「뭔지 이해안된다. 나 자야되니 A로 너가 해」 (A = log only) · 같음 (A ★)
- 11 B31 · 선택지 keep ★ / check removes / watcher · 고른 것 「일단 A로 하자. 복잡하게 하지 말고 나중에 기능 추가하는 식으로.」 · 같음 → T-104
- 12 step 5 confirmation · 고른 것 (요지) nothing to decide → main closed as approved
- 6/7/10 class A/B, B30 · 고른 것 (요지) reviewed when shown, no objection
- 실행 대기 시간 · 상황: slow PC / sleeping NAS fails launches under 5 s; unattended agents · 선택지 keep 5 s / launch-only longer ★ / async reply · 고른 것 「시간을 늘려.」 「시간은 60초로 하던지.」 · 같음 → ADR-0360; 설정은 나중 「어 나중에 한다고 치고」; base = later settings-file loader 「파일 읽어노는 구조? 자체는 base에 있고 … 어쨋든 파일로 빠지는거잖아.」 → T-105
- 타이밍 클래스 · 선택지 requirement + ordering rule in step 7, values in step 8 ★ / all numbers now · 고른 것 「알아서 해 어쨋든 나중에 결국 다 적용되는거잖아.」 · 같음
- codex 셸 도구 바깥 대기(N5) · 고른 것 「너가 알아서해. 가장 좋은 방향으로.」 → main: step 8 measures it
- 휴지통 규칙 피드백 · 고른 것 「이거 수정은 몰아서 할거야 다른거랑 합쳐서.」
Main: P4 coder plan (one coder, sequential); B32 remove `raw_to_json` + port `merge` (+ agent `toml`); launch values in agent `types.rs` (85 s reply bound, 88 s caller wait); shell link follows ADR-0181 (raise the shared timeout in step 8); review intensities (TRD full, P4 code deep); `pub mod validate` accepted.
Open: the pre-existing codex-lane failure decision (top).

## Doc pass owed at the end of step 7 (collected, add to the previous list)
- Previous list (ADR-0315 amendment, ADR-0357 amendment for the four codes, TRD fixes, stale `cwd_reaches_codex_unchanged` location, step-log for S36 merge and step-7 units, P3 follow-ups, P4/P6 brief notes) still stands — see history `20261111-122129-*`.
- New: step-log entries for this session (ADR-0359/0360, TRD drafts 9–12, P4); TRD §3-6 line ref `split_config_line` moved to ~`codex/settings.rs:303-324`; rename the overclaiming `check_skill` test; T-56 update per the user's answer.

## Improvements
- Recorded (global feedback, Engram `92d3497`, `6e23bf6`): deletion rule sends regenerable junk to the recycle bin (user batches it with others); worker re-ran a classifier-denied destructive command.
- Report only: CLAUDE.md 「관례를 먼저 확인한다」 should also cover external CLI behaviour (grep `docs/research/` first).

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 status --short | grep -v bindings/ && git -C I:/Engram/apps/engram-dashboard-wt1 log --oneline -3`
To commit P4 after the user's answer: stage exactly the 12 P4 paths above (never `src-tauri/bindings/*.ts`), verify `read-tree HEAD` + `add -A -- crates Cargo.lock` + `write-tree` = `7bd62bc7…`, commit `S25: feat(asset-library,agent): step 7 P4 — hand-over checks; schema-only settings-check members removed (ADR-0359)` with review/QA/label in the body, push.

## References
- Child TRD `docs/process/S25-preset-library/trd-asset-library.md` (draft 12) — §3-6, §3-9a (P4), §3-5 (P5), §3-8 + §6 (P6/P9, ADR-0360 values), §12 hand-over incl. the step-8 entrance table; overview `trd-asset-library.html`.
- ADR-0359 · ADR-0360 · ADR-0341 · ADR-0334 · ADR-0357 · ADR-0181.
- Briefs/patterns: `.review-s25-d9/p4-coder-brief.md`, `p4-review-brief.md`, `p4-qa-brief.md`, `p4-trd-extract.md`.
