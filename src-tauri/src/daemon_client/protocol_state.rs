//! 프로토콜 의미론(순수 상태/결정 함수) — epoch 가드 · seq dedup · resubscribe · pending 매칭
//! (S14 모듈① T3, ADR-0037).
//!
//! ## 무엇 / 왜 (load-bearing)
//! 프론트 `src/api/protocolClient.ts` 가 carrier-무관 JS 한 곳에 모았던 전송 의미론을 **Rust 단독
//! 진실원**으로 옮긴 것이다(ADR-0037 D1=A안). 데몬 단일 WS 연결에서 라우팅 전 1회 적용하고, 창 N개로는
//! 깨끗한 청크만 fan-out 한다. ADR-0037 전제: **dedup/epoch 가드는 여기 Rust 가 유일한 진실원** —
//! JS 2차 방어선 없음. 그래서 경계 판정이 한 치 어긋나면 출력이 화면에 전멸하거나 중복된다.
//!
//! ## ★순수성(테스트 격리)★
//! 이 모듈은 소켓·tokio runtime·Tauri 의존이 **0**이다. 모든 함수는 `&mut SubState` / `&mut PendingMap`
//! 를 인자로 받아 결정만 내린다(부수효과는 호출자 = 연결 task 가 수행: 실제 frame 배달·wire send·
//! reply resolve). 그래서 단위 테스트가 런타임 없이 동기로 돈다. `protocolClient.test.ts` 의 케이스
//! 중 **순수 결정에 해당하는 부분만** 옮긴다(event-routing/콜백 배선은 T5/T6 — 아래 tests mod 주석).
//!
//! ## 와이어 타입 정합 (TS number → Rust 분리)
//! TS 는 epoch/seq 가 둘 다 `number` 였지만, protocol crate wire 는 `epoch: u32` · `seq: u64` 다
//! (`codec.rs`·`messages.rs`). 그대로 재사용한다(로컬 재정의 안 함). `request_id` 는 `RequestId(Uuid)`.
//! ★TS `lastDeliveredSeq: -1`(센티넬) 매핑★: u64 로는 -1 을 못 쓰므로 `Option<u64>`(None=아직 아무것도
//! 배달 안 함)로 표현한다 — "seq<=last drop" / "epoch 변경 시 리셋(=None)" 의미를 타입으로 못 박는다.

use std::collections::HashMap;

use engram_dashboard_protocol::{AgentCommand, AgentEvent, RequestId};

// ── request_id 추출(T6a — request/reply 상관) ─────────────────────────────────────────
/// 명령에 실린 request_id 를 꺼낸다. side-effect 명령(Spawn/Kill/…)은 모두 request_id 를 갖지만,
/// 일부(Auth/Subscribe/Unsubscribe/Resize)는 request_id 가 없다(데몬이 reply 를 안 보냄) → `None`.
///
/// ★T6a 계약★: `send_command` 은 reply 를 기대하므로 request_id 가 있는 명령에만 쓴다. None 인 명령을
/// 넣으면 매칭할 키가 없어 영구 pending(hang) 이 되므로, 호출자(send_command)가 None 을 거른다.
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
        | AgentCommand::GetSnapshot { request_id, .. } => Some(*request_id),
        // request_id 없는 명령 — reply 매칭 대상 아님(데몬이 전용 reply 를 안 echo).
        AgentCommand::Auth { .. }
        | AgentCommand::Resize { .. }
        | AgentCommand::Subscribe { .. }
        | AgentCommand::Unsubscribe { .. } => None,
    }
}

