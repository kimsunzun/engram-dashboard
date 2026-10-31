//! 프로토콜 의미론(순수 상태/결정 함수) — epoch 가드 · pending 매칭 (S14 모듈①, ADR-0037).
//!
//! ## 무엇 / 왜 (load-bearing)
//! 프론트 `src/api/protocolClient.ts` 가 carrier-무관 JS 한 곳에 모았던 전송 의미론을 Rust 로 옮긴 것이다
//! (ADR-0037 D1=A안). 데몬 단일 WS 연결에서 라우팅 전 1회 적용하고, 창 N개로는 깨끗한 청크만 fan-out 한다.
//! **epoch 가드는 여기 Rust 가 유일한 진실원** — JS 2차 방어선이 없어 판정이 한 치 어긋나면 출력이 화면에서
//! 전멸한다. dedup/진도는 이 레이어에 없다(ADR-0046 이 ADR-0037 의 거처 조항을 웹뷰 뷰(slot) 단위로 개정).
//!
//! ## ★순수성(테스트 격리)★
//! 이 모듈은 소켓·tokio runtime·Tauri 의존이 **0**이다. 모든 함수는 `&mut SubState` / `&mut PendingMap`
//! 를 인자로 받아 결정만 내린다(부수효과는 호출자 = 연결 task 가 수행: 실제 frame 배달·wire send·
//! reply resolve). 그래서 단위 테스트가 런타임 없이 동기로 돈다. `protocolClient.test.ts` 의 케이스
//! 중 **순수 결정에 해당하는 부분만** 옮긴다(event-routing/콜백 배선은 제외 — 아래 tests mod 주석).
//!
//! ## 와이어 타입 정합 (TS number → Rust 분리)
//! TS 는 epoch/seq 가 둘 다 `number` 였지만, protocol crate wire 는 `epoch: u32` · `seq: u64` 다
//! (`codec.rs`·`messages.rs`). 그대로 재사용한다(로컬 재정의 안 함). `request_id` 는 `RequestId(Uuid)`.

use std::collections::HashMap;
use std::fmt;

use engram_dashboard_command::CommandEnvelope;
use engram_dashboard_protocol::{AgentCommand, AgentEvent, RequestId};
use serde_json::value::RawValue;

// ── 명령 칸 끝자락 · request_id 추출(request/reply 상관) ──────────────────────────────
// ADR-0272 결정 4: 패킷의 명령 버스 칸은 원문 JSON 이라 protocol 은 그 안의 번호를 못 읽는다 — 짝 맞추기는
//   봉투 · 답장을 푸는 끝자락(여기)이 진다. 데몬은 받는 쪽 거절에 실을 번호를 자기 끝자락에서 따로 읽는다.

/// 패킷의 봉투 칸(원문 JSON — `AgentCommand::Command` · `AgentEvent::CommandRequest`)을 명령 버스 봉투로
/// 푼다(ADR-0272 결정 4).
///
/// `Err` = 패킷은 풀렸는데 봉투 모양이 틀렸다(ADR-0272 결정 6). 셸은 그것을 「요청 번호 없음」으로 다룬다 —
/// 보낼 때는 대기 슬롯을 만들지 않고([`command_request_id`] 가 `None`), 받을 때는 답할 번호가 없어 실행도
/// 답장도 하지 않는다(연결 루프의 `CommandRequest` 갈래).
// ADR-0272
pub fn envelope_from_wire(raw: &RawValue) -> Result<CommandEnvelope, serde_json::Error> {
    serde_json::from_str(raw.get())
}

/// 패킷의 답장 칸(원문 JSON — `AgentEvent::CommandReply` · `AgentCommand::CommandOutcome`)을 명령 버스 답장으로
/// 푼다(ADR-0272 결정 4).
///
/// `Err` = 패킷은 풀렸는데 답장 모양이 틀렸다(ADR-0272 결정 6). 셸은 그것도 「요청 번호 없음」으로 다룬다 —
/// 받은 답장이면 깨울 대기 슬롯을 못 찾아([`event_reply_request_id`] 가 `None`) 로그만 남기고 버린다(연결 루프의
/// `CommandReply` 갈래).
// ADR-0272
pub fn reply_from_wire(
    raw: &RawValue,
) -> Result<engram_dashboard_command::CommandReply, serde_json::Error> {
    serde_json::from_str(raw.get())
}

/// 인바운드 명령의 결말을 결말 패킷(`AgentCommand::CommandOutcome`)의 원문 칸으로 싼다(ADR-0272 결정 4).
// ADR-0272
pub fn reply_to_wire(
    reply: &engram_dashboard_command::CommandReply,
) -> Result<Box<RawValue>, serde_json::Error> {
    serde_json::value::to_raw_value(reply)
}

/// 웹뷰가 보낸 패킷의 명령 칸(봉투 · 답장 — `Command` · `CommandOutcome`)을 한 번 풀어 보고 구조체 순서 바이트로
/// 다시 싼다 — 그 밖 variant 는 그대로 돌려준다.
///
/// 웹뷰 길은 패킷을 `serde_json::Value` 에서 풀어 원문 칸이 키 정렬된 바이트로 담긴다(`preserve_order` 없음) —
/// 다시 싸야 셸이 직접 보내는 칸과 같은 바이트가 나간다. `Err` = 칸이 깨졌다: 봉투는 일방 발사로 흘러가 웹뷰의 대기가
/// 답장 상한까지 매달리고, 답장은 데몬이 번호 없는 거절로 받아 웹뷰가 실패를 모른다 — 그래서 부르는 쪽이 invoke 를
/// 바로 실패시킨다.
// ADR-0272
pub fn reseal_webview_command_slots(cmd: AgentCommand) -> Result<AgentCommand, serde_json::Error> {
    match cmd {
        AgentCommand::Command { envelope } => {
            let envelope = serde_json::value::to_raw_value(&envelope_from_wire(&envelope)?)?;
            Ok(AgentCommand::Command { envelope })
        }
        AgentCommand::CommandOutcome { reply } => {
            let reply = reply_to_wire(&reply_from_wire(&reply)?)?;
            Ok(AgentCommand::CommandOutcome { reply })
        }
        other => Ok(other),
    }
}

/// 봉투 · 답장이 품은 상관 키를 protocol 의 상관 키로 옮긴다 — ★uuid 를 그대로 나른다★(새 값을 만들지 않는 것이
/// 계약이다 — 상관 키는 왕복 전 구간 동일해야 한다, ADR-0081 결정 4).
fn request_id_from_bus(id: engram_dashboard_command::RequestId) -> RequestId {
    RequestId(id.0)
}

