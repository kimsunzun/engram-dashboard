You are a subagent — this brief is the source of truth. You act as the coder for stage 3-2 unit U3b of the Engram Dashboard repo `I:\Engram\apps\engram-dashboard`, branch `v0.4.1/refactor/transport-attach`. Base = the HEAD you find at start (U3a landed as `a835aeb3`; later commits are the 15 s auth-timeout change and docs). Check `git status` is clean before you start; if not, stop and report.

## Goal — U3b only (U3c is a later, separate unit)
On the new driver `connection_v2` (test-only, `#[cfg(test)]`): everything that shares `subs` + `ReplayFlightSet`:
- Subscribe / Unsubscribe / RequestReplay as new `DriverCmd`s received by ONE actor in order and written to ONE Peer queue with `notify` from the actor (never `request`; never await on the actor); the router check happens when the actor takes the command (A1 invariant — TRD U3 row, master re-check A1).
- Replay events (SubscribeAck · ReplayComplete · SubscribeFailed) → the old `apply_replay_event` logic; follow-up Subscribe with the "send failure ⇒ drop the connection" rule.
- Output relay: `Routed::Output` → `frame_relay::relay_binary_frame` + `output_channel::send_to_windows`; `FrameRelay::Disconnect` ⇒ drop the connection. Decide and document what "drop the connection" is in V2 (`peer.reconnect_now()` vs close/re-add).
- Replay deadline sweep on the injected clock (not `tokio::time::interval`), `REPLAY_DEADLINE` 10 s / tick 1 s.
- Per-connection state cleared on `Routed::LinkDown`.
- Same-key re-add: driver-level test with the old supervisor tail overlapping (its lifecycle events and a stale `AgentListUpdated` after the new snapshot leave the new flight, frontend list and name book intact).
- Move A1 `a_replay_request_after_the_close_request_sends_no_subscribe` and A2 `agent_names_sink_receives_each_daemon_list_after_the_frontend_event` (in `src-tauri/src/daemon_client/tests.rs` — find by name) onto the new driver per the TRD fate table; the old suite stays on the old driver (do not parametrize).
- U3a review items deferred to U3b (decided by main): replaced waiters currently get `TaskGone` — answer every waiter with the newest handle's first answer if that is clean (else report why not); re-add of a once-connected key shows its first failure as Down — decide with a test which state the key should show (keep "Reconnecting" for a key that has connected before, unless the code shows a reason not to; report your choice); add a test for the `retryable_refusal` current-token guard (an old handle's tail must not stop the new peer); remove the triplicated token filter in `link/route.rs` (`route()`, `RouteState::current`, `is_open`) into one helper.

## Read first (must)
- `.claude/handoff/attachments/20261105-s21-3-2-u3a-handover-for-u3b.md` — the whole memo (what U3a built, the exact old-driver pieces with file:line, contracts and traps, deferred findings, seam). It is the main map; it is a claim — confirm with code.
- `C:\Users\kimsunzun\.claude\skills\engram-code-conventions\references\conventions.md`.
- `CLAUDE.md` (design principles, modules, language rules).
- TRD `docs/process/S21-crate-boundaries/trd-3-transport.md` — extract with `sed -n` only what you need: line 24 (revision 9, incl. ⑧ ⑨), the §6-2 U3 row (`grep -n '^| \*\*U3\*\* | 새 연결'`), §3-3 (three filter rules), the "셸 시험 운명" fate table (grep for it).
- ADRs `docs/decisions/0346-*.md`, `0347-*.md` (decisions), `0345-*.md`.

## Rules
- Production keeps the old driver; nothing in production may call the new code. Old driver behaviour and its tests unchanged (except moving A1/A2 as the fate table says).
- `src-tauri/src/daemon_client/link/` stays free of any `tauri`/`tauri_*`/`engram_dashboard_agent` path, alias or `use` (shell gate 4); transport `test-support` stays out of the shell release graph (gate 5).
- Comments and new log messages in English; structured fields; never log tokens. Name `AUTH_TIMEOUT` etc. without restating numbers in comments.
- Before coding, count the call sites / items the memo names; if they differ, stop and report first.
- Tests on `ManualClock` + memory network; hold endpoints in tests; advance the clock only while the peer is in `Backoff` (memo §4).
- Long runs via `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-detached.ps1 -Command "<cargo ...>" -WorkDir "I:/Engram/apps/engram-dashboard" -LogFile "<scratch>/<name>.log"`; scratch = `C:\Users\kimsunzun\AppData\Local\Temp\claude\I--Engram-apps-engram-dashboard\61738848-291c-4ae5-bac3-87b13b651f21\scratchpad\u3b\`. Restore byte-identical `bindings/*.ts` files the test run marks modified.
- Forbidden: spawning subagents · committing · editing ADRs / TRD / step-log / handoff files.

## Report
1. Change summary + file list. 2. Tests: commands, counts, full shell `lib_unit` name diff vs your base (lost 0 · added N, or explain each lost). 3. Self-check (surfaces re-read; doubts raised as they are). 4. Differs from memo / TRD (with reasons) — including your choices on the four deferred items. 5. Most expensive step.
