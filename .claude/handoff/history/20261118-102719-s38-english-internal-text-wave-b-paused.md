# Handoff: S38 (internal text → English) — docs + leaf crates + agent crate done; wave B (daemon / src-tauri / frontend) paused mid-translation

## Read first

- **Cost is the user's main concern now.** The user was upset at token use (「뭔 에이전트를 50개를 스폰을해」, 「내부 번역을 비싸게 쓸 필요있음?」). For the rest: Sonnet translates and runs builds/tests (`worker-scout-sonnet`), ONE Opus reviewer per unit reviews-and-fixes (S33 style), Codex at most once per unit only to confirm "no code change beyond strings", at most 3 workers at once. Do not spawn per-area reviewer trios again. Confirm this plan with the user in one line before spawning (the user agreed the direction but did not answer the final config question).
- The user wants a remaining-work estimate to plan; give it first when they return (numbers below).
- S33 had NOT finished code comments and left code strings out by design — that is why S38 grew (~45k code lines + ~11k doc lines). The user thought S33 had done it all.
- Do not message other sessions; the user syncs other branches after the single master push (「그냥 푸쉬하면 내가 알아서 동기화 하라고 전달함」).
- Working state lives in the git-excluded folder `.s38-review/` of this worktree (briefs, tools, diffs). `STATE.md` there is a short status; this handoff supersedes it.
- Never let a worker use `isolation: "worktree"` (it bases on master and deletes unchanged worktrees). Main creates worktrees with `git worktree add` and the brief pins `cd <path> &&`.

## One-line state · next first action

- State: branch `v0.5.0/chore/english-internal-text` at `961551a7` (pushed). Done and committed: PRD/TRD 2.1, ADR-0383, all docs (W1/W2a/W2b/W3), U1 leaf crates, U2 agent crate. Wave B: U6 (scripts/CI/launch/_wip + `scripts/check-hangul.mjs`) finished but unreviewed; U3 daemon / U4 src-tauri / U5 frontend stopped mid-way by main at a usage limit, partial edits uncommitted in their worktrees.
- Remaining (measured 10:25): daemon 5,261 Korean lines left (of 8,761), src-tauri 3,327 (of 6,329), frontend 4,171 (of 7,037; ~1,000 of these are kept Korean UI assertion values) → ~11k lines to translate; then review of U3–U6, U9 integration, QA, master merge.
- Next first action (after the user confirms the cheap plan): resume U3/U4/U5 with fresh `worker-scout-sonnet` coders that continue from `git status` in each worktree (briefs `.s38-review/coder-common.md` + `.s38-review/wave-b-units.md`), max 3 at once; then one Opus reviewer-fixer per unit.

## Repo state (measured 2026-11-18 10:27)