/// 명령에 실린 request_id 를 꺼낸다. side-effect 명령(Spawn/Kill/…)은 모두 request_id 를 갖지만,
/// 일부(Subscribe/Unsubscribe/Resize)는 request_id 가 없다(데몬이 reply 를 안 보냄) → `None`.
/// (핸드셰이크는 이 표에 없다 — 명령이 아니라 따로 된 프레임(`handshake::AuthFrame`)이라 이 함수에 오지
///  않는다. ADR-0129 0-4.)
///
/// ★계약★: 데몬 클라이언트의 pending 매칭(`send_command`)은 reply 를 기대하므로 request_id 가 있는
/// 명령에만 쓴다. None 인 명령을 넣으면 매칭할 키가 없어 영구 pending(hang)이 되므로 호출자가 None 을 거른다.
/// ★봉투가 깨진 `Command` 도 `None` 이다★(ADR-0272 결정 6) — 번호를 못 읽으니 대기 슬롯을 만들지 않는다.
/// 웹뷰가 보낸 그런 패킷은 여기 오기 전에 [`reseal_webview_command_slots`] 가 막아 invoke 가 실패한다.
pub fn command_request_id(cmd: &AgentCommand) -> Option<RequestId> {
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
        // 트리 rename(ADR-0061 리치화) — Ack 매칭 대상(SetProfileAutoRestore 와 동형).
        | AgentCommand::RenameProfile { request_id, .. }
        // 트리 reparent(ADR-0072 계층) — Ack 매칭 대상(RenameProfile 와 동형).
        | AgentCommand::ReparentProfile { request_id, .. }
        | AgentCommand::GetSnapshot { request_id, .. }
        // 프리셋 CRUD(ADR-0061) — 넷 다 request_id 동봉(reply 매칭 대상).
        | AgentCommand::ListPresets { request_id }
        | AgentCommand::CreatePreset { request_id, .. }
        | AgentCommand::DeletePreset { request_id, .. }
        // 프리셋 rename(ADR-0061 리치화) — Ack 매칭 대상.
        | AgentCommand::RenamePreset { request_id, .. }
        // 봉투 포맷 전역 스위치(ADR-0096) — Ack 매칭 대상(데몬이 상태 변경 후 Ack echo).
        | AgentCommand::SetEnvelopeFormat { request_id, .. }
        // 명령 버스 등록 wire(ADR-0155/0156) — 셋 다 답장을 기다린다. 등록·차분은 Ack, 조회는
        //   전용 reply CommandList 로 온다(아래 event_reply_request_id 가 그 짝).
        | AgentCommand::RegisterCommands { request_id, .. }
        | AgentCommand::UpdateCommands { request_id, .. }
        | AgentCommand::ListCommands { request_id }
        // 대기 입력 목록(ADR-0231) — 둘 다 전용 reply(QueuedInputs/QueuedInputCancelReply)를 기다린다.
        | AgentCommand::ListQueuedInputs { request_id, .. }
        | AgentCommand::CancelQueuedInput { request_id, .. }
        // ⟳ — 답은 기존 `Ack` 라 아래 event_reply_request_id 에 새 갈래가 없다.
        | AgentCommand::RefreshUsageLimits { request_id, .. } => Some(*request_id),
        // ★명령 요청은 상관 대상이다 — 키만 봉투 안에 있다★(ADR-0155). 형제들처럼 제 칸이 없다고 여기서
        //   None 을 고르면 셸이 답장을 받고도 깨울 슬롯을 못 만들어 마감시각까지 매달린다. 아래
        //   event_reply_request_id 의 `CommandReply` 갈래와 **한 쌍으로만** 성립한다.
        // ★uuid 는 그대로 옮긴다★ — 홉에서 새 키가 나면 답장이 이 요청에 못 붙는다(`request_id_from_bus`).
        // ADR-0272 결정 4 · 6: 봉투를 풀어 번호를 읽는다 — 깨진 봉투는 번호 없음(위 doc).
        AgentCommand::Command { envelope } => envelope_from_wire(envelope)
            .ok()
            .map(|envelope| request_id_from_bus(envelope.request_id)),
        // request_id 없는 명령 — reply 매칭 대상 아님(데몬이 전용 reply 를 안 echo).
        AgentCommand::Resize { .. }
        | AgentCommand::Subscribe { .. }
        | AgentCommand::Unsubscribe { .. }
        | AgentCommand::UsageSubscribe { .. }
        // ★CommandOutcome 은 **내가 보내는 답장**이라 여기 None 이다★ — 상관 키가 `reply` 안에 있지만 그것은
        //   데몬이 나에게 준 요청의 키이고, 내 pending 표의 키가 아니다. Some 을 돌려주면 답장을 보내는 그
        //   순간 그 키로 빈 pending 슬롯이 생겨 연결이 끊길 때까지 남는다.
        | AgentCommand::CommandOutcome { .. } => None,
    }
}

/// reply 이벤트에 실린 request_id 를 꺼낸다(매칭용). **전용 reply**(요청 하나에 붙는 답)만 request_id 를
/// echo 하고, broadcast(AgentListUpdated/StatusChanged/…)는 `None` 이라 pending 매칭을 우회한다(편승 매칭
/// 제거). ★어느 variant 가 어느 쪽인지의 목록은 아래 match 하나다★ — 여기 베껴 두면 variant 가 늘 때
/// 한쪽만 고쳐진다.
///
/// ★Error 분기★: `Error{request_id: Some(_)}` = 특정 명령 실패(매칭해 reject), `Error{request_id: None}`
/// = 명령 무관 오류(broadcast 성격, 매칭 안 함). SubscribeAck 는 request_id 가 없어(agent_id 기반) 여기
/// None — 데몬 클라이언트의 pending 매칭 대상이 아니다(Subscribe 는 request_id 없는 명령).
/// ★답장이 깨진 `CommandReply` 도 `None` 이다★(ADR-0272 결정 6) — 깨울 슬롯을 못 찾으니 그 대기는 부르는
/// 쪽의 답장 상한으로 끝난다(오늘은 `forward_daemon_command` 의 30s). 상한 없이 부르는 쪽은 연결이 끊길
/// 때까지 기다린다 — 그 잔여는 `daemon_client/mod.rs` 의 `send_command_with_origin` 주석 「알려진 잔여」.
pub fn event_reply_request_id(ev: &AgentEvent) -> Option<RequestId> {
    match ev {
        AgentEvent::Ack { request_id }
        | AgentEvent::AgentList { request_id, .. }
        | AgentEvent::ProfileList { request_id, .. }
        // PresetList = 전용 reply(request_id echo, ADR-0061). PresetListUpdated 는 broadcast(아래 None).
        | AgentEvent::PresetList { request_id, .. }
        | AgentEvent::Snapshot { request_id, .. }
        | AgentEvent::Created { request_id, .. }
        // ★CommandList 는 상관 대상이다★(ADR-0155) — ListCommands 조회의 전용 reply라 AgentList/
        //   ProfileList/PresetList 와 같은 자리다. 여기서 None 을 고르면 셸이 확실히 매달린다: 위
        //   command_request_id 가 ListCommands 에 Some 을 돌려주므로 pending 매칭이 슬롯을 만드는데,
        //   그걸 깨울 짝이 없어져 연결이 끊길 때까지 안 풀린다. 두 함수는 명령↔답장 쌍마다 같이
        //   움직여야 하는 한 쌍이다 — 이 쌍의 고정은 아래 tests 가 박는다.
        | AgentEvent::CommandList { request_id, .. }
        // 대기 입력 목록(ADR-0231) — 위 command_request_id 의 두 갈래와 한 쌍이다.
        | AgentEvent::QueuedInputs { request_id, .. }
        | AgentEvent::QueuedInputCancelReply { request_id, .. }
        | AgentEvent::Spawned { request_id, .. } => Some(*request_id),
        // ★명령 답장도 상관 대상이다 — 위 `AgentCommand::Command` 갈래의 짝★(ADR-0155). 요청이 Some 을
        //   주는데 여기서 None 을 고르면 그 슬롯을 깨울 짝이 없어져 연결이 끊길 때까지 안 풀린다.
        // ADR-0272 결정 4 · 6: 답장을 풀어 번호를 읽는다 — 깨진 답장은 번호 없음(`reply_from_wire` doc). 번호만
        //   골라 읽지 않는다(같은 ADR 「거부한 대안」 — 부분 파싱).
        AgentEvent::CommandReply { reply } => reply_from_wire(reply)
            .ok()
            .map(|reply| request_id_from_bus(reply.request_id)),
        AgentEvent::Error { request_id, .. } => *request_id,
        // request_id 없는 이벤트(broadcast 또는 agent_id 기반) — pending 매칭 대상 아님.
        AgentEvent::Hello { .. }
        | AgentEvent::SubscribeAck { .. }
        // SubscribeFailed 도 agent_id 기반이다(Subscribe 는 request_id 없는 명령) — 매칭 대상 아님.
        | AgentEvent::SubscribeFailed { .. }
        | AgentEvent::Output { .. }
        | AgentEvent::ReplayComplete { .. }
        | AgentEvent::StatusChanged { .. }
        | AgentEvent::AgentListUpdated { .. }
        | AgentEvent::RestoreResult { .. }
        | AgentEvent::InputLeaseChanged { .. }
        | AgentEvent::ProfileListUpdated { .. }
        // PresetListUpdated = broadcast(request_id 없음, ADR-0061) — pending 매칭 대상 아님.
        | AgentEvent::PresetListUpdated { .. }
        // 구독한 연결로 가는 사용량 한 장 — ⟳ 의 답(`Ack`)이 아니다.
        | AgentEvent::UsageLimitsUpdated { .. }
        // ★CommandRequest 는 **들어오는 요청**이라 여기 None 이다(load-bearing)★ — 봉투에 request_id 가
        //   있으니 Some 이 자연스러워 보이지만, 그 키는 **데몬이 만든 요청의 키**이고 내 pending 표의 키가
        //   아니다. Some 을 돌려주면 받는 쪽이 이것을 「내가 기다린 답장」으로 읽어 봉투를 삼킨다 — 명령은
        //   실행되지 않고 보낸 쪽은 마감시각까지 매달린다.
        | AgentEvent::CommandRequest { .. } => None,
    }
}

