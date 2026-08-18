//! transport-중립 연결 코어(ConnectionCore) — ADR-0020 Stage 1.
//!
//! carrier(WS/embedded/gRPC) 와 무관한 dispatch 를 소유한다. 입력은 `AgentCommand`, 출력은
//! `Outbound`(AgentEvent/binary/close)를 `OutboundSink` 로만 흘린다 — TcpStream/tungstenite/frame
//! codec 을 이 모듈은 모른다. 어댑터(`agent_conn::FrameOutboundSink`)가 `OutboundSink` 를 구현해
//! 이 코어를 구동한다(미래 carrier 는 sink 만 새로 구현).
//!
//! ★불변식(R1~R7, ADR-0020) 보존이 절대 원칙★:
//! - **R1 Ack→replay→ReplayComplete FIFO** — 유지 기전은 `handle_subscribe`.
//! - **R6 close_signal**: 큐 포화 out-of-band 종료는 WS-특정 → sink 구현(어댑터)이 SinkError
//!   해석으로 처리한다. 코어는 SinkError 만 본다(이 모듈은 close_signal 을 모른다).
//!
//! ★status fanout(broadcast)도 carrier-중립이다★: lease/profile 변경의 전-연결 브로드캐스트는
//! per-conn 응답(OutboundSink)이 아니라 `engram_dashboard_net::frame_port::FrameFanout`(전-연결
//! 출구)으로 간다 — 인코딩된 text 하나를 넘길 뿐이라 연결이 몇 개인지도, 누가 등록돼 있는지도
//! 이 모듈은 모른다(ADR-0129).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use engram_dashboard_core::agent::manager::RenameOutcome as CoreRenameOutcome;
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
    ClaudeOutputFormat as WireClaudeOutputFormat, CommandListEntry, ControlCaps as WireControlCaps,
    EnvelopeFormat as WireEnvelopeFormat, InputCaps as WireInputCaps, ModelCaps as WireModelCaps,
    OutputCaps as WireOutputCaps, Preset as WirePreset, RestartPolicy as WireRestartPolicy,
    RestoreOutcome as WireRestoreOutcome, RestoreReport, SessionCaps as WireSessionCaps,
    SnapshotChunk as WireSnapshotChunk, StructuredEvent as WireStructuredEvent, SubscribeAction,
    PROTOCOL_VERSION,
};

use tokio::sync::watch;

use crate::command_roster::CommandRoster;
use crate::control::registry::ControlRegistry;
use engram_dashboard_command::{CommandError, OwnerToken};
use engram_dashboard_messaging::envelope::EnvelopeFormat as CoreEnvelopeFormat;
use engram_dashboard_net::frame_port::{ConnId, FrameFanout};

// ── OutboundSink seam(ADR-0003 OutputSink 결을 따름) ──────────────────────────────

/// carrier-중립 송신 단위. 네트워크 행의 `Frame`(Text/Binary/Close)에 대응하되, control 은 아직
/// 인코딩 전 `AgentEvent` 라는 점이 다르다(인코딩은 어댑터가 소유).
///
/// ★Box<AgentEvent>★: AgentEvent 가 ~272B 라 다른 variant(24B)와 크기 차가 크다(clippy
/// large_enum_variant). control 경로는 hot path 가 아니므로(출력 binary 는 Binary variant) Box
/// 1회 할당이 무해하다. 생성은 `Outbound::event()` 헬퍼로 통일해 Box 를 숨긴다.
#[derive(Debug)]
pub enum Outbound {
    Event(Box<AgentEvent>),
    /// 이미 인코딩된 출력 frame 바이트(codec).
    Binary(Vec<u8>),
    Close(String),
}

impl Outbound {
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
pub trait OutboundSink: Send + Sync {
    fn enqueue(&self, out: Outbound) -> Result<(), SinkError>;

    /// 코어 `subscribe_from` 에 넘길 output sink + replay drop 플래그를 만든다.
    ///
    /// carrier(WS/embedded/gRPC)마다 인코딩이 달라(WS=binary frame, embedded=base64 PtyEvent) sink
    /// 구현이 다르므로, 반환을 trait object 로 두어 carrier-중립으로 만든다. 함께 반환하는
    /// `Arc<AtomicBool>` 은 replay 구간 중 frame drop(try_send full)이 있었는지다.
    fn make_output_sink(&self) -> (Arc<dyn OutputSink>, Arc<AtomicBool>);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchFlow {
    Continue,
    /// 연결 종료 — StopDaemon 수신에서만 난다.
    Close,
}

// ── per-conn 수명 상태(ConnectionSession) ─────────────────────────────────────────

/// 한 연결의 수명 상태. dispatch(`on_text`)와 정리(`on_disconnect`)가 공유하므로 내부 필드는
/// `Arc<Mutex<..>>` 다.
pub struct ConnectionSession {
    pub conn_id: ConnId,
    /// 이 연결이 등록한 (agent_id → sink_id) — cleanup 에서 누수 없이 unsubscribe.
    pub subs: Arc<Mutex<HashMap<AgentId, SinkId>>>,
    /// 이 연결이 등록한 (agent_id, viewport_id) 들 — cleanup 에서 viewport 협상 맵 정리.
    pub owned_viewports: Arc<Mutex<Vec<(AgentId, String)>>>,
    /// 남의 토큰 형식을 적은 등록을 이 연결에서 이미 한 번 남겼나(`note_claimed_owner`).
    claimed_owner_warned: AtomicBool,
    /// 배달 표 없이 온 명령 결말을 이 연결에서 이미 한 번 경고했나(아래 `CommandOutcome` arm).
    ///
    /// ★래치가 필요한 이유★: 이 프레임은 **클라이언트가 몇 번이든 보낼 수 있고** 릴리즈 데몬의 파일 sink 는
    /// 무조건 켜져 있다(`docs/reference/logging-conventions.md`). 프레임마다 `warn!` 이면 로그 크기가 상대에게
    /// 통제권을 넘긴다 — 형제 래치(`claimed_owner_warned`)와 같은 이유다.
    stray_outcome_warned: AtomicBool,
}

impl ConnectionSession {
    pub fn new(conn_id: ConnId) -> Self {
        Self {
            conn_id,
            subs: Arc::new(Mutex::new(HashMap::new())),
            owned_viewports: Arc::new(Mutex::new(Vec::new())),
            claimed_owner_warned: AtomicBool::new(false),
            stray_outcome_warned: AtomicBool::new(false),
        }
    }

