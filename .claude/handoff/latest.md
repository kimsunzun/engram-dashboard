# Handoff: S25 step 9a — master S38 merged, U2 and U3 landed; U4 coded and reviewed but stopped at the review-round cap (user question) — next = ask the U4 question, then QA + commit U4 and freeze 9a

## Know first
- **One user question blocks U4** (ask it first, in plain words — what the user sees):
  - Situation: relink reports each old→new pair it "fixed". If an agent attached to the old preset is deleted by another request at the same moment as the relink, the pair can be reported fixed although no agent changed. Disk and registry stay correct; only the answer's `fixed` list is off, in a rare race. Raised by codex in U4 re-review round 4; the lead reviewer called the same area negligible.
  - Why it is a question: U4 used all 3 fix rounds (implement flow §3 cap) and the `fixed`-accounting class produced a new case two rounds in a row (review flow §4) — both rules say stop and ask, even in autonomous mode.
  - Options: **A ★ accept as a known edge** — record it in `docs/tracking.md`, land U4 as is (QA → commit). **B fix** — count a pair's agent half from the port's answer (the changed agent ids ∩ the agents that attached the pair in the in-change registry read); small change in `relink.rs` + a test; then one more codex re-review, QA, commit.
  - ★ reason: reporting-only, needs a concurrent delete inside a relink; B is cheap but reopens the same accounting area a fourth time.
- U4 is **uncommitted and gate-unpassed** (code review stopped at the cap; QA not run). Do not commit it before the answer. Files: `crates/engram-dashboard-asset-library/src/{relink.rs,create.rs}` (untracked, new), modified `agent_edit.rs`, `catalog.rs`, `edit.rs`, `launch.rs`, `lib.rs`, `store.rs`, `docs/reference/structure/asset-library.md`. Main's batched wording fixes for U4 are already applied in the tree (relink header order, skip/stop docs, a `relink stopped` warn log, new_file leftover note); `cargo check` + fmt + Hangul clean after them; full test not rerun after the wording edits.
- Autonomous mode (default); the user went to sleep and said 「자율모드 ㄱㄱ」.
- Untracked `.claude/explain/` is not ours. `*/bindings/*.ts` line-ending noise — never stage (restore with `git checkout -- <paths>`; content-identical).
- Worker coder for U4 is at ~460k context — if B is chosen, spawn a fresh coder with `.review-s25-d9/s9a-u4-progress.md` + `fix-r3.md` + the codex round-4 finding instead of resuming.

## One-line state · next first action
Master S38 (English internal text + Hangul CI gate) merged into the branch (`76546226`); 9a U2 (`2fc21c12`) and U3 (`6aae4b76`) committed, pushed, CI green; ADR-0384 (9a coding-time decisions, partially amends ADR-0380 d3) committed and pushed. **Next: ask the U4 question above; then (A) `/engram-qa` for U4 → commit → 9a freeze, or (B) fix → codex re-review → QA → commit → freeze.**

9a freeze commit (implement flow §5, binding): put `Implemented` on the step-9a TRD status line (`docs/process/S25-preset-library/trd-library-edit.md`) and update the parent `trd.md` item 9a status line; do not rewrite TRD bodies. Move any still-open decisions to `docs/tracking.md`. Record U3's unfixed 권장 if the user says "later" (see below). Hand to 10a in the 10a item of the parent TRD: `roundtrip_smoke` does not plug the profile-edit port (needed once 10a routes agent-preset commands there); TRD §3.6 sentence 「`stopped: {code}` with no `path`」 is superseded by ADR-0384 d8.

