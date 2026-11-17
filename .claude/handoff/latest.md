# Handoff: S25 step 9a — U1 and U1b landed (root setting, persist-after-success, background boot scan); CI was red on long ADR file names (fixed, rerun pending) — next = check CI, then U2

## Know first
- **CI**: every push from `7e764daa` to `8b0ad3e3` failed at checkout — ADR file names 0375–0382 were too long (「Filename too long」). Fixed by renaming to short slugs in `02e07498` (pushed). **First action: confirm CI on `02e07498` is green** (`gh run list --branch v0.5.0/feat/preset-library --limit 3`). If anything other than checkout fails, that is new.
- **`core.hooksPath` changed outside this session**: it is now `I:\Engram\apps\engram-dashboard\scripts\git-hooks` (absolute, set in the main checkout's `.git/config`), not `scripts/git-hooks`. The QA binding's work-tree mutation precondition is literal, so QA fell back to the isolated copy. Unknown who changed it — tell the user; do not change git config without asking.
- **Questions waiting for the user** (ask one at a time, shortest first):
  1. T-114 — codex lane `backend_contract` `q10_shutdown_empties_the_process_tree` failed once in U1b QA (survivor = pre-existing `vctip.exe` via a reused parent PID; agent crate untouched; passes at the baseline). Marked 넘어감(잠정). Fix now or later?
  2. The `core.hooksPath` change above — intended?
  3. U1 unfixed 권장 (user has not answered; default = leave): no deterministic test for a failed copy-aside (behaviour is already correct — gives up and answers unsaved).
- User standards from this session (apply them):
  - The library root is **one setting with a default value** (`<install>\library`), not a separate "default library" concept; deleting or re-creating the folder, or setting it to the default place, keeps the default.
  - **Persist only after success** — never write then undo. Recorded as a conventions candidate (Engram `f5c9272`).
  - Before presenting a convention-based option, search PRD / ADR / prior user decisions for a conflicting rule and present the conflict (missed once with U-D3 vs the 2026-10-17 rule; feedback `e9f2dd8`).
  - Explain choices by what the user sees, with a concrete example; avoid internal terms (the user got lost in 「설정 파일에 무엇을 적을까요」 — rephrase as visible behaviour).
- Autonomous mode (default) — the user went to sleep and said 「자율로 쭉 진행해」.
- Untracked `.claude/explain/` is not ours. `src-tauri/bindings/*.ts` is line-ending noise — never stage.
- QA worker in auto mode: mutation runs were denied once (U1 — the user ran a script with `!`); in U1b they ran in the isolated copy without denial. If denied again: try the isolated copy, then give the user a `!` script (template: the U1 script in the earlier scratchpad — rewrite it, the scratchpad is gone).

## One-line state · next first action
9a units U1 (`7e764daa`) and U1b (`8b0ad3e3`) are committed and pushed; TRD `docs/process/S25-preset-library/trd-library-edit.md` is draft 8 (reviewed; draft 4–8 deltas re-reviewed). **Next: confirm CI green on `02e07498`, ask the waiting questions, then start U2** with the ready brief `.review-s25-d9/s9a-u2-coder-brief.md` (written against draft 7 — re-read TRD draft 8 §7 U2 / §9 U2 and adjust: U1b has landed, sequencing is U1 → U1b → U2 → U3 → U4). Then `/engram-implement` flow: coder → `/engram-review code` (full; deep if lock order is touched) → `/engram-qa` → commit.

## Repo state (branch v0.5.0/feat/preset-library · worktree wt1)
- HEAD `02e07498` (+ this handoff commit), 0 ahead / 0 behind origin at save time (this handoff commit will be ahead until pushed).
- This session's commits (main ones): `49a90181` / `be8a6176` (binding cherry-picks: reference standard; QA mutation guard), `15e8947e` (asset-library.md fixes), TRD drafts `43f64c4f` `becbbeb9` `8928a421` `fc199a9f` `a1e57905` `ed506ecc` `d273f5dc`, ADRs 0374–0382 (`10305720`…`a8ea6cb1`), T-112 `934bc2fb`, U1 `7e764daa`, master (S35 i18n) merge `06f6ced5`, U1b `8b0ad3e3` (+ T-114), ADR rename `02e07498`.
- Uncommitted: only bindings noise and `.claude/explain/`.
- Engram (not pushed): feedback commits `f5c9272` (conventions: persist after success), `9fd510b` (qa: auto-mode mutation denial), `e9f2dd8` (implement: prior-decision check), plus the adr slug feedback commit.
- `.review-s25-d9/` (git-excluded) holds this session's briefs, diffs and reports (`s9a-*`).

## Verification state
- U1: code review deep, 3 rounds (round 3 PASS after the U-D2 rework); QA full regression (6265), claude + codex lanes PASS, shell real-daemon lane PASS; mutation 2/2 + `.engram-dev` isolation run by the user (PASS). User accepted the ~3 s worst-case boot delay (three locked state files on the long load budget).
- U1b: code review deep, round 1 PASS ×3; three 권장 applied (closed by main's diff check); QA full regression (6300, frontend 2868, gates 33/33), claude lane PASS, isolation PASS, mutation 2/2 (isolated copy); codex lane FAIL on T-114 only (넘어감(잠정)).
- Master merge `06f6ced5`: worker ran workspace 6291 + frontend 2868 + tsc + shell targets — PASS.
- CI: not green since `7e764daa` (checkout); rerun on `02e07498` pending at save time.
- Rerun: `cargo test --workspace -- --test-threads=4` (detached runner per `.claude/skill-bindings/qa.md`).

## Failed approaches (do-not)
- Write `library.json` first and undo on refusal (draft 3 order) — the undo produced a new failure class every review round (stale state after a failed undo, clobbering an outside write, compare-then-write window). Replaced by persist-after-success (ADR-0381).
- A clock-based root-generation base — repeats / pre-epoch 0; replaced by a random base from the daemon (getrandom, below 2^52).
- Reusing `Gate::change_root` with an empty build for the no-walk publish — it raises the first scan, so nothing would walk.
- Long ADR titles as file slugs — break CI checkout on Windows; keep slugs short (rename with `git mv` if the script makes them long).
- Running a command the classifier denied to a worker from main — do not; give the user a `!` script.

## Decisions
User (2026-11-16 ~ 18):
- 레퍼런스 기준 바인딩 반영 · 상황: binding §레퍼런스 still old, standard only in feedback · 선택지 지금 바인딩에 반영 ★ / skill-factory 대기 · 고른 것 「햇깔리는데 그냥 스킬팩토리한테 메세지 보내봐.」 → skill-factory applied `e297b42b` · 갈림.
- 첫 시작 기본 라이브러리 기록(U-D1) · 상황: moving the install folder · 선택지 절대경로 / 기본 위치 표시 ★ · 고른 것 「일단 저 A,B라고하면 당연히 B지.」 then 「그냥 동일룰이라잖아. 기본 라이브러리라는 개념은 없어. 단지 디폴트 경로가 그거일 뿐이지」 · 같음 · 조건: 「동일룰」 — one setting with a default value.
- 빠진 파일 실행(ADR-0374, T-112) · 상황: missing items block launches today · 선택지 설정은 막기 ★ / 전부 건너뛰기 · 고른 것 「그냥 B로하고 해당 내용만 호출자 클로드 코덱스가 피드백 하면 되지 않음? … 한개만 막는게 옵션이 있지 않는 이상 통일성이 좀 그런데?」 · 갈림 · 근거: consistency; risk (a skipped settings file can drop a permission limit) accepted.
- 쓰기 순서(U-D2, ADR-0381) · 상황: write-then-undo kept producing review findings · 선택지 틈 수용 ★ / 성공 뒤 쓰기 · 고른 것 「아니 성공해야 적는게맞지. 왜 먼저적어?」 · 갈림.
- 부팅 최악 3초 · 선택지 수용 ★ / 짧은 재시도 · 고른 것 「A」 · 같음.
- master(S35) 반영 · 선택지 지금 ★ / 9a 뒤 · 고른 것 「그냥 받아. 상관없잖아.」 · 같음.
- 부팅 훑기(U-D3, ADR-0382) · 상황: first start walked before the daemon was reachable; convention (JetBrains, rust-analyzer) = background · 선택지 첫 요청 때 / 뜬 직후 백그라운드 ★ · 고른 것 「ㅇㅇ 백그라운드로 훑는게 좋은데 일단 서버쪽 작업하고 UI 쪽은 나중에」, then on the 2026-10-17 conflict 「백그라운드로 해. 데이터가 필요할때 훓는건 백그라운드 없을때 얘기이고.」 · 같음 · 조건: UI indexing cue later (screen step 13).
- 레퍼런스 일괄 정리 시점 · (묻지 않음) · 「한꺼번에 정리하는건 좀 피드백을 받아서 정리된 이후에」 → after the standard settles; measure `asset-library.md` cost first.
Main: U1 review 권장 fixes (absolute default place, long boot load budget, logging, identity v4); `library-config-write-failed` maps non-retaining although applied; port store errors pass through `store_command_error`; relink skips repeating per-file refusals and stops only on IO; U1b `Kind::Root { walks }`; `first_start` precondition failure stays `Failed` with honest wording; T-114 넘어감(잠정).
Open: T-114 (user); `core.hooksPath` change (user); U1 copy-aside test 권장 (default leave).

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 log --oneline -3 && gh run list --branch v0.5.0/feat/preset-library --limit 3`

## References
- TRD `docs/process/S25-preset-library/trd-library-edit.md` (draft 8) — §7 units, §9 tests, §8 decisions.
- ADRs 0374–0382; reference `docs/reference/structure/asset-library.md`.
- U2 brief `.review-s25-d9/s9a-u2-coder-brief.md`; earlier briefs `s9a-u1*` for shape.
- Bindings `.claude/skill-bindings/implement.md` (§레퍼런스 new standard), `review.md`, `qa.md`.
