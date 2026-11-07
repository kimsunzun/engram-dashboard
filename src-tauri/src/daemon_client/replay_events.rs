//! The daemon's replay events applied to the single-flight replay state (ADR-0046 M1) — no socket, no Tauri.
//!
//! The connection driver (`connection.rs`) calls [`apply_replay_event`] for every daemon event and writes the
//! `Subscribe` it returns; the deadline constants pace its sweep. Kept apart from the driver so the integration
//! harness (`tests/daemon_client_replay.rs`) drives these rules without a connection.
// ADR-0046

use std::collections::HashMap;
use std::time::{Duration, Instant};

use engram_dashboard_protocol::{AgentCommand, AgentEvent, AgentId};

use super::protocol_state::{self, SubState};
use super::replay_flight::{self, RefusalOutcome, ReplayFlightSet, Resolution};
use crate::output_router::{OutputRouter, WindowLabel};

// ★replay 진행 기반 deadline(ADR-0046)★. single-flight in-flight 가 이 시간 동안 진행(frame/Ack) 0 이면
// 실패 마커로 종결한다(agent 소멸·subscribe 실패로 Ack/Complete 자체가 안 오는 경로). healthy-slow replay
// (디버그 빌드·경합)는 frame/Ack 마다 리셋돼 절대 트립 안 된다(진행 기반). 운영 10s급.
pub(super) const REPLAY_DEADLINE: Duration = Duration::from_secs(10);

// The deadline sweep period — the granularity at which expired in-flight replays are swept (the driver arms it only
// while a replay waits).
pub(super) const REPLAY_DEADLINE_TICK: Duration = Duration::from_secs(1);

/// [`apply_replay_event`] 의 결말.
#[derive(Debug)]
pub enum ReplayFollowUp {
    /// replay 계열 이벤트가 아니다 — 호출자가 자기 갈래(인바운드 명령 / broadcast)로 넘긴다.
    NotHandled,
    /// 처리했다. `Some(cmd)` = 지금 wire 로 내야 할 **다음 세대 `Subscribe`**(병합돼 있던 요청).
    Handled(Option<AgentCommand>),
}

