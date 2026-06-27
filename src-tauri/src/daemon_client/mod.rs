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

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use engram_dashboard_protocol::DaemonInfo;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, watch};

use connection::{run_connection, ConnectionCommand, HandshakeError, HANDSHAKE_TIMEOUT};
use lifecycle::Lifecycle;

/// 연결 수명 상태. 재연결 전이(connecting→reconnecting→down 백오프)는 T4 가 채운다 —
/// T2 는 "한 번 연결해 connected 도달" + 명시 close 만 표현한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// 아직 연결 시도 전 또는 명시 종료됨(close). 재연결 소진 종착(T4)도 여기로 모인다.
    Down,
    /// 연결/핸드셰이크 진행 중(소켓 open ~ Hello 수신 전).
    Connecting,
    /// Hello 수신 = 인증 성공. 명령/구독 가능.
    Connected,
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
    /// 연결 task 를 spawn 할 런타임 핸들. 운영=Tauri/전용 multi-thread, 테스트=현재 런타임.
    rt: Handle,
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
    pub async fn connect(&self) -> Result<(), HandshakeError> {
        if self.state() == ConnectionState::Connected {
            return Ok(());
        }
        // ★spawn 허용 경로★: ensure_spawn(데몬 없으면 띄움). blocking 이라 spawn_blocking 으로 감싼다.
        let discovery = self.discovery.clone();
        let info = self
            .rt
            .spawn_blocking(move || discovery.ensure_spawn(DEFAULT_CONNECT_TIMEOUT))
            .await
            .map_err(|e| HandshakeError::Discovery(format!("ensure join 실패: {e}")))?
            .map_err(HandshakeError::Discovery)?;
        self.start_connection(info).await
    }

    /// attach-only 진입점(ADR-0021 B-1) = wsTransport `ensureReady()` 대응.
    ///
    /// ★no-spawn★: `discovery.read_live`(daemon.json read-only)만 부른다 — 데몬이 없으면 띄우지
    /// 않고 실패한다(명령이 데몬을 respawn 하면 안 됨). 살아있는 데몬에만 attach.
    /// 이미 connected 면 즉시 Ok(주 가드는 generation, 이건 보조 단축).
    pub async fn ensure(&self) -> Result<(), HandshakeError> {
        if self.state() == ConnectionState::Connected {
            return Ok(());
        }
        // ★ADR-0021 no-spawn 불변식★: read_live 만 — ensure 는 절대 ensure_spawn 을 부르지 않는다.
        // 데몬이 없으면 여기서 끝(spawn 0회). 복구는 명시 connect() 로만.
        let Some(info) = self.discovery.read_live() else {
            return Err(HandshakeError::NoLiveDaemon);
        };
        self.start_connection(info).await
    }

    /// 주어진 접속 정보로 연결 task 를 띄우고 Hello 까지 await 한다(connect/ensure 공통 후반부).
    ///
    /// 연결 task 가 stream 을 단독 소유한다 — 여기선 cmd_tx 끝만 보관하고, 핸드셰이크 완료
    /// 신호(oneshot)만 기다린다.
    ///
    /// ## ★generation 가드(Fix B — 락으로 원자화)★
    /// 진입 즉시 lifecycle 락 아래서 세대를 bump+캡처(`my_gen`)하고 같은 락 안에서 Connecting 전이를
    /// 발행한다 — 동시 connect/ensure 가 둘 다 들어오면 둘 다 bump 해 서로 다른 세대를 갖고, 더 새 task
    /// 만 current 가 된다. close() 도 같은 락 아래서 세대를 올려 진행 중 task 를 전부 stale 화한다.
    /// 공유 상태(state watch·cmd_tx) 변경은 항상 "세대 비교 + 변경" 을 같은 critical section 으로 묶는
    /// lifecycle 메서드(`publish_if_current`/`store_cmd_if_current`)로만 한다 — 비교와 변경 사이에 다른
    /// 스레드가 세대를 못 바꾸므로 stale caller/task 는 절대 공유 상태를 못 건드린다(Connecting/Down/
    /// cmd_tx clobber 불가). ★ADR-0006★: 락 메서드는 전부 동기(await 없음)라, 아래 `ready_rx.await`
    /// 등 모든 await 는 락을 보유하지 않은 채 일어난다.
    async fn start_connection(&self, info: DaemonInfo) -> Result<(), HandshakeError> {
        // 세대 bump + 캡처 + Connecting 전이를 한 락 아래 원자로 묶는다(bump 후 Connecting 사이에 다른
        // connect/close 가 끼어 내 Connecting 이 stale 한 상태를 덮는 일을 차단). 락은 이 메서드 호출
        // 안에서만 잡혔다 즉시 풀린다 — 아래 await 들은 락 밖.
        let my_gen = self
            .lifecycle
            .bump_and_capture(Some(ConnectionState::Connecting));

        let (cmd_tx, cmd_rx) = mpsc::channel::<ConnectionCommand>(64);
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<Result<(), HandshakeError>>();

        // ★단일 연결 task 소유★: run_connection 이 WebSocketStream 을 split 해 단독 소유하고,
        //   cmd_rx 로 들어오는 명령을 처리한다(T2 는 핸드셰이크까지, 명령 처리 로직은 T6).
        //   my_gen + lifecycle 핸들을 넘겨, task 가 stale 이면 공유 상태를 안 건드리게 한다(Fix B).
        self.rt.spawn(run_connection(
            info,
            my_gen,
            self.lifecycle.clone(),
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
                self.lifecycle.store_cmd_if_current(my_gen, cmd_tx);
                Ok(())
            }
            Ok(Err(e)) => {
                // current 일 때만 Down(stale 이면 더 새 연결의 상태를 clobber 하면 안 됨) — 원자 가드.
                self.lifecycle
                    .publish_if_current(my_gen, ConnectionState::Down);
                Err(e)
            }
            // ready_tx 가 send 없이 drop 됨 = (a) task panic 또는 (b) ★stale self-close★(run_connection
            // 의 generation 가드가 ready 송신을 건너뛰고 빠짐). 둘 다 이 caller 입장에선 핸드셰이크 실패.
            // stale 한 경우 더 새 연결이 진행 중이므로 여기서 Down 을 쏘면 안 된다 → 원자 가드로 current 만.
            Err(_) => {
                self.lifecycle
                    .publish_if_current(my_gen, ConnectionState::Down);
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
        self.lifecycle.close();
    }

    // ── T6 자리: invoke 명령(spawn/kill/write/resize/subscribe) ─────────────────────
    // 여기에 `pub async fn send_command(&self, cmd) -> reply` 가 들어간다 — cmd_tx.send 후
    // oneshot await 패턴(connection.rs ConnectionCommand 의 reply 채널). T2 는 채널 스켈레톤만.
    // TODO(T6): send_command/spawn/kill/write/resize/subscribe invoke 경로.

    // ── T4 자리: 재연결·백오프·generation 가드·closedByUser ──────────────────────────
    // TODO(T4): 비의도 끊김 시 read_live 기반 attach-only 재연결(지수 백오프 500ms→10s MAX5→Down),
    //   generation(openGen) 가드, closedByUser 가드(명령/재연결이 spawn 안 하게).

    // ── T3 자리: epoch/seq dedup·resubscribe·pending(request_id) 매칭 ────────────────
    // TODO(T3): protocol_state(SubState by agent: epoch·last_delivered_seq) + pending HashMap.

    // ── T5 자리: OutputRouter(arc-swap 라우팅) 연결 ───────────────────────────────────
    // TODO(T5): 연결 task 가 디코드한 output frame 을 OutputRouter 로 라우팅(ViewManager 기반).
}

#[cfg(test)]
mod tests;
