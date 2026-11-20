# Transport crate (link library)

> Live reference doc — edit in place when the code moves. Anchors are file and symbol names only.
> Decisions and their reasons: ADR-0177 (`docs/decisions/0177-전송을-transport-crate-로-새로-세우고-밖에는-패킷-정의만-남긴다.md`) · ADR-0180 (connection events, refusal budget) · ADR-0181 · ADR-0182 (event stream) · ADR-0183 · ADR-0310 (stage-3 decisions) · ADR-0346 (shell retry policy) · ADR-0388 (the iroh adapter).
> Canonical contract and isolation gates = the header of `crates/engram-dashboard-transport/src/lib.rs`; WS limits = the header of `src/ws.rs`; iroh limits = the header of `src/iroh.rs`. This doc does not copy them.

**Whole picture** = `docs/reference/architecture.md` · **Consumers** = `docs/reference/structure/daemon-link.md`

## What it is

`engram-dashboard-transport` is a reusable link library with zero workspace dependencies. It owns connection lifetime for N peers: dial / accept, an opaque handshake, request–reply pairing, keepalive, write deadlines, backoff and reconnect, an optional incoming stream layer (sequence, resume, gaps), bounded queues with backpressure, and a structured event stream. It does not know any packet vocabulary: the consumer supplies one `Wire` impl (the only generic), while links and time are `dyn` seams (`Dialer` · `Listener` · `LinkTx` · `LinkRx` · `Clock`). A `Registry<W>` runs one supervisor task per peer; each raised peer gives the caller a `Peer<W>` (send surface) and an `Inbox<W>` (the only receive surface, carrying messages and events on one ordered line). Two real adapters exist, each behind its own feature (off by default): `ws` — a WS dialer and a loopback-only WS listener; `iroh` — one iroh QUIC endpoint with a dialer and a key-gated listener, peers authenticated by their 32-byte node key. Address syntax, auth, version negotiation, trust and tie-break policy are the consumer's.

## Modules

All under `crates/engram-dashboard-transport/src/`.

- `lib.rs` — re-exports, crate contract, receiver-side invariants, isolation gates ①–⑤, known limits.
- `wire.rs` — `Wire` (the consumer vocabulary: `Out` / `In` / `Tag` / `StreamKey`, `encode` / `decode`, `request_tag` / `reply_tag` / `stream_mark` / `resume_request`) and `DecodeClass`.
- `frame.rs` — `Frame` (`Text` · `Binary` · `Keepalive`), `Close`, `CloseCode` (crate-reserved range, `CloseCode::CONSUMER_BASE` marks where consumer codes start).
- `link.rs` — the transport seams: `Dialer`, `LinkTx` / `LinkRx`, `LinkRead`, `Listener`, `Accepted`, `LinkInfo`, `PermitPool` / `ConnPermit`, `Handshake` / `HandshakeStep`, `RejectKind`, `DialFailure`, `Address`, `LinkError`.
- `event.rs` — what goes up: `Incoming` (`Message(Arrival)` | `Event(TransportEvent)`), `Arrival` (with its connection `Generation`), `TransportEvent`, `PeerId` (the registry label), `Direction`.
- `policy.rs` — `Policy`, `ReconnectPolicy`, `QueueFullPolicy`, `DecodeFailurePolicy`; every value has a crate default the consumer overrides. `Policy::supervisor_keepalive` (default on) switches the supervisor's own ping and silence cut.
- `registry.rs` — `Registry<W>` (`new` · `add` / `add_with_policy` · `attach` / `attach_with_policy` · `remove` · `close_all` · `get` · `peers` · `broadcast`) and `Inbox<W>`.
- `peer.rs` — the per-peer supervisor task (driving layer) and the `Peer<W>` handle (`request` / `request_in` · `notify` / `notify_in` · `resume_stream` · `reconnect_now` · `drop_link_in` · `close` · `close_after_flush` · `state`), `SendError`.
- `machine.rs` — pure connection state machine (`Machine`, `Input`, `Action`, `PeerState`, `GaveUpReason`, `DisconnectCause`); no runtime or I/O.
- `pending.rs` — pure request–reply table (`Pending`, `RequestError`, retired-tag ring).
- `stream.rs` — pure incoming stream bookkeeping (`Streams`, `StreamMark`, `Verdict`).
- `budget.rs` — private per-supervisor received-byte tally and read gate (`BYTE_BUDGET` lives in `peer.rs`).
- `clock.rs` — `Clock`, `SystemClock`, `with_deadline`.
- `mask.rs` — `mask_reason`: masks peer-given text before it reaches an event or `Display`.
- `ws.rs` (feature `ws`) — `WsDialer`, `WsListener` (`bind` · `local_addr` · `dial_address`), `AcceptPolicy`, `AcceptRefusals` / `AcceptRefusal` / `RefusalKind`, `ACCEPT_ERROR_PAUSE`; maps `Frame` ↔ WS messages and close codes.
- `iroh.rs` (feature `iroh`) — `IrohEndpoint` (`bind` · `id` · `bound_addrs` · `dialer` · `close`), `IrohConfig` / `Helper` (plain values: secret key bytes, ALPN, port, relay, address lookup, mDNS, payload cap), `IrohAcceptPolicy` (`accept_if`, stream-accept timeout, permits), `IrohDialer`, `IrohListener`, `dial_address`, `PREAMBLE`, `MAX_CLOSE_REASON_BYTES`; frames one link onto one QUIC bi-stream and maps `CloseCode` ↔ QUIC application codes.
- `testing.rs` (feature `test-support` or `cfg(test)`) — `ManualClock`, `MemoryNetwork` / `MemoryEndpoint`, `MemoryListener`, `settle` / `settle_until`; never referenced by production code.
- `tests/` — the real-socket suites `ws_dial` · `ws_reject` · `ws_write_deadline` · `ws_accept` (behind `ws` + `test-support`) and `iroh_link` (behind `iroh` + `test-support`; loopback endpoints, no helper servers); `tests/common/` is their shared scaffolding.

