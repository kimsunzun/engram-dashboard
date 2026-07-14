//! transport-중립 연결 코어(ConnectionCore) — ADR-0020 Stage 1.
//!
//! ws.rs 의 dispatch 로직을 carrier(WS/embedded/gRPC) 와 무관하게 빼낸 곳이다. 입력은
//! `AgentCommand`, 출력은 `Outbound`(AgentEvent/binary/close)를 `OutboundSink` 로만 흘린다 —
//! TcpStream/tungstenite/frame codec 을 이 모듈은 모른다. WS 어댑터(ws.rs)가 `OutboundSink`
//! 를 구현해 이 코어를 구동한다(미래 carrier 는 sink 만 새로 구현).
//!
//! ★불변식(R1~R7, ADR-0020) 보존이 절대 원칙 — Stage 1 은 behavior-preserving★:
//! - **R1 Ack→replay→ReplayComplete FIFO**: handle_subscribe 가 subscribers lock 보유 중
//!   on_ready 콜백으로 SubscribeAck 를 replay binary 보다 **먼저** enqueue 한다(아래 §3 참조).
//! - **R6 close_signal**: 큐 포화 out-of-band 종료는 WS-특정 → sink 구현(어댑터)이 SinkError
//!   해석으로 처리한다. 코어는 SinkError 만 본다(이 모듈은 close_signal 을 모른다).
//!
//! ★status fanout(broadcast)은 어댑터 디테일★: lease/profile 변경의 전-연결 브로드캐스트는
//! per-conn 응답(OutboundSink)이 아니라 `ConnRegistry`(carrier 별 fanout)로 간다. Stage 1 은
//! 동작 0 변경이므로 registry 를 ConnectionCore 가 참조로 들고 기존 broadcast 를 그대로 한다
//! (carrier-중립 fanout sink 로의 추상화는 Stage 2+ 작업).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use engram_dashboard_core::agent::manager::{default_shell, AgentManager};
use engram_dashboard_core::agent::profile::RestoreReport as CoreRestoreReport;
use engram_dashboard_core::agent::profile::SpawnMode;
use engram_dashboard_core::agent::types::{
    AgentId, AgentInfo as CoreAgentInfo, AgentStatus as CoreStatus, OutputSink, ReplayKind, SinkId,
    SubscribeOutcome,
};

use engram_dashboard_core::agent::preset::Preset as CorePreset;
use engram_dashboard_core::agent::profile::{
    AgentCommand as CoreSpawnCommand, AgentProfile as CoreProfile,
    ClaudeOutputFormat as CoreClaudeOutputFormat, RestartPolicy as CoreRestartPolicy,
    RestoreOutcome as CoreRestoreOutcome,
};
use engram_dashboard_core::agent::types::{
    Capabilities as CoreCaps, OutputChunk as CoreOutputChunk, OutputEvent as CoreOutputEvent,
};

use engram_dashboard_protocol::{
    AgentCommand, AgentEvent, AgentInfo as WireAgentInfo, AgentProfile as WireProfile,
    AgentSpawnCommand as WireSpawnCommand, Capabilities as WireCaps,
    ClaudeOutputFormat as WireClaudeOutputFormat, ControlCaps as WireControlCaps,
    InputCaps as WireInputCaps, ModelCaps as WireModelCaps, OutputCaps as WireOutputCaps,
    Preset as WirePreset, RestartPolicy as WireRestartPolicy, RestoreOutcome as WireRestoreOutcome,
    RestoreReport, SessionCaps as WireSessionCaps, SnapshotChunk as WireSnapshotChunk,
    StructuredEvent as WireStructuredEvent, SubscribeAction, PROTOCOL_VERSION,
};

use tokio::sync::watch;

use crate::ws::{ConnId, ConnRegistry};

// ── OutboundSink seam(ADR-0003 OutputSink 결을 따름) ──────────────────────────────
//
// dispatch 가 쓰던 reply/send_error/event_json 은 모두 conn_tx.send(WsOutbound::Text/Binary)
// 였다. 이를 carrier-중립 Outbound enqueue 로 치환한다. 인코딩(JSON text / binary frame)은
// sink 구현이 소유한다(코어는 모름) — OutputSink 가 frame→codec 을 sink 에 맡기는 것과 동형.

/// carrier-중립 송신 단위. WsOutbound(Text/Binary/Close)의 상위 개념.
/// Event=control(AgentEvent), Binary=출력 frame(이미 인코딩된 바이트), Close=연결 종료 요청.
///
/// ★Box<AgentEvent>★: AgentEvent 가 ~272B 라 다른 variant(24B)와 크기 차가 크다(clippy
/// large_enum_variant). control 경로는 hot path 가 아니므로(출력 binary 는 Binary variant) Box
/// 1회 할당이 무해하다. 생성은 `Outbound::event()` 헬퍼로 통일해 Box 를 숨긴다.
#[derive(Debug)]
pub enum Outbound {
    /// control 이벤트 — 직렬화(JSON 등)는 sink 구현이 소유. Box 로 enum 크기 축소.
    Event(Box<AgentEvent>),
    /// 이미 인코딩된 출력 frame 바이트(codec). subscribe 경로 외엔 거의 안 쓰임.
    Binary(Vec<u8>),
    /// 연결 종료 요청(reason 은 로그/디버깅용).
    Close(String),
}

impl Outbound {
    /// control 이벤트 Outbound 생성(Box 래핑을 숨기는 헬퍼).
    pub fn event(ev: AgentEvent) -> Self {
        Outbound::Event(Box::new(ev))
    }
}

/// sink enqueue 실패(큐 포화/닫힘). 어댑터가 carrier 별로 해석(WS=close_signal 발동 등, R6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SinkError;

impl std::fmt::Display for SinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "outbound sink enqueue failed")
    }
}

impl std::error::Error for SinkError {}

/// 한 연결의 출력 송신 추상. dispatch 의 모든 응답/이벤트가 이걸 통해 나간다.
/// WS 어댑터의 `WsOutboundSink` 가 conn_tx 에 push 하며 인코딩을 소유한다.
pub trait OutboundSink: Send + Sync {
    /// Outbound(control/binary/close)를 큐잉. 실패(포화/닫힘)면 SinkError(어댑터가 carrier 별로 해석).
    fn enqueue(&self, out: Outbound) -> Result<(), SinkError>;

    /// 코어 subscribe_from 에 넘길 output sink(코어 OutputSink 구현) + replay drop 플래그를 만든다.
    ///
    /// ★Stage 2 generic 화★: output frame 평면(replay/live binary)은 코어가 `Arc<dyn OutputSink>`
    /// (코어 trait)로 받는다. carrier(WS/embedded/gRPC)마다 인코딩이 달라(WS=binary frame,
    /// embedded=base64 PtyEvent) sink 구현이 다르므로, 반환을 trait object 로 두어 carrier-중립으로
    /// 만든다. 함께 반환하는 `Arc<AtomicBool>` 은 replay 구간 중 frame drop(try_send full) 여부 —
    /// handle_subscribe 가 ReplayComplete 직전 검사해 SubscribeAck.truncated 를 사후 보정한다.
    /// (Stage 1 은 구체 `Arc<WsOutputSink>` 반환이라 carrier 추가가 막혔다 — reviewer-deep 지적.)
    fn make_output_sink(&self) -> (Arc<dyn OutputSink>, Arc<AtomicBool>);
}

/// dispatch 의 연결 종료 흐름. 현 dispatch 의 bool 반환(true=StopDaemon)을 대체한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchFlow {
    /// 연결 유지(현 dispatch 의 false).
    Continue,
    /// 연결 종료(현 dispatch 의 true — StopDaemon).
    Close,
}

// ── per-conn 수명 상태(ConnectionSession) ─────────────────────────────────────────
//
// 현 handle_connection 의 지역변수(subs/owned_viewports)와 conn_id 를 묶었다. 연결당 1개.
// read_task/cleanup 이 공유하므로 내부 필드는 그대로 Arc<Mutex<..>> 를 유지한다(동시성 동일).

