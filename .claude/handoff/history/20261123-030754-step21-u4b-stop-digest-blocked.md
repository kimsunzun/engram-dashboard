# Handoff: S25 step 21 U4b (renameMount) — uncommitted, review FIX; stopped at the re-fix cap on the "stop answer digest" class — user decision needed

## Know first
- **U4b is uncommitted and NOT gate-passed** (review round 5 = FIX, must 1 from both lead and Codex). Do not commit it as is. The tree is the working tree of wt1 on top of `e82d54d4`.
- **Why stopped:** U4b hit the re-fix cap (3 fix rounds: R1, R2, R4-fix) and the same finding class came back two rounds in a row (implement flow §3 cap + review flow §4 「same class two rounds → raise the class to the user」). The question below must be put to the user before any more fixing.
- The user wants: short answers, plain words, every noun either the user's own word, a real file/command name in backticks (said what it is at first use), or a generic term — never design-doc labels (the user corrected this many times; feedback rows `e5fd629`, `ba21698`, `5fadb82`). Big picture first; one question at a time; progress as a numbered list of the work by name.
- Subagent cap = 5 running at once, all families combined. Replace a worker past ~500k context (handover memo first).
- Unshipped data gets no migration (ADR-0398). `src-tauri/bindings/*.ts` line-ending noise and untracked `.claude/explain/` are not ours — never stage.
- Real-claude tests (CI skip list) and the claude / codex lanes are deferred to the end of step 21 (user agreement), before the freeze.
- Machine note: `core.hooksPath` points into another worktree, so QA mutation always runs in an isolated copy (cold build). Bash moves foreground waits to the background at ~180 s.

## One-line state · next first action
Server stage 16 = library mounts (step 21), 5 pieces: 1–3 done; **4 = mount commands: U4a (add / remove / relocate, status per mount) committed `64897a7b`, CI green; U4b (rename + `agent.new` re-check) coded, reviewed through round 5, uncommitted, blocked**; 5 = help texts / reference / live scenes not started. Then 17 file move/copy/trash, 18 their commands.
**Next first action: ask the user the stop-answer question (below), then send the chosen fix to a fresh coder (handover `handover-u4b.md` + this file), re-review (lead `a021c296…` context ~390k — prefer fresh), full QA, commit U4b, then U5.**

