//! DaemonClient — the shell's single authority over the daemon WS connection (S14 module ①, ADR-0036).
//!
//! **One connection to the daemon however many windows** — the frontend reaches the daemon through the shell, not
//! with a WS per window (`src/api/wsTransport.ts`). This module owns that connection's setup, handshake and life.
//! Protocol semantics (epoch guard · pending matching) are `protocol_state.rs`, output routing is `output_router` /
//! `output_channel`.
//!
//! - The connection runs on the driver in `connection_v2.rs` over the transport crate (`link/`) — its module doc holds
//!   the concurrency model (one actor owns the routing; callers' round trips run on their own tasks).
//! - `connect` (explicit, may spawn the daemon) and `ensure` (attach-only, never spawns) stay apart — ADR-0021.
//! - States: connected / down / reconnecting (the link driver never shows connecting).
//! - The old driver (`connection.rs` — one connection task owning the stream; `lifecycle.rs` — the generation guard,
//!   Fix B) runs only under its own test constructors until 3-2 U4b deletes it.

pub mod connection;
// ADR-0310: the driver on the transport crate (TRD S21 trd-3 §6-2 U3) — every client runs on it, except the old driver's
//   test constructors (their suite goes with 3-2 U4b).
mod connection_v2;
// ADR-0012: the seam for the connection's emits to the frontend (real AppHandle / recording fake / no-op). Without it
// this module's unit harness cannot stand — the reasons and contract live in that file's header.
// ★Being `pub(crate)` is the point★: its consumers are the connection drivers inside this crate; opening it would
//   leak the `AppHandle` carrier onto the lib surface with nowhere outside the crate to plug it in.
pub(crate) mod events;
// ADR-0231: 출력 binary frame 한 장의 중계 판정(디코드 · 화신 표식 거름 · 보는 창) — seq 를 건너뛰지 않는다.
mod frame_relay;
// ADR-0155 결정 4: 데몬이 보낸 명령을 받는 입구. 적용은 연결 태스크 밖에서 돈다.
pub mod inbound;
// The old driver's generation guard — only its test constructors run it (gone with 3-2 U4b).
#[cfg(test)]
mod lifecycle;
// ADR-0310: the daemon link on the transport crate (TRD S21 trd-3 §6-2 U2).
pub mod link;
pub mod protocol_state;
pub(crate) mod refusal;
// ADR-0046 M1: single-flight replay 채번/펜스 상태기계 + replay 경계 마커 인코딩(순수 — 소켓/Tauri 의존 0).
pub mod replay_flight;
// ADR-0271: graceful 데몬 끄기(StopDaemon 일방 발사) — 3-2 가 다시 쓸 때까지의 임시 거처.
pub mod stop;
// 사용량 관심(레이아웃 → 데몬 구독 집합) + 구독 대조·캐시 상태기(순수 — TRD S21 usage-limit-slot §1-7).
pub mod usage_interest;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use engram_dashboard_base::sync;
use engram_dashboard_protocol::{AgentCommand, AgentEvent, AgentId, AgentInfo, DaemonInfo};
use engram_dashboard_transport::ws::WsDialer;
use engram_dashboard_transport::SystemClock;
use tokio::runtime::Handle;
#[cfg(test)]
use tokio::sync::mpsc;
use tokio::sync::watch;

use connection::HandshakeError;
#[cfg(test)]
use connection::{run_connection, ConnectionCommand, HANDSHAKE_TIMEOUT};
#[cfg(test)]
use events::ConnectionStateEvent;
use events::{DaemonEvents, TauriEmitter};
// 알림을 버리는 조립은 하네스 전용이다(`events::NoDaemonEvents` doc) — 운영 빌드엔 아예 없다.
#[cfg(test)]
use events::NoDaemonEvents;
use inbound::InboundSlot;
#[cfg(test)]
use lifecycle::Lifecycle;
use link::endpoint::LocalEndpointSource;
use link::DaemonKey;
use refusal::{RefusalAlerts, TauriRefusalBox};
// 박스를 버리는 조립도 하네스 전용이다(`refusal::NoRefusalNotice` doc).
#[cfg(test)]
use refusal::{NoRefusalNotice, RefusalNotice};
use usage_interest::{InterestAction, UsageInterest};

use crate::layout::ViewManager;
use crate::output_channel::WindowChannelRegistry;
use crate::output_router::OutputRouter;

/// The connection's lifetime state. On the link driver, routing (`link/route.rs`) is its only emitter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Not attached, explicitly closed, refused, or a never-connected handle's first failure (the link may still retry
    /// in the background — ADR-0346, TRD rev 9 ①).
    Down,
    // Connecting / handshaking (socket open … `Hello`) — the old driver only; the link driver never shows it.
    Connecting,
    // `Hello` received = authenticated. Commands and subscriptions work.
    Connected,
    // Retrying after an unintended loss (backoff … next attempt); the link driver never gives up on network failures.
    // Matches wsTransport's `reconnecting` — distinct from an explicit close (Down).
    Reconnecting,
}

/// The shared handle to one shell's usage interest ([`UsageInterest`]) — the connection driver, the side that changes
/// the interest (layout recompute · hide · shrink timer) and the webview pull hold the same one (TRD S21
/// usage-limit-slot §1-7).
///
/// ★Lock order = `ViewManager → this lock`★. On the link driver a nudge ([`DaemonClient::nudge_usage_interest`])
/// issued inside this lock only sends to the driver's actor (non-blocking); the driver takes the ViewManager lock
/// ([`AgentNamesSink`]) holding no lock, so there is no reverse order. (The old driver's test-only path takes its
/// lifecycle lock inside here too — gone with 3-2 U4b.) Inside this lock only [`UsageInterest`] methods, that nudge and
/// the link driver's usage-subscribe queueing run; ★emits happen after the lock is released★ (ADR-0006). The link
/// driver queues its usage subscribe inside this lock on purpose, so the order sets are read is the order they are
/// queued — that queueing (`Peer::notify_in`) is a non-blocking leaf, and the peer is looked up before the lock is
/// taken.
///
/// ★오염돼도 되찾는다★: 그 메서드 안에서 패닉이 나 상태가 반쯤 고쳐진 채 남아도 결과는 데몬과 어긋난 구독이나
/// 빗나간 전달 대상이고, 다음 재계산 · 스냅숏 대조 · 새 소켓이 다시 맞춘다. 오염을 전파하면 연결 태스크가 그
/// 자리에서 죽어 출력·명령이 통째로 멎는다.
#[derive(Clone, Default)]
pub struct SharedUsageInterest(Arc<Mutex<UsageInterest>>);

impl SharedUsageInterest {
    pub fn lock(&self) -> MutexGuard<'_, UsageInterest> {
        sync::lock(&self.0)
    }
}

/// 데몬 `AgentListUpdated` 의 이름 몫을 받는 포트 — 탭 자동 이름이 읽는 에이전트 이름 장부를 채운다(TRD
/// S26-window-chrome §4-3). 실물 = 셸 조립(`lib.rs`)이 레이아웃 적용 서비스 `layout::apply::agent_names_updated` 를
/// 부르는 것, 하네스 = 버림(`NoAgentNames`) · 기록형.
///
/// - 알림 포트 `events::DaemonEvents` 를 넓히지 않고 따로 끊은 형제다(그 doc 이 넓히기를 금한다) — 그 포트는 프론트에
///   알리기만 하고, 이 포트의 실물은 셸 상태(레이아웃 락 안의 장부)를 고친다.
/// - ★연결 태스크가 아무 락도 쥐지 않은 채 부른다★ — 실물이 레이아웃 락을 잡고, 그 락 안에서 사용량 관심 · lifecycle
///   락을 잡는 차례가 있다([`SharedUsageInterest`] doc). 그 둘을 쥔 채 부르면 역순이다.
/// - 구현은 레이아웃 락의 짧은 임계구역 말고는 막지 않고 실패를 삼킨다 — 장부를 못 고친 것은 연결을 되돌릴 사유가
///   아니고, 다음 목록이 다시 채운다.
pub trait AgentNamesSink: Send + Sync {
    /// `AgentListUpdated` 마다 그 목록 그대로 — 프론트 알림(`agent_list_updated`)을 낸 뒤에 불린다.
    fn agents(&self, agents: &[AgentInfo]);
}

/// 이름 장부 포트를 버리는 조립 — ★하네스 전용이다★. `#[cfg(test)]` 인 이유는 `events::NoDaemonEvents` 와 같다 —
/// 운영 빌드에 두면 데몬 목록이 와도 탭 이름이 「에이전트」에 머무는 클라이언트를 조용히 조립할 길이 열린다.
#[cfg(test)]
pub(crate) struct NoAgentNames;

#[cfg(test)]
impl AgentNamesSink for NoAgentNames {
    fn agents(&self, _agents: &[AgentInfo]) {}
}

// 데몬 발견 경계(seam). connect 경로는 spawn 가능(`ensure_spawn`), ensure 경로는 no-spawn
// (`read_live`)만 — ADR-0021 분리를 이 trait 의 **서로 다른 메서드**로 못박는다.
//
// ★왜 trait 인가★: 실제 구현은 `crate::discovery`(WMI spawn·파일 IO·실시간)에 닿아 단위 테스트가
// 실 데몬을 띄워야 한다. seam 으로 끊어 테스트가 "spawn 호출 0회"(ensure no-spawn 불변)와
// "주어진 host/port 반환"을 실 WMI 없이 단언한다(`crate::discovery` 의 DaemonReader/Spawner 주입 동형).
pub trait DaemonDiscovery: Send + Sync + 'static {
    // 명시 연결(connect) 경로. 살아있는 데몬을 찾고, 없으면 **spawn** 해서 접속 정보를 돌려준다.
    // wsTransport 의 `invoke('discover_daemon')` 대응(spawn 유발 = 데몬이 살아날 수 있음).
    fn ensure_spawn(&self, timeout: Duration) -> Result<DaemonInfo, String>;

    // 재연결/ensure(attach-only) 경로. 현재 daemon.json 을 **읽기만** 한다(no-spawn). 살아있는
    // 호환 데몬이면 Some, 없으면 None. wsTransport 의 `invoke('read_daemon_info')` 대응.
    // ★불변식(ADR-0021)★: 이 메서드는 절대 spawn 하지 않는다 — 명령/재연결이 데몬을 깨우면 안 된다.
    fn read_live(&self) -> Option<DaemonInfo>;
}

// 운영 DaemonDiscovery — `crate::discovery` 에 위임. connect=ensure_daemon(spawn 가능),
// ensure=read_live_daemon(no-spawn).
//
// ★blocking 주의★: ensure_daemon 은 폴링·sleep·WMI 동기 호출을 포함한다. 호출자(연결 task)는
// `spawn_blocking` 으로 감싸 async executor 를 막지 않는다(connection.rs 참조).
pub struct RealDiscovery;

impl DaemonDiscovery for RealDiscovery {
    fn ensure_spawn(&self, timeout: Duration) -> Result<DaemonInfo, String> {
        let data_dir: PathBuf = crate::discovery::default_data_dir();
        // console=false: windowless spawn(콘솔 가시화는 daemon_start command 전용).
        let exe = crate::discovery::locate_daemon_exe().map_err(|e| e.to_string())?;
        crate::discovery::ensure_daemon(&data_dir, &exe, timeout, false).map_err(|e| e.to_string())
    }

    fn read_live(&self) -> Option<DaemonInfo> {
        let data_dir: PathBuf = crate::discovery::default_data_dir();
        crate::discovery::read_live_daemon(&data_dir)
    }
}

/// The discovery of a client on the new driver ([`DaemonClient::new_on_link`]) — that driver finds the daemon through
/// its endpoint source, so nothing may reach this one; it fails instead of finding a daemon.
#[cfg(test)]
struct OnTheLink;

#[cfg(test)]
impl DaemonDiscovery for OnTheLink {
    fn ensure_spawn(&self, _timeout: Duration) -> Result<DaemonInfo, String> {
        Err("this client connects through the link driver".to_owned())
    }

    fn read_live(&self) -> Option<DaemonInfo> {
        None
    }
}

// discover(spawn 가능) timeout 기본값(wsTransport discover_daemon 5s 와 정렬).
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

// TRD S21 trd-3 §5 15 (C10): one ws message or frame on the daemon link is at most 16 MiB — the shell's side; the
//   daemon's accept side takes the same value in stage 3-3. The largest legitimate message is not measured.
const DAEMON_MESSAGE_MAX_BYTES: usize = 16 << 20;

