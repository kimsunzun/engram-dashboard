//! Integration test of the daemon client's replay single-flight **wiring** — zero sockets · daemons · Tauri.
//!
//! ★Why `tests/` (an integration target)★: the rules measured here sit behind a connection in the driver, and the
//! functions that hold them (`daemon_client::replay_events`) are driven here without one — no socket, daemon or Tauri.
//! The driver's own suite (`src/daemon_client/connection/tests.rs`, `--test lib_unit`) drives the same path on a memory
//! network; this target buys determinism and speed, not the only way (the target's decision = ADR-0174 · status =
//! `.claude/skill-bindings/qa.md` 「모듈별 명령」). Run:
//! `cargo test -p engram-dashboard --test daemon_client_replay`(자식 프로세스를 하나도 안 띄우므로
//! `-- --test-threads=4` 를 붙이지 않는다 — 판정 규칙 정본 = `.claude/skill-bindings/qa.md` 「모듈별 명령」).
//! CI 는 이 타깃만 따로 부르는 전용 스텝도 갖는다(`.github/workflows/ci.yml`).
//!
//! ## ★What it guards — a refused subscription can go out again★
//! Right after a daemon restart there is no session (boot auto-restore OFF), so the `Subscribe` of the reconnect replay is refused. A refusal is not
//! followed by `SubscribeAck`/`ReplayComplete`, so unless that signal releases the slot, the slot is left as a zombie and
//! **that agent's Subscribe can never go out again**, so even after re-spawn output is permanently cut off in every window
//! (measured 2026-08-19). So what is measured here is three things: a refusal releases the slot · the merged **next-generation
//! `Subscribe` is actually built and goes out** (deleting this arm gives the same symptom as a zombie) · an already accepted (acked)
//! subscription is not released by a refusal.
//!
//! ## ★The event branches run here too★
//! The `SubscribeAck` · `ReplayComplete` · `SubscribeFailed` the daemon sent — even **which branch to send them to** is
//! owned by [`apply_replay_event`]. In the days when that branching was inline inside `select!`, deleting the refusal arm entirely
//! left every gate green — even though that arm is **the only
//! junction** connecting the daemon's new event to the production carrier's slot release. Here that junction is exercised: an event goes in, the marker goes out **all the way to the window**, and the merged generation's
//! `Subscribe` is built, measured with a real `OutputRouter`.
//!
//! ## ★What is not covered here★
//! The driver's call of this function and its write of the returned `Subscribe` — the driver's suite covers those on
//! a memory network. The delivery itself (`output_channel::send_to_windows`) needs a `tauri::ipc::Channel`; instead
//! **which windows · which bytes** are measured against a real routing table.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use engram_dashboard_lib::daemon_client::protocol_state::SubState;
use engram_dashboard_lib::daemon_client::replay_events::{
    apply_replay_event, plan_subscribe_refusal, ReplayFollowUp,
};
use engram_dashboard_lib::daemon_client::replay_flight::{
    Marker, ReplayFlightSet, MARKER_FRAME_LEN, MARKER_TAG,
};
use engram_dashboard_lib::layout::{tree, ViewManager, MAIN_WINDOW_LABEL};
use engram_dashboard_lib::output_router::{OutputRouter, WindowLabel};
use engram_dashboard_protocol::{AgentCommand, AgentEvent, AgentId};

// The same type as the map the connection task carries — here it holds only the epoch guard.
type Subs = HashMap<AgentId, SubState>;

// ★Use the real routing table (do not imitate it with a fake)★: what we want to measure is "the marker goes to **the window where that agent is seen**",
//   so if the test invents the destination, it ends up verifying the invented value. Plug the agent into main window's first empty slot
//   and actually rebuild the router.
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

// Receive the delivery outlet in its place and record (destination window, bytes).
#[derive(Default)]
struct Delivered(Vec<(Vec<WindowLabel>, Vec<u8>)>);

// Run one event through and collect together what was delivered + the follow-up command.
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

// The same-grade no-progress limit as production. This suite advances time directly, so it does not touch expiry.
fn flight() -> ReplayFlightSet {
    ReplayFlightSet::new(Duration::from_secs(10))
}