/// 한 연결의 수명 상태. dispatch 가 구독/viewport 추적을 갱신한다.
pub struct ConnectionSession {
    /// 연결 식별자(lease 보유자 판정·등).
    pub conn_id: ConnId,
    /// 이 연결이 등록한 (agent_id → sink_id) — cleanup 에서 누수 없이 unsubscribe.
    pub subs: Arc<Mutex<HashMap<AgentId, SinkId>>>,
    /// 이 연결이 등록한 (agent_id, viewport_id) 들 — cleanup 에서 viewport 협상 맵 정리.
    pub owned_viewports: Arc<Mutex<Vec<(AgentId, String)>>>,
}

impl ConnectionSession {
    pub fn new(conn_id: ConnId) -> Self {
        Self {
            conn_id,
            subs: Arc::new(Mutex::new(HashMap::new())),
            owned_viewports: Arc::new(Mutex::new(Vec::new())),
        }
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
    pub fn remove_conn_viewports(
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
    pub fn release_all_for_conn(&self, conn_id: ConnId) -> Vec<AgentId> {
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
            structured: c.output.structured,
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
pub(crate) fn agent_info_to_wire(a: &CoreAgentInfo) -> WireAgentInfo {
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
pub(crate) fn restore_outcome_to_wire(outcome: &CoreRestoreOutcome) -> WireRestoreOutcome {
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

pub(crate) fn core_agents_to_wire(agents: Vec<CoreAgentInfo>) -> Vec<WireAgentInfo> {
    agents.iter().map(agent_info_to_wire).collect()
}

/// core profile::AgentCommand → wire AgentSpawnCommand. 2 variant 전수 명시.
fn spawn_command_to_wire(cmd: &CoreSpawnCommand) -> WireSpawnCommand {
    match cmd {
        CoreSpawnCommand::Claude {
            extra_args,
            output_format,
        } => WireSpawnCommand::Claude {
            extra_args: extra_args.clone(),
            // output_format(ADR-0044) 명시 매핑 — 두 enum 전수 대응(추가 시 컴파일 에러).
            output_format: match output_format {
                CoreClaudeOutputFormat::Terminal => WireClaudeOutputFormat::Terminal,
                CoreClaudeOutputFormat::StreamJson => WireClaudeOutputFormat::StreamJson,
            },
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
        // ADR-0061 리치화: 표시명 override(트리 rename). None 이면 프론트가 cwd basename 파생(기존 동작 불변).
        display_name: p.display_name.clone(),
        // ADR-0072: 트리 계층 부모 id. None 이면 루트. AgentId/ProfileId 는 동일 Uuid alias 라 그대로 복사.
        parent_id: p.parent_id,
        command: spawn_command_to_wire(&p.command),
        cwd: p.cwd.to_string_lossy().into_owned(),
        env: p.env.clone(),
        claude_session_id: p.claude_session_id.map(|u| u.to_string()),
        old_session_ids: p.old_session_ids.iter().map(|u| u.to_string()).collect(),
        epoch: p.epoch,
        auto_restore: p.auto_restore,
        restart_policy: restart_policy_to_wire(p.restart_policy),
        restart_count: p.restart_count,
        failed_reason: p.failed_reason.clone(),
        created_at: p.created_at,
        last_active: p.last_active,
        last_start_at: p.last_start_at,
    }
}

fn core_profiles_to_wire(profiles: Vec<CoreProfile>) -> Vec<WireProfile> {
    profiles.iter().map(profile_to_wire).collect()
}

/// core Preset → wire(ADR-0061). profile_to_wire 와 동일 원칙 — PathBuf→String 명시 변환
/// (reflection 왕복 금지, 필드 추가/개명 시 컴파일 에러). 표시명 override(name)는 그대로 옮기고,
/// None 이면 프론트가 cwd basename 을 파생한다(리치화 전 동작 불변, ADR-0061).
fn preset_to_wire(p: &CorePreset) -> WirePreset {
    WirePreset {
        id: p.id,
        cwd: p.cwd.to_string_lossy().into_owned(),
        name: p.name.clone(),
    }
}

fn core_presets_to_wire(presets: Vec<CorePreset>) -> Vec<WirePreset> {
    presets.iter().map(preset_to_wire).collect()
}

/// core OutputChunk → wire SnapshotChunk. {seq, data} 명시 매핑.
fn snapshot_chunk_to_wire(c: &CoreOutputChunk) -> WireSnapshotChunk {
    WireSnapshotChunk {
        seq: c.seq,
        data: c.data.clone(),
    }
}

/// ★S15 B7 (ADR-0045)★: core `OutputEvent` → wire `StructuredEvent`(tag1 payload). 각 core variant 를
/// wire variant 로 **명시 매핑**(필드 그대로 옮김) — variant/필드 추가·개명 시 컴파일 에러로 강제한다
/// (다른 `*_to_wire` 와 동일 원칙: reflection 왕복 금지, silent drop 차단). turn_id/message_id/id 는
/// optional 그대로 옮겨 교체성(codex/gemini 가 못 채우면 None)을 보존한다.
///
/// ★반환이 Option 인 이유(TerminalBytes 방어)★: 정상 경로에서 `OutputEvent::TerminalBytes` 는 이 변환에
/// **오지 않는다** — 콘솔 raw 바이트는 sink 에서 tag0 terminal frame(`OutputPayload::Bytes`)으로 갈리고,
/// 이 함수는 `OutputPayload::Event` arm(tag1)에서만 불린다(ws.rs). wire `StructuredEvent` 에는 TerminalBytes
/// variant 가 없으므로(tag1 payload 에 raw 바이트를 안 싣는다 — ADR-0045), 만약 TerminalBytes 가 이 arm 에
/// 도달하면 매핑 불가다. 그때 패닉 대신 `None` 을 돌려 호출부(ws.rs)가 warn 후 drop 하게 한다(런타임 안전 —
/// tag0/tag1 오분류는 상류 배선 버그지 이 frame 하나로 연결을 죽일 사안이 아님). debug 빌드는 호출부에서
/// debug_assert 로 조기 발견한다.
pub(crate) fn output_event_to_wire(ev: &CoreOutputEvent) -> Option<WireStructuredEvent> {
    match ev {
        // tag0 전용 — tag1 payload 에 안 실린다(위 주석). 매핑 불가 → None(호출부 방어).
        CoreOutputEvent::TerminalBytes(_) => None,
        CoreOutputEvent::TextDelta {
            text,
            turn_id,
            message_id,
        } => Some(WireStructuredEvent::TextDelta {
            text: text.clone(),
            turn_id: turn_id.clone(),
            message_id: message_id.clone(),
        }),
        CoreOutputEvent::ToolCall {
            name,
            args_json,
            id,
            turn_id,
            message_id,
        } => Some(WireStructuredEvent::ToolCall {
            name: name.clone(),
            args_json: args_json.clone(),
            id: id.clone(),
            turn_id: turn_id.clone(),
            message_id: message_id.clone(),
        }),
        CoreOutputEvent::Usage {
            input_tokens,
            output_tokens,
            turn_id,
        } => Some(WireStructuredEvent::Usage {
            input_tokens: *input_tokens,
            output_tokens: *output_tokens,
            turn_id: turn_id.clone(),
        }),
        CoreOutputEvent::MessageDone {
            turn_id,
            message_id,
        } => Some(WireStructuredEvent::MessageDone {
            turn_id: turn_id.clone(),
            message_id: message_id.clone(),
        }),
        // core 는 tuple variant Error(String), wire 는 struct variant { message } — 명시 옮김.
        CoreOutputEvent::Error(message) => Some(WireStructuredEvent::Error {
            message: message.clone(),
        }),
        CoreOutputEvent::Structured { kind, json } => Some(WireStructuredEvent::Structured {
            kind: kind.clone(),
            json: json.clone(),
        }),
    }
}

/// core RestoreReport → wire. 모든 필드 명시(누락 시 컴파일 에러).
pub(crate) fn core_report_to_wire(report: CoreRestoreReport) -> RestoreReport {
    RestoreReport {
        agent_id: report.agent_id,
        epoch: report.epoch,
        outcome: restore_outcome_to_wire(&report.outcome),
    }
}

/// core AgentStatus → wire. StatusChanged 직렬화에 사용.
pub(crate) fn core_status_to_wire(status: CoreStatus) -> engram_dashboard_protocol::AgentStatus {
    status_to_wire(&status)
}

/// AgentEvent 를 JSON 문자열로 직렬화(control 전송용). 실패는 거의 불가능하나 로그 후 None.
/// (WS 어댑터의 DaemonStatusSink/WsOutboundSink 가 인코딩에 재사용한다.)
pub(crate) fn event_json(ev: &AgentEvent) -> Option<String> {
    match serde_json::to_string(ev) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::error!("AgentEvent 직렬화 실패: {e}");
            None
        }
    }
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

// ── ConnectionCore ────────────────────────────────────────────────────────────────

/// transport-중립 연결 코어. dispatch + 멀티뷰어 협상 + (Stage 1 한정) status fanout registry.
/// 연결마다가 아니라 **서버 전체에 1개** — manager/multiview/registry 는 전 연결이 공유한다.
/// per-conn 상태는 `ConnectionSession` 으로 dispatch 에 주입한다.
pub struct ConnectionCore {
    manager: Arc<AgentManager>,
    multiview: MultiViewState,
    /// status/lease/profile 브로드캐스트용. ★Stage 1 한정★: ADR-0020 R6/§5 대로 fanout 은
    /// carrier 디테일이라 추상 sink 로 안 올리고, behavior-preserving 위해 registry 를 그대로 든다.
    registry: ConnRegistry,
    /// StopDaemon 수신 시 main 종료를 트리거하는 watch(어댑터가 주입).
    shutdown_tx: watch::Sender<bool>,
}

impl ConnectionCore {
    pub fn new(
        manager: Arc<AgentManager>,
        multiview: MultiViewState,
        registry: ConnRegistry,
        shutdown_tx: watch::Sender<bool>,
    ) -> Self {
        Self {
            manager,
            multiview,
            registry,
            shutdown_tx,
        }
    }

    /// 멀티뷰어 협상 상태 접근(어댑터 cleanup 이 viewport/lease 정리에 사용).
    pub fn multiview(&self) -> &MultiViewState {
        &self.multiview
    }

    /// registry 접근(어댑터 cleanup 의 lease-freed 브로드캐스트에 사용).
    pub fn registry(&self) -> &ConnRegistry {
        &self.registry
    }

    /// manager 접근(어댑터 cleanup 의 unsubscribe/resize 에 사용).
    pub fn manager(&self) -> &Arc<AgentManager> {
        &self.manager
    }

    /// 단일 명령 dispatch. 반환 Close = 연결 종료 요청(StopDaemon). side-effect 명령은
    /// request_id 있으면 Ack/Error 를 sink 로 enqueue.
    ///
    /// ★sink.enqueue 실패(SinkError)는 무시★: side-effect 명령의 Ack/Error 송신 실패는 삼킨다.
    /// 단 의미 변경 주의 — baseline 의 control 응답은 `conn_tx.send(..).await`(큐 full 시 backpressure
    /// 블록)였고, 신규 WsOutboundSink::enqueue 는 `try_send`(full 시 즉시 drop + close_signal).
    /// 정상 단일 연결은 큐 여유로 control drop 이 안 나며, full 시 종착점(연결 종료)은 동일.
    pub async fn dispatch(
        &self,
        cmd: AgentCommand,
        session: &ConnectionSession,
        sink: &dyn OutboundSink,
    ) -> DispatchFlow {
        use engram_dashboard_protocol::RequestId;

        let manager = &self.manager;
        let multiview = &self.multiview;
        let registry = &self.registry;
        let conn_id = session.conn_id;
        let subs = &session.subs;
        let owned_viewports = &session.owned_viewports;

        /// side-effect 결과를 Ack/Error 로 변환해 sink 로 enqueue.
        fn reply(sink: &dyn OutboundSink, request_id: RequestId, result: Result<(), String>) {
            let ev = match result {
                Ok(()) => AgentEvent::Ack { request_id },
                Err(message) => AgentEvent::Error {
                    request_id: Some(request_id),
                    message,
                },
            };
            let _ = sink.enqueue(Outbound::event(ev));
        }

        match cmd {
            // 2번째 Auth 는 무시(Error 만 — 이미 인증된 연결).
            AgentCommand::Auth { .. } => {
                send_error(sink, None, "already authenticated".into());
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
                reply(sink, request_id, result);
            }

            AgentCommand::Kill {
                agent_id,
                request_id,
            } => {
                let result = manager.kill_agent(agent_id).map_err(|e| e.to_string());
                reply(sink, request_id, result);
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
                reply(sink, request_id, result);
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
                reply(sink, request_id, result);
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
                            let mut owned =
                                owned_viewports.lock().expect("owned_viewports poisoned");
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
                    send_error(sink, None, format!("resize failed: {e}"));
                }
            }

            AgentCommand::Subscribe {
                agent_id,
                epoch,
                after_seq,
            } => {
                // Step 4c: epoch/after_seq 를 코어 subscribe_from 으로 전달 → 무손실 resume(tail 만)
                // 또는 truncated/full replay 분기.
                self.handle_subscribe(agent_id, epoch, after_seq, subs, sink);
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
                        reply(sink, request_id, Ok(()));
                    }
                    Ok(false) => reply(sink, request_id, Ok(())), // idempotent
                    Err(()) => reply(
                        sink,
                        request_id,
                        Err("input held by another viewer".to_string()),
                    ),
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
                        reply(sink, request_id, Ok(()));
                    }
                    Ok(false) => reply(sink, request_id, Ok(())), // 원래 비어 있음
                    Err(()) => reply(
                        sink,
                        request_id,
                        Err("input lease held by another viewer".to_string()),
                    ),
                }
            }

            AgentCommand::ListAgents { request_id } => {
                // 조회 응답은 request_id 동봉 전용 reply(AgentList)로 요청 연결에만 — 편승 매칭 제거.
                // broadcast 인 AgentListUpdated(트리 실시간 갱신)는 StatusSink/agent_list_updated 가 별도 담당.
                let _ = sink.enqueue(Outbound::event(AgentEvent::AgentList {
                    request_id,
                    agents: core_agents_to_wire(manager.list_agents()),
                }));
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
                        sink,
                        Some(request_id),
                        format!(
                            "active agents present ({active_count}); use force=true to stop the daemon"
                        ),
                    );
                    return DispatchFlow::Continue; // 거부 — 연결 유지, main 종료 안 함.
                }

                // ★kill_agents 는 v1 에서 무시(always-kill)★: 데몬은 자식 PTY 를 자기
                //   KILL_ON_JOB_CLOSE Job Object 에 담는다. 따라서 데몬이 종료되면 Job 핸들이
                //   닫히며 자식이 **무조건** 함께 죽는다 — detach(데몬만 내리고 자식 유지)는 현
                //   Job 모델에선 불가능하다. kill_agents 플래그는 미래에 detach 를 지원하게 될
                //   여지로 protocol 에 남겨두되, v1 동작은 값과 무관하게 항상 자식을 정리한다.
                let _ = kill_agents; // 의도적 무시(위 주석) — 미래 detach 지원 여지.
                let mgr = manager.clone();
                let _ = tokio::task::spawn_blocking(move || mgr.shutdown_all()).await;

                reply(sink, request_id, Ok(()));
                // main 종료 트리거(watch). 수신측은 run() 의 select! 가 감지.
                let _ = self.shutdown_tx.send(true);
                return DispatchFlow::Close;
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
                        let _ = sink.enqueue(Outbound::event(AgentEvent::Spawned {
                            request_id,
                            agent: agent_info_to_wire(&info),
                        }));
                    }
                    Err(e) => reply(sink, request_id, Err(e.to_string())),
                }
            }