/// reply 이벤트에 실린 request_id 를 꺼낸다(매칭용). 전용 reply variant(Ack/Spawned/Created/
/// SubscribeAck-는 request_id 없음/AgentList/ProfileList/Snapshot/Error)만 request_id 를 echo 한다 —
/// broadcast(AgentListUpdated/StatusChanged/…)는 `None` 이라 pending 매칭을 우회한다(편승 매칭 제거).
///
/// ★Error 분기★: `Error{request_id: Some(_)}` = 특정 명령 실패(매칭해 reject), `Error{request_id: None}`
/// = 명령 무관 오류(broadcast 성격, 매칭 안 함). SubscribeAck 는 request_id 가 없어(agent_id 기반) 여기
/// None — T6a 의 send_command 대상이 아니다(Subscribe 는 request_id 없는 명령). T6b 가 agent_id 로 처리.
pub fn event_reply_request_id(ev: &AgentEvent) -> Option<RequestId> {
    match ev {
        AgentEvent::Ack { request_id }
        | AgentEvent::AgentList { request_id, .. }
        | AgentEvent::ProfileList { request_id, .. }
        | AgentEvent::Snapshot { request_id, .. }
        | AgentEvent::Created { request_id, .. }
        | AgentEvent::Spawned { request_id, .. } => Some(*request_id),
        AgentEvent::Error { request_id, .. } => *request_id,
        // request_id 없는 이벤트(broadcast 또는 agent_id 기반) — pending 매칭 대상 아님.
        AgentEvent::Hello { .. }
        | AgentEvent::SubscribeAck { .. }
        | AgentEvent::Output { .. }
        | AgentEvent::ReplayComplete { .. }
        | AgentEvent::StatusChanged { .. }
        | AgentEvent::AgentListUpdated { .. }
        | AgentEvent::RestoreResult { .. }
        | AgentEvent::InputLeaseChanged { .. }
        | AgentEvent::ProfileListUpdated { .. } => None,
    }
}

/// reply 이벤트가 성공(Ok)인지 실패(Err)인지 가른다(T6a — oneshot resolve). `Error{message}` 만
/// Err(message), 나머지 전용 reply 는 Ok(event). 호출자가 take_pending 으로 꺼낸 oneshot 에 이 결과를
/// 넣는다.
pub fn reply_outcome(ev: AgentEvent) -> Result<AgentEvent, String> {
    match ev {
        AgentEvent::Error { message, .. } => Err(message),
        other => Ok(other),
    }
}

/// 에이전트별 출력 구독 상태(JS `protocolClient.ts` `SubState` 승격).
///
/// epoch 가드 + high-water dedup 의 per-agent 진실원. 연결 task 가 agent_id → SubState 맵으로 들고,
/// SubscribeAck/output frame 마다 아래 결정 함수에 `&mut` 로 넘긴다.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SubState {
    /// 마지막 `SubscribeAck.current_epoch`. output frame epoch 매칭용(불일치 frame 폐기) +
    /// 재연결 resubscribe wire epoch. `None` = 아직 Ack 못 받음(첫 구독 직후).
    pub epoch: Option<u32>,
    /// onChunk 로 **실제 배달한** 최고 seq(high-water). `None` = 아무것도 배달 안 함(TS 의 -1).
    ///
    /// dedup 기준이자 재연결 after_seq. `replay_from` 에 의존하지 않는다(replay_from 은 "데몬이 보내는
    /// 첫 seq"이지 "마지막으로 본 seq"가 아니라 off-by-one 유발 — TS 버그 B).
    pub last_delivered_seq: Option<u64>,
}

impl SubState {
    /// 신규 구독 직후 초기값(JS `subscribeOutput`: epoch=undefined, lastDeliveredSeq=-1).
    /// 같은 agent_id 재구독 시 이걸로 덮는다(컴포넌트가 epoch 바뀌면 재구독).
    pub fn new() -> Self {
        Self {
            epoch: None,
            last_delivered_seq: None,
        }
    }
}

/// output frame 배달 판정 결과(JS `handleOutput` 의 분기를 명시 enum 으로).
///
/// 호출자(연결 task)는 `Deliver` 일 때만 `last_delivered_seq` 가 이미 갱신된 SubState 를 바탕으로
/// 구독자/Router 에 청크를 배달한다. `Drop*` 이면 아무것도 하지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputDecision {
    /// 가드 통과 — 구독자에 배달. `seq` 동봉(호출자가 OutputChunk{seq,bytes} 구성).
    Deliver { seq: u64 },
    /// epoch 불일치 — 옛 세션 잔여 frame. 화면 오염 방지로 버린다.
    DropEpochMismatch,
    /// seq<=high-water — 이미 배달한 중복(재연결 경계 등). 버린다.
    DropDuplicate,
}

/// resubscribe 시 보낼 Subscribe 파라미터(JS `resubscribeAll` 산출 + `subscribeOutput` 첫 구독).
///
/// wire `AgentCommand::Subscribe { agent_id, epoch: Option<u32>, after_seq: Option<u64> }` 로 그대로
/// 매핑된다(여기선 agent_id 를 빼고 가드 산출분만 — 호출자가 agent_id 를 붙인다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubscribeParams {
    /// `Some(current)` = Resume(tail-only) 의도. `None` = 전체 replay(FromOldest).
    pub epoch: Option<u32>,
    /// `Some(last)` = 그 seq 이후만. `None` = 처음부터.
    pub after_seq: Option<u64>,
}

