# ADR-0390: Remote peers may only send mail — pairwise trust by pasted IDs, a receiver-side per-PC backstop instead of per-pair limits, a sender in-flight cap

- 상태: 확정 (2026-11-20, basis: user decisions D1 2026-11-18 and D5 2026-11-19, revised 2026-11-20 (P-G) + S37 PRD+TRD review PASS, 4 rounds, 2026-11-20)
- 관련: ADR-0388 (the iroh link and its `accept_if` hook) · ADR-0389 (identity = node key) · ADR-0391 (resolution, `ResolveScope`) · ADR-0393 (verdicts in the ledger) · ADR-0394 (trust changes need a human) · ADR-0110 (policy in the messaging lib) · ADR-0125 (decision 1: blocking inject) · ADR-0179 (decision 2: shell → N daemons later) · ADR-0310 (D9 refusal log) · `docs/process/S37-multi-daemon/trd.md` (§3-1 · §3-3 · §4-2 · §4-4 · §4-5 · T5 · T7 · T11 · T27 · T28 · T29 · T45) · PRD R4 · R8 · R9 · D1 · D5

## 맥락
Remote mail reaches agents that run with broad permissions, so what a trusted PC may do and who counts as trusted must be narrow. S37 v1 is daemon↔daemon agent mail between the user's own PCs; the app driving a remote daemon comes later on the same link (ADR-0179 decision 2). The receiving daemon must also survive a broken peer daemon (which could invent sender names) without limiting the user's own normal traffic.

## 결정
1. **Remote peers may only send mail.** The PeerWire vocabulary has no other action: a frame naming a non-mail permission action does not exist, so R9 holds by construction — no agent start / stop, no input outside mail, no commands. The trusted entry's `permission` accepts only `"mail"` in v1; `"control"` is reserved for the shell → remote daemon stage.
2. **Pairwise trust by pasting IDs on both PCs** (D1, Syncthing device-ID exchange). Entry = `{id, name, address?, permission}`. An accept prompt, a one-time code or an invite link are later front-ends on the same `peer.add`.
3. **The gate runs before any stream.** The daemon fills the transport hook `accept_if(peer_key)` (called before `accept_bi`): not listed, or permission ≠ `mail` → the connection is closed with `HANDSHAKE_REJECTED` and "not paired"; no stream is accepted and no frame is read; the dialer reads it as `Terminal` and stops without spending budget. The refusal log is throttled like the shell link's batched `AcceptRefusal` log.
4. **Removal bites at once.** `peer.remove` revokes in memory first (the gate and the per-frame check see it at once), closes that node's handles and inboxes with `HANDSHAKE_REJECTED`, drops its directory copy, then saves the list; a refused save is reported and the revocation stays for this run. Every inbound frame is checked against the current list; a frame from a node no longer listed is dropped and its handles closed.
5. **The remote entrance validates like ingress before `handle_send`**: `validate_contract`, single-recipient reply, non-empty `to`, the envelope-format check; plus ≤ 64 rows with unique `tag`s, `@all` / `@here` refused, `from.name` within the remote name rule (non-empty, ≤ 256 bytes, no control characters — the sender and the directory publisher enforce the same rule), `from.agent_id` not the nil id. A failure refuses the whole frame with `REMOTE_REFUSED`. R8: `from.agent_id` must not be a local agent id, where local = the full manager roster; the refusal hints at a copied data folder. The sending PC comes only from the link; `from` carries no node.
6. **No forwarding**: `handle_send` takes a `ResolveScope` argument (`All` for ingress, `LocalOnly` for the remote entrance); under `LocalOnly` listed PCs and directory copies are ignored.
7. **Receive work runs off the inbox task**, ordered per (node, recipient token): ≤ 4 frames per node run at once, ≤ 64 queued per node; a frame arriving at a full queue is refused whole with `REMOTE_LIMIT` ("receiver busy").
8. **No peer-chosen notice text.** Overdue = `ReplyOverdue{request_id}` sent to the node recorded at request time and rendered by the requester's daemon from its own template, naming the origin PC; accepted only once and only for an own open request row sent to that node; not sent while a reply verdict for that contract is in flight. Verdict codes come from a closed set (anything else → `REMOTE_REFUSED`); a peer's `reason` is ≤ 512 bytes, shown labelled with the origin PC and never in the daemon's notice voice.
9. **Inbound limits = one per-PC backstop at the receiver** (D5 revised, P-G): 600 mails/min per PC, whole frame refused, plus the 64 KiB body cap (the local `MAX_BODY_BYTES` value); a hit = `refused` + `REMOTE_LIMIT` with the limit as reason. No per-agent or per-pair limit. One limits value injected by the daemon, ready for a later settings system; counters in memory, clock injected. Directory frames do not count against it.
10. **Sender in-flight cap**: at most 256 `Mail` requests in flight per node; over it, that PC's rows fail locally with `REMOTE_LIMIT` and nothing is sent.

