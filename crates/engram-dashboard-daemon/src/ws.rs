//! WebSocket 서버 본체 — 소켓 살림만 하는 네트워크 행(ADR-0129).
//!
//! 책임: accept 된 TCP stream 을 WS 업그레이드(Origin allowlist) → 1초 내 첫 frame 토큰 auth →
//! 연결 수명·단일 writer·keepalive·레지스트리. **프레임 내용의 어휘는 모른다** — 들어온 text/binary
//! 는 `ConnectionHandler`(frame_port)로 올리고, 나가는 것은 `FrameSink` 로 받는다.
//!
//! ★동시성 모델(위험 지점)★
//! - **연결당 단일 writer**: SplitSink 는 동시 write 불가. 그래서 모든 출력 프레임을 연결당 단일
//!   `mpsc::Sender<Frame>`(conn_tx)에 넣고, write_task 한 곳만 SinkHalf 에 write 한다.
//!   SubscribeAck→replay→live 의 FIFO 순서가 이 단일 큐로 보장된다.
//! - **try_send vs await 경계**: 위층 sink 가 pump 스레드에서 부르는 `FrameSink::try_send` 는 절대
//!   block 금지. async 문맥의 `FrameSink::send` 는 await 허용(.send().await).
//! - **out-of-band 종료 신호(close_signal)**: conn_tx 가 full 이면 큐 안 마커(`Frame::Close`)도
//!   try_send 실패해 좀비 연결이 된다. 그래서 큐 **밖**의 `Arc<Notify>` close_signal 을 둔다.
//!   `ConnFrameSink` 가 try_send 에서 full 을 만나면 `close_signal.notify_one()`(sync 안전)으로
//!   신호하고, write_task 는 `tokio::select!` 로 conn_rx.recv() 와 close_signal.notified() 를 동시에
//!   대기해 큐가 막혀 있어도 깨어 sink_half.close() 후 break → cleanup 한다.
//! - **레지스트리**: status 브로드캐스트용. 모든 연결의 conn_tx 를 ConnId→Sender 맵으로 보관해
//!   DaemonStatusSink 가 try_send(Text) 로 전 연결에 fanout.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use engram_dashboard_core::agent::profile::RestoreReport as CoreRestoreReport;
use engram_dashboard_core::agent::types::{
    AgentId, AgentInfo as CoreAgentInfo, AgentStatus as CoreStatus, StatusSink,
};

// ★ADR-0129 잔여 — auth 핸드셰이크★: 핸드셰이크를 데몬 소유 타입으로 옮기는 것은 후속 슬라이스라,
//   지금은 `AgentCommand::Auth`/`PROTOCOL_VERSION` 만 네트워크 행에 남는다. `AgentEvent` 는 아직 이
//   파일에 사는 상태 sink(DaemonStatusSink·MessagingFlushSink — 에이전트 어휘, 이사 예정)의 것이다.
use engram_dashboard_protocol::{AgentCommand, AgentEvent, PROTOCOL_VERSION};

use crate::connection_core::{
    core_agents_to_wire, core_report_to_wire, core_status_to_wire, event_json,
};
use crate::frame_port::{
    ConnFlow, ConnId, ConnectionHandler, ConnectionHandlerFactory, Frame, FrameError, FrameSink,
};

use futures_util::future::BoxFuture;
use futures_util::{SinkExt, Stream, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Notify};
use tokio_tungstenite::tungstenite::handshake::server::{
    Callback, ErrorResponse, Request, Response,
};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::Message;

/// 연결당 송신 큐 용량. ReplayBuffer.max_events(4096) + control_slack(512) = 4608.
/// replay 전체가 들어가도 control 여유가 남게 한다(output_core.rs 불변식과 정합).
const CONN_TX_CAP: usize = 4608;

/// auth 첫 frame 대기 한도. 이 안에 Auth Text 가 안 오면 close.
const AUTH_TIMEOUT: Duration = Duration::from_secs(1);

/// 운영 기본 keepalive 주기 — 데몬이 능동 WS Ping 을 보내는 간격.
const DEFAULT_PING_INTERVAL: Duration = Duration::from_secs(20);
/// 운영 기본 idle 한도 — 마지막 클라 수신 후 이 시간 넘게 무응답이면 half-open 으로 보고 close.
/// ping_interval 의 2.5배(여러 Ping 을 놓쳐야 끊김 — 일시 지연 위양성 방지).
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(50);

/// WS application-level keepalive 설정(A). 능동 Ping 주기 + idle 한도.
///
/// ★half-open 감지★: tungstenite 는 들어온 Ping 에 자동 Pong 만 하고 능동 Ping 은 안 보낸다.
/// FIN 없이 끊기는 연결(sleep/wake·NAT 타임아웃·모바일 터널)에서 TCP keepalive(기본 2시간)는
/// 무의미하므로, write_task 가 ping_interval 마다 Ping 을 보내고 read_task 가 마지막 수신 시각을
/// 기록한다. idle_timeout 초과면 close_signal 로 그 연결을 끊는다(좀비 구독/broadcast 누수 방지).
///
/// ★테스트 주입★: 상수 하드코딩이면 테스트가 수십 초 걸리므로, 짧은 값(예 200ms/600ms)을
/// 주입할 수 있게 설정 가능하게 둔다. 운영 경로는 `default()`(20s/50s) 그대로.
#[derive(Clone, Copy, Debug)]
pub struct KeepaliveConfig {
    pub ping_interval: Duration,
    pub idle_timeout: Duration,
}

impl Default for KeepaliveConfig {
    fn default() -> Self {
        Self {
            ping_interval: DEFAULT_PING_INTERVAL,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
        }
    }
}

/// 허용 Origin allowlist(기본). Origin 없음(네이티브/하네스)은 허용 — 토큰이 주 방어.
const ALLOWED_ORIGINS: &[&str] = &[
    "http://localhost:1420",
    "http://127.0.0.1:1420",
    "tauri://localhost",
    "https://tauri.localhost",
];

/// status 브로드캐스트용 연결 레지스트리. connect 시 등록, disconnect 시 제거.
/// DaemonStatusSink 가 전 연결 conn_tx 에 try_send 하기 위해 공유된다.
#[derive(Clone)]
pub struct ConnRegistry {
    inner: Arc<Mutex<HashMap<ConnId, mpsc::Sender<Frame>>>>,
    next_id: Arc<AtomicU64>,
}

impl ConnRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    fn alloc_id(&self) -> ConnId {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn register(&self, id: ConnId, tx: mpsc::Sender<Frame>) {
        self.inner
            .lock()
            .expect("conn registry poisoned")
            .insert(id, tx);
    }

    fn unregister(&self, id: ConnId) {
        self.inner
            .lock()
            .expect("conn registry poisoned")
            .remove(&id);
    }

    /// 이 conn 이 아직 fanout 대상인지 — 테스트가 cleanup 순서(정리 후 등록 해제)를 관측한다.
    #[cfg(test)]
    pub(crate) fn contains(&self, id: ConnId) -> bool {
        self.inner
            .lock()
            .expect("conn registry poisoned")
            .contains_key(&id)
    }

    /// 전 연결에 Text 브로드캐스트(try_send). full 인 연결은 느린 것으로 보고 로그만.
    pub(crate) fn broadcast_text(&self, text: String) {
        let conns: Vec<(ConnId, mpsc::Sender<Frame>)> = {
            let guard = self.inner.lock().expect("conn registry poisoned");
            guard.iter().map(|(id, tx)| (*id, tx.clone())).collect()
        };
        for (id, tx) in conns {
            // try_send 만 — StatusSink 는 pump/manager 스레드(sync)에서 불릴 수 있어 block 금지.
            if let Err(e) = tx.try_send(Frame::Text(text.clone())) {
                tracing::warn!(
                    conn = id,
                    "status 브로드캐스트 try_send 실패(느린 소비자): {e}"
                );
            }
        }
    }
}

impl Default for ConnRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── DaemonStatusSink(global) ─────────────────────────────────────────────────────

/// AgentManager 에 주입되는 전역 StatusSink. status_changed/agent_list_updated/restore_result
/// 를 AgentEvent JSON 으로 직렬화해 레지스트리의 모든 conn_tx 에 try_send(Text) 한다.
/// (LogStatusSink 대체 — build_manager 가 이걸 주입.)
///
/// ★호출 컨텍스트: pump/manager 의 동기 스레드★ → 절대 block 금지. broadcast_text 가 try_send 만 쓴다.
pub struct DaemonStatusSink {
    registry: ConnRegistry,
}

impl DaemonStatusSink {
    pub fn new(registry: ConnRegistry) -> Self {
        Self { registry }
    }
}

impl StatusSink for DaemonStatusSink {
    fn status_changed(&self, id: AgentId, status: CoreStatus, epoch: u32) {
        let ev = AgentEvent::StatusChanged {
            agent_id: id,
            status: core_status_to_wire(status),
            epoch,
        };
        if let Some(text) = event_json(&ev) {
            self.registry.broadcast_text(text);
        }
    }

    fn agent_list_updated(&self, agents: Vec<CoreAgentInfo>) {
        let ev = AgentEvent::AgentListUpdated {
            agents: core_agents_to_wire(agents),
        };
        if let Some(text) = event_json(&ev) {
            self.registry.broadcast_text(text);
        }
    }

    fn restore_result(&self, report: CoreRestoreReport) {
        let ev = AgentEvent::RestoreResult {
            report: core_report_to_wire(report),
        };
        if let Some(text) = event_json(&ev) {
            self.registry.broadcast_text(text);
        }
    }
}

// ── MessagingFlushSink(등장/epoch flush 트리거, ADR-0104 — C1/C2) ──────────────────────────

/// flush worker 로 흐르는 작업 단위(C1 등장 flush + C2 idle 게이트 도어벨).
///
/// ★하나의 채널·하나의 소비자★: 두 종류를 따로 나르면 같은 에이전트의 등장과 턴 종료가 서로 앞질러
///   배선 추론이 어려워진다.
/// ★공통 계약★: 이 메시지들은 전부 **status 콜백/pump 콜백(블록 금지)** 에서 논블록으로 enqueue 되고,
///   실제 작업(messaging 락·blocking stdin write)은 worker 가 수행한다(finding 5 계열 규율).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlushMsg {
    /// 로스터 등장/epoch bump(그 이름의 도달 후보가 **유일**할 때만) — 그 **이름** 앞 파킹을 일괄 flush.
    Appear { name: String, id: AgentId },
    /// 턴 종료(idle 전이) 관측 — 그 id 의 파킹을 오래된 순 일괄 주입(C2 idle 게이트, ADR-0104 결정 3).
    Idle { id: AgentId },
}

/// ★Idle 통지 coalescer(C2 리뷰 fix 10)★ — 같은 id 의 **미처리** Idle 이 이미 큐에 있으면 새 enqueue 를 접는다.
///
/// ★왜 필요한가(유계 채널 압력)★: 통지는 MessageDone **마다** 나간다(누락 < 잉여 — busy.rs `IdleNotifier`).
///   에이전트가 도구 호출을 연달아 돌리면 짧은 시간에 MessageDone 이 여러 번 나올 수 있고, unbounded 채널
///   이라 그만큼 항목이 쌓인다(메모리·처리 낭비 — 대부분 빈 큐 no-op). flush 는 **큐 전체를 drain** 하므로
///   같은 id 의 Idle N개는 1개와 결과가 같다 → 접어도 의미가 보존된다.
/// ★lost wakeup 이 없는 이유(load-bearing)★: 코어가 "① 턴 관측 표 갱신 → ② 통지" 순서를 지키므로
///   (output_core.rs emit), 접힌 통지가 가리키는 상태 변화는 **아직 처리 안 된 그 Idle** 이 대표한다.
///   소비자는 **집어들 때 먼저 집합에서 지우고**(그 뒤에 게이트를 보고 flush) 처리하므로, 처리 도중 도착한
///   새 턴 종료 신호는 다시 enqueue 된다.
/// ★Appear 는 절대 접지 않는다★: 등장은 고유한 사건이라 무손실이어야 한다. 이 coalescer 는 **Idle 전용**이다.
#[derive(Debug, Default)]
pub struct IdleCoalescer {
    pending: Mutex<std::collections::HashSet<AgentId>>,
}

impl IdleCoalescer {
    pub fn new() -> Self {
        Self::default()
    }

    /// 이 id 의 Idle 을 enqueue 해야 하나(= 미처리 항목이 아직 없나). true 면 호출자가 send 한다.
    fn claim(&self, id: AgentId) -> bool {
        self.pending
            .lock()
            .expect("idle coalescer poisoned")
            .insert(id)
    }

    /// 소비자가 이 id 의 Idle 을 집어들었다 — 이후 도착하는 통지는 다시 enqueue 돼야 한다.
    fn taken(&self, id: AgentId) {
        self.pending
            .lock()
            .expect("idle coalescer poisoned")
            .remove(&id);
    }
}

/// 커널 → flush worker 통지 구현 — `IdleNotifier`(상한 sweep 의 깨우기)와 `FlushTrigger`(서비스 도어벨)를 **같은
///   채널**로 잇는다(C2). 둘 다 결과가 "그 id 의 파킹 큐를 flush" 라 메시지를 나눌 이유가 없다.
///
/// ★논블록 계약(load-bearing)★: `notify_idle` 은 **pump 스레드**에서, `request_flush` 는 발신(MCP/HTTP)
///   스레드에서 불린다. unbounded 채널의 `send` 는 논블록·무-await 라 그 계약을 만족한다. 채널이 닫혔으면
///   (worker 종료) 결과를 버린다 — 그 시점엔 데몬이 내려가는 중이고 파킹은 인메모리라 잃을 게 없다
///   (spec §0 영속화 없음).
pub struct ChannelIdleNotifier {
    tx: mpsc::UnboundedSender<FlushMsg>,
    coalescer: Arc<IdleCoalescer>,
}

impl ChannelIdleNotifier {
    pub fn new(tx: mpsc::UnboundedSender<FlushMsg>, coalescer: Arc<IdleCoalescer>) -> Self {
        Self { tx, coalescer }
    }

    /// 공통 경로 — coalescing 을 거쳐 Idle 을 enqueue 한다.
    fn enqueue(&self, id: AgentId) {
        if self.coalescer.claim(id) {
            let _ = self.tx.send(FlushMsg::Idle { id });
        }
    }
}

impl engram_dashboard_messaging::busy::IdleNotifier for ChannelIdleNotifier {
    fn notify_idle(&self, id: AgentId) {
        self.enqueue(id);
    }
}

