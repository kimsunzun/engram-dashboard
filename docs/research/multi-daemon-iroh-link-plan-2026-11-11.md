# Multi-daemon link — iroh vs alternatives, fit measurement, and the build plan (2026-11-11)

- **Status:** reviewed (codex FIX, 8 findings → applied). Follows `multi-daemon-federation-direction-2026-11-11.md` (read that first for the GPT-draft check, peers, Reason Network/Syncthing/hcom source reads).
- **User criteria (2026-11-11):** no external account and no separate install — 「로그인을 해야된다고? 그럼 우리게 아니잖아」; we only send data, a library handles encryption / peer auth / connectivity; keep v1 small; daemon↔daemon first — 「난 그냥 데몬간 통신만 생각하고」. 「iroh 말고 다른 대안 찾아보고 만약에 없으면 이걸 하되 …」.
- **Method:** `/engram-research` medium, design-decision mode. Collectors: candidate survey (Claude scout-sonnet, ~10 searches + `gh api`), seam fit + Reason Network iroh usage + isolated build measurement (Claude senior; scratch projects outside the repo). Main grounding: iroh blog v1 fetched, `link.rs` `peer_key` and probe `Cargo.lock`/exe size re-checked. Cross-family review: §9.
- **Confidence legend:** as in the direction report (certain / likely / uncertain / *inference*).

## 0. Bottom line

1. **No alternative beats iroh under the user's criteria; go with iroh.** (likely — *hypothesis, scope not exhaustive* for "no better candidate")
2. **It fits our transport seam without changing trait shapes**: one QUIC connection per link with one long-lived bi-stream implements `Dialer::dial`, `Listener::accept`, `LinkTx::{send,ping,close}`, `LinkRx::recv`; the remote node id goes into the already-reserved `LinkInfo::peer_key` (`transport/src/link.rs:240`, doc: "TLS key pinning · iroh node id — TRD 3 §3-5 · §4"). (certain — code read; adapter not built)
3. **Cost**: +~172 crates to the daemon graph (certain — `cargo tree` vs the daemon graph); standalone probes under the repo release profile were 6.78 MB (iroh) vs 1.25 MB (WS) per the collector's measurement (artifact not preserved — likely); the real daemon delta is **unmeasured** (*inference*: a few MB). No extra DLL or file to ship — everything links statically (certain — `dumpbin`).
4. **Plan**: v1 = daemon↔daemon agent mail between the user's own PCs, paired by pasting IDs, mail only, fail-fast when the other PC is off; 5 units (§6). Shell→N daemons, outbox, pairing codes, task delegation come later on the same link.

## 1. Alternatives (candidate survey)

