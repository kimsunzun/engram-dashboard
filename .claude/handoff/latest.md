# Handoff: S25 T-112 frozen (launch skips what is absent, blocks on what exists but cannot be read) — next = step 21 (several library roots)

## Know first
- The user wants: autonomous mode, short answers, plain words. **Progress briefings: a numbered list of the work by name with "now at N of M" — never work labels alone (T-112, 9b …).** The user said this briefing shape is right (「지금이 딱 좋네. 한눈에 볼 수 있어야지」). Minor review items are not shown to the user (「사소한거? 는 내가 알 필요없지?」) — record them in `docs/tracking.md`.
- Server-stage position (UI 전 18개): 1–15 done. 16 = several library roots (step 21) · 17 = file move/copy/delete/trash (9b) · 18 = their commands (10b) · then the server-stage user check · then the client stage (screen discussion first).
- Token cost: the local full regression runs 6 real-claude tests (CI skips them). For this task main skipped them per unit and ran them plus the claude/codex lanes once at the end (user agreed). Ask again per task if it matters; the QA binding itself is unchanged.
- `src-tauri/bindings/*.ts` line-ending noise and untracked `.claude/explain/` are not ours — never stage.
- Workers: foreground Bash moves to background after ~180 s (worker rule says 9 min — feedback row filed); QA uses `pwsh Get-Process` instead of `Get-CimInstance` (classifier denied it before — feedback row filed).

## One-line state · next first action
T-112 is implemented, reviewed, QA'd, frozen and pushed; CI green on `d112a558` (backend · frontend · gates success). **Next: step 21 (several library roots, ADR-0385; the built-in becomes an always-present read-only root)** via `/engram-implement` — parent `trd.md` item 21 now carries the T-112 hand-over paragraph (per-root rule, what is inherited). Then 9b / 10b, then the server-stage user check.

## Repo state (branch v0.5.0/feat/preset-library · worktree wt1)
- HEAD `d112a558`, 0 ahead / 0 behind origin at save time (this handoff commit will be ahead until pushed).
- This session's commits (oldest first): TRD drafts `5cb5a69f` `e01d4dfe` `8c6fecd7` `20a3197b` `edd9fa9a` `7b4db191` `0de9822c` `e24eb277` (review PASS), ADR-0387 `40eced98` + body `c5d4b753`, README row `c5d4fc58`, U1 `d0c251ee`, U2 `ada48af7`, U3 `7ea7441d`, freeze `d112a558`.
- Uncommitted: nothing of ours.
- Engram repo `I:/Engram` master is 11 ahead of origin (feedback rows; this session: `f53450f`, `c7b527a`) — not pushed.

## Verification state
- Ran: TRD review deep (lead · risk lens · Codex), 5 rounds → PASS. Code review per unit: U1 deep PASS, U2 deep (Codex FIX → fixed → all PASS), U3 full (FIX → fixed → PASS). QA: U1 full regression PASS (workspace 6532); U2 PASS (6547, CI skip list); final U2+U3 PASS — workspace 6552 incl. real-claude tests, frontend 2884, gates 34/34, bindings in sync, claude lane (control_send 5, usage 5, launch_kits_live 3 incl. `claude_l5_…`) and codex lane (usage 2, backend_contract 11, codex_launch_live 3, launch_kits_live 2 incl. `codex_l5_…`) PASS; live: CLI end to end in a spawned agent, share probe on unreachable UNC paths (error 53/67 → blocked), GUI double-click (DOM events — session was locked). CI green on `d112a558`.
- Not verified: a remembered but disconnected mapped drive letter (cannot be created on this PC — `net use` to an unreachable share fails with error 67); whether `GetDriveTypeW` / `WNetGetConnectionW` can stall for such a drive (TRD R12, tracking T-120). GUI double-click with real OS input.
- Rerun: `cargo test --workspace -- --test-threads=4` (detached per `.claude/skill-bindings/qa.md`); lanes per qa.md 「claude · codex 레인」.

