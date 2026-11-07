# Handoff: stage 3 transport — master merged twice + qa binding fixed · next = 3-1 U5 → 3-2

## Know first
- Project rules changed under this branch today (master S30 merged in): CLAUDE.md is now the slim version; new `docs/` text and code comments are **English** (rewritten paragraphs too — CLAUDE.md 「언어」); branch/commit procedure lives in `docs/handbook/branch-commit.md`; full regression is local before commit (ADR-0340, procedure = `.claude/skill-bindings/qa-verify.md`); deletion = `bash ~/.claude/references/recycle.sh <path>` (`rm -rf` is blocked).
- The qa binding is master's new skeleton (known failures · tracking records · baseline copy · triage · recycle cleanup) plus this branch's two edits (below).

## One-line state · next first action
Branch `v0.4.1/refactor/transport-attach`, HEAD = this handoff commit (code/binding last = `c2ac9dd8`). Work tree clean. **Not pushed** — origin is at `dd0499a7`, local is 53+ commits ahead.
This session (no stage-3 code work):
- `31069634` merge master `671b7ece` (S30 skill bindings/docs · S31 gate regex · S32 codex contract). Conflicts were md only: qa.md + CLAUDE.md took master (4k block dropped; `shell gate 3` stays in ci.yml) · decisions/README + tracking kept both rows · testing-strategy took master's role lines and carries the branch's net facts + shell gate 3 (rewritten in English) · stale lines fixed: architecture.md crate graph (no shell → net; net → protocol unconditional; net bullet in English) · T-60 「다음」 → `docs/handbook/branch-commit.md`.
- `a4ffa4e9` merge master `9152bcaa` (qa binding skeleton follow-up; no conflicts).
- `c2ac9dd8` S21 chore(qa-binding): ① qa-verify backend job gets the `ws_accept --list` line (「네 줄」) ② qa.md 「Rust 테스트 판정」: net features-off line runs 0 tests on this branch (tests moved to protocol, ADR-0310 D9) → PASS = `__EXIT=0` + a `running 0 tests` line, reported as 「컴파일 PASS (시험 0개)」, not a test pass; remove with the net crate in 3-3 U6.
**Next first action = 3-1 U5** (TRD §6-1 U5 row: delete protocol → command Cargo edge + protocol dependency-cap gate + docs). Gate copies now live in ci.yml (+ qa-verify extracts it) — the old 「three copies incl. CLAUDE.md 빌드·검증 명령」 no longer exists (slim CLAUDE.md has no gate lines); check where the gate text must go under the new binding before editing. `/engram-review doc` for doc changes.
Then 3-2 (shell attach — §6-2 U1~U5 · GUI `/qa` real measurement) → 3-3 (daemon attach · remove net — §6-3 U1~U6). Before 3-2 read TRD §6-2 U2 「3-0 착지가 더한 주의 셋」.
Progress: stage 3 ≈12 of 23 units (3-0 U1~U7 · 3-1 U1~U4).

## Verification state
- Full regression on the final tree `4f585839` (= HEAD `c2ac9dd8` tree, verified by `git rev-parse HEAD^{tree}`): PASS — workspace 61 result lines · 5299 passed · 0 failed · 35 ignored · lib_unit 1351 · gates 31/31 · frontend 2802 + tsc · `ws_accept --list` 12 · net features-off = compile PASS (0 tests).
- Earlier today on tree `c2a5f281` (= `31069634`): same numbers + codex lane 13/13 (lanes.tsv updated) + Job lane: only known failure `ws_e2e real_process::ignored_daemon_kill_cleans_pty_child` + GUI smoke PASS (claude terminal agent spawn/output/kill).
- Not measured: the intermediate merge commit tree `a4ffa4e9` alone (differs from the measured final tree only by the two binding md edits).
- Rerun = `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-detached.ps1 -Command "cargo test --workspace -- --test-threads=4" -WorkDir "I:/Engram/apps/engram-dashboard" -LogFile "<path>.log"` (full CI-three-jobs procedure = qa-verify 「전체 회귀」).

