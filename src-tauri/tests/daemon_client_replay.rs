//! 데몬 클라이언트의 replay single-flight **배선** 통합 테스트 — 소켓·데몬·Tauri 0.
//!
//! ★이 파일이 `tests/`(통합 타깃)에 있는 이유★: 재는 대상이 **실 소켓을 요구하는 배선**이라, 그 자리를
//! 포트로 끊어 세우는 통합 하네스가 제자리다(그래서 하네스 자신은 소켓·데몬·Tauri 를 하나도 안 쓴다).
//! ★"요구한다"이지 "못 세운다"가 아니다★ — 아래 `tests.rs` 의 루프백 WS 하네스가 그 배선을 실제로
//! 돌린다. 여기서 사는 것은 소켓 없이 얻는 결정론과 속도이지 유일한 가능성이 아니다.
//! 순수 단위 단언은 모듈 옆 `#[cfg(test)]` 로 가고 그쪽은 `--test lib_unit` 으로 돈다
//! (`src/daemon_client/tests.rs` 의 mock WS 하네스가 거기 있다 · 그 타깃을 세운 결정 = ADR-0174 · 현황 =
//! CLAUDE.md 「빌드·검증 명령」). 실행:
//! `cargo test -p engram-dashboard --test daemon_client_replay`(자식 프로세스를 하나도 안 띄우므로
//! `-- --test-threads=4` 를 붙이지 않는다 — 판정 규칙 정본 = CLAUDE.md 「빌드·검증 명령」).
//! ★워크스페이스 회귀에 안 실린다★ — 그 명령이 `--exclude engram-dashboard` 로 이 패키지를 통째로 뺀다.
//! 그래서 CI가 이 타깃만 따로 부르는 전용 스텝을 갖는다(`.github/workflows/ci.yml`).
//!
//! ## ★무엇을 지키나 — 거절당한 구독이 다시 나갈 수 있다★
//! 데몬 재기동 직후엔 세션이 없어(부팅 자동 복원 OFF) 재연결 replay 의 `Subscribe` 가 거절된다. 거절엔
//! `SubscribeAck`/`ReplayComplete` 가 뒤따르지 않으므로, 그 신호로 슬롯을 풀지 않으면 슬롯이 좀비로 남고
//! **그 에이전트의 Subscribe 가 두 번 다시 나가지 못해** 재spawn 해도 출력이 모든 창에서 영구 두절된다
//! (실측 2026-08-19). 그래서 여기서 재는 것은 셋이다: 거절이 슬롯을 푼다 · 병합된 **다음 세대의
//! `Subscribe` 가 실제로 만들어져 나간다**(이 팔을 지우면 좀비와 증상이 같다) · 이미 받아들여진(acked)
//! 구독은 거절로 풀리지 않는다.
//!
//! ## ★이벤트 갈래도 여기서 돈다★
//! 데몬이 보낸 `SubscribeAck`·`ReplayComplete`·`SubscribeFailed` 를 **어느 갈래로 보낼지**까지
//! [`apply_replay_event`] 가 소유한다. 그 갈래가 `select!` 안에 인라인으로 있던 시절엔 거절 팔을 통째로
//! 지워도 모든 게이트가 초록이었다 — 그 팔이 데몬의 새 이벤트와 운영 캐리어의 슬롯 해제를 잇는 **유일한
//! 접합**인데도. 여기서 그 접합을 태운다: 이벤트가 들어가 마커가 **창까지** 나가고 병합된 세대의
//! `Subscribe` 가 만들어지는 것을 실 `OutputRouter` 로 잰다.
//!
//! ## ★무엇이 안 덮이나(정직성)★
//! 남은 무검증 표면은 호출부의 **두 줄**이다 — 이 함수를 부르는 줄과, 돌려받은 명령을 `send_fire` 로 실
//! 소켓에 미는 줄. 그 자리는 실 소켓을 요구한다
//! (`tests/daemon_client_pending.rs`·`tests/layout_commands.rs` 헤더가 적는 같은 잔여다).
//! 배달구(`output_channel::send_to_windows`)도 안 돈다 — `tauri::ipc::Channel` 이 필요하다. 대신 **어느
//! 창으로 · 무슨 바이트를** 보내는지는 실 라우팅 표로 잰다.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use engram_dashboard_lib::daemon_client::connection::{
    apply_replay_event, plan_subscribe_refusal, ReplayFollowUp,
};
use engram_dashboard_lib::daemon_client::protocol_state::SubState;
use engram_dashboard_lib::daemon_client::replay_flight::{
    Marker, ReplayFlightSet, MARKER_FRAME_LEN, MARKER_TAG,
};
use engram_dashboard_lib::layout::{tree, ViewManager, MAIN_WINDOW_LABEL};
use engram_dashboard_lib::output_router::{OutputRouter, WindowLabel};
use engram_dashboard_protocol::{AgentCommand, AgentEvent, AgentId};

