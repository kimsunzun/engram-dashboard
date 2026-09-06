# TRD — `transport` crate 신설 (S21)

> 상태: **1판(2026-09-06).** ADR-0177~0182 를 입력으로 받아 crate 하나를 그리는 문서다.
> **범위 = ADR-0177 결정 1 의 붙이는 순서 중 ①(crate 를 세우고 혼자 검증한다) 뿐이다.** 이사·데몬 적용·화면은 이 문서가 다루지 않는다(§10).
> **읽는 법:** 각 덩어리마다 **「지금 확정」**(경계·seam·타입 — 나중에 바꾸면 비싼 것)과 **「껍데기만」**(실측 안 된 내부)을 표시했다. 판정 규칙 = CLAUDE.md 「아키텍처 원칙 / 0. 판단 기준」 — *추상화는 YAGNI 가 아니라 **위험도 × 기간**으로 판단한다. 저위험 + 장기는 지금 충분히 깔고, 고비용·불확실은 껍데기만 두고 실측 때 채운다.*
> **문서 배치 규약:** `docs/README.md` 「새 내용을 어디에 넣나」의 **「새 기능 **설계 착수** → `process/SN-name/` 새 폴더」** 줄. 폴더가 곧 step이고(같은 파일 「문서 종류」 표), 선례가 `docs/process/S20-command-bus/trd.md` 다.
> 앵커: **ADR-0177**(이 crate 의 헌장 — 안/밖 분할) · **ADR-0178**(버전 불일치 = 데몬 핸드셰이크 판정) · **ADR-0179**(데몬 N 클라 · 클라 N 데몬) · **ADR-0180**(알림 표면 · 재연결 층 가르기) · **ADR-0181**(요청 시한) · **ADR-0182**(사건 방출) · ADR-0129(net 경계) · ADR-0130(`frame_port` feature 미결 — §9-3 에서 안 깨우는 것을 확인) · ADR-0046(재생 single-flight) · ADR-0163/0164(화신 표식) · ADR-0155(도구 crate 선례) · ADR-0175(잎 crate 입주 조건 · `replay_flight` 순수성 게이트 선례) · ADR-0012(모듈 격리 하네스) · ADR-0003(코어 격리) · `docs/research/reusable-transport-crate-boundary-2026-09-03.md` · step-log S21.

---

## 0. 이 판의 정본 요약

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

**밖에 남는 것**(ADR-0177 결정 4 그대로): 패킷 구조체 정의 · 주소 · 인증 자료 · 정책 값 · 짝짓기 번호 생성 · 큐 정책 선택 · 버전/능력 협상 판정.

---

## 1. 목표 · 비목표

### 이 crate 가 하는 것

- 프레임 경계 · 요청/응답 상관(**태그 타입에 제네릭 — crate 는 그 이름을 모른다**) · 역할 구분(요청/응답/알림/스트림)
- 스트림 살림 = 식별자 · 세대 · 순번 · 이어받기 · 중복 제거 · **구멍 찾기와 그 지점부터 재요청**
- 연결 수립 · **불투명한 핸드셰이크**(왕복 횟수 무관) · **양방향 keepalive** · **쓰기 시한** · **백오프+지터 재연결**
- **상대 N 개 명부 + 상대마다 감독** · 인바운드에 **출처 표식** · **세대 경계 노출**
- 팬아웃 · 유계 큐와 배압 신호 · **연결 사건 구조화 방출**(ADR-0182)

### 이 crate 가 하지 않는 것

§10 에 따로 모았다. 「안 하는 것」은 목록으로 박아 두지 않으면 다음 세션이 슬금슬금 넣는다.

---

## 2. 모듈 경계와 파일 배치

**crate 이름 = `engram-dashboard-transport`**(짧은 이름 `transport`). ★**결정 근거와 남는 애매함**★ — ADR-0177 결정 1 은 이름을 「`transport`」로만 적고, 같은 ADR 「영향」은 *「crate 이름이 `engram-dashboard` 접두를 벗으면 CI 의존 상한 게이트가 그 crate 를 조용히 안 본다」*고 경고한다. 두 줄이 서로 안 맞는다. 이 문서는 **다른 여섯 멤버와 같은 관례**(문서·대화에서는 짧은 이름, 패키지는 접두형 — `net`·`agent`·`base`·`command` 전부 그렇다)로 읽어 접두형을 택했고, 그러면 게이트 정규식을 손댈 일이 없다. **접두를 정말로 뗄 생각이면 §11-13 을 볼 것.**

파일 배치는 이 저장소의 lib 관례를 그대로 따른다 — **평평한 `src/*.rs`, 하위 디렉터리 없음, 단위 테스트는 `#[cfg(test)]` 인라인, 통합은 `tests/`**(`net` 6 파일 · `command` 11 파일 · `messaging` 7 파일이 전부 이 모양이다).

| 파일 | 무엇을 소유하나 | 단독 검증 (ADR-0012) |
|---|---|---|
| `lib.rs` | crate 헤더 = **경계·격리 게이트·불변식의 정본**. 재수출만 하고 로직 0 | — |
| `frame.rs` | `Frame` 어휘(`Text`/`Binary`/`Keepalive`) · `Close`(코드+문구) · `CloseCode` 배정 | 인라인 단위 |
| `wire.rs` | ★`Wire` trait★ — 소비자가 주는 패킷 어휘 전부(§3-1) | 인라인 단위(가짜 Wire 로) |
| `link.rs` | 전송 seam — `Dialer`·`LinkTx`·`LinkRx`·`LinkError` | 인라인 단위 |
| `clock.rs` | 시간 seam — `Clock`·`SystemClock`·`with_deadline` | 인라인 단위 |
| `policy.rs` | 정책 값 struct + 기본값(§6) | 인라인 단위(기본값 불변식 단언) |
| `event.rs` | 위로 올리는 사건 어휘(§7) | — (자료형뿐) |
| `pending.rs` | 요청/응답 상관 — 태그 제네릭 pending 표 · 시한 · **만료 번호 재사용 거절** | ★순수★ 인라인 단위 |
| `stream.rs` | 스트림 살림 — 세대 대조 · 순번 중복 제거 · **구멍 검출** · 「유효 하한」 | ★순수★ 인라인 단위 |
| `machine.rs` | ★**순수**★ 연결 하나의 상태 기계. 입력 사건 + `Instant` → 행동 목록. **tokio·I/O 0줄** | ★순수★ 인라인 단위 — **시간을 인자로 받으므로 하네스조차 필요 없다** |
| `peer.rs` | 감독 태스크 — `machine` 을 `Link`·`Clock` 위에서 돌린다 · 단일 writer · 유계 큐 · `Peer` 핸들 | 인메모리 Link + 수동 Clock |
| `registry.rs` | 상대 N 개 명부 · 출처 표식 팬인 · 팬아웃 · `Incoming` 한 줄 | 인메모리 Link ×N |
| `ws.rs` | WS 어댑터(`Dialer`/`Link` 구현). **feature `ws`** | 실소켓 `127.0.0.1:0` |
| `testing.rs` | 하네스 — 인메모리 `Link` · 수동 `Clock` · 기록형 사건 수집기. **feature `test-support`** | — |

