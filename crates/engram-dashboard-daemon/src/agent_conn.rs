//! 에이전트 시스템이 네트워크 프레임 포트에 꽂는 어댑터(ADR-0129).
//!
//! 소유하는 것: 연결 수명 훅(`AgentConnection` — Hello/목록 push · 명령 dispatch · 연결 정리)과
//! **wire 인코딩 전부**(`AgentEvent`→JSON text, `OutputFrame`→codec binary). 네트워크 행은 여기서
//! 나온 불투명 프레임만 받는다.
//!
//! ★단일 writer 합류(FIFO)★: control 평면(`FrameOutboundSink`)과 출력 평면(`FrameOutputSink`)이
//!   **같은 `FrameSink`** 로 나가므로, dispatch 가 SubscribeAck 를 replay binary 보다 먼저 넣으면
//!   그 순서가 그대로 보존된다.
//! ★블록 금지★: 두 sink 의 enqueue 는 pump/manager 동기 스레드에서 불릴 수 있어 `FrameSink::try_send`
//!   (논블록)만 쓴다. 큐 포화 시 종료 신호를 울리는 것은 `FrameSink` 구현(네트워크 행)의 몫이다.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use engram_dashboard_core::agent::manager::AgentManager;
use engram_dashboard_core::agent::types::{
    AgentId, OutputFrame, OutputPayload, OutputSink, SinkError, SinkId,
};
use engram_dashboard_protocol::{
    encode_structured_frame, encode_terminal_frame, AgentCommand, AgentEvent,
};

use futures_util::future::BoxFuture;
use tokio::sync::watch;

use crate::connection_core::{
    agent_list_event, broadcast_lease_changed, event_json, hello_event, output_event_to_wire,
    ConnectionCore, ConnectionSession, DispatchFlow, MultiViewState, Outbound,
    OutboundSink as CoreOutboundSink, SinkError as CoreSinkError,
};
use engram_dashboard_net::frame_port::{
    ConnFlow, ConnId, ConnectionHandler, ConnectionHandlerFactory, Frame, FrameFanout, FrameSink,
};

// ── 출력 평면 sink(연결당 구독 1개, pump 스레드에서 호출) ─────────────────────────

/// 한 연결의 한 에이전트 구독에 대응하는 `OutputSink`. pump 스레드가 `send` 를 호출한다.
/// frame 을 codec binary 로 인코딩해 `FrameSink` 에 **논블록으로만** 넣는다.
/// 큐가 full/closed 면 `SinkError` 반환(코어가 dead-sink 로 제거).
pub struct FrameOutputSink {
    frames: Arc<dyn FrameSink>,
    /// replay 구간 중 try_send 실패(frame drop)가 한 번이라도 있었는지.
    /// handle_subscribe 가 ReplayComplete 직전 검사해 SubscribeAck.truncated 를 사후 보정한다.
    /// 평소(라이브)엔 코어가 dead-sink 로 제거하므로 의미가 없고, replay 구간 정확성에만 쓴다.
    replay_dropped: Arc<AtomicBool>,
    sink_id: SinkId,
}

impl FrameOutputSink {
    pub(crate) fn new(frames: Arc<dyn FrameSink>) -> Self {
        Self {
            frames,
            replay_dropped: Arc::new(AtomicBool::new(false)),
            sink_id: uuid::Uuid::new_v4(),
        }
    }

    /// replay 구간 동안 frame 이 drop 됐는지 사후 검사용 핸들(handle_subscribe 가 공유 보관).
    pub(crate) fn replay_dropped_flag(&self) -> Arc<AtomicBool> {
        self.replay_dropped.clone()
    }
}

impl OutputSink for FrameOutputSink {
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
        // ★pump 스레드 — 논블록만(절대 block 금지). full/closed = 느린 소비자 → 코어가 이 sink 제거.
        //   포화 시 큐 밖 종료 신호는 FrameSink 구현이 울린다(좀비 연결 방지).
        match self.frames.try_send(Frame::Binary(buf)) {
            Ok(()) => Ok(()),
            Err(_) => {
                // frame 이 drop 됐음을 기록(replay 구간 truncated 사후 보정용).
                self.replay_dropped.store(true, Ordering::Release);
                Err(SinkError)
            }
        }
    }

    fn sink_id(&self) -> SinkId {
        self.sink_id
    }
}

