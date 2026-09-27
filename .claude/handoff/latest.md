# 핸드오프: codex 턴 도중 입력 단순화 — 문서·코드 리뷰 반영 · /qa full PASS(GUI 포함) 완료(로컬 커밋) — 남은 것 = 아침 보고 · 사용자 질문 · push 결정

(English body — user decision 2026-09-24. Chat with the user in Korean. The user went to sleep and delegated 「쭉 진행」; the planned work is now DONE. ★When you ask the user something, make it the LAST thing in the turn and stop★ (user complaint: 「물어보고나서 툴들이 쭉쭉 나오는데 나한테 물어보는게 의미가 있어?」). One question at a time, recommendation first. The user wants simple (「단순하게 가고 복잡할것같으면 먼저 나에게 보고해」).)

## One-line state

S1–S5 (previous session) + this session's ✕ fix, S6 docs (ADR-0235), `/review doc` full (FIX → applied), `/review code deep` (FIX → applied, one item left for the user) and `/qa full` (PASS: standard gates + GUI on release builds, codex 6/6, claude 4/5 with 1 not triggerable) are all committed LOCALLY on `v0.3.2/feat/json-midturn-queue`. Nothing pushed.

## Next first actions

1. Give the user a short morning report (Korean): done items, verification, and the pending items below; ask ONE question last.
2. After the user's answers: public-repo privacy decision → push → CI green → merge per CLAUDE.md 「브랜치·커밋」 (merge-tree/commit-tree/update-ref procedure; `--no-ff`-equivalent; merge body lists what went in). Do not push before the privacy decision.

## Pending with the user (list them, expand one)

