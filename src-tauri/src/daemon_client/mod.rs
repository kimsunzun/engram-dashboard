//! DaemonClient — the shell's single authority over the daemon WS connection (S14 module ①, ADR-0036).
//!
//! **One connection to the daemon however many windows** — the frontend reaches the daemon through the shell, not
//! with a WS per window (`src/api/wsTransport.ts`). This module owns that connection's setup, handshake and life.
//! Protocol semantics (epoch guard · request id extraction · reply classification) are `protocol_state.rs`, output
//! routing is `output_router` / `output_channel`.
//!
//! - The connection runs on the driver in `connection.rs` over the transport crate (`link/`) — its module doc holds
//!   the concurrency model (one actor owns the routing; callers' round trips run on their own tasks).
//! - `connect` (explicit, may spawn the daemon) and `ensure` (attach-only, never spawns) stay apart — ADR-0021.
//! - States: connected / down / reconnecting — there is no connecting state.

// ADR-0310: the driver on the transport crate (TRD S21 trd-3 §6-2 U3) — every client runs on it.
mod connection;
// ADR-0012: the seam for the connection's emits to the frontend (real AppHandle / recording fake / no-op). Without it
// this module's unit harness cannot stand — the reasons and contract live in that file's header.
// ★Being `pub(crate)` is the point★: its consumers are the connection drivers inside this crate; opening it would
//   leak the `AppHandle` carrier onto the lib surface with nowhere outside the crate to plug it in.
pub(crate) mod events;
// ADR-0231: 출력 binary frame 한 장의 중계 판정(디코드 · 화신 표식 거름 · 보는 창) — seq 를 건너뛰지 않는다.
mod frame_relay;
// ADR-0155 결정 4: 데몬이 보낸 명령을 받는 입구. 적용은 연결 태스크 밖에서 돈다.
pub mod inbound;
// ADR-0310: the daemon link on the transport crate (TRD S21 trd-3 §6-2 U2).
pub mod link;
pub mod protocol_state;
pub(crate) mod refusal;
// ADR-0046 M1: the daemon's replay events applied to the single-flight state — no socket, no Tauri.
pub mod replay_events;
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
use engram_dashboard_protocol::{AgentCommand, AgentEvent, AgentId, AgentInfo};
use engram_dashboard_transport::ws::WsDialer;
use engram_dashboard_transport::SystemClock;
use tokio::runtime::Handle;
use tokio::sync::watch;

pub use connection::HandshakeError;
use events::{DaemonEvents, TauriEmitter};
// 알림을 버리는 조립은 하네스 전용이다(`events::NoDaemonEvents` doc) — 운영 빌드엔 아예 없다.
#[cfg(test)]
use events::NoDaemonEvents;
use inbound::InboundSlot;
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
/// ([`AgentNamesSink`]) holding no lock, so there is no reverse order. Inside this lock only [`UsageInterest`] methods,
/// that nudge and the link driver's usage-subscribe queueing run; ★emits happen after the lock is released★ (ADR-0006). The link
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
/// - ★The connection driver calls it holding no lock★ — the real one takes the layout lock, and inside that lock the
///   usage interest lock is taken ([`SharedUsageInterest`] doc); calling it while holding that lock is the reverse
///   order.
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

/// The endpoint source of [`DaemonClient::unconnected`] — no daemon, and none is ever started.
#[cfg(test)]
struct NoDaemonAnywhere;

#[cfg(test)]
impl link::endpoint::EndpointSource for NoDaemonAnywhere {
    fn ensure(&self, _timeout: Duration) -> Result<link::endpoint::Endpoint, String> {
        Err("this test client never finds a daemon".to_owned())
    }

    fn current(&self) -> link::endpoint::Current {
        link::endpoint::Current::Absent
    }
}

// discover(spawn 가능) timeout 기본값(wsTransport discover_daemon 5s 와 정렬).
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

// TRD S21 trd-3 §5 15 (C10): one ws message or frame on the daemon link is at most 16 MiB — the shell's side; the
//   daemon's accept side takes the same value in stage 3-3. The largest legitimate message is not measured.
const DAEMON_MESSAGE_MAX_BYTES: usize = 16 << 20;

/// The text a caller gets when no connection takes commands right now (ADR-0195) — "never connected",
/// "lost and reconnecting" and "explicitly closed" **all** fall into this one. The text does not tell them apart
/// because the caller's next step is the same for each.
///
/// ★It does not say "connect first"★ — while the link is reconnecting, a caller following that would re-add the
/// link and cut the recovery in progress. This text reaches LLM callers through the control surface (§5), so a
/// wrong next step written here is one they would actually take. ADR-0195 decided that the caller picks when to
/// retry, and the text says only that.
pub(crate) const NOT_CONNECTED: &str =
    "데몬에 연결되어 있지 않음 — 지금은 명령을 받지 않는다(재시도 시점은 호출자가 정한다)";

