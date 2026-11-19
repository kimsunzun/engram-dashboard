# Handoff: S25 — 9a frozen; ADR-0385 several roots (new step 21); 10a TRD draft 2 re-reviewed PASS by both Claude reviewers — next = fold the round-2 권장, ask the 4 user holds, record review pass, then code 10a

## Know first
- The user's token budget is near its end (account switched once mid-session). Keep turns short.
- **4 user decisions pending (holds — commit of the review-pass line waits on them)** — ask in plain words, one question ("추천대로 할까요?"), recommendations already written into the TRD as 「provisional — user」:
  1. Step-8 hand-over live check: only the wake-block scene (G1) now; the "blocked right after create" scene moves to tracking until an entrance creates-and-starts with presets. ★ as stated.
  2. `preset.previewFolder` answer = typed rows (`PreviewSetRow`, bindings for the screen), not free JSON. ★
  3. Priming leaves ADR-0383's user-held list (now English) — record by ADR link at the 10a freeze. ★
  4. NEW (risk lens round 2): `agent.new {presets}` carries no `root_generation` unlike attach — a caller who read paths under root A gets them judged under B after someone's setRoot. Options: add the optional arg (like attach) ★? / leave it. Main has not decided a recommendation yet — check code cost first (likely: add it, same shape as attach, cheap).
- Asked earlier and answered (2026-11-19): O1 priming = short pointer lines in existing format; O2 translate whole priming to English now (content rewrite later = T-118); O3/O4 taken as recommended on 「진행해」 (refuse whole `agent.new` when library unavailable; duplicate lands beside the original) — tell the user they can still overturn O3/O4.
- Autonomous mode. `src-tauri/bindings/*.ts` line-ending noise and untracked `.claude/explain/` are not ours — never stage.

## One-line state · next first action
10a TRD `docs/process/S25-preset-library/trd-library-commands.md` draft 2 (`a0646f9b`) — round 2: risk lens PASS, lead PASS (codex's round-1 must confirmed fixed by both). **Next:** (1) ask the 4 holds; (2) send the round-2 권장 + batched (below) to a fresh TRD editor (old drafter is at ~395k context — use the fix-file pattern `.review-s25-d9/s10a-trd-fix1.md`), main diff-checks (doc-only, no re-review unless a decision/boundary changes); (3) write the review-pass line in TRD §13 (date · deep · PASS · commit); (4) write ADRs for TRD decisions later work relies on (implement binding: before coding); (5) start coding U1 via `/engram-implement` §1–§2 (5 sequential units).

Round-2 items to fold (all 권장/batched, no must):
- Risk lens: G2 window wrong (stat runs before the reservation — use a stalled gate holder or leave to manager tests); "no last failure" must name both record sites (`spawn_fresh_settled`→`note_spawn_result`, `resume_no_fallback` records before `refused_before_spawn`); step-5 `try_update_with` always saves → use a conditional registry method (`try_mutate_if` shape) so a `conversed` refusal writes nothing and is not masked by a store error; R3/§5 "still resolves" over-claim after setRoot/refresh.
- Lead: `SkillFile` rows in `library.list` have no `LibraryKind` (state word / filter / excluded); setWorkspace should pre-check running + session id before reserving (re-check under reservation) so a doomed call doesn't block launches; batched b1–b9 (wire callers get error text not a bus code; boot restore is dormant in production; relink rerun covers an escaped agent; timeout → `workspace-not-found` why; `PreviewSetRow.unsure` = word + `message?`; port crosses views without `name`; ADR-0321 d4 exception → ADR link at freeze; §11 correction `(2)`→`(1)` + `policy_refusal` text for duplicate; `make_daemon_table` real caller list).

## Repo state (branch v0.5.0/feat/preset-library · worktree wt1)
- HEAD `a0646f9b`; 2 commits ahead of origin (TRD draft 1 `ba8a98d1`, draft 2 `a0646f9b`) — push with this handoff.
- This session's commits: U4 `bda7d849`, ADR-0384 line `797d1b51`, 9a freeze `4a945160`, ADR-0385 `9154b900`/`0f88a68f`, step 21 inserted `e5848eae`, TRD draft 1 + T-118 `ba8a98d1`, draft 2 `a0646f9b`.
- Uncommitted: nothing of ours.
- Engram repo (not pushed): feedback `7b84eb0` (user-facing: expanding review leftovers spelled code names).
- Branch is 5 behind origin/master (S39 skill bindings + `scripts/qa-mutate-test.mjs`, qa binding mutation procedure changed) — merge master before 10a QA (qa binding changed).

## Verification state
- U4: QA PASS (workspace 6401, frontend 2884, gates 34/34, claude+codex lanes, mutation 2/2); CI green on `797d1b51`.
- 10a: no code yet. TRD review deep: round 1 FIX ×3 → draft 2 → round 2 PASS (lead, risk lens). Codex not re-run (its must was also raised by Claude → Claude-only recheck per review flow §4).
- Rerun: `cargo test --workspace -- --test-threads=4` (detached per `.claude/skill-bindings/qa.md`).

## Failed approaches (do-not)
- Python heredoc with `'19\\. '` inside `py - <<'EOF'` prints SyntaxWarning but works; prefer raw strings.
- `rm -rf` blocked — `bash ~/.claude/references/recycle.sh`.

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 log --oneline -4 && git -C I:/Engram/apps/engram-dashboard-wt1 status --short | grep -v bindings`

## Decisions
User (2026-11-19):
- U4 relink `fixed` race · 상황: pair counted fixed when its agent is deleted mid-relink, reporting only · 선택지 기록하고 진행 ★ / 고치기 · 고른 것 「삭제해서 링크가 바뀌든 링크가 바뀌고 삭제하든 상관없는거잖아. 성공이지」 · 같음(더 강하게 — 결함 아님) · 근거: 참조가 사라졌으니 성공.
- 9a 미수정 권장 · 선택지 전부 미룸 ★ / 고침 · 고른 것 「그냥 미뤄.」 · 같음 → T-117.
- 여러 라이브러리 루트 · (묻지 않음) · 「여러개 도 될수있으면 좋을것같은데 물론 서로간에 간섭하지 않는다고하고」 · 근거: 샘플 기본 연결 + 별도 git, Engram/회사 git 분리 · 시점 선택지 C 10a 뒤 9b 전 ★ / A S25 뒤 / B 지금 · 고른 것 「ㅇㅇ 너 추천대로해.」 · 같음 → ADR-0385, step 21.
- 안내문 언어 · 선택지 전체 영어 ★ / 한국어 유지 · 고른 것 「영어로 하고 해당 문서도 나중에 개선하긴 해야되는데 어쨋든 현재는 기존 포맷 맞춰서 진행해.」 · 같음 · 조건: 기존 포맷 → T-118.
- 10a TRD 브리핑 방식 · 「trd 간단하게 풀어서 브리핑하면 됨 html … 구지 생관없을듯」.
Main: 10a own step TRD; review deep (public command surface + concurrency); duplicate consults `llm_creation_refusal` (B21, ADR-0279 precedent); typed rows; 5 sequential units.
Open: the 4 holds above.

## References
- 10a TRD `docs/process/S25-preset-library/trd-library-commands.md` (draft 2); parent `trd.md` §5-2 items 10a, 21; ADR-0384, ADR-0385, ADR-0383.
- Review trail `.review-s25-d9/s10a-*` (brief, common, codex, fix1, rereview-r2, r2.diff).
- Bindings `.claude/skill-bindings/implement.md`, `review.md`, `qa.md`.
