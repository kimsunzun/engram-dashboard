//! 상대 하나의 감독 태스크 — ★구동 층★.
//!
//! [`crate::machine`] 이 정한 것을 [`crate::link`]·[`crate::clock`] 위에서 집행한다. 순수 층이 "무엇을"
//! 이라면 이 파일은 "어떻게" 다.
//!
//! ## 이 파일이 지는 불변식
//!
//! - ★단일 writer★ — [`LinkTx`] 를 쥐는 것은 감독 태스크 하나뿐이다. 밖에서 오는 것은 전부 큐를 지난다.
//! - ★대기 슬롯 등록은 **보내기 전에**★ — 뒤집으면 빠른 답장이 슬롯을 못 찾는다(오늘 셸도 그렇다).
//! - ★연결을 버리는 **모든** 갈래가 대기자를 깨운다★ — 표에 있는 것뿐 아니라 **큐에 남은 요청과 운영
//!   단계를 떠난 뒤 도착하는 요청까지**. 하나라도 빠지면 그 슬롯은 영구 대기가 된다: 포기 상태에는
//!   시한을 돌릴 루프가 없고, 대기 중인 호출자가 핸들을 붙들고 있어 채널이 닫히지도 않는다.
//! - ★모든 쓰기에 시한이 붙는다★ — `write_deadline < ping_interval` 이라 진행 중 쓰기가 keepalive 를
//!   굶기지 못한다. 오늘 `net` 의 실물 결함이 정확히 그 굶김이다.
//! - ★핸들은 재연결에 불사★ — 세대 경계는 핸들을 죽이지 않고 [`crate::Arrival::generation`] 에 표식으로
//!   나온다.
//!
//! ## 운영 루프의 우선순위 (되살리지 마라)
//!
//! 옛 판은 `select!` 를 `biased` 로 두고 프레임 팔을 타이머·나가는 큐보다 앞에 놓았다. `biased` 는 선언
//! 순서로 폴링하고 **먼저 준비된 팔에서 멈추므로**, 프레임이 처리 속도만큼만 들어와도 뒤의 두 팔이
//! **한 번도 폴링되지 않는다.** 그러면 ping 이 안 나가 상대가 우리를 침묵으로 끊고, 요청 시한이 안 돌아
//! 대기자가 상한을 넘겨 매달리고, 나가는 큐가 차서 기본 정책이 스스로 연결을 끊는다 — ★출력이 가장
//! 많을 때 골라서 끊어지는★ 모양이다. 지금 루프는 ① 제어를 매 바퀴 먼저 걷고 ② 시한이 이미 지났으면
//! `select` 를 건너뛰며 ③ 프레임을 [`FRAME_BURST`] 개 처리할 때마다 나가는 큐에 한 바퀴를 양보한다.
//!
//! ## 채널이 둘인 이유
//!
//! 데이터 큐(유계)와 제어 큐(무계)를 가른다. ★큐가 꽉 찬 순간에도 「닫아라」·「지금 다시 붙어라」는
//! 들어가야 한다★ — 한 채널이면 배압이 제어까지 막는다.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{mpsc, oneshot, watch};

use crate::clock::{with_deadline, Clock};
use crate::event::{Arrival, Direction, Incoming, PeerId, TransportEvent};
use crate::frame::{Close, CloseCode, Frame};
use crate::link::{Address, Dialer, Handshake, HandshakeStep, LinkError, LinkRead, LinkRx, LinkTx};
use crate::machine::{
    Action, ConnectFailure, DisconnectCause, Generation, Input, Machine, MachineEvent, PeerState,
};
use crate::pending::{Insert, Pending, RequestError};
use crate::policy::{DecodeFailurePolicy, Policy, QueueFullPolicy};
use crate::stream::{Streams, Verdict};
use crate::wire::Wire;

/// 프레임을 이만큼 연달아 처리하면 나가는 큐에 한 바퀴를 양보한다.
const FRAME_BURST: usize = 32;

/// 운영 루프의 이번 바퀴가 무엇을 먼저 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Turn {
    /// 시한이 이미 지났다 — `select` 를 건너뛰고 타이머부터 본다.
    Timer,
    /// 나가는 큐를 이만큼 비운다.
    Outbound(usize),
    /// 평소 — 네 팔을 공평하게 기다린다.
    Wait,
}

/// ★굶주림 방지 규칙이 사는 유일한 자리★.
///
/// 순수 함수인 것은 의도다: 이 규칙이 지키는 성질(프레임이 끊이지 않아도 타이머와 나가는 큐가
/// 돈다)은 **인메모리 하네스로 재현되지 않는다** — 그쪽에서는 읽기 태스크가 구조적 병목이라
/// 프레임 큐가 늘 비고, 그러면 `select` 가 무엇을 하든 뒤의 팔이 폴링된다(실측: `biased` 를 되살린
/// 변이가 홍수 테스트를 하나도 못 빨갛게 만들었다). 그래서 **규칙 자체를 여기서 단언**한다.
fn next_turn(
    now: Instant,
    wake: Instant,
    frames_in_row: usize,
    outbound_len: usize,
    outbound_cap: usize,
) -> Turn {
    if wake <= now {
        return Turn::Timer;
    }
    if outbound_len == 0 {
        return Turn::Wait;
    }
    // ★고정 비율만으로는 부족하다★ — 32 프레임마다 한 개씩 빼면 그 비율을 넘는 생산자 앞에서
    //   큐는 여전히 자라 `QueueFull` 로 끝난다(문턱만 32배 높아진 같은 모양이다). 그래서 규칙이
    //   **깊이**를 본다: 절반을 넘으면 버스트를 기다리지 않고, 양보할 때는 쌓인 만큼 비운다.
    if outbound_len.saturating_mul(2) >= outbound_cap || frames_in_row >= FRAME_BURST {
        return Turn::Outbound(outbound_len);
    }
    Turn::Wait
}

/// 답장을 안 기다리는 것을 밀어 넣지 못했다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    /// 나가는 큐가 찼다.
    QueueFull,
    /// 지금 운영 단계가 아니다(연결 중·백오프 중·포기함).
    Disconnected,
    /// 감독 태스크가 사라졌다 — 이 상대는 끝났다.
    Closed,
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::QueueFull => f.write_str("outbound queue full"),
            Self::Disconnected => f.write_str("peer is not live"),
            Self::Closed => f.write_str("peer supervisor is gone"),
        }
    }
}

impl std::error::Error for SendError {}

enum PeerCmd<W: Wire> {
    Request {
        out: W::Out,
        reply: oneshot::Sender<Result<W::In, RequestError>>,
    },
    Notify {
        out: W::Out,
    },
    /// 팬아웃 — 명부가 한 번만 인코딩해 상대 수만큼 나눠 준다.
    Raw(Frame),
    Resume {
        key: W::StreamKey,
        after: Option<u64>,
    },
}

enum Ctl {
    ReconnectNow,
    Close,
}

/// 상대가 「너를 안 받는다」고 **말한** 것인가.
///
/// ★정상 종료 계열은 거절이 아니다★ — RFC 6455 의 `1000`(Normal)·`1001`(Going Away)과 우리
/// [`CloseCode::GOING_AWAY`] 는 「나 이제 간다」이지 거절이 아니다. 그것을 거절로 읽으면 데몬이 곱게
/// 종료할 때마다 클라가 **재시도 한 번 없이** 포기한다. 나머지 코드는 상대가 굳이 골라 실은 것이므로
/// 그 말을 그대로 받는다.
fn is_rejection(code: CloseCode) -> bool {
    !matches!(code.0, 1000 | 1001) && code != CloseCode::GOING_AWAY
}

/// 마지막으로 알려진 끊김 사유를 핸들 쪽에서도 읽게 하는 최소 표현.
fn cause_code(cause: DisconnectCause) -> u8 {
    match cause {
        DisconnectCause::PeerClosed => 0,
        DisconnectCause::LinkError => 1,
        DisconnectCause::Idle => 2,
        DisconnectCause::WriteDeadline => 3,
        DisconnectCause::QueueFull => 4,
        DisconnectCause::Explicit => 5,
    }
}

fn cause_from_code(code: u8) -> DisconnectCause {
    match code {
        0 => DisconnectCause::PeerClosed,
        2 => DisconnectCause::Idle,
        3 => DisconnectCause::WriteDeadline,
        4 => DisconnectCause::QueueFull,
        5 => DisconnectCause::Explicit,
        _ => DisconnectCause::LinkError,
    }
}

struct PeerInner<W: Wire> {
    id: PeerId,
    data_tx: mpsc::Sender<PeerCmd<W>>,
    ctl_tx: mpsc::UnboundedSender<Ctl>,
    state_rx: watch::Receiver<PeerState>,
    generation: Arc<AtomicU64>,
    /// 핸들이 **큐가 차서** 못 넣은 횟수. 감독이 걷어 가 [`QueueFullPolicy`] 대로 처리한다.
    overflow: Arc<AtomicU64>,
    /// 핸들이 **운영 단계가 아니라서** 버린 횟수. ★연결을 끊지 않는다★ — 이미 안 붙어 있다.
    refused: Arc<AtomicU64>,
    /// 마지막 끊김 사유. 한 번도 안 붙었으면 [`DisconnectCause::LinkError`] 다.
    last_cause: Arc<AtomicU8>,
}

/// 값싼 clone 핸들 = 감독 태스크로 가는 명령 채널.
///
/// ★재연결을 가로질러 살아남는다★ — 끊겼다고 이 핸들이 죽지 않는다. 소비자 코드가 짧아지는 대신
/// 조용한 유실이 안 생기도록 **데이터 쪽에 세대 표식**을 붙인다(ADR-0163/0164 의 idiom).
pub struct Peer<W: Wire> {
    inner: Arc<PeerInner<W>>,
}

impl<W: Wire> Clone for Peer<W> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<W: Wire> Peer<W> {
    pub fn id(&self) -> &PeerId {
        &self.inner.id
    }

    /// 지금 상태 한 장.
    pub fn peer_state(&self) -> PeerState {
        *self.inner.state_rx.borrow()
    }

    /// 상태 변화를 지켜보는 손잡이.
    pub fn state(&self) -> watch::Receiver<PeerState> {
        self.inner.state_rx.clone()
    }

    /// 지금 연결 세대. 한 번도 안 붙었으면 명부가 준 바닥값이다.
    pub fn generation(&self) -> Generation {
        Generation(self.inner.generation.load(Ordering::Acquire))
    }

    /// 답장을 기다린다.
    ///
    /// 오류 조건: 시한 만료 · 끊김 · 큐 포화 · 번호 겹침 · 만료 번호 재사용 ·
    /// [`Wire::request_tag`] 가 `None`([`RequestError::NotARequest`] — `notify` 를 쓰라는 뜻).
    ///
    /// ★시한이 실제로 도달하는 경우는 「연결은 멀쩡한데 그 요청만 침묵」 하나뿐이다★ — 연결을 버리는
    /// 어느 갈래든 대기자를 즉시 깨우기 때문이다.
    pub async fn request(&self, out: W::Out) -> Result<W::In, RequestError> {
        if !self.peer_state().is_live() {
            return Err(self.disconnected());
        }
        let (tx, rx) = oneshot::channel();
        match self
            .inner
            .data_tx
            .try_send(PeerCmd::Request { out, reply: tx })
        {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.inner.overflow.fetch_add(1, Ordering::AcqRel);
                return Err(RequestError::QueueFull);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => return Err(self.disconnected()),
        }
        // 감독이 사라져 슬롯이 함께 떨어져도 여기서 오류로 깨어난다.
        rx.await.unwrap_or_else(|_| Err(self.disconnected()))
    }

    /// 답장을 안 기다린다. ★await 이 없다★ — 큐에 못 넣으면 즉시 `Err`.
    pub fn notify(&self, out: W::Out) -> Result<(), SendError> {
        if !self.peer_state().is_live() {
            return Err(SendError::Disconnected);
        }
        self.push(PeerCmd::Notify { out })
    }

    /// 스트림을 이어받는다. 반환값은 이 요청이 실린 연결 세대다.
    ///
    /// ★받아들여졌다는 뜻이 아니다★ — 운영 단계가 아니거나 큐가 찼으면 요청은 버려지고
    /// [`TransportEvent::Dropped`] 로만 보인다. 반환값은 "지금 세대가 이것이다" 일 뿐이다.
    pub fn resume_stream(&self, key: W::StreamKey, after: Option<u64>) -> Generation {
        if self.peer_state().is_live() {
            let _ = self.push(PeerCmd::Resume { key, after });
        } else {
            // 부르는 쪽이 오류를 못 받는 유일한 입구다 — 세지 않으면 조용한 유실이 된다.
            self.inner.refused.fetch_add(1, Ordering::AcqRel);
        }
        self.generation()
    }

    /// ★「지금 다시 해라」 입구★ — 예산을 초기화하고 즉시 시도한다(ADR-0180 결정 7). 포기 상태에서도 듣는다.
    pub fn reconnect_now(&self) {
        let _ = self.inner.ctl_tx.send(Ctl::ReconnectNow);
    }

    /// 명시 종료 — 다시 붙지 않는다.
    ///
    /// ★완료 신호가 없다★ — 돌아온 시점에 감독이 아직 통로를 닫는 중일 수 있고, 그 사이 핸드셰이크를
    /// 마쳐 **세대를 하나 더 발행할 수도** 있다. 같은 이름표를 다시 올리는 쪽은 그 겹침을
    /// **이름표별 공용 세대 계수기**로 흡수한다(`Registry::add_with_policy`) — 겹침을 없애는 것이
    /// 아니라 겹쳐도 번호가 부딪히지 않게 하는 장치다.
    pub fn close(&self) {
        let _ = self.inner.ctl_tx.send(Ctl::Close);
    }

    pub(crate) fn send_frame(&self, frame: Frame) -> Result<(), SendError> {
        if !self.peer_state().is_live() {
            return Err(SendError::Disconnected);
        }
        self.push(PeerCmd::Raw(frame))
    }

    fn disconnected(&self) -> RequestError {
        RequestError::Disconnected {
            generation: self.generation(),
            cause: cause_from_code(self.inner.last_cause.load(Ordering::Acquire)),
        }
    }