// 연결 태스크가 들고 도는 그 맵과 같은 타입 — 여기선 epoch 가드만 담는다.
type Subs = HashMap<AgentId, SubState>;

// ★실 라우팅 표를 쓴다(가짜로 흉내내지 않는다)★: 재려는 것이 "마커가 **그 에이전트가 보이는 창**으로
//   간다" 이므로, 목적지를 테스트가 지어내면 지어낸 값을 검증하는 꼴이 된다. main 창의 첫 빈 슬롯에
//   에이전트를 꽂고 라우터를 실제로 rebuild 한다.
fn router_showing(agent: AgentId) -> OutputRouter {
    let mut mgr = ViewManager::new();
    let view = mgr
        .list_tabs(MAIN_WINDOW_LABEL)
        .expect("main 창은 항상 있다")
        .active;
    let slot = tree::first_empty_slot_id(&mgr.snapshot(view).expect("뷰 존재").layout)
        .expect("빈 슬롯 존재");
    mgr.assign_agent(view, slot, agent.to_string())
        .expect("슬롯 배치");
    let router = OutputRouter::new();
    router.rebuild(&mgr);
    router
}

// 배달구를 대신 받아 (목적지 창, 바이트)를 기록한다.
#[derive(Default)]
struct Delivered(Vec<(Vec<WindowLabel>, Vec<u8>)>);

// 이벤트 하나를 태우고 배달된 것 + 후속 명령을 함께 회수한다.
fn feed(
    ev: &AgentEvent,
    fs: &mut ReplayFlightSet,
    subs: &mut Subs,
    router: &OutputRouter,
    now: Instant,
) -> (Delivered, ReplayFollowUp) {
    let mut out = Delivered::default();
    let follow = {
        let mut deliver = |labels: &[WindowLabel], bytes: &[u8]| {
            out.0.push((labels.to_vec(), bytes.to_vec()));
        };
        apply_replay_event(ev, fs, subs, router, now, &mut deliver)
    };
    (out, follow)
}

// 운영과 같은 급의 무진행 상한. 이 스위트는 시간을 직접 넘기므로 만료를 건드리지 않는다.
fn flight() -> ReplayFlightSet {
    ReplayFlightSet::new(Duration::from_secs(10))
}

// 마커 프레임에서 gen(BE u64)과 failed 플래그(bit1)를 도로 꺼낸다 — 레이아웃 정본은
// `engram_dashboard_agent::replay_flight::encode_marker_frame` 문단이고, 여기선 그 값을 읽기만 한다.
fn decode_marker(frame: &[u8]) -> (AgentId, u32, Marker) {
    assert_eq!(frame.len(), MARKER_FRAME_LEN, "마커 프레임 길이");
    assert_eq!(frame[0], MARKER_TAG, "tag=255");
    let agent = AgentId::from_slice(&frame[1..17]).expect("agentId 16바이트");
    let epoch = u32::from_be_bytes(frame[17..21].try_into().unwrap());
    let generation = u64::from_be_bytes(frame[21..29].try_into().unwrap());
    let flags = frame[29];
    (
        agent,
        epoch,
        Marker {
            generation,
            truncated: flags & 0b0000_0001 != 0,
            failed: flags & 0b0000_0010 != 0,
        },
    )
}

// ★이 단언이 결함의 핵심★ — 고치기 전엔 거절 뒤 그 에이전트의 Subscribe 가 영영 안 나갔다.
#[test]
fn refusal_releases_slot_so_the_next_request_can_send_again() {
    let mut fs = flight();
    let subs = Subs::new();
    let agent = AgentId::new_v4();
    let now = Instant::now();

    let first = fs.request_replay(agent, now);
    assert!(first.send_now, "idle 요청은 즉시 Subscribe");

    let plan = plan_subscribe_refusal(&mut fs, &subs, agent, now);
    let frame = plan.marker_frame.expect("첫 거절은 실패 마커를 낸다");
    let (got_agent, _epoch, marker) = decode_marker(&frame);
    assert_eq!(got_agent, agent, "마커는 자기 에이전트 앞으로");
    assert_eq!(marker.generation, first.generation, "해제되는 건 그 세대");
    assert!(marker.failed, "거절 = 실패 마커(뷰가 10초를 안 기다리게)");
    assert!(plan.next_subscribe.is_none(), "대기열이 없으면 다음 없음");

    let next = fs.request_replay(agent, now);
    assert!(next.send_now, "해제됐으니 새 Subscribe 가 즉시 나간다");
}

