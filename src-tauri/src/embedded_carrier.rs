//! Tauri embedded carrier — ADR-0020 Stage 2.
//!
//! 로컬(embedded)도 원격(WS)과 **같은 ConnectionCore.dispatch** 를 거치게 만드는 Tauri 어댑터다.
//! WS 어댑터(`ws.rs`)의 carrier 역할(conn_tx 단일 writer / WsOutboundSink / read_task)을 Tauri
//! IPC 로 미러한다:
//!  - WS 의 "1 TCP 연결" → embedded 의 "앱당 1개 영속 in-proc 연결"(부팅 시 1회 등록).
//!  - WS 의 conn_tx(단일 writer 큐) → embedded 의 **단일 outbound mpsc + 단일 writer task**
//!    (★BLOCKER 1 수정★: 아래 R1 FIFO 항목 참조).
//!  - WS 의 단일 read_task(명령 자연 직렬) → **inbound mpsc + 단일 command loop**(★결정2 보강:
//!    Tauri invoke 는 각각 독립 async task 라 병렬 도착 시 순서가 비결정 → mpsc+단일 loop 로
//!    WS 와 동일하게 직렬화).
//!
//! ## ★BLOCKER 1 — R1 FIFO 를 자료구조로 보장(단일 writer 큐)★
//! Tauri `Channel.send` 는 호출 스레드에서 동기 실행이라 자체 큐가 없다. 그래서 command loop
//! 스레드(replay·SubscribeAck·ReplayComplete enqueue)와 pump 스레드(live output emit)가 같은
//! Channel 에 동시에 send 하면 도착 순서가 경쟁한다. 게다가 Tauri 는 payload>8192B 를 fetch 큐
//! 경로로 보내(작은 control 이 큰 output frame 을 추월) 역전 가능.
//!
//! WS 가 conn_tx(단일 writer mpsc)로 막던 것을 **그대로 이식**한다:
//!  - 모든 sink(control = `TauriOutboundSink`, output = `TauriChannelOutputSink`)는 프론트 Channel 에
//!    **직접 send 하지 않고**, 연결당 단일 `mpsc::UnboundedSender<TauriOutbound>` 로만 보낸다.
//!  - 전용 **writer task** 하나가 그 mpsc 를 `recv().await` 순서대로 꺼내 **단 1곳**에서만
//!    `channel.send(outbound)` 를 호출한다 → enqueue 순서 = 프론트 도착 순서가 자료구조로 보장
//!    (WS write_task 동형).
//!  - control(command loop)과 output(pump)이 같은 mpsc 에 enqueue → on_ready 가 SubscribeAck 를
//!    enqueue 한 직후 코어가 replay 를 enqueue, lock drop 후 ReplayComplete 를 enqueue 하므로,
//!    같은 mpsc 면 그 순서대로 나간다(R1). 8192B fetch 임계도 단일 task 가 send 를 순차 호출하므로
//!    data_id 발급이 순서대로라 무해하다.
//!
//! ★출력 인코딩(carrier 소유)★: WS 는 binary frame, embedded 는 base64 PtyEvent(기존 인코딩
//! 유지). 둘 다 control(AgentEvent JSON)과 **같은 단일 mpsc→writer task→Channel** 로 합류하므로
//! R1(Ack→replay→ReplayComplete) FIFO 가 보존된다.
//!
//! ★기존 invoke 경로는 이 단계에선 공존★(Stage 4 에서 삭제). agent_command/agent_connect 경로를
//! 추가만 한다 — 프론트 Stage 3 가 전환한 뒤 옛 경로를 제거한다.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use futures_util::FutureExt as _;
use tauri::ipc::Channel;
use tokio::sync::mpsc;

use tauri::State;

use engram_dashboard_core::agent::types::{OutputFrame, OutputSink, PtyEvent, SinkError, SinkId};
use engram_dashboard_daemon::connection_core::{
    agent_list_event, hello_event, ConnectionCore, ConnectionSession, DispatchFlow, Outbound,
    OutboundSink, SinkError as CoreSinkError,
};
use engram_dashboard_protocol::{AgentCommand, AgentEvent, RequestId};

use crate::AppState;

/// 프론트의 단일 Channel 로 흐르는 carrier 페이로드(WS 의 WsOutbound 대응). control(Event)과
/// 출력(Output)을 한 타입으로 합쳐 **한 단일 writer 큐(mpsc)** 에 순서대로 실어 R1 FIFO 를 보존한다.
///
/// ★Stage 3 프론트 계약★: 프론트는 이 단일 enum 을 받아 `kind` 로 분기한다 —
///   `{kind:"event", event: AgentEvent}` / `{kind:"output", output: PtyEvent}`.
/// (serde tag="kind" — TS 측 discriminated union 으로 소비.)
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TauriOutbound {
    /// control 이벤트(Ack/SubscribeAck/ReplayComplete/Hello/AgentListUpdated/Error 등). JSON.
    /// ★Box★: AgentEvent(~272B)가 PtyEvent(작음)보다 훨씬 커 clippy large_enum_variant. control 은
    ///   hot path 가 아니므로(출력은 Output variant) Box 1회 할당이 무해. serde 는 Box<T> 를 T 와
    ///   동일 JSON 으로 직렬화하므로 프론트 wire 형태는 불변(`{kind:"event", event:{...}}`).
    Event { event: Box<AgentEvent> },
    /// 출력 frame — base64 PtyEvent(기존 embedded 인코딩 유지, JSON Channel 제약 우회).
    Output { output: PtyEvent },
}