    fn push(&self, cmd: PeerCmd<W>) -> Result<(), SendError> {
        match self.inner.data_tx.try_send(cmd) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.inner.overflow.fetch_add(1, Ordering::AcqRel);
                Err(SendError::QueueFull)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => Err(SendError::Closed),
        }
    }
}

/// 감독 태스크를 세우는 데 필요한 것 전부.
pub(crate) struct PeerConfig<W: Wire> {
    pub id: PeerId,
    pub addr: Address,
    pub wire: Arc<W>,
    pub dialer: Arc<dyn Dialer>,
    pub clock: Arc<dyn Clock>,
    pub handshake: Arc<dyn Handshake>,
    pub policy: Policy,
    pub inbound: mpsc::Sender<Incoming<W>>,
    /// ★이 이름표의 세대 계수기★ — 같은 이름표를 다시 올려도 **같은 Arc 를 물려준다**.
    ///
    /// 옛 감독의 마지막 세대를 읽어 바닥값으로 넘기는 방식은 경합이 있었다(D1): 옛 감독이
    /// `shake_hands` 의 `select` 안에서 핸드셰이크와 큐에 든 `Close` 를 **동시에** 준비된 채로 만나면
    /// 핸드셰이크가 이겨 세대를 하나 더 발행할 수 있는데, 명부는 그 직전 값을 이미 읽어 간 뒤다.
    /// 계수기를 공유하면 누가 이기든 같은 번호가 두 번 나오지 않는다.
    pub generations: Arc<AtomicU64>,
}

/// 감독 태스크를 띄우고 핸들을 돌려준다.
pub(crate) fn spawn_peer<W: Wire>(cfg: PeerConfig<W>) -> Peer<W> {
    let (data_tx, data_rx) = mpsc::channel(cfg.policy.outbound_queue.max(1));
    let (ctl_tx, ctl_rx) = mpsc::unbounded_channel();
    let (state_tx, state_rx) = watch::channel(PeerState::Idle);
    let generation_floor = cfg.generations.load(Ordering::Acquire);
    let generation = Arc::new(AtomicU64::new(generation_floor));
    let overflow = Arc::new(AtomicU64::new(0));
    let refused = Arc::new(AtomicU64::new(0));
    let last_cause = Arc::new(AtomicU8::new(cause_code(DisconnectCause::LinkError)));

    let handle = Peer {
        inner: Arc::new(PeerInner {
            id: cfg.id.clone(),
            data_tx,
            ctl_tx,
            state_rx,
            generation: generation.clone(),
            overflow: overflow.clone(),
            refused: refused.clone(),
            last_cause: last_cause.clone(),
        }),
    };

    let now = cfg.clock.now();
    let supervisor = Supervisor {
        machine: Machine::new(
            cfg.policy.clone(),
            cfg.id.jitter_seed(),
            now,
            generation_floor,
        ),
        pending: Pending::new(cfg.policy.retired_tags),
        streams: Streams::new(
            cfg.policy.max_streams,
            cfg.policy.resume_buffer,
            cfg.policy.resume_timeout,
        ),
        id: cfg.id,
        addr: cfg.addr,
        wire: cfg.wire,
        dialer: cfg.dialer,
        clock: cfg.clock,
        handshake: cfg.handshake,
        policy: cfg.policy,
        inbound: cfg.inbound,
        data_rx,
        ctl_rx,
        state_tx,
        generations: cfg.generations,
        generation,
        overflow,
        refused,
        last_cause,
        tx: None,
        rx_raw: None,
        frames_rx: None,
        reader: None,
        fresh_link: false,
        consumer_gone: AtomicBool::new(false),
        last_rx: now,
        last_ping: now,
        dropped_in: 0,
        dropped_out: 0,
    };
    tokio::spawn(supervisor.run());
    handle
}

enum LinkMsg {
    Frame(Frame),
    /// 상대가 닫았다.
    ///
    /// ★여기에는 닫기 코드를 싣지 않는다★ — 이 태스크는 **핸드셰이크가 끝난 뒤에만** 돌고, 운영 중
    /// 끊김이 나르는 것은 [`DisconnectCause`] 뿐이라 코드를 실을 칸이 없다(TRD §7 의 사건 표 그대로).
    /// 코드가 판정을 가르는 자리는 핸드셰이크이고, 그쪽은 [`LinkRx`] 를 직접 읽어 [`LinkRead::Closed`]
    /// 의 코드를 그대로 본다.
    Closed,
    /// ★통로가 준 문구를 여기서 버린다★ — [`TransportEvent::Disconnected`] 가 나르는 것은
    /// [`DisconnectCause`] 뿐이고 문구를 실을 칸이 없다(TRD §7 의 사건 표 그대로).
    Failed,
}

enum Wake<W: Wire> {
    Ctl(Option<Ctl>),
    Link(Option<LinkMsg>),
    Cmd(Option<PeerCmd<W>>),
    Timer,
}

enum Down<W: Wire> {
    Ctl(Option<Ctl>),
    Cmd(Option<PeerCmd<W>>),
    Elapsed,
}

struct Supervisor<W: Wire> {
    id: PeerId,
    addr: Address,
    wire: Arc<W>,
    dialer: Arc<dyn Dialer>,
    clock: Arc<dyn Clock>,
    handshake: Arc<dyn Handshake>,
    policy: Policy,
    machine: Machine,
    pending: Pending<W::Tag, oneshot::Sender<Result<W::In, RequestError>>>,
    streams: Streams<W::StreamKey, W::In>,
    inbound: mpsc::Sender<Incoming<W>>,
    data_rx: mpsc::Receiver<PeerCmd<W>>,
    ctl_rx: mpsc::UnboundedReceiver<Ctl>,
    state_tx: watch::Sender<PeerState>,
    /// 이 이름표가 발행한 세대의 공용 계수기(D1).
    generations: Arc<AtomicU64>,
    generation: Arc<AtomicU64>,
    overflow: Arc<AtomicU64>,
    refused: Arc<AtomicU64>,
    last_cause: Arc<AtomicU8>,
    tx: Option<Box<dyn LinkTx>>,
    /// 핸드셰이크 동안만 쥐는 읽는 쪽. 운영 단계로 들어서면 [`read_link`] 태스크로 넘어간다.
    rx_raw: Option<Box<dyn LinkRx>>,
    frames_rx: Option<mpsc::Receiver<LinkMsg>>,
    reader: Option<tokio::task::JoinHandle<()>>,
    /// 방금 붙었다 — 미결 이어받기를 다시 내보내야 한다.
    fresh_link: bool,
    /// 위로 올릴 데가 아예 닫혔다. ★다시 붙을 일이 아니라 접을 일이다★.
    consumer_gone: AtomicBool,
    last_rx: Instant,
    last_ping: Instant,
    dropped_in: u64,
    dropped_out: u64,
}

impl<W: Wire> Supervisor<W> {
    async fn run(mut self) {
        let mut acts = self.machine.on(self.clock.now(), Input::Start);
        loop {
            self.publish();
            let follow = self.apply(acts).await;
            self.publish();
            if self.machine.state() == PeerState::Closed {
                break;
            }
            let input = match follow {
                Some(input) => input,
                None if self.consumer_gone.load(Ordering::Acquire) => Input::Close,
                None => match self.machine.state() {
                    PeerState::Live => self.run_live().await,
                    PeerState::GaveUp(_) | PeerState::Idle => self.park().await,
                    other => {
                        // Dialing/Backoff/Handshaking 는 `apply` 가 항상 후속 입력을 낸다.
                        // ★여기서 상대를 죽이지 않는다★ — 도달했다면 그건 우리 실수이고, 그 대가를
                        //   「이 연결을 영구히 끝냄」으로 치르는 것이 가장 파괴적인 기본값이다.
                        debug_assert!(false, "후속 입력 없이 {other:?} 로 떨어졌다");
                        Input::ReconnectNow
                    }
                },
            };
            let now = self.clock.now();
            acts = self.machine.on(now, input);
        }
        self.teardown().await;
    }

    fn publish(&mut self) {
        self.generation
            .store(self.machine.generation().0, Ordering::Release);
        self.state_tx.send_replace(self.machine.state());
    }

    async fn apply(&mut self, acts: Vec<Action>) -> Option<Input> {
        let mut follow = None;
        for act in acts {
            match act {
                Action::Emit(event) => self.emit_machine(event),
                Action::FailPending(cause) => self.fail_pending(cause),
                Action::DropLink { code } => self.drop_link(code).await,
                Action::Dial { deadline } => follow = Some(self.dial(deadline).await),
                Action::Handshake { deadline } => follow = Some(self.shake_hands(deadline).await),
                Action::Wait { until } => follow = Some(self.wait(until).await),
            }
        }
        follow
    }

    /// 쌓여 있는 제어 신호를 한 번에 걷는다. `Close` 가 `ReconnectNow` 를 이긴다.
    ///
    /// ★연달아 온 `ReconnectNow` 를 하나로 접는 것이 요점이다★ — 접지 않으면, 셸이 감독보다 빠르게
    /// 누를 때 매번 dial 을 **폴링되기도 전에** 취소해 「끊고 → 걸다 말고」를 무한히 되풀이한다.
    fn drain_ctl(&mut self) -> Option<Ctl> {
        let mut seen = None;
        while let Ok(signal) = self.ctl_rx.try_recv() {
            match signal {
                Ctl::Close => return Some(Ctl::Close),
                Ctl::ReconnectNow => seen = Some(Ctl::ReconnectNow),
            }
        }
        seen
    }

    // ── 연결 수립 ────────────────────────────────────────────────────────────

    /// ★두 구간(dial · 핸드셰이크) 내내 제어 신호를 듣는다★ — 안 그러면 `close()` 가 `connect_timeout`
    /// 만큼 늦게 들린다. 제어가 이기면 통로는 취소된 future 와 함께 닫힌다.
    async fn dial(&mut self, deadline: Instant) -> Input {
        // 이미 쌓여 있던 「지금 다시 붙어라」는 지금 하려는 일 그 자체이므로 삼킨다.
        if let Some(Ctl::Close) = self.drain_ctl() {
            return Input::Close;
        }
        let dialer = self.dialer.clone();
        let addr = self.addr.clone();
        let clock = self.clock.clone();
        let outcome = {
            let ctl = &mut self.ctl_rx;
            tokio::select! {
                signal = ctl.recv() => Err(signal),
                result = dial_once(dialer, addr, clock, deadline) => Ok(result),
            }
        };
        match outcome {
            Ok(Ok((tx, rx))) => {
                self.tx = Some(tx);
                self.rx_raw = Some(rx);
                Input::Dialed
            }
            Ok(Err(failure)) => Input::ConnectFailed(failure),
            Err(Some(Ctl::ReconnectNow)) => Input::ReconnectNow,
            Err(Some(Ctl::Close)) | Err(None) => Input::Close,
        }
    }

    async fn shake_hands(&mut self, deadline: Instant) -> Input {
        // ★`dial` 의 같은 줄과 짝이다★ — 쌓인 「지금 다시 붙어라」를 여기서도 접는다.
        if let Some(Ctl::Close) = self.drain_ctl() {
            return Input::Close;
        }
        let (Some(tx), Some(rx)) = (self.tx.take(), self.rx_raw.take()) else {
            return Input::ConnectFailed(ConnectFailure::Unreachable(LinkError::new(
                "no link to hand shake on",
            )));
        };
        let handshake = self.handshake.clone();
        let clock = self.clock.clone();
        let outcome = {
            let ctl = &mut self.ctl_rx;
            tokio::select! {
                signal = ctl.recv() => Err(signal),
                result = handshake_once(tx, rx, handshake, clock, deadline) => Ok(result),
            }
        };
        match outcome {
            Ok(Ok((tx, rx))) => {
                self.tx = Some(tx);
                self.attach_reader(rx);
                // 번호를 여기서 뽑는다 — 이 자리가 「이 통로가 운영 단계에 들어섰다」의 유일한 지점이다.
                let minted = Generation(self.generations.fetch_add(1, Ordering::AcqRel) + 1);
                Input::Connected(minted)
            }
            Ok(Err(failure)) => Input::ConnectFailed(failure),
            Err(Some(Ctl::ReconnectNow)) => Input::ReconnectNow,
            Err(Some(Ctl::Close)) | Err(None) => Input::Close,
        }
    }

    /// 읽기를 따로 태스크로 뗀다 — 그 사유는 [`read_link`] 헤더에 있다.
    fn attach_reader(&mut self, rx: Box<dyn LinkRx>) {
        let (frames_tx, frames_rx) = mpsc::channel(self.policy.frame_buffer.max(1));
        self.reader = Some(tokio::spawn(read_link(rx, frames_tx)));
        self.frames_rx = Some(frames_rx);
        self.fresh_link = true;
        let now = self.clock.now();
        self.last_rx = now;
        self.last_ping = now;
    }

    /// 백오프 대기. ★기다리는 동안에도 큐에 들어오는 요청을 즉시 실패시킨다★ — 안 그러면 그 슬롯이
    /// 다음 운영 단계까지(또는 포기 상태에서는 영원히) 매달린다.
    async fn wait(&mut self, until: Instant) -> Input {
        loop {
            // ★여기서도 신고한다★ — 안 그러면 백오프·포기 중에 쌓인 유실이 사용자가 「다시 연결」을
            //   누를 때까지 아무 데도 안 나온다.
            let _ = self.drain_overflow();
            self.flush_drop_reports();
            if let Some(signal) = self.drain_ctl() {
                return match signal {
                    Ctl::ReconnectNow => Input::ReconnectNow,
                    Ctl::Close => Input::Close,
                };
            }
            let delay = until.saturating_duration_since(self.clock.now());
            if delay.is_zero() {
                return Input::BackoffElapsed;
            }
            let sleep = self.clock.sleep(delay);
            let woken = {
                let ctl = &mut self.ctl_rx;
                let data = &mut self.data_rx;
                tokio::select! {
                    signal = ctl.recv() => Down::Ctl(signal),
                    cmd = data.recv() => Down::Cmd(cmd),
                    _ = sleep => Down::Elapsed,
                }
            };
            match woken {
                Down::Ctl(Some(Ctl::ReconnectNow)) => return Input::ReconnectNow,
                Down::Ctl(Some(Ctl::Close)) | Down::Ctl(None) => return Input::Close,
                Down::Cmd(None) => return Input::Close,
                Down::Cmd(Some(cmd)) => self.reject_command(cmd),
                Down::Elapsed => return Input::BackoffElapsed,
            }
        }
    }