/// pending request_id → reply 콜백 슬롯의 키→값 타입. 값(T)은 호출자가 정한다 —
/// 운영에선 `oneshot::Sender<reply>`, 테스트에선 결과를 적재하는 mock. 이 모듈은 매칭 로직만 소유한다.
pub type PendingMap<T> = HashMap<RequestId, T>;

// ── output frame 배달 판정(JS handleOutput 승격) ─────────────────────────────────────
/// epoch 가드 + high-water dedup 후 배달 여부를 결정한다. `Deliver` 면 `st.last_delivered_seq` 를
/// frame seq 로 **갱신한 뒤** 반환한다(호출자는 갱신된 상태 위에서 배달만 하면 됨).
///
/// ★가드 순서(JS 와 동일, load-bearing)★: ① epoch 불일치 → drop(옛 세션 잔여가 화면 오염) ②
/// seq<=high-water → drop(중복). epoch 가 `None`(첫 Ack 전)이면 epoch 가드는 통과시킨다 — Ack 전
/// 도착 frame 도 배달해야 초반 출력이 안 사라진다(JS `st.epoch !== undefined` 가드와 동형).
pub fn decide_output(st: &mut SubState, frame_epoch: u32, frame_seq: u64) -> OutputDecision {
    // epoch 불일치 frame 은 옛 세션 잔여 — 버린다(SubscribeAck.current_epoch 기준). epoch=None(첫 Ack
    // 전)이면 비교를 건너뛰고 통과(아직 기준 epoch 모름 → 일단 배달).
    if let Some(cur) = st.epoch {
        if frame_epoch != cur {
            return OutputDecision::DropEpochMismatch;
        }
    }
    // dedup — 클라가 실제 배달한 high-water 기준. 재연결 경계 중복 방어. InProc(순서 보존)에선 항상
    // seq>high-water 라 무해 통과(no-op 수렴). last=None(아무것도 배달 안 함)이면 어떤 seq 든 통과.
    if let Some(last) = st.last_delivered_seq {
        if frame_seq <= last {
            return OutputDecision::DropDuplicate;
        }
    }
    st.last_delivered_seq = Some(frame_seq);
    OutputDecision::Deliver { seq: frame_seq }
}

// ── SubscribeAck 처리(JS handleEvent 의 SubscribeAck 분기 승격) ───────────────────────
/// SubscribeAck 수신 시 epoch 갱신 + 필요 시 high-water 리셋.
///
/// ★버그 B 가드★: `replay_from` 으로 dedup 기준(last_delivered_seq)을 **건드리지 않는다**. replay_from
/// 은 "데몬이 보내는 첫 seq"(resume 시 after_seq+1)이지 "마지막으로 본 seq"가 아니다 — 그걸 dedup
/// 기준으로 쓰면 첫 정상 프레임(seq==replay_from)을 버린다. 그래서 이 함수는 replay_from 을 인자로도
/// 받지 않는다(호출자가 truncated 경고만 별도 처리).
///
/// ★epoch 변경 리셋(ADR-0007 epoch 재구독 대응)★: epoch 이 바뀌면(데몬 재기동·재시작) 새 스트림 →
/// high-water 리셋(None). 안 하면 새 세션의 낮은 seq 가 옛 high-water 에 막혀 출력이 전멸한다.
/// 첫 Ack(epoch=None)은 리셋 불필요(이미 None).
pub fn apply_subscribe_ack(st: &mut SubState, current_epoch: u32) {
    if let Some(prev) = st.epoch {
        if current_epoch != prev {
            // 새 세션 스트림 — high-water 리셋. 안 하면 새 낮은 seq 가 전부 dedup drop 돼 출력 전멸.
            st.last_delivered_seq = None;
        }
    }
    st.epoch = Some(current_epoch);
}