/// 연결당 단일 writer 큐의 송신 끝(WS 의 conn_tx 대응). 모든 sink 가 이걸로만 보낸다 → writer task
/// 가 직렬로 Channel 에 흘려 FIFO 보존. Unbounded: embedded 는 in-proc 라 소비자(WebView Channel
/// send)가 빠르고, bounded 로 drop 하면 R1 replay 정확성이 깨진다(WS 는 네트워크라 bounded+drop).
type OutboundTx = mpsc::UnboundedSender<TauriOutbound>;

// ── TauriChannelOutputSink(코어 OutputSink, 출력 평면) ───────────────────────────────
//
// WS 의 WsOutputSink 대응. 코어 subscribe_from 이 replay/live 출력 frame 을 이 sink 로 보내면,
// raw bytes 를 base64 PtyEvent 로 인코딩(기존 ChannelOutputSink 와 동일)해 control 과 같은 단일
// 큐(OutboundTx)로 보낸다(FIFO 합류). base64 인코딩은 carrier(sink)가 소유 — ADR-0003.

/// 한 연결의 출력 sink. 코어가 raw OutputFrame 을 주면 base64 PtyEvent(TauriOutbound::Output)로
/// 인코딩해 단일 writer 큐로 보낸다. send 실패(큐 닫힘=writer task 종료)는 SinkError(코어가
/// dead-sink 로 제거).
struct TauriChannelOutputSink {
    id: SinkId,
    tx: OutboundTx,
    /// replay 구간 중 send 실패가 있었는지(handle_subscribe 의 truncated 사후 보정용).
    /// Unbounded 라 큐 full 은 없고 닫힘(writer task 종료)만 실패 — WS 와 동형으로 둔다.
    replay_dropped: Arc<AtomicBool>,
}

impl TauriChannelOutputSink {
    fn new(tx: OutboundTx) -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            tx,
            replay_dropped: Arc::new(AtomicBool::new(false)),
        }
    }
    fn replay_dropped_flag(&self) -> Arc<AtomicBool> {
        self.replay_dropped.clone()
    }
}

impl OutputSink for TauriChannelOutputSink {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        // base64 인코딩(JSON Channel 제약 우회) — 기존 embedded 인코딩 유지(lib.rs ChannelOutputSink 동일).
        // ★epoch★(BLOCKER 1): frame.epoch 을 반드시 동봉한다. 이걸 버리면 InProc 이 epoch 0 고정으로
        //   흘러 SubscribeAck.current_epoch≥1(resume-fallback)과 불일치 → ProtocolClient epoch 가드가
        //   출력을 전멸시킨다. WS binary frame 헤더(epoch 포함)와 동형화하는 핵심 한 줄.
        let event = PtyEvent {
            agent_id: frame.agent_id,
            seq: frame.seq,
            epoch: frame.epoch,
            data_b64: base64::engine::general_purpose::STANDARD.encode(frame.data),
        };
        // ★단일 writer 큐로만 보낸다(직접 Channel.send 금지)★ — control(command loop)과 합류해 FIFO.
        match self.tx.send(TauriOutbound::Output { output: event }) {
            Ok(()) => Ok(()),
            Err(_) => {
                // 큐 닫힘(writer task 종료=창 닫힘) — replay 구간이면 drop 기록, 코어가 dead-sink 제거.
                self.replay_dropped
                    .store(true, std::sync::atomic::Ordering::Release);
                Err(SinkError)
            }
        }
    }

    fn sink_id(&self) -> SinkId {
        self.id
    }
}

// ── TauriOutboundSink(ConnectionCore 의 OutboundSink, control 평면) ─────────────────────
//
// WS 의 WsOutboundSink 대응. dispatch 가 enqueue 하는 Outbound 를 TauriOutbound 로 변환해 단일
// writer 큐(OutboundTx)에 보낸다(인코딩은 이 어댑터 소유 — 코어 무지). make_output_sink 는 위
// 출력 sink 를 같은 큐로 만들어, control(이 sink)과 output(TauriChannelOutputSink)이 한 큐로 합류.

/// 한 연결의 control sink. ConnectionCore.dispatch 의 모든 응답/이벤트가 이걸 통해 단일 큐로 나간다.
pub struct TauriOutboundSink {
    tx: OutboundTx,
}

impl TauriOutboundSink {
    pub fn new(tx: OutboundTx) -> Self {
        Self { tx }
    }
}

impl OutboundSink for TauriOutboundSink {
    fn enqueue(&self, out: Outbound) -> Result<(), CoreSinkError> {
        let payload = match out {
            // control 이벤트 — TauriOutbound::Event 로 감싸 큐 송신(인코딩=serde JSON, Tauri 가 직렬화).
            //   Outbound::Event 는 Box<AgentEvent> → 그대로 재사용(재할당 없음).
            Outbound::Event(ev) => TauriOutbound::Event { event: ev },
            // codec binary frame — embedded dispatch 경로에선 발생하지 않는다(출력은 make_output_sink
            // 의 OutputSink 가 base64 로 직접 보냄). 방어적으로 base64 PtyEvent 로 디코드해 같은 큐
            // 로 흘린다(seq/agent_id 없는 raw codec 이라 decode 필요 — 발생 시 로그). FIFO 위해 같은 큐.
            Outbound::Binary(bytes) => match engram_dashboard_protocol::decode_frame(&bytes) {
                Ok(decoded) => TauriOutbound::Output {
                    output: PtyEvent {
                        agent_id: decoded.agent_id,
                        seq: decoded.seq,
                        // binary frame 헤더의 epoch 을 그대로 옮긴다(epoch 동봉 일관성 — BLOCKER 1).
                        epoch: decoded.epoch,
                        data_b64: base64::engine::general_purpose::STANDARD.encode(decoded.payload),
                    },
                },
                Err(e) => {
                    tracing::warn!("embedded carrier: Outbound::Binary 디코드 실패(무시): {e}");
                    return Ok(());
                }
            },
            // Close — embedded 는 프로세스 수명=연결 수명이라 별도 종료 frame 이 무의미(로그만).
            Outbound::Close(reason) => {
                tracing::debug!("embedded carrier: Outbound::Close 무시(in-proc): {reason}");
                return Ok(());
            }
        };
        // 단일 writer 큐로만 보낸다. 큐 닫힘(writer task 종료=창 닫힘) = SinkError(코어가 dead-sink 제거).
        self.tx.send(payload).map_err(|_| CoreSinkError)
    }