/// 셸이 보낸 명령이 성공 답으로 끝나지 못했을 때 **누가 실패를 말했나** — 데몬의 답인가, 셸 쪽 사정인가.
///
/// - `Daemon` = 데몬이 그 명령에 `Error` 로 답했다 — 코드 접두가 있든 없든, 셸이 아는 코드든 아니든.
/// - `Local` = 데몬의 답 없이 셸 쪽에서 끝났다(미연결 · 미전송 · 송신 실패 · 끊김으로 결과 불명 · 답 유실 ·
///   승계 등 — 문구는 자리마다 다르다). ★「적용되지 않았다」가 아니다★ — 결과 불명
///   (`connection::SENT_OUTCOME_UNKNOWN`)처럼 데몬이 이미 실행했을 수 있는 갈래가 섞여 있다.
///
/// `Display` 는 담은 글을 꾸밈 없이 그대로 낸다 — `DaemonClient::send_command` 가 이것으로 `String` 을 만들므로,
/// 출처를 안 보는 호출자는 데몬 문구 원문 · 로컬 문구를 바이트 그대로 받는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandFailure {
    Daemon { message: String },
    Local(String),
}

impl fmt::Display for CommandFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandFailure::Daemon { message } | CommandFailure::Local(message) => {
                f.write_str(message)
            }
        }
    }
}

/// 대기 슬롯의 결말(`connection::CommandReply`)을 출처 타입으로 가른다(ADR-0281).
///
/// `Ok(Error{..})` → `Daemon` · 그 밖의 `Ok` → 그대로 · `Err` → `Local`. 슬롯의 `Err` 가 셸 로컬 실패뿐인
/// 것은 슬롯 계약이다(`connection::CommandReply` doc).
/// ★문구를 읽지 않는다★ — 출처는 「데몬이 답했나」가 정한다. 코드 접두가 없든 셸이 모르는 코드든 데몬이
/// 답했으면 `Daemon` 이다(코드 추가는 허용된 확장 — `engram_dashboard_command::ErrorCode` doc).
pub fn classify_reply(slot: Result<AgentEvent, String>) -> Result<AgentEvent, CommandFailure> {
    match slot {
        Ok(AgentEvent::Error { message, .. }) => Err(CommandFailure::Daemon { message }),
        Ok(other) => Ok(other),
        Err(local) => Err(CommandFailure::Local(local)),
    }
}

/// 에이전트별 출력 구독 상태(JS `protocolClient.ts` `SubState` 승격).
///
/// epoch 가드의 per-agent 진실원. 연결 task 가 agent_id → SubState 맵으로 들고,
/// SubscribeAck/output frame 마다 아래 결정 함수에 `&mut` 로 넘긴다.
///
/// 이 struct 는 epoch 가드만 담는다(ADR-0046).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SubState {
    // 마지막 `SubscribeAck.current_epoch`. output frame epoch 매칭용(불일치 frame 폐기) +
    // 재연결 resubscribe wire epoch. `None` = 아직 Ack 못 받음(첫 구독 직후).
    pub epoch: Option<u32>,
}

impl SubState {
    // 신규 구독 직후 초기값(JS `subscribeOutput`: epoch=undefined).
    // 같은 agent_id 재구독 시 이걸로 덮는다(컴포넌트가 epoch 바뀌면 재구독).
    pub fn new() -> Self {
        Self { epoch: None }
    }
}

/// output frame 의 epoch 가드 판정 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpochDecision {
    /// epoch 가드 통과 — 원본 frame 그대로 fan-out.
    Deliver,
    // epoch 불일치 — 옛 세션 잔여 frame. 화면 오염 방지로 버린다.
    DropEpochMismatch,
}

// pending request_id → reply 콜백 슬롯의 키→값 타입. 값(T)은 호출자가 정한다 —
// 운영에선 `oneshot::Sender<reply>`, 테스트에선 결과를 적재하는 mock. 이 모듈은 매칭 로직만 소유한다.
pub type PendingMap<T> = HashMap<RequestId, T>;

// ── output frame epoch 가드 ─────────────────────────────────────────────────────────
/// epoch 가드만 판정한다. dedup(seq high-water)은 src-tauri 에 없다 — 프론트 뷰가 전담한다(ADR-0046).
///
/// ★high-water 를 이 레이어에 되돌리면 안 되는 이유(ADR-0046 결정 3 — 재도입은 명시 위반)★: 진도를
/// agentId 단위로 들면 같은 에이전트를 보는 뷰(slot) 여럿이 서로를 굶긴다 — 뷰 A 가 seq 100 을 렌더한
/// 뒤엔 0부터 필요한 뷰 B 도 seq<=100 을 못 받는다(버그 B). epoch 는 세션 단위라 뷰와 무관 — "이 epoch 의
/// frame 인가?" 만 여기서 1회 판정하면 충분하다.
///
/// ★epoch=None(첫 Ack 전) 통과(load-bearing)★: epoch 기준이 없으면 비교를 건너뛰고 통과시킨다 —
/// Ack 전 도착 frame 도 배달해야 초반 출력이 사라지지 않는다(JS `st.epoch !== undefined` 가드와 동형).
pub fn decide_epoch(st: &SubState, frame_epoch: u32) -> EpochDecision {
    if let Some(cur) = st.epoch {
        if frame_epoch != cur {
            return EpochDecision::DropEpochMismatch;
        }
    }
    EpochDecision::Deliver
}

// ── SubscribeAck 처리(JS handleEvent 의 SubscribeAck 분기 승격) ───────────────────────
/// SubscribeAck 수신 시 epoch 갱신. epoch 가 변경됐으면 `true`, 동일하면 `false` 반환.
///
/// ★반환값 `bool`(epoch 변경 시 true)★: 운영 호출자(`connection.rs`)는 `let _ =` 로 의도적으로 버린다 —
/// ADR-0046 이후 src-tauri 에 리셋할 진도 상태가 없고, epoch 채택은 프론트가 성공 마커로 한다. 반환 계약을
/// 박는 쪽은 이 파일 tests(첫 Ack·동일 epoch·변경 3케이스가 반환값을 단언) — 지우면 그 셋이 깨진다.
///
/// ★버그 B 가드(유지)★: `replay_from` 은 "데몬이 보내는 첫 seq"이지 "마지막으로 본 seq"가 아니다 —
/// dedup 기준으로 쓰면 첫 정상 프레임을 버린다. 그래서 이 함수는 replay_from 을 인자로 받지 않는다.
///
/// ★epoch 변경(ADR-0007 최초 도입 · 화신 표식 의미=ADR-0163 · 재부착 계기=ADR-0164)★: epoch 이 바뀌면(데몬 재기동·에이전트 재시작) 새 스트림이다 —
/// 재구독은 프론트가 권위 명부 관측(`observeRoster`)으로 걸고, 구독 deps `[viewId, agentId]`는 화신 표식을 의도적으로 제외한다(ADR-0164 결정 8). 이 함수는 SubState.epoch 만 새 값으로 넘긴다.
pub fn apply_subscribe_ack(st: &mut SubState, current_epoch: u32) -> bool {
    let epoch_changed = match st.epoch {
        Some(prev) => current_epoch != prev,
        None => false, // 비교 기준이 없던 최초 설정 — None → Some 은 "변경" 이 아니다.
    };
    st.epoch = Some(current_epoch);
    epoch_changed
}