**14 파일.** 「구조 작게」와 부딪히지 않는다고 보는 근거: 각 파일이 **하나의 명사**를 갖고, 셋(`pending`·`stream`·`machine`)은 순수라 하네스 없이 돌며, 둘(`ws`·`testing`)은 feature 뒤라 기본 빌드에 없다. 남는 실질 구동 코드는 `peer.rs`+`registry.rs` 둘이다.

### 왜 `machine.rs` 를 따로 두나

★**순수 층을 파일로 가르는 것이 이 crate 의 유일한 「나중에 리팩터 안 하게」 장치다**★. ADR-0177 이 후보 B(코어를 별도 crate 로)를 **약한 근거로** 기각했고 후보 A(한 crate 안에서 모듈로 가르기)를 택했는데, 그 후보 A 의 알려진 약점이 *「sans-IO 순수성이 한 crate 안에서는 컴파일러 강제 없이 관례로만 버틴다」*(조사 §3 후보 A 단점)다. 이 저장소엔 그 약점을 정확히 메우는 선례가 **이미 있다** — `src-tauri/src/daemon_client/replay_flight.rs` 의 순수성 게이트(ADR-0175 결정 3, CLAUDE.md 「빌드·검증 명령」). 같은 형태를 그대로 쓴다(§9-2 게이트 3).

- **지금 확정:** 순수/구동 파일 분리 · 그 경계를 지키는 게이트 · `machine` 이 시간을 **인자로** 받는 것.
- **껍데기만:** `machine` 안의 행동 목록 자체는 늘어난다(취소·중계 등). 그건 enum 에 variant 를 더하는 것이라 경계를 안 흔든다.

---

## 3. 공개 인터페이스

★**이 절이 「나중에 리팩터 안 하게」의 본체다.**★ 아래 타입 이름과 메서드 집합은 **지금 확정**이고, 본문 구현은 얼마든지 갈아엎어도 된다.

### 3-1. `Wire` — 소비자가 주는 패킷 어휘 전부 (지금 확정)

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

**이 여섯 함수가 오늘 코드에 그대로 대응한다(실측):**

| `Wire` 칸 | 오늘 실물 |
|---|---|
| `Out`/`In` | `AgentCommand` / `AgentEvent` |
| `Tag` | `protocol::RequestId`(`ids.rs:16` — uuid 래퍼) |
| `StreamKey` | `protocol::AgentId = uuid::Uuid`(`ids.rs:4`) |
| `request_tag` | `protocol::command_request_id`(`messages.rs:653`) — **이미 있는 정본. 새로 만들지 않는다**(ADR-0177 결정 4) |
| `reply_tag` | `protocol::event_reply_request_id`(`messages.rs:711`) |
| `stream_mark` | 출력 hot path 고정헤더 `[tag:1][agent_id:16][epoch:4 BE][seq:8 BE]`(`protocol/src/codec.rs:3`) — **세 칸이 그대로 `StreamMark` 다** |
| `encode`/`decode` | 코덱이 둘이다 — control = JSON(WS Text) · 출력 = binary(WS Binary). ★`Frame` 이 둘 다 나르므로 한 `Wire` 가 흡수한다★ |

★**역할 구분(요청/응답/알림/스트림)은 새 어휘가 아니라 위 접근자들의 조합이다**★ — 나가는 쪽은 `request_tag` 의 유무, 들어오는 쪽은 **`reply_tag` → `stream_mark` → 그 외** 순으로 본다. **순서가 계약이다**: 한 `In` 이 둘 다 `Some` 을 주면 답장으로 먼저 처리하고 `Event::AmbiguousRole` 을 올린다(소비자 계약 위반 신호).

- **지금 확정:** trait 의 여섯 칸과 연관 타입 다섯. **껍데기만:** 없음 — 이 trait 은 전부 지금 확정해야 한다(나중에 칸을 더하면 모든 소비자가 깨진다).

### 3-2. 명부와 핸들 (지금 확정)

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

★**핸들이 재연결에 불사인 것과 세대 경계를 드러내는 것은 양립한다**★. 조사 §5-3 이 「불사 명령 핸들」과 「세대에 묶인 핸들」을 **방어 가능한 양 끝**으로 세웠고 이 저장소는 후자가 idiom(화신 표식, ADR-0163/0164)이라고 적었다. 우리는 **핸들은 불사, 데이터에는 세대 표식**을 고른다 — 소비자 코드가 짧아지면서도 조용한 유실이 안 생긴다(경계가 `Arrival.generation` 으로 보인다). ★이것이 조사 §5-4 가 「이 설계의 최대 위험」이라 부른 자리의 답이다.★

### 3-3. 위로 올라오는 것 — 한 줄 (지금 확정)

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

★**옆 채널을 두지 않는 것이 결정이다**★ — ADR-0177 결정 9 가 *「신호는 **소비자가 이미 읽는 스트림 안에** 넣는다 — 옆 채널은 안 들으면 없는 것과 같고, NATS 문서가 그것을 자기 함정으로 적었다」*로 못박았다. 유실 신호만 그렇게 하고 나머지 사건은 옆 채널로 두면 그 함정이 절반 남는다. **그래서 전부 한 줄이다.** 대가 = 패킷만 원하는 소비자도 `match` 를 한 겹 쓴다. (나중에 로그 창이 사건을 따로 읽어야 하면 갈래가 생긴다 — §11-1.)

### 3-4. 갈아끼우는 두 축 (지금 확정)

**규칙 한 줄: 타입은 제네릭, 자원은 dyn.**

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

**왜 trait 이고 왜 dyn 인가 — 축마다 답이 다르다:**

