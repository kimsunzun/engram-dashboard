# Handoff: S25 step 22 (hook components) — P1, P2 committed; P3a coded + reviewed (uncommitted); next P3b

## Know first
- **P3a code is uncommitted and has not passed QA** (gate not passed — review + one fix round passed, QA runs together with P3b). Files: `Cargo.lock`, `crates/engram-dashboard-agent/Cargo.toml` (sha2 0.10, toml 1.1 — no new package), agent `backend/codex/hook_assembly.rs` (new, untracked), `backend/codex/cmdline.rs`, `backend/codex/mod.rs`, `backend/hooks.rs`, `backend/claude/launch.rs`, `launch.rs`. Snapshot ref: `refs/s25/p3a-wip` = `c4a0d6b3` (tree `60a0df35`, parent `e1ed09fa`) — restore with `git checkout refs/s25/p3a-wip -- Cargo.lock crates/engram-dashboard-agent` if the working tree is lost. Reviewed tree (code) = `6f6652dc`. `docs/reference/structure/agent-backend.md`'s P3a line was committed early in `e1ed09fa`.
- **Codex rules changed by the user (global, 2026-11-25):** review effort by strength (deep = xhigh, full/light = high; Claude high reviewer = `worker-reviewer-high`); resume a codex thread only for re-reviews of the SAME target; new call when the thread's last `tokens used` > 200k; never change effort while resuming. Codex weekly limit was at 31% left (2026-11-25) — keep codex calls lean.
- Step notes: `.review-s25-22/` (git-excluded): `agents.md` (all agent ids / trees / verdicts), unit briefs (`p1a-unit.md` … `p3a-unit.md`, `p3a-fix-r1.md`), `coder-brief.md` (standing coder rules), handovers `handover-p1a.md`, `handover-p1b.md`, `handover-p2.md`, `handover-p3a.md` (P3b entry points + open holds), review / QA results.
- Not ours: `src-tauri/bindings/*.ts` line-ending noise, `.claude/explain/`.
- QA binding quirk: `core.hooksPath` is an absolute path to the main checkout, so mutation runs in an isolated copy (cold build ~3 min) — filed as feedback (2nd).