/// 지금 명령을 받을 소켓이 없을 때 호출자가 받는 문구(ADR-0195) — 「한 번도 연결 안 함」·「끊겨 재연결
/// 중」·「재연결 소진」·「명시 종료」가 **전부** 이 하나로 떨어진다. 갈래를 문구로 가르지 않는 것은
/// 호출자가 할 일이 넷 다 같기 때문이다.
///
/// ★「connect 먼저」라고 시키지 않는다★ — 재연결이 도는 중에 그 말을 따르면 `connect()` 가 세대를 올리고
/// `cancel_tx` 를 쏴서 **진행 중인 복구를 승계로 끊는다**. 이 문구는 §5 제어 표면을 타고 LLM 호출자에게도
/// 그대로 가므로, 틀린 다음 행동을 적어 두면 그쪽이 실제로 그것을 한다. 재시도 시점을 호출자가 정한다는
/// 것이 ADR-0195 의 결정이고, 문구도 그렇게만 말한다.
pub(crate) const NOT_CONNECTED: &str =
    "데몬에 연결되어 있지 않음 — 지금은 명령을 받지 않는다(재시도 시점은 호출자가 정한다)";

/// 창구는 열려 있었는데 그 채널의 수신단이 이미 사라진 경우 — [`NOT_CONNECTED`] 와 **다른 사실**이다
/// (사유·도달 경로는 `DaemonClient::send_command` 의 「두 not-connected 문구가 갈리는 자리」).
pub(crate) const CHANNEL_GONE: &str = "연결 task 가 명령을 받지 못함(끊김)";

/// 명령은 채널에 들어갔는데 그 답을 **아무도 돌려주지 않은 채** reply oneshot 이 떨어진 경우. 도달 경로
/// 둘: 연결 task 가 죽었거나, 재연결 없이 끝나는 갈래(소진 · 명시 종료 · Stop)에서 아직 큐에 있던
/// 명령이 표식 대조를 지나지 못한 채 `cmd_rx` 와 함께 사라졌거나(그 자리 = `connection` 의 버퍼 drain
/// 주석). ★끊김 계열 문구들과 갈라 두는 이유★: 이건 「나갔나 안 나갔나」를 **모르는** 상태라,
/// [`connection::UNSENT_ON_DISCONNECT`] 의 「재전송 안전」을 여기에 붙이면 거짓이 된다.
#[cfg(test)]
pub(crate) const REPLY_LOST: &str = "명령 응답 수신 실패(연결 task 종료)";

/// A replay request that wrote no `Subscribe` and got no generation — the link dropped, or no window shows the agent.
/// Shared by the old connection task's dropped reply and the link driver's refusal, so the two cannot drift.
pub(crate) const REPLAY_UNSENT: &str =
    "replay 요청 미전송(연결 끊김 · 그 에이전트를 보이는 창 없음) — 프론트 재요청 안전";

/// A round trip asked for a command that takes no reply (no `request_id`) — shared by the old driver's entry check
/// and the link's error texts (`link::errors`), so the two cannot drift.
pub(crate) const NO_REQUEST_ID: &str =
    "send_command: request_id 없는 명령은 reply 를 기대할 수 없다";

// The single handle to the daemon connection, shared by invoke handlers, the tray and state subscribers
// (`Arc<DaemonClient>`).
//
// The link driver (`connection_v2`) owns the connection: its actor owns the routing and the transport peer owns the
// socket; this struct holds the driver's caller side (`link`) and the state watch (`state_rx`) — never the stream.
pub struct DaemonClient {
    // 연결 task 를 spawn 할 런타임 핸들. 운영=전용 multi-thread(`_owned_rt`), 테스트=현재 런타임.
    rt: Handle,
    // ★전용 런타임 소유(운영 — T6a)★. `setup` 콜백은 tokio 런타임 컨텍스트가 아닐 수 있어
    // `Handle::current()` 가 패닉한다. 그래서 운영 생성자(`new_real_with_owned_runtime`)는 spike §2
    // "tokio multi-thread(데몬처럼)" 대로 전용 멀티스레드 런타임을 *직접 만들어* 그 Handle 을 쓴다.
    // 이 필드가 런타임을 살려둔다 — drop 되면 Handle 이 무효가 돼 연결 task 가 죽으므로 DaemonClient
    // 수명과 묶는다(`Arc<DaemonClient>` 가 app 수명). `None` = 외부 핸들 주입(테스트=현재 런타임).
    _owned_rt: Option<Arc<tokio::runtime::Runtime>>,
    /// The old driver's daemon discovery (connect may spawn / ensure never does) — its test constructors only.
    #[cfg(test)]
    discovery: Arc<dyn DaemonDiscovery>,
    /// The emit port (seam — ADR-0012) the old driver reads here; the link driver holds its own clone in its ports.
    /// Harness = `events::NoDaemonEvents` (no-op) or a recording fake. ★Not an `Option` on purpose★ — an emit path that
    /// may be absent grew into a "nothing to emit to, so no connection" short circuit that once killed this module's
    /// whole suite (↓ `start_connection`).
    #[cfg(test)]
    events: Arc<dyn DaemonEvents>,
    /// The filter for the command-registration refusal box (ADR-0281). Production = `refusal::TauriRefusalBox`,
    /// harness = `refusal::NoRefusalNotice` (record with [`Self::with_refusal_notice`]).
    /// ★The client holds one and shares it with the driver — never make one per connection★: repeat suppression lives
    /// as long as the shell process (`RefusalAlerts` doc), so one per connection re-shows the box on every reconnect
    /// that gets the same refusal.
    refusals: Arc<RefusalAlerts>,
    /// The agent-name port ([`AgentNamesSink`]) the old driver reads here; the link driver holds its own clone in its
    /// ports. Harness = `NoAgentNames` (record with [`Self::with_agent_names`]).
    #[cfg(test)]
    names: Arc<dyn AgentNamesSink>,
    /// The connection state, read without a lock — the driver's watch (routing on the link driver; the old driver
    /// publishes it under its lifecycle lock). Only borrowed here.
    state_rx: watch::Receiver<ConnectionState>,
    /// The old driver's generation guard — its test constructors only.
    #[cfg(test)]
    lifecycle: Arc<Lifecycle>,
    /// The old driver's handshake bound (socket open … `Hello`) — its test constructors only.
    #[cfg(test)]
    handshake_timeout: Duration,
    /// Output routing — agent_id → window labels (arc-swap hot path) that the layout commands rebuild. The old driver
    /// reads it here; the link driver holds its own clone in its ports.
    #[cfg(test)]
    router: Arc<OutputRouter>,
    /// The windows' output channels — window label → channel, inserted by `subscribe_output`. The old driver reads it
    /// here; the link driver holds its own clone in its ports.
    #[cfg(test)]
    registry: WindowChannelRegistry,
    /// ★The entry for commands the daemon delivers★ (ADR-0155 decision 4). Filled late — the table's spawn port holds
    ///   this client, so the assembly has a cycle (`inbound::InboundSlot` doc). Shared with the driver.
    inbound: Arc<InboundSlot>,
    /// The usage interest — outlives every connection (the interest comes from the layout, not the socket). Shared
    ///   with the driver.
    usage: SharedUsageInterest,
    /// The driver on the transport crate (`connection_v2`) — connect, ensure, close, the command round trips,
    /// fire-and-forget sends, unsubscribe, replay requests and usage nudges go there; `state_rx` is its watch.
    #[cfg(not(test))]
    link: connection_v2::LinkClient,
    /// As in production, but `None` on the old driver's test constructors — those take the old path (gone with 3-2
    /// U4b).
    #[cfg(test)]
    link: Option<connection_v2::LinkClient>,
}

impl DaemonClient {
    // 핸들만 만든다(연결은 connect/ensure 호출 시). `rt` 는 연결 task 를 띄울 런타임 핸들.
    // 핸드셰이크 상한은 운영 기본값(HANDSHAKE_TIMEOUT).
    //
    // ★emit 은 `NoDaemonEvents`(버림)로 조립된다★ — 연결·재연결·명령 왕복은 전부 실코드로 돌고 **프론트
    // 알림만** 나가지 않는다. 운영 조립은 `new_real_with_owned_runtime`(실 `AppHandle`)이다.
    //
    // ## ★`#[cfg(test)]` 인 것이 요점이다(되돌리지 말 것)★
    // 2026-08-24 이전 이 생성자는 `pub` 이고 `#[cfg(test)]` 도 아니었다. 그때는 그 조립이 **연결 태스크를
    // 아예 안 띄워서**(↓ `start_connection` 의 옛 단락) 조용한 no-op 이었고, 단락을 지운 지금은 소켓·핸드셰이크
    // ·명령 왕복이 전부 진짜로 도는 채 **화면만 갱신되지 않는다**. 프론트의 자가복구는 연결 상태만 메우고
    // (`src/api/tauriTransport.ts` Fix-D) `agent-list-updated` 는 못 메우므로, 그런 클라이언트를 조립한
    // 비테스트 호출자는 「연결은 됐는데 목록이 안 뜬다」를 만난다. 그래서 덫을 타입 밖으로 밀어냈다 —
    // 알림을 버리는 조립은 이제 **테스트 빌드에만 존재한다**. 알림을 재고 싶으면 `new_with_events` 로 간다.
    #[cfg(test)]
    pub fn new(rt: Handle, discovery: Arc<dyn DaemonDiscovery>) -> Self {
        Self::new_with_handshake_timeout(rt, discovery, HANDSHAKE_TIMEOUT)
    }

    // 핸드셰이크 상한을 주입하는 생성자(Fix A 테스트 용이성 — 테스트가 짧은 값으로 Timeout 을 검증).
    // const 하드코딩이 테스트를 10초 기다리게 만들지 않도록, 상한을 필드로 받는다.
    #[cfg(test)]
    pub fn new_with_handshake_timeout(
        rt: Handle,
        discovery: Arc<dyn DaemonDiscovery>,
        handshake_timeout: Duration,
    ) -> Self {
        Self::new_with_events(rt, discovery, handshake_timeout, Arc::new(NoDaemonEvents))
    }

    /// ★알림 포트의 주입점 — 이 seam 이 검증을 사는 자리가 여기 하나다★(ADR-0012 ·
    /// `inbound::InboundReceiver::with_view` 와 동형).
    ///
    /// 위 두 생성자는 `NoDaemonEvents` 를 꽂아 **발화를 버린다** — 그 조립만 있으면 11개 emit 은 영원히
    /// 무검증으로 남는다(포트를 끊어 놓고 그 값을 안 사는 셈이다). 기록형 가짜를 여기로 넣으면 연결 태스크가
    /// **무엇을 어떤 순서로** 알리는지를 실 소켓 왕복 위에서 단언할 수 있다 —
    /// `tests.rs` 의 `recording_events_capture_connected_then_broadcasts` 가 그 첫 소비자다.
    ///
    /// ★`pub(crate)` 인 이유★: `DaemonEvents` 자체가 crate 안에서만 보이므로(`events` 모듈 주석) 이보다
    /// 넓히면 lib 표면에 crate-private 타입이 새는 경고가 난다. crate 밖에서 꽂을 자리도 없다.
    #[cfg(test)]
    pub(crate) fn new_with_events(
        rt: Handle,
        discovery: Arc<dyn DaemonDiscovery>,
        handshake_timeout: Duration,
        events: Arc<dyn DaemonEvents>,
    ) -> Self {
        let (lifecycle, state_rx) = Lifecycle::new();
        Self {
            rt,
            _owned_rt: None,
            discovery,
            state_rx,
            lifecycle: Arc::new(lifecycle),
            handshake_timeout,
            // ★테스트 기본값★: router/registry 를 주입받지 않는 생성자(new/new_real/테스트)는 빈 라우팅
            //   표 + 빈 registry 로 둔다 — connection task 가 frame 을 라우팅해도 대상 0(no-op). 운영은
            //   new_real_with_owned_runtime 이 lib.rs setup 이 만든 공유 Arc 를 주입한다.
            router: Arc::new(OutputRouter::new()),
            registry: WindowChannelRegistry::default(),
            inbound: Arc::new(InboundSlot::new()),
            events,
            refusals: Arc::new(RefusalAlerts::new(Arc::new(NoRefusalNotice))),
            names: Arc::new(NoAgentNames),
            usage: SharedUsageInterest::default(),
            link: None,
        }
    }