- **`Wire` 는 제네릭이다.** 지우면 `W::Out`/`W::In`/`W::Tag` 가 사라져 crate 가 **바이트 파이프**가 된다 — 그것이 ADR-0177 이 기각한 후보 C 다. 타입 seam 은 이 crate 의 존재 이유라 절대 지우지 않는다.
- **`Dialer`/`Link` 는 dyn 이다.** 사유 셋. ① ★**한 명부 안에 서로 다른 전송이 섞일 수 있어야 한다**★ — 로컬 WS 상대와 (나중) TLS 상대를 같은 `Registry` 에 담으려면 제네릭 파라미터로는 표현이 안 된다. ② 프레임은 이미 `String`/`Vec<u8>` 값이라 프레임당 dyn 호출은 syscall 옆에서 잡음이다. ③ 같은 저장소가 이미 그 형태다 — `net` 의 `frame_port` 가 `Arc<dyn FrameSink>` + `BoxFuture` 로 지운다(`async fn` in trait 이 dyn-호환이 아니라서, `frame_port.rs:107`).
- **`Clock` 은 dyn 이다.** 제네릭으로 두면 `Peer`·`Registry`·`Pending`·`Stream` 네 타입 시그니처에 전부 번지고 명부의 컬렉션 타입까지 오염된다. 운영 구현이 하나뿐이라 단형화로 얻을 것도 없다. 선례가 `crates/engram-dashboard-daemon/src/command_delivery.rs:176`(`pub trait Clock: Send + Sync { fn now(&self) -> Instant; }`)이고 **이 crate 는 거기에 `sleep` 을 더한다** — 백오프·keepalive·쓰기 시한이 *기다림*을 필요로 하는데 `now()` 만으로는 못 만든다.
- ★**`tokio::time::pause()` 로 대신하지 않는 이유**★ — ① 그것은 **런타임 전역** 장치라 런타임을 세워야만 쓰고, `machine.rs`(future 가 하나도 없는 순수 층)에는 애초에 적용할 데가 없다 ② ADR-0177 결정 7 이 **전송도 갈아끼우라** 했는데 시간을 tokio 에 묶으면 그 둘이 도로 붙는다 ③ `test-util` 은 dev 전용이라 소비자의 하네스에서 켜지지 않을 수 있다(`src-tauri/Cargo.toml:109` 가 그 경계를 명시한다). **단 실소켓 테스트(§8-2)에서는 `pause()` 가 더 싸므로 금지하지 않는다.**

### 3-5. 핸드셰이크 — 불투명, 왕복 횟수 무관, 방향 대칭 (지금 확정)

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

- **왕복 횟수를 crate 가 세지 않는다** — `Send`/`Await` 를 몇 번 돌려주든 받는다. 오늘 클라 쪽은 `Send(auth) → Await → (Hello) → Done` 두 걸음이다(`connection.rs:411`·`1644-1674`).
- **방향 대칭** — 데몬 쪽은 같은 trait 으로 `start() = Await`, `on_frame` 에서 토큰·버전을 보고 `Done` 또는 `Reject` 를 준다. ★**그래서 ADR-0178 의 「판정 주체 = 데몬 단독」이 crate 를 안 건드리고 성립한다**★ — 버전 어휘는 `Handshake` 구현체 안에만 있고 crate 는 여전히 모른다(ADR-0129 어휘 격리 유지).
- ★**`Reject` 와 「못 붙음」은 다른 것이고 그 구분이 예산을 가른다**★ — ADR-0180 결정 5. §5 상태 기계가 그것을 집행한다.

### 3-6. 오류 어휘 (지금 확정)

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

`RequestError::NotARequest` 는 오늘 동작 보존이다 — `mod.rs:704-706` 이 이미 *「send_command: request_id 없는 명령은 reply 를 기대할 수 없다」*로 거절한다. ★그리고 이것이 ADR-0181 「영향」의 `CommandOutcome` 조항을 구조로 닫는다★: 그 variant 는 `command_request_id` 가 `None` 을 주므로 `request()` 에 넣으면 **영구 대기가 아니라 즉시 `NotARequest`** 가 난다.

---

## 4. 상관 · 스트림

### 4-1. 요청/응답 (지금 확정)

- **대기 표 = `HashMap<W::Tag, Waiter>`**, 감독 태스크가 단독 소유(락 없음). 오늘과 같은 모양(`protocol_state.rs:71` `PendingMap<T>`, `connection.rs:580` 이 `&mut` 로 넘긴다).
- **등록은 보내기 **전**에** — 오늘도 그렇다(`connection.rs:1030-1036`). 뒤집으면 빠른 답장이 슬롯을 못 찾는다.
- ★**겹친 번호 = 옛 대기자를 `Superseded` 로 깨우고 새 요청도 `TagRetired` 로 실패시킨다**★(확정 — 사용자 결정 2026-09-06, 적대 리뷰 지적 **M1**). 겹치는 순간 그 번호를 **은퇴**시켜 어느 쪽도 표에 남기지 않는다.
  - ★**이 줄은 뒤집힌 것이다**★ — 1판의 「옛 대기자를 `Superseded` 로 깨우고 **새 것이 승계** — 오늘 동작 보존(`connection.rs:1445-1460`)」을 M1 이 번복했다. 지켜진 절반은 「옛 대기자 = `Superseded`」이고, 번복된 절반은 「새 것이 승계」다.
  - **왜** — 승계를 두면 **먼저 나간 요청의 늦은 답장이 나중 요청의 짝으로 배달되는 경로가 남는다**(`route` 는 `pending.take(&tag)` 로만 짝을 찾고, 상관 키가 그 번호 하나뿐이라 두 답장을 가를 재료가 wire 에 없다). ★**틀린 답을 받는 것이 실패를 받는 것보다 나쁘다**★ — 그래서 잘못 배달하느니 둘 다 실패시킨다.
  - **대가** — ★이 경로에서 오늘 셸 동작은 보존되지 않는다★. 살아 있는 번호를 되쓰는 소비자는 이제 **성공 하나 대신 실패 둘**을 받는다. 그 대신 오배달이 구조적으로 불가능해진다(살아 있는 번호에 대해서만 — 잊힌 번호 쪽 한계는 §4-2 와 `Pending` rustdoc 이 정본).
  - 회귀망 = `pending::tests::a_reply_for_a_collided_tag_reaches_nobody`.
  - **결정 기록은 이 문서가 아니다** — 별도로 정리된다. 여기 적은 것은 TRD 로서의 확정 내용뿐이다.
- **끊기면 대기자 전원을 `Disconnected` 로 깨운다** — 오늘 동작 보존(`connection.rs:621-626`). ★**그래서 시한이 실제로 도달하는 경우는 「연결은 멀쩡한데 그 요청만 침묵」 하나뿐이다**★(ADR-0181 맥락 그대로).

### 4-2. 시한과 번호 재사용 (지금 확정 · 값은 §6)

- **명령 무관 공용 시한 하나, 기본 8초**(ADR-0181 결정 1). 만료하면 **그 요청만** `TimedOut` 으로 깨우고 **연결은 그대로 둔다**(결정 2).
- **자동 재전송 없음**(결정 3). **만료 뒤 늦게 온 답장은 버린다**(결정 4) — 슬롯이 없으므로 자동으로 그렇게 된다.
- ★**「그 번호는 재사용하지 않는다」는 crate 가 반만 집행할 수 있다**★ — 진짜 보증은 **번호를 만드는 쪽**에 있다(우리는 uuid v4, `ids.rs:16`). crate 가 할 수 있는 것은 **최근 만료된 번호를 기억해 두었다가 다시 오면 `TagRetired` 로 거절**하는 것뿐이고, 기억은 무한할 수 없으므로 **유계 링**이다(기본 256, ★미검★). 이 한계를 `lib.rs` 헤더에 적는다 — 적어 두지 않으면 다음 세션이 이 링을 보증으로 읽는다.
- **「기다리는 중」 표시(1초 제안)는 crate 밖이다**(ADR-0181 결정 5) — 표시와 포기는 다른 층이고, 한쪽 값을 고치며 다른 쪽을 따라 맞추지 않는다.