    /// 포기한 채(또는 아직 시작 전) 제어만 기다린다. 대기 요청은 여기서도 즉시 실패시킨다.
    async fn park(&mut self) -> Input {
        loop {
            let _ = self.drain_overflow();
            self.flush_drop_reports();
            if let Some(signal) = self.drain_ctl() {
                return match signal {
                    Ctl::ReconnectNow => Input::ReconnectNow,
                    Ctl::Close => Input::Close,
                };
            }
            let woken = {
                let ctl = &mut self.ctl_rx;
                let data = &mut self.data_rx;
                tokio::select! {
                    signal = ctl.recv() => Down::Ctl(signal),
                    cmd = data.recv() => Down::Cmd(cmd),
                }
            };
            match woken {
                Down::Ctl(Some(Ctl::ReconnectNow)) => return Input::ReconnectNow,
                Down::Ctl(Some(Ctl::Close)) | Down::Ctl(None) => return Input::Close,
                Down::Cmd(None) => return Input::Close,
                Down::Cmd(Some(cmd)) => self.reject_command(cmd),
                Down::Elapsed => return Input::Close,
            }
        }
    }

    // ── 운영 단계 ────────────────────────────────────────────────────────────

    async fn run_live(&mut self) -> Input {
        if std::mem::take(&mut self.fresh_link) {
            if let Some(input) = self.reissue_resumes().await {
                return input;
            }
        }
        let mut frames_in_row = 0usize;
        loop {
            if self.consumer_gone.load(Ordering::Acquire) {
                return Input::Close;
            }
            if let Some(input) = self.drain_overflow() {
                return input;
            }
            self.flush_drop_reports();
            if self.frames_rx.is_none() || self.tx.is_none() {
                return Input::LinkLost(DisconnectCause::LinkError);
            }
            // ① 제어는 매 바퀴 먼저 — 프레임이 끊이지 않아도 즉시 듣는다.
            if let Some(signal) = self.drain_ctl() {
                return match signal {
                    Ctl::ReconnectNow => Input::ReconnectNow,
                    Ctl::Close => Input::Close,
                };
            }

            let now = self.clock.now();
            let wake = self.wake_at();
            // ②③ 굶주림 방지 규칙. 판정은 `next_turn` 이 하고 여기서는 집행만 한다.
            match next_turn(
                now,
                wake,
                frames_in_row,
                self.data_rx.len(),
                self.policy.outbound_queue.max(1),
            ) {
                Turn::Timer => {
                    frames_in_row = 0;
                    if let Some(input) = self.on_timer().await {
                        return input;
                    }
                    // ★진도를 확인한다★ — 어떤 타이머가 자기 시한을 못 밀면 이 루프는 `await` 를 한 번도
                    //   지나지 않고 영원히 돈다. current_thread 런타임에서는 그 순간 **다른 태스크가
                    //   아무것도 못 하고 프로세스가 멎는다**(실측: `expire_resumes` 를 무력화한 변이가
                    //   테스트를 실패시키는 대신 10분 넘게 매달렸다).
                    if self.wake_at() <= now {
                        debug_assert!(
                            false,
                            "타이머가 자기 시한을 못 밀었다 — 운영 루프가 await 없이 돈다"
                        );
                        tokio::task::yield_now().await;
                    }
                    continue;
                }
                Turn::Outbound(budget) => {
                    frames_in_row = 0;
                    for _ in 0..budget {
                        let Ok(cmd) = self.data_rx.try_recv() else {
                            break;
                        };
                        if let Some(input) = self.on_cmd(cmd).await {
                            return input;
                        }
                    }
                    continue;
                }
                Turn::Wait => {}
            }

            let sleep = self.clock.sleep(wake.saturating_duration_since(now));
            let woken = {
                let ctl = &mut self.ctl_rx;
                let data = &mut self.data_rx;
                let frames = self.frames_rx.as_mut().expect("checked above");
                // ★`biased` 를 다시 붙이지 말 것★ — 선언 순서 폴링이 프레임 홍수 아래에서 뒤 팔을
                //   통째로 굶긴다(이 파일 헤더의 「운영 루프의 우선순위」).
                tokio::select! {
                    signal = ctl.recv() => Wake::Ctl(signal),
                    msg = frames.recv() => Wake::Link(msg),
                    cmd = data.recv() => Wake::Cmd(cmd),
                    _ = sleep => Wake::Timer,
                }
            };

            let outcome = match woken {
                Wake::Ctl(Some(Ctl::ReconnectNow)) => Some(Input::ReconnectNow),
                Wake::Ctl(Some(Ctl::Close)) | Wake::Ctl(None) => Some(Input::Close),
                Wake::Link(None) | Wake::Link(Some(LinkMsg::Failed)) => {
                    Some(Input::LinkLost(DisconnectCause::LinkError))
                }
                Wake::Link(Some(LinkMsg::Closed)) => {
                    Some(Input::LinkLost(DisconnectCause::PeerClosed))
                }
                Wake::Link(Some(LinkMsg::Frame(frame))) => {
                    frames_in_row += 1;
                    self.on_frame(frame).await
                }
                Wake::Cmd(None) => Some(Input::Close),
                Wake::Cmd(Some(cmd)) => {
                    frames_in_row = 0;
                    self.on_cmd(cmd).await
                }
                Wake::Timer => {
                    frames_in_row = 0;
                    self.on_timer().await
                }
            };
            if let Some(input) = outcome {
                return input;
            }
        }
    }

    /// 끊긴 채 남아 있던 이어받기를 다시 내보낸다.
    ///
    /// ★이것이 없으면 구멍이 영구히 남는다★ — 끊긴 순간 나가던 재요청은 상대에게 닿지 않았을 수 있고,
    /// 그 답을 기다리는 상태만 남아 다음 조각들이 전부 보류되다 잘림으로 접힌다.
    async fn reissue_resumes(&mut self) -> Option<Input> {
        let outstanding = self.streams.pending_resumes();
        if outstanding.is_empty() {
            return None;
        }
        for (key, after) in outstanding {
            let out = self.wire.resume_request(&key, Some(after));
            let frame = self.wire.encode(&out);
            if let Err(input) = self.write(frame).await {
                return Some(input);
            }
        }
        let now = self.clock.now();
        self.streams.rearm_resumes(now);
        None
    }

    /// 다음에 깨어날 시각. ★모든 타이머 원천이 여기 한 곳에 모인다★ — 새 원천을 더하면서 이 함수를
    /// 안 고치면 그 시한은 아무도 깨우지 않고, `Turn::Timer` 의 진도 확인도 그것을 못 본다.
    fn wake_at(&self) -> Instant {
        let mut wake = (self.last_ping + self.policy.ping_interval)
            .min(self.last_rx + self.policy.idle_timeout);
        if let Some(deadline) = self.pending.next_deadline() {
            wake = wake.min(deadline);
        }
        // ★없으면 아무도 재요청 창을 못 닫는다★ — 잘림 판정이 조각 도착에만 매달려 있으면,
        //   구멍 뒤에 스트림이 조용해진 평범한 경우(턴이 끝났다)에 그 창이 영원히 열려 있다.
        if let Some(deadline) = self.streams.next_resume_deadline() {
            wake = wake.min(deadline);
        }
        wake
    }

    async fn on_timer(&mut self) -> Option<Input> {
        let now = self.clock.now();
        if now.saturating_duration_since(self.last_rx) >= self.policy.idle_timeout {
            return Some(Input::LinkLost(DisconnectCause::Idle));
        }
        let after = self.policy.request_timeout;
        for (tag, slot) in self.pending.expire(now) {
            let _ = slot.send(Err(RequestError::TimedOut { after }));
            self.emit(TransportEvent::RequestTimedOut {
                peer: self.id.clone(),
                tag,
                after,
            });
        }
        for collapsed in self.streams.expire_resumes(now) {
            self.emit(TransportEvent::StreamTruncated {
                peer: self.id.clone(),
                stream: collapsed.key,
                generation: collapsed.generation,
                oldest_valid: collapsed.oldest_valid,
            });
            for msg in collapsed.flushed {
                self.deliver(msg);
            }
        }
        if now.saturating_duration_since(self.last_ping) >= self.policy.ping_interval {
            self.last_ping = now;
            if let Err(input) = self.ping().await {
                return Some(input);
            }
        }
        None
    }

    async fn on_cmd(&mut self, cmd: PeerCmd<W>) -> Option<Input> {
        match cmd {
            PeerCmd::Raw(frame) => self.write(frame).await.err(),
            PeerCmd::Notify { out } => {
                let frame = self.wire.encode(&out);
                self.write(frame).await.err()
            }
            PeerCmd::Resume { key, after } => {
                let out = self.wire.resume_request(&key, after);
                let frame = self.wire.encode(&out);
                self.write(frame).await.err()
            }
            PeerCmd::Request { out, reply } => {
                let Some(tag) = self.wire.request_tag(&out) else {
                    let _ = reply.send(Err(RequestError::NotARequest));
                    return None;
                };
                let deadline = self.clock.now() + self.policy.request_timeout;
                match self.pending.insert(tag, reply, deadline) {
                    Insert::Retired(reply) => {
                        let _ = reply.send(Err(RequestError::TagRetired));
                        return None;
                    }
                    Insert::Collided {
                        displaced,
                        rejected,
                    } => {
                        let _ = displaced.send(Err(RequestError::Superseded));
                        let _ = rejected.send(Err(RequestError::TagRetired));
                        return None;
                    }
                    Insert::Placed => {}
                }
                let frame = self.wire.encode(&out);
                self.write(frame).await.err()
            }
        }
    }

    async fn on_frame(&mut self, frame: Frame) -> Option<Input> {
        let now = self.clock.now();
        if matches!(frame, Frame::Keepalive) {
            self.last_rx = now;
            return None;
        }
        // ★쓰레기는 살아있음의 증거가 아니다★ — 디코드 전에 되감으면 못 읽을 것만 보내는 상대가
        //   침묵 판정에 영원히 안 걸리고, 기본 정책이 프레임만 버리므로 끊는 밸브가 어디에도 없다.
        let msg = match self.wire.decode(frame) {
            Ok(msg) => {
                self.last_rx = now;
                msg
            }
            Err(err) => {
                let reason = err.to_string();
                let generation = self.machine.generation();
                self.emit(TransportEvent::DecodeFailed {
                    peer: self.id.clone(),
                    generation,
                    reason,
                });
                return match self.policy.decode_failure {
                    DecodeFailurePolicy::DropFrame => None,
                    DecodeFailurePolicy::Disconnect => {
                        Some(Input::LinkLost(DisconnectCause::LinkError))
                    }
                };
            }
        };
        self.route(msg).await
    }

    async fn route(&mut self, msg: W::In) -> Option<Input> {
        let reply = self.wire.reply_tag(&msg);
        let mark = self.wire.stream_mark(&msg);
        if reply.is_some() && mark.is_some() {
            self.emit(TransportEvent::AmbiguousRole {
                peer: self.id.clone(),
            });
        }
        // 순서가 계약이다 — 답장 · 스트림 조각 · 그 외.
        if let Some(tag) = reply {
            if let Some(slot) = self.pending.take(&tag) {
                let _ = slot.send(Ok(msg));
                return None;
            }
            // 만료·겹침으로 은퇴한 번호의 늦은 답장은 설계대로 조용히 버린다(ADR-0181 결정 4).
            //   그 밖의 것은 우리가 청한 적 없는 번호라 신고한다.
            if !self.pending.is_retired(&tag) {
                self.emit(TransportEvent::UnmatchedReply {
                    peer: self.id.clone(),
                    tag,
                });
            }
            return None;
        }
        let Some(mark) = mark else {
            self.deliver(msg);
            return None;
        };

        let now = self.clock.now();
        let verdict = self.streams.observe(&mark, msg, now);
        let (event, resume_after, delivered) = match verdict {
            Verdict::Deliver(msg) => (None, None, vec![msg]),
            Verdict::Duplicate | Verdict::Held { .. } => (None, None, Vec::new()),
            Verdict::GenerationChanged {
                from,
                to,
                msg,
                discarded,
            } => (
                Some(TransportEvent::StreamGenerationChanged {
                    peer: self.id.clone(),
                    stream: mark.key.clone(),
                    from,
                    to,
                    discarded: discarded as u64,
                }),
                None,
                vec![msg],
            ),
            Verdict::Gap {
                expected,
                got,
                resume_from,
            } => (
                Some(TransportEvent::StreamGap {
                    peer: self.id.clone(),
                    stream: mark.key.clone(),
                    generation: mark.generation,
                    expected,
                    got,
                    resumed_from: Some(resume_from),
                }),
                Some(resume_from),
                Vec::new(),
            ),
            Verdict::Truncated {
                oldest_valid,
                flushed,
            } => (
                Some(TransportEvent::StreamTruncated {
                    peer: self.id.clone(),
                    stream: mark.key.clone(),
                    generation: mark.generation,
                    oldest_valid,
                }),
                None,
                flushed,
            ),
        };
        if let Some(event) = event {
            self.emit(event);
        }
        for msg in delivered {
            self.deliver(msg);
        }
        if let Some(input) = self.flush_evictions().await {
            return Some(input);
        }
        if let Some(after) = resume_after {
            let out = self.wire.resume_request(&mark.key, Some(after));
            let frame = self.wire.encode(&out);
            return self.write(frame).await.err();
        }
        None
    }

    /// 자리를 만들려고 잊은 스트림을 신고하고, 그것이 붙들고 있던 조각을 내보낸다.
    ///
    /// ★축출이 미결 재요청을 통째로 삼키면 그 구멍은 영영 안 메워진다★ — `pending_resumes` 에서
    /// 사라지므로 재연결이 다시 청하지도 못하고, 잘림 사건도 안 난다.
    async fn flush_evictions(&mut self) -> Option<Input> {
        for evicted in self.streams.drain_evicted() {
            self.emit(TransportEvent::StreamForgotten {
                peer: self.id.clone(),
                stream: evicted.key.clone(),
                generation: evicted.generation,
                last_seq: evicted.last_seq,
            });
            if let Some(oldest_valid) = evicted.oldest_valid {
                self.emit(TransportEvent::StreamTruncated {
                    peer: self.id.clone(),
                    stream: evicted.key,
                    generation: evicted.generation,
                    oldest_valid,
                });
            }
            for msg in evicted.flushed {
                self.deliver(msg);
            }
        }
        None
    }