            AgentCommand::ListProfiles { request_id } => {
                // Tauri `list_profiles` 미러 — 읽기 전용 조회. request_id 동봉 전용 reply(ProfileList)로
                // 요청 연결에만 응답(ListAgents 와 동형). broadcast ProfileListUpdated 는 CRUD 후 별도 push.
                let _ = sink.enqueue(Outbound::event(AgentEvent::ProfileList {
                    request_id,
                    profiles: core_profiles_to_wire(manager.profiles().list()),
                }));
            }

            AgentCommand::CreateProfile {
                name,
                cwd,
                extra_args,
                env,
                auto_restore,
                output_format,
                request_id,
            } => {
                // Tauri `create_claude_profile` 미러: claude 프로필 생성·upsert(스폰 안 함).
                // ADR-0044 M2: wire output_format → core 로 명시 매핑(spawn_command_to_wire 의 역방향).
                //   StreamJson 이면 프로필이 json 모드로 저장돼, 이후 SpawnProfile → spawn_agent 가
                //   is_json_mode 로 StdioTransport(구조화 caps)를 고른다. Terminal 은 기존 동작 불변.
                let core_output_format = match output_format {
                    WireClaudeOutputFormat::Terminal => CoreClaudeOutputFormat::Terminal,
                    WireClaudeOutputFormat::StreamJson => CoreClaudeOutputFormat::StreamJson,
                };
                let profile = CoreProfile::new(
                    name,
                    CoreSpawnCommand::Claude {
                        extra_args,
                        output_format: core_output_format,
                    },
                    std::path::PathBuf::from(cwd),
                    env,
                    auto_restore,
                );
                // upsert 가 profile 을 move 하므로 wire 변환을 먼저 떠둔다.
                let wire = profile_to_wire(&profile);
                manager.profiles().upsert(profile);
                // requester 에겐 Created(생성된 프로필 동봉)로 응답 — Ack 는 보내지 않는다(중복 resolve 방지).
                let _ = sink.enqueue(Outbound::event(AgentEvent::Created {
                    request_id,
                    profile: wire,
                }));
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
                reply(sink, request_id, Ok(()));
                broadcast_profile_list(registry, manager);
            }