| Candidate | Account/install free | Key auth + encryption | NAT traversal + relay | Rust/Windows | License | Maturity | Verdict |
|---|---|---|---|---|---|---|---|
| **iroh** (n0) | ✓ library | ✓ ed25519 node key = identity, QUIC/TLS | ✓ hole punching + relay fallback; n0 public relays (no account, rate-limited) or self-hosted `iroh-relay` | ✓ | Apache-2.0 (gh) / MIT-or-Apache (Cargo) | 12,967★; 1.0 on 2026-06-15, v1.4.1 2026-10-22; wire-stable within v1 | **choose** |
| rust-libp2p | ✓ | ✓ Noise/TLS | ◐ relay v2 + DCUtR + AutoNAT, but needs access to a relay node (ours or another reachable libp2p node — no ready relay service like n0's); DCUtR success ≈ 70% in an IPFS measurement (arXiv 2510.27500 — snippet only) | ✓ | MIT | 5,641★; still 0.x (v0.57.1) | runner-up — heavier (Swarm/behaviours/multiaddr), no ready relay |
| webrtc-rs / str0m | ✓ | ✓ DTLS | ◐ needs our own signaling server + STUN/TURN | ✓ | Apache / MIT | 5,184★ / 653★ | ✕ — we would build the hard parts |
| quinn alone | ✓ | ✓ | ✕ no punching or relay | ✓ | Apache | 5,343★ | ✕ |
| tailscale-rs / tsnet | ✕ Tailscale account or Headscale server | ✓ | ✓ | ◐ preview; Linux/macOS listed | BSD-3 | experimental | ✕ (user criterion) |
| ZeroTier libzt | ◐ | ✓ | ✓ | ◐ C++ | unclear (NOASSERTION) | 278★ | ✕ license |
| Nebula / Yggdrasil / NetBird | ✕ separate Go daemons | ✓ | ✓ | ✕ embed | mixed | high | ✕ |
| magic-wormhole.rs | ✓ | ✓ PAKE | ◐ transit relay | ✓ | EUPL-1.2 | 1,060★ | ✕ one-shot transfer (pairing step only) |
| Hyperswarm / HyperDHT | ✓ | ✓ | ✓ | ✕ JavaScript | MIT | 1,346★ | ✕ |
| rustp2p (rustp2p-quic) — added by review | ✓ | ◐ defaults to skipped certificate verification | ✓ NAT traversal + relay forwarding | ✓ | — | not assessed | ✕ unsafe default for peer auth (per review, not re-checked) |
| Veilid — added by review | ✓ embeddable | ✓ | ✓ (its own DHT/routing network) | ◐ Rust core | — | not assessed | unassessed — routes through the Veilid network; not re-checked |

Sources: `gh api repos/<owner>/<repo>` (2026-11-11), project READMEs; iroh facts from iroh.computer/blog/v1 (fetched): "An iroh v1 endpoint will be able to communicate with another iroh v1 endpoint, regardless of minor version or language"; public relays "rate-limited for relayed traffic, which can change at any time"; "more than 200 million endpoints created, in the last 30 days alone".

**Who uses iroh** (corrects an earlier impression that it is "widely used"): in agent tools, Reason Network and agentbus (source / README read). Elsewhere, Delta Chat for realtime P2P channels with relays on chatmail servers (uncertain — iroh.computer page snippet only). Spacedrive: not verified. So: a mature, actively maintained library with heavy crates.io traffic (≈ 4.17M total / 2.44M recent downloads), but few named production users we could confirm. (likely)

**iroh caveats** (likely): n0 public relays had a global outage on 2024-11-05 (blocked new relayed connections only); a Windows receive-error regression was fixed and tested in v1.0.2; frequent breaking changes before 1.0 (65 versions / 4 years — snippet); iroh uses n0's own QUIC fork `noq` (not quinn — certain, probe `Cargo.lock`); `portmapper` sends UPnP/NAT-PMP probes to the local router by default (can be disabled).

## 2. Fit to our transport seam (code read — certain unless marked)

What the adapter must handle (all inside a new transport `iroh` feature module, beside `ws.rs`):
- **Framing**: QUIC gives a byte stream → kind byte + u32 length + max size per frame (Reason Network: u32 BE length + JSON, 4 MiB max).
- **One stream per frame is wrong**: QUIC streams are unordered relative to each other; our seq/resume assumes an ordered link → one long-lived bi-stream.
- **Stream preamble**: `open_bi` is invisible to the peer until a byte is written → dialer writes a preamble byte immediately.
- **Cancel safety**: iroh's accept steps are not jointly cancel-safe → run accept in a background task feeding a ready queue (same shape as `WsListener`).
- **Close loses data**: QUIC `CONNECTION_CLOSE` can discard data the remote QUIC stack received but its application has not read (docs.rs iroh `Connection::close`) → the final `Reject{last}` frame is best-effort diagnostics only; `finish()` + a short wait improves the odds (Reason Network sleeps 2 s before its 403 close) but guarantees nothing. Correctness must never depend on the last frame; test that boundary.
- **Implicit close = code 0** → map to `Closed(None)` / `GOING_AWAY`, not a retryable rejection in `RejectKind::of`.
- **Close reason must fit one packet** → adapter-local cap constant (exact cap not verified).
- **Keepalive**: keep our supervisor ping — QUIC keepalive only proves the peer's QUIC stack is alive; QUIC has no auto-pong, so `ping()` sends a real keepalive record.
- **Reconnect**: iroh switches paths (direct ↔ relay) inside a live connection; our supervisor redials whole connections. Simultaneous dial needs a tie-break — keep the connection dialed by the smaller id (Reason Network `net.rs:1060-1072`; same rule as TRD 3 L270-271 premise).
- **Identity**: dial side — iroh verifies the remote key against the address we dial; accept side — read `Accepted::info().peer_key` before `Registry::attach` and gate there.

Messaging seam (proposal — *inference* from code read; no code edited): `Resolution::Remote{node, name, id}` in `resolve_token` after local live/dormant layers (ADR-0309 d3); a messaging-local `NodeId = uuid::Uuid`-style alias keeps zero workspace deps (ADR-0110); a non-blocking `RemoteDeliveryPort::enqueue` shaped like `FlushTrigger` (do not reuse blocking `DeliveryPort::inject` for a network round trip); `on_remote_verdict(...)`; receiving side `Entrance::Remote` and an optional node on `SenderIdentity`. Ledger rows keyed by `PeerId` are the largest unsized change.

## 3. Weight and linking (measured in scratch projects, toolchain 1.95.0)

| | iroh 1.4.1 (defaults) | iroh, `tls-ring` only | tokio + tokio-tungstenite + rustls |
|---|---|---|---|
| Unique crates (normal / with build deps) | 247 / 255 | 240 / 248 | 53 / 57 |
| Clean release build (deps downloaded) | 114–145 s | 116 s | 27 s |
| Exe, default profile | 12.8 MB | 10.35 MB | 1.71 MB |
| Exe, repo release profile (LTO, strip) | 6.78 MB | — | 1.25 MB |

- Against the daemon's graph: 78 shared, **172 new** crates (new: rustls, ring, noq, ed25519-dalek, blake3, h2, portmapper, netwatch, wmi, n0 crates). No ring/aws-lc-rs conflict (repo has neither today). MSRV 1.91 ≤ repo 1.95.0. Timings noisy (a parallel workspace test run).
- **Linking**: all static. ring's C/asm and blake3's asm compile into static libs at build time (needs the MSVC C compiler — present). The only `-sys` crate is `windows-sys`. `dumpbin /dependents`: only Windows system DLLs + `VCRUNTIME140.dll` / UCRT (already imported by today's daemon); new imports `iphlpapi`, `ole32`, `combase`, `crypt32` — all system DLLs. **Nothing extra ships.**
- Probe ran offline with relay disabled and printed an endpoint id; `presets::N0` (public relays) not exercised.

## 4. What Reason Network does with iroh — copy vs replace (source read, `I:\Engram_Workspace\opensource\reason-network` @ `5442c8b5`)

- Copy: endpoint recipe `Endpoint::builder(presets::N0)` (or `presets::Minimal` + `RelayMode::custom` for self-hosted relays) + `.secret_key` + optional mDNS (`net.rs:292-304`); a separate ALPN for joining vs members (`tincan/1`, `tincan/join/1`, `:796-798`); gate on `conn.remote_id()` and close non-members (`:729-743`); length-prefixed framing; smaller-id tie-break; 20 s connect timeout, 2→60 s redial backoff; receiver verdicts `queued | delivered | held | refused | retry`.
- Replace with ours: their iroh `Router` + `run_link` + `dial_loop` → our `Listener`/`Dialer` + `Registry` supervisor; their acks map → our `Wire` request/reply correlation; their JSON `Wire` → our `PeerWire`.
- Don't copy: key file with no protection on Windows (`daemon/mod.rs:223-233`, chmod 600 on unix only); full-mesh presence broadcast; revocation without effect; hook/MCP delivery (we own the PTY/stdio).

## 5. Architecture after the change (*inference*)

```
PC-A                                                PC-B
engram app (shell) ── loopback WS ── daemon A        daemon B ── loopback WS ── engram app
                                       │  messaging     ▲ messaging
                                       │  └ Remote      │ Entrance::Remote
                                       ▼                │
                         transport Registry<PeerWire> ══ iroh (QUIC, node keys) ══ Registry<PeerWire>
                                 direct / hole-punched UDP, else relay (n0 public or self-hosted)
```
- New relationship = its own `PeerWire` · `Registry` · `Listener` (ADR-0310 D2). Shell link untouched; loopback rule untouched (the iroh listener is a different carrier).
- Agent crate untouched (ADR-0003). Messaging gains a port + resolution variant only (ADR-0110, ADR-0309 d3).
- Identity: the daemon's iroh secret key in its state folder = the persistent daemon identity (ADR-0310 D3's "bind DaemonId to a key" is satisfied by construction → an ADR revises D3's UUID shape).

## 6. Build plan

**Claude-decided defaults (internal; reported, not asked)**
- Link: iroh 1.x behind a transport `iroh` feature (Dialer/Listener/LinkTx/LinkRx), `tls-ring` only; `portmapper` off unless measured necessary. (LAN discovery is user-visible — moved to decisions.)
- Address lookup + relays (review finding, verified in `iroh-1.4.1/src/endpoint/presets.rs`): `presets::N0` = n0 relays **plus** publishing/resolving the endpoint's address info through n0's DNS/pkarr server `iroh.link` — n0 then sees each PC's node id and reachable addresses (not message content). Relays also bootstrap connections before they migrate to a direct path, so "relay only when direct fails" is wrong. `Minimal` + custom relay drops that lookup, so a pasted ID alone is not dialable — the pairing record must then carry addresses/relay hints (Reason Network's ticket carries relay + IP hints). This is a user decision (§6 decisions).
- Identity: one secret key file per daemon (base `file` rules, ADR-0291); Windows file ACL hardening via `platform` (ADR-0266) — scope in the TRD.
- Trust: `peers.json` allowlist (node id + display name); non-members closed at accept with a reason; removal closes live links immediately and blocks re-auth.
- Sender binding (review finding): the sender's node identity comes only from the authenticated transport key, never from `Hello.node` / `Mail.from`; a frame claiming another node or a local agent is rejected (Reason Network `daemon/mail.rs` "cannot send as"). Note `Registry::attach` discards `LinkInfo` after attachment (`registry.rs:299-305`) → the peer key must be carried into the per-link state.
- Wire: `PeerWire` vocabulary `Hello{node, protocol_version}`, `Mail{id, from, to, body, reply_to}`, `MailVerdict{id, …}`, keepalive; request/reply correlation from transport. Verdict vocabulary must mirror local semantics (review finding): local messaging accepts dormant/busy recipients and parks the mail (`service.rs:689-712`), so the verdict needs "accepted, parked (volatile)" distinct from "injected" and from refusals — fixed in the TRD; tests cover lost verdicts and receiver restart.
- Permissions: remote may only send mail. Rate limit, loop guard and body cap are user-visible → moved to the decision list with Reason Network's values as the precedent (`src/daemon/mail.rs:27-32`).
- Failure: other PC off / no verdict within the shared request deadline → the sender agent gets "not delivered" / "delivery unconfirmed". No automatic resend (avoids duplicates without a dedup table).
- LLM surface on the daemon bus (ADR-0055/0307): `peer.my_id`, `peer.list`, `peer.add`, `peer.remove`, `peer.status`; mail uses the existing send with a remote address.

**Units** (each through `/engram-implement`; S-number on topic start)
- U0 — ADRs: iroh as the daemon↔daemon link (rejected alternatives from §1); identity = node key (revise ADR-0310 D3); remote = mail only; product principle "no external account / separate install".
- U1 — transport `iroh` adapter + tests (two in-process endpoints, relay disabled): reject / close-code / keepalive / tie-break, malformed and oversized frames, ALPN / protocol-version mismatch.
- U2 — daemon identity key + `peers.json` + protocol `peer` vocabulary + daemon `link/peer.rs` (second Listener, dial listed peers, online/offline status). Lay for (a) now so it is additive later (user 2026-11-11: 「그 순서로 하면 된다는거지? 괜히 구조 바꾸고 하는건 없는거고」): a permission field per allowlist entry (v1 uses only `mail`; (a) adds `control`), and one ALPN per relationship on the iroh endpoint (v1 opens only the daemon↔daemon ALPN; (a) adds a shell↔remote-daemon ALPN with its own Wire/Registry per ADR-0310 D2).
- U3 — messaging remote branch: address resolution, `RemoteDeliveryPort`, verdict back to the sender, receiving entrance, rate limit + loop guard.
- U4 — app: "My ID" (copy) plus "My address" (LAN IP:port, copy), peer list (add = ID required + name + optional address, remove, online state), remote mail visible in the existing mail UI; LLM commands. One form, not two modes (Syncthing precedent: device ID required, addresses default `dynamic` or explicit `tcp://ip:port`); an IP-only mode is rejected — without the ID the peer cannot be verified. Needs a fixed listen port setting (iroh binds a random UDP port by default; exact builder API to confirm in U1) and covers LANs that block multicast (mDNS fails) — user 2026-11-11: 「ip 입력하는 모드도 있고 아니면 id로 해주는 모드도 있고」.
- U5 — real two-PC check: same LAN, then home ↔ office through the relay path; unpaired peer refused; removal during live traffic; Windows Firewall first-run prompt (allow / deny / non-admin user).

**Later, same link**: (a) shell → N daemons (3-2 U6 N-daemon seat, T-85, T-86); outbox + persistence (SQLite, durability boundary per direction report §5); one-time pairing code / accept prompt (separate ALPN); task delegation (A2A task state names).
- Idea parked, not planned — several people driving one host PC's agents (user 2026-11-15: 「여러사람이 PC에서 채팅 창 입력하면 한개의 클로드가 설치된 pc가 소비되는걸」 → 「툴콜이 안되는거아냐? 애초에 좀 더 생각해보긴 해야됨」). Two blockers: (1) Anthropic Consumer Terms (effective 2025-10-08): "You may not share your account login information, Anthropic API key or account credentials with any other person or provide your account to any other person." — may cover one personal subscription serving several people (legal reading not ours to make; API keys / Team plans are the multi-user products); (2) agent tools run on the host PC, so remote users cannot have their own local files read/edited without remote tool execution back to their PC (large, and a security problem). Structurally it would be (a) with N clients (ADR-0179 decision 1; its open items: input lease, client identity).
- "My PCs" group (user 2026-11-15: 「한 계정을 여러 pc에서 사용하는 구조도 되겠네」): v1 pairing is pairwise (N PCs → N·(N−1)/2 pairings); later a replicated signed roster so a new PC joins once (Reason Network `team.rs` `TeamState`, `Signed<T>`). Keep `peers.json` extensible toward a group record. Suggested trigger: 3+ PCs.
- Server swap as a product feature (user 2026-11-15: 「배포판에서 서버를 둔 사용자들이 쉽게 교체할 수 있게 뭔가 시스템 화」): v1 has only the two URL settings (relay, address lookup). Later: a 「연결 서버」 settings section (default n0 / custom), a one-line server profile string to copy between PCs, LLM commands (`server.*`), and a self-hosting guide for `iroh-relay` + `iroh-dns-server` (Docker). Precedents: Syncthing relay/discovery fields with `default`; Tailscale `--login-server` (Headscale); Happier `happier relay host install`.

**User decisions (visible behaviour) — few**
1. Pairing in v1: paste the other PC's ID on both PCs ★ / one-time code (more work: separate join protocol).
2. Remote address shown to agents: `name@pc-name` ★ / bare names that search other PCs too.
3. Other PC off: fail immediately and tell the sender ★ / keep and send later (outbox — later unit).
4. Connection help: n0 relays + n0 address lookup by default (pasting an ID is enough; n0 sees node ids and addresses, never content), plus settings for our own relay / DNS server URLs and an optional peer address on pairing so n0 can be dropped without code changes ★ (revised 2026-11-11 after the user's 「유료로되면 못쓴다는거네」 concern) / n0 only / no n0 at all (pairing must carry addresses; home ↔ office needs our own relay). User understanding confirmed: 「중계 서버는 반드시 필요하고 나중에 내걸로 바꾸는 장치는 있다는거잖아」.
5. Limits on remote mail: per-PC rate limit 60/min, per-agent-pair loop guard 12 per 10 min, body cap 64 KB (Reason Network's values) ★ / other values / none.
6. LAN discovery (find paired PCs on the same network without n0): on ★ / off.

## 7. Risks

- n0 public relay rate limits / outages → home ↔ office may fail when hole punching fails; mitigation: self-hosted relay option (`iroh-relay`).
- Corporate firewalls may block UDP entirely → only the relay path (HTTPS) works; not measured.
- Dependency growth (+172 crates, +≤5.5 MB) and build time; CI cache impact not measured.
- iroh is n0-led; long-term maintenance is a vendor risk (mitigated by the seam — the adapter is replaceable).
- Messaging ledger keyed by local ids — remote senders need a qualified identity; sized in the TRD.

## 8. Gaps

- libp2p hole-punch figure and Delta Chat usage are snippet-only. Veilid and rustp2p only per review. Exact QUIC close-reason cap and noq send-stream drop behaviour not verified. `presets::N0` not run (offline rule). UDP-through-corporate-firewall behaviour not measured.

## 9. Review result

codex (GPT family, effort high, independence level 2–3) → **FIX**, 8 findings; main session verified #2 in the installed iroh source; all applied:
1. high · refuted — rate limit / loop guard / body cap / LAN discovery are user-visible (CLAUDE.md 「사용자 대면 동작」) → moved to §6 decisions 5–6.
2. high · refuted — `presets::N0` also publishes/resolves addresses via n0 DNS/pkarr (`iroh.link`); relays bootstrap before direct; `Minimal` loses lookup → §6 defaults + decision 4. Verified: `iroh-1.4.1/src/endpoint/presets.rs` doc comment and `impl Preset for N0`.
3. med · refuted — `finish()` + wait does not guarantee the final frame (docs.rs `Connection::close`) → §2.
4. med · not-found — "+5.5 MB at most" is not an upper bound; the 6.78 MB artefact was not preserved → §0 item 3 reworded; daemon delta unmeasured.
5. high · omission — bind sender identity to the transport key; `LinkInfo` dropped after attach → §6 trust.
6. med · omission — Veilid, rustp2p-quic; libp2p needs a relay node, not necessarily ours → §1 rows (not re-checked).
7. high · omission — verdict vocabulary vs dormant/busy parking → §6 wire.
8. med · omission — acceptance checks (unpaired, removal under traffic, malformed/oversized, version mismatch, Windows Firewall) → §6 U1/U5.
Reviewer spot checks supported: `peer_key` reservation and pre-attach inspection, cancel-safe Listener contract, accepted peers also ping (`registry.rs:311-328`), messaging seam, ordered single stream + preamble, key auth + v1 wire compatibility, probe `Cargo.lock` (iroh 1.4.1, noq 1.4.0, no quinn), Reason Network gate / tie-break / unix-only key protection.