// ── control 평면 sink(ConnectionCore.dispatch 의 응답 경로) ─────────────────────────

/// `ConnectionCore` 의 `OutboundSink` 를 프레임 포트로 구현한다. dispatch 가 enqueue 하는
/// `Outbound` 를 프레임으로 인코딩해 `FrameSink` 에 넣는다 — 인코딩(AgentEvent→JSON text)은 이
/// 어댑터가 소유한다(코어는 모름 — ADR-0003 정합).
pub struct FrameOutboundSink {
    frames: Arc<dyn FrameSink>,
}

impl FrameOutboundSink {
    pub(crate) fn new(frames: Arc<dyn FrameSink>) -> Self {
        Self { frames }
    }
}

impl CoreOutboundSink for FrameOutboundSink {
    fn enqueue(&self, out: Outbound) -> Result<(), CoreSinkError> {
        let frame = match out {
            // control 이벤트 — JSON text 로 인코딩(어댑터 소유). 직렬화 실패는 drop(event_json 동작).
            Outbound::Event(ev) => match event_json(&ev) {
                Some(text) => Frame::Text(text),
                None => return Ok(()),
            },
            Outbound::Binary(b) => Frame::Binary(b),
            Outbound::Close(reason) => Frame::Close(reason),
        };
        // 논블록만 — 이 trait 은 sync 라 await 가 불가능하다. 큐 포화/닫힘 → SinkError 이고,
        //   종료 신호는 FrameSink 구현이 울린다(carrier 특정 처리).
        self.frames.try_send(frame).map_err(|_| CoreSinkError)
    }

    fn make_output_sink(&self) -> (Arc<dyn OutputSink>, Arc<AtomicBool>) {
        // handle_subscribe 가 코어 subscribe_from 에 넘길 output 평면 sink. 같은 FrameSink 를 공유해
        // control(이 sink)과 output 이 한 단일 writer 큐로 합류한다(FIFO). replay_dropped 플래그를
        // 함께 돌려 handle_subscribe 가 truncated 사후 보정에 쓰게 한다.
        let sink = Arc::new(FrameOutputSink::new(self.frames.clone()));
        let flag = sink.replay_dropped_flag();
        (sink, flag)
    }
}

// ── 연결 수명 훅 ────────────────────────────────────────────────────────────────

/// 연결 1개에 대응하는 에이전트측 핸들러. `ConnectionCore`(dispatch)와 그 연결의 수명 상태를 묶는다.
pub struct AgentConnection {
    core: Arc<ConnectionCore>,
    session: Arc<ConnectionSession>,
}

