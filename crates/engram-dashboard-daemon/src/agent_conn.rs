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

// ── 출력 평면 sink ────────────────────────────────────────────────────────────────

/// 한 연결의 한 에이전트 구독에 대응하는 `OutputSink`. pump 스레드가 `send` 를 호출한다.
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
        match self.frames.try_send(Frame::Binary(buf)) {
            Ok(()) => Ok(()),
            Err(_) => {
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
            // 직렬화 실패는 drop(event_json 동작).
            Outbound::Event(ev) => match event_json(&ev) {
                Some(text) => Frame::Text(text),
                None => return Ok(()),
            },
            Outbound::Binary(b) => Frame::Binary(b),
            Outbound::Close(reason) => Frame::Close(reason),
        };
        self.frames.try_send(frame).map_err(|_| CoreSinkError)
    }

    fn make_output_sink(&self) -> (Arc<dyn OutputSink>, Arc<AtomicBool>) {
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

/// `on_text` 이 명령 디코드에 실패한 뒤에만 부른다(ADR-0129 0-4).
///
/// ★왜 필요한가★: 인증은 이미 네트워크 행이 첫 프레임에서 끝냈으므로 그 뒤에 오는 핸드셰이크는
/// **프로토콜 위반**이다. 0-4 로 그 모양이 네트워크 lib 소유가 되면서 명령으로 디코드되지 않으니
/// 여기서 되잡는다.
///
/// ★온전함이 아니라 **태그**로 판정한다★: 필드가 빠졌거나 본문 모양이 어긋난 Auth 프레임도 핸드셰이크로
/// 센다. 온전한 것만 세면 낡은 클라가 ``unknown variant `Auth` `` 를 받는데, 이 서버가 첫 프레임으로
/// 유일하게 받아 주는 것이 Auth 라서 그 답은 디버깅을 반대 방향으로 보낸다.
/// ★위층 어휘가 늘지 않는다★: 최상위 키 하나만 보므로 명령 목록을 알 필요가 없고, 본문은
/// `IgnoredAny` 로 건너뛰어 값 트리를 만들지 않는다 — 피어가 프레임마다 도달시킬 수 있는 경로라
/// 할당을 얹지 않는다.
fn is_handshake_frame(text: &str) -> bool {
    /// ★태그 문자열의 정본은 `engram_dashboard_net::auth::AuthFrame`★ — 아래
    /// `handshake_frame_is_recognized_after_auth` 가 그 타입을 직렬화한 바이트로 이 판정을 걸어,
    /// 두 쪽이 갈라지면 깨지게 한다.
    #[derive(serde::Deserialize)]
    enum HandshakeTag {
        Auth(serde::de::IgnoredAny),
    }
    serde_json::from_str::<HandshakeTag>(text).is_ok()
}

impl ConnectionHandler for AgentConnection {
    fn on_connect<'a>(
        &'a self,
        _conn_id: ConnId,
        frames: &'a Arc<dyn FrameSink>,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
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
                // ★연결을 닫지 않는다★: Error 한 줄만 내고 살려 둔다 — 회귀 방어 `tests/ws_e2e.rs`
                //   case21 이 후속 명령이 여전히 응답되는지까지 본다.
                // ★순서가 이렇다★: 명령 파싱이 **먼저**다. 태그가 겹치지 않아 결과는 순서와 무관하지만,
                //   정상 경로(명령)에 프레임마다 추가 파싱을 얹지 않으려는 것이다.
                Err(_) if is_handshake_frame(text) => {
                    tracing::warn!(conn = conn_id, "인증 완료된 연결에 2차 핸드셰이크 — 거절");
                    let _ = sink.enqueue(Outbound::event(AgentEvent::Error {
                        request_id: None,
                        message: "already authenticated".into(),
                    }));
                    ConnFlow::Continue
                }
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
            // 클라→데몬 binary 는 프로토콜에 없다.
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
        // 안 하면 죽은 큐로 영원히 try_send 하는 좀비 sink 가 코어 subscribers 에 남는다(코어가
        // try_send 실패로 결국 제거하긴 하나, 다음 emit 까지 잔존 — 명시적으로 끊는다).
        //
        // ★알려진 경쟁 — 이 스냅샷이 구독 하나를 놓칠 수 있다★:
        //   `handle_subscribe` 는 코어에 sink 를 등록한 **뒤** `subs` 에 기록한다. 그 두 단계 사이엔
        //   `.await` 가 없어 *취소*는 못 끼어들지만, ① 다른 워커에서 도는 read_task 가 아직 그 구간에
        //   있는 동안 이 정리가 스냅샷을 뜨거나(`ConnectionHandler::on_disconnect` 문서의 abort-겹침
        //   경쟁) ② 그 구간에서 패닉하면(debug 빌드) 코어에만 남고 여기엔 안 잡힌다.
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
        // (a) viewport 재협상 — 남은 뷰어 기준 smallest 로 다시 resize(tmux detach 후 잔여 클라
        //     기준으로 다시 키우는 것과 동일).
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
                    let _ = manager.resize(agent_id, cols, rows);
                }
                // None = 남은 뷰어 없음 → 그대로 둔다(마지막 크기 유지).
            }
        }
        // (b) 입력 lease 자동 해제.
        for agent_id in self.core.multiview().release_all_for_conn(conn_id) {
            broadcast_lease_changed(self.core.fanout(), agent_id, false);
        }
    }
}

