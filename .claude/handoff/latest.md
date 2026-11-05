# Handoff: stage 3 transport — 3-1 done · 3-2 U2 (A/B/C) done · next = 3-2 U3

## Know first
- Two user decisions made at the end of this session (decision 2 is now ADR-0344; decision 1 is a parity choice — note it in the TRD U3 landing, no ADR needed) (the ADR call was interrupted by the user — 「하던거 다 끝나고 핸드오프 하면 됨」). Record them before U3/U5 work relies on them; ask the user before writing if unsure:
  1. **Cold-start connect failure (U3): show 「끊김」(Down) immediately**, like today — no 5-try "재연결 중" phase on the first connect. (Main recommended; user: 「쭉 진행해」.)
  2. **Version-mismatch action (U5) — RECORDED as ADR-0344:** the band carries ONE action button 「데몬 끄기」 (next to the cause text and ✕), with a confirmation step (agents of the old daemon stop); stop path = ADR-0343 decisions 2–3; no automatic restart; button only when the daemon is OLDER than the app; tray 「데몬 끄기」 uses the same path; post-restart version re-check text. User: 「행동 버튼하나 둬 그럼」 after an OSS/industry survey (`docs/research/banner-action-buttons-2026-11-05.md`). Note: when the user says 「관례」 they mean open-source/industry convention, not the project's ADRs.
- Graceful stop of any shipped old daemon is possible: authenticate with the token and the daemon's OWN version from `daemon.json`, send only `StopDaemon` (ADR-0343 decision 2 — single-purpose exception to the handshake rule). Fallback = identity-checked kill (pid + start time through one handle).
- Project rules (unchanged from last handoff): new `docs/` text and code comments are English (also inside Korean docs — I got this wrong once in a coder brief); branch/commit procedure = `docs/handbook/branch-commit.md`; full regression local before commit (ADR-0340); delete = `bash ~/.claude/references/recycle.sh`.

## One-line state · next first action
Branch `v0.4.1/refactor/transport-attach`, HEAD = this handoff commit (code last = `37994b9d`). Work tree clean. **Not pushed** — origin `dd0499a7`, local 63 commits ahead after this commit.
This session: `8792a013` 3-1 U5 (protocol → command edge removed + `protocol gate 1`) → 3-1 complete · `85f72023` 3-2 U2 A (shell → transport dep, `ShellWire`, shell gates 4/5) · `8463b236` U2 B (`LocalEndpointSource`, `LocalDaemonDialer` per-registration credential slot, `ShellHandshake`) · `37994b9d` U2 C (`RouteState`/`route`/`adopt`/`route_end`, pump with `Ended`, error texts, `emit_broadcast` moved, flaky capture-test fix) · ADR-0342 (`b50ba383`, version mismatch on the existing connection-failure surface, no popup) · ADR-0343 + research (`a2821a31`).
**Next first action = 3-2 U3** (TRD §6-2 U3 row: new driver `connection_v2` behind a `DaemonClient` constructor choice, test constructor first; one actor serializes Subscribe/Unsubscribe/RequestReplay; same-key re-add path; move A1 `tests.rs:6069` / A2 `:3954`; new driver tests on `ManualClock` + fake server; production stays on the old driver). Read first: `.claude/handoff/attachments/20261105-s21-3-2-u2-handover-for-u3.md` (pinned interfaces, contracts U3 must honour — esp. the three Addenda) and the TRD §6-2 U2 C note "Contracts for the driver (U3)". Record decision 1 above before U3 coding.
Then U4 (production switch, GUI `/qa full`) → U5 (stop client on transport + version-mismatch button) → U6 (N daemons — **stop and give the user the network/structure lecture first**, standing condition) → 3-3.
Progress: stage 3 ≈15 of 23 units (3-0 all · 3-1 all · 3-2 U2).

## Verification state
- Each commit ran review + full local regression on its exact tree:
  - `8792a013`: review full (Claude + codex) R2 PASS · QA PASS — 61 result lines · 5299 passed · 0 failed · gates 32/32 · frontend 2802 + tsc.
  - `85f72023`: review deep R2 PASS · QA PASS — 5355 passed · gates 34/34 · mutations caught.
  - `8463b236`: review deep R2 PASS · QA = FAIL(known failure only) — 5381 passed · `stop_smoke send_stop_makes_real_daemon_self_exit` T-52 (shell real-daemon lane was on) · mutations caught.
  - `37994b9d`: review deep R2 PASS · QA PASS — 61 lines · 5404 passed · 0 failed · 35 ignored · lib_unit 1408 · gates 34/34 · 2802 + tsc · handshake test 20/20 · no hang.
- Not verified: one unexplained hang of a full `lib_unit` run seen once in ~127 stress runs during C's coding (test unknown; not reproduced in 120 later runs nor in QA). Watch for it.
- The new link code has no production caller yet → no GUI check so far (U4 does `/qa full`).
- Rerun = `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-detached.ps1 -Command "cargo test --workspace -- --test-threads=4" -WorkDir "I:/Engram/apps/engram-dashboard" -LogFile "<path>.log"`; link only = `cargo test -p engram-dashboard --test lib_unit daemon_client::link::`.