## Repo state (branch v0.5.0/feat/preset-library · worktree wt1)
- HEAD `22ff0b21` (ADR link) — 0 ahead / 0 behind origin at save time (this handoff commit will be ahead until pushed).
- This session's commits: merge `76546226` (master S38 → branch; 150 conflicts resolved, our Korean code text translated, our T-114 → T-115), `fe1ca9ba` (tracking T-116 — drop the CI Hangul check later, user note), U2 `2fc21c12`, U3 `6aae4b76`, ADR-0384 `14de5b4a` / `77dabbaa` / link `22ff0b21`.
- Uncommitted: U4 (above).
- Engram (not pushed): feedback commits `8be2347` (handoff: CI-pending claim when a doc-only push starts no CI), `67d97c6` (qa: lock check reads another session's LogonUI), `3727cde` (implement: coding-time decisions committed without the ADR the binding requires before each piece).
- `.review-s25-d9/` (git-excluded) holds all briefs, diffs and reports: `m38-*` (merge), `s9a-u2-*`, `s9a-u3-*`, `s9a-u4-*` (incl. `s9a-u4-r1/` review rounds, `fix-r1..r3.md`, progress files).

## Verification state
- Merge `76546226`: QA PASS (workspace 6300, frontend 2884, gates 34/34, claude + codex lanes, mutation 1/1, GUI smoke); CI green.
- U2: review deep 3 rounds (round 3 PASS ×2); QA PASS (workspace 6345, lanes, mutation 2/2); CI green.
- U3: review deep 2 rounds (round 2 PASS ×3); QA PASS (workspace 6364, lanes, mutation 2/2); CI green.
- U4: coder-run tests asset-library 495 passed, workspace 6401 passed (before main's wording edits). Review: lead PASS (round 3), risk lens PASS (round 2), codex FIX (round 4 — the question above). **QA not run. Gate unpassed.**
- Rerun: `cargo test --workspace -- --test-threads=4` (detached runner per `.claude/skill-bindings/qa.md`).
- T-115 (codex Q10 flake) did not fire in any of the three QA runs; the user wants it fixed the next time it fails.

## Failed approaches (do-not)
- Writing a heredoc with 「」 / backticks inside `$(...)` in Bash for long docs — it broke (`unexpected EOF`); use the Write tool.
- `rm -rf` is blocked — use `bash ~/.claude/references/recycle.sh` or a new scratch folder.
- Counting Hangul on the branch's own files to size the merge — it counted pre-S38 Korean master had already translated; count on the merge-tree result instead (`git archive <merge-tree>` → `node scripts/check-hangul.mjs .`).

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 log --oneline -3 && git -C I:/Engram/apps/engram-dashboard-wt1 status --short | grep -v bindings`

## Decisions
User (2026-11-18 ~ 19):
- master 업데이트 반영 · 상황: master had S38 (57 commits, 937 files), branch behind · 선택지 지금 받기 / 나중 · 고른 것 「업데이트 받고 쭉 진행해. 이번에 저쪽에서 영문번역 한것들 있거든.」 · — .
- T-115 수정 시점 · 상황: codex Q10 flake (stale parent PID), test-only · 선택지 지금 / 다음에 걸리면 ★ · 고른 것 「다음에 걸리면 수정할꺼 고려하자.」 · 같음.
- (묻지 않음) CI 한글 검사 · 고른 것 「ci에서 한글을 체크하는건.. 초반에만 좀 적용하다가 나중에 배자.」, 「그냥 나중에 이런게 있었지 빼자. 정도의 느낌」 · — · recorded as tracking T-116.
- 자율 진행 · (묻지 않음) · 「나 잘테니깐 자율모드 ㄱㄱ」.
Main:
- Merge: two workers by file area; our code kept, master English taken in conflicts, our Korean translated + pruned; our T-114 renumbered T-115; `capabilities.json` staged as the generator writes it (trailing newline only).
- U2 review: a lead-reviewer hold on the check-answer shape resolved by conforming to the parent TRD (reviewer confirmed dissolved). U3: two holds dissolved on re-judgement (detach raw text fixed; shell-backend edits stay applied per 「최소한만 막기」). U4: self-cycle → preset-cycle skip (codex withdrew hold); lasting IO failure keeps the TRD stop rule, wording fixed (lens withdrew hold).
- ADR-0384 records the 9a coding-time decisions (late — the binding wanted it before each piece's commit; feedback `3727cde`).
Unfixed 권장 to show the user (ask "고칠 것 있나요?"; "later" → tracking): U3 — every agent-preset edit stats the root even with no library preset (slow share → up to 5 s); the second-plug branch untested; `roundtrip_smoke` does not plug the port (10a). U4 — `create_new` crash residue / leftover file on a failed write (accepted form); part (b)'s missing-target test is item-only.
Open: the U4 question (user).

## References
- TRD `docs/process/S25-preset-library/trd-library-edit.md` (draft 8) §3.3–§3.6, §7, §9; parent `trd.md` items 9a / 10a.
- ADR-0384 (9a coding-time rules), ADR-0380, ADR-0383 (Hangul gate).
- U4 review trail `.review-s25-d9/s9a-u4-r1/` (`codex-r1/r2.md`, `lens-r1/r2.md`, `lead-r1.md`, `fix-r1..r3.md`, `fix-r1/r2-report.md`); progress `.review-s25-d9/s9a-u4-progress.md`.
- Bindings `.claude/skill-bindings/implement.md` (ADR timing, `Implemented`), `review.md`, `qa.md`.