// ── resubscribe 파라미터 산출(JS resubscribeAll 승격) ────────────────────────────────
/// connected 재전이 시 한 agent 의 Subscribe 파라미터 산출.
///
/// ★버그 A 가드★: epoch=null 을 보내면 안 된다. 데몬은 `requested_epoch==Some(current)` 만 일치로
/// 보고 None 은 불일치 취급 → FromOldest 전체 replay(이미 본 프레임 중복). 그래서 **마지막으로 알려진
/// epoch(st.epoch)을 그대로** 보내 데몬이 Resume(tail-only) 하게 한다. after_seq=last_delivered_seq →
/// 데몬이 seq>last 만 송신 → 클라 가드(decide_output 의 DropDuplicate)와 정합.
///
/// epoch 이 `None`(첫 Ack 전에 끊겼다 재연결)이면 epoch=None + after_seq=None → 전체 replay(중복).
/// JS 가 보존한 분기 — Ack 도 못 받았으면 기준이 없어 처음부터 받는 수밖에 없다(중복은 dedup 이 거른다).
pub fn resubscribe_params(st: &SubState) -> SubscribeParams {
    SubscribeParams {
        epoch: st.epoch,
        // JS: `after_seq: st.lastDeliveredSeq >= 0 ? st.lastDeliveredSeq : null`. None = 아무것도 배달
        //   안 함 → 처음부터. Some(last) = 그 이후만(Resume tail-only).
        after_seq: st.last_delivered_seq,
    }
}

/// 첫 구독 파라미터(JS `subscribeOutput`: 둘 다 null = FromOldest, 전부 받음).
pub fn initial_subscribe_params() -> SubscribeParams {
    SubscribeParams {
        epoch: None,
        after_seq: None,
    }
}

// ── pending request_id 매칭(JS resolvePending/rejectPending + connected→down reject 승격) ──
/// request_id 에 대응하는 pending 슬롯을 꺼낸다(매칭 성공 시 맵에서 제거 — 1회성). `None` 이면 무시
/// (JS `resolvePending` 의 "없으면 no-op"). 호출자가 반환된 슬롯을 resolve/reject 한다.
///
/// ★편승 매칭 제거(protocol v2)★: 조회도 전용 reply variant(AgentList/ProfileList/Snapshot)가
/// request_id 를 echo 하므로 broadcast(AgentListUpdated 등)가 조회 응답에 편승하지 않는다 — 호출자가
/// broadcast variant 는 이 함수를 안 거치고 콜백만 호출하면 된다.
pub fn take_pending<T>(pending: &mut PendingMap<T>, request_id: &RequestId) -> Option<T> {
    pending.remove(request_id)
}

/// connected→비connected 전이 시 **모든** pending 을 꺼내 비운다(JS handleClose 의 일괄 reject).
/// 호출자가 각 슬롯을 "connection lost" 로 reject 한다. spawn/kill 등 1회성이라 자동 재전송은 중복
/// 부작용 위험 — 호출자가 catch 후 재시도가 단순·안전(JS 주석 보존). 반환 후 맵은 빈다.
///
/// ★1회성★: 한 번 끊기면 모든 in-flight 가 동시에 죽는다 — 부분 reject 없음(전부 또는 없음).
pub fn drain_pending<T>(pending: &mut PendingMap<T>) -> Vec<T> {
    pending.drain().map(|(_k, v)| v).collect()
}

#[cfg(test)]
mod tests {
    //! `src/api/protocolClient.test.ts` 케이스 중 **순수 상태/결정 함수**에 해당하는 부분만 옮긴다
    //! (대응 TS 케이스명을 각 테스트에 주석으로 단다). 1:1 매핑이 아니다 — TS 한 케이스가 라우팅+결정을
    //! 섞으면 결정 부분만 이식한다.
    //!
    //! ★검증 범위(순수 N=20)★: pending 매칭 4 · seq dedup/epoch 가드 6 · resubscribe/initial 5 ·
    //!   InProc no-op + BLOCKER1 회귀 4 · close drain 1.
    //!
    //! ★T5/T6 로 미룬 event-routing(M=5)★: 아래는 순수 결정이 아니라 InboundMessage variant 라우팅 +
    //!   콜백 호출/unsubscribe 라 protocol_state 단독으론 보호 대상이 없다 → 실제 배선이 도는 T5/T6
    //!   (연결 task main_loop · eventBus 표면)에서 검증한다(여기서 헛 단언으로 이식 X):
    //!     broadcast-no-consume(`:141`) · two-concurrent-ordering(`:161`) · StatusChanged+off(`:302`) ·
    //!     RestoreResult(`:315`) · ProfileListUpdated+off(`:325`).
    //!   (그 외 transport.close/connect/disconnect 위임(`:399~`,`:411~`)도 carrier 배선이라 T6.)

    use super::*;

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