    /// 이 연결이 명령 명부에서 갖는 주인 토큰.
    ///
    /// ★연결 id 에서 **파생**한다 — 등록 패킷의 `owner` 칸을 신뢰하지 않는다★: 그 칸을 그대로 쓰면 아무
    /// 연결이나 남의 토큰을 적어 그 이름들을 가져갈 수 있다(등록은 **살아 있는** 주인의 이름도 인수인계로
    /// 덮는다 — `Roster::register`). 신원이 봉투가 아니라 **그 봉투가 온 연결**이라는 계약은 이미 서 있고
    /// (TRD §4-⑧ · `CommandEnvelope::owner` 주석), TRD §3-7 은 데몬이 어느 쪽을 쓰는지를 열어 두었다.
    /// ★오늘은 연결마다 새 토큰이 나는데, 그것은 **설계가 아니라 잠정 상태**다★: 재연결이 새 `ConnId` 를
    /// 받으므로 데몬 눈에는 남남이고, 옛 연결의 등록은 끊길 때 지워지므로(ADR-0144 결정 3) 물려받을 것도
    /// 없다 — 이름의 인수인계는 명부의 last-wins 규칙이 한다.
    /// ★슬라이스 B 가 이 자리를 대체한다★ — 주인 키는 **클라이언트가 만들어 첫 인사에 실어 보낸 식별자**가
    /// 되고, 그 값은 **재접속을 가로질러 같다**(ADR-0144 결정 1·2 · TRD §3-7 조항 1·5). 명부의 last-wins 가
    /// 적힌 대로 「덮기」로 서는 것이 그 동일성에 달려 있다 — 매 연결이 남남인 지금은 같은 규칙이 덮기가
    /// 아니라 **쌓기**로 돌고, 그래서 자취를 버려야 했다(ADR-0144 맥락). 연결 id 파생은 그때 식별자를 안
    /// 보낸 연결의 fail-open 갈래로만 남는다(그 결정의 영향절).
    /// ★이 토큰은 wire 에 나가지 않는다★ — 그래서 클라이언트는 자기 토큰을 알 길이 없고, 등록 패킷의
    /// `owner` 칸이 데몬의 파생값과 다른 것은 위반이 아니라 **정상**이다(그래서 거절이 아니다 —
    /// `note_claimed_owner`). 그 칸은 식별자가 들어온 뒤에도 계속 무시한다 — 떼는 것도 계약 변경인데 얻는
    /// 것이 없다(ADR-0144 영향절).
    ///
    /// 파생 자체는 [`CommandRoster::owner_of`] 하나뿐이다 — 형식을 두 곳에 두지 않는다.
    ///
    /// ★이 값이 **권위는 아니다**★: 명부가 실제로 쓰는 주인 토큰은 `attach` 가 그때 계산해 표에 넣어 둔
    /// 값이고, 그것을 묻는 곳은 [`CommandRoster::attached_owner`] 다(ADR-0148). 오늘 둘이 같은 것은
    /// 「파생 지점이 하나여서」가 아니라 **`attach` 가 마침 같은 규칙으로 파생하기 때문**이다 — 슬라이스 B
    /// 가 `attach` 를 클라이언트 자작 식별자로 바꾸면 이 메서드는 그 값을 따라가지 못한다.
    /// ★그래서 운영 경로에는 이 메서드의 소비자가 없다★ — 클라이언트의 주장을 견주는
    /// `note_claimed_owner` 도 표의 저장값을 쓴다. 여기 남은 것은 위 정책 서술과, `attach` 가 **무엇을
    /// 저장하는지**를 무는 테스트의 기대값이다.
    // ADR-0144
    pub fn owner_token(&self) -> OwnerToken {
        CommandRoster::owner_of(self.conn_id)
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

#[derive(Clone, Default)]
pub struct MultiViewState {
    inner: Arc<Mutex<MultiViewInner>>,
}

#[derive(Default)]
struct MultiViewInner {
    /// agent_id → (viewport_id → (cols, rows)).
    viewports: HashMap<AgentId, HashMap<String, (u16, u16)>>,
    leases: HashMap<AgentId, ConnId>,
}

enum LeasePass {
    Allow,
    Denied,
}

impl MultiViewState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 반환 None = 등록된 viewport 가 없음(이론상 방금 넣었으므로 항상 Some).
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

    /// 반환: (agent_id, Some(min) = 남은 viewport 의 smallest / None = 이제 viewport 없음).
    pub fn remove_conn_viewports(
        &self,
        owned: &[(AgentId, String)],
    ) -> Vec<(AgentId, Option<(u16, u16)>)> {
        let mut g = self.inner.lock().expect("multiview poisoned");
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

    /// Ok(true)=새로 획득(상태 변경), Ok(false)=이미 이 conn 보유(멱등), Err=다른 conn 이 보유 중.
    fn acquire(&self, agent_id: AgentId, conn_id: ConnId) -> Result<bool, ()> {
        let mut g = self.inner.lock().expect("multiview poisoned");
        match g.leases.get(&agent_id) {
            None => {
                g.leases.insert(agent_id, conn_id);
                Ok(true)
            }
            Some(&holder) if holder == conn_id => Ok(false),
            Some(_) => Err(()),
        }
    }

    /// Ok(true)=해제됨(상태 변경), Ok(false)=원래 비어 있었음, Err=다른 conn 이 보유 중
    /// (보유자만 해제 가능).
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

    fn check_input(&self, agent_id: AgentId, conn_id: ConnId) -> LeasePass {
        let g = self.inner.lock().expect("multiview poisoned");
        match g.leases.get(&agent_id) {
            None => LeasePass::Allow,
            Some(&holder) if holder == conn_id => LeasePass::Allow,
            Some(_) => LeasePass::Denied,
        }
    }

    /// 좀비 lock 방지. 반환된 agent 들은 이제 lease 가 비었으므로 InputLeaseChanged{held:false} 를
    /// 브로드캐스트할 대상이다.
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
// ★reflection 왕복 금지(되살리지 말 것)★: serde_json::to_value→from_value 로 core↔wire 를 돌리면
// 한쪽 필드/태그가 어긋나도 컴파일은 통과하고 런타임에 silent drop(None) 된다. 그래서 필드를 하나씩
// 명시 매핑한다 — core 에 필드가 추가/개명되면 **컴파일 에러**가 나게.
//
// 변환은 데몬 crate 에 둔다(core 는 protocol 무의존 유지 — §1 불변). orphan rule 때문에 외부 두
// 타입 사이 `impl From` 은 불가하나, 데몬이 양쪽을 다 의존하므로 자유 함수로 직접 필드 접근한다.

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

fn spawn_command_to_wire(cmd: &CoreSpawnCommand) -> WireSpawnCommand {
    match cmd {
        CoreSpawnCommand::Claude {
            extra_args,
            output_format,
        } => WireSpawnCommand::Claude {
            extra_args: extra_args.clone(),
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

fn restart_policy_to_wire(p: CoreRestartPolicy) -> WireRestartPolicy {
    match p {
        CoreRestartPolicy::Never => WireRestartPolicy::Never,
        CoreRestartPolicy::OnCrash => WireRestartPolicy::OnCrash,
        CoreRestartPolicy::Always => WireRestartPolicy::Always,
    }
}

fn profile_to_wire(p: &CoreProfile) -> WireProfile {
    WireProfile {
        id: p.id,
        name: p.name.clone(),
        display_name: p.display_name.clone(),
        // core AgentId 와 wire ProfileId 는 같은 Uuid alias 라 변환 없이 복사한다.
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

fn snapshot_chunk_to_wire(c: &CoreOutputChunk) -> WireSnapshotChunk {
    WireSnapshotChunk {
        seq: c.seq,
        data: c.data.clone(),
    }
}

/// ★반환이 Option 인 이유(TerminalBytes 방어)★: 정상 경로에서 `OutputEvent::TerminalBytes` 는 이 변환에
/// **오지 않는다** — 콘솔 raw 바이트는 sink 에서 tag0 terminal frame(`OutputPayload::Bytes`)으로 갈리고,
/// 이 함수는 `OutputPayload::Event` arm(tag1)에서만 불린다. wire `StructuredEvent` 에는 TerminalBytes
/// variant 가 없으므로(tag1 payload 에 raw 바이트를 안 싣는다 — ADR-0045), 만약 TerminalBytes 가 이 arm 에
/// 도달하면 매핑 불가다. 그때 패닉 대신 `None` 을 돌린다 — tag0/tag1 오분류는 상류 배선 버그지 이 frame
/// 하나로 연결을 죽일 사안이 아니다(호출부 처리는 `agent_conn::FrameOutputSink::send`).
pub(crate) fn output_event_to_wire(ev: &CoreOutputEvent) -> Option<WireStructuredEvent> {
    match ev {
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
        CoreOutputEvent::Error(message) => Some(WireStructuredEvent::Error {
            message: message.clone(),
        }),
        CoreOutputEvent::Structured { kind, json } => Some(WireStructuredEvent::Structured {
            kind: kind.clone(),
            json: json.clone(),
        }),
    }
}

pub(crate) fn core_report_to_wire(report: CoreRestoreReport) -> RestoreReport {
    RestoreReport {
        agent_id: report.agent_id,
        epoch: report.epoch,
        outcome: restore_outcome_to_wire(&report.outcome),
    }
}

pub(crate) fn core_status_to_wire(status: CoreStatus) -> engram_dashboard_protocol::AgentStatus {
    status_to_wire(&status)
}

/// None = 직렬화 실패(이 함수가 이미 로그를 남겼다).
pub(crate) fn event_json(ev: &AgentEvent) -> Option<String> {
    match serde_json::to_string(ev) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::error!("AgentEvent 직렬화 실패: {e}");
            None
        }
    }
}

fn kind_to_action(kind: ReplayKind) -> SubscribeAction {
    match kind {
        ReplayKind::FromOldest => SubscribeAction::Reset,
        ReplayKind::Truncated => SubscribeAction::TruncatedReplay,
        ReplayKind::Resumed => SubscribeAction::Resume,
    }
}

// ── ConnectionCore ────────────────────────────────────────────────────────────────

/// ★이 struct 는 **연결마다 새로 만들어진다**(`agent_conn::AgentConnections::handler_for`)★. 서버 전체에
/// 하나인 것은 그 공장이고, 여기 든 **필드들이** 전 연결이 공유하는 핸들의 clone 이다 — 아래 6개
/// (manager · multiview · fanout · control_registry · messaging · shutdown_tx)가 전부 그렇다.
/// ★그래서 새 필드를 넣을 때 반드시 확인할 것★: 공유 핸들이 아닌 값을 여기 넣으면 "서버 전체 1개" 로
/// 읽히는 자리에 조용히 **연결마다 별개**인 상태가 생긴다. 연결 고유 상태의 자리는 `ConnectionSession`
/// (dispatch 에 주입)이다.
pub struct ConnectionCore {
    manager: Arc<AgentManager>,
    multiview: MultiViewState,
    // ADR-0129
    fanout: Arc<dyn FrameFanout>,
    // ADR-0096
    control_registry: Arc<ControlRegistry>,
    /// 빈 슬롯 = 이 조립(실험 bin·일부 테스트)엔 메시징이 아예 없다는 뜻 — `DeleteProfile` 의 삭제
    ///   정리를 건너뛰어도 정리할 것이 없다.
    // ADR-0116
    messaging: Arc<crate::control::mcp_server::MessagingSlot>,
    // ADR-0140
    commands: CommandRoster,
    shutdown_tx: watch::Sender<bool>,
}

impl ConnectionCore {
    pub fn new(
        manager: Arc<AgentManager>,
        multiview: MultiViewState,
        fanout: Arc<dyn FrameFanout>,
        control_registry: Arc<ControlRegistry>,
        messaging: Arc<crate::control::mcp_server::MessagingSlot>,
        commands: CommandRoster,
        shutdown_tx: watch::Sender<bool>,
    ) -> Self {
        Self {
            manager,
            multiview,
            fanout,
            control_registry,
            messaging,
            commands,
            shutdown_tx,
        }
    }

    pub fn multiview(&self) -> &MultiViewState {
        &self.multiview
    }

    pub fn commands(&self) -> &CommandRoster {
        &self.commands
    }

    pub fn fanout(&self) -> &dyn FrameFanout {
        self.fanout.as_ref()
    }

    pub fn manager(&self) -> &Arc<AgentManager> {
        &self.manager
    }

    /// ★sink.enqueue 실패(SinkError)는 무시★: side-effect 명령의 Ack/Error 송신 실패는 삼킨다.
    /// 어댑터의 enqueue 는 논블록(try_send)이라 큐 포화면 backpressure 로 기다리는 게 아니라 즉시
    /// drop + close 신호다 — 정상 단일 연결은 큐 여유로 control drop 이 안 나고, 포화 시 종착점
    /// (연결 종료)은 어느 쪽이든 같다.
    pub async fn dispatch(
        &self,
        cmd: AgentCommand,
        session: &ConnectionSession,
        sink: &dyn OutboundSink,
    ) -> DispatchFlow {
        use engram_dashboard_protocol::RequestId;

        let manager = &self.manager;
        let multiview = &self.multiview;
        let fanout = self.fanout.as_ref();
        let conn_id = session.conn_id;
        let subs = &session.subs;
        let owned_viewports = &session.owned_viewports;

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

        /// 명부 조작의 실패를 답장으로 만든다 — **코드까지 실어 보낸다**.
        ///
        /// `Error` 이벤트엔 타입드 칸이 없어 `CommandError` 의 Display(`CODE: 문구`)로 나간다. 코드가
        /// 빠지면 호출자는 문구를 패턴매칭해 재시도 여부를 정해야 하고, 그 취약함이 타입드 오류 모델이
        /// 없애려던 것이다(TRD §4-⑦). 실패를 답장 없이 흘리는 것은 더 나쁘다 — 보낸 쪽은 `request_id` 의
        /// 답을 기다리므로 연결이 끊길 때까지 안 깨는 pending 이 된다.
        ///
        /// ★거절은 서버에도 남긴다★: 이름 도둑질을 막은 그물이 발동한 자리인데, 답장만으로는 **그 짓을 한
        /// 쪽**에만 보이고 운영자에겐 아무 흔적이 없다. 정상 경로는 조용하다(등록은 연결마다 한 번이라
        /// 거절만 올려도 시끄럽지 않다).
        /// ★문구는 클라이언트 문자열을 인용한다★ — 명부가 길이를 이미 묶어 두지만(`MAX_NAME_BYTES`)
        /// 모양은 안 묶으므로 `sanitize_for_log` 를 거쳐 나간다(거기 적힌 위조 항목 문제).
        fn reply_roster(
            sink: &dyn OutboundSink,
            conn_id: ConnId,
            verb: &'static str,
            request_id: RequestId,
            result: Result<(), CommandError>,
        ) {
            if let Err(e) = &result {
                tracing::warn!(
                    conn = conn_id,
                    verb,
                    code = %e.code(),
                    "명령 명부 거절: {}",
                    sanitize_for_log(e.message())
                );
            }
            reply(sink, request_id, result.map_err(|e| e.to_string()));
        }

        match cmd {
            AgentCommand::Spawn {
                profile_id,
                request_id,
            } => {
                let result = match manager.agent_snapshot(profile_id) {
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
                let target = match viewport_id {
                    Some(v) => {
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
                    // viewport_id 없음(v1 프론트) = 협상 우회, 그 크기로 직접 resize(하위호환).
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
                self.handle_subscribe(agent_id, epoch, after_seq, subs, sink);
            }

            AgentCommand::Unsubscribe { agent_id } => {
                let sink_id = subs.lock().expect("subs poisoned").remove(&agent_id);
                if let Some(sid) = sink_id {
                    let _ = manager.unsubscribe(agent_id, sid);
                }
            }

            AgentCommand::AcquireInput {
                agent_id,
                request_id,
            } => match multiview.acquire(agent_id, conn_id) {
                Ok(true) => {
                    broadcast_lease_changed(fanout, agent_id, true);
                    reply(sink, request_id, Ok(()));
                }
                Ok(false) => reply(sink, request_id, Ok(())),
                Err(()) => reply(
                    sink,
                    request_id,
                    Err("input held by another viewer".to_string()),
                ),
            },

            AgentCommand::ReleaseInput {
                agent_id,
                request_id,
            } => match multiview.release(agent_id, conn_id) {
                Ok(true) => {
                    broadcast_lease_changed(fanout, agent_id, false);
                    reply(sink, request_id, Ok(()));
                }
                Ok(false) => reply(sink, request_id, Ok(())),
                Err(()) => reply(
                    sink,
                    request_id,
                    Err("input lease held by another viewer".to_string()),
                ),
            },

            AgentCommand::ListAgents { request_id } => {
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
                // force=false 거부는 실수로 데몬을 내려 살아있는 PTY 세션을 모두 죽이는 사고를 막는다.
                // ★실활성만 카운트★: 이미 죽은(Exited/Killed/Failed)·종료중(Exiting) 세션은 제외한다 —
                //   이들 때문에 거부하면 살릴 게 없는데도 데몬을 못 내리는 오작동이 된다.
                let active_count = manager
                    .list_agents()
                    .iter()
                    .filter(|a| matches!(a.status, CoreStatus::Running))
                    .count();
                if !force && active_count > 0 {
                    send_error(
                        sink,
                        Some(request_id),
                        format!(
                            "active agents present ({active_count}); use force=true to stop the daemon"
                        ),
                    );
                    return DispatchFlow::Continue;
                }

                // ★kill_agents 는 v1 에서 무시(always-kill)★: 자식 PTY 가 데몬의 KILL_ON_JOB_CLOSE
                //   Job 에 담기므로 데몬이 죽으면 자식도 **무조건** 함께 죽는다 — detach(데몬만 내리고
                //   자식 유지)가 현 Job 모델에선 불가능하다. 플래그는 미래 detach 여지로 protocol 에
                //   남겨두되 v1 동작은 값과 무관하다.
                let _ = kill_agents;
                let mgr = manager.clone();
                let _ = tokio::task::spawn_blocking(move || mgr.shutdown_all()).await;

                reply(sink, request_id, Ok(()));
                // main 종료 트리거(watch). 수신측은 run() 의 select! 가 감지.
                let _ = self.shutdown_tx.send(true);
                return DispatchFlow::Close;
            }

            // ── 프로필 CRUD + ad-hoc spawn(phase4 1단계) ───────────────────────────────
            AgentCommand::SpawnByCwd { cwd, request_id } => {
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
                let _ = sink.enqueue(Outbound::event(AgentEvent::ProfileList {
                    request_id,
                    profiles: core_profiles_to_wire(manager.agent_snapshots()),
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
                match manager.create_agent(profile) {
                    Ok(stored) => {
                        let wire = profile_to_wire(&stored);
                        // Created 하나로 응답한다 — Ack 는 보내지 않는다(중복 resolve 방지).
                        let _ = sink.enqueue(Outbound::event(AgentEvent::Created {
                            request_id,
                            profile: wire,
                        }));
                        broadcast_profile_list(fanout, manager);
                    }
                    Err(e) => reply(sink, request_id, Err(e.to_string())),
                }
            }

            AgentCommand::DeleteProfile {
                profile_id,
                request_id,
            } => {
                // ★삭제 정리 훅(ADR-0116 결정 3)★: 여기가 유일한 프로필 제거 지점이라 훅도 여기 하나다.
                //   호출자 의무(이름 파생 시점·락 순서·게이트 축)는 `handle_profile_deleted` 가 정본이다.
                // ★발동 조건(로스터 부재)은 커널이 판정한다★: 로스터는 커널의 DeliveryPort 소유 축이고, 여기서
                //   미리 보면 조건이 두 곳에 갈린다(그 판정이 곧 정책 — spec §5).
                let deleted_name = manager
                    .agent_snapshot(profile_id)
                    .map(|p| p.canonical_name_when_live());
                manager.delete_agent(profile_id);
                reply(sink, request_id, Ok(()));
                broadcast_profile_list(fanout, manager);
                if let (Some(name), Some(messaging)) = (deleted_name, self.messaging.get()) {
                    // 이 경로엔 자식 stdin blocking write 가 없다(큐 정리 + 장부 전이 + 짧은 로스터 스냅샷) —
                    //   그래서 spawn_blocking 없이 이 async 컨텍스트에서 그대로 돈다(ingress 조회 경로와 동형).
                    let out = messaging.handle_profile_deleted(profile_id, &name);
                    tracing::debug!(
                        profile = %profile_id,
                        name = %name,
                        skipped_live = out.skipped_live,
                        parked_failed = out.failed_parked,
                        contracts_failed = out.failed_contracts,
                        "프로필 삭제 — 메시징 삭제 정리 훅 실행(ADR-0116 결정 3)"
                    );
                }
            }

            AgentCommand::SpawnProfile {
                profile_id,
                resume,
                request_id,
            } => {
                // ★모드 = 세션 존재 여부로 유도(ADR-0076)★: 사용자 결정 — "에이전트 활성화 = 기존 세션
                //   이어받기, 새로 로드할 거면 새 에이전트를 만든다". 그래서 저장된 세션이 있으면 wire
                //   `resume` 플래그(프론트는 false 로 보낸다)와 무관하게 **항상 Resume** 이다.
                //   ★resume=true 는 존중★: 세션이 없어도 Resume 로 남긴다 — spawn_agent(Resume)가
                //     ensure_session_id 로 최초 sid 를 발급하므로 안전하다. 즉 mode = resume-요청 OR 세션-존재.
                match manager.agent_snapshot(profile_id) {
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
                let ok = manager.set_agent_auto_restore(profile_id, auto_restore);
                if ok {
                    reply(sink, request_id, Ok(()));
                    broadcast_profile_list(fanout, manager);
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
            } => match manager.rename_agent(profile_id, name) {
                CoreRenameOutcome::Renamed(_) | CoreRenameOutcome::Unchanged(_) => {
                    reply(sink, request_id, Ok(()));
                    broadcast_profile_list(fanout, manager);
                }
                CoreRenameOutcome::NotFound => reply(
                    sink,
                    request_id,
                    Err(format!("profile not found: {profile_id}")),
                ),
                CoreRenameOutcome::Exhausted => reply(
                    sink,
                    request_id,
                    Err(format!(
                        "name suffix space exhausted — cannot assign a unique name: {profile_id}"
                    )),
                ),
            },

            AgentCommand::ReparentProfile {
                child_id,
                parent_id,
                request_id,
            } => {
                let ok = manager.reparent_agent(child_id, parent_id);
                if ok {
                    reply(sink, request_id, Ok(()));
                    broadcast_profile_list(fanout, manager);
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
                // Snapshot 하나로 응답한다 — 별도 Ack 는 보내지 않는다(중복 resolve 방지).
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

            // ── 프리셋 CRUD(ADR-0061) ──────────────────────────────────────────────
            AgentCommand::ListPresets { request_id } => {
                let _ = sink.enqueue(Outbound::event(AgentEvent::PresetList {
                    request_id,
                    presets: core_presets_to_wire(manager.presets().list()),
                }));
            }

            AgentCommand::CreatePreset { cwd, request_id } => {
                manager.presets().create(std::path::PathBuf::from(cwd));
                reply(sink, request_id, Ok(()));
                broadcast_preset_list(fanout, manager);
            }

            AgentCommand::DeletePreset {
                preset_id,
                request_id,
            } => {
                manager.presets().remove(preset_id);
                reply(sink, request_id, Ok(()));
                broadcast_preset_list(fanout, manager);
            }

            AgentCommand::RenamePreset {
                preset_id,
                name,
                request_id,
            } => {
                manager.presets().rename(preset_id, name);
                reply(sink, request_id, Ok(()));
                broadcast_preset_list(fanout, manager);
            }

            AgentCommand::SetEnvelopeFormat { format, request_id } => {
                // broadcast 하지 않는다 — 전역 상태는 다음 메시지에서 관측되지 목록 push 대상이 아니다.
                let core_format = match format {
                    WireEnvelopeFormat::Colon => CoreEnvelopeFormat::Colon,
                    WireEnvelopeFormat::Xml => CoreEnvelopeFormat::Xml,
                };
                self.control_registry.set_envelope_format(core_format);
                reply(sink, request_id, Ok(()));
            }

            // ── 명령 버스 등록 wire(ADR-0140/0141, TRD §3-7) ──────────────────────────────
            //
            // ★`_ =>` 로 묶지 않는 이유★: 이 match 가 exhaustive 라서 variant 를 늘릴 때마다 여기가
            //   컴파일 에러로 걸린다. 그게 「배선을 빠뜨리지 않았나」를 묻는 유일한 지점이라 catch-all 로
            //   덮으면 다음 variant 는 아무 신호 없이 조용히 무시된다.
            AgentCommand::RegisterCommands {
                owner: claimed,
                decls,
                catalog_version,
                request_id,
            } => {
                // ★`attached_owner` 를 `session.owner_token()` 으로 되돌리지 마라 — 저장소 어느 테스트도
                //   안 잡는다★: 오늘은 둘이 같은 값이라 두 구현이 같은 로그를 내고, 둘이 갈리는 상태
                //   (저장값 ≠ 파생값)를 이 층에서는 만들 수 없다 — 명부의 표는 `command_roster` 사설이다.
                //   되돌리면 슬라이스 B 에서 사칭이 조용히 통과한다(인과 = `note_claimed_owner` 주석,
                //   판정 자체를 무는 것 = 그 함수를 직접 부르는 세 테스트).
                // ADR-0148
                note_claimed_owner(
                    session,
                    &claimed,
                    self.commands.attached_owner(conn_id).as_ref(),
                );
                // ★`catalog_version` 으로 거절하지 않는다★: 세대 번호는 crate 마다라 받는 쪽이 자기
                //   번호와 비교해 뜻을 부여하면 틀린다 — 진단용이다(TRD §4-①).
                tracing::debug!(
                    conn = conn_id,
                    catalog_version,
                    names = decls.len(),
                    "명령 등록"
                );
                reply_roster(
                    sink,
                    conn_id,
                    "register",
                    request_id,
                    self.commands.register(conn_id, decls),
                );
            }

            AgentCommand::UpdateCommands {
                owner: claimed,
                added,
                removed,
                request_id,
            } => {
                // ★`attached_owner` 를 `session.owner_token()` 으로 되돌리지 마라 — 저장소 어느 테스트도
                //   안 잡는다★: 오늘은 둘이 같은 값이라 두 구현이 같은 로그를 내고, 둘이 갈리는 상태
                //   (저장값 ≠ 파생값)를 이 층에서는 만들 수 없다 — 명부의 표는 `command_roster` 사설이다.
                //   되돌리면 슬라이스 B 에서 사칭이 조용히 통과한다(인과 = `note_claimed_owner` 주석,
                //   판정 자체를 무는 것 = 그 함수를 직접 부르는 세 테스트).
                // ADR-0148
                note_claimed_owner(
                    session,
                    &claimed,
                    self.commands.attached_owner(conn_id).as_ref(),
                );
                tracing::debug!(
                    conn = conn_id,
                    added = added.len(),
                    removed = removed.len(),
                    "명령 차분"
                );
                reply_roster(
                    sink,
                    conn_id,
                    "update",
                    request_id,
                    self.commands.update(conn_id, added, removed),
                );
            }

            AgentCommand::ListCommands { request_id } => {
                // 주인 토큰은 안 내린다 — 선언처가 주인이라 등급 칸이 없어졌다(TRD §3-7 개정 ㉠).
                let entries: Vec<CommandListEntry> = self
                    .commands
                    .entries()
                    .into_iter()
                    .map(|entry| CommandListEntry {
                        name: entry.name,
                        help: entry.help,
                        // 명부에 있는 것은 주인이 있는 이름뿐이다 — 계약이 이 칸을 왜 남겼는지는
                        //   `CommandListEntry` 주석.
                        // ADR-0144
                        available: true,
                    })
                    .collect();
                let _ = sink.enqueue(Outbound::event(AgentEvent::CommandList {
                    request_id,
                    entries,
                }));
            }

            // ★결말을 붙일 왕복이 아직 없다 — 그래도 **침묵하지 않는다**★: 데몬이 명령을 **배달**하는 다리
            //   (`request_id` → 원 연결 라우팅 표)는 미구현이라, 지금 이 프레임이 오는 경로는 미솔리시트뿐이다.
            //   그렇다고 조용히 버리면 보낸 쪽은 답도 오류도 없이 자기 마감시각을 다 쓴다 — 이 파일이 다른
            //   자리에서 이미 적어 둔 그 손실(`reply` 헬퍼 주석)이다.
            // ★그래서 형제 위반 경로와 **같은 답**을 낸다 — `Error{request_id: None}`★. 상관 없는 오류라
            //   라우팅 표가 필요 없고(`agent_conn.rs` 의 2차 핸드셰이크·파싱 실패와 동형), 그 형태가 「이
            //   데몬은 그 명령을 배달하지 않는다」를 상대에게 말하는 유일한 수단이다.
            // ★`Ack` 는 못 낸다★: 그건 「전달했다」는 뜻이 되고, 상관도 안 된다(보낸 쪽은 이 명령으로 pending
            //   슬롯을 만들지 않는다 — protocol `command_request_id` 가 이 variant 에 `None`).
            // 배달 다리가 서면 이 자리가 그 표를 보고 **원 연결로 중계**하는 곳이 된다.
            AgentCommand::CommandOutcome { reply } => {
                // 첫 건만 warn — 나머지는 debug(래치 근거는 `stray_outcome_warned` doc).
                if !session.stray_outcome_warned.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        conn = conn_id,
                        request_id = %reply.request_id,
                        "명령 결말이 왔으나 이 데몬은 명령을 배달하지 않는다 — 거절(이 연결에서 한 번만 남긴다)"
                    );
                } else {
                    tracing::debug!(
                        conn = conn_id,
                        request_id = %reply.request_id,
                        "명령 결말 거절(반복)"
                    );
                }
                let _ = sink.enqueue(Outbound::event(AgentEvent::Error {
                    request_id: None,
                    message: "this daemon does not relay commands, so it has nothing to attach this outcome to".into(),
                }));
            }
        }
        DispatchFlow::Continue
    }

    /// ★get_snapshot 으로 SubscribeAck 를 예측하지 말 것★: 예측이 뜬 스냅샷과 subscribe_from 이 실제로
    /// replay 하는 스냅샷 사이에 evict 가 끼면 Ack.replay_from/latest 가 첫 전송 seq 와 어긋나 클라가
    /// 손실을 인지하지 못한다. Ack 의 모든 필드는 subscribe_from 의 **단일 스냅샷 outcome** 으로 채운다.
    ///
    /// ★두 평면★: 코어 subscribe_from 에 넘기는 sink(`agent_conn::FrameOutputSink`)는 출력 frame
    /// 평면이고, control(Ack/ReplayComplete/Error)만 dispatch 의 `OutboundSink` 로 나간다. 둘 다
    /// **같은 프레임 출구**(연결당 단일 writer 큐)로 합류하므로 FIFO 가 보존된다 — R1 이 여기 걸려 있다.
    fn handle_subscribe(
        &self,
        agent_id: AgentId,
        requested_epoch: Option<u32>,
        after_seq: Option<u64>,
        subs: &Arc<Mutex<HashMap<AgentId, SinkId>>>,
        sink: &dyn OutboundSink,
    ) {
        let manager = &self.manager;

        // agent 가 없으면 subscribe_from 을 부르지 않으므로 Ack 도 나가지 않는다.
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
        let epoch_matches = requested_epoch == Some(current_epoch);

        let (out_sink, replay_dropped) = sink.make_output_sink();

        // enqueue 실패를 삼키는 이유: control 은 작아 보통 성공하고, 큐가 full 이면 어차피 같은 큐를
        //   쓰는 replay 도 막혀 truncated 로 잡힌다.
        let on_ready = |outcome: &SubscribeOutcome| {
            let _ = sink.enqueue(Outbound::event(AgentEvent::SubscribeAck {
                agent_id,
                action: kind_to_action(outcome.kind),
                current_epoch,
                oldest_seq: outcome.oldest_seq,
                latest_seq: outcome.latest_seq,
                replay_from: outcome.replay_from,
                truncated: outcome.kind == ReplayKind::Truncated,
            }));
        };

        let outcome =
            match manager.subscribe_from(agent_id, out_sink, after_seq, epoch_matches, on_ready) {
                Ok(o) => o,
                Err(e) => {
                    send_error(sink, None, format!("subscribe failed: {e}"));
                    return;
                }
            };

        let old = subs
            .lock()
            .expect("subs poisoned")
            .insert(agent_id, outcome.sink_id);
        if let Some(old_sid) = old {
            if old_sid != outcome.sink_id {
                let _ = manager.unsubscribe(agent_id, old_sid);
            }
        }

        // Ack 에는 kind 기반 truncated 가 이미 나갔다. 여기서 더 통보하는 것은 kind!=Truncated 인데
        //   replay 동기 전송 중 실제 drop 이 난 경우뿐이다.
        if outcome.kind != ReplayKind::Truncated && replay_dropped.load(Ordering::Acquire) {
            send_error(
                sink,
                None,
                format!("replay truncated for agent {agent_id}: output dropped during replay; please refresh"),
            );
        }

        let _ = sink.enqueue(Outbound::event(AgentEvent::ReplayComplete {
            agent_id,
            epoch: current_epoch,
        }));
    }
}

/// 등록 패킷이 적어 온 주인 토큰은 **광고**일 뿐 권한이 아니다 — 명부의 주인은 그 연결이 붙을 때 명부가
/// 저장한 값(`actual`)이고, 그 출처는 [`CommandRoster::attached_owner`] 하나다.
///
/// ★`actual` 을 여기서 파생하지 않는 것이 이 함수의 정확성을 좌우한다★: 파생값과 견주면, 저장값이 파생을
/// 떠나는 날 **정직한 클라이언트가 걸리고 사칭이 통과한다** — 자기 저장 토큰을 그대로 적어 온 연결은
/// 「다르다」로 갈라지고, 옛 파생형(`conn-<id>`)을 적어 온 연결은 아래 첫 갈래에서 조용히 빠져나간다.
/// 그 형식을 잡자고 세운 갈래가 정확히 그것을 못 잡게 된다(ADR-0148).
/// ★`None`(안 붙은 연결) 이면 견줄 값이 없다★ — 어떤 주장도 「같다」로 접지 않고 아래 형식 갈래로 보낸다.
/// 그 패킷은 어차피 명부가 반려하지만(`CommandRoster::register`), 그 상태에서 우리 형식을 적어 온 것은
/// 오히려 더 또렷한 사칭 시도다.
///
/// ★거절하지 않는 이유★: 연결 id 는 wire 에 나가지 않아 클라이언트가 자기 토큰을 알 수 없다
/// (`owner_token` 주석). 같지 않다고 거절하면 정상 등록이 **채울 수 없는 칸** 때문에 전부 막힌다.
/// ★그래서 어긋남 자체는 기본 채널로 올리지 않는다★: 위 이유로 **정상 등록도 어긋난다.** 등록마다
/// warn 을 한 줄씩 쌓으면 기본 레벨(warn)에서 진짜 경고가 그 밑에 묻힌다.
/// ★한 갈래만 올린다 — **우리 토큰 형식**을 적어 온 경우★: 그 형식은 데몬 안에서만 나므로(`conn-<id>`),
/// 남의 연결 앞으로 이름을 얹으려는 시도가 아니고서는 클라이언트가 그 모양을 적을 이유가 없다.
/// ★빈 칸은 아무것도 남기지 않는다★: `owner` 는 필수 칸이라 빠질 수는 없고, 빈 문자열은 「채울 값을
/// 모른다」는 뜻이다 — 주장이 아니므로 보고할 것도 없다.
/// ★그 갈래도 연결당 한 줄뿐이다★: 패킷은 클라이언트가 원하는 만큼 밀 수 있고 그 속도는 우리 손에
/// 없다. 같은 연결의 두 번째 줄은 운영자에게 새 사실을 주지 않으면서 기본 채널만 채운다.
/// ★한 줄 제한을 debug 갈래와 **같은 표식으로 묶지 않는다**★ — 묶으면 무해한 패킷 하나를 먼저 보내는
/// 것만으로 뒤따르는 경고를 잠글 수 있다.
/// ★찍는 길이를 자른다★: 이 칸은 검증 안 된 클라이언트 문자열이고 프레임 크기 상한이 없어, 통째로
/// 찍으면 프레임 하나가 메가바이트짜리 로그 줄이 된다.
// ADR-0140
fn note_claimed_owner(
    session: &ConnectionSession,
    claimed: &OwnerToken,
    actual: Option<&OwnerToken>,
) {
    if claimed.as_str().is_empty() || Some(claimed) == actual {
        return;
    }
    let conn_id = session.conn_id;
    if claimed
        .as_str()
        .starts_with(CommandRoster::OWNER_TOKEN_PREFIX)
    {
        if !session.claimed_owner_warned.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                conn = conn_id,
                claimed = %sanitize_for_log(claimed.as_str()),
                "다른 연결의 주인 토큰을 적은 등록 — 무시하고 이 연결의 토큰으로 얹는다(이 연결에서 한 번만 남긴다)"
            );
        }
    } else {
        tracing::debug!(
            conn = conn_id,
            claimed = %sanitize_for_log(claimed.as_str()),
            "등록 패킷의 주인 토큰은 쓰지 않는다"
        );
    }
}

/// 로그에 실을 **클라이언트가 준 문자열**을 다듬는다 — 길이와 **모양** 둘 다.
///
/// ★길이★: wire 에 프레임 크기 상한이 없어 자르지 않으면 로그 줄 하나가 프레임만큼 커진다. 문자
/// 경계로 자른다(바이트로 자르면 멀티바이트가 쪼개진다).
/// ★모양★: 제어문자를 이스케이프한다. 로그 레이어는 Display 값을 **그대로** 쓰므로, 이름에 개행을 넣은
/// 등록 하나가 줄을 둘로 쪼개고 뒤 줄이 우리가 찍은 것처럼 보이는 **위조 항목**이 된다(타임스탬프·레벨을
/// 흉내 낸 문자열을 그 자리에 앉힐 수 있다).
/// 알려진 범위: `char::is_control`(C0/C1)까지다 — U+2028 같은 유니코드 줄 구분자는 그대로 나가고, 지금
/// 쓰는 fmt 레이어는 그것으로 줄을 쪼개지 않는다.
fn sanitize_for_log(text: &str) -> String {
    const MAX_CHARS: usize = 64;
    let mut out = String::new();
    for ch in text.chars().take(MAX_CHARS) {
        if ch.is_control() {
            out.extend(ch.escape_debug());
        } else {
            out.push(ch);
        }
    }
    if text.chars().nth(MAX_CHARS).is_some() {
        out.push('…');
    }
    out
}

/// 팬아웃 포트가 논블록 구현을 요구하므로 pump/cleanup 등 어느 컨텍스트에서 불려도 안전하다.
pub(crate) fn broadcast_lease_changed(fanout: &dyn FrameFanout, agent_id: AgentId, held: bool) {
    let ev = AgentEvent::InputLeaseChanged { agent_id, held };
    if let Some(text) = event_json(&ev) {
        fanout.broadcast_text(text);
    }
}

/// `control/agent` 라우트가 부르는 명부 통지 포트의 실물 어댑터(ADR-0132).
///
/// ★왜 여기 사는가★: 통지의 내용물(어떤 이벤트를 어떤 wire 모양으로 미는가)은 이 모듈이 소유한 지식이고,
///   `control/` 은 "명부가 바뀌었다" 만 안다. 반대로 두면 제어 라우트가 wire 매핑을 알게 되고, 데몬 층
///   결정(ADR-0130)이 추적하는 `control/` 의 나가는 간선이 생긴다.
/// ★짝 불일치 방지★: 생성자를 통해 **한 조립에서 나온** 팬아웃과 매니저만 묶인다 —
///   `DaemonWiring::roster_broadcast` 가 유일한 운영 생성 지점이다(그 struct 주석의 규칙).
pub struct RosterFanout {
    fanout: Arc<dyn FrameFanout>,
    manager: Arc<AgentManager>,
}

impl RosterFanout {
    pub fn new(fanout: Arc<dyn FrameFanout>, manager: Arc<AgentManager>) -> Self {
        Self { fanout, manager }
    }
}

impl crate::control::agent::RosterBroadcast for RosterFanout {
    fn roster_changed(&self) {
        broadcast_profile_list(self.fanout.as_ref(), &self.manager);
    }
}

fn broadcast_profile_list(fanout: &dyn FrameFanout, manager: &Arc<AgentManager>) {
    let ev = AgentEvent::ProfileListUpdated {
        profiles: core_profiles_to_wire(manager.agent_snapshots()),
    };
    if let Some(text) = event_json(&ev) {
        fanout.broadcast_text(text);
    }
}

/// ★create/delete 는 반드시 이 broadcast 로 이어진다★(안 그러면 다른 창이 stale — ADR-0061 불변식).
fn broadcast_preset_list(fanout: &dyn FrameFanout, manager: &Arc<AgentManager>) {
    let ev = AgentEvent::PresetListUpdated {
        presets: core_presets_to_wire(manager.presets().list()),
    };
    if let Some(text) = event_json(&ev) {
        fanout.broadcast_text(text);
    }
}

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

pub fn hello_event(daemon_version: String) -> AgentEvent {
    AgentEvent::Hello {
        protocol_version: PROTOCOL_VERSION,
        daemon_version,
        capabilities: None,
    }
}

pub fn agent_list_event(manager: &Arc<AgentManager>) -> AgentEvent {
    AgentEvent::AgentListUpdated {
        agents: core_agents_to_wire(manager.list_agents()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_dashboard_net::frame_port;
    use engram_dashboard_protocol::RequestId;
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;

    fn rid() -> RequestId {
        RequestId(uuid::Uuid::new_v4())
    }

    /// ★output 평면은 mock 으로 안 만든다★ — 실 프레임 출구(conn_tx)로 흘려 별도 채널로 받는다
    /// (그래야 control 과 같은 단일 writer 큐에 실려 FIFO 순서를 관측할 수 있다).
    struct MockOutboundSink {
        events: Arc<StdMutex<Vec<AgentEvent>>>,
        conn_tx: tokio::sync::mpsc::Sender<frame_port::Frame>,
    }

    impl MockOutboundSink {
        fn new(conn_tx: tokio::sync::mpsc::Sender<frame_port::Frame>) -> Self {
            Self {
                events: Arc::new(StdMutex::new(Vec::new())),
                conn_tx,
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
                    if let Some(text) = event_json(&ev) {
                        let _ = self.conn_tx.try_send(frame_port::Frame::Text(text));
                    }
                    self.events.lock().unwrap().push(*ev);
                    Ok(())
                }
                Outbound::Binary(b) => {
                    let _ = self.conn_tx.try_send(frame_port::Frame::Binary(b));
                    Ok(())
                }
                Outbound::Close(r) => {
                    let _ = self.conn_tx.try_send(frame_port::Frame::Close(r));
                    Ok(())
                }
            }
        }
        fn make_output_sink(&self) -> (Arc<dyn OutputSink>, Arc<AtomicBool>) {
            let sink = Arc::new(crate::agent_conn::FrameOutputSink::new(Arc::new(
                crate::test_doubles::FakeFrameSink::new(self.conn_tx.clone()),
            )));
            let flag = sink.replay_dropped_flag();
            (sink, flag)
        }
    }

    /// ★flush 트리거는 꽂지 않는다(의도)★: 운영은 로스터 diff(MessagingFlushSink)가 등장 시 파킹을 비우지만,
    ///   여기서는 "스폰 후에도 파킹분이 남아 있는" 상태가 필요하다(삭제 정리가 그걸 죽이는지가 검증 대상).
    ///   그 diff 배선 자체는 messaging_host.rs·control_send 테스트가 지킨다.
    fn test_core_with_messaging() -> (
        ConnectionCore,
        Arc<engram_dashboard_messaging::service::MessagingService>,
    ) {
        let (core, _rx) = test_core();
        let messaging = Arc::new(crate::messaging_host::messaging_for_manager(
            core.manager.clone(),
            core.control_registry.clone(),
        ));
        core.messaging.set(messaging.clone());
        (core, messaging)
    }

    fn test_core() -> (ConnectionCore, watch::Receiver<bool>) {
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

        let fanout: Arc<dyn FrameFanout> = Arc::new(crate::test_doubles::RecordingFanout::new());
        let store: Arc<dyn ProfileStore> = Arc::new(MemStore::default());
        let preset_store: Arc<dyn PresetStore> = Arc::new(MemPresetStore::default());
        let status_sink = Arc::new(crate::status_fanout::DaemonStatusSink::new(fanout.clone()));
        let profiles = Arc::new(ProfileRegistry::new(store));
        let presets = Arc::new(PresetRegistry::new(preset_store));
        let tracker = Arc::new(SessionTracker::new(
            TrackerConfig::default(),
            Arc::new(|_aid, _sid| {}),
        ));
        let manager = Arc::new(AgentManager::new(status_sink, profiles, presets, tracker));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let control_registry = Arc::new(ControlRegistry::new());
        let core = ConnectionCore::new(
            manager,
            MultiViewState::new(),
            fanout,
            control_registry,
            Arc::new(crate::control::mcp_server::MessagingSlot::new()),
            CommandRoster::new(),
            shutdown_tx,
        );
        (core, shutdown_rx)
    }

    fn test_core_with_control_registry() -> (ConnectionCore, Arc<ControlRegistry>) {
        let (core, _rx) = test_core();
        let control_registry = core.control_registry.clone();
        (core, control_registry)
    }

    // ── R1: Subscribe ────────────────────────────────────────────────────────────
    #[tokio::test]
    async fn subscribe_emits_ack_then_replay_then_complete_in_order() {
        let (core, _rx) = test_core();
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

        let (tx, mut conn_rx) = tokio::sync::mpsc::channel::<frame_port::Frame>(4608);
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

        let mut items = Vec::new();
        while let Ok(item) = conn_rx.try_recv() {
            items.push(item);
        }
        assert!(items.len() >= 2, "최소 Ack+ReplayComplete: {}", items.len());
        match &items[0] {
            frame_port::Frame::Text(s) => {
                assert!(s.contains("SubscribeAck"), "1번째는 SubscribeAck: {s}")
            }
            other => panic!("1번째는 Text(SubscribeAck) 여야 함: {other:?}"),
        }
        match items.last().unwrap() {
            frame_port::Frame::Text(s) => {
                assert!(s.contains("ReplayComplete"), "마지막은 ReplayComplete: {s}")
            }
            other => panic!("마지막은 Text(ReplayComplete) 여야 함: {other:?}"),
        }
        for mid in &items[1..items.len() - 1] {
            assert!(
                matches!(mid, frame_port::Frame::Binary(_)),
                "Ack 와 ReplayComplete 사이엔 replay Binary 만: {mid:?}"
            );
        }

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

    // ── Subscribe: 없는 agent ─────────────────────────────────────────────────────
    #[tokio::test]
    async fn subscribe_unknown_agent_emits_error_no_ack() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
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

    // ── Spawn: 없는 profile ──────────────────────────────────────────────────────
    #[tokio::test]
    async fn spawn_missing_profile_errors() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
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

    // ── ReparentProfile: 거부(false) (ADR-0072) ──────────────────────────────────
    //    broadcast_profile_list 는 팬아웃 포트로 나가고 mock sink 은 그 포트 뒤에 없다.
    //    (성공 경로였다면 mock 에 Ack 가 enqueue 된다 — Ack 부재로 broadcast 분기 스킵을 방증.)
    #[tokio::test]
    async fn reparent_rejected_emits_error_no_ack_no_broadcast() {
        let (core, _rx) = test_core();
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
        core.manager.create_agent(child).expect("등록 성공");

        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
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

        match mock.events().as_slice() {
            [AgentEvent::Error {
                request_id: Some(r),
                ..
            }] => assert_eq!(*r, req, "거부 Error 에 request_id 동봉"),
            other => panic!("거부 시 Error 1건만 기대(Ack/broadcast 없음): {other:?}"),
        }
        assert_eq!(
            core.manager.agent_snapshot(cid).unwrap().parent_id,
            None,
            "거부된 reparent 는 상태를 바꾸지 않아야 함"
        );
    }

    // ── Kill: 없는 agent ─────────────────────────────────────────────────────────
    #[tokio::test]
    async fn kill_unknown_agent_errors() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
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

    // ── WriteStdin: lease 다른 conn 보유 ─────────────────────────────────────────
    #[tokio::test]
    async fn write_stdin_denied_when_lease_held_by_other() {
        let (core, _rx) = test_core();
        let agent_id = uuid::Uuid::new_v4();
        let _ = core.multiview.acquire(agent_id, 2);
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
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

    // ── AcquireInput ─────────────────────────────────────────────────────────────
    #[tokio::test]
    async fn acquire_input_acks_and_broadcasts() {
        let (core, _rx) = test_core();
        let agent_id = uuid::Uuid::new_v4();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
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
        match mock.events().as_slice() {
            [AgentEvent::Ack { request_id }] => assert_eq!(*request_id, req),
            other => panic!("Ack 기대: {other:?}"),
        }
        let (tx2, _r) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
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

    // ── ListAgents ───────────────────────────────────────────────────────────────
    #[tokio::test]
    async fn list_agents_returns_agent_list() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
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

    // ── 명령 버스 등록 wire(ADR-0140/0141 · TRD §3-7) ────────────────────────────
    use engram_dashboard_command::{CommandDecl, ErrorCode, Roster};

    fn decl(name: &str) -> CommandDecl {
        CommandDecl {
            name: name.to_string(),
            help: format!("{{\"name\":\"{name}\"}}"),
        }
    }

    /// `note_claimed_owner` 가 낸 이벤트를 **레벨과 함께** 모은다 — 이 파일엔 로그 수집기가 없어 여기 둔다.
    /// `with_default` 는 이 스레드에만 걸려 병렬 테스트와 섞이지 않는다(`command_roster` 의 같은 형태).
    ///
    /// ★DEBUG 까지 켜는 이유★: 「조용하다」를 WARN 만 보고 판정하면 **debug 갈래로 떨어진 것**과 **아무
    /// 갈래에도 안 간 것**이 같아 보인다.
    fn capture_logs(body: impl FnOnce()) -> Vec<(tracing::Level, String)> {
        use tracing::subscriber;

        struct Collector {
            lines: Arc<StdMutex<Vec<(tracing::Level, String)>>>,
        }
        struct Visit<'a>(&'a mut String);
        impl tracing::field::Visit for Visit<'_> {
            fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
                self.0.push_str(&format!("{}={:?} ", f.name(), v));
            }
        }
        impl subscriber::Subscriber for Collector {
            fn enabled(&self, m: &tracing::Metadata<'_>) -> bool {
                *m.level() <= tracing::Level::DEBUG
            }
            fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::Id {
                tracing::Id::from_u64(1)
            }
            fn record(&self, _: &tracing::Id, _: &tracing::span::Record<'_>) {}
            fn record_follows_from(&self, _: &tracing::Id, _: &tracing::Id) {}
            fn event(&self, event: &tracing::Event<'_>) {
                let mut buf = String::new();
                event.record(&mut Visit(&mut buf));
                self.lines
                    .lock()
                    .expect("lines poisoned")
                    .push((*event.metadata().level(), buf));
            }
            fn enter(&self, _: &tracing::Id) {}
            fn exit(&self, _: &tracing::Id) {}
        }

        let lines: Arc<StdMutex<Vec<(tracing::Level, String)>>> = Arc::default();
        subscriber::with_default(
            Collector {
                lines: lines.clone(),
            },
            body,
        );
        let captured = lines.lock().expect("lines poisoned");
        captured.clone()
    }

    // ── 주장한 주인 토큰의 판정(ADR-0148) ────────────────────────────────────────
    //
    // ★셋 다 **명부의 저장값과 견주는가**를 문다★ — 세션에서 다시 파생하는 구현은 셋 다 뒤집힌다.
    // 저장값을 인자로 받으므로 그 상태를 여기서 그냥 만들 수 있다(명부도 `attach` 도 안 건드린다).
    // ★판정을 전부 **warn callsite** 로 몬 것은 의도다★: 이 자리를 때리는 테스트가 여기 셋뿐이라
    // 다른 테스트가 tracing 의 전역 callsite 관심 캐시를 먼저 눌러 놓을 길이 없다. debug 갈래는
    // 그렇지 않아(등록 wire 테스트들이 구독자 없이 그 자리를 때린다) 판정 근거로 쓰지 않는다.

    /// 사칭 — 우리 형식을 적어 왔고 진짜 주인은 그게 아니다. 재파생 구현은 `conn-7` 이 자기 파생값과
    /// 같아 보여 **조용히 통과시킨다**.
    #[test]
    fn a_claim_of_our_token_shape_is_warned_when_the_real_owner_is_not_derived() {
        let session = ConnectionSession::new(7);
        let actual = OwnerToken::new("shell-a1");

        let logged = capture_logs(|| {
            note_claimed_owner(&session, &OwnerToken::new("conn-7"), Some(&actual))
        });

        assert_eq!(logged.len(), 1, "한 줄이어야: {logged:?}");
        assert_eq!(logged[0].0, tracing::Level::WARN, "사칭은 warn: {logged:?}");
        assert!(
            session.claimed_owner_warned.load(Ordering::Relaxed),
            "연결당 1회 래치가 서야 한다"
        );
    }

    /// 정직 — 자기 저장 토큰을 그대로 적어 왔다. 재파생 구현은 그것이 자기 파생값과 달라 **정직한 쪽을
    /// 사칭으로 경고한다**.
    ///
    /// 저장 토큰을 우리 형식(`conn-9`)으로 잡은 것은 그 뒤집힘이 warn 으로 나오게 하려는 것이다 —
    /// `shell-a1` 같은 모양이면 뒤집힘이 debug 로 떨어져 위 「판정 근거」 조건을 못 채운다.
    #[test]
    fn a_claim_that_matches_the_real_owner_is_silent() {
        let session = ConnectionSession::new(7);
        let actual = OwnerToken::new("conn-9");

        let logged = capture_logs(|| note_claimed_owner(&session, &actual, Some(&actual)));

        assert!(logged.is_empty(), "진짜 주인과 같으면 조용하다: {logged:?}");
    }

    /// 안 붙은 연결 — 견줄 값이 아예 없다. 재파생 구현은 여기서도 `conn-7` 을 만들어 내 통과시킨다.
    #[test]
    fn a_claim_of_our_token_shape_is_warned_when_nothing_is_attached() {
        let session = ConnectionSession::new(7);

        let logged =
            capture_logs(|| note_claimed_owner(&session, &OwnerToken::new("conn-7"), None));

        assert_eq!(logged.len(), 1, "한 줄이어야: {logged:?}");
        assert_eq!(logged[0].0, tracing::Level::WARN, "{logged:?}");
    }

    /// ★주인 칸에 파생값이 **아닌** 토큰을 일부러 싣는다★ — 명부의 주인이 패킷이 아니라 연결에서
    /// 난다는 것이 이 wire 의 계약이다(`ConnectionSession::owner_token`).
    fn register(decls: Vec<CommandDecl>, request_id: RequestId) -> AgentCommand {
        AgentCommand::RegisterCommands {
            owner: OwnerToken::new("whatever-the-client-thinks"),
            decls,
            catalog_version: 7,
            request_id,
        }
    }

    fn update(added: Vec<CommandDecl>, removed: Vec<&str>, request_id: RequestId) -> AgentCommand {
        AgentCommand::UpdateCommands {
            owner: OwnerToken::new("whatever-the-client-thinks"),
            added,
            removed: removed.into_iter().map(str::to_string).collect(),
            request_id,
        }
    }

    /// 연결이 선 상태의 세션 — 명부는 **붙어 있는 연결**의 등록만 받으므로, 명단에 올리는 이 한 줄이
    /// 운영의 `on_connect` 자리다(`CommandRoster::attach`).
    fn attached(core: &ConnectionCore, conn_id: ConnId) -> ConnectionSession {
        // 닿는 길은 여기 관심사가 아니라 자리만 채운다 — 그 표를 재는 것은 `command_roster` 쪽이다.
        // ★받는 쪽이 이 함수와 함께 죽는다★: 여기 든 출구로는 아무것도 못 나간다. 이 파일이 언젠가
        //   **배달**을 단언하려 들면 그 단언은 조용히 빈손이 되므로, 그때는 받는 쪽을 호출자에게
        //   돌려주도록 이 헬퍼를 고쳐야 한다.
        let (tx, _rx) = tokio::sync::mpsc::channel::<frame_port::Frame>(1);
        let frames: Arc<dyn frame_port::FrameSink> =
            Arc::new(crate::test_doubles::FakeFrameSink::new(tx));
        core.commands().attach(conn_id, &frames);
        ConnectionSession::new(conn_id)
    }

    #[tokio::test]
    async fn register_commands_acks_and_fills_the_roster() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = attached(&core, 1);
        let req = rid();

        core.dispatch(register(vec![decl("tab.create")], req), &session, &mock)
            .await;

        match mock.events().as_slice() {
            [AgentEvent::Ack { request_id }] => assert_eq!(*request_id, req),
            other => panic!("등록은 Ack 로 답한다: {other:?}"),
        }
        let entries = core.commands().entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "tab.create");
        assert_eq!(
            entries[0].owner,
            session.owner_token(),
            "주인은 연결에서 파생한다 — 패킷이 적어 온 토큰이 아니다"
        );
    }

    #[tokio::test]
    async fn update_commands_acks_and_applies_the_delta() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = attached(&core, 1);
        core.dispatch(
            register(vec![decl("tab.create"), decl("tab.close")], rid()),
            &session,
            &mock,
        )
        .await;

        let req = rid();
        core.dispatch(
            update(vec![decl("tab.split")], vec!["tab.close"], req),
            &session,
            &mock,
        )
        .await;

        match mock.events().as_slice() {
            [_, AgentEvent::Ack { request_id }] => assert_eq!(*request_id, req),
            other => panic!("차분도 Ack 로 답한다: {other:?}"),
        }
        let names: Vec<String> = core
            .commands()
            .entries()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(
            names,
            vec!["tab.create".to_string(), "tab.split".to_string()],
            "내린 이름은 자리째 사라지고(ADR-0144) 차분에 안 실린 이름은 그대로다"
        );
    }

    /// ★`help` 는 데몬에게 불투명 문자열 한 칸이다★ — JSON 이 아닌 값을 넣어도 등록·조회가 그대로
    /// 성공해야 한다. 데몬이 파싱·검증을 끼우면 여기서 깨진다(TRD §3-7 하드 제약 · §7 seam 표).
    #[tokio::test]
    async fn list_commands_returns_the_roster_projection_with_the_help_bytes_intact() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = attached(&core, 1);
        let opaque = "not json at all — 임의의 바이트 {[(";
        core.dispatch(
            register(
                vec![CommandDecl {
                    name: "tab.create".to_string(),
                    help: opaque.to_string(),
                }],
                rid(),
            ),
            &session,
            &mock,
        )
        .await;

        let req = rid();
        core.dispatch(
            AgentCommand::ListCommands { request_id: req },
            &session,
            &mock,
        )
        .await;

        match mock.events().as_slice() {
            [_, AgentEvent::CommandList {
                request_id,
                entries,
            }] => {
                assert_eq!(*request_id, req);
                assert_eq!(
                    entries,
                    &vec![CommandListEntry {
                        name: "tab.create".to_string(),
                        help: opaque.to_string(),
                        available: true,
                    }]
                );
            }
            other => panic!("조회는 CommandList 로 답한다: {other:?}"),
        }
    }

    /// 정리 뒤에 내려앉는 등록(겹침의 근거 = `CommandRoster` 헤더). 갈래별 단언은 `command_roster` 쪽에
    /// 있고, **여기서 보는 것은 dispatch 가 그 판정에 연결 id 를 넘기는지 하나**다 — 안 넘기면 이 반려가
    /// 아예 서지 않는다.
    #[tokio::test]
    async fn a_registration_dispatched_after_the_disconnect_is_refused() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = attached(&core, 1);
        core.commands().detach(1);
        let req = rid();

        core.dispatch(register(vec![decl("tab.create")], req), &session, &mock)
            .await;

        match mock.events().as_slice() {
            [AgentEvent::Error {
                request_id,
                message,
            }] => {
                assert_eq!(*request_id, Some(req));
                assert!(
                    message.starts_with(ErrorCode::Conflict.as_str()),
                    "끊긴 연결의 등록은 명부 **상태** 거절이다: {message}"
                );
            }
            other => panic!("거절은 Error 로 답한다: {other:?}"),
        }
        assert!(
            core.commands().entries().is_empty(),
            "이름이 하나도 얹히면 안 된다"
        );
    }

    /// ★실패를 삼키지 않는다 — 코드까지 실어 보낸다★: 코드가 없으면 부르는 쪽이 문구를 패턴매칭해
    /// 재시도 여부를 정해야 한다(TRD §4-⑦).
    #[tokio::test]
    async fn a_refused_registration_replies_with_the_error_code() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = attached(&core, 1);
        let req = rid();
        let too_long = CommandDecl {
            name: "n".repeat(Roster::MAX_NAME_BYTES + 1),
            help: "{}".to_string(),
        };

        core.dispatch(register(vec![too_long], req), &session, &mock)
            .await;

        match mock.events().as_slice() {
            [AgentEvent::Error {
                request_id,
                message,
            }] => {
                assert_eq!(*request_id, Some(req));
                assert!(
                    message.starts_with(ErrorCode::InvalidArgument.as_str()),
                    "코드가 문구 앞에 실린다: {message}"
                );
            }
            other => panic!("거절은 Error 로 답한다: {other:?}"),
        }
        assert!(
            core.commands().entries().is_empty(),
            "실패한 등록은 명부를 건드리지 않는다"
        );
    }

    /// 연결마다 주인이 갈리므로 남의 산 등록은 못 가져간다 — 두 세션이 **같은 명부**를 본다는 것도 함께
    /// 단언한다(공유가 끊기면 conn 2 는 빈 명부를 보고 성공한다).
    #[tokio::test]
    async fn an_update_from_another_connection_cannot_take_a_live_name() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let first = attached(&core, 1);
        let second = attached(&core, 2);
        let mock = MockOutboundSink::new(tx.clone());
        core.dispatch(register(vec![decl("tab.create")], rid()), &first, &mock)
            .await;

        let intruder = MockOutboundSink::new(tx);
        let req = rid();
        core.dispatch(
            update(vec![decl("tab.create")], vec![], req),
            &second,
            &intruder,
        )
        .await;

        match intruder.events().as_slice() {
            [AgentEvent::Error {
                request_id,
                message,
            }] => {
                assert_eq!(*request_id, Some(req));
                assert!(
                    message.starts_with(ErrorCode::Conflict.as_str()),
                    "산 남의 등록을 뺏는 것은 명부 **상태** 거절이다: {message}"
                );
            }
            other => panic!("거절은 Error 로 답한다: {other:?}"),
        }
        let entries = core.commands().entries();
        assert_eq!(
            entries.len(),
            1,
            "거절이 남의 이름을 지워 놓고 끝나면 산 명령이 조용히 사라진다"
        );
        assert_eq!(
            entries[0].owner,
            first.owner_token(),
            "먼저 붙은 연결이 그대로 주인이다"
        );
    }

    // ── ADR-0096: SetEnvelopeFormat ──────────────────────────────────────────────
    #[tokio::test]
    async fn set_envelope_format_acks_and_mutates_send_path_state() {
        let (core, control_registry) = test_core_with_control_registry();
        assert_eq!(
            control_registry.envelope_format(),
            CoreEnvelopeFormat::Xml,
            "초기 봉투 포맷은 xml(기본, ADR-0103)"
        );
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
        let mock = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let req = rid();
        core.dispatch(
            AgentCommand::SetEnvelopeFormat {
                format: WireEnvelopeFormat::Colon,
                request_id: req,
            },
            &session,
            &mock,
        )
        .await;
        match mock.events().as_slice() {
            [AgentEvent::Ack { request_id }] => assert_eq!(*request_id, req),
            other => panic!("SetEnvelopeFormat 는 Ack 를 돌려줘야: {other:?}"),
        }
        assert_eq!(
            control_registry.envelope_format(),
            CoreEnvelopeFormat::Colon,
            "dispatch 후 봉투 포맷 전역 상태가 colon 으로 바뀌어야(send path 가 이 값을 읽음)"
        );

        core.dispatch(
            AgentCommand::SetEnvelopeFormat {
                format: WireEnvelopeFormat::Xml,
                request_id: rid(),
            },
            &session,
            &mock,
        )
        .await;
        assert_eq!(
            control_registry.envelope_format(),
            CoreEnvelopeFormat::Xml,
            "xml 재전환도 반영"
        );
    }

    // ── CreateProfile ────────────────────────────────────────────────────────────
    #[tokio::test]
    async fn create_profile_returns_created() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
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
        assert_eq!(core.manager.agent_snapshots().len(), 1, "프로필 1개 등록");
    }

    // ── ADR-0044 M2: CreateProfile(output_format=StreamJson) ─────────────────────
    #[tokio::test]
    async fn create_profile_stream_json_stores_json_mode() {
        let (core, _rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
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
        let profiles = core.manager.agent_snapshots();
        assert_eq!(profiles.len(), 1, "프로필 1개 등록");
        assert!(
            profiles[0].command.is_json_mode(),
            "StreamJson 으로 만든 프로필은 json 모드여야 함"
        );
    }

    // ── StopDaemon(force=false, 활성 0) ──────────────────────────────────────────
    #[tokio::test]
    async fn stop_daemon_no_active_closes_and_signals() {
        let (core, mut rx) = test_core();
        let (tx, _rx2) = tokio::sync::mpsc::channel::<frame_port::Frame>(16);
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

    // ── core→wire AgentInfo 변환 roundtrip ───────────────────────────────────────
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
            reads_messages: true,
        };
        let wire = core_agents_to_wire(vec![core.clone()]);
        assert_eq!(wire.len(), 1, "변환 성공(JSON 형태 일치)");
        assert_eq!(wire[0].name, "t");
        assert_eq!(wire[0].epoch, 3);
    }

    // ── (M3) core::AgentStatus 모든 variant 가 wire 로 roundtrip 되는지 ────────────────
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

        // (core status, 기대 wire status) 쌍 — variant 전수(6 케이스).
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
                reads_messages: true,
            };
            let wire = core_agents_to_wire(vec![core]);
            assert_eq!(
                wire.len(),
                1,
                "variant {core_status:?} 가 core→wire 에서 drop 됨(태깅/필드 불일치)"
            );
            assert_eq!(
                wire[0].status, expected_wire,
                "variant {core_status:?} 가 다른 wire status 로 변환됨"
            );
            let direct = core_status_to_wire(core_status.clone());
            assert_eq!(direct, expected_wire, "직접 변환 경로도 일치해야 함");
        }
    }

    // ── (적용1) core::RestoreOutcome 전 variant → wire 명시 변환 ──────────────────────
    #[test]
    fn all_restore_outcomes_convert_to_wire() {
        use engram_dashboard_core::agent::profile::RestoreOutcome as Co;
        use engram_dashboard_protocol::RestoreOutcome as Wo;

        let old = uuid::Uuid::new_v4();
        let new = uuid::Uuid::new_v4();

        assert_eq!(restore_outcome_to_wire(&Co::Resumed), Wo::Resumed);
        assert_eq!(restore_outcome_to_wire(&Co::Started), Wo::Started);

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

        match restore_outcome_to_wire(&Co::FreshFallback {
            old_sid: None,
            new_sid: new,
            reason: "r2".into(),
        }) {
            Wo::FreshFallback { old_sid, .. } => assert_eq!(old_sid, None, "None 보존"),
            other => panic!("FreshFallback 기대, got {other:?}"),
        }

        assert_eq!(
            restore_outcome_to_wire(&Co::Blocked { reason: "b".into() }),
            Wo::Blocked { reason: "b".into() }
        );
        assert_eq!(
            restore_outcome_to_wire(&Co::Failed { reason: "f".into() }),
            Wo::Failed { reason: "f".into() }
        );
    }

    // ── S15 B7: output_event_to_wire ─────────────────────────────────────────────
    #[tokio::test]
    async fn output_event_to_wire_maps_all_variants_preserving_fields() {
        use engram_dashboard_protocol::StructuredEvent as W;

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

        assert_eq!(
            output_event_to_wire(&CoreOutputEvent::Error("boom".into())),
            Some(W::Error {
                message: "boom".into()
            })
        );

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

        assert_eq!(
            output_event_to_wire(&CoreOutputEvent::TerminalBytes(vec![1, 2, 3])),
            None,
            "TerminalBytes(tag0 전용)는 wire StructuredEvent 로 매핑 안 됨"
        );
    }

    // ══════════════════════════════════════════════════════════════════════════════════════════
    // 리뷰 fix D1 — `DeleteProfile` 삭제 정리 훅의 **배선** 통합 테스트(ADR-0116 결정 3)
    // ══════════════════════════════════════════════════════════════════════════════════════════

    /// ★왜 데몬 통합인가(가짜 포트 단위 테스트가 원리적으로 못 잡는다)★: 이 결함은 **두 배선**의 순서·축에
    /// 있었다 — ① 정리 대상 이름을 프로필 **제거 전에** 뽑나 ② 발동 게이트를 **id** 로 거나. 커널 단위
    /// 테스트는 그 둘을 하네스가 대신 정해 주므로(이름·게이트를 테스트가 직접 스크립트한다) 배선이 뒤바뀐 걸
    /// 볼 수 없다. 여기서는 실제 프로필·실제 spawn·실물 `ManagerDeliveryPort` 로 dispatch 를 태운다.
    #[tokio::test]
    async fn deleting_a_renamed_profile_whose_session_is_live_keeps_its_mail_and_contracts() {
        use engram_dashboard_messaging::envelope::Entrance;
        use engram_dashboard_messaging::service::SendMeta;
        use engram_dashboard_messaging::SenderIdentity;

        let (core, messaging) = test_core_with_messaging();
        let (tx, _rx) = tokio::sync::mpsc::channel::<frame_port::Frame>(64);
        let sink = MockOutboundSink::new(tx);
        let session = ConnectionSession::new(1);

        // (1) 프로필 2개 — boss(곧 산 세션이 된다) · sleepy(계속 잠들어 있다 = 파킹 수신자).
        let mk = |name: &str| {
            engram_dashboard_core::agent::profile::AgentProfile::new(
                name.into(),
                engram_dashboard_core::agent::profile::AgentCommand::Shell {
                    program: default_shell().to_string(),
                    args: vec![],
                },
                std::env::temp_dir(),
                vec![],
                false,
            )
        };
        let boss = mk("boss-raw");
        let sleepy = {
            let mut p = mk("sleepy-raw");
            p.display_name = Some("sleepy".into());
            p
        };
        core.manager.create_agent(boss.clone()).expect("등록 성공");
        core.manager
            .create_agent(sleepy.clone())
            .expect("등록 성공");

        // (2) ★개명(RenameProfile)★
        core.dispatch(
            AgentCommand::RenameProfile {
                profile_id: boss.id,
                name: Some("boss".into()),
                request_id: engram_dashboard_protocol::RequestId(uuid::Uuid::new_v4()),
            },
            &session,
            &sink,
        )
        .await;
        assert_eq!(
            core.manager
                .agent_snapshot(boss.id)
                .expect("프로필")
                .canonical_name_when_live(),
            "boss",
            "개명이 canonical 이름을 바꿨다(이 등식이 이 테스트의 전제)"
        );

        // (3) boss 가 잠든 동안: ⓐ boss 앞으로 파킹 1건 ⓑ boss 가 **요청자**인 계약 1건.
        let sender = SenderIdentity {
            peer_id: uuid::Uuid::new_v4(),
            epoch: 0,
        };
        let req = SendMeta {
            request: true,
            reply_by: None,
            reply_by_raw: None,
            reply_to: None,
            to_attr: None,
        };
        let rows = messaging
            .handle_send(
                "m-in",
                sender,
                "outsider",
                &["boss".to_string()],
                "쌓아둔다",
                Entrance::Cli,
                &SendMeta::default(),
            )
            .expect("행 응답");
        assert_eq!(
            rows[0].status,
            engram_dashboard_messaging::service::SendStatus::Pending,
            "잠든 프로필 이름 앞으로 파킹된다: {rows:?}"
        );
        assert_eq!(messaging.parked_len("boss"), 1);
        messaging
            .handle_send(
                "m-out",
                SenderIdentity {
                    peer_id: boss.id,
                    epoch: 0,
                },
                "boss",
                &["sleepy".to_string()],
                "해줘",
                Entrance::Cli,
                &req,
            )
            .expect("행 응답");
        assert_eq!(
            messaging.contract_outcome_for_test("m-out", "sleepy"),
            Some("awaiting_reply"),
            "boss 가 요청자인 계약이 열렸다"
        );

        // (4) boss 스폰 — 이제 **산 세션**이다.
        core.dispatch(
            AgentCommand::Spawn {
                profile_id: boss.id,
                request_id: engram_dashboard_protocol::RequestId(uuid::Uuid::new_v4()),
            },
            &session,
            &sink,
        )
        .await;
        assert!(
            core.manager.list_agents().iter().any(|a| a.id == boss.id),
            "스폰된 세션이 목록에 있어야(이 테스트의 전제)"
        );

        // (5) ★DeleteProfile — 실제 dispatch★. 세션은 죽지 않는다(킬은 별도 커맨드).
        core.dispatch(
            AgentCommand::DeleteProfile {
                profile_id: boss.id,
                request_id: engram_dashboard_protocol::RequestId(uuid::Uuid::new_v4()),
            },
            &session,
            &sink,
        )
        .await;

        // (6) 보호 단언.
        assert_eq!(
            messaging.parked_len("boss"),
            1,
            "★D1★ 산 세션의 파킹 메일이 삭제 정리에 죽었다(게이트가 이름 축이면 여기서 0이 된다)"
        );
        assert_eq!(
            messaging.contract_outcome_for_test("m-out", "sleepy"),
            Some("awaiting_reply"),
            "★D1★ 산 세션이 요청자인 계약이 실패 종결됐다(ADR-0118 결정 2 위반)"
        );

        // (7) ★반대 방향도 못 박는다 — 이름은 **제거 전에** 뽑아야 한다★.
        let rows2 = messaging
            .handle_send(
                "m-sleep",
                sender,
                "outsider",
                &["sleepy".to_string()],
                "잠든 상대에게",
                Entrance::Cli,
                &SendMeta::default(),
            )
            .expect("행 응답");
        assert_eq!(
            rows2[0].status,
            engram_dashboard_messaging::service::SendStatus::Pending
        );
        core.dispatch(
            AgentCommand::DeleteProfile {
                profile_id: sleepy.id,
                request_id: engram_dashboard_protocol::RequestId(uuid::Uuid::new_v4()),
            },
            &session,
            &sink,
        )
        .await;
        assert_eq!(
            messaging.parked_len("sleepy"),
            0,
            "잠든 프로필 삭제는 정리가 발동해야(이름을 제거 전에 뽑았다는 증거)"
        );
        let view = messaging
            .message_state("m-sleep", std::time::Instant::now())
            .expect("조회");
        assert_eq!(
            view.rows[0].code,
            Some("RECIPIENT_DELETED"),
            "장부 종점 + 사유 코드가 남는다(조용히 버리지 않는다): {view:?}"
        );

        core.manager.kill_agent(boss.id).ok();
    }

    // ── 6. Subscribe 시 conn_tx 에 SubscribeAck → ReplayComplete 순서로 들어가는지 ──
    //    (mock manager 가 없어 실 AgentManager 의 비어있는 snapshot 경로로는 NotFound 가 나므로,
    //     여기선 control 메시지 순서 로직을 직접 재현해 검증한다. 실 manager subscribe 의 replay
    //     동기 전송은 output_core.rs 단위테스트가 이미 커버.)
    #[tokio::test]
    async fn subscribe_control_order_ack_then_complete() {
        use engram_dashboard_net::frame_port::Frame;
        use engram_dashboard_protocol::{encode_terminal_frame, SubscribeAction};
        use tokio::sync::mpsc;
        let (tx, mut rx) = mpsc::channel::<Frame>(16);
        let agent_id = uuid::Uuid::new_v4();

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
        tx.send(Frame::Binary(encode_terminal_frame(agent_id, 0, 0, b"r")))
            .await
            .unwrap();
        let complete = event_json(&AgentEvent::ReplayComplete { agent_id, epoch: 0 }).unwrap();
        tx.send(Frame::Text(complete)).await.unwrap();

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
}
