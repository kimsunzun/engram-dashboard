# Handoff: S25 step 20 (one create body) — P1 coded + reviewed (2 PASS, Codex must being re-asked); md translation done; user is out (autonomous)

## Know first
- The user went out and said 「쭉 진행해 나 외출」 — autonomous. Do not commit P1–P3 code until the U2 hold is answered (review hold rule); keep going otherwise.
- **U2 is RE-OPENED and pending with the user.** The user had accepted (b) "accept TIMEOUT" on main's claim that a healthy codex answers in ≈130 ms. Review found `crates/engram-dashboard-agent/src/backend/codex/usage_probe.rs:38-39`: a healthy `initialize` was observed at ≈ 9.7 s right after a daemon restart. So a healthy codex `agent.spawnInto` can answer `TIMEOUT` (7 s seat deadline) while running and placed; the 10 s `HANDSHAKE_BUDGET` may be tight too. Ask the user again: (a) longer deadline for spawn verbs (seat + CLI) vs (b) accept. Main told the user it would re-ask before commit. Also tied to U2: on a slow success the notice set is recorded as shown while the caller gets TIMEOUT (no live effect until step 8).
- **P1 code is UNCOMMITTED in the working tree** (gate not passed — review FIX round pending). Files: `crates/engram-dashboard-agent/src/{manager.rs,commands.rs}`, `crates/engram-dashboard-agent/tests/activation.rs`, `crates/engram-dashboard-agent/bindings/commands.schema.json`, `crates/engram-dashboard-daemon/tests/control_agent.rs`, `prompts/engram-help.md`. Review snapshot = git tree `831855016fdc2088616738cbcac3cb6e26b6f92d` (base HEAD `006b4f12`).
- The 26–39 ` M src-tauri/bindings/*.ts` files are line-ending-only noise — never stage them (check `git diff -w --ignore-cr-at-eol`).
- Never push this session's work unless asked: the user said no push (「푸쉬없이」). Local regression is the gate (ADR-0340).

## One-line state · next first action
Step 20 = unify the daemon's two "create a new agent for a folder and start it" bodies (WS `SpawnByCwd` arm vs bus `agent.spawn --cwd` `create_and_start`) into one agent-crate body; shell `agent.spawnInto` stops dropping reasons/notices; plus wire `SpawnByCwd.deliver_notices` and `Error.created_agent_id`. ADR-0348, TRD `docs/process/S25-preset-library/trd-create-body.md` (review PASSED, `006b4f12`). P1 (agent body + bus adapter) coded by a worker-senior; full regression green (5564 Rust tests, 34 gates); 6 mutations bite.
P1 code review (deep: lead PASS, risk-axis PASS, Codex FIX → must WITHDRAWN on re-ask (「참고」)) — **verdict PASS**; Codex was re-asked about its must "tests count roster notifies but do not prove the notify follows activation / daemon fixture sees no frames"; facts sent: notify is inside the result-dependent branch, refusal paths assert 0, coder's "notify before activation" mutation turned 8 tests red. If Codex keeps it → coder adds an ordering observer test; if it drops it → no change.
**Next first action:** (in progress at save time — a later P1 coder round may already exist) send the P1 coder (new worker-senior if the old one is gone; give it the P1 brief `scratchpad/coder-p1-brief.md` content + the list below) one FIX round with the adopted 권장:
1. body test with `NoticeDelivery::Withheld` asserting the host receives `Withheld` (lead 1);
2. created-asleep name test with a wrapper host that registers under a different name (e.g. `gamma(2)`) (lead 2);
3. extract a shared `new_profile(cwd, name, command)` used by `register` and `create_and_start` (lead 3);
4. source pin that the `verb_spawn` cwd arm passes `llm_creation_refusal` (door ③ wiring) (risk 2);
Then re-review (only reviewers who raised; delta), then main applies the batched doc fixes once (list below), then P2 → P3 (briefs from TRD §7), then `/engram-qa` incl. the §6-2 live run, then ask U2, then commit per piece.

## Batched (main fixes at the last round, no coder)
- `commands.rs:517` `AgentCommandHost` trait doc: add `create_and_activate` + its contract (store refusal of the add → `created:false`; failure after the add → `created:true`, row stays).
- `activation.rs:1772`: PTY "falls back to another folder" → unverified observation (only measured: ConPTY starts with a missing cwd, `activation.rs:~750`); `:1609` "(review finding 1)" → "TRD S25 step 20 review finding 1".
- `commands.rs:1291` and `:13`: docs describe post-P2 state — qualify or leave if P2 lands next.
- `src/commands/agentCommands.ts:89` and `src-tauri/src/layout/commands.rs:1781` comments now false (P3 touches the latter).
- `docs/reference/structure/session-path-ownership.md:673-675` stale (doc pass).
- `commands.rs:1167-1171` moved comment → one line + ADR id.
- `RefuseOnStart` comments (~3721, ~3745) and TRD §4-2 `CreatedAsleep` store row: a store failure after the add no longer happens on the real manager — mark defensive.
- `commands.rs:1280` / `manager.rs:161` / `:1384`: concurrent delete can make `NotCreated` after the add — say "nothing is registered now"; LaunchRaced arm reachable narrowly.
- Keep a one-line "do not roll back" note with ADR-0122 on `NotStarted`/`create_and_activate`.
- `connection_core.rs:1727-1732` asymmetry comment (P2 deletes it).