/// Something that should take the command is gone — the driver's actor stopped while a caller waited for the
/// connection to open (`connection.rs`), or a write found the peer's queue closed (`link::errors::send_error_text`
/// for `SendError::Closed`). A **different fact** from [`NOT_CONNECTED`] (no connection right now).
pub(crate) const CHANNEL_GONE: &str = "연결 task 가 명령을 받지 못함(끊김)";

/// A replay request that wrote no `Subscribe` and got no generation — the link dropped, or no window shows the agent.
pub(crate) const REPLAY_UNSENT: &str =
    "replay 요청 미전송(연결 끊김 · 그 에이전트를 보이는 창 없음) — 프론트 재요청 안전";

/// A round trip asked for a command that takes no reply (no `request_id`) — read by the link's error texts
/// (`link::errors`).
pub(crate) const NO_REQUEST_ID: &str =
    "send_command: request_id 없는 명령은 reply 를 기대할 수 없다";

// The single handle to the daemon connection, shared by invoke handlers, the tray and state subscribers
// (`Arc<DaemonClient>`).
//
// The link driver (`connection`) owns the connection: its actor owns the routing and the transport peer owns the
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
    /// The filter for the command-registration refusal box (ADR-0281). Production = `refusal::TauriRefusalBox`,
    /// harness = `refusal::NoRefusalNotice` or a recording fake ([`Self::new_on_link`]).
    /// ★The client holds one and shares it with the driver — never make one per connection★: repeat suppression lives
    /// as long as the shell process (`RefusalAlerts` doc), so one per connection re-shows the box on every reconnect
    /// that gets the same refusal.
    refusals: Arc<RefusalAlerts>,
    /// The connection state, read without a lock — the driver's watch, which routing publishes. Only borrowed here.
    state_rx: watch::Receiver<ConnectionState>,
    /// The driver's output routing — tests set which windows show which agent here.
    #[cfg(test)]
    router: Arc<OutputRouter>,
    /// The driver's window output channels — tests register a window here.
    #[cfg(test)]
    registry: WindowChannelRegistry,
    /// ★The entry for commands the daemon delivers★ (ADR-0155 decision 4). Filled late — the table's spawn port holds
    ///   this client, so the assembly has a cycle (`inbound::InboundSlot` doc). Shared with the driver.
    inbound: Arc<InboundSlot>,
    /// The usage interest — outlives every connection (the interest comes from the layout, not the socket). Shared
    ///   with the driver.
    usage: SharedUsageInterest,
    /// The driver on the transport crate (`connection`) — connect, ensure, close, the command round trips,
    /// fire-and-forget sends, unsubscribe, replay requests and usage nudges go there; `state_rx` is its watch.
    link: connection::LinkClient,
}

impl DaemonClient {
    /// A client on the link driver over a test network (TRD S21 trd-3 §6-2 U3): `inner` dials the address the endpoint
    /// `source` resolves (a memory network), `clock` drives the transport. The assembly is production's
    /// ([`Self::on_link`]); this client's `router` and `registry` are its own, empty ones. ★The ports go in here★ — the
    /// driver starts with this call and keeps what it got.
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

