//! WebSocket 서버 본체 (phase 2 step 4b).
//!
//! 책임: accept 된 TCP stream 을 WS 업그레이드(Origin allowlist) → 1초 내 첫 frame 토큰 auth →
//! AgentCommand/AgentEvent 프레임 핸들링(manager 위임). 출력 hot path 는 binary frame(codec),
//! control 은 JSON.
//!
//! ★동시성 모델(위험 지점)★
//! - **연결당 단일 writer**: SplitSink 는 동시 write 불가. 그래서 모든 출력 frame·control JSON 을
//!   연결당 단일 `mpsc::Sender<WsOutbound>`(conn_tx)에 넣고, write_task 한 곳만 SinkHalf 에 write
//!   한다. SubscribeAck→replay→live 의 FIFO 순서가 이 단일 큐로 보장된다.
//! - **try_send vs await 경계**: pump 스레드에서 호출되는 `WsOutputSink::send` 는 절대 block 금지
//!   (try_send 만). async read_task 의 control 전송은 await 허용(.send().await).
//! - **out-of-band 종료 신호(close_signal)**: conn_tx 가 full 이면 큐 안 마커(WsOutbound::Close)도
//!   try_send 실패해 좀비 연결이 된다. 그래서 큐 **밖**의 `Arc<Notify>` close_signal 을 둔다.
//!   WsOutputSink 가 full 을 만나면 `close_signal.notify_one()`(sync 안전)으로 신호하고,
//!   write_task 는 `tokio::select!` 로 conn_rx.recv() 와 close_signal.notified() 를 동시에 대기해
//!   큐가 막혀 있어도 깨어 sink_half.close() 후 break → cleanup 한다.
//! - **레지스트리**: status 브로드캐스트용. 모든 연결의 conn_tx 를 ConnId→Sender 맵으로 보관해
//!   DaemonStatusSink 가 try_send(Text) 로 전 연결에 fanout.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use engram_dashboard_core::agent::manager::AgentManager;
use engram_dashboard_core::agent::profile::RestoreReport as CoreRestoreReport;
use engram_dashboard_core::agent::types::{
    AgentId, AgentInfo as CoreAgentInfo, AgentStatus as CoreStatus, OutputFrame, OutputPayload,
    OutputSink, SinkError, SinkId, StatusSink,
};

use engram_dashboard_protocol::{
    encode_structured_frame, encode_terminal_frame, AgentCommand, AgentEvent, PROTOCOL_VERSION,
};

use crate::connection_core::{
    agent_list_event, core_agents_to_wire, core_report_to_wire, core_status_to_wire, event_json,
    hello_event, output_event_to_wire, ConnectionCore, ConnectionSession, MultiViewState, Outbound,
    OutboundSink as CoreOutboundSink, SinkError as CoreSinkError,
};

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch, Notify};
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

/// 연결 식별자(단조 증가). 레지스트리 키.
pub type ConnId = u64;

/// 단일 writer 큐로 흐르는 출력 단위. 모든 frame·control·close 가 이걸 통해 write_task 로 간다.
#[derive(Debug)]
pub enum WsOutbound {
    /// control JSON(AgentEvent 직렬화).
    Text(String),
    /// 출력 binary frame(codec).
    Binary(Vec<u8>),
    /// 연결 종료 — write_task 가 이걸 받으면 close 후 break. reason 은 로그/디버깅용.
    Close(String),
}

/// status 브로드캐스트용 연결 레지스트리. connect 시 등록, disconnect 시 제거.
/// DaemonStatusSink 가 전 연결 conn_tx 에 try_send 하기 위해 공유된다.
#[derive(Clone)]
pub struct ConnRegistry {
    inner: Arc<Mutex<HashMap<ConnId, mpsc::Sender<WsOutbound>>>>,
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