## Decisions
- net features-off verdict: binding exception, not a revived test in net. Reason: restoring a test in net splits ownership of frame shape/golden again (branch moved it to protocol); the only test left to write would be a re-export identity check that always passes. Told the user the exception is framed as a compile check so it does not conflict with the qa skeleton's sealed 「0 tests is not a pass」; user did not object.
- User conditions this session: 「qa 에서 바인딩의 알려진 실패(T-52 stop_smoke · ws_e2e ignored_daemon_kill_cleans_pty_child)만 남으면 코더에게 돌리지 말고 진행해」. Earlier standing: user delegates 「쭉쭉 진행 · 끝나면 핸드오프」 per unit with review · QA · commit; multi-daemon work waits for a network/structure lecture to the user first; token-saving notes go to handoff, not skill briefs.
- Unresolved: none blocking.

## Do-not / lessons
- Do not answer subagent interim notifications (「stopped with background work still running · not reported yet」) with user-facing lines — user flagged 20+ 「QA 진행 중」 lines as noise; stay silent until the final hand-back.
- Cause of those notifications: QA worker wrote its own polling driver + Monitor `tail -f` watcher because the binding gives no way to wait for `__EXIT` across 25 detached commands. In the next QA brief, tell the worker to wait inside a single tool call (no turn-ending polling) until a shared driver exists.
- In `I:/Engram`, commit feedback files with an explicit pathspec only — a `git add <dir>` there once staged an unrelated untracked file (undone).

## Token-saving practice (still in force for this topic)
- Coder 150k~500k/unit · Claude reviewer read-only 160k~250k/round · codex ~80k/round · QA 110k~230k/run (today: 134k and 111k new input).
- Claude reviewer = read-only; first round = Claude + codex lens 1 (diff) + lens 2 (call sites, skip when only new API / no consumer outside transport); confirm rounds = delta only.
- Coder past 300k context → new coder with a short brief. Code and docs in parallel when files don't overlap.
- QA = worker-senior (『qa 실행자』 = 주도 상급); gate blocks needing the `rg` function must be `source`d.

## Open notes (carried)
- 3-3 U4: no auth-wait slot · no operating slot (upgrade slot covers up to Live) · no cap on accepted peers in operation (recorded in TRD 「TRD 와 다른 점」).
- Masking received `Close.reason` = TRD §9 follow-up.
- Pre-existing harness defect parked: `testing.rs` `MemTx::ping` (MemoryEndpoint) ping stall · two-stage write-stall wait.
- `mask.rs` not in gate ③ pure-layer list (file-level gate).
- transport `[dev-dependencies] tokio` `rt-multi-thread` (U6b test) — no production graph change.
- Tools: `py` not `python` · `rg` = `C:\Users\kimsunzun\AppData\Local\OpenAI\Codex\bin\0354bc6c584f210c\rg.exe` · codex review hitting Bash 600 s → recover from `tasks/<id>.output` · worktree `.claude/worktrees/stage3-protocol` idle · leftover temp dirs for the user: `I:\Engram\apps\rv-u1a-target`, `%TEMP%\engram-review\31u2\{tgt,blocks}`.

## Improvement feedback filed today (Engram repo, all 미반영)
- `core/claude-global-shared/references/global-feedback.md` — interim-notification noise · QA worker polling cause (`9d5ca04`, `b0a1383`).
- `core/claude-global-shared/skills/engram-qa/feedback.md` — qa-verify ws_accept drift (now fixed on this branch; master still has 3 lines) · net 0-test verdict · `$QA_TMP` cleanup blocked / own watcher holding files · shared wait driver · plain-shell-agent brief error (`4b92c1c`, `b0a1383`).
- Note for master: master's ci.yml has no `ws_accept` (checked `9152bcaa`) and its qa-verify lists 3 `--list` lines — consistent there; both the gate line and the binding line land on master together when this branch merges.

## Resume command
`git -C I:/Engram/apps/engram-dashboard log --oneline -5 && git -C I:/Engram/apps/engram-dashboard status -sb`

## References
- TRD stage 3: `docs/process/S21-crate-boundaries/trd-3-transport.md` (§6-1 U5 row, §6-2 U2 cautions).
- `.claude/skill-bindings/qa.md` · `.claude/skill-bindings/qa-verify.md` · `docs/handbook/branch-commit.md` · `docs/reference/architecture.md`.
