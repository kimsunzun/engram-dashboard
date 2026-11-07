# U3c — review round 2 fixes (main → coder) — PRESCRIPTIVE

Re-review of tree `70f29cfafaa9596cb52f354baaaed7e3a3805e74`: FIX. F4–F9, T1–T7 (except below) are confirmed fixed — keep them. All new findings come from the F2/F3 mechanism (`queued_fires` counter + publish hold-back + `DriverCmd::Fire`). Instead of patching it case by case, main prescribes a replacement. If you can show this design breaks something concrete, stop and report before building an alternative.

## Findings being closed
1. (must) Publish stays held back after a stale fire is dropped: `fire()`'s drop/held branches never re-check `publish_opened` → g stays unpublished, every round trip waits the full timeout. Also cap-undo briefly inflates the count.
2. (must/권장) Caller decision vs actor publish are not atomic: caller reads `opened == None`, actor sees 0 queued and publishes, caller increments and sends `Fire` to the actor, same caller's `send_command` then writes directly ahead of its own fire. Also: while published, a fire taking the actor path (another caller's fire in flight) can be overtaken by the same caller's round trip.
3. (must) The F1 test does not prove F1: the waiter fails at the not-live check before g+1 exists; it passes with F1 reverted.
4. (권장) The fire half of `a_command_right_after_a_close_fails_at_once_and_nothing_goes_out` catches a missing F4 check only ~half the time (supervisor `select!` races `Ctl::Close` and the notice).

## Prescribed design — one gate instead of counter + hold-back
Replace `queued_fires`, `QUEUED_FIRES_MAX`-as-counter, `unpublished`, the publish hold-back and `DriverCmd::Fire` with a shared gate on the client side:

```
struct Gate { opened: Option<Opened /* {generation, closes} */>, pending: VecDeque<(Generation, PendingFire)> }
shared: Arc<std::sync::Mutex<Gate>>  (+ keep the existing `opened` watch only as a wake-up signal for waiting round trips)
```

- **`send_fire_and_forget` (caller):** lock the gate. Let `g` = the live generation at call (as today).
  - If `gate.opened == Some{g, e}` with `e` current (same close-epoch rule as F4) → write `notify_in(g)` while holding the lock (it is a non-blocking leaf: `try_send` + notify), then unlock.
  - Else if the peer is live on `g` and the epoch is current → push `(g, cmd)` into `pending` if `pending.len() < QUEUED_FIRES_MAX` (the old channel bound), else drop with the existing log; unlock.
  - Else drop (ADR-0195: never held while disconnected).
- **Actor on `LinkUp(g)`:** do the on-open writes first (RegisterCommands via `start_now`, on-open UsageSubscribe — unchanged). Then lock the gate: drain `pending` in order — entries with generation `== g` are written with `notify_in(g)`, older are dropped, newer are kept; set `gate.opened = Some{g, add-epoch}`; unlock; then signal the watch.
- **`close` / `LinkDown` / `drop_connection`:** lock the gate, `opened = None` (`close` also clears `pending`), unlock, signal the watch.
- **`send_command` (caller):** keep F1/F4 exactly (bound generation captured once; epoch check). Read `opened` from the gate under its lock for the decision; use the watch only to wake. No need to queue the request under the lock: anything it must follow (on-open writes, flushed pending fires, the caller's own direct fires) is already on the peer queue when the publish becomes visible.
- No other lock may be taken while holding the gate (it is a leaf, like the roster lock); the usage lock is never held together with it — on `LinkUp` the usage on-open write happens before taking the gate.
- Order of fires vs Unsubscribe/RequestReplay: not ordered (U3b parity — fires were direct `peer.notify` then). A1 (Subscribe/Unsubscribe/RequestReplay on one actor, one queue) is untouched. State this in the module doc; fix the module doc's ordering sentence so it promises exactly what the gate gives: one caller's fire then round trip keep their order, at all times; no order across callers.

## Tests
- Remove/replace tests that pin the counter or hold-back as such (F2/T2 bench) with gate equivalents:
  - pending bounded at the cap (cap+1 sent while not opened → exactly cap written on `LinkUp`, in order);
  - pending entries follow their connection: `== g` written before the publish, older dropped, newer kept across an older `LinkUp`; `close` clears pending;
  - **the stale-fire case from finding 1**: a pending fire for g1 survives g1's loss, `LinkUp(g2)` → g2 is published and a round trip on g2 succeeds (no timeout);
  - **finding 2**: same caller `fire(A)` issued while not opened, then `send_command(B)` after publish → wire order A, B; and while published, `fire(A); send_command(B)` → A, B. Deterministic (Bench or hand-polled futures), not by luck.
- **Finding 3 (F1 proof):** hand-poll `send_command` once while g is live and not opened (→ Pending, inside its wait); then lose g, redial, push hello, wait until g2 is published; only then poll again → `UNSENT_ON_DISCONNECT`, and the next frame on g2 is a marker (nothing of the call). Verify it fails with the `peer.generation() != bound` check removed (say how you checked — e.g. temporarily reverting locally, then restoring). Keep the current test renamed as the "loss while waiting" case.
- **Finding 4:** make the fire half of the F4 test deterministic (e.g. check the direct fire path drops at the caller — observable via the gate/peer queue or a log — rather than racing the supervisor).
- For each new/changed test, state that it fails with its property removed.

## Report
Overwrite `R\coder-report.md` (R = scratch `u3c` folder): per finding 1–4 → done/rejected (evidence); what was removed; files; tests (commands, counts, full `lib_unit` + transport name diff vs base `1d3a8486`: lost 0 · added N — list removed tests with reasons); self-check; most expensive step. Same rules as before (run-detached, fmt, bindings restore, no commit, no subagents). Wording items for main stay untouched unless your change makes them wrong.