/// Applies the daemon's three replay events (`SubscribeAck` · `ReplayComplete` · `SubscribeFailed`) to the
/// single-flight state and hands the boundary marker for the windows to `deliver`.
///
/// ★Why the three arms live in one function★: inline in the driver's event handling they are reachable only through
/// a connection, and a rule nobody can cheaply drive goes untested — the refusal arm once could be deleted with every
/// test still green. Here the harness (`tests/daemon_client_replay.rs`) drives them without a connection; what stays
/// with the driver is the call and the write of the returned `Subscribe`.
///
/// ★`deliver` is a parameter★: the real delivery (`output_channel::send_to_windows`) needs a `tauri::ipc::Channel`,
/// which the harness cannot build. Which windows (`router.targets`) and which bytes are decided here, so the harness
/// captures those two.
///
/// ★A marker takes the binary frames' path (ADR-0046)★ — through the event port (`events`) it would lose its place in
/// the order, so production's `deliver` is the binary fan-out function itself.
pub fn apply_replay_event(
    ev: &AgentEvent,
    flight: &mut ReplayFlightSet,
    subs: &mut HashMap<AgentId, SubState>,
    router: &OutputRouter,
    now: Instant,
    deliver: &mut dyn FnMut(&[WindowLabel], &[u8]),
) -> ReplayFollowUp {
    match ev {
        // ★구독 ack★: SubState.epoch 갱신(binary 팔 decide_epoch 의 기준) + in-flight 를 acked 로 전이 +
        //   truncated·continues_conversation(ADR-0226)·replay 머리 `replay_from`(ADR-0231) 기억(성공 마커에
        //   전파) + 진행(deadline 리셋).
        //   ★ADR-0046: 버퍼/커서 reset 없음★ —
        //   epoch 전환 재구독은 프론트의 권위 명부 관측(observeRoster)이 담당한다(ADR-0164 결정 8) —
        //   구독 deps `[viewId, agentId]`는 화신 표식을 의도적으로 제외한다.
        // ★반환 bool(epoch_changed) 의도적 무시★: 옛 배선은 이 값으로 창 render_seq 를 리셋했으나, 미러
        //   버퍼 제거(ADR-0046)로 진도 상태가 src-tauri 에 없다 → 리셋 대상이 없다. epoch 채택은 프론트가
        //   성공 마커 epoch 로 한다(gen 펜스).
        // ★`replay_from` 을 버리지 말 것(ADR-0231)★: 뷰는 성공 flush 를 `max(마지막+1, replay_from)` 에서
        //   시작한다. 버퍼의 최소 seq 로 대신하면 빈 replay(새 화신)에서 마커 뒤로 늦게 온 seq 0 을 건너뛴다.
        AgentEvent::SubscribeAck {
            agent_id,
            current_epoch,
            replay_from,
            truncated,
            continues_conversation,
            ..
        } => {
            let _ = protocol_state::apply_subscribe_ack(
                subs.entry(*agent_id).or_default(),
                *current_epoch,
            );
            flight.on_ack(
                *agent_id,
                *truncated,
                *continues_conversation,
                *replay_from,
                now,
            );
            ReplayFollowUp::Handled(None)
        }
        // ★replay 경계 각인(ADR-0046 M1)★: acked in-flight 를 성공 마커로 해소한다(Ack 전 도착 Complete =
        //   전대 고아 → Ignore). 대기열이 있으면 병합된 다음 Subscribe 를 호출자에게 돌려준다.
        AgentEvent::ReplayComplete { agent_id, epoch } => {
            match flight.on_complete(*agent_id, now) {
                Resolution::Ignore => ReplayFollowUp::Handled(None),
                Resolution::Emit { marker, send_next } => {
                    let frame = replay_flight::encode_marker_frame(*agent_id, *epoch, marker);
                    deliver(&router.targets(*agent_id), &frame);
                    // 마커의 epoch(= 방금 끝난 replay 의 것)과 다음 재구독의 epoch 는 별개다 — 다음 replay 는
                    //   **현재 알려진** epoch 로 전량 재구독한다.
                    ReplayFollowUp::Handled(send_next.then(|| AgentCommand::Subscribe {
                        agent_id: *agent_id,
                        epoch: known_epoch(subs, *agent_id),
                        after_seq: None,
                    }))
                }
            }
        }
        // ★거절 = single-flight 슬롯의 해제 계기★. 거절된 Subscribe 엔 Ack 도 Complete 도 뒤따르지 않으므로
        //   (데몬 `handle_subscribe` 가 Ack 발행 전 return) 슬롯을 지금 풀어도 오귀속될 응답이 없다 —
        //   만료(좀비) 해제를 택하지 않은 이유가 이 차이다(근거 정본 = `replay_flight::check_deadlines`).
        //   ★안 풀면 어떻게 되나★: 그 에이전트의 Subscribe 가 두 번 다시 못 나가 재spawn 해도 출력이 모든
        //   창에서 영구 두절된다(실측 2026-08-19).
        // ★warn 유지★: 거절이 `Error` 를 떠났으니 `emit_broadcast` 의 error 팔이 더는 이걸 못 잡는다. 이
        //   줄이 그 진단을 잇는다(사유 문장은 데몬이 쓴 것).
        AgentEvent::SubscribeFailed { agent_id, reason } => {
            tracing::warn!(
                %agent_id,
                // The daemon's free text — masked as the transport masks handshake text.
                reason = %engram_dashboard_transport::mask_reason(reason),
                "데몬이 Subscribe 를 거절 — single-flight 슬롯 해제"
            );
            let plan = plan_subscribe_refusal(flight, subs, *agent_id, now);
            if let Some(frame) = plan.marker_frame {
                deliver(&router.targets(*agent_id), &frame);
            }
            ReplayFollowUp::Handled(plan.next_subscribe)
        }
        _ => ReplayFollowUp::NotHandled,
    }
}