            AgentCommand::SpawnProfile {
                profile_id,
                resume,
                request_id,
            } => {
                // Tauri `spawn_profile` 미러: 저장된 프로필을 Resume/Fresh 로 spawn. 없으면 Error.
                //
                // ★모드 = 세션 존재 여부로 유도(ADR-0076 — "activate=resume, fresh=new agent")★:
                //   사용자 결정 — "에이전트 활성화 = 기존 세션 이어받기, 새로 로드할 거면 새 에이전트를 만든다".
                //   그래서 저장된 세션이 있는 프로필을 활성화하면 wire `resume` 플래그(프론트가 false 로 보냄)와
                //   무관하게 **항상 Resume** 으로 이어받는다. 세션이 없는(진짜 신규) 프로필만 Fresh 로 시작한다.
                //   ★resume=true 는 존중★: 명시적 resume 요청은 세션이 없어도 Resume 로 남긴다 — 그 경우
                //     spawn_agent(Resume)가 ensure_session_id 로 최초 sid 를 발급하므로 안전하다(sid 발급은
                //     spawn_agent 단일 권위점, ADR-0076). 즉 mode = resume-요청 OR 세션-존재.
                //
                // ★이어받기 전용 + 재활성화 가드(ADR-0082 — fresh-fallback 폐지)★:
                //   spawn_agent 이 아니라 activate_profile 을 부른다. activate_profile 이 세 갈래를 처리한다:
                //   ① 이미 실행 중이면 산 에이전트를 놔두고 현재 AgentInfo 를 그대로 반환(재활성화 가드 —
                //      a4aac1a 회귀 수정: 이중-spawn 가드 Err 가 옛 fresh-fallback 을 발화해 산 에이전트를
                //      파괴하던 경로를 원천 차단). ② Fresh(세션 없음)는 정상 신규 spawn. ③ Resume 은
                //      이어받기만 시도하고, 이어받을 수 없으면(빈/미대화/손상 — claude "No conversation
                //      found ...") **새 대화를 만들지 않고** Failed(시체)로 남기고 원인을 로그로 남긴다
                //      (LLM 이 읽어 사용자에게 에스컬레이션 — 사용자 결정: "아무것도 죽지마, 새로 만들지마").
                //   Resume 은 blocking(EARLY_EXIT_WINDOW)이라 이 연결 응답만 지연(다른 세션 무영향).
                // 성공(resume·재활성화·fresh) 시 Spawned(AgentInfo 동봉)로 응답, 실패/없음은 Error.
                // agent_list_updated 는 StatusSink 가 브로드캐스트(Spawn arm 과 동일).
                match manager.profiles().get(profile_id) {
                    Some(profile) => {
                        let mode = if resume || profile.claude_session_id.is_some() {
                            SpawnMode::Resume
                        } else {
                            SpawnMode::Fresh
                        };
                        match manager.activate_profile(&profile, mode) {
                            Ok(info) => {
                                let _ = sink.enqueue(Outbound::event(AgentEvent::Spawned {
                                    request_id,
                                    agent: agent_info_to_wire(&info),
                                }));
                            }
                            Err(e) => reply(sink, request_id, Err(e.to_string())),
                        }
                    }
                    None => reply(
                        sink,
                        request_id,
                        Err(format!("profile not found: {profile_id}")),
                    ),
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
                    reply(sink, request_id, Ok(()));
                    broadcast_profile_list(registry, manager);
                } else {
                    reply(
                        sink,
                        request_id,
                        Err(format!("profile not found: {profile_id}")),
                    );
                }
            }

            AgentCommand::RenameProfile {
                profile_id,
                name,
                request_id,
            } => {
                // ADR-0061 리치화(트리 rename): 표시명 override set/clear. SetProfileAutoRestore 와 동형 —
                // update_with(persist 일원화) 로 mutate 후 없으면 Error. 성공 시 전 연결에 broadcast(모든 창
                // 동기화·낙관 갱신 X — 프론트는 broadcast 로만 표시명 반영).
                let ok = manager.profiles().rename(profile_id, name);
                if ok {
                    reply(sink, request_id, Ok(()));
                    broadcast_profile_list(registry, manager);
                } else {
                    reply(
                        sink,
                        request_id,
                        Err(format!("profile not found: {profile_id}")),
                    );
                }
            }

            AgentCommand::ReparentProfile {
                child_id,
                parent_id,
                request_id,
            } => {
                // ADR-0072 트리 계층 reparent: 부모 지정/해제. 검증(self-parent·nonexistent parent·1단 상한·
                // 2단 금지)은 ProfileRegistry::reparent 가 한 임계구역에서 수행 — 위반이면 false 로 Error,
                // 성공이면 Ack + 전 연결 broadcast(RenameProfile 와 동형, 모든 창 동기화·낙관 갱신 X).
                let ok = manager.profiles().reparent(child_id, parent_id);
                if ok {
                    reply(sink, request_id, Ok(()));
                    broadcast_profile_list(registry, manager);
                } else {
                    reply(
                        sink,
                        request_id,
                        Err(format!(
                            "reparent rejected (missing/self-parent/cycle/2-level): child={child_id}"
                        )),
                    );
                }
            }

            AgentCommand::GetSnapshot {
                agent_id,
                request_id,
            } => {
                // Tauri `get_agent_snapshot` 미러: 그 시점 replay buffer 스냅샷 1회 조회. 없으면 Error.
                // Snapshot 에 request_id 를 동봉(전용 reply)하므로 별도 Ack 는 보내지 않는다 — Created/
                // Spawned 와 동형(응답 1건만, 중복 resolve 방지).
                match manager.get_snapshot(agent_id) {
                    Ok(chunks) => {
                        let _ = sink.enqueue(Outbound::event(AgentEvent::Snapshot {
                            request_id,
                            agent_id,
                            chunks: chunks.iter().map(snapshot_chunk_to_wire).collect(),
                        }));
                    }
                    Err(e) => reply(sink, request_id, Err(e.to_string())),
                }
            }

