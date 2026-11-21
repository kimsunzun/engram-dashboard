# ADR-0404: Remote mail rows in the kernel: no local key for remote rows, a bounded remote store, one final verdict per mail, overdue names only unanswered agents

- 상태: 확정 (2026-11-22, 근거: S37 U3a code review deep, 3 rounds + QA)
- 관련: Amends ADR-0393 (decisions 1 and 6 remote row key and overdue attribution) · ADR-0116 (decision 2, reply close rule) · ADR-0388 (decision 8, the 120 s `Mail` deadline) · TRD `docs/process/S37-multi-daemon/trd.md` §4-3 … §4-6

## 맥락
Building the messaging kernel's remote branch (`crates/engram-dashboard-messaging/src/{ledger,service}.rs`) showed that ADR-0393 decision 1 ("`to_id` = none until the verdict", i.e. the peer's recipient id becomes the row's key at the verdict) and decision 6 (overdue across PCs, attribution left open) break in concrete sequences: a remote row carrying a local-looking `(msg_id, to_id)` key captures a local agent's transitions (the local row stays pending forever); remote facts kept only on the 4096-row history ring are lost on eviction (late notices, reply closes, overdue reports); with several rows of one request to one PC, "the next unreported row" names an agent that already replied. The kernel also needed a verdict contract the daemon (U3b) can keep, and a rule for what it trusts from a peer.

## 결정
1. **Remote rows never take a local key.** A remote history row keeps `to_id` = none for good; the peer's recipient id lives in the remote store. Local verbs (`transition`, `fail_pending`, reply stamping, TTL and delete cleanup) skip remote rows. Amends ADR-0393 decision 1.
2. **Remote correlation lives in a bounded remote store, independent of the history ring.** An entry lives while it has a purpose — a verdict or the agent's answer still due (at most `REMOTE_VERDICT_HORIZON`, 10 min), or a request with a deadline until it is reported, replied, failed, or deadline + `REMOTE_OVERDUE_GRACE`. The grace is built from named bounds: the 120 s transport deadline + `REPLY_HOLD_MAX` (180 s) + the receiver's 60 s sweep + a 240 s link margin = 600 s. Cap 4096 entries, oldest first, with a warn. Reply holds on a contract are counted and lapse after `REPLY_HOLD_MAX`.
3. **Exactly one final verdict call per mail** (`RemoteDeliveryPort` contract): tags it leaves out are settled unconfirmed by the kernel; a mail with no call stays unconfirmed until its entry ages out; a second call for the same mail is a host wiring defect and is logged as a warning.
4. **Overdue reports name only unanswered agents.** On `ReplyOverdue(R)` from node N the candidates are R's rows to N that have a deadline and are not failed, replied or reported: one → that agent is named; several → the PC is named with the candidates listed; none → refused. A report at or before `created_at + reply_by` (this PC's clock) is refused. A row is marked replied only when the reply was accepted here (the replier's own closing rule, ADR-0116 decision 2), matched by the verdict's recipient id or, when none came back, by the sent token parsed as an id or name. Amends ADR-0393 decision 6 (no wire change: `ReplyOverdue` still carries only the request id).
5. **The receiving host drops an inbound mail whose 120 s transport deadline has passed** (counted from arrival on that PC — the PCs share no clock) instead of running it late; the overdue grace relies on it.
6. **The remote entrance re-checks the pure rules in the kernel** (defence in depth): mail id 1–64 bytes of `[A-Za-z0-9_-]`, row tokens ≤ 256 bytes, the contract fields (request and reply_to exclusive, reply_by only with request and ≤ 30 days, notation ≤ 16 alphanumerics, reply_to a mail id, no `to_attr`). The daemon's ingress checks stay and still run first. The sender applies the same per-row rules before sending, so one bad row never sinks the other rows to that PC.

## 거부한 대안
- **The peer's recipient id as the row's `to_id` (ADR-0393 decision 1 as written)** — shares the local key space; a local agent's later transitions hit the remote row and its own row stays pending forever (review event sequence; test `a_settled_remote_row_never_takes_a_local_rows_transition`).
- **Remote facts on the history row only** — the ring evicts after 4096 rows; a request with a 24 h deadline loses its overdue notice (tests `remote_correlation_survives_history_eviction`, `a_late_verdict_still_gives_its_notice_after_the_history_ring_rolled_over`).
- **One verdict call per row** — a row the host forgets stays pending for good with no notice; one final call per mail lets the kernel close the gaps.
- **Name the next unreported row in an overdue notice** — names an agent that replied (review event sequence; test with bob replying and dan not).
- **Carry the recipient id in `ReplyOverdue`** — needs a wire change (TRD §3-3, ADR-0390 decision 8); the kernel-only rule is enough for v1.
- **Trust `SendMeta` from a peer as ingress-validated** — a peer frame never passed this PC's ingress unless the host remembers to run it; an unbounded `reply_by` would overflow `created_at + reply_by`.

## 근거
Code review deep (lead + risk-axis lens + codex), 3 rounds, findings with event sequences; messaging tests 273 → 327, each rule pinned by a test; QA full regression with the claude lane and 2/2 mutations caught (`accept_reply_overdue` deadline compare, the remote store horizon).

## 영향 / 불변식
- `ledger.rs` (`RemoteEntry`, `record_remote`, `settle_remote`, `prune_remote`, `accept_reply_overdue`, `mark_remote_replied`, `msg_id_in_use`, `REMOTE_*` constants), `service.rs` (`RemoteDeliveryPort`, `handle_remote_verdict`, `await_remote_answer`, `handle_remote_mail`, `validate_inbound`, `validate_inbound_contract`, `handle_reply_overdue`).
- U3b must: make one final verdict call per mail; drop late inbound mail; run the daemon's ingress contract checks before `handle_remote_mail`; keep the receiver sweep period equal to `RECEIVER_SWEEP_PERIOD` (daemon `SWEEP_INTERVAL`).
- Breaking 1 strands local rows; breaking 2 loses notices on busy PCs; breaking 4 sends wrong or duplicate overdue notices.