    /// TS: "spawnAgent → SpawnByCwd{request_id} 전송 + Spawned{request_id,agent} resolve"
    /// 순수 핵심: 등록된 request_id 의 reply 가 도착하면 그 슬롯이 정확히 매칭돼 꺼내진다.
    #[test]
    fn pending_resolve_on_matching_reply() {
        let mut pending: PendingMap<&str> = PendingMap::new();
        let r = rid("req-1");
        pending.insert(r, "agent-a1");
        let got = take_pending(&mut pending, &r);
        assert_eq!(got, Some("agent-a1"));
        assert!(pending.is_empty(), "매칭 후 슬롯 제거(1회성)");
    }

    /// TS: "killAgent → Ack{request_id} 로 void resolve"
    /// 순수 핵심: void 응답(payload 없음)도 request_id 로만 매칭된다(여기선 unit 값).
    #[test]
    fn pending_resolve_void_ack() {
        let mut pending: PendingMap<()> = PendingMap::new();
        let r = rid("kill-1");
        pending.insert(r, ());
        assert_eq!(take_pending(&mut pending, &r), Some(()));
        assert!(pending.is_empty());
    }

    /// TS: "Error{request_id} 로 reject"
    /// 순수 핵심: Error 도 request_id echo 로 같은 슬롯을 꺼낸다(호출자가 reject — 여기선 take 까지).
    #[test]
    fn pending_take_for_error() {
        let mut pending: PendingMap<&str> = PendingMap::new();
        let r = rid("kill-2");
        pending.insert(r, "slot");
        assert_eq!(take_pending(&mut pending, &r), Some("slot"));
        assert!(pending.is_empty());
    }

    /// TS: "잘못된 request_id 의 응답은 무시(pending 유지)"
    /// 순수 핵심: 매칭 안 되는 request_id 면 None 반환 + 기존 슬롯 보존.
    #[test]
    fn pending_unknown_request_id_ignored() {
        let mut pending: PendingMap<&str> = PendingMap::new();
        let mine = rid("kill-mine");
        pending.insert(mine, "slot");
        let got = take_pending(&mut pending, &rid("nonexistent"));
        assert_eq!(got, None, "없는 id → None");
        assert_eq!(pending.get(&mine), Some(&"slot"), "기존 pending 유지");
    }

    // ── T5/T6 로 미룬 event-routing 케이스(여기서 검증 안 함 — 정직성) ──────────────────
    //   아래 TS 케이스들은 "순수 결정 레이어"가 아니라 InboundMessage variant 라우팅 + 콜백 호출/
    //   unsubscribe 동작이라 protocol_state 단독으론 보호 대상이 없다. HashMap 존재·동작만 재확인하는
    //   헛 단언(vacuous)으로 false confidence 를 주지 않으려 여기서 이식하지 않고, 실제 배선이 도는
    //   T5/T6(연결 task main_loop · eventBus 표면) 테스트에서 검증한다:
    //     - broadcast-no-consume (`protocolClient.test.ts:141`): 진짜 의미 = broadcast variant
    //       (AgentListUpdated)가 take_pending 경로를 우회하고 콜백만 호출 = variant 라우팅(T5/T6).
    //     - two-concurrent-ordering (`:161`): 도착순서 무관 resolve = 역순 reply variant 라우팅(T5/T6).
    //     - StatusChanged 콜백 + off()/unsubscribe (`:302`)
    //     - RestoreResult{report} 콜백 (`:315`)
    //     - ProfileListUpdated 콜백 + off()/unsubscribe (`:325`)
    //   (take_pending 의 순수 매칭 자체는 위 pending_resolve_*/pending_unknown_request_id_ignored 가
    //    이미 박는다 — 위 케이스의 라우팅 분기가 추가 검증 대상.)

    // ── seq dedup / epoch 가드(R2) ──────────────────────────────────────────────────

    /// 구독 직후 + SubscribeAck(current_epoch) 적용 헬퍼.
    fn subscribed_with_ack(epoch: u32) -> SubState {
        let mut st = SubState::new();
        apply_subscribe_ack(&mut st, epoch);
        st
    }

    /// 배달 판정 후 Deliver 면 seq 를 누적하는 헬퍼(received 배열 = TS).
    fn feed(st: &mut SubState, received: &mut Vec<u64>, epoch: u32, seq: u64) {
        if let OutputDecision::Deliver { seq } = decide_output(st, epoch, seq) {
            received.push(seq);
        }
    }