## Decisions (this session)
- 3-2 U2 split into A → B ∥ C (file overlap); B/C run serially.
- ADR-0342 (user, 2026-11-04): version mismatch shown on the existing UI connection-failure surface, fixed local text by close code, never the peer's text, no popup. User: 「그냥 UI 화면에 띄우면 안됨?」.
- ADR-0343 (user delegated 「리서치해서 결정해」, research medium single-family — cross-family review skipped by user agreement): user-pressed action, graceful stop first. **Partly overridden by the two unrecorded decisions in "Know first".**
- Version-mismatch UI text until U5 = cause only: 「실행 중인 데몬이 이 앱과 버전이 다릅니다」 (`link::route::refusal_message`, the single mapping; 4001 is shared with 3-3 U7's identity refusal — U7 must separate it).
- Credential slot per registration (`engram-local:<key>#<n>`), not per key (codex must — overlapping supervisors could pair tokens with the wrong address).
- Every `Disconnected` (incl. Explicit) shows Reconnecting; route is the only state emitter.
- Logs: new code writes English log messages (CLAUDE.md 「언어」). **Conflict unresolved:** `docs/reference/logging-conventions.md:32` says log messages are Korean — raise with the user.
- Still open with the user: what to show after `GaveUp(BudgetExhausted)` (ADR-0180 decision 2/7 "다시 연결" popup).
- Standing user conditions: 「qa 에서 알려진 실패(T-52 stop_smoke · ws_e2e ignored_daemon_kill_cleans_pty_child)만 남으면 코더에게 돌리지 말고 진행」; 「쭉 진행 · 이슈 있으면 말해줘」.

## Do-not / lessons
- User-facing questions: start from what the user sees, explain why it's asked now, then the options — in plain text. The user interrupted AskUserQuestion three times and asked 「뭔소리야」 / 「뭐 혼자 얘기해?」 (filed to global-feedback).
- Don't run medium research for a one-line UI decision without telling the user the scale first (「문구 하나 조사하는데 거창한거 아님?」 — filed to engram-research feedback).
- Coder briefs: new text in `docs/` is English even inside Korean docs — say so in every brief.
- Do not answer subagent interim notifications with user-facing lines (from last handoff).
- `py` heredoc with Korean on stdin breaks (cp949) — write the script to a file or use the Edit tool.

## Open notes (carried + new)
- U3/U4 carry list is in the attachment's Addenda: unregister on every `Ended`; re-add = `register` + `adopt` + `spawn_pump` in one driver step; `close()` must not emit Down itself; optional defensive `link_down()` on `Closed`; U4 logs the masked `Unreachable` text in `ConnectFailed`; U4 rewords `wire.rs` `SERIALIZE_FAILED_PREFIX` reference and refreshes "production still runs the old code" statements (CLAUDE.md:50, architecture.md:51, link/mod.rs).
- Follow-up flake: three other hand-rolled tracing capture subscribers (`usage_interest.rs:1225`, `settings/tests.rs:1220`, `tab_drop/mod.rs:885`) carry the `has_just_one` flake fixed in `link/handshake.rs` `capture()` — move `capture()` to a shared test helper.
- Not done this session: `docs/process/step-log.md` S21 entries for 3-1 U5 and 3-2 U2 A/B/C; ADR-0272 landing notes (OwnerToken closed by 3-1 U3, invariant landed by 3-1 U5); TRD §9 「메모 §10 3. 착지 표시」.
- Carried: 3-3 U4 slot notes; `Close.reason` masking = TRD §9; `MemTx::ping` stall; `mask.rs` not in gate ③; tools (`py`, rg path); worktree `.claude/worktrees/stage3-protocol` idle; leftover temp dirs `I:\Engram\apps\rv-u1a-target`, `%TEMP%\engram-review\31u2\{tgt,blocks}`.

## Improvement feedback filed this session (Engram repo `d5755e7`, 미반영)
- `core/claude-global-shared/references/global-feedback.md` — user-facing question framing / AskUserQuestion rejections.
- `core/claude-global-shared/skills/engram-research/feedback.md` — research scale for small design decisions.

## Resume command
`git -C I:/Engram/apps/engram-dashboard log --oneline -8 && git -C I:/Engram/apps/engram-dashboard status -sb`

## References
- TRD: `docs/process/S21-crate-boundaries/trd-3-transport.md` (§6-2 U2 A/B/C landing notes incl. "Contracts for the driver (U3)", §6-2 U3–U6 rows).
- U3 handover: `.claude/handoff/attachments/20261105-s21-3-2-u2-handover-for-u3.md`.
- ADRs: 0310 (stage 3 decisions), 0342, 0343; research `docs/research/daemon-version-mismatch-restart-2026-11-05.md`.
- Code: `src-tauri/src/daemon_client/link/{wire,endpoint,dialer,handshake,route,pump,errors}.rs`; old driver `src-tauri/src/daemon_client/connection.rs`.
