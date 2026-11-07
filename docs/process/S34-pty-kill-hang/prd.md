# S34 PRD — killing a PTY agent mid-burst wedges the daemon connection

## 1. What · why

- Repro (S21 U4a GUI QA, 2026-11-07): a terminal agent prints a large burst; it (or another agent) is killed ~0.3 s after spawn while output is still flowing. The kill's reply never arrives; every later command on that shell's connection times out (30 s); output freezes in every view; the shell keeps showing "connected" for minutes. Reproduced on the old shell driver too (`0d4f79d1`) — the cause is in the daemon/agent, not the shell.
- Root cause (investigation, probes run on this machine — `.claude/handoff/attachments/` copy on the S21 branch; summary in the TRD):
  1. The daemon runs `Kill` inline on the connection's async `dispatch_task` (no `spawn_blocking`).
  2. The kill path calls `PtyTransport::shutdown`: it sets a flag, the pump stops reading after its next read, then the master is dropped → `ClosePseudoConsole`.
  3. `ClosePseudoConsole` blocks while conhost still holds buffered output that nobody drains — probe: blocked > 20 s in 3/3 runs when killed mid-burst; 11–17 ms when the burst was over or the reader kept draining.
  4. The blocked async worker likely also starves the connection's `write_task` (woken into that worker's tokio LIFO slot, which other workers cannot steal — mechanism confirmed by a probe, its occurrence in the QA run likely) — no output, no daemon ping/idle check; the daemon's read side still answers pings, so the shell's silence detection likely never fires.
- Users hit this today (it is not tied to the transport work).

## 2. Scope / non-goals

- Scope: the agent crate's PTY shutdown (on Windows, keep draining until the master is closed) and the daemon's `Kill` dispatch (off the async worker). Correct the wrong "bounded by 5 s" claims in code comments.
- Non-goals: a per-connection progress watchdog in the daemon net layer (the net crate is replaced in S21 stage 3-3 — that work owns it); a shell-side "N reply timeouts → drop the link" heuristic; auditing every other synchronous call on the dispatch task (listed as follow-up only).

## 3. Requirements

- R1 — Killing a PTY agent while it is still producing output completes (the kill returns) instead of blocking indefinitely.
- R2 — A kill never blocks the daemon's async worker that runs the connection's dispatch; other tasks on that runtime (the same connection's writer included) keep running while a kill is in progress.
- R3 — Output that arrives after shutdown began is not emitted to viewers (it is read and discarded), and a normal (non-burst) kill behaves as before.
- R4 — Code comments no longer claim a 5 s bound where none exists.

## 4. Acceptance criteria

- AC1 — Agent-level test: spawn a PTY agent whose burst comes from a grandchild, kill it ~300 ms into the burst; the kill returns well under the existing 5 s pump-join ceiling and the agent ends `Killed` (fails today — blocks). Exact bounds in the TRD §4.
- AC2 — Daemon-level test: a `Kill` dispatched on a connection does not block the async worker running that connection (it goes through the blocking pool like activation and `StopDaemon`'s shutdown), so a slow kill does not stop that connection's writer.
- AC3 — Live (QA): the U4a repro (spew agent, kill ~0.3 s after spawn) no longer freezes the connection — the kill reply arrives and later commands work.
- AC4 — Full regression green; existing kill/shutdown tests unchanged in meaning.

## 5. Open decisions

- None for the user: no user-visible behaviour changes except the bug going away. Implementation choices (drain-until-EOF, `spawn_blocking` mirroring `Spawn`) are main's, reported.

## 6. Risks

- EOF never arrives after shutdown (a child that keeps the pipe open) → the drain must stay bounded (it ends with the master close on Windows; non-Windows keeps today's rule) so the pump still finishes and the agent is reaped.
- Moving `Kill` to the blocking pool changes ordering relative to other commands on the same connection — must keep "the reply goes out after the kill finished" and the existing per-connection command order.