    /// TS: "같은 seq 재수신 → drop(high-water 기준)"
    #[test]
    fn dedup_same_seq_dropped() {
        let mut st = subscribed_with_ack(1);
        let mut got = vec![];
        feed(&mut st, &mut got, 1, 0);
        feed(&mut st, &mut got, 1, 0); // 중복
        feed(&mut st, &mut got, 1, 1);
        assert_eq!(got, vec![0, 1]);
    }

    /// TS: "seq <= high-water drop, replay_from 은 dedup 기준 아님(버그 B 회귀)"
    /// apply_subscribe_ack 은 replay_from 을 인자로조차 안 받는다 → high-water 는 여전히 None(초기).
    #[test]
    fn dedup_replay_from_is_not_dedup_basis() {
        let mut st = SubState::new();
        // replay_from=5 였더라도 dedup 기준(high-water)은 None — 첫 frame seq=0 이 버려지면 안 됨.
        apply_subscribe_ack(&mut st, 1);
        assert_eq!(
            st.last_delivered_seq, None,
            "replay_from 으로 high-water 안 건드림"
        );
        let mut got = vec![];
        feed(&mut st, &mut got, 1, 0);
        feed(&mut st, &mut got, 1, 1);
        assert_eq!(got, vec![0, 1]);
    }

    /// seq=0 센티넬 경계(TS `lastDeliveredSeq: -1` → Rust `Option<u64> None` 매핑 등가성) 직접 박제.
    /// feed 헬퍼는 received 배열만 보는 간접 커버라, 여기선 decide_output 반환 enum + last_delivered_seq
    /// 상태 변화를 정면 단언한다. ★핵심★: 미배달(None) 상태에서 seq 0 이 정상 배달돼야 하고(TS 의 -1<0
    /// 통과), 이후 같은 0 은 high-water(=Some(0))에 막혀 중복 drop 돼야 한다 — u64 가 0 을 "미배달
    /// 센티넬"로 오인하면 첫 출력이 전멸하거나(=None 을 못 구분) 0 중복을 못 막는다.
    #[test]
    fn seq_zero_sentinel_boundary_direct() {
        let mut st = subscribed_with_ack(1);
        assert_eq!(st.last_delivered_seq, None, "Ack 직후 미배달(=TS -1)");
        // None 상태에서 seq 0 → Deliver{0} + high-water 가 Some(0) 으로 전진(None != Some(0)).
        assert_eq!(
            decide_output(&mut st, 1, 0),
            OutputDecision::Deliver { seq: 0 }
        );
        assert_eq!(
            st.last_delivered_seq,
            Some(0),
            "0 배달 후 high-water=Some(0)"
        );
        // 같은 seq 0 재도착 → DropDuplicate(0 <= 0) + 상태 불변(0 이 센티넬로 오인되면 안 됨).
        assert_eq!(decide_output(&mut st, 1, 0), OutputDecision::DropDuplicate);
        assert_eq!(
            st.last_delivered_seq,
            Some(0),
            "중복 drop — high-water 불변"
        );
        // epoch 불일치도 enum 직접 단언(상태 불변 확인).
        assert_eq!(
            decide_output(&mut st, 2, 1),
            OutputDecision::DropEpochMismatch
        );
        assert_eq!(
            st.last_delivered_seq,
            Some(0),
            "epoch drop — high-water 불변"
        );
    }

    /// TS: "epoch 안 맞는 frame → drop(stale 세션)"
    #[test]
    fn epoch_mismatch_dropped() {
        let mut st = subscribed_with_ack(5);
        assert_eq!(
            decide_output(&mut st, 4, 0),
            OutputDecision::DropEpochMismatch
        );
        assert_eq!(
            decide_output(&mut st, 5, 0),
            OutputDecision::Deliver { seq: 0 }
        );
    }

    /// TS: "SubscribeAck.current_epoch 변경 → high-water 리셋 → 새 스트림 낮은 seq 배달(R3)"
    #[test]
    fn epoch_change_resets_high_water() {
        let mut st = subscribed_with_ack(10);
        let mut got = vec![];
        feed(&mut st, &mut got, 10, 0);
        feed(&mut st, &mut got, 10, 1);
        feed(&mut st, &mut got, 10, 2);
        assert_eq!(got, vec![0, 1, 2]);
        // epoch 11 → 리셋. 새 스트림 seq 0 이 다시 배달돼야.
        apply_subscribe_ack(&mut st, 11);
        assert_eq!(st.last_delivered_seq, None, "epoch 변경 시 high-water 리셋");
        feed(&mut st, &mut got, 11, 0);
        feed(&mut st, &mut got, 11, 1);
        assert_eq!(got, vec![0, 1, 2, 0, 1]);
    }