## The question for the user (put it in plain words)
Situation: a confirmed rename (`library.renameMount` with `yes` + `root_generation` + `plan_digest`) stops part-way (a file write fails, or the agents' save is refused). Round-4 fix made the stop answer carry the `plan_digest` of "what is left", so repeating with the answered tokens finishes in one call. Round 5 (lead must + Codex must): that digest is computed after the rename's lock is released, so a preset edit that slips in between can add a new `from/…` file into the digest — the repeat would then rewrite a file the caller never saw (breaks ADR-0405 decision 1).
- **(a) ★ recommended — no digest in a stop answer.** After a stop, the repeat answers the remaining plan and needs a fresh `yes` (one extra round trip, only after a rare stop). The caller always sees what will be rewritten; the whole "stop digest" surface disappears. Docs/tests change to "repeat → confirm the remaining plan"; ADR-0405 needs a line (ADR-0400 d12 / ADR-0335 "the same call finishes it" no longer literal for confirmed renames).
- **(b) compute the stop digest inside the lock** (lead's fix: `Sides::of` on the catalog at the stop point, lost-mount files moved to the unread side). Keeps one-call finishing, but also needs a call on ADR-0405 d2 vs d3 for a mount lost during the rename (lead hold: post-stop digest lets the repeat finish leaving that mount's files unread without the caller ever seeing a plan listing them).
User's general preference seen this session: simpler, fewer exceptions, "confirm what you saw" (chose A with Terraform / force-with-lease conventions).

## Repo state (branch v0.5.0/feat/preset-library · worktree wt1)
- HEAD `e82d54d4`, 0 ahead / 0 behind origin at save time (this handoff commit will be ahead until pushed). origin/master is 4 commits ahead of the branch base (not merged in).
- This session's commits: ADR-0402 `dc9800cd` + body `97bb650e` + fixes `1b9bc021` `307b854a`; U4a `64897a7b`; ADR-0405 `a5d3d9ac` + body `e82d54d4`.
- **Uncommitted (U4b, gate NOT passed):** asset-library `src/rename.rs` (new), `gate.rs`, `catalog.rs`, `mounts.rs`, `config.rs`, `lib.rs`, `host.rs`, `agent_edit.rs`, `store.rs`; agent `commands.rs`, `library_commands.rs`, `manager.rs`, `profile.rs`, tests `activation.rs`, `command_declarations.rs`, `ts_export.rs`, bindings `commands.schema.json` + new `LibraryRenameMount{Args,Ok}.ts`; daemon `connection_core.rs`, `control/commands.rs`, `control/help.rs`, `tests/control_agent.rs`; `prompts/engram-help.md`; `docs/reference/structure/asset-library.md`. Last snapshot tree `e362a8fd`.
- Engram core repo (`I:/Engram/core`, same repo as `I:/Engram`): feedback rows `e5fd629`, `ba21698`, `5fadb82`, `57cef08` (implement: user-visible name collision), `daf758d` (heredoc trap). Not pushed by main.

## Verification state
- U4a: committed after deep review (5 rounds) + QA full regression PASS (workspace 6649, frontend 2884, gates 34/34, mutation 2/2); CI green on `64897a7b`.
- U4b: review rounds 1–3 PASS (lead, user-data-integrity axis, Codex); QA full regression PASS on tree `50ce7fc8` (round-3 tree: workspace 6672, frontend 2884, gates 34/34, bindings no drift, mutation 2/2). **After that, decisions A and B + round-4 fixes changed the code; the current tree `e362a8fd` was tested only by the coder (asset-library 680, agent 1895, daemon 933, ts_export) — no QA run, and review round 5 = FIX.**
- Rerun: `cargo test -p engram-dashboard-asset-library` · `cargo test -p engram-dashboard-agent -- --test-threads=4` · `cargo test -p engram-dashboard-daemon -- --test-threads=4` · full QA per `.claude/skill-bindings/qa.md` (briefs to copy: `.review-s25-d9/s21-code/u4b-qa-brief.md`).

## Failed approaches (do-not)
- A stop answer digest computed after the hold (round 4 option a as built) — admits files edited after the lock (round 5 must).
- Codex re-reviews via "resume the same thread" return `WRONG THREAD` — always start a new Codex task with the earlier findings in the prompt.
- Inline Python / heredoc with Windows paths or regex in the main session mangles backslashes (`\a` → bell char) — write scripts with the Write tool and run the file.
- After editing a command doc comment in agent `commands.rs`, always rerun `cargo test -p engram-dashboard-agent --test ts_export` (QA caught a stale `commands.schema.json` once).

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 log --oneline -3 && git -C I:/Engram/apps/engram-dashboard-wt1 status --short | grep -v bindings/ && gh run list --branch v0.5.0/feat/preset-library --limit 1`

## Decisions
User:
- library.json 잘못된 줄 처리 · 상황: 손으로 고친 마운트 한 줄이 규칙 위반일 때, 되돌릴 수 있음 · 목록 전체 손상(A) ★ / 그 줄만 제외(B) · 「A가 좀더 구현이 간단한가? 이건 그냥 구현 간단한걸로해.」 · 같음 → ADR-0402 d7.
- U4a 리뷰 잔여 처리 · 상황: 커밋 메시지에 남은 권장 10건 · 1,3,4,8,9,10 U4에 포함 ★ · (묻지 않음) 요지: 사용자가 「피드백 올려 대면 답변 고쳐야될것같아」 후 시작 지시 · 미상.
- 앱 시작 직후 정상 마운트 "사용 불가" 기록 · 상황: 응답 없는 드라이브가 첫 확인 시간을 다 씀 · 고침 ★ / 안 함 · 「ㅇㅇ 그렇게 고쳐.」 · 같음 · 조건: 「하나 한다음에 시간제한 실패하면 그냥 다음거 넘기고 쿨타임 다시 주는게 좋을듯」 (설명 후 동시 확인 방식 유지로 정리).
- 시작 검사가 시간 없이 끝나는 문제(옛 3번) · 상황: 0초 남은 첫 명령이 검사 → 철자 판정 · 고침 ★ / 기록만 · 「저런것까지 우리가 검사해야됨?」 → 설명 후 기록만 · 갈림 → T-121.
- Engram 자기 폴더 겹침 제한 · 상황: 데이터·실행·내장 폴더, 실행 폴더는 부팅마다 비워짐 · 실행 폴더만 막음 ★ / 전부 허용 · 「그냥 전부 허용해. 예외처리 시팔 계속 추가할래?」 · 갈림 → ADR-0402 d2.
- "사용 불가" 해제 조건 · 상황: 겹친 쪽을 지워도 유지 · 지금대로 ★ / 자동 해제 · 「음? 뭔소리야 껏다 키면 되는거 아님?」 · 같음 → ADR-0402 d6.
- 거절된 추가의 빈 폴더 · 순서 바꿈(검사 먼저) ★ / 그대로 · 「순서 바꿔.」 · 같음 → ADR-0402 d3.
- 첫 시작 저장 실패 뒤 삭제 재시도 · 특별 처리 ★(처음) → 그대로(바꾼 추천) · 「그대로 두는게 좋을것같아」 · 같음 → ADR-0402 d7.
- 기본 마운트 이름 · 상황: `library`가 제품 용어와 겹침 · `user` ★ / `default_library` / `local` · 「그냥 디폴트 라이브러리로해.」 · 갈림 · 조건: 「폴더 이름도 바꾸고」 → ADR-0402 d1.
- 이름 변경 확인 목록 밖 파일(A/B) · A(목록 다르면 거절) ★ / B · 「ㅇㅇ A로 해.」 · 같음 · 근거: 관례(Terraform, force-with-lease, LSP) → ADR-0405 d1.
- 멈춘 이름 변경의 `to` 예약 · 그대로 ★ / 예약 · 「그대로둬.」 · 같음 → ADR-0405 d5.
- 건너뛸(읽을 수 없는) 파일 확인 · B(먼저 묻기) ★ / A · 「ㅇㅇ 쭉 진행해」 · 같음 → ADR-0405 d2.
- 자율 진행 · (묻지 않음) · 「그거 끝나면 계속 이어서 자율모드로 진행해.」 · 「ㅇㅇ 잘게」 · —.
Main (reported): U4a split into U4a/U4b; load-time overlap judgement on the first caller's budget (ADR-0402 d4); a starved scan records nothing (d5); a held / never-walked mount makes every rename ask (B interpretation — tell the user); stop for a lost mount + record it (ADR-0405 d3); repeat must be the same rename (d4); one classifier `Sides::of`; empty-plan supplied-digest compare.
Open: the stop-answer question above (blocking U4b).

## Pending records (do at the U4b commit)
- T-121 additions (U4b): a held mount's `why` keeps the other mount's old name after it is renamed; a file deleted outside Engram is stat'ed once per rename until a scan; a rename touching a dead share can exceed the CLI wait (U5 help: read `library.status` after a timeout); duplicate held mounts on one folder can pass the repeat folder check.
- Main's doc pass still owed: `commands.rs` `library.renameMount` arg docs (`root_generation` / `plan_digest` also required when references are left unread); declaration line for whatever the stop-answer decision is.
- Re-anchor test comments citing "decision A / B, user 2026-11-23" to ADR-0405.

## References
- Step TRD `docs/process/S25-preset-library/trd-library-mounts.md` (§3.2 rename, §9, §10 U4/U5); ADR-0396, ADR-0398, ADR-0400, ADR-0402, ADR-0405.
- Briefs and handovers (git-excluded): `.review-s25-d9/s21-code/` — `coder-brief.md`, `u4b-unit.md`, `handover-u4a.md`, `handover-u4a-2.md`, `handover-u4b.md`, review briefs `u4b-review-brief.md`, `u4b-r2..r4-brief.md`, findings `u4b-round1.md`, QA brief `u4b-qa-brief.md`, diffs `u4b*.diff`.
- Code: asset-library `rename.rs` (`rename_mount`, `rename_plan`, `rename_in`, `Sides::of`, `RenameAnswer.plan_digest`, `mount_gone`), `gate.rs` `Gate::rename_mount` / `record_unusable`; reference `docs/reference/structure/asset-library.md`; `docs/tracking.md` T-121.
