//! AgentSession — 에이전트 1개 = OutputCore(출력 측) + Box<dyn AgentTransport>(채널/자원 측) 합성.
//!
//! transport 종류(PTY/API)와 무관한 공용 표면을 노출하고, 내부에서 core/transport로 위임한다.
//!
//! 소유권 분할(impl-spec 표): master/child/shutdown/job/reader/writer → transport(PtyTransport) 안,
//!   subscribers/replay/seq/status/finalized → core(OutputCore) 안.
//!
//! 따라서 모든 메서드는 자기 필드(cols/rows atomic)를 만지거나 core/transport로 위임할 뿐이다.
//!
//! tauri import 0.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::backend::InputEncoder;
use crate::output_core::{CancelRequest, OutputCore};
use crate::queued_input::{overlay_unconfirmed, ListedRow, QueuedInputs, RowPhase};
use crate::session_id_latch::SessionIdLatch;
use crate::transport::AgentTransport;
use crate::types::{
    AgentId, AgentStatus, BackendCaps, CancelError, CancelOutcome, Capabilities, DeliveryAck,
    DeliveryAckState, DropCause, Incarnation, InputEvent, InputOrigin, MidTurnPolicy, OutputChunk,
    OutputEvent, OutputSink, PtyError, QueuedInputEvent, SinkId, SubscribeReply, TerminationIntent,
    TurnInput, Withdraw, WriteOutcome,
};

pub struct AgentSession {
    pub id: AgentId,
    pub cwd: PathBuf,
    pub epoch: u32,
    pub cols: AtomicU16,
    pub rows: AtomicU16,
    /// 유저 종료 의도. `Arc` 인 이유: finalize hook 클로저가 같은 값을 공유 캡처한다.
    intent: Arc<AtomicU8>,
    /// backend(프로그램)가 결정한 caps(session/model). manager.spawn 이 profile.command 로 산출해
    /// 주입한다 — transport 는 이 값을 모른다.
    backend_caps: BackendCaps,
    /// write_input 을 transport 로 넘기기 **직전** 적용하는 입력 인코딩(ADR-0044/0004).
    /// manager.spawn 이 산출해 주입한다.
    encoder: InputEncoder,
    /// 이 세션이 **편지를 읽는 주체**인가(= 우편 수신자 명단 자격). spawn 시 backend 가 산출한 값을
    /// 그대로 들고 있는다 — encoder 와 같은 부류의 backend 파생 사실이다.
    ///
    /// ★프로필이 아니라 **세션**이 드는 이유(load-bearing)★: `DeleteProfile` 은 산 세션을 죽이지
    ///   않는다. 프로필로 판정하면 프로필이 지워진 산 셸이 "모름" 이 되어 명단에 되돌아오고, 봉투가
    ///   명령으로 실행된다. 세션은 자기가 무엇으로 spawn 됐는지 알고 그 사실은 프로필 삭제로 안 변한다.
    reads_messages: bool,
    /// 본문 write 와 제출 write 사이의 대기 — 근거·값 출처는 `backend::SUBMIT_PACING`.
    ///
    /// ★기본값은 `new` 가 박는다(생성자 인자가 아니다)★: 호출자가 값을 고를 수 있게 하면 어느 조립
    ///   경로 하나가 0 을 넘기는 순간 이 결함이 조용히 재발한다. 낮추는 길은 테스트 전용 seam 하나뿐이다.
    submit_pacing: Duration,
    /// 위 대기를 실제로 재우는 함수. 운영은 블로킹 sleep 이고, 테스트는 **재우지 않고 호출만 기록**하는
    /// 것을 꽂아 "대기가 발행됐다" 를 시간 측정 없이(= 비플래키) 단언한다.
    sleeper: fn(Duration),
    /// 이 화신은 저장된 대화를 이어받으려고 떴다 — 화신 불변. 구독 응답이 `epoch` 과 함께 싣는다.
    /// ★화신 표식은 이 칸이 아니라 `epoch` 하나다★ — 여기에 표식을 함께 두면 표식이 둘이 된다.
    continues_conversation: bool,
    /// 이 화신의 세션 id 첫 제출 래치. `None` = 입력 두 동사의 제출 세기가 무동작이다.
    session_id_latch: Option<Arc<SessionIdLatch>>,
    /// 턴 도중 입력 정책 — backend 신고값. 기본값(`with_mid_turn` 을 안 부름) = `None`(오늘 경로).
    // ADR-0231
    mid_turn: MidTurnPolicy,
    /// backend 가 채우는 받음 알림 가능 여부 — `SpawnParts` 가 실어 온 바로 그 Arc.
    delivery_ack: Arc<DeliveryAck>,
    /// 세션 입력 자물쇠 — ★`SessionClassified` 의 사용자 입력과 취소만 잡는다★. 출력 펌프는 잡지 않는다: 이
    ///   자물쇠는 첫 제출 래치 commit(디스크 쓰기)을 품으므로, 펌프가 기다리면 출력이 디스크 I/O 뒤에 선다.
    ///   락 순서 = `input_order` → replay → 명부 → 대기 목록 표(emit 은 이것을 잡지 않으므로 역순이 없다).
    // ADR-0231
    input_order: Mutex<()>,
    core: Arc<OutputCore>,
    transport: Box<dyn AgentTransport>,
}

/// 배달 write 가 **실제로 나갔는지** 확인하기를 포기하는 시한([`AgentTransport::flush_input`]).
///
/// ★이 값이 재는 축★: 「아직 안 나갔다」와 「영영 안 나간다」를 가르는 자리다. 너무 짧으면 잠깐 바쁜
///   수신자(턴 렌더링 중이라 입력을 늦게 읽는 TUI)를 배달 실패로 신고해 **재배달이 중복을 만들고**,
///   너무 길면 물린 에이전트 하나가 배달 루프를 그만큼 붙잡는다. ★둘 중 더 나쁜 쪽은 중복이다★ —
///   실패는 재시도로 복구되지만 중복은 수신자에게 같은 편지를 두 번 읽힌다. 그래서 넉넉한 쪽으로 기울였다.
/// ★값의 출처 = 이 저장소가 「이쯤이면 끝났어야 한다」에 이미 쓰는 숫자★: kill 인과의 `join_pump` 상한이
///   5 초다(ADR-0001). 새 숫자를 발명하는 대신 같은 자리를 쓴다 — 이 경로도 판정이 같다("5 초가 지나도
///   안 나갔으면 그건 물린 것이다").
/// ★이 대기를 타는 경로는 우편 배달 하나뿐이다★ — 그 경로는 [`crate::backend::SUBMIT_PACING`] 때문에
///   이미 호출자를 0.5 초 붙잡는 동사라, 여기서 기다리는 것이 **새로운 종류의 블로킹이 아니다.** 키 입력
///   경로(`write_input`)는 이 대기를 타지 않는다.
const INPUT_FLUSH_BUDGET: Duration = Duration::from_secs(5);

/// 운영 sleeper — 이 층은 동기 경로라 그냥 블로킹으로 잔다(배달 루프가 그만큼 붙잡히는 것은 수용한 성질:
/// 터미널 수신은 시연 용도이고, 비동기 분리는 구조 변경이라 별도 결정 사항이다).
fn blocking_sleep(d: Duration) {
    std::thread::sleep(d);
}

