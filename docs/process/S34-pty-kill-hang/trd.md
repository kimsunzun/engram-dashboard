# S34 TRD — killing a PTY agent mid-burst wedges the daemon connection

> Status: revision 3 (2026-11-07) — PRD+TRD review rounds 1–2 (full: lead + codex) applied (§8). Canonical ADRs: ADR-0001 (its premise "master drop → reader EOF → pump break, no hang" breaks while the pump has stopped reading — link/amend through `/engram-adr`), ADR-0206 (lanes — unchanged). Written in an autonomous session (user: 「쭉 진행해」 while away) — the PRD has no open user decision.

## 1. Scope and relation to the PRD

Two changes, both main's implementation choices: (A) the agent crate's PTY pump keeps draining after shutdown until the master is dropped (bounded — §3A); (B) the daemon runs `Kill` off the async worker. Plus comment and doc corrections (C). PRD R1–R4, AC1–AC4.

## 2. Facts the design rests on (claims — the coder confirms with code)

- PTY pump: `crates/engram-dashboard-agent/src/transport/pty.rs` ~296-309 — after each `read`, `if shutdown.load(..) { break; }`, then `emit(TerminalBytes)`. `finish(Killed|Exited)` runs only after the loop exits (~326). `emit` never blocks (`FrameOutputSink` uses `try_send`).
- `PtyTransport::shutdown` (~395-415): set flag → close input queue → `child.kill()` + `wait()` → `group.terminate(1)` → `master.take()` (drop → `ClosePseudoConsole`). It has no bound. `session.kill` = `shutdown()` then `join_pump(5 s)` (`session.rs` ~644-647, `output_core.rs` ~827-836); `join_pump` only times out — it does not finalize.
- `ClosePseudoConsole` blocks while conhost holds output nobody reads (Microsoft: keep draining the output pipe while closing). Probe on this machine (portable-pty 0.8.1, Windows 26200): kill 300 ms into a burst produced by a grandchild (cmd → node) with the reader stopped → blocked > 20 s (3/3); direct-child producer 1/2; reader draining → 12–17 ms.
- On Windows, EOF does follow the close: the pipe handles are non-inheritable and `PsuedoCon::new` drops the write end (filedescriptor 0.8.3, portable-pty `psuedocon.rs`). On non-Windows, `GroupRef::terminate` is a no-op, so a surviving grandchild can keep the pty open and writing.
- Daemon: a normal `Kill` is `DispatchOrder::InOrder` on the connection's `dispatch_task` (`connection_core.rs` ~272); `InboundLane::Bypass` (~207) applies only when the inbound queue is saturated (`bypass_task`). The arm (~1393-1399) calls `manager.kill_agent(agent_id)` synchronously. Activation (~1366, ~1761) and the `StopDaemon` arm's `shutdown_all` (~1543) already use `tokio::task::spawn_blocking`.
- Writer starvation (the visible freeze): `enter_exiting` broadcasts `StatusChanged` before the block, waking the connection's `write_task` into the blocking worker's tokio LIFO slot (not stealable) — mechanism confirmed by a scheduler probe (10/10); that it happened in the QA run is likely (fits the 2/5 repro rate), not captured. The shell keepalive staying satisfied by daemon Pongs is likely (not captured on the wire).
- Wrong "bounded by 5 s" claims: `connection_core.rs` ~186-191, ~218-221 (`dispatch_order` doc: in-order commands "have no await point"), ~1357, ~2764; `manager.rs` ~1702. Find all with `rg -n "5초|5 ?s\b|5s" crates/engram-dashboard-daemon/src/connection_core.rs crates/engram-dashboard-agent/src`. The research note `docs/research/child-process-shutdown-peers-2026-10-14.md` (~22) says `ClosePseudoConsole` "returns immediately from 24H2" — false on this machine.

## 3. Design