impl engram_dashboard_messaging::service::FlushTrigger for ChannelIdleNotifier {
    fn request_flush(&self, id: AgentId) {
        self.enqueue(id);
    }
}

/// ★파킹 flush 트리거(ADR-0104 · S18 메시징 v1 C1)★: `DaemonStatusSink` 를 **감싸** 로스터 변화를
///   데몬측에서 관측하고, 새로 살아났거나 epoch 이 bump 된 이름 앞으로 파킹된 메시지를 flush 시킨다.
///
/// ★왜 sink 를 감싸나(core seam 무변경 — ADR-0104)★: 코어는 메시징을 몰라야 한다(격리 ADR-0028/0104).
///   AgentManager 의 상태 sink 가 이미 `agent_list_updated(Vec<AgentInfo>)` 로 로스터 스냅샷을 push 하므로
///   (ADR-0028 single-push broadcast), 그 사실을 **데몬측에서 diff** 해 flush 를 건다 — 코어에 새 seam 을
///   내지 않고 이미 흐르는 이벤트에 얹는다. wrap 이라 기존 broadcast(프론트 fanout)는 그대로 delegate 된다.
///
/// ★diff 규칙(load-bearing)★: 직전 스냅샷의 (name→epoch) 와 새 스냅샷을 비교해, **새로 등장**(이전에 없던
///   이름)하거나 **epoch bump**(같은 이름 epoch 증가 = 재스폰/재활성화)한 이름을 flush 대상으로 본다. 그
///   이름의 현재 id 로 `messaging.flush_for(name, id)` 를 부른다 — 서비스가 파킹 큐를 오래된 순 일괄 주입.
///   ★epoch 감소·동일은 flush 안 함★(같은 incarnation 재-push = 노이즈, 이미 flush 됐거나 busy).
///
/// ★flush 작업을 status-sink 콜백에서 분리(finding 5 · load-bearing)★: 예전엔 이 sink 가
///   `agent_list_updated` **안에서 동기적으로** `flush_for` 를 돌렸다 — 이 콜백은 manager/reaper 스레드가
///   부르는 block 금지 경로인데, 큰 배치가 여러 blocking write 를 마칠 때까지 로스터 이벤트 forwarding 이
///   막혀 spawn/reap/프론트 업데이트가 지연됐다. 이제 sink 는 **싼 diff 만** 하고 대상 (name, id) 를
///   unbounded 채널에 push 한 뒤 status 이벤트를 **즉시** forward 한다. 실제 flush 는 데몬 부팅 때 sweep
///   task 옆에 띄우는 전용 flush worker(종료 시 abort)가 채널을 소비하며 수행한다.
///
/// ★messaging 늦은 주입(순환)★: MessagingService 는 manager 를 감싸 manager 조립 후에야 만들어지므로,
///   sink 는 이 wrapper 생성 시점엔 아직 서비스를 모른다 → flush worker 가 `MessagingSlot`(OnceLock)로 받아
///   나중에 소비한다. set 전(부팅 초기 짧은 창)엔 worker 가 flush 를 건너뛴다(그 시점엔 파킹이 없으니 무해).
///
/// ★락 규율(ADR-0006)★: prev 스냅샷 Mutex 를 잡아 diff 대상(name,id)을 **수집**한 뒤 lock 을 놓고, 그 뒤
///   채널 send(락 미보유)만 한다 — prev lock 을 든 채 외부 호출 없음. 실제 flush(messaging 락 + port 호출)는
///   worker 스레드로 완전히 옮겨졌다.
/// ★호출 컨텍스트 = pump/manager 동기 스레드★: `agent_list_updated` 는 block 금지 경로다. 이제 여기선
///   diff(HashMap 조작) + unbounded send(논블록)만 하므로 배치 크기와 무관하게 짧다.
pub struct MessagingFlushSink {
    /// 감싼 실제 sink — status_changed/agent_list_updated/restore_result 를 그대로 delegate.
    /// ★Box<dyn>★: 운영은 DaemonStatusSink(프론트 broadcast), 통합 테스트는 NoopSink 를 감싸 flush 만
    ///   검증한다 — 감싼 대상이 무엇이든 diff/flush 로직은 동일하므로 trait object 로 받는다.
    inner: Box<dyn StatusSink>,
    /// flush 작업(FlushMsg)을 flush worker 로 보내는 채널(unbounded — status 콜백을 절대 막지 않게).
    ///   worker 미가동/드롭이어도 send 실패는 무시(파킹은 다음 등장에 재시도 — 무손실 유지).
    flush_tx: mpsc::UnboundedSender<FlushMsg>,
    /// 로스터 diff 시퀀싱 상태(스냅샷 + enqueue 직렬화).
    diff: RosterDiff,
    /// 턴 종료 push(코어 `StatusSink::turn_ended`)를 도어벨로 옮기는 출구 — coalescer 를 통지 측과 공유한다.
    idle: ChannelIdleNotifier,
}

/// ★로스터 diff 시퀀싱 상태(C2 리뷰 fix 7)★ — 직전 스냅샷을 락 아래 두고, 그 락을 **든 채로 채널
///   enqueue 까지** 끝낸다.
///
/// ★왜 락 보유 중 send 인가(load-bearing)★: 스냅샷을 락 안에서 갱신하고 락을 놓은 뒤 send 하면,
///   `agent_list_updated` 콜백 둘이 동시에 들어올 때(코어는 이 콜백의 직렬화를 보장하지 않는다) 스냅샷
///   갱신 순서와 enqueue 순서가 **갈릴 수 있다** — 옛 스냅샷이 만든 Appear 가 새 스냅샷의 것보다 뒤에
///   도착해 사라진 incarnation 으로 flush 를 건다. enqueue 를 락 안으로 넣으면 "스냅샷 순서 = 채널 순서"
///   가 구조적으로 보장된다. unbounded send 는 논블록이라 락 보유 구간이 여전히 짧다(콜백 blocking 금지
///   규율 유지).
#[derive(Debug, Default)]
pub struct RosterDiff {
    inner: Mutex<RosterSnapshots>,
}

#[derive(Debug, Default)]
struct RosterSnapshots {
    /// 직전 로스터 스냅샷(name→(epoch, id)). diff 로 newly-live/epoch-bump 를 판정(배달 축).
    prev: HashMap<String, (u32, AgentId)>,
}

impl RosterDiff {
    pub fn new() -> Self {
        Self::default()
    }

    /// 로스터 업데이트 1회 처리 — diff 를 계산해 **락 보유 중** 순서대로 enqueue 한다.
    fn dispatch(&self, agents: &[CoreAgentInfo], flush_tx: &mpsc::UnboundedSender<FlushMsg>) {
        let mut st = self.inner.lock().expect("flush roster diff poisoned");

        // 1) ★산(Running|Exiting) 후보 전원★을 **이름별로 그룹핑**한다. ★4차 개정(ADR-0116 결정 7)★:
        //   여기엔 **structured 조건을 걸지 않는다** — 로스터 자격에서 capability 가 빠졌으므로 턴 신호 없는
        //   세션도 파킹을 들고 있을 수 있다(그 부류의 유일한 파킹 경로 = **주입 실패**, spec §5 분기 3).
        //   그 조건을 되살리면 그 파킹분에 재등장 flush 계기가 **영원히 없어** 24h TTL 로 조용히 만료된다.
        // ★finding 2(BLOCK): 동명 다수 skip(last-write-wins 금지)★: 예전엔 같은 이름을 마지막 것으로
        //   덮어(last-write-wins) 임의 incarnation 으로 flush 했다 — 이름-키 파킹이 엉뚱한 동명 에이전트로
        //   갈 수 있어 send-side RECIPIENT_AMBIGUOUS 정책과 어긋난다. 이제 그 이름을 지닌 도달 가능
        //   후보가 **정확히 1개**일 때만 flush 대상으로 삼고, 동명 다수는 건너뛴다(tracing::debug) — 파킹
        //   메일은 그 이름이 다시 유일해지거나 TTL 로 만료될 때까지 대기한다.
        let mut by_name: HashMap<String, Vec<(u32, AgentId)>> = HashMap::new();
        for a in agents {
            // ★술어는 **커널 로스터 술어 그 자체**를 부른다(리뷰 fix N4)★: 인라인 `matches!` 복제본 +
            //   "같은 조건" 주석이었는데, 술어가 바뀔 때 한쪽만 고치면 발송 측(입구 판정)과 flush 측(이 diff)이
            //   다른 세계를 본다 — 정의 1곳(`messaging_host::is_live`)만 두고 여기선 호출만 한다.
            if !crate::messaging_host::is_live(a) {
                continue;
            }
            by_name
                .entry(a.name.clone())
                .or_default()
                .push((a.epoch, a.id));
        }
        // 2) 유일 이름만 다음 스냅샷·flush 후보로 승격. 동명 다수는 skip(prev 에도 안 남긴다 — 다시
        //   유일해지면 "새로 등장" 으로 잡혀 flush 되게).
        let mut next: HashMap<String, (u32, AgentId)> = HashMap::new();
        for (name, candidates) in by_name {
            if candidates.len() != 1 {
                tracing::debug!(
                    name = %name,
                    count = candidates.len(),
                    "flush skip: 동명 도달 후보 다수 — 유일해질 때까지 파킹 대기(finding 2)"
                );
                continue;
            }
            next.insert(name, candidates[0]);
        }
        for (name, (epoch, id)) in &next {
            // ★flush 트리거 조건(finding 3 — id 반영)★: ① 새로 등장(이전에 없던 이름/동명 해소로 재-유일)
            //   OR ② **id 변경**(같은 이름의 **다른** 에이전트 = 새 AgentId — 예: 같은 이름의 새 프로필)
            //   OR ③ 같은 id + epoch bump(같은 incarnation 재스폰/재활성화). ②가 load-bearing: 옛 diff 는
            //   이름별 epoch 만 비교해, id 가 다른데 epoch 이 이전 것보다 ≤ 이면(새 프로필 epoch 0 < 옛
            //   epoch 3) "새로 살아남" 을 놓쳐 그 이름 앞 파킹이 영영 stranded 됐다. id 가 바뀌면 그건
            //   별개 에이전트의 등장이니 epoch 대소와 무관하게 flush 후보다.
            let trigger = match st.prev.get(name) {
                None => true, // ① 새로 등장(또는 동명 해소로 다시 유일).
                Some((prev_epoch, prev_id)) => {
                    id != prev_id // ② 동명 다른 에이전트(새 AgentId) — epoch 대소와 무관.
                        || epoch > prev_epoch // ③ 같은 id + epoch bump(재스폰/재활성화).
                }
            };
            if trigger {
                let _ = flush_tx.send(FlushMsg::Appear {
                    name: name.clone(),
                    id: *id,
                });
            }
        }
        st.prev = next;
    }
}

impl MessagingFlushSink {
    /// 운영 생성자 — DaemonStatusSink 를 감싼다(프론트 broadcast delegate + flush 트리거). `flush_tx` 는
    ///   flush worker 로 이어지는 채널의 송신단이고, `idle` 은 flush 레인과 공유하는 Idle coalescer 다.
    pub fn new(
        inner: DaemonStatusSink,
        flush_tx: mpsc::UnboundedSender<FlushMsg>,
        idle: Arc<IdleCoalescer>,
    ) -> Self {
        Self::new_boxed(Box::new(inner), flush_tx, idle)
    }

    /// 테스트 생성자 — 임의 inner StatusSink(NoopSink 등)를 감싼다. flush 로직만 검증할 때.
    pub fn new_test(
        inner: Box<dyn StatusSink>,
        flush_tx: mpsc::UnboundedSender<FlushMsg>,
        idle: Arc<IdleCoalescer>,
    ) -> Self {
        Self::new_boxed(inner, flush_tx, idle)
    }

    fn new_boxed(
        inner: Box<dyn StatusSink>,
        flush_tx: mpsc::UnboundedSender<FlushMsg>,
        idle: Arc<IdleCoalescer>,
    ) -> Self {
        Self {
            idle: ChannelIdleNotifier::new(flush_tx.clone(), idle),
            inner,
            flush_tx,
            diff: RosterDiff::new(),
        }
    }
}