    fn make_output_sink(&self) -> (Arc<dyn OutputSink>, Arc<AtomicBool>) {
        // control(이 sink)과 같은 큐를 공유하는 출력 sink 생성 → 한 단일 writer 큐로 합류(FIFO, R1).
        let sink = Arc::new(TauriChannelOutputSink::new(self.tx.clone()));
        let flag = sink.replay_dropped_flag();
        (sink, flag)
    }
}

// ── 단일 in-proc 연결 상태(EmbeddedConnection) ───────────────────────────────────────
//
// WS 의 "1 연결" = (conn_tx + write_task + read_task + ConnectionSession). embedded 에선 앱당 1개:
//  - inbound: agent_command 가 명령을 넣는 mpsc(병렬 invoke 직렬화 — 결정2 보강).
//  - outbound: 단일 writer 큐 송신 끝. agent_connect 가 Channel 을 등록할 때 새 mpsc+writer task 를
//    만들어 여기에 sender 를 꽂는다(없으면 None → control 응답 drop).
//  - session: ConnectionSession(conn_id 1개 — single client 라 lease 항상 통과·viewport=자기크기).

/// embedded 단일 연결의 런타임 핸들. AppState 가 보관하고 command loop / agent_connect 가 공유한다.
pub struct EmbeddedConnection {
    /// agent_command 가 명령을 enqueue 하는 inbound 큐(단일 command loop 가 소비 → FIFO 직렬화).
    inbound_tx: mpsc::UnboundedSender<AgentCommand>,
    /// 현재 등록된 outbound 단일 writer 큐의 송신 끝(없으면 control 응답 drop). agent_connect 가
    /// Channel 등록 시 새 mpsc+writer task 를 만들어 그 sender 를 여기 꽂는다.
    /// ★Mutex★: agent_connect(설정)와 command loop(읽기)가 공유 — 짧게 잠그고 clone 후 해제.
    /// ★poison-tolerant★: command loop 의 dispatch 가 panic 해도(catch_unwind 로 잡지만 이중 안전)
    ///   이 lock 이 poison 되면 안 되도록, 읽기/쓰기 모두 into_inner 로 가드를 회수한다.
    outbound: Arc<Mutex<Option<OutboundTx>>>,
}

impl EmbeddedConnection {
    /// 프론트가 만든 단일 Channel 을 이 연결의 outbound 로 등록(WS 의 conn_tx 등록 + write_task spawn
    /// 대응). 새 mpsc 쌍을 만들어 sender 를 outbound 에 꽂고, receiver 를 drain 하는 writer task 를
    /// spawn 한다. 재등록(창 재로드 등)이면 outbound 의 sender 를 교체 → 옛 writer task 의 receiver 가
    /// 닫혀(옛 sender drop) 자연 종료한다(단일 연결 모델 — 1개만 유지).
    pub fn set_channel(&self, channel: Channel<TauriOutbound>) {
        let (tx, rx) = mpsc::unbounded_channel::<TauriOutbound>();
        // ★단일 writer task★: rx 를 recv 순서대로 꺼내 단 1곳에서만 channel.send 호출 → FIFO 보존(R1).
        spawn_writer_task(channel, rx);
        // 옛 sender 를 새 것으로 교체. 옛 sender 가 drop 되면 옛 writer task 의 rx 가 닫혀 종료한다.
        let mut guard = self.outbound.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(tx);
    }