1. ★Open review item (reviewers split — main must not decide)★: while the writer drains at a tool end (a few ms), an item typed in that window is steered at the same boundary, so its ✕ disappears early. Both codex blind lenses: FIX (capture ids/arrival cutoff at the tool end). Claude reviewers did not flag it. Main's view: not a defect (a next sampling is certain after a tool end, so it gets answered; GUI T2 showed steer 0.1 ms after the tool end, echo 3.9 s before tool 2). Recommend: keep. Code: `backend/codex/transport.rs` `take_steer_locked` ~2054 / `note_tool_item` ~3493.
2. ★Correction owed★: main told the user 「목록 최대 32개, 33번째 거절」 — `INPUT_QUEUE_LIMIT` counts the whole transport queue (mail + held Direct bubbles too), so a user input can be refused earlier. ADR-0235/plan already corrected.
3. ADR-0130 reopen notice (from `/qa standard`, not a gate): this BRANCH (earlier phases, not this session's range) added production edges `control → connection_core::INPUT_LOCKED_REFUSAL` in `crates/engram-dashboard-daemon/src/control/agent.rs:37` and `control/commands.rs:32`. The binding says the main should reopen ADR-0130 condition ② → user decision.
4. Public-repo privacy decision (push blocked until then).
5. Observations to tell (no decision needed unless the user wants): codex followed a steered message and dropped the rest of the original instruction in GUI T1 (vendor behaviour); ✕ 「cancelling」 interim is usually invisible (backend answers in ~8–22 ms); claude 「not removed」 could not be triggered (1–11 ms window) — the shared front rule was verified on codex (T6); the QA claude agent ran with the app's default `--permission-mode bypassPermissions` (fact, flagged by the harness pattern matcher); claude path has no cancel/lifecycle debug trace (QA worker suggestion, not done); `getSnapshot` on a codex JSON agent drops structured events with 354 WARNs (pre-existing `B7 미배선`, triggered by a probe only).
6. Optional/later: claude CLI behaviour under a usage limit (unknown); revisit M10 recorded-but-unanswered (「나중에 한번 살펴보자」); watch Direct `turn/start` failure frequency (「나오는거 보고 대응하자」); daemon-owned list simplification discussion; CLAUDE.md measured numbers are dated (workspace now 58 lines · 3158 pass; `lib_unit` 342) — update at merge time via `/review doc`.

## Decisions this session (user — authoritative record = `simplify-plan.md` 「User decisions」 + ADR-0235)

- ✕ answered not-removed → drawn like sent (✕ hidden), both backends, live + reattach: 「이미 넘어가면 codex와 같이 못누르게 하면 되지 않음?」.
- Six accepted consequences confirmed one by one (quotes in ADR-0235 「받아들인 결과」): halt lifts on a new user item · ✕ on it doesn't re-halt · failed Direct `turn/start` bubble stays (watch) · single rule 「push at the next chance」 decided by event type · persistent failure retries once per new item · recorded-but-unanswered steer stays (revisit later).

## Repo state

- Branch `v0.3.2/feat/json-midturn-queue` (wt1). Remote tip still `37ecc5e`. ★Nothing pushed★.
- Commits this session: `071f50a` ✕ fix · `ddda492` `426d124` S6 docs · `1d0ebb0` doc-review fixes (+ agent binding regen) · `7a57d52` code-review fixes · `014df62` mid-point handoff · `d2bfacd` step-log · + this handoff commit.
- Uncommitted: only `src-tauri/bindings/*.ts` line-ending noise — leave.
- Release builds at `014df62` exist in `target/release/` (daemon + client with embedded frontend). QA data dirs are in this session's scratchpad (volatile, OK to lose).

## Verification state

- `/qa full` on `014df62` = PASS. standard: build · workspace regression (58 result lines · 3158 passed · 0 failed · 20 ignored) · shell integration 4 targets · `lib_unit` 342 · fmt · isolation gates · dependency ceilings · `tsc` · `npm test` 70 files / 1344 — all `__EXIT=0`.
- GUI (release builds, fresh `ENGRAM_DATA_DIR`, daemon pre-started with `RUST_LOG=warn,engram::codex_steer=debug`): codex T1 tool-end push · T2 three tools (steer 0.1 ms after tool 1 end, echo 3.9 s before tool 2) · T3 parallel tools (push at the first end) · T4 answer-only waits for turn end · T5 ✕ held → removed (interim state not visible) · T6 sent survives reload, not-removed row drawn as sent — all PASS. claude C1 tool boundary · C2 ✕ held · C4 answer-only · C5 reload — PASS; C3 not-removed — NOT RUN (window 1–11 ms).
- Reviews: `/review doc` full FIX applied; `/review code deep` FIX applied (except pending item 1); no re-review after fixes.
- NOT run: CI (not pushed). 1 GUI pass = smoke, not race-free proof.

## Traps

1. CRLF files: `backend/codex/transport.rs`, `commands.rs`, `docs/process/step-log.md`, `docs/reference/architecture-overview.md`, `QueuedInputList*.tsx` — rustfmt flips to LF; restore CRLF on bytes.
2. Builds/tests only via `scripts/run-detached.ps1 -Command -WorkDir -LogFile`, one per call; agent/daemon tests need `-- --test-threads=4`.
3. Port 1420 is owned by wt2's vite → GUI QA must use release builds (`cargo build --release -p engram-dashboard-daemon` + `npm run tauri -- build --no-bundle`). Pass the same `ENGRAM_DATA_DIR` to a pre-started daemon and to the app to get daemon-side `RUST_LOG` and avoid auto-restoring stale profiles in `target/release/data`. Record short DOM states with an in-page MutationObserver (100 ms polling misses 「sent」).
4. Parallel workers in one worktree: forbid git writes; main commits.
5. codex reviews via `codex:codex-rescue`, foreground, `--fresh`, read-only (`~/.claude/skills/review/codex-call-spec.md`).
6. `<plugin>` hooks misfire every turn — ignore.
7. Next ADR number = 0236.

## Stop conditions

- Any user-visible behaviour choice not in `simplify-plan.md` → ask the user.
- Never without the user: push, master merge, `v*` tag, force-push, touching other worktrees.
