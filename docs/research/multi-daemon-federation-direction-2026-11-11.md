# Multi-daemon / federation direction — GPT draft checked against the codebase + conventions (2026-11-11)

- **Status:** reviewed (codex FIX → applied). No decision yet; feeds the multi-daemon PRD.
- **Method:** `/engram-research` medium, design-decision mode. Input = a user-supplied GPT draft (written from the public README only). Collectors: 1 internal (code + ADRs, Claude scout) and 3 web (peers · identity/pairing/network · reliability/protocols, Claude scout-sonnet), plus 1 source read of two cloned references (§8). Main session grounded load-bearing claims; cross-family review = codex (§10).
- **Confidence legend:** certain = grounded + independent second source or direct code read · likely = grounded single source · uncertain = snippet-only / not grounded · *inference* = main-session reasoning mapped onto Engram (one step lower than facts; not source-backed).

## 0. Bottom line

1. **The GPT draft's direction is sound and mostly matches both conventions and our own records** — own protocol over the existing transport, a separate remote endpoint, key-based daemon identity with explicit pairing, at-least-once + receiver dedup with a persistent outbox, mail first and no remote command execution in v1, A2A/NATS/Dapr deferred. (likely — §3–§6)
2. **It is missing half of the plan we already decided.** ADR-0179 / ADR-0310 L9 anticipate two relationships: (a) **one shell attaching to N daemons** (the app shows and drives agents on another PC's daemon) and (b) **daemon↔daemon** (agents mail each other across PCs). GPT only has (b). (certain — ADR-0179 L18–21, ADR-0310 L9/L29/L33)
3. **Several current-state claims are wrong**: there is no persistent daemon id today (ADR-0310 D3 decided it, 3-3 U7 was cut), no `Principal`/`admit` authorization layer, messaging is entirely in-memory (lost on daemon restart), there is no database in the workspace, and the WS listener refuses non-loopback binds with TLS off. (certain — code read §2)
4. **Most surveyed AI-agent tools cross machines through a relay/cloud server or SSH — but hub-less daemon-to-daemon peering exists.** Reason Network (Rust, MIT/Apache-2.0, ~1.7k★) runs one daemon per machine over iroh (direct P2P, relay fallback), persists received messages before acking (`queued | delivered | held | refused | retry` on the wire), and keeps a 7-day sender outbox — for Claude Code and Codex. agentbus (Rust, 1★) is a smaller one. Reason Network is the closest precedent for relationship (b). (likely — DESIGN.md / README read; found by the cross-family review, missed by the first survey)
5. **Two defensible link choices for a personal two-PC setup** (*inference* from §4/§8; cost not measured): (i) pinned self-signed keys (Syncthing model) over TLS on a separate listener, reachability delegated to the user's VPN (Tailscale/WireGuard) or SSH tunnel — smallest dependency, user sets up the network; (ii) iroh as the link — key identity + NAT traversal in one crate, proven for coding agents by Reason Network — bigger dependency, public relays unless self-hosted. Both fit the transport `Dialer`/`Listener` seam.

## 1. Two relationships — which one first is the first PRD decision

| | (a) shell → N daemons | (b) daemon ↔ daemon |
|---|---|---|
| What the user sees | the app lists/drives agents of the office PC's daemon, as if local | an agent at home sends mail to an agent at the office; each PC's app shows only its own agents |
| Our records | ADR-0179 decision 2 ("the one that attaches is the shell"); ADR-0029 ("remote daemon + WSS"); ADR-0310 D4 (`primary()` main daemon, events keyed `(DaemonKey, AgentId)`) | ADR-0310 L9 user instruction "daemons talk to each other"; D5 ③ deferred (`PeerWire`, `peer` vocabulary, `link/peer.rs`, second Listener) |
| Built so far | transport N-peer `Registry`, `Registry::attach`; **shell N-daemon seat (3-2 U6) not built** — `connection.rs:380` builds one `DaemonKey` + one `Registry` | nothing (no `link/peer.rs`) |
| Peer precedent | common: Happier, AgentsMesh, Codex (multi-host), Zed remote, Docker contexts | rare: hcom (MQTT relay hub), Claude Relay (central WS server), mcp_agent_mail (shared git / HTTP server) |
| Shared prerequisite | daemon identity + pairing + remote-capable link (TLS/pinning) | same |

(certain for the records/code rows; likely for the precedent row — §3)

*Inference:* both share the same foundation (identity, pairing, secure remote link). The order changes what is visible first: (a) gives a working "see the other PC" result with no new message semantics; (b) needs remote addressing, ack semantics and (eventually) persistence before it is useful.

## 2. GPT draft vs code (internal branch — certain unless marked)

| GPT claim | Verdict | Evidence |
|---|---|---|
| Claude/Codex, PTY, stdio JSON, standalone daemon, multi-agent, mail, CLI, command bus, local WS | correct | `agent/src/backend/*`, `agent/src/transport/{pty,stdio}.rs`, `daemon/src/main.rs`, `agent/src/manager.rs`, `crates/engram-dashboard-messaging`, `crates/engram-dashboard-cli`, `crates/engram-dashboard-command` |
| Messaging = Mailbox, Delivery, Ledger | partial | `mailbox.rs`, `ledger.rs`, `groups.rs`, `envelope.rs`, `busy.rs`, `service.rs`; "Delivery" is the `DeliveryPort` trait (`service.rs:311`) |
| Transport = WebSocket abstraction | partial | carrier-neutral lib (generic `Wire`, N-peer `Registry`, reconnect/backoff, keepalive, request correlation, `Dialer`/`Listener`/`Handshake` seams); WS is an optional adapter (`transport/src/ws.rs`) |
| Prompt components "in development" | correct | S25 preset library, PRD/TRD drafts on `v0.5.0/feat/preset-library` |
| Persistent `daemon_id` (proposed as new) | needed — **does not exist** | ADR-0310 D3 decided `DaemonId`; step-log L2924: U7 left out of 3-3; `Hello` (`protocol/src/messages.rs:457`) has no `daemon_id`; stale doc comment `src-tauri/src/daemon_client/link/mod.rs:37` |
| "Don't reuse local WS auth for remote" | correct for remote; stricter than our same-PC premise | TRD 3 L268–273: same-PC `PeerDaemon` reuses the receiving daemon's `daemon.json` token at `LocalClient` trust; remote needs pairing |
| Separate federation endpoint | already decided | ADR-0310 D2: each relationship gets its own Wire · Registry · Listener |
| SQLite for outbox/inbox | no DB today | no sqlite/sled/redb in any `Cargo.toml`; history: ADR-0086 SQLite mailbox → ADR-0087 deferred → ADR-0103 in-memory; `ledger.rs:131` expects "v2 persistence (SQLite)" |
| Local link is loopback | correct, and enforced | `transport/src/ws.rs:538` `ensure_loopback`; TLS off (`ws.rs:62`, `wss://` fails) |
| Message router in MessagingService | seam already named | ADR-0309 decision 3: "「여기 없으면 다른 데몬으로」 gets inserted into that one place" = `resolve_addressing` → `resolve_token` (`service.rs:2560/2666`); unknown target today = `RECIPIENT_NOT_FOUND` |

Facts GPT could not know (certain):
- Messaging today: mailbox TTL 24h (`mailbox.rs:60`), 100 per recipient + 64 notices, ledger ring 4096 — all lost on daemon restart.
- `DeliveryPort::inject` is synchronous and its success means "the recipient receives and starts processing"; a remote hop needs its own ack meaning (matches GPT's transport-ack vs delivery-ack split).
- Authorization today = one 256-bit token in `daemon.json` with full authority (shell WS), plus a per-(agent, epoch) Bearer token on the control plane (`127.0.0.1:0`).
- Prior research that already pointed the same way: `agent-messaging-survey-2026-06-28.md` ("remote/persistent target = NATS JetStream", "A2A = external boundary only"); `orchestration-survey-2026-07-12.md` (local task journal; borrow only A2A's task state machine); `transport-security-build-vs-adopt-2026-10-12.md` (borrow encryption at the `Dialer`/`Link` seam; pairing/Origin/authz are ours); `client-identity-survey-2026-08-17.md` (left open: "write the threat model first").

## 3. Peer products — who crosses machines and how (branch 1)

| Product | What crosses machines | Topology | Identity / auth | Offline queue | Confidence |
|---|---|---|---|---|---|
| Happier (happier-dev/happier, MIT, ~2.2k★) | UI → agents on several machines | relay server, E2E encrypted; "add a machine over SSH" | account + E2E keys | unknown | likely |
| hcom (aannoo/hcom, MIT, ~650★) | agent ↔ agent messages across machines | MQTT broker relay; `hcom relay new` → token, `hcom relay connect <token>`; local SQLite per machine | join token = relay id + raw PSK + broker; public brokers by default | per-device retained snapshot + backfill from an online sender; no broker queue | certain (source read §8) |
| AgentsMesh (BSL 1.1, ~2.4k★ per aggregator) | UI → remote dev envs | gRPC + mTLS control, WS relay for terminal I/O | mTLS | unknown | likely; license not OSI-open |
| Gas Town (steveyegge/gastown, MIT, ~18.7k★) | local multi-agent; "Wasteland" task-level federation | not verified (source page 403) | — | — | uncertain |
| Claude Code Remote Control (Anthropic) | phone/web → local session | outbound HTTPS to Anthropic; transcript stored server-side over TLS | account | server-side transcript; reconnect gives up after ~10 min in server mode | likely |
| Codex remote / SSH hosts | UI → remote host | SSH; "Codex Remote" seen only on a third-party blog | SSH keys | — | uncertain |
| RelayRoom (named by GPT) | — | **not found** (possible mix-up with AgentsRoom) | — | — | abstain |

| Reason Network (reason-machines/reason-network, MIT OR Apache-2.0, ~1.7k★) | agent ↔ agent messages across machines (Claude Code, Codex, others) | one Rust daemon per machine; iroh 1.4 direct P2P with hole punching, HTTPS relay fallback (own regional relays by default, custom relays supported) | iroh node keys (details: §8) | receiver writes to SQLite then acks `queued/delivered/held/refused/retry`; sender outbox (7 d) resends every 30 s and on reconnect; no remote read receipts on the wire (DESIGN.md's `stored`/`read` are not in `Wire`) | certain (source read §8) |
| agentbus (luqs1/agentbus, Rust, 1★) | agent messages across devices | "No central hub: every device runs its own small daemon"; Tailscale within a personal tailnet, iroh across users | Tailscale identity / iroh | unknown | likely (README) |

- Pattern within the surveyed sample (likely): **relay/cloud server with outbound-only host connections** is most common; **SSH** is second; **one UI → N hosts** is common; **hub-less per-machine daemons** exist (Reason Network, agentbus) — optional transport relays there carry encrypted traffic, not a message broker.
- Not checked: claude-squad, Conductor, Crystal, Sculptor, OpenHands, VS Code tunnels, Agent Farm.
- *Inference:* "multi-machine agent mail without a hub" is not unique to Engram (Reason Network already does it); the differentiation is the native app + PTY/structured runtime + both relationships (a) and (b) on one transport. Upside: real prior art to copy for (b).

## 4. Identity, pairing, reachability (branch 2)

- **Shared pattern** (likely; Syncthing docs, iroh docs, Tailscale docs): identity = a public key or a hash of a self-signed cert; first trust = an explicit human step (exchange IDs, or accept a prompt); the channel then pins the peer key.
- **Syncthing = closest template** (likely; docs.syncthing.net device-ids/security): self-signed Ed25519 cert; Device ID = SHA-256 of the cert DER, base32 with check digits; a connection is accepted only if the remote ID is in the local allowlist; each side adds the other; relays/discovery are optional and see device IDs but not data. Source-level detail in §8.
- **Reachability families** (likely): (a) overlay VPN — Tailscale (STUN/ICE + DERP relay fallback), WireGuard; KDE Connect docs endorse VPN + manual IP; (b) built-in relay + hole punching — Syncthing relays, iroh, Magic Wormhole transit, VS Code tunnels (Microsoft relay); (c) SSH or mTLS — Docker's recommendation for exposing its daemon.
- **Exposing a daemon on the network** (likely): Docker — not exposed by default; if exposed, SSH or mTLS, never plain TCP. Syncthing GUI — localhost-only by default. Jupyter — token on by default (snippet only, original not checked). *Inference:* a bare bearer token on `0.0.0.0` without TLS is discouraged; acceptable only inside an encrypted overlay or with TLS + pinning.
- **Rust options** (likely, versions from crate pages): rustls 0.23.x (Apache-2.0/ISC/MIT; custom certificate verifier for pinning); snow (Noise; "has not received any formal audit"); iroh 1.4.x (MIT/Apache-2.0; QUIC P2P with hole punching + relay fallback; the `N0` preset uses Number 0's public relays automatically, but n0's deployment guidance discourages public relays for production and offers shared/dedicated relays — refutes the earlier "must be configured" claim); Tailscale embedding in Rust is weak — `tsnet` crate 0.1.0 (2023, wraps libtailscale), official tailscale-rs experimental (snippet only).
- **Pairing UX options:** paste the other side's ID on both sides (Syncthing) · accept prompt for an unknown device (KDE Connect, Syncthing pending device) · short code with SPAKE2 (Magic Wormhole) for the pairing step only.

## 5. Reliability of cross-PC mail (branch 3)

- **GPT's core is the documented standard** (certain — multiple primary sources): transactional outbox + at-least-once + receiver dedup by message id (microservices.io outbox, RabbitMQ publisher confirms, MQTT QoS 1, SMTP). End-to-end exactly-once is not achievable; MQTT QoS 2 and JetStream claim exactly-once only per hop / within a dedup window.
- **Ack = "receiver took responsibility", not "processed"** (likely): SMTP 250 is a formal handoff of responsibility (RFC 5321); XMPP XEP-0198 "handled"; RabbitMQ confirms persistent messages routed to durable queues after persisting — but it also confirms unroutable messages, so "ack = persisted" is not universal; each system defines the contract. *Inference:* Engram should define "accepted = persisted" explicitly (Reason Network's `stored` ack is that contract). A lost ack causes a resend → duplicates are expected.
- **Numbers** (with confidence): SMTP retry interval ≥ 30 min, give up after ≥ 4–5 days (RFC 5321 — likely); JetStream default dedup window 2 min (uncertain — snippet only); Matrix federation transaction ≤ 50 PDUs / 100 EDUs, the same txnId retried until 200 before sending another (likely). No sourced backoff defaults for MQTT/XMPP/Matrix (abstain).
- *Inference:* if retries can run for days, a time-window dedup is not enough — keep a persistent processed-id table with TTL ≥ the outbox expiry horizon.
- **Storage** (likely): SQLite via rusqlite (`bundled` feature for Windows, WAL = one writer) fits outbox queries (status, next_retry_at); redb (stable, ACID, MIT/Apache) and fjall 3.x are alternatives; sled is still beta — avoid. Desktop-daemon outbox precedent: Reason Network (§8).
- **Durability boundary must be stated before "persisted" means anything** (likely — sqlite.org/wal.html): in WAL mode `synchronous=NORMAL` can lose recently committed transactions on power loss; `FULL` does not. The PRD/TRD must name the failure model (process crash vs power loss), the transaction that precedes the ack, and the sync policy.
- *Inference — scope question for the PRD:* persist only the cross-PC outbox/inbox, or make all mail persistent (the deferred ADR-0086 direction)? The former is smaller; the latter fixes "mail lost on daemon restart" locally too.

## 6. Protocol landscape (branch 3)

- **A2A** (likely unless marked): Linux Foundation project (donated June 2025), Apache-2.0; the spec page banner shows 1.0.0, while the GitHub release history also has v1.0.1 (per review — not re-checked); AAIF (LF Agentic AI Foundation) now hosts A2A per an AAIF article of 2026-08-20 (per review — not re-checked); 1.0 release month March 2026 — uncertain, snippet only; bindings JSON-RPC / gRPC / HTTP+JSON; Agent Cards; a Rust SDK exists. IBM ACP merged into A2A (Aug 2025 — uncertain, snippet only).
- A2A task states: SUBMITTED, WORKING, INPUT_REQUIRED, AUTH_REQUIRED, and terminal COMPLETED / FAILED / CANCELED / REJECTED (likely). Results go in Artifacts; Send Message "MAY be idempotent" via messageId → **A2A does not provide delivery reliability**; the outbox layer is still ours (likely).
- MCP = agent↔tools, A2A = agent↔agent; Anthropic donated MCP to the LF Agentic AI Foundation; no explicit Anthropic position on A2A found (abstain).
- Dapr Agents, LangGraph, CrewAI are Python (LangGraph also JS) orchestration frameworks — not applicable to a Rust daemon managing CLI agents; at most remote A2A peers (likely).
- *Inference:* reuse A2A's task state names for phase 4 internally (cheap, keeps a later A2A adapter easy); "internal vocabulary without compliance" as an established practice was not found (*hypothesis*).

## 7. Constraint fit (design-decision mode)

Constraints (from CLAUDE.md, ADRs — certain): daemon owns agents, federation owns none · agent crate knows neither Tauri nor transport (ADR-0003) · each relationship gets its own Wire/Registry/Listener, no union Wire (ADR-0310 D2, rejected alt ②) · `DaemonId` is claimed/unverified — no persistent trust keyed on it until bound to a key by pairing (ADR-0310 D3) · plain `ws://` stays loopback-only · encryption/peer-auth borrowed from the link layer, authz/pairing/Origin/caps are ours (ADR-0310 L9) · messaging kernel has zero workspace deps, ports only (ADR-0110); remote branch only at the single resolution point (ADR-0309 d3) · LLM path on existing surfaces for every new UI (ADR-0055/0307) · OS branches only in `platform` (ADR-0266) · base `file` rules for new data files (ADR-0291).

**Remote link security**

| Candidate | Fit | Note |
|---|---|---|
| TLS (rustls) + pinned self-signed Ed25519 keys, separate listener | ✅ recommended (*inference*) | matches ADR-0310 split; Syncthing model; works over LAN, VPN or SSH tunnel |
| iroh (QUIC P2P + relay) as an alternative `Dialer`/`Listener` | ◐ strong alternative | peer auth (node keys) + reachability without a user VPN in one crate; Reason Network ships exactly this for coding agents; cost = a large dependency and public-relay reliance unless self-hosted. Strongest counter-argument to the TLS-over-VPN default |
| Embed Tailscale | ✕ now | Rust embedding immature (§4); user-installed Tailscale works without embedding |
| Plain WS + `daemon.json` token on `0.0.0.0` | ✕ reject | discouraged by Docker/Syncthing practice; violates loopback-only rule |
| Central relay server (Happier/Remote Control style) | ✕ reject for v1 | contradicts "no mandatory master"; needs hosted infra for a personal tool |

**Message transport / broker**

| Candidate | Fit | Note |
|---|---|---|
| Own envelope over transport `Wire` (daemon↔daemon relationship) | ✅ recommended | reuses N-peer registry, reconnect, correlation; ADR-0310 D5 ③ already sketched `PeerWire` |
| MQTT broker (hcom style) | ✕ reject | requires a hub process; adds a dependency the transport crate already covers |
| NATS JetStream | ◐ later | prior survey's "remote/persistent future target"; a broker is overkill for two PCs |
| A2A | ◐ adapter later | external boundary only; no delivery reliability |

**Persistent store**

| Candidate | Fit | Note |
|---|---|---|
| SQLite (rusqlite, bundled) | ✅ recommended | query-shaped outbox; prior ADR-0086 intent; ledger comment expects it |
| redb / fjall | ◐ | fine KV stores; outbox queries are less natural |
| sled | ✕ reject | still beta per its README |

Rejected candidates for ADR "rejected alternatives": plain token on 0.0.0.0, central relay server, embedded Tailscale, MQTT broker, sled, Dapr/LangGraph/CrewAI.

## 8. Reference clones (source read)

Clones (shallow, depth 1), registered in `reference-implementations.md`: `I:\Engram_Workspace\opensource\reason-network` @ `5442c8b5` (MIT OR Apache-2.0) · `I:\Engram_Workspace\opensource\syncthing` @ `ed2e0fd4` (MPL-2.0) · `I:\Engram_Workspace\opensource\hcom` @ `de33e31a` (MIT). Not cloned on purpose: iroh (only if NAT traversal is needed), Happier (cloud relay premise), AgentsMesh (BSL), Gas Town (task-level, phase 4 at most), mcp_agent_mail (maintenance mode). Code facts below are direct source reads (certain unless marked).

**Reason Network — closest precedent for relationship (b); follow for message semantics and policy**
- One crate / one binary, ~27.5k lines of Rust. Identity = one iroh secret key per machine (`~/.reason/network/machine.key`); its ed25519 `EndpointId` is the machine identity. QUIC authenticates every connection by that key; messages are not signed — the receiver checks a peer speaks only for its own agents (`daemon/mail.rs` `receive_to`: "cannot send as").
- Trust: a signed roster replicated to every machine (`team.rs` `TeamState`, `Signed<T>`); join via a reusable team link (`Ticket`) or a one-time 4-word code (~41 bits) on a separate join protocol `tincan/join/1`; non-members are closed with 403 at accept (`PeerProto::accept`); any member can remove a machine, but removal only blocks connections (the code calls it "for friends").
- Remote permissions = messaging only: per-person inbound policy `accept | hold | refuse` (`config.rs` `Policy`), 60 msgs/min per machine, loop guard 12 msgs per agent pair per 10 min, agents may not message groups. No remote command execution / task delegation found.
- Reachability: direct → LAN mDNS → hole punching → relay fallback; default relays = Reason's regional relays from a signed list (`relays.rs`), self-hosted `iroh-relay` supported; full mesh, documented as "fine to ~50 machines".
- Message: `proto.rs` `Message{id (ULID), from, from_agent, from_repo, to, via, reply_to, body ≤64KB, wake, notify, sent_at, approval, files}`; addresses `person/session`, `person/session@machine`, bare unique session name, groups (people only).
- Ack verdicts (`net.rs` `Wire::Ack{id,state,note,to?}`): `queued | delivered | held | refused | retry`; `retry` = rate/file quota full → sender keeps it in the outbox; `refused` = policy, unknown session, loop guard, agent→group. Sender waits 5 s for an ack, else falls back to the outbox.
- Persistence: SQLite via rusqlite 0.40 (bundled), `journal_mode = WAL` (`store.rs:79`), `synchronous` never set (SQLite default). Receiver writes before acking. `outbox(id, target, msg, created_at)` kept 7 days; resend every 30 s for entries older than 20 s and on reconnect; redial backoff 2–60 s; dedup by sender ULID (`messages.id` PK); `send_keys` makes local sends idempotent.
- Agent integration (no PTY injection): Claude Code — its MCP child posts a `{"type":"user"}` frame to Claude Code's own `CLAUDE_CODE_MESSAGING_SOCKET` (`agent.rs:441`); Codex — hooks mid-turn (PostToolUse `additionalContext`, Stop `decision:block`), `codex queue --thread` or app-server `turn/start` when idle.
- Follow: identity = public key, authorization at the authenticated connection + "a peer speaks only for its own agents" check · receiver persists then acks with a verdict that separates `retry` (keep in outbox) from `refused` (drop) — maps onto `DeliveryPort::inject` results and transport request correlation · sender ULID + receiver dedup + bounded outbox expiry, duplicate → reply with the original outcome · gate non-members at accept, join on a separate protocol · per-peer inbound policy + rate limit + per-pair loop guard at our single resolution point · frame peer text as peer input, never as the user.
- Don't follow (*inference* where marked): iroh + third-party relays as a hard dependency when we already own a transport crate (put any NAT traversal behind our `Dialer`/`Listener`) · full-mesh presence broadcast replicating repo/branch/activity to every peer (heavy and leaky for a few own PCs) · revocation without cryptographic effect + a reusable team secret every member holds — our daemons drive agents, so removal must stop authentication · hook/MCP-based delivery sprawl — Engram owns the PTY/stdio and injects directly.
- Side finding (not about federation): Claude Code exposes `CLAUDE_CODE_MESSAGING_SOCKET` for injecting user frames into a running session — possibly relevant to Engram's own input path; not investigated.

**Syncthing — follow for identity and pairing**
- Device ID = `sha256.Sum256(rawCert)` (`lib/protocol/deviceid.go` `NewDeviceID`).
- Checks: incoming — exactly one cert, reject self, drop ignored devices (`lib/connections/service.go` `handleConns` / `connectionCheckEarly`); outgoing — `validateIdentity` compares against the expected ID; allowlist — `lib/model/model.go` `OnHello` checks `cfg.Device(remoteID)`. TLS runs with `InsecureSkipVerify=true` + `RequestClientCert`, so the ID hash is the only pin.
- Unknown device → `AddOrUpdatePendingDevice` (`internal/db/observed.go`) + `PendingDevicesChanged` event → user adds it to config (accept) or dismisses / ignores it.
- Cert: self-signed Ed25519, 20-year lifetime (`lib/tlsutil/tlsutil.go` `generateCertificate`).
- Relayed connections run a fresh TLS session over the relay stream, so the same ID check applies; discovered addresses are dial hints only (discovery-server binding not read — uncertain).
- Follow: per-device key · mutual allowlist · pending-device accept prompt · identity check on every path (direct, relayed). Rust mirror ≈ rcgen Ed25519 cert + rustls custom verifiers on both sides (accept any cert, still verify the handshake signature) + hash the peer key and look it up in allowlist / pending table (*inference*: ~200 lines).
- Don't follow: the full BEP / discovery / relay-pool stack; a 63-char ID as the only pairing path; hashing the whole cert (*inference*: hash the public key instead so the cert can be reissued without re-pairing).

**hcom — follow for cross-machine message semantics, not for trust**
- Rust (rumqttc, rustls, chacha20poly1305).
- `relay new` defaults to **public shared brokers** (`src/relay/mod.rs` `DEFAULT_BROKERS` = `broker.emqx.io:8883`, `broker.hivemq.com:8883`, `test.mosquitto.org:8886`); `--broker` / `--password` for your own.
- Payload sealed end-to-end with XChaCha20-Poly1305 (`src/relay/crypto.rs` `seal`/`open`): random 24-byte nonce, AAD = `relay_id‖0‖topic‖0‖ts_be`, key = the raw 32-byte PSK (no KDF, no forward secrecy).
- Join token (`src/relay/token.rs` `encode_join_token`) = relay UUID + PSK + broker; bearer secret with no expiry / scope / revocation (its SECURITY.md says so).
- No broker-side queue (`set_clean_start(true)`). Each machine keeps its own SQLite; each device publishes a retained QoS1 snapshot of its last 50 events (≤ 96 KiB); `backfill.rs` detects gaps and asks the sender — recovery only while that peer is online. Dedup = per-peer import cursor + nonce LRU (8192, 600 s) + 60 s clock-skew window on control messages. No per-message recipient ack; no expiry found.
- Remote agent address = `name:DEVICE` (4-letter tag hashed from the device UUID, `device_short_id`); `pull.rs` `localize_name` keeps local names bare.
- Follow: sealed envelope with AAD binding · nonce/replay window · per-peer cursor + gap backfill · `name:DEVICE`-style addressing with local names bare (fits ADR-0309 d2 "edge addresses stay names").
- Don't follow: one shared PSK as the whole trust model · a non-expiring token carrying the raw key · public brokers by default · every member able to start/kill agents on every machine.
- Limit shared with any P2P design (no store-and-forward hub): undelivered data cannot move while its sender is offline. A sender-side outbox only keeps the retry duty across restarts (Reason Network: "retries when the target machine comes back"); delivery still needs both daemons online at the same time (*inference* from both designs).

## 9. Open decisions for the user (visible behaviour)

1. Which relationship ships first — (a) the app shows/drives the other PC's agents, or (b) agents on two PCs mail each other.
2. How PCs reach each other in v1 — rely on the user's VPN/SSH tunnel (no NAT traversal of our own) vs build built-in traversal.
3. Pairing UX — paste the other side's ID on both PCs vs accept prompt on first contact (+ optional short code).
4. Mail persistence scope — cross-PC only vs all mail (fixes loss on daemon restart).
Deferred items that join this topic: T-85 (who starts a daemon), T-86 (tray icon after tray start), 3-2 U6 (N-daemon seat), U7 (`DaemonId`), D6 (`Principal`/`admit`), the LLM `daemon.*` surface.

## 10. Review result

codex (GPT family, effort high, independence level 2–3) → **FIX**, 6 findings; all applied after the main session re-checked the counter-evidence:
1. high · omission — hub-less agent peering exists (Reason Network DESIGN.md, agentbus README) → §0 item 4, §3 rows/pattern, §7 iroh row, §8. Re-checked: both pages fetched; quotes confirmed.
2. med · refuted — iroh relays "must be configured" → `N0` preset uses n0 public relays (docs.rs iroh 1.4.1, re-checked) → §4.
3. med · refuted — "ack = persisted" as universal → RabbitMQ confirms unroutable messages (rabbitmq.com/docs/confirms; not re-fetched) → §5 reworded.
4. med · refuted — "sender-side outbox fixes sender-offline catch-up" → logic gap → §8 reworded.
5. low · omission — A2A v1.0.1 release and AAIF governance → §6 (marked "per review — not re-checked").
6. med · omission — SQLite WAL durability boundary (sqlite.org/wal.html) → §5.
Reviewer spot re-verification: ADR-0179/0310 lines, step-log L2923–2924, `connection.rs:380`, `messages.rs:457`, `ws.rs:62/457/538`, `service.rs:311/2666`, mailbox/ledger limits, hcom `DEFAULT_BROKERS`/crypto, Syncthing `deviceid.go`/`OnHello`, RFC 5321 numbers, A2A task states — all supported. §7 recommendations: "partly supported as labelled inference; 'cheapest' is unmeasured".

## 11. Gaps / limits

- Peers not checked: claude-squad, Conductor, Crystal, Sculptor, OpenHands, VS Code tunnels, Agent Farm; Gas Town Wasteland mechanism unverified.
- Not fetched: SSH TOFU docs, libp2p, Zed collab internals, Headscale, kubectl contexts, KDE Connect protocol details.
- Snippet-only: A2A 1.0 release month, ACP merge date, JetStream dedup default, tailscale-rs status, Jupyter token default.
- No sourced retry/backoff defaults for MQTT/XMPP/Matrix/JetStream/RabbitMQ.
- Fetch tool paraphrases pages — quoted phrases from web sources may be loose; code quotes are direct.