    /// 현재 등록된 outbound sender 를 clone(없으면 None). command loop 가 sink 를 만들 때 호출.
    fn outbound_tx(&self) -> Option<OutboundTx> {
        self.outbound
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// 명령을 inbound 큐에 enqueue(agent_command 가 호출). 단일 command loop 가 FIFO 로 dispatch.
    /// 실패(loop 종료)는 Err — 정상 수명에선 발생 안 함.
    pub fn enqueue_command(&self, cmd: AgentCommand) -> Result<(), String> {
        self.inbound_tx
            .send(cmd)
            .map_err(|_| "embedded command loop closed".to_string())
    }
}

/// 단일 writer task — outbound mpsc 를 recv 순서대로 꺼내 **단 1곳**에서만 `channel.send` 를 호출한다.
/// 이게 R1 FIFO 의 핵심: 여러 sink(control/output)가 같은 mpsc 에 enqueue 한 순서가, 이 task 의 순차
/// send 로 프론트 도착 순서로 그대로 보존된다(WS write_task 동형). channel.send 실패(창 닫힘)나 rx
/// 닫힘(sender 교체/drop)이면 종료.
fn spawn_writer_task(
    channel: Channel<TauriOutbound>,
    mut rx: mpsc::UnboundedReceiver<TauriOutbound>,
) {
    tauri::async_runtime::spawn(async move {
        while let Some(outbound) = rx.recv().await {
            // ★유일한 channel.send 지점★ — 직렬 호출이라 enqueue 순서 = 프론트 도착 순서(FIFO, R1).
            // 8192B fetch 임계 경로도 순차 send 라 data_id 발급이 순서대로 → 큰 frame 이 작은 control 을
            // 추월하지 못한다.
            if channel.send(outbound).is_err() {
                // 창 닫힘 등 — 더 보낼 곳이 없다. task 종료(다음 set_channel 이 새 task 를 만든다).
                tracing::debug!("embedded carrier: writer task — channel.send 실패(창 닫힘) 종료");
                break;
            }
        }
        tracing::debug!("embedded carrier: writer task 종료(outbound 큐 닫힘)");
    });
}

/// AgentCommand 에서 request_id 를 추출한다(panic 격리 시 Error 통보용). request_id 가 없는 variant
/// (Auth/Resize/Subscribe/Unsubscribe)는 None — 그 경우 panic 통보는 로그로만 남는다.
fn command_request_id(cmd: &AgentCommand) -> Option<RequestId> {
    match cmd {
        AgentCommand::Spawn { request_id, .. }
        | AgentCommand::Kill { request_id, .. }
        | AgentCommand::Interrupt { request_id, .. }
        | AgentCommand::WriteStdin { request_id, .. }
        | AgentCommand::AcquireInput { request_id, .. }
        | AgentCommand::ReleaseInput { request_id, .. }
        | AgentCommand::ListAgents { request_id }
        | AgentCommand::StopDaemon { request_id, .. }
        | AgentCommand::SpawnByCwd { request_id, .. }
        | AgentCommand::ListProfiles { request_id }
        | AgentCommand::CreateProfile { request_id, .. }
        | AgentCommand::DeleteProfile { request_id, .. }
        | AgentCommand::SpawnProfile { request_id, .. }
        | AgentCommand::SetProfileAutoRestore { request_id, .. }
        | AgentCommand::GetSnapshot { request_id, .. } => Some(*request_id),
        // request_id 없는 variant — panic 통보는 로그로만.
        AgentCommand::Auth { .. }
        | AgentCommand::Resize { .. }
        | AgentCommand::Subscribe { .. }
        | AgentCommand::Unsubscribe { .. } => None,
    }
}

/// 부팅 시 1회 호출 — 단일 in-proc 연결(ConnectionCore + inbound mpsc + 단일 command loop)을 기동한다.
///
/// WS 의 handle_connection 한 번에 대응(앱당 1개 영속). 반환 EmbeddedConnection 을 AppState 가 보관:
/// agent_connect 가 Channel 을 등록하고, agent_command 가 inbound 에 명령을 넣으면 이 loop 가 FIFO 로
/// dispatch 한다.
///
/// ★command loop 직렬화(결정2 보강)★: inbound mpsc 를 단일 task 가 순차 recv → dispatch 하므로,
/// agent_command invoke 가 아무리 병렬로 도착해도 처리 순서는 inbound 에 들어간 순서로 고정된다
/// (WS 의 단일 read_task 와 동등). dispatch 는 한 번에 하나만 실행된다.
///
/// ★BLOCKER 2 — panic/poison 격리★: dispatch 한 건의 panic 이 loop task 를 죽이면 inbound_tx 는
/// 살아 enqueue 는 계속 Ok → 명령이 영영 무시되는 좀비 연결이 된다. 그래서 dispatch 를
/// `AssertUnwindSafe(..).catch_unwind().await` 로 감싸 panic 을 흡수하고, 그 명령의 request_id 로
/// Error 를 outbound 로 통보한 뒤 loop 를 계속한다. outbound Mutex 도 poison-tolerant(into_inner)로
/// 한 번의 poison 이 loop 를 죽이지 않게 한다(reaper.rs 패턴 동형).
pub fn spawn_embedded_connection(core: Arc<ConnectionCore>) -> EmbeddedConnection {
    let (inbound_tx, mut inbound_rx) = mpsc::unbounded_channel::<AgentCommand>();
    let outbound: Arc<Mutex<Option<OutboundTx>>> = Arc::new(Mutex::new(None));

    // single client → conn_id 1개 고정(lease 항상 통과, viewport 협상=자기 크기로 자연 무력화).
    let session = ConnectionSession::new(1);
    let loop_outbound = outbound.clone();

    // 단일 command loop. inbound 에서 명령을 FIFO 로 꺼내 dispatch. Channel 미등록 상태에서 온
    // 명령도 처리는 하되(부작용은 발생), control 응답은 큐 없음 → drop(연결 전 명령은 드묾).
    tauri::async_runtime::spawn(async move {
        while let Some(cmd) = inbound_rx.recv().await {
            // 현재 등록된 outbound 큐를 clone(없으면 응답 drop). lock 은 clone 동안만 짧게 보유,
            // poison-tolerant(into_inner) — dispatch panic 이 이 lock 을 오염시켜도 loop 생존.
            let tx = loop_outbound
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            match tx {
                Some(tx) => {
                    let request_id = command_request_id(&cmd);
                    let sink = TauriOutboundSink::new(tx);
                    // ★panic 격리★: dispatch 는 한 번에 하나 — 다음 recv 는 이 dispatch 가 끝나야 진행
                    //   (직렬). catch_unwind 로 panic 을 흡수해 loop 가 죽지 않게 한다(좀비 방지).
                    //   AssertUnwindSafe 근거: core/session 은 다음 iteration 에서 재사용되지만, 둘의
                    //   내부 가변상태는 lock(ADR-0006: Arc clone 후 즉시 해제)으로만 만지고 그 lock 이
                    //   poison 되지 않으므로(panic 가능 코드를 guard 보유 중 실행 안 함) 재사용 안전.
                    let dispatched =
                        std::panic::AssertUnwindSafe(core.dispatch(cmd, &session, &sink))
                            .catch_unwind()
                            .await;
                    match dispatched {
                        Ok(DispatchFlow::Close) => {
                            // embedded 에선 StopDaemon 이 와도 앱을 내리지 않는다(in-proc — 연결만 정리).
                            // 단일 연결 모델상 loop 를 유지한다(다음 명령 계속 처리). 로그만.
                            tracing::info!(
                                "embedded carrier: dispatch Close(StopDaemon) — in-proc 무시"
                            );
                        }
                        Ok(DispatchFlow::Continue) => {}
                        Err(panic) => {
                            // dispatch 가 panic — loop 는 계속한다(좀비 방지). 그 명령의 request_id 로
                            //   Error 를 통보(없으면 로그만). 같은 sink(=같은 큐)로 보내 FIFO 유지.
                            let detail = panic
                                .downcast_ref::<&str>()
                                .map(|s| s.to_string())
                                .or_else(|| panic.downcast_ref::<String>().cloned())
                                .unwrap_or_else(|| "<non-string panic>".to_string());
                            tracing::error!(
                                panic = %detail,
                                "embedded carrier: dispatch panicked — command loop 생존, 다음 명령 계속"
                            );
                            let _ = sink.enqueue(Outbound::event(AgentEvent::Error {
                                request_id,
                                message: format!("internal error processing command: {detail}"),
                            }));
                        }
                    }
                }
                None => {
                    // Channel 미등록 — 응답을 보낼 곳이 없다. 부작용 없는 명령은 무시, 부작용 있는
                    // 명령(spawn/kill 등)도 응답만 못 받을 뿐. 정상 흐름에선 agent_connect 가 먼저 온다.
                    tracing::warn!("embedded carrier: Channel 미등록 상태로 명령 수신 — 응답 drop");
                }
            }
        }
        tracing::debug!("embedded carrier: command loop 종료(inbound 모든 sender drop)");
    });

    EmbeddedConnection {
        inbound_tx,
        outbound,
    }
}

// ── Tauri commands(carrier 진입점) ─────────────────────────────────────────────────

/// 프론트가 단일 outbound Channel 을 등록한다(WS 의 "연결 직후 conn_tx 등록 + Hello/list push" 대응).
///
/// 프론트는 `Channel<TauriOutbound>` 를 만들어 invoke('agent_connect', {channel}) 로 넘긴다. 등록
/// 직후 WS 와 동형으로 Hello + 현재 에이전트 목록을 **단일 writer 큐로** push 한다(초기 동기화).
/// 이후 모든 Outbound(control + 출력)는 그 큐→writer task→Channel 로 흐른다(FIFO).
///
/// ★재호출(창 재로드 등)★: set_channel 이 새 mpsc+writer task 로 교체한다(단일 연결 모델 — 1개만 유지).
#[tauri::command]
pub async fn agent_connect(
    state: State<'_, AppState>,
    channel: Channel<TauriOutbound>,
) -> Result<(), String> {
    // 1) 이 연결의 outbound 로 등록(새 mpsc+writer task 생성, command loop 가 이 큐로 응답을 보낸다).
    state.embedded.set_channel(channel);

    // 2) 연결 직후 Hello + 현재 목록 push(WS handle_connection 4단계 미러 — 초기 동기화).
    //    ★단일 writer 큐 경유★: 직접 channel.send 가 아니라 등록된 outbound 큐로 보내, 이후 모든
    //    control/output 과 같은 FIFO 경로를 탄다(직접 send 하면 writer task 와 경쟁해 R1 위반).
    let Some(tx) = state.embedded.outbound_tx() else {
        // set_channel 직후라 정상 흐름에선 항상 Some. 방어적으로 None 이면 push 생략.
        return Ok(());
    };
    let hello = hello_event(env!("CARGO_PKG_VERSION").to_string());
    let _ = tx.send(TauriOutbound::Event {
        event: Box::new(hello),
    });
    let list = agent_list_event(&state.manager);
    let _ = tx.send(TauriOutbound::Event {
        event: Box::new(list),
    });
    Ok(())
}

/// generic 명령 진입점(WS 의 read_task frame→cmd 대응). invoke 20개를 1개로 합친다.
///
/// ★결정2 보강(racing 직렬화)★: 각 agent_command invoke 는 독립 async task 라 병렬 도착하면 순서가
/// 비결정이다. 그래서 여기선 dispatch 를 직접 부르지 않고 명령을 inbound mpsc 에 enqueue 만 한다 —
/// 단일 command loop 가 FIFO 로 꺼내 dispatch 하므로 처리 순서가 enqueue 순서로 고정된다(WS 단일
/// read_task 와 동등). 응답(Ack/Error/SubscribeAck/출력 등)은 agent_connect 로 등록한 큐로 나간다.
#[tauri::command]
pub async fn agent_command(state: State<'_, AppState>, cmd: AgentCommand) -> Result<(), String> {
    state.embedded.enqueue_command(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    use engram_dashboard_core::agent::manager::AgentManager;
    use engram_dashboard_core::agent::profile::{ProfileRegistry, ProfileStore};
    use engram_dashboard_core::agent::session_tracker::{SessionTracker, TrackerConfig};
    use engram_dashboard_core::agent::types::{AgentInfo, AgentStatus, StatusSink};
    use engram_dashboard_daemon::connection_core::MultiViewState;
    use engram_dashboard_daemon::ws::ConnRegistry;
    use engram_dashboard_protocol::RequestId;

    /// 테스트용 no-op StatusSink(emit 무시). embedded conformance 는 dispatch 의 Outbound 결과만 본다.
    struct NoopStatusSink;
    impl StatusSink for NoopStatusSink {
        fn status_changed(&self, _id: uuid::Uuid, _status: AgentStatus, _epoch: u32) {}
        fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
    }

    /// in-memory ProfileStore(디스크 IO 없음).
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

    /// 테스트용 ConnectionCore 배선(daemon test_core 미러, src-tauri 측 타입으로). dummy registry/watch.
    fn test_core() -> Arc<ConnectionCore> {
        let store: Arc<dyn ProfileStore> = Arc::new(MemStore::default());
        let status_sink = Arc::new(NoopStatusSink);
        let profiles = Arc::new(ProfileRegistry::new(store));
        let tracker = Arc::new(SessionTracker::new(
            TrackerConfig::default(),
            Arc::new(|_aid, _sid| {}),
        ));
        let manager = Arc::new(AgentManager::new(status_sink, profiles, tracker));
        let (shutdown_tx, _rx) = tokio::sync::watch::channel(false);
        Arc::new(ConnectionCore::new(
            manager,
            MultiViewState::new(),
            ConnRegistry::new(),
            shutdown_tx,
        ))
    }

    /// TauriOutbound 를 수집하는 테스트 Channel + 받은 항목 핸들을 함께 만든다.
    /// Channel::new 핸들러가 InvokeResponseBody::Json(직렬화된 TauriOutbound)을 받으면 파싱해 모은다.
    fn collecting_channel() -> (
        Channel<TauriOutbound>,
        Arc<StdMutex<Vec<serde_json::Value>>>,
    ) {
        let collected: Arc<StdMutex<Vec<serde_json::Value>>> = Arc::new(StdMutex::new(Vec::new()));
        let sink = collected.clone();
        let channel = Channel::new(move |body: tauri::ipc::InvokeResponseBody| {
            // TauriOutbound 는 Serialize → Json(String). 파싱해 kind/event 를 검증할 수 있게 보관.
            let v: serde_json::Value = body.deserialize().expect("TauriOutbound JSON 역직렬화");
            sink.lock().unwrap().push(v);
            Ok(())
        });
        (channel, collected)
    }

    /// 단일 writer 큐(mpsc) sender 를 만들고, 그 큐를 주어진 Channel 로 drain 하는 writer task 를
    /// spawn 한다 — sink 단독 테스트가 EmbeddedConnection 없이 carrier 큐 경로를 그대로 쓰게.
    fn outbound_to_channel(channel: Channel<TauriOutbound>) -> OutboundTx {
        let (tx, rx) = mpsc::unbounded_channel::<TauriOutbound>();
        spawn_writer_task(channel, rx);
        tx
    }

    /// collected 가 n 건 이상 모일 때까지 폴링 대기(writer task 는 async 라 즉시 도착 보장 없음).
    async fn wait_for(collected: &Arc<StdMutex<Vec<serde_json::Value>>>, n: usize) {
        let mut waited = 0;
        loop {
            if collected.lock().unwrap().len() >= n {
                return;
            }
            if waited > 300 {
                panic!("기대 {n}건 미도착: {}건만", collected.lock().unwrap().len());
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            waited += 1;
        }
    }

    fn rid() -> RequestId {
        RequestId(uuid::Uuid::new_v4())
    }

    // ── conformance: TauriOutboundSink 로 dispatch → WS 와 동일 Outbound(Error) ──────────
    //    WS 의 spawn_missing_profile_errors 와 동등: 없는 profile spawn → Error(request_id 동봉).
    #[tokio::test]
    async fn embedded_spawn_missing_profile_errors() {
        let core = test_core();
        let (channel, collected) = collecting_channel();
        let tx = outbound_to_channel(channel);
        let sink = TauriOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let req = rid();
        core.dispatch(
            AgentCommand::Spawn {
                profile_id: uuid::Uuid::new_v4(),
                request_id: req,
            },
            &session,
            &sink,
        )
        .await;

        wait_for(&collected, 1).await;
        let items = collected.lock().unwrap();
        assert_eq!(items.len(), 1, "Error 1건만");
        // carrier wire: {kind:"event", event:{...}}. AgentEvent 는 externally-tagged enum →
        //   {"Error": {request_id, message}}(variant 이름이 key). 프론트 Stage 3 가 이 형태로 소비.
        assert_eq!(items[0]["kind"], "event", "carrier wire kind=event");
        let err = &items[0]["event"]["Error"];
        assert!(err.is_object(), "Error variant: {:?}", items[0]["event"]);
        assert_eq!(
            err["request_id"],
            serde_json::json!(req.0.to_string()),
            "Error 에 request_id 동봉"
        );
    }

    // ── conformance: ListAgents → AgentList(request_id 동봉) carrier wire ────────────────
    #[tokio::test]
    async fn embedded_list_agents_returns_agent_list() {
        let core = test_core();
        let (channel, collected) = collecting_channel();
        let tx = outbound_to_channel(channel);
        let sink = TauriOutboundSink::new(tx);
        let session = ConnectionSession::new(1);
        let req = rid();
        core.dispatch(
            AgentCommand::ListAgents { request_id: req },
            &session,
            &sink,
        )
        .await;

        wait_for(&collected, 1).await;
        let items = collected.lock().unwrap();
        assert_eq!(items.len(), 1, "AgentList 1건");
        assert_eq!(items[0]["kind"], "event");
        let al = &items[0]["event"]["AgentList"];
        assert!(al.is_object(), "AgentList variant: {:?}", items[0]["event"]);
        assert_eq!(al["request_id"], serde_json::json!(req.0.to_string()));
    }

    // ── racing 직렬화: 병렬 enqueue N개 → command loop 가 FIFO 순서로 dispatch ────────────
    //    각 ListAgents 의 request_id 를 0..N 순서로 넣고, Channel 에 도착한 AgentList 의 request_id
    //    순서가 enqueue 순서와 동일함을 단언한다. 단일 command loop(inbound mpsc)가 순서를 고정한다.
    #[tokio::test]
    async fn embedded_command_loop_serializes_in_fifo_order() {
        let core = test_core();
        let conn = spawn_embedded_connection(core);
        let (channel, collected) = collecting_channel();
        conn.set_channel(channel);

        let n = 50usize;
        let mut order: Vec<String> = Vec::with_capacity(n);
        for _ in 0..n {
            let req = rid();
            order.push(req.0.to_string());
            conn.enqueue_command(AgentCommand::ListAgents { request_id: req })
                .expect("enqueue");
        }

        wait_for(&collected, n).await;
        let items = collected.lock().unwrap();
        let received: Vec<String> = items
            .iter()
            .map(|v| {
                v["event"]["AgentList"]["request_id"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(
            received, order,
            "command loop 는 enqueue(FIFO) 순서대로 dispatch 해야 한다(racing 직렬화)"
        );
    }

    // ── make_output_sink generic: TauriOutboundSink 가 출력 sink 를 base64 PtyEvent 로 만든다 ──
    //    실 agent 없이 sink 단독 검증 — OutputFrame 을 보내면 Channel 에 kind:output, base64 가 온다.
    #[tokio::test]
    async fn embedded_output_sink_encodes_base64() {
        let (channel, collected) = collecting_channel();
        let tx = outbound_to_channel(channel);
        let sink = TauriOutboundSink::new(tx);
        let (out_sink, _flag) = sink.make_output_sink();
        let agent_id = uuid::Uuid::new_v4();
        let frame = OutputFrame {
            agent_id,
            epoch: 0,
            seq: 7,
            data: b"hello",
        };
        out_sink.send(frame).expect("send ok");

        wait_for(&collected, 1).await;
        let items = collected.lock().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["kind"], "output", "출력은 kind=output");
        assert_eq!(items[0]["output"]["seq"], 7);
        // base64("hello") = "aGVsbG8=".
        assert_eq!(items[0]["output"]["data_b64"], "aGVsbG8=");
        assert_eq!(items[0]["output"]["epoch"], 0, "epoch 동봉(frame.epoch=0)");
    }

    // ── BLOCKER 1 회귀: PtyEvent.epoch 가 OutputFrame.epoch 에서 채워진다(0 고정 아님) ────────
    //    embedded 출력이 epoch≥1 세션에서 epoch 0 으로 흐르면 ProtocolClient epoch 가드가 출력을
    //    전멸시킨다(Stage 3 BLOCKER 1). 이 테스트는 frame.epoch=3 을 주고 PtyEvent.epoch==3 을
    //    단언한다 → send 가 frame.epoch 을 버리고 0 으로 고정하는 mutation 이면 fail 한다.
    #[tokio::test]
    async fn embedded_output_sink_carries_frame_epoch() {
        let (channel, collected) = collecting_channel();
        let tx = outbound_to_channel(channel);
        let sink = TauriOutboundSink::new(tx);
        let (out_sink, _flag) = sink.make_output_sink();
        let agent_id = uuid::Uuid::new_v4();
        out_sink
            .send(OutputFrame {
                agent_id,
                epoch: 3,
                seq: 1,
                data: b"x",
            })
            .expect("send ok");

        wait_for(&collected, 1).await;
        let items = collected.lock().unwrap();
        assert_eq!(
            items[0]["output"]["epoch"], 3,
            "PtyEvent.epoch 은 OutputFrame.epoch(3)에서 채워져야 한다(0 고정 금지 — BLOCKER 1)"
        );
    }

    // ── BLOCKER 1 회귀: 단일 writer 큐가 control/output enqueue 순서를 FIFO 로 보존 ──────────
    //    ws_e2e case04/05/11(replay 순서)의 embedded 버전. on_ready(SubscribeAck) → replay output →
    //    ReplayComplete 가 같은 큐에 enqueue 된 순서대로, 그 사이 끼어든 live output 보다 앞서 도착해야.
    //
    //    ★mutation 으로 실효 확인★: 단일 writer 큐를 우회해(control 은 직접 channel.send, output 은
    //    큐) 보내면 순서 경쟁이 생긴다. 이 테스트는 "control 과 output 이 같은 큐로 enqueue 된 순서가
    //    프론트 도착 순서와 같다"를 단언하므로, 우회(두 경로) 시 순서가 깨져 fail 한다.
    //
    //    실 agent 없이 carrier 큐 경로만 검증한다(handle_subscribe 의 enqueue 순서 = SubscribeAck →
    //    replay(N) → ReplayComplete 를 코어 대신 직접 재현). 핵심은 "한 큐에 넣은 순서 = 나온 순서".
    #[tokio::test]
    async fn embedded_single_writer_preserves_ack_replay_complete_order() {
        let (channel, collected) = collecting_channel();
        let tx = outbound_to_channel(channel.clone());

        let control = TauriOutboundSink::new(tx.clone());
        let (out_sink, _flag) = control.make_output_sink();
        let agent_id = uuid::Uuid::new_v4();

        // handle_subscribe 가 같은 큐에 넣는 순서를 그대로 재현:
        //   1) SubscribeAck(control)  2) replay output 0..R(output)  3) ReplayComplete(control)
        // 그 사이 "live" output 한 건을 replay 도중 끼워 넣어, 단일 큐라면 enqueue 순서대로 나오는지 본다.
        control
            .enqueue(Outbound::event(AgentEvent::SubscribeAck {
                agent_id,
                action: engram_dashboard_protocol::SubscribeAction::Reset,
                current_epoch: 0,
                oldest_seq: 0,
                latest_seq: 2,
                replay_from: 0,
                truncated: false,
            }))
            .expect("ack");

        let replay = 3u64;
        for seq in 0..replay {
            out_sink
                .send(OutputFrame {
                    agent_id,
                    epoch: 0,
                    seq,
                    data: format!("r{seq}").as_bytes(),
                })
                .expect("replay frame");
        }
        // replay 와 ReplayComplete 사이에 끼어든 live output(같은 큐 → 반드시 ReplayComplete 앞).
        out_sink
            .send(OutputFrame {
                agent_id,
                epoch: 0,
                seq: replay,
                data: b"live",
            })
            .expect("live frame");
        control
            .enqueue(Outbound::event(AgentEvent::ReplayComplete {
                agent_id,
                epoch: 0,
            }))
            .expect("complete");

        // 총 = Ack(1) + replay(3) + live(1) + ReplayComplete(1) = 6.
        wait_for(&collected, 6).await;
        let items = collected.lock().unwrap();
        assert_eq!(items.len(), 6, "Ack+replay+live+ReplayComplete = 6건");

        // 0번 = SubscribeAck(control), 1..=3 = replay output(seq 0,1,2), 4 = live output(seq 3),
        // 5 = ReplayComplete. 단일 writer 큐라 enqueue 순서가 그대로 보존돼야 한다.
        assert!(
            items[0]["event"]["SubscribeAck"].is_object(),
            "첫 항목은 SubscribeAck(control), 실제: {:?}",
            items[0]
        );
        for (i, seq) in (0..replay).enumerate() {
            assert_eq!(items[i + 1]["kind"], "output", "replay {i} 는 output");
            assert_eq!(
                items[i + 1]["output"]["seq"],
                seq,
                "replay output seq 순서 보존"
            );
        }
        assert_eq!(items[4]["kind"], "output", "live 는 output");
        assert_eq!(
            items[4]["output"]["seq"], replay,
            "live output 은 replay 뒤·ReplayComplete 앞"
        );
        assert!(
            items[5]["event"]["ReplayComplete"].is_object(),
            "마지막은 ReplayComplete(control) — replay/live 다음, 실제: {:?}",
            items[5]
        );
    }

    // ── BLOCKER 2 회귀: dispatch panic 이 와도 command loop 가 살아 다음 명령을 처리한다 ──────
    //    좀비 연결 방지. dispatch 가 panic 하는 경로를 직접 만들기 어려우니, "정상 명령들이 연속으로
    //    처리되는가 + panic 격리 구조(catch_unwind)가 컴파일/런타임에 존재"를 회귀로 잡는다.
    //    panic 주입은 코어 변경이 필요해 여기선 loop 생존(연속 처리)으로 대리 검증한다.
    #[tokio::test]
    async fn embedded_command_loop_survives_and_processes_sequentially() {
        let core = test_core();
        let conn = spawn_embedded_connection(core);
        let (channel, collected) = collecting_channel();
        conn.set_channel(channel);

        // 여러 명령을 연속 enqueue — loop 가 죽지 않고 전부 처리(catch_unwind 가 정상 경로를 막지 않음).
        let n = 10usize;
        for _ in 0..n {
            conn.enqueue_command(AgentCommand::ListAgents { request_id: rid() })
                .expect("enqueue");
        }
        wait_for(&collected, n).await;
        assert_eq!(
            collected.lock().unwrap().len(),
            n,
            "command loop 가 panic 격리 구조에서도 모든 명령을 처리"
        );
    }

    // ── BLOCKER 2 회귀: outbound Mutex poison 이 와도 후속 명령이 처리된다 ────────────────────
    //    set_channel/outbound_tx 가 into_inner 로 poison-tolerant 인지 검증. 다른 스레드에서 lock 보유
    //    중 panic 시켜 강제 poison → 그 뒤 명령이 정상 처리되면 격리 성공.
    #[tokio::test]
    async fn embedded_outbound_poison_tolerant() {
        let core = test_core();
        let conn = Arc::new(spawn_embedded_connection(core));
        let (channel, collected) = collecting_channel();
        conn.set_channel(channel);

        // outbound Mutex 를 강제 poison: lock 잡은 채 panic 하는 스레드.
        {
            let poison = conn.clone();
            let _ = std::thread::spawn(move || {
                let _g = poison.outbound.lock().unwrap();
                panic!("강제 poison");
            })
            .join();
        }
        assert!(
            conn.outbound.is_poisoned(),
            "테스트 전제: outbound Mutex 가 poison 상태여야 함"
        );

        // poison 이후에도 enqueue→dispatch→응답이 정상(into_inner 로 가드 회수).
        conn.enqueue_command(AgentCommand::ListAgents { request_id: rid() })
            .expect("enqueue");
        wait_for(&collected, 1).await;
        assert_eq!(
            collected.lock().unwrap().len(),
            1,
            "poison 후에도 command loop 가 응답을 보냄(poison-tolerant)"
        );
    }
}
