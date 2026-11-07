# Transport security — keep our own transport vs adopt or wrap an off-the-shelf library (2026-10-12)

- Status: **adversarial review FIX applied (re-decision not run)** · no decision (waiting for the user's choice)
- Method: `/research` medium · design-decision mode · four collection tracks (general-program peers · app-level Rust RPC/WS libraries · secure channel/remote layers · measuring our current code) → main session synthesis + source cross-check (grounding) → cross-family adversarial review (codex)
- Trigger: user question — "if there is a library with good security, then we don't need to implement it ourselves, do we (or wrap it and swap it if there is a problem)". Once external communication works, what must be considered doubles.
- Comparison basis: **general programs**, not agent products (user instruction 2026-10-12). Agent and terminal peers are the closest in shape, so they are kept for reference only.
- Confidence legend: **certain (확실)** (source cross-check + independent corroboration) · **likely (가능성 높음)** (source cross-check, single source) · **uncertain (불확실)** (not cross-checked · from memory · third-party aggregation) · **contested** (counter-evidence exists)
- Earlier survey: `docs/research/reusable-transport-crate-boundary-2026-09-03.md` (the connection lifecycle management axis — this document revisits only the **security axis**) · ADR-0177

## 0. Conclusion first

1. **There is no library that does security as a whole.** Security has four layers and each layer has a different answer — so the question is not "pick one library" but **what to borrow and what we do ourselves, layer by layer**. `가능성 높음`
2. **The layer where borrowing clearly pays off = encryption · remote (③).** Most general programs leave the remote segment to an **existing secure channel** (SSH · mutual TLS/certificate pinning · end-to-end encrypted relay · an overlay such as Tailscale). We found no one who built it themselves. The `Dialer`/`Link` seam of our transport is exactly that place, so **wrap it and swap it if there is a problem** is already structurally possible. `가능성 높음`
3. **The layers where, even if we borrow, we still have to turn it on and decide = defence against local attacks from a browser (①) · authentication (②) · pairing.** There was no library that **turns Origin checking on by default** — some, like Salvo, offer an allow-list option but it is off by default (found by the adversarial review · main session cross-check). A localhost WS with no Origin check is a **failure that produced real CVEs repeatedly** in this class. But those CVEs were **unauthenticated** endpoints, and we have a first-frame token, so it is not the same path — Origin is a **double defence** for us. Our `net` already blocks it, but **the transport's `WsListener` does not** — swapping it in as is would lose that one layer. `확실`
4. **DoS · resources (④) is half and half.** Libraries give a frame size and a deadline at most, and connection count caps and rate limits are usually the user's job. There is a hole measured on our side (§4). `가능성 높음`
5. **The option of adopting an app-level RPC framework (jsonrpsee · tonic · tarpc and others) wholesale gains little** — the wire format must change (ADR-0177 decision 5), and yet authentication · Origin are still ours. `가능성 높음`

## 1. Map by layer — what to borrow and what to do

| Layer | What it blocks | General program practice | Can we borrow? | Our status |
|---|---|---|---|---|
| ① Local attack from a browser | a malicious web page the user opened connects to the `127.0.0.1` port (Origin) · DNS rebinding (Host) | Origin/Host allow-list · SameSite cookie. Where it was not done, a CVE | **△** — only a hook or an option that is off by default (Salvo) · we decide the values | `net` = has an Origin allow-list (no Host check) · transport = none |
| ② Authentication | someone else who reached the port sends commands | the server generates a random secret · constant-time comparison · a new device is paired | **✕** — we write it ourselves (we borrow only raw parts) | 256-bit CSPRNG token · first frame 1 second · constant-time comparison |
| ③ Encryption · remote | network eavesdropping · man in the middle · connecting across NAT | **leave it to an existing secure channel** (SSH · mTLS/certificate pinning · E2E relay · Tailscale) | **○** — this layer is where we borrow | none (plaintext `ws://`) — not a problem now since it is loopback only |
| ④ DoS · resources | connection floods · huge frames · slow peers | frame cap · deadline · connection cap | **△** — only part | the holes in §4 |
| (Pairing) | the procedure for trusting a new device the first time | QR · pairing URL · device ID approval | **✕** | none |

## 2. How general programs do it (peers)

### 2-1. General programs

| Program | Local authentication | Remote segment | Local hardening | Source · confidence |
|---|---|---|---|---|
| Syncthing | device ID = certificate SHA-256, checked against an approved list | end-to-end TLS — **the relay sees only ciphertext** | the GUI is localhost by default | docs.syncthing.net/users/security — `확실` (main session cross-check) |
| Docker Engine | socket file permissions | mutual TLS (`--tlsverify`) or `ssh://` | by default only the local socket, TCP off | collector's memory — `불확실` |
| VS Code Remote Tunnels | account authentication (GitHub/MS) | vendor tunnel + inner SSH | — | secondary source — `불확실` |
| code-server | password → cookie | TLS optional · reverse proxy recommended | **Origin check added** in 4.10.1 (CVE-2023-26114) | wiz.io — `확실` (main session cross-check) |
| Jupyter Server | random token → cookie · XSRF | TLS optional · SSH forwarding | Host/Origin check | blog.jupyter.org · CVE-2022-29241 — `가능성 높음` |
| Home Assistant | access token | Nabu Casa relay (SNI routing, TLS end to end) | — | support.nabucasa.com — `가능성 높음` |
| OBS WebSocket | challenge-response (the password itself is not sent) | none | password optional | protocol docs (v4.5) — `가능성 높음` |
| Chrome DevTools | **no authentication** (anyone who reaches the port has full control) | SSH forwarding only | Chrome 111 = rejects an unknown Origin · 136 = ignores the port in the default profile (because of cookie-theft abuse) | developer.chrome.com — `확실` (main session cross-check, the 136 part) |
| Redis | AUTH/ACL | TLS optional (6.0+) | protected mode · `POST`/`Host:` make the connection QUIT (a response to browser cross-protocol attacks) | lwn.net — `가능성 높음` |

### 2-2. Peers of the same shape (reference)

| Peer | Stack | Authentication | Remote | Local hardening | Source |
|---|---|---|---|---|---|
| zellij web | axum + rustls | UUIDv4 token stored as a SHA-256 hash · HttpOnly SameSite=Strict cookie · read-only token | rustls | CSP · SameSite | local clone `zellij-client/src/web_client/` — `가능성 높음` |
| paseo | express + ws | bcrypt password · constant-time comparison | relay + NaCl box end-to-end encryption + pairing | Host allow-list (DNS rebinding) | local clone `packages/server/src/server/auth.ts` — `가능성 높음` |
| t3code | Effect node | one-time expiring pairing → HMAC-signed cookie · DPoP | SSH · Tailscale package | per-port cookie name | local clone `apps/server/src/auth/` — `가능성 높음` |
| orca | Electron own RPC | QR pairing token rotation | relay + tweetnacl box | unknown | local clone `src/shared/e2ee-crypto.ts` — `가능성 높음` |
| herdr | `interprocess` local socket | socket permissions (presumed) | SSH | not exposed to the network | local clone `Cargo.toml` — `가능성 높음` |
| ttyd | libwebsockets | Basic authentication · proxy header | built-in TLS · client certificate | has `--check-origin` but **off by default** | local clone `src/server.c` — `가능성 높음` |

### 2-3. Incidents of this class (localhost WS · exposed ports)

- **CVE-2026-44211** Cline `kanban` — no Origin validation on the `127.0.0.1:3484` WS → a malicious website hijacks the agent terminal. 2.13.0 and below. advisories.gitlab.com — `확실` (main session cross-check)
- **CVE-2023-26114** code-server — no Origin validation in the WS handshake, CVSS 9.3, fixed in 4.10.1. wiz.io — `확실` (main session cross-check)
- **CVE-2025-24964** Vitest — no WS Origin check → RCE by saving and re-running a test file. db.gcve.eu — `가능성 높음`
- **CVE-2026-59723** Cline Hub — `/browser` no Origin check · no room secret when bound to 127.0.0.1. opencve.io — `가능성 높음`
- **CVE-2016-10517** Redis — HTTP sent by a browser is partly interpreted as commands. lwn.net — `가능성 높음`
- Chrome remote debugging port — abused for cookie theft after App-Bound Encryption → restricted in 136. developer.chrome.com — `확실` (main session cross-check)
- Not cross-checked: CVE-2026-77359 · CVE-2026-77318 (DNS rebinding of the Jupyter MCP server) · CVE-2026-86049 (Jupyter token exposed in logs) — `불확실`

## 3. Library candidates

### 3-1. App-level RPC / WS (candidates for layers ② · ④)

| Candidate | Authentication | TLS | Origin/Host | Size · connection · rate caps | Wire fixed? | Maintenance · advisories | License |
|---|---|---|---|---|---|---|---|
| tokio-tungstenite 0.30.0 (used now) | none (callback) | feature | none (directly in a callback) | message 64 MiB · frame 16 MiB default · no connection or rate cap | **no** — arbitrary frames | 2026-07-11 · RUSTSEC-2023-0065 (≤0.20.0, handshake header DoS — an edition already past) | MIT |
| axum 0.8.9 ws | middleware | outside hyper/rustls | not in the WebSocketUpgrade docs | 64 MiB/16 MiB | no | h2 advisory history | MIT (not confirmed) |
| fastwebsockets 0.10.0 | none | no docs | none | no docs | no | 2026-10-05 · fuzzed · passes Autobahn · Windows not measured (docs.rs builds only the default target, so the docs target cannot decide it — adversarial review) | Apache-2.0 |
| Salvo (WebSocketUpgrade) | framework middleware | framework | **`allowed_origins()` · `check_origin()` — off by default** (the docs warn 「검사 없으면 방문한 아무 사이트나 인증된 WS 를 연다」) | not confirmed | no — app messages as they are | not confirmed | not confirmed |
| tower-mcp (reference) | — | — | explicitly has Origin · Host validation + a session cap | session cap | yes (MCP) — for pattern reference | not confirmed | not confirmed |
| jsonrpsee 0.26.1 | not found | not in the server file | not confirmed | connections 100 · body 10 MiB · batch unlimited by default · ping off by default | **yes** — JSON-RPC 2.0 | 2026-09-30 | MIT |
| tonic 0.14.6 | interceptor | `tls_config` | none | concurrent streams · per-connection concurrency · connection lifetime | **yes** — gRPC/protobuf | 2026-09-24 · RUSTSEC-2024-0376 (server shutdown DoS from an accept error, fixed ≥0.12.3) | MIT (not confirmed) |
| tarpc 0.38.0 | not found | feature in old docs | none | not confirmed | **yes** — its own serde framing | 2026-08-12 | MIT |
| ezsockets 0.7.2 | none | rustls/native-tls | no docs | not confirmed | not confirmed | 2026-05-18 · automatic reconnect | MIT |
| bevy_simplenet 0.16.0 | server-side client authentication | optional | not confirmed | not confirmed | its own channel API | **stalled since 2025-04-26** | MIT/Apache-2.0 |
| zenoh | TLS · mTLS · user · ACL | TLS/QUIC | — | transport allow-list · ACL | **yes** — its own protocol | not confirmed | Apache-2.0/EPL-2.0 |

Sources: tungstenite settings = doc.servo.org (tungstenite `protocol/mod.rs`) · axum/tonic/fastwebsockets = docs.rs · jsonrpsee defaults = GitHub `server/src/server.rs` · editions · dates = crates.io API · advisories = rustsec.org. **RUSTSEC-2023-0065 · RUSTSEC-2024-0376 were cross-checked against the original by the main session** — the other editions · dates are a single collector source `가능성 높음`. The h2 advisories (RUSTSEC-2024-0332 · RUSTSEC-2025-0034) are search summaries only — `불확실`.

### 3-2. Secure channel · remote layer (candidates for layer ③ — below our `Dialer`/`Link`)

| Candidate | What it protects | Peer authentication | NAT · relay | How it fits | Maturity · audit · advisories | License |
|---|---|---|---|---|---|---|
| rustls + tokio-rustls | TLS confidentiality · integrity | CA/PKI · mTLS · key pinning with a custom verifier | none (reachability is ours) | a byte stream below WS | **Cure53 audit (2020)** · RUSTSEC-2024-0336 (infinite loop, fixed in 0.23.5 and others) · RUSTSEC-2024-0399 (panic on a fragmented ClientHello — **affects 0.23.13~0.23.17 · fixed in 0.23.18** · tokio-rustls `LazyConfigAcceptor` affected · `TlsAcceptor` not affected, main session cross-check) · the audit covers the 2020 edition, it is not an audit of the current edition | Apache-2.0/MIT/ISC (not confirmed) |
| iroh 1.3.0 | QUIC + TLS 1.3 · the relay sees only ciphertext | **authenticates the peer by public key** — ★whether to accept that peer (authorization) is the app's job, the docs state explicitly★ (adversarial review) | hole punching + relay fallback · the default preset is **public infrastructure** — for production it recommends dedicated infrastructure (those docs) → relay · metadata · availability decisions come separately | QUIC streams · datagrams → to carry our JSON · binary we must **decide message boundaries ourselves** (the WS framing assumption is amended) | 1.3.0 = 2026-09-28 (main session cross-check) · **no audit found** · advisories not confirmed · explicit support for Linux/macOS/**Windows/Android/iOS** (docs.iroh.computer/compatibility — main session cross-check) | Apache-2.0/MIT (main session cross-check) |
| snow (Noise) | AEAD · mutual authentication | static key · PSK | none | runs directly over a byte stream | **no formal audit (README)** · CVE-2024-58265 (nonce increment DoS, fixed in 0.9.5) | Apache-2.0/MIT |
| russh | SSH channel | host key · user key | none | channel · tokio | 0.62.4 · CVE numbers only from third-party aggregation — `불확실` | not confirmed |
| libp2p | Noise/TLS 1.3 | peer id derived from a key | relay · hole punching | multiplexed streams | audit mentioned only at forum level · RUSTSEC-2022-0009 | not confirmed |
| tsnet / libtailscale embedded | WireGuard overlay | Tailscale identity (control plane) | Tailscale DERP · **account required** | Go runtime or C library · Rust wrapper 0.1.0 | immature wrapper | BSD-3 (libtailscale) |
| Relying on the user's Tailscale/WireGuard | overlay | the user's network | the user's network solves it | the daemon only binds to the tailnet IP | our code 0 | — |

**Main session observation (inference):** the cheapest remote path = **rustls (certificate pinning or mTLS) + the user's VPN**. If we must solve NAT ourselves = **iroh**. Both fit into the transport's `Dialer` and the (new) `Listener` place, and the connection lifecycle management above is used as is.

### 3-3. Pairing (no layer does it for us)

- Syncthing: exchange of device IDs (certificate hash) · QR optional — that ID is the fixed identity. `가능성 높음`
- t3code: a one-time expiring pairing grant → signed cookie. `가능성 높음` (local clone)
- orca · paseo: QR pairing → end-to-end encryption over a relay. `가능성 높음` (local clone)
- OpenClaw: `openclaw qr` gives a QR + setup code · **it refuses plaintext `ws://` on Tailscale/public hosts and requires `wss://`**. docs.openclaw.ai — `가능성 높음`
- SPAKE2 of magic-wormhole (short code) — source not obtained · `spake2` crate 0.4.0 · no audit found — `불확실`

## 4. Our current status (measured from code · 2026-10-12)

**What the current production path (`net` + daemon + shell `daemon_client`) has:** loopback bind (`127.0.0.1:0`) · 256-bit CSPRNG token · first-frame authentication with a 1 second deadline · **constant-time comparison** (main session cross-check `crates/engram-dashboard-net/src/ws.rs:291`) · Origin allow-list (a request without Origin passes · the four allowed values are **unmeasured design values** — that file admits it) · ping 20 seconds/no-news 50 seconds.

**Holes (in the order they grow once remote works):**
1. **No encryption** — the shell uses plaintext `ws://`. If remote, the token passes in plaintext. The transport has not turned on `wss://` either.
2. **No deadline on the WS upgrade** — `accept_hdr_async` runs without a deadline **before** the authentication deadline (1 second) → a peer that only opens TCP and sends nothing holds the task forever (main session cross-check, order of `ws.rs:323` → `:331`. **Not measured**, read from the code order).
3. **No cap on connection count** — the accept loop starts a task per connection with no cap.
4. **No rate limit on authentication failures** — a failure closes only that socket.
5. **The message cap stays at the tungstenite default 64 MiB** — that file left it as 「동작 변경이라 별건」.
6. **`net` has no write deadline** (a known hole) — the transport has 5 seconds.
7. **The token is protected only by file location** — `daemon.json` ACL not set · rotated at every boot · the agent control token is an environment variable · default ACL file.
8. No Host header check (DNS rebinding) — for the control HTTP `/mcp` only a comment says it relies on the rmcp default `allowed_hosts`, and no check was found for `/control/*`.

**The transport crate is better than `net` on connection lifecycle management (write deadline · bounded queue) and weaker on edge security:** authentication is the consumer's `Handshake` job (intended) · no Origin check · `WsListener::bind` accepts any address · one peer's slow handshake blocks other accepts (that file admits it).

## 5. Fit to constraints (candidates × our constraints)

Constraint sources = CLAUDE.md 「아키텍처 원칙」 (swappable abstraction · core isolation · platform neutrality) · ADR-0177 decision 5 (wire preservation) · preference for MIT/Apache licenses (the final decision is up to the responsible department).

| Option | Wire preserved (wire 보존) | Swappable seam | Windows | Security gain | Cost |
|---|---|---|---|---|---|
| **A. Keep the transport + only layer ③ off the shelf (rustls now · iroh/SSH/VPN when remote) + move ①② over from the `net` one** | ○ | ○ (`Dialer`/`Listener` as is) | ○ (rustls) · iroh not confirmed | encryption · peer authentication left to audited parts | Listener seam · porting Origin/deadline/caps |
| B. Adopt tonic | ✕ (gRPC) | △ | ○ | TLS · concurrency caps | full wire replacement · authentication · Origin still ours |
| C. Adopt jsonrpsee | ✕ (JSON-RPC envelope) | △ | not confirmed | connection · body caps | same + the defaults are loose |
| D. Use iroh wholesale as the link | △ (content as is, message boundaries decided anew) | ○ | ○ (per docs — not measured) | encryption · key authentication · NAT (authorization still ours) | forces QUIC even for local only · no audit · relay infrastructure decision |
| E. Borrow nothing (TLS too ourselves) | ○ | ○ | ○ | — | writing our own cryptography = taboo |

**Rejected candidates (→ candidates for ADR rejected alternatives):** jsonrpsee · tonic · tarpc · irpc · zenoh · capnp-rpc = force their own envelope/transport (violates wire preservation) · bevy_simplenet = stalled since 2025-04 + its own channel API · tsnet embedded = Go runtime · immature wrapper · account required · snow = no audit + no NAT (weak reason to choose it when TLS suffices).

## 6. Open issues · limits

- **iroh's audit · RustSec history not confirmed** — D and the remote stage of A rely on it (platform support is confirmed by docs · not measured).
- **Pairing precedents need reinforcement** — the magic-wormhole protocol (short code + SPAKE2) is a mature design to compare against. The Rust implementation crate is EUPL, so it is outside the preferred licenses (the final decision is up to the responsible department) — only the protocol design is a reference candidate.
- The Windows behavior of fastwebsockets is not measured (an open item, not a reason to exclude).
- **The Docker · PostgreSQL rows are the collector's memory** — the conclusion (remote is left to an existing channel) stands on the other rows alone.
- **The DoS axis has almost no peer documents** — since peers cannot be compared on this axis, the holes in §4 are set only by our own judgment.
- **The russh CVE numbers · some h2 advisories are third-party aggregation · search summaries** — not used in the conclusion.
- **The 「업그레이드 시한 없음」 in our code is a reading of the code order, not a measurement.**
- Not surveyed: Plex/Jellyfin · Steam Remote Play · Coder · Cloudflare Tunnel · boringtun · remoc · actix-ws.

## 7. Adversarial review result

- Reviewer: codex (cross-family) · effort high · level 2~3 · web search on. ★From the input, §4 (file:line of our code · hole details) was left out and only a summary was sent★ (to minimize taking internal information outside).
- Verdict: **FIX** — 8 items. The main session cross-checked the key three against the original (the RUSTSEC-2024-0399 edition range · the iroh platform table · the Salvo Origin option) → in all three the reviewer is right.
- Applied:
  1. (high · omission) Salvo's Origin allow-list — conclusion 3 · §1 · §3-1 fixed. 「기본으로 켜 주는 곳 없음」 is kept (Salvo too is off by default).
  2. (high · wrong edition) RUSTSEC-2024-0399 = 0.23.13 **introduced** it, 0.23.18 fixed it · API range — §3-2 fixed.
  3. (medium · unfounded exclusion) the reason to exclude fastwebsockets on Windows is withdrawn — §3-1 · §5 fixed.
  4. (high · logic gap) iroh only authenticates and authorization is the app's job — §3-2 · §5 fixed.
  5. (medium · excessive uncertainty) iroh explicitly supports Windows/Android/iOS — §3-2 · §6 fixed.
  6. (medium · omission) iroh production recommends dedicated relay infrastructure — §3-2 fixed.
  7. (medium · exaggeration) the CVEs were unauthenticated endpoints — Origin is a double defence for us — conclusion 3 fixed.
  8. (medium · logic gap) with iroh, 「wire 보존」 requires deciding message boundaries anew — §3-2 · §5 fixed.
- Reviewer cross-check (supported): tungstenite 64 MiB/16 MiB default · axum 0.8.9 same defaults · RUSTSEC-2023-0065 · RUSTSEC-2024-0376 · code-server 4.10.1 Origin · Cline kanban · rustls Cure53 2020 (but not an audit of the current edition).
- Candidates the reviewer added: Salvo · tower-mcp (pattern reference) · the magic-wormhole protocol (pairing).
- The reviewer did not strike conclusions 1·2·5 and the recommended direction (A) — passing does not mean proof, it means no counter-evidence appeared.