- wt2: branch `v0.5.0/chore/english-internal-text`, HEAD `961551a7`, 0 ahead of its remote (this handoff's commit comes after this text). Tracked working tree clean. master unchanged (`36a055da`).
- Unit worktrees (all branched from `961551a7`, local branches `s38-u3..u6`, NOT pushed):
  - `I:/Engram/apps/s38-wt/u3` daemon — uncommitted partial translation.
  - `I:/Engram/apps/s38-wt/u4` src-tauri — uncommitted partial translation.
  - `I:/Engram/apps/s38-wt/u5` frontend — uncommitted partial translation.
  - `I:/Engram/apps/s38-wt/u6` — local commit `71d6fb6a` (complete, unreviewed).
  - Old: `s38-wt/u1`, `u2a`, `u2b` (already merged into the topic branch; can be removed later with `git worktree remove`), and a harness worktree `I:/Engram/apps/engram-dashboard/.claude/worktrees/agent-a5f44decd17fe2489` plus stray branches `worktree-agent-a036dc1388a9a34fa`, `worktree-agent-aab949ee597c6cff9` (all at `d62374f2`, unused — ask before deleting).
- Engram repo: feedback commit `6d82f4e` (global-feedback rows: bulk-translation cost, worktree isolation) — not pushed (rule).

## Verification

- Done: per-unit package tests in the unit worktrees — U1 (base 93, command 110, protocol 126, messaging 273, platform 85, transport 386/434, cli 157, shell lib_unit 1489, vitest wsFrame+StructuredTextView 86), U2 (agent 1553, daemon ws_e2e 45); test counts per target unchanged; comment-stripped token compare = rustfmt only. Docs: `py .s38-review/doclines.py scan @.s38-review/<unit>.txt` = 0 remaining for w1/w2a/w2b/w3; `adr.mjs lint` clean. U6: `npx vitest run src/util/checkHangul.test.ts …` 27 passed, `npx tsc --noEmit` 0.
- Reviews done: PRD+TRD full PASS; U1 full PASS (1 fix round); U2 full PASS (1 fix round + main batched). Docs: Sonnet translate + Opus review-fix per batch.
- **Not verified (labels):** 「QA 미실행」 — no full regression (CI three jobs locally) and no GUI run yet on the topic branch; committed U1/U2 carry "QA pending" in their commit bodies. 「리뷰 미통과」 — U6 (`71d6fb6a`) and the partial U3/U4/U5 edits are untrusted diffs. 「미커밋」 — U3/U4/U5 worktree edits.
- Re-run check of the topic branch Hangul status: `node I:/Engram/apps/s38-wt/u6/scripts/check-hangul.mjs I:/Engram/apps/engram-dashboard-wt2` (folder mode) once U6 lands.

## Failed approaches (do not repeat)

- Agent `isolation: "worktree"`: base was master, not the topic branch; workers stopped on the base check, the unchanged worktrees were auto-deleted, and resumed workers ran `git checkout -B` in wt2 (main checkout). Fixed by main-made worktrees.
- Heavy review layers (3 Claude + Codex per unit, Opus re-read of every doc batch) — correct results but far too expensive for internal text (user correction).
- One Opus coder for a 14k-line crate ran out of context ("Prompt is too long"); a continue-coder from `git status` worked. Keep units ≤ ~5k lines per coder or let coders hand over.
- Python heredocs with `\u` in replacement strings broke (`unicodeescape`); write scripts with raw strings or via Write.
- `git add $(cat list)` broke on Korean paths; use `git add --pathspec-from-file=<LF list>`.

## Resume command

`git -C I:/Engram/apps/engram-dashboard-wt2 log --oneline -3 && for u in u3 u4 u5 u6; do git -C I:/Engram/apps/s38-wt/$u status --porcelain | wc -l; done`

## Decisions

User decisions (this session):
- (묻지 않음) scope criterion · 상황: S35 left logs/errors/LLM text Korean · 선택지 없음 · 고른 것 「일단 내가 닿는거라면 모를까 닿지 않는건 영어임」 · —.
- include S33's leftover docs · 상황: docs had ~11k leftover Korean lines · 선택지 같이 넣음 ★ / 따로 · 고른 것 「같이 넣어」 · 같음 · 조건: 「번역이라서 내가 딱히 얘기할건 없지? 너 플랜 한번 듣고 쭉 진행하면 될듯」.
- (묻지 않음) hold list · 고른 것 「engram help 그냥 놔두고, 너 스킬같은것도 놔두고,(나중에 내가 볼수도 있어서 일단은 보류라는거)」 · — (priming also held by Claude's reading; `.claude/**`, `CLAUDE.md` held).
- merge cadence · 상황: hundreds of files conflict with open branches · 선택지 docs/comments first merge ★ / one push at the end · 고른 것 「그냥 다 작업하고 마스터 푸쉬하면 그것들 한번에 동기화 하는게 좋을것같은데」 · 갈림 · 조건: 「다른세션에 얘기하지는마」, 「그냥 푸쉬하면 내가 알아서 동기화 하라고 전달함」.
- mode · 고른 것 「딱히 이슈 없으면 자율모드로 진행해」, later 「자율로 쭉 진행해」 · —.
- (묻지 않음) cost · 상황: 50+ workers, mostly Opus · 고른 것 요지: internal translation should not use expensive setups (「내부 번역을 비싸게 쓸 필요있음?」); agreed the direction (「그게 맞기는하는데」) · 선택지 lighter review A ★ / Sonnet coders too B / keep C, then Claude proposed S33 style (Sonnet translate + 1 Opus review) — final config not explicitly confirmed · 조건: 「일단 핸드오프해서 진행하는게맞고」.
- Pending (asked, no answer yet): keep Korean HTML reports under `docs/` (★ keep — applied); build script output English (★ yes — applied in U6). Confirm before the master push.

Claude decisions: ADR-0383 decisions 1–7 (screen errors = English cause in translated head; CATALOG_VERSION not bumped; units commit regenerated bindings; English placeholder marker + frontend accepts the old Korean one until the next PROTOCOL_VERSION bump; Hangul check over code/config, docs not checked; Korean file names / ADR lint tokens / ADR index / HTML reports kept; frontend tests stay in `ko`). During coding: references to held Korean doc headings keep the Korean heading in 「」 with `hangul-ok`; ADR section names English in comments; 명부 = "roster" (agent roster, ADR-0119) vs "registry" (queued-input `Registry`); old-marker accept moved from U5 to U1 after Codex review; U1/U2 commits not added to `.git-blame-ignore-revs` (they mix in the marker code); one U1 fix round included batched wording via the coder (deviation from implement §3, to avoid a separate main pass).

Open: none blocking besides the two pending user answers and the cost-plan confirmation.

## U9 checklist (after wave B)

Glossary rows (`.s38-review/manual-fixes.txt`), `docs/reference/logging-conventions.md` §Format rewrite (English log line; replace the Korean example), `docs/reference/structure/i18n.md` line (causes English), `.git-blame-ignore-revs` (docs commits in `.s38-review/blame-ignore-candidates.txt`), move `hangul-ok` off `//!` lines (transport `lib.rs:283`, platform `lib.rs:104/149/168`, `group/mod.rs`, `group/windows.rs`), `check-hangul.mjs` to zero then the CI `gates` step (`shell: bash`), AC7 md scan, `ui_text.rs` continuation lines 84/87/99 markers (U4), S38 README + TRD `Implemented`, then `/engram-qa` (full regression + GUI: AC3 logs English, AC4 band en/ko + live switch), then ask the user before the master push.

## References

- `docs/process/S38-english-internal-text/` (`prd.md`, `trd.md` ed. 2.1), `docs/decisions/0383-internal-text-is-english-only-screen-text-stays-switchable.md`.
- `.s38-review/`: `coder-common.md`, `wave-b-units.md`, `code-review-brief.md`, `translator-brief.md`, `docs-reviewer-brief.md`, `doclines.py`, `manual-fixes.txt`, `blame-ignore-candidates.txt`, `STATE.md`.
