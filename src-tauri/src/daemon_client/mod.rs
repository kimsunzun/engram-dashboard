//! DaemonClient — 데몬 WS 연결의 src-tauri측 단일 권위 (S14 모듈①, ADR-0036).
//!
//! 프론트가 각 창마다 데몬에 N개 WS 를 직결하던 구조(src/api/wsTransport.ts)를 src-tauri 로
//! 끌어올린다 — **창이 몇 개든 데몬엔 연결 1개**. 이 모듈은 그 연결의 수립·핸드셰이크·생애를
//! 소유한다. 프로토콜 의미론(epoch/seq dedup·resubscribe·pending 매칭)·재연결·라우팅은 후속
//! 태스크(T3/T4/T5/T6)가 채운다.
//!
//! ## T2 범위(이 파일들이 구현하는 것)
//! - 연결 수립 + Auth/Hello 핸드셰이크(`connection.rs`).
//! - `connect`(명시 spawn 진입점) / `ensure`(attach-only, no-spawn) 분리 — ADR-0021.
//! - 단일 연결 task(actor) 스켈레톤: 한 task 가 `WebSocketStream` 을 단독 소유(Mutex 없음),
//!   invoke 는 `cmd_tx.send` → 연결 task 가 수신해 처리(실제 명령 처리는 T6).
//! - connected/connecting/down 상태 표현(재연결 전이는 T4).
//!
//! ## ★동시성 모델(load-bearing)★
//! - **단일 연결 task 가 stream 을 단독 소유한다(Mutex 없이).** WebSocketStream 의 SplitSink 는
//!   동시 write 불가라, 여러 호출자가 직접 ws 를 만지면 write 가 교차한다. 그래서 데몬 ws.rs 의
//!   "연결당 단일 writer" 와 대칭으로, 클라도 **하나의 task** 가 read/write 를 전담하고 다른
//!   주체(invoke 핸들러)는 `cmd_tx`(mpsc) 로 의도만 보낸다. 이게 openGen(wsTransport)의 zombie
//!   가드를 task lifetime 으로 대체하는 토대다(완전한 가드는 T4).
//! - **generation 가드(openGen 씨앗, Fix B)**: `generation`·`cmd_tx`·watch 전이를 **하나의
//!   `Mutex<Lifecycle>` 아래로** 통합한다(`lifecycle.rs`). connect/ensure/close 마다 락 안에서 세대를
//!   올리고, 각 연결 task 는 spawn 시점 세대(my_gen)를 캡처한다. task·caller 는 공유 상태(state
//!   watch·cmd_tx)를 건드리는 "세대 비교 + 변경" 을 **같은 critical section** 으로 묶어 원자화한다 —
//!   `publish_if_current`/`store_cmd_if_current`. 비교와 변경 사이에 다른 스레드가 세대를 못 바꾸므로,
//!   밀려난(stale) task 는 공유 상태를 절대 못 건드린다 → 동시 connect·close-in-flight 에서 고아 Down
//!   clobber·좀비 cmd_tx·close 후 Connected 부활을 막는다. ⚠️ atomic 하나(load→send 분리)는 SeqCst
//!   여도 체크+변경을 못 묶어 TOCTOU 가 reachable 했다(Codex 적출) — 그래서 락으로 원자화했다. T2 는
//!   *씨앗*까지만 — 짧은 순간 소켓 2개가 동시에 열릴 수 있음(둘 다 connect_async)은 허용하되 관찰
//!   가능한 상태 오염만 없앤다. 완전한 동시-시도 abort·백오프 재연결은 T4.
//! - 상태(`ConnectionState`)는 `watch` 채널로 노출 — 읽기 측(여러 구독자)이 락 없이 현재값을 본다.

pub mod connection;
mod lifecycle;
pub mod protocol_state;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use engram_dashboard_protocol::{AgentCommand, AgentEvent, DaemonInfo};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, watch};

use connection::{run_connection, ConnectionCommand, HandshakeError, HANDSHAKE_TIMEOUT};
use lifecycle::Lifecycle;

