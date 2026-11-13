# Handoff: S25 step 7 complete (P8 · P9 · doc pass landed) — next = show the user step 7 (Stop after), then step 18 → 17 → 8

## Know first
- Step 7 is *Stop (after)* (parent TRD): the user has not yet been shown a step-7 summary for confirmation. The user asked to save the handoff once the work was done — next session: give the short step-7 summary, then proceed to step 18 unless the user says otherwise.
- `src-tauri/bindings/*.ts` (~45) line-ending noise — never stage.
- New project rules from this session (already in the repo): log messages are **English, short, plain, no narrative** (`CLAUDE.md` 「언어」, `docs/reference/logging-conventions.md`); the existing 765 Korean log calls are converted later in one batch (tracking T-108). Coders/reviewers briefed after today must follow it.
- User's review-fix criterion still applies (2026-11-12): a theoretically reachable finding is fixed only if the fix adds no conditions/states/branches; otherwise record it. 「조건이 많아지는것도 버그가 끼어들 틈이 있어서.」
- User wants continuous progress without asking at unit boundaries; ask only real decisions. Explain user-visible questions with a concrete example first — the user repeatedly asked 「뭔소리야? 예시좀」 when a question was framed in internal terms.
- Before raising a user-visible policy, check conventions (CLAUDE.md).