    fn register(&self, id: ConnId, tx: mpsc::Sender<WsOutbound>) {
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

    /// 전 연결에 Text 브로드캐스트(try_send). full 인 연결은 느린 것으로 보고 로그만.
    pub(crate) fn broadcast_text(&self, text: String) {
        let conns: Vec<(ConnId, mpsc::Sender<WsOutbound>)> = {
            let guard = self.inner.lock().expect("conn registry poisoned");
            guard.iter().map(|(id, tx)| (*id, tx.clone())).collect()
        };
        for (id, tx) in conns {
            // try_send 만 — StatusSink 는 pump/manager 스레드(sync)에서 불릴 수 있어 block 금지.
            if let Err(e) = tx.try_send(WsOutbound::Text(text.clone())) {
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

/// flush worker 로 흐르는 작업 단위(C1 등장 flush + C2 idle 게이트 배선).
///
/// ★왜 enum 인가(옛 `(String, AgentId)` 튜플 확장)★: C2 가 같은 worker 에 **세 종류**의 일을 더 얹는다 —
///   턴 관측 tap 부착/해제와 턴 종료 flush. 채널을 늘리면 순서가 갈려(같은 에이전트의 Attach 와 Appear 가
///   서로 앞지름) 배선 추론이 어려워지므로, **하나의 채널·하나의 소비자**로 유지하고 메시지 종류만 늘린다.
/// ★공통 계약★: 이 메시지들은 전부 **status 콜백/pump 콜백(블록 금지)** 에서 논블록으로 enqueue 되고,
///   실제 작업(코어 락·링 replay·blocking stdin write)은 worker 가 수행한다(finding 5 계열 규율).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlushMsg {
    /// 로스터 등장/epoch bump(그 이름의 도달 후보가 **유일**할 때만) — 그 **이름** 앞 파킹을 일괄 flush.
    Appear { name: String, id: AgentId },
    /// 턴 관측 tap 부착 요청(C2). subscribe 는 core 락 구간에 들어가므로 status 콜백에서 못 한다.
    /// ★이름 유일성과 무관하게 id 단위★: tap 은 출력 스트림(= 세션) 단위라 동명 다수여도 각각 붙는다.
    /// ★재시도 상태를 메시지에 싣지 않는다★: 실패 재시도는 worker 안에서 **즉시 1회**로 끝내고(상한이
    /// 코드에 박혀 있다) 그 이후는 로스터 diff 재발행에 맡긴다(`handle_attach`) — 메시지에 시도 횟수를
    /// 실으면 아무도 증가시키지 않는 상태 필드가 남는다.
    Attach { id: AgentId, epoch: u32 },
    /// 로스터 이탈(죽음) — 그 id 의 턴 상태·부착 표시 청소(C2). 죽은 에이전트의 busy 플래그가 남으면
    /// 그 이름 앞 파킹이 영영 대기한다.
    Detach { id: AgentId },
    /// 턴 종료(idle 전이) 관측 — 그 id 의 파킹을 오래된 순 일괄 주입(C2 idle 게이트, ADR-0104 결정 3).
    Idle { id: AgentId },
}

/// ★Idle 통지 coalescer(C2 리뷰 fix 10)★ — 같은 id 의 **미처리** Idle 이 이미 큐에 있으면 새 enqueue 를 접는다.
///
/// ★왜 필요한가(유계 채널 압력)★: 통지는 MessageDone **마다** 나간다(누락 < 잉여 — busy.rs `IdleNotifier`).
///   에이전트가 도구 호출을 연달아 돌리면 짧은 시간에 MessageDone 이 여러 번 나올 수 있고, unbounded 채널
///   이라 그만큼 항목이 쌓인다(메모리·처리 낭비 — 대부분 빈 큐 no-op). flush 는 **큐 전체를 drain** 하므로
///   같은 id 의 Idle N개는 1개와 결과가 같다 → 접어도 의미가 보존된다.
/// ★lost wakeup 이 없는 이유(load-bearing)★: 통지 순서가 "① tap 이 busy 해제 → ② notify" 이므로, 접힌
///   통지가 가리키는 상태 변화는 **아직 처리 안 된 그 Idle** 이 대표한다. 소비자는 **집어들 때 먼저 집합에서
///   지우고**(그 뒤에 게이트를 보고 flush) 처리하므로, 처리 도중 도착한 새 MessageDone 은 다시 enqueue 된다.
/// ★로스터 생애주기 메시지는 절대 접지 않는다★: Attach/Detach/Appear 는 각각 고유한 사건이라 무손실이어야
///   한다(접으면 tap 부착·상태 청소가 사라진다). 이 coalescer 는 **Idle 전용**이다.
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

/// tap → flush worker 통지 구현 — `IdleNotifier`(턴 종료 관측)와 `FlushTrigger`(서비스 도어벨)를 **같은
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

impl crate::messaging::busy::IdleNotifier for ChannelIdleNotifier {
    fn notify_idle(&self, id: AgentId) {
        self.enqueue(id);
    }
}

impl crate::messaging::service::FlushTrigger for ChannelIdleNotifier {
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
    /// 로스터 diff 시퀀싱 상태(스냅샷 2축 + enqueue 직렬화) — flush worker 와 **공유**한다(attach 실패
    ///   피드백이 여기 스냅샷을 무효화해야 재시도가 열린다 — `RosterDiff::forget_attached`).
    diff: Arc<RosterDiff>,
}

/// ★로스터 diff 시퀀싱 상태(C2 리뷰 fix 7)★ — 이름 축·id 축 직전 스냅샷을 **한 락** 아래 두고, 그 락을
///   **든 채로 채널 enqueue 까지** 끝낸다.
///
/// ★왜 한 락 + 락 보유 중 send 인가(load-bearing)★: 옛 구현은 스냅샷을 락 안에서 갱신하고 **락을 놓은
///   뒤** send 했다. 그러면 `agent_list_updated` 콜백 둘이 동시에 들어올 때(코어는 이 콜백의 직렬화를
///   보장하지 않는다) 스냅샷 갱신 순서와 enqueue 순서가 **갈릴 수 있다** — 예: 나중 스냅샷(에이전트 부활)
///   의 Attach 가 먼저 enqueue 되고, 앞선 스냅샷(그 에이전트 사망)의 Detach 가 뒤에 도착해 방금 붙인
///   tap 의 상태·부착 표시를 지운다(그 에이전트는 그 뒤로 영구 idle 폴백 = 턴 중 주입). enqueue 를 락
///   안으로 넣으면 "스냅샷 순서 = 채널 순서" 가 구조적으로 보장된다. unbounded send 는 논블록이라 락
///   보유 구간이 여전히 짧다(콜백 blocking 금지 규율 유지).
/// ★두 축을 따로 두는 게 load-bearing★: 이름 축(`prev`)은 동명 다수를 skip 하지만(배달 대상 모호), tap 은
///   출력 스트림 단위라 동명이어도 **전부** 붙어야 한다(붙지 않으면 그 에이전트는 영구 idle 폴백). 판정
///   기준이 다르므로 스냅샷도 분리한다 — 단, **같은 락** 아래 둔다(위 순서 보장).
#[derive(Debug, Default)]
pub struct RosterDiff {
    inner: Mutex<RosterSnapshots>,
}

#[derive(Debug, Default)]
struct RosterSnapshots {
    /// 직전 로스터 스냅샷(name→(epoch, id)). diff 로 newly-live/epoch-bump 를 판정(배달 축).
    prev: HashMap<String, (u32, AgentId)>,
    /// 직전 **id 축** 스냅샷(id→epoch) — 턴 관측 tap 의 부착/해제 판정용.
    prev_ids: HashMap<AgentId, u32>,
}

impl RosterDiff {
    pub fn new() -> Self {
        Self::default()
    }

    /// ★attach 실패 피드백(C2 리뷰 fix 8a)★: 이 id 의 부착 스냅샷 항목을 지운다 → 다음 로스터 업데이트가
    ///   그 id 를 "새로 등장" 으로 보고 Attach 를 다시 낸다.
    ///
    /// ★없으면 무슨 일이 나나★: 스냅샷은 Attach 를 **낸 시점에** 갱신되므로, worker 의 부착이 실패해도
    ///   스냅샷은 "붙었다" 고 남는다 → 로스터가 그대로인 동안 Attach 가 다시 나오지 않아 그 에이전트는
    ///   **영구히 tap 없이** 돈다(게이트 관점 항상 idle = 턴 중 주입). 조용한 기능 상실이라 반드시 되돌린다.
    pub fn forget_attached(&self, id: AgentId) {
        self.inner
            .lock()
            .expect("flush roster diff poisoned")
            .prev_ids
            .remove(&id);
    }

    /// 로스터 업데이트 1회 처리 — diff 를 계산해 **락 보유 중** 순서대로 enqueue 한다.
    ///
    /// enqueue 순서(같은 업데이트 안에서 load-bearing): **Attach → Detach → Appear**.
    ///   Attach 를 Appear 보다 앞세우는 이유: 등장 flush 주입 **이전**에 tap 이 붙어야 그 주입이 만드는
    ///   유저 에코부터 관측된다(턴 상태 표가 첫 턴부터 정확해짐). 뒤집으면 첫 턴을 놓쳐 그 사이 도착한
    ///   메시지가 턴 중에 주입될 수 있다. worker 도 Attach 를 **완료한 뒤** Appear 를 flush lane 으로
    ///   넘기므로(ws.rs run_flush_worker) 이 순서가 실행까지 보존된다.
    fn dispatch(&self, agents: &[CoreAgentInfo], flush_tx: &mpsc::UnboundedSender<FlushMsg>) {
        let mut st = self.inner.lock().expect("flush roster diff poisoned");

        // ── ① id 축 diff(tap 부착/해제) ────────────────────────────────────────────────
        //   도달 가능(산 + structured) 후보만 — 비-structured 는 턴 이벤트 자체가 없고(busy 관측 불가)
        //   파킹 수신 대상도 아니다(게이트가 보는 집합과 tap 집합을 일치시킨다 — busy.rs 헤더 프록시).
        let mut next_ids: HashMap<AgentId, u32> = HashMap::new();
        for a in agents {
            let reachable = matches!(a.status, CoreStatus::Running | CoreStatus::Exiting)
                && a.capabilities.output.structured;
            if reachable {
                next_ids.insert(a.id, a.epoch);
            }
        }
        for (id, epoch) in &next_ids {
            // 새 id 또는 epoch 변경(= 새 OutputCore — 구독은 epoch 을 넘지 못한다) → 부착 요청.
            //   중복 요청은 tracker 가 접는다(attach dedup) — 여기선 노이즈만 줄인다.
            if st.prev_ids.get(id) != Some(epoch) {
                let _ = flush_tx.send(FlushMsg::Attach {
                    id: *id,
                    epoch: *epoch,
                });
            }
        }
        // 이탈(로스터에서 사라짐 = terminal/reap) → 상태 청소. 죽은 대상의 busy 플래그를 남기면
        //   그 이름 앞 파킹이 다음 등장까지 stranded 된다.
        for id in st.prev_ids.keys() {
            if !next_ids.contains_key(id) {
                let _ = flush_tx.send(FlushMsg::Detach { id: *id });
            }
        }
        st.prev_ids = next_ids;

        // ── ② 이름 축 diff(등장 flush) ─────────────────────────────────────────────────
        // 1) 산(Running|Exiting) + structured(도달 가능) 후보를 **이름별로 그룹핑**한다 — 파킹은 이 조건의
        //   수신자 앞으로만 배달 가능(비-도달 이름은 애초에 파킹 수신 대상 아님).
        // ★finding 2(BLOCK): 동명 다수 skip(last-write-wins 금지)★: 예전엔 같은 이름을 마지막 것으로
        //   덮어(last-write-wins) 임의 incarnation 으로 flush 했다 — 이름-키 파킹이 엉뚱한 동명 에이전트로
        //   갈 수 있어 send-side RECIPIENT_AMBIGUOUS 정책과 어긋난다. 이제 그 이름을 지닌 도달 가능
        //   후보가 **정확히 1개**일 때만 flush 대상으로 삼고, 동명 다수는 건너뛴다(tracing::debug) — 파킹
        //   메일은 그 이름이 다시 유일해지거나 TTL 로 만료될 때까지 대기한다.
        let mut by_name: HashMap<String, Vec<(u32, AgentId)>> = HashMap::new();
        for a in agents {
            let reachable = matches!(a.status, CoreStatus::Running | CoreStatus::Exiting)
                && a.capabilities.output.structured;
            if !reachable {
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
    ///   flush worker 로 이어지는 채널의 송신단이고, `diff` 는 그 worker 와 공유하는 시퀀싱 상태다(부팅에서 만든다).
    pub fn new(
        inner: DaemonStatusSink,
        flush_tx: mpsc::UnboundedSender<FlushMsg>,
        diff: Arc<RosterDiff>,
    ) -> Self {
        Self::new_boxed(Box::new(inner), flush_tx, diff)
    }

    /// 테스트 생성자 — 임의 inner StatusSink(NoopSink 등)를 감싼다. flush 로직만 검증할 때.
    pub fn new_test(
        inner: Box<dyn StatusSink>,
        flush_tx: mpsc::UnboundedSender<FlushMsg>,
        diff: Arc<RosterDiff>,
    ) -> Self {
        Self::new_boxed(inner, flush_tx, diff)
    }

    fn new_boxed(
        inner: Box<dyn StatusSink>,
        flush_tx: mpsc::UnboundedSender<FlushMsg>,
        diff: Arc<RosterDiff>,
    ) -> Self {
        Self {
            inner,
            flush_tx,
            diff,
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
/// ★C2 추가 책임★: 같은 채널이 턴 관측 tap 의 **부착(Attach)/해제(Detach)** 와 **턴 종료 flush(Idle)** 도
///   나른다. Attach 는 코어 subscribe(락 구간)이라 status 콜백에서 할 수 없어 여기서 blocking pool 로 던지고,
///   Detach 는 순수 맵 조작이라 인라인, Idle 은 Appear 와 같은 flush 경로를 쓰되 이름 대신 **id** 로
///   진입한다(tap 은 이름을 모른다).
///
/// ★2-레인 파이프라인(C2 리뷰 fix 3 — head-of-line blocking 제거, load-bearing)★: 단일 소비자였을 때는
///   한 수신자의 **막힌 stdin write** 가 그 뒤의 **모든** 메시지를 세우는 문제가 있었다 — Attach/Detach 는
///   배달과 무관한 생애주기 작업(tap 부착·상태 청소)인데도 배달 뒤에서 굶어, 그 사이 등장한 에이전트들이
///   전부 tap 없이(=게이트 없이) 돌게 된다. 그래서 레인을 둘로 나눈다:
///   - **main lane(이 함수)** — Attach/Detach 를 **인라인 처리**하고, 배달 작업(Appear/Idle)은 flush 레인으로
///     **forward** 한다. Attach 의 `spawn_blocking(...).await` 는 허용한다: replay 는 링(≤4096, 메모리)
///     한계이고 **자식 프로세스에 의존하지 않으므로** 유계 시간에 끝난다(막히는 write 와 성질이 다르다).
///     live-only 구독(busy.rs fix 1)이라 그 replay 조차 0건이다.
///   - **flush lane(`run_flush_lane`)** — 자체 task + 자체 채널. 여기의 `spawn_blocking` 이 막혀도 main
///     lane 은 계속 돌아 Attach/Detach 가 제때 처리된다. 레인 내부는 **여전히 직렬**이라 같은 수신자 배치
///     순서(오래된 순)는 보존된다(병렬화하면 순서가 깨진다).
///   ★Attach → Appear 순서 보존은 **한 로스터 업데이트 안에서만** 성립한다(round-3 finding 7 — 범위 정정)★:
///     같은 업데이트가 낸 Attach 를 main lane 이 처리 완료한 뒤 그 업데이트의 Appear 를 forward 하므로, 그
///     업데이트가 트리거한 등장 flush 주입 전엔 tap 이 붙어 있다(RosterDiff 주석). **업데이트를 넘으면
///     보장되지 않는다** — 업데이트 N 의 Appear 가 레인에서 처리되는 동안 업데이트 N+1 의 Attach 가 main
///     lane 에서 돌 수 있고(두 레인은 서로 기다리지 않는다), 그러면 N+1 이 등장시킨 에이전트의 첫 턴 일부를
///     tap 이 놓칠 수 있다. 결과는 **타이밍 어긋남뿐이고 유실은 없다**: 놓친 구간의 busy 관측이 없으면
///     게이트는 idle 폴백(positive-knowledge-only)이라 그 배치가 턴 중에 주입될 수 있을 뿐이고, CLI 는 턴 중
///     stdin 을 다음 턴으로 큐잉한다(busy.rs `BUSY_MAX_TURN` 주석의 같은 근거).
///     ★"attached 표시 없으면 Appear forward 를 미룬다" 는 사전 점검은 **채택하지 않았다**(내부 결정 — 보고):
///     그 조건은 정상 상태(부착 실패·비-structured 수신자)에서도 참이라 배달을 **영구 보류**할 수 있고,
///     막으려는 것은 유실 없는 타이밍 어긋남뿐이다. 배달 정지 위험을 타이밍 개선과 바꾸지 않는다.
///
/// ★레인 task 는 **호출자(부팅)가 소유**한다 — 이 함수가 spawn 하지 않는다(round-3 finding 1, BLOCK)★:
///   옛 구현은 레인을 이 future **안에서** spawn 하고 JoinHandle 을 지역 변수로 들었다. 그러면 종료 경로가
///   main lane 을 abort 할 때 그 핸들이 **그냥 drop**(= detach, abort 아님)되므로 레인은 계속 살아 있고,
///   lib.rs 의 5s join belt 는 **정작 blocking 작업이 없는** main lane 만 감시하게 된다(모든 blocking inject 는
///   레인으로 옮겨졌다) → 진짜 blocking 을 지닌 task 가 belt 밖에 남아 런타임 drop 이 종료 시점에 hang 할 수
///   있다(round-3 finding 1 이 고쳤던 실패 모드의 재발). 그래서 두 task 를 **둘 다 호출자가 들고** 각각
///   abort + belt 로 내린다(`spawn_flush_worker` / `FlushWorkerHandles::shutdown`).
/// ★수명★: main lane 이 끝나면(또는 abort 되면) `lane_tx` 가 drop 되어 레인도 자연 종료한다 — 단 레인은
///   **큐에 남은 배달을 다 처리한 뒤** 끝나므로 즉시 멈추지 않는다. 그래서 종료 경로는 `shutdown_all`(자식
///   kill·파이프 닫기)로 막힌 write 를 먼저 풀고, main → lane 순으로 abort 한다(lib.rs 종료 주석).
pub async fn run_flush_worker(
    mut flush_rx: mpsc::UnboundedReceiver<FlushMsg>,
    wiring: FlushWiring,
    lane_tx: mpsc::UnboundedSender<FlushMsg>,
) {
    while let Some(msg) = flush_rx.recv().await {
        match msg {
            FlushMsg::Detach { id } => {
                // 순수 맵 조작(짧은 락 2개, 외부 호출 없음) — spawn_blocking 불요.
                wiring.busy.forget(id);
            }
            FlushMsg::Attach { id, epoch } => {
                handle_attach(&wiring, id, epoch).await;
            }
            // 배달 작업 → flush 레인으로 넘기고 즉시 다음 생애주기 메시지를 본다.
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
    /// 생애주기 레인(Attach/Detach) — blocking 작업은 유계(subscribe).
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
    ///     ② 배달 레인 밖의 유일한 blocking 작업인 `BusyTracker::attach` 의 subscribe(replay)는 **메모리
    ///        바운드**다(링 ≤4096 복사, 외부 I/O 없음) → 유계 시간에 끝난다.
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
    let lane = tokio::spawn(run_flush_lane(
        lane_rx,
        wiring.messaging.clone(),
        wiring.idle.clone(),
    ));
    let main = tokio::spawn(run_flush_worker(flush_rx, wiring, lane_tx));
    FlushWorkerHandles { main, lane }
}

/// 종료 join belt — abort 후 이 시간 안에 안 끝나면 warn 후 detach(데몬 종료 hang 방지, round-3 finding 1).
const FLUSH_JOIN_BELT: Duration = Duration::from_secs(5);

/// Attach 1건 집행 — blocking pool 격리 + 실패 피드백(C2 리뷰 fix 8a).
///
/// `AttachOutcome::Failed`(subscribe 가 Err 를 돌려준 정상 실패)면:
///   ① `RosterDiff::forget_attached` 로 diff 스냅샷을 무효화한다 → 다음 로스터 업데이트가 Attach 를 다시
///      낸다(스냅샷을 그대로 두면 그 에이전트는 영구히 tap 없이 = 게이트 없이 돈다).
///   ② **유계 즉시 재시도 1회**. 채널 자기-send 로 재큐잉하지 않는 이유: worker 가 자기 채널 송신단을
///      들면 채널이 영원히 닫히지 않아 종료 인과가 흐려진다. 인라인 재시도는 상한이 이 함수 구조에 박혀
///      있어(재시도의 재시도가 없다) 폭주가 불가능하다.
/// ★성공 재시도 후의 잔여★: `forget_attached` 로 스냅샷을 이미 지웠으므로 다음 로스터 업데이트가 같은
///   Attach 를 한 번 더 낸다 → tracker 가 `AlreadyAttached` 로 접는다(무해한 no-op).
///
/// ★패닉은 **재시도하지 않는다**(round-3 finding 6 · load-bearing)★: core `subscribe_from` 은 sink 를
///   구독자 목록에 **push 한 뒤** replay 락을 `expect` 로 잡는다(output_core.rs) — 거기서 패닉하면 ① 그 sink
///   는 **이미 등록된 채** 남고 ② subscribers 락이 poison 돼 그 에이전트의 pump 가 죽는다. 즉 패닉 후의
///   재시도는 "고장 난 core 에 sink 를 하나 더 붙이는" 시도이고, 성공하면 tap 이 둘(통지 중복), 실패하면
///   그냥 낭비다. 그래서 패닉(JoinError)은 warn + 스냅샷 무효화까지만 하고 **즉시 반환**한다 — 재시도 판단은
///   다음 로스터 업데이트에 맡긴다(그때는 epoch/상태가 바뀌어 있을 수 있다). 그 전까지 그 대상은 게이트
///   관점 idle 폴백(positive-knowledge-only)이라 배달이 막히지는 않는다.
///   JoinError 가 Cancelled 인 경우(런타임 종료)도 같은 처리 — 종료 중에 재시도할 이유가 없다.
async fn handle_attach(wiring: &FlushWiring, id: AgentId, epoch: u32) {
    use crate::messaging::busy::AttachOutcome;
    let busy = wiring.busy.clone();
    // subscribe = 코어 subscribers 락 구간 → runtime worker 를 굶기지 않게 blocking pool 로.
    //   live-only 구독이라 링 replay 는 0건이고, 이 호출은 자식 프로세스에 의존하지 않는다(유계).
    let outcome = match tokio::task::spawn_blocking(move || busy.attach(id, epoch)).await {
        Ok(o) => o,
        Err(e) => {
            // blocking task panic/cancel — 부착 표시는 tracker 의 Drop 가드가 이미 되돌렸다(busy.rs fix 8b).
            //   ★재시도 금지★(위 헤더): 등록된 sink + poison 된 락 위에 두 번째 sink 를 얹지 않는다.
            tracing::warn!(
                agent = %id,
                epoch,
                panicked = e.is_panic(),
                "턴 tap attach blocking task 비정상 종료 — 재시도 없이 다음 로스터 업데이트에 맡김(그 전까지 idle 폴백): {e}"
            );
            wiring.diff.forget_attached(id);
            return;
        }
    };
    if outcome != AttachOutcome::Failed {
        return;
    }
    // 정상 실패 → 다음 로스터 diff 가 재시도할 수 있게 스냅샷 무효화 + 즉시 1회 재시도.
    wiring.diff.forget_attached(id);
    tracing::debug!(agent = %id, epoch, "턴 tap attach 실패 — 즉시 1회 재시도(유계)");
    let busy = wiring.busy.clone();
    match tokio::task::spawn_blocking(move || busy.attach(id, epoch)).await {
        Ok(AttachOutcome::Failed) | Err(_) => {
            // 재시도도 실패 — 여기서 멈춘다(다음 로스터 업데이트가 다시 시도). 그 전까지 그 대상은
            //   게이트 관점 idle 폴백(positive-knowledge-only) — 배달은 막히지 않는다.
            wiring.diff.forget_attached(id);
            tracing::warn!(agent = %id, epoch, "턴 tap attach 재시도 실패 — 다음 로스터 업데이트에 재시도(그 전까지 idle 폴백)");
        }
        Ok(_) => {}
    }
}

/// ★flush 레인(C2 리뷰 fix 3)★ — 배달 작업(Appear/Idle) 전용 **직렬** 소비자. 여기의 blocking write 가
///   막혀도 main lane(Attach/Detach)은 계속 돈다.
///
/// ★직렬 유지가 load-bearing★: 같은 수신자의 배치는 "오래된 순" 을 지켜야 하므로(ADR-0104) 이 레인 안에서
///   병렬 실행하지 않는다. 서로 다른 수신자끼리도 직렬이라 한 막힌 수신자가 다른 수신자의 배달을 늦출 수는
///   있으나(수용된 잔여 — 사람 대화 수준 메시지율), **생애주기 작업**은 더 이상 그 뒤에 서지 않는다.
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
            // 생애주기 메시지는 main lane 이 처리한다(여기 오지 않는다 — forward 분기 참조).
            FlushMsg::Attach { .. } | FlushMsg::Detach { .. } => {}
        }
    }
    tracing::debug!("flush 레인 종료(채널 닫힘)");
}

/// flush worker 배선 묶음 — 인자 수를 줄이고 "무엇을 공유하는가" 를 한눈에 보이게 한다(부팅에서 조립).
#[derive(Clone)]
pub struct FlushWiring {
    /// MessagingService 늦은 주입 슬롯(manager 조립 후에 채워진다).
    pub messaging: Arc<crate::control::mcp_server::MessagingSlot>,
    /// 턴 관측 tracker — Attach/Detach 집행 대상.
    pub busy: Arc<crate::messaging::busy::BusyTracker>,
    /// 로스터 diff 시퀀싱 상태 — attach 실패 시 스냅샷 무효화(재시도 개방)용으로 공유한다.
    pub diff: Arc<RosterDiff>,
    /// Idle coalescing 집합 — 통지 측(ChannelIdleNotifier)과 공유(집어들 때 해제).
    pub idle: Arc<IdleCoalescer>,
}

impl StatusSink for MessagingFlushSink {
    fn status_changed(&self, id: AgentId, status: CoreStatus, epoch: u32) {
        self.inner.status_changed(id, status, epoch);
    }

    fn agent_list_updated(&self, agents: Vec<CoreAgentInfo>) {
        // ★로스터 diff → flush 작업 enqueue(C2 리뷰 fix 7)★: 두 축(id·이름) 스냅샷 갱신과 채널 enqueue 를
        //   `RosterDiff` 가 **한 락 아래에서** 한다 — 동시 콜백 사이에서 "스냅샷 순서 = 채널 순서" 를
        //   보장해야 옛 Detach 가 새 Attach 뒤에 도착해 방금 붙인 tap 상태를 지우는 사고가 없다(그 rationale
        //   과 Attach→Detach→Appear 순서 근거는 `RosterDiff::dispatch`). unbounded send 는 논블록이라 이
        //   콜백은 여전히 배치 크기와 무관하게 짧다(finding 5 — 실제 flush 는 worker 가 수행).
        self.diff.dispatch(&agents, &self.flush_tx);

        // 프론트 fanout 은 그대로 delegate(감싼 sink 의 본래 책임 — broadcast). flush 를 worker 로 뺐으므로
        //   이 forwarding 이 blocking write 뒤로 밀리지 않는다(spawn/reap/프론트 업데이트 지연 제거).
        self.inner.agent_list_updated(agents);
    }

    fn restore_result(&self, report: CoreRestoreReport) {
        self.inner.restore_result(report);
    }
}

// ── WsOutputSink(연결당 출력 sink, pump 스레드에서 호출) ───────────────────────────

/// 한 연결의 한 에이전트 구독에 대응하는 OutputSink. pump 스레드가 `send` 를 호출한다.
/// frame 을 codec binary 로 인코딩해 conn_tx 에 **try_send 만**(block 금지) 한다.
/// 큐가 full/closed 면 SinkError 반환(코어가 dead-sink 로 제거) + out-of-band close 신호.
pub struct WsOutputSink {
    conn_tx: mpsc::Sender<WsOutbound>,
    /// 큐 밖 종료 신호. full 감지 시 notify_one — write_task 가 큐가 막혀도 깨어 닫는다.
    /// ★pump 스레드(sync)에서 notify_one 호출 OK — Notify 는 sync-safe.
    close_signal: Arc<Notify>,
    /// replay 구간 중 try_send 실패(frame drop)가 한 번이라도 있었는지.
    /// handle_subscribe 가 ReplayComplete 직전 검사해 SubscribeAck.truncated 를 사후 보정한다.
    /// 평소(라이브)엔 코어가 dead-sink 로 제거하므로 의미가 없고, replay 구간 정확성에만 쓴다.
    replay_dropped: Arc<AtomicBool>,
    sink_id: SinkId,
}

impl WsOutputSink {
    pub(crate) fn new(conn_tx: mpsc::Sender<WsOutbound>, close_signal: Arc<Notify>) -> Self {
        Self {
            conn_tx,
            close_signal,
            replay_dropped: Arc::new(AtomicBool::new(false)),
            sink_id: uuid::Uuid::new_v4(),
        }
    }

    /// replay 구간 동안 frame 이 drop 됐는지 사후 검사용 핸들(handle_subscribe 가 공유 보관).
    pub(crate) fn replay_dropped_flag(&self) -> Arc<AtomicBool> {
        self.replay_dropped.clone()
    }
}

// ── WsOutboundSink(연결당 control sink, ConnectionCore.dispatch 의 응답 경로) ──────────
//
// ConnectionCore 의 `OutboundSink` 를 WS 로 구현한다. dispatch 가 enqueue 하는 Outbound 를
// WsOutbound 로 변환해 conn_tx(단일 writer 큐)에 넣는다. 인코딩(AgentEvent→JSON text)은 이
// 어댑터가 소유한다(코어는 모름 — ADR-0003 정합).
//
// ★FIFO(R1)★: dispatch 의 control(Ack/SubscribeAck/ReplayComplete/Error 등)과 코어 output
// 평면(WsOutputSink 의 binary frame)이 같은 conn_tx 단일 writer 로 합류하므로, dispatch 가
// SubscribeAck 를 replay binary 보다 먼저 enqueue 하면 순서가 보존된다.
//
// ★R6 close_signal(out-of-band)★: 큐 포화 시 SinkError 를 반환하고, 동시에 close_signal 을
// notify 해 write_task 가 큐가 막혀도 깨어 닫게 한다(WS-특정 처리 — 어댑터에 잔류). enqueue 의
// `.await` 가 불가능한 sync trait 이므로 try_send 만 쓴다(control 도 큐 여유분으로 보통 성공).
pub struct WsOutboundSink {
    conn_tx: mpsc::Sender<WsOutbound>,
    close_signal: Arc<Notify>,
}

impl WsOutboundSink {
    pub(crate) fn new(conn_tx: mpsc::Sender<WsOutbound>, close_signal: Arc<Notify>) -> Self {
        Self {
            conn_tx,
            close_signal,
        }
    }
}

impl CoreOutboundSink for WsOutboundSink {
    fn enqueue(&self, out: Outbound) -> Result<(), CoreSinkError> {
        let msg = match out {
            // control 이벤트 — JSON text 로 인코딩(어댑터 소유). 직렬화 실패는 drop(기존 event_json 동작).
            Outbound::Event(ev) => match event_json(&ev) {
                Some(text) => WsOutbound::Text(text),
                None => return Ok(()), // 직렬화 실패는 무시(기존 `let _ = ...` 동작과 동일)
            },
            Outbound::Binary(b) => WsOutbound::Binary(b),
            Outbound::Close(reason) => WsOutbound::Close(reason),
        };
        match self.conn_tx.try_send(msg) {
            Ok(()) => Ok(()),
            Err(_) => {
                // 큐 포화/닫힘 — out-of-band close 신호로 write_task 를 깨운다(R6, WS-특정 잔류).
                self.close_signal.notify_one();
                Err(CoreSinkError)
            }
        }
    }

    fn make_output_sink(&self) -> (Arc<dyn OutputSink>, Arc<AtomicBool>) {
        // handle_subscribe 가 코어 subscribe_from 에 넘길 output 평면 sink. 같은 conn_tx/close_signal
        // 을 공유해 control(이 sink)과 output(WsOutputSink)이 한 단일 writer 큐로 합류한다(FIFO).
        // ★Stage 2 generic★: 반환을 Arc<dyn OutputSink> trait object 로(carrier-중립). replay_dropped
        //   플래그를 함께 돌려 handle_subscribe 가 truncated 사후 보정에 쓰게 한다.
        let sink = Arc::new(WsOutputSink::new(
            self.conn_tx.clone(),
            self.close_signal.clone(),
        ));
        let flag = sink.replay_dropped_flag();
        (sink, flag)
    }
}

impl OutputSink for WsOutputSink {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        // ★S15 B5/B7 payload 분기(ADR-0045)★: 콘솔 바이트는 tag0 terminal frame, 구조화 이벤트는 tag1
        //   structured frame 으로 인코딩한다. sink 가 wire 인코딩을 소유(코어는 wire 모름, ADR-0003) —
        //   Bytes 는 raw payload 를, Event 는 core `OutputEvent` → wire `StructuredEvent`(daemon adapter)
        //   → JSON payload 를 헤더에 실어 보낸다.
        //   ★현 배선 상태★: 구조화 이벤트 생산자(B3 decoder→pump 배선)는 아직 미배선이라 런타임엔 Bytes 만
        //   흐른다 — Event arm 은 B7 단위테스트(합성 OutputEvent)로만 도달·검증된다(정상).
        let buf = match frame.payload {
            OutputPayload::Bytes(b) => {
                encode_terminal_frame(frame.agent_id, frame.epoch, frame.seq, b)
            }
            // ★tag1 인코딩(B7)★: core OutputEvent → wire StructuredEvent(adapter) → JSON payload →
            //   tag1 structured frame. codec 은 payload 스키마 무지(opaque) — 직렬화 형식(JSON)·이벤트
            //   타입은 여기(daemon)가 소유한다(ADR-0045 self-describing).
            OutputPayload::Event(ev) => {
                // (1) core→wire 변환. TerminalBytes 가 여기 오면(정상 경로상 tag0 로 갈려 안 옴 — 상류
                //     배선 버그) 매핑 불가(None) → debug 는 조기 발견, release 는 warn 후 drop(연결 유지).
                let wire = match output_event_to_wire(ev) {
                    Some(w) => w,
                    None => {
                        debug_assert!(
                            false,
                            "TerminalBytes(tag0 전용)가 Event(tag1) arm 에 도달 — 상류 payload 분기 버그"
                        );
                        tracing::warn!(
                            agent = %frame.agent_id,
                            "tag1 인코딩 불가(TerminalBytes 가 Event arm 도달) — drop"
                        );
                        return Ok(());
                    }
                };
                // (2) JSON 직렬화. 실패는 거의 불가능(문자열/숫자 필드뿐)하나, 나면 이 frame 만 warn 후
                //     drop 한다(SinkError 로 연결을 죽이지 않음 — 직렬화 실패는 슬로우 소비자와 무관한
                //     데이터 문제고, control event_json 실패 처리와 동일 관례).
                let payload = match serde_json::to_vec(&wire) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!(
                            agent = %frame.agent_id,
                            "StructuredEvent 직렬화 실패 — drop: {e}"
                        );
                        return Ok(());
                    }
                };
                // (3) tag1 frame(헤더+payload). 헤더 레이아웃은 tag0 과 동일, tag=1(codec, ADR-0045).
                encode_structured_frame(frame.agent_id, frame.epoch, frame.seq, &payload)
            }
        };
        // ★pump 스레드 — try_send 만(절대 block 금지). full/closed = 느린 소비자 → 코어가 이 sink 제거.
        match self.conn_tx.try_send(WsOutbound::Binary(buf)) {
            Ok(()) => Ok(()),
            Err(_) => {
                // frame 이 drop 됐음을 기록(replay 구간 truncated 사후 보정용).
                self.replay_dropped.store(true, Ordering::Release);
                // ★out-of-band 종료 신호★: 큐가 full 이라 WsOutbound::Close try_send 는 실패할 수
                //   있으나, Notify 는 큐와 무관하게 write_task 를 깨운다(좀비 연결 방지).
                self.close_signal.notify_one();
                Err(SinkError)
            }
        }
    }

    fn sink_id(&self) -> SinkId {
        self.sink_id
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
/// WS 업그레이드 → auth → Hello/list push → read/write task → cleanup.
///
/// `expected_token` 은 daemon.json 의 토큰. `shutdown_tx` 는 StopDaemon 수신 시 main 종료를 트리거.
#[allow(clippy::too_many_arguments)]
pub async fn handle_connection(
    stream: TcpStream,
    peer: std::net::SocketAddr,
    manager: Arc<AgentManager>,
    registry: ConnRegistry,
    multiview: MultiViewState,
    // ADR-0096: 제어 채널 레지스트리(봉투 포맷 전역 상태 거처) — SetEnvelopeFormat dispatch 가 쓴다.
    //   handle_send(MCP/CLI)가 relay 마다 읽는 그 같은 Arc(전역 상태 하나).
    control_registry: Arc<crate::control::registry::ControlRegistry>,
    expected_token: Arc<String>,
    shutdown_tx: watch::Sender<bool>,
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
                        let _ = send_error_and_close(&mut ws, "auth failed").await;
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
                            &format!(
                                "protocol_version mismatch: client {protocol_version} != server {PROTOCOL_VERSION}"
                            ),
                        )
                        .await;
                        return;
                    }
                    tracing::info!(%peer, "auth 성공");
                }
                Ok(_) => {
                    tracing::warn!(%peer, "첫 frame 이 Auth 가 아님 — close");
                    let _ = send_error_and_close(&mut ws, "expected Auth as first frame").await;
                    return;
                }
                Err(e) => {
                    tracing::warn!(%peer, "첫 frame 파싱 실패: {e} — close");
                    let _ = send_error_and_close(&mut ws, "invalid first frame").await;
                    return;
                }
            }
        }
        Ok(Some(Ok(_))) => {
            tracing::warn!(%peer, "첫 frame 이 Text 가 아님 — close");
            let _ = send_error_and_close(&mut ws, "expected Auth text frame").await;
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
            let _ = send_error_and_close(&mut ws, "auth timeout").await;
            return;
        }
    }

    // 3) conn_tx/rx 생성 + close_signal + 레지스트리 등록 + split.
    let (conn_tx, conn_rx) = mpsc::channel::<WsOutbound>(CONN_TX_CAP);
    // ★out-of-band 종료 신호★: 큐 포화로 WsOutbound::Close 마저 못 들어갈 때 write_task 를 깨운다.
    let close_signal = Arc::new(Notify::new());
    let conn_id = registry.alloc_id();
    registry.register(conn_id, conn_tx.clone());
    tracing::info!(%peer, conn = conn_id, "연결 인증 완료 — 등록");

    let (sink_half, stream_half) = ws.split();

    // 3b) ConnectionCore(transport-중립 dispatch) 배선. 연결당 1개 — manager/multiview/registry/
    //     shutdown_tx 는 전 연결이 공유하나, dispatch 호출 경로를 캡슐화하려고 연결마다 묶는다.
    //     read_task 가 이걸 통해 명령을 처리하고, cleanup 도 core 의 manager/multiview/registry 를 쓴다.
    let core = Arc::new(ConnectionCore::new(
        manager.clone(),
        multiview.clone(),
        registry.clone(),
        control_registry.clone(),
        shutdown_tx,
    ));

    // 4) 연결 직후 Hello + 현재 목록 push(단일 writer 큐 경유 — 이후 모든 출력과 FIFO 정렬).
    if let Some(text) = event_json(&hello_event(env!("CARGO_PKG_VERSION").into())) {
        let _ = conn_tx.send(WsOutbound::Text(text)).await;
    }
    if let Some(text) = event_json(&agent_list_event(&manager)) {
        let _ = conn_tx.send(WsOutbound::Text(text)).await;
    }

    // 5) 이 연결의 per-conn 수명 상태(subs/owned_viewports + conn_id). read_task 와 cleanup 이 공유.
    let session = Arc::new(ConnectionSession::new(conn_id));

    // ── keepalive 공유 시계(A) ──────────────────────────────────────────────────────
    // base = 연결 시작 시각(tokio Instant). last_recv = base 기준 경과 ms(AtomicU64).
    // read_task 가 클라로부터 무언가(Pong 포함) 받을 때마다 갱신하고, write_task 의 ping arm 이
    // base.elapsed() - last_recv 로 idle 경과를 계산해 idle_timeout 초과 시 close_signal 발동.
    let keepalive_base = tokio::time::Instant::now();
    let last_recv = Arc::new(AtomicU64::new(0));

    // read_task: stream_half 에서 명령을 읽어 ConnectionCore.dispatch 로 처리. 응답은 WsOutboundSink
    //   (control)와 WsOutputSink(output, handle_subscribe 가 생성)가 conn_tx 로 큐잉한다.
    //   close_signal 은 두 sink 에 공유(full 시 write_task 깨우기 — R6).
    let mut read_handle = tokio::spawn(read_task(
        stream_half,
        conn_tx.clone(),
        core.clone(),
        session.clone(),
        conn_id,
        close_signal.clone(),
        keepalive_base,
        last_recv.clone(),
    ));

    // write_task: conn_rx 에서 받은 WsOutbound 를 sink_half 로 순서대로 write(단일 writer).
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

    // 6) 하나라도 끝나면 cleanup. ★살아남은 쪽을 명시적으로 abort★ — JoinHandle 을 그냥 drop 하면
    //    task 가 detach 되어 계속 돈다(WS half 를 붙든 채 좀비). 그래서 &mut 로 select 해 핸들을
    //    소비하지 않고, 진 쪽을 abort 한다(연결의 read/write 가 함께 끝나게).
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
    // 이 연결이 등록한 모든 (agent_id, sink_id) 를 manager 에서 unsubscribe + 레지스트리 제거.
    // 안 하면 죽은 conn_tx 로 영원히 try_send 하는 좀비 sink 가 코어 subscribers 에 남는다
    // (코어가 try_send 실패로 결국 제거하긴 하나, 다음 emit 까지 잔존 — 명시적으로 끊는다).
    let leftovers: Vec<(AgentId, SinkId)> = {
        let guard = session.subs.lock().expect("subs poisoned");
        guard.iter().map(|(a, s)| (*a, *s)).collect()
    };
    for (agent_id, sink_id) in leftovers {
        let _ = manager.unsubscribe(agent_id, sink_id);
    }

    // ── 멀티뷰어 cleanup ───────────────────────────────────────────────────────
    // (a) viewport 재협상: 끊긴 연결의 viewport 들을 맵에서 빼고, 영향받은 agent 를 남은 뷰어 기준
    //     smallest 로 다시 resize 한다(tmux detach 후 잔여 클라 기준으로 다시 키우는 것과 동일).
    //     ★lock 순서★: remove_conn_viewports 가 multiview lock 안에서 협상값만 계산해 반환한 뒤
    //     lock 을 푼 상태에서 manager.resize 를 부른다(lock 보유 중 코어 호출 금지).
    let owned: Vec<(AgentId, String)> = {
        let g = session
            .owned_viewports
            .lock()
            .expect("owned_viewports poisoned");
        g.clone()
    };
    if !owned.is_empty() {
        for (agent_id, negotiated) in core.multiview().remove_conn_viewports(&owned) {
            if let Some((cols, rows)) = negotiated {
                // 남은 뷰어가 있으면 그 smallest 로 복귀. 없으면(None) 그대로 둔다(마지막 크기 유지).
                let _ = manager.resize(agent_id, cols, rows);
            }
        }
    }
    // (b) 입력 lease 자동 해제: 보유자가 끊기면 다른 뷰어가 영영 막히면 안 된다(좀비 lock 방지).
    //     해제된 agent 는 이제 lease 가 비었으니 InputLeaseChanged{held:false} 를 전 연결에 통보.
    for agent_id in core.multiview().release_all_for_conn(conn_id) {
        crate::connection_core::broadcast_lease_changed(&registry, agent_id, false);
    }

    registry.unregister(conn_id);
    tracing::info!(%peer, conn = conn_id, "연결 종료 — cleanup 완료");
}

/// auth 실패 시 Error + close 를 직접(레지스트리 등록 전이라 conn_tx 없음) 보낸다.
async fn send_error_and_close(
    ws: &mut tokio_tungstenite::WebSocketStream<TcpStream>,
    message: &str,
) -> Result<(), tokio_tungstenite::tungstenite::Error> {
    if let Some(text) = event_json(&AgentEvent::Error {
        request_id: None,
        message: message.to_string(),
    }) {
        ws.send(Message::Text(text.into())).await?;
    }
    ws.close(None).await
}

// ── write_task(단일 writer) ───────────────────────────────────────────────────

type SinkHalf =
    futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<TcpStream>, Message>;

/// conn_rx 에서 받은 출력을 sink_half 로 순서대로 write. 이게 이 연결의 유일한 writer.
/// 종료 트리거 3가지: (1) conn_rx 큐의 WsOutbound::Close, (2) sink send 실패, (3) close_signal.
///
/// ★out-of-band 종료(M1 핵심)★: conn_tx 가 full 이면 WsOutbound::Close 마저 큐에 못 들어가
/// 좀비 연결이 된다. 그래서 `tokio::select!` 로 conn_rx.recv() 와 close_signal.notified() 를
/// 동시에 대기한다. WsOutputSink 가 full 을 만나 `close_signal.notify_one()` 하면, 큐가
/// 가득 차 있어도 이 select 가 깨어 sink_half.close() 후 break → cleanup 으로 이어진다.
#[allow(clippy::too_many_arguments)]
async fn write_task(
    mut sink_half: SinkHalf,
    mut conn_rx: mpsc::Receiver<WsOutbound>,
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
                    WsOutbound::Text(s) => Message::Text(s.into()),
                    WsOutbound::Binary(b) => Message::Binary(b.into()),
                    WsOutbound::Close(reason) => {
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

type StreamHalf = futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<TcpStream>>;

/// stream_half 에서 명령 frame 을 읽어 ConnectionCore.dispatch 로 처리. 응답은 WsOutboundSink
/// (control)를 통해 conn_tx 로 큐잉된다(직접 write 안 함).
///
/// ★Stage 1 배선★: 옛 read_task 는 dispatch 자유함수를 직접 불렀다. 이제 transport-중립
/// ConnectionCore 가 dispatch 를 소유하고, read_task 는 WS 프레임→AgentCommand 파싱과
/// WsOutboundSink(control 인코딩)만 담당한다(carrier 경계). DispatchFlow::Close 면 루프 탈출
/// (옛 dispatch 의 bool true 와 동일 동작 — StopDaemon).
#[allow(clippy::too_many_arguments)]
async fn read_task(
    mut stream_half: StreamHalf,
    conn_tx: mpsc::Sender<WsOutbound>,
    core: Arc<ConnectionCore>,
    session: Arc<ConnectionSession>,
    conn_id: ConnId,
    close_signal: Arc<Notify>,
    keepalive_base: tokio::time::Instant,
    last_recv: Arc<AtomicU64>,
) {
    use crate::connection_core::DispatchFlow;

    // 이 연결의 control 응답 sink — dispatch 의 Ack/Error/SubscribeAck/ReplayComplete 등이 여기로.
    // output 평면(replay/live binary)은 handle_subscribe 가 make_output_sink 로 별도 생성하나,
    // 같은 conn_tx/close_signal 을 공유해 한 단일 writer 큐로 합류한다(FIFO 보존).
    let ws_sink = WsOutboundSink::new(conn_tx.clone(), close_signal.clone());

    while let Some(item) = stream_half.next().await {
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
                match serde_json::from_str::<AgentCommand>(&text) {
                    Ok(cmd) => {
                        if core.dispatch(cmd, &session, &ws_sink).await == DispatchFlow::Close {
                            // dispatch 가 연결 종료를 요청(StopDaemon 등) — 루프 탈출.
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(conn = conn_id, "명령 파싱 실패: {e}");
                        // 옛 send_error(conn_tx, ..) 와 동일: Error 이벤트를 conn_tx 로 큐잉.
                        let _ = ws_sink.enqueue(Outbound::event(AgentEvent::Error {
                            request_id: None,
                            message: format!("invalid command: {e}"),
                        }));
                    }
                }
            }
            Message::Binary(_) => {
                // 클라→데몬 binary 는 프로토콜에 없음 — 오류로 보고 종료.
                tracing::warn!(conn = conn_id, "예상치 못한 binary frame — close");
                let _ = ws_sink.enqueue(Outbound::event(AgentEvent::Error {
                    request_id: None,
                    message: "unexpected binary frame".into(),
                }));
                let _ = conn_tx
                    .send(WsOutbound::Close("protocol error".into()))
                    .await;
                break;
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

    // ── 3. WsOutbound 매핑(Text/Binary/Close → Message) ──────────────────────
    // write_task 의 변환 로직과 동일한 매핑을 직접 검증(실제 WS 없이).
    #[test]
    fn ws_outbound_maps_to_message() {
        let t = WsOutbound::Text("hi".into());
        let b = WsOutbound::Binary(vec![1, 2, 3]);
        let c = WsOutbound::Close("bye".into());

        let to_msg = |o: WsOutbound| -> Message {
            match o {
                WsOutbound::Text(s) => Message::Text(s.into()),
                WsOutbound::Binary(b) => Message::Binary(b.into()),
                WsOutbound::Close(_) => Message::Close(None),
            }
        };
        assert!(matches!(to_msg(t), Message::Text(_)));
        assert!(matches!(to_msg(b), Message::Binary(_)));
        assert!(matches!(to_msg(c), Message::Close(_)));
    }

    // ── 4. WsOutputSink 가 conn_tx 에 binary frame 을 try_send 하는지 ─────────
    #[tokio::test]
    async fn ws_output_sink_encodes_and_sends_binary() {
        let (tx, mut rx) = mpsc::channel::<WsOutbound>(8);
        let sink = WsOutputSink::new(tx, Arc::new(Notify::new()));
        let agent_id = uuid::Uuid::new_v4();
        let data = b"abc";
        let frame = OutputFrame {
            agent_id,
            epoch: 7,
            seq: 42,
            payload: OutputPayload::Bytes(data),
        };
        sink.send(frame).expect("send ok");

        match rx.recv().await.expect("one item") {
            WsOutbound::Binary(buf) => {
                // codec 으로 디코드해 헤더가 맞는지 확인.
                let decoded = engram_dashboard_protocol::decode_frame(&buf).expect("decode");
                assert_eq!(decoded.agent_id, agent_id);
                assert_eq!(decoded.epoch, 7);
                assert_eq!(decoded.seq, 42);
                assert_eq!(decoded.payload, b"abc");
            }
            _ => panic!("Binary 가 아님"),
        }
    }

    // ── 4b. (S15 B7) WsOutputSink 가 Event(구조화) payload 를 tag1 frame 으로 인코딩하는지 ──────
    //    합성 OutputEvent → send → conn_tx 의 Binary 를 decode_frame 으로 풀어 tag1·헤더 확인 후,
    //    payload 를 다시 wire StructuredEvent 로 serde 파싱해 필드가 보존됐는지 단언(ADR-0045 self-describing).
    #[tokio::test]
    async fn ws_output_sink_encodes_event_as_tag1_structured_frame() {
        use engram_dashboard_core::agent::types::OutputEvent as CoreOutputEvent;
        use engram_dashboard_protocol::{
            decode_frame, StructuredEvent as WireStructuredEvent, FRAME_TAG_STRUCTURED_EVENT,
        };

        let (tx, mut rx) = mpsc::channel::<WsOutbound>(8);
        let sink = WsOutputSink::new(tx, Arc::new(Notify::new()));
        let agent_id = uuid::Uuid::new_v4();
        // 합성 구조화 이벤트(B3 미배선이라 런타임 생산자 없음 — 여기선 직접 만들어 tag1 경로를 태운다).
        let ev = CoreOutputEvent::ToolCall {
            name: "read".into(),
            args_json: r#"{"path":"/x"}"#.into(),
            id: Some("call_1".into()),
            turn_id: Some("t9".into()),
            message_id: None,
        };
        let frame = OutputFrame {
            agent_id,
            epoch: 3,
            seq: 100,
            payload: OutputPayload::Event(&ev),
        };
        sink.send(frame).expect("Event send ok");

        match rx.recv().await.expect("one item") {
            WsOutbound::Binary(buf) => {
                let decoded = decode_frame(&buf).expect("decode");
                // tag=1(structured) + 헤더 필드 그대로.
                assert_eq!(decoded.tag, FRAME_TAG_STRUCTURED_EVENT, "tag1 이어야 함");
                assert_eq!(decoded.agent_id, agent_id);
                assert_eq!(decoded.epoch, 3);
                assert_eq!(decoded.seq, 100);
                // payload = JSON self-describing StructuredEvent. 파싱해 필드 보존 단언.
                let parsed: WireStructuredEvent =
                    serde_json::from_slice(decoded.payload).expect("payload JSON 파싱");
                assert_eq!(
                    parsed,
                    WireStructuredEvent::ToolCall {
                        name: "read".into(),
                        args_json: r#"{"path":"/x"}"#.into(),
                        id: Some("call_1".into()),
                        turn_id: Some("t9".into()),
                        message_id: None,
                    },
                    "tag1 payload 가 wire StructuredEvent 로 무손실 복원"
                );
            }
            other => panic!("Binary(tag1) 여야 함: {other:?}"),
        }
    }

    // ── 5. WsOutputSink full → SinkError + close_signal notify + replay_dropped ──
    #[tokio::test]
    async fn ws_output_sink_full_returns_error_and_notifies_close_signal() {
        // cap 1 채널을 가득 채운 뒤: send 가 Err 를 반환하고, 큐가 막혀 있어도 out-of-band
        // close_signal 이 발동(write_task 를 깨움)하며, replay_dropped 가 set 되는지.
        let (tx, mut rx) = mpsc::channel::<WsOutbound>(1);
        let close_signal = Arc::new(Notify::new());
        let sink = WsOutputSink::new(tx, close_signal.clone());
        let replay_dropped = sink.replay_dropped_flag();
        let agent_id = uuid::Uuid::new_v4();
        let frame = |seq: u64| OutputFrame {
            agent_id,
            epoch: 0,
            seq,
            payload: OutputPayload::Bytes(b"x"),
        };
        // 첫 send 성공(큐 1칸 채움).
        sink.send(frame(0)).expect("first ok");
        // 두 번째는 full → Err.
        assert!(sink.send(frame(1)).is_err(), "full 이면 SinkError");

        // ★out-of-band 종료 신호★: 큐가 full 이어도 close_signal 은 발동해야 한다.
        //   notified() 가 즉시 깨면 write_task 가 깨어 닫을 수 있다는 의미(M1 핵심 근거).
        tokio::time::timeout(Duration::from_millis(200), close_signal.notified())
            .await
            .expect("close_signal 이 full 에서도 발동해야 함");

        // replay 구간 사후 보정용 플래그도 set.
        assert!(
            replay_dropped.load(Ordering::Acquire),
            "drop 시 replay_dropped set"
        );

        // 큐 첫 항목은 Binary(첫 frame).
        assert!(matches!(rx.recv().await.unwrap(), WsOutbound::Binary(_)));
    }

    // ── 6. Subscribe 시 conn_tx 에 SubscribeAck → ReplayComplete 순서로 들어가는지 ──
    //    (mock manager 가 없어 실 AgentManager 의 비어있는 snapshot 경로로는 NotFound 가 나므로,
    //     여기선 control 메시지 순서 로직을 직접 재현해 검증한다. 실 manager subscribe 의 replay
    //     동기 전송은 output_core.rs 단위테스트가 이미 커버.)
    #[tokio::test]
    async fn subscribe_control_order_ack_then_complete() {
        use engram_dashboard_protocol::SubscribeAction;
        let (tx, mut rx) = mpsc::channel::<WsOutbound>(16);
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
        tx.send(WsOutbound::Text(ack)).await.unwrap();
        // 가상의 replay binary 1건.
        tx.send(WsOutbound::Binary(encode_terminal_frame(
            agent_id, 0, 0, b"r",
        )))
        .await
        .unwrap();
        let complete = event_json(&AgentEvent::ReplayComplete { agent_id, epoch: 0 }).unwrap();
        tx.send(WsOutbound::Text(complete)).await.unwrap();

        // 순서 검증: Text(SubscribeAck) → Binary(replay) → Text(ReplayComplete).
        let first = rx.recv().await.unwrap();
        let second = rx.recv().await.unwrap();
        let third = rx.recv().await.unwrap();

        match first {
            WsOutbound::Text(s) => assert!(s.contains("SubscribeAck")),
            _ => panic!("1번째는 SubscribeAck Text 여야 함"),
        }
        assert!(
            matches!(second, WsOutbound::Binary(_)),
            "2번째는 replay binary"
        );
        match third {
            WsOutbound::Text(s) => assert!(s.contains("ReplayComplete")),
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
        let (sink, rx, _diff) = flush_sink_with_diff();
        (sink, rx)
    }

    /// diff 상태까지 함께 돌려주는 조립 — attach 실패 피드백(forget_attached) 검증용.
    fn flush_sink_with_diff() -> (
        MessagingFlushSink,
        mpsc::UnboundedReceiver<FlushMsg>,
        Arc<RosterDiff>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel::<FlushMsg>();
        let diff = Arc::new(RosterDiff::new());
        let sink = MessagingFlushSink::new_test(Box::new(TestNoopSink), tx, diff.clone());
        (sink, rx, diff)
    }

    /// 채널에 쌓인 모든 작업을 순서대로 뽑는다(C2 — Attach/Detach/Appear/Idle 전부).
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
    fn flush_sink_skips_non_reachable() {
        // 비-structured(TUI) 또는 terminal 상태는 flush 후보 아님(파킹 수신 대상 아님).
        let (sink, mut rx) = flush_sink();
        let tui = AgentId::new_v4();
        let dead = AgentId::new_v4();
        sink.agent_list_updated(vec![
            flush_info(tui, "tui", 0, false, CoreStatus::Running), // 비-structured
            flush_info(dead, "dead", 0, true, CoreStatus::Killed), // terminal
        ]);
        assert!(
            drain_msgs(&mut rx).is_empty(),
            "비-도달·terminal 은 flush/tap 대상 아님"
        );
    }

    // ── 7b. C2: id 축 diff — 턴 관측 tap 부착/해제 enqueue(ADR-0104 결정 3) ─────────────────

    #[test]
    fn flush_sink_enqueues_attach_before_appear_for_new_agent() {
        // ★순서가 load-bearing★: Attach 가 Appear 보다 앞서야 등장 flush 주입이 만드는 유저 에코부터
        //   tap 이 관측한다(첫 턴을 놓치지 않음 — 그 사이 도착 메시지의 턴 중 주입 방지).
        let (sink, mut rx) = flush_sink();
        let id = AgentId::new_v4();
        sink.agent_list_updated(vec![flush_info(id, "alice", 0, true, CoreStatus::Running)]);
        assert_eq!(
            drain_msgs(&mut rx),
            vec![
                FlushMsg::Attach { id, epoch: 0 },
                FlushMsg::Appear {
                    name: "alice".to_string(),
                    id
                },
            ],
            "Attach → Appear 순서(단일 채널 FIFO)"
        );
    }

    #[test]
    fn flush_sink_attaches_all_ids_even_when_name_ambiguous() {
        // ★이름 축과 id 축의 판정이 다르다★: 동명 다수는 **배달**(Appear)에선 skip 이지만, tap 은 출력
        //   스트림 단위라 **둘 다** 붙어야 한다(안 붙으면 그 에이전트는 영구 idle 폴백 = 턴 중 주입).
        let (sink, mut rx) = flush_sink();
        let a = AgentId::new_v4();
        let b = AgentId::new_v4();
        sink.agent_list_updated(vec![
            flush_info(a, "dup", 0, true, CoreStatus::Running),
            flush_info(b, "dup", 0, true, CoreStatus::Running),
        ]);
        let msgs = drain_msgs(&mut rx);
        let mut attached: Vec<AgentId> = msgs
            .iter()
            .filter_map(|m| match m {
                FlushMsg::Attach { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        attached.sort();
        let mut expect = vec![a, b];
        expect.sort();
        assert_eq!(attached, expect, "동명이어도 id 별로 전부 tap 부착");
        assert!(
            !msgs.iter().any(|m| matches!(m, FlushMsg::Appear { .. })),
            "동명 다수는 배달 flush(Appear) 대상 아님(C1 정책 불변)"
        );
    }

    #[test]
    fn flush_sink_reattaches_on_epoch_bump_only() {
        // epoch bump = 새 OutputCore → 재부착 필요. 같은 (id, epoch) 재-push 는 노이즈라 skip.
        let (sink, mut rx) = flush_sink();
        let id = AgentId::new_v4();
        sink.agent_list_updated(vec![flush_info(id, "a", 0, true, CoreStatus::Running)]);
        assert!(drain_msgs(&mut rx).contains(&FlushMsg::Attach { id, epoch: 0 }));
        sink.agent_list_updated(vec![flush_info(id, "a", 0, true, CoreStatus::Running)]);
        assert!(
            drain_msgs(&mut rx).is_empty(),
            "같은 (id, epoch) 재-push 는 attach 도 안 낸다"
        );
        sink.agent_list_updated(vec![flush_info(id, "a", 1, true, CoreStatus::Running)]);
        assert!(
            drain_msgs(&mut rx).contains(&FlushMsg::Attach { id, epoch: 1 }),
            "epoch bump 은 재부착(구독은 epoch 을 넘지 못한다)"
        );
    }

    #[test]
    fn flush_sink_enqueues_detach_when_agent_leaves_roster() {
        // 이탈(죽음/reap) → Detach 로 턴 상태 청소 요청. 안 하면 죽은 대상 busy 플래그가 남아 그 이름 앞
        //   파킹이 stranded 된다.
        let (sink, mut rx) = flush_sink();
        let id = AgentId::new_v4();
        sink.agent_list_updated(vec![flush_info(id, "a", 0, true, CoreStatus::Running)]);
        let _ = drain_msgs(&mut rx);
        sink.agent_list_updated(vec![]);
        assert_eq!(
            drain_msgs(&mut rx),
            vec![FlushMsg::Detach { id }],
            "로스터 이탈 → Detach"
        );
        // 다시 등장하면 재부착(이탈로 스냅샷에서 지워졌으므로).
        sink.agent_list_updated(vec![flush_info(id, "a", 0, true, CoreStatus::Running)]);
        assert!(drain_msgs(&mut rx).contains(&FlushMsg::Attach { id, epoch: 0 }));
    }

    #[test]
    fn flush_sink_detaches_when_agent_becomes_terminal() {
        // terminal 상태(Killed)로 바뀌면 도달 후보에서 빠지므로 이탈과 동일 처리(Detach).
        let (sink, mut rx) = flush_sink();
        let id = AgentId::new_v4();
        sink.agent_list_updated(vec![flush_info(id, "a", 0, true, CoreStatus::Running)]);
        let _ = drain_msgs(&mut rx);
        sink.agent_list_updated(vec![flush_info(id, "a", 0, true, CoreStatus::Killed)]);
        assert_eq!(drain_msgs(&mut rx), vec![FlushMsg::Detach { id }]);
    }

    #[test]
    fn flush_sink_mixed_diff_enqueues_attach_detach_then_appear_in_order() {
        // ★fix 7 순서 계약★: 한 업데이트 안에서 죽은 에이전트와 새 에이전트가 섞여도 enqueue 순서는
        //   **Attach → Detach → Appear** 다(스냅샷 갱신과 같은 락 구간에서 send 하므로 순서가 갈리지 않는다).
        //   Appear 가 Attach 뒤라는 게 load-bearing(등장 flush 주입 전에 tap 이 붙어야 첫 턴을 관측).
        let (sink, mut rx) = flush_sink();
        let dying = AgentId::new_v4();
        let fresh = AgentId::new_v4();
        // 1) dying 만 있는 상태로 스냅샷 세팅.
        sink.agent_list_updated(vec![flush_info(
            dying,
            "dying",
            0,
            true,
            CoreStatus::Running,
        )]);
        let _ = drain_msgs(&mut rx);
        // 2) 한 업데이트에서 dying 이탈 + fresh 등장.
        sink.agent_list_updated(vec![flush_info(
            fresh,
            "fresh",
            0,
            true,
            CoreStatus::Running,
        )]);
        assert_eq!(
            drain_msgs(&mut rx),
            vec![
                FlushMsg::Attach {
                    id: fresh,
                    epoch: 0
                },
                FlushMsg::Detach { id: dying },
                FlushMsg::Appear {
                    name: "fresh".to_string(),
                    id: fresh
                },
            ],
            "Attach → Detach → Appear 순서(단일 락 구간에서 enqueue)"
        );
    }

    #[test]
    fn roster_diff_forget_attached_reopens_attach_on_next_update() {
        // ★fix 8a★: attach 실패 피드백은 id 축 스냅샷을 지워 **다음 로스터 업데이트가 재시도**하게 만든다.
        //   피드백이 없으면 로스터가 그대로인 동안 Attach 가 다시 나오지 않아 그 에이전트는 영구 tap 없음.
        let (sink, mut rx, diff) = flush_sink_with_diff();
        let id = AgentId::new_v4();
        sink.agent_list_updated(vec![flush_info(id, "a", 0, true, CoreStatus::Running)]);
        let _ = drain_msgs(&mut rx);
        // 같은 로스터 재-push 는 아무것도 내지 않는다(스냅샷 동일).
        sink.agent_list_updated(vec![flush_info(id, "a", 0, true, CoreStatus::Running)]);
        assert!(drain_msgs(&mut rx).is_empty());
        // worker 가 부착 실패를 피드백 → 스냅샷 무효화.
        diff.forget_attached(id);
        sink.agent_list_updated(vec![flush_info(id, "a", 0, true, CoreStatus::Running)]);
        assert_eq!(
            drain_msgs(&mut rx),
            vec![FlushMsg::Attach { id, epoch: 0 }],
            "피드백 후 같은 로스터에서도 Attach 재발행(Appear 는 이름 스냅샷이 그대로라 안 나온다)"
        );
    }

    #[test]
    fn idle_coalescer_folds_pending_notifications_until_taken() {
        // ★fix 10★: 같은 id 의 미처리 Idle 은 하나로 접힌다(MessageDone 폭풍의 채널 압력 상한). 소비자가
        //   집어들면(taken) 다시 열려 이후 통지가 큐에 들어간다 — 그래서 lost wakeup 이 없다.
        use crate::messaging::busy::IdleNotifier;
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
        // 서비스 도어벨(FlushTrigger)과 tap 의 턴 종료 통지는 **같은 메시지**로 나간다 — 결과가 같기
        //   때문이다("그 id 의 파킹 큐를 flush"). 그래서 coalescing 도 함께 받는다.
        use crate::messaging::service::FlushTrigger;
        let (tx, mut rx) = mpsc::unbounded_channel::<FlushMsg>();
        let coalescer = Arc::new(IdleCoalescer::new());
        let notifier = ChannelIdleNotifier::new(tx, coalescer.clone());
        let id = AgentId::new_v4();
        notifier.request_flush(id);
        notifier.request_flush(id);
        assert_eq!(drain_msgs(&mut rx), vec![FlushMsg::Idle { id }]);
    }

    // ── 9b. flush worker: attach 패닉 격리(round-3 finding 6) + 2-레인 소유/종료(finding 1) ──────────
    use crate::messaging::busy::{BusyTracker, SubscribeError, TapHost};
    use engram_dashboard_core::agent::types::OutputSink as CoreOutputSink;

    /// subscribe 마다 **패닉**하는 TapHost — core `subscribe_from` 이 sink 를 push 한 **뒤** replay 락
    ///   `expect` 에서 패닉하는 실제 형태를 모사한다(그 상태에선 sink 가 이미 등록돼 있고 subscribers 락이
    ///   poison 이라 재시도가 두 번째 sink 를 얹는 꼴이 된다 — 그래서 재시도 금지).
    struct PanickingTapHost {
        calls: Arc<AtomicU64>,
    }
    impl TapHost for PanickingTapHost {
        fn subscribe_output(
            &self,
            _id: AgentId,
            _expect_epoch: u32,
            _sink: Arc<dyn CoreOutputSink>,
        ) -> Result<(), SubscribeError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            panic!("fake: subscribe panicked (의도된 패닉 — 재시도 금지 검증)");
        }
        fn current_epoch(&self, _id: AgentId) -> Option<u32> {
            Some(0) // 사전 검증 통과(패닉 지점까지 도달시킨다).
        }
    }

    /// 매번 `Err(Failed)` 를 내는 TapHost — **정상 실패**는 유계 1회 재시도가 유지돼야 한다(대조군).
    struct FailingTapHost {
        calls: Arc<AtomicU64>,
    }
    impl TapHost for FailingTapHost {
        fn subscribe_output(
            &self,
            _id: AgentId,
            _expect_epoch: u32,
            _sink: Arc<dyn CoreOutputSink>,
        ) -> Result<(), SubscribeError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(SubscribeError::Failed("fake: gone".to_string()))
        }
        fn current_epoch(&self, _id: AgentId) -> Option<u32> {
            Some(0)
        }
    }

    /// 통지를 버리는 IdleNotifier(이 테스트들은 부착 정책만 본다).
    struct SinkNotifier;
    impl crate::messaging::busy::IdleNotifier for SinkNotifier {
        fn notify_idle(&self, _id: AgentId) {}
    }

    fn attach_wiring(host: Arc<dyn TapHost>) -> (FlushWiring, Arc<RosterDiff>) {
        let diff = Arc::new(RosterDiff::new());
        let wiring = FlushWiring {
            messaging: Arc::new(crate::control::mcp_server::MessagingSlot::new()),
            busy: Arc::new(BusyTracker::new(host, Arc::new(SinkNotifier))),
            diff: diff.clone(),
            idle: Arc::new(IdleCoalescer::new()),
        };
        (wiring, diff)
    }

    #[tokio::test]
    async fn attach_panic_is_not_retried() {
        // ★round-3 finding 6 회귀★: 패닉은 "sink 가 이미 등록됐고 락이 poison" 일 수 있는 상태다 — 재시도
        //   하면 두 번째 sink 를 얹어(통지 중복) 상황을 악화시킨다. 정확히 1회만 시도하고 물러난다.
        let calls = Arc::new(AtomicU64::new(0));
        let (wiring, diff) = attach_wiring(Arc::new(PanickingTapHost {
            calls: calls.clone(),
        }));
        let id = AgentId::new_v4();
        handle_attach(&wiring, id, 0).await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "패닉 후 재시도 금지 — subscribe 시도는 정확히 1회"
        );
        // 스냅샷은 무효화돼 다음 로스터 업데이트가 판단할 수 있어야 한다(조용한 기능 상실 방지).
        diff.forget_attached(id); // idempotent 확인(패닉 경로가 이미 지웠다 — 두 번 지워도 무해).
    }

    #[tokio::test]
    async fn attach_normal_failure_still_retries_once() {
        // 대조군: **정상 실패**(subscribe Err)는 유계 1회 재시도를 유지한다(fix 8a) — 패닉만 예외다.
        let calls = Arc::new(AtomicU64::new(0));
        let (wiring, _diff) = attach_wiring(Arc::new(FailingTapHost {
            calls: calls.clone(),
        }));
        handle_attach(&wiring, AgentId::new_v4(), 0).await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "정상 실패는 즉시 1회 재시도(총 2회) — 그 뒤는 로스터 diff 에 맡긴다"
        );
    }

    #[tokio::test]
    async fn flush_worker_handles_shutdown_stops_both_lanes() {
        // ★round-3 finding 1 회귀★: 배달 레인은 **호출자 소유**여야 한다 — 옛 구현은 worker future 안에서
        //   spawn 해 종료 시 detach 됐고(abort 아님), 5s belt 는 blocking 없는 main lane 만 감시했다.
        //   `shutdown()` 이 두 레인을 모두 끝내는지(= 핸들을 들고 있는지) 관측한다.
        let (tx, rx) = mpsc::unbounded_channel::<FlushMsg>();
        let (wiring, _diff) = attach_wiring(Arc::new(FailingTapHost {
            calls: Arc::new(AtomicU64::new(0)),
        }));
        let handles = spawn_flush_worker(rx, wiring);
        // 배달 작업 1건을 넣어 레인이 실제로 돌게 한다(messaging slot 미주입이라 즉시 skip = 결정적).
        tx.send(FlushMsg::Idle {
            id: AgentId::new_v4(),
        })
        .expect("main lane 수신");
        // shutdown 은 belt(5s) 안에 끝나야 한다 — 레인을 detach 하면 이 단언은 여전히 통과하지만,
        //   그 경우 레인 task 가 살아남는다. 그래서 아래에서 "채널이 닫혔음" 으로 종료를 교차 확인한다.
        tokio::time::timeout(Duration::from_secs(6), handles.shutdown())
            .await
            .expect("shutdown 이 belt 안에 반환해야");
        // main lane 이 죽었으므로 그 수신단은 닫혔다(송신 실패 = 소비자 종료의 관측 가능한 증거).
        assert!(
            tx.send(FlushMsg::Idle {
                id: AgentId::new_v4()
            })
            .is_err(),
            "shutdown 후 main lane 은 더 이상 수신하지 않는다"
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
}