/// 연결 수명 상태. T4 가 재연결 전이(connected→reconnecting→connected 회복 / 소진 시 down)를 채웠다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// 아직 연결 시도 전 또는 명시 종료됨(close). 재연결 소진 종착(T4)도 여기로 모인다.
    Down,
    /// 연결/핸드셰이크 진행 중(소켓 open ~ Hello 수신 전).
    Connecting,
    /// Hello 수신 = 인증 성공. 명령/구독 가능.
    Connected,
    /// 비의도 끊김 후 재연결 시도 중(백오프 sleep ~ 다음 attach 시도). 소진 시 Down, 성공 시 Connected.
    /// wsTransport `reconnecting` 상태 대응 — 명시 close(Down)와 구분된다(자동 회복 진행 중).
    Reconnecting,
}

/// 데몬 발견 경계(seam). connect 경로는 spawn 가능(`ensure_spawn`), ensure 경로는 no-spawn
/// (`read_live`)만 — ADR-0021 분리를 이 trait 의 **서로 다른 메서드**로 못박는다.
///
/// ★왜 trait 인가★: 실제 구현은 discovery crate(WMI spawn·파일 IO·실시간)에 닿아 단위 테스트가
/// 실 데몬을 띄워야 한다. seam 으로 끊어 테스트가 "spawn 호출 0회"(ensure no-spawn 불변)와
/// "주어진 host/port 반환"을 실 WMI 없이 단언한다(discovery crate 의 DaemonReader/Spawner 주입 동형).
pub trait DaemonDiscovery: Send + Sync + 'static {
    /// 명시 연결(connect) 경로. 살아있는 데몬을 찾고, 없으면 **spawn** 해서 접속 정보를 돌려준다.
    /// wsTransport 의 `invoke('discover_daemon')` 대응(spawn 유발 = 데몬이 살아날 수 있음).
    fn ensure_spawn(&self, timeout: Duration) -> Result<DaemonInfo, String>;

    /// 재연결/ensure(attach-only) 경로. 현재 daemon.json 을 **읽기만** 한다(no-spawn). 살아있는
    /// 호환 데몬이면 Some, 없으면 None. wsTransport 의 `invoke('read_daemon_info')` 대응.
    /// ★불변식(ADR-0021)★: 이 메서드는 절대 spawn 하지 않는다 — 명령/재연결이 데몬을 깨우면 안 된다.
    fn read_live(&self) -> Option<DaemonInfo>;
}

/// 운영 DaemonDiscovery — discovery crate 에 위임. connect=ensure_daemon(spawn 가능),
/// ensure=read_live_daemon(no-spawn). 데이터 폴더·exe 경로는 discovery 단일 출처(ADR-0024/0029).
///
/// ★blocking 주의★: ensure_daemon 은 폴링·sleep·WMI 동기 호출을 포함한다. 호출자(연결 task)는
/// `spawn_blocking` 으로 감싸 async executor 를 막지 않는다(connection.rs 참조).
pub struct RealDiscovery;

impl DaemonDiscovery for RealDiscovery {
    fn ensure_spawn(&self, timeout: Duration) -> Result<DaemonInfo, String> {
        let data_dir: PathBuf = engram_dashboard_discovery::default_data_dir();
        // console=false: windowless spawn(콘솔 가시화는 daemon_start command 전용).
        let exe = engram_dashboard_discovery::locate_daemon_exe().map_err(|e| e.to_string())?;
        engram_dashboard_discovery::ensure_daemon(&data_dir, &exe, timeout, false)
            .map_err(|e| e.to_string())
    }

    fn read_live(&self) -> Option<DaemonInfo> {
        // ★ADR-0021 no-spawn★: read_live_daemon 은 daemon.json 을 읽기만 한다(데몬을 깨우지 않음).
        let data_dir: PathBuf = engram_dashboard_discovery::default_data_dir();
        engram_dashboard_discovery::read_live_daemon(&data_dir)
    }
}

