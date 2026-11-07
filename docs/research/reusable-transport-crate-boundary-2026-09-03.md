# Boundary of a reusable transport crate — the shape where 「패킷 정의만 주면 되는」 (one that only needs the packet definitions handed over) and the multi-connection ownership model

- **Status:** survey complete · adversarial review **applied** (verdict **BLOCK** → findings applied · §9) · ★**re-judgment was not run**★ — a second review of the revised version has not been run
- **Method:** `/research` deep · design-decision mode · 5 collectors (1 internal constraints + 4 external, parallel · mutually isolated) · grounding = the main session checked primary sources directly · adversarial review = cross-family
- **Date:** 2026-09-03
- **Confidence legend:** `확실` (certain: original text checked + independent cross-confirmation) · `가능성 높음` (likely: original text checked, single source) · `불확실` (uncertain: not examined · absence check) · `출처 없음` (no source: do not cite)

> ★**This document is not a decision.**★ It is a heavy design, so the user makes the choice. What is here is the options and their costs.

---

## §0 One-line conclusion

★**A library that owns both the type seam and the connection lifetime does exist. What I could not find is one that adds, on top of that, 「정체성이 각자 다른 피어 N 대의 레지스트리」 (a registry of N peers, each with its own identity).**★

- **Old claim refuted:** the claim 「타입 seam 소유 진영과 수명 소유 진영이 둘로 갈리고 합친 물건이 없다」 (the camp that owns the type seam and the camp that owns the lifetime are split in two, and nothing combines them) **is false.** I checked two counterexamples against the original text — `bevy_simplenet` has the consumer **define the concrete message types** through the `ChannelPack` trait (*"Messages are automatically serialized and deserialized, so concrete types are required"*) while the crate owns connection acceptance (`AcceptorConfig`) · authentication (`Authenticator::None/Secret/Token`) · **automatic reconnection** · lifetime events (`ClientReport`/`ServerReport`). **It is not tied to Bevy either** — bevy is an optional feature that adds the `Resource` derive. `tonic` also puts reconnection · connect/serve · keepalive · endpoint balancing on top of generated typed messages (the fixed IDL is a *cost*, not an absence). `확실` (original text checked)
- **The narrowed claim that survives:** the `bevy_simplenet` docs **do not cover one client attaching to several different servers.** So what I could not find is not the 「타입 seam + 수명」 (type seam + lifetime) combination but one that **also carries an N-peer registry and per-peer identity.** `불확실` (absence check — the strength of evidence is weak)

So **the causal claim 「재사용하려면 I/O 를 포기해야 한다」 (to be reusable you must give up I/O) does not hold either.** The libraries that were actually reused simply **did not sell, or sold separately,** that I/O layer, and layering them on top of each other is not itself ruled out.

**The design question is not 「두 진영 중 어느 쪽을 고르나」 (which of the two camps to pick) but 「타입 seam·수명·N-피어 레지스트리 셋을 한 crate 안에서 어떻게 층으로 겹치나」 (how to layer the type seam, lifetime, and N-peer registry on top of each other inside one crate).** The four candidates differ in how they overlap (§3).

---

## §1 Relation to earlier surveys — overlaps · corrections

★**Before starting I checked `docs/research/` exhaustively.**★ It is the remedy for a failure where the previous session skipped this step and ran the same topic twice.

### 1-1. Overlap map

| Earlier document | Verdict | What overlaps |
|---|---|---|
| `crate-granularity-survey-2026-08-17` | **overlap (authoritative)** | The canonical source of the criterion for 「새 crate 를 파도 되는가」 (may we create a new crate). The justification for a new crate in this document must pass that criterion |
| `module-split-criteria-2026-08-26` | **partial overlap** | An extension of the same question. **§9 (adversarial review) corrects several lines of its body, so do not cite the body alone** |
| `client-identity-survey-2026-08-17` | **partial overlap** | Handshake · reattach ownership. **But that document has no conclusion** — with the threat model undocumented, it states explicitly "do not decide on these 2 editions alone" |
| `daemon-version-mismatch-2026-08-14` | **overlap** | The layer that owns version negotiation. The confirmed decision moved on to ADR-0158, and this document read it |
| `multi-window-layout-authority-topology-2026-06-27` | **partial overlap** | **ADR-0036 has already decided** the Q-B that document left open (see 1-3 below) |
| `agent-messaging-survey-2026-06-28` | **partial overlap** | Logical separation of control/data plane + a transport seam recommendation |
| `unified-command-bus-survey-2026-08-12` | **partial overlap** | The constraint 「전송에 코드를 묶지 않는다」 (do not tie code to the transport) is the same decision axis |
| `tauri-channel-multiwindow-carrier-2026-06-28` | **no overlap** | About an in-proc carrier. Unrelated to a network transport boundary |

### 1-2. Collapsed premises — do not cite as evidence

- ★**「의존 방향을 강제하려면 crate 로 쪼개야 한다」 (to enforce dependency direction you must split into crates) = false**★ — ADR-0151 decision 4 says so explicitly, and this repository already enforces direction with an `rg` isolation gate + a `cargo tree` upper-bound gate. `확실`
- ★**「crate 로 빼려면 두 번째 소비자가 있어야 한다」 (to extract into a crate there must be a second consumer) = broken by a counterexample**★ — rust-analyzer takes `ide`, which has only one consumer, as an API-boundary crate. ADR-0151 decision 1 = "does not ask whether a real consumer exists now". `가능성 높음` (the counterexample is real, but the 「규칙이 없다」 (there is no rule) side is an absence check)
- **The living criterion = 「독립적으로 쓸 수 있는가」 (can it be used independently) + 「무게」 (weight)** (ADR-0175 decision 6 added the weight axis: *libs are kept at roughly similar weight, and a single-file lib is not created*). `확실`

★**Correction — the 「rule of three」 is a different thing from the two claims above, and it has not collapsed.**★ This session wrote it as a 「무너진 전제」 (collapsed premise) in the survey brief, which was wrong. 「소비자 수를 세라」 (count the consumers) (collapsed) and 「일반화 전 실사용 사례를 보라」 (look at real use cases before generalizing) (not collapsed) are different claims.

**But the number 「3」 itself has no source.** A genealogy survey traced it back through Biggerstaff (1987) → Lanergan & Grasso (1984) → Lanergan & Poynton (Raytheon, 1978/79) and judged it folklore with *"there's no reason that it shouldn't be the 'Rule of Five' or the 'Rule of Seven' instead"*. The direction has a named source (Roberts & Johnson 1996, *Evolving Frameworks*, the first pattern "Three Examples"). ★**Recommended wording = write it as a condition, not a number**★ — instead of "extract on the third", "do not fix the boundary until a real second consumer exists in code". `출처 없음` (the number) / `가능성 높음` (the direction)