impl AgentSession {
    /// 합성 세션 생성. **start는 여기서 호출하지 않는다** — manager가 new 이전에
    /// `transport.start(core.clone())`를 직접 부른다(impl-spec: 테스트 가시성·spawn 흐름 명시성).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: AgentId,
        cwd: PathBuf,
        epoch: u32,
        cols: u16,
        rows: u16,
        intent: Arc<AtomicU8>,
        backend_caps: BackendCaps,
        encoder: InputEncoder,
        reads_messages: bool,
        core: Arc<OutputCore>,
        transport: Box<dyn AgentTransport>,
    ) -> Self {
        Self {
            id,
            cwd,
            epoch,
            cols: AtomicU16::new(cols),
            rows: AtomicU16::new(rows),
            intent,
            backend_caps,
            encoder,
            reads_messages,
            submit_pacing: crate::backend::SUBMIT_PACING,
            sleeper: blocking_sleep,
            continues_conversation: false,
            session_id_latch: None,
            mid_turn: MidTurnPolicy::None,
            delivery_ack: Arc::new(DeliveryAck::new()),
            input_order: Mutex::new(()),
            core,
            transport,
        }
    }

    /// 화신 사실을 싣는다 — 기본값(부르지 않음) = 이어받기 아님.
    // ADR-0226
    pub(crate) fn with_incarnation(mut self, continues_conversation: bool) -> Self {
        self.continues_conversation = continues_conversation;
        self
    }

    /// 세션 id 첫 제출 래치를 싣는다 — 기본값(부르지 않음) = 래치 없음.
    ///
    /// ★운영 조립점에서 빠뜨리면 모든 이어받기가 조용히 사라진다★ — 제출이 한 번도 안 세어져 어느
    ///   화신도 id 를 영속하지 못하고, 다음 활성화가 전부 새 대화가 된다(오류는 없다).
    // ADR-0226
    pub(crate) fn with_session_id_latch(mut self, latch: Arc<SessionIdLatch>) -> Self {
        self.session_id_latch = Some(latch);
        self
    }

    /// 턴 도중 입력 정책과 받음 알림 가능 여부를 싣는다 — 기본값(부르지 않음) = `None` · `Unknown`.
    ///
    /// ★운영 조립점에서 빠뜨리면 목록이 조용히 사라진다★ — 모든 입력이 오늘 경로로 가고, 오류는 없다.
    // ADR-0231
    pub(crate) fn with_mid_turn(
        mut self,
        mid_turn: MidTurnPolicy,
        delivery_ack: Arc<DeliveryAck>,
    ) -> Self {
        self.mid_turn = mid_turn;
        self.delivery_ack = delivery_ack;
        self
    }

    /// ★테스트 전용 seam★ — 제출 대기를 낮추거나(하네스가 0.5초씩 자지 않게) 대기 발행 자체를 관측한다.
    /// **운영 기본값은 언제나 `backend::SUBMIT_PACING`**(위 필드 doc) — 이 함수는 그 기본값을 바꾸지 않고,
    /// 테스트가 자기 인스턴스에 대해서만 명시로 내린다.
    #[cfg(test)]
    pub(crate) fn with_submit_pacing(mut self, pacing: Duration, sleeper: fn(Duration)) -> Self {
        self.submit_pacing = pacing;
        self.sleeper = sleeper;
        self
    }

    /// 이 세션이 편지를 읽는 주체인가 — 판정 근거는 필드 doc.
    pub fn reads_messages(&self) -> bool {
        self.reads_messages
    }

    /// 이 화신의 대기 입력 명부 — 코어가 emit 에서 먹이는 **바로 그** Arc 다.
    /// ★세션에 따로 쥐지 않고 코어에서 꺼낸다★: 두 곳에 따로 꽂으면 세션이 읽는 명부와 코어가 먹이는 명부가
    ///   갈릴 수 있는데, 그 어긋남은 컴파일도 오류도 없이 빈 목록으로만 보인다.
    /// ★가드를 쥔 채 emit 하지 말 것★ — 락 순서가 replay → 명부다(`OutputCore::queued_inputs`).
    // ADR-0231
    pub fn queued_inputs(&self) -> &Arc<QueuedInputs> {
        self.core.queued_inputs()
    }

    /// 유저 종료 의도 태깅(ADR-0019) — kill_agent 가 transport.shutdown **전에** 호출한다.
    /// finish hook 이 이 값을 finish 순간 snapshot 하므로, shutdown 전에 set 해야 pump 가
    /// 깨어 finish 할 때 UserKill 이 관측된다(순서가 race 방지의 핵심).
    pub fn set_intent(&self, intent: TerminationIntent) {
        self.intent.store(intent as u8, Ordering::SeqCst);
    }

    /// 지금까지 관측된 종료 의도 — ★단조 래치로 읽으라고 있는 값이다★.
    ///
    /// ★왜 읽는 동사가 필요했나★: 활성화 판정이 **사용자 취소를 종점 상태보다 먼저** 알아야 한다.
    ///   `kill_agent` 는 이 값을 `enter_exiting` 보다도 앞에 세우고([`AgentManager::kill_agent`]),
    ///   그 뒤에야 통로를 내린다. 그래서 kill 로 인한 어떤 관측(통로의 배달 중단 · 배달 채널 닫힘 ·
    ///   `Exiting` 상태)보다 이 래치가 **반드시 먼저** 보인다 — 종점 전이(`Killed`)는 pump 가 깨어날
    ///   때까지 안 서므로 그것만 보는 판정은 그 사이를 「연결 실패」로 오독한다.
    /// ★`Ordering::SeqCst` 로 읽는다★ — 쓰는 쪽(`set_intent`)과 같은 순서라 짝이 맞는다.
    /// ★화신마다 새 값이다★ — `Arc<AtomicU8>` 을 `spawn_session` 이 화신마다 새로 만든다. 그래서 앞
    ///   화신의 kill 의도가 이 화신에 보이지 않는다.
    // ADR-0019
    pub fn termination_intent(&self) -> TerminationIntent {
        TerminationIntent::from_u8(self.intent.load(Ordering::SeqCst))
    }

    /// pump 기동을 위임(transport.start). ★ADR-0019 reaper 순서★: manager 는 이 세션을 sessions
    /// 맵에 **insert 한 뒤** start 한다. pump 가 즉시 EOF→finish→ReapMsg 를 보내도 그땐 이미 맵에
    /// 존재하므로 reaper 가 정상 reap 한다(insert 전 start 면 hook send 가 맵에 없는 id 를 가리켜
    /// 좀비). attach_pump 는 start 내부 동기 완료라 insert 순서와 무관(join_pump 영향 없음).
    pub fn start_pump(&self) {
        self.transport.start(self.core.clone());
    }

    /// 입력 바이트 전달 → (encoder 적용) → transport.
    ///
    /// ★배선 지점(ADR-0044)★: encoder를 적용해 텍스트 턴을 백엔드 규약대로 감싼 뒤
    ///   **항상 Raw 바이트**로 transport에 넘긴다. transport는 바보 파이프라 형태를 모른다.
    ///   - Raw(터미널·shell): `encode`가 바이트를 그대로 복사 → 기존 경로와 **바이트 동일**.
    ///   - ClaudeStreamJson(json 모드): 텍스트를 claude 유저 JSON 라인으로 감싼다(escape·스키마는
    ///     backend/claude/ 단독 — session은 태그만 들고 형태를 모른다, ADR-0004 격리).
    ///
    /// ★호출 계약(FIX 6a) — json 모드에서 `1 write_input 호출 == 완결된 유저 턴 1개`★:
    ///   ClaudeStreamJson 인코더는 매 호출을 `{"type":"user",…}\n` 라인 **하나**로 감싼다. 즉 호출
    ///   1회당 claude 는 유저 턴 1개를 통째로 받는다. 터미널 경로처럼 **키 입력 1글자씩** 호출하면
    ///   글자마다 한 글자짜리 잘못된 턴이 만들어져 대화가 깨진다. 따라서 json 모드 호출자(RichSlot·M2)는
    ///   **완성된 메시지 전체를 한 번에** 보내야 한다(부분 입력 누적은 프론트 입력창 몫). 터미널 경로는
    ///   Raw 라 기존대로 스트리밍 바이트 호출이 정상(이 계약은 json 모드 한정).
    /// 출처 = `Mail`([`AgentSession::write_input_observed`] 와 같다) — 사람의 입력은
    /// [`AgentSession::write_input_from`] 으로 출처를 싣는다.
    pub fn write_input(&self, bytes: &[u8]) -> Result<(), PtyError> {
        self.write_input_observed(bytes).map(|_| ())
    }

    /// 출처를 실은 쓰기 — 동작·반환·바이트 계약은 [`AgentSession::write_input_observed`] 와 같고, 차이는
    /// `origin` 이 정책 갈래에 닿는다는 것뿐이다.
    ///
    /// ★정책 `None` 이면 `origin` 은 아무것도 바꾸지 않는다★ — 두 출처 모두 오늘 경로다(바이트 동일 · 목록
    ///   사건 없음). `TransportOwned` 는 본문과 출처를 통로 턴 동사로 넘긴다 — 분류는 통로가 한다.
    ///   `SessionClassified` 는 `User` 만 분류하고(아래 [`Self::write_user_classified`]) `Mail` 은 오늘 경로다.
    // ADR-0231
    pub fn write_input_from(
        &self,
        bytes: &[u8],
        origin: InputOrigin,
    ) -> Result<WriteOutcome, PtyError> {
        match (self.mid_turn, origin) {
            (MidTurnPolicy::None, _) => self.write_now(bytes, None),
            (MidTurnPolicy::SessionClassified { .. }, InputOrigin::User) => {
                self.write_user_classified(bytes)
            }
            (MidTurnPolicy::SessionClassified { .. }, InputOrigin::Mail) => {
                self.write_now(bytes, None)
            }
            (MidTurnPolicy::TransportOwned, _) => self.write_now(bytes, Some(origin)),
        }
    }

    /// `SessionClassified` 의 사용자 입력 — 입력 자물쇠를 쥔 채 분류 → 목록 사건 → 쓰기가 한 덩어리로 돈다.
    ///
    /// ★한가하거나 받음 불가 판명(`Unavailable`)이면 오늘 경로다(쓰기 + 합성 에코)★. 한가 = 대기 목록 표 빔 ∧
    ///   턴 관측이 턴 아님([`OutputCore::classified_input_busy`]) — 오류 뒤 멈춤은 읽지 않는다(멈춤 중 친 글도
    ///   한가면 오늘 경로이고, 그 턴이 멈춤을 푼다).
    /// 그 밖은 목록 갈래이고 합성 에코를 내지 않는다(말풍선은 벤더의 받음 자리에 선다):
    ///   - `Available` — `Queued` 를 쓰기 **앞**에 낸다: 수명주기는 쓴 뒤 1 ms 안에 와서, 뒤에 내면 받음이 모르는
    ///     id 로 버려진다. 쓰기 실패 = `Dropped{Rejected}` 를 내고 `Err`.
    ///   - `Unknown` — 쓰기가 성공한 **뒤에만** `Queued` 를 낸다. 앞에 내면 쓰기가 도는 사이 펌프가 받음 불가를
    ///     판명할 때 그 항목이 받음으로 닫히고, 이어 쓰기가 실패해도 `Dropped{Rejected}` 는 종결 묘비에 삼켜져
    ///     보내지 못한 글이 말풍선으로 남는다. 쓰기 실패 = 아무것도 안 내고 `Err`.
    /// ★자물쇠가 지키는 것★: 두 입력의 분류와 방출 순서가 어긋나지 않고, 같은 id 의 취소 줄은 이 쓰기가 돌아온
    ///   뒤에만 나간다([`Self::cancel_queued_input`]). 이 자물쇠 아래의 emit 은 구독자 fanout 을 쥔 채 돈다 —
    ///   `OutputSink::send` 가 막히지 않는다는 계약이 그것을 받친다(ADR-0006 의 예외).
    // ADR-0231
    fn write_user_classified(&self, bytes: &[u8]) -> Result<WriteOutcome, PtyError> {
        let _order = self
            .input_order
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let ack = self.delivery_ack.state();
        if ack == DeliveryAckState::Unavailable || !self.core.classified_input_busy() {
            return self.write_now(bytes, None);
        }
        // ADR-0226: 오늘 경로와 같은 자리 — 보내기 전에 센다.
        self.count_turn_submission(bytes)?;
        let msg_uuid = uuid::Uuid::new_v4();
        let id = msg_uuid.to_string();
        let encoded = self.encoder.encode(bytes, msg_uuid);
        let queued = OutputEvent::QueuedInput(QueuedInputEvent::Queued {
            id: id.clone(),
            text: String::from_utf8_lossy(bytes).into_owned(),
        });
        if ack == DeliveryAckState::Unknown {
            self.transport.send_input(InputEvent::Raw(encoded))?;
            self.core.emit(queued);
        } else {
            self.core.emit(queued);
            if let Err(e) = self.transport.send_input(InputEvent::Raw(encoded)) {
                self.core
                    .emit(OutputEvent::QueuedInput(QueuedInputEvent::Dropped {
                        id,
                        cause: DropCause::Rejected,
                    }));
                return Err(e);
            }
        }
        let n = bytes.len();
        Ok(WriteOutcome {
            bytes_requested: n,
            bytes_written: n,
            msg_uuid,
            epoch: self.epoch,
        })
    }

    /// `write_input` 의 배달-경계 계측판(ADR-0088 Stage 0) — 성공 시 `WriteOutcome`(논리 메시지 바이트 +
    ///   이 턴의 `msg_uuid`)을 돌려준다. 동작·바이트는 `write_input` 과 **완전히 동일**하고(같은 본체),
    ///   차이는 **관측 산출물을 삼키지 않고 반환**하는 것뿐이다. 제어 채널 relay(ingress::handle_send)가
    ///   이 산출물로 배달 관측 레코드를 만든다("전송 실패" vs "모델 무시" 구별의 전제 — ADR-0088).
    ///
    /// ★완결성 = Ok-vs-Err★: `send_input` 이 `Ok(())` 를 돌려주면 요청 바이트가 **전량 수용**된 것이다
    ///   (부분 수용을 `Ok` 로 숨기지 않음). 전량 미수용은 이 함수가 `Err` 로 반환하지 `Ok` 로 축소
    ///   반환하지 않는다. `WriteOutcome` 의 바이트 필드는 완결성 판정 레버가 아니다(이유는
    ///   `WriteOutcome` 주석) — 완결성은 이 함수의 `Ok`/`Err` 로 본다.
    ///
    /// ★★그 「수용」의 뜻이 바뀌었다 — 옛 문장의 「내부 `write_all`」·「std write_all 계약」은 이제
    ///   거짓이다★★: 통로 셋(PTY·stdio·codex)이 전부 **유계 입력 큐**에 담고 전담 라이터 스레드가 빼서
    ///   쓴다. 그래서 `Ok` 는 「자식에게 갔다」가 아니라 **「전량을 순서까지 확정해 받았다」**이고,
    ///   ★받아 둔 뒤에 실패한 쓰기는 이 반환값으로 돌아오지 않는다★ — 나타나는 곳은 **다음 호출의
    ///   `Err`** 와 로그, 그리고 대개 곧 이어지는 종점 전이다.
    /// ★ADR-0088 의 배달 관측에 미치는 영향(정확히)★: 그 레코드가 가르던 「전송 실패 vs 모델 무시」에서
    ///   **「전송 실패」가 잡는 범위가 줄었다** — 이제 그 자리는 「받지도 않았다」(통로가 닫혔거나 상한
    ///   초과)를 잡고, 「받았는데 못 썼다」는 **그 다음** 배달이 `Err` 로 신고한다. 계약의 정본은
    ///   [`crate::transport::input_queue`] 모듈 헤더.
    /// 출처 = `Mail`(우편 배달 · 사람 아닌 호출자) — 턴 도중에도 대기 목록에 오르지 않는다.
    // ADR-0088
    // ADR-0231
    pub fn write_input_observed(&self, bytes: &[u8]) -> Result<WriteOutcome, PtyError> {
        self.write_input_from(bytes, InputOrigin::Mail)
    }

    /// 지금 보내는 쓰기 — 오늘 경로의 본체. `turn_origin` = `Some` 이면 통로 턴 동사로 출처와 함께 넘긴다
    /// (`TransportOwned`), `None` 이면 `send_input(Raw)` 다. 두 갈래가 넘기는 바이트는 같다.
    fn write_now(
        &self,
        bytes: &[u8],
        turn_origin: Option<InputOrigin>,
    ) -> Result<WriteOutcome, PtyError> {
        // ADR-0226: 턴을 여는 쓰기는 보내기 **전에** 센다 — 첫 턴이 상대에게 가기 전에 세션 id 가 영속된다.
        self.count_turn_submission(bytes)?;
        // ★이 유저 턴의 메시지 uuid(replay dedup 키)★: 한 write_input 당 하나 생성해 (a) stdin user
        //   라인(encode)과 (b) 입력-시점 합성 에코(input_echo_event) **양쪽에 같은 값**으로 넘긴다.
        //   json 모드에서 claude 가 replay 로 이 uuid 를 그대로 되울린다(실측). session 은 불투명 Uuid
        //   토큰만 알고 json 형태·uuid 부착 위치는 모른다(ADR-0004 격리 — 스키마 지식은
        //   backend/claude/ 단독). Raw(터미널) encoder 는 이 uuid 를 무시한다.
        let msg_uuid = uuid::Uuid::new_v4();
        let encoded = self.encoder.encode(bytes, msg_uuid);
        match turn_origin {
            None => self.transport.send_input(InputEvent::Raw(encoded))?,
            // ADR-0231: 통로가 목록 사건에 쓰는 id = 이 쓰기의 `msg_uuid` — 영수증과 목록이 같은 값을 가리킨다.
            Some(origin) => self.transport.send_turn(TurnInput {
                id: msg_uuid.to_string(),
                body: encoded,
                origin,
            })?,
        }

        // ★ADR-0044/0045 · 왜: 입력-시점 유저 에코★: 터미널(Raw)은 PTY 가 입력을 즉시 로컬 에코하지만,
        //   json(stream-json) 모드는 claude 가 `--replay-user-messages` 로 되울릴 때까지(왕복 지연)
        //   유저 메시지가 화면에 안 뜬다. 그래서 send_input **성공 후**, encoder 가 json 모드면 동일한
        //   유저 이벤트를 즉시 core.emit 해 터미널의 즉시 에코를 흉내낸다(체감 반응성). 이후 claude 가
        //   되울린 replay 중복은 프론트 accumulator 가 uuid 로 dedup 한다(같은 msg_uuid) — decoder 는
        //   억제하지 않고 uuid 를 실어 그대로 통과시킨다(backend/claude/). 과거/비매칭 uuid 의 user
        //   text(resume 재개분)는 dedup 되지 않아 전부 보존된다(vanish 회귀 제거).
        //   ★락 규율(ADR-0006)★: 새 락 없이 core.emit 재사용 — emit 이 replay/subscribers 락을 짧게만
        //   잡고 lock 밖 send 하는 규율을 그대로 탄다. send_input 성공 후 emit 이라 순서도 자연스럽다.
        if let Some(event) = self.encoder.input_echo_event(bytes, msg_uuid) {
            self.core.emit(event);
        }
        let n = bytes.len();
        Ok(WriteOutcome {
            bytes_requested: n,
            bytes_written: n,
            msg_uuid,
            epoch: self.epoch,
        })
    }

    /// `write_input_observed` + **제출**: 본문을 쓴 뒤, encoder 가 제출 바이트를 요구하면 그것을
    /// **별도 write** 로 한 번 더 낸다. 반환값·유저 에코·바이트 회계는 `write_input_observed` 와 동일하다
    /// (제출 바이트는 논리 메시지가 아니라 `WriteOutcome` 에 세지 않는다).
    ///
    /// ★왜 두 번 쓰나 · 왜 encoder 가 못 합치나★ = `InputEncoder::submit_sequence` 주석(실측 근거).
    /// ★두 write 사이에 `backend::SUBMIT_PACING` 만큼 잔다★: 나눠 쓰는 것만으로는 부족하고, 간격이
    ///   없으면 PTY 에서 한 덩이로 묶여 수신자가 한 번의 read 로 받는다(그 상수 doc — "0ms 로 된다" 는
    ///   옛 관측은 측정 오류였다). **그래서 이 동사는 호출 스레드를 그만큼 붙잡는다.**
    /// ★★그 간격은 **본문이 실제로 나간 뒤**부터 잰다 — 큐에 넣은 시각부터가 아니다★★: `send_input` 이
    ///   큐 넣기가 된 뒤로 넣은 시각부터 재면 실제 간격이 `대기 - 본문이 나가는 데 걸린 시간` 으로 줄고,
    ///   그 시간이 대기보다 길면 **0 이 된다**(두 write 가 한 덩이로 묶여 제출이 안 된다). 그래서 본체가
    ///   `confirm_written` 으로 본문의 OS 착지를 먼저 확인한 뒤에 잔다.
    /// ★한때 여기 「그 조건은 자식이 stdin 을 안 읽는다는 뜻이라 제출이 어차피 안 먹힌다」로 적혀 있었다 —
    ///   **그 변명은 틀렸다**★: 파이프·ConPTY 버퍼(4–64KiB)보다 큰 본문은 **멀쩡히 읽는 자식** 상대로도
    ///   여러 번의 write 주기가 걸리고, 큰 봉투를 보내는 우편 배달이 정확히 그 경로다. 즉 물린 에이전트가
    ///   아니라 **정상 배달**에서 간격이 무너진다. 그리고 간격은 그 임계를 넘기 전에도 이미 `본문 쓰기
    ///   시간` 만큼 **항상** 줄어 있었다 — 크기와 무관하게 방향이 틀린 것이다.
    /// ★왜 이 층인가★: transport 를 소유해 `send_input` 을 두 번 낼 수 있는 가장 낮은 층이 여기다. 위층
    ///   (manager·데몬 어댑터·메시징 커널)은 transport 를 모르고, 아래층(encoder)은 바이트열 하나를
    ///   돌려주는 계약이라 write 경계를 만들 수 없다.
    /// ★`write_input` 과 갈라 둔 이유★: 터미널 키 입력은 사람이 Enter 를 직접 친다 — 그 경로가 이 동사를
    ///   타면 키 한 번마다 턴이 제출된다. 이 동사는 "완성된 메시지 하나를 턴으로 넣는" 호출자(우편 배달)
    ///   전용이다.
    /// ★에러 계약★: 본문은 갔는데 제출 write 가 실패하면 `Err` 다 — 턴이 시작되지 않은 배달은 배달이
    ///   아니므로 상위(파킹·재시도)가 실패로 다뤄야 한다.
    pub fn submit_input_observed(&self, bytes: &[u8]) -> Result<WriteOutcome, PtyError> {
        let outcome = self.write_input_observed(bytes)?;
        if let Some(submit) = self.encoder.submit_sequence() {
            // ★★대기를 **본문이 실제로 나간 뒤**부터 잰다 — 이 한 줄이 아래 대기를 의미 있게 만든다★★:
            //   `send_input` 은 이제 큐에 넣고 즉시 돌아오므로, 넣은 시각부터 재면 실제 간격은
            //   `대기 - 본문이 나가는 데 걸린 시간` 으로 줄고 그 시간이 대기보다 길면 **0 이 된다.**
            //   그러면 두 write 가 한 덩이로 묶여 수신자가 한 번의 read 로 받아 **턴이 제출되지 않는다**
            //   (`backend::SUBMIT_PACING` 이 실측으로 기록한 그 결함). 벌려야 하는 것은 우리 호출 간격이
            //   아니라 **수신자의 read 경계**라, 기준점은 OS 로 나간 시각이어야 한다.
            self.confirm_written().map_err(|e| {
                tracing::warn!(
                    agent = %self.id,
                    epoch = self.epoch,
                    bytes = bytes.len(),
                    "본문 write 확인 실패 — 제출을 시도하지 않는다: {e}"
                );
                e
            })?;
            // ★이 대기가 제출의 일부다(빼면 제출되지 않는다 — 실측)★: 근거·값 출처·"0ms 로 된다" 는
            //   옛 관측이 왜 틀렸는지는 `backend::SUBMIT_PACING` doc.
            (self.sleeper)(self.submit_pacing);
            // ADR-0226: ★제출 CR 바로 앞에서 센다 — 맨 앞으로 올리지 말 것★. 턴을 여는 것은 이 CR 이고,
            //   이것은 `write_input_observed` 를 거치지 않고 나간다. 맨 앞에서 세면 본문 쓰기·착지 확인·
            //   대기가 영속과 턴 사이에 끼어, 그 창의 kill 이나 실패가 **턴 없는 영속**을 남긴다.
            self.count_turn_submission(submit)?;
            //
            // ★두 실패를 로그에서 가른다(본문도 못 감 vs 본문은 갔고 제출만 실패)★: 후자는 수신자
            //   입력창에 미제출 봉투가 남은 상태라, 상위의 무손실 재파킹이 다음 flush 에서 같은 봉투를
            //   그 위에 덧쓴다(한 턴에 두 벌). 입력창을 비우는 동사는 이 층의 계약 밖이라 지우지는
            //   못하고, 사람이 그 잔여물을 알아볼 수 있게 남기는 것이 여기서 할 수 있는 전부다.
            if let Err(e) = self.transport.send_input(InputEvent::Raw(submit.to_vec())) {
                tracing::warn!(
                    agent = %self.id,
                    epoch = self.epoch,
                    bytes = bytes.len(),
                    "본문은 썼으나 제출 write 실패 — 수신자 입력창에 미제출 봉투가 남았고 재시도가 그 위에 덧쓴다: {e}"
                );
                return Err(e);
            }
        }
        // ★★영수증의 뜻을 「받았다」에서 「나갔다」로 되돌리는 자리★★ — 이 확인이 없으면 큐에 들어가기만
        //   한 본문이 **배달 성공**으로 기록되고, ADR-0088 이 가르려는 두 경우("전송 실패" vs "모델이
        //   무시")가 정확히 뒤집힌다. 제출 경로가 있든(터미널) 없든(json) 마지막은 여기로 모인다.
        // ★제출 바이트까지 포함해 확인한다★: 제출이 안 나간 배달은 턴이 시작되지 않은 배달이고, 그것은
        //   위 `★에러 계약★` 이 이미 `Err` 로 정한 상태다.
        self.confirm_written().map_err(|e| {
            tracing::warn!(
                agent = %self.id,
                epoch = self.epoch,
                bytes = bytes.len(),
                "배달 write 확인 실패 — 영수증을 내지 않는다: {e}"
            );
            e
        })?;
        Ok(outcome)
    }

    /// 이 쓰기가 턴을 연다면([`InputEncoder::submits_turn`]) 세션 id 래치에 제출을 센다 — 호출자는 그 쓰기를
    /// **보내기 전에** 부르고, `Err` 면 보내지 않는다. 래치가 없으면 무동작이다.
    ///
    /// ★사용자 종료 중이면 세지도 보내지도 않는다(`Err`)★: 세고 보내면 대화 없는 id 가 영속되고, 세지만
    ///   않고 보내면 입력 큐가 닫히기 전에 받아들여진 턴이 죽어 가는 자식에게 넘어가 **영속되지 않은 id 의
    ///   대화**가 생길 수 있다. 결말은 큐가 닫힌 뒤의 `send_input` 과 같은 `WriteFailed` 다 — 그 오류를
    ///   종료 의도가 선 순간으로 앞당길 뿐이다.
    /// ★확인은 원자 읽기 하나다★ — 락을 잡지 않는다. 래치가 이미 영속을 마친 뒤에도 확인한다.
    // ADR-0226
    fn count_turn_submission(&self, bytes: &[u8]) -> Result<(), PtyError> {
        let Some(latch) = &self.session_id_latch else {
            return Ok(());
        };
        if !self.encoder.submits_turn(bytes) {
            return Ok(());
        }
        if self.termination_intent() == TerminationIntent::UserKill {
            tracing::info!(
                agent = %self.id,
                epoch = self.epoch,
                bytes = bytes.len(),
                "사용자 종료 중에 온 턴 제출이라 보내지 않는다 — 세션 id 영속도 하지 않는다"
            );
            return Err(PtyError::WriteFailed(
                "사용자가 이 에이전트를 종료하는 중이라 턴을 보내지 않았다".into(),
            ));
        }
        latch.note_submission();
        Ok(())
    }

    /// 받아 둔 입력이 실제로 나갔는지 통로에 확인한다. ★확인 수단이 **없다고 말하는** 통로는 그대로
    /// 통과시킨다★ — 그 통로들(codex app-server·시험대 seam)은 이 변경 **이전부터** 그 수준의 앎만
    /// 갖고 있었고, 여기서 `Err` 로 바꾸면 이번 변경이 고치려는 것과 무관한 배달을 새로 실패시킨다.
    ///
    /// ★그래서 이 함수는 두 가지를 **다르게** 다룬다★: 「확인해 봤는데 안 나갔다」(`WriteFailed`)는
    ///   실패이고, 「확인할 수단이 없다」(`Unsupported`)는 실패가 아니다. 뒤엣것을 실패로 접으면 정직해
    ///   보이지만 실제로는 멀쩡한 배달을 죽인다.
    /// ★알려진 잔여 — 시한 초과는 「안 나갔다」가 아니다★: [`INPUT_FLUSH_BUDGET`] 을 넘겨 `Err` 로
    ///   돌아가도 큐에 든 바이트는 **취소되지 않아** 늦게 나갈 수 있다. 그러면 상위의 재파킹이 같은
    ///   봉투를 한 번 더 배달한다(한 턴에 두 벌). 그 잔여를 없애려면 시한 초과 시 그 덩이를 큐에서
    ///   빼는 취소 동사가 필요한데, 그것은 「부분 배달을 어디까지 되돌릴 수 있나」라는 별개 질문이다.
    fn confirm_written(&self) -> Result<(), PtyError> {
        match self.transport.flush_input(INPUT_FLUSH_BUDGET) {
            Err(PtyError::Unsupported(_)) => Ok(()),
            other => other,
        }
    }

    /// 목록의 대기 입력 하나를 취소한다(명령 `agent.cancelQueuedInput` 의 세션 쪽).
    ///
    /// ★명부 확인이 먼저다★: 목록에 없는 id(모름 · 이미 결말) = `NotFound` · 이미 취소 대기 = 사건 없이
    ///   `Requested`(멱등). 목록을 쓰지 않는 세션(`None`)은 늘 `NotFound` 다 — 통로에 닿지 않는다.
    /// 정책별 갈래:
    ///   - `SessionClassified` — 입력 자물쇠 안에서 확인 → `CancelRequested` → 취소 줄 `send_input` →
    ///     `Requested`. 사용자 입력도 같은 자물쇠 안에서 쓰므로 그 id 의 취소 줄은 **글 쓰기가 돌아온 뒤에만**
    ///     나간다(취소가 글을 앞지르면 턴 도중엔 예약이 안 걸려 글이 전달되는데 화면은 취소를 믿는다).
    ///     ★확인과 `CancelRequested` 는 한 replay 구간이다★(`OutputCore::emit_cancel_request`) — 펌프의 결말이
    ///     확인 뒤에 끼면 그 항목은 `NotFound` 로 답하고 취소 줄도 쓰지 않는다. 기록 뒤에 온 결말(늦은 받음 등)은
    ///     `Requested` 그대로이고 명부가 결말을 정한다.
    ///     ★취소 줄 쓰기 실패 = `CancelFailed` 를 내고 `Err(Write)`★ — 항목은 취소 대기 그대로다(목록으로
    ///     되돌리면 모든 창에서 빠진 항목이 다시 그려진다).
    ///   - `TransportOwned` — 자물쇠 없이 확인 → `withdraw`: `Withdrawn` = `Cancelled` · `TooLate` = `Requested`
    ///     · `NotHeld` = `NotFound`(확인과 거두기 사이에 결말이 났다 — 그 결말 사건이 곧 명부에 선다). 목록
    ///     사건은 통로가 낸다.
    /// ★명부 가드를 쥔 채 emit 하지 않는다★ — 락 순서가 replay → 명부다(`TransportOwned` 의 확인은 복사해 곧바로
    ///   놓고, `SessionClassified` 의 확인은 코어가 replay 락 아래에서 한다).
    // ADR-0231
    pub fn cancel_queued_input(&self, id: &str) -> Result<CancelOutcome, CancelError> {
        match self.mid_turn {
            MidTurnPolicy::None => Err(CancelError::NotFound),
            MidTurnPolicy::SessionClassified { cancel_line } => {
                let _order = self
                    .input_order
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                match self.core.emit_cancel_request(id) {
                    CancelRequest::NotListed => return Err(CancelError::NotFound),
                    CancelRequest::AlreadyCancelling => return Ok(CancelOutcome::Requested),
                    CancelRequest::Recorded => {}
                }
                if let Err(e) = self.transport.send_input(InputEvent::Raw(cancel_line(id))) {
                    self.core
                        .emit(OutputEvent::QueuedInput(QueuedInputEvent::CancelFailed {
                            id: id.to_owned(),
                        }));
                    return Err(CancelError::Write(e));
                }
                Ok(CancelOutcome::Requested)
            }
            MidTurnPolicy::TransportOwned => {
                if let Some(answer) = self.answer_from_the_registry(id) {
                    return answer;
                }
                match self.transport.withdraw(id) {
                    Withdraw::Withdrawn => Ok(CancelOutcome::Cancelled),
                    Withdraw::TooLate => Ok(CancelOutcome::Requested),
                    Withdraw::NotHeld => Err(CancelError::NotFound),
                }
            }
        }
    }

    /// 명부만으로 답이 나는 취소 — `None` = 목록에 `Queued` 로 서 있다(정책 갈래로 간다).
    fn answer_from_the_registry(&self, id: &str) -> Option<Result<CancelOutcome, CancelError>> {
        let phase = self
            .core
            .queued_inputs()
            .lock()
            .rows()
            .iter()
            .find(|row| row.id == id)
            .map(|row| row.phase);
        match phase {
            None => Some(Err(CancelError::NotFound)),
            Some(RowPhase::Cancelling { .. }) => Some(Ok(CancelOutcome::Requested)),
            Some(RowPhase::Queued) => None,
        }
    }

    /// 목록 조회의 세션 쪽 — 명부 행과 `as_of_seq` 를 한 락 아래 읽고, 그 락을 놓은 **뒤** 통로에 수락 모름을
    /// 물어 `Queued` 행에 덧댄다. 표지는 조회 순간의 통로 상태라 행 스냅숏과 한 원자가 아니다(다음 조회가 고친다).
    /// 입력 자물쇠를 잡지 않는다 — 읽기다.
    // ADR-0006
    // ADR-0231
    pub fn list_queued_inputs(&self) -> (Vec<ListedRow>, Option<u64>) {
        let (rows, as_of_seq) = self.core.queued_inputs().snapshot();
        let unconfirmed = self.transport.unconfirmed_inputs();
        (overlay_unconfirmed(rows, &unconfirmed), as_of_seq)
    }

    /// transport.resize 성공 후에만 cols/rows atomic 을 갱신한다 — 실패 시 옛 값 유지.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError> {
        self.transport.resize(cols, rows)?;
        self.cols.store(cols, Ordering::Relaxed);
        self.rows.store(rows, Ordering::Relaxed);
        Ok(())
    }

    /// 진행 중 작업만 중단(≠kill — 프로세스는 살아 있다). PTY=0x03 주입.
    pub fn interrupt(&self) -> Result<(), PtyError> {
        self.transport.interrupt()
    }

    /// 자원 강제 종료 + pump 종료 대기. **이 2동사 순서(shutdown THEN join_pump)가 kill 인과의 핵심.**
    /// shutdown이 master를 drop해 pump read를 EOF로 깨우고(→core.finish(Killed)), join_pump가
    /// 그 pump 종료를 기다린다. 역전 시 hang(아직 살아있는 pump를 기다림).
    pub fn kill(&self, timeout: Duration) {
        self.transport.shutdown();
        self.core.join_pump(timeout);
    }

    /// 과도기 Exiting 전이 — kill 직전 manager가 먼저 호출(stage 6). enter_exiting과 kill은
    /// 별개 동사다. terminal(이미 종료)이면 false.
    pub fn enter_exiting(&self) -> bool {
        self.core.enter_exiting()
    }

    /// 최종 capability — transport(물리: input/output/control)와 backend(프로그램: session/model)
    /// 의 합성. 출처가 타입으로 분리돼 있어 transport 가 resume 을, backend 가 resize 를 섞어
    /// 채우는 사고가 구조적으로 불가능하다.
    pub fn capabilities(&self) -> Capabilities {
        Capabilities::compose(self.transport.capabilities(), self.backend_caps.clone())
    }

    pub fn subscribe(&self, sink: Arc<dyn OutputSink>) -> SinkId {
        self.core.subscribe(sink)
    }

    /// `on_ready`: replay 전송 직전(subscribers lock 보유 중) 1회 호출 — core 위임(불변식 2/TOCTOU).
    ///
    /// `requested_epoch` 을 **이 세션의** 표식과 대조해 seq 이어받기 여부를 정한다. `on_ready` 가 받는 응답과
    /// 돌려주는 응답은 같은 값이다 — 표식·이어받기 표식·replay 가 모두 이 화신 하나에서 나온다.
    // ADR-0226
    pub fn subscribe_from(
        &self,
        sink: Arc<dyn OutputSink>,
        after_seq: Option<u64>,
        requested_epoch: Option<u32>,
        on_ready: impl FnOnce(&SubscribeReply),
    ) -> SubscribeReply {
        let incarnation = Incarnation {
            epoch: self.epoch,
            continues_conversation: self.continues_conversation,
        };
        let epoch_matches = requested_epoch == Some(self.epoch);
        let outcome = self
            .core
            .subscribe_from(sink, after_seq, epoch_matches, |outcome| {
                on_ready(&SubscribeReply {
                    outcome: *outcome,
                    incarnation,
                })
            });
        SubscribeReply {
            outcome,
            incarnation,
        }
    }

    pub fn unsubscribe(&self, sink_id: SinkId) {
        self.core.unsubscribe(sink_id);
    }

    pub fn snapshot(&self) -> Vec<OutputChunk> {
        self.core.snapshot()
    }

    /// 마지막 콘솔 바이트 최대 `max_bytes`(계약·비용은 `OutputCore::terminal_tail`).
    // ADR-0172
    pub fn terminal_tail(&self, max_bytes: usize) -> Vec<u8> {
        self.core.terminal_tail(max_bytes)
    }

    /// 이 화신이 낸 **진단(stderr) 텍스트** 꼬리(계약·상한은 `OutputCore::diagnostic_tail`).
    ///
    /// ★`terminal_tail` 의 대체재가 아니라 짝이다★: 두 스트림은 transport 에 따라 배타적으로 찬다
    ///   — 파이프(구조화) 세션은 여기가 차고 링은 비고, PTY(터미널) 세션은 반대다. 그래서 분류
    ///   호출자는 둘 중 하나를 고르지 않고 **합쳐서** 넘긴다.
    // ADR-0172
    pub fn diagnostic_tail(&self) -> String {
        self.core.diagnostic_tail()
    }

    pub fn status(&self) -> AgentStatus {
        self.core.status()
    }

    pub fn cols(&self) -> u16 {
        self.cols.load(Ordering::Relaxed)
    }

    pub fn rows(&self) -> u16 {
        self.rows.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{AgentBackend, ClaudeBackend, ShellBackend};
    use crate::queued_input::{CancelAnswer, ListedState};
    use crate::transport::stdio::StdioTransport;
    use crate::types::{ControlCaps, InputCaps, OutputCaps, TransportCaps};
    use std::sync::Mutex;

    /// 실 프로세스 없이 인코딩 배선을 단언하기 위한 격리 하네스(ADR-0012).
    /// `fail_from`: 이 순번(0-based)부터의 write 를 실패시킨다 — 본문은 성공하고 제출만 실패하는 갈래를
    ///   만들기 위한 것이다(`None` = 전부 성공).
    struct CapturingTransport {
        captured: Arc<Mutex<Vec<Vec<u8>>>>,
        fail_from: Option<usize>,
    }
    impl AgentTransport for CapturingTransport {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
            let InputEvent::Raw(bytes) = input;
            let mut captured = self.captured.lock().unwrap();
            if self.fail_from.is_some_and(|n| captured.len() >= n) {
                return Err(PtyError::WriteFailed("harness: write refused".into()));
            }
            captured.push(bytes);
            Ok(())
        }
        fn resize(&self, _cols: u16, _rows: u16) -> Result<(), PtyError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), PtyError> {
            Ok(())
        }
        fn shutdown(&self) {}
        fn capabilities(&self) -> TransportCaps {
            harness_caps()
        }
    }

    fn harness_caps() -> TransportCaps {
        TransportCaps {
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
                resize: false,
                interrupt: false,
                cancel: false,
                graceful_shutdown: false,
            },
        }
    }

    struct NoopStatusSink;
    impl crate::types::StatusSink for NoopStatusSink {
        fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
        fn agent_list_updated(&self, _agents: Vec<crate::types::AgentInfo>) {}
    }

    /// 실 프로세스 없이 입력-시점 유저 에코 emit(ADR-0044/0045)을 단언하기 위한 하네스.
    /// 수집 태그: `Structured` 는 `"structured:<kind>"`, 그 외는 variant 명.
    struct EmitCapturingSink {
        id: SinkId,
        seen: Arc<Mutex<Vec<String>>>,
    }
    impl OutputSink for EmitCapturingSink {
        fn send(
            &self,
            frame: crate::types::OutputFrame<'_>,
        ) -> Result<(), crate::types::SinkError> {
            use crate::types::{OutputEvent, OutputPayload};
            if let OutputPayload::Event(e) = frame.payload {
                let tag = match e {
                    OutputEvent::Structured { kind, .. } => format!("structured:{kind}"),
                    other => format!("{other:?}"),
                };
                self.seen.lock().unwrap().push(tag);
            }
            Ok(())
        }
        fn sink_id(&self) -> SinkId {
            self.id
        }
    }

    fn session_with(encoder: InputEncoder) -> (AgentSession, Arc<Mutex<Vec<Vec<u8>>>>) {
        session_harness(encoder, None)
    }

    fn session_failing_after(
        encoder: InputEncoder,
        ok_writes: usize,
    ) -> (AgentSession, Arc<Mutex<Vec<Vec<u8>>>>) {
        session_harness(encoder, Some(ok_writes))
    }

    fn session_harness(
        encoder: InputEncoder,
        fail_from: Option<usize>,
    ) -> (AgentSession, Arc<Mutex<Vec<Vec<u8>>>>) {
        let id = uuid::Uuid::new_v4();
        let core = Arc::new(OutputCore::new(
            id,
            0,
            Arc::new(NoopStatusSink),
            crate::output_core::TurnWiring::detached(),
        ));
        let captured = Arc::new(Mutex::new(Vec::new()));
        let transport = Box::new(CapturingTransport {
            captured: captured.clone(),
            fail_from,
        });
        let shell_cmd = crate::profile::AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        let session = AgentSession::new(
            id,
            PathBuf::from("."),
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            ShellBackend.capabilities(&shell_cmd),
            encoder,
            true,
            core,
            transport,
        )
        // 하네스는 안 잔다 — 대기 **발행 여부**는 아래 전용 테스트가 sleeper 호출로 단언한다(시간
        //   측정 없이). 여기서 실제로 자면 단위 테스트가 호출마다 0.5초씩 붙잡힌다.
        .with_submit_pacing(Duration::ZERO, |_| {});
        (session, captured)
    }

    // ── Raw 인코더(터미널 경로 회귀 불변) ──
    #[test]
    fn write_input_raw_is_byte_identical() {
        let (session, captured) = session_with(InputEncoder::Raw);
        let input = b"echo hi\r\n\x1b[A\x03";
        session.write_input(input).unwrap();
        let got = captured.lock().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0], input.to_vec(), "Raw 는 바이트 동일이어야 함");
    }

    // ── ADR-0088: 배달-경계 계측 ──
    #[test]
    fn write_input_observed_surfaces_bytes_and_msg_uuid() {
        let (session, captured) = session_with(InputEncoder::Raw);
        let input = b"hello-observed"; // 14바이트 ASCII.
        let outcome = session
            .write_input_observed(input)
            .expect("write_input_observed ok");
        // ★FIX-5★: off-by-one·계층 회귀를 거르는 exact 카운트 단언.
        assert_eq!(
            outcome.bytes_requested, 14,
            "요청 바이트 = 넘긴 입력의 정확 바이트 수"
        );
        assert_eq!(outcome.bytes_requested, input.len(), "요청 = 입력 len");
        assert_eq!(
            outcome.bytes_written, outcome.bytes_requested,
            "by-construction 항등(bytes_written = bytes_requested 복사) — short-write 탐지 아님"
        );
        assert!(
            !outcome.msg_uuid.is_nil(),
            "이 유저 턴의 msg_uuid 를 노출해야(상관 키)"
        );
        assert_eq!(
            outcome.epoch, 0,
            "WriteOutcome.epoch = write 를 수행한 세션의 epoch(by-construction)"
        );
        // 계측판이 Raw 바이트 동일성을 깨지 않는다.
        assert_eq!(captured.lock().unwrap()[0], input.to_vec());
    }

    // ── ADR-0088(FIX-5): 멀티바이트(UTF-8) 본체 ──
    #[test]
    fn write_input_observed_counts_bytes_not_chars_multibyte() {
        // "안녕" = 한글 2자, 각 3바이트 UTF-8 = 6바이트. char 수(2)로 세면 여기서 깨진다.
        let (session, _captured) = session_with(InputEncoder::Raw);
        let input = "안녕".as_bytes();
        assert_eq!(input.len(), 6, "UTF-8 로 6바이트여야(테스트 전제)");
        let outcome = session
            .write_input_observed(input)
            .expect("write_input_observed ok");
        assert_eq!(
            outcome.bytes_requested, 6,
            "멀티바이트 요청은 char 수(2)가 아니라 바이트 수(6)여야"
        );
        assert_eq!(
            outcome.bytes_written, 6,
            "by-construction 복사도 바이트 수(6)"
        );
    }

    // ── 제출 분리(submit_input_observed) ──
    //
    // ★회귀 축은 바이트가 아니라 **write 경계**다★: 봉투 바이트는 예전에도 PTY 에 도착했지만 턴이 시작되지
    //   않았다. `본문+CR` 을 한 write 로 합치면 claude TUI 가 제출하지 않고, 두 write 로 나누면 제출된다
    //   (실측 2026-08-17). 그래서 아래 단언은 write **횟수와 경계**를 본다 — 이어붙인 바이트를 보지 않는다.

    #[test]
    fn submit_input_terminal_writes_the_body_then_the_submit_byte_separately() {
        let (session, captured) = session_with(InputEncoder::Raw);
        let body = b"<message from=\"bob\">hi</message>";

        let outcome = session
            .submit_input_observed(body)
            .expect("submit_input_observed ok");

        let got = captured.lock().unwrap();
        assert_eq!(
            got.len(),
            2,
            "본문과 제출은 분리된 write 여야(한 write 로 합치면 TUI 가 제출하지 않음): {got:?}"
        );
        assert_eq!(
            got[0],
            body.to_vec(),
            "본문 write 는 봉투 바이트 그대로 — 제출 바이트를 섞지 않는다"
        );
        assert_eq!(got[1], b"\r".to_vec(), "두 번째 write = 제출(CR) 단독");
        assert_eq!(
            outcome.bytes_requested,
            body.len(),
            "회계는 논리 메시지 바이트만 — 제출 바이트는 세지 않는다"
        );
        assert_eq!(outcome.bytes_written, body.len());
    }

    #[test]
    fn submit_input_json_mode_writes_exactly_the_encoded_line_and_nothing_else() {
        let (session, captured) = session_with(InputEncoder::ClaudeStreamJson);
        let body = b"hello";

        let outcome = session
            .submit_input_observed(body)
            .expect("submit_input_observed ok");

        // ★exact 비교★: 돌려받은 msg_uuid 로 기대 라인을 재구성해 **바이트 정확 일치**를 본다. 제출 write 를
        //   더하거나 봉투에 문자를 덧붙이는 회귀는 전부 여기서 걸린다("CR 이 없다" 류의 약한 단언과 달리
        //   무엇이 추가돼도 잡힌다). `write_input` 과의 동일성을 직접 비교하지 않는 이유 = 그쪽은 자체
        //   msg_uuid 를 새로 뽑아 두 산출물이 uuid 만큼 다르기 때문이다.
        let expected = InputEncoder::ClaudeStreamJson.encode(body, outcome.msg_uuid);
        let got = captured.lock().unwrap();
        assert_eq!(
            got.len(),
            1,
            "json 경로는 종전 그대로 write 1회 — 제출 write 를 더하면 CR 만 든 빈 턴이 하나 더 생긴다: {got:?}"
        );
        assert_eq!(
            got[0],
            expected,
            "인코더 산출물과 바이트 정확 일치여야: {}",
            String::from_utf8_lossy(&got[0])
        );
    }

    /// ★대기가 **실제로 발행되는지** + 운영 기본값이 무엇인지를 함께 못 박는다★.
    ///
    /// 이 결함의 본체는 write 횟수가 아니라 수신자의 read 경계였다 — 나눠 써도 간격이 없으면 한 덩이로
    /// 묶여 제출되지 않는다(`backend::SUBMIT_PACING` doc). 그래서 "제출 write 전에 대기가 있었나" 와
    /// "그 값이 운영 기본값인가" 둘 다 회귀 축이다. 시간을 재지 않고 sleeper 호출을 기록해 단언하므로
    /// 플래키하지 않고, 테스트가 0.5초를 실제로 자지도 않는다.
    ///
    /// ★★이 항목이 **못 잡는 것을 분명히 적는다** — 그래서 짝이 되는 항목이 따로 있다★★: 여기 오라클은
    ///   「대기가 불렸나」뿐이라 **대기를 엉뚱한 시점부터 재는 결함을 통과시킨다.** 큐가 들어온 뒤 실제로
    ///   그 결함이 났다 — 대기를 큐에 넣은 시각부터 재면 본문이 나가는 동안 간격이 먹혀 0 이 된다. 그
    ///   갈래를 잡는 것은 **물리 read 경계**를 보는 항목이고 그것은 실 파이프가 필요해 통합 시험대에 있다:
    ///   `tests/submit_delivery_boundary.rs` 의
    ///   `submit_byte_lands_in_its_own_read_even_when_the_body_takes_many_write_cycles`.
    ///   ★둘 중 하나만 남기지 말 것★ — 이 항목은 상수와 호출을, 저 항목은 그 호출이 만드는 실제 간격을 진다.
    #[test]
    fn submit_input_waits_between_the_body_and_the_submit_write() {
        use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
        // 이 테스트 전용 기록판 — 다른 테스트의 하네스는 no-op sleeper 를 쓰므로 여기 안 닿는다.
        static SLEPT_MICROS: AtomicU64 = AtomicU64::new(0);
        static CALLS: AtomicU64 = AtomicU64::new(0);
        fn recording_sleep(d: Duration) {
            SLEPT_MICROS.store(d.as_micros() as u64, AtomicOrdering::SeqCst);
            CALLS.fetch_add(1, AtomicOrdering::SeqCst);
        }

        let (session, captured) = session_harness(InputEncoder::Raw, None);
        // ★운영 기본값 그대로 두고 재우는 함수만 바꾼다★: 값까지 테스트가 정하면 기본값 회귀를 못 본다.
        let session = session.with_submit_pacing(crate::backend::SUBMIT_PACING, recording_sleep);

        session
            .submit_input_observed(b"envelope")
            .expect("submit ok");

        assert_eq!(
            CALLS.load(AtomicOrdering::SeqCst),
            1,
            "본문과 제출 사이에 대기가 정확히 한 번 발행돼야(빠지면 두 write 가 한 read 로 묶인다)"
        );
        assert_eq!(
            SLEPT_MICROS.load(AtomicOrdering::SeqCst),
            crate::backend::SUBMIT_PACING.as_micros() as u64,
            "대기 값 = 운영 기본값(줄이면 이 결함이 재발한다 — 상수 doc 의 근거를 먼저 읽을 것)"
        );
        assert!(
            crate::backend::SUBMIT_PACING > Duration::ZERO,
            "★운영 기본값이 0 이 되는 형태 금지★ — 0 이면 나눠 쓴 의미가 사라진다"
        );
        assert_eq!(
            captured.lock().unwrap().len(),
            2,
            "대기를 넣어도 write 는 여전히 본문 + 제출 둘"
        );
    }

    #[test]
    fn submit_input_reports_a_failure_that_only_hit_the_submit_write() {
        // 본문은 이미 수신자 입력창에 들어간 뒤 제출만 실패하는 갈래 — 상위가 재파킹으로 다루려면
        // 성공으로 삼켜지면 안 된다(그 잔여물의 의미는 submit_input_observed doc).
        let (session, captured) = session_failing_after(InputEncoder::Raw, 1);

        let err = session.submit_input_observed(b"envelope");

        assert!(
            matches!(err, Err(PtyError::WriteFailed(_))),
            "제출 write 실패는 Err 로 표면화돼야: {err:?}"
        );
        assert_eq!(
            captured.lock().unwrap().as_slice(),
            &[b"envelope".to_vec()],
            "본문은 이미 나갔다 — 그래서 재시도가 같은 봉투를 덧쓰는 잔여물이 남는다"
        );
    }

    #[test]
    fn write_input_does_not_submit() {
        // 사람이 Enter 를 직접 치는 키 입력 경로 — 여기에 제출이 끼면 키 한 번마다 턴이 나간다.
        let (session, captured) = session_with(InputEncoder::Raw);
        session.write_input(b"partial").unwrap();
        let got = captured.lock().unwrap();
        assert_eq!(got.len(), 1, "write_input 은 write 1회 그대로: {got:?}");
        assert_eq!(got[0], b"partial".to_vec());
    }

    // ── ADR-0088: 실패 표면화 ──
    #[test]
    fn write_input_observed_surfaces_transport_error() {
        struct FailingTransport;
        impl AgentTransport for FailingTransport {
            fn start(&self, _core: Arc<OutputCore>) {}
            fn send_input(&self, _input: InputEvent) -> Result<(), PtyError> {
                Err(PtyError::WriteFailed("stdin closed".into()))
            }
            fn resize(&self, _c: u16, _r: u16) -> Result<(), PtyError> {
                Ok(())
            }
            fn interrupt(&self) -> Result<(), PtyError> {
                Ok(())
            }
            fn shutdown(&self) {}
            fn capabilities(&self) -> crate::types::TransportCaps {
                use crate::types::{ControlCaps, InputCaps, OutputCaps, TransportCaps};
                TransportCaps {
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
                        resize: false,
                        interrupt: false,
                        cancel: false,
                        graceful_shutdown: false,
                    },
                }
            }
        }
        let id = uuid::Uuid::new_v4();
        let core = Arc::new(OutputCore::new(
            id,
            0,
            Arc::new(NoopStatusSink),
            crate::output_core::TurnWiring::detached(),
        ));
        let shell_cmd = crate::profile::AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        let session = AgentSession::new(
            id,
            PathBuf::from("."),
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            ShellBackend.capabilities(&shell_cmd),
            InputEncoder::Raw,
            true,
            core,
            Box::new(FailingTransport),
        );
        let err = session.write_input_observed(b"x");
        assert!(
            matches!(err, Err(PtyError::WriteFailed(_))),
            "send_input 실패는 Err 로 표면화돼야(성공으로 삼키지 않음): {err:?}"
        );
    }

    // ── ClaudeStreamJson 인코더(ADR-0044) ──
    #[test]
    fn write_input_json_mode_wraps_as_stream_json_line() {
        let (session, captured) = session_with(InputEncoder::ClaudeStreamJson);
        session.write_input(b"hello").unwrap();
        let got = captured.lock().unwrap();
        assert_eq!(got.len(), 1);
        let line = &got[0];
        assert_eq!(*line.last().unwrap(), b'\n', "라인 종단 \\n");
        let s = String::from_utf8(line.clone()).unwrap();
        assert!(s.contains("\"type\":\"user\""), "user 턴 스키마: {s}");
        assert!(s.contains("\"text\":\"hello\""), "text 보존: {s}");
    }

    // ── ADR-0044/0045: 입력-시점 유저 에코 ──────────
    #[test]
    fn write_input_json_mode_emits_input_time_user_echo() {
        let (session, _captured) = session_with(InputEncoder::ClaudeStreamJson);
        let seen = Arc::new(Mutex::new(Vec::new()));
        session.subscribe(Arc::new(EmitCapturingSink {
            id: uuid::Uuid::new_v4(),
            seen: seen.clone(),
        }));

        session.write_input("안녕 클로드".as_bytes()).unwrap();

        let got = seen.lock().unwrap();
        assert_eq!(
            *got,
            vec!["structured:user".to_string()],
            "json 모드 write_input 은 입력-시점 유저 에코 1건을 emit 해야 함"
        );
    }

    #[test]
    fn write_input_terminal_mode_does_not_emit_user_echo() {
        // Raw(터미널·shell)는 PTY 로컬 에코가 이미 있어 합성 에코를 emit 하면 중복 → 아무 것도 emit 안 함.
        let (session, _captured) = session_with(InputEncoder::Raw);
        let seen = Arc::new(Mutex::new(Vec::new()));
        session.subscribe(Arc::new(EmitCapturingSink {
            id: uuid::Uuid::new_v4(),
            seen: seen.clone(),
        }));

        session.write_input(b"echo hi\r\n").unwrap();

        assert!(
            seen.lock().unwrap().is_empty(),
            "터미널(Raw) 경로는 입력-시점 유저 에코를 emit 하지 않아야 함(PTY 에코 중복 방지)"
        );
    }

    // ── json 모드 세션 caps: StdioTransport ⊕ ClaudeBackend 합성 ──
    #[cfg(windows)]
    #[test]
    fn json_mode_session_caps_are_structured() {
        let id = uuid::Uuid::new_v4();
        let core = Arc::new(OutputCore::new(
            id,
            0,
            Arc::new(NoopStatusSink),
            crate::output_core::TurnWiring::detached(),
        ));
        let spec = crate::types::CommandSpec {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), "echo probe".into()],
            env: vec![],
            cwd: PathBuf::from("."),
        };
        // json 모드 = structured 캐리어 → StdioTransport 에 structured=true 주입(운영에선 그 통로를
        //   만드는 backend 가 같은 값을 넣는다 — ADR-0191).
        let (transport, _pid) = StdioTransport::open(&spec, true, None).expect("open");
        // json 모드 command — backend 가 이걸 보고 caps 를 산출한다.
        let json_cmd = crate::profile::AgentCommand::Claude {
            extra_args: vec![],
            output_format: crate::profile::AgentOutputFormat::StreamJson,
        };
        let session = AgentSession::new(
            id,
            PathBuf::from("."),
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            // json 모드도 backend는 여전히 ClaudeBackend(resume/model은 프로그램 소관, ADR-0030).
            ClaudeBackend.capabilities(&json_cmd),
            InputEncoder::ClaudeStreamJson,
            true,
            core,
            Box::new(transport),
        );
        let caps = session.capabilities();
        assert!(caps.output.structured, "json 세션 → 구조화 출력");
        assert!(!caps.output.terminal_bytes, "터미널 바이트 아님");
        assert!(!caps.control.resize, "resize 불가");
        assert!(!caps.control.interrupt, "interrupt 불가(MVP)");
        // ★ADR-0044 후속 완료★: json 모드도 --resume 지원(spike-verified, claude 2.1.170) → resume=true.
        //   build_spec 이 SpawnMode::Resume 에서 --resume 을 내고 통제-sid(ADR-0008)를 재사용하므로 sid
        //   충돌 없음.
        assert!(
            caps.session.resume,
            "json 모드 세션 → resume=true(--resume 지원, spike-verified)"
        );
        session.kill(Duration::from_secs(5));
    }

    // ── 세션 id 첫 제출 래치 — 입력 두 동사가 세는 자리(ADR-0226) ──
    //
    // ★재는 축은 순서다★: 통로의 쓰기와 래치 commit 포트 호출을 **한 사건 기록**에 세워, 「턴을 여는 쓰기보다
    //   commit 이 먼저」를 바이트가 아니라 사건 순서로 본다.

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Ev {
        Commit(String),
        Send(Vec<u8>),
    }

    /// `fail_from`: 이 순번(0-based)부터의 쓰기를 거절한다. `flush_fails`: 착지 확인이 실패한다.
    struct EventTransport {
        events: Arc<Mutex<Vec<Ev>>>,
        fail_from: Option<usize>,
        flush_fails: bool,
    }
    impl AgentTransport for EventTransport {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
            let InputEvent::Raw(bytes) = input;
            let mut events = self.events.lock().unwrap();
            let sent = events.iter().filter(|e| matches!(e, Ev::Send(_))).count();
            if self.fail_from.is_some_and(|n| sent >= n) {
                return Err(PtyError::WriteFailed("harness: write refused".into()));
            }
            events.push(Ev::Send(bytes));
            Ok(())
        }
        fn flush_input(&self, _timeout: Duration) -> Result<(), PtyError> {
            if self.flush_fails {
                Err(PtyError::WriteFailed("harness: never landed".into()))
            } else {
                Ok(())
            }
        }
        fn resize(&self, _cols: u16, _rows: u16) -> Result<(), PtyError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), PtyError> {
            Ok(())
        }
        fn shutdown(&self) {}
        fn capabilities(&self) -> TransportCaps {
            harness_caps()
        }
    }

    fn recording_port(events: &Arc<Mutex<Vec<Ev>>>) -> crate::backend::SessionIdSink {
        let events = events.clone();
        Arc::new(move |raw: &str| events.lock().unwrap().push(Ev::Commit(raw.to_owned())))
    }

    /// 래치를 실은 세션. 래치에는 id 가 **이미** 들어가 있다(claude 처럼 래치를 만드는 자리에서 offer) —
    /// 그래서 첫 제출이 그 자리에서 commit 하고, 사건 기록에서 제출과 commit 의 순서가 보인다.
    fn latched_session_with(
        encoder: InputEncoder,
        fail_from: Option<usize>,
        flush_fails: bool,
        events: &Arc<Mutex<Vec<Ev>>>,
        port: crate::backend::SessionIdSink,
    ) -> AgentSession {
        let id = uuid::Uuid::new_v4();
        let core = Arc::new(OutputCore::new(
            id,
            0,
            Arc::new(NoopStatusSink),
            crate::output_core::TurnWiring::detached(),
        ));
        let shell_cmd = crate::profile::AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        let latch = SessionIdLatch::new(id, 0, port);
        latch.offer("sid-1");
        AgentSession::new(
            id,
            PathBuf::from("."),
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            ShellBackend.capabilities(&shell_cmd),
            encoder,
            true,
            core,
            Box::new(EventTransport {
                events: events.clone(),
                fail_from,
                flush_fails,
            }),
        )
        .with_submit_pacing(Duration::ZERO, |_| {})
        .with_session_id_latch(latch)
    }

    fn latched(encoder: InputEncoder) -> (AgentSession, Arc<Mutex<Vec<Ev>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let session = latched_session_with(encoder, None, false, &events, recording_port(&events));
        (session, events)
    }

    fn commits(events: &Arc<Mutex<Vec<Ev>>>) -> usize {
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, Ev::Commit(_)))
            .count()
    }

    #[test]
    fn a_key_fragment_without_cr_is_not_counted() {
        let (session, events) = latched(InputEncoder::Raw);
        session.write_input(b"hel").expect("write");
        assert_eq!(*events.lock().unwrap(), vec![Ev::Send(b"hel".to_vec())]);
    }

    #[test]
    fn a_key_fragment_with_cr_is_counted_before_it_is_sent() {
        let (session, events) = latched(InputEncoder::Raw);
        session.write_input(b"hi\r").expect("write");
        assert_eq!(
            *events.lock().unwrap(),
            vec![Ev::Commit("sid-1".into()), Ev::Send(b"hi\r".to_vec())],
            "영속이 턴보다 먼저여야 한다"
        );

        session.write_input(b"again\r").expect("write");
        assert_eq!(commits(&events), 1, "둘째 제출은 다시 commit 하지 않는다");
    }

    #[test]
    fn a_json_turn_is_counted_before_it_is_sent() {
        let (session, events) = latched(InputEncoder::ClaudeStreamJson);
        session.write_input(b"hello").expect("write");
        let got = events.lock().unwrap();
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(
            got[0],
            Ev::Commit("sid-1".into()),
            "영속이 턴보다 먼저: {got:?}"
        );
        assert!(matches!(got[1], Ev::Send(_)), "{got:?}");
    }

    /// ★`Raw` 우편은 본문 뒤·대기 뒤·제출 CR 바로 앞에서 센다★ — 맨 앞에서 세면 본문 쓰기·착지 확인·대기가
    ///   영속과 턴 사이에 끼어, 그 창의 실패나 kill 이 턴 없는 영속을 남긴다.
    #[test]
    fn raw_mail_is_counted_after_the_body_and_the_pacing_right_before_the_submit_cr() {
        use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
        // 이 테스트 전용 기록판 — 대기가 commit 보다 먼저 발행됐는지를 commit 순간에 읽는다.
        static SLEPT: AtomicBool = AtomicBool::new(false);
        fn mark_slept(_d: Duration) {
            SLEPT.store(true, AtomicOrdering::SeqCst);
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let port_events = events.clone();
        let port: crate::backend::SessionIdSink = Arc::new(move |raw: &str| {
            let slept = SLEPT.load(AtomicOrdering::SeqCst);
            port_events
                .lock()
                .unwrap()
                .push(Ev::Commit(format!("{raw} slept={slept}")));
        });
        let session = latched_session_with(InputEncoder::Raw, None, false, &events, port)
            .with_submit_pacing(Duration::ZERO, mark_slept);

        session.submit_input_observed(b"envelope").expect("submit");

        assert_eq!(
            *events.lock().unwrap(),
            vec![
                Ev::Send(b"envelope".to_vec()),
                Ev::Commit("sid-1 slept=true".into()),
                Ev::Send(b"\r".to_vec()),
            ]
        );
    }

    /// JSON 인코더는 제출 CR 이 없다 — 본문이 곧 턴이라 본문 **앞**에서 센다.
    #[test]
    fn json_mail_is_counted_before_the_body() {
        let (session, events) = latched(InputEncoder::ClaudeStreamJson);
        session.submit_input_observed(b"hello").expect("submit");
        let got = events.lock().unwrap();
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[0], Ev::Commit("sid-1".into()), "{got:?}");
        assert!(matches!(got[1], Ev::Send(_)), "{got:?}");
    }

    #[test]
    fn raw_mail_whose_body_write_fails_is_not_counted() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let session = latched_session_with(
            InputEncoder::Raw,
            Some(0),
            false,
            &events,
            recording_port(&events),
        );
        assert!(session.submit_input_observed(b"envelope").is_err());
        assert_eq!(commits(&events), 0, "본문도 못 간 배달이 영속을 남겼다");
    }

    #[test]
    fn raw_mail_whose_body_never_lands_is_not_counted() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let session = latched_session_with(
            InputEncoder::Raw,
            None,
            true,
            &events,
            recording_port(&events),
        );
        assert!(session.submit_input_observed(b"envelope").is_err());
        assert_eq!(
            *events.lock().unwrap(),
            vec![Ev::Send(b"envelope".to_vec())],
            "착지 확인이 실패하면 제출 CR 도 영속도 없다"
        );
    }

    /// 래치 없는 기본 세션은 세기도 사용자 종료 확인도 무동작이다 — 운영이 래치를 싣기 전까지 동작 불변.
    #[test]
    fn a_session_without_a_latch_is_untouched_even_during_a_user_kill() {
        let (session, captured) = session_with(InputEncoder::Raw);
        session.set_intent(TerminationIntent::UserKill);

        session
            .write_input(b"hi\r")
            .expect("래치가 없으면 오늘처럼 보낸다");
        session
            .submit_input_observed(b"envelope")
            .expect("래치가 없으면 오늘처럼 배달한다");

        assert_eq!(
            *captured.lock().unwrap(),
            vec![b"hi\r".to_vec(), b"envelope".to_vec(), b"\r".to_vec()]
        );
    }

    fn assert_user_kill_refusal(result: Result<WriteOutcome, PtyError>) {
        match result {
            Err(PtyError::WriteFailed(msg)) => assert!(
                msg.contains("종료") && !msg.contains("덧쓴다"),
                "사용자 종료를 명시하는 제 문구여야 한다(제출 write 실패 문구 재사용 금지): {msg}"
            ),
            other => panic!("사용자 종료 중의 턴 제출은 WriteFailed 여야 한다: {other:?}"),
        }
    }

    /// ★사용자 종료 중이면 세지도 보내지도 않는다★ — 세고 보내면 대화 없는 id 가 영속되고, 세지 않고
    ///   보내면 영속되지 않은 id 의 대화가 생길 수 있다.
    #[test]
    fn a_user_kill_refuses_turn_submissions_without_counting_them() {
        // 터미널 키 입력: CR 이 든 조각은 통째로 안 나가고, CR 없는 조각은 그대로 나간다.
        let (session, events) = latched(InputEncoder::Raw);
        session.set_intent(TerminationIntent::UserKill);
        assert_user_kill_refusal(session.write_input_observed(b"hi\r"));
        session
            .write_input(b"typing")
            .expect("턴을 열지 않는 키 입력은 세는 자리가 아니다");
        assert_eq!(*events.lock().unwrap(), vec![Ev::Send(b"typing".to_vec())]);

        // JSON 턴: 본문째 안 나간다.
        let (session, events) = latched(InputEncoder::ClaudeStreamJson);
        session.set_intent(TerminationIntent::UserKill);
        assert_user_kill_refusal(session.write_input_observed(b"hello"));
        assert_user_kill_refusal(session.submit_input_observed(b"hello"));
        assert!(
            events.lock().unwrap().is_empty(),
            "{:?}",
            events.lock().unwrap()
        );

        // `Raw` 우편: 본문은 나갔을 수 있지만 턴을 여는 CR 은 안 나간다.
        let (session, events) = latched(InputEncoder::Raw);
        session.set_intent(TerminationIntent::UserKill);
        assert_user_kill_refusal(session.submit_input_observed(b"envelope"));
        assert_eq!(
            *events.lock().unwrap(),
            vec![Ev::Send(b"envelope".to_vec())]
        );
    }

    /// 영속을 마친 뒤(빠른 길)에도 확인한다 — 둘째 턴도 사용자 종료 중이면 안 나간다.
    #[test]
    fn a_user_kill_is_checked_even_after_the_latch_settled() {
        let (session, events) = latched(InputEncoder::Raw);
        session.write_input(b"a\r").expect("첫 턴");
        session.set_intent(TerminationIntent::UserKill);
        assert_user_kill_refusal(session.write_input_observed(b"b\r"));
        assert_eq!(
            *events.lock().unwrap(),
            vec![Ev::Commit("sid-1".into()), Ev::Send(b"a\r".to_vec())]
        );
    }

    // ── 구독 응답의 화신 사실(ADR-0226) ──

    #[test]
    fn subscribe_from_matches_the_requested_epoch_against_its_own_incarnation() {
        let (session, _captured) = session_with(InputEncoder::Raw);
        let sink = || -> Arc<dyn OutputSink> {
            Arc::new(EmitCapturingSink {
                id: uuid::Uuid::new_v4(),
                seen: Arc::new(Mutex::new(Vec::new())),
            })
        };
        use crate::types::ReplayKind;

        let same = session.subscribe_from(sink(), Some(0), Some(0), |_| {});
        assert_eq!(same.outcome.kind, ReplayKind::Resumed);
        let other = session.subscribe_from(sink(), Some(0), Some(1), |_| {});
        assert_eq!(other.outcome.kind, ReplayKind::FromOldest);
        let unknown = session.subscribe_from(sink(), Some(0), None, |_| {});
        assert_eq!(unknown.outcome.kind, ReplayKind::FromOldest);
    }

    #[test]
    fn subscribe_from_hands_the_same_incarnation_to_on_ready_and_the_caller() {
        let sink = || -> Arc<dyn OutputSink> {
            Arc::new(EmitCapturingSink {
                id: uuid::Uuid::new_v4(),
                seen: Arc::new(Mutex::new(Vec::new())),
            })
        };
        for continues in [false, true] {
            let (session, _captured) = session_with(InputEncoder::Raw);
            let session = session.with_incarnation(continues);
            let mut at_ready = None;
            let reply =
                session.subscribe_from(sink(), None, None, |r| at_ready = Some(r.incarnation));
            let expected = Incarnation {
                epoch: session.epoch,
                continues_conversation: continues,
            };
            assert_eq!(reply.incarnation, expected);
            assert_eq!(at_ready, Some(expected));
        }
    }

    // ── ADR-0231: 입력 경로 뼈대 — 출처 · 정책 · 취소 ──

    /// 이벤트를 통째로 모으는 sink — 목록 사건의 모양까지 본다.
    struct EventSink {
        id: SinkId,
        seen: Arc<Mutex<Vec<OutputEvent>>>,
    }
    impl OutputSink for EventSink {
        fn send(
            &self,
            frame: crate::types::OutputFrame<'_>,
        ) -> Result<(), crate::types::SinkError> {
            if let crate::types::OutputPayload::Event(e) = frame.payload {
                self.seen.lock().unwrap().push(e.clone());
            }
            Ok(())
        }
        fn sink_id(&self) -> SinkId {
            self.id
        }
    }

    /// 구독한 **뒤**의 이벤트만 모은다 — 구독이 동기로 되감은 링은 버린다.
    fn watch(session: &AgentSession) -> Arc<Mutex<Vec<OutputEvent>>> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        session.subscribe(Arc::new(EventSink {
            id: uuid::Uuid::new_v4(),
            seen: seen.clone(),
        }));
        seen.lock().unwrap().clear();
        seen
    }

    fn list_events(seen: &Arc<Mutex<Vec<OutputEvent>>>) -> Vec<QueuedInputEvent> {
        seen.lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                OutputEvent::QueuedInput(op) => Some(op.clone()),
                _ => None,
            })
            .collect()
    }

    /// 매니저와 같은 배선(공용 턴 관측 표 + 대기 목록 표)으로 선 정책 `None` 세션 — **코어가 턴 중이라고 답하는**
    /// 상태로 돌려준다.
    fn in_turn_policy_none_session(
        encoder: InputEncoder,
    ) -> (
        AgentSession,
        Arc<Mutex<Vec<Vec<u8>>>>,
        Arc<crate::turn::TurnObservations>,
        Arc<crate::inputs_pending::InputsPendingTable>,
    ) {
        let id = uuid::Uuid::new_v4();
        let turns = Arc::new(crate::turn::TurnObservations::new());
        let pending = Arc::new(crate::inputs_pending::InputsPendingTable::new());
        turns.register(id, 0);
        pending.register(id, 0);
        turns.observe(id, 0, 1, crate::turn::TurnSignal::Progress);
        assert!(turns.is_in_turn(id, 0), "전제: 코어가 턴 중이라고 답한다");
        let core = Arc::new(
            OutputCore::new(
                id,
                0,
                Arc::new(NoopStatusSink),
                crate::output_core::TurnWiring::new(turns.clone(), crate::backend::no_turn_signals),
            )
            .with_queued(crate::output_core::QueuedWiring {
                registry: Arc::new(QueuedInputs::new()),
                pending: pending.clone(),
            }),
        );
        let captured = Arc::new(Mutex::new(Vec::new()));
        let shell_cmd = crate::profile::AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        let session = AgentSession::new(
            id,
            PathBuf::from("."),
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            ShellBackend.capabilities(&shell_cmd),
            encoder,
            true,
            core,
            Box::new(CapturingTransport {
                captured: captured.clone(),
                fail_from: None,
            }),
        )
        .with_submit_pacing(Duration::ZERO, |_| {});
        (session, captured, turns, pending)
    }

    // TRD §7-1 새 회귀.
    #[test]
    fn a_policy_none_session_never_emits_queued_input_even_while_the_core_says_in_turn() {
        for encoder in [InputEncoder::Raw, InputEncoder::ClaudeStreamJson] {
            let (session, captured, _turns, _pending) = in_turn_policy_none_session(encoder);
            let seen = watch(&session);
            let mut expected = Vec::new();
            for origin in [InputOrigin::User, InputOrigin::Mail] {
                let body = b"echo hi\r\n";
                let outcome = session.write_input_from(body, origin).unwrap();
                expected.push(encoder.encode(body, outcome.msg_uuid));
            }
            let mail = b"<message from=\"bob\">hi</message>";
            let outcome = session.submit_input_observed(mail).unwrap();
            expected.push(encoder.encode(mail, outcome.msg_uuid));
            if let Some(submit) = encoder.submit_sequence() {
                expected.push(submit.to_vec());
            }

            assert_eq!(
                *captured.lock().unwrap(),
                expected,
                "{encoder:?}: 정책 None 은 두 출처 모두 오늘 바이트 그대로다"
            );
            assert!(
                list_events(&seen).is_empty(),
                "{encoder:?}: 턴 중이어도 정책 None 세션은 목록 사건을 한 건도 내지 않는다"
            );
            let echoes = seen
                .lock()
                .unwrap()
                .iter()
                .filter(|e| matches!(e, OutputEvent::Structured { .. }))
                .count();
            assert_eq!(
                echoes,
                if encoder == InputEncoder::Raw { 0 } else { 3 },
                "{encoder:?}: 합성 에코도 오늘과 같다(json 은 쓰기마다 하나 · 터미널은 없음)"
            );
            assert!(session.queued_inputs().lock().is_empty());
        }
    }

    // TRD §7-1 새 회귀.
    #[test]
    fn a_policy_none_agent_never_reports_inputs_pending() {
        let (session, _captured, turns, pending) = in_turn_policy_none_session(InputEncoder::Raw);
        let id = session.id;
        for origin in [InputOrigin::User, InputOrigin::Mail] {
            session.write_input_from(b"typed\r", origin).unwrap();
        }
        session.submit_input_observed(b"mail").unwrap();
        assert!(matches!(
            session.cancel_queued_input("anything"),
            Err(CancelError::NotFound)
        ));

        assert_eq!(
            pending.get(id, 0),
            Some(false),
            "터미널 화신의 대기 목록 사실은 늘 비었다 — 바쁨은 턴 관측만으로 선다"
        );
        assert!(
            turns.is_in_turn(id, 0),
            "입력 경로는 턴 관측을 건드리지 않는다"
        );
    }

    #[test]
    fn cancel_under_policy_none_is_not_found_and_touches_nothing() {
        let (session, captured) = session_with(InputEncoder::Raw);
        // 방어: 정책 None 에 목록 행이 설 길은 없지만, 서 있어도 통로에 닿지 않는다.
        session.core.emit(queued("q1"));
        let seen = watch(&session);
        assert!(matches!(
            session.cancel_queued_input("q1"),
            Err(CancelError::NotFound)
        ));
        assert!(captured.lock().unwrap().is_empty());
        assert!(list_events(&seen).is_empty());
    }

    fn queued(id: &str) -> OutputEvent {
        OutputEvent::QueuedInput(QueuedInputEvent::Queued {
            id: id.into(),
            text: format!("text of {id}"),
        })
    }

    fn test_cancel_line(id: &str) -> Vec<u8> {
        format!("cancel:{id}\n").into_bytes()
    }

    /// 정책 갈래를 관측하는 통로 — raw 쓰기 · 턴 넘기기 · 거두기 호출을 기록하고, 거두기 답을 대본대로 낸다.
    #[derive(Default)]
    struct Probe {
        raw: Mutex<Vec<Vec<u8>>>,
        turns: Mutex<Vec<TurnInput>>,
        withdraws: Mutex<Vec<String>>,
        withdraw_answer: Mutex<Option<Withdraw>>,
        fail_raw: std::sync::atomic::AtomicBool,
        unconfirmed: Mutex<Vec<String>>,
    }
    struct ProbeTransport(Arc<Probe>);
    impl AgentTransport for ProbeTransport {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
            let InputEvent::Raw(bytes) = input;
            if self.0.fail_raw.load(Ordering::SeqCst) {
                return Err(PtyError::WriteFailed("probe: write refused".into()));
            }
            self.0.raw.lock().unwrap().push(bytes);
            Ok(())
        }
        fn send_turn(&self, turn: TurnInput) -> Result<(), PtyError> {
            self.0.turns.lock().unwrap().push(turn);
            Ok(())
        }
        fn withdraw(&self, id: &str) -> Withdraw {
            self.0.withdraws.lock().unwrap().push(id.to_owned());
            self.0
                .withdraw_answer
                .lock()
                .unwrap()
                .unwrap_or(Withdraw::NotHeld)
        }
        fn unconfirmed_inputs(&self) -> Vec<String> {
            self.0.unconfirmed.lock().unwrap().clone()
        }
        fn resize(&self, _cols: u16, _rows: u16) -> Result<(), PtyError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), PtyError> {
            Ok(())
        }
        fn shutdown(&self) {}
        fn capabilities(&self) -> TransportCaps {
            harness_caps()
        }
    }

    fn probed(encoder: InputEncoder, mid_turn: MidTurnPolicy) -> (AgentSession, Arc<Probe>) {
        let id = uuid::Uuid::new_v4();
        let core = Arc::new(OutputCore::new(
            id,
            0,
            Arc::new(NoopStatusSink),
            crate::output_core::TurnWiring::detached(),
        ));
        let probe = Arc::new(Probe::default());
        let shell_cmd = crate::profile::AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        let session = AgentSession::new(
            id,
            PathBuf::from("."),
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            ShellBackend.capabilities(&shell_cmd),
            encoder,
            true,
            core,
            Box::new(ProbeTransport(probe.clone())),
        )
        .with_submit_pacing(Duration::ZERO, |_| {})
        .with_mid_turn(mid_turn, Arc::new(DeliveryAck::new()));
        (session, probe)
    }

    fn classified() -> MidTurnPolicy {
        MidTurnPolicy::SessionClassified {
            cancel_line: test_cancel_line,
        }
    }

    fn phase_of(session: &AgentSession, id: &str) -> Option<RowPhase> {
        session
            .queued_inputs()
            .lock()
            .rows()
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.phase)
    }

    #[test]
    fn a_session_classified_cancel_announces_the_request_then_writes_the_cancel_line() {
        let (session, probe) = probed(InputEncoder::ClaudeStreamJson, classified());
        session.core.emit(queued("q1"));
        let seen = watch(&session);

        assert_eq!(
            session.cancel_queued_input("q1").unwrap(),
            CancelOutcome::Requested
        );
        assert_eq!(
            list_events(&seen),
            vec![QueuedInputEvent::CancelRequested { id: "q1".into() }]
        );
        assert_eq!(
            *probe.raw.lock().unwrap(),
            vec![test_cancel_line("q1")],
            "취소 줄은 backend 조각 그대로 한 번 — 인코더를 지나지 않는다"
        );
        assert!(matches!(
            phase_of(&session, "q1"),
            Some(RowPhase::Cancelling { .. })
        ));
    }

    #[test]
    fn a_second_cancel_of_a_cancelling_item_is_requested_without_events_or_vendor_calls() {
        for mid_turn in [classified(), MidTurnPolicy::TransportOwned] {
            let (session, probe) = probed(InputEncoder::ClaudeStreamJson, mid_turn);
            session.core.emit(queued("q1"));
            session.core.emit(OutputEvent::QueuedInput(
                QueuedInputEvent::CancelRequested { id: "q1".into() },
            ));
            let seen = watch(&session);

            assert_eq!(
                session.cancel_queued_input("q1").unwrap(),
                CancelOutcome::Requested,
                "{mid_turn:?}: 이미 취소 대기면 멱등"
            );
            assert!(list_events(&seen).is_empty(), "{mid_turn:?}");
            assert!(probe.raw.lock().unwrap().is_empty(), "{mid_turn:?}");
            assert!(probe.withdraws.lock().unwrap().is_empty(), "{mid_turn:?}");
        }
    }

    #[test]
    fn a_failed_cancel_line_write_reports_cancel_failed_and_keeps_the_item_cancelling() {
        let (session, probe) = probed(InputEncoder::ClaudeStreamJson, classified());
        session.core.emit(queued("q1"));
        probe.fail_raw.store(true, Ordering::SeqCst);
        let seen = watch(&session);

        assert!(matches!(
            session.cancel_queued_input("q1"),
            Err(CancelError::Write(PtyError::WriteFailed(_)))
        ));
        assert_eq!(
            list_events(&seen),
            vec![
                QueuedInputEvent::CancelRequested { id: "q1".into() },
                QueuedInputEvent::CancelFailed { id: "q1".into() },
            ]
        );
        assert!(
            matches!(phase_of(&session, "q1"), Some(RowPhase::Cancelling { .. })),
            "취소 요청이 실패해도 목록으로 되돌리지 않는다"
        );
    }

    #[test]
    fn cancel_of_an_unknown_or_settled_id_is_not_found_under_every_listing_policy() {
        for mid_turn in [classified(), MidTurnPolicy::TransportOwned] {
            let (session, probe) = probed(InputEncoder::ClaudeStreamJson, mid_turn);
            session.core.emit(queued("done"));
            session
                .core
                .emit(OutputEvent::QueuedInput(QueuedInputEvent::Delivered {
                    id: "done".into(),
                }));
            let seen = watch(&session);

            for id in ["never-listed", "done"] {
                assert!(
                    matches!(session.cancel_queued_input(id), Err(CancelError::NotFound)),
                    "{mid_turn:?}: {id}"
                );
            }
            assert!(list_events(&seen).is_empty(), "{mid_turn:?}");
            assert!(probe.raw.lock().unwrap().is_empty(), "{mid_turn:?}");
            assert!(probe.withdraws.lock().unwrap().is_empty(), "{mid_turn:?}");
        }
    }

    #[test]
    fn a_transport_owned_cancel_maps_the_three_withdraw_answers() {
        for (answer, expected) in [
            (Withdraw::Withdrawn, Ok(CancelOutcome::Cancelled)),
            (Withdraw::TooLate, Ok(CancelOutcome::Requested)),
            (Withdraw::NotHeld, Err(())),
        ] {
            let (session, probe) =
                probed(InputEncoder::TransportFramed, MidTurnPolicy::TransportOwned);
            session.core.emit(queued("q1"));
            *probe.withdraw_answer.lock().unwrap() = Some(answer);
            let seen = watch(&session);

            let got = session.cancel_queued_input("q1");
            match expected {
                Ok(outcome) => assert_eq!(got.unwrap(), outcome, "{answer:?}"),
                Err(()) => assert!(matches!(got, Err(CancelError::NotFound)), "{answer:?}"),
            }
            assert_eq!(*probe.withdraws.lock().unwrap(), vec!["q1".to_string()]);
            assert!(
                list_events(&seen).is_empty(),
                "{answer:?}: 목록 사건은 통로가 낸다 — 세션은 내지 않는다"
            );
            assert!(probe.raw.lock().unwrap().is_empty(), "{answer:?}");
        }
    }

    #[test]
    fn the_listing_marks_what_the_transport_holds_unconfirmed_on_queued_rows_only() {
        let (session, probe) = probed(InputEncoder::TransportFramed, MidTurnPolicy::TransportOwned);
        for id in ["q1", "q2", "q3"] {
            session.core.emit(queued(id));
        }
        session.core.emit(OutputEvent::QueuedInput(
            QueuedInputEvent::CancelRequested { id: "q3".into() },
        ));
        *probe.unconfirmed.lock().unwrap() = vec!["q2".into(), "q3".into(), "gone".into()];

        let (rows, as_of_seq) = session.list_queued_inputs();
        let states: Vec<(&str, ListedState)> =
            rows.iter().map(|r| (r.id.as_str(), r.state)).collect();
        assert_eq!(
            states,
            vec![
                ("q1", ListedState::Queued),
                ("q2", ListedState::Unconfirmed),
                (
                    "q3",
                    ListedState::Cancelling {
                        answer: CancelAnswer::Unanswered,
                        vendor_closed: false
                    }
                ),
            ],
            "취소 대기가 표지를 이기고, 명부에 없는 id 는 행이 되지 않는다"
        );
        assert_eq!(rows[0].text, "text of q1");
        assert!(as_of_seq.is_some());
        assert_eq!(
            as_of_seq,
            session.queued_inputs().snapshot().1,
            "as_of_seq 는 명부 스냅숏의 것 그대로다"
        );
    }

    #[test]
    fn a_transport_that_holds_nothing_unconfirmed_lists_every_row_as_registered() {
        let (session, _captured) = session_with(InputEncoder::Raw);
        assert_eq!(
            session.list_queued_inputs(),
            (vec![], None),
            "목록 사건이 없는 화신 — 빈 목록 · as_of_seq 없음"
        );
        session.core.emit(queued("q1"));
        let (rows, as_of_seq) = session.list_queued_inputs();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].state, ListedState::Queued);
        assert!(as_of_seq.is_some());
    }

    #[test]
    fn a_transport_owned_write_hands_the_turn_over_with_its_origin_and_receipt_id() {
        let (session, probe) = probed(InputEncoder::TransportFramed, MidTurnPolicy::TransportOwned);
        let seen = watch(&session);

        let typed = session.write_input_from(b"hi", InputOrigin::User).unwrap();
        let mail = session.write_input_observed(b"letter").unwrap();

        let turns = probe.turns.lock().unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].id, typed.msg_uuid.to_string());
        assert_eq!(turns[0].body, b"hi".to_vec());
        assert_eq!(turns[0].origin, InputOrigin::User);
        assert_eq!(turns[1].id, mail.msg_uuid.to_string());
        assert_eq!(turns[1].body, b"letter".to_vec());
        assert_eq!(
            turns[1].origin,
            InputOrigin::Mail,
            "`*_observed` 동사는 우편이다"
        );
        assert!(
            probe.raw.lock().unwrap().is_empty(),
            "턴은 통로 턴 동사로만 간다"
        );
        assert!(list_events(&seen).is_empty(), "분류는 통로가 한다");
    }

    // ── ADR-0231: 세션 분류(SessionClassified) — 사용자 입력 · 취소 (TRD §7-1 claude 세션 행) ──

    use crate::inputs_pending::InputsPendingTable;
    use crate::turn::{TurnEndKind, TurnObservations, TurnSignal};
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc;

    /// 쓰기를 관측하는 통로 — 쓰기마다 (바이트, **그 순간** 명부에 선 id) 를 적고, 대본대로 실패하거나 붙잡힌다.
    struct Gate {
        core: Arc<OutputCore>,
        writes: Mutex<Vec<(Vec<u8>, Vec<String>)>>,
        fail: AtomicBool,
        /// `Some` 이면 다음 쓰기가 들어오자마자 앞의 것을 울리고 뒤의 것을 받을 때까지 멈춘다(한 번만).
        hold: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
        /// 쓰기 시도·래치 commit·`Queued` 방출을 한 줄에 세우는 사건 기록(래치 순서 단언용).
        trace: Arc<Mutex<Vec<String>>>,
    }
    struct GateTransport(Arc<Gate>);
    impl AgentTransport for GateTransport {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
            let InputEvent::Raw(bytes) = input;
            let listed: Vec<String> = self
                .0
                .core
                .queued_inputs()
                .lock()
                .rows()
                .iter()
                .map(|r| r.id.clone())
                .collect();
            self.0.trace.lock().unwrap().push("write".into());
            let hold = self.0.hold.lock().unwrap().take();
            if let Some((entered, release)) = hold {
                entered.send(()).unwrap();
                release.recv().unwrap();
            }
            if self.0.fail.load(Ordering::SeqCst) {
                return Err(PtyError::WriteFailed("gate: write refused".into()));
            }
            self.0.writes.lock().unwrap().push((bytes, listed));
            Ok(())
        }
        fn resize(&self, _cols: u16, _rows: u16) -> Result<(), PtyError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), PtyError> {
            Ok(())
        }
        fn shutdown(&self) {}
        fn capabilities(&self) -> TransportCaps {
            harness_caps()
        }
    }

    /// 출력 seq 와 함께 모으는 sink — 링 순서(seq)로 사건 순서를 단언한다(fanout 도착 순서는 링 순서가 아니다).
    struct SeqSink {
        id: SinkId,
        seen: Arc<Mutex<Vec<(u64, OutputEvent)>>>,
    }
    impl OutputSink for SeqSink {
        fn send(
            &self,
            frame: crate::types::OutputFrame<'_>,
        ) -> Result<(), crate::types::SinkError> {
            if let crate::types::OutputPayload::Event(e) = frame.payload {
                self.seen.lock().unwrap().push((frame.seq, e.clone()));
            }
            Ok(())
        }
        fn sink_id(&self) -> SinkId {
            self.id
        }
    }

    /// `Queued` 방출을 사건 기록에 적는 sink — fanout 은 emit 안에서 동기로 돈다.
    struct TraceSink {
        id: SinkId,
        trace: Arc<Mutex<Vec<String>>>,
    }
    impl OutputSink for TraceSink {
        fn send(
            &self,
            frame: crate::types::OutputFrame<'_>,
        ) -> Result<(), crate::types::SinkError> {
            if let crate::types::OutputPayload::Event(OutputEvent::QueuedInput(
                QueuedInputEvent::Queued { .. },
            )) = frame.payload
            {
                self.trace.lock().unwrap().push("queued".into());
            }
            Ok(())
        }
        fn sink_id(&self) -> SinkId {
            self.id
        }
    }

    /// 매니저와 같은 배선(공용 턴 관측 표 + 대기 목록 표 + 분류기)으로 선 claude JSON 세션 하나.
    struct Classified {
        session: Arc<AgentSession>,
        gate: Arc<Gate>,
        turns: Arc<TurnObservations>,
        pending: Arc<InputsPendingTable>,
        ack: Arc<DeliveryAck>,
        trace: Arc<Mutex<Vec<String>>>,
    }

    impl Classified {
        fn new(classify: crate::backend::TurnClassifier) -> Self {
            Self::build(classify, false)
        }

        /// 세션 id 래치를 실은 claude 세션 — 래치엔 id 가 이미 들어 있어(`latched_session_with` 와 같다) 첫 제출이
        /// 그 자리에서 commit 하고, 그 commit 이 쓰기·`Queued` 와 같은 사건 기록(`trace`)에 선다.
        fn with_session_id_latch() -> Self {
            let fx = Self::build(crate::backend::claude::classify_turn, true);
            fx.session.subscribe(Arc::new(TraceSink {
                id: uuid::Uuid::new_v4(),
                trace: fx.trace.clone(),
            }));
            fx.trace.lock().unwrap().clear();
            fx
        }

        fn build(classify: crate::backend::TurnClassifier, latched: bool) -> Self {
            let id = uuid::Uuid::new_v4();
            let turns = Arc::new(TurnObservations::new());
            let pending = Arc::new(InputsPendingTable::new());
            turns.register(id, 0);
            pending.register(id, 0);
            let core = Arc::new(
                OutputCore::new(
                    id,
                    0,
                    Arc::new(NoopStatusSink),
                    crate::output_core::TurnWiring::new(turns.clone(), classify),
                )
                .with_queued(crate::output_core::QueuedWiring {
                    registry: Arc::new(QueuedInputs::new()),
                    pending: pending.clone(),
                }),
            );
            let trace = Arc::new(Mutex::new(Vec::new()));
            let gate = Arc::new(Gate {
                core: core.clone(),
                writes: Mutex::new(Vec::new()),
                fail: AtomicBool::new(false),
                hold: Mutex::new(None),
                trace: trace.clone(),
            });
            let ack = Arc::new(DeliveryAck::new());
            let shell_cmd = crate::profile::AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec![],
            };
            let session = AgentSession::new(
                id,
                PathBuf::from("."),
                0,
                80,
                24,
                Arc::new(AtomicU8::new(0)),
                ShellBackend.capabilities(&shell_cmd),
                InputEncoder::ClaudeStreamJson,
                true,
                core,
                Box::new(GateTransport(gate.clone())),
            )
            .with_submit_pacing(Duration::ZERO, |_| {})
            .with_mid_turn(classified(), ack.clone());
            let session = if latched {
                let port_trace = trace.clone();
                let latch = SessionIdLatch::new(
                    id,
                    0,
                    Arc::new(move |raw: &str| {
                        port_trace.lock().unwrap().push(format!("commit:{raw}"))
                    }),
                );
                latch.offer("sid-1");
                session.with_session_id_latch(latch)
            } else {
                session
            };
            Self {
                session: Arc::new(session),
                gate,
                turns,
                pending,
                ack,
                trace,
            }
        }

        fn trace(&self) -> Vec<String> {
            self.trace.lock().unwrap().clone()
        }

        fn claude() -> Self {
            Self::new(crate::backend::claude::classify_turn)
        }

        fn id(&self) -> AgentId {
            self.session.id
        }

        /// 벤더 출력 한 줄로 코어를 턴 중으로 만든다(분류기가 진행으로 센다).
        fn start_turn(&self) {
            self.session.core.emit(OutputEvent::TextDelta {
                text: "thinking".into(),
                turn_id: None,
                message_id: None,
            });
            assert!(self.turns.is_in_turn(self.id(), 0), "전제: 턴 중");
        }

        /// 구독한 **뒤**의 사건만 (seq, 사건) 으로 모은다.
        fn watch(&self) -> Arc<Mutex<Vec<(u64, OutputEvent)>>> {
            let seen = Arc::new(Mutex::new(Vec::new()));
            self.session.subscribe(Arc::new(SeqSink {
                id: uuid::Uuid::new_v4(),
                seen: seen.clone(),
            }));
            seen.lock().unwrap().clear();
            seen
        }

        fn written(&self) -> Vec<Vec<u8>> {
            self.gate
                .writes
                .lock()
                .unwrap()
                .iter()
                .map(|(bytes, _)| bytes.clone())
                .collect()
        }

        fn listed(&self) -> Vec<String> {
            self.session
                .queued_inputs()
                .lock()
                .rows()
                .iter()
                .map(|r| r.id.clone())
                .collect()
        }

        fn hold_next_write(&self) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
            let (entered_tx, entered_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            *self.gate.hold.lock().unwrap() = Some((entered_tx, release_rx));
            (entered_rx, release_tx)
        }
    }

    fn seq_list_events(seen: &Arc<Mutex<Vec<(u64, OutputEvent)>>>) -> Vec<QueuedInputEvent> {
        let mut got = seen.lock().unwrap().clone();
        got.sort_by_key(|(seq, _)| *seq);
        got.into_iter()
            .filter_map(|(_, e)| match e {
                OutputEvent::QueuedInput(op) => Some(op),
                _ => None,
            })
            .collect()
    }

    fn echoes(seen: &Arc<Mutex<Vec<(u64, OutputEvent)>>>) -> usize {
        seen.lock()
            .unwrap()
            .iter()
            .filter(|(_, e)| matches!(e, OutputEvent::Structured { .. }))
            .count()
    }

    fn queued_op(id: &str, text: &str) -> QueuedInputEvent {
        QueuedInputEvent::Queued {
            id: id.into(),
            text: text.into(),
        }
    }

    // TRD §7-1: 한가 ∧ 명부 빔 → 에코, 사건 없음.
    #[test]
    fn an_idle_user_input_is_written_and_echoed_without_a_list_event() {
        let fx = Classified::claude();
        fx.ack.set_available();
        let seen = fx.watch();

        let outcome = fx
            .session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();

        assert_eq!(
            fx.written(),
            vec![InputEncoder::ClaudeStreamJson.encode(b"typed", outcome.msg_uuid)]
        );
        assert_eq!(echoes(&seen), 1, "오늘 경로 — 합성 에코 한 건");
        assert!(seq_list_events(&seen).is_empty());
    }

    // TRD §7-1: 턴 중(`Available`) → `Queued` 가 `send_input` 보다 먼저 링에 있다 · 합성 에코 없음.
    #[test]
    fn a_user_input_during_a_turn_is_queued_before_its_write_and_not_echoed() {
        let fx = Classified::claude();
        fx.ack.set_available();
        fx.start_turn();
        let seen = fx.watch();

        let outcome = fx
            .session
            .write_input_from("안녕".as_bytes(), InputOrigin::User)
            .unwrap();
        let id = outcome.msg_uuid.to_string();

        let writes = fx.gate.writes.lock().unwrap().clone();
        assert_eq!(writes.len(), 1);
        assert_eq!(
            writes[0].0,
            InputEncoder::ClaudeStreamJson.encode("안녕".as_bytes(), outcome.msg_uuid),
            "쓰는 줄은 오늘과 같다(uuid 단 user 줄)"
        );
        assert_eq!(writes[0].1, vec![id.clone()], "쓰기 순간 명부에 이미 섰다");
        assert_eq!(seq_list_events(&seen), vec![queued_op(&id, "안녕")]);
        assert_eq!(echoes(&seen), 0, "목록 갈래는 합성 에코를 내지 않는다");
    }

    // TRD §7-1: 턴 없음 ∧ 명부 있음 → Queued(PRD §3-5).
    #[test]
    fn a_user_input_while_items_wait_between_turns_is_queued() {
        let fx = Classified::claude();
        fx.ack.set_available();
        fx.session.core.emit(queued("q0"));
        assert!(!fx.turns.is_in_turn(fx.id(), 0));
        assert_eq!(fx.pending.get(fx.id(), 0), Some(true));

        let outcome = fx
            .session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();

        assert_eq!(
            fx.listed(),
            vec!["q0".to_string(), outcome.msg_uuid.to_string()]
        );
    }

    // TRD §7-1: drain `started` 의 `Delivered` 가 적힌 뒤(코어 턴 중 · 첫 출력 전) → Queued.
    #[test]
    fn a_user_input_after_a_drain_started_before_any_output_is_queued() {
        let fx = Classified::claude();
        fx.ack.set_available();
        fx.session.core.emit(queued("q0"));
        fx.session
            .core
            .emit(OutputEvent::QueuedInput(QueuedInputEvent::Delivered {
                id: "q0".into(),
            }));
        assert!(fx.listed().is_empty());
        assert_eq!(fx.pending.get(fx.id(), 0), Some(false));
        assert!(fx.turns.is_in_turn(fx.id(), 0), "받음 = 진행");
        let seen = fx.watch();

        let outcome = fx
            .session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();

        assert_eq!(fx.listed(), vec![outcome.msg_uuid.to_string()]);
        assert_eq!(echoes(&seen), 0);
    }

    /// 목록을 비우는 drain `Delivered` 의 emit 을 명부 환원 **뒤**, 턴 관측 **앞**에서 붙잡는 분류기 — id
    /// [`DRAIN_GATE_ID`] 한 건에만 걸리므로 병행하는 다른 시험과 섞이지 않는다.
    static DRAIN_GATE: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>> = Mutex::new(None);
    const DRAIN_GATE_ID: &str = "drain-gate";
    fn drain_gated_classifier(event: &OutputEvent) -> Option<TurnSignal> {
        if let OutputEvent::QueuedInput(QueuedInputEvent::Delivered { id }) = event {
            if id == DRAIN_GATE_ID {
                let gate = DRAIN_GATE.lock().unwrap().take();
                if let Some((entered, release)) = gate {
                    entered.send(()).unwrap();
                    release.recv().unwrap();
                }
            }
        }
        crate::backend::claude::classify_turn(event)
    }

    // TRD §7-1: 목록을 비우는 drain `Delivered` 의 emit 안(명부는 비었고 진행은 아직)에 들어온 입력 → Queued —
    //   분류가 명부를 읽지 않고 대기 목록 표를 먼저 읽는다.
    #[test]
    fn a_user_input_inside_the_draining_delivered_emit_is_queued() {
        let fx = Classified::new(drain_gated_classifier);
        fx.ack.set_available();
        fx.session.core.emit(queued(DRAIN_GATE_ID));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        *DRAIN_GATE.lock().unwrap() = Some((entered_tx, release_rx));

        let core = fx.session.core.clone();
        let pump = std::thread::spawn(move || {
            core.emit(OutputEvent::QueuedInput(QueuedInputEvent::Delivered {
                id: DRAIN_GATE_ID.into(),
            }))
        });
        entered_rx.recv().unwrap();
        assert!(fx.listed().is_empty(), "전제: 명부는 이미 비었다");
        assert!(!fx.turns.is_in_turn(fx.id(), 0), "전제: 진행은 아직이다");
        assert_eq!(
            fx.pending.get(fx.id(), 0),
            Some(true),
            "전제: 「비었다」도 아직"
        );

        let outcome = fx
            .session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();
        release_tx.send(()).unwrap();
        pump.join().unwrap();

        assert_eq!(fx.listed(), vec![outcome.msg_uuid.to_string()]);
        assert_eq!(
            fx.pending.get(fx.id(), 0),
            Some(true),
            "락 밖으로 미룬 「비었다」가 뒤에 선 「찼다」를 덮지 않는다"
        );
    }

    // TRD §7-1: 분류가 명부를 읽지 않는다 — 대기 목록 표만 본다(표와 명부가 갈린 상태를 손으로 만든다).
    #[test]
    fn the_classification_reads_the_pending_table_not_the_registry() {
        let fx = Classified::claude();
        fx.ack.set_available();
        fx.session.core.emit(queued("q0"));
        fx.pending.set(fx.id(), 0, 1_000, false);
        let seen = fx.watch();
        fx.session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();
        assert!(
            seq_list_events(&seen).is_empty(),
            "명부가 차 있어도 표가 비었으면 한가다"
        );
        assert_eq!(echoes(&seen), 1);

        let fx = Classified::claude();
        fx.ack.set_available();
        fx.pending.set(fx.id(), 0, 1_000, true);
        let outcome = fx
            .session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();
        assert_eq!(
            fx.listed(),
            vec![outcome.msg_uuid.to_string()],
            "명부가 비어 있어도 표가 찼으면 목록 갈래다"
        );
    }

    // TRD §7-1: Queued 갈래가 턴 표에 아무것도 안 적는다(입력 경로는 진행을 쓰지 않는다).
    #[test]
    fn the_queued_branch_writes_nothing_to_the_turn_table() {
        let fx = Classified::claude();
        fx.ack.set_available();
        fx.pending.set(fx.id(), 0, 1_000, true);
        let before = fx.turns.get(fx.id(), 0).unwrap();

        fx.session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();

        let after = fx.turns.get(fx.id(), 0).unwrap();
        assert!(!after.in_turn);
        assert_eq!(after.last_signal, before.last_signal);
    }

    // TRD §7-1: `Unknown` → Queued(가정) · 쓰기 뒤에 `Queued`(쓰기 순간 링에 없다).
    #[test]
    fn an_unknown_ack_announces_queued_only_after_a_successful_write() {
        let fx = Classified::claude();
        fx.start_turn();
        let seen = fx.watch();

        let outcome = fx
            .session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();
        let id = outcome.msg_uuid.to_string();

        let writes = fx.gate.writes.lock().unwrap().clone();
        assert_eq!(writes.len(), 1);
        assert!(writes[0].1.is_empty(), "쓰기 순간 명부에 없다");
        assert_eq!(seq_list_events(&seen), vec![queued_op(&id, "typed")]);
        assert_eq!(echoes(&seen), 0);
    }

    // TRD §7-1: `Unknown` 쓰기 실패 → 사건 0 건 · `Err`.
    #[test]
    fn an_unknown_ack_write_failure_emits_nothing() {
        let fx = Classified::claude();
        fx.start_turn();
        fx.gate.fail.store(true, Ordering::SeqCst);
        let seen = fx.watch();

        assert!(matches!(
            fx.session.write_input_from(b"typed", InputOrigin::User),
            Err(PtyError::WriteFailed(_))
        ));
        assert!(seen.lock().unwrap().is_empty());
        assert!(fx.listed().is_empty());
    }

    // TRD §7-1: 쓰기를 붙잡아 둔 동안 펌프 쪽에서 `AckUnavailable` 을 세우고 쓰기를 실패시키면 그 글이 말풍선
    //   (`Delivered`)이 되지 않는다 · 펌프 쪽 방출은 입력 자물쇠를 안 잡는다.
    #[test]
    fn a_failed_unknown_ack_write_is_not_turned_into_a_bubble_by_ack_unavailable() {
        let fx = Classified::claude();
        fx.start_turn();
        let seen = fx.watch();
        let (entered, release) = fx.hold_next_write();

        let session = fx.session.clone();
        let writer =
            std::thread::spawn(move || session.write_input_from(b"typed", InputOrigin::User));
        entered.recv().unwrap();
        assert!(
            fx.session.input_order.try_lock().is_err(),
            "전제: 쓰기가 자물쇠를 쥐고 있다"
        );

        let (done_tx, done_rx) = mpsc::channel();
        let (core, ack) = (fx.session.core.clone(), fx.ack.clone());
        std::thread::spawn(move || {
            assert!(ack.try_set_unavailable());
            core.emit(OutputEvent::QueuedInput(QueuedInputEvent::AckUnavailable {
                delivered: vec![],
            }));
            done_tx.send(()).unwrap();
        });
        done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("펌프 쪽 방출이 입력 자물쇠를 기다렸다");

        fx.gate.fail.store(true, Ordering::SeqCst);
        release.send(()).unwrap();
        assert!(writer.join().unwrap().is_err());

        assert_eq!(
            seq_list_events(&seen),
            vec![QueuedInputEvent::AckUnavailable { delivered: vec![] }],
            "쓰지 못한 글은 목록에도 말풍선에도 오르지 않는다"
        );
        assert!(fx.listed().is_empty());
        assert_eq!(echoes(&seen), 0);
    }

    // TRD §7-1: `Unavailable` → 턴 중이어도 오늘 에코(N1 (a)).
    #[test]
    fn an_unavailable_ack_takes_todays_path_even_during_a_turn() {
        let fx = Classified::claude();
        assert!(fx.ack.try_set_unavailable());
        fx.start_turn();
        let seen = fx.watch();

        fx.session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();

        assert_eq!(fx.written().len(), 1);
        assert_eq!(echoes(&seen), 1);
        assert!(seq_list_events(&seen).is_empty());
    }

    // TRD §7-1: `origin=Mail` → 턴 중이어도 `Queued` 없음, 오늘 에코 그대로(AC22) · 우편은 입력 자물쇠를 안 탄다.
    #[test]
    fn a_mail_input_takes_todays_path_even_during_a_turn() {
        let fx = Classified::claude();
        fx.ack.set_available();
        fx.start_turn();
        let seen = fx.watch();

        let _order = fx.session.input_order.lock().unwrap();
        let (done_tx, done_rx) = mpsc::channel();
        let session = fx.session.clone();
        std::thread::spawn(move || {
            session.write_input_from(b"a", InputOrigin::Mail).unwrap();
            session.write_input_observed(b"b").unwrap();
            session.submit_input_observed(b"c").unwrap();
            done_tx.send(()).unwrap();
        });
        done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("우편이 입력 자물쇠를 기다렸다");

        assert_eq!(fx.written().len(), 3);
        assert_eq!(echoes(&seen), 3);
        assert!(seq_list_events(&seen).is_empty());
    }

    // TRD §7-1: 쓰기 실패 → `Dropped{Rejected}` + `Err`.
    #[test]
    fn a_failed_queued_write_is_dropped_as_rejected_and_errors() {
        let fx = Classified::claude();
        fx.ack.set_available();
        fx.start_turn();
        fx.gate.fail.store(true, Ordering::SeqCst);
        let seen = fx.watch();

        assert!(matches!(
            fx.session.write_input_from(b"typed", InputOrigin::User),
            Err(PtyError::WriteFailed(_))
        ));

        let ops = seq_list_events(&seen);
        assert_eq!(ops.len(), 2, "{ops:?}");
        let QueuedInputEvent::Queued { id, .. } = &ops[0] else {
            panic!("첫 사건은 Queued: {ops:?}")
        };
        assert_eq!(
            ops[1],
            QueuedInputEvent::Dropped {
                id: id.clone(),
                cause: DropCause::Rejected
            }
        );
        assert!(fx.listed().is_empty());
        assert_eq!(echoes(&seen), 0);
        assert!(
            matches!(
                fx.session.cancel_queued_input(id),
                Err(CancelError::NotFound)
            ),
            "쓰지 못해 닫힌 항목의 취소는 NOT_FOUND 다"
        );
    }

    // ── ADR-0226 × ADR-0231: 세션 분류 갈래에서도 첫 제출 래치는 보내기 전에 한 번 센다 ──

    /// 턴 중 첫 사용자 입력(`Available`) — commit 한 번이 `Queued` 방출보다, 쓰기보다 먼저다.
    // ADR-0226
    #[test]
    fn a_first_queued_user_input_commits_the_latch_once_before_queued_and_the_write() {
        let fx = Classified::with_session_id_latch();
        fx.ack.set_available();
        fx.start_turn();

        let first = fx
            .session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();
        let second = fx
            .session
            .write_input_from(b"again", InputOrigin::User)
            .unwrap();

        assert_eq!(
            fx.trace(),
            vec!["commit:sid-1", "queued", "write", "queued", "write"],
            "영속이 첫 목록 사건·첫 쓰기보다 먼저 · 둘째 제출은 다시 commit 하지 않는다"
        );
        assert_ne!(first.msg_uuid, second.msg_uuid);
    }

    /// 턴 중 첫 사용자 입력(`Unknown`) — 쓰기 뒤 `Queued` 갈래에서도 commit 이 쓰기보다 먼저다.
    // ADR-0226
    #[test]
    fn a_first_unknown_ack_user_input_commits_the_latch_before_the_write() {
        let fx = Classified::with_session_id_latch();
        fx.start_turn();

        fx.session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();

        assert_eq!(fx.trace(), vec!["commit:sid-1", "write", "queued"]);
    }

    /// 사용자 종료 중이면 목록 갈래도 래치가 거절한다 — `Err` · 목록 사건 0 · 쓰기 0 · commit 0.
    // ADR-0226
    #[test]
    fn a_user_kill_refuses_a_queued_user_input_without_listing_writing_or_committing() {
        let fx = Classified::with_session_id_latch();
        fx.ack.set_available();
        fx.start_turn();
        let seen = fx.watch();
        fx.session.set_intent(TerminationIntent::UserKill);

        assert_user_kill_refusal(fx.session.write_input_from(b"typed", InputOrigin::User));

        assert!(seq_list_events(&seen).is_empty());
        assert!(fx.listed().is_empty());
        assert!(fx.written().is_empty());
        assert!(fx.trace().is_empty(), "{:?}", fx.trace());
    }

    // TRD §7-1: 입력 자물쇠 경합 — 글 쓰기를 붙잡아 둔 채 다른 스레드가 같은 id 를 취소해도 통로에 적힌 순서는
    //   글 줄 → 취소 줄이다.
    #[test]
    fn a_cancel_line_is_never_written_before_its_input_line() {
        let fx = Classified::claude();
        fx.ack.set_available();
        fx.start_turn();
        let (entered, release) = fx.hold_next_write();

        let session = fx.session.clone();
        let writer =
            std::thread::spawn(move || session.write_input_from(b"typed", InputOrigin::User));
        entered.recv().unwrap();
        let listed = fx.listed();
        assert_eq!(listed.len(), 1, "Queued 는 쓰기 앞에 섰다");
        let id = listed[0].clone();

        // ★헛돌지 않게★: 장벽으로 취소 스레드가 취소 호출 바로 앞까지 왔음을 확인하고, 그 순간 글 쓰기가 입력
        //   자물쇠를 쥐고 있음을 확인한다 — 그래서 아래 창 동안 취소는 실제로 그 자물쇠를 다투는 중이다. 창의 길이는
        //   순서의 근거가 아니라, 자물쇠를 건너뛰는 회귀가 제 모습을 드러낼 유예다.
        let started = Arc::new(std::sync::Barrier::new(2));
        let (done_tx, done_rx) = mpsc::channel();
        let session = fx.session.clone();
        let cancel_id = id.clone();
        let canceller_started = started.clone();
        let canceller = std::thread::spawn(move || {
            canceller_started.wait();
            let answer = session.cancel_queued_input(&cancel_id);
            done_tx.send(()).unwrap();
            answer
        });
        started.wait();
        assert!(
            fx.session.input_order.try_lock().is_err(),
            "전제: 취소가 출발한 순간 글 쓰기가 자물쇠를 쥐고 있다"
        );
        assert!(
            done_rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "취소는 글 쓰기가 자물쇠를 놓기 전에 끝나지 않는다"
        );
        assert!(fx.written().is_empty(), "붙잡힌 글 앞에 아무것도 안 적혔다");
        assert!(
            matches!(phase_of(&fx.session, &id), Some(RowPhase::Queued)),
            "자물쇠를 기다리는 동안 취소 요청도 기록되지 않았다"
        );
        release.send(()).unwrap();

        let outcome = writer.join().unwrap().unwrap();
        done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("글 쓰기가 끝나면 취소가 자물쇠를 얻는다");
        assert_eq!(canceller.join().unwrap().unwrap(), CancelOutcome::Requested);
        assert_eq!(
            fx.written(),
            vec![
                InputEncoder::ClaudeStreamJson.encode(b"typed", outcome.msg_uuid),
                test_cancel_line(&id),
            ]
        );
        assert!(matches!(
            phase_of(&fx.session, &id),
            Some(RowPhase::Cancelling { .. })
        ));
    }

    // TRD §7-1: 동시 두 입력의 분류와 방출 순서가 일치한다 — 한가할 때 겹친 둘은 정확히 하나가 오늘 경로(에코)고,
    //   먼저 쓴 쪽이 그 하나다.
    #[test]
    fn concurrent_user_inputs_are_classified_in_the_order_they_are_written() {
        for _ in 0..50 {
            let fx = Classified::claude();
            fx.ack.set_available();
            let seen = fx.watch();
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let writers: Vec<_> = ["one", "two"]
                .into_iter()
                .map(|text| {
                    let (session, barrier) = (fx.session.clone(), barrier.clone());
                    std::thread::spawn(move || {
                        barrier.wait();
                        session
                            .write_input_from(text.as_bytes(), InputOrigin::User)
                            .unwrap()
                            .msg_uuid
                            .to_string()
                    })
                })
                .collect();
            let ids: Vec<String> = writers.into_iter().map(|w| w.join().unwrap()).collect();

            let written = fx.written();
            assert_eq!(written.len(), 2);
            let first = ids
                .iter()
                .find(|id| String::from_utf8_lossy(&written[0]).contains(id.as_str()))
                .unwrap()
                .clone();
            let second = ids.iter().find(|id| **id != first).unwrap().clone();
            assert_eq!(echoes(&seen), 1, "한가로 분류된 것은 하나뿐이다");
            let echoed = seen
                .lock()
                .unwrap()
                .iter()
                .find_map(|(_, e)| match e {
                    OutputEvent::Structured { json, .. } => Some(json.clone()),
                    _ => None,
                })
                .unwrap();
            assert!(echoed.contains(&first), "먼저 쓴 것이 에코됐다");
            assert_eq!(fx.listed(), vec![second], "뒤에 쓴 것이 목록에 섰다");
        }
    }

    // 취소 대 펌프의 받음 — 확인과 `CancelRequested` 가 한 replay 구간이라, `Requested` 로 답했으면
    //   `CancelRequested` 가 링에서 `Delivered` 보다 앞이고, `NOT_FOUND` 면 사건도 취소 줄도 없다.
    #[test]
    fn a_cancel_racing_the_delivered_answers_consistently_with_the_ring() {
        for _ in 0..200 {
            let fx = Classified::claude();
            fx.ack.set_available();
            fx.session.core.emit(queued("q1"));
            let seen = fx.watch();
            let barrier = Arc::new(std::sync::Barrier::new(2));

            let (core, b) = (fx.session.core.clone(), barrier.clone());
            let pump = std::thread::spawn(move || {
                b.wait();
                core.emit(OutputEvent::QueuedInput(QueuedInputEvent::Delivered {
                    id: "q1".into(),
                }));
            });
            barrier.wait();
            let got = fx.session.cancel_queued_input("q1");
            pump.join().unwrap();

            let ops = seq_list_events(&seen);
            let requested = ops
                .iter()
                .position(|op| *op == QueuedInputEvent::CancelRequested { id: "q1".into() });
            let delivered = ops
                .iter()
                .position(|op| *op == QueuedInputEvent::Delivered { id: "q1".into() })
                .expect("받음은 늘 링에 선다");
            match got {
                Ok(CancelOutcome::Requested) => {
                    assert!(requested.is_some_and(|r| r < delivered), "{ops:?}");
                    assert_eq!(fx.written(), vec![test_cancel_line("q1")]);
                }
                Err(CancelError::NotFound) => {
                    assert!(requested.is_none(), "{ops:?}");
                    assert!(fx.written().is_empty());
                }
                other => panic!("{other:?}"),
            }
            assert!(fx.listed().is_empty(), "결말은 받음이다");
        }
    }

    // TRD §7-1(L744 세션): 오류 뒤 멈춤 ∧ 한가 ∧ 목록 빔 → Direct(쓰기 + 에코 — 세션은 그 칸을 읽지 않는다).
    #[test]
    fn a_halted_idle_session_still_sends_a_user_input_directly() {
        let fx = Classified::claude();
        fx.ack.set_available();
        fx.turns.observe(fx.id(), 0, 1, TurnSignal::Failed);
        fx.turns
            .observe(fx.id(), 0, 2, TurnSignal::Ended(TurnEndKind::Clean));
        let fact = fx.turns.get(fx.id(), 0).unwrap();
        assert!(fact.last_end_failed && !fact.in_turn, "전제: 오류 뒤 멈춤");
        let seen = fx.watch();

        fx.session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();

        assert_eq!(fx.written().len(), 1);
        assert_eq!(echoes(&seen), 1);
        assert!(seq_list_events(&seen).is_empty());
    }
}
