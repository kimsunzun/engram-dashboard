# ADR-0388: Daemon-to-daemon link runs on iroh behind the transport seam — one QUIC connection and one ordered stream per link, liveness from QUIC

- 상태: 확정 (2026-11-20, basis: user criteria 2026-11-11 (iroh chosen from the plan's candidate survey) + user decision P-F 2026-11-20 (liveness from iroh) + S37 PRD+TRD review PASS, 4 rounds, 2026-11-20)
- 관련: ADR-0392 (no external account, no separate install) · ADR-0389 (identity = the node key) · ADR-0390 (the accept gate policy) · ADR-0177 (decision 4: no relationship policy in transport) · ADR-0179 (decision 3: transport registry of N peers) · ADR-0181 (decision 1: one deadline per registry · decision 4: a late reply is dropped) · ADR-0310 (D2: own Wire · Registry · Listener per relationship · decision 16: no transport stream layer) · ADR-0266 · `docs/process/S37-multi-daemon/trd.md` (§2 · §3-2 · §4-2 · T1 · T2 · T5 · T17 · T23 · T44) · `docs/research/multi-daemon-iroh-link-plan-2026-11-11.md` (§1 · §2 · §3)

## 맥락
S37 links Engram daemons on the user's own PCs (same LAN, or home ↔ office across NAT) to carry agent mail. The link needs peer authentication, encryption, NAT traversal with a relay fallback, and no external account or separate install (ADR-0392). Today transport has only the `ws` adapter (loopback shell link). `LinkInfo::peer_key` was already reserved for an iroh node id (`transport/src/link.rs`). The plan's candidate survey (2026-11-11) compared iroh with the alternatives below; the user's instruction was 「iroh 말고 다른 대안 찾아보고 만약에 없으면 이걸 하되 …」.

## 결정
1. **iroh 1.x is the daemon↔daemon link library, inside a new transport `iroh` feature module** beside `ws` (opt-in, off by default). Build = `default-features = false` + `tls-ring` (no `portmapper`, no metrics). ★iroh types never leave the module★: the daemon's manifest gets no iroh dependency; the endpoint is built from plain config values (secret key bytes · relay = default | none | URL · address lookup = default | none | URL · mDNS on/off · listen port · ALPN); transport takes and returns raw 32-byte keys (`LinkInfo::peer_key`), and the ID text format is the daemon's.
2. **One QUIC connection and one long-lived bi-stream per link**, opened by the dialer, which writes a 1-byte preamble at once. Frame = kind byte (text · binary · keepalive) + u32 BE length + payload, max payload given by the consumer (like `ws` `max_message_bytes`); oversized or malformed inbound framing closes with `PROTOCOL_ERROR` (4005). One ALPN per relationship: `engram/peer/1` in v1; `engram/shell/1` reserved, not registered.
3. **The dialer never uses 0-RTT** (`into_0rtt` forbidden).
4. **Accept = a policy-free hook.** QUIC accept → the consumer's `accept_if(peer_key)` predicate, called before `accept_bi`; `false` closes the connection with `HANDSHAKE_REJECTED` (read by the dialer as `Terminal`) → ALPN + bi-stream accept under a stream-accept timeout → a ready queue bounded by a `PermitPool` (the `WsListener` shape), so `Listener::accept` stays cancel-safe and neither unlisted nor silent dialers hold accept permits.
5. **Close mapping.** Our `CloseCode` u16 ↔ the QUIC application error code; reason truncated to an adapter-local byte cap. A peer-sent close with code 0 → `LinkRead::Closed(None)`; an idle-out or any locally detected loss → `Err(LinkError)`, never `Closed`. The final `Reject{last}` frame is best-effort diagnostics.
6. **Liveness comes from iroh's QUIC connection** (user decision P-F): the peer link has no supervisor ping / idle and treats the connection close as link loss. Transport's `Policy` gains an off switch for ping / idle (crate default on, `ws` unaffected), and `Policy::check`'s `write_deadline < ping_interval < idle_timeout` applies only while they are on.
7. **Tie-break lives in the daemon, without churn.** A PC does not dial a node it already has a live link to (its dialing handle is removed with `Registry::remove` while an accepted link from that node is live and re-added when it ends — transport has no pause). Two links that come up within 5 s of each other → keep the one dialed by the smaller ID, close the other with `GOING_AWAY`; outside that window the newer link replaces the older. Directory frames go on the chosen link only.
8. **The PeerWire `Mail` request has its own transport deadline, 120 s** — the PeerWire registry's `Policy::request_timeout` (one value per registry, ADR-0181 decision 1; `Mail` is PeerWire's only request), apart from the agent-facing wait. The shell registry's shared deadline is unchanged.

## 거부한 대안
- **rust-libp2p** — needs access to a relay node (no ready relay service like n0's public relays), heavier (Swarm / behaviours / multiaddr), still 0.x (v0.57.1) (plan §1).
- **webrtc-rs / str0m** — needs our own signaling server plus STUN/TURN; we would build the hard parts (plan §1).
- **quinn alone** — no hole punching and no relay (plan §1).
- **tailscale-rs / tsnet** — needs a Tailscale account or a Headscale server (against ADR-0392); experimental (plan §1).
- **ZeroTier libzt** — license unclear (NOASSERTION), C++ (plan §1).
- **Nebula / Yggdrasil / NetBird** — separate Go daemons, not embeddable (plan §1).
- **magic-wormhole.rs** — a one-shot transfer, usable only for a pairing step (plan §1).
- **Hyperswarm / HyperDHT** — JavaScript (plan §1).
- **rustp2p-quic** — defaults to skipped certificate verification, unsafe for peer authentication (plan §1, per review, not re-checked).
- **One QUIC stream per frame** — QUIC streams are not ordered with each other, and PeerWire, like the shell wires, uses no transport stream layer (ADR-0310 decision 16), so ordering must come from one stream.
- **0-RTT on dial** — 0-RTT data is replayable, and a replayed `Mail` would be delivered twice.
- **Keeping our own supervisor ping / idle, aligned with iroh's timers** (Claude's first proposal; the plan §2 kept the supervisor ping) — user rejected directly: 「우리 감지를 빼면 안됨? iroh 피드백 받으면 되는거 아님?」. QUIC already heartbeats every 5 s; a daemon that hangs while its QUIC stack answers is covered by the `Mail` deadlines (7 s → unconfirmed, 120 s late window).
- **One short request deadline for `Mail` (the registry's shared one)** — transport drops a reply that arrives after its request's deadline (`peer.rs` `route`, ADR-0181 decision 4), so the later outcome notice for an unconfirmed send could never fire.
- **The smaller ID's dial always wins** (the plan's Reason Network rule without a time window) — a stale link left by an abrupt restart of the other side could beat the fresh one; the 5 s window is Reason Network's `DIALED_TOGETHER`.
- **Tie-break in transport** — relationship policy is the consumer's (ADR-0177 decision 4); transport only proves two concurrent links stay distinct (TRD §9 P8).

## 근거
- Plan candidate survey (`gh api`, READMEs, iroh v1 blog, 2026-11-11): iroh is the only candidate that is account- and install-free, key-authenticated and encrypted, with hole punching and a relay fallback, in Rust; v1 is wire-stable within v1 (「An iroh v1 endpoint will be able to communicate with another iroh v1 endpoint, regardless of minor version or language」).
- Weight, measured in scratch projects (plan §3): +~172 crates against the daemon graph; standalone probes 6.78 MB (iroh) vs 1.25 MB (WS) under the repo release profile; all static (`dumpbin /dependents` — only Windows system DLLs). The real daemon exe delta is unmeasured.
- Seam fit (plan §2, code read): one connection with one bi-stream implements `Dialer::dial`, `Listener::accept`, `LinkTx`, `LinkRx` without changing trait shapes.
- QUIC liveness values: heartbeat 5 s, path idle 15 s direct / 30 s relay (`iroh-1.4.1` `socket.rs` `HEARTBEAT_INTERVAL` · `PATH_MAX_IDLE_TIMEOUT` · `RELAY_PATH_MAX_IDLE_TIMEOUT`); Reason Network leaves dead links to the QUIC idle-out and gates before `accept_bi`.
- S37 PRD+TRD review, three reviewers, PASS after 4 rounds (2026-11-20).

## 영향 / 불변식
- No iroh type crosses the transport module boundary and the daemon has no iroh dependency; otherwise the adapter stops being replaceable (PRD §6 vendor risk — iroh is maintained by one vendor).
- Correctness never depends on the final `Reject{last}` frame (QUIC `CONNECTION_CLOSE` can discard unread data).
- `into_0rtt` stays unused on the dial side.
- A peer-sent code 0 is never a terminal rejection (`RejectKind::of` maps 0 explicitly); a local idle-out is `Err(LinkError)` — `link.rs` reserves `Closed` for what the peer said.
- `Policy` defaults stay on for every other consumer; turning ping / idle off is the PeerWire registry's choice only.
- Gates: transport gate ④ gains the `iroh` feature combination, gate ⑤ gains the real-socket suite `iroh_link`; `ci.yml` transport gate 4d becomes `--features ws,iroh` once the daemon enables `iroh`.
- Cost accepted: crate count, build time and exe size grow; corporate networks blocking UDP or the relay domain may stop home ↔ office links (PRD §6).