### A. Drain while closing (agent crate — the root fix)
- **Only where closing needs it.** Draining matters only because Windows' `ClosePseudoConsole` waits for its output to be read. Add one platform fact in the `platform` crate (ADR-0266 — the OS branch lives there only), e.g. `platform::pty::CLOSE_NEEDS_DRAIN: bool` (true on Windows, false elsewhere). Where it is false the pump keeps **exactly today's rule** (break on the first read after the shutdown flag) — so non-Windows behaviour, including a silent surviving grandchild, is unchanged.
- **Where it is true (Windows):** once the shutdown flag is set the pump stops emitting but keeps reading and discarding **until the master has been destroyed**. Add a second signal, `master_closed: AtomicBool`, stored (Release; the pump loads with Acquire) by `shutdown()` **after the statement that drops the master has completed** — i.e. after today's `let _ = self.master.lock()…take();`, never between binding the taken master and dropping it (portable-pty closes its own master read handle only after `ClosePseudoConsole` returns; publishing earlier lets the pump stop while the close still waits → T-79 again). The pump breaks on EOF/error, or on the first read that returns after `master_closed` is set.
  - While `ClosePseudoConsole` runs the pump drains, so the close returns; EOF (or the next read) then ends the loop.
  - If the watcher already took the master, `shutdown`'s `take()` returns `None`; correctness relies on the watcher dropping the master **while holding the master lock** (today's statement shape — the temporary drops before the guard), so `shutdown`'s `lock()` waits for that close. Pin this in a comment at the watcher's drop.
  - A second `shutdown` (e.g. `kill_agent` then `shutdown_all`): `take()` → `None`, flag already set — no change.
- The pump's Killed/Exited classification stays as today (read at loop end). The watcher's natural-exit path is unchanged (it never sets the shutdown flag).
- Comments pin the new invariant at the pump loop, at `shutdown` step 5 and in the `session.kill` doc: "after shutdown the pump keeps draining until the master is closed — otherwise `ClosePseudoConsole` can block" (ADR-0001).
- `stdio.rs` and the codex transport (`backend/codex/transport.rs` ~4129) kill, terminate the group and return without waiting on a drain — no change (the coder confirms and reports).

### B. `Kill` off the async worker (daemon)
- Run `manager.kill_agent(agent_id)` via `tokio::task::spawn_blocking`, awaiting it in the same arm, mirroring the activation arm (clone the manager handle; a join error → `Err` reply). The reply still goes out after the kill finished and after its status broadcast; the next command on `dispatch_task` (and on `bypass_task` under saturation) is taken only after the reply — command order unchanged.
- Kill stays in order (not detached): after A the kill is short, and in-order keeps the `Kill(X) → SpawnProfile(X)` restart sequence.
- The investigation's "bounded wait + `Error` reply" is **rejected for now**: it needs a new timeout value with no measured basis (the project's convention-first rule), and after A no unbounded wait remains on the reproduced path. Residual risk (recorded, not fixed): any future unbounded wait inside `shutdown()` would hold the connection's in-order line (output and pings keep flowing after B; commands time out). Known instance: a kill that lands after a contained pump panic (ADR-0292 decision 6) but before the reaper runs drops the master with no live pump.

### C. Comments and docs
- Correct every "Kill / teardown is bounded by 5 s" claim (§2 list) to what is true after A/B: the pump join is bounded (5 s ceiling); `ClosePseudoConsole` returns because the pump drains during the close; Kill runs on the blocking pool. Rewrite the `dispatch_order` doc: in-order commands may await (Kill awaits its blocking task) and hold the next command until replied. English rewrite of each touched paragraph.
- Correct the research note's `ClosePseudoConsole` row (with the probe result and this topic's link).
- ADR: link this topic to ADR-0001 (premise refined) via `/engram-adr link` by main after the code lands.

## 4. Test plan