impl ConnectionHandler for AgentConnection {
    fn on_connect<'a>(
        &'a self,
        _conn_id: ConnId,
        frames: &'a Arc<dyn FrameSink>,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            // Hello + 현재 목록을 단일 writer 큐로 push — 이후 모든 출력과 FIFO 정렬된다.
            // ★2프레임 고정★: 이 호출 시점엔 큐 소비자가 없고 여유도 status fanout 과 나눠 쓰므로
            //   상한을 숫자로 못 잡는다(포트 계약). 그래서 상수 개수만 넣는다.
            if let Some(text) = event_json(&hello_event(env!("CARGO_PKG_VERSION").into())) {
                let _ = frames.send(Frame::Text(text)).await;
            }
            if let Some(text) = event_json(&agent_list_event(self.core.manager())) {
                let _ = frames.send(Frame::Text(text)).await;
            }
        })
    }

    fn on_text<'a>(
        &'a self,
        conn_id: ConnId,
        text: &'a str,
        frames: &'a Arc<dyn FrameSink>,
    ) -> BoxFuture<'a, ConnFlow> {
        let sink = FrameOutboundSink::new(frames.clone());
        Box::pin(async move {
            match serde_json::from_str::<AgentCommand>(text) {
                Ok(cmd) => match self.core.dispatch(cmd, &self.session, &sink).await {
                    DispatchFlow::Close => ConnFlow::Close,
                    DispatchFlow::Continue => ConnFlow::Continue,
                },
                Err(e) => {
                    tracing::warn!(conn = conn_id, "명령 파싱 실패: {e}");
                    let _ = sink.enqueue(Outbound::event(AgentEvent::Error {
                        request_id: None,
                        message: format!("invalid command: {e}"),
                    }));
                    ConnFlow::Continue
                }
            }
        })
    }

    fn on_binary<'a>(
        &'a self,
        conn_id: ConnId,
        _payload: &'a [u8],
        frames: &'a Arc<dyn FrameSink>,
    ) -> BoxFuture<'a, ConnFlow> {
        let sink = FrameOutboundSink::new(frames.clone());
        Box::pin(async move {
            // 클라→데몬 binary 는 프로토콜에 없음 — 내용을 보지 않고 오류로 보고 종료한다.
            tracing::warn!(conn = conn_id, "예상치 못한 binary frame — close");
            let _ = sink.enqueue(Outbound::event(AgentEvent::Error {
                request_id: None,
                message: "unexpected binary frame".into(),
            }));
            let _ = frames.send(Frame::Close("protocol error".into())).await;
            ConnFlow::Close
        })
    }

    fn on_disconnect(&self, conn_id: ConnId) {
        let manager = self.core.manager();

        // ── 구독 누수 방지 ──────────────────────────────────────────────────────
        // 이 연결이 등록한 모든 (agent_id, sink_id) 를 manager 에서 unsubscribe. 안 하면 죽은 큐로
        // 영원히 try_send 하는 좀비 sink 가 코어 subscribers 에 남는다(코어가 try_send 실패로 결국
        // 제거하긴 하나, 다음 emit 까지 잔존 — 명시적으로 끊는다).
        //
        // ★알려진 경쟁 — 이 스냅샷이 구독 하나를 놓칠 수 있다(HEAD 도 동일, 이 슬라이스 범위 밖)★:
        //   `handle_subscribe` 는 코어에 sink 를 등록한 **뒤** `subs` 에 기록한다. 그 두 단계 사이엔
        //   `.await` 가 없어 *취소*는 못 끼어들지만, ① 다른 워커에서 도는 read_task 가 아직 그 구간에
        //   있는 동안 이 정리가 스냅샷을 뜨거나(`ConnectionHandler::on_disconnect` 문서의 abort-겹침
        //   경쟁 — 같은 뿌리) ② 그 구간에서 패닉하면(debug 빌드) 코어에만 남고 여기엔 안 잡힌다.
        // ★결과 2가지★: (a) 좀비 sink 가 다음 emit 까지 잔존(코어의 dead-sink 제거로 자연 회복)
        //   (b) 그 sink 가 `Sender<Frame>` 사본을 붙들어 **write_task 의 송신단-드롭 자기종료가 깨진다**
        //   — `handle_connection` 의 select! 가 write 쪽 `abort()` 를 지워선 안 되는 이유(거기 주석).
        let leftovers: Vec<(AgentId, SinkId)> = {
            let guard = self.session.subs.lock().expect("subs poisoned");
            guard.iter().map(|(a, s)| (*a, *s)).collect()
        };
        for (agent_id, sink_id) in leftovers {
            let _ = manager.unsubscribe(agent_id, sink_id);
        }

        // ── 멀티뷰어 cleanup ───────────────────────────────────────────────────
        // (a) viewport 재협상: 끊긴 연결의 viewport 들을 맵에서 빼고, 영향받은 agent 를 남은 뷰어 기준
        //     smallest 로 다시 resize 한다(tmux detach 후 잔여 클라 기준으로 다시 키우는 것과 동일).
        //     ★lock 순서★: remove_conn_viewports 가 multiview lock 안에서 협상값만 계산해 반환한 뒤
        //     lock 을 푼 상태에서 manager.resize 를 부른다(lock 보유 중 코어 호출 금지).
        let owned: Vec<(AgentId, String)> = {
            let g = self
                .session
                .owned_viewports
                .lock()
                .expect("owned_viewports poisoned");
            g.clone()
        };
        if !owned.is_empty() {
            for (agent_id, negotiated) in self.core.multiview().remove_conn_viewports(&owned) {
                if let Some((cols, rows)) = negotiated {
                    // 남은 뷰어가 있으면 그 smallest 로 복귀. 없으면(None) 그대로 둔다(마지막 크기 유지).
                    let _ = manager.resize(agent_id, cols, rows);
                }
            }
        }
        // (b) 입력 lease 자동 해제: 보유자가 끊기면 다른 뷰어가 영영 막히면 안 된다(좀비 lock 방지).
        //     해제된 agent 는 이제 lease 가 비었으니 InputLeaseChanged{held:false} 를 전 연결에 통보.
        for agent_id in self.core.multiview().release_all_for_conn(conn_id) {
            broadcast_lease_changed(self.core.fanout(), agent_id, false);
        }
    }
}