// ★`next_subscribe` 팔이 이 스위트의 두 번째 존재 이유★: 이걸 지우면(또는 만들지 않으면) 병합된 세대의
//   Subscribe 가 wire 로 못 나가 증상이 좀비와 똑같아진다 — 슬롯은 풀렸는데 아무도 다시 안 묻는다.
#[test]
fn refusal_hands_the_coalesced_generation_a_subscribe_to_send() {
    let mut fs = flight();
    let mut subs = Subs::new();
    // 이 에이전트는 이미 Ack 을 받아 epoch 7 로 알려져 있다 — 다음 재구독은 그 epoch 로 나가야 한다.
    subs.insert(AgentId::nil(), SubState { epoch: None }); // 남의 항목(오배달 방어)
    let agent = AgentId::new_v4();
    subs.insert(agent, SubState { epoch: Some(7) });
    let now = Instant::now();

    let first = fs.request_replay(agent, now); // gen_k = wire 로 나감
    let waiter = fs.request_replay(agent, now); // gen_k+1 = 병합(아직 안 나감)
    assert!(!waiter.send_now, "in-flight 중이라 병합");

    let plan = plan_subscribe_refusal(&mut fs, &subs, agent, now);
    let (_, epoch, marker) = decode_marker(&plan.marker_frame.expect("실패 마커"));
    assert_eq!(marker.generation, first.generation, "해제되는 건 앞 세대");
    assert_eq!(epoch, 7, "마지막으로 알려진 epoch 를 싣는다(권위값 아님)");

    match plan
        .next_subscribe
        .expect("병합된 세대의 Subscribe 가 나와야")
    {
        AgentCommand::Subscribe {
            agent_id,
            epoch,
            after_seq,
        } => {
            assert_eq!(agent_id, agent);
            assert_eq!(epoch, Some(7), "현재 알려진 epoch 로 재구독");
            assert_eq!(after_seq, None, "재replay 는 항상 전량(from-oldest)");
        }
        other => panic!("Subscribe 여야: {other:?}"),
    }

    // 그 세대가 in-flight 로 올라섰다 — 새 요청은 다시 병합되고, 정상 완료 경로가 그대로 산다.
    assert!(!fs.request_replay(agent, now).send_now);
}

// ★방어★: Ack 를 받은 구독은 데몬이 받아들인 것이라 거절과 공존할 수 없다. 그래도 풀지 않는 이유 =
//   풀면 뒤따라오는 ReplayComplete 가 빈 슬롯을 만나 무시되고, 그 replay 를 기다리던 뷰는 성공 마커를
//   영영 못 받는다(잘못 발화한 거절 하나가 정상 replay 를 죽이는 경로).
#[test]
fn refusal_leaves_an_acked_healthy_subscription_alone() {
    let mut fs = flight();
    let subs = Subs::new();
    let agent = AgentId::new_v4();
    let now = Instant::now();

    fs.request_replay(agent, now);
    fs.on_ack(agent, false, now);

    let plan = plan_subscribe_refusal(&mut fs, &subs, agent, now);
    assert!(plan.marker_frame.is_none(), "acked 슬롯엔 실패 마커 없음");
    assert!(plan.next_subscribe.is_none(), "대기열 전진도 없음");
    assert!(
        !fs.request_replay(agent, now).send_now,
        "슬롯은 그대로 점유"
    );
}

// 우리가 낸 적 없는 구독의 거절(stray)·이미 해제된 슬롯의 두 번째 거절은 아무 것도 하지 않는다 —
// 중복 실패 마커는 뷰의 재요청 사다리를 두 번 민다.
#[test]
fn stray_refusal_produces_nothing() {
    let mut fs = flight();
    let subs = Subs::new();
    let agent = AgentId::new_v4();
    let now = Instant::now();

    let plan = plan_subscribe_refusal(&mut fs, &subs, agent, now);
    assert!(plan.marker_frame.is_none() && plan.next_subscribe.is_none());

    fs.request_replay(agent, now);
    let _ = plan_subscribe_refusal(&mut fs, &subs, agent, now);
    let second = plan_subscribe_refusal(&mut fs, &subs, agent, now);
    assert!(
        second.marker_frame.is_none() && second.next_subscribe.is_none(),
        "이미 해제된 슬롯의 두 번째 거절은 무시"
    );
}