## Repo state (branch v0.5.0/feat/preset-library · wt1)
- HEAD `006b4f12`; ahead of origin by 207 (includes the master merge); nothing pushed this session. master merged in at `907449da` (S33 translation + S21 transport-attach; 9 conflicts resolved; full regression green).
- Uncommitted: the 6 P1 files above (gate not passed) + bindings line-ending noise.
- origin/master has moved further since (7+ commits at last check) — merge again just before step 20 lands or at the next natural point.

## Verification state
- Ran: master merge full regression (Rust 5551, vitest 2762, tsc, 34 gates, bindings sync) — green. P1 coder full regression (5564 passed, 34 gates, bindings sync) — green; 6 mutations red-then-reverted.
- Not run: QA (`/engram-qa`) for step 20 — after P3, including the mandatory live run (TRD §6-2: R1–R8, R7c fake app-server with `fakes.log` pid tracking, before-run on the head before P1).
- Rerun: `cargo test --workspace -- --test-threads=4`; full procedure `.claude/skill-bindings/qa-verify.md` 「전체 회귀」.

## Translation (S33 follow-up for this branch)
- Done & committed: ADR 37 (`9b4aaf36`), research 21 (`ae12490b`, `1d742858`), S25 TRD (`a70e3ecd`), S25 PRD (`b8665ca7`) — masked pipeline + review-fix per batch. Kit copy + term sheet: `C:\Users\kimsunzun\AppData\Local\Temp\claude\I--Engram-apps-engram-dashboard-wt1\b31462c6-bddf-409e-817d-4af79896887f\scratchpad\kit` (`terms-s25.md`).
- Excluded on purpose: `docs/process/step-log.md`, `docs/tracking.md` (shared — master side), `docs/decisions/README.md` (generated). **Open question for the user:** `prompts/engram-help.md` (runtime prompt injected into agents) — translate or not.
- Code comments of this branch's files: user decision (b) — translate at the END of S25 (guide `I:\Engram\apps\engram-dashboard-wt2\.claude\handoff\attachments\s33-translation-kit\GUIDE-translate-your-branch.md`), then append translate SHAs to `.git-blame-ignore-revs` before merging.

## Decisions
User:
- 물은 것 · 상황: shell `agent.spawnInto` vs daemon `agent.spawn`, two create bodies drifted, codex link-verdict bug · 선택지 unify now ★ / later · 고른 것 「이부분 너가 알아서 쭉 진행」 · 같음 · 조건: 「실테스트 필요할듯. 지금 귀찮아서 너무 대충 넘기는게 많아」 (real-run verification mandatory — ADR-0348 decision 7).
- 물은 것 · 상황: screen/WS create failures get the ban glyph · 선택지 yes ★ / no · 고른 것 delegated (same message) · 같음.
- 물은 것 U2 · 상황: codex `agent.spawnInto` waits for link verdict vs 7 s seat deadline · 선택지 (c/b) accept ★ / (a) longer deadline · 고른 것 「추천으로 가」 · 같음 · **re-opened** (9.7 s evidence) — pending.
- 물은 것 U3 · 상황: created-asleep answer on `agent.spawnInto` · 선택지 (a) id in `args.agent_id` ★ / (b) name in text · 고른 것 「1번 a」 · 같음.
- 물은 것 U4 · 상황: claude exiting right after launch answered "started" · 선택지 (a) known limit + T-81 ★ / (b) fix now · 고른 것 「너 추천으로 해」 · 같음.
- 물은 것 translation scope · 상황: 119 code files (96% legacy Korean) + 66 md · 선택지 (b) md now, code at S25 end ★ / (a) all now / (c) all at end · 고른 것 「b 로 하자」 · 같음.
- (묻지 않음) push · 고른 것 「푸쉬없이」 — no push.
Claude (main):
- U1 live launch block → debug-only daemon hook `ENGRAM_QA_BLOCK_CWD` (TRD §3-7, removed at step 8); rejected deferring R4 to step 8 because the user demanded real runs.
- `deliver_notices` WS field (shell bus true, webview false until step 11) to keep ADR-0341 once-per-agent; rejected leaving Withheld (re-delivery).
- Translation: excluded step-log/tracking (shared), README index (generated); term sheet `terms-s25.md` (relink, bundle, launch materials, 판 edition/version, 걷기 by context…).
- tracking T-81 added (claude early exit); T-76 closes when step 20 lands (delete row+section then).

## Do-not / lessons
- Temp-index snapshot: get the real index path BEFORE setting `GIT_INDEX_FILE` (`RI=$(git rev-parse --git-path index)`), else the tree is nearly empty.
- Translation: parallel workers render shared terms differently — give the term sheet from the first batch on; source lines with a one-backtick span `` ` `` or nested 「…「…」…」 make checker false positives (accepted after hand review).
- Codex reviewer re-ask must resume the same thread (`--resume`, first line WRONG THREAD check).
- User feedback: the user found main's explanations too jargon-heavy — answer in plain words, short, conclusion first.

## Improvements to file (said in chat, not yet written to feedback files)
- Translation kit: add a cross-batch term-sheet step (main builds it from batch 1 and passes it to later batches).
- engram-review: TRD reviewers caught a live-test filter that could never match (R7c `--cd`) — test-procedure lines need the same code-check as design lines.

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 status --short | grep -v src-tauri/bindings; git -C I:/Engram/apps/engram-dashboard-wt1 log --oneline -6`