### 1-3. Two decisions this survey recovered — the design premises change

★**Correction 1 — layout authority is the shell (src-tauri), not the daemon.**★ The earlier survey recommended 「데몬 권위」 (daemon authority), but **ADR-0035 rejected that recommendation.** And that ADR fixes the lineage — *tmux/Wezterm Mux keep layout on the server because there pane==PTY, and engram lacks that premise. We belong not to the tmux model but to the **editor model**.* `확실`
→ **Implication:** on the layout axis only, the terminal-multiplexer family is not our peer. (On the transport · session-survival axes they are still peers — read the axes separately.)

★**Correction 2 — the Q-B the earlier survey left open has already been decided (ADR-0036).**★
> "**src-tauri holds a single WS connection to the daemon** and fans out output to each window by the layout routing table it owns. Windows do IPC only with src-tauri — **zero direct daemon connections.**"
> Invariants: "**a single choke point**" · "**no direct window↔daemon connection**" · "**the daemon does not know the number of clients and sees only 1 connection.**"

No discarded stamp, no later overturning ADR (exhaustive grep). `확실`

★**So the user's 「다중 데몬 접속」 (multi-daemon connection) requirement stands on top of this decision.**★ One certain thing = **the party holding the N connections must be the shell, not a window,** so as not to collide with this decision. **Whether a shell holding 1 connection each to daemons A and B violates or extends this decision is not answered by the documents** — because 「1 연결만 본다」 (sees only 1 connection) is a statement from the viewpoint of one daemon. ★**This is user decision 1 of §10. The session does not decide arbitrarily.**★

---

## §2 Market observation — why the thing we want does not exist

### 2-1. Two leanings — but not exclusive

**Type-seam side** — hand over only the types and get a typed channel. In exchange, it **does not establish the connection.**
- `remoc` — the consumer only attaches a `serde` derive. It sits on `AsyncRead+AsyncWrite` **or** `Sink+Stream` (a WebSocket plugs in as the latter). Original: *"Remoc implements no transport itself and thus depends on no networking crate; it runs over any byte stream you already have."* — **establishing the byte stream is the consumer's part.** ★**Correction — 「재연결·keepalive 가 없다」 (it has no reconnection · keepalive) was false**★: `chmux::Cfg::connection_timeout` **turns on automatic ping** with *"Time after which the connection is closed when no data is received. Pings are send automatically when this is enabled"* (default 150 seconds), and the 0.20 docs document a **reconnecting resilient transport** as one of the adaptable transports, with typed remoc channels standing on it as is. What remains true is only that **establishing the initial byte stream is the consumer's part.** `확실` (original text checked)
- `tarpc` — the transport requirement is only `Stream<Item=Request> + Sink<Response>`. Even serde is an optional feature (*"in-memory transports can be used, as well, so the price of serialization doesn't have to be paid when it's not needed"*). `확실`

**Lifetime side** — does reconnection · heartbeat for you. In exchange, it **does no typing at all.**
- `ezsockets` — *"Automatic reconnection of WebSocket Clients"*, *"Heartbeat mechanism to keep the connection alive"*, `client`/`server` features separated. But the message type is not generic but **fixed to concrete types** (`Utf8Bytes`/`Bytes`), so all deserialization is the consumer's part. `확실`

★**But this leaning is not exclusive — there are real artifacts that overlap the two**★ (`bevy_simplenet`·`tonic`, §0). All I could not find is one that adds an **N-peer registry** on top. `불확실` (absence check)

### 2-2. The price the near-misses paid — all 「축 하나 고정」 (fixed one axis)

| Library | The axis it fixed | What it lost because of that |
|---|---|---|
| `zbus` (D-Bus) | **wire protocol** | Not project-neutral. But the seam shape (`#[proxy]` trait / `#[interface]` impl duality) is worth stealing |
| `jsonrpsee` | **wire protocol** (JSON-RPC) | Same |
| `tonic` | **IDL + code generation** | Forces a build step. The failure mode is bad (a codegen failure shows up only as "No such file" from `include!`) |
| `irpc` | **transport (QUIC) + serialization (postcard)** | The docs admit it, in paraphrase: this library does not abstract the transport. ★**Do not read it together with `quic-rpc`**★ — `irpc` explains itself in its own docs **in contrast to** `quic-rpc` |
| `postcard-rpc` | **serialization (postcard)** | Forces no_std types + effectively forces a 3-crate layout |
| `bevy_replicon` | — | *"The library doesn't provide any I/O, so you need to add a messaging backend."* = gave up lifetime |

`확실` (each checked against the original text or manifest — but the download · license figures of `postcard-rpc` are `모름`)

★**Correction — `quic-rpc` does not belong in this table. It does not fix the transport axis.**★ It exposes `Connector`/`Listener` traits so that the transport can be swapped — *"A Connector can be used to open bidirectional typed channels using Connector::open. A Listener can be used to accept bidirectional typed channels"* — and it ships four built-in transports (flume = in-memory · hyper = HTTP/2 · quinn = QUIC · iroh). ★**That is, a mature precedent of 「타입 있는 채널 + 갈아끼우는 전송 seam」 (typed channels + a swappable transport seam) exists in real form, and it is the basis of candidate D below**★ (§3 candidate D). `확실` (original text checked)

### 2-3. Four mechanisms worth stealing

1. ★**path + schema hash as the routing key**★ — `postcard-rpc` builds the key by hashing the path string + the type schema. If the schema drifts, **routing fails instead of a silent misinterpretation.** **Same topology as ours** (host 1 ↔ devices N). `가능성 높음`
   - ★**But do not read it as a 「값싼 버전 협상 수단」 (cheap means of version negotiation) — it does *not replace* version negotiation.**★ The hash is **non-cryptographic FNV1a**, the wire key width is selectable among 1/2/4/8 bytes, and the protocol **reserves a version field separately.** So ① a hash mismatch only says 「다르다」 (different) and does not *supply* negotiation, and ② **on a collision even that guarantee breaks.** The mechanism itself is still worth stealing. `확실` (original text checked)
2. ★**separate wire messages from app commands by type**★ — the `Call` associated type of `ezsockets`: `on_text`/`on_binary` (coming from the wire) and `on_call` (the app instructing the connection) are separated. It maps directly onto our `agentClient` structure. `확실`
3. **ALPN string as protocol ID and version negotiation at once** — `iroh`. The cheapest handshake shape. `가능성 높음`
4. **deliver the schema at handshake time, not build time** — Colyseus sends type information + the full state together on joining. `불확실` (license · version unchecked)