            // ── 프리셋 CRUD(ADR-0061) — 프로필 arm 미러 ─────────────────────────────
            AgentCommand::ListPresets { request_id } => {
                // 읽기 전용 조회. request_id 동봉 전용 reply(PresetList)로 요청 연결에만 응답
                // (ListProfiles 와 동형). broadcast PresetListUpdated 는 CRUD 후 별도 push.
                let _ = sink.enqueue(Outbound::event(AgentEvent::PresetList {
                    request_id,
                    presets: core_presets_to_wire(manager.presets().list()),
                }));
            }

            AgentCommand::CreatePreset { cwd, request_id } => {
                // 프리셋 생성·persist(스폰 안 함). cwd 정규화(dunce::canonicalize)는 PresetRegistry 가 한다.
                // remove/create 는 무조건 성공(중복 판정 없음 — MVP) → Ack. 생성은 공유 상태 변경이므로
                // 전 연결에 갱신 목록 broadcast(모든 창 동기화, ADR-0061 불변식).
                manager.presets().create(std::path::PathBuf::from(cwd));
                reply(sink, request_id, Ok(()));
                broadcast_preset_list(registry, manager);
            }

            AgentCommand::DeletePreset {
                preset_id,
                request_id,
            } => {
                // 등록 해제·persist. ★프리셋 삭제 ≠ 에이전트 종료★(ADR-0061) — remove 는 프리셋만 지운다.
                // 없는 id 면 no-op(프로필 DeleteProfile 과 동일하게 Ack). 이후 broadcast.
                manager.presets().remove(preset_id);
                reply(sink, request_id, Ok(()));
                broadcast_preset_list(registry, manager);
            }

            AgentCommand::RenamePreset {
                preset_id,
                name,
                request_id,
            } => {
                // ADR-0061 리치화: 표시명 override set/clear. DeletePreset 과 동형 — 없는 id 면 no-op(Ack).
                // 변경은 공유 PresetRegistry 상태를 바꾸므로 전 연결에 broadcast(모든 창 동기화·낙관 갱신 X).
                manager.presets().rename(preset_id, name);
                reply(sink, request_id, Ok(()));
                broadcast_preset_list(registry, manager);
            }
        }
        DispatchFlow::Continue
    }

    /// Subscribe 처리(Step 4c — afterSeq resume). **M-A(TOCTOU) 근본 해결판.**
    ///
    /// ★TOCTOU 제거★: 옛 구현은 get_snapshot(스냅샷 A)으로 SubscribeAck 를 예측해 보낸 뒤,
    /// subscribe_from 이 내부에서 다시 스냅샷 B 를 떠 replay 했다. A≠B(사이에 evict 가 끼면)면
    /// Ack.replay_from/latest 가 실제 첫 전송 seq 와 어긋나 클라가 손실을 인지 못 했다. 이제는
    /// SubscribeAck 의 모든 필드를 subscribe_from 의 **단일 스냅샷 outcome** 으로 채운다 —
    /// get_snapshot/predict_ack 자체를 제거했다.
    ///
    /// ★불변식 R1(Ack→replay FIFO) 유지★: subscribe_from 은 subscribers lock 을 보유한 채,
    /// replay 를 sink 로 보내기 **직전**에 on_ready(&outcome) 콜백을 1회 호출한다. 콜백 안에서
    /// SubscribeAck(control)를 sink 로 enqueue 하므로, 그 enqueue 가 replay binary 의 enqueue
    /// 보다 반드시 먼저 일어난다(단일 writer FIFO → Ack→replay→ReplayComplete 순서).
    ///
    /// ★output sink 의 정체★: 코어 subscribe_from 에 넘기는 sink 는 어댑터가 만든
    /// `WsOutputSink`(코어 OutputSink) 다 — 이건 응답용 `OutboundSink`(dispatch 의 sink)와는
    /// 다른 평면(코어 출력 frame 평면). Stage 1 은 동작 보존이라 WS 어댑터의 WsOutputSink 를
    /// 코어에 직접 넘긴다(handle_connection 이 만든 conn_tx/close_signal 공유). control(Ack/
    /// ReplayComplete/Error)만 dispatch sink(OutboundSink)로 enqueue 한다 — 둘 다 같은 conn_tx
    /// 단일 writer 큐로 합류하므로 FIFO 가 보존된다.
    fn handle_subscribe(
        &self,
        agent_id: AgentId,
        requested_epoch: Option<u32>,
        after_seq: Option<u64>,
        subs: &Arc<Mutex<HashMap<AgentId, SinkId>>>,
        sink: &dyn OutboundSink,
    ) {
        let manager = &self.manager;

        // 1. current_epoch 경량 조회. agent 없으면 즉시 error(이 경우 subscribe_from 미호출 → Ack 안 나감).
        let current_epoch = match manager.agent_epoch(agent_id) {
            Some(e) => e,
            None => {
                send_error(
                    sink,
                    None,
                    format!("subscribe failed: agent {agent_id} not found"),
                );
                return;
            }
        };
        // epoch 일치 = 요청 epoch 이 현재 epoch 과 정확히 같을 때만. None(미지정)은 불일치 취급
        // → 코어가 FromOldest 로 전체 replay(안전 기본값).
        let epoch_matches = requested_epoch == Some(current_epoch);

        // 2. output sink 생성(코어 OutputSink) + replay drop 플래그. carrier 가 인코딩을 소유한다
        //    (WS=binary frame, embedded=base64 PtyEvent). 반환은 trait object 라 carrier-중립.
        //    ★output frame 평면★: 이 sink 는 replay/live 출력 frame 을 carrier 큐로 보낸다.
        let (out_sink, replay_dropped) = sink.make_output_sink();

        // 3. subscribe_from(.., on_ready). on_ready 는 코어가 replay 를 sink 로 보내기 직전
        //    (subscribers lock 보유 중) 1회 호출 → 그 안에서 SubscribeAck 를 control sink 로 먼저 enqueue.
        //    ★콜백은 sync 클로저(await 불가) → enqueue(try_send)만★. control 은 작아 보통 성공하나,
        //    full 이면 어차피 같은 큐(output sink)도 막혀 replay 가 truncated 로 잡히므로 진행한다.
        //    ★FIFO 핵심★: 두 sink(control/output)가 같은 conn_tx 단일 writer 로 합류하므로,
        //    여기서 control 을 먼저 enqueue 하면 replay binary 보다 반드시 앞선다(R1).
        let on_ready = |outcome: &SubscribeOutcome| {
            let _ = sink.enqueue(Outbound::event(AgentEvent::SubscribeAck {
                agent_id,
                action: kind_to_action(outcome.kind),
                current_epoch,
                oldest_seq: outcome.oldest_seq,
                latest_seq: outcome.latest_seq,
                replay_from: outcome.replay_from,
                // 단일 스냅샷 기준 truncated. 실측 drop 보정은 호출 후 별도(아래 5).
                truncated: outcome.kind == ReplayKind::Truncated,
            }));
        };

        let outcome =
            match manager.subscribe_from(agent_id, out_sink, after_seq, epoch_matches, on_ready) {
                Ok(o) => o,
                Err(e) => {
                    // agent 없음 등 — 콜백 미호출이라 Ack 안 나감(정상).
                    send_error(sink, None, format!("subscribe failed: {e}"));
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
                sink,
                None,
                format!("replay truncated for agent {agent_id}: output dropped during replay; please refresh"),
            );
        }

        // 6. ReplayComplete — 이후는 라이브(클라측 C4 전환 신호).
        let _ = sink.enqueue(Outbound::event(AgentEvent::ReplayComplete {
            agent_id,
            epoch: current_epoch,
        }));
    }
}