// Take back out of the marker frame gen (BE u64) · flag set (bit0~2) · replay head (BE u64) — the canonical layout is the
// `engram_dashboard_lib::daemon_client::replay_flight::encode_marker_frame` paragraph, and here those values are
// only read.
fn decode_marker(frame: &[u8]) -> (AgentId, u32, Marker) {
    assert_eq!(frame.len(), MARKER_FRAME_LEN, "마커 프레임 길이");
    assert_eq!(frame[0], MARKER_TAG, "tag=255");
    let agent = AgentId::from_slice(&frame[1..17]).expect("agentId 16바이트");
    let epoch = u32::from_be_bytes(frame[17..21].try_into().unwrap());
    let generation = u64::from_be_bytes(frame[21..29].try_into().unwrap());
    let flags = frame[29];
    assert_eq!(
        flags & !0b0000_0111,
        0,
        "정의 안 된 플래그 비트: {flags:#010b}"
    );
    let replay_from = u64::from_be_bytes(frame[30..38].try_into().unwrap());
    (
        agent,
        epoch,
        Marker {
            generation,
            truncated: flags & 0b0000_0001 != 0,
            failed: flags & 0b0000_0010 != 0,
            continues_conversation: flags & 0b0000_0100 != 0,
            replay_from,
        },
    )
}

// ★This assertion is the core of the defect★ — before the fix, after a refusal that agent's Subscribe never went out again.
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

// ★The `next_subscribe` arm is this suite's second reason to exist★: if you delete it (or do not build it), the merged generation's
//   Subscribe cannot go out on the wire and the symptom becomes exactly the zombie — the slot is released but no one asks again.
#[test]
fn refusal_hands_the_coalesced_generation_a_subscribe_to_send() {
    let mut fs = flight();
    let mut subs = Subs::new();
    // This agent has already received Ack and is known at epoch 7 — the next re-subscription must go out with that epoch.
    subs.insert(AgentId::nil(), SubState { epoch: None }); // Another's entry (defense against misdelivery)
    let agent = AgentId::new_v4();
    subs.insert(agent, SubState { epoch: Some(7) });
    let now = Instant::now();

    let first = fs.request_replay(agent, now); // gen_k = went out on the wire
    let waiter = fs.request_replay(agent, now); // gen_k+1 = merged (not yet gone out)
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

    // That generation has risen to in-flight — a new request is merged again, and the normal completion path stays alive.
    assert!(!fs.request_replay(agent, now).send_now);
}

// ★Defense★: a subscription that received Ack was accepted by the daemon, so it cannot coexist with a refusal. The reason we still do not release it =
//   if released, the ReplayComplete that follows meets an empty slot and is ignored, and the view waiting for that replay never
//   receives the success marker (a path where one wrongly fired refusal kills a normal replay).
#[test]
fn refusal_leaves_an_acked_healthy_subscription_alone() {
    let mut fs = flight();
    let subs = Subs::new();
    let agent = AgentId::new_v4();
    let now = Instant::now();

    fs.request_replay(agent, now);
    fs.on_ack(agent, false, false, 0, now);

    let plan = plan_subscribe_refusal(&mut fs, &subs, agent, now);
    assert!(plan.marker_frame.is_none(), "acked 슬롯엔 실패 마커 없음");
    assert!(plan.next_subscribe.is_none(), "대기열 전진도 없음");
    assert!(
        !fs.request_replay(agent, now).send_now,
        "슬롯은 그대로 점유"
    );
}

// A refusal of a subscription we never issued (stray) · a second refusal of an already released slot does nothing —
// a duplicate failure marker pushes the view's re-request ladder twice.
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

// ★Do not create a phantom entry in `subs`★: the most common reason for a refusal is "that agent does not exist", so if inserted here,
//   an empty SubState for a nonexistent agent would pile up for the whole life of the connection.
//
// ★`plan_subscribe_refusal` cannot measure this (that function takes `&HashMap` and the type already blocks it — if the defect
//   is revived, compilation breaks rather than the source going red)★. So we measure it through the outer entry point **that holds `&mut`** —
//   there `entry().or_default()` compiles. One and the same event represents the rule of the three read spots (next-generation re-subscription ·
//   expiry marker · refusal marker) (`known_epoch`).
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

// ── The junction from event to window (the branching that the select! arm held inline) ─────────────
// ★This suite's reason to exist★: in the days when the refusal arm was inline, deleting that arm entirely left every gate green.
//   The three tests here catch that junction (event recognition → slot release → **marker to the window** → next-generation Subscribe).
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