### 2-4. The conditions that actually produced reuse — different from expected

What the reused libraries have in common is not 「범용으로 설계했다」 (it was designed to be general-purpose) but 「강제하는 것이 적다」 (it enforces little).

| Property | Verdict | Basis |
|---|---|---|
| not forcing an async runtime | `확실` | the account that runtime coupling splits the ecosystem into silos + the sans-IO retrospective |
| **having no I/O (sans-IO)** | `가능성 높음` | Firezone production retrospective. ★**Evidence**: Firezone **took only the ICE agent** out of `str0m` — a case where reuse outside the original project actually happened★. ★**But one case establishes one successful combination, not a causal rule of the whole market**★ — so it is not `확실` |
| separating a thin stable core crate | `확실` | `tracing-core` (*"the minimal, stable kernel of functionality"*) · `jsonrpsee-core` · `http` |
| minimal dependency surface | `가능성 높음` | there are compile-time figures but they are not a basis for *adoption* |
| `no_std` support | `불확실` | no data found |

★**The sans-IO retrospective itself admits its costs**★ — the cost of writing a pure state machine, lack of community adoption. And it was done to avoid the **function colouring** problem.

---

## §3 Candidate architectures — for the user to choose

All four candidates **create a new crate** (claim 4) and **do not tear apart the existing `net`**. The axes on which they differ are two: ★**「I/O 를 어디에 두나」 (where does I/O go)**★ and ★**「피어 N 을 누가 소유하나」 (who owns the N peers)**★.

### Candidate A — a single crate · 2 layers inside (sans-IO core + I/O layer) · direction by feature

**Shape:** one crate divided by modules into a pure state-machine layer and an I/O layer. The direction is chosen with the `client`/`server` features (`default = []`).

| Inside the crate | What the consumer provides (seam) |
|---|---|
| frame-boundary handling | **packet type definitions** (serde derive) |
| envelope + tag correlation (**generic over the tag type**) | codec choice (one of the provided codecs or its own implementation) |
| handshake **sequence** (the shape is the frames the consumer gave) | the **tag type** + how to attach and read the tag |
| connection establishment (`connect`/`accept`) | address · authentication material |
| reconnection + backoff (policy injected as a value) | reconnection policy value |
| keepalive initiation · response | TLS layer (wrapped from outside if wanted) |
| multi-connection registry + fan-out | version · capability negotiation (ADR-0158 = owned by the upper layer) |
| bounded queue = backpressure **signal** | backpressure **policy** (block or drop) |

**Multi-connection representation:** a long-lived upper object owns N connections by id and hands them out in a `get_or_connect` form. **Handle = a command channel to a background task,** so it survives across reconnection.

**Pros:** closest to the shape the user wanted. One crate, so it does not hit the weight clause of ADR-0175. The maintenance burden is lowest (there is a real precedent of a large project coming back to this form from separate crates).
**Cons:** ★sans-IO purity is held in a single crate only by convention, without compiler enforcement★ (a gate must be set up separately). And **because of feature unification, even a client-only consumer can pull in server-side dependencies** (§7).

### Candidate B — two crates · thin sans-IO core + I/O layer

**Shape:** `…-core` (sans-IO: frame state machine · handshake sequence · correlation · keepalive decision, zero workspace dependencies, no runtime) + `…` (I/O: transport · lifetime · reconnection · registry · fan-out, direction by feature).

| Inside the crate (core) | Inside the crate (I/O) | What the consumer provides |
|---|---|---|
| frame state machine | `connect`/`accept` | packet type definitions |
| handshake sequence (pure) | reconnection · backoff | codec choice |
| tag correlation (generic) | keepalive timer | tag type |
| keepalive **decision** (time as an argument) | multi-connection registry · fan-out | address · authentication material · policy values |

**Multi-connection representation:** same as candidate A, but the registry is owned by the I/O crate.

**Pros:** ★the only candidate that actually satisfies the reuse conditions (§2-4)★ — the core does not force a runtime · I/O, so another project can take just the core. It is tested with time passed as an argument, so the verification harness is the strongest. There is a mature precedent of this shape (the `jsonrpsee` hybrid: separate crates underneath + an umbrella on top that pulls them in by feature).
**Cons:** ★**It may collide head-on with ADR-0175 decision 6**★ — *a single-file lib is not created · libs are kept at similar weight.* If the core comes out thin, that clause becomes a reason for rejection. **That weight is unknown now — it cannot be measured before designing.**

### Candidate C — the transport is a byte pipe · correlation and typing are all in the upper layer

**Shape:** the crate carries only bytes/strings. It knows nothing of serialization · correlation · the packet vocabulary.

| Inside the crate | What the consumer provides |
|---|---|
| connection establishment · reconnection · keepalive | **all serialization** |
| frame send/receive (bytes) | **all correlation** (including the pending map) |
| multi-connection registry · fan-out | packet definitions + the code that turns them into bytes |

**Multi-connection representation:** the registry is owned by the crate. Handle = command channel.

**Pros:** **zero friction** with engram's 「어휘 격리는 타입 단위」 (vocabulary isolation is per type) invariant (ADR-0129). A mature precedent has exactly this shape — its transport trait has only the methods `send(String)`·`send_ping()`·`close()`·`receive()` and **does not mention id at all** (original text checked).
**Cons:** ★**Farthest from the user requirement**★ — it is not "just hand over the packet definitions" but "it carries only bytes". The least amount can be moved over from the shell's `connection.rs`.

### Candidate D — a typed multi-host lifetime engine · the transport is swapped through a `Connector`/`Listener` seam · handles are bound to a generation

**Shape:** the crate owns **both** the packet type seam and the connection lifetime, but puts the actual transport behind `Connector`/`Listener` traits and ships a default adapter (WS) with it. One supervisor task per host carries reconnection · backoff · re-subscription, and inbound comes up as **a single merged stream tagged with the host marker.** A handle is bound to a generation, so sending with an old handle after a reconnect **does not silently succeed but is rejected with a 「재연결됨」 (reconnected) error.**

| Inside the crate | What the consumer provides (seam) |
|---|---|
| packet seam | **packet type definitions** |
| tag correlation (generic) | a transport adapter (0 if the provided one is used) |
| connection establishment · lifetime | address · authentication material |
| reconnection/backoff · keepalive | reconnection policy value |
| **N-host registry + per-host supervision** | the re-seed strategy when a generation breaks |
| host-marker fan-in · **exposing generation boundaries** · fan-out | — |

**Multi-connection representation:** ★**first-class.**★ The host id is at the front of the API and events arrive carrying the host marker.

