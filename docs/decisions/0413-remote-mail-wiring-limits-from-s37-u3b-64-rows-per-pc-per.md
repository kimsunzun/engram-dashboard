# ADR-0413: Remote mail wiring limits from S37 U3b — 64 rows per PC per send, 512 KiB mail text refused before queueing, overdue reports kept across endpoint restarts

- 상태: 확정 (2026-11-23, 근거: S37 U3b coding and review rounds 1-4 (lead + codex), 2026-11-23)
- 관련: ADR-0404 (kernel remote branch: one final verdict per mail) · ADR-0390 (survive a broken peer) · ADR-0411 (4 MiB frames) · ADR-0388 (decision 8: 120 s transport deadline) · `docs/process/S37-multi-daemon/trd.md` (U3b) · `docs/reference/structure/peer-link.md` · `docs/tracking.md` T-120 (remote outbox)

## 맥락
U3b wires the kernel's remote branch (U3a) to the peer link: outbound Mail per (send, PC), inbound queues per PC, verdicts, `ReplyOverdue`. Coding and four review rounds fixed limits that the TRD left open (a sender-side cap or split for more than 64 rows to one PC) or that ADR-0411's larger frames made necessary.

## 결정
1. **64 rows per PC per send.** A send's rows to one PC go as one `Mail` of at most 64 rows (the kernel's `MAX_REMOTE_ROWS`); rows past the first 64 fail `REMOTE_LIMIT` with a "split the send" hint. The receiver refuses a frame with more rows (`MAX_MAIL_ROWS`, pinned equal in the host tests) before queueing.
2. **Inbound mail text is bounded before queueing.** A `Mail` whose text fields sum to more than `MAX_MAIL_BYTES` (512 KiB, `mail::mail_bytes`) is refused `REMOTE_REFUSED` before it enters the PC's queue, so a queue holds at most `MAX_QUEUED` × 512 KiB of mail text whatever the frame cap. The kernel's 64 KiB body limit fits well under it (host test).
3. **Overdue reports**: kept per PC (each id once, at most 256 waiting, 4 running); cleared on `peer.remove` and on stop, re-checked for trust and stop when each job starts; **kept across an endpoint restart** (`peer.setNetwork`, `peer.regenerateKey`).
4. The kernel adapters (`RemoteDeliveryPort`, `MailInbox`) live in the daemon's `messaging_host.rs`, not in `link/peer/`, so the peer link does not depend on the messaging crate (daemon gate 1).

## 거부한 대안
- **Splitting more than 64 rows to one PC into several frames automatically** — each `Mail` gets exactly one final verdict (ADR-0404); a split send would need cross-frame verdict joining and ordering for a case a caller can split itself.
- **A body-only size check before queueing** — misses oversized names, reply ids and recipient tokens, and needs a copy of the kernel's body limit on the link side; the field sum catches all of them with one constant.
- **Clearing overdue queues on an endpoint restart** — the PC stays trusted and its sweep sends each report once and never again, so clearing would lose real notices.
- **No pre-queue size bound, relying on the kernel's body check** — with 4 MiB frames one blocked recipient lets ~63 near-4-MiB frames sit in the queue (~250 MiB per PC) until the 120 s expiry (both reviewers, round 3).

## 근거
- Review rounds 3-4 (lead + codex): the pre-queue bound was a must both reviewers raised; the transport already caps received bytes per connection (`BYTE_BUDGET` 16 MiB), so smaller frame counts were refused with evidence.
- QA 2026-11-23: workspace tests, gates and the claude lane; one pre-existing race recorded as T-128.

## 영향 / 불변식
- A new inbound PeerWire request kind must be bounded before it is queued, like `Mail` (decision 2), because the frame cap no longer bounds it (ADR-0411).
- Accepted residual: an inbound `Mail` that passed the trust check just before `peer.remove` may still be committed by the kernel; verdicts, in contrast, are serialized with revocation by the verdict gate.
- Rows past 64 to one PC fail rather than wait; the remote outbox (T-120) is where delays instead of refusals belong.
