# S37 TRD — daemon↔daemon agent mail over iroh (parent)

- Status: draft 1 · 2026-11-15 · canonical ADRs: none yet (U0 writes them); governing existing ADRs = ADR-0310 (D2 · D3 · D5 ③) · ADR-0309 (decision 3) · ADR-0110 · ADR-0177 · ADR-0179 · ADR-0291 · ADR-0266.
- ★Assumes the ★ defaults of PRD §5 D1–D7 — the user has not approved the PRD yet.★ §5 marks every choice that depends on an open D-item.
- PRD: `prd.md` (draft 1). Plan: `../../research/multi-daemon-iroh-link-plan-2026-11-11.md` (read as claims; §9 lists where code disagrees).
- Reference: new `docs/reference/structure/peer-link.md` (created with U2's code) · `docs/reference/architecture.md` — 「Connection diagram」 (add the daemon↔daemon iroh line), 「Crate dependencies」 block (`daemon ─▶ … transport(ws)` gains `iroh`), 「Modules」 bullets **daemon** (「It accepts shell connections on the transport (`link` …)」 — add the peer link and the daemon-declared `peer.*` commands), **command** (「`mail.*` = daemon」 — `mail.*` bus commands do not exist; the daemon's first declared commands are `peer.*`), **messaging** (「delivery · parking · bounced · ledger」 — add the remote branch and inbound limits), **transport** (「The shell depends on it (`ws` feature) … The daemon's accepting side runs on it too」 — add the `iroh` feature and the daemon peer link) · transport `src/lib.rs` head (U1: gate ④ feature combinations, gate ⑤ suite list, 「There is only one adapter」 known limit) · messaging `src/lib.rs` head (U3: port list) · daemon `src/lib.rs` head (U2: 「The accepting side runs on the transport crate (`link`)」) · `CLAUDE.md` 「모듈」 transport · daemon · messaging bullets · `prompts/engram-help.md` + `prompts/agent-priming.md` (U2 `peer.*` · U3 remote address). Each fixed in the commit that makes it wrong.

## 1. Scope and PRD relation

Covers R1–R15 / AC1–AC12. Out: everything in PRD §2 non-goals (shell → N daemons, outbox, pairing codes, groups, task delegation).

| Requirement | Unit |
|---|---|
| R1 identity · R2 trusted list · R3 links + online state · R4 gate · R11 port · R12 mDNS · R13 off when unpaired | U2 (adapter U1) |
| R5 send · R6 results · R7 reply · R8 sender = link · R9 limits · mail only | U3 |
| R10 helper servers | U1 (builder) · U2 (settings) |
| R14 LLM commands | U2 (`peer.*`) · U3 (remote address on the existing send) |
| R2 / R3 screen | U4 |
| R15 local unchanged | every unit (shell link, loopback rule, local mail untouched) |

## 2. Module boundaries

```
daemon A                                               daemon B
 control ingress (MCP eg_send · /control/send)          link/peer: accept gate → remote entrance
   └ messaging handle_send ─ Resolution::Remote          messaging: limits → handle_send(Entrance::Remote)
        └ RemoteDeliveryPort (non-blocking)                   ▲ verdict ◄─┐
 link/peer: Registry<PeerWire> ══ transport `iroh` (QUIC, node keys) ══ Registry<PeerWire>
 link (shell, WS loopback) — unchanged                 link (shell, WS loopback) — unchanged
```

- **transport** — new `iroh` feature module beside `ws` (opt-in, off by default). Owns everything iroh: endpoint build from plain config values (secret key bytes · relay = default | none | URL · address lookup = default | none | URL · mDNS on/off · listen port · ALPN), `Dialer` / `Listener` / `LinkTx` / `LinkRx`, framing, close-code mapping, `LinkInfo::peer_key`. ★iroh types do not leave this module★ — the daemon's manifest gets no iroh dependency, so the adapter stays replaceable (PRD §6 vendor risk). No relationship policy here: no trust list, no tie-break, no PeerWire (ADR-0177 decision 4).
- **daemon `link/peer/`** (new, beside the shell `link/`) — identity key file, trusted-list store (`peers.json`), `PeerWire`, its own `Registry` + `Listener` (ADR-0310 D2), accept gate, dial-every-listed-peer, tie-break, online state, endpoint lifecycle (D7), the `peer.*` command table, the messaging adapters (`RemoteDeliveryPort` adapter, remote entrance caller).
- **messaging** — remote branch at the single resolution point (ADR-0309 d3), `RemoteDeliveryPort`, verdict entry, remote entrance with limits, node qualifier on `SenderIdentity`. Still zero workspace deps (ADR-0110).
- **platform** — new owner-only private file write (Windows DACL · elsewhere mode 0600) (ADR-0266).
- **protocol** — one additive peer-state broadcast for the app (precedent `UsageSubscribe` / `UsageLimitsUpdated`); no daemon↔daemon vocabulary (§5 T6).
- **shell · frontend** — U4 only: relay the peer-state broadcast; the peers panel.
- **Untouched** — agent crate (ADR-0003), cli (the send path and its route are unchanged; the address is just a string), base (`file` used as is), the shell↔daemon link and its loopback rule, `ShellWire` / `DaemonWire`.

## 3. Data shapes

### 3-1. Persistent files (daemon state folder `<data root>\daemon\state\` — `DataLayout::daemon_state_dir`)

- **Identity key** — `peer-key` (raw 32-byte secret). Created once, on first need (first `peer.*` call or first add), never rewritten. Written owner-only through platform (temp created with the private DACL → sync → rename without replace). The public half's canonical text = the PC's ID. Unreadable (lock / IO) → the feature does not start and `peer.list` says so; never regenerate on a read failure. Corrupt (wrong length) → set aside as `peer-key.corrupt`, create a new key, loud warning log, `peer.list` flags "ID changed" (§8 X5).
- **Trusted list + network settings** — `peers.json`, base `file` rules (ADR-0291: version key, cap, newer-version / unreadable = refuse to write; the feature does not start and `peer.*` writes answer `CONFLICT`). Shape:
  - `peers[]` = `{ id, name, address?, permission }` — `id` = the other PC's ID text (required, unique); `name` = display name, used as the address suffix (required, unique case-insensitively, no `@`, no whitespace); `address` = optional `ip:port` direct address; `permission` = `"mail"` (only value accepted in v1; `"control"` reserved for the shell → remote daemon stage). Room for a later group record = a sibling top-level key, not a field on entries.
  - `network` = `{ port?, relay, address_lookup, mdns }` — `port` absent = random (§8 X1); `relay` / `address_lookup` = `"default"` (n0) | `"none"` | a URL (Syncthing `default` keyword precedent); `mdns` = bool, default `true`.
  - Why one file: one owner (the peer module), one load/refuse rule, one writer. The daemon has no settings file today and shell settings cannot carry these — the daemon runs and links without the app (R13 · R3).

### 3-2. Link framing (transport `iroh`)

- One ALPN per relationship: `engram/peer/1` opened in v1; `engram/shell/1` reserved (not registered on the endpoint in v1). Unknown ALPN is refused by the QUIC handshake.
- One QUIC connection per link, one long-lived bi-stream opened by the dialer; the dialer writes a 1-byte preamble at once (the stream is invisible to the peer until a byte is written).
- Frame = kind byte (text · binary · keepalive) + u32 BE length + payload; max payload given by the consumer like `ws` `max_message_bytes` — PeerWire uses 256 KiB (64 KiB body + headroom). Oversized or malformed framing = close with `PROTOCOL_ERROR` (4005).
- Close: our `CloseCode` u16 ↔ QUIC application error code; reason truncated to an adapter-local byte cap (exact QUIC cap confirmed in U1). QUIC implicit close (code 0) → `LinkRead::Closed(None)` → read as "could not connect / going away", never a terminal rejection (`RejectKind::of` returns `None` only for 1000 · 1001 · 4000 — U1 maps 0 explicitly). The final `Reject{last}` frame is best-effort diagnostics; correctness never depends on it.
- `ping()` sends a real keepalive frame (QUIC has no auto-pong); the transport supervisor's ping/idle stays the liveness rule.
- Accept: QUIC accept + ALPN + bi-stream accept run in a background task feeding a ready queue bounded by a `PermitPool` (same shape as `WsListener`), so `Listener::accept` stays cancel-safe. `Accepted.info.peer_key` = the authenticated 32-byte endpoint id; `remote` = the observed socket address (diagnostics only).
- Dial address = `<id>` or `<id>@<ip:port>`; the adapter parses it. iroh verifies the remote key against the id on dial.
- iroh build: `default-features = false` + `tls-ring` (no `portmapper`, no metrics). Presets: `N0` when relay and lookup are both `default`; otherwise `Minimal` + custom relay map / pkarr + DNS lookup / none. Typed peer addresses go in as direct addresses (memory lookup). Fixed port = `bind_addr` on `0.0.0.0:<port>` (+ `[::]:<port>`). mDNS = the separate crate `iroh-mdns-address-lookup` (§9 P3).

### 3-3. PeerWire vocabulary (daemon `link/peer/`, JSON text frames)

| Message | Direction | Fields (boundary level) |
|---|---|---|
| `Hello` | both (dialer first; acceptor answers after `Done`, the shell-link pattern) | `protocol_version` — no node field; the node is the link key |
| `Mail` | request (tag = mail id) | `id` · `from {agent_id, name}` (no node — stamped from the link) · `to[]` (agent names/ids on the receiver) · `body` · `reply_to?` · `request {reply_by_raw}?` · `kind` (`mail` \| `notice`) |
| `MailVerdict` | reply to `Mail` | `id` · rows `[{to, outcome, recipient_id?, code?, reason?}]`; outcome = `delivered` \| `held` \| `refused` \| `unknown_recipient` |
| keepalive | both | `Frame::Keepalive` (never reaches `Wire::decode`) |

- One `Mail` per (send, remote PC) carries that PC's recipients; one verdict answers it.
- Version: equal `protocol_version` required; mismatch = `Reject{Terminal}` with "version mismatch" (shown in peer state; redial re-armed on daemon start and on `peer.add`).
- Decode failure = drop the frame + log (`DecodeFailurePolicy::DropFrame`); a frame naming a non-`mail` permission action does not exist in the vocabulary (R9 by construction).
- `held` = the receiver's local `SendStatus::Pending` (parked: recipient asleep, or live but the drain could not inject — busy; in memory, 24 h TTL, mailbox cap 100). `refused` codes = `REMOTE_LIMIT` (§4-5) · the receiver's own row codes (`RECIPIENT_AMBIGUOUS` · `MAILBOX_FULL` · `REQUEST_CAPACITY`) · `REMOTE_REFUSED` (permission · id collision · body too large).

### 3-4. Sender-side result mapping (what the sending agent sees — R6)

| Result | `SendStatus` | code / hint |
|---|---|---|
| delivered | `delivered` | — |
| held | `pending` | hint: held on `<pc>`, recipient asleep or busy |
| refused | `failed` | the relayed code or `REMOTE_LIMIT` / `REMOTE_REFUSED` + reason |
| unknown recipient | `failed` | `RECIPIENT_NOT_FOUND` (hint names the PC) |
| not delivered (no live link) | `failed` | new `PEER_OFFLINE` |
| unconfirmed (no verdict within the shared request deadline, 8 s — ADR-0181) | `pending` | hint: unconfirmed — may or may not have arrived; do not resend blindly |

- Keeps the three-word status vocabulary (spec §6); new codes = `PEER_OFFLINE` · `REMOTE_LIMIT` · `REMOTE_REFUSED`. A late verdict still updates the ledger row (visible in `eg_messages`). v1 sends no later receipt when held mail is finally injected (§8 X3).

## 4. Flows and concurrency

### 4-1. Endpoint lifecycle (D7 · R13)
The peer module starts the iroh endpoint only while `peers` is non-empty: at boot if the list has entries, on the first `peer.add`; it stops it (closing all links) when the last entry is removed. Creating the key file contacts nothing. `peer.setNetwork` restarts a running endpoint.

### 4-2. Links (R3 · R4)
- Dial: every listed peer gets a dialing `Registry::add` handle — reconnect without limit (`max_attempts = u32::MAX`, the shell precedent ADR-0346), backoff capped at 60 s and connect timeout 20 s (Reason Network `net.rs` 2→60 s · 20 s).
- Accept loop: read `peer_key` from `Accepted::info()` before attach. Not listed (or permission ≠ `mail`) → attach with a handshake whose `start()` is `Reject{Terminal, "not paired"}` — no frame is read (R4), the dialer gives up without spending budget; the refusal is logged with the key's ID text. Listed → attach with the PeerWire handshake. `Registry::attach` drops `LinkInfo`, so the peer module keeps `link label → node id` in its own per-link state.
- Tie-break: when two live links exist for one node, keep the one dialed by the smaller ID and close the other with `GOING_AWAY`. A dial that ended in a terminal refusal is re-armed on: an accepted link from that node ending, `peer.add`, daemon start.
- Online state per PC = `online` (≥1 live link) · `offline` · `refused` (terminal refusal from the other side) · `version mismatch` · `endpoint error` (e.g. port in use).
- Removal (`peer.remove`): drop the entry (file first), close that node's handles with `HANDSHAKE_REJECTED`, and later accepts are refused by the gate (R4 · AC7 ≤ 1 s).
- Transport receiver invariants hold: the inbox-draining task never awaits `request` (sends run on spawned tasks); every inbox is drained to `None`.

### 4-3. Send (A side)
`handle_send` resolves each token once from one snapshot. Local resolution runs first, unchanged; only a token that is not found locally **and** ends in `@<listed PC name>` becomes `Resolution::Remote{node, name}` (ADR-0309 d3 — 「여기 없으면 다른 데몬으로」). `@all` / `@here` never expand to remote agents. A remote row is recorded in the ledger (`to` = the notation, `to_id` = none until the verdict) without opening a contract (§4-6), then handed to `RemoteDeliveryPort::enqueue` — non-blocking, called outside the messaging lock (same discipline as `DeliveryPort`). The daemon adapter: no live link → immediate `PEER_OFFLINE` verdict; else a PeerWire request on a spawned task, verdict or deadline → the kernel's verdict entry transitions the row. The control entrance answers the agent once every remote row has a verdict or the shared deadline has passed (AC6); local rows are unaffected by the wait.

### 4-4. Receive (B side)
Peer inbox → decode `Mail` → messaging remote entrance with the link's node: R8 checks (`from.agent_id` must not be a local agent id; no node claim exists in the frame) → limits (§4-5) → body cap → `handle_send` with `Entrance::Remote`, `SenderIdentity` carrying the link's node and sender display `"<name>@<B's name for that PC>"` (the envelope shows the address the recipient replies to — R7) → per-row results → `MailVerdict`. `Entrance` stays a log label (pipeline does not branch on it).

### 4-5. Limits (D5 · R9)
At the receiving side's resolution point (messaging remote entrance — policy in the lib, ADR-0110): per PC 60 mails/min (whole frame refused), per (remote agent, local recipient) pair 12 per 10 min (that row refused), body 64 KiB (same value as the local `MAX_BODY_BYTES` in `control/ingress.rs`; the sender already caps, the receiver re-checks). Hit = `refused` + `REMOTE_LIMIT` with the limit as reason (AC10). Counters are in memory, clock injected (`now: Instant`).

### 4-6. Ledger identity (boundary decision)
- Keys stay agent UUIDs — no node in any key. Rests on ADR-0309's 「an id must be unique across servers」 (fresh UUID v4 per profile); a remote frame reusing a local agent id is refused (R8).
- The node is a display/routing qualifier: `SenderIdentity` gains `node: Option<NodeRef>`; `NodeRef` = a messaging-local opaque string (the ID text) — not a uuid alias (§9 P2). `AddressingSources` gains the listed PCs `(NodeRef, name)` so resolution stays one snapshot.
- Reply contracts live on the **receiving** daemon: `handle_send` there opens the contract for its recipient and the recipient's reply closes it there (closing is replier-side today). The sending daemon opens none for remote rows. The overdue `<notice>` to a remote requester travels back as `Mail{kind: notice}`, best-effort (lost if the link is down). The requester's own "awaiting reply" view does not list remote requests in v1 (§8 X4).
- Mail ids are unique per daemon only; a remote id that collides in the receiver's ledger → `REMOTE_REFUSED` (probability as for local collisions).
- Size: `SenderIdentity` + `MessageRecord` (optional node) + `AddressingSources` + one resolution variant + one port + verdict entry + remote entrance + limiter. No key migration (mailbox and ledger are not persisted).

## 5. Decisions

| # | Decision | Depends on | Reason |
|---|---|---|---|
| T1 | iroh 1.x behind transport (`iroh` feature), `tls-ring` only, no `portmapper`, iroh types never leave the module | — | user criteria; replaceable seam |
| T2 | One QUIC connection + one bi-stream per link, preamble, length-prefixed frames, ready-queue accept | — | ordered link for seq/resume; QUIC streams are unordered; cancel safety |
| T3 | Own `PeerWire` · `Registry` · `Listener`; shell link untouched | — | ADR-0310 D2 |
| T4 | Identity = iroh secret key file; ID = its public half; ADR revises ADR-0310 D3 | — | key-bound identity by construction; `DaemonId` was never built (§9 P1) |
| T5 | Gate at accept on `peer_key` before attach; terminal refusal via a reject-at-start handshake | D1 | R4; the unpaired dialer stops retrying |
| T6 | PeerWire vocabulary lives in the daemon, not protocol | — | protocol = shell↔daemon contract; only daemons speak PeerWire; ADR-0310 D2 rejected the shell knowing daemon↔daemon types |
| T7 | Trusted entry `{id, name, address?, permission}`, pairwise, both sides paste the ID | D1 | ★ D1; a one-time code would add a join ALPN + code lookup + a new unit |
| T8 | Address `name@pc`; local first, remote only on local not-found + listed PC suffix; PC names unique, no `@`; ADR amends ADR-0101 | D2 | ADR-0309 「`이름@서버` form addresses (an amendment of ADR-0101 — user decision)」; bare names would need a merged remote directory in PeerWire |
| T9 | No outbox: no link → `PEER_OFFLINE` at once; no resend, no dedup table | D3 | ★ D3; outbox = persistent queue + resend loop + receiver dedup set (later unit) |
| T10 | Network settings `relay` / `address_lookup` = `default` \| `none` \| URL; `N0` preset only when both default; typed address = direct address | D4 | ★ D4; "n0 only" drops the two settings; "none" makes `address` required in T7 and home↔office needs an own relay |
| T11 | Limits in messaging remote entrance: 60/min per PC, 12/10 min per pair, 64 KiB | D5 | ★ D5 (Reason Network `daemon/mail.rs`); other values = constants; "none" removes the limiter, the body cap stays (local cap) |
| T12 | mDNS via `iroh-mdns-address-lookup`, setting default on | D6 | ★ D6; "off" lets the crate be dropped |
| T13 | Endpoint runs only while ≥1 trusted PC exists | D7 | ★ D7; "always" starts it at boot and AC11 changes |
| T14 | Sender results mapped onto the existing three statuses + three new codes | — (R6) | keeps the LLM-facing vocabulary; AC6 needs the send to answer within the deadline |
| T15 | Ledger keys stay agent UUIDs; node = qualifier; contracts on the receiving daemon | — | §4-6; smallest change that keeps ADR-0309's single key |
| T16 | Non-blocking `RemoteDeliveryPort` + verdict entry; `DeliveryPort::inject` not reused | — | a network round trip must not run on the blocking inject contract or under the messaging lock |
| T17 | Tie-break (smaller ID's dial wins) in the daemon, not transport | — | relationship policy is the consumer's (ADR-0177 d4) |
| T18 | LLM surface: `peer.list` (one area query: my ID + address, settings, PCs + state), `peer.add`, `peer.remove` (`--name` / `--id` separate args), `peer.setNetwork` — first daemon-declared table, reached only through `call_daemon_command` | — (R14) | `llm-command-conventions.md` §2-2 (one query per area) and camelCase names; PRD AC1 / AC12's `peer.my_id` reads as `peer.list` (PRD wording to align) |
| T19 | `peer.*` commands land with U2, not U4 | — | U2's own checks (AC1 · AC2 · AC5 · AC7) need them; the app (U4) calls the same commands |
| T20 | Identity key owner-only via a new platform function | — | Reason Network protects the key on unix only (`daemon/mod.rs`); ADR-0310 D26 deferred file ACLs to this stage |

ADRs written in U0 (after the PRD+TRD review, before coding): ① iroh as the daemon↔daemon link (T1 · T2 · T17, rejected alternatives from the plan §1); ② daemon identity = node key (T4, revises ADR-0310 D3; records that ADR-0310 D5 ③'s handover is now built); ③ remote peers = mail only, pairwise trust, permission field (T5 · T7 · T11); ④ edge address `name@pc` (T8, amends ADR-0101); ⑤ product principle: no external account, no separate install.

## 6. Test plan

Rows of `.claude/skill-bindings/qa.md` 「돌릴 것 표」 unless named otherwise.

| Unit | What | With |
|---|---|---|
| U1 | adapter over two in-process endpoints, relay and lookup off, loopback direct addresses: accept + attach, `peer_key` present, terminal / retryable reject codes cross, implicit close 0 is not a rejection, keepalive and idle cut, two concurrent links between one pair stay distinct, malformed and oversized frames close with 4005, ALPN mismatch refused, `Hello` version mismatch terminal, cancel-safe accept | 「Rust 소스」 → 전체 회귀; transport 「모듈별 명령」 row gains `cargo check -p engram-dashboard-transport --features iroh` (gate ④) and a new real-socket suite `iroh_link` added to the three gate ⑤ copies (lib.rs head · `ci.yml` transport gate 4c · qa transport row). No network contact. |
| U2 | key file create / reuse / corrupt / unreadable; owner-only DACL read back (platform, Windows); `peers.json` load / refuse rules; gate refuses unlisted before any frame; removal closes ≤ 1 s and refuses later; tie-break; online state transitions; endpoint not started with an empty list (R13); `peer.*` shapes, errors with candidates | 전체 회귀 (daemon `--test-threads=4`, platform); two peer modules in one process over real `iroh` loopback endpoints |
| U3 | remote resolution order (local first, `@all` local only); sender rows for all six results incl. deadline → unconfirmed and late verdict; receiver limits, R8 refusal, held = parked; reply closes the receiver-side contract; notice back | messaging `cargo test -p engram-dashboard-messaging` + daemon; ★claude lane triggers★ (`crates/engram-dashboard-messaging/src/**` · `daemon/src/control/**` — cost; tell the user before running) |
| U4 | peers panel: My ID / My address copy, add (ID required) / remove, online state, remote mail visible in the recipient's slot | 「프론트」 + 「Tauri command · IPC」 rows → 전체 회귀 + GUI 실측 (「GUI 실측」 procedure) |
| U5 | AC2–AC11 on two real PCs: same LAN; home ↔ office (relay / hole punch); unpaired C refused; removal under traffic; helper servers off + typed address; Windows Firewall first-run prompt (allow / deny / non-admin) and whether it appears at all for the WMI-launched daemon; mDNS blocked → typed address | manual, recorded per AC in the U5 item; the qa binding has no two-PC row (none added) |

## 7. Implementation units

Order: U0 → U1 → U2 → U4; U3a can run beside U1/U2 (messaging crate only — no file overlap); U3b after U2 (both edit `daemon/src/link/peer/` and assembly). Each unit ends with a green build and its rows of §6.

- **U0 — ADRs.** Status: not started. The five ADRs of §5; README ADR line.
- **U1 — transport `iroh` adapter.** Status: not started. Feature `iroh` (off by default) with the module of §3-2; config struct of plain values; Cargo.toml description and lib.rs head; gate ④ / ⑤ copies; `ci.yml` transport gates. The daemon does not enable the feature yet.
- **U2 — identity, trusted list, PeerWire, peer link, `peer.*`.** Status: not started. platform private write; `link/peer/` (key, `peers.json`, PeerWire, registry, accept gate, dial, tie-break, state, lifecycle); daemon-declared command table joined into `DaemonTable` behind `call_daemon_command`; `prompts/engram-help.md`; new `docs/reference/structure/peer-link.md`; architecture.md daemon / transport / command bullets + diagram + dependency block; daemon lib.rs head; CLAUDE.md 「모듈」.
- **U3a — messaging kernel remote branch.** Status: not started. `Resolution::Remote`, `NodeRef`, `SenderIdentity.node`, `AddressingSources` PCs, `RemoteDeliveryPort`, verdict entry, remote entrance + limiter, new codes; messaging lib.rs head; architecture.md messaging bullet.
- **U3b — daemon wiring.** Status: not started. Port adapter over PeerWire, receive → remote entrance → verdict, entrance wait for verdicts, notice back; `prompts/agent-priming.md` remote address line.
- **U4 — app.** Status: not started. Protocol additive peer-state broadcast; shell relay; peers panel at the place §8 X2 decides (draft: a new slot content type beside `usage` — `SlotContent` in `src/api/layoutTypes.ts`, commands in `src/commands/`, store in `src/store/`, component in `src/components/slot/`, precedent TRD S21 usage-limit-slot). Remote mail needs no new UI: mail is shown only as the injected envelope in the recipient agent's slot, and remote mail arrives the same way with sender `name@pc`.
- **U5 — two-PC check.** Status: not started. §6 U5 row; results recorded here.

## 8. Risks and items for the user beyond D1–D7

- X1 — **Default listen port**: random unless set (Reason Network; iroh default) vs a fixed default (Syncthing listens on 22000). User-visible: with random, "My address" and a typed address break on every restart. Port busy at start: the endpoint reports `endpoint error` (draft: no silent fallback to random — exact iroh behaviour with a non-required bind confirmed in U1).
- X2 — **Where the PC list lives in the app**: `src/` has no settings screen (settings are commands only); draft = a new slot content type like the usage slot. User-visible placement.
- X3 — **No later receipt for held mail**: the sender sees `pending/held`; when the other PC finally injects it, the sender is not told (would add a `Receipt` message). User-visible.
- X4 — **Request deadlines across PCs**: the contract lives on the receiving PC; the overdue notice comes back best-effort; the requester's awaiting-reply view does not list remote requests. User-visible.
- X5 — **Corrupt identity key**: draft = set aside, new ID, loud warning, every pairing must be redone; alternative = stop the feature until the user acts.
- Firewall: binding UDP on all interfaces may raise the Windows Firewall prompt; whether it appears for a WMI-launched daemon is unknown — if it never appears, inbound direct paths stay blocked and only outbound / relay paths work (U5 measures).
- Weight: +~172 crates (plan, certain) + the mDNS crate (unmeasured); exe delta unmeasured; CI build time grows (transport `--all-features` builds iroh).
- Corporate networks blocking UDP / multicast / the relay domain; n0 public servers rate-limited (PRD §6).
- mDNS announces the PC's ID to the whole LAN (Reason Network keeps mDNS out of its debug log for that reason).

## 9. Plan vs code

- P1 — `DaemonId` / `Hello.daemon_id` (ADR-0310 D3, trd-3 3-3 U7) do not exist in code; the revising ADR replaces an unbuilt decision, nothing to migrate.
- P2 — plan's messaging `NodeId = uuid::Uuid`-style alias does not fit: the node identity is a 32-byte key → opaque `NodeRef`.
- P3 — mDNS is not an iroh 1.4.1 feature (features: `metrics` · `fast-apple-datapath` · `portmapper` · `tls-ring` · `tls-aws-lc-rs` · …); it is the separate crate `iroh-mdns-address-lookup` (Reason Network: 0.6 with iroh 1.4), not in the local registry.
- P4 — "remote mail in the existing mail UI": there is no mail UI in `src/`; mail is visible only as envelope text injected into the agent's slot.
- P5 — architecture.md 「`mail.*` = daemon」: no `mail.*` bus commands exist (mail goes through MCP `eg_send` and `/control/send`); the daemon declares no commands today (`control/commands.rs` head), so `peer.*` is the first daemon-declared table.
- P6 — no daemon settings file exists; port / helper URLs get a daemon-owned place (§3-1).
- P7 — platform has no owner-only ACL helper; T20 adds one.
- P8 — plan put tie-break tests in U1; tie-break is daemon policy (T17), U1 only proves two concurrent links stay distinct.
- P9 — plan's protocol `peer` vocabulary (from ADR-0310 D5) → daemon-owned PeerWire (T6).
- P10 — the 64 KiB body cap already exists locally (`control/ingress.rs` `MAX_BODY_BYTES`); remote reuses the value.
- P11 — plan's `peer.my_id` / `peer.status` → folded into `peer.list` (T18).

## Review record