**Pros:** it is **the only one that targets all four user requirements at once** (packet seam · lifetime ownership · multi-connection · reuse). ★The form that exposes generation boundaries is the same idiom as engram's incarnation marker · replay invariant, so it absorbs the §5-4 risk by design★. There are real precedents for the transport seam (`quic-rpc`'s `Connector`/`Listener`) and for multi-host supervision (AHP `MultiHostClient`) respectively.
**Cons:** ★**The largest**★ — the ownership scope is wide so the weight is large, and the fan-out contract (per-host fairness · partial failure · slow-consumer isolation · reconnection loss) **must be designed explicitly**, and if it is left out it repeats the same defects as A·B on a larger surface. And 「소비자는 패킷 정의만 준다」 (the consumer gives only the packet definitions) is still **not entirely true** (§9 C1).

### Candidate comparison

| Axis | A (single · 2 layers) | B (core + I/O) | C (byte pipe) | D (multi-host engine) |
|---|---|---|---|---|
| Degree of achieving 「패킷만 주면 됨」 (just give the packets) | high | high | low | **high** (but not literally) |
| Project-neutral reuse | medium | **high** | medium | medium (not low, thanks to the transport seam) |
| ADR-0175 weight clause | passes | **risky** | passes | ★**riskiest**★ |
| ADR-0129 vocabulary isolation friction | low (if the tag is generic) | low | **none** | low (on the premise of a generic tag) |
| Strength of independent verification harness | medium | **high** | medium | medium (the transport seam allows an in-memory adapter) |
| Maintenance burden | **low** | high | low | ★highest★ |
| First-class multi-connection support | partial | partial | partial | ★**first-class**★ |
| Amount of `connection.rs` that can be moved over | much | much | little | **the most** |

---

## §4 Boundaries by responsibility — where precedents place them

★**This table is independent of the candidates**★ — whichever candidate is chosen, whether each item is inside or outside uses the same basis.

| Responsibility | Usual location | Basis (what was read in the original) | Confidence |
|---|---|---|---|
| byte framing | **inside** (but the codec is injected) | framing is the library, the **boundary rule is the consumer's `Decoder`** | `확실` |
| serialization | **split** | fixed (gRPC·remoc) vs optional (tarpc — *"It's entirely optional"*) | `확실` |
| connection establishment · handshake | **split** | inside (tungstenite·zenoh) vs outside (hyper — *"Connecting to a host, pooling connections, and the like are not handled at this level."*) | `확실` |
| **authentication/token exchange** | **almost always outside** | RFC 6455 §10.5 — the protocol **does not specify any** server-side client authentication method and **allows any means a general-purpose HTTP server can use**, such as cookies · HTTP authentication · TLS authentication. ★The old edition (*"does not define client authentication mechanisms beyond what is provided by HTTP"*) **was an invented paraphrase — do not cite**★ | `가능성 높음` |
| **version negotiation** | **split — but decided for engram** | ADR-0158 fixed it as **owned by the upper layer.** The transport carries that frame opaquely | `확실` (internal decision) |
| keepalive | **response = inside, initiation = split** | RFC 6455: ping is *MAY*, pong response is *MUST* — ★**but not unconditionally**★: §5.5.2 **exempts an endpoint that has already received a Close frame.** gRPC A8 puts app-level keepalive inside the library | `확실` |
| **reconnection + backoff** | **split — RPC frameworks put it inside** | the gRPC doc is titled ★*"Proposed Backoff Algorithm"*★ and **allows alternatives — it is not a mandatory spec.** The constants (INITIAL 1s·MULT 1.6·MAX 120s·JITTER 0.2) are used only as **example defaults**. tower **separates it as a separate middleware** | `확실` |
| **request/response correlation** | ★**a separate layer right above the transport**★ | §4-1 | `확실` |
| ordering/deduplication | **split** | remoc pushes it out as a requirement (needs an *"ordered, reliable transport"*) | `가능성 높음` |
| backpressure | **signal = inside, policy = outside** | Netty `isWritable()` gives only a signal, stopping writes is the app's responsibility | `가능성 높음` |
| fan-out | **split** | Netty `ChannelGroup` is inside / hyper·tungstenite·tokio-tower handle **only 1 connection** | `가능성 높음` |
| **TLS** | **almost always outside** (QUIC only is the exception) | hyper vision doc: ★*"We learned early that bundling TLS directly in hyper has problems"*★ — **a history of putting it in once and taking it out** | `확실` |
| address resolution · discovery | **split** | hyper removed the `client::connect` module itself in 1.0 and moved it out to `hyper-util` | `확실` |
| graceful shutdown | **primitives = inside, orchestration = outside** | GOAWAY·Close are protocol, the `GracefulShutdown` utility is in `hyper-util`, not hyper | `가능성 높음` |

### 4-1. Request/response correlation — the answer to our fork

There are **five camps** that faced our problem (correlation needs a request id, but that id type lives in the `protocol` crate) **exactly and solved it differently.**

**(A) Bake a concrete id into the framework → the abandoned road.** `tokio-proto` put `pub type RequestId = u64;` and made the transport yield `(RequestId, Frame)`. It was abandoned, and the reasons left in the issue are **complexity** (*"It tries to provide many capabilities at the cost of significant complexity."*) · **error visibility** · **HTTP version limits.**

★**Correction — the lesson 「id 타입을 고정해서 안 쓰였다」 (it went unused because it fixed the id type) has no basis.**★ None of the cited discussions **names id typing as a cause of abandonment.** What remains is not causation but only the **observed shift in API shape** — concrete `u64` → (in the next generation) an associated type. Reading this shift as a cause is an **interpretation,** and no source writes it that way. `불확실`

- **The error citation is corrected too** — in that issue, *"Errors that are encountered reading, parsing, and writing are kind of just lost"* is **immediately followed** by 「그 오류들이 tokio-proto 까지 *도달은 하지만* 사용자가 얻어내기 어렵다」 (those errors *do reach* tokio-proto but are hard for the user to get out). Citing it cut off as 「사라진다」 (they vanish) is stronger than the original. `불확실`

**(B) Make the tag type generic as an associated type — `tokio-tower`.** Result of checking the original:
```rust
pub trait TagStore<Request, Response> {
    type Tag: Eq;
    fn assign_tag(self: Pin<&mut Self>, r: &mut Request) -> Self::Tag;
    fn finish_tag(self: Pin<&mut Self>, r: &Response) -> Self::Tag;
}
```
★**The only bound is `Eq`**★. And the crate docs: *"tokio-tower leaves the on-the-wire implementations of protocols to other crates … and instead operates at the level of Sinks and Streams."* The pending map is owned by the **client layer,** not the transport. `확실` (original text checked)