### 4-3. 스트림 살림 (지금 확정 · 재요청 정책은 껍데기)

```rust
pub struct StreamMark<K> {
    pub key: K,
    /// 화신 표식. ★비교는 일치/불일치만★ — 대소로 "더 새 것"을 유도하지 않는다(ADR-0163)
    pub generation: u32,
    pub seq: u64,
}
```

crate 가 스트림마다 쥐는 것: **마지막 세대 · 마지막 순번**. 그 위에서 넷을 한다.

1. **중복 제거** — `seq <= last` 면 버린다(오늘 프론트가 하는 것을 crate 로 올린다).
2. **세대 불일치** — `generation != last_generation` 이면 순번 대조를 **초기화**하고 `StreamGenerationChanged` 를 올린다. ★일치/불일치만 본다★.
3. ★**구멍 검출**★ — `seq > last + 1` 이면 `StreamGap` 을 올린다. **오늘 이 검사는 아예 없다**(ADR-0177 근거: *「현행은 순번 구멍 검사가 아예 없고 단조 dedup 만 있다」*).
4. ★**그 지점부터 재요청**★ — 구멍을 만나면 `Wire::resume_request(key, Some(last))` 를 지어 자동으로 내보낸다. **검출만 하고 끝내지 않는다**(ADR-0177 결정 9).

**잘림 신호는 참/거짓이 아니라 「지금 유효한 가장 오래된 번호」다**(ADR-0177 결정 9) — `StreamTruncated { oldest_valid }`. ★**그 값은 wire 가 이미 나르고 있다**★: `AgentEvent::SubscribeAck` 에 `oldest_seq`/`latest_seq`/`replay_from` 칸이 있는데 **셸이 안 읽는다**(`connection.rs:1297-1309` 는 `current_epoch` 과 `truncated` 만 꺼낸다 — 정적 판독). 즉 crate 가 이 값을 얻는 경로는 소비자의 `Wire` 를 통해 이미 열려 있다.

- **껍데기만:** 구멍 하나에 재요청을 **몇 번까지** 할지. 지금은 **구멍당 1회**, 그 재요청이 또 구멍을 만나면 사건만 올리고 멈춘다. 폭주 방지 정책은 실측 때 채운다.
- ★**나가는 쪽 스트림(데몬이 순번을 찍는 쪽)은 crate 밖이다**★ — 오늘 `OutputCore` 가 찍고, 그건 에이전트 어휘다.

---

## 5. 상태 기계 (지금 확정)

**연결 하나의 수명.** `machine.rs` 가 순수 함수로 갖는다 — 입력 = (지금 시각, 사건) · 출력 = 행동 목록.

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

| 전이 | 계기 | 비고 |
|---|---|---|
| `Idle → Dialing` | `Registry::add` 또는 `reconnect_now()` | 예산을 0 으로 초기화 |
| `Dialing → Handshaking` | `Dialer::dial` 성공 | `connect_timeout` 이 이 구간을 감싼다 |
| `Dialing → Backoff` | `dial` 실패/시한 초과 = `Unreachable` | ★예산 1 소모★ |
| `Handshaking → Live` | `HandshakeStep::Done` | 세대 +1, `Connected` 사건 |
| `Handshaking → GaveUp(Rejected)` | `HandshakeStep::Reject` **또는** 상대가 거절 프레임+close | ★**예산을 안 쓴다. 첫 거절에서 즉시 포기**★(ADR-0180 결정 5) |
| `Live → Backoff` | 상대 종료 · `LinkError` · **침묵**(`idle_timeout`) · **쓰기 시한 초과** · **큐 포화**(정책이 `Disconnect` 일 때) | 예산 1 소모 · 대기자 전원 `Disconnected` |
| `Backoff → Dialing` | 대기 만료 | `base · 2^attempt` 를 `cap` 으로 자르고 지터를 섞는다 |
| `Backoff → GaveUp(BudgetExhausted)` | `attempt >= max_attempts` | ★앱은 여기서 「다시 연결」 버튼을 낸다(ADR-0180 결정 7)★ |
| `* → Closed` | `Peer::close()` | 다시 붙지 않는다 |

**N 개 명부가 그 위에 얹히는 방식:** 상대마다 이 기계 하나 + 감독 태스크 하나. **상대끼리 상태를 공유하지 않는다** — 데몬 A 가 `GaveUp` 이어도 B 는 `Live` 다.

### ★ADR-0180 의 열린 질문이 여기서 닫힌다★

그 ADR 은 *「팝업 계기를 연결마다 볼 것인가 앱 전체로 볼 것인가」*를 열어 두었다. **사용자 결정(2026-09-06): 팝업 판정은 앱 전체 기준 — 붙어 있는 데몬이 하나라도 남아 있으면 띠, 전부 잃었을 때만 팝업. 연결 하나가 죽으면 그 자리에서 눌러 다시 붙는다(연결별 재연결 어포던스).**

★**그런데 그 판정을 이 crate 가 하지 않는다**★ — 「앱 전체」는 화면의 개념이고 crate 는 상대를 셀 뿐 앱을 모른다. crate 가 주는 재료는 둘이다: `Registry::peers() -> Vec<(PeerId, PeerState)>`(앱 전체를 접을 수 있게) · `Peer::reconnect_now()`(연결별 어포던스가 부를 자리). **셸이 그 둘로 접는다.** *(이 문단은 ADR 갱신용 기록이다 — ADR-0180 본문 수정은 이 문서가 하지 않는다.)*

---

## 6. 정책 값 표

★**전부 crate 기본값이 있고 소비자가 덮는다**★(ADR-0177 결정 4). `Registry::new` 가 기본을, `add_with_policy` 가 상대별 덮어쓰기를 받는다.