// ★`subs` 에 유령 항목을 만들지 않는다★: 거절의 가장 흔한 사유가 "그 에이전트가 없다" 라서, 여기서
//   삽입하면 존재하지 않는 에이전트의 빈 SubState 가 연결 수명 내내 쌓인다.
//
// ★`plan_subscribe_refusal` 로는 이걸 못 잰다(그 함수는 `&HashMap` 을 받아 타입이 이미 막는다 — 결함을
//   되살리면 컴파일이 깨지지 소스가 붉어지지 않는다)★. 그래서 **`&mut` 를 쥔** 바깥 진입점으로 잰다 —
//   거기서는 `entry().or_default()` 가 컴파일된다. 같은 이벤트 하나가 읽기 자리 셋(다음 세대 재구독 ·
//   만료 마커 · 거절 마커)의 규칙을 대표한다(`known_epoch`).
#[test]
fn a_refusal_for_an_unknown_agent_leaves_no_ghost_substate() {
    let mut fs = flight();
    let mut subs = Subs::new();
    let agent = AgentId::new_v4();
    let router = router_showing(agent);
    let now = Instant::now();

    fs.request_replay(agent, now);
    let (_out, _follow) = feed(
        &AgentEvent::SubscribeFailed {
            agent_id: agent,
            reason: "agent not found".to_string(),
        },
        &mut fs,
        &mut subs,
        &router,
        now,
    );
    assert!(
        subs.is_empty(),
        "읽기 자리는 항목을 만들지 않는다 — 사라진 에이전트의 빈 항목이 연결 수명 내내 쌓인다"
    );
}

// ── 이벤트 → 창까지의 접합(select! arm 이 인라인으로 갖고 있던 그 갈래) ─────────────
// ★이 스위트의 존재 이유★: 거절 팔이 인라인이던 시절엔 그 팔을 통째로 지워도 모든 게이트가 초록이었다.
//   여기 세 테스트가 그 접합(이벤트 인식 → 슬롯 해제 → **창으로 마커** → 다음 세대 Subscribe)을 잡는다.
#[test]
fn a_refusal_delivers_a_failure_marker_to_the_windows_showing_that_agent() {
    let mut fs = flight();
    let mut subs = Subs::new();
    let agent = AgentId::new_v4();
    let router = router_showing(agent);
    let now = Instant::now();

    let first = fs.request_replay(agent, now);
    assert!(first.send_now, "전제: 이 세대가 wire 로 나갔다");

    let (out, follow) = feed(
        &AgentEvent::SubscribeFailed {
            agent_id: agent,
            reason: "agent not found".to_string(),
        },
        &mut fs,
        &mut subs,
        &router,
        now,
    );

    assert_eq!(out.0.len(), 1, "거절 하나 = 마커 하나");
    let (labels, frame) = &out.0[0];
    assert_eq!(
        labels.as_slice(),
        &[MAIN_WINDOW_LABEL.to_string()],
        "그 에이전트가 보이는 창으로 간다(router.targets)"
    );
    let (got_agent, _epoch, marker) = decode_marker(frame);
    assert_eq!(got_agent, agent, "마커는 자기 에이전트 앞으로");
    assert_eq!(marker.generation, first.generation, "해제되는 건 그 세대");
    assert!(marker.failed, "거절 = 실패 마커(뷰가 10초를 안 기다리게)");
    assert!(
        matches!(follow, ReplayFollowUp::Handled(None)),
        "대기열이 없으면 다음 Subscribe 없음"
    );

    assert!(
        fs.request_replay(agent, now).send_now,
        "슬롯이 풀려 새 Subscribe 가 다시 나간다(이게 결함의 핵심)"
    );
}