    /// A client that is never connected — its endpoint source finds no daemon, so `connect` / `ensure` fail and every
    /// command fails as not connected. For tests of callers that do not connect.
    #[cfg(test)]
    pub(crate) fn unconnected(rt: Handle) -> Self {
        Self::new_on_link(
            rt,
            engram_dashboard_transport::testing::MemoryNetwork::new().dialer(),
            Arc::new(NoDaemonAnywhere),
            engram_dashboard_transport::testing::ManualClock::new().handle(),
            Arc::new(NoDaemonEvents),
            Arc::new(NoAgentNames),
            Arc::new(NoRefusalNotice),
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
        Self::around_link(
            rt,
            owned_rt,
            events,
            names,
            refusals,
            router,
            registry,
            |rt, ports| connection::LinkClient::start(rt, inner, source, clock, ports),
        )
    }

    /// The client around the link `start` makes from this client's ports — [`Self::on_link`] starts the driver; a test
    /// may build one it never starts.
    #[allow(clippy::too_many_arguments)]
    fn around_link(
        rt: Handle,
        owned_rt: Option<Arc<tokio::runtime::Runtime>>,
        events: Arc<dyn DaemonEvents>,
        names: Arc<dyn AgentNamesSink>,
        refusals: Arc<RefusalAlerts>,
        router: Arc<OutputRouter>,
        registry: WindowChannelRegistry,
        start: impl FnOnce(&Handle, connection::Ports) -> connection::LinkClient,
    ) -> Self {
        let inbound = Arc::new(InboundSlot::new());
        let usage = SharedUsageInterest::default();
        let link = start(
            &rt,
            connection::Ports {
                events,
                names,
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
            state_rx: link.state_rx(),
            #[cfg(test)]
            router,
            #[cfg(test)]
            registry,
            inbound,
            refusals,
            usage,
            link,
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
                DaemonKey::new(connection::DAEMON),
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

    /// Explicit connect (ADR-0021 §1 — may start a daemon; wsTransport `start()`). Contract: `LinkClient::connect`.
    pub async fn connect(&self) -> Result<(), HandshakeError> {
        self.link.connect().await
    }

    /// Attach-only connect (ADR-0021 B-1 — never starts a daemon; wsTransport `ensureReady()`). Contract:
    /// `LinkClient::ensure`.
    pub async fn ensure(&self) -> Result<(), HandshakeError> {
        self.link.ensure().await
    }

    /// Explicit close (wsTransport `close()`) — the link does not reconnect. Returns at once; every window learns `down`
    /// through routing. Contract: `LinkClient::close`.
    pub fn close(&self) {
        self.link.close();
    }

    // ── T6a: invoke 명령 request/reply 평면(spawn/kill/interrupt/write/resize/…) ─────────
    // Sends a side-effect command through the connection driver and awaits the daemon's reply (matched by request_id).
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
        self.link.send_command(cmd).await
    }

    // ── T6b: fire-and-forget 평면(Subscribe/Unsubscribe/Resize — reply 없음) ─────────────
    /// Writes `Unsubscribe` on the open connection, behind earlier unsubscribes and replay requests; with none open it
    /// does nothing — the daemon drops a connection's subscriptions with it. Never blocks (the layout calls it under its
    /// lock). Contract: `LinkClient::unsubscribe`.
    pub fn unsubscribe(&self, agent_id: AgentId) {
        self.link.unsubscribe(agent_id);
    }

    /// A command without a reply (Resize …) — `agent_resize` uses it. Never blocks; with no connection to take it the
    /// command is dropped (ADR-0195) and the frontend re-issues a lost Resize. Contract:
    /// `LinkClient::send_fire_and_forget`.
    pub fn send_fire_and_forget(&self, cmd: AgentCommand) {
        self.link.send_fire_and_forget(cmd);
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
        self.link.nudge_usage();
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
                // Holds the shared interest and a nudger that does not keep the actor alive — not the client (a nudge
                //   after the client is gone does nothing).
                let usage = self.usage.clone();
                let nudger = self.link.usage_nudger();
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

    /// View-driven replay (ADR-0046 M1 — single flight): the generation the view fences its flush with. `Err` when no
    /// connection is open, no window shows the agent or its `Subscribe` could not be written. Contract:
    /// `LinkClient::request_replay`.
    pub async fn request_replay(&self, agent_id: AgentId) -> Result<u64, String> {
        self.link.request_replay(agent_id).await
    }
}

impl Drop for DaemonClient {
    fn drop(&mut self) {
        // The link driver ends with its `LinkClient` (dropped with the fields below).
        // ★전용 런타임은 여기서 **비블로킹으로** 접는다★ — 이 함수 본문이 끝나면 필드들이 이어서 drop
        //   되고, 그중 [`Self::_owned_rt`] 는 tokio `Runtime` 이다. `Runtime` 의 기본 drop 은 워커가 멈출
        //   때까지 블로킹하는데, **async 컨텍스트 안에서 그 drop 이 일어나면 tokio 가 패닉한다**. `Drop`
        //   안의 패닉은 unwinding 중이면 프로세스를 abort 시킨다 — 진단 없이 죽는 결말이다.
        //   `shutdown_background` 는 워커를 detach 해 즉시 돌아오므로 어느 컨텍스트에서든 같은 값을 낸다.
        //   소멸자가 원하는 것이 정확히 그것이다(기다리지 않음 + 패닉 없음).
        // ★"오늘은 못 닿는다"에 이걸 걸지 않는다★: 지금 이 자리가 그 조합에 닿을 길은 없다 — 테스트
        //   생성자는 전부 `_owned_rt: None` 이고, 운영 클라이언트는 `Arc` 순환(`lib.rs` 가 클라이언트 자신의 `Arc`
        //   로 명령 표를 굽고 그 표를 `install_command_table` 로 이 클라이언트에 꽂는다) 때문에 애초에 drop 되지
        //   않는다. 그러나 그 무해함은 **우연이지 구조가 아니다**(런타임을 소유하는
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
mod test_support;