| 이름 | 기본값 | 누가 덮나 | 근거 / 검증 상태 |
|---|---|---|---|
| `request_timeout` | **8s** | 셸(설정 파일) | ADR-0181 결정 1 = 사용자 판단. ★**미검 — 에이전트 spawn 실소요를 잰 적이 없다**★(그 ADR 이 유일한 실질 위험으로 적었다) |
| `connect_timeout` | **10s** | 셸 | 현행값 보존 — `connection.rs:71` `HANDSHAKE_TIMEOUT = 10s`(dial + Hello 대기 둘 다 감싼다) |
| `write_deadline` | **5s** | 셸·데몬 | ★**미검 — 실측 없음. 업계 참고값뿐**★(NATS 10s · nginx 60s · gRPC keepalive timeout 20s, ADR-0177 근거). 불변식: `write_deadline < ping_interval` |
| `ping_interval` | **20s** | 셸·데몬 | 현행 데몬값 보존 — `net/src/ws.rs:51` `DEFAULT_PING_INTERVAL` |
| `idle_timeout` | **50s** | 셸·데몬 | 현행 데몬값 보존 — `net/src/ws.rs:52-53`, ping 주기의 2.5배 |
| `reconnect.max_attempts` | **3** | ★**셸이 소유·주입**★ | ADR-0180 결정 4(사용자 결정). 오늘 코드는 **5**다(`connection.rs:77` · `src/api/wsTransport.ts:352`) — §11-6 |
| `reconnect.base` | **500ms** | 셸 | 현행값 보존 — `connection.rs:80` `BACKOFF_BASE` |
| `reconnect.cap` | **10s** | 셸 | 현행값 보존 — `connection.rs:83` `BACKOFF_CAP` |
| `reconnect.multiplier` | **2.0** | 셸 | 현행값 보존 — `connection.rs:87-92`. 3회 = 500ms→1s→2s = **합계 3.5초**(ADR-0180 이 든 그 수치) |
| `reconnect.jitter` | **0.2** | 셸 | ★**미검 — gRPC 「제안」 알고리즘의 예시값 차용**★. 그 문서가 스스로 *"Proposed Backoff Algorithm"* 이라 제목을 달고 대안을 허용한다(조사 §4). **오늘은 지터가 아예 없다** |
| `outbound_queue` | **512** | 셸·데몬 | 현행값 보존 — `src-tauri/src/daemon_client/mod.rs:473` `mpsc::channel(512)` |
| `inbound_queue` | **512** | 셸·데몬 | ★**미검 — 오늘 대응물이 없다**★(셸은 Tauri Channel 로 직행해 이 큐 자체가 없다). §11-10 |
| `full_queue` | **`Disconnect`** | 셸·데몬 | ADR-0177 결정 6(현행 유지). ★**단 「현행」이 하나가 아니다 — §11-2**★ |
| `retired_tags` | **256** | 셸 | ★미검★ — §4-2 의 유계 링 |

★**양방향 keepalive 가 「개선」이 아니라 「없던 절반을 채우는 것」인 이유(실측 — 정적 판독)**★ — 오늘 셸에는 **능동 ping 도 idle 타이머도 없다.** 읽기 루프 주석이 *「Ping/Pong 은 tungstenite 가 자동 응답(내부)」*이라 적고(`src-tauri/src/daemon_client/connection.rs:1008`) 그것이 전부다 — 즉 셸은 **자동으로 답할 뿐 스스로 묻지 않고, 상대가 조용해진 것을 재는 시계도 없다.** 데몬 쪽 20초 ping 이 오는 동안에는 그것이 대신 살아 있음을 증명해 주지만, ★**데몬이 죽거나 회선이 끊기면 그 ping 자체가 안 오므로 셸은 아무 신호도 못 받는다**★. ADR-0177 근거의 *「물리 끊김은 스스로 물어봐야 안다」*가 겨냥한 자리가 정확히 여기다.

★**한 칸이 다른 문서의 근거를 흔든다**★ — ADR-0177 근거가 *「큐 상한이 되감기 링보다 크면 안 되는데 현행이 거꾸로다(`CONN_TX_CAP = 4608` > replay `max_events = 4096`)」*를 적출했다. 그 4608 은 **데몬 쪽 연결당 큐**(`net/src/ws.rs:47`)이고 위 표의 512 는 **셸 쪽 명령 큐**라 같은 칸이 아니다. **데몬에 이 crate 를 붙일 때(단계 ④) 그 자리의 기본값을 되감기 링 이하로 잡아야 한다** — 이 문서는 그것을 정하지 않는다(§11-3).

**지터 난수원(내부 결정 · 보고):** ★새 의존을 들이지 않는다★ — `Clock::now()` 의 하위 나노초에 `PeerId` 해시를 섞는다. 선례가 있다: `crates/engram-dashboard-agent/src/profile.rs:680` 이 *「난수 crate 를 들이지 않으려고 uuid v4 의 바이트를 쓴다」*고 적고 그렇게 한다. **한계 둘을 적어 둔다** — ① 암호학적 난수가 아니다(지터엔 필요 없다) ② 수동 시계 아래에서는 결정적이 된다(하네스가 **원하는** 성질이지만, 그래서 지터의 *분산*은 실소켓에서만 관측된다). §11-9 에 열어 둔다.

---

## 7. 사건 목록 (모양은 지금 확정 · 항목은 늘어난다)

```rust
pub enum TransportEvent<W: Wire> { /* 전부 peer: PeerId 를 갖는다 */ }
```

| 사건 | 실어야 할 것 | 왜 필요한가 |
|---|---|---|
| `Connected` | `peer` · `generation` | 세대 경계의 시작 |
| `Disconnected` | `peer` · `generation` · `cause`(상대 종료 / 오류 / 침묵 / 쓰기 시한 / 큐 포화 / 명시 종료) | ★오늘 이 구분이 없다 — 전부 「끊김」 한 덩어리★ |
| `ConnectFailed` | `peer` · `attempt` · `of` · `cause` · `retry_in` | ★**「몇 번째 재시도인가」**(ADR-0182 결정 1). 오늘 재연결 중 사유는 `debug!` 로만 남고 버려진다★ |
| `Rejected` | `peer` · `code` · `reason` | ★**「왜 거절당했나」**★. ADR-0178 의 버전 거절 문구가 이 칸으로 올라와 팝업 문안이 된다 |
| `GaveUp` | `peer` · `reason`(예산 소진 / 거절) | 셸이 팝업 자격을 판정하는 입력(ADR-0180 결정 2) |
| `StreamGap` | `peer` · `stream` · `generation` · `expected` · `got` · `resumed_from` | ★**「어디서 몇 개를 잃었나」**★ + 무엇을 다시 청했나 |
| `StreamTruncated` | `peer` · `stream` · `generation` · **`oldest_valid`** | ★참/거짓이 아니라 **번호**★(ADR-0177 결정 9). 오늘은 브라우저 콘솔 경고로만 난다 |
| `StreamGenerationChanged` | `peer` · `stream` · `from` · `to` | 화신이 갈렸다 — 순번 대조가 초기화됐다는 뜻 |
| `RequestTimedOut` | `peer` · `tag` · `after` | 8초 시한이 실제로 물었다 |
| `Dropped` | `peer` · `direction` · `count` | 큐 정책이 `DropAndReport` 일 때. ★**조용한 유실을 안 만들기 위한 칸이다**★ |
| `DecodeFailed` | `peer` · `generation` · `reason`(문자열) | `Wire::decode` 실패. **연결을 끊을지는 정책** — §11-11 |
| `AmbiguousRole` | `peer` | 한 `In` 이 답장이면서 스트림 조각이라고 신고됐다(§3-1 계약 위반) |

★**이 명단이 곧 crate 경계다**★(ADR-0182 「영향」) — 에이전트 오류·명령 실패는 **여기 없다**. 위층이 아는 일이고, 합치는 것은 나중에 화면이 한다.

**부가 가치:** 이 사건들이 §8 인메모리 검증의 **관측 지점**이다 — ADR-0182 가 적은 대로, ADR-0177 결정 8 의 하네스가 「무엇을 보고 판정할지」를 여기서 얻는다.

---

## 8. 오류·닫기 어휘 (지금 확정)

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