// 병합돼 있던 세대가 **wire 로 나갈 명령으로** 나와야 한다 — 안 나오면 슬롯은 풀렸는데 아무도 다시 안
// 물어 증상이 좀비와 같아진다.
#[test]
fn a_refusal_hands_back_the_coalesced_generations_subscribe() {
    let mut fs = flight();
    let mut subs = Subs::new();
    let agent = AgentId::new_v4();
    let router = router_showing(agent);
    let now = Instant::now();

    // 먼저 Ack 을 태워 이 에이전트의 알려진 epoch 를 7 로 만든다(다음 재구독이 그 값을 실어야 한다).
    let (ack_out, ack_follow) = feed(
        &AgentEvent::SubscribeAck {
            agent_id: agent,
            action: engram_dashboard_protocol::SubscribeAction::Reset,
            current_epoch: 7,
            oldest_seq: 0,
            latest_seq: 0,
            replay_from: 0,
            truncated: false,
        },
        &mut fs,
        &mut subs,
        &router,
        now,
    );
    assert!(ack_out.0.is_empty(), "Ack 자체는 창으로 아무것도 안 보낸다");
    assert!(matches!(ack_follow, ReplayFollowUp::Handled(None)));

    // 그 Ack 은 in-flight 가 없던 시점의 것이라 슬롯을 만들지 않는다 — 이제 진짜 요청 둘을 낸다.
    let first = fs.request_replay(agent, now);
    let waiter = fs.request_replay(agent, now);
    assert!(!waiter.send_now, "in-flight 중이라 병합");

    let (out, follow) = feed(
        &AgentEvent::SubscribeFailed {
            agent_id: agent,
            reason: "agent not found".to_string(),
        },
        &mut fs,
        &mut subs,
        &router,
        now,
    );

    let (_, epoch, marker) = decode_marker(&out.0[0].1);
    assert_eq!(marker.generation, first.generation, "해제되는 건 앞 세대");
    assert_eq!(epoch, 7, "마지막으로 알려진 epoch 를 싣는다(권위값 아님)");

    match follow {
        ReplayFollowUp::Handled(Some(AgentCommand::Subscribe {
            agent_id,
            epoch,
            after_seq,
        })) => {
            assert_eq!(agent_id, agent);
            assert_eq!(epoch, Some(7), "현재 알려진 epoch 로 재구독");
            assert_eq!(after_seq, None, "재replay 는 항상 전량(from-oldest)");
        }
        other => panic!("병합된 세대의 Subscribe 가 나와야: {other:?}"),
    }
}

// 성공 경로도 같은 접합을 지난다 — 성공 마커가 창까지 가야 뷰가 flush 한다.
#[test]
fn a_replay_complete_delivers_a_success_marker_to_the_same_windows() {
    let mut fs = flight();
    let mut subs = Subs::new();
    let agent = AgentId::new_v4();
    let router = router_showing(agent);
    let now = Instant::now();

    let first = fs.request_replay(agent, now);
    let (_, _) = feed(
        &AgentEvent::SubscribeAck {
            agent_id: agent,
            action: engram_dashboard_protocol::SubscribeAction::Reset,
            current_epoch: 3,
            oldest_seq: 0,
            latest_seq: 0,
            replay_from: 0,
            truncated: true,
        },
        &mut fs,
        &mut subs,
        &router,
        now,
    );

    let (out, follow) = feed(
        &AgentEvent::ReplayComplete {
            agent_id: agent,
            epoch: 3,
        },
        &mut fs,
        &mut subs,
        &router,
        now,
    );

    assert_eq!(out.0.len(), 1, "완료 하나 = 마커 하나");
    let (labels, frame) = &out.0[0];
    assert_eq!(labels.as_slice(), &[MAIN_WINDOW_LABEL.to_string()]);
    let (_, epoch, marker) = decode_marker(frame);
    assert_eq!(epoch, 3, "완료된 replay 의 epoch 를 싣는다");
    assert_eq!(marker.generation, first.generation);
    assert!(!marker.failed, "완료 = 성공 마커");
    assert!(marker.truncated, "Ack 의 truncated 가 마커까지 전파");
    assert!(
        matches!(follow, ReplayFollowUp::Handled(None)),
        "대기열 없음"
    );
}

// replay 계열이 아닌 이벤트는 이 함수가 삼키면 안 된다 — 삼키면 인바운드 명령과 broadcast 가 통째로
// 사라진다(그 둘의 유일한 입구가 이 함수의 `NotHandled` 갈래다).
#[test]
fn unrelated_events_fall_through_to_the_callers_other_branches() {
    let mut fs = flight();
    let mut subs = Subs::new();
    let agent = AgentId::new_v4();
    let router = router_showing(agent);

    let (out, follow) = feed(
        &AgentEvent::AgentListUpdated { agents: vec![] },
        &mut fs,
        &mut subs,
        &router,
        Instant::now(),
    );
    assert!(out.0.is_empty(), "창으로 아무것도 안 보낸다");
    assert!(matches!(follow, ReplayFollowUp::NotHandled));
}