    // ── 쓰기 ────────────────────────────────────────────────────────────────

    /// ★쓰는 동안에도 제어를 듣는다★ — 안 들으면 `close()` 가 `write_deadline` 만큼 늦게 들린다.
    /// 제어가 이기면 절반 나간 프레임 뒤이므로 통로를 그대로 버린다(machine 이 `DropLink` 를 낸다).
    async fn write(&mut self, frame: Frame) -> Result<(), Input> {
        let clock = self.clock.clone();
        let deadline = self.policy.write_deadline;
        let ctl = &mut self.ctl_rx;
        let Some(tx) = self.tx.as_mut() else {
            return Err(Input::LinkLost(DisconnectCause::LinkError));
        };
        tokio::select! {
            signal = ctl.recv() => Err(match signal {
                Some(Ctl::ReconnectNow) => Input::ReconnectNow,
                Some(Ctl::Close) | None => Input::Close,
            }),
            result = with_deadline(&*clock, deadline, tx.send(frame)) => match result {
                Ok(Ok(())) => Ok(()),
                Ok(Err(_)) => Err(Input::LinkLost(DisconnectCause::LinkError)),
                Err(_) => Err(Input::LinkLost(DisconnectCause::WriteDeadline)),
            },
        }
    }

    async fn ping(&mut self) -> Result<(), Input> {
        let clock = self.clock.clone();
        let deadline = self.policy.write_deadline;
        let ctl = &mut self.ctl_rx;
        let Some(tx) = self.tx.as_mut() else {
            return Err(Input::LinkLost(DisconnectCause::LinkError));
        };
        tokio::select! {
            signal = ctl.recv() => Err(match signal {
                Some(Ctl::ReconnectNow) => Input::ReconnectNow,
                Some(Ctl::Close) | None => Input::Close,
            }),
            result = with_deadline(&*clock, deadline, tx.ping()) => match result {
                Ok(Ok(())) => Ok(()),
                Ok(Err(_)) => Err(Input::LinkLost(DisconnectCause::LinkError)),
                Err(_) => Err(Input::LinkLost(DisconnectCause::WriteDeadline)),
            },
        }
    }

    // ── 정리 ────────────────────────────────────────────────────────────────

    /// 통로를 버린다 — 닫기 코드를 실어 보내려 하고, 읽는 태스크를 접는다.
    ///
    /// ★여기서는 제어를 듣지 않는다 — 닫기를 끝까지 밀어 넣는다★. `select!` 는 `biased` 가 없으면
    /// 폴링마다 시작 팔을 난수로 고르는데(외부 사실), 제어 팔을 걸어 두면 **큐에 신호가 한 칸이라도
    /// 남아 있는 동안** 그 팔이 곧바로 `Ready` 라서 닫기 future 가 한 번도 폴링되지 않은 채 버려진다.
    /// 실측 2026-09-07: `reconnect_now()` 를 두 번 부르면(둘째 신호가 큐에 남는다) 혼잡이 하나도 없는데도
    /// 100회 중 41·54·49·49회 goodbye 가 안 나갔다. ★되돌리지 말 것★ — 그 손실은 문구가 아니라 소비자가
    /// 분기하는 값을 뒤집는다: 닫기 프레임이 나가면 상대는 「말하고 갔다」로 읽고, 안 나가면 tungstenite 가
    /// `ResetWithoutClosingHandshake` 를 올려 「통로 오류」로 읽는다.
    ///
    /// ★제어 팔이 벌던 것은 지연뿐이고 그 지연은 이미 유계였다★ — 걸린 닫기를 제어가 잘라도 닫기 코드는
    /// 어차피 안 나가고, 안 자르면 `write_deadline` 에서 잘린다. 즉 그 팔은 **한 프레임도 더 배달하지
    /// 않으면서** 배달되는 프레임을 절반 버리고 있었다.
    ///
    /// ★혼잡하면 코드는 여전히 잃는다★ — `write_deadline` 이 이 대기를 자른다(`lib.rs` 「닫기 코드가
    /// 상대에게 닿는 자리는 좁다」).
    ///
    /// ★알려진 한계 — 이 대기는 시계에 매인다★. 자기 시계를 손으로 미는 하네스에서 쓰기가 막힌 채
    /// 통로가 버려지면, 시계를 `write_deadline` 만큼 밀거나 멈춤을 풀 때까지 감독이 여기서 선다
    /// (증상: `close()` 를 불렀는데 상대가 접히지 않는다). 운영 시계는 항상 흐르므로 그쪽에서는
    /// `write_deadline` 이 상한이다. 하네스 쪽 계산법의 정본 = `testing::MemoryEndpoint::stall_writes`.
    async fn drop_link(&mut self, code: CloseCode) {
        if let Some(mut tx) = self.tx.take() {
            let clock = self.clock.clone();
            let deadline = self.policy.write_deadline;
            let _ = with_deadline(&*clock, deadline, tx.close(Close::new(code, "closing"))).await;
        }
        self.rx_raw = None;
        self.frames_rx = None;
        self.fresh_link = false;
        if let Some(reader) = self.reader.take() {
            reader.abort();
            // ★취소를 기다린다★ — 읽는 태스크가 통로의 나머지 절반을 쥐고 있어서, 그것이 떨어지기 전에는
            //   소켓이 안 닫힌다(`ws::split_link` 의 그 계약). 안 기다리면 닫힘 시점이 런타임의 회수
            //   시점이 되고, 상대가 붙은 통로를 세면 한동안 둘로 보인다. `read_link` 는 항상 `await`
            //   자리에서 멈춰 있어 이 대기가 한 바퀴를 넘지 않는다.
            let _ = reader.await;
        }
    }

    fn fail_pending(&mut self, cause: DisconnectCause) {
        self.last_cause.store(cause_code(cause), Ordering::Release);
        let generation = self.machine.generation();
        for (_, slot) in self.pending.drain() {
            let _ = slot.send(Err(RequestError::Disconnected { generation, cause }));
        }
        // ★큐에서 잠든 것까지 걷는다★ — 요청은 깨우고, 답장 없는 것은 **센다**(핸들은 이미 `Ok` 를
        //   받아 갔으므로 여기서 안 세면 그대로 조용한 유실이다).
        while let Ok(cmd) = self.data_rx.try_recv() {
            self.reject_command(cmd);
        }
    }

    /// 운영 단계가 아닐 때 도착한 명령 하나를 처리한다.
    fn reject_command(&mut self, cmd: PeerCmd<W>) {
        match cmd {
            PeerCmd::Request { reply, .. } => {
                let generation = self.machine.generation();
                let cause = cause_from_code(self.last_cause.load(Ordering::Acquire));
                let _ = reply.send(Err(RequestError::Disconnected { generation, cause }));
            }
            _ => self.dropped_out += 1,
        }
    }

    async fn teardown(&mut self) {
        self.drop_link(CloseCode::GOING_AWAY).await;
        self.fail_pending(DisconnectCause::Explicit);
        self.data_rx.close();
        // 남은 것도 마저 깨운다 — 닫은 뒤에 들어온 것은 채널이 `Closed` 로 거절한다.
        while let Ok(cmd) = self.data_rx.try_recv() {
            self.reject_command(cmd);
        }
        self.flush_drop_reports();
    }

    // ── 위로 올리기 ──────────────────────────────────────────────────────────

    fn emit_machine(&mut self, event: MachineEvent) {
        let peer = self.id.clone();
        let event = match event {
            MachineEvent::Connected { generation } => {
                TransportEvent::Connected { peer, generation }
            }
            MachineEvent::Disconnected { generation, cause } => {
                self.last_cause.store(cause_code(cause), Ordering::Release);
                TransportEvent::Disconnected {
                    peer,
                    generation,
                    cause,
                }
            }
            MachineEvent::ConnectFailed {
                attempt,
                of,
                cause,
                retry_in,
            } => TransportEvent::ConnectFailed {
                peer,
                attempt,
                of,
                cause,
                retry_in,
            },
            MachineEvent::Rejected { code, reason } => {
                TransportEvent::Rejected { peer, code, reason }
            }
            MachineEvent::GaveUp { reason } => TransportEvent::GaveUp { peer, reason },
        };
        self.emit(event);
    }

    fn emit(&mut self, event: TransportEvent<W>) {
        match self.inbound.try_send(Incoming::Event(event)) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => self.note_inbound_drop(),
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.consumer_gone.store(true, Ordering::Release)
            }
        }
    }

    fn deliver(&mut self, msg: W::In) {
        let arrival = Arrival {
            peer: self.id.clone(),
            generation: self.machine.generation(),
            msg,
        };
        match self.inbound.try_send(Incoming::Message(arrival)) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => self.note_inbound_drop(),
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.consumer_gone.store(true, Ordering::Release)
            }
        }
    }

    /// ★위로 올라가는 줄이 찼다고 **이 연결을 끊지 않는다**★.
    ///
    /// 그 줄은 상대 전부가 함께 쓰는 자원이라, 시끄러운 상대 하나가 채운 것을 조용한 상대가 자기
    /// 잘못으로 갚게 된다 — 그러면 「A 하나 죽어도 B·C 는 잘 동작해야 한다」가 깨진다(ADR-0180 결정 8).
    /// 세어 두었다가 자리가 나면 신고한다.
    fn note_inbound_drop(&mut self) {
        self.dropped_in += 1;
    }

    /// 세어 둔 유실을 자리가 나는 대로 신고한다. ★정책과 무관하게 신고한다★ — 세는 것을 그만두면
    /// `Disconnect` 보다 나쁜 선택이 된다(policy.rs 의 그 문장).
    fn flush_drop_reports(&mut self) {
        for direction in [Direction::Inbound, Direction::Outbound] {
            self.report_drops(direction);
        }
    }

    fn report_drops(&mut self, direction: Direction) {
        let count = match direction {
            Direction::Inbound => self.dropped_in,
            Direction::Outbound => self.dropped_out,
        };
        if count == 0 {
            return;
        }
        let event = TransportEvent::Dropped {
            peer: self.id.clone(),
            direction,
            count,
        };
        match self.inbound.try_send(Incoming::Event(event)) {
            Ok(()) => match direction {
                Direction::Inbound => self.dropped_in = 0,
                Direction::Outbound => self.dropped_out = 0,
            },
            // 자리가 없으면 그대로 들고 있다가 다음 바퀴에 다시 낸다.
            Err(mpsc::error::TrySendError::Full(_)) => {}
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.consumer_gone.store(true, Ordering::Release)
            }
        }
    }

    /// 핸들이 못 넣은 것을 걷어 처리한다.
    ///
    /// ★나가는 큐 포화만 연결을 끊는다★ — 「내 큐가 찼다」는 이 연결의 문제이고, 「공유 줄이 찼다」는
    /// 아니다. 둘을 한 세 칸으로 합치면 남의 홍수가 내 연결을 끊는다.
    #[must_use = "운영 중에는 이 판정이 연결을 끊는다"]
    fn drain_overflow(&mut self) -> Option<Input> {
        let refused = self.refused.swap(0, Ordering::AcqRel);
        if refused > 0 {
            self.dropped_out += refused;
        }
        let full = self.overflow.swap(0, Ordering::AcqRel);
        if full == 0 {
            return None;
        }
        self.dropped_out += full;
        self.report_drops(Direction::Outbound);
        match self.policy.full_queue {
            QueueFullPolicy::Disconnect => Some(Input::LinkLost(DisconnectCause::QueueFull)),
            QueueFullPolicy::DropAndReport => None,
        }
    }
}

/// 통로만 연다. ★시한은 dial 과 핸드셰이크가 **함께** 쓴다★ — 현행 `HANDSHAKE_TIMEOUT` 이 같은 모양이라,
/// dial 이 오래 끌면 핸드셰이크에 남는 시간이 그만큼 줄어든다.
async fn dial_once(
    dialer: Arc<dyn Dialer>,
    addr: Address,
    clock: Arc<dyn Clock>,
    deadline: Instant,
) -> Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), ConnectFailure> {
    let left = deadline.saturating_duration_since(clock.now());
    match with_deadline(&*clock, left, dialer.dial(&addr)).await {
        Ok(Ok(pair)) => Ok(pair),
        Ok(Err(err)) => Err(ConnectFailure::Unreachable(err)),
        Err(_) => Err(ConnectFailure::Unreachable(LinkError::new(
            "connect timed out",
        ))),
    }
}