// ── pending request_id 매칭(JS resolvePending/rejectPending + connected→down reject 승격) ──
// request_id 에 대응하는 pending 슬롯을 꺼낸다(매칭 성공 시 맵에서 제거 — 1회성). `None` 이면 무시
// (JS `resolvePending` 의 "없으면 no-op"). 호출자가 반환된 슬롯을 resolve/reject 한다.
//
// ★편승 매칭 제거(protocol v2)★: 조회도 전용 reply variant(AgentList/ProfileList/Snapshot)가
// request_id 를 echo 하므로 broadcast(AgentListUpdated 등)가 조회 응답에 편승하지 않는다 — 호출자가
// broadcast variant 는 이 함수를 안 거치고 콜백만 호출하면 된다.
pub fn take_pending<T>(pending: &mut PendingMap<T>, request_id: &RequestId) -> Option<T> {
    pending.remove(request_id)
}

// connected→비connected 전이 시 **모든** pending 을 꺼내 비운다(JS handleClose 의 일괄 reject).
// 호출자가 각 슬롯을 "connection lost" 로 reject 한다. spawn/kill 등 1회성이라 자동 재전송은 중복
// 부작용 위험 — 호출자가 catch 후 재시도가 단순·안전(JS 주석 보존). 반환 후 맵은 빈다.
//
// ★1회성★: 한 번 끊기면 모든 in-flight 가 동시에 죽는다 — 부분 reject 없음(전부 또는 없음).
pub fn drain_pending<T>(pending: &mut PendingMap<T>) -> Vec<T> {
    pending.drain().map(|(_k, v)| v).collect()
}

#[cfg(test)]
mod tests {
    //! `src/api/protocolClient.test.ts` 케이스 중 **순수 상태/결정 함수**에 해당하는 부분만 옮긴다
    //! (대응 TS 케이스명을 각 테스트에 주석으로 단다). 1:1 매핑이 아니다 — TS 한 케이스가 라우팅+결정을
    //! 섞으면 결정 부분만 이식한다.
    //!
    //! ★검증 범위★: seq dedup/high-water 테스트는 없다 — 그 진도 상태 자체가 src-tauri 에 없다(ADR-0046).
    //!   여기 박는 것은 순수 결정 함수뿐이다(봉투 · 답장 칸 풀기 · request_id 추출·reply 분류 · pending 매칭 · epoch 가드 ·
    //!   apply_subscribe_ack 반환값).
    //!
    //! ★event-routing(M=5)은 여기서 검증하지 않는다★: InboundMessage variant 라우팅 + 콜백 호출/
    //!   unsubscribe 는 순수 결정이 아니라 protocol_state 단독으론 보호 대상이 없다 → 실제 배선이 도는
    //!   연결 task main_loop · eventBus 표면 테스트가 담당한다(헛 단언 이식 금지). 해당 TS 케이스 목록 =
    //!   아래 event-routing 구획 주석. transport.close/connect/disconnect 위임(`:399~`,`:411~`)도 carrier
    //!   배선이라 여기 없다.

    use super::*;
    use engram_dashboard_protocol::AgentCommand;