## 거부한 대안
- **Accept prompt / one-time short code / invite link as the v1 pairing** (Syncthing pending device, KDE Connect, Reason Network, Magic Wormhole) — user rejected directly: 「결국 2,3,4는 1이 되야 가능한거 아냐?」 → 「1먼저 해.」; they all end in registering the ID and stay later conveniences on `peer.add`.
- **Reason Network's limits — 60 mails/min per PC and 12 per 10 min per pair** (D5's first version, 「resason network 에 맞춰서 두던지」) — user rejected directly (「1개 에이전트가 60개 한꺼번에 보낼일은 없을테니 그냥 의미없는 수치」 → 「ㅇㅇ 일단 넘어가고」); an LLM ping-pong loop runs at a few mails a minute, so per-minute caps would not catch it; those values were tuned for occasional human-watched sessions.
- **Per-agent or per-pair limits** — normal use never reaches one and a loop runs below any useful per-minute cap; remote mail behaves like local mail, which has none; limiting our own agents belongs at the sender, defending against a broken peer at the receiver. Loop detection for both is T-121.
- **Deferring mail over the backstop instead of refusing** — needs the outbox (persistent queue + resend + receiver dedup), the next unit T-120 (D3).
- **Gating after `accept_bi`** — unlisted or silent dialers would hold accept permits; Reason Network gates before `accept_bi`.
- **Saving the list before revoking** — removal must bite at once (AC7 ≤ 1 s) even if the file write fails.
- **Leaving validation to `handle_send`** — ingress is today's only validator (`handle_send` comment) and the remote frame skips ingress.
- **No-forwarding by handing `handle_send` an emptied snapshot** — `handle_send` takes its own snapshot (`port.addressing_sources()`), so the caller cannot; a named seam is needed.
- **Running receive work on the inbox task** — `handle_send` injects inline through the blocking `DeliveryPort::inject` (ADR-0125 decision 1), and the inbox-draining task must never await; one stuck recipient must not stall a PC's other mail.
- **Peer-written notice text, or routing the overdue notice by the requester id** — a peer must not speak in the daemon's notice voice to a local agent, and the requester id is not trusted for routing back.
- **Relying on transport's pending table without a cap** — it has no cap (`pending.rs` `Pending::insert`), and with a 120 s deadline a dead-but-not-yet-closed PC would pile up waiters.

## 근거
- D1 (2026-11-18) and D5 (2026-11-19, revised 2026-11-20): user quotes above; 「서로 상호 연결이니 공격하는건 없을테니」.
- Reason Network re-checks membership per frame (`on_wire`) and gates before `accept_bi`; Syncthing pairs by device-ID exchange.
- 256 in-flight sits above the 600/min backstop's normal traffic; the 64 KiB body cap already exists locally (`control/ingress.rs` `MAX_BODY_BYTES`).
- S37 PRD+TRD review, PASS after 4 rounds (2026-11-20).

## 영향 / 불변식
- A new PeerWire message that does anything other than mail (or directory / liveness housekeeping) needs a new decision and a permission value beyond `mail`.
- No frame is read from an unlisted node; removal takes effect before the save.
- The remote entrance never forwards to a third PC (`ResolveScope::LocalOnly`).
- Which PC sent a mail is decided only from the link (R8); the agent name is the sending daemon's claim and is shown with that PC's name.
- New sender-visible codes: `REMOTE_LIMIT` (backstop, body re-check, full receive queue, in-flight cap) · `REMOTE_REFUSED` (permission, id collision, validation, unknown code) · `NAME_TOO_LONG`.
- The backstop is policy in the messaging lib (ADR-0110); the value is the daemon's to inject.
- Over the backstop v1 refuses; deferral arrives with T-120.