    /// 거절 박스 포트를 갈아 끼운다 — ★연결 전에 부른다★(연결 task 는 그때 쥔 거르개를 끝까지 쓴다).
    /// 기록형 대역(`refusal::RecordingRefusals`)으로 박스 결정을 실 소켓 왕복 위에서 단언하는 자리다.
    #[cfg(test)]
    pub(crate) fn with_refusal_notice(mut self, port: Arc<dyn RefusalNotice>) -> Self {
        self.refusals = Arc::new(RefusalAlerts::new(port));
        self
    }

    /// 이름 장부 포트를 갈아 끼운다 — ★연결 전에 부른다★([`Self::with_refusal_notice`] 와 같은 까닭). 기록형 대역으로
    /// 실 소켓 왕복 위에서 목록이 이 포트에 닿는지 단언하는 자리다.
    #[cfg(test)]
    pub(crate) fn with_agent_names(mut self, port: Arc<dyn AgentNamesSink>) -> Self {
        self.names = port;
        self
    }

    /// A client on the link driver over a test network (TRD S21 trd-3 §6-2 U3): `inner` dials the address the endpoint
    /// `source` resolves (a memory network), `clock` drives the transport. The assembly is production's
    /// ([`Self::on_link`]); this client's `router` and `registry` are its own, empty ones. ★The ports go in here, not
    /// through [`Self::with_agent_names`] · [`Self::with_refusal_notice`]★ — the driver starts with this call and keeps
    /// what it got.
    #[cfg(test)]
    pub(crate) fn new_on_link(
        rt: Handle,
        inner: Arc<dyn engram_dashboard_transport::Dialer>,
        source: Arc<dyn link::endpoint::EndpointSource>,
        clock: Arc<dyn engram_dashboard_transport::Clock>,
        events: Arc<dyn DaemonEvents>,
        names: Arc<dyn AgentNamesSink>,
        refusal_notice: Arc<dyn RefusalNotice>,
    ) -> Self {
        Self::on_link(
            rt,
            None,
            inner,
            source,
            clock,
            events,
            names,
            Arc::new(RefusalAlerts::new(refusal_notice)),
            Arc::new(OutputRouter::new()),
            WindowChannelRegistry::default(),
        )
    }

    /// Starts the link driver on `rt` and builds the client around it. Every connection path of the client runs on
    /// the driver, and the driver's ports are this client's own objects (`router`, `registry`, command table slot,
    /// refusal filter, usage interest), so [`Self::install_command_table`] · [`Self::refusal_alerts`] ·
    /// [`Self::usage_interest`] see what the driver sees.
    #[allow(clippy::too_many_arguments)]
    fn on_link(
        rt: Handle,
        owned_rt: Option<Arc<tokio::runtime::Runtime>>,
        inner: Arc<dyn engram_dashboard_transport::Dialer>,
        source: Arc<dyn link::endpoint::EndpointSource>,
        clock: Arc<dyn engram_dashboard_transport::Clock>,
        events: Arc<dyn DaemonEvents>,
        names: Arc<dyn AgentNamesSink>,
        refusals: Arc<RefusalAlerts>,
        router: Arc<OutputRouter>,
        registry: WindowChannelRegistry,
    ) -> Self {
        let inbound = Arc::new(InboundSlot::new());
        let usage = SharedUsageInterest::default();
        let link = connection_v2::LinkClient::start(
            &rt,
            inner,
            source,
            clock,
            connection_v2::Ports {
                events: events.clone(),
                names: names.clone(),
                router: router.clone(),
                windows: registry.clone(),
                inbound: inbound.clone(),
                refusals: refusals.clone(),
                usage: usage.clone(),
            },
        );
        Self {
            rt,
            _owned_rt: owned_rt,
            #[cfg(test)]
            discovery: Arc::new(OnTheLink),
            state_rx: link.state_rx(),
            #[cfg(test)]
            lifecycle: Arc::new(Lifecycle::new().0),
            #[cfg(test)]
            handshake_timeout: HANDSHAKE_TIMEOUT,
            #[cfg(test)]
            router,
            #[cfg(test)]
            registry,
            inbound,
            #[cfg(test)]
            events,
            refusals,
            #[cfg(test)]
            names,
            usage,
            #[cfg(not(test))]
            link,
            #[cfg(test)]
            link: Some(link),
        }
    }

    // 실 discovery + `NoDaemonEvents`. ★emit 없는 조립이라는 것이 `new` 와 같다★ — 운영 셸은 이걸 부르지
    // 않는다(`lib.rs` = `new_real_with_owned_runtime`). ★`#[cfg(test)]` 사유는 `new` 와 같고, 실 discovery 를
    // 함께 쥐는 만큼 이쪽이 더 위험했다★ — 진짜 데몬에 붙은 채 화면만 죽는 조립이었다.
    #[cfg(test)]
    pub fn new_real(rt: Handle) -> Self {
        Self::new(rt, Arc::new(RealDiscovery))
    }

    /// ★테스트 전용★: 외부에서 만든 `OutputRouter` 를 주입한다(handshake_timeout 은 운영 기본값).
    /// **호출자 0** — eager resubscribe 검증 테스트가 ADR-0046 으로 삭제된 뒤 남았다.
    #[cfg(test)]
    pub fn new_with_router(
        rt: Handle,
        discovery: Arc<dyn DaemonDiscovery>,
        router: Arc<OutputRouter>,
    ) -> Self {
        let (lifecycle, state_rx) = Lifecycle::new();
        Self {
            rt,
            _owned_rt: None,
            discovery,
            state_rx,
            lifecycle: Arc::new(lifecycle),
            handshake_timeout: HANDSHAKE_TIMEOUT,
            router,
            registry: WindowChannelRegistry::default(),
            inbound: Arc::new(InboundSlot::new()),
            events: Arc::new(NoDaemonEvents),
            refusals: Arc::new(RefusalAlerts::new(Arc::new(NoRefusalNotice))),
            names: Arc::new(NoAgentNames),
            usage: SharedUsageInterest::default(),
            link: None,
        }
    }

    /// ★테스트 전용★: router + handshake_timeout 둘 다 주입 — new_with_router 와 동형 + timeout 만 추가.
    /// **호출자 0** — 원 용도(start_paused 로 짧은 상한이 필요한 재연결 resubscribe 테스트)가 ADR-0046 으로
    /// 삭제됐다.
    #[cfg(test)]
    pub fn new_with_router_and_timeout(
        rt: Handle,
        discovery: Arc<dyn DaemonDiscovery>,
        router: Arc<OutputRouter>,
        handshake_timeout: Duration,
    ) -> Self {
        let (lifecycle, state_rx) = Lifecycle::new();
        Self {
            rt,
            _owned_rt: None,
            discovery,
            state_rx,
            lifecycle: Arc::new(lifecycle),
            handshake_timeout,
            router,
            registry: WindowChannelRegistry::default(),
            inbound: Arc::new(InboundSlot::new()),
            events: Arc::new(NoDaemonEvents),
            refusals: Arc::new(RefusalAlerts::new(Arc::new(NoRefusalNotice))),
            names: Arc::new(NoAgentNames),
            usage: SharedUsageInterest::default(),
            link: None,
        }
    }