/// ★flush worker(finding 5)★: `MessagingFlushSink` 가 채널로 보낸 flush 대상 (name, id) 을 소비해 실제
///   `flush_for`(messaging 락 + inject 블로킹 write)를 수행하는 전용 tokio task. 데몬 부팅에서 sweep task
///   옆에 띄우고 종료 시 abort 한다(start_test_server_inner 도 동일 패턴). status-sink 콜백을 blocking write
///   에서 떼어내 spawn/reap/프론트 업데이트가 배치 flush 에 물리지 않게 한다.
///
/// ★spawn_blocking 격리(round-4 finding 1 — executor starvation)★: `flush_for` 안의 `inject` 는
///   transport.send_input = **동기 blocking write_all+flush**다(논블록 채널 send 가 아니라 실제 자식
///   stdin 파이프 write — pty.rs:302-308 / stdio.rs:322-332). 이걸 async task 본문에서 **직접** 부르면
///   그 blocking 이 runtime worker 스레드를 점유한다 — current-thread/단일 worker 런타임에선 이 한 task 가
///   executor 를 독점해 다른 task(종료 시 shutdown_all·5s join belt 등)가 폴링될 틈이 없다(실제로 통합
///   테스트가 이 굶주림 때문에 multi_thread 로 우회해야 했다). 그래서 각 flush 를 `spawn_blocking` 으로
///   던져 blocking pool 스레드에서 돌린다: (1) blocking write 는 runtime worker 가 아닌 blocking pool 을
///   점유하고 (2) abort 는 아래 `.await` 지점에서 즉시 먹으며 (3) 5s join belt·종료 task 가 계속 폴링돼
///   current-thread 런타임도 건강하게 유지된다.
///   ※주의(종료 순서는 그대로 load-bearing): spawn_blocking 클로저 자체는 abort 불가 — worker 의 .await 를
///     abort 해도 pool 스레드의 blocking write 는 자식이 stdin 을 비울(또는 파이프가 닫힐) 때까지 계속 돈다.
///     그래서 lib.rs 종료 경로의 "shutdown_all 로 자식 kill·파이프 닫기 → 그 다음 worker abort" 순서는
///     여전히 필수다(막힌 write 를 에러로 풀어 pool 스레드를 회수). 이 fix 가 고치는 건 executor 굶주림뿐.
///
/// ★slot 늦은 주입★: MessagingService 는 manager 조립 후 생기므로 worker 도 slot(OnceLock)로 받아, 대상이
///   와도 slot 미설정이면 건너뛴다(부팅 초기 짧은 창엔 파킹 없음 — 무해). 채널 닫힘(sink 드롭)이면 종료.
///
/// ★2-레인 파이프라인(C2 리뷰 fix 3 — head-of-line blocking 격리, load-bearing)★:
///   - **main lane(이 함수)** — 채널에서 꺼내 flush 레인으로 **forward** 만 한다(논블록). 여기가 막히지
///     않아야 status 콜백이 넣은 작업이 계속 흡수된다.
///   - **flush lane(`run_flush_lane`)** — 자체 task + 자체 채널. `spawn_blocking` 이 자식 stdin write 로
///     막히는 곳이 여기다. 레인 내부는 **여전히 직렬**이라 같은 수신자 배치 순서(오래된 순)는 보존된다
///     (병렬화하면 순서가 깨진다).
///
/// ★레인 task 는 **호출자(부팅)가 소유**한다 — 이 함수가 spawn 하지 않는다(round-3 finding 1, BLOCK)★:
///   레인을 이 future **안에서** spawn 하고 JoinHandle 을 지역 변수로 들면, 종료 경로가 main lane 을 abort
///   할 때 그 핸들이 **그냥 drop**(= detach, abort 아님)되므로 레인은 계속 살아 있고 lib.rs 의 5s join belt 는
///   **정작 blocking 작업이 없는** main lane 만 감시하게 된다(모든 blocking inject 는 레인에 있다) → 진짜
///   blocking 을 지닌 task 가 belt 밖에 남아 런타임 drop 이 종료 시점에 hang 할 수 있다. 그래서 두 task 를
///   **둘 다 호출자가 들고** 각각 abort + belt 로 내린다(`spawn_flush_worker` / `FlushWorkerHandles::shutdown`).
/// ★수명★: main lane 이 끝나면(또는 abort 되면) `lane_tx` 가 drop 되어 레인도 자연 종료한다 — 단 레인은
///   **큐에 남은 배달을 다 처리한 뒤** 끝나므로 즉시 멈추지 않는다. 그래서 종료 경로는 `shutdown_all`(자식
///   kill·파이프 닫기)로 막힌 write 를 먼저 풀고, main → lane 순으로 abort 한다(lib.rs 종료 주석).
pub async fn run_flush_worker(
    mut flush_rx: mpsc::UnboundedReceiver<FlushMsg>,
    lane_tx: mpsc::UnboundedSender<FlushMsg>,
) {
    while let Some(msg) = flush_rx.recv().await {
        match msg {
            other @ (FlushMsg::Appear { .. } | FlushMsg::Idle { .. }) => {
                // ★send 실패를 삼키지 않는다(round-3 finding 1)★: 레인이 죽었다면(패닉 등) 이 경로의 조용한
                //   `let _ =` 는 **모든 배달이 영구 정지**한 사실을 감춘다(파킹만 쌓이다 TTL 로 만료 —
                //   메시징 최악 실패 모드인데 로그 한 줄도 없다). 그래서 warn 으로 표면화한다. 여기서
                //   복구(레인 재기동)는 하지 않는다 — 레인은 개별 flush 의 패닉을 격리하므로(run_flush_lane,
                //   단 **debug 한정** — release 는 panic=abort 라 패닉 = 프로세스 종료다) 이 실패는
                //   "종료 중" 이거나 진짜 버그이고, 후자면 로그가 유일한 단서다.
                if let Err(e) = lane_tx.send(other) {
                    tracing::warn!("flush 레인 forward 실패(레인 종료/패닉 — 이후 배달 정지): {e}");
                }
            }
        }
    }
    tracing::debug!("flush worker(main lane) 종료(채널 닫힘)");
}

/// flush worker **2-레인 묶음** 핸들 — 부팅(운영 `run()` / 테스트 서버)이 두 task 를 함께 소유한다.
///
/// ★왜 묶음인가(round-3 finding 1)★: 레인이 detach 되면 종료 belt 가 무의미해진다(위 `run_flush_worker`
///   주석). 조립·종료를 한 타입에 모아 호출자가 **한쪽만 내리는 실수**를 구조적으로 못 하게 한다.
pub struct FlushWorkerHandles {
    /// 수신 레인 — 채널에서 꺼내 배달 레인으로 넘기기만 한다(blocking 작업 없음).
    main: tokio::task::JoinHandle<()>,
    /// 배달 레인(Appear/Idle) — 여기 `spawn_blocking` 이 자식 stdin write 로 막힐 수 있다.
    lane: tokio::task::JoinHandle<()>,
}

impl FlushWorkerHandles {
    /// 두 레인을 내린다 — **호출 전에 `shutdown_all`(자식 kill·파이프 닫기)이 끝나 있어야 한다**(순서가
    ///   load-bearing: lib.rs 종료 주석). 각 abort 뒤 join 을 5s belt 로 감싸, 예측 못 한 blocking 이
    ///   남아도 데몬 종료를 hang 시키지 않고 warn 후 detach 한다(프로세스 종료가 스레드를 회수).
    ///
    /// ★수용된 잔여(residual) — abort·belt 로 `spawn_blocking` **본문**을 끊을 수는 없다(round-4 finding 2)★:
    ///   abort 는 task 의 `.await` 지점에서만 먹으므로 blocking pool 스레드가 syscall 안에 있으면 그대로 돈다.
    ///   그래도 이 종료 경로가 hang 하지 않는 이유는 belt 가 아니라 **호출 순서**다:
    ///     ① 여기 오기 전에 `shutdown_all` 이 끝나 있다 → 자식이 kill 되고 stdin 파이프가 닫힌다 → 그 파이프에
    ///        막혀 있던 `inject`(동기 write_all+flush)가 **에러로 풀려** 클로저가 스스로 반환한다.
    ///     ② 배달 레인 밖에는 blocking 작업이 없다(main lane 은 forward 만 한다).
    ///   즉 **kill-first 순서가 실제 보증이고, abort + 5s belt 는 관측 장치**다 — belt 가 실제로 발화한다면
    ///   그건 위 두 전제 중 하나가 깨졌다는 신호(warn 로그)이고, 그러려면 kill 된 자식의 파이프 write 가 에러도
    ///   안 내고 영원히 blocking 하는 **병리적 OS 동작**이 필요하다. 그래서 여기서 더 강한 취소 수단(별도
    ///   프로세스·스레드 강제 종료 등)을 도입하지 않는다 — 비용은 크고 막는 실패는 가정상 존재하지 않는다.
    pub async fn shutdown(self) {
        // main 먼저 — abort 시 lane_tx 가 drop 되어 레인이 새 작업을 받지 않는다.
        self.main.abort();
        if tokio::time::timeout(FLUSH_JOIN_BELT, self.main)
            .await
            .is_err()
        {
            tracing::warn!("flush worker(main lane) 종료 {FLUSH_JOIN_BELT:?} 타임아웃 — detach");
        }
        // 그 다음 배달 레인. 여기 남은 배달은 버린다(파킹은 인메모리 — 프로세스와 함께 소멸, spec §0).
        self.lane.abort();
        if tokio::time::timeout(FLUSH_JOIN_BELT, self.lane)
            .await
            .is_err()
        {
            tracing::warn!("flush 레인 종료 {FLUSH_JOIN_BELT:?} 타임아웃 — detach(종료 hang 방지)");
        }
    }
}

/// flush worker 조립 — 배달 레인 채널·task 와 main lane task 를 함께 띄운다(운영·테스트 공용 단일 지점).
///
/// 레인 채널은 **unbounded** 다: forward 가 논블록이어야 한다(main lane 이 레인 진행을 기다리면 head-of-line
///   blocking 이 되살아난다 — `run_flush_worker` 헤더).
pub fn spawn_flush_worker(
    flush_rx: mpsc::UnboundedReceiver<FlushMsg>,
    wiring: FlushWiring,
) -> FlushWorkerHandles {
    let (lane_tx, lane_rx) = mpsc::unbounded_channel::<FlushMsg>();
    let lane = tokio::spawn(run_flush_lane(lane_rx, wiring.messaging, wiring.idle));
    let main = tokio::spawn(run_flush_worker(flush_rx, lane_tx));
    FlushWorkerHandles { main, lane }
}

/// 종료 join belt — abort 후 이 시간 안에 안 끝나면 warn 후 detach(데몬 종료 hang 방지, round-3 finding 1).
const FLUSH_JOIN_BELT: Duration = Duration::from_secs(5);

/// ★flush 레인(C2 리뷰 fix 3)★ — 배달 작업(Appear/Idle) 전용 **직렬** 소비자. 여기의 blocking write 가
///   막혀도 수신 레인은 계속 돌아 채널을 비운다.
///
/// ★직렬 유지가 load-bearing★: 같은 수신자의 배치는 "오래된 순" 을 지켜야 하므로(ADR-0104) 이 레인 안에서
///   병렬 실행하지 않는다. 서로 다른 수신자끼리도 직렬이라 한 막힌 수신자가 다른 수신자의 배달을 늦출 수는
///   있으나(수용된 잔여 — 사람 대화 수준 메시지율), 채널 수신 자체는 그 뒤에 서지 않는다.
async fn run_flush_lane(
    mut lane_rx: mpsc::UnboundedReceiver<FlushMsg>,
    messaging: Arc<crate::control::mcp_server::MessagingSlot>,
    idle: Arc<IdleCoalescer>,
) {
    while let Some(msg) = lane_rx.recv().await {
        match msg {
            FlushMsg::Appear { name, id } => {
                let Some(svc) = messaging.get() else {
                    // 서비스 미주입(부팅 초기) — 파킹이 없으니 스킵. 다음 등장 이벤트에 다시 온다.
                    continue;
                };
                // flush_for 의 inject 는 blocking write(자식 stdin) — runtime worker 를 굶기지 않게 blocking
                //   pool 로 던지고 그 완료를 await 한다(위 헤더 rationale). Arc·name 을 move-in.
                let svc = svc.clone();
                let join = tokio::task::spawn_blocking(move || svc.flush_for(&name, id));
                if let Err(e) = join.await {
                    // blocking task 실패(패닉 또는 취소). 레인은 죽지 않고 다음 대상으로 계속 — 한 flush 의
                    //   실패가 이후 배달을 막지 않게(유계 격리).
                    // ★release 빌드엔 **패닉 갈래가 없다**(리뷰 fix 9 — 옛 주석 보정)★: 워크스페이스
                    //   `[profile.release] panic = "abort"` 라 blocking task 가 패닉하면 프로세스가 즉시
                    //   죽는다(JoinError::is_panic 을 볼 기회 자체가 없다). 즉 이 격리가 실제로 작동하는
                    //   건 **debug/테스트 빌드**이고, release 에서 이 갈래는 사실상 abort(Cancelled) —
                    //   런타임 종료 중 — 뿐이다. "패닉해도 운영에서 레인이 살아남는다" 로 읽지 말 것.
                    tracing::warn!("flush blocking task 실패(레인 계속 — 패닉 격리는 debug 한정, release=abort): {e}");
                }
            }
            FlushMsg::Idle { id } => {
                // ★집어들 때 coalescing 집합에서 먼저 지운다(fix 10 — lost wakeup 방지)★: 이 flush 처리
                //   도중 도착하는 새 MessageDone 은 다시 enqueue 돼야 한다(그 턴 종료는 이 배치가 대표하지
                //   않는다). 지우는 시점이 flush **전**인 게 핵심이다.
                idle.taken(id);
                let Some(svc) = messaging.get() else {
                    continue;
                };
                // Appear 와 동일한 blocking 격리. 이름 해석은 서비스가 한다(flush_for_agent — 빈 큐 조기
                //   반환으로 잉여 idle 통지를 싸게 흡수).
                let svc = svc.clone();
                let join = tokio::task::spawn_blocking(move || svc.flush_for_agent(id));
                if let Err(e) = join.await {
                    // 위 Appear 갈래와 동일 — release 는 panic=abort 라 패닉 갈래가 도달 불가하다(fix 9).
                    tracing::warn!("idle flush blocking task 실패(레인 계속 — 패닉 격리는 debug 한정, release=abort): {e}");
                }
            }
        }
    }
    tracing::debug!("flush 레인 종료(채널 닫힘)");
}

/// flush worker 배선 묶음 — 인자 수를 줄이고 "무엇을 공유하는가" 를 한눈에 보이게 한다(부팅에서 조립).
#[derive(Clone)]
pub struct FlushWiring {
    /// MessagingService 늦은 주입 슬롯(manager 조립 후에 채워진다).
    pub messaging: Arc<crate::control::mcp_server::MessagingSlot>,
    /// Idle coalescing 집합 — 통지 측(`MessagingFlushSink`)과 공유(집어들 때 해제).
    pub idle: Arc<IdleCoalescer>,
}

impl StatusSink for MessagingFlushSink {
    fn status_changed(&self, id: AgentId, status: CoreStatus, epoch: u32) {
        self.inner.status_changed(id, status, epoch);
    }

    fn agent_list_updated(&self, agents: Vec<CoreAgentInfo>) {
        // ★로스터 diff → flush 작업 enqueue(C2 리뷰 fix 7)★: 스냅샷 갱신과 채널 enqueue 를 `RosterDiff` 가
        //   **한 락 아래에서** 한다(그 rationale 은 `RosterDiff` 주석). unbounded send 는 논블록이라 이
        //   콜백은 여전히 배치 크기와 무관하게 짧다(finding 5 — 실제 flush 는 worker 가 수행).
        self.diff.dispatch(&agents, &self.flush_tx);

        // 프론트 fanout 은 그대로 delegate(감싼 sink 의 본래 책임 — broadcast). flush 를 worker 로 뺐으므로
        //   이 forwarding 이 blocking write 뒤로 밀리지 않는다(spawn/reap/프론트 업데이트 지연 제거).
        self.inner.agent_list_updated(agents);
    }

    fn restore_result(&self, report: CoreRestoreReport) {
        self.inner.restore_result(report);
    }

    /// ★턴 종료 push → flush 도어벨(ADR-0113 결정 3 — 데몬은 중계만)★. 여기서 하는 일은 coalescing
    ///   판정 + 논블록 채널 send 뿐이다(그 계약은 `StatusSink::turn_ended`).
    // ADR-0113
    fn turn_ended(&self, id: AgentId, epoch: u32) {
        // epoch 을 도어벨에 싣지 않는 이유: flush 는 **에이전트 단위** 큐를 여는 동작이고, 어느 화신이
        //   끝났든 그 시점의 현재 화신에게 배달하는 게 맞다(메일은 논리 에이전트를 향한다 — ADR-0086 §F5).
        self.idle.enqueue(id);
        // ★감싼 sink 로도 반드시 흘린다(decorator 계약)★: 이 wrapper 가 데몬이 설치하는 **유일한**
        //   StatusSink 이고 이 훅은 기본 구현이 no-op 이라, 빠뜨리면 안쪽이 이 훅을 구현하는 날
        //   **컴파일 에러 없이** 조용히 죽는다. 턴 상태를 프론트/LLM 제어 표면으로 내보내는 경로가
        //   그 안쪽에 생길 예정이다(ADR-0113 §영향 — §5 정합).
        self.inner.turn_ended(id, epoch);
    }
}