/// discover(spawn 가능) timeout 기본값(wsTransport discover_daemon 5s 와 정렬).
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// 데몬 연결의 단일 핸들. invoke 핸들러·트레이·상태 구독자가 공유한다(`Arc<DaemonClient>`).
///
/// 연결 task 본체는 spawn 된 tokio task(`run_connection`)가 소유하고, 이 구조체는 그 task 와
/// 통신하는 채널 끝(`cmd_tx`)과 상태 구독(`state_rx`)만 들고 있다 — stream 자체는 절대 들지 않는다
/// (단일 task 소유 불변식).
pub struct DaemonClient {
    /// 연결 task 를 spawn 할 런타임 핸들. 운영=전용 multi-thread(`_owned_rt`), 테스트=현재 런타임.
    rt: Handle,
    /// ★전용 런타임 소유(운영 — T6a)★. `setup` 콜백은 tokio 런타임 컨텍스트가 아닐 수 있어
    /// `Handle::current()` 가 패닉한다. 그래서 운영 생성자(`new_real_with_owned_runtime`)는 spike §2
    /// "tokio multi-thread(데몬처럼)" 대로 전용 멀티스레드 런타임을 *직접 만들어* 그 Handle 을 쓴다.
    /// 이 필드가 런타임을 살려둔다 — drop 되면 Handle 이 무효가 돼 연결 task 가 죽으므로 DaemonClient
    /// 수명과 묶는다(`Arc<DaemonClient>` 가 app 수명). `None` = 외부 핸들 주입(테스트=현재 런타임).
    _owned_rt: Option<Arc<tokio::runtime::Runtime>>,
    /// 데몬 발견 경계(connect=spawn 가능 / ensure=no-spawn).
    discovery: Arc<dyn DaemonDiscovery>,
    /// 현재 연결 상태 빠른 읽기(watch). 여러 구독자가 락 없이 현재값을 본다. 송신은 항상 lifecycle
    /// 락 아래서만(가드된 전이) — 그래야 "세대 체크 + watch send" 가 원자적이다. 이 rx 는 borrow 만.
    state_rx: watch::Receiver<ConnectionState>,
    /// ★연결 lifecycle 가드(Fix B — openGen 씨앗)★. generation·cmd_tx·watch 전이를 하나의 락 아래로
    /// 통합한다(lifecycle.rs). connect/ensure/close 가 락 안에서 세대를 올리고, 각 연결 task 는 spawn
    /// 시점 세대를 캡처해 공유 상태 변경을 "세대 비교 + 변경" 한 critical section 으로 원자화한다 —
    /// 밀려난 task 는 공유 상태를 못 건드려 고아 Down clobber·좀비 cmd_tx·Connected 부활을 막는다.
    /// AtomicU64+분리 send 의 TOCTOU(SeqCst 로도 못 묶음, Codex 적출)를 락으로 닫았다.
    lifecycle: Arc<Lifecycle>,
    /// 핸드셰이크(소켓 open ~ Hello) 상한. 운영=HANDSHAKE_TIMEOUT, 테스트=짧은 값 주입(Fix A).
    handshake_timeout: Duration,
}

impl DaemonClient {
    /// 핸들만 만든다(연결은 connect/ensure 호출 시). `rt` 는 연결 task 를 띄울 런타임 핸들.
    /// 핸드셰이크 상한은 운영 기본값(HANDSHAKE_TIMEOUT).
    pub fn new(rt: Handle, discovery: Arc<dyn DaemonDiscovery>) -> Self {
        Self::new_with_handshake_timeout(rt, discovery, HANDSHAKE_TIMEOUT)
    }

