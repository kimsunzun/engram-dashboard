# ADR-0108: Request contract lifetime — an unreplied request stays open even past its deadline + cap retirement is mark-and-sweep (D gate rounds 2-7)

- 상태: 확정 (2026-07-27, basis: 6 rounds of real findings from the D adversarial review + user decision 2026-07-27) · 부분 폐기 by ADR-0114 (decision 2 the REQUEST_CAPACITY reject tier) · 부분 폐기 by ADR-0118 (decision 1 the contract close condition)
- 관련: ADR-0103(reply contract proper) · ADR-0107(mailbox capacity model — the precedent pattern for pressure reclaim) · spec §3(reply contract)·§6(REQUEST_CAPACITY) · Amended by ADR-0114 (decision 2 the REQUEST_CAPACITY reject tier) · Amended by ADR-0118 (decision 1 the contract close condition)

## 맥락

D's `messages` (open-items query) tool exposed, one after another, hidden defects in request contract tracking:

1. **Obligation evaporation** — a request pushed out of the history ring (4096) was purged after its deadline-exceeded notice and vanished from the open-items query. The cause was treating "we notified" ≠ "a reply was received" as the same closure.
2. Correcting that to "stay open until replied" produced **permanent cap exhaustion** — a request sent to a dead or unresponsive recipient never closes, so it fills the global limit `MAX_OPEN_REQUESTS=512`, and with no relief path, no agent could open a new request until the daemon restarted (round-2 BLOCK).
3. The first retirement implementation (two steps, "remove, then restore on failure") exposed a false ledger state (victim absent, provisional present) during the out-of-lock delivery, and was breached three rounds in a row (rounds 3-5): a reject erased an innocent contract → a panic leak, a 513 window from a lock split, a restore-conflict guard that dropped the original → a provisional contract became another's retirement target, a normal reply addressed to the victim was lost, 513 became permanent.

## 결정

1. **Deadline exceeded ≠ closed.** Exceeding `reply_by` is only a reason to notify the sender (notice); the contract stays open until a reply (`in_reply_to`) arrives and keeps showing in the open-items query with `timed_out=true`. Purge applies only to contracts closed by a reply.
2. **Cap retirement = oldest first when full (user decision 2026-07-27).** On reaching 512, retire the **oldest of the retirable contracts** and accept the new one. Retirable = only those with "no promise left to tell the sender" — ① a contract whose deadline-exceeded notice has already gone out ② a contract with no deadline at all (no notice promise). A contract whose notice duty has not yet fired is never retirable — if all of them are in that state, a `REQUEST_CAPACITY` reject is the right answer. Retirement does not notify the sender (only a daemon info log — a follow-up review item).
3. **Retirement mechanism = mark-and-sweep.** While the delivery (out of the lock, unbounded in time) is in progress, nothing is physically removed from the ledger: the existing victim is marked `pending_retirement`, and the new contract is admitted as `provisional`. Only on a successful commit is the victim actually removed (+ a retirement log); on failure/unwind, the marks are cleared and the provisional one is removed (RAII guard, poison-tolerant).
4. **Core invariant: the lifetime of an unsettled item is owned by its guard.** A provisional contract is ⓐ excluded from retirement candidate selection ⓑ even if it closes (a quick reply), it keeps its slot weight until the guard settles ⓒ exempt from purge ⓓ excluded from the deadline sweep (after the commit, the next 60s sweep notifies based on the original time — only delayed, nothing lost). The capacity decision is derived from a single predicate (`occupied_slots`) only.

## 거부한 대안

- **Purge a notified contract immediately (the state before the correction)** — the misjudgment "notice = closure". Combined with history eviction, an unreplied obligation silently evaporates (found for real).
- **Keep forever (no relief)** — requests to dead recipients occupy 512 permanently and global sending becomes impossible (round-2 BLOCK, found for real).
- **Auto-close 24h after the deadline (alone or together)** — not adopted by the user (2026-07-27): retirement alone is adopted. It adds no benefit as a rule against the vocabulary confusion where a late reply after closure becomes a contract-less ordinary reply.
- **Two-step "remove then restore" retirement** — discarded after being found for real three rounds in a row. Each patch that narrowed the window (RAII, single lock, id reservation) was breached at another corner — the existence of the window itself is the cause.
- **The id-reservation (reserved_ids) machinery** — unnecessary once mark-and-sweep is adopted (the victim never leaves the ledger, so normal tracking covers the conflict check). Deleted.
- **Open the contract after commit (the other direction for removing the window)** — if a reply arrives between injection and open, it is a reply with no contract (a phantom-contract race — found for real in C3), so opening first is mandatory.

## 근거

- Real findings of the review: round 2 BLOCK (permanent exhaustion) → 3 (reject atomicity) → 4 (4 two-step edges) → 5 (3 window structure defects — the redesign turning point) → 6 (2 predicate edges) → 7 **PASS**. Each fix mutation-verified.
- Accepted residuals (priced): marked-victim ID-ABA (36⁸ redraw × reselection — damage = one retirement cancelled, rollback_open comment) · no truly parallel stress test (the invariant is enforced inside a single mutex + covered by a deterministic re-entrancy test) · senders not notified when their contracts are retired (backlog).

## 영향 / 불변식

- The firing condition of `REQUEST_CAPACITY` changes: a deadline-exceeded contract also occupies a slot, and a reject happens only when "512 contracts that cannot be retired". The 5 code anchors `// ADR-0108` = `ledger.rs` (the occupies_slot predicate · cap retirement oldest selection · purge exemption · due_timeouts skip) and `service.rs` (ReservationGuard).
- If violated: unreplied obligations evaporate (violating 1) / global sending becomes impossible (violating 2) / a reject or panic erases an innocent contract, 513 drift, reply loss (violating 3 and 4) come back.
- Spec alignment: the §3 reply contract (stay-open and retirement policy) and the §6 REQUEST_CAPACITY entries are updated by this ADR.