// ── ConnFrameSink(연결당 프레임 출구 — 프레임 포트의 WS 구현) ──────────────────────

/// 한 연결의 단일 writer 큐를 `FrameSink` 로 노출한다. 위층(에이전트 시스템)은 이걸 통해서만
/// 내보내므로 프레임에 실린 어휘가 무엇이든 이 파일은 모른다.
///
/// ★R6 close_signal(out-of-band)★: `try_send` 가 큐 포화를 만나면 `FrameError` 를 돌려주면서
/// close_signal 을 notify 해 write_task 가 큐가 막혀도 깨어 닫게 한다(WS-특정 처리라 여기 잔류).
/// `send`(backpressure 허용)는 기다릴 수 있는 호출자이므로 신호하지 않는다 — 이 비대칭이 계약이다.
// ADR-0129
pub(crate) struct ConnFrameSink {
    conn_tx: mpsc::Sender<Frame>,
    /// 큐 밖 종료 신호. full 감지 시 notify_one — write_task 가 큐가 막혀도 깨어 닫는다.
    /// ★pump 스레드(sync)에서 notify_one 호출 OK — Notify 는 sync-safe.
    close_signal: Arc<Notify>,
}

impl ConnFrameSink {
    pub(crate) fn new(conn_tx: mpsc::Sender<Frame>, close_signal: Arc<Notify>) -> Self {
        Self {
            conn_tx,
            close_signal,
        }
    }
}

impl FrameSink for ConnFrameSink {
    fn try_send(&self, frame: Frame) -> Result<(), FrameError> {
        match self.conn_tx.try_send(frame) {
            Ok(()) => Ok(()),
            Err(_) => {
                self.close_signal.notify_one();
                Err(FrameError)
            }
        }
    }

    fn send(&self, frame: Frame) -> BoxFuture<'_, Result<(), FrameError>> {
        Box::pin(async move { self.conn_tx.send(frame).await.map_err(|_| FrameError) })
    }
}

// ── Origin allowlist 콜백 ─────────────────────────────────────────────────────────

/// upgrade 콜백 — Origin 헤더 검사. 없으면 허용(네이티브/하네스), 있고 allowlist 밖이면 거부.
struct OriginCheck;

impl Callback for OriginCheck {
    fn on_request(self, request: &Request, response: Response) -> Result<Response, ErrorResponse> {
        match request.headers().get("origin") {
            None => {
                // Origin 없음 = 네이티브/하네스 클라이언트. 토큰이 주 방어이므로 허용.
                tracing::debug!("WS upgrade: Origin 없음 — 허용(토큰 검증으로 방어)");
                Ok(response)
            }
            Some(value) => {
                let origin = value.to_str().unwrap_or("");
                if ALLOWED_ORIGINS.contains(&origin) {
                    tracing::debug!(origin, "WS upgrade: Origin 허용");
                    Ok(response)
                } else {
                    // ★TODO(실측)★: 실제 Tauri WebView2/모바일이 보내는 Origin 문자열을 실측해
                    // allowlist 를 확정할 것(설계값 기준). 불일치 = 거부.
                    tracing::warn!(origin, "WS upgrade: Origin 불일치 — 거부");
                    let mut resp = ErrorResponse::new(Some("origin not allowed".into()));
                    *resp.status_mut() = StatusCode::FORBIDDEN;
                    Err(resp)
                }
            }
        }
    }
}

// ── 상수시간 토큰 비교 ──────────────────────────────────────────────────────────

/// 토큰 상수시간 비교 — 길이 먼저(다르면 즉시 false), 같으면 바이트 XOR 누적으로
/// timing 부채널을 줄인다. 길이 노출은 토큰 길이가 고정(hex 64자)이라 무해.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// ── 연결 핸들러 ────────────────────────────────────────────────────────────────

/// 연결 1개의 전 수명을 처리한다. accept 된 raw TCP stream 을 받아:
/// WS 업그레이드 → auth → 핸들러 부착 → read/write task → cleanup.
///
/// `expected_token` 은 daemon.json 의 토큰. `handlers` 는 이 연결에 붙일 위층 핸들러 공장 —
/// 프레임의 의미(명령 해석·이벤트 인코딩·연결 정리)는 전부 그쪽이 소유하므로, 이 함수는 에이전트
/// 어휘를 auth 핸드셰이크 말고는 알지 못한다(ADR-0129).
// ADR-0129
pub async fn handle_connection(
    stream: TcpStream,
    peer: std::net::SocketAddr,
    registry: ConnRegistry,
    handlers: Arc<dyn ConnectionHandlerFactory>,
    expected_token: Arc<String>,
    keepalive: KeepaliveConfig,
) {
    // 1) WS 업그레이드 + Origin 검사.
    let mut ws = match tokio_tungstenite::accept_hdr_async(stream, OriginCheck).await {
        Ok(ws) => ws,
        Err(e) => {
            tracing::warn!(%peer, "WS 업그레이드 실패(또는 Origin 거부): {e}");
            return;
        }
    };

    // 2) 첫 frame(1초 내) → Auth 파싱 + 토큰 상수시간 비교 + 버전 검사.
    match tokio::time::timeout(AUTH_TIMEOUT, ws.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => {
            match serde_json::from_str::<AgentCommand>(&text) {
                Ok(AgentCommand::Auth {
                    token,
                    protocol_version,
                }) => {
                    // 토큰 비교(상수시간). 보안: 토큰 값은 로그 금지.
                    if !constant_time_eq(&token, expected_token.as_str()) {
                        tracing::warn!(%peer, "auth 실패: 토큰 불일치 — close");
                        let _ = send_error_and_close(
                            &mut ws,
                            handlers.handshake_error_frame("auth failed"),
                        )
                        .await;
                        return;
                    }
                    if protocol_version != PROTOCOL_VERSION {
                        tracing::warn!(
                            %peer,
                            client = protocol_version,
                            server = PROTOCOL_VERSION,
                            "auth 실패: protocol_version 불일치 — close"
                        );
                        let _ = send_error_and_close(
                            &mut ws,
                            handlers.handshake_error_frame(&format!(
                                "protocol_version mismatch: client {protocol_version} != server {PROTOCOL_VERSION}"
                            )),
                        )
                        .await;
                        return;
                    }
                    tracing::info!(%peer, "auth 성공");
                }
                Ok(_) => {
                    tracing::warn!(%peer, "첫 frame 이 Auth 가 아님 — close");
                    let _ = send_error_and_close(
                        &mut ws,
                        handlers.handshake_error_frame("expected Auth as first frame"),
                    )
                    .await;
                    return;
                }
                Err(e) => {
                    tracing::warn!(%peer, "첫 frame 파싱 실패: {e} — close");
                    let _ = send_error_and_close(
                        &mut ws,
                        handlers.handshake_error_frame("invalid first frame"),
                    )
                    .await;
                    return;
                }
            }
        }
        Ok(Some(Ok(_))) => {
            tracing::warn!(%peer, "첫 frame 이 Text 가 아님 — close");
            let _ = send_error_and_close(
                &mut ws,
                handlers.handshake_error_frame("expected Auth text frame"),
            )
            .await;
            return;
        }
        Ok(Some(Err(e))) => {
            tracing::warn!(%peer, "첫 frame 수신 오류: {e} — close");
            return;
        }
        Ok(None) => {
            tracing::warn!(%peer, "auth 전에 연결 종료");
            return;
        }
        Err(_) => {
            tracing::warn!(%peer, "auth 타임아웃(1s) — close");
            let _ =
                send_error_and_close(&mut ws, handlers.handshake_error_frame("auth timeout")).await;
            return;
        }
    }

    // 3) conn_tx/rx 생성 + close_signal + 레지스트리 등록 + split.
    let (conn_tx, conn_rx) = mpsc::channel::<Frame>(CONN_TX_CAP);
    // ★out-of-band 종료 신호★: 큐 포화로 `Frame::Close` 마저 못 들어갈 때 write_task 를 깨운다.
    let close_signal = Arc::new(Notify::new());
    let conn_id = registry.alloc_id();
    registry.register(conn_id, conn_tx.clone());
    tracing::info!(%peer, conn = conn_id, "연결 인증 완료 — 등록");

    let (sink_half, stream_half) = ws.split();

    // 3b) 프레임 출구 + 위층 핸들러 부착. 이 연결의 모든 출력은 frames(단일 writer 큐)로만 나가고,
    //     들어온 프레임의 의미 해석은 handler 가 소유한다(ADR-0129 — 이 함수는 어휘를 모른다).
    let frames: Arc<dyn FrameSink> =
        Arc::new(ConnFrameSink::new(conn_tx.clone(), close_signal.clone()));
    let handler = handlers.handler_for(conn_id);

    // 4) 연결 직후 인사·초기 상태 push(단일 writer 큐 경유 — 이후 모든 출력과 FIFO 정렬).
    //    ★소비자보다 먼저다★: write_task 는 아직 없으므로 여기서 넣은 프레임은 큐에 쌓이기만 한다
    //    (그 제약이 ConnectionHandler::on_connect 의 계약). 순서상 여기가 두 task 스폰보다 앞이어야
    //    명령 dispatch 가 인사보다 앞설 수 없다. ★단 "연결의 첫 프레임" 보장은 아니다★ — 등록이 이미
    //    끝났으므로 status fanout 이 그 사이 큐에 먼저 들어갔을 수 있다.
    handler.on_connect(conn_id, &frames).await;
    // ★관측만 하고 흐름은 바꾸지 않는다★: 패닉을 쓰지 않는 이유는 여기서 죽으면 아래 정리 훅과
    //   레지스트리 해제를 통째로 건너뛴 채 죽은 큐가 fanout 대상으로 남기 때문이다(HEAD 에 없던 종료 경로).
    // ★잡히는 범위 = "정확히 가득 찬 채로 on_connect 이 **반환한**" 경계 하나뿐★: 정작 위험한 쪽
    //   (용량을 넘겨 넣어 `send` 가 영구 대기)은 이 줄에 **도달조차 못 한다** — 그 hang 은 어디에도
    //   로그가 남지 않는다(알려진 미로그 구멍 — `ConnectionHandler::on_connect` 계약에 서술).
    // ★"지금 막혔다"는 뜻이 아니다★: 바로 아래에서 write_task 가 떠 큐를 비우므로 진행은 계속된다.
    if conn_tx.capacity() == 0 {
        tracing::warn!(
            conn = conn_id,
            "on_connect 반환 시점에 연결 큐가 가득 찼다 — 한 프레임만 더 넣었으면 writer 기동 전에 영구 대기했을 것(핸들러 푸시 + 등록 후 status fanout 합계)"
        );
    }

    // ── keepalive 공유 시계(A) ──────────────────────────────────────────────────────
    // base = 연결 시작 시각(tokio Instant). last_recv = base 기준 경과 ms(AtomicU64).
    // read_task 가 클라로부터 무언가(Pong 포함) 받을 때마다 갱신하고, write_task 의 ping arm 이
    // base.elapsed() - last_recv 로 idle 경과를 계산해 idle_timeout 초과 시 close_signal 발동.
    let keepalive_base = tokio::time::Instant::now();
    let last_recv = Arc::new(AtomicU64::new(0));

    // read_task: stream_half 에서 프레임을 읽어 handler 로 올린다. 응답은 handler 가 frames 로
    //   큐잉하므로 read_task 자신은 소켓에 직접 쓰지 않는다.
    let mut read_handle = tokio::spawn(read_task(
        stream_half,
        frames,
        handler.clone(),
        conn_id,
        keepalive_base,
        last_recv.clone(),
    ));

    // write_task: conn_rx 에서 받은 프레임을 sink_half 로 순서대로 write(단일 writer).
    //   close_signal 발동 시 큐가 막혀 있어도 깨어 닫는다(좀비 방지). keepalive Ping 도 여기서 송신.
    let mut write_handle = tokio::spawn(write_task(
        sink_half,
        conn_rx,
        conn_id,
        close_signal,
        keepalive,
        keepalive_base,
        last_recv,
    ));

    // 5) 하나라도 끝나면 cleanup. ★살아남은 쪽을 명시적으로 abort★ — JoinHandle 을 그냥 drop 하면
    //    task 가 detach 되어 계속 돈다(WS half 를 붙든 채 좀비). 그래서 &mut 로 select 해 핸들을
    //    소비하지 않고, 진 쪽을 abort 한다(연결의 read/write 가 함께 끝나게).
    //    회귀 방어 = `the_losing_task_is_aborted_not_detached`(read 를 abort 하는 갈래만).
    //    ★write 를 abort 하는 갈래도 abort 를 제거하지 말 것★: 대개는 송신단이 모두 드롭돼 write_task 가
    //    스스로 끝나지만, 그건 "모든 Sender<Frame> 사본이 함께 죽는다" 는 조건부다 — 구독 기록 누락
    //    (아래 on_disconnect 경쟁)으로 사본이 살아남으면 자기종료가 성립하지 않는다. 전수 열거와 실측
    //    범위는 그 테스트 주석에 있다.
    tokio::select! {
        _ = &mut read_handle => {
            tracing::debug!(conn = conn_id, "read_task 종료 → write_task abort + cleanup");
            write_handle.abort();
        }
        _ = &mut write_handle => {
            tracing::debug!(conn = conn_id, "write_task 종료 → read_task abort + cleanup");
            read_handle.abort();
        }
    }

    // ── cleanup(누수 방지 — 리뷰 필수) ──────────────────────────────────────────
    // 위층 정리(구독 해제·viewport 재협상·lease 반납)가 **먼저**, 레지스트리 제거가 **나중**이다.
    // ★이 순서에 배달 정합성이 걸려 있지는 않다★: 브로드캐스트는 맵 스냅샷으로 돌아 다른 연결이
    // 받는 것은 순서와 무관하고, 이 연결 자신의 몫은 배달을 전제할 수 없다(writer 가 먼저 끝난
    // 갈래면 확실히 버려지고, reader 가 먼저 끝난 갈래면 abort 가 먹기 전에 나갈 수도 있다 —
    // ConnectionHandler::on_disconnect 문서). 순서를 지키는 이유는 순수 리팩터로 두려는 것뿐이다.
    // ★위 abort 는 완료를 기다리지 않는다★: 취소된 read_task 가 아직 핸들러 호출 안에 있을 수 있어
    // on_disconnect 와 겹칠 수 있다(HEAD 도 동일한 잔여 경쟁 — ConnectionHandler 문서).
    handler.on_disconnect(conn_id);

    registry.unregister(conn_id);
    tracing::info!(%peer, conn = conn_id, "연결 종료 — cleanup 완료");
}