- ★**코드와 문구를 둘 다 싣고, 분기는 코드로만 한다**★(사용자 결정). 문구는 사람이 읽는 것이고 **예고 없이 바뀐다** — 오늘 `AgentEvent::SubscribeFailed` 의 `reason` 주석이 이미 그렇게 적어 두었다.
- **코드를 닫기 프레임에 실으면 「오류 프레임보다 닫기가 먼저 도착」 경합이 사라진다.** 오늘은 사유가 **Text 프레임**으로 따로 가고 닫기엔 코드가 없어(`net/src/ws.rs:439-447` `send_error_and_close`) 그 경합이 실재한다.
- ★**한계를 숨기지 않는다**★ — RFC 6455 §7.4.1 이 `1006` 을 **와이어로 보낼 수 없는 예약값**으로 못박고 §7.1.5 가 그것을 수신 측이 로컬 합성하는 값으로 정의한다(ADR-0177 거부한 대안). **즉 우리가 `QUEUE_FULL`·`WRITE_DEADLINE` 으로 끊을 때 상대가 그 코드를 볼 가능성은 낮다** — 혼잡한 소켓에 닫기 프레임을 밀어 넣을 자리가 없기 때문이다. **코드가 실제로 전달되는 자리는 핸드셰이크 거절**이다(그때는 아무것도 안 밀려 있다 — ADR-0178 근거가 이 구분을 명시한다).

**「거절」 vs 「못 붙음」:** `ConnectFailure::Rejected` = 통로는 열렸고 상대가 안 받아 줬다(예산 안 씀) · `Unreachable` = 통로 자체가 안 열렸다(예산 씀).

