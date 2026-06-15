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

use engram_dashboard_core::pty::manager::{default_shell, AgentManager};
use engram_dashboard_core::pty::profile::RestoreReport as CoreRestoreReport;
use engram_dashboard_core::pty::profile::SpawnMode;
use engram_dashboard_core::pty::types::{
    AgentId, AgentInfo as CoreAgentInfo, AgentStatus as CoreStatus, OutputFrame, OutputSink,
    ReplayKind, SinkError, SinkId, StatusSink, SubscribeOutcome,
};

use engram_dashboard_protocol::{
    encode_terminal_frame, AgentCommand, AgentEvent, AgentInfo as WireAgentInfo, RestoreReport,
    SubscribeAction, PROTOCOL_VERSION,
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
    fn broadcast_text(&self, text: String) {
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

// ── 멀티뷰어 협상 상태(MultiViewState) ───────────────────────────────────────────────
//
// 데몬은 한 agent 를 여러 연결(메인창/팝업/모바일)이 동시 attach 하는 것을 전제한다. 그래서 두
// 정책을 데몬측에 둔다(코어 무변경 — 코어는 최종 크기·통과 여부만 받는다):
//  - resize 협상(tmux smallest): 각 viewport 가 자기 크기를 등록하면, agent 의 모든 viewport 중
//    가장 작은(min cols, min rows) 크기로 PTY 를 맞춘다(작은 화면이 안 깨짐).
//  - 입력 lease(Zellij 명시 lease): 한 agent 의 입력 권한을 한 연결만 쥘 수 있다(인터리브 방지).
//
// ★동시성★: 여러 연결 task 가 동시 접근하므로 Arc<Mutex>. **lock 보유 중 manager.resize/await 호출
//   금지** — lock 을 잡고 짧게 협상값만 계산해 해제한 뒤 그 결과로 manager 를 부른다(코어 §10 락 순서).

/// agent 별 viewport 크기 맵 + agent 별 입력 lease 를 묶은 멀티뷰어 협상 상태.
#[derive(Clone, Default)]
pub struct MultiViewState {
    inner: Arc<Mutex<MultiViewInner>>,
}

#[derive(Default)]
struct MultiViewInner {
    /// agent_id → (viewport_id → (cols, rows)). 빈 맵이면 협상 대상 없음(직접 resize).
    viewports: HashMap<AgentId, HashMap<String, (u16, u16)>>,
    /// agent_id → 입력 lease 보유 conn(None = 비어 있음 → WriteStdin/Interrupt 자유 통과).
    leases: HashMap<AgentId, ConnId>,
}

/// 입력 lease 정책 판정 결과.
enum LeasePass {
    /// 통과 — lease 가 비었거나 이 conn 이 보유자.
    Allow,
    /// 거부 — 다른 conn 이 보유 중.
    Denied,
}

impl MultiViewState {
    pub fn new() -> Self {
        Self::default()
    }

    /// viewport 크기를 등록/갱신하고, 그 agent 의 협상된 smallest 크기를 반환한다.
    /// 반환 None = 등록된 viewport 가 없음(이론상 방금 넣었으므로 항상 Some). lock 은 이 안에서만 보유.
    fn set_viewport(
        &self,
        agent_id: AgentId,
        viewport_id: String,
        cols: u16,
        rows: u16,
    ) -> Option<(u16, u16)> {
        let mut g = self.inner.lock().expect("multiview poisoned");
        g.viewports
            .entry(agent_id)
            .or_default()
            .insert(viewport_id, (cols, rows));
        smallest(g.viewports.get(&agent_id))
    }

    /// 한 연결이 보유한 viewport 들을 제거하고, 영향받은 agent 별 재협상 결과를 반환한다.
    /// 반환: (agent_id, Some(min) = 남은 viewport 의 smallest / None = 이제 viewport 없음).
    /// cleanup 에서 호출 — 끊긴 연결의 크기를 빼고 남은 뷰어 기준으로 다시 키운다(tmux detach 동치).
    fn remove_conn_viewports(
        &self,
        owned: &[(AgentId, String)],
    ) -> Vec<(AgentId, Option<(u16, u16)>)> {
        let mut g = self.inner.lock().expect("multiview poisoned");
        // 같은 agent 가 여러 viewport 를 가질 수 있어 agent 단위로 1회만 재협상한다.
        let mut affected: Vec<AgentId> = Vec::new();
        for (agent_id, viewport_id) in owned {
            if let Some(m) = g.viewports.get_mut(agent_id) {
                m.remove(viewport_id);
                if m.is_empty() {
                    g.viewports.remove(agent_id);
                }
                if !affected.contains(agent_id) {
                    affected.push(*agent_id);
                }
            }
        }
        affected
            .into_iter()
            .map(|a| (a, smallest(g.viewports.get(&a))))
            .collect()
    }

    /// 입력 lease 획득 시도. Ok(true)=새로 획득(상태 변경), Ok(false)=이미 이 conn 보유(idempotent),
    /// Err=다른 conn 이 보유 중. lock 은 이 안에서만.
    fn acquire(&self, agent_id: AgentId, conn_id: ConnId) -> Result<bool, ()> {
        let mut g = self.inner.lock().expect("multiview poisoned");
        match g.leases.get(&agent_id) {
            None => {
                g.leases.insert(agent_id, conn_id);
                Ok(true)
            }
            Some(&holder) if holder == conn_id => Ok(false), // 재획득 idempotent
            Some(_) => Err(()),                              // 타 conn 보유
        }
    }

    /// 입력 lease 해제 시도. Ok(true)=해제됨(상태 변경), Ok(false)=원래 비어 있었음,
    /// Err=다른 conn 이 보유 중(보유자만 해제 가능).
    fn release(&self, agent_id: AgentId, conn_id: ConnId) -> Result<bool, ()> {
        let mut g = self.inner.lock().expect("multiview poisoned");
        match g.leases.get(&agent_id) {
            Some(&holder) if holder == conn_id => {
                g.leases.remove(&agent_id);
                Ok(true)
            }
            None => Ok(false),
            Some(_) => Err(()),
        }
    }

    /// WriteStdin/Interrupt 입력 권한 판정. lease 비었으면 Allow, 보유자면 Allow, 타 conn 이면 Denied.
    fn check_input(&self, agent_id: AgentId, conn_id: ConnId) -> LeasePass {
        let g = self.inner.lock().expect("multiview poisoned");
        match g.leases.get(&agent_id) {
            None => LeasePass::Allow,
            Some(&holder) if holder == conn_id => LeasePass::Allow,
            Some(_) => LeasePass::Denied, // 타 conn 이 lease 보유 중 → 거부
        }
    }

    /// 한 연결이 보유한 모든 agent lease 를 해제하고, 실제 해제된 agent 들을 반환한다(좀비 lock 방지).
    /// 반환된 agent 들은 이제 lease 가 비었으므로 InputLeaseChanged{held:false} 를 브로드캐스트할 대상.
    fn release_all_for_conn(&self, conn_id: ConnId) -> Vec<AgentId> {
        let mut g = self.inner.lock().expect("multiview poisoned");
        let freed: Vec<AgentId> = g
            .leases
            .iter()
            .filter(|(_, &h)| h == conn_id)
            .map(|(a, _)| *a)
            .collect();
        for a in &freed {
            g.leases.remove(a);
        }
        freed
    }
}

/// viewport 맵에서 smallest(min cols, min rows) 계산. tmux 기본 정책 — 가장 작은 뷰에 맞춰
/// 어느 뷰도 PTY 보다 작아 깨지지 않게 한다. 빈/없는 맵이면 None.
fn smallest(views: Option<&HashMap<String, (u16, u16)>>) -> Option<(u16, u16)> {
    let m = views?;
    let mut it = m.values();
    let &(mut c, mut r) = it.next()?;
    for &(vc, vr) in it {
        c = c.min(vc);
        r = r.min(vr);
    }
    Some((c, r))
}

// ── 타입 변환(core → wire) ─────────────────────────────────────────────────────
//
// ★명시 매핑(runtime reflection 폐기)★: 옛 구현은 serde_json::to_value→from_value 로 core↔wire
// 를 변환했다. 이러면 한쪽 필드/태그가 어긋나도 컴파일은 통과하고 런타임에 silent drop(None) 됐다.
// 이제는 필드를 하나하나 명시 매핑한다 — core 에 필드가 추가/개명되면 **컴파일 에러**가 나게.
//
// 변환은 데몬 crate 에 둔다(core 는 protocol 무의존 유지 — §1 불변). orphan rule 때문에 외부 두
// 타입 사이 `impl From` 은 불가하나, 데몬이 양쪽을 다 의존하므로 자유 함수로 직접 필드 접근한다.

use engram_dashboard_core::pty::profile::{
    AgentCommand as CoreSpawnCommand, AgentProfile as CoreProfile,
    RestartPolicy as CoreRestartPolicy, RestoreOutcome as CoreRestoreOutcome,
};
use engram_dashboard_core::pty::types::{Capabilities as CoreCaps, OutputChunk as CoreOutputChunk};
use engram_dashboard_protocol::{
    AgentProfile as WireProfile, AgentSpawnCommand as WireSpawnCommand, Capabilities as WireCaps,
    ControlCaps as WireControlCaps, InputCaps as WireInputCaps, ModelCaps as WireModelCaps,
    OutputCaps as WireOutputCaps, RestartPolicy as WireRestartPolicy,
    RestoreOutcome as WireRestoreOutcome, SessionCaps as WireSessionCaps,
    SnapshotChunk as WireSnapshotChunk,
};

/// core Capabilities → wire. 5개 sub-cap 의 모든 bool 필드를 명시 매핑.
fn caps_to_wire(c: &CoreCaps) -> WireCaps {
    WireCaps {
        input: WireInputCaps {
            raw: c.input.raw,
            message: c.input.message,
            attachment: c.input.attachment,
        },
        output: WireOutputCaps {
            terminal_bytes: c.output.terminal_bytes,
            markdown: c.output.markdown,
            tool_events: c.output.tool_events,
            usage: c.output.usage,
        },
        control: WireControlCaps {
            resize: c.control.resize,
            interrupt: c.control.interrupt,
            cancel: c.control.cancel,
            graceful_shutdown: c.control.graceful_shutdown,
        },
        session: WireSessionCaps {
            resume: c.session.resume,
            snapshot: c.session.snapshot,
            cwd_env: c.session.cwd_env,
        },
        model: WireModelCaps {
            select: c.model.select,
            temperature: c.model.temperature,
            max_tokens: c.model.max_tokens,
        },
    }
}

/// core AgentStatus → wire. 5개 variant 전수 명시 — variant 추가 시 컴파일 에러로 강제.
fn status_to_wire(status: &CoreStatus) -> engram_dashboard_protocol::AgentStatus {
    use engram_dashboard_protocol::AgentStatus as W;
    match status {
        CoreStatus::Running => W::Running,
        CoreStatus::Exiting => W::Exiting,
        CoreStatus::Exited { code } => W::Exited { code: *code },
        CoreStatus::Failed { message } => W::Failed {
            message: message.clone(),
        },
        CoreStatus::Killed => W::Killed,
    }
}

/// core AgentInfo → wire. 모든 필드 명시(누락 시 컴파일 에러).
fn agent_info_to_wire(a: &CoreAgentInfo) -> WireAgentInfo {
    WireAgentInfo {
        id: a.id,
        name: a.name.clone(),
        cwd: a.cwd.clone(),
        status: status_to_wire(&a.status),
        cols: a.cols,
        rows: a.rows,
        epoch: a.epoch,
        capabilities: caps_to_wire(&a.capabilities),
    }
}

/// core RestoreOutcome → wire. 전 variant 명시.
/// ★Uuid→String★: core FreshFallback{old_sid: Option<Uuid>, new_sid: Uuid} 를 wire 의
/// {Option<String>, String} 으로 `to_string()` 변환(옛 reflection 은 JSON string 우연 호환에
/// 의존했음). 명시 변환으로 이 변환을 코드로 못박는다.
fn restore_outcome_to_wire(outcome: &CoreRestoreOutcome) -> WireRestoreOutcome {
    match outcome {
        CoreRestoreOutcome::Resumed => WireRestoreOutcome::Resumed,
        CoreRestoreOutcome::Started => WireRestoreOutcome::Started,
        CoreRestoreOutcome::FreshFallback {
            old_sid,
            new_sid,
            reason,
        } => WireRestoreOutcome::FreshFallback {
            old_sid: old_sid.map(|u| u.to_string()),
            new_sid: new_sid.to_string(),
            reason: reason.clone(),
        },
        CoreRestoreOutcome::Blocked { reason } => WireRestoreOutcome::Blocked {
            reason: reason.clone(),
        },
        CoreRestoreOutcome::Failed { reason } => WireRestoreOutcome::Failed {
            reason: reason.clone(),
        },
    }
}

fn core_agents_to_wire(agents: Vec<CoreAgentInfo>) -> Vec<WireAgentInfo> {
    agents.iter().map(agent_info_to_wire).collect()
}

/// core profile::AgentCommand → wire AgentSpawnCommand. 2 variant 전수 명시.
fn spawn_command_to_wire(cmd: &CoreSpawnCommand) -> WireSpawnCommand {
    match cmd {
        CoreSpawnCommand::Claude { extra_args } => WireSpawnCommand::Claude {
            extra_args: extra_args.clone(),
        },
        CoreSpawnCommand::Shell { program, args } => WireSpawnCommand::Shell {
            program: program.clone(),
            args: args.clone(),
        },
    }
}

/// core RestartPolicy → wire. 3 variant 전수 명시(추가 시 컴파일 에러).
fn restart_policy_to_wire(p: CoreRestartPolicy) -> WireRestartPolicy {
    match p {
        CoreRestartPolicy::Never => WireRestartPolicy::Never,
        CoreRestartPolicy::OnCrash => WireRestartPolicy::OnCrash,
        CoreRestartPolicy::Always => WireRestartPolicy::Always,
    }
}

/// core AgentProfile → wire. 모든 필드 명시(누락/개명 시 컴파일 에러).
/// ★Uuid→String / PathBuf→String★: claude_session_id·old_session_ids 는 Uuid, cwd 는 PathBuf 라
/// JSON 표현(문자열)으로 명시 변환한다(reflection 왕복 금지 — agent_info_to_wire 와 동일 원칙).
fn profile_to_wire(p: &CoreProfile) -> WireProfile {
    WireProfile {
        id: p.id,
        name: p.name.clone(),
        command: spawn_command_to_wire(&p.command),
        cwd: p.cwd.to_string_lossy().into_owned(),
        env: p.env.clone(),
        claude_session_id: p.claude_session_id.map(|u| u.to_string()),
        old_session_ids: p.old_session_ids.iter().map(|u| u.to_string()).collect(),
        epoch: p.epoch,
        auto_restore: p.auto_restore,
        restart_policy: restart_policy_to_wire(p.restart_policy),
        created_at: p.created_at,
        last_active: p.last_active,
        last_restore: p.last_restore,
    }
}

fn core_profiles_to_wire(profiles: Vec<CoreProfile>) -> Vec<WireProfile> {
    profiles.iter().map(profile_to_wire).collect()
}

/// core OutputChunk → wire SnapshotChunk. {seq, data} 명시 매핑.
fn snapshot_chunk_to_wire(c: &CoreOutputChunk) -> WireSnapshotChunk {
    WireSnapshotChunk {
        seq: c.seq,
        data: c.data.clone(),
    }
}

/// core RestoreReport → wire. 모든 필드 명시(누락 시 컴파일 에러).
fn core_report_to_wire(report: CoreRestoreReport) -> RestoreReport {
    RestoreReport {
        agent_id: report.agent_id,
        epoch: report.epoch,
        outcome: restore_outcome_to_wire(&report.outcome),
    }
}

/// core AgentStatus → wire. StatusChanged 직렬화에 사용.
fn core_status_to_wire(status: CoreStatus) -> engram_dashboard_protocol::AgentStatus {
    status_to_wire(&status)
}

/// AgentEvent 를 JSON 문자열로 직렬화(control 전송용). 실패는 거의 불가능하나 로그 후 None.
fn event_json(ev: &AgentEvent) -> Option<String> {
    match serde_json::to_string(ev) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::error!("AgentEvent 직렬화 실패: {e}");
            None
        }
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
    fn new(conn_tx: mpsc::Sender<WsOutbound>, close_signal: Arc<Notify>) -> Self {
        Self {
            conn_tx,
            close_signal,
            replay_dropped: Arc::new(AtomicBool::new(false)),
            sink_id: uuid::Uuid::new_v4(),
        }
    }

    /// replay 구간 동안 frame 이 drop 됐는지 사후 검사용 핸들(handle_subscribe 가 공유 보관).
    fn replay_dropped_flag(&self) -> Arc<AtomicBool> {
        self.replay_dropped.clone()
    }
}

impl OutputSink for WsOutputSink {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        let buf = encode_terminal_frame(frame.agent_id, frame.epoch, frame.seq, frame.data);
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

    // 4) 연결 직후 Hello + 현재 목록 push(단일 writer 큐 경유 — 이후 모든 출력과 FIFO 정렬).
    if let Some(text) = event_json(&AgentEvent::Hello {
        protocol_version: PROTOCOL_VERSION,
        daemon_version: env!("CARGO_PKG_VERSION").into(),
        capabilities: None,
    }) {
        let _ = conn_tx.send(WsOutbound::Text(text)).await;
    }
    if let Some(text) = event_json(&AgentEvent::AgentListUpdated {
        agents: core_agents_to_wire(manager.list_agents()),
    }) {
        let _ = conn_tx.send(WsOutbound::Text(text)).await;
    }

    // 5) 이 연결이 등록한 (agent_id → sink_id) 추적 — cleanup 에서 누수 없이 unsubscribe 하기 위함.
    //    read_task 와 cleanup 이 공유하므로 Arc<Mutex<..>>.
    let subs: Arc<Mutex<HashMap<AgentId, SinkId>>> = Arc::new(Mutex::new(HashMap::new()));

    // 5b) 이 연결이 등록한 (agent_id, viewport_id) 들 — cleanup 에서 viewport 협상 맵을 정리하기 위함.
    //     한 연결이 여러 viewport 를 가질 수 있어(여러 agent·여러 뷰) Vec 로 추적한다.
    let owned_viewports: Arc<Mutex<Vec<(AgentId, String)>>> = Arc::new(Mutex::new(Vec::new()));

    // ── keepalive 공유 시계(A) ──────────────────────────────────────────────────────
    // base = 연결 시작 시각(tokio Instant). last_recv = base 기준 경과 ms(AtomicU64).
    // read_task 가 클라로부터 무언가(Pong 포함) 받을 때마다 갱신하고, write_task 의 ping arm 이
    // base.elapsed() - last_recv 로 idle 경과를 계산해 idle_timeout 초과 시 close_signal 발동.
    let keepalive_base = tokio::time::Instant::now();
    let last_recv = Arc::new(AtomicU64::new(0));

    // read_task: stream_half 에서 명령을 읽어 dispatch. conn_tx 로 응답을 큐잉.
    //   close_signal 은 handle_subscribe 가 만드는 WsOutputSink 에 주입(full 시 깨우기용).
    let mut read_handle = tokio::spawn(read_task(
        stream_half,
        conn_tx.clone(),
        manager.clone(),
        registry.clone(),
        multiview.clone(),
        subs.clone(),
        owned_viewports.clone(),
        shutdown_tx,
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
        let guard = subs.lock().expect("subs poisoned");
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
        let g = owned_viewports.lock().expect("owned_viewports poisoned");
        g.clone()
    };
    if !owned.is_empty() {
        for (agent_id, negotiated) in multiview.remove_conn_viewports(&owned) {
            if let Some((cols, rows)) = negotiated {
                // 남은 뷰어가 있으면 그 smallest 로 복귀. 없으면(None) 그대로 둔다(마지막 크기 유지).
                let _ = manager.resize(agent_id, cols, rows);
            }
        }
    }
    // (b) 입력 lease 자동 해제: 보유자가 끊기면 다른 뷰어가 영영 막히면 안 된다(좀비 lock 방지).
    //     해제된 agent 는 이제 lease 가 비었으니 InputLeaseChanged{held:false} 를 전 연결에 통보.
    for agent_id in multiview.release_all_for_conn(conn_id) {
        broadcast_lease_changed(&registry, agent_id, false);
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

/// stream_half 에서 명령 frame 을 읽어 dispatch. 응답은 conn_tx 로 큐잉(직접 write 안 함).
#[allow(clippy::too_many_arguments)]
async fn read_task(
    mut stream_half: StreamHalf,
    conn_tx: mpsc::Sender<WsOutbound>,
    manager: Arc<AgentManager>,
    registry: ConnRegistry,
    multiview: MultiViewState,
    subs: Arc<Mutex<HashMap<AgentId, SinkId>>>,
    owned_viewports: Arc<Mutex<Vec<(AgentId, String)>>>,
    shutdown_tx: watch::Sender<bool>,
    conn_id: ConnId,
    close_signal: Arc<Notify>,
    keepalive_base: tokio::time::Instant,
    last_recv: Arc<AtomicU64>,
) {
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
                        if dispatch(
                            cmd,
                            &conn_tx,
                            &manager,
                            &registry,
                            &multiview,
                            &subs,
                            &owned_viewports,
                            conn_id,
                            &shutdown_tx,
                            &close_signal,
                        )
                        .await
                        {
                            // dispatch 가 연결 종료를 요청(StopDaemon 등) — 루프 탈출.
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(conn = conn_id, "명령 파싱 실패: {e}");
                        send_error(&conn_tx, None, format!("invalid command: {e}")).await;
                    }
                }
            }
            Message::Binary(_) => {
                // 클라→데몬 binary 는 프로토콜에 없음 — 오류로 보고 종료.
                tracing::warn!(conn = conn_id, "예상치 못한 binary frame — close");
                send_error(&conn_tx, None, "unexpected binary frame".into()).await;
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

/// 단일 명령 dispatch. 반환 true = 연결 종료 요청(StopDaemon).
/// side-effect 명령은 request_id 있으면 Ack/Error.
#[allow(clippy::too_many_arguments)]
async fn dispatch(
    cmd: AgentCommand,
    conn_tx: &mpsc::Sender<WsOutbound>,
    manager: &Arc<AgentManager>,
    registry: &ConnRegistry,
    multiview: &MultiViewState,
    subs: &Arc<Mutex<HashMap<AgentId, SinkId>>>,
    owned_viewports: &Arc<Mutex<Vec<(AgentId, String)>>>,
    conn_id: ConnId,
    shutdown_tx: &watch::Sender<bool>,
    close_signal: &Arc<Notify>,
) -> bool {
    use engram_dashboard_protocol::RequestId;

    /// side-effect 결과를 Ack/Error 로 변환해 큐잉.
    async fn reply(
        conn_tx: &mpsc::Sender<WsOutbound>,
        request_id: RequestId,
        result: Result<(), String>,
    ) {
        let ev = match result {
            Ok(()) => AgentEvent::Ack { request_id },
            Err(message) => AgentEvent::Error {
                request_id: Some(request_id),
                message,
            },
        };
        if let Some(text) = event_json(&ev) {
            let _ = conn_tx.send(WsOutbound::Text(text)).await;
        }
    }

    match cmd {
        // 2번째 Auth 는 무시(Error 만 — 이미 인증된 연결).
        AgentCommand::Auth { .. } => {
            send_error(conn_tx, None, "already authenticated".into()).await;
        }

        AgentCommand::Spawn {
            profile_id,
            request_id,
        } => {
            // 프로필 기반 spawn(profile.rs spawn_profile 미러, resume=false=Fresh).
            let result = match manager.profiles().get(profile_id) {
                Some(profile) => manager
                    .spawn_agent(&profile, SpawnMode::Fresh)
                    .map(|_| ())
                    .map_err(|e| e.to_string()),
                None => Err(format!("profile not found: {profile_id}")),
            };
            reply(conn_tx, request_id, result).await;
        }

        AgentCommand::Kill {
            agent_id,
            request_id,
        } => {
            let result = manager.kill_agent(agent_id).map_err(|e| e.to_string());
            reply(conn_tx, request_id, result).await;
        }

        AgentCommand::Interrupt {
            agent_id,
            request_id,
        } => {
            // Interrupt(Ctrl+C)도 입력 평면이라 lease 게이트를 거친다(WriteStdin 과 동일 정책).
            let result = match multiview.check_input(agent_id, conn_id) {
                LeasePass::Allow => manager.interrupt(agent_id).map_err(|e| e.to_string()),
                LeasePass::Denied => {
                    Err("input locked by another viewer; acquire first".to_string())
                }
            };
            reply(conn_tx, request_id, result).await;
        }

        AgentCommand::WriteStdin {
            agent_id,
            data,
            request_id,
        } => {
            // ★입력 lease 게이트★: lease 가 비었거나(단일 뷰어 흔한 경우 마찰 0) 이 conn 이 보유자면
            //   통과, 타 conn 이 보유 중이면 거부(stdin 인터리브 방지). lock 은 check_input 안에서만.
            let result = match multiview.check_input(agent_id, conn_id) {
                LeasePass::Allow => manager
                    .write_stdin(agent_id, &data)
                    .map_err(|e| e.to_string()),
                LeasePass::Denied => {
                    Err("input locked by another viewer; acquire first".to_string())
                }
            };
            reply(conn_tx, request_id, result).await;
        }

        AgentCommand::Resize {
            agent_id,
            cols,
            rows,
            viewport_id,
        } => {
            // Resize 는 request_id 가 없는 명령(messages.rs) — Ack 없이 best-effort, 실패만 Error.
            // ★멀티뷰어 협상(tmux smallest)★: viewport_id 가 있으면 그 뷰의 크기를 협상 맵에 기록하고
            //   그 agent 의 모든 viewport 중 smallest 로 PTY 를 맞춘다(작은 화면이 안 깨짐). viewport_id
            //   가 없으면(v1 프론트 기본) 협상을 우회해 그 크기로 직접 resize(하위호환).
            //   ★lock 순서★: set_viewport 가 multiview lock 안에서 협상값만 계산해 반환한 뒤 lock 을 푼
            //   상태에서 manager.resize 를 부른다(lock 보유 중 코어 호출 금지).
            let target = match viewport_id {
                Some(v) => {
                    // 이 연결이 등록한 viewport 추적(cleanup 재협상용). 중복 등록은 무시.
                    {
                        let mut owned = owned_viewports.lock().expect("owned_viewports poisoned");
                        if !owned.iter().any(|(a, vid)| *a == agent_id && vid == &v) {
                            owned.push((agent_id, v.clone()));
                        }
                    }
                    multiview
                        .set_viewport(agent_id, v, cols, rows)
                        .unwrap_or((cols, rows))
                }
                // 단일 뷰어 — 협상 우회.
                None => (cols, rows),
            };
            if let Err(e) = manager.resize(agent_id, target.0, target.1) {
                send_error(conn_tx, None, format!("resize failed: {e}")).await;
            }
        }

        AgentCommand::Subscribe {
            agent_id,
            epoch,
            after_seq,
        } => {
            // Step 4c: epoch/after_seq 를 코어 subscribe_from 으로 전달 → 무손실 resume(tail 만)
            // 또는 truncated/full replay 분기.
            handle_subscribe(
                agent_id,
                epoch,
                after_seq,
                conn_tx,
                manager,
                subs,
                close_signal,
            )
            .await;
        }

        AgentCommand::Unsubscribe { agent_id } => {
            // 이 연결의 그 agent sink_id 로 unsubscribe + 기록 제거.
            let sink_id = subs.lock().expect("subs poisoned").remove(&agent_id);
            if let Some(sid) = sink_id {
                let _ = manager.unsubscribe(agent_id, sid);
            }
        }

        AgentCommand::AcquireInput {
            agent_id,
            request_id,
        } => {
            // lease 비었으면 획득(Ack) + InputLeaseChanged{held:true} 브로드캐스트. 같은 conn 재획득은
            // 멱등(Ack, 상태 변경 없음 → 브로드캐스트 생략). 타 conn 보유면 Error.
            match multiview.acquire(agent_id, conn_id) {
                Ok(true) => {
                    broadcast_lease_changed(registry, agent_id, true);
                    reply(conn_tx, request_id, Ok(())).await;
                }
                Ok(false) => reply(conn_tx, request_id, Ok(())).await, // idempotent
                Err(()) => {
                    reply(
                        conn_tx,
                        request_id,
                        Err("input held by another viewer".to_string()),
                    )
                    .await
                }
            }
        }

        AgentCommand::ReleaseInput {
            agent_id,
            request_id,
        } => {
            // 보유자만 해제 가능. 해제 시 InputLeaseChanged{held:false} 브로드캐스트. 원래 비어 있었으면
            // 멱등(Ack). 타 conn 이 보유 중이면 Error(남의 lease 를 뺏지 못함).
            match multiview.release(agent_id, conn_id) {
                Ok(true) => {
                    broadcast_lease_changed(registry, agent_id, false);
                    reply(conn_tx, request_id, Ok(())).await;
                }
                Ok(false) => reply(conn_tx, request_id, Ok(())).await, // 원래 비어 있음
                Err(()) => {
                    reply(
                        conn_tx,
                        request_id,
                        Err("input lease held by another viewer".to_string()),
                    )
                    .await
                }
            }
        }

        AgentCommand::ListAgents => {
            if let Some(text) = event_json(&AgentEvent::AgentListUpdated {
                agents: core_agents_to_wire(manager.list_agents()),
            }) {
                let _ = conn_tx.send(WsOutbound::Text(text)).await;
            }
        }

        AgentCommand::StopDaemon {
            force,
            kill_agents,
            request_id,
        } => {
            // ── M4: force 정책 ──────────────────────────────────────────────────
            // force=false 인데 **실활성** 에이전트가 남아 있으면 거부(종료하지 않음). 실수로 데몬을
            // 내려 살아있는 PTY 세션을 모두 죽이는 사고를 막는다. 실활성 0이거나 force=true 면 진행.
            // ★실활성만 카운트★: 이미 죽은(Exited/Killed/Failed)·종료중(Exiting) 세션은 제외한다 —
            //   이들 때문에 거부하면 살릴 게 없는데도 데몬을 못 내리는 오작동이 된다.
            let active_count = manager
                .list_agents()
                .iter()
                .filter(|a| {
                    matches!(
                        a.status,
                        CoreStatus::Running // 비-terminal·비-Exiting 만 실활성
                    )
                })
                .count();
            if !force && active_count > 0 {
                send_error(
                    conn_tx,
                    Some(request_id),
                    format!(
                        "active agents present ({active_count}); use force=true to stop the daemon"
                    ),
                )
                .await;
                return false; // 거부 — 연결 유지, main 종료 안 함.
            }

            // ★kill_agents 는 v1 에서 무시(always-kill)★: 데몬은 자식 PTY 를 자기
            //   KILL_ON_JOB_CLOSE Job Object 에 담는다. 따라서 데몬이 종료되면 Job 핸들이
            //   닫히며 자식이 **무조건** 함께 죽는다 — detach(데몬만 내리고 자식 유지)는 현
            //   Job 모델에선 불가능하다. kill_agents 플래그는 미래에 detach 를 지원하게 될
            //   여지로 protocol 에 남겨두되, v1 동작은 값과 무관하게 항상 자식을 정리한다.
            let _ = kill_agents; // 의도적 무시(위 주석) — 미래 detach 지원 여지.
            let mgr = manager.clone();
            let _ = tokio::task::spawn_blocking(move || mgr.shutdown_all()).await;

            reply(conn_tx, request_id, Ok(())).await;
            // main 종료 트리거(watch). 수신측은 run() 의 select! 가 감지.
            let _ = shutdown_tx.send(true);
            return true;
        }

        // ── 프로필 CRUD + ad-hoc spawn(phase4 1단계) ───────────────────────────────
        // 각 arm 은 대응 Tauri command(EmbeddedClient)와 같은 동작을 해야 한다(인자/부작용 동일).
        AgentCommand::SpawnByCwd { cwd, request_id } => {
            // Tauri `spawn_agent(cwd)` 미러: 기본 셸 ad-hoc 프로필(auto_restore=false)을 Fresh spawn.
            // (영속 등록은 manager.spawn_agent 내부 upsert 가 처리 — Tauri 경로와 동일.)
            let profile = CoreProfile::new(
                cwd.clone(),
                CoreSpawnCommand::Shell {
                    program: default_shell().to_string(),
                    args: vec![],
                },
                std::path::PathBuf::from(&cwd),
                vec![],
                false,
            );
            // spawn 성공 시 AgentInfo 를 request_id 에 동봉(Spawned)해 requester 가 "내 것"을 식별.
            // agent_list_updated 는 StatusSink 가 이미 전 연결에 브로드캐스트(Spawn arm 과 동일).
            match manager.spawn_agent(&profile, SpawnMode::Fresh) {
                Ok(info) => {
                    if let Some(text) = event_json(&AgentEvent::Spawned {
                        request_id,
                        agent: agent_info_to_wire(&info),
                    }) {
                        let _ = conn_tx.send(WsOutbound::Text(text)).await;
                    }
                }
                Err(e) => reply(conn_tx, request_id, Err(e.to_string())).await,
            }
        }

        AgentCommand::ListProfiles => {
            // Tauri `list_profiles` 미러 — 읽기 전용 조회. 요청 연결에만 응답(ListAgents 와 동형).
            if let Some(text) = event_json(&AgentEvent::ProfileListUpdated {
                profiles: core_profiles_to_wire(manager.profiles().list()),
            }) {
                let _ = conn_tx.send(WsOutbound::Text(text)).await;
            }
        }

        AgentCommand::CreateProfile {
            name,
            cwd,
            extra_args,
            env,
            auto_restore,
            request_id,
        } => {
            // Tauri `create_claude_profile` 미러: claude 프로필 생성·upsert(스폰 안 함).
            let profile = CoreProfile::new(
                name,
                CoreSpawnCommand::Claude { extra_args },
                std::path::PathBuf::from(cwd),
                env,
                auto_restore,
            );
            // upsert 가 profile 을 move 하므로 wire 변환을 먼저 떠둔다.
            let wire = profile_to_wire(&profile);
            manager.profiles().upsert(profile);
            // requester 에겐 Created(생성된 프로필 동봉)로 응답 — Ack 는 보내지 않는다(중복 resolve 방지).
            if let Some(text) = event_json(&AgentEvent::Created {
                request_id,
                profile: wire,
            }) {
                let _ = conn_tx.send(WsOutbound::Text(text)).await;
            }
            // 생성은 공유 상태 변경 → 나머지 연결엔 갱신된 목록 broadcast.
            broadcast_profile_list(registry, manager);
        }

        AgentCommand::DeleteProfile {
            profile_id,
            request_id,
        } => {
            // Tauri `delete_profile` 미러: 등록 해제·persist(실행 중 세션은 별도 Kill).
            // remove 는 무조건 성공(없는 id 면 no-op) — Tauri 경로와 동일하게 Ack.
            manager.profiles().remove(profile_id);
            reply(conn_tx, request_id, Ok(())).await;
            broadcast_profile_list(registry, manager);
        }

        AgentCommand::SpawnProfile {
            profile_id,
            resume,
            request_id,
        } => {
            // Tauri `spawn_profile` 미러: 저장된 프로필을 Resume/Fresh 로 spawn. 없으면 Error.
            let mode = if resume {
                SpawnMode::Resume
            } else {
                SpawnMode::Fresh
            };
            // 성공 시 Spawned(AgentInfo 동봉)로 응답, 실패/없음은 Error.
            // agent_list_updated 는 StatusSink 가 브로드캐스트(Spawn arm 과 동일).
            match manager.profiles().get(profile_id) {
                Some(profile) => match manager.spawn_agent(&profile, mode) {
                    Ok(info) => {
                        if let Some(text) = event_json(&AgentEvent::Spawned {
                            request_id,
                            agent: agent_info_to_wire(&info),
                        }) {
                            let _ = conn_tx.send(WsOutbound::Text(text)).await;
                        }
                    }
                    Err(e) => reply(conn_tx, request_id, Err(e.to_string())).await,
                },
                None => {
                    reply(
                        conn_tx,
                        request_id,
                        Err(format!("profile not found: {profile_id}")),
                    )
                    .await
                }
            }
        }

        AgentCommand::SetProfileAutoRestore {
            profile_id,
            auto_restore,
            request_id,
        } => {
            // Tauri `set_profile_auto_restore` 미러: update_with 로 토글. 없으면 Error(Tauri 와 동일).
            let ok = manager
                .profiles()
                .update_with(profile_id, |p| p.auto_restore = auto_restore);
            if ok {
                reply(conn_tx, request_id, Ok(())).await;
                broadcast_profile_list(registry, manager);
            } else {
                reply(
                    conn_tx,
                    request_id,
                    Err(format!("profile not found: {profile_id}")),
                )
                .await;
            }
        }

        AgentCommand::GetSnapshot {
            agent_id,
            request_id,
        } => {
            // Tauri `get_agent_snapshot` 미러: 그 시점 replay buffer 스냅샷 1회 조회. 없으면 Error.
            match manager.get_snapshot(agent_id) {
                Ok(chunks) => {
                    if let Some(text) = event_json(&AgentEvent::Snapshot {
                        agent_id,
                        chunks: chunks.iter().map(snapshot_chunk_to_wire).collect(),
                    }) {
                        let _ = conn_tx.send(WsOutbound::Text(text)).await;
                    }
                    // Ack 로 요청 완료 확정(request_id echo).
                    reply(conn_tx, request_id, Ok(())).await;
                }
                Err(e) => reply(conn_tx, request_id, Err(e.to_string())).await,
            }
        }
    }
    false
}

/// 코어 ReplayKind → protocol SubscribeAction 매핑. SubscribeAck.action 구성에 사용한다.
/// (옛 predict_ack 의 별도 분기 예측을 제거 — 분기는 코어 subscribe_from 단일 스냅샷이 소유한다.)
fn kind_to_action(kind: ReplayKind) -> SubscribeAction {
    match kind {
        ReplayKind::FromOldest => SubscribeAction::Reset,
        ReplayKind::Truncated => SubscribeAction::TruncatedReplay,
        ReplayKind::Resumed => SubscribeAction::Resume,
    }
}

/// Subscribe 처리(Step 4c — afterSeq resume). **M-A(TOCTOU) 근본 해결판.**
///
/// ★TOCTOU 제거★: 옛 구현은 get_snapshot(스냅샷 A)으로 SubscribeAck 를 예측해 보낸 뒤,
/// subscribe_from 이 내부에서 다시 스냅샷 B 를 떠 replay 했다. A≠B(사이에 evict 가 끼면)면
/// Ack.replay_from/latest 가 실제 첫 전송 seq 와 어긋나 클라가 손실을 인지 못 했다. 이제는
/// SubscribeAck 의 모든 필드를 subscribe_from 의 **단일 스냅샷 outcome** 으로 채운다 —
/// get_snapshot/predict_ack 자체를 제거했다.
///
/// ★불변식 2(Ack→replay FIFO) 유지★: subscribe_from 은 subscribers lock 을 보유한 채,
/// replay 를 sink 로 보내기 **직전**에 on_ready(&outcome) 콜백을 1회 호출한다. 콜백 안에서
/// SubscribeAck(control)를 conn_tx 에 try_send 하므로, 그 enqueue 가 replay binary 의
/// try_send 보다 반드시 먼저 일어난다(단일 writer FIFO → Ack→replay→ReplayComplete 순서).
///
/// 흐름:
/// 1. agent_epoch 으로 epoch_matches 계산(없으면 error). (옛 list_agents 전체 순회 대체 — m-1.)
/// 2. WsOutputSink 생성(close_signal/replay_dropped 공유).
/// 3. subscribe_from(.., on_ready) — 콜백 안에서 outcome 기반 SubscribeAck 를 먼저 큐잉,
///    이어서 코어가 replay 를 sink 로 전송.
/// 4. 반환 outcome 으로 subs 맵 교체(옛 sid 다르면 unsubscribe).
/// 5. 사후 truncated 보정: outcome.kind==Truncated 가 아닌데 실측 replay_dropped 이면 Error 통보.
/// 6. ReplayComplete.
#[allow(clippy::too_many_arguments)]
async fn handle_subscribe(
    agent_id: AgentId,
    requested_epoch: Option<u32>,
    after_seq: Option<u64>,
    conn_tx: &mpsc::Sender<WsOutbound>,
    manager: &Arc<AgentManager>,
    subs: &Arc<Mutex<HashMap<AgentId, SinkId>>>,
    close_signal: &Arc<Notify>,
) {
    // 1. current_epoch 경량 조회. agent 없으면 즉시 error(이 경우 subscribe_from 미호출 → Ack 안 나감).
    let current_epoch = match manager.agent_epoch(agent_id) {
        Some(e) => e,
        None => {
            send_error(
                conn_tx,
                None,
                format!("subscribe failed: agent {agent_id} not found"),
            )
            .await;
            return;
        }
    };
    // epoch 일치 = 요청 epoch 이 현재 epoch 과 정확히 같을 때만. None(미지정)은 불일치 취급
    // → 코어가 FromOldest 로 전체 replay(안전 기본값).
    let epoch_matches = requested_epoch == Some(current_epoch);

    // 2. WsOutputSink 생성(close_signal/replay_dropped 공유).
    let sink = Arc::new(WsOutputSink::new(conn_tx.clone(), close_signal.clone()));
    let replay_dropped = sink.replay_dropped_flag();

    // 3. subscribe_from(.., on_ready). on_ready 는 코어가 replay 를 sink 로 보내기 직전
    //    (subscribers lock 보유 중) 1회 호출 → 그 안에서 SubscribeAck 를 conn_tx 에 먼저 try_send.
    //    ★콜백은 sync 클로저(await 불가) → try_send 만★. control 은 작아 보통 성공하나, full 이면
    //    어차피 같은 큐(sink)도 막혀 replay 가 truncated 로 잡히므로 로깅 후 진행한다.
    let conn_tx_cb = conn_tx.clone();
    let on_ready = move |outcome: &SubscribeOutcome| {
        if let Some(text) = event_json(&AgentEvent::SubscribeAck {
            agent_id,
            action: kind_to_action(outcome.kind),
            current_epoch,
            oldest_seq: outcome.oldest_seq,
            latest_seq: outcome.latest_seq,
            replay_from: outcome.replay_from,
            // 단일 스냅샷 기준 truncated. 실측 drop 보정은 호출 후 별도(아래 5).
            truncated: outcome.kind == ReplayKind::Truncated,
        }) {
            if let Err(e) = conn_tx_cb.try_send(WsOutbound::Text(text)) {
                tracing::warn!(%agent_id, "SubscribeAck try_send 실패(느린 소비자): {e}");
            }
        }
    };

    let outcome = match manager.subscribe_from(agent_id, sink, after_seq, epoch_matches, on_ready) {
        Ok(o) => o,
        Err(e) => {
            // agent 없음 등 — 콜백 미호출이라 Ack 안 나감(정상).
            send_error(conn_tx, None, format!("subscribe failed: {e}")).await;
            return;
        }
    };

    // 4. 같은 agent 재구독 시 옛 sink 가 남지 않게 교체(옛 것 unsubscribe).
    let old = subs
        .lock()
        .expect("subs poisoned")
        .insert(agent_id, outcome.sink_id);
    if let Some(old_sid) = old {
        if old_sid != outcome.sink_id {
            let _ = manager.unsubscribe(agent_id, old_sid);
        }
    }

    // 5. ReplayComplete 직전 사후 보정: replay 동기 전송 중 실제 drop 이 있었다면(코어가 sink 로
    //    try_send 하다 full 을 만남) Error 로 통보한다. Ack 엔 이미 정확한 kind 기반 truncated 가
    //    나갔고, 여기선 kind!=Truncated 인데 실측 drop 이 추가로 발생한 경우만 추가 통보한다.
    //    ★사전 capacity 추정 제거★: 단일 스냅샷이라 추정이 무의미 — replay_dropped 실측이 더 정확.
    if outcome.kind != ReplayKind::Truncated && replay_dropped.load(Ordering::Acquire) {
        send_error(
            conn_tx,
            None,
            format!("replay truncated for agent {agent_id}: output dropped during replay; please refresh"),
        )
        .await;
    }

    // 6. ReplayComplete — 이후는 라이브(클라측 C4 전환 신호).
    if let Some(text) = event_json(&AgentEvent::ReplayComplete {
        agent_id,
        epoch: current_epoch,
    }) {
        let _ = conn_tx.send(WsOutbound::Text(text)).await;
    }
}

/// 입력 lease 상태 변경을 전 연결에 브로드캐스트(다른 뷰어가 "잠김/해제" 를 알게). 보유자 식별값은
/// 노출하지 않고 held(bool) 만 — §5(LLM 도 leaseholder 변화를 관측 가능). registry 의 try_send 라
/// pump/cleanup 등 어느 컨텍스트에서 불려도 안전(block 없음).
fn broadcast_lease_changed(registry: &ConnRegistry, agent_id: AgentId, held: bool) {
    let ev = AgentEvent::InputLeaseChanged { agent_id, held };
    if let Some(text) = event_json(&ev) {
        registry.broadcast_text(text);
    }
}

/// 현재 프로필 목록을 전 연결에 브로드캐스트(ProfileListUpdated). 프로필 CRUD(생성/삭제/토글)는
/// 공유 ProfileRegistry 상태를 바꾸므로 모든 뷰어가 최신 목록을 보게 한다(agent_list_updated 와 동형).
fn broadcast_profile_list(registry: &ConnRegistry, manager: &Arc<AgentManager>) {
    let ev = AgentEvent::ProfileListUpdated {
        profiles: core_profiles_to_wire(manager.profiles().list()),
    };
    if let Some(text) = event_json(&ev) {
        registry.broadcast_text(text);
    }
}

/// Error 이벤트를 conn_tx 로 큐잉(control).
async fn send_error(
    conn_tx: &mpsc::Sender<WsOutbound>,
    request_id: Option<engram_dashboard_protocol::RequestId>,
    message: String,
) {
    if let Some(text) = event_json(&AgentEvent::Error {
        request_id,
        message,
    }) {
        let _ = conn_tx.send(WsOutbound::Text(text)).await;
    }
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

    // ── 1b. kind_to_action 매핑(Step 4c — M-A fix) ──────────────────────────
    //    옛 predict_ack(분기 예측)을 제거하고, 코어 outcome.kind → SubscribeAction 단순 매핑만
    //    남겼다(분기는 코어 단일 스냅샷이 소유). 3 variant 전수 검증.
    #[test]
    fn kind_to_action_maps_all_variants() {
        assert_eq!(
            kind_to_action(ReplayKind::FromOldest),
            SubscribeAction::Reset
        );
        assert_eq!(
            kind_to_action(ReplayKind::Truncated),
            SubscribeAction::TruncatedReplay
        );
        assert_eq!(kind_to_action(ReplayKind::Resumed), SubscribeAction::Resume);
    }

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
            data,
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
            data: b"x",
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

    // ── 7. core→wire AgentInfo 변환 roundtrip(serde 형태 일치) ────────────────
    #[test]
    fn core_agent_info_converts_to_wire() {
        use engram_dashboard_core::pty::types::{
            AgentInfo as Ci, Capabilities, ControlCaps, InputCaps, ModelCaps, OutputCaps,
            SessionCaps,
        };
        let core = Ci {
            id: uuid::Uuid::new_v4(),
            name: "t".into(),
            cwd: "/tmp".into(),
            status: CoreStatus::Running,
            cols: 80,
            rows: 24,
            epoch: 3,
            capabilities: Capabilities {
                input: InputCaps {
                    raw: true,
                    message: false,
                    attachment: false,
                },
                output: OutputCaps {
                    terminal_bytes: true,
                    markdown: false,
                    tool_events: false,
                    usage: false,
                },
                control: ControlCaps {
                    resize: true,
                    interrupt: true,
                    cancel: false,
                    graceful_shutdown: true,
                },
                session: SessionCaps {
                    resume: true,
                    snapshot: true,
                    cwd_env: true,
                },
                model: ModelCaps {
                    select: false,
                    temperature: false,
                    max_tokens: false,
                },
            },
        };
        let wire = core_agents_to_wire(vec![core.clone()]);
        assert_eq!(wire.len(), 1, "변환 성공(JSON 형태 일치)");
        assert_eq!(wire[0].name, "t");
        assert_eq!(wire[0].epoch, 3);
    }

    // ── 8. (M3) core::AgentStatus 모든 variant 가 wire 로 roundtrip 되는지 ────────
    //    어느 한 variant 라도 serde 태깅/필드가 어긋나면 core_agents_to_wire 가 그 agent 를
    //    silent drop 하므로 wire.len() < 1 이 되어 실패한다. status 값 자체도 wire 와 동일
    //    JSON tag 인지 직접 비교해 "변환은 됐지만 다른 variant 로 둔갑" 도 잡는다.
    #[test]
    fn all_core_status_variants_roundtrip_to_wire() {
        use engram_dashboard_core::pty::types::{
            AgentInfo as Ci, Capabilities, ControlCaps, InputCaps, ModelCaps, OutputCaps,
            SessionCaps,
        };
        use engram_dashboard_protocol::AgentStatus as WireStatus;

        let caps = Capabilities {
            input: InputCaps {
                raw: true,
                message: false,
                attachment: false,
            },
            output: OutputCaps {
                terminal_bytes: true,
                markdown: false,
                tool_events: false,
                usage: false,
            },
            control: ControlCaps {
                resize: true,
                interrupt: true,
                cancel: false,
                graceful_shutdown: true,
            },
            session: SessionCaps {
                resume: true,
                snapshot: true,
                cwd_env: true,
            },
            model: ModelCaps {
                select: false,
                temperature: false,
                max_tokens: false,
            },
        };

        // (core status, 기대 wire status) 쌍 — 6개 variant 전수.
        let cases: Vec<(CoreStatus, WireStatus)> = vec![
            (CoreStatus::Running, WireStatus::Running),
            (CoreStatus::Exiting, WireStatus::Exiting),
            (
                CoreStatus::Exited { code: Some(0) },
                WireStatus::Exited { code: Some(0) },
            ),
            (
                CoreStatus::Exited { code: None },
                WireStatus::Exited { code: None },
            ),
            (
                CoreStatus::Failed {
                    message: "boom".into(),
                },
                WireStatus::Failed {
                    message: "boom".into(),
                },
            ),
            (CoreStatus::Killed, WireStatus::Killed),
        ];

        for (core_status, expected_wire) in cases {
            let core = Ci {
                id: uuid::Uuid::new_v4(),
                name: "v".into(),
                cwd: "/tmp".into(),
                status: core_status.clone(),
                cols: 80,
                rows: 24,
                epoch: 0,
                capabilities: caps.clone(),
            };
            // (a) AgentInfo 전체 변환에서 drop 되지 않아야 한다(silent drop 회귀 방지).
            let wire = core_agents_to_wire(vec![core]);
            assert_eq!(
                wire.len(),
                1,
                "variant {core_status:?} 가 core→wire 에서 drop 됨(태깅/필드 불일치)"
            );
            // (b) status 가 같은 wire variant 로 정확히 매핑됐는지(둔갑 방지).
            assert_eq!(
                wire[0].status, expected_wire,
                "variant {core_status:?} 가 다른 wire status 로 변환됨"
            );
            // (c) core_status_to_wire 단독 경로도 동일 결과.
            let direct = core_status_to_wire(core_status.clone());
            assert_eq!(direct, expected_wire, "직접 변환 경로도 일치해야 함");
        }
    }

    // ── 9. (적용1) core::RestoreOutcome 전 variant → wire 명시 변환 ──────────────────
    //    특히 FreshFallback 의 Uuid→String 변환을 명시 검증(옛 reflection 의 우연 호환 제거).
    #[test]
    fn all_restore_outcomes_convert_to_wire() {
        use engram_dashboard_core::pty::profile::RestoreOutcome as Co;
        use engram_dashboard_protocol::RestoreOutcome as Wo;

        let old = uuid::Uuid::new_v4();
        let new = uuid::Uuid::new_v4();

        // Resumed / Started — unit variant.
        assert_eq!(restore_outcome_to_wire(&Co::Resumed), Wo::Resumed);
        assert_eq!(restore_outcome_to_wire(&Co::Started), Wo::Started);

        // FreshFallback(old=Some) — Uuid → String 변환 단언.
        match restore_outcome_to_wire(&Co::FreshFallback {
            old_sid: Some(old),
            new_sid: new,
            reason: "r".into(),
        }) {
            Wo::FreshFallback {
                old_sid,
                new_sid,
                reason,
            } => {
                assert_eq!(old_sid, Some(old.to_string()), "old_sid Uuid→String");
                assert_eq!(new_sid, new.to_string(), "new_sid Uuid→String");
                assert_eq!(reason, "r");
            }
            other => panic!("FreshFallback 기대, got {other:?}"),
        }

        // FreshFallback(old=None) — None 보존.
        match restore_outcome_to_wire(&Co::FreshFallback {
            old_sid: None,
            new_sid: new,
            reason: "r2".into(),
        }) {
            Wo::FreshFallback { old_sid, .. } => assert_eq!(old_sid, None, "None 보존"),
            other => panic!("FreshFallback 기대, got {other:?}"),
        }

        // Blocked / Failed — reason 보존.
        assert_eq!(
            restore_outcome_to_wire(&Co::Blocked { reason: "b".into() }),
            Wo::Blocked { reason: "b".into() }
        );
        assert_eq!(
            restore_outcome_to_wire(&Co::Failed { reason: "f".into() }),
            Wo::Failed { reason: "f".into() }
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