/// 열린 통로 위에서 핸드셰이크를 끝까지 돌린다.
async fn handshake_once(
    mut tx: Box<dyn LinkTx>,
    mut rx: Box<dyn LinkRx>,
    handshake: Arc<dyn Handshake>,
    clock: Arc<dyn Clock>,
    deadline: Instant,
) -> Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), ConnectFailure> {
    let left = |clock: &dyn Clock| deadline.saturating_duration_since(clock.now());
    let mut step = handshake.start();
    loop {
        match step {
            HandshakeStep::Done => return Ok((tx, rx)),
            HandshakeStep::Reject { last, reason } => {
                let mut link_usable = true;
                if let Some(frame) = last {
                    // 절반 나간 프레임 뒤에 더 쓰지 않는다 — 통로를 그냥 버린다.
                    link_usable = matches!(
                        with_deadline(&*clock, left(&*clock), tx.send(frame)).await,
                        Ok(Ok(()))
                    );
                }
                if link_usable {
                    let close = Close::new(CloseCode::HANDSHAKE_REJECTED, reason.clone());
                    let _ = with_deadline(&*clock, left(&*clock), tx.close(close)).await;
                }
                return Err(ConnectFailure::Rejected {
                    code: Some(CloseCode::HANDSHAKE_REJECTED),
                    reason,
                });
            }
            HandshakeStep::Send(frame) => {
                match with_deadline(&*clock, left(&*clock), tx.send(frame)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(err)) => return Err(ConnectFailure::Unreachable(err)),
                    Err(_) => {
                        return Err(ConnectFailure::Unreachable(LinkError::new(
                            "handshake write timed out",
                        )))
                    }
                }
                step = HandshakeStep::Await;
            }
            HandshakeStep::Await => match with_deadline(&*clock, left(&*clock), rx.recv()).await {
                Ok(Ok(LinkRead::Frame(Frame::Keepalive))) => step = HandshakeStep::Await,
                Ok(Ok(LinkRead::Frame(frame))) => step = handshake.on_frame(&frame),
                // ★상대가 코드를 실어 닫았다 = 거절이다★. 그 판정이 재연결 예산을 가르므로
                //   (ADR-0180 결정 5) 이 코드가 없으면 「거절은 예산을 안 쓴다」가 코드로는 성립하지 않는다.
                Ok(Ok(LinkRead::Closed(Some(close)))) if is_rejection(close.code) => {
                    return Err(ConnectFailure::Rejected {
                        code: Some(close.code),
                        reason: close.reason,
                    })
                }
                // ★맨 종료는 「거절」이 아니다★ — 상대가 그렇게 **말했을** 때만 거절이고, 말 없이
                //   끊긴 것은 「못 붙었다」라 예산을 쓴다(데몬 재시작 중이면 다음 시도에 붙는다).
                Ok(Ok(LinkRead::Closed(_))) => {
                    return Err(ConnectFailure::Unreachable(LinkError::new(
                        "peer closed during handshake",
                    )))
                }
                Ok(Err(err)) => return Err(ConnectFailure::Unreachable(err)),
                Err(_) => {
                    return Err(ConnectFailure::Unreachable(LinkError::new(
                        "handshake timed out",
                    )))
                }
            },
        }
    }
}