★**오늘은 이 둘이 같은 예산을 태운다(실측 — 정적 판독)**★ — 재연결 루프의 실패 갈래가 `HandshakeOutcome::Err(e)` 하나이고 그 자리 주석이 스스로 *「시도 실패(데몬 죽음/**거부**) — 다음 백오프로」*라 적는다(`src-tauri/src/daemon_client/connection.rs:805-808`). 사유 `e` 는 `tracing::debug!` 로만 남고 버려진다. ★**이것이 ADR-0178 「검증 안 된 것」 2번(「거절당한 뒤 클라가 무한 재접속을 반복하는지」)의 답이다 — 무한이 아니라 5회 예산을 태운 뒤 `Down` 으로 앉는다**★(`connection.rs:77` · `:819-829`). 그리고 같은 ADR 의 미검 1(「클라가 거절 사유 프레임을 실제로 받아 읽는지」)도 **읽는다**가 답이다 — `wait_for_hello` 가 Hello 앞에 온 `AgentEvent::Error` 를 `HandshakeError::AuthRejected(message)` 로 타입 지어 올린다(`connection.rs:1657-1659` · 열거는 `:111-129`). ★**다만 그 문구가 화면까지 가는 것은 첫 연결 때뿐이고 재연결 중에는 안 간다**★ — 위 `debug!` 갈래가 그 자리다.

---

## 9. 검증 계획

### 9-1. 무엇을 어디서 재나

**인메모리 + 시간 주입으로 재는 것(전부)** — ADR-0177 결정 8. 백오프·keepalive·쓰기 시한을 **실제로 기다리지 않는다.**

| 대상 | 단언 |
|---|---|
| `machine.rs` | §5 표의 전 전이. **future 도 런타임도 없다** — `(now, event) → actions` 순수 함수라 하네스조차 필요 없다 |
| `pending.rs` | 시한 만료가 그 요청만 깨운다 · 늦은 답장은 버려진다 · 겹친 번호가 옛 대기자를 `Superseded` 로 깨운다 · 만료 번호 재사용이 `TagRetired` · **링이 넘치면 조용히 잊는다** |
| `stream.rs` | 중복 제거 · 구멍 검출 · 구멍당 재요청 1회 · 세대 불일치가 순번을 초기화 · `oldest_valid` 전달 |
| `peer.rs` (+ 인메모리 `Link` + 수동 `Clock`) | 백오프 일정이 500/1000/2000 · **거절은 예산을 안 쓴다** · 예산 소진이 `GaveUp` · `reconnect_now()` 가 예산을 초기화 · 침묵이 `idle_timeout` 에 끊는다 · 큐 포화 정책 둘 |
| `registry.rs` | 출처 표식이 붙는다 · 상대 하나가 죽어도 나머지가 산다 · 팬아웃 부분 실패가 상대별 사건으로 온다 · 인바운드가 **한 줄로** 합쳐진다 |
| `event.rs` 전반 | 위 전부를 **사건으로** 단언한다(ADR-0182 「부가 가치」 — 이 crate 의 첫 소비자는 테스트다) |

**실소켓으로만 재는 것(소수)** — `tests/` 에 둔다. 포트는 **0번을 요청**한다.

1. **WS 어댑터가 실제로 붙나** — `tests/ws_dial.rs`. 손으로 세운 tungstenite 서버에 붙어 핸드셰이크 두 걸음을 왕복한다.
2. ★**상대가 안 읽을 때 쓰기 시한이 실제로 잘라내나**★ — `tests/ws_write_deadline.rs`. **커널 버퍼가 있어야 재므로 인메모리로 못 잰다**(ADR-0177 결정 8 이 그렇게 못박았다).
3. **버전 거절 왕복** — `tests/ws_reject.rs`. 서버가 거절 프레임 + 닫기 코드를 보내고, 클라가 `ConnectFailure::Rejected{code, reason}` 을 얻으며 **예산을 안 태우고** `GaveUp(Rejected)` 로 앉는다.

★**`-- --test-threads=4` 를 붙이지 않는다**★ — 이 crate 는 자식 프로세스를 하나도 안 띄운다(소켓뿐). CLAUDE.md 「빌드·검증 명령」의 판정 규칙 그대로이고, `command`·`protocol`·`messaging`·`net` 과 같은 처지다.

### 9-2. 게이트 초안

기존 게이트의 **형태를 그대로 따른다**(`.github/workflows/ci.yml` 의 `base`·`net` 스텝이 본이다). 기대값·근거의 **정본은 `crates/engram-dashboard-transport/src/lib.rs` 헤더**이고, CLAUDE.md 는 규칙만·`/qa` 바인딩은 실행 사본을 갖는다 — 이 문서에서 개수를 세지 않는다.

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

★**게이트 1 이 「워크스페이스 의존 0」의 유일한 벽이다**★ — 컴파일러는 그것을 강제하지 않는다. `command`·`base`·`messaging` 이 같은 줄을 갖고 있고, **셋 다 공통으로 남는 구멍**(멤버를 `engram-dashboard` **이름 접두**로 식별한다 — 다른 이름을 단 멤버는 그냥 통과한다)도 그대로 물려받는다.

★**소스 정규식 짝을 두지 않는다**★ — `messaging` 은 crate 이름 알파벳을 손으로 박은 정규식을 함께 갖는데, 이 crate 는 **남을 안 부르는 것**이 불변식이라 부르는 이름의 알파벳을 관리할 대상이 없다(`base` 헤더가 같은 논거로 그것을 안 둔다).

### 9-3. 워크스페이스 등재

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

CI 는 `backend` 잡에 `cargo test --locked -p engram-dashboard-transport --all-features` 한 줄, `fmt + isolation gates` 잡에 위 게이트 넷을 더한다. **워크스페이스 회귀(`cargo test --workspace`)가 이 crate 를 자동으로 집으므로 그쪽은 손댈 게 없다** — 다만 **테스트 바이너리 수가 43 → 늘어난다**(그 수치의 정본은 CLAUDE.md 「빌드·검증 명령」이고 여기서 세지 않는다).

★**ADR-0130 의 `frame_port` feature 미결은 깨어나지 않는다**★ — ADR-0177 「영향」이 *「새 crate 가 그 계약을 참조하려 하면 즉시 걸린다」*고 경고했는데, 이 설계는 `net::frame_port` 를 **참조하지 않는다**(워크스페이스 의존 0). `Frame`·`Link` 를 자기 것으로 새로 정의하며, 그 둘이 net 의 것과 **모양이 겹치는 것은 의도**다(나중에 단계 ④ 에서 net 이 걷힐 때 한쪽이 사라진다).

---

## 10. ★안 하는 것★

| 안 하는 것 | 왜 |
|---|---|
| **화면 · 로그 창** | ADR-0182 결정 2. *"올리는 것까지 해야지. 화면 만드는 건 나중에 view에서."* ★올리기만 하고 화면이 없으므로 사용자에게 보이는 변화가 0 이다 — 그것이 의도다★ |
| **셸을 이 crate 위로 이사** | 붙이는 순서 ②. 이번 범위는 ① 뿐 |
| **`Registry` 를 N 개로 채워 쓰기** | 붙이는 순서 ③. **API 는 지금 N 이지만 셸이 실제로 여럿을 담는 것은 나중이다** |
| **데몬 쪽 적용** | 붙이는 순서 ④ |
| **기존 `net` 수정** | ADR-0177 결정 1 — ★이번 작업에서 뜯지 않는다★ |
| **데몬끼리 메시지 중계** | 주소·되돌아옴 방지·상대 부재가 전부 미설계. 나중 리서치(§12-1) |
| **원격 인증 · 페어링 · TLS · 원격 주소 발견** | ADR-0177 「미룬 것」. 인증은 **다중 클라 신원과 같은 문제라 함께 푼다** |
| **클라별 되감기 · 다중 클라 입력 권한 · 화면 상태 동기화** | ADR-0179 「영향」이 넷 다 미결로 남겼다 |
| **짝짓기 번호 생성** | `protocol` 에 정본이 있다(`RequestId::new`) |
| **봉투 struct 정의** | ADR-0177 결정 5 — ★봉투는 소비자가 소유한다★. wire 바이트를 안 바꾼다 |
| **버전·능력 협상 판정** | 소비자(데몬)의 `Handshake` 구현 몫(ADR-0178) |
| **중복 흡수(idempotency) 저장소** | ADR-0162 가 「결과 저장소 대신 상태 조회」로 닫았다 |
| **포트파일 · 단일 인스턴스** | 전송이 아니라 데몬 살림. discovery 가 이미 쓴다 |
| **ts-rs 바인딩 / `bindings/`** | 이 crate 는 생성물을 만들지 않는다 — CI sync 게이트 경로 목록에 넣지 말 것 |
| **`client`/`server` feature 로 방향 가르기** | ★방향은 별도 층이 아니라 **고르는 능력**이다★(ADR-0177 결정 3) — `dial` 이냐 `accept` 냐는 런타임 선택이지 컴파일 시간 선택이 아니다. 그래서 조사 §7-2 의 **feature union 벽**(「client 는 켜되 server 는 끈다」를 Cargo 가 표현 못 한다)에 애초에 안 걸린다 |

---

## 11. 의존

| crate | 자리 | 왜 |
|---|---|---|
| `tokio` (`rt`·`sync`·`time`·`macros`) | **무조건** | 감독 태스크·타이머·채널. 소비자 셋(데몬·셸·모바일)이 전부 tokio 라 런타임을 optional 로 두는 것은 **아무도 안 쓰는 재사용성**을 산다(ADR-0177 결정 2 가 범위를 그 셋으로 좁혔다) |
| `futures-util` | **무조건** | `BoxFuture`·`select`. dyn seam 이 `async fn` in trait 을 못 써서 필요하다(net 이 같은 이유로 쓴다) |
| `tokio-tungstenite = "0.26"` | **feature `ws`** | 버전을 워크스페이스에 맞춘다(실측 — `net`·`daemon`·`src-tauri` 가 전부 `0.26`) |
| — | — | ★**`tracing` 을 들이지 않는다**★ — 사건 스트림이 그 자리다(§7). **대가:** 소비자가 사건을 안 읽는 동안은 **아무 흔적도 안 남는다.** 소비자가 사건을 `tracing` 으로 다리 놓는다. §12-10 에 열어 둔다 |
| — | — | `serde` 없음(직렬화는 `Wire` 몫) · `thiserror` 없음(오류 타입이 적어 손으로 `Display`. `net` 도 같다) · `rand` 없음(§6 지터) · `uuid` 없음(`Tag`·`StreamKey` 는 제네릭이라 crate 가 uuid 를 안 본다) |

**워크스페이스 crate 의존 = 0.** 게이트 = §9-2 게이트 1(정확히 1줄 = 자기 자신).

**features:** `default = []` · `ws` · `test-support`. `default = []` 인 사유는 `net` 과 같다 — *조용한 회귀보다 시끄러운 실패*. **단 이 저장소에서 빈 default 의 실이득은 「소비자 쪽 실수를 시끄럽게 만드는 것」이지 「빌드 그래프에서 빼는 것」이 아니다**(feature 합집합은 cargo 호출 1회 단위라 루트 빌드에서는 어차피 켜진다 — 조사 §7-3). `test-support` 는 `command` 의 선례를 따른다(`crates/engram-dashboard-command/src/testing.rs:1` — *「하네스 전용 — 기능 플래그 `test-support` 뒤에 산다(ADR-0012)」*).

---

## 12. 열린 질문

★**지어내서 닫지 않는다. 아래는 전부 사용자가 정할 것이거나, 정할 때까지 기록만 하는 것이다.**★

### 이 설계가 부딪혀 새로 연 것

1. **사건을 인바운드와 한 줄로 흘리는 것으로 충분한가, 나중 로그 창을 위해 방송을 하나 더 두는가.** 한 줄이면 ADR-0177 결정 9 를 문자 그대로 지키지만, 소비자가 사건을 소비해 버리면 **두 번째 독자(로그 창)가 못 본다.** 방송을 더 두면 그 함정이 다시 열린다.
2. ★**느린 소비자 정책 — 「현행 유지」가 어느 현행인가.**★ ADR-0177 결정 6 이 「차면 끊는다(현행 유지)」라 적었는데 **오늘 `net` 은 두 정책을 동시에 쓴다**: 연결당 `try_send` 실패는 **끊고**(`ws.rs:171-179`), 팬아웃 `broadcast_text` 는 **그 연결 것만 조용히 버린다**(`ws.rs:132-146` · `frame_port.rs:67-77`). 후자가 `docs/tracking.md` 에 실물 결함으로 적혀 있다(명부 프레임 유실 → 산 슬롯이 종료 막을 쓴 채 남는다). **crate 의 팬아웃 기본값을 어느 쪽으로 두나.**
3. **큐 상한 기본값.** 셸 쪽 512 는 현행 보존이라 안전한데, **데몬 쪽에 붙일 때(단계 ④) 되감기 링 이하로 잡아야 한다**(ADR-0177 근거의 4608 > 4096 적출). 지금 정할지 그때 정할지.
4. **`write_deadline` 값.** 실측 없음. 5s 는 업계 참고값 사이에서 고른 것이고 근거가 「`ping_interval` 보다 작아야 한다」뿐이다.
5. **양방향 keepalive 를 켜면 데몬이 클라 N 개에 각각 20초마다 ping 을 보낸다.** 오늘은 클라가 하나라 문제가 안 됐다. 주기를 그대로 두나, 상대 수에 따라 늘리나.
6. **재연결 상한을 3 으로 바꾸면 네 자리가 갈린다.** `connection.rs:77`(상수 5) · `src/api/wsTransport.ts:352`(상수 5) · `src/api/agentClient.ts:132`·`src/api/daemonControl.ts:92`(산문 「5회」). ★crate 기본값 3 은 ADR-0180 「영향」이 명시적으로 허용한다★(위반은 값이 아니라 *주입 경로가 없어지는 것*). **누가 언제 맞추나 — 이번 범위 밖이지만 crate 를 세우는 순간 두 층이 갈린다.**
7. ★**`Listener`(accept 쪽) 를 지금 확정하나 껍데기만 두나.**★ **여기서 「나중에 리팩터 안 하게」와 「구조 작게」가 정면으로 부딪힌다.** ADR-0179 결정 3 이 *「N 개에 붙는다」와 「N 개를 받는다」가 같은 레지스트리·같은 감독*이라 못박았고, `Handshake` 는 이미 방향 대칭이라 accept 쪽이 그대로 얹힌다. 그러나 **이번 범위(①)는 dial 쪽만 검증한다.** 갈리는 대가: 지금 확정하면 검증 없는 공개 타입이 하나 늘고, 나중에 두면 `Registry::new` 시그니처가 그때 바뀐다(소비자 전부가 손을 댄다).
8. **`PeerId` 를 소비자가 짓나 crate 가 번호를 뽑나.** 이 값이 **사건의 출처 표식**이라 나중에 화면에 그대로 뜬다 — 사용자 체감에 걸린다. 지금 안은 소비자가 문자열을 준다(데몬의 데이터 폴더 이름 같은 것).
9. **지터 난수원.** 의존 없이 시계+`PeerId` 해시에서 뽑을 것인가, `uuid`(또는 `rand`)를 들일 것인가. 지금 안은 전자이고 선례가 있다(`profile.rs:680`).
10. **`tracing` 부재.** 소비자가 사건을 안 읽는 동안은 흔적이 0 이다. 받아들이나, 아니면 crate 가 사건을 `tracing` 으로도 흘리나.
11. **`Wire::decode` 실패 시 그 프레임만 버리나 연결을 끊나.** 사용자 체감이다(프레임 하나가 깨졌다고 화면이 통째로 끊기나). 지금 안은 **정책 값**으로 열어 두고 기본을 「버리고 사건만」으로 두는 것인데, 그러면 프로토콜이 어긋난 채 계속 도는 조합이 생긴다.
12. **인바운드 큐가 차면.** 오늘 셸엔 이 큐 자체가 없다(Tauri Channel 로 직행). 새 crate 는 `Incoming` 채널을 두므로 **새 실패 모드가 생긴다** — 셸이 느릴 때 무엇을 하나.
13. **crate 이름 접두.** 이 문서는 `engram-dashboard-transport` 로 잡았다(다른 여섯 멤버와 같은 관례). ADR-0177 결정 1 은 「`transport`」라고만 적었고 같은 ADR 「영향」은 접두를 벗으면 게이트가 안 본다고 경고한다 — **두 줄이 안 맞아 한쪽을 골라야 한다.**

### 정하지 말고 기록만 (지시받은 대로)

14. **데몬끼리 메시지 중계** — 주소를 어떻게 알고, 되돌아옴을 어떻게 막고, 상대가 없을 때 무엇을 하나. **나중 리서치.**
15. **「표시는 이름, 배달은 고유키」** — 에이전트 식별자는 이미 UUID 라(`crates/engram-dashboard-protocol/src/ids.rs:4`) **내부는 이미 고유키**이고, **우편(messaging)만 이름을 주소로 쓴다.** 같은 이름이 둘일 때의 배달 규칙은 **나중 리서치.**
16. **재연결 상수 `5`** — 위 6 번의 기록면. 코드 두 곳 + 문구 두 곳.

---

## 13. 근거 · 검증 상태

- ★**이 문서는 코드를 한 줄도 고치지 않았다.**★ 아래 인용은 전부 **정적 판독**이고, 테스트나 빌드를 돌리지 않았다.
- **직접 열어 확인한 자리:** `src-tauri/src/commands/agent.rs:240-276` · `src-tauri/src/daemon_client/*`(구조 조사) · `crates/engram-dashboard-net/src/{lib,ws,frame_port}.rs`(경계 조사) · `crates/engram-dashboard-protocol/src/{codec,ids,messages}.rs` · `crates/engram-dashboard-agent/src/profile.rs:662-682` · `crates/engram-dashboard-daemon/src/command_delivery.rs:176-187` · `crates/engram-dashboard-command/src/{lib,testing}.rs` · `.github/workflows/ci.yml:485-534` · 루트 `Cargo.toml` · `docs/testing-strategy.md`.
- **`AgentCommand` 갈래 = 30**(실측 2026-09-06, `awk` + `rg -c`) — ADR-0181 이 든 수치와 일치.
- **`REPLY_TIMEOUT` 은 `src-tauri/src/commands/` 를 통틀어 유일한 요청 시한이다**(실측: `rg "REPLY_TIMEOUT|tokio::time::timeout|Duration::from_" src-tauri/src/commands/` → 3줄, 그중 하나는 discovery 의 5초 폴링이라 축이 다르다).
- **미검 표시가 붙은 값**(§6): `request_timeout` 8s · `write_deadline` · `reconnect.jitter` · `inbound_queue` · `retired_tags`.
- **재판정 안 한 것:** `docs/research/reusable-transport-crate-boundary-2026-09-03.md` 는 적대 리뷰 판정이 **BLOCK → 수정 반영, 재판정 미실행** 상태다. 이 문서가 그 조사를 근거로 인용할 때 그 상태를 함께 읽어야 한다.