**(C) The transport does not know ids at all — `jsonrpsee`.** The transport trait has only the methods `send(msg: String)` · `send_ping()` (default implementation) · `close()` and **does not mention id at all.** The `Id` type lives in a separate types crate and correlation is the core client's job. ★**The precedent that matches our arrangement most exactly.**★ Side observation: the core decides *when* to ping, and only *how* to ping is known to the transport. `확실` (original text checked)

**(D) The framework defines the id but the transport is generic over the message type — `tarpc`.** The transport carries the `Request` whole and does not interpret the id. Cancellation is also built on top of the id. `확실`

**(E) Split out as a session-layer protocol — Finagle mux.** *"Mux is a pure session layer protocol"* — mux carries tagged T/R pairs · cancellation · liveness ping · window advertisement, and the app protocol sits on top. `확실`

**(F) A workaround that does not use ids — NATS (a unique inbox subject per request) / remoc (channel lifetime expresses correlation).** `가능성 높음`

★**Overall verdict: the id is wire vocabulary, not transport vocabulary.** All five camps put the id in the protocol/session layer and the transport carries only frames. **I found no arrangement in the surveyed range where the transport learns the id.** It is also consistent that the pending map is owned by the client/session layer.★

**So the answer to our fork:** ★**put the correlation 「기제」 (mechanism) inside the new crate, but write it generic over the tag type and leave `RequestId` in `protocol`**★ (candidate B type). Then the new crate owns correlation without depending on `protocol`. **This is one of two forms that do not conflict with ADR-0129's 「어휘 격리는 타입 단위」** — ★**「유일한」 (only) is refuted by this document's own §4-1 (C)**★: a form like `jsonrpsee`, where the **transport trait does not know the id at all,** is the second, and that is candidate C.

**But this verdict has interpretation mixed in** — I **did not find** a maintainer sentence that states explicitly that it 「제네릭하게 짜야 한다」 (must be written generic). The shift from `tokio-proto` (concrete `u64`) to `tokio-tower` (associated type) is an observation obtained by comparing two sources, and no text says "so we changed it to generic". **Read fact and interpretation separately.** `가능성 높음`

---

## §5 Multi-connection ownership model

### 5-1. There are two kinds of 「N 개 서버」 (N servers) and precedents split there

★**This is the second yield of this survey.**★

- If the N are **one logical service that is interchangeable** (a DB cluster · message broker · gRPC backend pool) → **the library owns the set and the consumer receives only one cheap clone handle.** The consumer cannot see individual connections at all. NATS·tonic·grpc-go·etcd·Cassandra/Scylla·MongoDB·reqwest all have this shape. `확실`
- If the N are **separate peers each with a different identity · state** (★**our case**★ — each daemon has its own agent registry) → it splits into an **Endpoint/Connection 2-layer separation, and in that camp reconnection · failover remain the consumer's part.** quinn·iroh are the canonical examples. `확실`

★**Correction — 「정체성이 다른 피어면 재연결·페일오버가 *필연적으로* 소비자 몫」 (if the peers have different identities, reconnection · failover *necessarily* falls to the consumer) is false.**★ A counterexample exists in real form — the Rust `MultiHostClient` of the Microsoft Agent Host Protocol owns N transports and **attaches supervision per host** (each host runs in its own `HostRuntime` task), and the library carries backoff · cancellation · **re-subscription across reconnects** (*"re-subscribes to known URIs across reconnects"*) · and even a **host-marker fan-in** in which events come up carrying a `host_id`. So **the 2-layer-separation camp handing that to the consumer is a property of those libraries, not a necessity of this topology.** `확실` (original text checked)

★**We are the latter topology. And in that topology there is a precedent where the library owns the lifetime** — that is the real basis of candidate D.★

### 5-2. A known shortcoming of the 2-layer model — a pool was added later

`iroh` (the canonical 2-layer) later added a `ConnectionPool`. **The stated reasons (original text checked):** *"opening a new connection every time you do a small exchange with a peer is very wasteful"*, *"Iroh connections are relatively lightweight, but even so you don't want to keep thousands of them open at the same time"*, and the purpose is *"whenever you have a protocol that has to talk to a large number of endpoints while keeping an upper bound of concurrent open connections"*. API = `get_or_connect` (*"will try to get an existing connection from the pool. If there is none, it will create one and store it"*), configuration = `idle_timeout`·`max_connections`·`connect_timeout`, and *"a drop-in replacement for endpoint.connect"*. `확실` (original text checked)

★**Correction — this session earlier reported this to the user as 「소비자가 N 개 핸들을 들고 있으라는 게 부족하다는 자백」 (an admission that having the consumer hold N handles is insufficient), but that is an exaggeration.**★ The reasons the original gives are **waste and an upper bound on the number of connections,** not a failure of the ownership model. The observation that survives is narrower — **a registry that looks up by id and creates when absent ended up being needed on top of the 2 layers.**

### 5-3. The axis that actually divides the API shape — 「핸들이 무엇인가」 (what is the handle)

★**This axis divides the outcome more than models 1/2/3 do.**★

- **Handle = socket** → after a reconnect it is a **new handle** and the consumer must update the map. (quinn·tokio-postgres·tokio-tungstenite)
- **Handle = a command channel to a background task** → **the handle survives across reconnection as is.** (NATS·redis `ConnectionManager`·gRPC `ClientConn`·tonic `Channel`)

★**Correction — do not generalize to 「그러니 핸들은 명령 채널이어야 한다」 (so the handle must be a command channel). There is a counterexample, and it goes the opposite way on purpose.**★ AHP's `MultiHostClient` chooses **generation invalidation** — *"Any `HostClientHandle` you obtained from a previous connection refuses to dispatch on the new one and returns `HostError::HostReconnected`"*. That is, instead of a handle that is immortal across reconnection, it **raises the boundary to the surface.** `확실` (original text checked)

**So this axis is not 「옳은 한쪽」 (one right side) but two defensible ends:**
- **Immortal command handle** — hides the boundary. The consumer code gets shorter, but the loss is not visible (§5-4).
- **Generation-bound handle** — exposes the boundary. The consumer must take on the re-seed decision, but no silent loss occurs.

★**The latter is what engram already does — the incarnation marker (epoch), ADR-0163/0164.**★ So in this repository the generation-bound end is the one that fits the idiom.

### 5-4. ★A risk that collides head-on with our replay invariant★