/// 그 에이전트에 대해 **마지막으로 알려진** epoch(모르면 `None`).
///
/// ★읽기는 항목을 만들지 않는다 — 이 맵의 증가 규칙 전부★: `subs` 에 항목을 **넣는** 자리는 두 곳뿐이다 —
/// `SubscribeAck` 각인과 binary frame 의 epoch 게이트. 둘 다 "그 에이전트가 실재한다" 는 증거를 손에 쥔
/// 자리다. 반면 읽는 자리(다음 세대 재구독 · 만료 마커 · 거절 마커)엔 그 증거가 없다 — 거기서
/// `entry().or_default()` 를 쓰면 사라진 에이전트의 빈 항목이 연결 수명 내내 쌓인다. 읽는 자리를 전부 이
/// 함수로 모아 그 규칙이 사이트마다 갈리지 않게 한다.
pub(super) fn known_epoch(subs: &HashMap<AgentId, SubState>, agent_id: AgentId) -> Option<u32> {
    subs.get(&agent_id).and_then(|st| st.epoch)
}

/// What one refusal produces — the failure marker frame for the windows and the next generation's `Subscribe` for the
/// wire. The caller (the connection driver) sends them.
#[derive(Debug)]
pub struct RefusalPlan {
    /// tag=255 마커 프레임. `None` = 풀 슬롯이 없었거나 이 세대의 실패 마커가 이미 1회 나갔다(중복 금지).
    pub marker_frame: Option<Vec<u8>>,
    /// 병합된 대기열의 `Subscribe`. `None` = 대기열 없음.
    pub next_subscribe: Option<AgentCommand>,
}

/// Decides what a daemon refusal of `Subscribe` (`AgentEvent::SubscribeFailed`) sends out — releasing the slot,
/// building the marker and advancing the queue happen here only.
///
/// ★Why `pub`★: the integration harness (`tests/daemon_client_replay.rs`) drives this rule without a connection.
///
/// ★The failure marker's `epoch` is a best guess, not authority★: a refusal has no `SubscribeAck`, so which session
/// this generation belongs to cannot be known. It carries the last known `SubState.epoch` (0 if none) — from a map that
/// outlives reconnects, so **it may be the previous daemon's**. The receiver does not trust it: the view applies no
/// epoch fence to failure markers (`src/api/protocolClient.ts` `evalMarker`) — with one the marker would be dropped
/// silently and the view would wait for its 10 s watchdog, defeating the marker.
///
/// ★Reads `subs` only★ — the rule and its reason belong to [`known_epoch`] (a refusal's commonest cause is "no such
/// agent", the spot where that rule matters most).
pub fn plan_subscribe_refusal(
    flight: &mut ReplayFlightSet,
    subs: &HashMap<AgentId, SubState>,
    agent_id: AgentId,
    now: Instant,
) -> RefusalPlan {
    let epoch = known_epoch(subs, agent_id);
    match flight.on_refused(agent_id, now) {
        RefusalOutcome::Ignore => RefusalPlan {
            marker_frame: None,
            next_subscribe: None,
        },
        RefusalOutcome::Released { marker, send_next } => RefusalPlan {
            // 실패 마커는 뷰의 대기를 지금 끊는다(없으면 10초 deadline 을 기다린다).
            marker_frame: marker
                .map(|m| replay_flight::encode_marker_frame(agent_id, epoch.unwrap_or(0), m)),
            // 다음 replay 는 **현재 알려진 epoch** 로 재구독한다(마커의 epoch 와 별개).
            next_subscribe: send_next.then_some(AgentCommand::Subscribe {
                agent_id,
                epoch,
                after_seq: None,
            }),
        },
    }
}