## One-line state · next first action
Step 22: TRD `trd-hook-components.md` (draft 7, reviewed deep PASS) · ADR-0420 (+ ADR-0421 path rule, ADR-0422 value escaping). **P1** (library kind + agent contract) `78b80e06` and **P2** (placeholder rewrite, path rule, claude route — claude hooks measured running) `17cb4829` committed + pushed after review + QA PASS. **P3a** (codex hook assembly: hash / key / TOML writer / escaping / raw folding / state carry / budget / U5 — NOT wired, codex still loads no hooks) coded, deep review PASS after one fix round.
**Next:** spawn a fresh worker-senior coder for **P3b** with a brief built from TRD P3 row + `handover-p3a.md` "Entry points P3b must call" + its Fix-round notes: wire `cmdline::assembled` into codex `command_line_fits` / `materialize`, `hooks_line_readable` in `config_args`, exempt raw hooks lines from `codex-config-unsafe-char` only through the same `split_hooks_lines` check (unreadable still refused — risk reviewer condition), `hook_config` on thread/start AND thread/resume, AppServerOpen expected entries, post-open `hooks/list {threadId}` check → `LinkResolution::Ready` notices → `deliver_notices`, codex `loads().hooks = true` both modes, remove the codex interim refusal + `allow(dead_code)`, notice wordings, log the dropped-state warning once, ADR-0216 test docs, codex lane tests (`codex_launch_live`: trust + hash vs `hooks/list` through Engram's own transport). Then review (deep → xhigh; new reviewers — new target), QA (full + both lanes + **GUI** — codex transport row), commit P3a+P3b together. Then P4 (lone `hooks.json` confirmation; `hook-unconfirmed` already exists as a merge `left` reason), P5 (help paragraph incl. refused path chars / two-% note / `library.list` direct-items wording, priming root line, reference files, scene H1 all four backend·mode pairs), mark TRD Implemented + parent item 22, then the end-of-server-stage CLI pass with the user, then the client stage (first slot = read-only structure viewer).

## Repo state (measured at save)
- Branch `v0.5.0/feat/preset-library`, HEAD `e1ed09fa`, 0 ahead / 0 behind origin (this handoff commit makes it 1 ahead). Base is behind origin/master (8+ commits at last check) — merge master at integration.
- Uncommitted: P3a code (above, gate not passed). Untracked: `hook_assembly.rs` (ours), `.claude/explain/` (not ours).

## Verification state
- Committed and gated: P1 `78b80e06` (QA: workspace 6990, gates 34/34, frontend 2884, claude 14/14, codex 19/19, mutation 5/5, GUI scene PASS); P2 `17cb4829` (QA: workspace 7005, gates 34/34, frontend 2884, claude 15/15 incl. L7 hook markers, codex 19/19, mutation 3/3; probe: an unknown hook event key in --settings is skipped alone).
- P3a: coder suites green (agent lib 1852, asset-library 946, daemon lib 727, cli 116+47), fmt clean, bindings no drift; hash goldens verified by hand (16) by the lead reviewer. **QA not run.**
- Rerun: `cargo test --workspace -- --test-threads=4` · full QA per `.claude/skill-bindings/qa.md`.

## Failed approaches (do-not)
- REPL scripts that contain a literal backslash followed by u and four hex digits, or backticks / dollar-brace inside template literals, are refused by auto mode or the parser — build such text with `String.fromCharCode(92)` or plain strings.
- Resuming one codex review thread across units / many rounds cost 300k–480k tokens per short re-check (now forbidden by the new global rule).
- `git write-tree` fingerprint after a docs commit landed mid-QA differs by that docs file only — verify with `git diff --stat <qa-tree> <new-tree>` before committing.

## Decisions
User:
- codex 훅 승인 처리 · 상황: codex는 미승인 훅을 조용히 건너뜀, JSON 모드엔 승인 화면 없음 · A 자동 승인 ★ / B 우회 플래그 / C 그대로 / D 사용자 한 번 승인 · 「우리건데?」 · 같음 · 근거: 프리셋에 붙인 것이 승인.
- 승인 기록 자리 · 상황: 세션 키가 위치 기반이라 조합이 바뀌면 덮어씀 · 매번 -c ★ / config.toml(관례) · 「-c 가 그럼 더 좋다는거아냐? 유동적이고」 · 같음.
- 설정 부품 안의 codex 훅 줄 · 상황: 같은 -c 키는 나중 것만 남음 · 실행 거부 ★ / 합치기 / 우리 것이 이김 · 「합치기가 맞고」 · 갈림.
- U5 프로필 추가 실행 인자의 hooks 줄 · 상황: 표기 여러 가지, 리뷰마다 구멍 · A 거부+안내 ★ / B 다 읽어 합치기 · 「A로 해서 코딩 들어가.」 · 같음.
- (묻지 않음) 옛 세션-id 훅 부활 · 「ㅇㅇ 그대로가」.
- (묻지 않음) UI 첫 작업 = 읽기 전용 구조 뷰어 슬롯, 앱 안 · 「앱안에 슬롯으로 만들긴 하는데 … 슬롯이 전부 플러그인이되서 나중에 유저가 참여할 수 있게」 · 조건: 「별도의 주제니깐 그렇다고 가정하고 구조 유지해서」 → tracking T-109 / T-135.
- (묻지 않음) 기본 플러그인 팩 · 「디폴트 플러그인 팩 같은 느낌으로 한곳에 모을거거든? 사용량 같은것도 거기에 있어야겠지.」 → T-109.
- (묻지 않음) 백엔드를 모듈로 · 「나중에는 백엔드만 모듈로 빼서 다른사람도 확장할 수 있게」 → T-134.
- (묻지 않음) 뷰어 동작 · 「전체 연결도 볼 수 있고 클릭하면 해당것만 포커싱」 → T-135 (mockup shown, throwaway).
- (묻지 않음) codex 사용 규칙 — 사용자가 전역 규칙을 직접 고침(위 Know first).
Main: P1 split P1a/P1b, P3 split P3a/P3b; O4 include (ADR-0420 d14); LibraryFolderRow (not FolderRow) and one origin per config argument; path rule additions (ADR-0421); value escaping instead of refusal (ADR-0422, all three reviewers recommended); cmd `%` documented not refused; hooks-not-loaded code; merge keeps hooks.json when an entry stays behind (hook-unconfirmed in left).
Open: ADR-0421 has two rejected alternatives and ADR-0422 one marked 「약함 (not put to the user)」 (flow §1-2 says show them to the user); holds in `handover-p3a.md` (invalid-regex matcher group trusted but skipped; float/date in a raw hooks line refuses); TRD O7 (boot-restored agents only log the post-open verdict), O8 (detect codex review screen in the terminal pump); tracking T-125 / T-127 / T-128 / T-131 still waiting for the user.

## References
- TRD `docs/process/S25-preset-library/trd-hook-components.md`; ADR-0420, ADR-0421, ADR-0422; research `docs/research/*-2026-11-24.md`.
- Codex source clone (rust-v0.171.0): previous session scratchpad `C:/Users/kimsunzun/AppData/Local/Temp/claude/5e8357076cb62dd9/ac276d0d-d059-40f0-9ac0-521d8901bf10/scratchpad/codex` — may be cleared; re-clone tag `rust-v0.171.0` if missing.
- Feedback filed this session: Engram `79c5a87` (qa: hooksPath 2nd, vite nohup), `1514a63` (global: codex resume cost).