/// 입력 lease 상태 변경을 전 연결에 브로드캐스트(다른 뷰어가 "잠김/해제" 를 알게). 보유자 식별값은
/// 노출하지 않고 held(bool) 만 — §5(LLM 도 leaseholder 변화를 관측 가능). registry 의 try_send 라
/// pump/cleanup 등 어느 컨텍스트에서 불려도 안전(block 없음).
pub(crate) fn broadcast_lease_changed(registry: &ConnRegistry, agent_id: AgentId, held: bool) {
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

/// 현재 프리셋 목록을 전 연결에 브로드캐스트(PresetListUpdated, ADR-0061). 프리셋 CRUD(생성/삭제)는
/// 공유 PresetRegistry 상태를 바꾸므로 모든 창이 최신 목록을 보게 한다(broadcast_profile_list 와 동형).
/// ★create/delete 는 반드시 이 broadcast 로 이어진다★(안 그러면 다른 창이 stale — ADR-0061 불변식).
fn broadcast_preset_list(registry: &ConnRegistry, manager: &Arc<AgentManager>) {
    let ev = AgentEvent::PresetListUpdated {
        presets: core_presets_to_wire(manager.presets().list()),
    };
    if let Some(text) = event_json(&ev) {
        registry.broadcast_text(text);
    }
}

/// Error 이벤트를 sink 로 enqueue(control).
fn send_error(
    sink: &dyn OutboundSink,
    request_id: Option<engram_dashboard_protocol::RequestId>,
    message: String,
) {
    let _ = sink.enqueue(Outbound::event(AgentEvent::Error {
        request_id,
        message,
    }));
}

/// Hello 이벤트(연결 직후 어댑터가 push). protocol_version/daemon_version 동봉.
/// (어댑터가 carrier-중립으로 만들 수 있게 코어가 헬퍼 제공.)
pub fn hello_event(daemon_version: String) -> AgentEvent {
    AgentEvent::Hello {
        protocol_version: PROTOCOL_VERSION,
        daemon_version,
        capabilities: None,
    }
}

/// 현재 에이전트 목록 이벤트(연결 직후/브로드캐스트). 어댑터·StatusSink 공용.
pub fn agent_list_event(manager: &Arc<AgentManager>) -> AgentEvent {
    AgentEvent::AgentListUpdated {
        agents: core_agents_to_wire(manager.list_agents()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_dashboard_protocol::RequestId;
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;

    /// 테스트용 request_id 생성(RequestId 는 Uuid newtype).
    fn rid() -> RequestId {
        RequestId(uuid::Uuid::new_v4())
    }

    /// dispatch 응답을 순서대로 기록하는 mock OutboundSink. control(Outbound::Event)만 검증한다.
    /// ★output sink(WsOutputSink)는 conn_tx 기반이라 mock 으로 못 만든다★ — make_output_sink 가
    /// WsOutputSink 를 요구하므로, output frame 평면(replay binary)은 실 conn_tx 로 흘려 별도 채널로
    /// 받는다. 여기 mock 은 control 평면(Ack/Error/ReplayComplete/Spawned 등)만 본다.
    struct MockOutboundSink {
        events: Arc<StdMutex<Vec<AgentEvent>>>,
        /// handle_subscribe 가 요구하는 output sink 의 conn_tx(replay binary 가 여기로).
        conn_tx: tokio::sync::mpsc::Sender<crate::ws::WsOutbound>,
        close_signal: Arc<tokio::sync::Notify>,
    }

    impl MockOutboundSink {
        fn new(conn_tx: tokio::sync::mpsc::Sender<crate::ws::WsOutbound>) -> Self {
            Self {
                events: Arc::new(StdMutex::new(Vec::new())),
                conn_tx,
                close_signal: Arc::new(tokio::sync::Notify::new()),
            }
        }
        fn events(&self) -> Vec<AgentEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    impl OutboundSink for MockOutboundSink {
        fn enqueue(&self, out: Outbound) -> Result<(), SinkError> {
            match out {
                Outbound::Event(ev) => {
                    // control 은 conn_tx 에도 흘려(FIFO 검증용) 기록도 남긴다.
                    if let Some(text) = event_json(&ev) {
                        let _ = self.conn_tx.try_send(crate::ws::WsOutbound::Text(text));
                    }
                    self.events.lock().unwrap().push(*ev);
                    Ok(())
                }
                Outbound::Binary(b) => {
                    let _ = self.conn_tx.try_send(crate::ws::WsOutbound::Binary(b));
                    Ok(())
                }
                Outbound::Close(r) => {
                    let _ = self.conn_tx.try_send(crate::ws::WsOutbound::Close(r));
                    Ok(())
                }
            }
        }
        fn make_output_sink(&self) -> (Arc<dyn OutputSink>, Arc<AtomicBool>) {
            let sink = Arc::new(crate::ws::WsOutputSink::new(
                self.conn_tx.clone(),
                self.close_signal.clone(),
            ));
            let flag = sink.replay_dropped_flag();
            (sink, flag)
        }
    }

    fn test_core() -> (ConnectionCore, watch::Receiver<bool>) {
        // in-memory manager 배선(lib.rs build_manager_with_store 와 같은 결, 여기선 직접).
        use engram_dashboard_core::agent::preset::{PresetRegistry, PresetStore};
        use engram_dashboard_core::agent::profile::{ProfileRegistry, ProfileStore};
        use engram_dashboard_core::agent::session_tracker::{SessionTracker, TrackerConfig};

        #[derive(Default)]
        struct MemStore {
            saved: StdMutex<Vec<engram_dashboard_core::agent::profile::AgentProfile>>,
        }
        impl ProfileStore for MemStore {
            fn save(&self, p: &[engram_dashboard_core::agent::profile::AgentProfile]) {
                *self.saved.lock().unwrap() = p.to_vec();
            }
            fn load(&self) -> Vec<engram_dashboard_core::agent::profile::AgentProfile> {
                self.saved.lock().unwrap().clone()
            }
        }

        // ADR-0061: 프리셋 store 도 in-memory 로 배선(프로필과 동형).
        #[derive(Default)]
        struct MemPresetStore {
            saved: StdMutex<Vec<engram_dashboard_core::agent::preset::Preset>>,
        }
        impl PresetStore for MemPresetStore {
            fn save(&self, p: &[engram_dashboard_core::agent::preset::Preset]) {
                *self.saved.lock().unwrap() = p.to_vec();
            }
            fn load(&self) -> Vec<engram_dashboard_core::agent::preset::Preset> {
                self.saved.lock().unwrap().clone()
            }
        }

        let registry = ConnRegistry::new();
        let store: Arc<dyn ProfileStore> = Arc::new(MemStore::default());
        let preset_store: Arc<dyn PresetStore> = Arc::new(MemPresetStore::default());
        let status_sink = Arc::new(crate::ws::DaemonStatusSink::new(registry.clone()));
        let profiles = Arc::new(ProfileRegistry::new(store));
        let presets = Arc::new(PresetRegistry::new(preset_store));
        let tracker = Arc::new(SessionTracker::new(
            TrackerConfig::default(),
            Arc::new(|_aid, _sid| {}),
        ));
        let manager = Arc::new(AgentManager::new(status_sink, profiles, presets, tracker));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let core = ConnectionCore::new(manager, MultiViewState::new(), registry, shutdown_tx);
        (core, shutdown_rx)
    }

    // ── R1: Subscribe 시 [Ack, (replay)Binary..., ReplayComplete] 순서가 conn_tx 기록에 그대로 ──
    //    실 manager 에 결정적 출력 agent 를 띄워 replay binary 가 실제로 끼게 한다.
    #[tokio::test]
    async fn subscribe_emits_ack_then_replay_then_complete_in_order() {
        let (core, _rx) = test_core();
        // 결정적 출력을 내는 agent 를 띄운다(echo 한 줄). spawn 후 출력이 buffer 에 쌓이길 기다린다.
        let profile = engram_dashboard_core::agent::profile::AgentProfile::new(
            "t".into(),
            engram_dashboard_core::agent::profile::AgentCommand::Shell {
                program: default_shell().to_string(),
                args: vec![],
            },
            std::env::temp_dir(),
            vec![],
            false,
        );
        let info = core
            .manager
            .spawn_agent(&profile, SpawnMode::Fresh)
            .expect("spawn");
        // 셸 프롬프트/배너가 buffer 에 쌓이도록 잠깐 대기(폴링).
        let agent_id = info.id;
        let mut waited = 0;
        loop {
            if let Ok(chunks) = core.manager.get_snapshot(agent_id) {
                if !chunks.is_empty() {
                    break;
                }
            }
            if waited > 50 {
                break; // 출력 없어도 Ack/Complete 순서는 검증된다.
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
            waited += 1;
        }

        let (tx, mut conn_rx) = tokio::sync::mpsc::channel::<crate::ws::WsOutbound>(4608);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);

        core.dispatch(
            AgentCommand::Subscribe {
                agent_id,
                epoch: None,
                after_seq: None,
            },
            &session,
            &mock,
        )
        .await;

        // conn_rx 에 들어간 순서: 첫 Text=SubscribeAck, (있으면) Binary..., 마지막 Text=ReplayComplete.
        let mut items = Vec::new();
        while let Ok(item) = conn_rx.try_recv() {
            items.push(item);
        }
        assert!(items.len() >= 2, "최소 Ack+ReplayComplete: {}", items.len());
        // 첫 항목은 SubscribeAck Text.
        match &items[0] {
            crate::ws::WsOutbound::Text(s) => {
                assert!(s.contains("SubscribeAck"), "1번째는 SubscribeAck: {s}")
            }
            other => panic!("1번째는 Text(SubscribeAck) 여야 함: {other:?}"),
        }
        // 마지막 항목은 ReplayComplete Text.
        match items.last().unwrap() {
            crate::ws::WsOutbound::Text(s) => {
                assert!(s.contains("ReplayComplete"), "마지막은 ReplayComplete: {s}")
            }
            other => panic!("마지막은 Text(ReplayComplete) 여야 함: {other:?}"),
        }
        // 중간 항목(있다면)은 전부 Binary(replay frame) 여야 한다 — control 이 끼면 FIFO 깨짐.
        for mid in &items[1..items.len() - 1] {
            assert!(
                matches!(mid, crate::ws::WsOutbound::Binary(_)),
                "Ack 와 ReplayComplete 사이엔 replay Binary 만: {mid:?}"
            );
        }

        // events 기록상 control 순서도 Ack → ReplayComplete.
        let evs = mock.events();
        assert!(
            matches!(evs.first(), Some(AgentEvent::SubscribeAck { .. })),
            "control 첫 이벤트=SubscribeAck"
        );
        assert!(
            matches!(evs.last(), Some(AgentEvent::ReplayComplete { .. })),
            "control 마지막=ReplayComplete"
        );

        let _ = core.manager.kill_agent(agent_id);
    }

    // ── Subscribe: 없는 agent → Error, Ack 안 나감 ─────────────────────────────────
    #[tokio::test]
    async fn subscribe_unknown_agent_emits_error_no_ack() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<crate::ws::WsOutbound>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        core.dispatch(
            AgentCommand::Subscribe {
                agent_id: uuid::Uuid::new_v4(),
                epoch: None,
                after_seq: None,
            },
            &session,
            &mock,
        )
        .await;
        let evs = mock.events();
        assert_eq!(evs.len(), 1, "Error 1건만");
        assert!(matches!(evs[0], AgentEvent::Error { .. }), "Error 여야 함");
    }

    // ── Spawn: 없는 profile → Error(request_id 동봉) ──────────────────────────────
    #[tokio::test]
    async fn spawn_missing_profile_errors() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<crate::ws::WsOutbound>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let req = rid();
        core.dispatch(
            AgentCommand::Spawn {
                profile_id: uuid::Uuid::new_v4(),
                request_id: req,
            },
            &session,
            &mock,
        )
        .await;
        let evs = mock.events();
        match evs.as_slice() {
            [AgentEvent::Error {
                request_id: Some(r),
                ..
            }] => assert_eq!(*r, req, "Error 에 request_id 동봉"),
            other => panic!("Error(request_id) 1건 기대: {other:?}"),
        }
    }

    // ── ReparentProfile: 거부(false) → Error(request_id 동봉), Ack/broadcast 없음 (ADR-0072) ──
    //    broadcast_profile_list 는 registry.broadcast_text 로 나가고 mock sink 은 registry 에
    //    등록돼 있지 않다 → 거부 경로에서 mock 이 받는 control 은 Error 딱 1건이어야 한다.
    //    (성공 경로였다면 mock 에 Ack 가 enqueue 된다 — Ack 부재로 broadcast 분기 스킵을 방증.)
    #[tokio::test]
    async fn reparent_rejected_emits_error_no_ack_no_broadcast() {
        let (core, _rx) = test_core();
        // 실존 child 하나 등록(존재하지 않는 부모로 reparent → reparent==false).
        let child = engram_dashboard_core::agent::profile::AgentProfile::new(
            "c".into(),
            engram_dashboard_core::agent::profile::AgentCommand::Shell {
                program: default_shell().to_string(),
                args: vec![],
            },
            std::env::temp_dir(),
            vec![],
            false,
        );
        let cid = child.id;
        core.manager.profiles().upsert(child);

        let (tx, _rx2) = tokio::sync::mpsc::channel::<crate::ws::WsOutbound>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let req = rid();
        core.dispatch(
            AgentCommand::ReparentProfile {
                child_id: cid,
                parent_id: Some(uuid::Uuid::new_v4()), // 존재하지 않는 부모 → 거부.
                request_id: req,
            },
            &session,
            &mock,
        )
        .await;

        // control 은 Error 1건뿐(Ack 없음 = broadcast 분기 스킵).
        match mock.events().as_slice() {
            [AgentEvent::Error {
                request_id: Some(r),
                ..
            }] => assert_eq!(*r, req, "거부 Error 에 request_id 동봉"),
            other => panic!("거부 시 Error 1건만 기대(Ack/broadcast 없음): {other:?}"),
        }
        // 부작용 없음: child 의 parent_id 는 여전히 None(거부 = no-op).
        assert_eq!(
            core.manager.profiles().get(cid).unwrap().parent_id,
            None,
            "거부된 reparent 는 상태를 바꾸지 않아야 함"
        );
    }

    // ── Kill: 없는 agent → Error(request_id 동봉) ─────────────────────────────────
    #[tokio::test]
    async fn kill_unknown_agent_errors() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<crate::ws::WsOutbound>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        core.dispatch(
            AgentCommand::Kill {
                agent_id: uuid::Uuid::new_v4(),
                request_id: rid(),
            },
            &session,
            &mock,
        )
        .await;
        assert!(
            matches!(mock.events().as_slice(), [AgentEvent::Error { .. }]),
            "없는 agent kill 은 Error"
        );
    }

    // ── WriteStdin: lease 다른 conn 보유 시 거부 ──────────────────────────────────
    #[tokio::test]
    async fn write_stdin_denied_when_lease_held_by_other() {
        let (core, _rx) = test_core();
        let agent_id = uuid::Uuid::new_v4();
        // conn 2 가 lease 획득.
        let _ = core.multiview.acquire(agent_id, 2);
        // conn 1 이 write 시도 → Denied → Error(manager 호출 없이).
        let (tx, _rx2) = tokio::sync::mpsc::channel::<crate::ws::WsOutbound>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        core.dispatch(
            AgentCommand::WriteStdin {
                agent_id,
                data: b"x".to_vec(),
                request_id: rid(),
            },
            &session,
            &mock,
        )
        .await;
        match mock.events().as_slice() {
            [AgentEvent::Error { message, .. }] => {
                assert!(
                    message.contains("input locked"),
                    "lease 거부 메시지: {message}"
                )
            }
            other => panic!("Denied Error 기대: {other:?}"),
        }
    }

    // ── AcquireInput → InputLeaseChanged 브로드캐스트 + Ack ────────────────────────
    #[tokio::test]
    async fn acquire_input_acks_and_broadcasts() {
        let (core, _rx) = test_core();
        let agent_id = uuid::Uuid::new_v4();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<crate::ws::WsOutbound>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let req = rid();
        core.dispatch(
            AgentCommand::AcquireInput {
                agent_id,
                request_id: req,
            },
            &session,
            &mock,
        )
        .await;
        // requester 엔 Ack(request_id 동봉).
        match mock.events().as_slice() {
            [AgentEvent::Ack { request_id }] => assert_eq!(*request_id, req),
            other => panic!("Ack 기대: {other:?}"),
        }
        // 재획득(같은 conn)은 멱등 Ack.
        let (tx2, _r) = tokio::sync::mpsc::channel::<crate::ws::WsOutbound>(16);
        let mock2 = MockOutboundSink::new(tx2);
        core.dispatch(
            AgentCommand::AcquireInput {
                agent_id,
                request_id: rid(),
            },
            &session,
            &mock2,
        )
        .await;
        assert!(
            matches!(mock2.events().as_slice(), [AgentEvent::Ack { .. }]),
            "재획득은 멱등 Ack"
        );
    }

    // ── ListAgents → AgentList(request_id 동봉) ──────────────────────────────────
    #[tokio::test]
    async fn list_agents_returns_agent_list() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<crate::ws::WsOutbound>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let req = rid();
        core.dispatch(
            AgentCommand::ListAgents { request_id: req },
            &session,
            &mock,
        )
        .await;
        match mock.events().as_slice() {
            [AgentEvent::AgentList { request_id, .. }] => assert_eq!(*request_id, req),
            other => panic!("AgentList 기대: {other:?}"),
        }
    }

    // ── CreateProfile → Created(request_id 동봉) + 목록 변경 ───────────────────────
    #[tokio::test]
    async fn create_profile_returns_created() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<crate::ws::WsOutbound>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let req = rid();
        core.dispatch(
            AgentCommand::CreateProfile {
                name: "p".into(),
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                extra_args: vec![],
                env: vec![],
                auto_restore: false,
                output_format: WireClaudeOutputFormat::Terminal,
                request_id: req,
            },
            &session,
            &mock,
        )
        .await;
        match mock.events().as_slice() {
            [AgentEvent::Created { request_id, .. }] => assert_eq!(*request_id, req),
            other => panic!("Created 기대: {other:?}"),
        }
        assert_eq!(core.manager.profiles().list().len(), 1, "프로필 1개 등록");
    }

    // ── ADR-0044 M2: CreateProfile(output_format=StreamJson) → 저장 프로필이 json 모드 ──
    // wire output_format 이 저장 프로필의 core AgentCommand 로 옮겨져, is_json_mode 가 true 인지 확인한다.
    // 이게 참이면 이후 SpawnProfile → spawn_agent 가 StdioTransport(구조화 caps)를 고른다(M1 검증분).
    #[tokio::test]
    async fn create_profile_stream_json_stores_json_mode() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<crate::ws::WsOutbound>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        core.dispatch(
            AgentCommand::CreateProfile {
                name: "json".into(),
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                extra_args: vec![],
                env: vec![],
                auto_restore: false,
                output_format: WireClaudeOutputFormat::StreamJson,
                request_id: rid(),
            },
            &session,
            &mock,
        )
        .await;
        let profiles = core.manager.profiles().list();
        assert_eq!(profiles.len(), 1, "프로필 1개 등록");
        assert!(
            profiles[0].command.is_json_mode(),
            "StreamJson 으로 만든 프로필은 json 모드여야 함"
        );
    }

    // ── StopDaemon(force=false, 활성 0) → Ack + DispatchFlow::Close + watch true ──
    #[tokio::test]
    async fn stop_daemon_no_active_closes_and_signals() {
        let (core, mut rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<crate::ws::WsOutbound>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let flow = core
            .dispatch(
                AgentCommand::StopDaemon {
                    force: false,
                    kill_agents: true,
                    request_id: rid(),
                },
                &session,
                &mock,
            )
            .await;
        assert_eq!(flow, DispatchFlow::Close, "활성 0 → Close");
        assert!(
            matches!(mock.events().as_slice(), [AgentEvent::Ack { .. }]),
            "Ack 1건"
        );
        // watch 가 true 로 신호됐는지.
        assert!(rx.has_changed().unwrap_or(false));
        assert!(*rx.borrow_and_update());
    }

    // ── kind_to_action 매핑(3 variant 전수) ──────────────────────────────────────
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

    // ── core→wire AgentInfo 변환 roundtrip(serde 형태 일치) ────────────────────────
    //    (ws.rs 에서 이동 — 변환 함수가 이 모듈로 옮겨짐. 단언 무변경.)
    #[test]
    fn core_agent_info_converts_to_wire() {
        use engram_dashboard_core::agent::types::{
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
                    structured: false,
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

    // ── (M3) core::AgentStatus 모든 variant 가 wire 로 roundtrip 되는지 ────────────────
    //    어느 한 variant 라도 serde 태깅/필드가 어긋나면 core_agents_to_wire 가 그 agent 를
    //    silent drop 하므로 wire.len() < 1 이 되어 실패한다. status 값 자체도 wire 와 동일
    //    JSON tag 인지 직접 비교해 "변환은 됐지만 다른 variant 로 둔갑" 도 잡는다.
    #[test]
    fn all_core_status_variants_roundtrip_to_wire() {
        use engram_dashboard_core::agent::types::{
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
                structured: false,
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

    // ── (적용1) core::RestoreOutcome 전 variant → wire 명시 변환 ──────────────────────
    //    특히 FreshFallback 의 Uuid→String 변환을 명시 검증(옛 reflection 의 우연 호환 제거).
    #[test]
    fn all_restore_outcomes_convert_to_wire() {
        use engram_dashboard_core::agent::profile::RestoreOutcome as Co;
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

    // ── S15 B7: output_event_to_wire — core OutputEvent → wire StructuredEvent 필드 보존 ──────
    //    각 variant 를 명시 매핑하고 turn_id/message_id/id 등 optional 필드가 그대로(None 포함) 옮겨지는지.
    #[tokio::test]
    async fn output_event_to_wire_maps_all_variants_preserving_fields() {
        use engram_dashboard_protocol::StructuredEvent as W;

        // TextDelta — optional 필드 Some/None 혼합 보존.
        assert_eq!(
            output_event_to_wire(&CoreOutputEvent::TextDelta {
                text: "hi".into(),
                turn_id: Some("t1".into()),
                message_id: None,
            }),
            Some(W::TextDelta {
                text: "hi".into(),
                turn_id: Some("t1".into()),
                message_id: None,
            })
        );

        // ToolCall — id/turn_id/message_id 전부 보존.
        assert_eq!(
            output_event_to_wire(&CoreOutputEvent::ToolCall {
                name: "read".into(),
                args_json: r#"{"p":1}"#.into(),
                id: Some("c1".into()),
                turn_id: None,
                message_id: Some("m1".into()),
            }),
            Some(W::ToolCall {
                name: "read".into(),
                args_json: r#"{"p":1}"#.into(),
                id: Some("c1".into()),
                turn_id: None,
                message_id: Some("m1".into()),
            })
        );

        // Usage — 숫자 필드 보존.
        assert_eq!(
            output_event_to_wire(&CoreOutputEvent::Usage {
                input_tokens: 7,
                output_tokens: 11,
                turn_id: Some("t2".into()),
            }),
            Some(W::Usage {
                input_tokens: 7,
                output_tokens: 11,
                turn_id: Some("t2".into()),
            })
        );

        // MessageDone.
        assert_eq!(
            output_event_to_wire(&CoreOutputEvent::MessageDone {
                turn_id: Some("t3".into()),
                message_id: Some("m2".into()),
            }),
            Some(W::MessageDone {
                turn_id: Some("t3".into()),
                message_id: Some("m2".into()),
            })
        );

        // Error(String) → { message } 구조 변환.
        assert_eq!(
            output_event_to_wire(&CoreOutputEvent::Error("boom".into())),
            Some(W::Error {
                message: "boom".into()
            })
        );

        // Structured 탈출구 — kind/json 보존.
        assert_eq!(
            output_event_to_wire(&CoreOutputEvent::Structured {
                kind: "k".into(),
                json: r#"{"a":1}"#.into(),
            }),
            Some(W::Structured {
                kind: "k".into(),
                json: r#"{"a":1}"#.into(),
            })
        );

        // ★TerminalBytes 는 tag1 매핑 불가 → None(호출부 방어)★. 정상 경로상 tag0 로 갈려 여기 안 옴.
        assert_eq!(
            output_event_to_wire(&CoreOutputEvent::TerminalBytes(vec![1, 2, 3])),
            None,
            "TerminalBytes(tag0 전용)는 wire StructuredEvent 로 매핑 안 됨"
        );
    }
}