## One-line state · next first action
S25 step 7 (asset-library crate) is done: P1–P9 committed, doc pass committed, all pushed. **Next: show the user a step-7 summary (Stop after) → step 18 (`S25: docs: CLAUDE.md — asset-library 항목 · 그 게이트 줄 · windows feature`) → 17 (ship `builtin\`) → 8 (plug `LibraryService` into the daemon).** Read the parent TRD step rows (they now carry "Handed over by step 7" paragraphs) before starting each.

## Repo state (branch v0.5.0/feat/preset-library · worktree wt1)
- HEAD `a3a565b7` (+ this handoff commit). Upstream 0/0 before this commit; vs origin/master 0 behind / 220 ahead.
- This session's commits: `f42d8538` P8, `d445f03e` P9, ADR commits `437ce603` … `9582cb5a` (ADR-0361 … ADR-0367 + links), `a3a565b7` doc pass. Engram repo: `1514d24`, `bf9326e` (engram-code-conventions feedback rows).
- CI: P8 green, P9 green, doc pass `a3a565b7` in progress at save time (comment/doc-only change).
- Uncommitted: only `src-tauri/bindings/*.ts` line-ending noise.
- `.review-s25-d9/` (git-excluded) holds every brief, coder report, fix list, review brief, QA brief and commit message for P5–P9 and the doc pass — reuse the patterns (`p9-*`, `docpass-*`, `step7-*`).
- Leftover refs/branches not mine to delete: `refs/wip/u1-partA`, `refs/wip/u1-review` (older sessions), local branches `v0.5.0/wip/s25-p4-final-pending`, `v0.5.0/wip/s25-p4-schema-validate-parked` (user's call).

## Verification state
- P8 (`f42d8538`): review full 3 rounds + holds delta (lead + cross-family) PASS; QA PASS on the commit tree — workspace 6135, frontend 2807, gates 33/33, flake 4/4, mutation 6/6 (holds delta).
- P9 (`d445f03e`): review deep (lead + risk-axis + cross-family) 4 rounds PASS; QA PASS on the commit tree — workspace 6183, frontend 2807 + tsc, gates 33/33 (license notices list sha2), flake 4/4 at 32 threads incl. under load, mutation 8/8.
- Doc pass (`a3a565b7`): `/engram-review doc` full 3 rounds PASS; comment-only check 0 lines; ADR lint 0 errors. No QA needed (docs + comments — qa binding 「주석만」 row).
- Not verified: CI of `a3a565b7` (running at save).
- Rerun: `cargo test --workspace --locked -- --test-threads=4`; lanes per `.claude/skill-bindings/qa.md`.

## Failed approaches (do-not)
- A QA run on a tree that will still change is wasted — stop it (TaskStop) before changing the tree (done once this session when hold 1 changed).
- Do not write a user decision from main's own explanation: the review caught two misattributions (log localization credited to the user; a quoted main message the transcript does not hold). Quote only what the transcript shows the user said or saw.
- Re-measure counts instead of trusting a quick script: main told the user "~620 Korean log lines"; the multi-line-aware count is 765 (T-108 records the method).
- Docs in this repo mix LF and CRLF; replacement scripts assuming `\n` silently miss CRLF files (tracking.md, step-log.md, logging-conventions.md are CRLF in the working tree).

## Decisions
User (2026-11-12/13):
- 라이브러리 폴더 없음 표시 · 상황: root unset/first scan failed/root unusable → ① rows beside ④; later the stale-catalog case also showed ②③ rows · 선택지 one line ★ / both · 고른 것 「줄 하나만 나오게 하는게 맞음.」 then 「라이브러리 폴터가 통째로 사라졌으면 한줄 얘기하면 되는거 아님?」 · 같음 → a block-④ resolution is exactly [④] (early return in `resolve`) — ADR-0361.
- 깨진 내장 파일 · 상황: a non-parsing built-in blocked every launch · 선택지 stand-in + warn ★ / keep blocking · 고른 것 「그냥 실행하고 알림만 하면 될듯」 · 같음 → stand-in + `builtin-unavailable` notice (main: one rule for every failure stand-in) — ADR-0361.
- 작업 폴더 지시서 위치 · 상황: codex folder entry `AGENTS.md` appended after all library instructions vs parent TRD §1-5 · 선택지 TRD order ★ / append · 고른 것 「A 하고 B 폴더가 있으면 순서대로해야지.」 · 같음 → `ResolvedFolder.instructions_at`.
- 차단 사유 꼬리표 · 상황: command-line / working-folder rows used the backend word / cwd as the item · 선택지 kind tag ★ / keep · 고른 것 「관행으로 너가 A가 좋다고 하면 그렇게하면됨」 · 같음; language: English ★ / Korean · 고른 것 「영어로해.」 · 같음 → `command-line:<backend>`, `working-folder:<path>` — ADR-0361.
- 로그 문장 · (묻지 않음) 「로그 메세지는 AI체로 하지 말아야되는거 알지?」; then localization question 선택지 not translated ★ / translate later · 고른 것 「로그는 그냥 영어.」 + 「대신에 서술이 아니라 누구나 알아볼 수 있게 심플하게 구성해야겠지.」 · 갈림 (main had recommended keeping Korean, not translated) · 조건: 「규약을 바꿔. … 어쨋든 지시서 바꿔 나중에 이부분 영어로 한꺼번에 변경은 할거야.」 → convention + CLAUDE.md changed; batch = T-108.
- 하위 폴더 건너뛰기 (2026-11-12, P6) — recorded as ADR-0363.
Main (2026-11-12/13): failed read worker = unanswered (like timeout); merged-file format `<!-- path -->` per source (convention: Claude Code "Contents of <path>", codex carries the AGENTS.md path); ADR-0366's rejected "widen a queued generation" (internal; user told, no objection); P9 `sha2` dependency (ADR-0336, already in Cargo.lock); successor coder at the 500k-token mark; batched comment fixes by main.
Open: none blocking. Recorded-not-fixed items are in tracking T-106 / T-107 (incl. the nested-preset / repeated-folder placement test, unpinned because of the round cap).

## Improvements
- Recorded (Engram `1514d24`, `bf9326e`): engram-code-conventions — log wording rule (short factual line, no narrative); follow-up/correction for the log language.
- Report only: worker suggestions — keep a log-counting script in `scripts/` for T-108; point doc pointers at headings, not line numbers; doc-pass briefs should warn about CRLF files; child TRD §12 parent-edit bullets need a "done" mark.

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 log --oneline -3 && gh run list --branch v0.5.0/feat/preset-library --limit 2`

## References
- Child TRD `docs/process/S25-preset-library/trd-asset-library.md` (draft 13) — §10 decisions, §12 hand-overs, §13 review record.
- Parent TRD `docs/process/S25-preset-library/trd.md` — step list (§ step rows 18, 17, 8, 9a with "Handed over by step 7"), 「Step 7 landed」.
- ADR-0361 … ADR-0367; tracking T-105 … T-108.
- `.review-s25-d9/` patterns: `p9-coder-brief.md`, `p9-review-brief.md`, `p9-review-risk.md`, `p9-review-codex.md`, `p9-qa-brief.md`, `p9-commit-msg.txt`, `docpass-review-brief.md`.
- Codex threads left in `~/.codex` by this session's reviews (deleting is the user's).