What the reconnecting wrapper whose shape is closest to ours (desktop → WS daemon) wrote in its own docs — original text checked:
> *"By default, the library is re-transmitting pending calls and re-establishing subscriptions that were closed until it's successful"*
> ★*"subscriptions, which may lose a few notifications when it's re-connecting, **it's not possible to know which ones**."*★

`확실` (original text checked)

★**Independent second confirmation — the same risk is written in production docs in another family too.**★ AHP's multi-host client has both event sources **based on `tokio::sync::broadcast`,** so when the buffer fills it *"drop envelopes on slow consumers"*, and the result is *"A dropped (or missed-because-reconnected) envelope permanently desyncs the mirror for that `(host, channel)` until it's re-seeded from a fresh snapshot"*. ★That is, silent reconnection loss is a real production risk, and that document even **names the remedy — re-seed from a new snapshot**★. `확실` (original text checked)

★**This is the biggest risk of this design.**★ If reconnection is hidden inside the library, **a subscription gap arises in a form invisible to the consumer.** But engram already has invariants that deal with that gap — the replay→live boundary, seq dedup, the incarnation marker. ★**For the new crate to own reconnection, it must take a form in which 「무엇이 유실됐는지 소비자가 알 수 있는」 (the consumer can tell what was lost)**★ (for example, raise the reconnect event to the consumer as an event and let the consumer decide re-subscription, or insert a generation-boundary marker into the stream). **If this requirement is not stated, candidates A and B both inherit this defect as is.**

Side evidence: with the etcd client, **tracking unhealthy endpoints directly was the cause of failure** (because of a fixed blacklist a recovered node stayed unusable), and it was finally simplified to, in paraphrase, not tracking state and just moving on to the next one. `가능성 높음`

### 5-5. Why model 4 (single event-loop polling) is not recommended

The libp2p `Swarm` monopolizes the authority to accept/reject connections, so users could not control inbound per condition and an issue was filed. rumqttc stops entirely when `poll()` stops (*"NOTE Don't block this while iterating"*). `가능성 높음`

---

## §6 Fit with engram constraints

| Constraint (canonical) | A | B | C | D | Note |
|---|---|---|---|---|---|
| **ADR-0129 vocabulary isolation (per type)** | ○ (conditional on a generic tag) | ○ | ◎ | ○ (conditional on a generic tag) | If the tag is not made generic, A·D violate it |
| **ADR-0158 version negotiation = upper layer** | ○ | ○ | ○ | ○ | In all four the transport must not know the version |
| **ADR-0036 single-connection invariant** | ★user decision needed★ | ★same★ | ★same★ | ★same — but D raises N to the front of the API and touches this decision **head-on**★ | Applies regardless of the candidate (§10-1) |
| **ADR-0175 weight clause** | ○ | ★risky★ | ○ | ★★riskiest★★ | The widest ownership scope |
| **ADR-0151 「독립적으로 쓸 수 있는가」 (can it be used independently)** | ○ | ◎ | ○ | ○ (the transport seam keeps independent execution alive) | B passes most strongly |
| **replay/seq invariant** | ★conditional★ | ★conditional★ | ○ | ◎ | In D, exposing generation boundaries is built into the design (§5-3) |
| **`default = []` convention** | ○ | ○ | ○ | ○ | Reason = *a loud failure over a silent regression* (net Cargo.toml) |
| **CI gate name prefix** | ★trap★ | ★trap★ | ★trap★ | ★trap★ | §10-3 |

**One more open item that comes alive immediately** — the feature membership of `frame_port` is on hold together with ADR-0130, and that comment reads *"decide before starting on resume — it is a public API change so it is expensive to do later"*. ★**If the new crate tries to reference the frame-port contract, this open item comes alive at once.**★

---

## §7 Splitting direction by feature — it works, with one condition

### 7-1. Precedent — there is a form almost the same as what we want to do

`hyper` 1.x manifest original text (checked):
```toml
[features]
# Nothing by default
default = []
full = ["client", "http1", "http2", "server"]
client = ["dep:want", "dep:pin-project-lite", "dep:smallvec"]
server = ["dep:httpdate", "dep:pin-project-lite", "dep:smallvec"]
```
★`full` turns on both at once = **the two are not exclusive but additive.** This is the only reason this design holds.★ `확실`

And **evidence in the opposite direction** — `tokio` abandoned separate crates and **migrated** to a single crate + features (2019). Reason: *"Maintaining a large number of crates comes with an increased maintainership burden… Users feel that large number of dependencies == bloat."* `가능성 높음`

`jsonrpsee` is a hybrid — separate crates underneath, and an umbrella on top pulls them in as optional dependencies by feature. ★**If candidate B is chosen, this form is the reference.**★ `확실`

### 7-2. ★An unscalable wall — 「client 는 켜되 server 는 끈다」 (turn client on but server off) cannot be expressed by Cargo★

Cargo book original text (checked):
> *"When a dependency is used by multiple packages, Cargo will use the union of all features enabled on that dependency when building it."*
> *"Features should be additive. That is, enabling a feature should not disable functionality, and it should usually be safe to enable any combination of features."*
> *"There are rare cases where features may be mutually incompatible with one another."* → the only means provided is `compile_error!` = **not preventing the union but killing the build after the union has happened.**

The request for mutually exclusive features has been open since 2016. `확실`

**But 「Cargo 가 표현 못 한다」 (Cargo cannot express it) is on a stable basis** — nightly has `-Z feature-unification`, which can resolve **dependency features separately per package** (by duplicating builds). The conclusion above for stable Cargo stands. `확실`

★**And we cannot control the union**★ — if any one crate in the dependency graph turns on `server`, everyone has it on. What `resolver = "2"` fixes is only the three axes of target-specific · build-dep · dev-dep, and **the union within an ordinary normal-dependency graph does not go away** (RFC 2957 admits it itself: *"The new feature resolver does not address all of the enhancement requests"*). `확실`

### 7-3. Two real incidents