    /// ★The production constructor★ — `lib.rs` `setup` calls it. The client owns a multi-thread runtime it builds here,
    /// because `setup` may run outside a tokio context, where `Handle::current()` panics (spike §2); the runtime lives
    /// as long as the client (`_owned_rt`). It runs on the link driver: the ws dialer, the local daemon's endpoint
    /// record (`daemon.json` in the default data folder) and the system clock. `Err` = the runtime could not be built;
    /// the caller reports it and goes on without daemon commands.
    ///
    /// `router` and `registry` are the shared instances `setup` also hands the layout commands and `subscribe_output`,
    /// so the output fan-out and those commands see the same ones.
    pub fn new_real_with_owned_runtime(
        router: Arc<OutputRouter>,
        registry: WindowChannelRegistry,
        app: tauri::AppHandle,
        names: Arc<dyn AgentNamesSink>,
    ) -> std::io::Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("engram-daemon-client")
            .build()?;
        let handle = rt.handle().clone();
        Ok(Self::on_local_daemon(
            handle,
            Some(Arc::new(rt)),
            crate::discovery::default_data_dir(),
            Arc::new(TauriEmitter(app.clone())),
            names,
            Arc::new(RefusalAlerts::new(Arc::new(TauriRefusalBox(app)))),
            router,
            registry,
        ))
    }

    /// [`Self::on_link`] on the local daemon of `data_dir`: the ws dialer, that folder's endpoint record
    /// (`daemon.json`) and the system clock — production's transport, which a test drives on a temporary folder.
    #[allow(clippy::too_many_arguments)]
    fn on_local_daemon(
        rt: Handle,
        owned_rt: Option<Arc<tokio::runtime::Runtime>>,
        data_dir: PathBuf,
        events: Arc<dyn DaemonEvents>,
        names: Arc<dyn AgentNamesSink>,
        refusals: Arc<RefusalAlerts>,
        router: Arc<OutputRouter>,
        registry: WindowChannelRegistry,
    ) -> Self {
        Self::on_link(
            rt,
            owned_rt,
            Arc::new(WsDialer::new(DAEMON_MESSAGE_MAX_BYTES)),
            Arc::new(LocalEndpointSource::new(
                DaemonKey::new(connection_v2::DAEMON),
                data_dir,
            )),
            Arc::new(SystemClock),
            events,
            names,
            refusals,
            router,
            registry,
        )
    }

    /// 이 클라이언트가 실행할 수 있는 명령 표를 꽂는다 — ★연결 전에 부른다★(ADR-0155 결정 4·5).
    ///
    /// ★Choosing the runtime here is the point★: the apply tasks must run on the **same runtime** as the connection
    /// driver — while `agent.spawnInto` waits on a daemon round trip, the driver's tasks must keep running for the
    /// answer to arrive (on another runtime that interleaving is left to chance). Letting each assembly pick the
    /// `Handle` would make every assembly re-keep that invariant, so the client decides with its own `rt`.
    /// Calling it late is allowed (the first winner wins), but **after a connection opened**, envelopes that arrived
    /// meanwhile get a "no table" error reply, and that connection sends no registration — the next connection does
    /// (the driver registers on every `LinkUp`).
    /// `view` = where names absent from the shell table go (the webview's share — TRD §3-8 hop ③). It is plugged in
    /// **at the same spot** as the table because the registration packet carries both layers in one go (TRD §3-7
    /// clause 2) — plugging them separately lets some assembly send a registration without the webview's share, and on
    /// that connection every view command is `UNKNOWN_COMMAND`.
    pub fn install_command_table(
        &self,
        table: engram_dashboard_command::CommandTable,
        catalog_version: u32,
        view: Arc<dyn inbound::ViewCommandPort>,
    ) {
        self.inbound
            .set(Arc::new(inbound::InboundReceiver::with_view(
                table,
                Arc::new(inbound::RuntimeSpawner(self.rt.clone())),
                catalog_version,
                view,
            )));
    }

    // 현재 연결 상태 스냅샷(락 없이).
    pub fn state(&self) -> ConnectionState {
        *self.state_rx.borrow()
    }

    // 상태 변경 구독(watch). 호출자가 await 로 다음 전이를 기다리거나 현재값을 본다.
    pub fn subscribe_state(&self) -> watch::Receiver<ConnectionState> {
        self.state_rx.clone()
    }

    /// 사용량 관심 상태 — 웹뷰 pull 이 캐시를 읽는다. 관심을 바꾸는 쪽은 이 손잡이가 아니라 재계산 셋
    /// ([`Self::usage_layout_changed`] · [`Self::usage_window_destroyed`] · [`Self::usage_window_visibility`])을
    /// 부른다 — 결과 처리(넛지 · 줄임 타이머)가 거기 한 벌이다. 락 차례는 [`SharedUsageInterest`] doc.
    pub fn usage_interest(&self) -> &SharedUsageInterest {
        &self.usage
    }

    /// 연결 task 의 등록 자리와 **같은** 거르개 — 차분 자리(`commands::view_bus`)가 쓴다.
    pub(crate) fn refusal_alerts(&self) -> &RefusalAlerts {
        &self.refusals
    }

    /// The link driver. ★Test builds★: an old-driver client (`link: None`) takes its own path before reaching this.
    #[cfg(not(test))]
    fn link(&self) -> &connection_v2::LinkClient {
        &self.link
    }

    /// See the production twin.
    #[cfg(test)]
    fn link(&self) -> &connection_v2::LinkClient {
        self.link
            .as_ref()
            .expect("an old-driver client takes its own path first")
    }

    /// Explicit connect (ADR-0021 §1 — may start a daemon; wsTransport `start()`). Contract: `LinkClient::connect`.
    pub async fn connect(&self) -> Result<(), HandshakeError> {
        #[cfg(test)]
        if self.link.is_none() {
            return self.connect_on_old_driver().await;
        }
        self.link().connect().await
    }

    // The old driver's path — its test constructors only (gone with 3-2 U4b).
    // 명시 연결 진입점(ADR-0021 §1) = wsTransport `start()` 대응.
    //
    // ★spawn 가능★: 데몬이 없으면 `discovery.ensure_spawn` 이 WMI 로 띄운다 — 부팅 연결/사용자
    // 명시 시작만 이 경로를 탄다. discover → WS → Auth → Hello → connected 까지 한 번에 간다.
    // 이미 connected 면 즉시 Ok(중복 연결 방지 — 주 가드는 generation, 이건 보조 단축).
    //
    // ## ★승계 취소를 discovery *전에* (FIX-1, T4 2차)★
    // 진입 즉시(느린 discovery await 전에) `bump_and_capture(Some(Connecting))` 으로 옛 세대를 취소+
    // 승계한다 — bump 가 cancel watch 에 신호를 쏘고 옛 cmd_tx 를 비운다. 그래야 discovery 창(spawn 가능
    // = 수십초까지 늘어날 수 있음) 동안 진행 중이던 옛 재연결 세대가 *그 창에서* 소켓을 열고 Auth 를
    // 보내지 못한다(OSS 정석: 승계 시 옛 토큰 즉시 취소). 이전엔 discovery 를 먼저 하고 start_connection
    // 안에서 bump 했어서 그 창이 무방비였다(Codex BLOCK). 캡처한 my_gen 을 그대로 start_connection 에
    // 넘겨 ★이중 bump 를 피한다★(start_connection 은 더 이상 bump 안 함).
    #[cfg(test)]
    async fn connect_on_old_driver(&self) -> Result<(), HandshakeError> {
        if self.state() == ConnectionState::Connected {
            return Ok(());
        }
        tracing::info!("데몬 연결 시작(connect — spawn 가능 경로)");
        // ★진입 즉시 승계 취소(FIX-1)★: discovery await 전에 세대를 올려 옛 재연결을 cancel + stale 화한다.
        //   bump_and_capture 가 (a)세대++ (b)closed_by_user=false (c)옛 cmd_tx=None (d)cancel 신호 (e)Connecting
        //   발행을 한 락 원자로 한다. 이 my_gen 을 start_connection 에 넘겨 이중 bump 를 피한다.
        let my_gen = self
            .lifecycle
            .bump_and_capture(Some(ConnectionState::Connecting));
        // ★spawn 허용 경로★: ensure_spawn(데몬 없으면 띄움). blocking 이라 spawn_blocking 으로 감싼다.
        //   이 await 동안 옛 **재연결** 세대는 취소·stale 이라 소켓을 못 연다(위 bump 가 닫은 창) — 첫
        //   핸드셰이크 중인 세대는 cancel 미구독이라 이 가드 밖이다(↑ 모듈 헤더 "남은 허용 범위").
        let discovery = self.discovery.clone();
        let info = match self
            .rt
            .spawn_blocking(move || discovery.ensure_spawn(DEFAULT_CONNECT_TIMEOUT))
            .await
        {
            Ok(Ok(info)) => info,
            Ok(Err(e)) => {
                tracing::warn!("데몬 발견/spawn 실패: {e}");
                // 내가 올린 세대가 아직 current 면 Down 으로(가드된). 더 새 connect/close 가 끼었으면 미발행.
                // ★내부 전이만 접으면 창은 모른다★: 이 실패를 아는 것은 반환 `Err` 를 받는 그 호출자 하나뿐이고,
                //   나머지 창은 **마지막으로 들은 발화**에 머문다. 진입의 `Connecting` 은 발화 어휘 자체가 없어
                //   (`events::ConnectionStateEvent` 는 셋뿐) 그 자리를 못 메운다 — 끊김 뒤 재진입이면 마지막
                //   발화가 `reconnecting` 인 채 굳고, 이 경로는 재연결 루프에도 들지 않아 아무도 재시도하지 않는다.
                // ★가드는 그대로★: stale 이면 미발행 — 더 새 연결의 Connected 를 Down 으로 clobber 하지 않는다.
                if self
                    .lifecycle
                    .publish_if_current(my_gen, ConnectionState::Down)
                {
                    self.events.connection_state(ConnectionStateEvent::Down);
                } else {
                    tracing::debug!(
                        generation = my_gen,
                        "발견/spawn 실패 — Down 미발행(더 새 연결이 current)"
                    );
                }
                return Err(HandshakeError::Discovery(e));
            }
            Err(e) => {
                tracing::warn!("데몬 discovery join 실패: {e}");
                // 발화를 짝지어야 하는 사유·가드는 위 분기와 같다(여기선 discovery 의 join 이 깨진 경우 — 그것이
                //   패닉이면 운영 셸은 훅이 먼저 끝내므로 훅 없는 시험 빌드의 갈래다 · ADR-0292).
                if self
                    .lifecycle
                    .publish_if_current(my_gen, ConnectionState::Down)
                {
                    self.events.connection_state(ConnectionStateEvent::Down);
                } else {
                    tracing::debug!(
                        generation = my_gen,
                        "discovery join 실패 — Down 미발행(더 새 연결이 current)"
                    );
                }
                return Err(HandshakeError::Discovery(format!("ensure join 실패: {e}")));
            }
        };
        self.start_connection(info, my_gen).await
    }

    /// Attach-only connect (ADR-0021 B-1 — never starts a daemon; wsTransport `ensureReady()`). Contract:
    /// `LinkClient::ensure`.
    pub async fn ensure(&self) -> Result<(), HandshakeError> {
        #[cfg(test)]
        if self.link.is_none() {
            return self.ensure_on_old_driver().await;
        }
        self.link().ensure().await
    }

    // The old driver's path — its test constructors only (gone with 3-2 U4b).
    // attach-only 진입점(ADR-0021 B-1) = wsTransport `ensureReady()` 대응.
    //
    // ★no-spawn★: `discovery.read_live`(daemon.json read-only)만 부른다 — 데몬이 없으면 띄우지
    // 않고 실패한다(명령이 데몬을 respawn 하면 안 됨). 살아있는 데몬에만 attach.
    // 이미 connected 면 즉시 Ok(주 가드는 generation, 이건 보조 단축).
    //
    // ## ★승계 취소를 read_live *전에* (FIX-1, T4 2차)★
    // connect() 와 동형: read_live(no-spawn 이라 짧지만, 파일 IO 가 느릴 여지)를 부르기 전에 bump 로 옛
    // 세대를 취소·승계한다. ensure 는 attach-only(ADR-0021 — no-spawn)지만 *승계 취소* 는 동일 적용 —
    // read_live 창 동안 옛 재연결이 소켓을 열지 못하게. 캡처한 my_gen 을 start_connection 에 넘긴다.
    #[cfg(test)]
    async fn ensure_on_old_driver(&self) -> Result<(), HandshakeError> {
        if self.state() == ConnectionState::Connected {
            return Ok(());
        }
        tracing::info!("데몬 연결 시작(ensure — attach-only, no-spawn)");
        // ★진입 즉시 승계 취소(FIX-1)★: read_live 전에 옛 세대를 취소·stale 화(connect 와 동형).
        let my_gen = self
            .lifecycle
            .bump_and_capture(Some(ConnectionState::Connecting));
        // ★ADR-0021 no-spawn 불변식★: read_live 만 — ensure 는 절대 ensure_spawn 을 부르지 않는다.
        // 데몬이 없으면 여기서 끝(spawn 0회). 복구는 명시 connect() 로만.
        let Some(info) = self.discovery.read_live() else {
            tracing::warn!("ensure 실패 — 살아있는 데몬 없음(no-spawn, connect 로만 복구)");
            // 내가 올린 세대가 current 면 Connecting 을 Down 으로 되돌린다(가드된).
            // ★발화도 함께★: 사유는 `connect()` 의 발견/spawn 실패 분기와 같고, 이 경로는 그보다 더 조용하다 —
            //   attach-only 라 연결 태스크를 아예 띄우지 않으므로 나중에 대신 알려 줄 발화자가 하나도 없다.
            //   복구가 명시 `connect()` 뿐이라(ADR-0021) 화면이 굳으면 사용자가 그것을 누를 계기도 사라진다.
            if self
                .lifecycle
                .publish_if_current(my_gen, ConnectionState::Down)
            {
                self.events.connection_state(ConnectionStateEvent::Down);
            } else {
                tracing::debug!(
                    generation = my_gen,
                    "ensure 실패 — Down 미발행(더 새 연결이 current)"
                );
            }
            return Err(HandshakeError::NoLiveDaemon(
                "daemon.json names no live daemon".to_owned(),
            ));
        };
        self.start_connection(info, my_gen).await
    }

    // 주어진 접속 정보로 연결 task 를 띄우고 Hello 까지 await 한다(connect/ensure 공통 후반부).
    //
    // 연결 task 가 stream 을 단독 소유한다 — 여기선 cmd_tx 끝만 보관하고, 핸드셰이크 완료
    // 신호(oneshot)만 기다린다.
    //
    // ## ★generation 가드(Fix B — 락으로 원자화)★
    // 세대 bump + 캡처는 **호출자(connect/ensure)가 진입 즉시 discovery 전에** 한다(FIX-1) — 그
    // `my_gen` 을 여기로 넘겨받는다. ★이 함수는 더 이상 bump 하지 않는다(이중 bump 회피)★. 동시
    // connect/ensure 가 둘 다 들어오면 각자 진입에서 bump 해 서로 다른 세대를 갖고, 더 새 task 만
    // current 가 된다.
    //
    // ★my_gen 계약★: 호출자가 `bump_and_capture` 로 막 캡처해 넘긴 값이다. 그 bump 와 이 함수 진입
    // 사이에 다른 connect/close 가 또 끼면 내 my_gen 은 이미 stale 일 수 있다 — 그래도 모든 발행이
    // publish_if_current/store_cmd_if_current 가드를 통과하므로 안전하다(stale 이면 그냥 미발행).
    #[cfg(test)]
    async fn start_connection(&self, info: DaemonInfo, my_gen: u64) -> Result<(), HandshakeError> {
        // ★capacity 512★: cmd_tx 는 bounded mpsc 라 full 이면 fire-and-forget(Unsubscribe/Fire/RequestReplay
        //   fire)이 try_send 실패로 조용히 drop 된다. 빠른 layout toggle/drag 가 짧은 시간에 다량의 델타를 쏘면
        //   64 로는 full 이 날 수 있어 저비용 상향으로 여유를 둔다. ★ADR-0046★: reply 있는 request_replay 는
        //   bounded `.send().await`(try_send 아님)라 drop 되지 않고 backpressure 로 대기한다(gen 회수 보장).
        let (cmd_tx, cmd_rx) = mpsc::channel::<ConnectionCommand>(512);
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<Result<(), HandshakeError>>();

        // ★단일 연결 task 소유★: run_connection 이 WebSocketStream 을 split 해 단독 소유하고,
        //   cmd_rx 로 들어오는 명령을 처리한다. 비의도 끊김 시 이 task 안에서 백오프 재연결을
        //   돈다(discovery.read_live no-spawn + rt.spawn_blocking). my_gen + lifecycle 핸들로
        //   stale task 가 공유 상태를 못 건드리게 한다(Fix B + reconnect_guard).
        // ★emit 경로 = seam(ADR-0012)★: `run_connection` 은 `AppHandle` 대신 `events::DaemonEvents` 포트를
        //   받는다. 그래서 **어느 조립이든 연결 task 는 무조건 뜬다** — 알림을 낼 곳이 없는 조립은
        //   `NoDaemonEvents` 로 알림만 버리고 소켓·핸드셰이크·재연결·왕복은 전부 실코드로 돈다.
        //
        //   ## ★여기 있던 단락 이야기(되살리지 말 것)★
        //   2026-08-24 이전 이 자리에는 `app: Option<AppHandle>` 이 `None` 이면 **task 를 안 띄운 채
        //   `Ok(())` 를 돌려주는** 단락이 있었다(커밋 ffcd766, 2026-07-01 선재). 소켓이 안 열려 상태가
        //   `Connecting` 에 고착하는데 호출자는 성공을 받는다 — `tests.rs` 의 connect/ensure 단언
        //   (`assert_eq!(state, Connected)`)이 전부 그 하나에서 깨졌다(`lib_unit` 32건). 운영은 항상
        //   `AppHandle` 을 쥐어 무영향이었지만, `new`/`new_real` 은 `pub` 이고 `#[cfg(test)]` 도 아니라
        //   **미래의 비테스트 호출자도 조용한 no-op 성공을 받을 수 있는** 덫이었다. 그래서 단락을 지우고
        //   emit 을 포트로 끊었다(`events.rs`) — 이제 `None` 상태 자체가 타입에 없다.
        //   ★기록 위치★: docs/process/step-log.md "모듈① T7c Fix-C" 섹션 ② 항목. ADR-0174 는 이 스위트를
        //   **세운 방법**이고, 이 seam 은 거기서 후속으로 남겨 둔 몫이다.
        //   ★수치·CI 등재 상태는 여기 적지 않는다★ — 등재 정본 = `ci.yml` 의 `--test lib_unit` 스텝 · 수치는 실행 결과에서 센다.
        self.rt.spawn(run_connection(
            info,
            my_gen,
            self.lifecycle.clone(),
            self.discovery.clone(),
            self.rt.clone(),
            self.handshake_timeout,
            cmd_rx,
            // ★weak 로 넘긴다 — 강한 clone 이면 이 채널이 영원히 닫히지 않는다★: 연결 태스크의 수명은
            //   「강한 송신단이 전부 사라졌나」로 정해지고(`close()` 도 stale 억제도 그 EOF 하나에 얹혀 있다),
            //   태스크가 자기 채널의 강한 clone 을 쥐면 그 신호가 절대 오지 않는다(`connection::OutcomeSender`).
            cmd_tx.downgrade(),
            ready_tx,
            // ★T6b 출력 평면 주입★: 연결 task 가 frame fan-out 에 쓴다(재연결 task 수명 초월 공유 Arc).
            self.router.clone(),
            self.registry.clone(),
            self.events.clone(),
            self.names.clone(),
            self.refusals.clone(),
            self.inbound.clone(),
            self.usage.clone(),
        ));

        // Hello 수신(=connected) 또는 핸드셰이크 실패를 기다린다. ★락 미보유 await★.
        match ready_rx.await {
            Ok(Ok(())) => {
                // ★가드된 cmd_tx 저장★: ready Ok 를 받았어도, 그 사이 더 새 connect/close 가 세대를
                //   올렸으면 이 연결은 stale 이다 — cmd_tx 를 저장하면 좀비 채널이 된다. store_cmd_if_current
                //   가 "세대 비교 + 저장" 을 원자로 해, current 일 때만 저장한다. stale 이면 저장하지 않고
                //   cmd_tx 가 여기서 drop → 연결 task 의 cmd_rx 가 EOF → 그 task 도 곧 정리된다.
                if self.lifecycle.store_cmd_if_current(my_gen, cmd_tx) {
                    // ★채널을 꽂은 직후 사용량 넛지 한 장(TRD S21 usage-limit-slot §1-7 「새 소켓」)★: 연결
                    //   task 는 창구를 연 직후 관심을 새 소켓에 이미 보냈지만, 여기까지 채널이 비어 있어 그 사이의
                    //   재계산 넛지는 no-op 이었다 — 그 변경은 아무 데도 안 갔다. 대조는 `sent` 기준이라 그 틈에
                    //   든 것이 없으면 이 넛지는 아무것도 안 보낸다.
                    self.nudge_usage_interest();
                } else {
                    // generation 가드 발동: 핸드셰이크 사이 더 새 connect/close 가 세대를 올림 → cmd_tx
                    //   미저장(좀비 채널 차단). 이 caller 입장에선 연결이 밀렸으나 핸드셰이크 자체는 성공.
                    tracing::debug!(
                        generation = my_gen,
                        "stale 연결 — cmd_tx 미저장(더 새 connect/close 가 세대를 올림)"
                    );
                }
                // ★ADR-0046★: 옛 handoff-resync(cmd_tx 저장 직후 재구독+sweep)는 제거됐다 — src-tauri 는
                //   더는 connect 진입 시 eager 재구독을 하지 않는다(진도/구독 상태 무보유).
                Ok(())
            }
            Ok(Err(e)) => {
                // ★재로깅 안 함★: 구체 실패 사유(connect/직렬화/Auth 송신/핸드셰이크)는 run_connection 이
                //   이미 정확한 문구로 warn 을 남겼다(connection.rs). 여기서 또 찍으면 같은 실패가 warn 2줄 +
                //   "reject" 로 오라벨된다 — 단일 출처 유지를 위해 caller 쪽은 무로깅으로 전파만 한다.
                // current 일 때만 Down(stale 이면 더 새 연결의 상태를 clobber 하면 안 됨) — 원자 가드.
                // ★이 분기만 발화를 붙이지 않는다 — 이미 나갔기 때문이다★: 이 `Err` 를 만드는 자리는
                //   `run_connection` 의 핸드셰이크 실패 분기 하나뿐이고(`connection.rs` 의 `ready_tx.send(Err)`),
                //   그 자리는 보고 직후 **같은 my_gen 으로** 가드된 Down 발화를 스스로 낸다. 여기서 또 내면
                //   같은 실패가 창에 `down` 두 장으로 간다 — 프론트의 `setState` 는 같은 값이면 단락하지만
                //   (`src/api/tauriTransport.ts`), 그 중복을 계약으로 박은 회귀망이 있다
                //   (`tests.rs` 의 `failed_handshake_tells_every_window_down`). ★그 테스트가 개수를 세는
                //   시점은 연결 태스크가 발화 포트를 놓은 뒤다★ — 그래서 "down 한 장" 은 먼저 도착한 하나를
                //   본 스냅샷이 아니라 **두 발화자 몫을 합친 총량**이고, 여기에 한 장을 보태면 깨진다.
                //   ★내부 전이는 그대로 둔다★ — 두 태스크의 순서가 뒤집혀 저쪽 가드가 stale 로 삼키는 창이
                //   있고, 그때 watch 를 접는 것은 이쪽 몫이다(발행은 멱등이라 겹쳐도 무해).
                self.lifecycle
                    .publish_if_current(my_gen, ConnectionState::Down);
                Err(e)
            }
            // ready_tx 가 send 없이 drop 됨 = (a) task panic 또는 (b) ★stale self-close★(run_connection
            // 의 generation 가드가 ready 송신을 건너뛰고 빠짐). 둘 다 이 caller 입장에선 핸드셰이크 실패.
            // (a) 는 운영 셸에선 훅이 먼저 셸을 끝내 훅 없는 시험 빌드에서만 닿는다(ADR-0292).
            // stale 한 경우 더 새 연결이 진행 중이므로 여기서 Down 을 쏘면 안 된다 → 원자 가드로 current 만.
            Err(_) => {
                // ★레벨을 stale 여부로 가른다★: publish_if_current 가 true(=current 였는데 ready 없이
                //   task 가 사라짐)면 진짜 이상(훅 없는 빌드면 panic 추정 — 위 (a)) → 사람이 봐야 함(warn).
                //   false(=stale)면 더 새 연결이 세대를 올려 publish_if_current 가 Down 을 삼킨 경우다 —
                //   stale 한 이 task 가 ready 없이 사라진 원인은 run_connection 의 가드 self-close *또는* stale
                //   task 의 panic 둘 다일 수 있으나(둘 다 false 분기로 귀결), 어느 쪽이든 이미 superseded 라
                //   진단용 debug 로 충분하다.
                //   Down 이 stale 이면 삼켜진다(clobber 방지 — connection.rs 의 main_loop 종료 Down 가드와 동형).
                // ★발화가 붙는 쪽은 true 분기뿐이고, 그것이 이 분기의 갈림과 정확히 겹친다★: false(=stale)는
                //   `run_connection` 의 가드 self-close 가 대부분이고 그때 화면의 주인은 더 새 세대다 — 그쪽이
                //   자기 결말을 알린다. true 는 **아무도 알리지 못한 채 태스크가 사라진** 경우다(그 태스크는
                //   `connected` 를 알리기 전에 죽었거나, 알린 직후 `ready_tx` 를 보내기 전에 죽었다). 어느
                //   쪽이든 이 자리가 유일한 발화자라 여기서 내지 않으면 창은 마지막으로 들은 상태에 굳는다.
                if self
                    .lifecycle
                    .publish_if_current(my_gen, ConnectionState::Down)
                {
                    self.events.connection_state(ConnectionStateEvent::Down);
                    tracing::warn!(
                        generation = my_gen,
                        "연결 task 가 ready 신호 전 사라짐(current 연결 — 훅 없는 빌드면 panic 추정)"
                    );
                } else {
                    tracing::debug!(
                        generation = my_gen,
                        "stale task 소멸(ready 전 — self-close 또는 panic, 어느 쪽이든 superseded)"
                    );
                }
                Err(HandshakeError::TaskGone)
            }
        }
    }

    // ★핸들이 떨어지면 연결 태스크도 함께 접는다★ — `close()` 를 안 부르고 버려도 소켓·재연결 루프가
    // 남지 않게.
    //
    // ## 왜 이게 없으면 안 접히나 — 송신단이 태스크 자신에게 걸려 있다
    // 연결 태스크의 즉시 종료 신호는 **`cmd_rx` 의 EOF 하나뿐**이고(`connection::OutcomeSender` doc),
    // 그 EOF 는 **강한 송신단이 전부 사라져야** 온다. 태스크가 받아 가는 송신단은 weak 이라 무해하지만
    // (↑ `start_connection` 의 `downgrade()`), 강한 송신단은 `Lifecycle` 이 쥐고(`store_cmd_if_current`)
    // 그 `Lifecycle` 을 `Arc` 로 **태스크 자신이 함께 쥔다**. 그래서 클라이언트 핸들만 떨어지면 고리가
    // 스스로 닫혀 EOF 가 영영 오지 않는다. ★재연결 소진 상한(MAX_RECONNECT_ATTEMPTS)은 이 고리를 못
    // 끊는다★ — 소켓이 멀쩡한 동안 `main_loop` 는 끊김 자체를 보지 않아 그 상한에 닿지도 않는다.
    //
    // ## ★이 자리가 비어 있어도 무해하던 시절이 있었다(되돌리지 말 것)★
    // 2026-08-24 이전에는 창 없는 조립이 연결 태스크를 **아예 안 띄웠고**(↑ `new` 의 「단락 이야기」),
    // 운영 조립은 앱 수명 내내 살아 drop 되지 않는다. 단락을 지워 **어느 조립이든 태스크가 뜨게 된**
    // 지금은 그 우연한 무해함이 사라졌다 — 이 `Drop` 이 그 자리를 메운다.
    //
    // ## ★운영에서는 이 `Drop` 이 **한 번도 돌지 않는다** — 운영 종료 경로로 읽지 말 것★
    // 운영 조립은 클라이언트를 되쥐는 강한 `Arc` 순환을 만든다: `lib.rs` 의 setup 이 클라이언트를
    // `Arc` 로 세운 뒤 `commands::layout::command_ports` 에 **그 `Arc` 자신**을 넘겨 명령 표를 굽고
    // (`layout::commands::make_table`), 그 표를 `install_command_table` 로 클라이언트 자신의
    // `inbound: Arc<InboundSlot>` 에 꽂는다. 그래서 운영 클라이언트의 강한 참조수는 0으로 내려가지 않고
    // 이 함수는 불리지 않는다 — 프로세스 수명과 같이 사는 싱글턴이 된다는 뜻이고, 오늘 이 앱에서는
    // 그것이 결함이라기보다 **의도된 수명**에 가깝다(연결 핸들은 앱이 살아 있는 동안 계속 필요하다).
    // ★따라서 이 `Drop` 은 운영 teardown 이 아니라 **비운영 조립의 안전망**이다★ — 하네스나 앞으로 생길
    // 짧은 수명 조립이 `close()` 없이 핸들을 버려도 소켓·재연결 루프가 남지 않게 하는 것이 전부다.
    // ★순환을 `Weak` 로 끊는 안은 여기서 하지 않는다★ — 살아 있는 명령 배달 경로의 배선을 갈아 끼우는
    // 별건이고, 그 판단(싱글턴 수명을 유지할지)이 선행한다. ★끊는다면 아래 ③을 함께 재판정한다★ —
    // 그 항은 이 순환에 얹혀 있어서, 순환이 사라지는 날 조용히 거짓이 된다.
    //
    // ## ★여기서는 발화하지 않는다 — 명시 `close()` 와의 이 비대칭은 의도다(맞추지 말 것)★
    // 사유 셋이 겹친다.
    // ① **전이가 일어났는지 모른다**: 이 경로가 부르는 `Lifecycle::close_best_effort` 는 락이 오염되면
    //    아무것도 하지 않고 조용히 빠진다(그래서 세대도 안 돌려준다). 그 위에 무조건 발화를 얹으면 *안
    //    일어난* 전이를 화면에 알린다.
    // ② **소멸자는 패닉하면 안 된다**: 그 함수의 계약이 「절대 패닉하지 않는다」인데, 발화는 우리가 구현을
    //    고르지 않는 포트(`events::DaemonEvents`) 너머의 외부 호출이다. unwinding 중이면 그 패닉이 프로세스를
    //    abort 시킨다 — 소멸자에서 살 값이 아니다.
    // ③ **알릴 상대가 없다** — ★이건 일반 성질이 아니라 ↑ `Arc` 순환의 **결과**다★: 그 순환이 클라이언트를
    //    teardown 까지 살려 두는 덕에 운영에서 이 `Drop` 이 한 번도 돌지 않고, 실제로 도는 조립은 창이 없는
    //    하네스뿐이다. 창이 있는 짧은 수명 조립이 생기더라도 그 다음 조립의 전이가 화면을 다시 채운다.
    //    ★그러니 그 순환을 `Weak` 로 끊는 변경은 이 ③을 함께 뒤집는다★ — 그날부터 이 `Drop` 은 창이 살아
    //    있는 운영 종료에서 돌고, 알릴 상대가 생긴다. 그때 ③을 다시 세우지 않으면 「연결은 끊겼는데
    //    화면만 connected」라는 이 작업이 없애려던 결함이 이 자리로 되돌아온다(①②는 그대로 남는다 —
    //    전이 확정 여부와 소멸자 패닉 금지는 순환과 무관하다).
    // ★명시 `close()` 는 셋 다 해당 없다★ — 전이가 확정이고, 소멸자가 아니며, 알릴 창이 살아 있다.
    #[cfg(test)]
    fn close_on_drop(&self) {
        self.lifecycle.close_best_effort();
    }

    /// Explicit close (wsTransport `close()`) — the link does not reconnect. Returns at once; every window learns `down`
    /// through routing. Contract: `LinkClient::close`.
    pub fn close(&self) {
        #[cfg(test)]
        if self.link.is_none() {
            self.close_on_old_driver();
            return;
        }
        self.link().close();
    }

    // The old driver's path — its test constructors only (gone with 3-2 U4b).
    // 명시 종료(wsTransport `close()` 대응). 연결 task 에 종료를 알리고 Down 으로 전이한다.
    //
    // ★재연결 금지는 T4★: T2 는 명시 close 만. closedByUser 가드(명령/재연결이 respawn 안 하게)는
    // 백오프 재연결과 함께 T4 가 채운다.
    //
    // ## ★명시 종료도 전 webview 에 알린다 — 부른 창 하나만 아는 것으로는 모자란다★
    // 이 연결은 **창이 몇 개든 하나**다(ADR-0036). 그래서 한 창이 닫으면 **모든 창의 연결이 끊긴다**.
    // 그런데 그 사실을 스스로 아는 것은 부른 창뿐이다 — `src/api/tauriTransport.ts` 의 `close()` 는
    // 리스너를 걷은(`cleanupListeners()`) **뒤에** 이 커맨드를 invoke 하고, 자기 `_state` 는 그 invoke 를
    // 띄운 **뒤에** `'down'` 으로 세운다. 나머지
    // 창은 마지막으로 들은 발화(`connected`)에 굳은 채 죽은 연결에 출력 채널을 걸고 앉는다. 프론트의
    // 연결 상태 자가복구는 리로드 시 pull 1회뿐이라(같은 파일 `selfHeal`) 살아 있는 창은 영영 못 깨어나고,
    // 복구 경로는 명시 `connect()` 뿐인데(ADR-0021) 화면이 연결됨으로 보이면 그것을 누를 계기도 없다.
    // ★연결 태스크가 대신 알려 주지 못한다★: 그 태스크는 `cmd_rx` EOF 로 종료하며 자기 Down 을 내려
    // 하지만, 이 함수가 **이미 세대를 올렸으므로** 그 발행은 stale 로 삼켜진다(`connection.rs` 의
    // `LoopExit::Closed` 분기) — 발화자가 아무도 남지 않는다. 그래서 여기가 유일한 자리다.
    //
    // ★발화가 락 밖인 것이 요점이다★(ADR-0006 — 락 보유 중 외부 호출 금지): `lifecycle.close()` 는 자기
    // 락을 잡았다 **반환하면서 풀고**, 세운 세대를 돌려준다. 발화는 그 뒤다. 그리고 그 세대로 한 번 더
    // 가드한다 — 반환~발화 사이에 승계 connect 가 끼면 화면의 주인은 더 새 세대이고, 뒤늦은 `down` 은 그
    // 주인의 `connected` 를 덮는다. 그때는 미발화가 옳다(그 세대가 자기 결말을 스스로 알린다).
    // ★그 재확인은 `is_current` 이지 `publish_if_current` 가 아니다★ — watch 의 `Down` 은 `lifecycle.close()`
    // 가 자기 락 안에서 **이미 실었다**. 여기서 또 발행하면 한 번의 close 가 구독자에게 변경 두 번으로
    // 보인다. 이 자리에 남은 물음은 「발화할까」뿐이다(사유 정본 = `lifecycle::Lifecycle::is_current`).
    // ★이중 보고 아님★: 부른 창은 `daemon_close` 를 invoke 하기 **전에** 자기 control 핸들러를 뗀다
    // (`src/api/tauriTransport.ts` `close()` → `cleanupListeners()`, invoke 보다 앞). 떼는 일의 웹뷰 쪽
    // 절반은 동기라(`@tauri-apps/api` 의 `_unlisten` 이 자기 await 보다 **먼저** JS 핸들러를 지운다) 그
    // 발화는 대개 닿지도 않는다. 그 창의 `_state='down'` 은 invoke 를 **띄운 뒤에** 세워지지만 그 순서는
    // 무해하다 — 닿더라도 같은 값이라 프론트 `setState` 가 단락한다. 그러니 이 발화가 처음부터 노리는
    // 것은 **나머지 창**이다.
    //
    // ## ★가드와 발화는 원자가 아니다 — 여기서 고치지 않는다(구조적 잔여)★
    // `is_current` 가 true 를 돌려준 뒤 아래 발화가 실행되기 전에, 세대 g+1 의 승계 `connect()` 가 끝까지
    // 달려 `connected` 를 먼저 낼 수 있다. 그러면 뒤늦은 이 `down` 이 그 화면을 덮는다 — 이 가드가 없애려던
    // 바로 그 낡은 화면이다. ★가드를 원자성으로 읽지 말 것★.
    // ★새로운 종류는 아니다★: `connection.rs` 의 가드된 발화 자리들이 같은 모양을 이미 갖는다. 다만
    // close→connect 는 사용자가 늘 하는 차례라 이 자리가 그중 가장 노출돼 있다.
    // ★국소 땜질로 닫히지 않는다★: 락 안에서 발화하면 ADR-0006 을 깬다. 순서를 세우려면 **락 아래서 밀어
    // 넣고 단일 소비자가 빼는 발화 큐**를 두거나, 발화에 세대를 실어 프론트가 정렬하게 해야 한다 — 둘 다
    // 이 자리 하나로 끝나지 않으므로 별건이다.
    #[cfg(test)]
    fn close_on_old_driver(&self) {
        tracing::info!("데몬 연결 명시 종료(close)");
        let my_gen = self.lifecycle.close();
        if self.lifecycle.is_current(my_gen) {
            self.events.connection_state(ConnectionStateEvent::Down);
        } else {
            tracing::debug!(
                generation = my_gen,
                "명시 종료 — Down 미발화(더 새 연결이 current · watch 전이는 이미 실렸다)"
            );
        }
    }

    // ── T6a: invoke 명령 request/reply 평면(spawn/kill/interrupt/write/resize/…) ─────────
    // side-effect 명령을 연결 task 로 보내고 데몬 reply(request_id 매칭)를 await 한다.
    /// [`Self::send_command_with_origin`] 의 실패를 글로만 돌려주는 형제 — 계약·흐름·잔여는 그쪽에 적는다.
    pub async fn send_command(&self, cmd: AgentCommand) -> Result<AgentEvent, String> {
        self.send_command_with_origin(cmd)
            .await
            .map_err(|failure| failure.to_string())
    }

    /// [`Self::send_command`] 와 같은 왕복이되 실패의 **출처**를 타입으로 남긴다 — 데몬이 `Error` 로 답했나
    /// (`CommandFailure::Daemon`), 셸 쪽에서 끝났나(`CommandFailure::Local`)를 문구를 읽지 않고 가려야 하는
    /// 호출자가 쓴다(ADR-0281). 글은 형제와 바이트 단위로 같다(`CommandFailure` 의 `Display`).
    /// 이 함수 안의 실패(request_id 없음 · 미연결 · 채널 끊김 · 답 유실)는 전부 `Local` 이다.
    ///
    /// ★호출 전제 — `cmd` 에 request_id 를 호출자가 이미 박는다★(commands/agent.rs 의 빌더가
    /// `RequestId::new()` 로 채운다). 그래야 reply 매칭 키가 호출자에게도 알려져 idempotency(재시도 시 같은
    /// 키)와 정합한다 — 이 왕복이 임의로 채우면 호출자가 키를 모른다.
    pub async fn send_command_with_origin(
        &self,
        cmd: AgentCommand,
    ) -> Result<AgentEvent, protocol_state::CommandFailure> {
        #[cfg(test)]
        if self.link.is_none() {
            return self.send_command_on_old_driver(cmd).await;
        }
        self.link().send_command(cmd).await
    }

    // The old driver's path — its test constructors only (gone with 3-2 U4b).
    // ★흐름★: (1) 현재 cmd_tx clone + 소켓 표식(없으면 not-connected Err) (2) oneshot 생성
    // (3) `SendCommand` enqueue (4) reply await. 연결 task 가 reply 를 resolve(Ok/Err)하거나, 끊김 시
    // drain 으로 Err 를 보낸다(no-hang). cmd_tx send 실패(채널 full/닫힘)·oneshot drop(연결 task
    // 사망)도 Err 로 귀결.
    //
    // ★ADR-0195 — 끊긴 동안에는 명령을 받지 않는다★: 아래 `current_cmd_tx()` 가 `None` 이면 담아 두지
    // 않고 그 자리에서 [`NOT_CONNECTED`] 로 돌려준다. 「아직 첫 연결 중」도 같게 본다 — 그 구분은
    // 프론트가 이미 자기 층에서 진다(`ProtocolClient.sendCommand` 가 매 명령 전에 `ensureReady()` 를
    // 기다린다).
    // ★두 not-connected 문구가 갈리는 자리★: 「살아 있는 소켓이 없다」는 **전부** [`NOT_CONNECTED`] 로
    // 떨어진다(창구가 닫혀 있으므로 — 재연결 중·소진·종료 모두). 둘째 문구([`CHANNEL_GONE`])는 창구가
    // 열린 것을 보고 clone 을 집어 간 **뒤에** 수신단이 사라진 경우다.
    // ★둘째가 「좁은 경합」이라고만 읽지 말 것★: `main_loop` 가 패닉하면 창구를 닫는 자리를 아무도 지나지
    // 않아 `cmd_tx` 도 `Connected` 도 그대로 남는다 — 그 뒤 **모든** 명령이 창구를 통과한 뒤 죽은 수신단에
    // 부딪혀 둘째 문구를 영구히 받는다. 결말 자체는 옳다(즉시 실패 · hang 없음)므로 여기서 고치지 않는다.
    // 운영 셸에선 그 패닉을 훅이 셸 종료로 만들어 이 모양은 훅 없는 시험 빌드에서만 선다(ADR-0292).
    //
    // ★ADR-0006(락 across await 금지)★: `current_cmd_tx()` 는 락을 잡았다 즉시 풀고 Sender clone +
    // 표식만 돌려준다 — 이후 `tx.send().await`·`rx.await` 는 락 미보유 상태다(Sender 는 lifecycle 락과
    // 독립).
    //
    // ## Known residual — this clone lives through `reply_rx.await` and blunts `close()`
    // The connection task ends only when every strong sender is gone (`connection::OutcomeSender`), and `cmd_tx` below
    //   stays strong while the reply is awaited. A daemon that keeps the socket open and never answers therefore keeps
    //   the caller waiting and the old `main_loop` and socket alive past `close()` — this path has no reply bound. Not
    //   fixed here: the link driver bounds every round trip with the policy's request timeout (ADR-0181). The window
    //   check and its texts follow ADR-0195; the per-call deadline idea is the ADR-0088 family.
    #[cfg(test)]
    async fn send_command_on_old_driver(
        &self,
        cmd: AgentCommand,
    ) -> Result<AgentEvent, protocol_state::CommandFailure> {
        use protocol_state::CommandFailure::Local;

        if protocol_state::command_request_id(&cmd).is_none() {
            return Err(Local(NO_REQUEST_ID.to_string()));
        }
        // 지금 명령을 받을 소켓이 있으면 그 채널과 표식을 함께 얻는다(없으면 연결 안 됨/끊김).
        let Some((cmd_tx, socket)) = self.lifecycle.current_cmd_tx() else {
            return Err(Local(NOT_CONNECTED.to_string()));
        };
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        // 연결 task 로 enqueue. send 실패 = 채널 닫힘(연결 task 종료) → not-connected 취급.
        if cmd_tx
            .send(ConnectionCommand::SendCommand {
                cmd,
                reply: reply_tx,
                socket,
            })
            .await
            .is_err()
        {
            return Err(Local(CHANNEL_GONE.to_string()));
        }
        // reply 대기. 연결 task 가 resolve(Ok/Err) 하거나 끊김 drain 으로 Err. oneshot 송신단이 reply
        //   없이 drop(연결 task 사망 등) 되면 RecvError → not-connected 취급.
        match reply_rx.await {
            Ok(slot) => protocol_state::classify_reply(slot),
            Err(_) => Err(Local(REPLY_LOST.to_string())),
        }
    }

    // ── T6b: fire-and-forget 평면(Subscribe/Unsubscribe/Resize — reply 없음) ─────────────
    /// Writes `Unsubscribe` on the open connection, behind earlier unsubscribes and replay requests; with none open it
    /// does nothing — the daemon drops a connection's subscriptions with it. Never blocks (the layout calls it under its
    /// lock). Contract: `LinkClient::unsubscribe`.
    pub fn unsubscribe(&self, agent_id: AgentId) {
        #[cfg(test)]
        if self.link.is_none() {
            self.unsubscribe_on_old_driver(agent_id);
            return;
        }
        self.link().unsubscribe(agent_id);
    }

    // The old driver's path — its test constructors only (gone with 3-2 U4b).
    // 출력 구독 enqueue(fire-and-forget). 연결 task 가 SubState 로 epoch/after_seq 를 채워 wire 송신한다.
    //
    // ★동기 + try_send★: layout command 는 `#[tauri::command] pub fn`(동기)이라 `async send` 를 못 한다.
    // cmd_tx 는 bounded(512) mpsc 라 `try_send` 로 넣는다 — 해제는 저빈도(레이아웃 변경 시에만)라
    // full 은 사실상 안 난다.
    //
    // ★창구가 닫혀 있으면 조용히 no-op 이고, **그래도 되는 이유는 이 명령에만 해당한다**★: 데몬의 구독은
    // 연결마다 따로 서고 그 연결이 죽을 때 함께 정리된다(`daemon::connection_core` 의 per-conn `subs`).
    // 그래서 소켓이 없는 동안의 해제는 보낼 필요가 **없다** — 이미 끝난 일이다. ★이 문장을 형제
    // [`Self::send_fire_and_forget`] 로 넓히지 말 것★ — 그쪽은 사정이 다르고, 그 doc 이 그것을 적는다.
    #[cfg(test)]
    fn unsubscribe_on_old_driver(&self, agent_id: AgentId) {
        self.try_enqueue(
            |socket| ConnectionCommand::Unsubscribe { agent_id, socket },
            "unsubscribe",
        );
    }

    /// A command without a reply (Resize …) — `agent_resize` uses it. Never blocks; with no connection to take it the
    /// command is dropped (ADR-0195) and the frontend re-issues a lost Resize. Contract:
    /// `LinkClient::send_fire_and_forget`.
    pub fn send_fire_and_forget(&self, cmd: AgentCommand) {
        #[cfg(test)]
        if self.link.is_none() {
            self.send_fire_and_forget_on_old_driver(cmd);
            return;
        }
        self.link().send_fire_and_forget(cmd);
    }

    // The old driver's path — its test constructors only (gone with 3-2 U4b).
    //
    // reply 없는 명령(Resize 등) enqueue(fire-and-forget). agent_resize invoke 가 쓴다.
    //
    // ## ★끊긴 동안의 Resize 는 여기서 유실되고, 되돌리는 것은 프론트의 재발행이다(ADR-0195)★
    // 형제 [`Self::unsubscribe`] 와 달리 이쪽은 창구가 닫혔다고 「의미 없는」 명령이 되지 않는다:
    // 에이전트는 데몬에서 멀쩡히 살아 있고 재연결이 그것을 다시 붙잡는다. 재동기가 없다는 사실 자체는
    // invoke 경계가 이미 적고 있다(`commands::agent::agent_resize` doc) — 여기서는 **그래서 무엇이
    // 유실되고 누가 다시 내나**만 적는다.
    // ★재구동자는 다섯이고, 그중 넷이 *화면 쪽 사건*이다★(`src/components/slot/TerminalSlot.tsx` 의
    // `resizePty` 호출부): 컨테이너 크기 변화(ResizeObserver 디바운스) · **숨김→보임 전이**
    // (IntersectionObserver 가 WebGL 을 다시 붙일 때) · 구독 성립 직후 · 화신 리셋 콜백. ★다섯째는
    // **네트워크 사건**이다★ — 연결 상태가 connected 로 *전이*하면 그 슬롯이 지금 치수를 한 번 다시 낸다.
    // ★그 다섯째를 구독 effect 라고 읽지 말 것★: 구독 deps 는 여전히 `[viewId, agentId]` 라 재연결로
    // 다시 돌지 않는다 — 거기에 연결 상태를 섞는 것이 ADR-0164 가 BLOCK 한 안이다. 재발행은 구독을
    // 건드리지 않는 별도 effect 이고, 그래서 화면을 지우지도 화신 표식을 잃지도 않는다.
    // ★그래도 여기서 담아 두는 것으로 고치지 않는다★ — 그것이 ADR-0195 가 거부한 바로 그 안이고,
    // 프론트 재발행이 선 지금도 그대로 거부다.
    // ★남은 갭★: 그 재발행은 **보이고 박스가 선 슬롯**만 낸다(숨은 탭이나 붕괴한 박스에서 내면 생성
    // 기본값 80×24 나 최소 치수가 그대로 PTY 로 간다 — 고치려는 그 값이다). 그런 슬롯은 다시 보이거나
    // 크기가 잡히는 순간 위 넷 중 하나가 발화해 고친다.
    #[cfg(test)]
    fn send_fire_and_forget_on_old_driver(&self, cmd: AgentCommand) {
        self.try_enqueue(|socket| ConnectionCommand::Fire { cmd, socket }, "fire");
    }

    /// 사용량 관심이 바뀌었을 수 있다 — 연결 task 에 **집합 없는** 넛지 한 장(TRD S21 usage-limit-slot §1-7
    /// 「한 직렬 경로」). 보낼 집합은 연결 task 가 꺼낼 때 [`UsageInterest::sync`] 로 **그때의** 관심을 읽는다 —
    /// 그래서 넛지가 몇 장이든 어떤 순서로 처리되든 소켓의 마지막 한 장이 최신 관심이다. 집합을 여기서 읽어
    /// 실어 보내면 두 변경이 읽기와 전송 사이에서 엇갈릴 때 옛 집합이 마지막으로 닿는다 — 되살리지 말 것
    /// (TRD §3 #74).
    ///
    /// 비블로킹이라 [`SharedUsageInterest`] 락 안에서 불러도 된다(락 차례는 그 doc).
    /// With no connection open it does nothing — the next one gets the whole interest when it opens
    /// (`LinkClient::nudge_usage`).
    pub fn nudge_usage_interest(&self) {
        #[cfg(test)]
        if self.link.is_none() {
            nudge_usage_on(&self.lifecycle);
            return;
        }
        self.link().nudge_usage();
    }

    /// 레이아웃이 바뀌었다 — 사용량 관심을 다시 계산하고 그 결과대로 넛지하거나 줄임 타이머를 띄운다(TRD S21
    /// usage-limit-slot §1-7 「재계산 자리」). 레이아웃 쓰기의 구독 재동기(`layout::SubscriptionSync`)가 부른다.
    ///
    /// ★ViewManager 락 안에서 부른다★ — 락 차례는 [`SharedUsageInterest`] doc 그대로이고, 하는 일은 메모리 안
    /// 계산 · 비블로킹 넛지 · 타이머 태스크 기동뿐이라 그 포트의 「락 안 · 동기 · 비블로킹」 계약을 지킨다. 이 셋
    /// 형제([`Self::usage_window_destroyed`] · [`Self::usage_window_visibility`])도 같다.
    pub fn usage_layout_changed(&self, mgr: &ViewManager) {
        let action = self.usage.lock().recompute(mgr, Instant::now());
        self.run_usage_action(action);
    }

    /// 팝업 창 `label` 이 소멸했다 — 모델에서 창을 지운 같은 ViewManager 락 안에서 부른다. 모델이 그 창을
    /// 쥐고 있었는지와 무관하게 그 label 의 숨김 표시를 잊는다([`UsageInterest::forget_window`]).
    pub fn usage_window_destroyed(&self, label: &str, mgr: &ViewManager) {
        let action = self.usage.lock().forget_window(label, mgr, Instant::now());
        self.run_usage_action(action);
    }

    /// 셸이 창 `label` 을 숨겼거나(`false`) 다시 보였다(`true`) — 트레이 보이기·숨기기가 창마다 ViewManager 락을
    /// 짧게 잡고 부른다(OS 창 호출은 그 락 밖).
    pub fn usage_window_visibility(&self, label: &str, visible: bool, mgr: &ViewManager) {
        let action = self
            .usage
            .lock()
            .set_visible(label, visible, mgr, Instant::now());
        self.run_usage_action(action);
    }

    // 재계산 결과를 처리하는 한 자리 — 사용량 관심 락은 이미 놓았다(넛지는 집합을 싣지 않아 락 밖이어도 순서가
    // 안 엇갈린다 — [`Self::nudge_usage_interest`]).
    fn run_usage_action(&self, action: InterestAction) {
        match action {
            InterestAction::Unchanged => {}
            InterestAction::Nudge => self.nudge_usage_interest(),
            InterestAction::Defer { gen, deadline } => {
                // 기한까지의 잠을 **부른 자리에서** 이 런타임의 시계로 짓는다 — 태스크 안에서 지으면 그 첫 poll
                //   만큼 기한이 밀리고, 가짜 시계 시험에서는 시계를 먼저 돌린 뒤 지어져 기한이 통째로 밀린다.
                let delay = deadline.saturating_duration_since(Instant::now());
                let sleep = {
                    let _rt = self.rt.enter();
                    tokio::time::sleep(delay)
                };
                tracing::debug!(
                    ?gen,
                    delay_ms = delay.as_millis() as u64,
                    "사용량 관심: 줄임 대기 — 기한에 넛지"
                );
                #[cfg(test)]
                if self.link.is_none() {
                    self.defer_usage_nudge_on_old_driver(gen, sleep);
                    return;
                }
                // Holds the shared interest and a nudger that does not keep the actor alive — not the client (a nudge
                //   after the client is gone does nothing).
                let usage = self.usage.clone();
                let nudger = self.link().usage_nudger();
                self.rt.spawn(async move {
                    sleep.await;
                    if !usage.lock().deferral_elapsed(gen) {
                        tracing::debug!(
                            ?gen,
                            "사용량 관심: 줄임 기한 — 그 사이 무르거나 새 세대가 섰다, 넛지 없음"
                        );
                        return;
                    }
                    tracing::debug!(?gen, "usage interest: shrink deadline — nudge");
                    nudger.nudge();
                });
            }
        }
    }

    // The old driver's deferral task: holds the shared interest and a weak lifecycle — not the client (after a close
    //   the nudge meets a closed window and does nothing).
    #[cfg(test)]
    fn defer_usage_nudge_on_old_driver(
        &self,
        gen: usage_interest::ShrinkGen,
        sleep: tokio::time::Sleep,
    ) {
        let usage = self.usage.clone();
        let lifecycle = Arc::downgrade(&self.lifecycle);
        self.rt.spawn(async move {
            sleep.await;
            if !usage.lock().deferral_elapsed(gen) {
                tracing::debug!(
                    ?gen,
                    "사용량 관심: 줄임 기한 — 그 사이 무르거나 새 세대가 섰다, 넛지 없음"
                );
                return;
            }
            match lifecycle.upgrade() {
                Some(lifecycle) => {
                    tracing::debug!(?gen, "사용량 관심: 줄임 기한 — 넛지");
                    nudge_usage_on(&lifecycle);
                }
                None => tracing::debug!(
                    ?gen,
                    "사용량 관심: 줄임 기한 — 클라이언트가 사라져 넛지 없음"
                ),
            }
        });
    }

    /// View-driven replay (ADR-0046 M1 — single flight): the generation the view fences its flush with. `Err` when no
    /// connection is open, no window shows the agent or its `Subscribe` could not be written. Contract:
    /// `LinkClient::request_replay`.
    pub async fn request_replay(&self, agent_id: AgentId) -> Result<u64, String> {
        #[cfg(test)]
        if self.link.is_none() {
            return self.request_replay_on_old_driver(agent_id).await;
        }
        self.link().request_replay(agent_id).await
    }

    // The old driver's path — its test constructors only (gone with 3-2 U4b).
    // ★뷰 주도 replay 채번(ADR-0046 M1 — single-flight, 반환 gen)★. 뷰가 mount/remount 시 호출하면 연결
    // task 가 single-flight 로 wire `Subscribe{after_seq:None}` 를 보내거나(idle) 다음 1회에 병합(in-flight)
    // 하고, 배정된 `gen`(세대)을 돌려준다. 뷰는 그 gen 이상의 성공 마커에만 flush(gen 펜스). 마커는 연결
    // task 가 ReplayComplete 각인 시 같은 출력 Channel 로 흘린다.
    //
    // ★계약★: 비연결이면 Err(프론트 재요청 구동자는 connected 전이 — M2). 연결 task 에 `RequestReplay` 를
    //   보내고 oneshot 으로 gen 을 회수한다(actor 가 single-flight 상태를 단독 소유 → 직렬). 그 에이전트를 보이는
    //   창이 없으면 Subscribe 없이 Err(연결 task 의 `RequestReplay` 팔 — 사유 정본 그 주석).
    #[cfg(test)]
    async fn request_replay_on_old_driver(&self, agent_id: AgentId) -> Result<u64, String> {
        let Some((cmd_tx, socket)) = self.lifecycle.current_cmd_tx() else {
            return Err(NOT_CONNECTED.to_string());
        };
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel::<u64>();
        if cmd_tx
            .send(ConnectionCommand::RequestReplay {
                agent_id,
                reply: Some(reply_tx),
                socket,
            })
            .await
            .is_err()
        {
            return Err("연결 task 가 replay 요청을 받지 못함(끊김)".to_string());
        }
        // ★send 실패 → Err 회수★. 연결 task 가 send_now Subscribe 를 wire 로 못 보내면(소켓 죽음) 그 자리에서
        //   **연결을 끊고** 이 reply oneshot 을 send 없이 drop 한다 — 따라서 여기 `reply_rx.await` 가
        //   RecvError → Err 로 귀결한다(gen 을 잘못 반환하지 않음). 정상 경로는 actor 가 `reply.send(gen)` 으로
        //   gen 을 넣어 Ok(gen).
        // ★이 Err 경로를 실제로 태우는 테스트는 없다(정직 표기)★: 끊는 결정은 `main_loop` 의 select 팔
        //   안이고, 그 자리를 태우려면 **송신이 실패하는 실 소켓**이 있어야 한다 — 루프백 WS 하네스
        //   (`tests.rs`)는 그 select 팔 자체는 돌리지만 송신 실패를 만들어 내지는 않는다. 코어의
        //   `send_failure_path_clears_via_disconnect_and_next_request_sends` 는 **`on_disconnect` 가
        //   슬롯을 비우고 다음 요청이 다시 나간다**만 재고, 송신 실패에서 그 `on_disconnect` 까지 가는
        //   구간은 안 탄다.
        // ★ADR-0195 도 같은 자리로 떨어진다★: 옛 소켓 몫으로 판정된 요청은 연결 task 가 이 oneshot 을
        //   그냥 drop 한다 — 나를 통로는 `u64` 뿐이라 실어 보낼 오류값이 없다. 그래서 아래 문구가 그
        //   경로의 **유일한** 관측이고, 「미전송 · 재요청 안전」이라 그 경우에도 정직하다. 보이는 창이 없는 요청도
        //   같은 drop 으로 온다.
        reply_rx.await.map_err(|_| REPLAY_UNSENT.to_string())
    }

    #[cfg(test)]
    fn try_enqueue(&self, make: impl FnOnce(u64) -> ConnectionCommand, kind: &str) {
        try_enqueue_on(&self.lifecycle, make, kind);
    }

    // ── 재연결·백오프·generation 가드·closedByUser ──────────────────────────────────
    // 비의도 끊김(데몬 stream 종료/오류/Close frame) 시 연결 task(connection.rs `connected_lifetime`)가
    // **그 task 안에서** attach-only 재연결을 돈다 — read_live(no-spawn) + 지수 백오프(500ms→10s MAX5) →
    // 성공 시 Connected 재전이, 소진 시 Down. close()(closed_by_user)·새 connect(세대 bump)는
    // reconnect_guard(lifecycle.rs)로 Stop → 재연결 즉시 중단(좀비/respawn 차단). 백오프 sleep 은
    // tokio::time::sleep 이라 테스트가 time::pause/advance 로 결정론 검증(ADR-0038).
    //   ★ADR-0046: eager resubscribe 삭제★ — connected 재전이 시 src-tauri 가 subs 를 순회해 wire
    //   Subscribe 를 재발행하던 배선(구 resubscribe_params)은 제거됐다. 재요청 구동자 = 프론트 단독
    //   (connected 전이에서 뷰 buffering 리셋 + request_replay). 라우터는 Unsubscribe(prune)만 wire 로 낸다.

    // ── protocol_state 순수 결정 함수(epoch decide·pending 매칭) ─────────────────────────
    // `protocol_state` 모듈이 SubState(epoch)·PendingMap·결정 함수(decide_epoch·apply_subscribe_ack·
    // take_pending·drain_pending)를 순수하게 소유한다(소켓·runtime 의존 0). 출력 진도(seq/cursor/버퍼)는
    // 없다 — ADR-0046 이후 진도 거처는 웹뷰 뷰 단위(프론트 lastDeliveredSeq) 단독이고, src-tauri 상태는
    // 요청 부기(epoch·single-flight replay_flight)뿐이다. binary frame 라우팅은 connection.rs 가 OutputRouter
    // (targets∩registered 창 Channel)로, replay 경계 마커 합성은 replay_flight 상태기계가 담당한다.
}