/// 연결마다 `AgentConnection` 을 조립하는 공장. 전 연결이 공유하는 실물(manager·multiview·
/// 팬아웃 포트·control_registry·messaging 슬롯·shutdown 신호)을 한 번만 들고, 소켓이 설 때마다
/// 그것들로 연결 1개짜리 `ConnectionCore` 를 묶는다.
pub struct AgentConnections {
    manager: Arc<AgentManager>,
    multiview: MultiViewState,
    /// 전-연결 브로드캐스트 출구(ADR-0129). 네트워크 행의 연결 레지스트리가 조립 시점에 꽂히지만,
    ///   이 행이 표현할 수 있는 것은 "전부에게 이 text" 뿐이다.
    // ADR-0129
    fanout: Arc<dyn FrameFanout>,
    /// ADR-0096: 봉투 포맷 전역 상태 거처 — SetEnvelopeFormat dispatch 가 쓴다. handle_send(MCP/CLI)가
    ///   relay 마다 읽는 그 **같은 Arc**(전역 상태 하나).
    // ADR-0096
    control_registry: Arc<crate::control::registry::ControlRegistry>,
    /// ADR-0116 결정 3: `DeleteProfile` dispatch 가 삭제 정리를 부를 메시징 커널 슬롯(늦은 주입 —
    ///   서비스는 manager 조립 후에 생긴다).
    // ADR-0116
    messaging: Arc<crate::control::mcp_server::MessagingSlot>,
    /// StopDaemon 수신 시 main 종료를 트리거하는 watch.
    shutdown_tx: watch::Sender<bool>,
}

impl AgentConnections {
    pub fn new(
        manager: Arc<AgentManager>,
        multiview: MultiViewState,
        fanout: Arc<dyn FrameFanout>,
        control_registry: Arc<crate::control::registry::ControlRegistry>,
        messaging: Arc<crate::control::mcp_server::MessagingSlot>,
        shutdown_tx: watch::Sender<bool>,
    ) -> Self {
        Self {
            manager,
            multiview,
            fanout,
            control_registry,
            messaging,
            shutdown_tx,
        }
    }
}

impl ConnectionHandlerFactory for AgentConnections {
    fn handler_for(&self, conn_id: ConnId) -> Arc<dyn ConnectionHandler> {
        // ConnectionCore 는 연결마다 새로 묶는다 — 안에 든 manager/multiview/fanout/shutdown_tx 는
        //   전 연결이 공유하나, dispatch 호출 경로를 연결 단위로 캡슐화한다.
        let core = Arc::new(ConnectionCore::new(
            self.manager.clone(),
            self.multiview.clone(),
            self.fanout.clone(),
            self.control_registry.clone(),
            self.messaging.clone(),
            self.shutdown_tx.clone(),
        ));
        Arc::new(AgentConnection {
            core,
            session: Arc::new(ConnectionSession::new(conn_id)),
        })
    }