    /// TS: "SubscribeAck 전 frame(epoch undefined) → epoch 가드 통과(배달)"
    #[test]
    fn frame_before_ack_passes_epoch_guard() {
        let mut st = SubState::new(); // Ack 전 — epoch=None
        assert_eq!(st.epoch, None);
        let mut got = vec![];
        feed(&mut st, &mut got, 99, 0); // 임의 epoch 라도 통과
        assert_eq!(got, vec![0]);
    }

    // ── resubscribe resume(R3) — connected 재전이 ──────────────────────────────────

    /// TS: "재연결(reconnecting→connected) 시 알려진 epoch + after_seq=마지막배달seq 로 resubscribe"
    /// 순수 핵심: resubscribe_params 가 epoch=Some(E)(null 아님 — 버그 A) + after_seq=Some(2)(버그 B).
    #[test]
    fn resubscribe_uses_known_epoch_and_last_seq() {
        let mut st = subscribed_with_ack(5);
        let mut got = vec![];
        feed(&mut st, &mut got, 5, 0);
        feed(&mut st, &mut got, 5, 1);
        feed(&mut st, &mut got, 5, 2);
        assert_eq!(got, vec![0, 1, 2]);

        // ★핵심★: resubscribe 가 epoch=5(null 아님) + after_seq=2(마지막 배달 seq).
        let params = resubscribe_params(&st);
        assert_eq!(params.epoch, Some(5), "버그 A: None 이면 FromOldest 중복");
        assert_eq!(
            params.after_seq,
            Some(2),
            "버그 B: None/replay_from 이면 off-by-one"
        );

        // 데몬 Resume → seq 3. 무손실·무중복(SubscribeAck epoch 동일 → 리셋 안 함).
        apply_subscribe_ack(&mut st, 5);
        feed(&mut st, &mut got, 5, 3);
        assert_eq!(got, vec![0, 1, 2, 3]);
    }

    /// TS: "재연결 후 데몬이 이미 본 seq(0,1,2) 재전송해도 dedup"
    #[test]
    fn resubscribe_then_redelivered_seqs_deduped() {
        let mut st = subscribed_with_ack(2);
        let mut got = vec![];
        feed(&mut st, &mut got, 2, 0);
        feed(&mut st, &mut got, 2, 1);
        feed(&mut st, &mut got, 2, 2);
        // 재연결 후 같은 epoch SubscribeAck(리셋 안 함) → 데몬이 0,1,2 재전송 → dedup, 3 만 새로.
        apply_subscribe_ack(&mut st, 2);
        feed(&mut st, &mut got, 2, 0);
        feed(&mut st, &mut got, 2, 1);
        feed(&mut st, &mut got, 2, 2);
        feed(&mut st, &mut got, 2, 3);
        assert_eq!(got, vec![0, 1, 2, 3]);
    }

    /// TS: "connected→reconnecting 전이 시 pending 명령 reject(connection lost)"
    /// 순수 핵심: 끊김 시 drain_pending 이 **모든** in-flight 를 꺼내 비운다(호출자가 일괄 reject).
    #[test]
    fn drain_pending_on_disconnect() {
        let mut pending: PendingMap<&str> = PendingMap::new();
        pending.insert(rid("kill-a"), "a");
        pending.insert(rid("kill-b"), "b");
        let drained = drain_pending(&mut pending);
        assert_eq!(drained.len(), 2, "전부 꺼냄(1회성)");
        assert!(pending.is_empty(), "drain 후 비어야 promise leak 없음");
    }

    // ── resubscribe — Ack 전 끊김(JS resubscribeAll 의 epoch=null 분기 보존) ─────────

    /// JS `resubscribeAll`: epoch=undefined 면 epoch=null + after_seq=null(전체 replay, 중복은 dedup).
    /// (TS 21케이스엔 단독 테스트 없으나 resubscribeAll 분기 보존 — 명시 박제.)
    #[test]
    fn resubscribe_before_ack_sends_full_replay() {
        let st = SubState::new(); // Ack 전 — epoch=None, last=None
        let params = resubscribe_params(&st);
        assert_eq!(params.epoch, None, "Ack 전엔 기준 없음 → FromOldest");
        assert_eq!(params.after_seq, None);
    }