- **AC0 (agent crate, deterministic, every platform):** extract the pump's read loop into a function over `impl Read` + the two flags + `CLOSE_NEEDS_DRAIN` (as a parameter) + an emit sink, and unit-test it in the process-free test module with a scripted reader: (a) bytes read after shutdown are not emitted; (b) with drain on, the loop keeps reading while `master_closed` is false; (c) with drain on, it breaks on the first read after `master_closed` even if the reader never sends EOF; (d) with drain off, it breaks on the first read after shutdown (today's rule); (e) EOF breaks the loop; (f) the outcome is classified `Killed` after shutdown, `Exited` without it.
- **AC1 (agent crate, real ConPTY — a dedicated `#[cfg(windows)]` test module in `pty.rs`, or share `CapturingStatusSink`):** spawn a PTY agent whose output comes from a **grandchild** (`cmd.exe` running an **unbounded** inner spew loop, e.g. `for /L` with step 0 — so output is certainly flowing at kill time); wait until output is flowing, then kill ~300 ms later. Assert (1) `kill` returns in **< 2 s** (well under the 5 s join) and (2) the status sink recorded `Killed`. Run the kill on a thread with a **separate 15 s watchdog** so a regression fails instead of hanging. Confirm it fails on the base code over **several runs** (revert A in a scratch copy) and report the rate.
- **AC2 (daemon, deterministic):** a `connection_core` unit test with `insert_test_session` + `SeamTransport` (~3820-3857) whose fake `shutdown()` signals "entered" and then waits on a std channel `recv_timeout(~2 s)`, recording whether it was released; on a current-thread runtime run `tokio::join!(core.dispatch(Kill), canary)` where the canary first waits for "entered", then releases it (so the poll order of `join!` does not matter). Today (inline) the only thread blocks → released = false → fail; with B → released = true. Keep `an_in_order_command_is_answered_before_on_text_returns` (`agent_conn.rs` ~1024) green (reply causality). Also cover Kill under the saturated (bypass) path if a seam exists; otherwise state it.
- **AC3 (QA live):** the U4a repro — spew fixture agent (grandchild producer), kill ~0.3 s after spawn; the kill reply arrives; later commands work; no freeze.
- **AC4:** full regression (qa binding: Rust source → full regression; agent lifetime / kill path → GUI live run).

## 5. Units

One unit, one coder. Order: A (with AC0/AC1) → B (with AC2) → C. AC0 scripted reader: an unexpected extra read fails at once, and cases (c)/(d) assert the read count.

## 6. Risks

- Non-Windows behaviour after A is today's rule exactly (`CLOSE_NEEDS_DRAIN = false`); a surviving grandchild there is a pre-existing platform gap (`terminate` no-op), not new.
- Windows: a read already in flight when `master_closed` is published ends when that read returns; it cannot block forever because the close has completed and dropping the pump's reader closes the last read end (conhost then sees a broken pipe).
- Moving Kill to the blocking pool: pool size (tokio default 512) is ample; the manager handle is `Send + 'static` (as for activation).
- The residual "unbounded wait inside shutdown" risk in §3B.

## 7. Follow-ups (not in this topic)

- A per-connection progress watchdog / handler-never-returns cleanup in the daemon connection layer (S21 stage 3-3 owns the net replacement).
- Audit other synchronous calls on the dispatch task (profile CRUD file IO, `SpawnByCwd`).

## 8. Review record

- 2026-11-07 — PRD+TRD review round 1 (full: lead Claude Opus xhigh + codex gpt-6.1-sol xhigh): FIX. Must (both): AC1 could pass a partial fix (10 s bound vs 5 s join; no `Killed` assertion). Must (codex) / 권장 (lead): never-EOF lifetime (drain forever → no `finish`; non-Windows grandchildren). Applied in revision 2: bounded drain (§3A), AC1 rewritten (grandchild producer, < 2 s, `Killed`, separate watchdog), AC2 specified, ADR-0001 named instead of ADR-0206, research note fix, rejected bounded wait + residual risk recorded, comment scope widened, lane/arm naming corrected.
- 2026-11-07 — PRD+TRD review round 2: lead PASS (권장: publish `master_closed` only after the master is destroyed; a deterministic loop test; an unbounded producer; join-order-proof AC2; test placement; stale PRD lines). codex FIX — must: on non-Windows the bounded drain adds one blocking read that a silent surviving grandchild never ends (regression vs today). Applied in revision 3: the drain is gated by a platform fact `CLOSE_NEEDS_DRAIN` (Windows only — non-Windows keeps today's rule exactly), `master_closed` published after the drop statement completes (and the watcher's drop-under-lock pinned), AC0 deterministic loop tests, AC1 unbounded grandchild producer + base failure over several runs, AC2 "entered" handshake, PRD synced.
- 2026-11-07 — PRD+TRD review round 3: codex PASS (round-2 must fixed; 권장 AC0 read counts and batched PRD/TRD wording applied in revision 3 as committed). **PRD+TRD review passed** (full, 3 rounds).
