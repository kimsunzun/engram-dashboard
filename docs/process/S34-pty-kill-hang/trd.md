# S34 TRD — killing a PTY agent mid-burst wedges the daemon connection

> Status: revision 1 (2026-11-07). Canonical ADR: ADR-0206 (the per-connection lanes; its "Kill is bounded by 5 s" premise is corrected here in code comments — an ADR amendment, if wanted, goes through `/engram-adr`). Written in an autonomous session (user: 「쭉 진행해」 while away) — the PRD has no open user decision.

## 1. Scope and relation to the PRD

Two changes, both reported as main's implementation choices: (A) the agent crate's PTY pump keeps draining after shutdown until EOF; (B) the daemon runs `Kill` off the async worker. Plus comment corrections (R4). PRD R1–R4, AC1–AC4.

## 2. Facts the design rests on (claims — the coder confirms with code)

- PTY pump: `crates/engram-dashboard-agent/src/transport/pty.rs` ~296-309 — after each `read`, `if shutdown.load(..) { break; }`, then `emit(TerminalBytes)`. So once `shutdown()` sets the flag, the pump stops reading after its next read.
- `PtyTransport::shutdown` (~395-415): set flag → close input queue → `child.kill()` + `wait()` → `group.terminate(1)` → `master.take()` (drop → `ClosePseudoConsole` → reader EOF). It has no bound; only the pump join in `output_core.rs` (~827-836) is bounded (5 s).
- `ClosePseudoConsole` blocks while conhost holds output nobody reads (Microsoft docs: the caller must keep draining the output pipe until it closes). Probe on this machine (portable-pty 0.8.1, Windows 26200): kill 300 ms into a burst with the reader stopped → blocked > 20 s (3/3); reader draining → 12–17 ms.
- Daemon: `crates/engram-dashboard-daemon/src/connection_core.rs` `AgentCommand::Kill` arm (~1393-1399) calls `manager.kill_agent(agent_id)` synchronously on the async task (lane = `InboundLane::Bypass`, ~207/272). `Spawn` (~1366) and `Shutdown` (~1543) already use `tokio::task::spawn_blocking`.
- Starvation: `enter_exiting` broadcasts `StatusChanged` synchronously before the block, waking the connection's `write_task` into the blocking worker's LIFO slot (tokio 1.52.3 — not stealable); probe confirmed 10/10.
- Comments claiming a 5 s bound for the whole kill: `connection_core.rs` ~186-191 (and any copy — `rg -n "5초|5 s" crates/engram-dashboard-daemon/src/connection_core.rs crates/engram-dashboard-agent/src`).

## 3. Design

### A. Drain until EOF after shutdown (agent crate — the root fix)
- In the PTY pump loop: when the shutdown flag is set, stop emitting but keep reading and discarding until `read` returns 0 / error (EOF arrives once `ClosePseudoConsole` completes). No new thread, no new timeout.
- The pump's exit classification (Killed vs Exited) must stay as today (it reads the flag at loop end).
- Platform-neutral: the loop is shared code; no `cfg(windows)` in callers (ADR-0266). The stdio transport (`stdio.rs` ~238) — check whether it has the same "stop reading on shutdown" shape; change it only if the same hazard exists there (report either way).

### B. `Kill` off the async worker (daemon)
- Run `manager.kill_agent(agent_id)` via `tokio::task::spawn_blocking`, awaiting it in the same arm, mirroring the `Spawn` arm's pattern (clone the manager handle; map a join error to an `Err` reply). The reply still goes out after the kill finished; command order on the connection stays as today (the arm still awaits before the next command on its lane).
- No new timeout value (no convention-unchecked constants); with A, a mid-burst kill returns in milliseconds plus the existing pump-join bound.

### C. Comments
- Correct the "Kill is bounded by 5 s" text to what is true after A/B (the pump join is bounded; `ClosePseudoConsole` returns once the pipe is drained; Kill runs on the blocking pool). English rewrite of each touched paragraph (CLAUDE.md language rule).

## 4. Test plan

- AC1 (agent crate): a test that spawns a PTY agent printing a large burst (a small script/fixture — reuse an existing test fixture pattern in the agent crate if one prints output), kills it ~300 ms later, and asserts `kill_agent`/`session.kill` returns within a bound (e.g. 10 s, test-side only). Run it on a thread with a timeout so a regression fails instead of hanging. It must fail on the base code (confirm locally: revert A in a scratch copy).
- AC2 (daemon): a test that the `Kill` arm runs `kill_agent` through the blocking pool — e.g. a fake manager/session whose kill blocks on a barrier while the test checks that the same connection's other async work (or a canary task on a current-thread / single-worker runtime) still progresses. If the daemon's test seams make this impractical, state why and cover it by code reading in review + the live QA repro.
- AC3 (QA live): U4a repro — spew fixture agent, kill ~0.3 s after spawn; the kill reply arrives; later commands work; no freeze.
- AC4: full regression (qa binding: Rust source → full regression; agent lifetime / kill path row → GUI live run).

## 5. Units

One unit (A + B + C share no file but are small; one coder). Order: A first (root), then B, then C.

## 6. Risks

- EOF never arrives (a child holding the pipe) → the pump drains forever; the pump join bound (5 s) still lets `kill_agent` return; the pump thread ends when the pipe finally closes. Confirm the join bound path is unchanged.
- Moving Kill to the blocking pool: the blocking pool size (tokio default 512) is ample; the manager handle must be `Send + 'static` (it is for `Spawn`).

## 7. Review record

- (pending) PRD+TRD review.