// fire-and-forget enqueue 공통(동기 try_send). 비연결=no-op, full/닫힘=debug 로깅. 클라이언트 없이 lifecycle 만
// 받는 것은 줄임 타이머 태스크가 클라이언트를 붙들지 않고 넛지하기 위해서다(`DaemonClient::run_usage_action`).
//
// ★명령을 **만들어 주는 클로저**를 받는다(ADR-0195)★: 소켓 표식은 창구를 여는 그 한 번의 락
// 조회에서만 나오므로, 완성된 명령을 받으면 그 칸을 채울 값이 없다.
#[cfg(test)]
fn try_enqueue_on(lifecycle: &Lifecycle, make: impl FnOnce(u64) -> ConnectionCommand, kind: &str) {
    let Some((cmd_tx, socket)) = lifecycle.current_cmd_tx() else {
        // 창구가 닫힘 — 조용히 no-op(ADR-0046: src-tauri 무상태). ★여기서 담아 두지 않는 것이
        //   ADR-0195 이고, 그 대가는 명령마다 다르다★ — 무해한 쪽과 유실되는 쪽의 판정은 각 진입점
        //   doc 에 있다([`DaemonClient::unsubscribe`] · [`DaemonClient::send_fire_and_forget`] ·
        //   [`DaemonClient::nudge_usage_interest`]).
        tracing::debug!(%kind, "fire-and-forget: 창구 닫힘 — no-op");
        return;
    };
    if let Err(e) = cmd_tx.try_send(make(socket)) {
        tracing::debug!(%kind, "fire-and-forget enqueue 실패(full/닫힘): {e}");
    }
}

