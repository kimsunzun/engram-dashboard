# Handoff: S25 step 6 TRD review passed (Draft 5) — next = code P1 (codex settings-check port); step 5 user confirmation still deferred

## Know first
- **Nothing is running.** All reviews landed; tree clean except `src-tauri/bindings/*.ts` line-ending noise (never stage) and the excluded, disposable review folders `.review-s25-5p1/`, `.review-s25-6trd/`.
- **Step 5 user confirmation is deferred** (user: 「내 확인을 하면 되긴하는데 나중에 확인해야될것같아서 지금은 사정상 안됨」, then 「진행해」 for step 6). Ask when the user is available — material: commit `baf769ff` body (QA PASS list) and the trd.md "Step 5 landed" block.
- Style: no translationese / AI tone in chat or app wording (「싣다/실리다」 …) — user correction 2026-11-09. Expand questions in plain terms; keep independent work running while one item waits.
- auto-handoff hook now says: don't save at 55%, save once at the next hand-over point (Engram `c29064a`, user 「훅 문구를 바꾸면 되는거 아님? 고쳐.」). Not verified in a live session yet — this save is the first under it.
- A test thread from step-6 measurements remains in the user's real `~/.codex` (preview "Use the $dup skill now …") — user informed; deletion is theirs.
- No push (user: 「푸쉬없이」).

## One-line state · next first action
Step 6 design is final: `docs/process/S25-preset-library/trd-codex-translation.md` Draft 5, review passed (deep, rounds 1–4, commit `a557bae5`). **Next:** /engram-implement §2 — spawn a fresh coder (worker-senior) for **P1** = settings-check port codex implementation + `-c` guard/budget (`backend/codex/settings.rs`, `backend/codex/cmdline.rs`, additive `backend/settings_check.rs`, `platform::shell::text_is_cli_safe`, registry → 2 slots, `toml` 1.1.2 dep, the no-token `codex.cmd` shim-line measurement that keeps or zeroes `SHIM_MARGIN`). Brief from TRD §8 P1 + §4-1..4-3 + §7-1 P1 + §9-1. Then `/engram-review code` (deep), then P2 (shared + terminal translation), P3 (JSON channel incl. the pre-existing resume fix), QA (full regression + codex lane + live run L1–L5), commit per piece, doc pass (TRD §8-1 hand-over into trd.md; ADR candidates (a)–(e) via /engram-adr — ADR-0215 d4/d5/d7, ADR-0337, ADR-0226/0231 note; PRD R37 wording; parent T9 reword).

## Repo state (branch v0.5.0/feat/preset-library · wt1)
- HEAD `a557bae5` + this handoff commit; ahead of origin by 225 (nothing pushed). Working tree clean (bindings noise only).
- Since the last handoff: `d502f97f` TRD Draft 5 · `a557bae5` review passed. Earlier this session: `0badd007`/`a3a25be6` step 20 · `b631515e` engram-help English · `baf769ff`/`829f920a` step 5 · ADR-0351 (`e77d4f68`, `1e36176d`) · TRD drafts `21bc0063` `a96a8b46` `cc546b14` `f444dd36`.

## Verification state
- Step 5 QA PASS on `baf769ff` (full regression 5629 Rust · 2762 frontend · tsc · 34 gates · claude + codex lanes · mutation 5/5 · GUI smoke). Real-material path is test-only until step 8.
- Step 6: design only — no code. Measurements M1–M9: `docs/research/codex-launch-materials-measurements-2026-11-09.md` (codex-cli 0.163.0).
- Rerun: `cargo test --workspace -- --test-threads=4`; QA procedure `.claude/skill-bindings/qa.md` + `qa-verify.md`.

## Failed approaches (do-not)
- Command-line accounting kept producing new cases (quote expansion → program token → env PATH) — settled by a fixed bound (`PROGRAM_TOKEN_BOUND = 262`, #17) and R18 as a recorded exception; do not reopen with exact path resolution.
- Do not stop the whole pipeline when one review class repeats; pause that class only.
- Do not record a new decision by editing an ADR body — use /engram-adr.

## Decisions
User (step 6, 2026-11-09):
- U1 priming that cannot go on the codex terminal command line · 선택지 block ★ / warn and launch without · 고른 것 「띄우지 말아」 · 같음.
- U2 same-name skills across roots · 선택지 block / launch + notice ★ / silent · 고른 것 「같은 이름이 여러곳있으면 띄우되 알려.」 · 같음.
- U3 notice wording · 고른 것 「그렇게 하던지」 (after rewording) · TRD §4-9.
- `toml` crate addition · reported, no objection.
- auto-handoff hook wording · (묻지 않음) · 고른 것 「훅 문구를 바꾸면 되는거 아님? 고쳐.」 · 근거: 「기준은 세션이 딱 바뀔때 한번 하면 되는건데」.
Earlier user decisions this session are in history `20261109-190658-…`.
Main (step 6): #1–#17 and rulings C1–C4 (TRD §9-1/§9-2), incl. #15 resume `developerInstructions` (≥ 0.155.0) — fixes a pre-existing defect: resumed codex JSON threads lose start instructions at compaction (M8); name it separately in the P3 commit body.
Open (user): QA binding — add `crates/engram-dashboard-platform/src/shell.rs` to the claude lane paths (recommended; explained, no answer); run the codex lane on codex CLI version change (risk-axis suggestion; not yet put to the user as a choice).

## Improvements
- Filed this session: engram-review pause-only-the-class (`8ee61b2`) · global no-AI-tone (`de1734d`) · auto-handoff timing (rows applied by `c29064a`, archived).

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 log --oneline -3 && git -C I:/Engram/apps/engram-dashboard-wt1 status --short | grep -v src-tauri/bindings`

## References
- `docs/process/S25-preset-library/trd-codex-translation.md` (step 6 TRD — §8 units, §9 decisions, §8-1 hand-over)
- `docs/research/codex-launch-materials-measurements-2026-11-09.md`
- `.claude/handoff/attachments/20261109-s25-step6-material.md` (scout material: spec excerpts with parent-TRD line numbers, code state)
- `docs/process/S25-preset-library/trd.md` §5-2 steps 5–8
