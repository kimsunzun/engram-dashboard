# Handoff: S25 step 10a frozen (library / preset / agent-preset commands, help, English priming) — next = T-112 (skip missing items at launch), then step 21

## Know first
- One open question to the user (not blocking): the 10a unfixed 권장 list — (1) `root_generation` is `bigint` in agent TS bindings (client stage); (2) answer-word casing (`PresetSection` / `FolderLoadKindWord` capitalised vs lowercase refusal args) — handed to step 21; (3) `preset.get engram:/default` drops the built-in stand-in notice — handed to step 21; (4) an agent-preset answer shows the name as resolved if renamed concurrently (documented); (5) T-119 PowerShell 5.1 JSON list args. Main recommended leaving all five as recorded; ask "고칠 것 있나요?" once, in plain words.
- The user wants: autonomous mode, short status answers, plain words (no code names), and only real user-visible decisions asked. Holds on details: main decides and reports (review-skill feedback row 2026-11-19).
- Handoff: save when the user asks or the auto-handoff hook says so, and before handing the session over (the user expected a save before exit). Do not save on a usage-limit warning on main's own initiative (handoff feedback row 2026-11-20).
- `src-tauri/bindings/*.ts` line-ending noise and untracked `.claude/explain/` are not ours — never stage.
- QA workers twice had a big preliminary command batch denied by the auto-mode classifier ("[Undermining Oversight]" — the batch held a `Get-CimInstance … engram-dashboard%` process listing); briefs now say run commands one at a time.

## One-line state · next first action
Step 10a is implemented and frozen (`trd-library-commands.md` status Implemented 2026-11-20; parent `trd.md` item 10a child-TRD line; README rows). **Next: T-112** (ADR-0374 — a missing item / unusable library root no longer blocks a launch; skip and notify) via `/engram-implement` — it has no TRD yet: draft one (worker), show the user a plain summary, review, then code. Then step 21 (several roots, built-in as an always-present read-only root), then 9b / 10b, then the server-stage user check, then the client stage (screen discussion first).

## Repo state (branch v0.5.0/feat/preset-library · worktree wt1)
- HEAD `835e37c2`, 0 ahead / 0 behind origin at save time (this handoff commit will be ahead until pushed).
- This session's commits (newest last): U4 9a `bda7d849`, ADR-0384 line `797d1b51`, 9a freeze `4a945160`, ADR-0385 `9154b900`/`0f88a68f`, step 21 `e5848eae`, 10a TRD drafts `ba8a98d1`/`a0646f9b`/`debec111`, review record `a525c0e3`, master merge `6a5317b2`, ADR-0386 `a43e1e57`… `4e9d5024` (+ links 0383/0321/0379), step-21 built-in note `7e991768`, U1 `33db6f86`, U2 `e61f0d68`, U3 `f458aa21`, U4 `77ab3baa`, U5 `19e4a626`, freeze `9c47851d`, tracking `835e37c2`.
- Uncommitted: nothing of ours.
- Engram repo (not pushed): feedback `7b84eb0` (user-facing), `8c03a92` (review: detail-level hold), `7e373b4` (handoff: unasked save).

## Verification state
- Every 10a unit: `/engram-review code` deep (lead · risk lens · Codex; U5 full), then `/engram-qa` full regression local + claude/codex lanes + mutation; U4 also GUI (start from tree, double start); U5 also a live check — a real claude agent ran `engram help library`, `library.status/newFolder/newFile/list/read`, a refused `../x`, `preset.add` with a JSON list (Git Bash) and received the English priming. Last full regression: workspace 6526, frontend 2884, gates 34/34. CI green through the 10a freeze commit.
- Not verified: U4 QA's toolchain assert and CI step cross-check (denied batch; CI covers them); the "answers every path" fallbacks have no test (BoundedIo answers every key — unreachable by fakes).
- Rerun: `cargo test --workspace -- --test-threads=4` (detached per `.claude/skill-bindings/qa.md`).

## Failed approaches (do-not)
- QA briefs suggesting mutations on lines without an operator token (`==`/`!=`, `<`/`<=`, `true`/`false`, int ±1) — the S39 mutate tool cannot run them; check the token first.
- A live CLI smoke outside an agent — `engram` needs an agent token (`NO_TOKEN`); run it inside a spawned agent.
- Python edits without `newline=''` flip CRLF files; edits on `docs/tracking.md` need CRLF-aware matching.

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 log --oneline -3 && git -C I:/Engram/apps/engram-dashboard-wt1 status --short | grep -v bindings`

## Decisions
User (2026-11-19 ~ 20):
- 여러 라이브러리 루트 · (묻지 않음) · 「여러개 도 될수있으면 좋을것같은데 물론 서로간에 간섭하지 않는다고하고」 · 근거: 샘플 기본 연결 + 별도 git, Engram/회사 git 분리 · 시점 C ★ / A / B · 「ㅇㅇ 너 추천대로해.」 · 같음 → ADR-0385, step 21.
- 「기본」 · 「기본 폴더는 무조건 있으면됨」, 「21에서 하면 하는거지 뭘 넣었다가 다시 뺌?」 → the built-in becomes an always-present read-only root in step 21; nothing patched in 10a.
- 안내문 언어 · 전체 영어 ★ / 한국어 · 「영어로 하고 … 현재는 기존 포맷 맞춰서 진행해.」 · 같음 · T-118.
- 검색 범위 · 폴더 자신 빼기 ★ · 「ㅇㅇ」 · 같음 (ADR-0386 d13).
- 재시도 표시 · 「llm이 알아서 수정해서 시도가 맞지.」 · 같음 (ADR-0386 d14; extended by main to the clearable working-folder refusals).
- 프리셋 없이도 실행 · 「있든없든 동작은 되야지」 · already true; missing items → T-112 right after 10a (main's order).
- 루트 확인 옵션(agent.new) · 「그냥 알아서해. 부담없음 넣고.」.
- 「그냥 미뤄.」 (9a 권장 → T-117); 「자율로 쭉 진행해.」.
Main: not-found stays out of the retry list (mostly typos); duplicate consults the LLM creation policy; folder spellings unquoted (d16); relative setWorkspace cwd refused; no reachable bounded-IO expect (d16).
Open: the 10a unfixed 권장 question above.

## References
- 10a TRD `docs/process/S25-preset-library/trd-library-commands.md` (Implemented); parent `trd.md` items 10a, 21 (with the built-in and 10a hand-over notes); ADR-0385, ADR-0386, ADR-0374 (T-112).
- `docs/tracking.md` T-112, T-117, T-118, T-119.
- Review / QA trail `.review-s25-d9/s10a-*` (git-excluded).
