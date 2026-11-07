# TRD — new `transport` crate (S21)

> Status: **Edition 1 (2026-09-06).** A document that draws one crate, taking ADR-0177~0182 as input.
> **Scope = only step ① of the attach order in ADR-0177 decision 1 (stand up the crate and verify it alone).** Moving code in, applying it to the daemon, and the screen are not covered by this document (§10).
> **How to read:** each block is marked **「지금 확정」** (boundaries, seams, types — expensive to change later) or **「껍데기만」** (internals not yet measured). The decision rule = CLAUDE.md 「아키텍처 원칙 / 0. 판단 기준」 — *judge abstraction not by YAGNI but by **risk × duration**. Low risk + long-lived: lay it down fully now. High cost or uncertain: leave only a shell and fill it in when measured.*
> **Document placement convention:** the **「새 기능 **설계 착수** → `process/SN-name/` 새 폴더」** line of `docs/README.md` 「새 내용을 어디에 넣나」. The folder is the step (the same file's 「문서 종류」 table), and the precedent is `docs/process/S20-command-bus/trd.md`.
> Anchors: **ADR-0177** (the charter of this crate — the inside/outside split) · **ADR-0178** (version mismatch = the daemon handshake decides) · **ADR-0179** (daemon N clients · client N daemons) · **ADR-0180** (notification surface · separating the reconnect layer) · **ADR-0181** (request deadline) · **ADR-0182** (event emission) · ADR-0129 (net boundary) · ADR-0130 (`frame_port` feature undecided — §9-3 confirms it is not woken) · ADR-0046 (replay single-flight) · ADR-0163/0164 (incarnation marker) · ADR-0155 (tool-crate precedent) · ADR-0175 (leaf-crate admission conditions · `replay_flight` purity-gate precedent) · ADR-0012 (module isolation harness) · ADR-0003 (core isolation) · `docs/research/reusable-transport-crate-boundary-2026-09-03.md` · step-log S21.

---

## 0. Canonical summary of this edition

```
① 타입은 제네릭, 자원은 dyn.
     제네릭 = Wire(소비자의 패킷 어휘)        ← 이것이 「패킷 정의만 주면 된다」의 실물
     dyn    = Dialer/Link(전송) · Clock(시간)  ← 갈아끼우는 두 축

② 순수 층과 구동 층을 파일로 가른다.
     machine.rs = 연결 하나의 상태 기계(시간을 인자로 받는다. tokio 0줄 — 게이트가 잰다)
     peer.rs    = 그 기계를 Link·Clock 위에서 돌리는 감독 태스크

③ 상대는 처음부터 N 개다.
     Registry 가 명부를 쥐고, 인바운드는 출처 표식이 붙은 한 줄로 합쳐 올라온다

④ 위로 올라가는 것은 한 스트림뿐이다.
     Incoming = Message(패킷) | Event(사건).  옆 채널을 두지 않는다(ADR-0177 결정 9)
```

**What stays outside** (ADR-0177 decision 4 as is): packet struct definitions · addresses · authentication material · policy values · pairing-number generation · queue-policy choice · version/capability negotiation decision.

---

## 1. Goals · non-goals

### What this crate does

- Frame boundaries · request/response correlation (**generic over the tag type — the crate does not know its name**) · role distinction (request/response/notification/stream)
- Stream lifecycle management = identifier · generation · sequence number · resume · duplicate removal · **finding gaps and re-requesting from that point**
- Connection establishment · **opaque handshake** (any number of round trips) · **bidirectional keepalive** · **write deadline** · **backoff + jitter reconnect**
- **A registry of N peers + supervision per peer** · **origin marker** on inbound · **generation boundary exposed**
- Fan-out · bounded queues and backpressure signals · **structured emission of connection events** (ADR-0182)

### What this crate does not do

Collected separately in §10. If the 「안 하는 것」 are not pinned down as a list, the next session sneaks them in.

---

## 2. Module boundaries and file layout

**Crate name = `engram-dashboard-transport`** (short name `transport`). ★**Basis for the decision and the remaining ambiguity**★ — ADR-0177 decision 1 writes the name only as 「`transport`」, and the same ADR's 「영향」 warns *「crate 이름이 `engram-dashboard` 접두를 벗으면 CI 의존 상한 게이트가 그 crate 를 조용히 안 본다」*. The two lines do not fit each other. This document reads it with **the same convention as the other six members** (the short name in documents and conversation, the prefixed form for the package — `net`·`agent`·`base`·`command` are all like that) and chose the prefixed form, so the gate regex needs no touching. **If you really intend to drop the prefix, see §11-13.**

The file layout follows this repository's lib convention as is — **flat `src/*.rs`, no subdirectories, unit tests inline in `#[cfg(test)]`, integration in `tests/`** (`net` 6 files · `command` 11 files · `messaging` 7 files all have this shape).

| File | What it owns | Standalone verification (ADR-0012) |
|---|---|---|
| `lib.rs` | The crate header = **the canonical home of boundaries · isolation gates · invariants**. Re-exports only, 0 logic | — |
| `frame.rs` | The `Frame` vocabulary (`Text`/`Binary`/`Keepalive`) · `Close` (code + text) · `CloseCode` assignment | inline unit |
| `wire.rs` | ★The `Wire` trait★ — the whole packet vocabulary the consumer provides (§3-1) | inline unit (with a fake Wire) |
| `link.rs` | Transport seam — `Dialer`·`LinkTx`·`LinkRx`·`LinkError` | inline unit |
| `clock.rs` | Time seam — `Clock`·`SystemClock`·`with_deadline` | inline unit |
| `policy.rs` | Policy value structs + defaults (§6) | inline unit (default-invariant assertions) |
| `event.rs` | The event vocabulary raised upward (§7) | — (data types only) |
| `pending.rs` | Request/response correlation — tag-generic pending table · deadline · **rejecting reuse of an expired number** | ★pure★ inline unit |
| `stream.rs` | Stream lifecycle — generation check · sequence-number duplicate removal · **gap detection** · 「유효 하한」 | ★pure★ inline unit |
| `machine.rs` | ★**Pure**★ state machine for one connection. Input event + `Instant` → action list. **0 lines of tokio·I/O** | ★pure★ inline unit — **it takes time as an argument, so not even a harness is needed** |
| `peer.rs` | Supervisor task — runs `machine` on `Link`·`Clock` · single writer · bounded queue · the `Peer` handle | in-memory Link + manual Clock |
| `registry.rs` | Registry of N peers · origin-marked fan-in · fan-out · the single `Incoming` line | in-memory Link ×N |
| `ws.rs` | WS adapter (`Dialer`/`Link` implementation). **feature `ws`** | real socket `127.0.0.1:0` |
| `testing.rs` | Harness — in-memory `Link` · manual `Clock` · recording event collector. **feature `test-support`** | — |

**14 files.** The grounds for seeing no clash with 「구조 작게」: each file owns **one noun**, three (`pending`·`stream`·`machine`) are pure and run without a harness, and two (`ws`·`testing`) are behind features and absent from the default build. The substantive driving code left is the two files `peer.rs`+`registry.rs`.

### Why `machine.rs` is separate

★**Splitting the pure layer by file is this crate's only 「나중에 리팩터 안 하게」 device**★. ADR-0177 rejected candidate B (the core as a separate crate) **on weak grounds** and chose candidate A (splitting by modules inside one crate), and candidate A's known weakness is *「sans-IO 순수성이 한 crate 안에서는 컴파일러 강제 없이 관례로만 버틴다」* (survey §3 candidate A, downside). This repository **already has** a precedent that fills exactly that weakness — the purity gate of `src-tauri/src/daemon_client/replay_flight.rs` (ADR-0175 decision 3, CLAUDE.md 「빌드·검증 명령」). We use the same form as is (§9-2 gate 3).

- **Fixed now:** the pure/driving file split · the gate that guards that boundary · `machine` taking time **as an argument**.
- **Shell only:** the action list inside `machine` itself will grow (cancel, relay, etc.). That is adding variants to an enum, so it does not shake the boundary.

---

## 3. Public interface

★**This section is the body of 「나중에 리팩터 안 하게」.**★ The type names and method set below are **fixed now**, and the body implementation may be torn up freely.

### 3-1. `Wire` — the whole packet vocabulary the consumer provides (fixed now)

```rust
/// 소비자의 패킷 어휘. ★crate 는 이 안의 어떤 이름도 모른다★ — 같은지(Eq)와 있는지(Option)만 본다.
pub trait Wire: Send + Sync + 'static {
    /// 내가 보내는 것.
    type Out: Send + 'static;
    /// 내가 받는 것.
    type In: Send + 'static;
    /// 짝짓기 번호. ★바운드가 Eq + Hash + Clone 뿐인 것이 계약이다★
    type Tag: Eq + std::hash::Hash + Clone + Send + Sync + 'static;
    /// 스트림 하나를 가리키는 키(우리 경우 = 에이전트 하나).
    type StreamKey: Eq + std::hash::Hash + Clone + Send + Sync + 'static;
    /// 디코드 실패 사유. crate 는 사건에 문구로 실을 뿐 분기하지 않는다.
    type DecodeError: std::fmt::Display + Send + 'static;

    fn encode(&self, out: &Self::Out) -> Frame;
    fn decode(&self, frame: Frame) -> Result<Self::In, Self::DecodeError>;

    /// 나가는 것이 답장을 기다리나 — Some 이면 대기 슬롯을 만든다.
    fn request_tag(&self, out: &Self::Out) -> Option<Self::Tag>;
    /// 들어온 것이 누구의 답장인가 — Some 이면 그 슬롯을 깨운다.
    fn reply_tag(&self, inn: &Self::In) -> Option<Self::Tag>;
    /// 들어온 것이 스트림 조각인가 — Some 이면 (스트림, 세대, 순번).
    fn stream_mark(&self, inn: &Self::In) -> Option<StreamMark<Self::StreamKey>>;
    /// 스트림을 이어받는 요청을 짓는다. ★crate 는 그 봉투를 모르고 "언제" 부를지만 안다★
    fn resume_request(&self, key: &Self::StreamKey, after: Option<u64>) -> Self::Out;
}
```

**These six functions correspond to today's code as is (measured):**

| `Wire` slot | Today's actual item |
|---|---|
| `Out`/`In` | `AgentCommand` / `AgentEvent` |
| `Tag` | `protocol::RequestId` (`ids.rs:16` — a uuid wrapper) |
| `StreamKey` | `protocol::AgentId = uuid::Uuid` (`ids.rs:4`) |
| `request_tag` | `protocol::command_request_id` (`messages.rs:653`) — **the canonical one that already exists. Do not create a new one** (ADR-0177 decision 4) |
| `reply_tag` | `protocol::event_reply_request_id` (`messages.rs:711`) |
| `stream_mark` | The output hot path fixed header `[tag:1][agent_id:16][epoch:4 BE][seq:8 BE]` (`protocol/src/codec.rs:3`) — **the three fields are `StreamMark` as is** |
| `encode`/`decode` | There are two codecs — control = JSON (WS Text) · output = binary (WS Binary). ★`Frame` carries both, so one `Wire` absorbs them★ |

★**The role distinction (request/response/notification/stream) is not a new vocabulary but a combination of the accessors above**★ — on the outgoing side, whether `request_tag` is present; on the incoming side, look in the order **`reply_tag` → `stream_mark` → anything else**. **The order is the contract**: if one `In` gives `Some` for both, handle it as a reply first and raise `Event::AmbiguousRole` (a signal of a consumer contract violation).

- **Fixed now:** the trait's six slots and five associated types. **Shell only:** none — this trait must be fully fixed now (adding a slot later breaks every consumer).

### 3-2. Registry and handles (fixed now)

```rust
pub struct PeerId(pub std::sync::Arc<str>);      // 소비자가 붙이는 이름표. 사건의 출처 표식이 이 값
pub struct Address(pub String);                   // 불투명 — crate 는 파싱하지 않는다(Dialer 몫)
pub struct Generation(pub u64);                   // 연결 세대. 단조 증가, 재사용 없음

pub struct Registry<W: Wire> { /* … */ }

impl<W: Wire> Registry<W> {
    /// 명부 하나를 세운다. 인바운드는 **한 줄로만** 나온다.
    pub fn new(
        wire: Arc<W>,
        dialer: Arc<dyn Dialer>,
        clock: Arc<dyn Clock>,
        policy: Policy,
    ) -> (Self, Inbound<W>);        // Inbound = 받는 쪽 손잡이 · Incoming = 그 위로 나오는 것(§3-3)

    pub fn add(&self, id: PeerId, addr: Address, hs: Arc<dyn Handshake>) -> Peer<W>;
    pub fn add_with_policy(&self, id: PeerId, addr: Address, hs: Arc<dyn Handshake>, policy: Policy) -> Peer<W>;
    pub fn remove(&self, id: &PeerId);
    pub fn get(&self, id: &PeerId) -> Option<Peer<W>>;
    pub fn peers(&self) -> Vec<(PeerId, PeerState)>;
    /// 팬아웃 — 전 상대에게 같은 것을 민다. 부분 실패는 상대별 사건으로 올라온다.
    pub fn broadcast(&self, out: W::Out);
}

/// 값싼 clone 핸들 = 감독 태스크로 가는 명령 채널.
/// ★재연결을 가로질러 살아남는다★ — 세대 경계는 핸들을 죽이지 않고 `Incoming` 에 표식으로 나온다.
pub struct Peer<W: Wire> { /* … */ }

impl<W: Wire> Peer<W> {
    pub fn id(&self) -> &PeerId;
    /// 답장을 기다린다. 시한 만료·끊김·큐 포화가 Err.
    pub async fn request(&self, out: W::Out) -> Result<W::In, RequestError>;
    /// 답장을 안 기다린다. 큐에 못 넣으면 즉시 Err(await 없음).
    pub fn notify(&self, out: W::Out) -> Result<(), SendError>;
    /// 스트림을 이어받는다. 반환값이 그 요청의 세대.
    pub fn resume_stream(&self, key: W::StreamKey, after: Option<u64>) -> Generation;
    /// ★「지금 다시 해라」 입구 — 셸이 소유한다★(ADR-0180 결정 4·7). 예산을 초기화하고 즉시 시도.
    pub fn reconnect_now(&self);
    /// 명시 종료 — 다시 붙지 않는다.
    pub fn close(&self);
    pub fn state(&self) -> tokio::sync::watch::Receiver<PeerState>;
}
```

★**The handle being immortal across reconnects and the exposure of generation boundaries are compatible**★. Survey §5-3 set 「불사 명령 핸들」 and 「세대에 묶인 핸들」 as **the two defensible ends**, and noted that this repository's idiom is the latter (incarnation marker, ADR-0163/0164). We choose **an immortal handle, with a generation marker on the data** — consumer code gets shorter, yet no silent loss arises (the boundary shows as `Arrival.generation`). ★This is the answer to the spot that survey §5-4 called 「이 설계의 최대 위험」.★

### 3-3. What comes upward — one line (fixed now)

```rust
/// 받는 쪽 손잡이. 명부 하나당 하나뿐이다 — 갈라 쓰려면 소비자가 갈라 쓴다.
pub struct Inbound<W: Wire> { /* … */ }
impl<W: Wire> Inbound<W> {
    pub async fn recv(&mut self) -> Option<Incoming<W>>;   // None = 명부가 통째로 닫혔다
}

pub enum Incoming<W: Wire> {
    Message(Arrival<W>),
    Event(TransportEvent<W>),
}

pub struct Arrival<W: Wire> {
    pub peer: PeerId,            // ★출처 표식★(ADR-0177 결정 3)
    pub generation: Generation,  // ★세대 경계★ — 이 조각이 어느 연결 세대의 것인가
    pub msg: W::In,
}
```

★**Not having a side channel is the decision**★ — ADR-0177 decision 9 pinned it down as *「신호는 **소비자가 이미 읽는 스트림 안에** 넣는다 — 옆 채널은 안 들으면 없는 것과 같고, NATS 문서가 그것을 자기 함정으로 적었다」*. If only the loss signal did so and the other events stayed on a side channel, half of that trap would remain. **So everything is one line.** The cost = even a consumer that wants only packets writes one extra layer of `match`. (If a log window later has to read events separately, a branch appears — §11-1.)

### 3-4. The two swappable axes (fixed now)

**The rule in one line: types are generic, resources are dyn.**

```rust
// ── 전송 축 ────────────────────────────────────────────────────────────────
pub trait Dialer: Send + Sync + 'static {
    /// 주소 하나로 통로를 연다. 시한은 부르는 쪽(감독)이 건다.
    fn dial(&self, addr: &Address)
        -> BoxFuture<'_, Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), LinkError>>;
}

pub trait LinkTx: Send + 'static {
    fn send(&mut self, frame: Frame) -> BoxFuture<'_, Result<(), LinkError>>;
    /// 살아있나 물어보기. ★"언제" 는 crate 가 정하고 "어떻게" 만 여기가 안다★
    fn ping(&mut self) -> BoxFuture<'_, Result<(), LinkError>>;
    /// 코드와 문구를 실어 닫는다.
    fn close(&mut self, close: Close) -> BoxFuture<'_, ()>;
}

pub trait LinkRx: Send + 'static {
    /// `Ok(None)` = 상대가 정상 종료. keepalive 응답은 `Frame::Keepalive` 로 올라온다.
    fn recv(&mut self) -> BoxFuture<'_, Result<Option<Frame>, LinkError>>;
}

// ── 시간 축 ────────────────────────────────────────────────────────────────
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> std::time::Instant;
    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()>;
}
pub struct SystemClock;  // tokio::time

/// dyn Clock 위에서 도는 시한. trait 메서드로 두면 제네릭이라 dyn 이 깨져서 자유 함수다.
pub async fn with_deadline<F: Future>(clock: &dyn Clock, d: Duration, f: F)
    -> Result<F::Output, Elapsed>;
```

**Why a trait and why dyn — the answer differs per axis:**

- **`Wire` is generic.** If erased, `W::Out`/`W::In`/`W::Tag` disappear and the crate becomes a **byte pipe** — that is candidate C, which ADR-0177 rejected. The type seam is the reason this crate exists, so it is never erased.
- **`Dialer`/`Link` are dyn.** Three reasons. ① ★**Different transports must be able to mix inside one registry**★ — to hold a local WS peer and (later) a TLS peer in the same `Registry`, a generic parameter cannot express it. ② Frames are already `String`/`Vec<u8>` values, so a dyn call per frame is noise next to a syscall. ③ The same repository already has that form — `net`'s `frame_port` erases it with `Arc<dyn FrameSink>` + `BoxFuture` (because `async fn` in a trait is not dyn-compatible, `frame_port.rs:107`).
- **`Clock` is dyn.** If left generic, it spreads to all four type signatures `Peer`·`Registry`·`Pending`·`Stream` and even contaminates the registry's collection type. With only one production implementation, monomorphization gains nothing. The precedent is `crates/engram-dashboard-daemon/src/command_delivery.rs:176` (`pub trait Clock: Send + Sync { fn now(&self) -> Instant; }`) and **this crate adds `sleep` to it** — backoff, keepalive and write deadline need *waiting*, which `now()` alone cannot build.
- ★**Why not substitute `tokio::time::pause()`**★ — ① it is a **runtime-global** facility that can be used only by standing up a runtime, and there is nowhere to apply it in `machine.rs` (the pure layer with no future at all) ② ADR-0177 decision 7 said to make **the transport swappable too**, and tying time to tokio would glue those two back together ③ `test-util` is dev-only, so it may not be enabled in the consumer's harness (`src-tauri/Cargo.toml:109` states that boundary). **But in the real-socket test (§8-2), `pause()` is cheaper, so it is not forbidden.**

### 3-5. Handshake — opaque, any number of round trips, direction-symmetric (fixed now)

```rust
pub trait Handshake: Send + Sync + 'static {
    fn start(&self) -> HandshakeStep;
    fn on_frame(&self, frame: &Frame) -> HandshakeStep;
}

pub enum HandshakeStep {
    Send(Frame),                                   // 보내고 계속
    Await,                                         // 상대의 다음 프레임을 기다린다
    Done,                                          // 운영 단계로
    Reject { last: Option<Frame>, reason: String },// ★붙었는데 거절★ — last 를 보내고 닫는다
}
```

- **The crate does not count round trips** — it accepts however many times `Send`/`Await` are returned. Today the client side is `Send(auth) → Await → (Hello) → Done`, two steps (`connection.rs:411`·`1644-1674`).
- **Direction-symmetric** — the daemon side uses the same trait with `start() = Await`, looks at the token and version in `on_frame`, and gives `Done` or `Reject`. ★**So ADR-0178's 「판정 주체 = 데몬 단독」 holds without touching the crate**★ — the version vocabulary lives only inside the `Handshake` implementation and the crate still does not know it (the ADR-0129 vocabulary isolation is maintained).
- ★**`Reject` and 「못 붙음」 are different things, and that distinction splits the budget**★ — ADR-0180 decision 5. The §5 state machine enforces it.

### 3-6. Error vocabulary (fixed now)

```rust
pub enum RequestError {
    /// 시한이 지났다. ★자동 재전송하지 않는다 — 무응답은 "실패"가 아니라 "모른다"다★(ADR-0181 결정 3)
    TimedOut { after: Duration },
    /// 보내기 전에 연결이 끊겼거나, 보낸 뒤 답장 전에 끊겼다.
    Disconnected { generation: Generation, cause: DisconnectCause },
    /// 나가는 큐가 찼다.
    QueueFull,
    /// 같은 번호가 겹쳤다 — 옛 대기자가 이것으로 깨어난다(오늘 동작 보존).
    Superseded,
    /// 이 명령은 답장을 안 받는다(`Wire::request_tag` 가 None). `notify` 를 쓰라는 뜻.
    NotARequest,
    /// 만료됐던 번호를 다시 썼다(§4-2).
    TagRetired,
}

pub enum ConnectFailure {
    Unreachable(LinkError),                              // 아예 못 붙었다 → 예산을 쓴다
    Rejected { code: Option<CloseCode>, reason: String },// 붙었는데 거절 → 예산을 안 쓰고 즉시 포기
}
```

`RequestError::NotARequest` preserves today's behavior — `mod.rs:704-706` already rejects with *「send_command: request_id 없는 명령은 reply 를 기대할 수 없다」*. ★And this closes the `CommandOutcome` clause of ADR-0181 「영향」 structurally★: since `command_request_id` gives `None` for that variant, putting it into `request()` yields an **immediate `NotARequest`, not a permanent wait**.

---

## 4. Correlation · streams

### 4-1. Request/response (fixed now)

- **The wait table = `HashMap<W::Tag, Waiter>`**, owned solely by the supervisor task (no lock). The same shape as today (`protocol_state.rs:71` `PendingMap<T>`, which `connection.rs:580` passes by `&mut`).
- **Register **before** sending** — today too (`connection.rs:1030-1036`). If reversed, a fast reply cannot find its slot.
- ★**A colliding number = wake the old waiter with `Superseded` and also fail the new request with `TagRetired`**★ (confirmed — user decision 2026-09-06, adversarial review finding **M1**). The moment they collide, that number is **retired** and neither side stays in the table.
  - ★**This line was overturned**★ — edition 1's 「옛 대기자를 `Superseded` 로 깨우고 **새 것이 승계** — 오늘 동작 보존(`connection.rs:1445-1460`)」 was reversed by M1. The half that was kept is 「옛 대기자 = `Superseded`」, and the half that was overturned is 「새 것이 승계」.
  - **Why** — keeping succession leaves **a path where the late reply to the earlier request is delivered as the pair of the later request** (`route` finds the pair only by `pending.take(&tag)`, and since the correlation key is that one number alone, the wire has no material to tell two replies apart). ★**Receiving a wrong answer is worse than receiving a failure**★ — so rather than misdeliver, fail both.
  - **Cost** — ★on this path, today's shell behavior is not preserved★. A consumer that reuses a live number now receives **two failures instead of one success**. In exchange, misdelivery becomes structurally impossible (only for live numbers — the limit on the forgotten-number side is canonical in §4-2 and the `Pending` rustdoc).
  - Regression net = `pending::tests::a_reply_for_a_collided_tag_reaches_nobody`.
  - **The decision record is not this document** — it is organized separately. What is written here is only the content confirmed as a TRD.
- **On disconnect, wake all waiters with `Disconnected`** — preserves today's behavior (`connection.rs:621-626`). ★**So the only case where the deadline actually arrives is 「연결은 멀쩡한데 그 요청만 침묵」**★ (ADR-0181 context as is).

### 4-2. Deadline and number reuse (fixed now · values in §6)

- **One common deadline regardless of command, default 8 seconds** (ADR-0181 decision 1). On expiry it wakes **only that request** with `TimedOut` and **leaves the connection as is** (decision 2).
- **No automatic resend** (decision 3). **A reply that arrives late after expiry is discarded** (decision 4) — since there is no slot, that happens automatically.
- ★**「그 번호는 재사용하지 않는다」 can be only half enforced by the crate**★ — the real guarantee lies with **the side that makes the numbers** (we use uuid v4, `ids.rs:16`). All the crate can do is **remember recently expired numbers and reject with `TagRetired` if one comes again**, and since memory cannot be infinite it is a **bounded ring** (default 256, ★unverified★). This limit is written in the `lib.rs` header — if not written, the next session reads this ring as a guarantee.
- **The 「기다리는 중」 indicator (1-second proposal) is outside the crate** (ADR-0181 decision 5) — display and giving up are different layers, and changing one side's value does not drag the other along with it.

### 4-3. Stream lifecycle management (fixed now · the re-request policy is a shell)

```rust
pub struct StreamMark<K> {
    pub key: K,
    /// 화신 표식. ★비교는 일치/불일치만★ — 대소로 "더 새 것"을 유도하지 않는다(ADR-0163)
    pub generation: u32,
    pub seq: u64,
}
```

What the crate holds per stream: **the last generation · the last sequence number**. On top of that it does four things.

1. **Duplicate removal** — if `seq <= last`, discard it (lifting what the frontend does today into the crate).
2. **Generation mismatch** — if `generation != last_generation`, **reset** the sequence check and raise `StreamGenerationChanged`. ★Only match/mismatch is looked at★.
3. ★**Gap detection**★ — if `seq > last + 1`, raise `StreamGap`. **Today this check does not exist at all** (ADR-0177 rationale: *「현행은 순번 구멍 검사가 아예 없고 단조 dedup 만 있다」*).
4. ★**Re-request from that point**★ — on meeting a gap, build `Wire::resume_request(key, Some(last))` and send it out automatically. **It does not stop at detection** (ADR-0177 decision 9).

**The truncation signal is not true/false but 「지금 유효한 가장 오래된 번호」** (ADR-0177 decision 9) — `StreamTruncated { oldest_valid }`. ★**The wire already carries that value**★: `AgentEvent::SubscribeAck` has `oldest_seq`/`latest_seq`/`replay_from` fields, but **the shell does not read them** (`connection.rs:1297-1309` takes out only `current_epoch` and `truncated` — static reading). That is, the path by which the crate obtains this value is already open through the consumer's `Wire`.

- **Shell only:** **how many times** to re-request for one gap. For now it is **once per gap**, and if that re-request meets another gap, it only raises the event and stops. The runaway-prevention policy is filled in when measured.
- ★**The outgoing-side stream (the side where the daemon stamps sequence numbers) is outside the crate**★ — today `OutputCore` stamps it, and that is agent vocabulary.

---

## 5. State machine (fixed now)

**The lifetime of one connection.** `machine.rs` holds it as a pure function — input = (current time, event) · output = action list.

```
                      ┌──────────────────────── close() ────────────────────────┐
                      │                                                          ▼
  Idle ──start──► Dialing ──열림──► Handshaking ──Done──► Live ──끊김/침묵──►  (예산 판정)
    ▲                 │                   │                                      │
    │            Unreachable          Reject                                     │
    │                 │                   │                            ┌─────────┴─────────┐
    │                 └──────┐            │                        예산 남음            예산 소진
    │                        ▼            ▼                            │                  │
    └────────────────── Backoff ◄─────────┼────────────────────────────┘                  │
                             │            │                                               │
                       (지터 섞은 대기)   │                                               │
                             │            ▼                                               ▼
                             └──────► GaveUp(Rejected)                        GaveUp(BudgetExhausted)
                                              │                                            │
                                              └────────── reconnect_now() ─────────────────┘
                                                             (예산 초기화 → Dialing)
```

| Transition | Trigger | Note |
|---|---|---|
| `Idle → Dialing` | `Registry::add` or `reconnect_now()` | Reset the budget to 0 |
| `Dialing → Handshaking` | `Dialer::dial` succeeds | `connect_timeout` wraps this span |
| `Dialing → Backoff` | `dial` fails/times out = `Unreachable` | ★Spends 1 budget★ |
| `Handshaking → Live` | `HandshakeStep::Done` | Generation +1, `Connected` event |
| `Handshaking → GaveUp(Rejected)` | `HandshakeStep::Reject` **or** the peer sends a rejection frame + close | ★**Does not spend budget. Gives up immediately on the first rejection**★ (ADR-0180 decision 5) |
| `Live → Backoff` | Peer closed · `LinkError` · **silence** (`idle_timeout`) · **write deadline exceeded** · **queue saturation** (when the policy is `Disconnect`) | Spends 1 budget · all waiters get `Disconnected` |
| `Backoff → Dialing` | The wait expires | Cut `base · 2^attempt` at `cap` and mix in jitter |
| `Backoff → GaveUp(BudgetExhausted)` | `attempt >= max_attempts` | ★The app shows a 「다시 연결」 button here (ADR-0180 decision 7)★ |
| `* → Closed` | `Peer::close()` | Does not reconnect |

**How the N-peer registry sits on top:** one such machine + one supervisor task per peer. **Peers do not share state** — even if daemon A is `GaveUp`, B is `Live`.

### ★The open question of ADR-0180 is closed here★

That ADR left open *「팝업 계기를 연결마다 볼 것인가 앱 전체로 볼 것인가」*. **User decision (2026-09-06): the popup decision is on an app-wide basis — as long as even one attached daemon remains, only a strip appears, and the popup only when all are lost. When one connection dies, press it right there to reattach (a per-connection reconnect affordance).**

★**But this crate does not make that decision**★ — 「앱 전체」 is a screen concept, and the crate only counts peers and does not know the app. The crate gives two materials: `Registry::peers() -> Vec<(PeerId, PeerState)>` (so the app-wide state can be folded) · `Peer::reconnect_now()` (the place the per-connection affordance calls). **The shell folds with those two.** *(This paragraph is a record for the ADR update — this document does not edit the ADR-0180 body.)*

---

## 6. Policy value table

★**All have crate defaults and the consumer overrides them**★ (ADR-0177 decision 4). `Registry::new` takes the default, and `add_with_policy` takes the per-peer override.

| Name | Default | Who overrides | Basis / verification status |
|---|---|---|---|
| `request_timeout` | **8s** | Shell (settings file) | ADR-0181 decision 1 = user judgment. ★**Unverified — the real time an agent spawn takes has never been measured**★ (that ADR noted it as the only substantive risk) |
| `connect_timeout` | **10s** | Shell | Current value preserved — `connection.rs:71` `HANDSHAKE_TIMEOUT = 10s` (wraps both the dial and the Hello wait) |
| `write_deadline` | **5s** | Shell·daemon | ★**Unverified — no measurement. Industry reference values only**★ (NATS 10s · nginx 60s · gRPC keepalive timeout 20s, ADR-0177 rationale). Invariant: `write_deadline < ping_interval` |
| `ping_interval` | **20s** | Shell·daemon | Current daemon value preserved — `net/src/ws.rs:51` `DEFAULT_PING_INTERVAL` |
| `idle_timeout` | **50s** | Shell·daemon | Current daemon value preserved — `net/src/ws.rs:52-53`, 2.5 times the ping period |
| `reconnect.max_attempts` | **3** | ★**The shell owns·injects**★ | ADR-0180 decision 4 (user decision). Today's code has **5** (`connection.rs:77` · `src/api/wsTransport.ts:352`) — §11-6 |
| `reconnect.base` | **500ms** | Shell | Current value preserved — `connection.rs:80` `BACKOFF_BASE` |
| `reconnect.cap` | **10s** | Shell | Current value preserved — `connection.rs:83` `BACKOFF_CAP` |
| `reconnect.multiplier` | **2.0** | Shell | Current value preserved — `connection.rs:87-92`. 3 attempts = 500ms→1s→2s = **3.5 seconds total** (the figure ADR-0180 cited) |
| `reconnect.jitter` | **0.2** | Shell | ★**Unverified — borrowed the example value of the gRPC 「제안」 algorithm**★. That document itself titles it *"Proposed Backoff Algorithm"* and allows alternatives (survey §4). **Today there is no jitter at all** |
| `outbound_queue` | **512** | Shell·daemon | Current value preserved — `src-tauri/src/daemon_client/mod.rs:473` `mpsc::channel(512)` |
| `inbound_queue` | **512** | Shell·daemon | ★**Unverified — there is no counterpart today**★ (the shell goes straight to the Tauri Channel, so this queue itself does not exist). §11-10 |
| `full_queue` | **`Disconnect`** | Shell·daemon | ADR-0177 decision 6 (keep current). ★**But 「현행」 is not one thing — §11-2**★ |
| `retired_tags` | **256** | Shell | ★Unverified★ — the bounded ring of §4-2 |

★**Why bidirectional keepalive is not an 「개선」 but 「없던 절반을 채우는 것」 (measured — static reading)**★ — today the shell has **neither an active ping nor an idle timer.** The read-loop comment says *「Ping/Pong 은 tungstenite 가 자동 응답(내부)」* (`src-tauri/src/daemon_client/connection.rs:1008`) and that is all — that is, the shell **only answers automatically, never asks by itself, and has no clock measuring that the peer has gone quiet.** While the daemon-side 20-second ping keeps coming, that proves liveness on its behalf, but ★**if the daemon dies or the line is cut, that ping itself stops coming and the shell receives no signal at all**★. This is exactly the spot targeted by the ADR-0177 rationale *「물리 끊김은 스스로 물어봐야 안다」*.

★**One cell shakes another document's rationale**★ — the ADR-0177 rationale extracted *「큐 상한이 되감기 링보다 크면 안 되는데 현행이 거꾸로다(`CONN_TX_CAP = 4608` > replay `max_events = 4096`)」*. That 4608 is the **daemon-side per-connection queue** (`net/src/ws.rs:47`) and the 512 in the table above is the **shell-side command queue**, so they are not the same cell. **When this crate is attached to the daemon (step ④), the default at that spot must be set at or below the rewind ring** — this document does not decide it (§11-3).

**Jitter randomness source (internal decision · report):** ★No new dependency is brought in★ — mix a hash of `PeerId` into the low-order nanoseconds of `Clock::now()`. There is a precedent: `crates/engram-dashboard-agent/src/profile.rs:680` writes *「난수 crate 를 들이지 않으려고 uuid v4 의 바이트를 쓴다」* and does so. **Two limits are noted** — ① it is not cryptographic randomness (jitter does not need it) ② under a manual clock it becomes deterministic (a property the harness **wants**, but therefore the jitter's *spread* is observed only on a real socket). Left open at §11-9.

---

## 7. Event list (the shape is fixed now · the items will grow)

```rust
pub enum TransportEvent<W: Wire> { /* 전부 peer: PeerId 를 갖는다 */ }
```

| Event | What it must carry | Why it is needed |
|---|---|---|
| `Connected` | `peer` · `generation` | The start of a generation boundary |
| `Disconnected` | `peer` · `generation` · `cause` (peer closed / error / silence / write deadline / queue saturation / explicit close) | ★Today this distinction does not exist — everything is one lump of 「끊김」★ |
| `ConnectFailed` | `peer` · `attempt` · `of` · `cause` · `retry_in` | ★**「몇 번째 재시도인가」** (ADR-0182 decision 1). Today the reason during a reconnect is left only as `debug!` and discarded★ |
| `Rejected` | `peer` · `code` · `reason` | ★**「왜 거절당했나」**★. The version-rejection text of ADR-0178 comes up through this field and becomes the popup wording |
| `GaveUp` | `peer` · `reason` (budget exhausted / rejected) | The input by which the shell decides popup eligibility (ADR-0180 decision 2) |
| `StreamGap` | `peer` · `stream` · `generation` · `expected` · `got` · `resumed_from` | ★**「어디서 몇 개를 잃었나」**★ + what was asked for again |
| `StreamTruncated` | `peer` · `stream` · `generation` · **`oldest_valid`** | ★Not true/false but a **number**★ (ADR-0177 decision 9). Today it appears only as a browser console warning |
| `StreamGenerationChanged` | `peer` · `stream` · `from` · `to` | The incarnation changed — meaning the sequence check was reset |
| `RequestTimedOut` | `peer` · `tag` · `after` | The 8-second deadline actually bit |
| `Dropped` | `peer` · `direction` · `count` | When the queue policy is `DropAndReport`. ★**A field for not creating silent loss**★ |
| `DecodeFailed` | `peer` · `generation` · `reason` (string) | `Wire::decode` failed. **Whether to cut the connection is policy** — §11-11 |
| `AmbiguousRole` | `peer` | One `In` was reported as both a reply and a stream fragment (a §3-1 contract violation) |

★**This list is the crate boundary**★ (ADR-0182 「영향」) — agent errors and command failures are **not here**. They are things the upper layer knows, and merging them is done later by the screen.

**Side benefit:** these events are the **observation points** of the §8 in-memory verification — as ADR-0182 wrote, the harness of ADR-0177 decision 8 gets 「무엇을 보고 판정할지」 from here.

---

## 8. Error · close vocabulary (fixed now)

```rust
pub struct Close { pub code: CloseCode, pub reason: String }
pub struct CloseCode(pub u16);   // WS 4000~4999 = 애플리케이션 구간(RFC 6455)

impl CloseCode {
    pub const GOING_AWAY: Self          = Self(4000); // 내 쪽이 정상 종료한다
    pub const HANDSHAKE_REJECTED: Self  = Self(4001); // 핸드셰이크에서 거절했다(사유 판정은 소비자)
    pub const KEEPALIVE_TIMEOUT: Self   = Self(4002); // 상대가 조용해졌다
    pub const WRITE_DEADLINE: Self      = Self(4003); // 쓰기 시한을 넘겼다
    pub const QUEUE_FULL: Self          = Self(4004); // 느린 소비자 — 큐가 찼다
    pub const PROTOCOL_ERROR: Self      = Self(4005); // 디코드가 깨졌다
    /// 소비자 전용 구간. crate 는 4000~4099 만 쓴다.
    pub const CONSUMER_BASE: u16 = 4100;
}
```

- ★**Carry both the code and the text, and branch only on the code**★ (user decision). The text is for humans to read and **changes without notice** — the `reason` comment of today's `AgentEvent::SubscribeFailed` already says so.
- **Carrying the code in the close frame removes the 「오류 프레임보다 닫기가 먼저 도착」 race.** Today the reason goes separately as a **Text frame** and the close has no code (`net/src/ws.rs:439-447` `send_error_and_close`), so that race is real.
- ★**We do not hide the limit**★ — RFC 6455 §7.4.1 pins `1006` as a **reserved value that cannot be sent on the wire**, and §7.1.5 defines it as a value the receiving side synthesizes locally (ADR-0177 rejected alternative). **That is, when we cut with `QUEUE_FULL`·`WRITE_DEADLINE`, the peer is unlikely to see that code** — because there is no room to push a close frame into a congested socket. **The place where the code is actually delivered is the handshake rejection** (nothing is backed up then — the ADR-0178 rationale states this distinction).

**「거절」 vs 「못 붙음」:** `ConnectFailure::Rejected` = the channel opened and the peer did not accept (no budget spent) · `Unreachable` = the channel itself did not open (budget spent).

★**Today these two burn the same budget (measured — static reading)**★ — the failure branch of the reconnect loop is the single `HandshakeOutcome::Err(e)`, and the comment at that spot itself says *「시도 실패(데몬 죽음/**거부**) — 다음 백오프로」* (`src-tauri/src/daemon_client/connection.rs:805-808`). The reason `e` is left only as `tracing::debug!` and discarded. ★**This is the answer to item 2 of ADR-0178 「검증 안 된 것」 (「거절당한 뒤 클라가 무한 재접속을 반복하는지」) — not infinite: it burns the 5-attempt budget and then settles into `Down`**★ (`connection.rs:77` · `:819-829`). And the same ADR's unverified item 1 (「클라가 거절 사유 프레임을 실제로 받아 읽는지」) is also answered with **it reads** — `wait_for_hello` types an `AgentEvent::Error` that arrives before Hello as `HandshakeError::AuthRejected(message)` and raises it (`connection.rs:1657-1659` · enumeration at `:111-129`). ★**But that text reaches the screen only on the first connection and not during a reconnect**★ — the `debug!` branch above is that spot.

---

## 9. Verification plan

### 9-1. What is measured where

**What is measured in-memory with time injection (everything)** — ADR-0177 decision 8. Backoff, keepalive and write deadline are **not actually waited for.**

| Target | Assertion |
|---|---|
| `machine.rs` | Every transition in the §5 table. **No future and no runtime** — it is a pure function `(now, event) → actions`, so not even a harness is needed |
| `pending.rs` | A deadline expiry wakes only that request · a late reply is discarded · a colliding number wakes the old waiter with `Superseded` · reuse of an expired number gives `TagRetired` · **when the ring overflows it forgets silently** |
| `stream.rs` | Duplicate removal · gap detection · one re-request per gap · a generation mismatch resets the sequence · `oldest_valid` delivery |
| `peer.rs` (+ in-memory `Link` + manual `Clock`) | The backoff schedule is 500/1000/2000 · **a rejection does not spend budget** · budget exhaustion gives `GaveUp` · `reconnect_now()` resets the budget · silence cuts at `idle_timeout` · both queue-saturation policies |
| `registry.rs` | The origin marker is attached · when one peer dies the rest live · a partial fan-out failure arrives as per-peer events · inbound is merged into **one line** |
| `event.rs` overall | Asserts all of the above **as events** (ADR-0182 「부가 가치」 — this crate's first consumer is the test) |

**What is measured only on a real socket (a few)** — placed in `tests/`. The port is **requested as 0**.

1. **Does the WS adapter actually attach** — `tests/ws_dial.rs`. Connect to a hand-built tungstenite server and round-trip the two handshake steps.
2. ★**Does the write deadline actually cut when the peer does not read**★ — `tests/ws_write_deadline.rs`. **It needs a kernel buffer to measure, so it cannot be measured in-memory** (ADR-0177 decision 8 pinned it so).
3. **Version-rejection round trip** — `tests/ws_reject.rs`. The server sends a rejection frame + close code, and the client gets `ConnectFailure::Rejected{code, reason}` and settles into `GaveUp(Rejected)` **without burning budget**.

★**Do not append `-- --test-threads=4`**★ — this crate starts no child process at all (sockets only). This is exactly the decision rule of CLAUDE.md 「빌드·검증 명령」, the same position as `command`·`protocol`·`messaging`·`net`.

### 9-2. Gate draft

**Follow the form of the existing gates as is** (the `base`·`net` steps of `.github/workflows/ci.yml` are the model). The **canonical home of expected values and rationale is the header of `crates/engram-dashboard-transport/src/lib.rs`**, CLAUDE.md has only the rule and the `/qa` binding has an execution copy — counts are not counted in this document.

```bash
# 게이트 1 — 직접 워크스페이스 의존 상한 (정확히 1줄 = 자기 자신)
#   ★매니페스트 텍스트 grep 으로 바꾸지 말 것★ (rename·테이블 형·build-dep·비활성 target·optional 이 빠져나간다)
cargo tree -p engram-dashboard-transport --depth 1 --prefix none \
  -e normal,dev,build --target all --all-features | rg "^engram-dashboard" | sort -u

# 게이트 2 — Tauri import 0 (ADR-0003)
#   ★"워크스페이스 의존 0이니 안전하다" 로 지우지 말 것★ — 게이트 1 은 rg "^engram-dashboard" 로
#   워크스페이스 멤버만 세므로 서드파티 tauri 는 그대로 통과한다. 축이 다르다(base 헤더와 같은 논거).
rg "^\s*use tauri" crates/engram-dashboard-transport/src/           # → 0줄

# 게이트 3 — machine.rs 순수성 (ADR-0175 결정 3 의 replay_flight 게이트와 같은 형태)
#   ★use 라인만 앵커하면 인라인 완전경로 한 줄에 뚫린다★ — 접두 (?:[^/]|/[^/])*? 가 「아직 // 를
#   안 만났다」를 뜻해 주석 밖만 잡고, 그래서 이 헤더의 자기인용이 오탐되지 않는다.
test -f crates/engram-dashboard-transport/src/machine.rs && \
rg "^(?:[^/]|/[^/])*?\b(tokio|futures_util|futures|std::net)::" \
   crates/engram-dashboard-transport/src/machine.rs                 # → 0줄

# 게이트 4 — 두 feature 조합이 각각 컴파일된다 (net 게이트 5 와 같은 근거)
#   출력이 아니라 성공 여부로 판정한다. 가려진 코드는 오류를 내지 않으므로 두 줄이 필요하다.
cargo test -p engram-dashboard-transport
cargo test -p engram-dashboard-transport --all-features
```

★**Gate 1 is the only wall for 「워크스페이스 의존 0」**★ — the compiler does not enforce it. `command`·`base`·`messaging` have the same line, and they also inherit as is the **hole all three share** (members are identified by the `engram-dashboard` **name prefix** — a member with another name simply passes).

★**No source-regex pair is placed**★ — `messaging` also carries a regex with a hand-written crate-name alphabet, but for this crate the invariant is that it **calls no one**, so there is no alphabet of called names to manage (the `base` header does not place one, for the same reason).

### 9-3. Workspace registration

```toml
# Cargo.toml (루트) — 멤버 10
members = [
    "crates/engram-dashboard-protocol",
    "crates/engram-dashboard-base",
    "crates/engram-dashboard-agent",
    "crates/engram-dashboard-command",
    "crates/engram-dashboard-discovery",
    "crates/engram-dashboard-messaging",
    "crates/engram-dashboard-net",
    # ADR-0177: 재사용 전송 lib(워크스페이스 crate 의존 0 · 타입 seam 은 Wire 제네릭 · 전송/시간은 dyn).
    #   ★기존 net 을 뜯지 않는다★ — 새로 세워 독립 검증한 뒤 단계별로 붙인다.
    "crates/engram-dashboard-transport",
    "crates/engram-dashboard-daemon",
    "src-tauri",
]
```

CI adds one line `cargo test --locked -p engram-dashboard-transport --all-features` to the `backend` job, and adds the four gates above to the `fmt + isolation gates` job. **The workspace regression (`cargo test --workspace`) picks up this crate automatically, so there is nothing to touch there** — but **the test-binary count grows from 43** (the canonical home of that figure is CLAUDE.md 「빌드·검증 명령」 and it is not counted here).

★**ADR-0130's undecided `frame_port` feature does not wake up**★ — ADR-0177 「영향」 warned *「새 crate 가 그 계약을 참조하려 하면 즉시 걸린다」*, but this design does **not reference** `net::frame_port` (workspace dependency 0). It newly defines `Frame` and `Link` as its own, and that the two **overlap in shape with net's is intentional** (later, when net is retired at step ④, one side disappears).

---

## 10. ★What we do not do★

| What we do not do | Why |
|---|---|
| **Screen · log window** | ADR-0182 decision 2. *"올리는 것까지 해야지. 화면 만드는 건 나중에 view에서."* ★Since we only raise and there is no screen, the user-visible change is 0 — that is the intent★ |
| **Moving the shell onto this crate** | Attach order ②. This scope is only ① |
| **Filling `Registry` with N and using it** | Attach order ③. **The API is N now, but the shell actually holding several is later** |
| **Application on the daemon side** | Attach order ④ |
| **Modifying the existing `net`** | ADR-0177 decision 1 — ★not torn up in this work★ |
| **Relaying messages between daemons** | Addressing, loop-back prevention and an absent peer are all undesigned. Later research (§12-1) |
| **Remote authentication · pairing · TLS · remote address discovery** | ADR-0177 「미룬 것」. Authentication is **the same problem as multi-client identity, so they are solved together** |
| **Per-client rewind · multi-client input authority · screen-state sync** | ADR-0179 「영향」 left all four undecided |
| **Pairing-number generation** | The canonical one is in `protocol` (`RequestId::new`) |
| **Envelope struct definition** | ADR-0177 decision 5 — ★the envelope is owned by the consumer★. It does not change wire bytes |
| **Version·capability negotiation decision** | The share of the consumer's (daemon's) `Handshake` implementation (ADR-0178) |
| **Duplicate-absorption (idempotency) store** | ADR-0162 closed it with 「결과 저장소 대신 상태 조회」 |
| **Port file · single instance** | Not transport but daemon lifecycle management. discovery already uses it |
| **ts-rs bindings / `bindings/`** | This crate produces no generated artifacts — do not put it in the CI sync gate's path list |
| **Splitting direction with `client`/`server` features** | ★Direction is not a separate layer but a **capability you choose**★ (ADR-0177 decision 3) — `dial` vs. `accept` is a runtime choice, not a compile-time one. So it never hits survey §7-2's **feature-union wall** (Cargo cannot express 「client 는 켜되 server 는 끈다」) |

---

## 11. Dependencies

| crate | Position | Why |
|---|---|---|
| `tokio` (`rt`·`sync`·`time`·`macros`) | **Unconditional** | Supervisor task · timer · channels. All three consumers (daemon·shell·mobile) are tokio, so making the runtime optional buys **reusability nobody uses** (ADR-0177 decision 2 narrowed the scope to those three) |
| `futures-util` | **Unconditional** | `BoxFuture`·`select`. Needed because the dyn seam cannot use `async fn` in trait (net uses it for the same reason) |
| `tokio-tungstenite = "0.26"` | **feature `ws`** | Match the version to the workspace (measured — `net`·`daemon`·`src-tauri` are all `0.26`) |
| — | — | ★**Do not bring in `tracing`**★ — the event stream is that place (§7). **Cost:** while the consumer does not read events, **no trace is left at all.** The consumer bridges events to `tracing`. Left open at §12-10 |
| — | — | No `serde` (serialization is `Wire`'s share) · no `thiserror` (the error types are few, so `Display` by hand. `net` is the same) · no `rand` (§6 jitter) · no `uuid` (`Tag`·`StreamKey` are generic, so the crate does not see uuid) |

**Workspace crate dependencies = 0.** The gate = §9-2 gate 1 (exactly 1 line = itself).

**features:** `default = []` · `ws` · `test-support`. The reason for `default = []` is the same as `net` — *a noisy failure over a silent regression*. **But in this repository, the real gain of an empty default is 「소비자 쪽 실수를 시끄럽게 만드는 것」, not 「빌드 그래프에서 빼는 것」** (feature union is per cargo invocation, so it is turned on in the root build anyway — survey §7-3). `test-support` follows `command`'s precedent (`crates/engram-dashboard-command/src/testing.rs:1` — *「하네스 전용 — 기능 플래그 `test-support` 뒤에 산다(ADR-0012)」*).

---

## 12. Open questions

★**We do not invent answers to close these. Everything below is either for the user to decide, or only recorded until decided.**★

### Newly opened by this design running into things

1. **Is flowing events in one line with inbound enough, or do we place one more broadcast for a later log window.** One line keeps ADR-0177 decision 9 literally, but if the consumer consumes the events, **a second reader (the log window) cannot see them.** Placing another broadcast reopens that trap.
2. ★**Slow-consumer policy — which current behavior does 「현행 유지」 mean.**★ ADR-0177 decision 6 wrote 「차면 끊는다(현행 유지)」, but **today `net` uses two policies at once**: a per-connection `try_send` failure **cuts** (`ws.rs:171-179`), and the fan-out `broadcast_text` **silently drops only that connection's share** (`ws.rs:132-146` · `frame_port.rs:67-77`). The latter is recorded in `docs/tracking.md` as a real defect (registry-frame loss → a live slot stays covered by its exit overlay). **Which way do we set the crate's fan-out default.**
3. **The queue-limit default.** The shell-side 512 is safe because it preserves the current value, but **when attaching to the daemon side (step ④) it must be set at or below the rewind ring** (the 4608 > 4096 extraction in the ADR-0177 rationale). Decide now or then.
4. **The `write_deadline` value.** No measurement. 5s was chosen among industry reference values and its only rationale is 「`ping_interval` 보다 작아야 한다」.
5. **Turning on bidirectional keepalive makes the daemon send a ping to each of N clients every 20 seconds.** Today there is one client so it was not a problem. Leave the period as is, or grow it with the number of peers.
6. **Changing the reconnect limit to 3 splits four places.** `connection.rs:77` (constant 5) · `src/api/wsTransport.ts:352` (constant 5) · `src/api/agentClient.ts:132`·`src/api/daemonControl.ts:92` (prose 「5회」). ★The crate default 3 is explicitly allowed by ADR-0180 「영향」★ (the violation is not the value but *the injection path disappearing*). **Who aligns it and when — outside this scope, but the moment the crate is stood up, the two layers diverge.**
7. ★**Fix `Listener` (the accept side) now, or leave only a shell.**★ **Here 「나중에 리팩터 안 하게」 and 「구조 작게」 collide head-on.** ADR-0179 decision 3 pinned that *「N 개에 붙는다」 and 「N 개를 받는다」 are the same registry and the same supervision*, and `Handshake` is already direction-symmetric, so the accept side sits on as is. But **this scope (①) verifies only the dial side.** The trade-off: fixing it now adds one public type without verification, and leaving it for later changes the `Registry::new` signature then (every consumer has to touch it).
8. **Does the consumer build `PeerId` or does the crate draw a number.** This value is the **event's origin marker**, so it later appears on screen as is — it touches what the user feels. The current proposal is that the consumer gives a string (like the name of the daemon's data folder).
9. **The jitter randomness source.** Draw it from clock + `PeerId` hash with no dependency, or bring in `uuid` (or `rand`). The current proposal is the former and there is a precedent (`profile.rs:680`).
10. **The absence of `tracing`.** While the consumer does not read events, the trace is 0. Accept it, or does the crate also flow events to `tracing`.
11. **When `Wire::decode` fails, drop only that frame or cut the connection.** It touches what the user feels (does the screen get cut off wholesale because one frame broke). The current proposal is to leave it open as a **policy value** with the default 「버리고 사건만」, but then a combination arises where the protocol keeps running out of step.
12. **When the inbound queue is full.** Today the shell has no such queue at all (it goes straight to the Tauri Channel). The new crate places an `Incoming` channel, so **a new failure mode arises** — what to do when the shell is slow.
13. **The crate-name prefix.** This document settled on `engram-dashboard-transport` (the same convention as the other six members). ADR-0177 decision 1 wrote only 「`transport`」, and the same ADR 「영향」 warns that if the prefix is dropped the gate does not see it — **the two lines do not fit, so one side must be chosen.**

### Record only, do not decide (as instructed)

14. **Relaying messages between daemons** — how to learn the address, how to prevent loop-back, and what to do when the peer is absent. **Later research.**
15. **「표시는 이름, 배달은 고유키」** — the agent identifier is already a UUID (`crates/engram-dashboard-protocol/src/ids.rs:4`), so **the inside is already a unique key**, and **only mail (messaging) uses the name as the address.** The delivery rule when two share the same name is **later research.**
16. **The reconnect constant `5`** — the record face of item 6 above. Two places in code + two places in wording.

---

## 13. Rationale · verification status

- ★**This document did not change a single line of code.**★ All the citations below are **static reading**, and no test or build was run.
- **Places opened and checked directly:** `src-tauri/src/commands/agent.rs:240-276` · `src-tauri/src/daemon_client/*` (structure survey) · `crates/engram-dashboard-net/src/{lib,ws,frame_port}.rs` (boundary survey) · `crates/engram-dashboard-protocol/src/{codec,ids,messages}.rs` · `crates/engram-dashboard-agent/src/profile.rs:662-682` · `crates/engram-dashboard-daemon/src/command_delivery.rs:176-187` · `crates/engram-dashboard-command/src/{lib,testing}.rs` · `.github/workflows/ci.yml:485-534` · root `Cargo.toml` · `docs/testing-strategy.md`.
- **`AgentCommand` variants = 30** (measured 2026-09-06, `awk` + `rg -c`) — matches the figure ADR-0181 cited.
- **`REPLY_TIMEOUT` is the only request deadline across `src-tauri/src/commands/`** (measured: `rg "REPLY_TIMEOUT|tokio::time::timeout|Duration::from_" src-tauri/src/commands/` → 3 lines, one of which is discovery's 5-second polling and so on a different axis).
- **Values carrying an unverified mark** (§6): `request_timeout` 8s · `write_deadline` · `reconnect.jitter` · `inbound_queue` · `retired_tags`.
- **Not re-decided:** the adversarial-review verdict of `docs/research/reusable-transport-crate-boundary-2026-09-03.md` is in the state **BLOCK → fixes applied, re-decision not run**. When this document cites that survey as rationale, that state must be read along with it.