// The old driver's nudge. ★A closed window makes it a no-op, and that is fine★: a new socket sends the whole interest
//   when it opens, and the gap on a first connect or a takeover — window open, channel not plugged yet — is covered by
//   the nudge right after the channel is plugged (`start_connection`). A nudge dropped on a full command channel is
//   fixed by that connection's next snapshot check — a connection with no subscription has nothing to check, ⟳ fixes
//   it (TRD §6 #25).
#[cfg(test)]
fn nudge_usage_on(lifecycle: &Lifecycle) {
    try_enqueue_on(
        lifecycle,
        |socket| ConnectionCommand::UsageInterest { socket },
        "usage-interest",
    );
}

// 연결 태스크를 접는 사유·불변식은 [`DaemonClient::close_on_drop`] 이 단독 소유한다 — 여기 다시 적지 않는다.
impl Drop for DaemonClient {
    fn drop(&mut self) {
        // The link driver ends with its `LinkClient` (dropped with the fields below).
        #[cfg(test)]
        self.close_on_drop();
        // ★전용 런타임은 여기서 **비블로킹으로** 접는다★ — 이 함수 본문이 끝나면 필드들이 이어서 drop
        //   되고, 그중 [`Self::_owned_rt`] 는 tokio `Runtime` 이다. `Runtime` 의 기본 drop 은 워커가 멈출
        //   때까지 블로킹하는데, **async 컨텍스트 안에서 그 drop 이 일어나면 tokio 가 패닉한다**. `Drop`
        //   안의 패닉은 unwinding 중이면 프로세스를 abort 시킨다 — 진단 없이 죽는 결말이다.
        //   `shutdown_background` 는 워커를 detach 해 즉시 돌아오므로 어느 컨텍스트에서든 같은 값을 낸다.
        //   소멸자가 원하는 것이 정확히 그것이다(기다리지 않음 + 패닉 없음).
        // ★"오늘은 못 닿는다"에 이걸 걸지 않는다★: 지금 이 자리가 그 조합에 닿을 길은 없다 — 테스트
        //   생성자는 전부 `_owned_rt: None` 이고, 운영 클라이언트는 ↑ `close_on_drop` 이 적은 `Arc` 순환
        //   때문에 애초에 drop 되지 않는다. 그러나 그 무해함은 **우연이지 구조가 아니다**(런타임을 소유하는
        //   생성자가 하나만 늘거나, 순환이 끊기면 그날로 사라진다). 그래서 구조로 막아 둔다.
        // ★마지막 참조일 때만 접는다★ — `Arc` 가 남아 있으면 다른 소유자가 그 런타임을 아직 쓰는 중이다.
        if let Some(rt) = self._owned_rt.take() {
            if let Ok(rt) = Arc::try_unwrap(rt) {
                rt.shutdown_background();
            }
        }
    }
}

#[cfg(test)]
mod tests;