/// 핸드셰이크 실패 통보 + close 를 소켓에 직접 쓴다(레지스트리 등록 전이라 단일 writer 큐가 없다).
/// `frame` 은 위층이 인코딩한 불투명 text — None 이면 본문 없이 close 만 한다.
async fn send_error_and_close(
    ws: &mut tokio_tungstenite::WebSocketStream<TcpStream>,
    frame: Option<String>,
) -> Result<(), tokio_tungstenite::tungstenite::Error> {
    if let Some(text) = frame {
        ws.send(Message::Text(text.into())).await?;
    }
    ws.close(None).await
}

// ── write_task(단일 writer) ───────────────────────────────────────────────────

type SinkHalf =
    futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<TcpStream>, Message>;

/// conn_rx 에서 받은 출력을 sink_half 로 순서대로 write. 이게 이 연결의 유일한 writer.
/// 종료 트리거 3가지: (1) 큐 안의 `Frame::Close`, (2) sink send 실패, (3) close_signal.
///
/// ★out-of-band 종료(M1 핵심)★: conn_tx 가 full 이면 `Frame::Close` 마저 큐에 못 들어가
/// 좀비 연결이 된다. 그래서 `tokio::select!` 로 conn_rx.recv() 와 close_signal.notified() 를
/// 동시에 대기한다. `ConnFrameSink` 가 try_send 에서 full 을 만나 `close_signal.notify_one()` 하면, 큐가
/// 가득 차 있어도 이 select 가 깨어 sink_half.close() 후 break → cleanup 으로 이어진다.
///
/// ★알려진 구멍 — 진행 중인 소켓 write 는 이 신호로 끊기지 않는다(HEAD 도 동일, 이 슬라이스 범위 밖)★:
/// recv arm 은 프레임을 꺼낸 **뒤** `sink_half.send(msg).await` 를 **select! 밖**(arm 본문)에서 기다린다.
/// 인증된 피어가 읽기를 멈추면 그 send 가 무기한 pending 일 수 있고, 그 동안 이 task 는 select! 를
/// 폴링하지 않으므로 `close_signal` 도 keepalive tick 도 그 연결을 구할 수 없다 — 같은 select! 안에
/// 있어서 둘 다 함께 멈춘다. 즉 코드가 광고하는 "슬로우 소비자 정리" 는 **이 경우를 덮지 못한다**.
/// `Notify` 는 대기자가 없을 때 permit 을 보관하므로 깨우기는 **유실이 아니라 지연**이다(그 send 가
/// 언젠가 풀리면 즉시 발화). 고치려면 write 자체에 타임아웃/취소를 걸어야 하는데 그건 동작 변경이다.
#[allow(clippy::too_many_arguments)]
async fn write_task(
    mut sink_half: SinkHalf,
    mut conn_rx: mpsc::Receiver<Frame>,
    conn_id: ConnId,
    close_signal: Arc<Notify>,
    keepalive: KeepaliveConfig,
    keepalive_base: tokio::time::Instant,
    last_recv: Arc<AtomicU64>,
) {
    // ★keepalive Ping 주기(A)★: ping_interval 마다 능동 Ping 을 보낸다(half-open 감지).
    //   tick 마다 마지막 수신 후 경과가 idle_timeout 초과면 close_signal 로 이 연결을 끊는다.
    let mut ping_tick = tokio::time::interval(keepalive.ping_interval);
    // 첫 tick 즉발 방지(연결 직후 바로 Ping 쏘지 않게) — 정상 첫 주기부터.
    ping_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            // 큐 밖 종료 신호 — full 로 큐가 막혀 있어도 여기로 깨어 닫는다.
            _ = close_signal.notified() => {
                tracing::info!(conn = conn_id, "write_task: close_signal(슬로우 소비자) — 종료");
                let _ = sink_half.close().await;
                break;
            }
            // keepalive: 주기적 능동 Ping + idle 판정.
            _ = ping_tick.tick() => {
                // idle 판정: 마지막 클라 수신(Pong 또는 임의 메시지) 이후 경과.
                let now_ms = keepalive_base.elapsed().as_millis() as u64;
                let last_ms = last_recv.load(Ordering::Acquire);
                let idle = Duration::from_millis(now_ms.saturating_sub(last_ms));
                if idle >= keepalive.idle_timeout {
                    // half-open 추정 — Pong 미응답이 idle_timeout 넘김. 이 연결을 닫는다.
                    tracing::info!(
                        conn = conn_id,
                        idle_ms = idle.as_millis() as u64,
                        "write_task: keepalive idle_timeout 초과(half-open 추정) — 종료"
                    );
                    let _ = sink_half.close().await;
                    break;
                }
                // 능동 Ping 송신. 실패(소켓 닫힘)면 종료.
                if let Err(e) = sink_half.send(Message::Ping(Vec::new().into())).await {
                    tracing::debug!(conn = conn_id, "write_task keepalive Ping 송신 실패 — 종료: {e}");
                    break;
                }
            }
            recv = conn_rx.recv() => {
                let Some(out) = recv else {
                    // 모든 conn_tx drop — 정상 종료.
                    break;
                };
                let msg = match out {
                    Frame::Text(s) => Message::Text(s.into()),
                    Frame::Binary(b) => Message::Binary(b.into()),
                    Frame::Close(reason) => {
                        tracing::info!(conn = conn_id, %reason, "write_task: close 신호 — 종료");
                        let _ = sink_half.close().await;
                        break;
                    }
                };
                if let Err(e) = sink_half.send(msg).await {
                    tracing::debug!(conn = conn_id, "write_task send 실패 — 종료: {e}");
                    break;
                }
            }
        }
    }
    tracing::debug!(conn = conn_id, "write_task 루프 종료");
}

// ── read_task ────────────────────────────────────────────────────────────────