    fn handshake_error_frame(&self, message: &str) -> Option<String> {
        event_json(&AgentEvent::Error {
            request_id: None,
            message: message.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_doubles::FakeFrameSink;
    use tokio::sync::mpsc;

    /// 프레임 출구 더블. 네트워크 행 실물(`engram_dashboard_net::ws::ConnFrameSink`)은 이제
    /// **부를 수도 없다** — 슬라이스 1 로 crate 가 갈리며 그 타입이 네트워크 crate 내부
    /// (`pub(crate)`)에 남았다(ADR-0129). 그래서 이 행의 테스트는 포트 계약(`frame_port`)에만
    /// 의존하는 더블을 쓴다.
    fn frame_sink(tx: mpsc::Sender<Frame>) -> Arc<dyn FrameSink> {
        Arc::new(FakeFrameSink::new(tx))
    }

    // ── 1. FrameOutputSink 가 conn_tx 에 binary frame 을 넣는지 ─────────────────
    #[tokio::test]
    async fn frame_output_sink_encodes_and_sends_binary() {
        let (tx, mut rx) = mpsc::channel::<Frame>(8);
        let sink = FrameOutputSink::new(frame_sink(tx));
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
            Frame::Binary(buf) => {
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

    // ── 1b. (S15 B7) Event(구조화) payload 를 tag1 frame 으로 인코딩하는지 ──────
    //    합성 OutputEvent → send → conn_tx 의 Binary 를 decode_frame 으로 풀어 tag1·헤더 확인 후,
    //    payload 를 다시 wire StructuredEvent 로 serde 파싱해 필드가 보존됐는지 단언(ADR-0045 self-describing).
    #[tokio::test]
    async fn frame_output_sink_encodes_event_as_tag1_structured_frame() {
        use engram_dashboard_core::agent::types::OutputEvent as CoreOutputEvent;
        use engram_dashboard_protocol::{
            decode_frame, StructuredEvent as WireStructuredEvent, FRAME_TAG_STRUCTURED_EVENT,
        };

        let (tx, mut rx) = mpsc::channel::<Frame>(8);
        let sink = FrameOutputSink::new(frame_sink(tx));
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
            Frame::Binary(buf) => {
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

    // ── 2. full → SinkError + replay_dropped ──────────────────────────────────
    #[tokio::test]
    async fn frame_output_sink_full_returns_error_and_marks_replay_dropped() {
        // cap 1 채널을 가득 채운 뒤: send 가 Err 를 반환하고 replay_dropped 가 set 되는지.
        // ★큐 포화의 out-of-band 종료 신호는 여기 관심사가 아니다★: 그건 프레임 출구 **구현**의 계약이고,
        //   그걸 지키는 테스트는 `impl FrameSink for ConnFrameSink` 옆에 있다(★테스트 더블 쪽이 아니다★ —
        //   더블은 종료 신호를 울리지 않는다). 테스트 함수명 대신 impl 블록을 가리키는 이유는 함수명이
        //   개명·crate 분리 때 끊긴 참조가 되기 때문. 이 행이 책임지는 것은 `FrameSink` 가 준 Err 를
        //   SinkError 로 올리고 replay 구간 사후 보정 플래그를 세우는 것까지다.
        let (tx, mut rx) = mpsc::channel::<Frame>(1);
        let sink = FrameOutputSink::new(frame_sink(tx));
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

        // replay 구간 사후 보정용 플래그도 set.
        assert!(
            replay_dropped.load(Ordering::Acquire),
            "drop 시 replay_dropped set"
        );

        // 큐 첫 항목은 Binary(첫 frame).
        assert!(matches!(rx.recv().await.unwrap(), Frame::Binary(_)));
    }

    // ── 3. control 평면: Outbound 3종이 프레임 3종으로 인코딩되는지 ───────────────
    #[tokio::test]
    async fn frame_outbound_sink_maps_outbound_to_frames() {
        let (tx, mut rx) = mpsc::channel::<Frame>(8);
        let sink = FrameOutboundSink::new(frame_sink(tx));
        sink.enqueue(Outbound::event(AgentEvent::Error {
            request_id: None,
            message: "boom".into(),
        }))
        .expect("event ok");
        sink.enqueue(Outbound::Binary(vec![1, 2, 3]))
            .expect("binary ok");
        sink.enqueue(Outbound::Close("bye".into()))
            .expect("close ok");

        match rx.recv().await.unwrap() {
            Frame::Text(s) => assert!(s.contains("boom"), "Error 가 JSON text 로"),
            other => panic!("Text 여야 함: {other:?}"),
        }
        assert!(matches!(rx.recv().await.unwrap(), Frame::Binary(_)));
        match rx.recv().await.unwrap() {
            Frame::Close(r) => assert_eq!(r, "bye"),
            other => panic!("Close 여야 함: {other:?}"),
        }
    }
}