/// 통로에서 읽어 감독에게 넘긴다.
///
/// ★따로 태스크인 이유★ — `select!` 팔에 `recv()` 를 직접 걸면 다른 팔이 이길 때마다 그 future 가
/// 취소되고, 취소 안전하지 않은 전송에서는 그 순간 프레임이 사라진다.
async fn read_link(mut rx: Box<dyn LinkRx>, out: mpsc::Sender<LinkMsg>) {
    loop {
        let msg = match rx.recv().await {
            Ok(LinkRead::Frame(frame)) => LinkMsg::Frame(frame),
            Ok(LinkRead::Closed(_)) => {
                let _ = out.send(LinkMsg::Closed).await;
                return;
            }
            Err(_) => {
                let _ = out.send(LinkMsg::Failed).await;
                return;
            }
        };
        if out.send(msg).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::ReconnectPolicy;
    use crate::testing::{
        settle, settle_until, ClientMsg, HelloHandshake, ImmediateHandshake, ListeningHandshake,
        ManualClock, MemoryEndpoint, MemoryNetwork, RejectingHandshake, TestIn, TestOut, TestWire,
    };
    use std::time::Duration;

    struct Harness {
        peer: Peer<TestWire>,
        net: MemoryNetwork,
        clock: ManualClock,
        inbound: mpsc::Receiver<Incoming<TestWire>>,
        /// 울타리를 보려고 미리 꺼내 온 것들. 뒤이은 단언이 이것부터 읽는다.
        buffered: Vec<Incoming<TestWire>>,
    }

    impl Harness {
        fn events(&mut self) -> Vec<TransportEvent<TestWire>> {
            self.drain()
                .into_iter()
                .filter_map(|item| match item {
                    Incoming::Event(event) => Some(event),
                    Incoming::Message(_) => None,
                })
                .collect()
        }

        fn kinds(&mut self) -> Vec<&'static str> {
            self.events().iter().map(|e| e.kind()).collect()
        }

        fn messages(&mut self) -> Vec<TestIn> {
            self.drain()
                .into_iter()
                .filter_map(|item| match item {
                    Incoming::Message(arrival) => Some(arrival.msg),
                    Incoming::Event(_) => None,
                })
                .collect()
        }

        fn drain(&mut self) -> Vec<Incoming<TestWire>> {
            let mut out = std::mem::take(&mut self.buffered);
            while let Ok(item) = self.inbound.try_recv() {
                out.push(item);
            }
            out
        }

        /// 이 문구를 실은 알림이 올라왔나. ★꺼내 온 것을 버리지 않는다★ — 뒤이은 단언이 그것을 본다.
        fn seen_notice(&mut self, body: &str) -> bool {
            let mut hit = false;
            let mut kept = Vec::new();
            while let Ok(item) = self.inbound.try_recv() {
                if let Some(TestIn::Notice(text)) = item.as_message().map(|a| &a.msg) {
                    if text == body {
                        hit = true;
                    }
                }
                kept.push(item);
            }
            self.buffered.extend(kept);
            hit
        }

        fn disconnect_cause(&mut self) -> Option<DisconnectCause> {
            self.events().iter().find_map(|e| match e {
                TransportEvent::Disconnected { cause, .. } => Some(*cause),
                _ => None,
            })
        }
    }

    fn no_jitter(policy: Policy) -> Policy {
        Policy {
            reconnect: ReconnectPolicy {
                jitter: 0.0,
                ..policy.reconnect
            },
            ..policy
        }
    }

    fn build_with(policy: Policy, handshake: Arc<dyn Handshake>, inbound_cap: usize) -> Harness {
        let net = MemoryNetwork::new();
        let clock = ManualClock::new();
        let (inbound_tx, inbound) = mpsc::channel(inbound_cap);
        let peer = spawn_peer(PeerConfig {
            id: PeerId::new("d1"),
            addr: Address::new("mem://d1"),
            wire: Arc::new(TestWire),
            dialer: net.dialer(),
            clock: clock.handle(),
            handshake,
            policy: no_jitter(policy),
            inbound: inbound_tx,
            generations: Arc::new(AtomicU64::new(0)),
        });
        Harness {
            peer,
            net,
            clock,
            inbound,
            buffered: Vec::new(),
        }
    }

    fn build(policy: Policy, handshake: Arc<dyn Handshake>) -> Harness {
        build_with(policy, handshake, 4096)
    }

    async fn live() -> (Harness, Arc<MemoryEndpoint>) {
        let mut h = build(Policy::default(), Arc::new(ImmediateHandshake));
        settle_until("첫 연결", || h.peer.peer_state() == PeerState::Live).await;
        assert_eq!(h.kinds(), vec!["Connected"]);
        let endpoint = h.net.accept().expect("통로가 열렸어야 한다");
        (h, endpoint)
    }

    /// 프레임이 **끊이지 않는** 상태를 만든다.
    ///
    /// ★유한한 다발로는 굶주림을 못 만든다★ — 감독이 그것을 다 비우고 나면 프레임 팔이 `Pending` 이
    /// 되어 뒤의 팔들이 그때 폴링된다. 즉 「1000개를 밀어 넣고 settle」 하는 테스트는 **`biased` 든
    /// 아니든 똑같이 통과한다**(실측 — 그 형태로 쓴 첫 판이 변이를 하나도 못 잡았다). 굶주림은
    /// **채워지는 속도 ≥ 비우는 속도**일 때만 생기므로 공급을 계속 돌린다.
    fn flood(endpoint: &Arc<MemoryEndpoint>) -> tokio::task::JoinHandle<()> {
        let endpoint = endpoint.clone();
        tokio::spawn(async move {
            let mut seq = 1u64;
            loop {
                for _ in 0..256 {
                    endpoint.push(TestWire::chunk(1, 1, seq));
                    seq += 1;
                }
                tokio::task::yield_now().await;
            }
        })
    }

    /// 홍수 아래에서 감독을 몇 바퀴 돌린다. [`settle`] 의 512 바퀴는 여기서 과하다(공급이 그만큼 쌓인다).
    async fn pump(rounds: usize) {
        for _ in 0..rounds {
            tokio::task::yield_now().await;
        }
    }

    /// ★결함이 되살아나면 **매달리는 대신 깨끗이 실패한다**★ — libtest 에는 테스트별 시한이 없어서,
    /// 영구 대기가 되는 회귀는 이 그물이 없으면 CI 를 통째로 멈춘다. 실시간 5초는 **실패 경로에서만**
    /// 흐른다(같은 그물의 선례가 셸 패키지의 `lib_unit` 이다).
    async fn joined(
        task: tokio::task::JoinHandle<Result<TestIn, RequestError>>,
    ) -> Result<TestIn, RequestError> {
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("요청이 안 깨어났다 — 영구 대기 회귀다")
            .expect("요청 태스크가 패닉했다")
    }

    fn spawn_request(
        peer: &Peer<TestWire>,
        tag: u64,
    ) -> tokio::task::JoinHandle<Result<TestIn, RequestError>> {
        let peer = peer.clone();
        tokio::spawn(async move {
            peer.request(TestOut::Request {
                tag,
                body: format!("body{tag}"),
            })
            .await
        })
    }

    // ── C2/F1: 굶주림 방지 규칙 ──
    //
    // ★이 여덟이 C2·F1 의 회귀망이다★ — 아래 홍수 테스트가 아니다. 홍수 쪽은 「부하 아래에서 살아
    //   있나」를 보는 것이고, **굶주림 자체는 이 하네스로 못 만든다**(읽기 태스크가 구조적 병목이라
    //   프레임 큐가 늘 빈다 — `next_turn` 주석의 실측).
    const CAP: usize = 512;

    #[test]
    fn a_due_deadline_beats_a_ready_frame_however_many_are_queued() {
        let t0 = Instant::now();
        for frames_in_row in [0usize, 1, FRAME_BURST, 10_000] {
            for outbound in [0usize, 1, CAP] {
                assert_eq!(
                    next_turn(t0, t0, frames_in_row, outbound, CAP),
                    Turn::Timer,
                    "frames={frames_in_row} outbound={outbound}"
                );
                assert_eq!(
                    next_turn(
                        t0 + Duration::from_secs(1),
                        t0,
                        frames_in_row,
                        outbound,
                        CAP
                    ),
                    Turn::Timer,
                    "이미 지난 시한은 더 급하다"
                );
            }
        }
    }

    #[test]
    fn a_frame_burst_yields_a_turn_to_the_outbound_queue() {
        let t0 = Instant::now();
        let future = t0 + Duration::from_secs(1);
        assert_eq!(
            next_turn(t0, future, FRAME_BURST, 1, CAP),
            Turn::Outbound(1)
        );
        assert_eq!(
            next_turn(t0, future, FRAME_BURST + 1, 3, CAP),
            Turn::Outbound(3)
        );
    }

    #[test]
    fn below_the_burst_and_before_the_deadline_the_loop_just_waits() {
        let t0 = Instant::now();
        let future = t0 + Duration::from_secs(1);
        assert_eq!(next_turn(t0, future, 0, 1, CAP), Turn::Wait);
        assert_eq!(next_turn(t0, future, FRAME_BURST - 1, 1, CAP), Turn::Wait);
    }

    #[test]
    fn an_empty_outbound_queue_never_takes_a_turn() {
        let t0 = Instant::now();
        let future = t0 + Duration::from_secs(1);
        assert_eq!(next_turn(t0, future, FRAME_BURST * 10, 0, CAP), Turn::Wait);
    }

    #[test]
    fn the_deadline_outranks_the_burst_when_both_are_due() {
        let t0 = Instant::now();
        assert_eq!(next_turn(t0, t0, FRAME_BURST, 5, CAP), Turn::Timer);
    }

    #[test]
    fn the_burst_bound_is_finite_so_the_outbound_queue_cannot_be_starved() {
        assert!(FRAME_BURST > 0);
        let t0 = Instant::now();
        let future = t0 + Duration::from_secs(1);
        // ★상한을 두고 찾는다★ — 무한 탐색으로 두면 규칙이 깨졌을 때 매달린다.
        let first_yield = (0..FRAME_BURST * 4)
            .find(|n| matches!(next_turn(t0, future, *n, 1, CAP), Turn::Outbound(_)))
            .expect("연달아 처리하는 프레임 수에 상한이 없으면 나가는 큐가 영원히 굶는다");
        assert_eq!(first_yield, FRAME_BURST);
    }

    // ── F1: 고정 비율만으로는 부족하다 ──
    #[test]
    fn a_backlog_past_half_capacity_does_not_wait_for_a_frame_burst() {
        let t0 = Instant::now();
        let future = t0 + Duration::from_secs(1);
        let half = CAP / 2;
        for outbound in [half, half + 1, CAP] {
            assert_eq!(
                next_turn(t0, future, 0, outbound, CAP),
                Turn::Outbound(outbound),
                "★깊이를 안 보면 비율을 넘는 생산자 앞에서 큐가 계속 자라 QueueFull 로 끝난다★"
            );
        }
        assert_eq!(
            next_turn(t0, future, 0, half - 1, CAP),
            Turn::Wait,
            "절반 아래에서는 평소대로 기다린다"
        );
    }

    #[test]
    fn a_yield_drains_the_whole_backlog_not_one_item() {
        let t0 = Instant::now();
        let future = t0 + Duration::from_secs(1);
        for outbound in [1usize, 7, 64, CAP] {
            assert_eq!(
                next_turn(t0, future, FRAME_BURST, outbound, CAP),
                Turn::Outbound(outbound),
                "한 바퀴에 한 개씩만 빼면 문턱만 32배 높아진 같은 결함이다"
            );
        }
    }

    // ── 연결 수립 ──
    #[tokio::test]
    async fn a_peer_dials_and_reaches_live_on_its_own() {
        let (h, _endpoint) = live().await;
        assert_eq!(h.peer.generation(), Generation(1));
        assert_eq!(h.net.dials(), 1);
    }

    #[tokio::test]
    async fn a_two_step_handshake_round_trips_before_going_live() {
        let h = build(Policy::default(), Arc::new(HelloHandshake));
        settle_until("핸드셰이크 진입", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        let endpoint = h.net.accept().expect("통로");
        assert_eq!(
            endpoint.frames(),
            vec![Frame::Text(HelloHandshake::HELLO.into())]
        );
        endpoint.push(Frame::Text(HelloHandshake::WELCOME.into()));
        settle_until("운영 진입", || h.peer.peer_state() == PeerState::Live).await;
    }

    // ── D1: 세대는 이름표별 공용 계수기에서 나온다 ──
    #[tokio::test]
    async fn the_supervisor_mints_its_generation_from_the_shared_counter() {
        let net = MemoryNetwork::new();
        let clock = ManualClock::new();
        let (inbound_tx, _inbound) = mpsc::channel(64);
        let counter = Arc::new(AtomicU64::new(41));
        let peer = spawn_peer(PeerConfig {
            id: PeerId::new("d1"),
            addr: Address::new("mem://d1"),
            wire: Arc::new(TestWire),
            dialer: net.dialer(),
            clock: clock.handle(),
            handshake: Arc::new(ImmediateHandshake),
            policy: Policy::default(),
            inbound: inbound_tx,
            generations: counter.clone(),
        });
        settle_until("연결", || peer.peer_state() == PeerState::Live).await;
        assert_eq!(peer.generation(), Generation(42));
        assert_eq!(
            counter.load(Ordering::Acquire),
            42,
            "계수기가 함께 올라간다"
        );
    }

    #[tokio::test]
    async fn two_supervisors_sharing_a_counter_never_mint_the_same_generation() {
        let net = MemoryNetwork::new();
        let clock = ManualClock::new();
        let (inbound_tx, _inbound) = mpsc::channel(256);
        let counter = Arc::new(AtomicU64::new(0));
        let spawn = |tag: &str| {
            spawn_peer(PeerConfig {
                id: PeerId::new(tag),
                addr: Address::new("mem://d1"),
                wire: Arc::new(TestWire),
                dialer: net.dialer(),
                clock: clock.handle(),
                handshake: Arc::new(ImmediateHandshake),
                policy: Policy::default(),
                inbound: inbound_tx.clone(),
                generations: counter.clone(),
            })
        };
        // 옛 감독이 아직 살아 있는 채로 새 감독이 붙는 상황(같은 이름표의 승계 경합).
        let first = spawn("d1");
        let second = spawn("d1");
        settle_until("둘 다 연결", || {
            first.peer_state() == PeerState::Live && second.peer_state() == PeerState::Live
        })
        .await;
        assert_ne!(
            first.generation(),
            second.generation(),
            "★같은 번호를 두 감독이 발행하면 소비자가 어느 소켓의 것인지 못 가린다★"
        );
    }

    // ── 요청/응답 ──
    #[tokio::test]
    async fn a_request_is_registered_before_it_is_written() {
        let (h, endpoint) = live().await;
        let task = spawn_request(&h.peer, 5);
        settle().await;
        assert_eq!(endpoint.frames(), vec![Frame::Text("req|5|body5".into())]);
        endpoint.push(TestWire::reply(5));
        settle().await;
        assert_eq!(
            joined(task).await,
            Ok(TestIn::Reply {
                tag: 5,
                body: "ok".into()
            })
        );
    }

    #[tokio::test]
    async fn a_request_that_takes_no_reply_fails_at_once() {
        let (h, _endpoint) = live().await;
        let out = h.peer.request(TestOut::Notice("hi".into())).await;
        assert_eq!(out, Err(RequestError::NotARequest));
    }

    #[tokio::test]
    async fn only_the_silent_request_times_out() {
        let (mut h, endpoint) = live().await;
        let a = spawn_request(&h.peer, 1);
        settle().await;
        let b = spawn_request(&h.peer, 2);
        settle().await;
        endpoint.push(TestWire::reply(2));
        settle().await;
        assert!(joined(b).await.is_ok());

        h.clock.advance(Duration::from_secs(8));
        settle().await;
        assert_eq!(
            joined(a).await,
            Err(RequestError::TimedOut {
                after: Duration::from_secs(8)
            })
        );
        assert!(h.kinds().contains(&"RequestTimedOut"));
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Live,
            "시한 만료는 연결을 건드리지 않는다"
        );
    }

    #[tokio::test]
    async fn a_reply_that_arrives_after_the_timeout_is_dropped_and_the_tag_is_retired() {
        let (mut h, endpoint) = live().await;
        let first = spawn_request(&h.peer, 9);
        settle().await;
        h.clock.advance(Duration::from_secs(8));
        settle().await;
        assert!(matches!(
            joined(first).await,
            Err(RequestError::TimedOut { .. })
        ));
        h.drain();

        endpoint.push(TestWire::reply(9));
        settle().await;
        assert!(h.messages().is_empty(), "늦은 답장은 조용히 버려진다");
        assert!(
            !h.kinds().contains(&"UnmatchedReply"),
            "설계된 늦은 답장은 신고 대상이 아니다"
        );

        let again = h.peer.request(TestOut::Request {
            tag: 9,
            body: "b".into(),
        });
        assert_eq!(again.await, Err(RequestError::TagRetired));
    }

    // ── M1: 겹친 번호는 배달을 못 가르므로 양쪽을 다 실패시킨다 ──
    #[tokio::test]
    async fn a_colliding_tag_wakes_the_older_waiter_and_refuses_the_newer_one() {
        let (h, endpoint) = live().await;
        let old = spawn_request(&h.peer, 3);
        settle().await;
        let new = spawn_request(&h.peer, 3);
        settle().await;
        assert_eq!(joined(old).await, Err(RequestError::Superseded));
        assert_eq!(joined(new).await, Err(RequestError::TagRetired));

        // 먼저 나간 요청의 늦은 답장이 도착해도 아무에게도 배달되지 않는다.
        endpoint.push(TestWire::reply(3));
        settle().await;
    }

    #[tokio::test]
    async fn a_reply_nobody_asked_for_is_reported() {
        let (mut h, endpoint) = live().await;
        h.drain();
        endpoint.push(TestWire::reply(77));
        settle().await;
        assert!(h.kinds().contains(&"UnmatchedReply"));
    }

    // ── C1: 연결을 버리는 어느 갈래도 대기자를 매달아 두지 않는다 ──
    #[tokio::test]
    async fn a_pending_request_is_woken_when_the_shell_asks_to_reconnect() {
        let (h, _endpoint) = live().await;
        let pending = spawn_request(&h.peer, 11);
        settle().await;
        h.peer.reconnect_now();
        settle().await;
        assert!(
            matches!(
                joined(pending).await,
                Err(RequestError::Disconnected { .. })
            ),
            "reconnect_now 만 FailPending 을 안 내면 이 슬롯은 영구 대기가 된다"
        );
    }

    #[tokio::test]
    async fn a_pending_request_does_not_hang_when_reconnect_then_exhausts_the_budget() {
        let (h, _endpoint) = live().await;
        let pending = spawn_request(&h.peer, 12);
        settle().await;

        h.net.fail_forever();
        h.peer.reconnect_now();
        settle().await;
        for delay in [500u64, 1000, 2000] {
            h.clock.advance(Duration::from_millis(delay));
            settle().await;
        }
        assert_eq!(
            h.peer.peer_state(),
            PeerState::GaveUp(crate::machine::GaveUpReason::BudgetExhausted)
        );
        assert!(matches!(
            joined(pending).await,
            Err(RequestError::Disconnected { .. })
        ));
    }

    #[tokio::test]
    async fn a_request_that_lands_after_the_link_is_gone_is_refused_not_parked() {
        let (h, endpoint) = live().await;
        // 큐에는 들어가지만 감독은 이미 운영 단계를 떠난 뒤인 경합을 흉내낸다.
        let late = {
            let peer = h.peer.clone();
            tokio::spawn(async move {
                peer.request(TestOut::Request {
                    tag: 21,
                    body: "x".into(),
                })
                .await
            })
        };
        endpoint.close();
        settle().await;
        let outcome = joined(late).await;
        assert!(
            matches!(outcome, Err(RequestError::Disconnected { .. })),
            "{outcome:?}"
        );
    }

    #[tokio::test]
    async fn losing_the_link_wakes_every_waiter() {
        let (mut h, endpoint) = live().await;
        let pending = spawn_request(&h.peer, 1);
        settle().await;
        endpoint.close();
        settle().await;
        assert!(matches!(
            joined(pending).await,
            Err(RequestError::Disconnected { .. })
        ));
        assert!(h.kinds().contains(&"Disconnected"));
    }

    // ── keepalive · 시한 ──
    #[tokio::test]
    async fn silence_past_the_idle_timeout_cuts_the_link() {
        let (mut h, _endpoint) = live().await;
        h.clock.advance(Duration::from_secs(50));
        settle().await;
        assert_eq!(h.disconnect_cause(), Some(DisconnectCause::Idle));
    }

    #[tokio::test]
    async fn keepalive_goes_out_on_its_own_schedule() {
        let (h, endpoint) = live().await;
        h.clock.advance(Duration::from_secs(20));
        settle().await;
        assert!(endpoint.drain().contains(&ClientMsg::Ping));
        endpoint.push(Frame::Keepalive);
        settle().await;
        h.clock.advance(Duration::from_secs(20));
        settle().await;
        assert!(endpoint.drain().contains(&ClientMsg::Ping));
        assert_eq!(h.peer.peer_state(), PeerState::Live);
    }

    #[tokio::test]
    async fn an_inbound_keepalive_keeps_the_idle_timer_from_firing() {
        let (h, endpoint) = live().await;
        for _ in 0..3 {
            h.clock.advance(Duration::from_secs(40));
            endpoint.push(Frame::Keepalive);
            settle().await;
        }
        assert_eq!(h.peer.peer_state(), PeerState::Live);
    }

    // ── C2: 프레임 홍수가 타이머·나가는 큐를 굶기지 않는다 ──
    // ── 부하 아래 생존 (★C2 회귀망이 아니다 — 위 `next_turn` 다섯이 그것이다★) ──
    #[tokio::test]
    async fn keepalive_still_goes_out_under_load() {
        let (h, endpoint) = live().await;
        let feeder = flood(&endpoint);
        pump(8).await;
        h.clock.advance(Duration::from_secs(20));
        pump(64).await;
        feeder.abort();
        assert!(
            endpoint.drain().contains(&ClientMsg::Ping),
            "★출력이 가장 많을 때 ping 이 멎으면 상대가 우리를 침묵으로 끊는다★"
        );
        assert_eq!(h.peer.peer_state(), PeerState::Live);
    }

    #[tokio::test]
    async fn the_outbound_queue_still_drains_under_load() {
        let (h, endpoint) = live().await;
        let feeder = flood(&endpoint);
        pump(8).await;
        h.peer.notify(TestOut::Notice("let-me-out".into())).unwrap();
        pump(64).await;
        feeder.abort();
        assert!(
            endpoint
                .frames()
                .contains(&Frame::Text("not|let-me-out".into())),
            "★프레임 팔이 계속 준비돼 있어도 나가는 큐가 굶으면 안 된다★"
        );
    }

    #[tokio::test]
    async fn the_request_deadline_still_fires_under_load() {
        let (h, endpoint) = live().await;
        let pending = spawn_request(&h.peer, 31);
        settle().await;
        let feeder = flood(&endpoint);
        pump(8).await;
        h.clock.advance(Duration::from_secs(8));
        pump(64).await;
        feeder.abort();
        assert!(
            matches!(joined(pending).await, Err(RequestError::TimedOut { .. })),
            "★시한이 안 돌면 요청이 상한을 넘겨 매달린다★"
        );
    }

    // ── 스트림 ──
    #[tokio::test]
    async fn a_stream_gap_is_reported_and_resumed_from_the_last_good_seq() {
        let (mut h, endpoint) = live().await;
        endpoint.push(TestWire::chunk(1, 4, 1));
        settle().await;
        assert_eq!(h.messages().len(), 1);

        endpoint.push(TestWire::chunk(1, 4, 5));
        settle().await;
        let gap = h.events().iter().find_map(|e| match e {
            TransportEvent::StreamGap {
                expected,
                got,
                resumed_from,
                stream,
                ..
            } => Some((*stream, *expected, *got, *resumed_from)),
            _ => None,
        });
        assert_eq!(gap, Some((1, 2, 5, Some(1))));
        assert_eq!(
            endpoint.frames(),
            vec![Frame::Text("res|1|1".into())],
            "구멍을 만나면 그 지점부터 다시 청한다"
        );
    }

    // ── C3: 재요청의 답을 잘림으로 오독하지 않는다 ──
    #[tokio::test]
    async fn chunks_in_flight_before_the_resume_do_not_look_like_truncation() {
        let (mut h, endpoint) = live().await;
        endpoint.push(TestWire::chunk(1, 4, 1));
        endpoint.push(TestWire::chunk(1, 4, 5));
        settle().await;
        h.drain();

        // 상대가 재요청을 받기 전에 이미 띄운 조각. ★그 뒤에 울타리를 하나 세운다★ — 「없다」를
        //   `settle()` 뒤에 그냥 단언하면 아직 처리가 안 됐을 때도 통과해, 결함을 축복하는 테스트가 된다.
        endpoint.push(TestWire::chunk(1, 4, 6));
        endpoint.push(TestWire::encode_in(&TestIn::Notice("fence".into())));
        settle_until("6번 조각과 울타리가 처리될 때까지", || {
            h.seen_notice("fence")
        })
        .await;
        assert!(
            !h.kinds().contains(&"StreamTruncated"),
            "★상대가 못 준다★ 가 아니라 ★아직 안 왔다★ 다"
        );

        // 상대의 재생분이 순서대로 올라온다.
        for seq in 2..=6 {
            endpoint.push(TestWire::chunk(1, 4, seq));
        }
        settle().await;
        let seqs: Vec<u64> = h
            .messages()
            .into_iter()
            .filter_map(|m| match m {
                TestIn::Chunk { seq, .. } => Some(seq),
                _ => None,
            })
            .collect();
        assert_eq!(seqs, vec![2, 3, 4, 5, 6], "잃은 구간이 실제로 돌아온다");
    }

    #[tokio::test]
    async fn a_resume_window_closes_without_any_further_arrival() {
        let (mut h, endpoint) = live().await;
        endpoint.push(TestWire::chunk(1, 4, 1));
        endpoint.push(TestWire::chunk(1, 4, 5));
        settle().await;
        h.drain();

        // ★조각이 더 안 와도 창이 닫혀야 한다★ — 턴이 끝나 스트림이 조용해지는 것이 평범한 경우이고,
        //   도착에만 매달린 판정은 그때 영원히 열려 있다.
        h.clock.advance(Duration::from_secs(2));
        settle().await;
        let items = h.drain();
        let oldest = items.iter().find_map(|i| match i.as_event() {
            Some(TransportEvent::StreamTruncated { oldest_valid, .. }) => Some(*oldest_valid),
            _ => None,
        });
        assert_eq!(oldest, Some(5), "★붙들고 있던 첫 번호가 하한이다★");
        let seqs: Vec<u64> = items
            .iter()
            .filter_map(|i| match i.as_message().map(|a| &a.msg) {
                Some(TestIn::Chunk { seq, .. }) => Some(*seq),
                _ => None,
            })
            .collect();
        assert_eq!(
            seqs,
            vec![5],
            "★신고한 하한부터 실제로 배달해야 그 값이 참이다★"
        );
    }

    #[tokio::test]
    async fn an_outstanding_resume_is_sent_again_after_a_reconnect() {
        let (mut h, endpoint) = live().await;
        endpoint.push(TestWire::chunk(1, 4, 1));
        endpoint.push(TestWire::chunk(1, 4, 5));
        settle().await;
        assert_eq!(endpoint.frames(), vec![Frame::Text("res|1|1".into())]);
        h.drain();

        endpoint.close();
        settle().await;
        h.clock.advance(Duration::from_millis(500));
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;

        let reconnected = h.net.accept().expect("새 통로");
        assert_eq!(
            reconnected.frames(),
            vec![Frame::Text("res|1|1".into())],
            "★재연결 뒤 다시 청하지 않으면 그 구멍은 영영 안 메워진다★"
        );
    }

    // ── B3: 축출이 미결 재요청을 삼키지 않는다 ──
    #[tokio::test]
    async fn forgetting_a_stream_is_reported_and_gives_back_what_it_held() {
        let mut h = build(
            Policy {
                max_streams: 1,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
        );
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();

        endpoint.push(TestWire::chunk(1, 4, 1));
        endpoint.push(TestWire::chunk(1, 4, 5));
        settle().await;
        h.drain();

        // 자리가 하나뿐이라 새 스트림이 옛 스트림을 밀어낸다.
        endpoint.push(TestWire::chunk(2, 4, 1));
        settle().await;
        let items = h.drain();
        let kinds: Vec<&str> = items
            .iter()
            .filter_map(|i| i.as_event().map(|e| e.kind()))
            .collect();
        assert!(
            kinds.contains(&"StreamForgotten"),
            "★무엇을 잊었는지 말하지 않으면 그 스트림의 장부가 조용히 처음부터 다시 시작된다★"
        );
        assert!(
            kinds.contains(&"StreamTruncated"),
            "붙들고 있던 조각이 축출과 함께 사라지면 안 된다"
        );
        let seqs: Vec<u64> = items
            .iter()
            .filter_map(|i| match i.as_message().map(|a| &a.msg) {
                Some(TestIn::Chunk { key: 1, seq, .. }) => Some(*seq),
                _ => None,
            })
            .collect();
        assert_eq!(seqs, vec![5], "붙들고 있던 5 를 돌려받는다");
    }

    #[tokio::test]
    async fn a_duplicate_chunk_never_reaches_the_consumer() {
        let (mut h, endpoint) = live().await;
        endpoint.push(TestWire::chunk(1, 4, 1));
        endpoint.push(TestWire::chunk(1, 4, 2));
        endpoint.push(TestWire::chunk(1, 4, 2));
        endpoint.push(TestWire::chunk(1, 4, 1));
        settle().await;
        assert_eq!(h.messages().len(), 2);
    }

    #[tokio::test]
    async fn a_generation_change_is_announced_and_restarts_the_ledger() {
        let (mut h, endpoint) = live().await;
        endpoint.push(TestWire::chunk(1, 4, 100));
        settle().await;
        h.drain();
        endpoint.push(TestWire::chunk(1, 5, 1));
        settle().await;
        assert!(h.kinds().contains(&"StreamGenerationChanged"));
    }

    #[tokio::test]
    async fn a_message_claiming_both_roles_is_reported_and_treated_as_a_reply() {
        let (mut h, endpoint) = live().await;
        let waiting = spawn_request(&h.peer, 8);
        settle().await;
        endpoint.push(TestWire::encode_in(&TestIn::Ambiguous {
            tag: 8,
            key: 1,
            generation: 1,
            seq: 1,
        }));
        settle().await;
        assert!(h.kinds().contains(&"AmbiguousRole"));
        assert!(joined(waiting).await.is_ok(), "답장으로 먼저 처리한다");
    }

    #[tokio::test]
    async fn resume_stream_writes_the_consumers_own_envelope() {
        let (h, endpoint) = live().await;
        let generation = h.peer.resume_stream(7, Some(42));
        settle().await;
        assert_eq!(generation, Generation(1));
        assert_eq!(endpoint.frames(), vec![Frame::Text("res|7|42".into())]);
    }

    // ── M2: 운영 단계가 아닐 때의 resume 은 조용히 사라지지 않는다 ──
    #[tokio::test]
    async fn a_resume_asked_while_down_is_counted_and_reported() {
        let (mut h, endpoint) = live().await;
        endpoint.close();
        settle().await;
        assert_ne!(h.peer.peer_state(), PeerState::Live);
        h.drain();

        h.peer.resume_stream(7, Some(1));
        h.clock.advance(Duration::from_millis(500));
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;

        let dropped = h.events().iter().find_map(|e| match e {
            TransportEvent::Dropped {
                direction, count, ..
            } => Some((*direction, *count)),
            _ => None,
        });
        assert!(matches!(dropped, Some((Direction::Outbound, n)) if n >= 1));
    }

    // ── 디코드 ──
    #[tokio::test]
    async fn a_frame_the_wire_cannot_read_is_reported_without_cutting_the_link() {
        let (mut h, endpoint) = live().await;
        endpoint.push(Frame::Text("garbage".into()));
        settle().await;
        assert!(h.kinds().contains(&"DecodeFailed"));
        assert_eq!(h.peer.peer_state(), PeerState::Live);
    }

    #[tokio::test]
    async fn the_disconnect_on_decode_failure_policy_cuts_the_link() {
        let mut h = build(
            Policy {
                decode_failure: DecodeFailurePolicy::Disconnect,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
        );
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        h.drain();
        endpoint.push(Frame::Text("garbage".into()));
        settle().await;
        assert!(h.kinds().contains(&"Disconnected"));
    }

    // ── 재연결 예산 ──
    #[tokio::test]
    async fn being_unreachable_burns_the_budget_and_then_gives_up() {
        let mut h = build(Policy::default(), Arc::new(ImmediateHandshake));
        h.net.fail_forever();
        settle_until("첫 실패", || h.peer.peer_state() == PeerState::Backoff).await;

        for delay in [500u64, 1000, 2000] {
            h.clock.advance(Duration::from_millis(delay));
            settle().await;
        }
        assert_eq!(
            h.peer.peer_state(),
            PeerState::GaveUp(crate::machine::GaveUpReason::BudgetExhausted)
        );
        assert_eq!(h.net.dials(), 4, "첫 시도 + 재시도 3회");
        let kinds = h.kinds();
        assert_eq!(kinds.iter().filter(|k| **k == "ConnectFailed").count(), 4);
        assert!(kinds.contains(&"GaveUp"));
    }

    #[tokio::test]
    async fn a_rejection_spends_no_budget_and_dials_only_once() {
        let mut h = build(Policy::default(), RejectingHandshake::new("version 3 != 4"));
        settle_until("거절", || h.peer.peer_state().is_terminal()).await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::GaveUp(crate::machine::GaveUpReason::Rejected)
        );
        assert_eq!(h.net.dials(), 1, "붙었는데 거절 = 재시도하지 않는다");
        let events = h.events();
        let reason = events.iter().find_map(|e| match e {
            TransportEvent::Rejected { reason, .. } => Some(reason.clone()),
            _ => None,
        });
        assert_eq!(reason.as_deref(), Some("version 3 != 4"));
        assert!(events.iter().any(|e| e.kind() == "GaveUp"));
        assert!(
            !events.iter().any(|e| e.kind() == "ConnectFailed"),
            "예산을 쓴 적이 없으므로 시도 실패 사건도 없다"
        );
    }

    // ── N2: 상대가 코드를 실어 닫으면 그것이 거절이다 ──
    #[tokio::test]
    async fn a_peer_that_closes_with_a_code_is_a_rejection_not_a_retry() {
        let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
        settle_until("핸드셰이크 대기", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        let endpoint = h.net.accept().unwrap();
        endpoint.reject(Close::new(CloseCode::HANDSHAKE_REJECTED, "protocol 3 != 4"));
        settle_until("거절 판정", || h.peer.peer_state().is_terminal()).await;

        assert_eq!(
            h.peer.peer_state(),
            PeerState::GaveUp(crate::machine::GaveUpReason::Rejected),
            "★코드를 읽지 못하면 여기가 BudgetExhausted 가 된다★"
        );
        assert_eq!(h.net.dials(), 1, "거절은 예산을 안 쓴다");
        let reported = h.events().into_iter().find_map(|e| match e {
            TransportEvent::Rejected { code, reason, .. } => Some((code, reason)),
            _ => None,
        });
        assert_eq!(
            reported,
            Some((
                Some(CloseCode::HANDSHAKE_REJECTED),
                "protocol 3 != 4".into()
            ))
        );
    }

    #[tokio::test]
    async fn a_peer_that_closes_without_a_code_is_treated_as_unreachable() {
        let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
        settle_until("핸드셰이크 대기", || {
            h.peer.peer_state() == PeerState::Handshaking
        })
        .await;
        h.net.accept().unwrap().close();
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        assert!(h.kinds().contains(&"ConnectFailed"), "예산을 쓰는 쪽이다");
    }

    #[tokio::test]
    async fn reconnect_now_refills_the_budget_and_dials_again() {
        let mut h = build(Policy::default(), Arc::new(ImmediateHandshake));
        h.net.fail_forever();
        settle().await;
        for delay in [500u64, 1000, 2000] {
            h.clock.advance(Duration::from_millis(delay));
            settle().await;
        }
        assert!(h.peer.peer_state().is_terminal());
        h.drain();

        h.net.allow();
        h.peer.reconnect_now();
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;
        assert!(h.kinds().contains(&"Connected"));
    }

    #[tokio::test]
    async fn a_burst_of_reconnect_requests_still_lets_a_dial_happen() {
        let (h, _endpoint) = live().await;
        for _ in 0..50 {
            h.peer.reconnect_now();
        }
        settle_until(
            "쌓인 신호를 접지 않으면 dial 이 폴링되기도 전에 매번 취소된다",
            || h.net.dials() >= 2,
        )
        .await;
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;
    }

    #[tokio::test]
    async fn a_lost_link_comes_back_on_the_first_backoff_step() {
        let (h, endpoint) = live().await;
        endpoint.close();
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        h.clock.advance(Duration::from_millis(500));
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;
        assert_eq!(h.peer.generation(), Generation(2), "세대가 하나 올랐다");
    }

    // ── N8: 붙자마자 끊기는 상대도 결국 포기에 닿는다(팝업 자격) ──
    #[tokio::test]
    async fn a_flapping_peer_reaches_gave_up_so_the_popup_can_fire() {
        let h = build(Policy::default(), Arc::new(ImmediateHandshake));
        let mut backoffs = [500u64, 1000, 2000].into_iter();
        for _ in 0..4 {
            settle().await;
            if h.peer.peer_state().is_terminal() {
                break;
            }
            if let Some(endpoint) = h.net.accept() {
                // 붙자마자 끊는다 — dwell 을 한 번도 못 채운다.
                endpoint.close();
            }
            settle().await;
            if let Some(delay) = backoffs.next() {
                h.clock.advance(Duration::from_millis(delay));
            }
        }
        settle().await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::GaveUp(crate::machine::GaveUpReason::BudgetExhausted),
            "★여기 못 닿으면 ADR-0180 결정 7 의 팝업이 영영 안 뜬다★"
        );
    }

    #[tokio::test]
    async fn a_connection_that_holds_long_enough_gets_its_budget_back() {
        let (h, endpoint) = live().await;
        h.clock.advance(Policy::default().live_dwell);
        settle().await;
        endpoint.push(Frame::Keepalive);
        settle().await;
        endpoint.close();
        settle_until("백오프", || h.peer.peer_state() == PeerState::Backoff).await;
        h.clock.advance(Duration::from_millis(500));
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;
    }

    // ── 큐 ──
    #[tokio::test]
    async fn requests_made_while_down_fail_without_waiting() {
        let (h, endpoint) = live().await;
        endpoint.close();
        settle().await;
        let out = h
            .peer
            .request(TestOut::Request {
                tag: 1,
                body: "x".into(),
            })
            .await;
        assert!(matches!(out, Err(RequestError::Disconnected { .. })));
        assert_eq!(
            h.peer.notify(TestOut::Notice("x".into())),
            Err(SendError::Disconnected)
        );
    }

    #[tokio::test]
    async fn a_full_outbound_queue_cuts_the_link_under_the_default_policy() {
        let mut h = build(
            Policy {
                outbound_queue: 1,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
        );
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        h.drain();
        endpoint.stall_writes(true);

        let mut refused = 0;
        for n in 0..8 {
            if h.peer.notify(TestOut::Notice(format!("n{n}"))).is_err() {
                refused += 1;
            }
            settle().await;
        }
        assert!(refused > 0, "유계 큐가 실제로 찼어야 한다");
        assert!(
            endpoint.frames().is_empty(),
            "막힌 동안에는 한 프레임도 안 나간다"
        );

        endpoint.stall_writes(false);
        settle().await;
        assert_eq!(h.disconnect_cause(), Some(DisconnectCause::QueueFull));
    }

    #[tokio::test]
    async fn the_drop_and_report_policy_keeps_the_link_and_counts_the_loss() {
        let mut h = build(
            Policy {
                outbound_queue: 1,
                full_queue: QueueFullPolicy::DropAndReport,
                ..Policy::default()
            },
            Arc::new(ImmediateHandshake),
        );
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        h.drain();
        endpoint.stall_writes(true);
        for n in 0..8 {
            let _ = h.peer.notify(TestOut::Notice(format!("n{n}")));
            settle().await;
        }
        assert!(endpoint.frames().is_empty());
        endpoint.stall_writes(false);
        settle().await;
        let dropped = h.events().iter().find_map(|e| match e {
            TransportEvent::Dropped {
                direction, count, ..
            } => Some((*direction, *count)),
            _ => None,
        });
        assert!(matches!(dropped, Some((Direction::Outbound, n)) if n > 0));
        assert_eq!(h.peer.peer_state(), PeerState::Live);
    }

    // ── N6a: 큐에서 걷어 버린 것도 센다 ──
    #[tokio::test]
    async fn commands_discarded_from_the_queue_are_reported_not_dropped_silently() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        h.peer.notify(TestOut::Notice("first".into())).unwrap();
        settle().await;
        assert!(endpoint.frames().is_empty(), "감독이 첫 쓰기에서 멈췄다");

        // 이것들은 큐에 남은 채로 버려지는 자리다 — 핸들은 이미 `Ok` 를 받아 갔다.
        for n in 0..3 {
            h.peer.notify(TestOut::Notice(format!("n{n}"))).unwrap();
        }
        h.drain();

        h.clock.advance(Duration::from_secs(5));
        settle().await;
        // ★막힌 통로에서는 `drop_link` 의 닫기도 함께 걸린다★ — 그것을 풀지 않으면 백오프 대기가
        //   시작되지 않아 아래 재연결이 오지 않는다(`stall_writes` rustdoc 의 그 계산).
        endpoint.stall_writes(false);
        settle().await;
        h.clock.advance(Duration::from_millis(500));
        settle_until("재연결", || h.peer.peer_state() == PeerState::Live).await;

        let dropped = h.events().iter().find_map(|e| match e {
            TransportEvent::Dropped {
                direction, count, ..
            } => Some((*direction, *count)),
            _ => None,
        });
        assert!(
            matches!(dropped, Some((Direction::Outbound, n)) if n >= 3),
            "핸들이 이미 Ok 를 받아 갔으므로 여기서 안 세면 조용한 유실이다: {dropped:?}"
        );
    }

    // ── N4: 위로 올릴 데가 닫히면 다시 붙지 않고 접는다 ──
    #[tokio::test]
    async fn a_closed_consumer_ends_the_peer_instead_of_triggering_reconnects() {
        let mut h = build(Policy::default(), Arc::new(ImmediateHandshake));
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        let endpoint = h.net.accept().unwrap();
        let dials_before = h.net.dials();

        // 소비자가 사라진다.
        h.inbound.close();
        while h.inbound.try_recv().is_ok() {}
        drop(std::mem::replace(&mut h.inbound, mpsc::channel(1).1));

        endpoint.push(TestWire::encode_in(&TestIn::Notice("x".into())));
        settle_until("종료", || h.peer.peer_state() == PeerState::Closed).await;
        assert_eq!(
            h.net.dials(),
            dials_before,
            "소비자 부재는 재연결할 일이 아니다"
        );
    }

    // ── F6: 곱게 끄는 것은 거절이 아니다 ──
    #[tokio::test]
    async fn a_peer_that_shuts_down_gracefully_is_not_a_rejection() {
        for code in [CloseCode(1000), CloseCode(1001), CloseCode::GOING_AWAY] {
            let mut h = build(Policy::default(), Arc::new(ListeningHandshake));
            settle_until("핸드셰이크 대기", || {
                h.peer.peer_state() == PeerState::Handshaking
            })
            .await;
            h.net
                .accept()
                .unwrap()
                .reject(Close::new(code, "shutting down"));
            settle_until("판정", || h.peer.peer_state() != PeerState::Handshaking).await;
            assert_eq!(
                h.peer.peer_state(),
                PeerState::Backoff,
                "★{code:?} 는 「나 이제 간다」이지 「너를 안 받는다」가 아니다 —                  거절로 읽으면 데몬이 곱게 종료할 때마다 재시도 없이 포기한다★"
            );
            assert!(h.kinds().contains(&"ConnectFailed"), "예산을 쓰는 쪽이다");
        }
    }

    // ── F5: 쓰레기는 살아있음의 증거가 아니다 ──
    #[tokio::test]
    async fn a_peer_sending_only_garbage_is_eventually_cut_by_the_idle_timer() {
        let (mut h, endpoint) = live().await;
        h.clock.advance(Duration::from_secs(25));
        for n in 0..4 {
            endpoint.push(Frame::Text(format!("garbage{n}")));
        }
        settle().await;
        assert!(h.kinds().contains(&"DecodeFailed"));

        // 연결 시점부터 idle_timeout(50s)이 지났다. 쓰레기가 시계를 되감았다면 안 끊긴다.
        h.clock.advance(Duration::from_secs(26));
        settle().await;
        assert_eq!(
            h.disconnect_cause(),
            Some(DisconnectCause::Idle),
            "★못 읽을 것만 보내는 상대를 끊는 밸브가 이것뿐이다★"
        );
    }

    // ── F4: 내려가 있는 동안에도 유실을 신고한다 ──
    #[tokio::test]
    async fn losses_are_reported_while_the_peer_is_still_down() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        h.peer.notify(TestOut::Notice("first".into())).unwrap();
        settle().await;
        for n in 0..3 {
            h.peer.notify(TestOut::Notice(format!("n{n}"))).unwrap();
        }
        h.drain();

        h.clock.advance(Duration::from_secs(5));
        settle().await;
        // ★막힌 통로에서는 `drop_link` 의 닫기도 함께 걸린다★ — 유실 신고는 그 다음 단계(백오프 대기)에서
        //   나오므로, 풀지 않으면 아래 신고가 아직 안 나온 채로 단언을 만난다. 시계로 자르지 않는 이유는
        //   그러면 백오프까지 지나가 「아직 안 붙었다」가 깨지기 때문이다.
        endpoint.stall_writes(false);
        settle().await;
        assert_eq!(
            h.peer.peer_state(),
            PeerState::Backoff,
            "아직 다시 붙지 않은 상태여야 이 테스트가 뜻을 갖는다"
        );
        let dropped = h.events().iter().find_map(|e| match e {
            TransportEvent::Dropped {
                direction, count, ..
            } => Some((*direction, *count)),
            _ => None,
        });
        assert!(
            matches!(dropped, Some((Direction::Outbound, n)) if n >= 3),
            "★다시 붙을 때까지 기다리면 사용자가 「다시 연결」을 누를 때까지 아무 데도 안 나온다★: {dropped:?}"
        );
    }

    // ── 종료 ──
    #[tokio::test]
    async fn closing_is_final() {
        let h = build(Policy::default(), Arc::new(ImmediateHandshake));
        settle_until("연결", || h.peer.peer_state() == PeerState::Live).await;
        h.peer.close();
        settle_until("종료", || h.peer.peer_state() == PeerState::Closed).await;
        h.peer.reconnect_now();
        settle().await;
        assert_eq!(h.peer.peer_state(), PeerState::Closed);
        assert_eq!(h.net.dials(), 1);
    }

    #[tokio::test]
    async fn a_write_that_never_completes_is_cut_by_the_write_deadline() {
        let (mut h, endpoint) = live().await;
        endpoint.stall_writes(true);
        let _ = h.peer.notify(TestOut::Notice("stuck".into()));
        settle().await;
        assert!(endpoint.frames().is_empty(), "실제로 막혀 있어야 한다");
        h.clock.advance(Duration::from_secs(5));
        settle().await;
        assert_eq!(h.disconnect_cause(), Some(DisconnectCause::WriteDeadline));
    }

    // ── M4: 쓰기 도중에도 제어를 듣는다 ──
    //
    // ★발행된 상태가 `Closed` 인 것으로는 감독이 접혔다는 뜻이 안 된다★ — `run` 이 행동을 **적용하기
    //   전에** 상태를 발행하므로(그 루프의 두 `publish` 사이), 아래 `settle_until` 은 정리가 아직 걸린
    //   닫기 위에 멈춰 있는 동안에도 통과한다. 그래서 멈춤을 풀고 **그 뒤까지** 잰다.
    #[tokio::test]
    async fn close_is_heard_while_a_write_is_parked() {
        let (h, endpoint) = live().await;
        endpoint.stall_writes(true);
        let _ = h.peer.notify(TestOut::Notice("stuck".into()));
        settle().await;
        h.peer.close();
        settle_until("종료", || h.peer.peer_state() == PeerState::Closed).await;

        endpoint.stall_writes(false);
        let mut seen = Vec::new();
        settle_until("걸려 있던 goodbye 가 나갔다", || {
            seen.extend(endpoint.drain());
            seen.iter()
                .any(|m| matches!(m, ClientMsg::Close(c) if c.code == CloseCode::GOING_AWAY))
        })
        .await;
        // 걸린 채 취소된 쓰기는 통로에 아무것도 남기지 않는다 — 절반 나간 프레임 뒤에 더 쓰지 않는다는
        //   [`LinkTx::send`] 계약의 관측 가능한 면이다.
        assert!(
            !seen.iter().any(|m| matches!(m, ClientMsg::Frame(_))),
            "취소된 쓰기가 뒤늦게 나갔다: {seen:?}"
        );
    }

    // ── 종료: goodbye 코드가 **실제로** 통로로 나간다 ──
    //
    // ★마지막 핸들까지 놓는 모양인 것은 의도★ — `Registry::remove` 가 그 모양이고(닫으라 한 뒤 핸들을
    //   버린다), 그 상태에서 제어 채널은 곧바로 `Ready(None)` 을 낸다. `drop_link` 에 제어 팔이 있던
    //   판에서는 `select!` 의 난수 팔 선택이 그 `None` 을 승자로 쳐 닫기 future 가 폴링되기도 전에
    //   버려졌다 — 실측으로 약 절반이었다(2026-09-07).
    // ★한 바퀴로 재지 말 것★ — 그래서 이 그물은 한 번에 절반만 잡는다. 아래 반복이 그것을 결정적으로
    //   만든다.
    #[tokio::test]
    async fn closing_speaks_the_goodbye_code_on_the_wire() {
        for round in 0..24 {
            let (h, endpoint) = live().await;
            h.peer.close();
            drop(h);
            let what = format!("{round}번째 바퀴: goodbye 가 통로로 나갔다");
            let mut seen = Vec::new();
            settle_until(&what, || {
                seen.extend(endpoint.drain());
                seen.iter()
                    .any(|m| matches!(m, ClientMsg::Close(c) if c.code == CloseCode::GOING_AWAY))
            })
            .await;
        }
    }

    // ── 걸린 닫기는 제어가 아니라 **시한**이 자른다 ──
    //
    // ★쓰기가 이미 막힌 통로에서 닫기도 함께 막힌다★ — 하네스의 `stall_writes` 가 `send`·`ping`·`close`
    //   셋 다 막으므로(`testing::MemoryEndpoint::stall_writes`) 실소켓의 그 모양이 여기서 재진다.
    // ★시계를 미는 줄을 빼지 말 것★ — `drop_link` 가 더는 제어로 그 대기를 자르지 않으므로(그 자리
    //   rustdoc 의 「알려진 한계」), 빼면 감독이 걸린 닫기 위에 선 채로 `settle_until` 이 한도에서 죽는다.
    #[tokio::test]
    async fn a_parked_closing_frame_is_cut_by_the_write_deadline_not_by_control() {
        let (h, endpoint) = live().await;
        endpoint.stall_writes(true);
        endpoint.fail("link died");
        settle_until("통로가 죽어 백오프로 내려갔다", || {
            h.peer.peer_state() == PeerState::Backoff
        })
        .await;
        assert!(
            !endpoint
                .drain()
                .iter()
                .any(|m| matches!(m, ClientMsg::Close(_))),
            "★닫기가 실제로 걸려 있어야 이 테스트가 무언가를 잰다★ — 나갔으면 아래 단언은 공짜다"
        );
        h.peer.close();
        // 걸린 닫기를 자르는 것은 이 줄이다. 제어는 큐에 남아 그 다음에 읽힌다.
        h.clock.advance(Duration::from_secs(5));
        settle_until("걸린 닫기 뒤에도 제어가 살아 있다", || {
            h.peer.peer_state() == PeerState::Closed
        })
        .await;
        assert!(
            !endpoint
                .drain()
                .iter()
                .any(|m| matches!(m, ClientMsg::Close(_))),
            "혼잡해서 시한에 잘리면 닫기 코드는 못 나간다 — `lib.rs` 가 인정한 그 한계다"
        );
    }

    // ── F1 회귀: 신호가 큐에 남아 있어도 goodbye 를 잃지 않는다 ──
    //
    // ★`reconnect_now()` 를 **두 번** 부르는 것이 요점이다★ — 감독은 한 바퀴에 신호를 하나만 집으므로
    //   둘째가 큐에 남고, 그 뒤 `drop_link` 가 제어 팔을 걸면 그 팔이 곧바로 `Ready` 라 닫기 future 가
    //   폴링조차 안 된 채 버려진다. 혼잡은 하나도 없다 — 실측 2026-09-07 로 100회 중 41·54·49·49회가
    //   그렇게 goodbye 를 잃었고, 고친 뒤 0회다.
    // ★한 바퀴로 재지 말 것★ — 회귀가 되살아나도 절반은 통과하므로, 세 번째 재발인 이 결함의 그물이
    //   동전 던지기가 된다. 아래 반복이 그것을 결정적으로 만든다.
    #[tokio::test]
    async fn a_queued_control_signal_does_not_swallow_the_goodbye_code() {
        let (mut h, mut endpoint) = live().await;
        for round in 0..24 {
            let dials = h.net.dials();
            h.peer.reconnect_now();
            h.peer.reconnect_now();
            settle_until("다시 붙었다", || {
                h.net.dials() == dials + 1 && h.peer.peer_state() == PeerState::Live
            })
            .await;
            assert!(
                endpoint
                    .drain()
                    .iter()
                    .any(|m| matches!(m, ClientMsg::Close(c) if c.code == CloseCode::GOING_AWAY)),
                "{round}번째 바퀴에서 goodbye 가 통로로 안 나갔다"
            );
            h.drain();
            endpoint = h.net.accept().expect("새 통로");
        }
    }
}
