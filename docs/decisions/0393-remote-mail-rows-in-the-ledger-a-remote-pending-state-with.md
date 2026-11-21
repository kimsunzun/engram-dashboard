# ADR-0393: Remote mail rows in the ledger — a remote-pending state with verdict-only transitions, the reply contract closed at verdict time on the receiving daemon

- 상태: 확정 (2026-11-20, basis: user decisions D3 · X3 · X4 2026-11-19 + S37 PRD+TRD review PASS, 4 rounds, 2026-11-20) · 부분 폐기 by ADR-0404 (decisions 1 and 6 remote row key and overdue attribution)
- 관련: Amends ADR-0116 (decision 4 deletion cleanup as the only after-the-fact terminal edge, and decision 2 close timing for a remote requester) · ADR-0111 (the ledger transition graph) · ADR-0118 · ADR-0309 (agent-id keys, ids unique across servers) · ADR-0104 (notice lane at the idle gate) · ADR-0181 (decision 4: a late reply is dropped) · ADR-0388 (120 s PeerWire deadline) · ADR-0390 (closed verdict codes, `ReplyOverdue`) · `docs/process/S37-multi-daemon/trd.md` (§3-3 · §3-4 · §4-3 · §4-6 · T15 · T23 · T25 · T26 · T28) · PRD D3 · R6 · R7 · Amended by ADR-0404 (decisions 1 and 6 remote row key and overdue attribution)

## 맥락
A remote row's outcome arrives after `handle_send` returns: the receiving PC answers with a `MailVerdict` (delivered · held · refused · unknown recipient), or the link drops, or no answer comes. Today a row's state is final when it is recorded, and the only edge into `Failed` is deletion cleanup's `Pending → Failed` (`can_fail_by_cleanup`, ADR-0116 decision 4); a remote-held row cannot be local `Pending`, which the expiry sweep and the wake flush act on. Reply contracts close on the replier side today, by ADR-0116 decision 2's rule at the time the reply is recorded — but a reply going to a requester on another PC does not have its outcome until the verdict. The user decided that a send cut off by a link drop still gets system feedback (D3), that held mail gets no later receipt (X3), and that request deadlines across PCs stay simple (X4).

## 결정
1. **Sender side: a remote-pending ledger state.** A remote row is recorded in the remote-pending state (`to` = the notation, `to_id` = none until the verdict) without opening a contract. Its transitions to delivered / held / failed are applied only by the verdict entry, which picks the row by (mail id, row `tag`) frozen at send time. A remote-held row is not local `Pending`: the expiry sweep and the wake flush do not act on it.
2. **Link loss is mapped by `RequestError::Disconnected{sent}`**: `sent: false` → not delivered (`failed`, `PEER_OFFLINE`); `sent: true` → unconfirmed (`pending` with the "may or may not have arrived" hint), never `failed`.
3. **Late verdicts.** Whether a row is reported in the agent's answer or by a later notice is decided atomically under the messaging lock at the 7 s boundary, so each row is reported exactly once. A verdict that arrives after the agent was answered still updates the ledger row and gives the sender one daemon notice with the final outcome and code only. Past the 120 s transport deadline, or when the link drops after sending, the row stays unconfirmed and the notice says the outcome is unknown. Notices go through the existing notice lane at the idle gate (ADR-0104).
4. **Reply contracts live on the receiving daemon.** `handle_send` there opens the contract for its recipient and the recipient's reply closes it there; the sending daemon opens none for remote rows. The remote entrance stores the requester's node (from the link) on the request entry.
5. **A reply to a remote requester closes the local contract at verdict time, by ADR-0116 decision 2's rule**: delivered or held → `replied`; `RECIPIENT_NOT_FOUND` → `reply_failed`; any other outcome → stays open (as `reply_disposition` `NoOp`).
6. **Overdue across PCs**: the receiving daemon sends `ReplyOverdue{request_id}` to the node recorded at request time (never by resolving the requester id), best-effort (lost if the link is down), and not while a reply verdict for that contract is in flight. The requester's daemon renders its own overdue notice naming the origin PC, and accepts a `ReplyOverdue` only for its own open remote request row sent to that node, and only once. The requester's "awaiting reply" view does not list remote requests in v1.
7. **Keys stay agent UUIDs — no node in any key.** The node is a display / routing qualifier: `SenderIdentity` gains `node: Option<NodeRef>`, `NodeRef` = a messaging-local opaque fixed 32-byte value (the node key), so `SenderIdentity` stays `Copy`. The remote entrance gets a per-row kernel result (row index and resolved recipient id) for tags and alias folding. A remote mail id that collides in the receiver's ledger → `REMOTE_REFUSED`.
8. **No later receipt for held mail** (X3), as for local held mail today.

