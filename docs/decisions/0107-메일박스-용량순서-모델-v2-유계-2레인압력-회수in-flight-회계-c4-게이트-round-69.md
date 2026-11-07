# ADR-0107: Mailbox capacity and order model v2 — bounded two lanes, pressure reclaim, in-flight accounting (C4 gate rounds 6-9)

- 상태: 확정 (2026-07-26, basis: 4 rounds of real findings from the C4 adversarial review + user decision 2026-07-26) · 부분 폐기 by ADR-0114 (decision 2 pressure reclaim and decision 1 NOTICE_CAP 20) · 부분 폐기 by ADR-0111 (decision 6 broadcast-bound parking)
- 관련: Amends ADR-0103 (the notice cap exception path and the clause that parking is for a single recipient only) · ADR-0104(batch flush) · ADR-0105(TTL 24h) · Amended by ADR-0114 (decision 2 pressure reclaim and decision 1 NOTICE_CAP 20) · Amended by ADR-0111 (decision 6 broadcast-bound parking)

## 맥락

Rounds 6-9 of the adversarial review at the C4 (group fan-out) gate found, one after another, defects in the mailbox capacity and order model. **A round label = the round that found the defect as a verdict** (each fix is re-verified by the round after it — the same convention as the round-N labels in code comments):

1. **The notice cap exception (ADR-0103) is an unbounded path, plus leftover starvation (found in round 6)** — being an exception path, notices could pile up without limit, and broadcast leftovers (items bound to a dead incarnation) also took up the quota, so a respawned agent really could starve for 24h (a consequence of the round-5 design of a two-tier quota plus exemption).
2. **Cap-bypass growth (found in round 6)** — in the drain↔inject window of a flush, a concurrent send was accepted into an empty queue, and the restore after an injection failure (`restore_ordered`) did not count against the cap, so the queue grew every cycle. The "temporary overshoot ≤ one batch" argument was circular, since the old queue itself could already be over the cap.
3. **Wrongly reclaiming a live mail (found in round 6)** — the stale decision `bound != current` took the bound mail of a same-name live agent for leftovers, so pressure reclaim retired a live mail as `skipped`.
4. **FIFO join blind spot (found in round 7)** — the join decision looked only at the queue, so a direct send overtook a batch that was mid-injection (in-flight) (a defect that existed since C2 — it became observable and fixable only once in-flight accounting existed. Bringing it into scope = user decision 2026-07-26).
5. **Deferral marker lost wakeup (found in round 8)** — the flush-overlap deferral marker was a single slot per name (last-writer-wins), so the deferral of a dead id overwrote the wake of a live id. Round 9 = PASS (closed).

## 결정