1. **Dependency leak → target build failure** — a case where, even with `default-features = false`, a runtime came along and the wasm target died (tonic#1783). `가능성 높음` (how it was resolved is `모름`)
2. ★**False compile success — the real trap of our design**★ — *"in practice `default-features = false` is almost impossible to use, because any dependency anywhere in the dependency tree that just includes `tokio = "1"` will silently bring all the default features back."* Even if a consumer forgets its own feature declaration, someone else turns it on, so **it compiles for now** → it blows up the day someone else's dependency changes. `가능성 높음`

**The `default = []` comment in net already wrote down the same limit** — *the feature union is per cargo invocation, so a root `cargo build` still builds net once as server because the daemon turns on `server`.* That is, **in this repository the gain of an empty default is 「소비자 쪽 실수를 시끄럽게 만드는 것」 (making consumer-side mistakes loud), not 「빌드 그래프에서 server 를 빼는 것」 (removing server from the build graph).**

### 7-4. Test burden

`cargo hack --each-feature` / `--feature-powerset --depth N`. Practical advice = for a crate with 2 or more features put `--each-feature` in CI, and powerset only for core crates · under 8 features (exponential). `가능성 높음`

★**I could not confirm that 「성숙 crate 들이 실제로 cargo-hack 을 CI 에서 돌린다」 (mature crates actually run cargo-hack in CI) — do not cite.**★

---

## §8 Grounding results (the main session checked primary sources directly)

| Claim | Verdict | How checked |
|---|---|---|
| `hyper` has `default = []` + `client`/`server` + `full` | **supported** | fetched the manifest original — matches the citation byte for byte |
| Cargo feature union · additive · no mutual-exclusion support | **supported** | fetched the Cargo book original, all three citations confirmed |
| `type Tag: Eq` of `tokio-tower` `TagStore` | **supported** | fetched the docs.rs original — confirmed the bound is `Eq` alone |
| tokio-tower leaves the wire implementation to others | **supported** | confirmed the crate-docs original citation |
| the `jsonrpsee` transport trait does not know ids | **supported** | fetched the docs.rs original — confirmed all three methods mention no id |
| notification loss during reconnection + not knowing which | **supported** | fetched the crate-docs original — confirmed the sentence as is |
| `remoc` does not implement the transport | **supported** | fetched the docs.rs original |
| the reason iroh introduced `ConnectionPool` | **partially supported → corrects this session's exaggeration** | the original's reason is **waste · an upper bound on connection count,** not 「소유 모델 실패의 자백」 (an admission of failure of the ownership model) (§5-2) |

**What the collectors themselves flagged as 「출처 없음」 (no source) and this document therefore excluded:** the number of the rule of three · the crates.io distribution of `default = []` · a case of abandoning a client/server split because of union (**not found — do not restate a similar case as this case**) · using Leptos as a maintainer-admission case (**would be fabrication**) · cargo-hack CI use by mature crates · the Rails extraction narrative.

---

## §9 Adversarial review result

**Verdict = ★BLOCK★** (subject to re-judgment after the fix) · reviewer = cross-family, effort high, multiple lenses + candidate ranking · **39 findings.** All the items reflected below were applied to the body, and ★**no re-judgment was run on the revised version**★.

### 9-1. Findings that changed the conclusion (the main session confirmed against the original text)

- ★**Refutation of the two-camp claim**★ (`bevy_simplenet`·`tonic`) — that is why §0 was fixed. Real artifacts that overlap type seam + lifetime exist.
- ★**There is a real artifact where the library owns multi-host lifetime**★ (AHP `MultiHostClient`) — that is why §5-1 was fixed. 「정체성이 다른 피어면 소비자 몫」 (if the identities differ it is the consumer's part) is not a necessity.
- ★**There is a design on the opposite side, generation invalidation**★ (AHP `HostError::HostReconnected`) — that is why §5-3 was fixed. 「핸들은 명령 채널이어야 한다」 is not a generalization.
- **A real precedent for the transport seam** (`quic-rpc`'s `Connector`/`Listener`) — the basis of candidate D. It was separated from `irpc` in §2-2.
- ★**The candidate space was incomplete**★ — **candidate D was missing.** Added to §3.
- ★**「소비자는 패킷 정의만 준다」 is not literally true even in A·B**★ — this report's **own split tables** write codec choice · tag type · tag accessor · address · authentication material · reconnection policy · backpressure policy as the consumer's part. **No candidate achieves the user goal at face value — it is a matter of degree.** → raised as a new decision item in §10.

### 9-2. Citation-accuracy findings (all applied)

- `remoc` has no reconnection · keepalive → **false.** Automatic ping (`connection_timeout`) + a documented reconnecting transport (§2-1).
- The RFC 6455 §10.5 quotation → **an invented paraphrase.** Replaced with the actual clause (§4).
- gRPC backoff constants → a **proposed algorithm,** not a mandatory spec (§4).
- RFC 6455 Pong *MUST* → **not unconditional.** Exempt after receiving a Close frame (§4).
- tokio-proto 「id 타이핑이 폐기 원인」 (id typing was the cause of abandonment) → **the source does not say so.** Downgraded to an observation + `불확실` (§4-1 A).
- tokio-proto 「오류가 사라진다」 (errors vanish) → the original continues with **they reach but are hard to get** (§4-1 A).
- `postcard-rpc` hash key = 「값싼 버전 협상」 (cheap version negotiation) → **it does not replace it.** Non-cryptographic FNV1a · variable key width · a separate version field (§2-3).

### 9-3. Failure modes of each candidate not covered (review findings · this report has no answer)

- **Candidate A** — there is no generation fence · delivery contract for a **send racing with a reconnect.** `미해결`
- **Candidate B** — if pure decisions are split from timers · I/O, the **ownership of atomic teardown/cancellation** stays unspecified. `미해결`
- **Candidate C** — an opaque byte pipe **cannot own the typed app handshake · ready boundary,** and so cannot claim 「패킷 정의만 주면 된다」. `미해결`
- **Common to A·B·C·D** — the fan-out contract (fairness · partial failure · slow consumer · reconnection loss) is unspecified. `미해결`

### 9-4. The review ranking and the main session's handling

Reviewer ranking = **D > A > B > C.**

★**The main session does not adopt this ranking.**★ Choosing the winner is the user's job, and the reviewer did not see engram's internal constraints — **the ADR-0036 single-connection invariant · the ADR-0175 weight clause · the `frame_port` open item.** The ranking is recorded **only as input from an external design viewpoint.**

### 9-5. Findings not adopted

- The reviewer named `ws-bridge`·Lightyear·Naia·`bevy_connect` as missing peers, but **the main session did not check the original text** → recorded as `미검` and **not used as a basis for the conclusion.**
- The remark that **tokio-tower's latest stable release is from 2021** was not fact-checked → `미검`. But the citation in §4-1 **went through an API-docs original check, so the citation itself is valid.**

---

## §10 What the user must decide

> ★It is a heavy design, so the session does not fix anything arbitrarily.★ Item 1 divides the rest.

1. ★**Relation to ADR-0036**★ — whether a shell holding 1 connection each to N daemons is an **extension or an amendment** of that decision. The documents do not answer. This must be decided before the new crate's multi-connection API can be drawn.
2. **Which of candidates A/B/C/D** — where I/O goes + who owns the N peers (§3).
3. ★**How far to compromise 「패킷 정의만 주면 된다」**★ — **it does not hold literally in any candidate** (last item of §9-1). **How much of the codec choice · tag type · address · authentication material · reconnection policy · backpressure policy to accept as given by the consumer** must be decided before the candidate choice for the candidate comparison to mean anything.
4. **How to guarantee loss visibility when the crate owns reconnection** (§5-4) — whether to raise the reconnect event to the consumer, insert a generation marker, or re-seed from a new snapshot.
5. **Whether the handle is a socket, a command channel, or bound to a generation** (§5-3) — whether the handle survives across reconnection is decided here.
6. **The crate name** — ★if a project-neutral name is given, the CI dependency upper-bound gates silently do not look at that crate★ (members are identified by the `engram-dashboard` name prefix). The gate regex must be fixed in the same commit.
7. **The feature membership of `frame_port`** (end of §6) — an open item that comes alive at once if the new crate references that contract.

---

## §11 Rejected candidates → material for ADR rejected alternatives

- **Extend the existing `net` directly** — excluded by user claim 4 (create a separate crate → independent verification first). Also, `net` exists not for sharing but for **breaking a cycle,** and the isolation gate stands to fit that boundary.
- **IDL + code generation** (tonic type) — forces a build step, bad failure mode (a codegen failure shows up only as an `include!` error). This repository already carries a `ts-rs` generated-output sync gate, so it would add one more axis.
- **Fix the transport to QUIC** (★`irpc` type — not `quic-rpc`★) — the `irpc` docs themselves state (paraphrased) that it does not abstract the transport, and explain themselves in contrast to `quic-rpc`. It does not fit claim 1 (the ability to choose the direction).
  - ★**`quic-rpc` is not a rejected candidate — it is the precedent for candidate D.**★ It moves the transport behind `Connector`/`Listener` traits and ships four built-in adapters (flume·hyper·quinn·iroh) (§2-2).
- **Single event-loop polling model** (libp2p Swarm / rumqttc type) — the issue of monopolizing accept/reject authority, full stop when poll stops (§5-5).
- **Bake correlation into the transport as a concrete `RequestId`** (tokio-proto type) — an abandoned road. ★**But do not write it as 「id 타입 고정이 폐기 원인」**★ — the reasons the source gives are complexity · error visibility · HTTP version limits, and what remains is only the **observed shift in API shape** from concrete `u64` to an associated type (§4-1 A). `불확실`

---

## §12 Limits

- ★**The claim 「타입 seam + 수명을 합친 물건이 없다」 (nothing combines type seam + lifetime) has been refuted**★ (§0). The absence check that remains is one narrower thing — **I could not find one that also has an N-peer registry and per-peer identity.** I ran several search axes but it is not exhaustive.
- **The weight of candidates B·D cannot be measured now** — whether they pass the ADR-0175 weight clause is known only by drawing the design.
- **I did not count how tangled `connection.rs` (1,674 lines) actually is** — only the file header · imports were read. It is needed after a candidate is chosen.
- **Merging the two earlier split criteria (`crate-granularity-survey` + `module-split-criteria`) into one has still not been done** — the homework the previous session left stays as is.
- ★**Two review findings left as `미검`**★ (§9-5) — the four missing-peer candidates (`ws-bridge`·Lightyear·Naia·`bevy_connect`) and the tokio-tower maintenance status. **Since the original text was not checked, they are not used as a basis for the conclusion.**
- **Re-judgment not run** — the §9 findings were applied, but a second adversarial review of the revised version did not run.

---

## §13 Sources

★**This is a reproduction list — the URLs the body used as evidence, collected by section.**★ Read any claim not here as not having gone through an original-text check.

**§0 · §2-1 (real artifacts that overlap type seam + lifetime · remoc correction)**
- `bevy_simplenet` — https://docs.rs/bevy_simplenet/latest/bevy_simplenet/
- `tonic` reconnection — https://docs.rs/tonic/latest/src/tonic/transport/channel/service/reconnect.rs.html
- `remoc` `chmux::Cfg::connection_timeout` — https://docs.rs/remoc/0.20.0/remoc/chmux/struct.Cfg.html
- `remoc` 0.20 adaptable/reconnecting transport — https://docs.rs/remoc/0.20.0/remoc/ · https://docs.rs/remoc/latest/remoc/

**§2-2 · §11 (transport seam precedent)**
- `quic-rpc` `Connector`/`Listener` + four built-in transports — https://docs.rs/quic-rpc/latest/quic_rpc/transport/

**§2-3 (hash routing key)**
- `postcard-rpc` `Key` (FNV1a · 1/2/4/8 bytes · separate version field) — https://docs.rs/postcard-rpc/latest/postcard_rpc/struct.Key.html

**§2-4 (sans-IO reuse case)**
- Firezone sans-IO retrospective — https://www.firezone.dev/blog/sans-io

**§4 (boundaries by responsibility)**
- RFC 6455 §10.5 authentication — https://www.rfc-editor.org/rfc/rfc6455.html#section-10.5
- gRPC proposed backoff algorithm — https://grpc.github.io/grpc/core/md_doc_connection-backoff.html

**§4-1 (correlation layer)**
- `tokio-tower` `TagStore` — https://docs.rs/tokio-tower/latest/tokio_tower/multiplex/client/trait.TagStore.html
- `tokio-tower` crate docs — https://docs.rs/tokio-tower/latest/tokio_tower/
- `jsonrpsee` transport trait — https://docs.rs/jsonrpsee-core/latest/jsonrpsee_core/client/trait.TransportSenderT.html
- tokio-proto abandonment discussion — https://github.com/tokio-rs/tokio/issues/1318

**§5 (multi-connection · reconnection loss · generation invalidation)**
- AHP `MultiHostClient` (per-host supervision · re-subscription · `HostError::HostReconnected` · broadcast loss and re-seed) — https://github.com/microsoft/agent-host-protocol/blob/main/clients/rust/MULTI_HOST.md
- A reconnecting wrapper's admission of notification loss — https://docs.rs/reconnecting-jsonrpsee-ws-client/latest/reconnecting_jsonrpsee_ws_client/
- iroh `ConnectionPool` introduction — https://www.iroh.computer/blog/iroh-blobs-0-95-new-features
  - ★**Note: this `ConnectionPool` came out of `iroh-blobs` and now lives in `iroh-util` — it is not the core `iroh`.**★

**§7 (feature axis)**
- `hyper` 1.x manifest — https://github.com/hyperium/hyper/blob/master/Cargo.toml
- Cargo features doc (union · additive) — https://doc.rust-lang.org/cargo/reference/features.html
- nightly `-Z feature-unification` — https://doc.rust-lang.org/cargo/reference/unstable.html#feature-unification