/// 수신 프레임을 위층 `ConnectionHandler` 로 올린다. 응답은 handler 가 `FrameSink` 로 큐잉하므로
/// 이 루프는 소켓에 직접 쓰지 않는다. handler 가 `ConnFlow::Close` 를 돌려주면 루프를 탈출한다
/// (StopDaemon·프로토콜 위반).
///
/// ★stream 이 generic 인 이유★: 소켓 없이 합성 프레임열로 이 루프를 돌리는 격리 하네스를 두려고
/// (ADR-0129 — 이 seam 이 뒤 슬라이스의 crate 분리 검증 근거다). 운영 경로는 WS stream half 로만
/// 단형화된다.
async fn read_task<S>(
    mut incoming: S,
    frames: Arc<dyn FrameSink>,
    handler: Arc<dyn ConnectionHandler>,
    conn_id: ConnId,
    keepalive_base: tokio::time::Instant,
    last_recv: Arc<AtomicU64>,
) where
    S: Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin + Send,
{
    while let Some(item) = incoming.next().await {
        let msg = match item {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!(conn = conn_id, "read_task 수신 오류 — 종료: {e}");
                break;
            }
        };
        // ★keepalive(A)★: 클라로부터 무언가 받았다 = 연결이 살아있다는 증거. Pong 포함 모든
        //   메시지에서 마지막 수신 시각을 갱신한다(write_task 의 idle 판정 분모). tungstenite 는
        //   Pong 을 Message::Pong 으로 올려주므로 능동 Ping 의 응답도 여기서 잡힌다.
        last_recv.store(
            keepalive_base.elapsed().as_millis() as u64,
            Ordering::Release,
        );
        match msg {
            Message::Text(text) => {
                if handler.on_text(conn_id, &text, &frames).await == ConnFlow::Close {
                    break;
                }
            }
            Message::Binary(payload) => {
                // 페이로드는 **빌려준다** — 거부 경로가 유일한 소비자라 복사하지 않는다(클라가 고른
                //   크기만큼 할당하게 두면 인증 후 최대 프레임 크기까지 낭비 할당이 된다).
                if handler.on_binary(conn_id, &payload, &frames).await == ConnFlow::Close {
                    break;
                }
            }
            // Ping/Pong 은 tungstenite 가 자동 응답(write_task 가 아닌 내부). 여기선 무시.
            Message::Ping(_) | Message::Pong(_) => {}
            Message::Close(_) => {
                tracing::debug!(conn = conn_id, "Close frame 수신 — 종료");
                break;
            }
            Message::Frame(_) => {}
        }
    }
    tracing::debug!(conn = conn_id, "read_task 루프 종료");
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 1. Auth 직렬화 roundtrip ────────────────────────────────────────────
    #[test]
    fn auth_command_roundtrip() {
        let cmd = AgentCommand::Auth {
            token: "deadbeef".repeat(8), // 64자
            protocol_version: PROTOCOL_VERSION,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: AgentCommand = serde_json::from_str(&json).unwrap();
        match back {
            AgentCommand::Auth {
                token,
                protocol_version,
            } => {
                assert_eq!(token, "deadbeef".repeat(8));
                assert_eq!(protocol_version, PROTOCOL_VERSION);
            }
            _ => panic!("Auth 가 아님"),
        }
    }

    // (kind_to_action 매핑 테스트는 connection_core.rs 로 이동 — 함수가 거기 있음.)

    // ── 2. 토큰 상수시간 비교 정확성 ──────────────────────────────────────────
    #[test]
    fn constant_time_eq_correctness() {
        let a = "a".repeat(64);
        assert!(constant_time_eq(&a, &"a".repeat(64)), "동일 토큰은 true");
        assert!(!constant_time_eq(&a, &"b".repeat(64)), "다른 토큰은 false");
        // 길이 다르면 즉시 false.
        assert!(!constant_time_eq(&a, &"a".repeat(63)));
        assert!(!constant_time_eq(&a, &"a".repeat(65)));
        // 한 바이트만 달라도 false.
        let mut almost = "a".repeat(64);
        almost.replace_range(63..64, "b");
        assert!(!constant_time_eq(&a, &almost));
        // 빈 문자열 동일.
        assert!(constant_time_eq("", ""));
    }

    // ── 3. Frame 매핑(Text/Binary/Close → Message) ───────────────────────────
    // write_task 의 변환 로직과 동일한 매핑을 직접 검증(실제 WS 없이).
    #[test]
    fn frame_maps_to_message() {
        let t = Frame::Text("hi".into());
        let b = Frame::Binary(vec![1, 2, 3]);
        let c = Frame::Close("bye".into());

        let to_msg = |o: Frame| -> Message {
            match o {
                Frame::Text(s) => Message::Text(s.into()),
                Frame::Binary(b) => Message::Binary(b.into()),
                Frame::Close(_) => Message::Close(None),
            }
        };
        assert!(matches!(to_msg(t), Message::Text(_)));
        assert!(matches!(to_msg(b), Message::Binary(_)));
        assert!(matches!(to_msg(c), Message::Close(_)));
    }

    // ── 4. ConnFrameSink: try_send 는 포화 시 out-of-band close_signal 을 울린다 ──
    #[tokio::test]
    async fn conn_frame_sink_notifies_close_signal_when_full() {
        // cap 1 채널을 가득 채운 뒤: try_send 가 Err 를 반환하고, 큐가 막혀 있어도 close_signal 이
        // 발동(write_task 를 깨움)하는지 — 이게 좀비 연결을 막는 유일한 경로(M1).
        let (tx, mut rx) = mpsc::channel::<Frame>(1);
        let close_signal = Arc::new(Notify::new());
        let sink = ConnFrameSink::new(tx, close_signal.clone());

        sink.try_send(Frame::Text("first".into()))
            .expect("빈 큐엔 들어간다");
        assert!(
            sink.try_send(Frame::Text("second".into())).is_err(),
            "full 이면 FrameError"
        );

        tokio::time::timeout(Duration::from_millis(200), close_signal.notified())
            .await
            .expect("close_signal 이 full 에서도 발동해야 함");

        assert!(matches!(rx.recv().await.unwrap(), Frame::Text(_)));
    }

    // ── 4b. ConnFrameSink: send(backpressure)는 close_signal 을 울리지 않는다 ──
    #[tokio::test]
    async fn conn_frame_sink_send_does_not_signal_close() {
        // ★try_send / send 비대칭★: 기다릴 수 있는 호출자는 슬로우 소비자 판정 대상이 아니다.
        //   cap 1 을 채운 뒤 send 가 **실제로 포화에 park 했음을 먼저 확정**하고, 그 상태에서 종료
        //   신호가 없었음을 본 다음 한 칸 비워 통과를 확인한다.
        let (tx, mut rx) = mpsc::channel::<Frame>(1);
        let close_signal = Arc::new(Notify::new());
        let sink = ConnFrameSink::new(tx, close_signal.clone());

        sink.try_send(Frame::Text("first".into())).expect("한 칸");

        // ★포화 경로를 탔다는 **양성 관측**★: future 를 직접 1회 폴링해 Pending 을 확인한다. 이게
        //   없으면 `spawn` + 곧바로 `recv()` 조합에서 send 가 **자리가 빈 뒤에야** 처음 폴링될 수 있어
        //   (spawn 은 폴링을 보장하지 않고, 비어있지 않은 채널의 recv 는 yield 없이 Ready) 정작 검증
        //   대상인 포화 분기를 한 번도 안 타고 통과할 수 있다(실측: 그 형태는 회귀를 놓쳤다).
        let mut pending = Box::pin(sink.send(Frame::Text("second".into())));
        assert!(
            futures_util::poll!(pending.as_mut()).is_pending(),
            "가득 찬 큐에서 send 는 반드시 park 해야(포화 경로 미실행이면 이 테스트가 무의미하다)"
        );
        // park 한 그 상태에서 종료 신호가 없어야 한다(try_send 와 갈리는 지점).
        assert!(
            tokio::time::timeout(Duration::from_millis(50), close_signal.notified())
                .await
                .is_err(),
            "send 는 포화를 기다릴 뿐 종료 신호를 울리지 않는다"
        );

        // 한 칸 비우면 park 이 풀려 통과한다.
        assert!(matches!(rx.recv().await.unwrap(), Frame::Text(_)));
        pending.await.expect("자리가 나면 backpressure 가 풀린다");
    }

    // ── 4c. ConnFrameSink: 세 프레임 종류가 그대로 단일 writer 큐에 FIFO 로 실린다 ──
    #[tokio::test]
    async fn conn_frame_sink_maps_frames_to_queue_in_order() {
        let (tx, mut rx) = mpsc::channel::<Frame>(8);
        let sink = ConnFrameSink::new(tx, Arc::new(Notify::new()));
        sink.send(Frame::Text("hi".into())).await.expect("text ok");
        sink.try_send(Frame::Binary(vec![1, 2, 3]))
            .expect("binary ok");
        sink.send(Frame::Close("bye".into()))
            .await
            .expect("close ok");

        assert!(matches!(rx.recv().await.unwrap(), Frame::Text(_)));
        assert!(matches!(rx.recv().await.unwrap(), Frame::Binary(_)));
        match rx.recv().await.unwrap() {
            Frame::Close(r) => assert_eq!(r, "bye"),
            other => panic!("Close 여야 함: {other:?}"),
        }
    }

    // ── 6. Subscribe 시 conn_tx 에 SubscribeAck → ReplayComplete 순서로 들어가는지 ──
    //    (mock manager 가 없어 실 AgentManager 의 비어있는 snapshot 경로로는 NotFound 가 나므로,
    //     여기선 control 메시지 순서 로직을 직접 재현해 검증한다. 실 manager subscribe 의 replay
    //     동기 전송은 output_core.rs 단위테스트가 이미 커버.)
    #[tokio::test]
    async fn subscribe_control_order_ack_then_complete() {
        use engram_dashboard_protocol::{encode_terminal_frame, SubscribeAction};
        let (tx, mut rx) = mpsc::channel::<Frame>(16);
        let agent_id = uuid::Uuid::new_v4();

        // handle_subscribe 가 보내는 control 순서를 직접 재현(SubscribeAck → [replay binary] → ReplayComplete).
        let ack = event_json(&AgentEvent::SubscribeAck {
            agent_id,
            action: SubscribeAction::Reset,
            current_epoch: 0,
            oldest_seq: 0,
            latest_seq: 0,
            replay_from: 0,
            truncated: false,
        })
        .unwrap();
        tx.send(Frame::Text(ack)).await.unwrap();
        // 가상의 replay binary 1건.
        tx.send(Frame::Binary(encode_terminal_frame(agent_id, 0, 0, b"r")))
            .await
            .unwrap();
        let complete = event_json(&AgentEvent::ReplayComplete { agent_id, epoch: 0 }).unwrap();
        tx.send(Frame::Text(complete)).await.unwrap();

        // 순서 검증: Text(SubscribeAck) → Binary(replay) → Text(ReplayComplete).
        let first = rx.recv().await.unwrap();
        let second = rx.recv().await.unwrap();
        let third = rx.recv().await.unwrap();

        match first {
            Frame::Text(s) => assert!(s.contains("SubscribeAck")),
            _ => panic!("1번째는 SubscribeAck Text 여야 함"),
        }
        assert!(matches!(second, Frame::Binary(_)), "2번째는 replay binary");
        match third {
            Frame::Text(s) => assert!(s.contains("ReplayComplete")),
            _ => panic!("3번째는 ReplayComplete Text 여야 함"),
        }
    }

    // ── 7. MessagingFlushSink diff/enqueue 로직(finding 2·5) — worker 없이 순수 diff 검증 ──────────
    //    sink 는 flush 대상 (name, id) 만 채널로 push 한다(실제 flush 는 worker). 여기선 그 diff 를
    //    직접 단언한다: 유일 이름만 enqueue, 동명 다수는 skip(finding 2), 콜백은 blocking 없이 즉시 반환.
    use engram_dashboard_core::agent::types::{
        AgentInfo as TAgentInfo, Capabilities, ControlCaps, InputCaps, ModelCaps, OutputCaps,
        SessionCaps,
    };

    /// 테스트용 AgentInfo — 이름·epoch·structured(도달성)·상태를 지정.
    fn flush_info(
        id: AgentId,
        name: &str,
        epoch: u32,
        structured: bool,
        status: CoreStatus,
    ) -> TAgentInfo {
        TAgentInfo {
            id,
            name: name.to_string(),
            cwd: ".".to_string(),
            status,
            cols: 80,
            rows: 24,
            epoch,
            capabilities: Capabilities {
                input: InputCaps {
                    raw: true,
                    message: false,
                    attachment: false,
                },
                output: OutputCaps {
                    terminal_bytes: !structured,
                    structured,
                    markdown: false,
                    tool_events: false,
                    usage: false,
                },
                control: ControlCaps {
                    resize: false,
                    interrupt: false,
                    cancel: false,
                    graceful_shutdown: false,
                },
                session: SessionCaps {
                    resume: false,
                    snapshot: false,
                    cwd_env: false,
                },
                model: ModelCaps {
                    select: false,
                    temperature: false,
                    max_tokens: false,
                },
            },
        }
    }

    /// 테스트용 no-op inner StatusSink — broadcast 는 무관(diff 만 검증). 3 콜백 모두 아무것도 안 한다.
    struct TestNoopSink;
    impl StatusSink for TestNoopSink {
        fn status_changed(&self, _: AgentId, _: CoreStatus, _: u32) {}
        fn agent_list_updated(&self, _: Vec<CoreAgentInfo>) {}
        fn restore_result(&self, _: CoreRestoreReport) {}
    }

    /// flush 작업만 뽑는 sink + 그 채널 수신단을 만든다(worker 미배선 — diff 만 관측).
    fn flush_sink() -> (MessagingFlushSink, mpsc::UnboundedReceiver<FlushMsg>) {
        let (tx, rx) = mpsc::unbounded_channel::<FlushMsg>();
        let sink = MessagingFlushSink::new_test(
            Box::new(TestNoopSink),
            tx,
            Arc::new(IdleCoalescer::new()),
        );
        (sink, rx)
    }

    /// 채널에 쌓인 모든 작업을 순서대로 뽑는다(Appear/Idle 전부).
    fn drain_msgs(rx: &mut mpsc::UnboundedReceiver<FlushMsg>) -> Vec<FlushMsg> {
        let mut out = Vec::new();
        while let Ok(t) = rx.try_recv() {
            out.push(t);
        }
        out
    }

    /// 이름 축 flush 대상(Appear)만 추린다 — C1 diff 단언을 그대로 유지하기 위한 필터.
    fn drain_targets(rx: &mut mpsc::UnboundedReceiver<FlushMsg>) -> Vec<(String, AgentId)> {
        drain_msgs(rx)
            .into_iter()
            .filter_map(|m| match m {
                FlushMsg::Appear { name, id } => Some((name, id)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn flush_sink_enqueues_newly_live_unique_name() {
        // 새로 등장한 유일 도달 이름 → flush 대상으로 enqueue.
        let (sink, mut rx) = flush_sink();
        let id = AgentId::new_v4();
        sink.agent_list_updated(vec![flush_info(id, "alice", 0, true, CoreStatus::Running)]);
        let targets = drain_targets(&mut rx);
        assert_eq!(
            targets,
            vec![("alice".to_string(), id)],
            "유일 등장 이름 flush"
        );
    }

    #[test]
    fn flush_sink_skips_ambiguous_name() {
        // ★finding 2(BLOCK) 회귀★: 같은 이름 도달 후보 2개 → skip(last-write-wins 금지). 임의 incarnation
        //   으로 flush 하지 않는다 — send-side RECIPIENT_AMBIGUOUS 정책과 정합.
        let (sink, mut rx) = flush_sink();
        let a = AgentId::new_v4();
        let b = AgentId::new_v4();
        sink.agent_list_updated(vec![
            flush_info(a, "dup", 0, true, CoreStatus::Running),
            flush_info(b, "dup", 0, true, CoreStatus::Running),
        ]);
        assert!(
            drain_targets(&mut rx).is_empty(),
            "동명 다수는 flush 대상에서 제외(임의 incarnation 배달 금지)"
        );
    }

    #[test]
    fn flush_sink_reflushes_when_name_becomes_unique_again() {
        // 동명(skip) → 하나가 사라져 유일해지면 "새로 등장" 으로 다시 flush 대상(파킹 대기 해소).
        let (sink, mut rx) = flush_sink();
        let a = AgentId::new_v4();
        let b = AgentId::new_v4();
        // 1) 동명 다수 — skip, prev 에도 안 남는다.
        sink.agent_list_updated(vec![
            flush_info(a, "dup", 0, true, CoreStatus::Running),
            flush_info(b, "dup", 0, true, CoreStatus::Running),
        ]);
        assert!(drain_targets(&mut rx).is_empty());
        // 2) b 사라짐 → dup 유일 → 새로 등장으로 flush.
        sink.agent_list_updated(vec![flush_info(a, "dup", 0, true, CoreStatus::Running)]);
        assert_eq!(
            drain_targets(&mut rx),
            vec![("dup".to_string(), a)],
            "동명 해소로 유일해지면 다시 flush 대상"
        );
    }

    #[test]
    fn flush_sink_enqueues_on_epoch_bump_but_not_same_epoch() {
        // epoch bump(재스폰) → flush. 같은 epoch 재-push → skip(노이즈).
        let (sink, mut rx) = flush_sink();
        let id = AgentId::new_v4();
        sink.agent_list_updated(vec![flush_info(id, "a", 0, true, CoreStatus::Running)]);
        assert_eq!(drain_targets(&mut rx), vec![("a".to_string(), id)]);
        // 같은 epoch 재-push → flush 안 함.
        sink.agent_list_updated(vec![flush_info(id, "a", 0, true, CoreStatus::Running)]);
        assert!(
            drain_targets(&mut rx).is_empty(),
            "같은 epoch 재-push 는 flush 안 함"
        );
        // epoch bump → flush.
        sink.agent_list_updated(vec![flush_info(id, "a", 1, true, CoreStatus::Running)]);
        assert_eq!(
            drain_targets(&mut rx),
            vec![("a".to_string(), id)],
            "epoch bump 은 flush(재스폰/재활성화)"
        );
    }

    #[test]
    fn flush_sink_enqueues_when_same_name_different_id_lower_epoch() {
        // ★finding 3 회귀★: 같은 이름의 **다른** 에이전트(새 AgentId)가 등장하면, 그 epoch 이 이전
        //   incarnation 보다 낮아도(새 프로필 epoch 0 < 옛 epoch 3) flush 대상이어야 한다. 옛 diff 는
        //   이름별 epoch 만 비교해 이 등장을 놓쳐 파킹이 stranded 됐다. id 변경을 감지해 flush 를 건다.
        let (sink, mut rx) = flush_sink();
        let old = AgentId::new_v4();
        let new = AgentId::new_v4();
        // 1) 옛 에이전트 epoch 3 등장 → flush.
        sink.agent_list_updated(vec![flush_info(old, "svc", 3, true, CoreStatus::Running)]);
        assert_eq!(drain_targets(&mut rx), vec![("svc".to_string(), old)]);
        // 2) 같은 이름의 다른 에이전트(new id) epoch 0 등장(옛 것은 사라짐) → epoch 이 낮아도 flush.
        sink.agent_list_updated(vec![flush_info(new, "svc", 0, true, CoreStatus::Running)]);
        assert_eq!(
            drain_targets(&mut rx),
            vec![("svc".to_string(), new)],
            "동명 다른 에이전트(새 id)는 epoch 이 낮아도 flush(finding 3)"
        );
        // 3) 같은 (id,epoch) 재-push → skip(노이즈 — id·epoch 모두 불변).
        sink.agent_list_updated(vec![flush_info(new, "svc", 0, true, CoreStatus::Running)]);
        assert!(
            drain_targets(&mut rx).is_empty(),
            "같은 id+epoch 재-push 는 flush 안 함"
        );
    }

    #[test]
    fn flush_sink_appears_for_a_turn_signal_less_agent_and_ignores_the_dead() {
        // ★4차 개정(ADR-0116 결정 7)★: 등장 flush 의 유일한 조건은 **상태**다 — 턴 신호 없는 산 세션도
        //   Appear 대상이다(그 부류도 주입 실패로 파킹을 들고 있을 수 있고, 재등장이 그 유일한 flush 계기다).
        //   terminal 은 어느 쪽도 아니다.
        let (sink, mut rx) = flush_sink();
        let tui = AgentId::new_v4();
        let dead = AgentId::new_v4();
        sink.agent_list_updated(vec![
            flush_info(tui, "tui", 0, false, CoreStatus::Running), // 비-structured = 턴 신호 없음
            flush_info(dead, "dead", 0, true, CoreStatus::Killed), // terminal
        ]);
        assert_eq!(
            drain_msgs(&mut rx),
            vec![FlushMsg::Appear {
                name: "tui".to_string(),
                id: tui,
            }],
            "턴 신호 없는 산 세션 = Appear · terminal 은 아무것도 아님"
        );
    }

    // ── 7b. 턴 종료 push → 도어벨(ADR-0113 — 데몬은 중계만) ─────────────────────────────

    #[test]
    fn a_turn_end_push_from_the_core_becomes_an_idle_doorbell() {
        // 코어가 출력 pump 스레드에서 부르는 `StatusSink::turn_ended` 가 flush 채널로 이어지는지 —
        //   이 배선이 끊기면 파킹이 턴 종료에 풀리지 않고 TTL 까지 앉아 있는다.
        let (sink, mut rx) = flush_sink();
        let id = AgentId::new_v4();
        sink.turn_ended(id, 3);
        assert_eq!(drain_msgs(&mut rx), vec![FlushMsg::Idle { id }]);
    }

    #[test]
    fn turn_end_pushes_are_coalesced_per_agent_until_taken() {
        // ★잉여 통지 흡수★: 종료 신호마다 push 가 나오므로(누락 < 잉여) 미처리 Idle 은 id 별 1건으로 접힌다.
        let (sink, mut rx) = flush_sink();
        let id = AgentId::new_v4();
        sink.turn_ended(id, 0);
        sink.turn_ended(id, 0);
        sink.turn_ended(id, 1);
        assert_eq!(
            drain_msgs(&mut rx),
            vec![FlushMsg::Idle { id }],
            "미처리분이 있으면 접는다(소비자가 집어들면 다시 열린다 — IdleCoalescer)"
        );
    }

    #[test]
    fn idle_coalescer_folds_pending_notifications_until_taken() {
        // ★fix 10★: 같은 id 의 미처리 Idle 은 하나로 접힌다(MessageDone 폭풍의 채널 압력 상한). 소비자가
        //   집어들면(taken) 다시 열려 이후 통지가 큐에 들어간다 — 그래서 lost wakeup 이 없다.
        use engram_dashboard_messaging::busy::IdleNotifier;
        let (tx, mut rx) = mpsc::unbounded_channel::<FlushMsg>();
        let coalescer = Arc::new(IdleCoalescer::new());
        let notifier = ChannelIdleNotifier::new(tx, coalescer.clone());
        let a = AgentId::new_v4();
        let b = AgentId::new_v4();
        notifier.notify_idle(a);
        notifier.notify_idle(a);
        notifier.notify_idle(a);
        notifier.notify_idle(b);
        assert_eq!(
            drain_msgs(&mut rx),
            vec![FlushMsg::Idle { id: a }, FlushMsg::Idle { id: b }],
            "id 별로 미처리 1건씩만(다른 id 는 서로 접히지 않는다)"
        );
        // 소비자가 a 를 집어들면 다시 enqueue 가능.
        coalescer.taken(a);
        notifier.notify_idle(a);
        assert_eq!(drain_msgs(&mut rx), vec![FlushMsg::Idle { id: a }]);
    }

    #[test]
    fn service_doorbell_shares_the_idle_channel_and_coalescing() {
        // 서비스 도어벨(FlushTrigger)과 턴 종료 통지는 **같은 메시지**로 나간다 — 결과가 같기
        //   때문이다("그 id 의 파킹 큐를 flush"). 그래서 coalescing 도 함께 받는다.
        use engram_dashboard_messaging::service::FlushTrigger;
        let (tx, mut rx) = mpsc::unbounded_channel::<FlushMsg>();
        let coalescer = Arc::new(IdleCoalescer::new());
        let notifier = ChannelIdleNotifier::new(tx, coalescer.clone());
        let id = AgentId::new_v4();
        notifier.request_flush(id);
        notifier.request_flush(id);
        assert_eq!(drain_msgs(&mut rx), vec![FlushMsg::Idle { id }]);
    }

    // ── 9b. flush worker: 2-레인 소유/종료(round-3 finding 1) ────────────────────────────────

    fn lane_wiring() -> FlushWiring {
        FlushWiring {
            messaging: Arc::new(crate::control::mcp_server::MessagingSlot::new()),
            idle: Arc::new(IdleCoalescer::new()),
        }
    }

    #[tokio::test]
    async fn flush_worker_handles_shutdown_stops_both_lanes() {
        // ★round-3 finding 1 회귀★: 배달 레인은 **호출자 소유**여야 한다 — worker future 안에서 spawn 하면
        //   종료 시 detach 되고(abort 아님), 5s belt 는 blocking 없는 수신 레인만 감시하게 된다.
        //   `shutdown()` 이 두 레인을 모두 끝내는지(= 핸들을 들고 있는지) 관측한다.
        let (tx, rx) = mpsc::unbounded_channel::<FlushMsg>();
        let handles = spawn_flush_worker(rx, lane_wiring());
        // 배달 작업 1건을 넣어 레인이 실제로 돌게 한다(messaging slot 미주입이라 즉시 skip = 결정적).
        tx.send(FlushMsg::Idle {
            id: AgentId::new_v4(),
        })
        .expect("수신 레인 수신");
        tokio::time::timeout(Duration::from_secs(6), handles.shutdown())
            .await
            .expect("shutdown 이 belt 안에 반환해야");
        // 수신 레인이 죽었으므로 그 수신단은 닫혔다(송신 실패 = 소비자 종료의 관측 가능한 증거).
        assert!(
            tx.send(FlushMsg::Idle {
                id: AgentId::new_v4()
            })
            .is_err(),
            "shutdown 후 수신 레인은 더 이상 수신하지 않는다"
        );
    }

    // ── 10. (적용4-1) OriginCheck::on_request 분기 — 무방비 였던 거부/허용 분기 검증 ──────
    //    순수 헤더 검사라 in-process 서버 불필요. Request 를 직접 만들어 콜백을 호출한다.
    fn run_origin_check(origin: Option<&str>) -> Result<(), ()> {
        use tokio_tungstenite::tungstenite::http::Request as HttpRequest;
        let mut builder = HttpRequest::builder().uri("/");
        if let Some(o) = origin {
            builder = builder.header("origin", o);
        }
        let request = builder.body(()).unwrap();
        // Response 는 콜백이 통과시키는 더미. on_request 는 self 를 소비한다.
        let response = tokio_tungstenite::tungstenite::http::Response::builder()
            .body(())
            .unwrap();
        OriginCheck
            .on_request(&request, response)
            .map(|_| ())
            .map_err(|_| ())
    }

    #[test]
    fn origin_check_allows_listed_origin() {
        // allowlist 에 있는 Origin → 허용.
        assert!(run_origin_check(Some("tauri://localhost")).is_ok());
        assert!(run_origin_check(Some("http://localhost:1420")).is_ok());
    }

    #[test]
    fn origin_check_rejects_unlisted_origin() {
        // allowlist 밖 Origin → 거부(mutation 으로 무방비 였던 분기).
        assert!(run_origin_check(Some("http://evil.example.com")).is_err());
    }

    #[test]
    fn origin_check_allows_missing_origin() {
        // Origin 헤더 없음 → 현 정책상 허용(네이티브/하네스, 토큰이 주 방어).
        assert!(run_origin_check(None).is_ok());
    }

    // ── 11. 프레임 포트 seam — 소켓 없이 도는 격리 하네스(ADR-0129) ──────────────────
    //    가짜 ConnectionHandler + 가짜 FrameSink 로 연결 수명을 재현한다. TcpStream 이 없어야
    //    뒤 슬라이스에서 네트워크 행이 별도 crate 로 떨어져도 이 검증이 그대로 산다.

    #[derive(Debug, PartialEq, Eq)]
    enum SeenFrame {
        Text(String),
        Binary(Vec<u8>),
        Close(String),
    }

    #[derive(Default)]
    struct FakeFrameSink {
        frames: Mutex<Vec<SeenFrame>>,
    }

    impl FakeFrameSink {
        fn frames(&self) -> Vec<String> {
            self.frames
                .lock()
                .unwrap()
                .iter()
                .map(|f| match f {
                    SeenFrame::Text(s) => format!("text:{s}"),
                    SeenFrame::Binary(b) => format!("bin:{}", b.len()),
                    SeenFrame::Close(r) => format!("close:{r}"),
                })
                .collect()
        }
    }

    impl FrameSink for FakeFrameSink {
        fn try_send(&self, frame: Frame) -> Result<(), FrameError> {
            let seen = match frame {
                Frame::Text(s) => SeenFrame::Text(s),
                Frame::Binary(b) => SeenFrame::Binary(b),
                Frame::Close(r) => SeenFrame::Close(r),
            };
            self.frames.lock().unwrap().push(seen);
            Ok(())
        }
        fn send(&self, frame: Frame) -> BoxFuture<'_, Result<(), FrameError>> {
            Box::pin(async move { self.try_send(frame) })
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    enum HandlerCall {
        Connect(ConnId),
        Text(ConnId, String),
        /// payload 길이만 — 내용은 이 seam 의 관심사가 아니다.
        Binary(ConnId, usize),
        Disconnect(ConnId),
    }

    struct FakeHandler {
        calls: Mutex<Vec<HandlerCall>>,
        /// 이 텍스트를 받으면 `ConnFlow::Close` 를 돌려준다(수신 루프 탈출 검증용).
        close_on: Option<String>,
        /// 이 텍스트를 받으면 `Frame::Close` 를 큐에 넣고 **Continue** 를 돌려준다 — 연결을
        /// read 쪽이 아니라 **write_task 쪽**에서 끝내, select! 의 "read 를 abort" 갈래를 태운다.
        close_queue_on: Option<String>,
        /// `on_connect` 가 인사 프레임을 넣은 뒤 여기서 대기한다. 테스트가 그 창 동안 "아직 아무것도
        /// 소켓으로 안 나갔다"(=writer 미기동) 와 "아직 아무 프레임도 처리 안 됐다"(=reader 미기동)를
        /// 관측한다.
        connect_gate: Option<Arc<Notify>>,
        /// `on_text` 1건 처리 완료 신호 — 테스트가 클라 close 타이밍과 무관하게 진행하기 위한 것.
        text_seen: Arc<Notify>,
        /// `on_connect` 이 받은 프레임 출구의 약참조. 강참조는 read_task 만 들고 있으므로, 연결이
        /// 끝난 뒤에도 upgrade 되면 그 task 가 abort 되지 않고 **detach** 됐다는 뜻이다.
        /// ★이 fake 는 프레임 출구의 **강참조를 절대 보관하면 안 된다**★ — 필드에 `Arc` 를 하나라도
        /// 남기면 upgrade 가 항상 성공하고, `the_losing_task_is_aborted_not_detached` 의 폴링 루프가
        /// 끝까지 `released == false` 로 돌아 **정상 코드에서 그 테스트가 항상 실패한다**(조용한 탐지
        /// 불능이 아니라 시끄러운 위양성 — 그래서 원인을 이 필드로 되짚기 어렵다).
        frames_weak: Mutex<Option<std::sync::Weak<dyn FrameSink>>>,
        /// `on_disconnect` 시점에 이 연결이 아직 fanout 레지스트리에 있었는지(cleanup 순서 관측).
        registry: Option<ConnRegistry>,
        registered_at_disconnect: Mutex<Option<bool>>,
    }

    impl FakeHandler {
        fn new(close_on: Option<&str>) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                close_on: close_on.map(|s| s.to_string()),
                close_queue_on: None,
                connect_gate: None,
                text_seen: Arc::new(Notify::new()),
                frames_weak: Mutex::new(None),
                registry: None,
                registered_at_disconnect: Mutex::new(None),
            }
        }

        /// `handle_connection` 의 순서 검증용 — 레지스트리를 들여다보고 on_connect 을 게이트로 잡는다.
        fn probing(registry: ConnRegistry, connect_gate: Arc<Notify>) -> Self {
            Self {
                registry: Some(registry),
                connect_gate: Some(connect_gate),
                ..Self::new(None)
            }
        }

        /// write_task 쪽에서 연결을 끝내는 변종(패자 abort 검증용).
        fn closing_via_writer(close_queue_on: &str) -> Self {
            Self {
                close_queue_on: Some(close_queue_on.to_string()),
                ..Self::new(None)
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|c| match c {
                    HandlerCall::Connect(id) => format!("connect:{id}"),
                    HandlerCall::Text(id, t) => format!("text:{id}:{t}"),
                    HandlerCall::Binary(id, n) => format!("binary:{id}:{n}"),
                    HandlerCall::Disconnect(id) => format!("disconnect:{id}"),
                })
                .collect()
        }

        fn registered_at_disconnect(&self) -> Option<bool> {
            *self.registered_at_disconnect.lock().unwrap()
        }

        /// `on_connect` 이 두 지점에서 쓰는 단언 — 정상 코드에선 `on_connect` 이 끝나기 전에 어떤
        /// 프레임도 처리될 수 없다(read_task 가 아직 없다).
        fn assert_nothing_processed_yet(&self, at: &str) {
            let calls = self.calls.lock().unwrap();
            assert!(
                calls.is_empty(),
                "on_connect({at}) 보다 먼저 처리된 프레임이 있다 — read_task 가 앞서 스폰됐다: {calls:?}"
            );
        }

        fn frames_still_held(&self) -> bool {
            self.frames_weak
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|w| w.upgrade())
                .is_some()
        }
    }

    impl ConnectionHandler for FakeHandler {
        fn on_connect<'a>(
            &'a self,
            conn_id: ConnId,
            frames: &'a Arc<dyn FrameSink>,
        ) -> BoxFuture<'a, ()> {
            Box::pin(async move {
                *self.frames_weak.lock().unwrap() = Some(Arc::downgrade(frames));
                // ★게이트 **앞** 단언(프로그램 순서로 결정)★: 정상 코드에선 read_task 가 아직 스폰조차
                //   안 됐으므로 처리된 프레임이 있을 수 없다. 스케줄러와 무관하게 참이다.
                self.assert_nothing_processed_yet("게이트 진입 전");
                // 인사를 **게이트 앞에서** 넣는다 — writer 가 이미 떠 있다면 이 프레임이 게이트 대기
                //   중에 소켓으로 나가고, 테스트가 그걸 잡는다.
                let _ = frames.send(Frame::Text("greeting".into())).await;
                if let Some(gate) = &self.connect_gate {
                    gate.notified().await;
                }
                // ★게이트 **뒤** 단언★: 게이트가 열릴 때까지의 창(테스트가 그 안에서 명령을 미리
                //   흘려둔다) 동안 잘못 스폰된 read_task 가 그 명령을 처리했는지 잡는다.
                self.assert_nothing_processed_yet("게이트 통과 후");
                self.calls
                    .lock()
                    .unwrap()
                    .push(HandlerCall::Connect(conn_id));
            })
        }

        fn on_text<'a>(
            &'a self,
            conn_id: ConnId,
            text: &'a str,
            frames: &'a Arc<dyn FrameSink>,
        ) -> BoxFuture<'a, ConnFlow> {
            Box::pin(async move {
                let close = self.close_on.as_deref() == Some(text);
                let close_via_writer = self.close_queue_on.as_deref() == Some(text);
                self.calls
                    .lock()
                    .unwrap()
                    .push(HandlerCall::Text(conn_id, text.to_string()));
                self.text_seen.notify_one();
                if close_via_writer {
                    let _ = frames.try_send(Frame::Close("테스트: writer 가 먼저 끝난다".into()));
                    return ConnFlow::Continue;
                }
                if close {
                    ConnFlow::Close
                } else {
                    ConnFlow::Continue
                }
            })
        }

        fn on_binary<'a>(
            &'a self,
            conn_id: ConnId,
            payload: &'a [u8],
            _frames: &'a Arc<dyn FrameSink>,
        ) -> BoxFuture<'a, ConnFlow> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .unwrap()
                    .push(HandlerCall::Binary(conn_id, payload.len()));
                ConnFlow::Continue
            })
        }

        fn on_disconnect(&self, conn_id: ConnId) {
            if let Some(registry) = &self.registry {
                *self.registered_at_disconnect.lock().unwrap() = Some(registry.contains(conn_id));
            }
            self.calls
                .lock()
                .unwrap()
                .push(HandlerCall::Disconnect(conn_id));
        }
    }

    struct FakeFactory {
        handler: Arc<FakeHandler>,
    }

    impl ConnectionHandlerFactory for FakeFactory {
        fn handler_for(&self, _conn_id: ConnId) -> Arc<dyn ConnectionHandler> {
            self.handler.clone()
        }
        fn handshake_error_frame(&self, message: &str) -> Option<String> {
            Some(message.to_string())
        }
    }

    fn text_frame(s: &str) -> Result<Message, tokio_tungstenite::tungstenite::Error> {
        Ok(Message::Text(s.to_string().into()))
    }

    #[tokio::test]
    async fn handler_sees_connect_then_frames_then_disconnect() {
        let fake_sink = Arc::new(FakeFrameSink::default());
        let frames: Arc<dyn FrameSink> = fake_sink.clone();
        let fake = Arc::new(FakeHandler::new(None));
        let handler: Arc<dyn ConnectionHandler> = fake.clone();

        handler.on_connect(7, &frames).await;
        read_task(
            futures_util::stream::iter(vec![
                text_frame("cmd"),
                Ok(Message::Binary(vec![1, 2, 3].into())),
                Ok(Message::Close(None)),
                text_frame("after-close"),
            ]),
            frames.clone(),
            handler.clone(),
            7,
            tokio::time::Instant::now(),
            Arc::new(AtomicU64::new(0)),
        )
        .await;
        handler.on_disconnect(7);

        assert_eq!(
            fake.calls(),
            vec![
                "connect:7",
                "text:7:cmd",
                "binary:7:3",
                "disconnect:7", // Close frame 뒤의 프레임은 소비되지 않는다
            ]
        );
        assert_eq!(
            fake_sink.frames(),
            vec!["text:greeting"],
            "on_connect 가 넣은 프레임이 단일 출구로 나간다"
        );
    }

    #[tokio::test]
    async fn close_flow_from_on_text_breaks_the_read_loop() {
        let frames: Arc<dyn FrameSink> = Arc::new(FakeFrameSink::default());
        let fake = Arc::new(FakeHandler::new(Some("stop")));
        let handler: Arc<dyn ConnectionHandler> = fake.clone();

        read_task(
            futures_util::stream::iter(vec![
                text_frame("go"),
                text_frame("stop"),
                text_frame("unreachable"),
            ]),
            frames,
            handler,
            3,
            tokio::time::Instant::now(),
            Arc::new(AtomicU64::new(0)),
        )
        .await;

        assert_eq!(
            fake.calls(),
            vec!["text:3:go", "text:3:stop"],
            "ConnFlow::Close 면 그 자리에서 수신 루프를 나간다"
        );
    }

    /// 메시지 1건을 수신 루프에 태우고 keepalive 시계가 갱신됐는지 돌려준다.
    /// 초기값을 도달 불가능한 sentinel 로 두어 "갱신됐다" 를 타이밍 없이 판정한다.
    async fn clock_updated_by(msg: Message) -> bool {
        let frames: Arc<dyn FrameSink> = Arc::new(FakeFrameSink::default());
        let handler: Arc<dyn ConnectionHandler> = Arc::new(FakeHandler::new(None));
        let last_recv = Arc::new(AtomicU64::new(u64::MAX));

        read_task(
            futures_util::stream::iter(vec![Ok(msg)]),
            frames,
            handler,
            1,
            tokio::time::Instant::now(),
            last_recv.clone(),
        )
        .await;

        last_recv.load(Ordering::Acquire) < u64::MAX
    }

    #[tokio::test]
    async fn every_received_message_updates_the_keepalive_clock() {
        // ★"무엇이든 받았다 = 살아있다"★: 갱신은 메시지 **종류를 가리지 않는다**(생산 코드가 match
        //   **앞에서** 갱신하는 이유). 특히 Pong 이 빠지면 능동 Ping 에만 답하는 정상 연결이 idle 로
        //   오판돼 끊긴다(half-open 위양성). 갱신을 일부 arm 안으로 옮기는 회귀를 잡으려면 6종을 다
        //   태워야 하므로 `Message` 의 variant 전부를 넣는다.
        assert!(clock_updated_by(Message::Text("cmd".to_string().into())).await);
        assert!(clock_updated_by(Message::Binary(vec![1].into())).await);
        assert!(clock_updated_by(Message::Ping(Vec::new().into())).await);
        assert!(clock_updated_by(Message::Pong(Vec::new().into())).await);
        assert!(clock_updated_by(Message::Close(None)).await);
        // Message::Frame 은 실소켓 수신으로는 안 올라오지만(tungstenite 문서) read_task 가 arm 을
        //   갖고 있으므로 합성해 태운다.
        assert!(
            clock_updated_by(Message::Frame(
                tokio_tungstenite::tungstenite::protocol::frame::Frame::pong(Vec::new())
            ))
            .await
        );
    }

    // ── 12. handle_connection 순서 계약(ADR-0129) ─────────────────────────────────

    /// 테스트 서버 1개를 띄우고 인증까지 마친 클라이언트를 돌려준다(공통 뼈대).
    async fn serve_one(
        registry: ConnRegistry,
        fake: Arc<FakeHandler>,
        keepalive: KeepaliveConfig,
    ) -> (
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let factory: Arc<dyn ConnectionHandlerFactory> = Arc::new(FakeFactory { handler: fake });
        let server = tokio::spawn(async move {
            let (stream, peer) = listener.accept().await.unwrap();
            handle_connection(
                stream,
                peer,
                registry,
                factory,
                Arc::new("tok".to_string()),
                keepalive,
            )
            .await;
        });

        let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/"))
            .await
            .unwrap();
        let auth = serde_json::to_string(&AgentCommand::Auth {
            token: "tok".to_string(),
            protocol_version: PROTOCOL_VERSION,
        })
        .unwrap();
        client.send(Message::Text(auth.into())).await.unwrap();
        (client, server)
    }

    /// 실제 `handle_connection` 이 지키는 세 순서를 고정한다 — read_task 하네스로는 못 잡는 것들:
    ///   ① `on_connect` 가 **write_task 스폰**보다 앞선다(그래야 `on_connect` 계약의 "소비자가 아직
    ///      없다" 전제가 성립한다)
    ///   ② `on_connect` 가 **read_task 스폰**보다 앞선다(그래야 명령이 인사를 앞지르지 못한다)
    ///   ③ `on_disconnect` 가 레지스트리 제거보다 앞선다
    ///
    /// ★탐지력의 정직한 범위★ — 세 단언 다 **정상 코드를 실패시킬 수는 없다**(전부 부정형: "아직
    ///   아무것도 처리/도달하지 않았다"). 그러나 회귀를 잡는 것은 ③만 결정적이다:
    ///   - ①·② 모두 **확률적**이다. 스폰은 그 자체로 아무것도 실행하지 않으므로, 잘못 앞당겨 스폰된
    ///     task 가 **이 창 안에 폴링되어야** 흔적이 남는다. ②의 경우 회귀 구현이 read_task 를 먼저
    ///     스폰해도 실행기가 그 전에 `on_connect` 을 게이트 앞 단언까지 폴링해 버릴 수 있고, 게이트가
    ///     열릴 때까지도 명령이 처리되지 않았으면 게이트 뒤 단언과 마지막 호출 순서 검사까지 전부
    ///     통과한다. 그래서 단언을 게이트 앞·뒤 **두 곳**에 둬 창을 넓히지만(공짜다) 보증은 아니다.
    ///     ★결정적 판별자는 없다★ — 스케줄링이나 task 생성에 직접 걸 훅이 없으면 만들 수 없다.
    ///   - ③은 `on_disconnect` 안에서 레지스트리를 직접 들여다보므로 사실상 결정적이다.
    ///   요약: 위양성 0 · ③ 결정적 · ①② 확률적(창을 넓힌 표집).
    #[tokio::test]
    async fn handle_connection_orders_connect_before_both_tasks_and_cleanup_before_unregister() {
        let registry = ConnRegistry::new();
        let gate = Arc::new(Notify::new());
        let fake = Arc::new(FakeHandler::probing(registry.clone(), gate.clone()));
        let (mut client, server) =
            serve_one(registry.clone(), fake.clone(), KeepaliveConfig::default()).await;

        // 게이트가 닫힌 동안 서버 소켓에 대기하도록 명령을 미리 흘려둔다.
        client
            .send(Message::Text("cmd".to_string().into()))
            .await
            .unwrap();

        // ★①★ 이 창에서 클라에 도달하는 게 있으면 writer 가 이미 떠 있다는 뜻이다. 정상 코드에선
        //   writer 가 없으므로 **영원히** 아무것도 안 온다 — 즉 위양성(flake)은 불가능하고, 부하가
        //   높으면 위음성(놓침) 쪽으로만 틀린다. **결정적 보증이 아니라 확률적 탐지**다(임계값 튜닝
        //   대상이 아닌 이유이기도 하다 — 값을 키워도 보증이 되지는 않는다).
        assert!(
            tokio::time::timeout(Duration::from_millis(200), client.next())
                .await
                .is_err(),
            "on_connect 진행 중엔 writer 가 없어 소켓으로 나가는 게 없어야 한다"
        );

        gate.notify_one();

        // ★클라 close 타이밍에 의존하지 않는다★: 명령이 실제로 처리된 걸 확인한 **뒤에** 닫는다.
        //   (닫기를 먼저 하면 writer 의 인사 write 실패 → reader abort 경합이 결과를 좌우할 수 있다.)
        tokio::time::timeout(Duration::from_secs(5), fake.text_seen.notified())
            .await
            .expect("명령이 처리돼야");
        client.close(None).await.unwrap();

        tokio::time::timeout(Duration::from_secs(10), server)
            .await
            .expect("handle_connection 이 반환해야")
            .unwrap();

        let calls = fake.calls();
        assert_eq!(
            calls,
            vec!["connect:1", "text:1:cmd", "disconnect:1"],
            "connect → 프레임 → disconnect 순서"
        );
        // ★③★
        assert_eq!(
            fake.registered_at_disconnect(),
            Some(true),
            "on_disconnect 시점엔 아직 fanout 레지스트리에 있다"
        );
        assert!(
            !registry.contains(1),
            "handle_connection 이 반환할 땐 등록 해제돼 있다"
        );
    }

    /// ★패자 task 의 명시적 abort★(`handle_connection` 의 select!) — JoinHandle 을 그냥 drop 하면
    /// detach 되어 WS half 를 쥔 task 가 살아남는다. 그 누수는 e2e 로는 안 보인다(전부 정상 종료라
    /// 남은 half 가 표에 안 드러난다).
    ///
    /// ★관측 방법★: 프레임 출구 `Arc` 의 **강참조는 read_task 만** 들고 있다(`handle_connection` 은
    /// 그것을 read_task 로 move 하고, `on_connect` 은 빌리기만 한다). 그래서 연결 종료 뒤에도 약참조가
    /// upgrade 되면 = read_task 가 살아 있다 = abort 대신 detach 됐다는 뜻이다.
    /// 이 방향(write 가 먼저 끝나 read 를 abort 하는 갈래)을 태우려고 핸들러가 `Frame::Close` 를 큐에
    /// 넣고 Continue 를 돌려준다 — read 는 계속 `next()` 에 파킹된 채로 남는다.
    /// ★대칭 갈래(read 가 먼저 끝나 write 를 abort)는 **미검증으로 남긴다**★ — 아래 조건부 관측 때문에
    /// 부정형 테스트를 못 만들었을 뿐이고, **그쪽 `abort()` 가 불필요하다는 뜻은 아니다.**
    ///
    /// ★관측된 것(2026-08-04, 현 `AgentConnection` + 아래 경쟁 미발생 조건에서만)★: write_task 를
    /// detach 해도 `handle_connection` 반환 시 마지막 `Sender<Frame>` 이 드롭되어 `conn_rx.recv()` 가
    /// None 을 돌려주고, write_task 가 **스스로 종료**하며 sink_half 를 놓았다(짧은 `ping_interval` 로
    /// "종료 후 Ping 이 더 오는가" 를 봤더니 즉시 EOF — 소켓이 닫혔다 = writer 가 이미 끝났다).
    ///
    /// ★그 자기종료는 "모든 `Sender<Frame>` 사본이 `handle_connection` 과 함께 죽는다" 는 **조건부**다★.
    /// 사본 전수: ① 이 함수의 지역 `conn_tx` ② 레지스트리 항목(등록 해제로 소멸) ③ read_task 가 소유한
    /// `ConnFrameSink` ④ 코어 subscribers 에 등록된 `FrameOutputSink` 사본들 — ④는 `session.subs` 기록을
    /// 통해서만 회수된다(`on_disconnect`).
    /// ★그래서 ④가 새면 조건이 깨지고, 살아남은 sender 때문에 detach 된 writer 는 **자기종료하지
    /// 않는다**★. 새는 경로가 실제로 있다: `handle_subscribe` 가 코어에 sink 를 등록한 **뒤**
    /// `subs.insert` 전에 read_task 가 **패닉**하면(그 사이엔 `.await` 가 없어 취소는 못 끼어들지만
    /// 패닉은 낀다) 그 sink 는 어디에도 기록되지 않은 채 코어에 남아 conn_tx 사본을 붙든다.
    /// 릴리스는 `panic=abort` 라 프로세스가 죽지만 debug/테스트 빌드에선 도달 가능하다.
    /// **결론: 그 갈래의 `abort()` 는 제거 금지** — 위 실측은 "그 경쟁이 안 났을 때 그렇더라" 이지
    /// 불변식이 아니다.
    /// 반대 갈래(이 테스트)는 read_task 가 `next()` 에 파킹돼 스스로 끝나지 않으므로 abort 가 유일한
    /// 회수 수단이다 — 그래서 여기만 검증한다.
    #[tokio::test]
    async fn the_losing_task_is_aborted_not_detached() {
        let registry = ConnRegistry::new();
        let fake = Arc::new(FakeHandler::closing_via_writer("die"));
        let (mut client, server) =
            serve_one(registry.clone(), fake.clone(), KeepaliveConfig::default()).await;

        client
            .send(Message::Text("die".to_string().into()))
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(10), server)
            .await
            .expect("write_task 가 Frame::Close 로 끝나면 handle_connection 도 반환해야")
            .unwrap();

        // abort 는 요청이라 future drop 이 반환과 동기는 아니다 — 넉넉히 폴링한다(정상 코드는 곧
        //   놓고, detach 회귀는 `next()` 에 영원히 파킹돼 절대 놓지 않는다).
        let mut released = false;
        for _ in 0..200 {
            if !fake.frames_still_held() {
                released = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            released,
            "패자 task 가 abort 되지 않았다(detach) — WS half 를 쥔 채 살아남는다"
        );
        drop(client);
    }
}