## 거부한 대안
- **Recording a remote row with its final status at send time** (today's model) — the outcome arrives after `handle_send` returns, and today's graph has no general edge into `Failed` (ADR-0111, kept by ADR-0116).
- **Reusing local `Pending` for a remote-held row** — the expiry sweep and the wake flush act on local `Pending` and would expire or flush a row parked on another PC.
- **Matching verdict rows by the notation** — the receiver sees the stripped name, and `peer.edit` can rename the PC while the request is pending.
- **Reporting `Disconnected{sent: true}` as failed** — `sent: true` means the peer may have processed it (`pending.rs`), so "failed" would be a false report.
- **One short request deadline** — transport drops a reply after its request's deadline (`peer.rs` `route`, ADR-0181 decision 4), so the later outcome notice could never fire.
- **Holding the contract on the sending daemon, or listing remote requests in the requester's awaiting-reply view** — user rejected directly (X4: 「거의 사용되지 않을것 같은데」 → simplest kept); closing is replier-side today, so keeping the contract on the receiving PC needs no extra work.
- **Routing `ReplyOverdue` by resolving the requester id** — the requester id is not trusted for routing back; the node recorded from the link is.
- **A later receipt when held mail is finally injected** — user rejected directly (X3: 「옵션에 따라 다르지 않음? 송수신 우편 옵션일때만 보내는거잖아」); a sender that needs confirmation sends a request and gets the reply or the overdue notice.
- **`NodeRef` as a `String` or a uuid alias** — a `String` drops `Copy` from `SenderIdentity` (~60 by-value uses in messaging and the daemon); the node identity is a 32-byte key, not a uuid (TRD §9 P2 · P15).

## 근거
- User decisions D3 (「다음 단위로 해야될것같은데? 이득에 비해서 얻는게 적다」, plus feedback for a send cut off by a link drop), X3 and X4 (2026-11-19); notices stay on the current lane — 「일단은 그냥 가자. 지금처럼」 (2026-11-19).
- Code facts cited by the TRD: `can_fail_by_cleanup` is the only edge into `Failed`; `pending.rs` `sent` semantics; `peer.rs` `route` drops late replies; `SenderIdentity` derives `Copy`.
- ADR-0309: ids are unique across servers, so keys need no node; mailbox and ledger are not persisted, so no key migration.
- S37 PRD+TRD review, PASS after 4 rounds (2026-11-20).

## 영향 / 불변식
- ADR-0116 decision 4's limitation ("deletion cleanup is the only path that terminates a record after the fact") gains one more after-the-fact path: verdict-only transitions out of remote-pending. Deletion cleanup's `pending → failed` and the ban on `delivered → failed` are unchanged.
- ADR-0116 decision 2's outcome rule is unchanged; only its timing moves to the verdict for a reply to a remote requester.
- Only the verdict entry moves a row out of remote-pending; no sweep or flush touches it.
- Each row is reported exactly once (answer or notice); a peer's `reason` never enters the daemon's notice voice.
- The TRD (§4-6) defers "exact states" to this ADR but fixes only what is recorded here: the remote-pending state, its verdict-time transitions to delivered / held / failed, and the sender-side mapping of §3-4 (an unconfirmed row is reported as `pending` with the unconfirmed hint). Any further state, or its name, is not decided here.
- Turning such notices into deferred inputs that ride the next input without a turn is T-115; keeping and resending mail is T-120.