    /// JS `subscribeOutput` 첫 구독: 둘 다 null(FromOldest, 전부 받음).
    #[test]
    fn initial_subscribe_is_from_oldest() {
        let params = initial_subscribe_params();
        assert_eq!(params.epoch, None);
        assert_eq!(params.after_seq, None);
    }

    // ── InProc no-op 수렴(항상 connected·순서보존) ─────────────────────────────────

    /// TS: "연결 전이 없이도 정상 출력 배달" — 순서 보존 frame 은 dedup 이 절대 막지 않는다(전부 배달).
    #[test]
    fn inproc_ordered_frames_all_delivered() {
        let mut st = subscribed_with_ack(0);
        let mut got = vec![];
        for s in 0..5u64 {
            feed(&mut st, &mut got, 0, s);
        }
        assert_eq!(got, vec![0, 1, 2, 3, 4]);
    }

    /// TS: "SubscribeAck epoch=1 + output epoch=1 → 배달(전멸 안 됨)" (BLOCKER 1 회귀)
    #[test]
    fn ack_epoch1_output_epoch1_delivered() {
        let mut st = subscribed_with_ack(1);
        let mut got = vec![];
        feed(&mut st, &mut got, 1, 0);
        feed(&mut st, &mut got, 1, 1);
        assert_eq!(got, vec![0, 1]);
    }

    /// TS: "SubscribeAck epoch=1 + output epoch=0(옛 버그 재현) → epoch 가드가 전부 drop"
    #[test]
    fn ack_epoch1_output_epoch0_all_dropped() {
        let mut st = subscribed_with_ack(1);
        let mut got = vec![];
        feed(&mut st, &mut got, 0, 0); // 0 != 1 → epoch 가드 drop
        feed(&mut st, &mut got, 0, 1);
        assert_eq!(
            got,
            Vec::<u64>::new(),
            "carrier 가 epoch 0 으로 버리면 화면 0건(버그 메커니즘)"
        );
    }

    /// TS: "epoch 0 output + epoch 0 SubscribeAck 정합(fresh 세션 epoch=0)"
    #[test]
    fn fresh_session_epoch0_delivered() {
        let mut st = subscribed_with_ack(0);
        let mut got = vec![];
        feed(&mut st, &mut got, 0, 0);
        feed(&mut st, &mut got, 0, 1);
        assert_eq!(got, vec![0, 1]);
    }

    // ── close ──────────────────────────────────────────────────────────────────────

    /// TS: "close() → pending reject + transport.close 호출"
    /// 순수 핵심: close 시 모든 pending 을 꺼내 비운다(transport.close 호출은 carrier 배선 = T6).
    #[test]
    fn close_drains_all_pending() {
        let mut pending: PendingMap<&str> = PendingMap::new();
        pending.insert(rid("kill-1"), "p");
        let drained = drain_pending(&mut pending);
        assert_eq!(drained, vec!["p"]);
        assert!(pending.is_empty());
    }

    // ── T6a: request_id 추출 + reply outcome 분류 ─────────────────────────────────────

    /// side-effect 명령은 request_id 를 반환하고, request_id 없는 명령(Auth/Resize/Subscribe/
    /// Unsubscribe)은 None — send_command 가 None 을 걸러 영구 pending(hang)을 막는 계약의 단위 박제.
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
        let auth = AgentCommand::Auth {
            token: "x".into(),
            protocol_version: 1,
        };
        assert_eq!(command_request_id(&auth), None);
    }

    /// 전용 reply variant 는 request_id 를 echo(매칭 대상), broadcast 는 None(매칭 우회 = 편승 제거).
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
            }),
            None
        );
    }

    /// reply_outcome: Error 만 Err(message), 나머지 전용 reply 는 Ok(event).
    #[test]
    fn reply_outcome_splits_ok_and_err() {
        let r = RequestId::new();
        // Ack → Ok.
        match reply_outcome(AgentEvent::Ack { request_id: r }) {
            Ok(AgentEvent::Ack { .. }) => {}
            other => panic!("Ack 은 Ok 여야: {other:?}"),
        }
        // Error → Err(message).
        match reply_outcome(AgentEvent::Error {
            request_id: Some(r),
            message: "boom".into(),
        }) {
            Err(m) => assert_eq!(m, "boom"),
            other => panic!("Error 는 Err(message) 여야: {other:?}"),
        }
    }
}