// The merged generation must come out **as a command to go out on the wire** — if not, the slot is released but no one asks again, so the
// symptom becomes the same as a zombie.
#[test]
fn a_refusal_hands_back_the_coalesced_generations_subscribe() {
    let mut fs = flight();
    let mut subs = Subs::new();
    let agent = AgentId::new_v4();
    let router = router_showing(agent);
    let now = Instant::now();

    // First run an Ack through to make this agent's known epoch 7 (the next re-subscription must carry that value).
    let (ack_out, ack_follow) = feed(
        &AgentEvent::SubscribeAck {
            agent_id: agent,
            action: engram_dashboard_protocol::SubscribeAction::Reset,
            current_epoch: 7,
            oldest_seq: 0,
            latest_seq: 0,
            replay_from: 0,
            truncated: false,
            continues_conversation: true,
        },
        &mut fs,
        &mut subs,
        &router,
        now,
    );
    assert!(ack_out.0.is_empty(), "Ack 자체는 창으로 아무것도 안 보낸다");
    assert!(matches!(ack_follow, ReplayFollowUp::Handled(None)));

    // That Ack was from a time when nothing was in-flight, so it does not create a slot — now issue the two real requests.
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
    assert!(
        !marker.continues_conversation,
        "앞선 Ack(표식 참)은 어느 슬롯에도 안 앉았고 거절되는 슬롯은 Ack 을 받은 적이 없다 — bit2 는 0"
    );

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

// The success path passes through the same junction too — the success marker must reach the window for the view to flush.
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
            continues_conversation: false,
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
        !marker.continues_conversation,
        "Ack 이 이어받기 화신이 아니라고 했다"
    );
    assert!(
        matches!(follow, ReplayFollowUp::Handled(None)),
        "대기열 없음"
    );
}

// ★The Ack's takeover marker reaches bit2 of the bytes going to the window (ADR-0226)★ — the webview uses this bit to draw loading instead of the first
//   screen. truncated · the marker value are flipped relative to the test right above, so even if the two fields are wired swapped, one of the
//   two goes red.
#[test]
fn a_replay_complete_carries_the_acks_continues_conversation_in_bit2() {
    let mut fs = flight();
    let mut subs = Subs::new();
    let agent = AgentId::new_v4();
    let router = router_showing(agent);
    let now = Instant::now();

    fs.request_replay(agent, now);
    let _ = feed(
        &AgentEvent::SubscribeAck {
            agent_id: agent,
            action: engram_dashboard_protocol::SubscribeAction::Reset,
            current_epoch: 3,
            oldest_seq: 0,
            latest_seq: 0,
            replay_from: 0,
            truncated: false,
            continues_conversation: true,
        },
        &mut fs,
        &mut subs,
        &router,
        now,
    );
    let (out, _) = feed(
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
    assert_eq!(frame.len(), MARKER_FRAME_LEN, "길이 38 불변");
    assert_eq!(frame[29], 0b0000_0100, "성공 + 이어받기 = bit2 하나만");
    let (_, _, marker) = decode_marker(frame);
    assert!(marker.continues_conversation);
    assert!(!marker.truncated && !marker.failed);
}

// ★The Ack's replay head reaches the marker bytes going to the window (ADR-0231)★ — the old wiring threw this field of the Ack away with the pattern `..`.
//   The view starts the success flush with this value (`max(마지막+1, replay_from)`), so if missing, a seq 0 that arrives late after the marker
//   would be skipped in a new incarnation with an empty replay. The value is set to a nonzero number to block the shape where 「안 실어도 0」 (0 even if not carried)
//   happens to match.
#[test]
fn a_replay_complete_carries_the_acks_replay_from() {
    let mut fs = flight();
    let mut subs = Subs::new();
    let agent = AgentId::new_v4();
    let router = router_showing(agent);
    let now = Instant::now();

    fs.request_replay(agent, now);
    let _ = feed(
        &AgentEvent::SubscribeAck {
            agent_id: agent,
            action: engram_dashboard_protocol::SubscribeAction::Reset,
            current_epoch: 3,
            oldest_seq: 40,
            latest_seq: 60,
            replay_from: 40,
            truncated: true,
            continues_conversation: false,
        },
        &mut fs,
        &mut subs,
        &router,
        now,
    );
    let (out, _) = feed(
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
    let (_, _, marker) = decode_marker(&out.0[0].1);
    assert!(!marker.failed);
    assert_eq!(marker.replay_from, 40, "Ack 의 replay 머리가 마커까지");
}

// Events that are not of the replay family must not be swallowed by this function — if swallowed, inbound commands and broadcast vanish
// entirely (the only entrance of those two is this function's `NotHandled` branch).
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