    fn rid(s: &str) -> RequestId {
        // 테스트용 결정적 RequestId — 문자열을 FNV-1a 64bit 해시 → u128 로 Uuid 생성(v5 feature 없이도
        // 서로 다른 입력=서로 다른 id, 같은 입력=같은 id). 충돌은 테스트 범위에선 무시 가능.
        let mut h: u64 = 0xcbf29ce484222325;
        for b in s.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x00000100000001B3);
        }
        RequestId(uuid::Uuid::from_u128(h as u128))
    }

    // ── request_id pending 매칭 ─────────────────────────────────────────────────────

    // TS: "spawnAgent → SpawnByCwd{request_id} 전송 + Spawned{request_id,agent} resolve"
    // 순수 핵심: 등록된 request_id 의 reply 가 도착하면 그 슬롯이 정확히 매칭돼 꺼내진다.
    #[test]
    fn pending_resolve_on_matching_reply() {
        let mut pending: PendingMap<&str> = PendingMap::new();
        let r = rid("req-1");
        pending.insert(r, "agent-a1");
        let got = take_pending(&mut pending, &r);
        assert_eq!(got, Some("agent-a1"));
        assert!(pending.is_empty(), "매칭 후 슬롯 제거(1회성)");
    }

    // TS: "killAgent → Ack{request_id} 로 void resolve"
    // 순수 핵심: void 응답(payload 없음)도 request_id 로만 매칭된다(여기선 unit 값).
    #[test]
    fn pending_resolve_void_ack() {
        let mut pending: PendingMap<()> = PendingMap::new();
        let r = rid("kill-1");
        pending.insert(r, ());
        assert_eq!(take_pending(&mut pending, &r), Some(()));
        assert!(pending.is_empty());
    }

    // TS: "Error{request_id} 로 reject"
    // 순수 핵심: Error 도 request_id echo 로 같은 슬롯을 꺼낸다(호출자가 reject — 여기선 take 까지).
    #[test]
    fn pending_take_for_error() {
        let mut pending: PendingMap<&str> = PendingMap::new();
        let r = rid("kill-2");
        pending.insert(r, "slot");
        assert_eq!(take_pending(&mut pending, &r), Some("slot"));
        assert!(pending.is_empty());
    }

    // TS: "잘못된 request_id 의 응답은 무시(pending 유지)"
    // 순수 핵심: 매칭 안 되는 request_id 면 None 반환 + 기존 슬롯 보존.
    #[test]
    fn pending_unknown_request_id_ignored() {
        let mut pending: PendingMap<&str> = PendingMap::new();
        let mine = rid("kill-mine");
        pending.insert(mine, "slot");
        let got = take_pending(&mut pending, &rid("nonexistent"));
        assert_eq!(got, None, "없는 id → None");
        assert_eq!(pending.get(&mine), Some(&"slot"), "기존 pending 유지");
    }

    // ── event-routing 케이스(여기서 검증 안 함 — 정직성) ────────────────────────────────
    //   아래 TS 케이스들은 "순수 결정 레이어"가 아니라 InboundMessage variant 라우팅 + 콜백 호출/
    //   unsubscribe 동작이라 protocol_state 단독으론 보호 대상이 없다. HashMap 존재·동작만 재확인하는
    //   헛 단언(vacuous)으로 false confidence 를 주지 않으려 여기서 이식하지 않고, 실제 배선이 도는
    //   연결 task main_loop · eventBus 표면 테스트에서 검증한다:
    //     - broadcast-no-consume (`protocolClient.test.ts:141`): 진짜 의미 = broadcast variant
    //       (AgentListUpdated)가 take_pending 경로를 우회하고 콜백만 호출 = variant 라우팅.
    //     - two-concurrent-ordering (`:161`): 도착순서 무관 resolve = 역순 reply variant 라우팅.
    //     - StatusChanged 콜백 + off()/unsubscribe (`:302`)
    //     - RestoreResult{report} 콜백 (`:315`)
    //     - ProfileListUpdated 콜백 + off()/unsubscribe (`:325`)
    //   (take_pending 의 순수 매칭 자체는 위 pending_resolve_*/pending_unknown_request_id_ignored 가
    //    이미 박는다 — 위 케이스의 라우팅 분기가 추가 검증 대상.)

    // ── epoch 가드(decide_epoch) ───────────────────────────────────────────────────

    // 구독 직후 + SubscribeAck(current_epoch) 적용 헬퍼.
    fn subscribed_with_ack(epoch: u32) -> SubState {
        let mut st = SubState::new();
        apply_subscribe_ack(&mut st, epoch);
        st
    }

    // TS: "epoch 안 맞는 frame → drop(stale 세션)".
    #[test]
    fn epoch_mismatch_dropped() {
        let st = subscribed_with_ack(5);
        assert_eq!(decide_epoch(&st, 4), EpochDecision::DropEpochMismatch);
        assert_eq!(decide_epoch(&st, 5), EpochDecision::Deliver);
    }

    // TS: "SubscribeAck 전 frame(epoch undefined) → epoch 가드 통과(배달)".
    #[test]
    fn frame_before_ack_passes_epoch_guard() {
        let st = SubState::new(); // Ack 전 — epoch=None
        assert_eq!(st.epoch, None);
        // epoch=None 이면 어떤 frame_epoch 라도 Deliver(기준 없음 → 통과).
        assert_eq!(decide_epoch(&st, 99), EpochDecision::Deliver);
        assert_eq!(decide_epoch(&st, 0), EpochDecision::Deliver);
    }

    // TS: "SubscribeAck epoch=1 + output epoch=1 → 배달(전멸 안 됨)" (BLOCKER 1 회귀).
    #[test]
    fn ack_epoch1_output_epoch1_delivered() {
        let st = subscribed_with_ack(1);
        assert_eq!(decide_epoch(&st, 1), EpochDecision::Deliver);
    }

    // TS: "SubscribeAck epoch=1 + output epoch=0(옛 버그 재현) → epoch 가드 drop".
    #[test]
    fn ack_epoch1_output_epoch0_dropped() {
        let st = subscribed_with_ack(1);
        assert_eq!(decide_epoch(&st, 0), EpochDecision::DropEpochMismatch);
    }

    // TS: "epoch 0 output + epoch 0 SubscribeAck 정합(fresh 세션 epoch=0)".
    #[test]
    fn fresh_session_epoch0_delivered() {
        let st = subscribed_with_ack(0);
        assert_eq!(decide_epoch(&st, 0), EpochDecision::Deliver);
    }

    // ── apply_subscribe_ack — bool 반환 ───────────────────────────────────────────

    // 첫 Ack 는 epoch=None → Some(E) 전이. epoch 가 없었으므로 changed=false.
    #[test]
    fn first_ack_returns_false() {
        let mut st = SubState::new();
        let changed = apply_subscribe_ack(&mut st, 10);
        assert!(
            !changed,
            "첫 Ack 는 epoch 변경 없음(None → Some 이지만 reset 불필요)"
        );
        assert_eq!(st.epoch, Some(10));
    }

    // 동일 epoch 재 Ack — changed=false.
    #[test]
    fn same_epoch_ack_returns_false() {
        let mut st = subscribed_with_ack(5);
        let changed = apply_subscribe_ack(&mut st, 5);
        assert!(!changed, "같은 epoch 재확인 — 창 리셋 불필요");
    }

    /// epoch 변경 Ack — changed=true(반환 계약 박제: 이전 epoch 와 다른 값이면 변경으로 본다).
    #[test]
    fn epoch_change_ack_returns_true() {
        let mut st = subscribed_with_ack(10);
        let changed = apply_subscribe_ack(&mut st, 11);
        assert!(changed, "epoch 11→10 변경 — 창 render_seq 리셋 필요");
        assert_eq!(st.epoch, Some(11));
    }

    // ── resubscribe_params/initial_subscribe_params 테스트가 없는 이유(ADR-0046) ────────
    //    src-tauri 는 connected 재전이 시 Subscribe 를 재발행하지 않고 초기 구독 파라미터도 산출하지
    //    않는다 — wire 구독 형성은 프론트 request_replay(뷰 주도 전량 재replay) 단독이라 박을 함수가 없다.

    // ── pending drain ───────────────────────────────────────────────────────────────

    // TS: "connected→reconnecting 전이 시 pending 명령 reject(connection lost)"
    // 순수 핵심: 끊김 시 drain_pending 이 **모든** in-flight 를 꺼내 비운다(호출자가 일괄 reject).
    #[test]
    fn drain_pending_on_disconnect() {
        let mut pending: PendingMap<&str> = PendingMap::new();
        pending.insert(rid("kill-a"), "a");
        pending.insert(rid("kill-b"), "b");
        let drained = drain_pending(&mut pending);
        assert_eq!(drained.len(), 2, "전부 꺼냄(1회성)");
        assert!(pending.is_empty(), "drain 후 비어야 promise leak 없음");
    }

    // ── close ──────────────────────────────────────────────────────────────────────

    // TS: "close() → pending reject + transport.close 호출"
    // 순수 핵심: close 시 모든 pending 을 꺼내 비운다(transport.close 호출은 carrier 배선 = T6).
    #[test]
    fn close_drains_all_pending() {
        let mut pending: PendingMap<&str> = PendingMap::new();
        pending.insert(rid("kill-1"), "p");
        let drained = drain_pending(&mut pending);
        assert_eq!(drained, vec!["p"]);
        assert!(pending.is_empty());
    }

    // ── request_id 추출 + reply outcome 분류 ──────────────────────────────────────────

    // side-effect 명령은 request_id 를 반환하고, request_id 없는 명령(Auth/Resize/Subscribe/
    // Unsubscribe)은 None — send_command 가 None 을 걸러 영구 pending(hang)을 막는 계약의 단위 박제.
    #[test]
    fn command_request_id_extracts_or_none() {
        let r = RequestId::new();
        let spawn = AgentCommand::Spawn {
            profile_id: uuid::Uuid::new_v4(),
            request_id: r,
        };
        assert_eq!(
            command_request_id(&spawn),
            Some(r),
            "Spawn 은 request_id 동봉"
        );

        let kill = AgentCommand::Kill {
            agent_id: uuid::Uuid::new_v4(),
            request_id: r,
        };
        assert_eq!(command_request_id(&kill), Some(r));

        // request_id 없는 명령들 → None(reply 매칭 대상 아님).
        let resize = AgentCommand::Resize {
            agent_id: uuid::Uuid::new_v4(),
            cols: 80,
            rows: 24,
            viewport_id: None,
        };
        assert_eq!(
            command_request_id(&resize),
            None,
            "Resize 는 request_id 없음"
        );
        let sub = AgentCommand::Subscribe {
            agent_id: uuid::Uuid::new_v4(),
            epoch: None,
            after_seq: None,
        };
        assert_eq!(
            command_request_id(&sub),
            None,
            "Subscribe 는 request_id 없음"
        );
        // (옛 Auth 케이스는 지웠다 — 핸드셰이크가 명령 enum 을 떠나 이 함수에 **넣을 수조차 없다**.
        //  ADR-0129 0-4. 단언이 약해진 게 아니라 타입이 그 입력을 없앴다.)
    }

    // 전용 reply variant 는 request_id 를 echo(매칭 대상), broadcast 는 None(매칭 우회 = 편승 제거).
    #[test]
    fn event_reply_request_id_only_for_replies() {
        let r = RequestId::new();
        assert_eq!(
            event_reply_request_id(&AgentEvent::Ack { request_id: r }),
            Some(r),
            "Ack 은 reply"
        );
        assert_eq!(
            event_reply_request_id(&AgentEvent::Error {
                request_id: Some(r),
                message: "x".into()
            }),
            Some(r),
            "Error{{Some}} 은 특정 명령 실패 — 매칭"
        );
        assert_eq!(
            event_reply_request_id(&AgentEvent::Error {
                request_id: None,
                message: "x".into()
            }),
            None,
            "Error{{None}} 은 명령 무관 — 매칭 안 함"
        );
        // broadcast — request_id 없음(매칭 우회).
        assert_eq!(
            event_reply_request_id(&AgentEvent::AgentListUpdated { agents: vec![] }),
            None,
            "AgentListUpdated 는 broadcast — 매칭 우회"
        );
        // ADR-0155 명령↔답장 쌍(ListCommands↔CommandList) 박제는 아래 `list_commands_reply_request_id_pairing`.
        // SubscribeAck 는 agent_id 기반(request_id 없음) — T6a reply 매칭 대상 아님.
        assert_eq!(
            event_reply_request_id(&AgentEvent::SubscribeAck {
                agent_id: uuid::Uuid::new_v4(),
                action: engram_dashboard_protocol::SubscribeAction::Reset,
                current_epoch: 0,
                oldest_seq: 0,
                latest_seq: 0,
                replay_from: 0,
                truncated: false,
                continues_conversation: false,
            }),
            None
        );
    }

    // classify_reply: 데몬이 답한 Error 는 문구와 무관하게 Daemon · 슬롯의 Err 는 Local · 그 밖은 Ok.
    //   ★모르는 코드와 접두 없는 글이 이 시험의 몫이다★ — 문구로 가르는 분류는 아는 코드 둘만으로도 초록이다.
    #[test]
    fn classify_reply_keeps_origin_without_reading_text() {
        let r = RequestId::new();
        for message in [
            "CONFLICT: …",
            "INVALID_ARGUMENT: …",
            "BRAND_NEW_CODE: …",
            "접두 없는 글",
        ] {
            let classified = classify_reply(Ok(AgentEvent::Error {
                request_id: Some(r),
                message: message.into(),
            }));
            match classified {
                Err(failure @ CommandFailure::Daemon { .. }) => {
                    assert_eq!(failure.to_string(), message, "Display = 데몬 문구 원문")
                }
                other => panic!("데몬 Error 는 Daemon 이어야 — {message:?}: {other:?}"),
            }
        }

        let local = crate::daemon_client::connection::SENT_OUTCOME_UNKNOWN;
        match classify_reply(Err(local.to_string())) {
            Err(failure @ CommandFailure::Local(_)) => {
                assert_eq!(failure.to_string(), local, "Display = 로컬 문구 그대로")
            }
            other => panic!("슬롯 Err 는 Local 이어야: {other:?}"),
        }

        match classify_reply(Ok(AgentEvent::Ack { request_id: r })) {
            Ok(AgentEvent::Ack { request_id }) => assert_eq!(request_id, r),
            other => panic!("Ack 은 Ok 그대로여야: {other:?}"),
        }
    }

    // ── 명령 칸 끝자락 · 명령↔답장 쌍(ADR-0272 결정 4) ──────────────────────────────────

    /// protocol 봉투 golden 과 같은 바이트(`messages.rs` 의 `NIL_ENVELOPE`) — 패킷 쪽 golden 은 이 바이트가 원문
    /// 칸을 그대로 건넌다는 것만 재고, 봉투 구조체 ↔ 바이트는 봉투를 푸는 이 끝자락이 잰다.
    const NIL_ENVELOPE: &str = r#"{"name":"tab.create","request_id":"00000000-0000-0000-0000-000000000000","owner":"shell","proto_ver":1,"args":{"window":"main"}}"#;

    fn nil_bus_envelope() -> CommandEnvelope {
        CommandEnvelope {
            name: "tab.create".to_string(),
            request_id: engram_dashboard_command::RequestId(uuid::Uuid::nil()),
            owner: engram_dashboard_command::OwnerToken::new("shell"),
            proto_ver: 1,
            args: serde_json::json!({ "window": "main" }),
        }
    }

    /// protocol 답장 golden 과 같은 바이트(`messages.rs` 의 `NIL_FAILED_REPLY` · `NIL_OK_REPLY`) — 봉투와 같은 법으로
    /// 답장 구조체 ↔ 바이트는 답장을 푸는 이 끝자락이 잰다.
    const NIL_FAILED_REPLY: &str = r#"{"request_id":"00000000-0000-0000-0000-000000000000","outcome":{"Err":{"code":"NOT_FOUND","message":"no view 'x'","retry":"never"}}}"#;
    const NIL_OK_REPLY: &str = r#"{"request_id":"00000000-0000-0000-0000-000000000000","outcome":{"Ok":{"view_id":"v1"}}}"#;

    fn nil_bus_replies() -> [(engram_dashboard_command::CommandReply, &'static str); 2] {
        let nil = engram_dashboard_command::RequestId(uuid::Uuid::nil());
        [
            (
                engram_dashboard_command::CommandReply::err(
                    nil,
                    engram_dashboard_command::CommandError::of(
                        engram_dashboard_command::ErrorCode::NotFound,
                        "no view 'x'",
                    ),
                ),
                NIL_FAILED_REPLY,
            ),
            (
                engram_dashboard_command::CommandReply::ok(
                    nil,
                    serde_json::json!({ "view_id": "v1" }),
                ),
                NIL_OK_REPLY,
            ),
        ]
    }

    fn raw(json: &str) -> Box<RawValue> {
        RawValue::from_string(json.to_string()).expect("유효한 JSON")
    }

    /// ★손으로 적은 TS 봉투 칸(protocol `AgentCommand::Command` · `AgentEvent::CommandRequest` 의
    /// `#[ts(type = …)]`)과 맞서는 golden★ — 원문 칸이라 protocol 은 봉투 모양을 못 보므로, 봉투 구조체가 그
    /// 바이트로 나가고 그 바이트가 같은 구조체로 풀리는지를 여기서 못박는다.
    #[test]
    fn the_envelope_slot_unpacks_to_the_bus_envelope_byte_for_byte() {
        let sealed = serde_json::value::to_raw_value(&nil_bus_envelope()).unwrap();
        assert_eq!(
            sealed.get(),
            NIL_ENVELOPE,
            "봉투 구조체 → 원문 칸 바이트가 golden 과 불일치 — 손으로 적은 TS 칸도 함께 고칠 것"
        );
        assert_eq!(
            envelope_from_wire(&raw(NIL_ENVELOPE)).unwrap(),
            nil_bus_envelope()
        );
    }

    /// ★패킷은 풀렸는데 봉투가 깨진 갈래 = 요청 번호 없음★(ADR-0272 결정 6 · 셸 쪽) — 대기 슬롯을 만들면 그
    /// 번호로 오는 답이 없어 답장 상한까지 매달린다.
    #[test]
    fn a_broken_envelope_has_no_request_id() {
        for broken in [
            r#"{"name":"tab.create"}"#,
            r#"{"name":"x","request_id":"not-a-uuid","owner":"o","proto_ver":1,"args":null}"#,
            "[]",
            "null",
        ] {
            assert!(envelope_from_wire(&raw(broken)).is_err(), "{broken}");
            assert_eq!(
                command_request_id(&AgentCommand::Command {
                    envelope: raw(broken)
                }),
                None,
                "{broken}"
            );
        }
    }

    /// ★웹뷰 길(`forward_daemon_command`)은 패킷을 `serde_json::Value` 에서 푼다★ — 원문 칸도 그 길로 풀려
    /// 번호를 읽어야 답장 대기 갈래를 탄다(serde_json 의 `Value` 역직렬화가 원문 칸을 다시 직렬화해 담는다).
    #[test]
    fn a_command_parsed_from_a_json_value_still_pairs() {
        let sent: AgentCommand = serde_json::from_value(serde_json::json!({
            "Command": { "envelope": serde_json::from_str::<serde_json::Value>(NIL_ENVELOPE).unwrap() }
        }))
        .expect("Value 에서도 원문 칸이 풀린다");
        assert_eq!(
            command_request_id(&sent),
            Some(RequestId(uuid::Uuid::nil()))
        );
    }

    /// ★웹뷰 길에서 다시 싼 봉투 칸 = 구조체 순서 golden 바이트★ — 키 순서를 섞어 보낸 봉투도 `Value` 를 지나며
    /// 키 정렬되므로, 다시 싸지 않으면 셸이 직접 보내는 봉투와 다른 바이트가 나간다.
    #[test]
    fn a_webview_envelope_is_resealed_into_the_golden_bytes() {
        let shuffled = serde_json::json!({
            "Command": { "envelope": {
                "args": { "window": "main" },
                "proto_ver": 1,
                "owner": "shell",
                "request_id": "00000000-0000-0000-0000-000000000000",
                "name": "tab.create",
            } }
        });
        let sent: AgentCommand = serde_json::from_value(shuffled).unwrap();
        match reseal_webview_command_slots(sent).expect("성한 봉투는 다시 싸진다") {
            AgentCommand::Command { envelope } => assert_eq!(envelope.get(), NIL_ENVELOPE),
            other => panic!("Command 그대로여야: {other:?}"),
        }
    }

    /// ★웹뷰 길의 깨진 봉투 = 다시 싸기 실패★ — 부르는 쪽(`forward_to_daemon`)이 그것을 invoke 실패로 돌려
    /// 프론트 pending 을 곧바로 reject 한다. 명령 칸이 없는 명령은 손대지 않는다.
    #[test]
    fn a_broken_webview_envelope_fails_to_reseal() {
        let sent: AgentCommand = serde_json::from_value(serde_json::json!({
            "Command": { "envelope": { "name": "tab.create" } }
        }))
        .expect("원문 칸은 패킷 파싱을 통과한다");
        assert!(reseal_webview_command_slots(sent).is_err());

        let resize = AgentCommand::Resize {
            agent_id: uuid::Uuid::nil(),
            cols: 80,
            rows: 24,
            viewport_id: None,
        };
        assert!(matches!(
            reseal_webview_command_slots(resize),
            Ok(AgentCommand::Resize { cols: 80, .. })
        ));
    }

    /// ★손으로 적은 TS 답장 칸(protocol `AgentCommand::CommandOutcome` · `AgentEvent::CommandReply` 의
    /// `#[ts(type = …)]`)과 맞서는 golden★ — 원문 칸이라 protocol 은 답장 모양을 못 보므로, 답장 구조체가 그
    /// 바이트로 나가고(결말을 보내는 길 = [`reply_to_wire`]) 그 바이트가 같은 구조체로 풀리는지를 여기서 못박는다.
    #[test]
    fn the_reply_slot_unpacks_to_the_bus_reply_byte_for_byte() {
        for (reply, golden) in nil_bus_replies() {
            assert_eq!(
                reply_to_wire(&reply).unwrap().get(),
                golden,
                "답장 구조체 → 원문 칸 바이트가 golden 과 불일치 — 손으로 적은 TS 칸도 함께 고칠 것"
            );
            assert_eq!(reply_from_wire(&raw(golden)).unwrap(), reply);
        }
    }

    /// ★패킷은 풀렸는데 답장이 깨진 갈래 = 요청 번호 없음★(ADR-0272 결정 6 · 셸 쪽) — 깨울 슬롯을 못 찾으니 연결
    /// 루프가 버린다. 원문에 맞는 번호가 적혀 있어도 번호만 골라 읽지 않는다(같은 ADR 「거부한 대안」 — 부분 파싱).
    #[test]
    fn a_broken_reply_has_no_request_id() {
        for broken in [
            r#"{"request_id":"00000000-0000-0000-0000-000000000000"}"#,
            r#"{"request_id":"not-a-uuid","outcome":{"Ok":null}}"#,
            "[]",
            "null",
        ] {
            assert!(reply_from_wire(&raw(broken)).is_err(), "{broken}");
            assert_eq!(
                event_reply_request_id(&AgentEvent::CommandReply { reply: raw(broken) }),
                None,
                "{broken}"
            );
        }
    }

    /// ★TS 칸이 광고하는 관용(오류 세 칸 부재 · `null` · 계약 밖 필드)을 받은 답장도 짝을 찾는다★ — 이 끝자락이
    /// 그런 답장을 깨진 것으로 읽으면 상대가 이미 답했는데도 대기가 답장 상한까지 매달린다(관용 자체는 데몬
    /// `the_outcome_slot_accepts_everything_its_ts_type_advertises` 가 잰다).
    #[test]
    fn a_sparse_reply_still_pairs() {
        let nil = "00000000-0000-0000-0000-000000000000";
        for sparse in [
            format!(r#"{{"request_id":"{nil}","outcome":{{"Err":{{"detail":{{"n":1}}}}}}}}"#),
            format!(
                r#"{{"request_id":"{nil}","outcome":{{"Err":{{"code":null,"message":null,"retry":null}}}}}}"#
            ),
        ] {
            assert_eq!(
                event_reply_request_id(&AgentEvent::CommandReply {
                    reply: raw(&sparse)
                }),
                Some(RequestId(uuid::Uuid::nil())),
                "{sparse}"
            );
        }
    }

    /// ★깨운 답장은 웹뷰로 JSON 값이 되어 올라간다★(`forward_to_daemon` 의 `serde_json::to_value`) — 원문 칸도 그
    /// 길로 값이 되어야 프론트가 받는 모양이 그대로다(serde_json 의 `Value` 직렬화가 원문 칸을 풀어 담는다).
    #[test]
    fn a_reply_event_becomes_the_same_json_value_for_the_webview() {
        let answered = AgentEvent::CommandReply {
            reply: raw(NIL_OK_REPLY),
        };
        assert_eq!(
            serde_json::to_value(&answered).unwrap(),
            serde_json::json!({ "CommandReply": { "reply": {
                "request_id": "00000000-0000-0000-0000-000000000000",
                "outcome": { "Ok": { "view_id": "v1" } },
            } } })
        );
    }

    /// ★웹뷰 길에서 다시 싼 답장 칸 = 구조체 순서 golden 바이트★ — 봉투와 같은 이유다(`Value` 를 지나며 키 정렬).
    #[test]
    fn a_webview_outcome_is_resealed_into_the_golden_bytes() {
        let shuffled = serde_json::json!({
            "CommandOutcome": { "reply": {
                "outcome": { "Ok": { "view_id": "v1" } },
                "request_id": "00000000-0000-0000-0000-000000000000",
            } }
        });
        let sent: AgentCommand = serde_json::from_value(shuffled).unwrap();
        match reseal_webview_command_slots(sent).expect("성한 답장은 다시 싸진다") {
            AgentCommand::CommandOutcome { reply } => assert_eq!(reply.get(), NIL_OK_REPLY),
            other => panic!("CommandOutcome 그대로여야: {other:?}"),
        }
    }

    /// ★웹뷰 답장의 계약 밖 필드는 다시 싸도 살아 남는다★ — TS 칸이 광고하는 모양(세 칸 부재 + 계약 밖
    /// `detail`)을 웹뷰가 보내면 다시 싸기가 받아들이고 `detail` 을 그대로 데몬에 넘긴다. 데몬 쪽 짝 =
    /// `connection_core` 의 `the_outcome_slot_accepts_everything_its_ts_type_advertises`.
    #[test]
    fn a_webview_outcome_keeps_its_out_of_contract_fields_when_resealed() {
        let sent: AgentCommand = serde_json::from_value(serde_json::json!({
            "CommandOutcome": { "reply": {
                "request_id": "00000000-0000-0000-0000-000000000000",
                "outcome": { "Err": { "detail": { "n": 1 } } },
            } }
        }))
        .unwrap();
        match reseal_webview_command_slots(sent).expect("세 칸이 없는 실패 답장도 다시 싸진다")
        {
            AgentCommand::CommandOutcome { reply } => assert!(
                reply.get().contains(r#""detail":{"n":1}"#),
                "계약 밖 필드가 다시 싸기에서 사라졌다: {}",
                reply.get()
            ),
            other => panic!("CommandOutcome 그대로여야: {other:?}"),
        }
    }

    /// ★웹뷰 길의 깨진 답장 = 다시 싸기 실패★ — 흘려보내면 데몬이 번호 없는 거절로 받아 웹뷰는 실패를 모른다.
    /// 부르는 쪽(`forward_to_daemon`)이 그것을 invoke 실패로 돌린다.
    #[test]
    fn a_broken_webview_reply_fails_to_reseal() {
        let sent: AgentCommand = serde_json::from_value(serde_json::json!({
            "CommandOutcome": { "reply": { "request_id": "00000000-0000-0000-0000-000000000000" } }
        }))
        .expect("원문 칸은 패킷 파싱을 통과한다");
        assert!(reseal_webview_command_slots(sent).is_err());
    }

    /// ★중계 다리의 두 variant 는 pending 매칭 밖이다★ — 하나는 들어오는 **요청**이고 하나는 나가는
    /// **답장**이라, 둘 중 어느 쪽이든 `Some` 을 주면 봉투가 삼켜지거나 빈 슬롯이 영구히 남는다.
    #[test]
    fn the_relay_leg_is_outside_pending_correlation() {
        assert_eq!(
            event_reply_request_id(&AgentEvent::CommandRequest {
                envelope: raw(NIL_ENVELOPE),
            }),
            None,
            "들어온 요청을 「내가 기다린 답장」으로 읽으면 그 명령은 실행되지 않고 사라진다"
        );
        assert_eq!(
            command_request_id(&AgentCommand::CommandOutcome {
                reply: raw(NIL_OK_REPLY),
            }),
            None,
            "답장을 보내면서 pending 슬롯을 만들면 깨울 짝이 없다"
        );
    }

    /// ★발신자 다리의 두 variant 는 **한 쌍으로** pending 매칭 안이다★ — 중계 다리(위)의 정반대다.
    ///
    /// 한쪽만 `Some` 이면 왕복이 안 닫힌다: 요청만 `Some` 이면 슬롯을 깨울 짝이 없고, 답장만 `Some` 이면
    /// 깰 슬롯이 없다. 둘 다 셸이 마감시각을 다 쓰거나 연결이 끊길 때까지 매달리는 것으로 끝난다.
    /// 이 테스트가 그 한쪽만 고친 편집을 떨어뜨린다.
    #[test]
    fn the_sender_leg_is_inside_pending_correlation_as_a_pair() {
        let envelope = CommandEnvelope {
            request_id: engram_dashboard_command::RequestId::new(),
            ..nil_bus_envelope()
        };
        let sent = AgentCommand::Command {
            envelope: serde_json::value::to_raw_value(&envelope).unwrap(),
        };
        let answered = AgentEvent::CommandReply {
            reply: reply_to_wire(&engram_dashboard_command::CommandReply::ok(
                envelope.request_id,
                serde_json::json!({ "view_id": "v1" }),
            ))
            .unwrap(),
        };

        let asked =
            command_request_id(&sent).expect("요청이 pending 슬롯을 못 만들면 답장이 와도 못 깬다");
        let woke =
            event_reply_request_id(&answered).expect("답장이 상관 밖이면 그 슬롯을 깨울 짝이 없다");
        assert_eq!(asked, woke, "두 다리가 같은 키를 봐야 왕복이 닫힌다");
        // 홉에서 키를 새로 만들면(`RequestId::new`·`default`) 답장이 이 요청에 못 붙는다.
        assert_eq!(
            asked.0, envelope.request_id.0,
            "봉투가 품은 uuid 그대로여야"
        );
    }

    /// ★명령↔답장 쌍 박제(ADR-0155)★: `AgentCommand::ListCommands` 가 pending 슬롯을 만드는 쪽이고
    /// `AgentEvent::CommandList` 가 그걸 깨우는 쪽이다. 한쪽만 고치면(예: 새 reply variant 를
    /// broadcast 로 잘못 분류) 그 왕복은 연결이 끊길 때까지 안 풀린다.
    #[test]
    fn list_commands_reply_request_id_pairing() {
        let r = RequestId::new();
        let cmd = AgentCommand::ListCommands { request_id: r };
        assert_eq!(
            command_request_id(&cmd),
            Some(r),
            "ListCommands 는 request_id 동봉 — pending 슬롯을 만드는 쪽"
        );
        let reply = AgentEvent::CommandList {
            request_id: r,
            entries: vec![],
        };
        assert_eq!(
            event_reply_request_id(&reply),
            Some(r),
            "CommandList 는 ListCommands 의 전용 reply — 같은 request_id 로 매칭돼야 슬롯이 깨어난다"
        );
    }

    /// ★명령↔답장 두 쌍 박제★ — 한쪽만 `Some` 이면 셸이 그 왕복을 연결이 끊기거나 답장 상한이 찰 때까지 붙든다.
    #[test]
    fn queued_input_commands_and_replies_pair_by_request_id() {
        let r = RequestId::new();
        let agent_id = uuid::Uuid::new_v4();
        let pairs = [
            (
                AgentCommand::ListQueuedInputs {
                    agent_id,
                    request_id: r,
                },
                AgentEvent::QueuedInputs {
                    request_id: r,
                    agent_id,
                    inputs: vec![],
                    as_of_seq: None,
                    epoch: 0,
                    stopped_after_error: false,
                },
            ),
            (
                AgentCommand::CancelQueuedInput {
                    agent_id,
                    input_id: "q1".into(),
                    request_id: r,
                },
                AgentEvent::QueuedInputCancelReply {
                    request_id: r,
                    agent_id,
                    input_id: "q1".into(),
                    outcome: "cancelled".into(),
                },
            ),
        ];
        for (cmd, reply) in pairs {
            assert_eq!(command_request_id(&cmd), Some(r), "{cmd:?}");
            assert_eq!(event_reply_request_id(&reply), Some(r), "{reply:?}");
        }
    }

    /// 구독은 답이 없고(대기표를 걸면 영구 pending) ⟳ 는 기존 `Ack` 와 한 쌍이며, 방송은 상관 밖이다.
    #[test]
    fn usage_wire_request_id_arms() {
        use engram_dashboard_protocol::AgentBackendKind;

        let r = RequestId::new();
        assert_eq!(
            command_request_id(&AgentCommand::UsageSubscribe {
                vendors: vec![AgentBackendKind::Claude],
            }),
            None
        );
        assert_eq!(
            command_request_id(&AgentCommand::RefreshUsageLimits {
                vendor: AgentBackendKind::Claude,
                request_id: r,
            }),
            Some(r)
        );
        assert_eq!(
            event_reply_request_id(&AgentEvent::Ack { request_id: r }),
            Some(r)
        );
        let updated: AgentEvent = serde_json::from_value(serde_json::json!({
            "UsageLimitsUpdated": {
                "snapshot": {
                    "vendor": "codex", "account_key": "default", "five_hour": null, "weekly": null,
                    "model_scoped": [], "plan": null, "in_flight": true,
                    "state": { "kind": "Ready" }, "revision": 1
                },
                "subscribed": []
            }
        }))
        .unwrap();
        assert_eq!(event_reply_request_id(&updated), None);
    }
}