pub struct AgentConnections {
    manager: Arc<AgentManager>,
    multiview: MultiViewState,
    // ADR-0129
    fanout: Arc<dyn FrameFanout>,
    // ADR-0096
    control_registry: Arc<crate::control::registry::ControlRegistry>,
    // ADR-0116
    messaging: Arc<crate::control::mcp_server::MessagingSlot>,
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

    /// 네트워크 행 실물(`engram_dashboard_net::ws::ConnFrameSink`)은 네트워크 crate 내부
    /// (`pub(crate)`)라 **부를 수 없다**(ADR-0129) — 그래서 이 행의 테스트는 포트 계약
    /// (`frame_port`)에만 의존하는 더블을 쓴다.
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
    #[tokio::test]
    async fn frame_output_sink_encodes_event_as_tag1_structured_frame() {
        use engram_dashboard_core::agent::types::OutputEvent as CoreOutputEvent;
        use engram_dashboard_protocol::{
            decode_frame, StructuredEvent as WireStructuredEvent, FRAME_TAG_STRUCTURED_EVENT,
        };

        let (tx, mut rx) = mpsc::channel::<Frame>(8);
        let sink = FrameOutputSink::new(frame_sink(tx));
        let agent_id = uuid::Uuid::new_v4();
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
                assert_eq!(decoded.tag, FRAME_TAG_STRUCTURED_EVENT, "tag1 이어야 함");
                assert_eq!(decoded.agent_id, agent_id);
                assert_eq!(decoded.epoch, 3);
                assert_eq!(decoded.seq, 100);
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
        // ★큐 포화의 out-of-band 종료 신호는 여기 관심사가 아니다★: 그건 프레임 출구 **구현**의
        //   계약이라 `impl FrameSink for ConnFrameSink` 옆에서 지켜진다(★테스트 더블은 종료 신호를
        //   울리지 않는다★). 함수명 대신 impl 블록을 가리키는 것은 개명·crate 분리에 끊기지 않게.
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
        sink.send(frame(0)).expect("first ok");
        assert!(sink.send(frame(1)).is_err(), "full 이면 SinkError");
        assert!(
            replay_dropped.load(Ordering::Acquire),
            "drop 시 replay_dropped set"
        );

        assert!(matches!(rx.recv().await.unwrap(), Frame::Binary(_)));
    }

    // ── 2b. 2차 핸드셰이크 판별(ADR-0129 0-4) ────────────────────────────────────
    #[test]
    fn handshake_frame_is_recognized_after_auth() {
        let frame = engram_dashboard_net::auth::AuthFrame::Auth {
            token: "deadbeef".into(),
            protocol_version: 3,
        };
        assert!(is_handshake_frame(&serde_json::to_string(&frame).unwrap()));
        // 필드 순서·공백이 달라도 같은 프레임이다(발신자마다 직렬화기가 다르다 — 손조립 JS 도 있다).
        assert!(is_handshake_frame(
            r#"{ "Auth": { "protocol_version": 3, "token": "deadbeef" } }"#
        ));
    }

    #[test]
    fn malformed_handshake_frames_are_recognized_too() {
        for text in [
            r#"{"Auth":{"token":"deadbeef"}}"#, // protocol_version 누락
            r#"{"Auth":123}"#,                  // 본문이 객체가 아님
            r#"{"Auth":null}"#,
        ] {
            assert!(
                is_handshake_frame(text),
                "Auth 태그를 단 어긋난 프레임도 핸드셰이크다: {text}"
            );
        }
    }

    #[test]
    fn non_handshake_text_is_not_recognized() {
        // ★일반 파싱 오류 경로를 잠식하면 안 된다★: 아래가 하나라도 true 가 되면 case23(깨진 JSON →
        //   "invalid command")이 조용히 "already authenticated" 로 바뀐다.
        for text in [
            r#"{"NotACommand":true}"#,
            r#"{"ListAgents":{"request_id":"00000000-0000-0000-0000-000000000000"}}"#,
            r#""Auth""#,                 // 태그만 있고 본문이 없다 = 프레임이 아니다
            r#"{"Auth":1,"Kill":2}"#,    // 최상위 키가 하나가 아니다
            r#"{"auth":{"token":"x"}}"#, // 태그는 대소문자까지 wire 계약이다
            "not json at all",
            "",
        ] {
            assert!(
                !is_handshake_frame(text),
                "핸드셰이크가 아닌데 인정됐다: {text}"
            );
        }
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