## Wiring and flow

- Consumers: the shell's daemon link (`src-tauri/src/daemon_client/link/`, `ShellWire`, dialing via `Registry::add`) and the daemon's accepting side (`crates/engram-dashboard-daemon/src/link/`, `DaemonWire`, accepting via `Registry::attach`). Both enable only `ws` in `[dependencies]`; `test-support` only in `[dev-dependencies]`.
- Dial path: `Registry::add(id, addr, handshake)` → supervisor → `Dialer::dial` (the registry's single dialer) → `Handshake::start` / `on_frame` until `Done` → operating phase; on loss, backoff (`ReconnectPolicy`) and redial with a new `Generation`. Re-adding the same `PeerId` closes and replaces the old supervisor; the new handle gets a new `Inbox`.
- Accept path: the consumer owns the accept loop — `Listener::accept` → read `Accepted::info()` if needed → `Registry::attach(label, accepted, handshake)`. An attached peer starts at the handshake, never redials, and removes itself from the registry when it ends. `attach` discards the `LinkInfo` (destructured away in `attach_with_policy`), so a consumer that needs it later must keep its own label → info map.
- Handshake: `start()` on the dialer usually returns `Send(auth)`; on the acceptor `Await`. `Reject { kind, last, reason }` closes with `RejectKind::close_code` (`Terminal` → `CloseCode::HANDSHAKE_REJECTED`, `Retryable` → `CloseCode::HANDSHAKE_RETRYABLE`). The dialer reads the received close code with `RejectKind::of`: `HANDSHAKE_REJECTED` gives up without spending reconnect budget (`GaveUpReason::Rejected`); code 0 (a QUIC close with no code), the WS normal-close codes and `GOING_AWAY` are not refusals; any other code is retryable. A dialer can refuse before dialing with `DialFailure::Refused`.
- Sending: `request` / `notify` bind to the generation current at the call; `request_in` / `notify_in` bind to a caller-given `Generation` (e.g. from `Arrival::generation`). Nothing is written on a different connection than the one it was bound to. Pairing uses `Wire::request_tag` / `reply_tag`; a reply wakes its `request` inside the supervisor and never reaches the inbox.
- Receiving: one `Inbox` per handle; full = the supervisor stops reading that peer (no discard, no disconnect). The end of a handle is `TransportEvent::Closed` followed by `None`.
- iroh adapter: `IrohEndpoint::bind(config, accept_policy, clock)` builds one endpoint (the `N0` preset only when relay and address lookup are both n0's, else `Minimal` plus the chosen pieces) and returns it with its only `IrohListener`. Dial: `dial_address(key, direct addrs)` → `IrohDialer::dial` → QUIC connect (TLS proves the key; no 0-RTT) → open the bi-stream and write `PREAMBLE`. Accept: QUIC + TLS handshake under the handshake timeout → `accept_if(key)` (false → close `HANDSHAKE_REJECTED`) → accept permit (none → close `GOING_AWAY`) → bi-stream + preamble under the stream-accept timeout (missed or wrong → close `PROTOCOL_ERROR`) → ready queue → `Listener::accept`. Frames = kind byte + u32 BE length + payload, capped by `IrohConfig::max_payload` in both directions.
- WS adapter: `WsListener::bind` refuses a non-loopback address, checks Origin and Host against `AcceptPolicy`, takes an upgrade permit at TCP accept, and reports refusals to `AcceptRefusals` (masked); when that queue (`AcceptPolicy::refusal_queue`) is full new reports are dropped and only counted (`AcceptRefusals::dropped`). `WsDialer::new(max_message_bytes)` and `AcceptPolicy::max_message_bytes` set the one-message cap for both directions; WS gives frame boundaries, so the crate adds no framing of its own.

## Promises no single file shows

- Receiver invariants (canonical: `lib.rs` 「Receiver-side invariants」): never await `Peer::request`, a `close_after_flush` future or `Registry::close_all` on the task that drains that inbox; always drain an inbox to `None`; dropping or closing an inbox folds its supervisor at once and discards accepted outgoing work.
- Single writer: only the supervisor holds `LinkTx`; every send goes through its queue. A command is stamped with its generation and checked just before the write (`Supervisor::on_cmd`).
- Ordering: messages and events of one handle come up in produced order through the supervisor's ordering area; a disconnect follows what preceded it. There is no order between an old and a new inbox of the same `PeerId`.
- Generations: one shared counter per `PeerId` (a floor passes to a replacing supervisor), so numbers never repeat across re-adds; accepted peers are outside the generation table and their labels are assumed never reused.
- The reserved close-code range belongs to the crate (`CloseCode::is_reserved`); a consumer picks its own codes from `CONSUMER_BASE` up. Delivery of a close code to the peer is best effort; branch only on `Close::code`, never on its text.
- `LinkInfo::peer_key` is `None` from `ws` and the in-memory harness and the authenticated 32-byte node key from the iroh listener. `LinkInfo` is `#[non_exhaustive]`: outside the crate build it from `Default`.
- Clock pairing on accept: the auth deadline of an attached peer = `Accepted::upgraded_at` + `Policy::connect_timeout`, so the listener must stamp `upgraded_at` with the attaching registry's `Clock` (`WsListener::bind` takes that clock; `Registry::attach_with_policy` debug-asserts that `upgraded_at` is not later than its clock's now). The daemon pairs them through `Listening::clock` → `Server::new`.
- `Accepted::new` is public and bypasses every accept guard (WS: upgrade permit, Origin / Host checks, refusal reports; iroh: key gate, accept permit, stream-accept timeout); `WsListener` and `IrohListener` are the only guarded paths from a real socket. A new listener implementation must supply its own guards — the compiler does not check it.
- iroh adapter: ★iroh types never leave `iroh.rs`★ — consumers get plain values and raw key bytes, and the key's text form is theirs. The key gate runs before any stream is accepted, so a refused or silent dialer holds no permit past the stream-accept timeout. A peer close with QUIC code 0 reads as `LinkRead::Closed(None)` (going away, not a refusal); every locally detected loss — QUIC idle-out included — is `Err(LinkError)`, never `Closed`; a write into a connection the peer closed with a code returns `Ok` so the code reaches the supervisor through the read side. A close waits, under the caller's deadline, until the peer has acknowledged what was written, so `close_after_flush` / `close_all` deliver their tail over iroh too. Exact mappings and limits: the `src/iroh.rs` header.
- Supervisor keepalive: with `Policy::supervisor_keepalive` off the supervisor neither pings nor cuts for silence and `Policy::check` skips `write_deadline < ping_interval < idle_timeout`; a dead peer is then noticed only when the link fails (over iroh: QUIC's idle timeout — measured 30 s on loopback, `src/iroh.rs` header).
- Byte bound per supervisor = `BYTE_BUDGET` (`peer.rs`) + one message; the one-message cap is the adapter's job.
- The crate's own code emits no `tracing`; the event stream is the only trace (with `iroh` on, iroh logs through the consumer's subscriber).
- Pure layers (`machine.rs` · `pending.rs` · `stream.rs`) must not name tokio, futures, `std::net` or a workspace crate — gate ③ in `lib.rs` is a regex, not the compiler.
- Who fills what: the consumer supplies `Wire`, `Handshake`, `Policy` overrides, the accept loop, the `PeerId` labels and any address meaning; the adapter supplies `LinkInfo` and refusal reports; the registry supplies generations and events.

## Where to look

- Contract and limits: `src/lib.rs` header, `src/ws.rs` header, `src/peer.rs` header (supervisor invariants, operating-loop priority).
- Seams: `src/link.rs` (`Handshake`, `HandshakeStep`, `RejectKind`, `LinkInfo`, `Accepted`, `Listener`, `Dialer`), `src/wire.rs` (`Wire`), `src/frame.rs` (`CloseCode`).
- Registry and handle: `src/registry.rs` (`Registry::add_with_policy`, `Registry::attach_with_policy`, `Inbox`), `src/peer.rs` (`Peer`, `Supervisor::run`, `teardown`, `handshake_once`, `shake_hands`).
- Events and policy: `src/event.rs` (`TransportEvent`, `Incoming`), `src/policy.rs` (`Policy`, `ReconnectPolicy::delay`).
- Adapters: `src/ws.rs` (`WsDialer::new`, `WsListener::bind`, `AcceptPolicy`), `src/iroh.rs` (`IrohEndpoint::bind`, `IrohConfig`, `IrohAcceptPolicy`, `dial_address`).
- Harness: `src/testing.rs`; real-socket suites in `tests/`.
- CI gates: `.github/workflows/ci.yml` (transport gates 4a–4e, `shell gate 5`).