    /// 핸드셰이크 상한을 주입하는 생성자(Fix A 테스트 용이성 — 테스트가 짧은 값으로 Timeout 을 검증).
    /// const 하드코딩이 테스트를 10초 기다리게 만들지 않도록, 상한을 필드로 받는다.
    pub fn new_with_handshake_timeout(
        rt: Handle,
        discovery: Arc<dyn DaemonDiscovery>,
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
        }
    }

    /// 운영 생성자 — RealDiscovery + 주어진 런타임 핸들.
    pub fn new_real(rt: Handle) -> Self {
        Self::new(rt, Arc::new(RealDiscovery))
    }

    /// ★운영 생성자(T6a — 전용 런타임 소유)★. `lib.rs` `setup` 에서 쓴다. tokio 런타임 컨텍스트 밖
    /// (`setup` 콜백)에서 `Handle::current()` 가 패닉하지 않도록, 전용 멀티스레드 런타임을 직접 만들어
    /// 그 Handle 로 연결 task 를 띄운다(spike §2). 런타임은 DaemonClient 가 소유(`_owned_rt`)해 app
    /// 수명 동안 살아있다. 실패(런타임 생성 불가)면 Err — 호출자가 보고하고 데몬 명령 없이 진행한다.
    pub fn new_real_with_owned_runtime() -> std::io::Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("engram-daemon-client")
            .build()?;
        let handle = rt.handle().clone();
        let (lifecycle, state_rx) = Lifecycle::new();
        Ok(Self {
            rt: handle,
            _owned_rt: Some(Arc::new(rt)),
            discovery: Arc::new(RealDiscovery),
            state_rx,
            lifecycle: Arc::new(lifecycle),
            handshake_timeout: HANDSHAKE_TIMEOUT,
        })
    }

    /// 현재 연결 상태 스냅샷(락 없이).
    pub fn state(&self) -> ConnectionState {
        *self.state_rx.borrow()
    }

    /// 상태 변경 구독(watch). 호출자가 await 로 다음 전이를 기다리거나 현재값을 본다.
    pub fn subscribe_state(&self) -> watch::Receiver<ConnectionState> {
        self.state_rx.clone()
    }

    /// 명시 연결 진입점(ADR-0021 §1) = wsTransport `start()` 대응.
    ///
    /// ★spawn 가능★: 데몬이 없으면 `discovery.ensure_spawn` 이 WMI 로 띄운다 — 부팅 연결/사용자
    /// 명시 시작만 이 경로를 탄다. discover → WS → Auth → Hello → connected 까지 한 번에 간다.
    /// 이미 connected 면 즉시 Ok(중복 연결 방지 — 주 가드는 generation, 이건 보조 단축).
    ///
    /// ## ★승계 취소를 discovery *전에* (FIX-1, T4 2차)★
    /// 진입 즉시(느린 discovery await 전에) `bump_and_capture(Some(Connecting))` 으로 옛 세대를 취소+
    /// 승계한다 — bump 가 cancel watch 에 신호를 쏘고 옛 cmd_tx 를 비운다. 그래야 discovery 창(spawn 가능
    /// = 수십초까지 늘어날 수 있음) 동안 진행 중이던 옛 재연결 세대가 *그 창에서* 소켓을 열고 Auth 를
    /// 보내지 못한다(OSS 정석: 승계 시 옛 토큰 즉시 취소). 이전엔 discovery 를 먼저 하고 start_connection
    /// 안에서 bump 했어서 그 창이 무방비였다(Codex BLOCK). 캡처한 my_gen 을 그대로 start_connection 에
    /// 넘겨 ★이중 bump 를 피한다★(start_connection 은 더 이상 bump 안 함).
    pub async fn connect(&self) -> Result<(), HandshakeError> {
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
        //   이 await 동안 옛 세대는 이미 취소·stale 이라 소켓을 못 연다(위 bump 가 닫은 창).
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
                self.lifecycle
                    .publish_if_current(my_gen, ConnectionState::Down);
                return Err(HandshakeError::Discovery(e));
            }
            Err(e) => {
                tracing::warn!("데몬 discovery join 실패: {e}");
                self.lifecycle
                    .publish_if_current(my_gen, ConnectionState::Down);
                return Err(HandshakeError::Discovery(format!("ensure join 실패: {e}")));
            }
        };
        self.start_connection(info, my_gen).await
    }

    /// attach-only 진입점(ADR-0021 B-1) = wsTransport `ensureReady()` 대응.
    ///
    /// ★no-spawn★: `discovery.read_live`(daemon.json read-only)만 부른다 — 데몬이 없으면 띄우지
    /// 않고 실패한다(명령이 데몬을 respawn 하면 안 됨). 살아있는 데몬에만 attach.
    /// 이미 connected 면 즉시 Ok(주 가드는 generation, 이건 보조 단축).
    ///
    /// ## ★승계 취소를 read_live *전에* (FIX-1, T4 2차)★
    /// connect() 와 동형: read_live(no-spawn 이라 짧지만, 파일 IO 가 느릴 여지)를 부르기 전에 bump 로 옛
    /// 세대를 취소·승계한다. ensure 는 attach-only(ADR-0021 — no-spawn)지만 *승계 취소* 는 동일 적용 —
    /// read_live 창 동안 옛 재연결이 소켓을 열지 못하게. 캡처한 my_gen 을 start_connection 에 넘긴다.
    pub async fn ensure(&self) -> Result<(), HandshakeError> {
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
            self.lifecycle
                .publish_if_current(my_gen, ConnectionState::Down);
            return Err(HandshakeError::NoLiveDaemon);
        };
        self.start_connection(info, my_gen).await
    }

    /// 주어진 접속 정보로 연결 task 를 띄우고 Hello 까지 await 한다(connect/ensure 공통 후반부).
    ///
    /// 연결 task 가 stream 을 단독 소유한다 — 여기선 cmd_tx 끝만 보관하고, 핸드셰이크 완료
    /// 신호(oneshot)만 기다린다.
    ///
    /// ## ★generation 가드(Fix B — 락으로 원자화)★
    /// 세대 bump + 캡처는 **호출자(connect/ensure)가 진입 즉시 discovery 전에** 한다(FIX-1) — 그
    /// `my_gen` 을 여기로 넘겨받는다. ★이 함수는 더 이상 bump 하지 않는다(이중 bump 회피)★. 동시
    /// connect/ensure 가 둘 다 들어오면 각자 진입에서 bump 해 서로 다른 세대를 갖고, 더 새 task 만
    /// current 가 된다. close() 도 같은 락 아래서 세대를 올려 진행 중 task 를 전부 stale 화한다.
    /// 공유 상태(state watch·cmd_tx) 변경은 항상 "세대 비교 + 변경" 을 같은 critical section 으로 묶는
    /// lifecycle 메서드(`publish_if_current`/`store_cmd_if_current`)로만 한다 — 비교와 변경 사이에 다른
    /// 스레드가 세대를 못 바꾸므로 stale caller/task 는 절대 공유 상태를 못 건드린다(Connecting/Down/
    /// cmd_tx clobber 불가). ★ADR-0006★: 락 메서드는 전부 동기(await 없음)라, 아래 `ready_rx.await`
    /// 등 모든 await 는 락을 보유하지 않은 채 일어난다.
    ///
    /// ★my_gen 계약★: 호출자가 `bump_and_capture` 로 막 캡처해 넘긴 값이다. 그 bump 와 이 함수 진입
    /// 사이에 다른 connect/close 가 또 끼면 내 my_gen 은 이미 stale 일 수 있다 — 그래도 모든 발행이
    /// publish_if_current/store_cmd_if_current 가드를 통과하므로 안전하다(stale 이면 그냥 미발행).
    async fn start_connection(&self, info: DaemonInfo, my_gen: u64) -> Result<(), HandshakeError> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<ConnectionCommand>(64);
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<Result<(), HandshakeError>>();

        // ★단일 연결 task 소유★: run_connection 이 WebSocketStream 을 split 해 단독 소유하고,
        //   cmd_rx 로 들어오는 명령을 처리한다(T6). 비의도 끊김 시 이 task 안에서 백오프 재연결을
        //   돈다(T4 — discovery.read_live no-spawn + rt.spawn_blocking). my_gen + lifecycle 핸들로
        //   stale task 가 공유 상태를 못 건드리게 한다(Fix B + reconnect_guard).
        self.rt.spawn(run_connection(
            info,
            my_gen,
            self.lifecycle.clone(),
            self.discovery.clone(),
            self.rt.clone(),
            self.handshake_timeout,
            cmd_rx,
            ready_tx,
        ));

        // Hello 수신(=connected) 또는 핸드셰이크 실패를 기다린다. ★락 미보유 await★.
        match ready_rx.await {
            Ok(Ok(())) => {
                // ★가드된 cmd_tx 저장★: ready Ok 를 받았어도, 그 사이 더 새 connect/close 가 세대를
                //   올렸으면 이 연결은 stale 이다 — cmd_tx 를 저장하면 좀비 채널이 된다. store_cmd_if_current
                //   가 "세대 비교 + 저장" 을 원자로 해, current 일 때만 저장한다. stale 이면 저장하지 않고
                //   cmd_tx 가 여기서 drop → 연결 task 의 cmd_rx 가 EOF → 그 task 도 곧 정리된다.
                if !self.lifecycle.store_cmd_if_current(my_gen, cmd_tx) {
                    // generation 가드 발동: 핸드셰이크 사이 더 새 connect/close 가 세대를 올림 → cmd_tx
                    //   미저장(좀비 채널 차단). 이 caller 입장에선 연결이 밀렸으나 핸드셰이크 자체는 성공.
                    tracing::debug!(
                        generation = my_gen,
                        "stale 연결 — cmd_tx 미저장(더 새 connect/close 가 세대를 올림)"
                    );
                }
                Ok(())
            }
            Ok(Err(e)) => {
                // ★재로깅 안 함★: 구체 실패 사유(connect/직렬화/Auth 송신/핸드셰이크)는 run_connection 이
                //   이미 정확한 문구로 warn 을 남겼다(connection.rs). 여기서 또 찍으면 같은 실패가 warn 2줄 +
                //   "reject" 로 오라벨된다 — 단일 출처 유지를 위해 caller 쪽은 무로깅으로 전파만 한다.
                // current 일 때만 Down(stale 이면 더 새 연결의 상태를 clobber 하면 안 됨) — 원자 가드.
                self.lifecycle
                    .publish_if_current(my_gen, ConnectionState::Down);
                Err(e)
            }
            // ready_tx 가 send 없이 drop 됨 = (a) task panic 또는 (b) ★stale self-close★(run_connection
            // 의 generation 가드가 ready 송신을 건너뛰고 빠짐). 둘 다 이 caller 입장에선 핸드셰이크 실패.
            // stale 한 경우 더 새 연결이 진행 중이므로 여기서 Down 을 쏘면 안 된다 → 원자 가드로 current 만.
            Err(_) => {
                // ★레벨을 stale 여부로 가른다★: publish_if_current 가 true(=current 였는데 ready 없이
                //   task 가 사라짐)면 진짜 이상(panic 추정) → 사람이 봐야 함(warn). false(=stale)면 더 새
                //   연결이 세대를 올려 publish_if_current 가 Down 을 삼킨 경우다 — stale 한 이 task 가 ready
                //   없이 사라진 원인은 run_connection 의 가드 self-close *또는* stale task 의 panic 둘 다일 수
                //   있으나(둘 다 false 분기로 귀결), 어느 쪽이든 이미 superseded 라 진단용 debug 로 충분하다.
                //   Down 이 stale 이면 삼켜진다(clobber 방지 — connection.rs 의 main_loop 종료 Down 가드와 동형).
                if self
                    .lifecycle
                    .publish_if_current(my_gen, ConnectionState::Down)
                {
                    tracing::warn!(
                        generation = my_gen,
                        "연결 task 가 ready 신호 전 사라짐(current 연결 — panic 추정)"
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

    /// 명시 종료(wsTransport `close()` 대응). 연결 task 에 종료를 알리고 Down 으로 전이한다.
    ///
    /// ★generation 가드(Fix B — 락으로 원자화)★: lifecycle 락 아래서 (a)세대 bump (b)cmd_tx=None
    /// (c)Down 발행 **을 한 원자 단위로** 한다. bump 가 진행 중인(핸드셰이크 중 포함) 모든 연결 task 를
    /// stale 화하고, bump 와 Down 사이에 stale task 가 끼어 Connected 를 발행할 수 없다(그 task 의
    /// publish_if_current 가 이미 올라간 세대를 보고 삼킨다). cmd_tx drop → 연결 task 의 cmd_rx EOF →
    /// task 정리. 이 Down 은 close 자신의 의도라 항상 유효.
    ///
    /// ★ADR-0006★: lifecycle.close() 는 전부 동기(bump+Option 교체+watch send) — await 없음.
    /// ★재연결 금지는 T4★: T2 는 명시 close 만. closedByUser 가드(명령/재연결이 respawn 안 하게)는
    /// 백오프 재연결과 함께 T4 가 채운다.
    pub fn close(&self) {
        tracing::info!("데몬 연결 명시 종료(close)");
        self.lifecycle.close();
    }

    // ── T6a: invoke 명령 request/reply 평면(spawn/kill/interrupt/write/resize/…) ─────────
    /// side-effect 명령을 연결 task 로 보내고 데몬 reply(request_id 매칭)를 await 한다.
    ///
    /// ★계약(request_id)★: `cmd` 는 **호출자가 request_id 를 이미 박은** 명령이다(commands/agent.rs 의
    /// 빌더가 `RequestId::new()` 로 채운다). 그래야 reply 매칭 키가 호출자에게도 알려져 idempotency
    /// (재시도 시 같은 키)와 정합한다 — send_command 가 임의로 채우면 호출자가 키를 모른다. request_id
    /// 없는 명령(Auth/Subscribe/Unsubscribe/Resize)은 reply 가 안 와 hang 이므로 여기서 거른다.
    ///
    /// ★흐름★: (1) 현재 cmd_tx clone(없으면 not-connected Err) (2) oneshot 생성 (3) `SendCommand`
    /// enqueue (4) reply await. 연결 task 가 reply 를 resolve(Ok/Err)하거나, 끊김 시 drain 으로 Err 를
    /// 보낸다(no-hang). cmd_tx send 실패(채널 full/닫힘)·oneshot drop(연결 task 사망)도 Err 로 귀결.
    ///
    /// ★ADR-0006(락 across await 금지)★: `current_cmd_tx()` 는 락을 잡았다 즉시 풀고 Sender clone 만
    /// 돌려준다 — 이후 `tx.send().await`·`rx.await` 는 락 미보유 상태다(Sender 는 lifecycle 락과 독립).
    pub async fn send_command(&self, cmd: AgentCommand) -> Result<AgentEvent, String> {
        // request_id 없는 명령은 reply 매칭 불가 → 즉시 거른다(연결 task 에서 영구 pending 방지).
        if protocol_state::command_request_id(&cmd).is_none() {
            return Err("send_command: request_id 없는 명령은 reply 를 기대할 수 없다".to_string());
        }
        // 현재 활성 연결의 cmd_tx 를 얻는다(없으면 연결 안 됨/끊김).
        let Some(cmd_tx) = self.lifecycle.current_cmd_tx() else {
            return Err("데몬에 연결되어 있지 않음(connect 먼저)".to_string());
        };
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        // 연결 task 로 enqueue. send 실패 = 채널 닫힘(연결 task 종료) → not-connected 취급.
        if cmd_tx
            .send(ConnectionCommand::SendCommand {
                cmd,
                reply: reply_tx,
            })
            .await
            .is_err()
        {
            return Err("연결 task 가 명령을 받지 못함(끊김)".to_string());
        }
        // reply 대기. 연결 task 가 resolve(Ok/Err) 하거나 끊김 drain 으로 Err. oneshot 송신단이 reply
        //   없이 drop(연결 task 사망 등) 되면 RecvError → not-connected 취급.
        match reply_rx.await {
            Ok(result) => result,
            Err(_) => Err("명령 응답 수신 실패(연결 task 종료)".to_string()),
        }
    }

    // ── T4 완료: 재연결·백오프·generation 가드·closedByUser ──────────────────────────
    // 비의도 끊김(데몬 stream 종료/오류/Close frame) 시 연결 task(connection.rs `connected_lifetime`)가
    // **그 task 안에서** attach-only 재연결을 돈다 — read_live(no-spawn) + 지수 백오프(500ms→10s MAX5) →
    // 성공 시 Connected 재전이, 소진 시 Down. close()(closed_by_user)·새 connect(세대 bump)는
    // reconnect_guard(lifecycle.rs)로 Stop → 재연결 즉시 중단(좀비/respawn 차단). 백오프 sleep 은
    // tokio::time::sleep 이라 테스트가 time::pause/advance 로 결정론 검증(ADR-0038).
    //   ★resubscribe 배선은 T5/T6★: connected *재*전이 직후 subs 순회하며 각 agent 에
    //   protocol_state::resubscribe_params(&sub) 로 Subscribe{epoch,after_seq} 산출 → wire send
    //   (JS resubscribeAll 대응). 끊김 시 protocol_state::drain_pending(&mut pending) → 일괄 reject.
    //   T4 는 *연결 carrier* 재연결만 — subs/pending 맵은 T5/T6 가 연결 task 에 들이면서 배선한다.

    // ── T3 완료: protocol_state 순수 결정 함수(epoch/seq dedup·resubscribe·pending 매칭) ─────
    // `protocol_state` 모듈이 SubState(epoch·last_delivered_seq)·PendingMap·결정 함수(decide_output·
    // apply_subscribe_ack·resubscribe_params·take_pending·drain_pending)를 순수하게 소유한다(소켓·
    // runtime 의존 0, 순수 결정 단위 테스트 20개 동반 — protocolClient.test.ts 의 event-routing 5케이스는
    // 여기 순수 레이어가 아니라 T5/T6 배선 테스트로 미룸, protocol_state.rs tests mod 주석 참조).
    // ★배선은 미완★: 연결 task 가 이 상태 맵
    // (subs: HashMap<AgentId, SubState>, pending: HashMap<RequestId, oneshot>)을 들고 결정 함수를
    // 호출하는 것은 T5(output 라우팅)/T6(invoke 명령) 가 한다 — connection.rs main_loop 의 TODO 참조.

    // ── T5 자리: OutputRouter(arc-swap 라우팅) 연결 ───────────────────────────────────
    // TODO(T5): 연결 task 가 디코드한 output frame 을 OutputRouter 로 라우팅(ViewManager 기반).
}

#[cfg(test)]
mod tests;