1. **Capacity = two independent lanes by kind, no exceptions.** The message lane `MAILBOX_CAP=100` (counts everything, whether or not it is bound) · the notice lane `NOTICE_CAP=20` (fully independent). This bounded lane replaces the ADR-0103 clause "notice is a cap exception path" (the reply contract's "a notice is not rejected" is preserved — see 3 below).
2. **Pressure reclaim (when the quota is reached).** Message lane = retire **only stale bound leftovers**, oldest first (ledger `skipped`; `expired` for those past the TTL); if nothing can be reclaimed, reject with `MAILBOX_FULL` (zero side effects). The stale decision = **not a member of the list of live incarnations with the same name** (not a single-current comparison — this protects live mail); if the live list is unknown, reclaim nothing.
3. **The notice lane has no reject branch** — on overflow, retire the oldest notice and then accept the new one (no silent loss of a notice = the ADR-0103 reply-contract invariant is preserved; only the means is replaced).
4. **Structurally bounded by in-flight accounting.** Count the batch that a flush has taken out of the lock and is injecting, per recipient and per lane, and make the park denominator = queue + in-flight → the equation "stored + being injected ≤ cap". Settlement happens per item at the two real exits, delivery and restore, with an RAII backstop.
5. **FIFO join = queue + in-flight.** The direct-send and group-immediate-delivery gates also count in-flight as "standing ahead" (over-blocking = only a delay, under-blocking = an order violation — an asymmetric cost, so the safe side). Overlapping flushes for the same recipient are deferred (marker = an **id set** per name, retap all — a dead id is a harmless no-op) + a re-tap doorbell at settlement.
6. **A member who is busy in a group broadcast (or has an earlier parked item — FIFO join) is parked — but the individual `(id,epoch)` binding at send time preserves "no retroactive broadcast".** This binding replaces the ADR-0103 clause "parking is for a single recipient only" (no parking of broadcasts) — the invariant (no delivery to anything spawned after the snapshot) is kept, and only the enforcement means is swapped from "do not park" to "bound parking" (this removes the loss where a busy member misses a broadcast entirely).

## 거부한 대안

- **Two-tier quota + exemption (the design up to round 5)** — the exemption axis reset the denominator at every incarnation swap, so the queue grew without bound, and undeliverable bound items took up the quota, starving a respawn. It produced only defects for its complexity, and was discarded by user decision (2026-07-26, "정원 단순화" (simplify the quota)).
- **A priority (importance) field** — unclear criteria + starvation and TTL cross bugs (an AMQP practitioner warning) + no LLM framework adopts it. Reconfirms the ADR-0103 rejected alternative — separating only what differs in nature into a lane (notice) is our way.
- **Reusing visible_to (the visibility filter) in the cap denominator** — coupling the order axis and the capacity axis was the cause of the unbounded growth (맥락 2). Separate order (who is ahead) from capacity (how many).
- **Registering in-flight at drain time** — counting even the share that is restored inside the lock inflates the denominator through an unobservable window, and rejects a normal send unfairly. Register only the batch that really leaves the lock (so the equation holds).
- **Reinterpreting the deferral marker by name (re-query the roster at settlement)** — an id named by a hint queue that the name cannot resolve (rename, several same-name) would be lost, + it adds a port call to the settlement path. A set that preserves the requested id as it is is smaller and more exact.
- **Adding a reject branch for notice (removing the +1 temporary overshoot)** — `deliver_notice` discards the park result, so a reject = a silent loss of a notice (half of the reply contract). We accept the +1 temporary overshoot (bounded, self-correcting).

## 근거

- Real findings of the review: round 6 FIX 3 items (unbounded growth, wrongly reclaiming live mail, TTL vocabulary) → round 7 FIX 1 item (FIFO blind spot) → round 8 FIX 1 item (marker lost wakeup) → round 9 **PASS**. Each fix is mutation-verified (reverting the fix fails only the matching regression test).
- Gate: core 285 + daemon 597 pass · fmt clean · blind (cross-family) closing verdict PASS.
- Accepted user-visible changes: ① during the flush injection window, a new send can get `MAILBOX_FULL` even when the queue looks empty (a visible reject > a silent loss — the spec §5 stance) ② when a same-name live twin exists, a full mailbox rejects the new send instead of consuming the other agent's queued mail ③ a send in the in-flight window gets `Parked/Pending` instead of `Delivered` (delivered in order by the flush right after).

## 영향 / 불변식

- **Structurally bounded equation:** per recipient, message lane (stored + being injected) ≤ 100 · notice lane ≤ 20 (+1 temporary overshoot allowed, self-correcting). The 7 code anchors `// ADR-0107` = `mailbox.rs` (the two quota constants `MAILBOX_CAP`/`NOTICE_CAP` · pressure reclaim `park` · in-flight accounting `take_in_flight` · FIFO join `has_pending_ahead`) and `service.rs` (the deferral marker field `deferred_flush` · the reclaim ledger endpoint `park_into`).
- **Reclaim does not eat live mail** — stale = only non-members of the live list. The basis for liveness = the reachable roster (bound items of a live non-structured agent can be reclaimed — an accepted residual).
- **No silent loss of an admitted (`pending`) message** — every removal leaves a ledger endpoint (`skipped`/`expired`). **But the ledger transition is best-effort** — if the record has already been pushed out of the ring (4096), it is degraded to a debug log instead of an endpoint (the fact of removal itself stays in the log). Re-parking (`restore_ordered`) bypasses the cap (it is inflow control, not storage control), but in-flight accounting bounds the total.
- **The order the recipient sees = global oldest first** — the join decision looks at queue + in-flight, and deferral/re-tap serializes overlapping flushes. A violation brings back direct-send cutting in and age inversion between batches.
- Spec alignment: `messaging-v1-spec.md` §1 (notice self-identification) · §3 item 4 (the exception-path wording) · §4 (per-member fan-out outcome and binding) · §5 (branch 3 · policy constants · ledger) are updated by this ADR.