## Failed approaches (do-not)
- Inserting a paragraph after "Handed over by step 7/20 (…)" in `trd.md` by first match — those markers repeat across items; anchor on the target item (use the last match or the item heading).
- Python here-docs with `\\server` paths in normal strings break (`\U` escapes); write such scripts to a file with raw triple-quoted strings.
- A coder writing JSON with backslashes through tool arguments lost the escapes (`C:\data` → invalid escape); use the Edit tool.

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 log --oneline -3 && git -C I:/Engram/apps/engram-dashboard-wt1 status --short | grep -v bindings`

## Decisions
User (2026-11-20):
- 브리핑 모양 · (묻지 않음) 진행 위치 브리핑 · 선택지 없음 · 「T-112는 뭐야? … 몇번째야 도대체」 → 일 이름 번호 목록 + 현재 위치, 「지금이 딱 좋네. 한눈에 볼 수 있어야지 이상한말들하면 어차피 내가 안봐.」 · — · 근거: 라벨로는 못 알아봄.
- 못 읽는 폴더 항목 · 상황: 네트워크 드라이브 끊김 등, 폴더는 있음 · 막기 ★ / 건너뛰기 · 「ㅇㅇ 막아.」 · 같음.
- 기준 통일 · 상황: 초안은 못 읽는 라이브러리 폴더만 건너뜀 · 없으면 건너뜀·있는데 못 읽으면 막기(라이브러리 폴더 포함) ★ / 전부 건너뜀 · 「ㅇㅇ 같은 기준으로해.」 · 같음 → ADR-0387.
- 못 읽는 폴더 표시 · 상황: 화면이 「찾을 수 없음」으로 보임(폴더는 있음) · 별도 구분 ★ / 그대로 · 「ㅇㅇ 별도로 구분해. 너무 디테일하게 원인을 쓰지는 말고.」 · 같음 · 조건: 「너무 디테일하게 원인을 쓰지는 말고」 → `folder-unreadable`, short fixed text.
- SKILL.md 없는 스킬 폴더 · 상황: 폴더는 있고 스킬 파일만 없음, 라이브러리는 스킬로 안 봄 · 없음(건너뜀) ★ / 깨짐(막기) · 「스킬 폴더는 있는데 스킬이 없으면 없음이지.」 · 같음.
- 토큰 테스트 · 상황: 로컬 전체 회귀가 실제 claude 테스트 6개를 돌림 · 마지막에 한 번 ★ / 조각마다 · 요지: 「자율모드로 쭉 진행해」 (질문에 직접 답 없이 자율 진행 지시) · 같음.
- 「쭉쭉 진행해」, 「자율모드로 쭉 진행해」, 「ci 까지 다 끝내고 핸드오프해」.
Main: drive letter absent → skipped (follows the rule); codex gets no note when Engram has no instruction text (would replace the user's `developer_instructions`); share probe in `BoundedIo::stat_all` with pure platform `share_root` + remembered-connection check; `library.status` word `unreadable`; edit folder refusals follow the rule; link-probe error → ② (NotADirectory absent); notices once per agent, `warn` log every launch; TRD/code-level choices in TRD §11 b/c. Unfixed rare-case recommendations → tracking T-120.
Open: none.

## References
- TRD `docs/process/S25-preset-library/trd-launch-skip-missing.md` (Implemented); ADR-0387 (amends 0374 / 0341 / 0361 / 0363).
- Parent `docs/process/S25-preset-library/trd.md` items 21 (T-112 hand-over), 11 (launch notices do not reach the screen), 13 edition 1 (tree / wording / root cases).
- `docs/tracking.md` T-120; `docs/tracking-archive.md` T-112.
- Review / QA trail `.review-s25-d9/t112-*` (git-excluded), incl. `t112-trd/coder-brief.md` and `coder-handover.md`.
