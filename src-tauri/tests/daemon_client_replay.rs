//! Integration test of the daemon client's replay single-flight wiring — zero sockets · daemons · Tauri.
//! Why `tests/`: the rules measured here sit behind a connection in the driver; the functions holding them (`daemon_client::replay_events`) are driven here without one. The driver's own suite (`src/daemon_client/connection/tests.rs`, `--test lib_unit`) drives the same path on a memory network — this target buys determinism and speed, not the only way (decision = ADR-0174,
// hangul-ok-begin
//! status = `.claude/skill-bindings/qa.md` 「모듈별 명령」). Run:
// hangul-ok-end
//! `cargo test -p engram-dashboard --test daemon_client_replay` (no child process, so
// hangul-ok-begin
//! no `-- --test-threads=4` — rule = `.claude/skill-bindings/qa.md` 「모듈별 명령」). CI has a dedicated step for this target (`.github/workflows/ci.yml`).
// hangul-ok-end
//!
//! ## What it guards — a refused subscription can go out again
//! Right after a daemon restart there is no session (boot auto-restore OFF), so the reconnect replay's `Subscribe` is refused. A refusal is not followed by `SubscribeAck`/`ReplayComplete`, so unless that signal releases the slot it stays a zombie and that agent's Subscribe can never go out again — output stays cut off in every window even after re-spawn (measured 2026-08-19). Measured: a refusal releases the slot · the merged next-generation `Subscribe` is actually built and goes out (deleting this arm gives the zombie symptom) · an already acked subscription is not released by a refusal.
//!
//! ## The event branches run here too
//! Which branch the daemon's `SubscribeAck` · `ReplayComplete` · `SubscribeFailed` go to is owned by [`apply_replay_event`]. When that branching was inline in `select!`, deleting the refusal arm left every gate green — though it is the only junction connecting the daemon's new event to the production carrier's slot release. Here the junction is exercised: an event goes in, the marker goes out all the way to the window, and the merged generation's `Subscribe` is built, measured with a real `OutputRouter`.
//!
//! ## Not covered here
//! The driver's call of this function and its write of the returned `Subscribe` (the driver's suite covers those on a memory network). The delivery itself (`output_channel::send_to_windows`) needs a `tauri::ipc::Channel`; instead which windows · which bytes are measured against a real routing table.

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

// Use the real routing table, not a fake: what is measured is "the marker goes to the window where that agent is seen", and an invented destination would verify the invented value. Plug the agent into main window's first empty slot and actually rebuild the router.
fn router_showing(agent: AgentId) -> OutputRouter {
    let mut mgr = ViewManager::new();
    let view = mgr
        .list_tabs(MAIN_WINDOW_LABEL)
        .expect("the main window always exists")
        .active;
    let slot = tree::first_empty_slot_id(&mgr.snapshot(view).expect("view exists").layout)
        .expect("an empty slot exists");
    mgr.assign_agent(view, slot, agent.to_string())
        .expect("slot placement");
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

// Reads gen (BE u64) · flag set (bit0~2) · replay head (BE u64) back out of the marker frame — the canonical layout is the `engram_dashboard_lib::daemon_client::replay_flight::encode_marker_frame` paragraph; here the values are only read.
fn decode_marker(frame: &[u8]) -> (AgentId, u32, Marker) {
    assert_eq!(frame.len(), MARKER_FRAME_LEN, "marker frame length");
    assert_eq!(frame[0], MARKER_TAG, "tag=255");
    let agent = AgentId::from_slice(&frame[1..17]).expect("agentId 16 bytes");
    let epoch = u32::from_be_bytes(frame[17..21].try_into().unwrap());
    let generation = u64::from_be_bytes(frame[21..29].try_into().unwrap());
    let flags = frame[29];
    assert_eq!(flags & !0b0000_0111, 0, "undefined flag bit: {flags:#010b}");
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
    assert!(first.send_now, "an idle request subscribes immediately");

    let plan = plan_subscribe_refusal(&mut fs, &subs, agent, now);
    let frame = plan
        .marker_frame
        .expect("the first refusal yields a failure marker");
    let (got_agent, _epoch, marker) = decode_marker(&frame);
    assert_eq!(got_agent, agent, "the marker is addressed to its own agent");
    assert_eq!(
        marker.generation, first.generation,
        "what is released is that generation"
    );
    assert!(
        marker.failed,
        "refusal = failure marker (so the view does not wait 10 s)"
    );
    assert!(plan.next_subscribe.is_none(), "no queue means no next one");

    let next = fs.request_replay(agent, now);
    assert!(
        next.send_now,
        "released, so the new Subscribe goes out immediately"
    );
}

// The `next_subscribe` arm is this suite's second reason to exist: deleted (or not built), the merged generation's Subscribe cannot go out and the symptom is exactly the zombie — the slot is released but no one asks again.
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
    assert!(!waiter.send_now, "in flight, so merged");

    let plan = plan_subscribe_refusal(&mut fs, &subs, agent, now);
    let (_, epoch, marker) = decode_marker(&plan.marker_frame.expect("failure marker"));
    assert_eq!(
        marker.generation, first.generation,
        "what is released is the earlier generation"
    );
    assert_eq!(
        epoch, 7,
        "carries the last known epoch (not an authoritative value)"
    );

    match plan
        .next_subscribe
        .expect("the merged generation's Subscribe must come out")
    {
        AgentCommand::Subscribe {
            agent_id,
            epoch,
            after_seq,
        } => {
            assert_eq!(agent_id, agent);
            assert_eq!(
                epoch,
                Some(7),
                "re-subscribes with the currently known epoch"
            );
            assert_eq!(
                after_seq, None,
                "a re-replay is always in full (from-oldest)"
            );
        }
        other => panic!("should be Subscribe: {other:?}"),
    }

    // That generation has risen to in-flight — a new request is merged again, and the normal completion path stays alive.
    assert!(!fs.request_replay(agent, now).send_now);
}

// Defense: a subscription that got an Ack was accepted by the daemon and cannot coexist with a refusal. Releasing it anyway would make the following ReplayComplete meet an empty slot and be ignored, so the view waiting for that replay never gets the success marker (one wrongly fired refusal killing a normal replay).
#[test]
fn refusal_leaves_an_acked_healthy_subscription_alone() {
    let mut fs = flight();
    let subs = Subs::new();
    let agent = AgentId::new_v4();
    let now = Instant::now();

    fs.request_replay(agent, now);
    fs.on_ack(agent, false, false, 0, now);

    let plan = plan_subscribe_refusal(&mut fs, &subs, agent, now);
    assert!(
        plan.marker_frame.is_none(),
        "no failure marker for an acked slot"
    );
    assert!(plan.next_subscribe.is_none(), "no queue advance either");
    assert!(
        !fs.request_replay(agent, now).send_now,
        "the slot stays occupied"
    );
}

// A refusal of a subscription we never issued (stray) · a second refusal of an already released slot does nothing — a duplicate failure marker would push the view's re-request ladder twice.
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
        "a second refusal on an already released slot is ignored"
    );
}

// Do not create a phantom entry in `subs`: the most common refusal reason is "that agent does not exist", so inserting here would pile up empty SubStates for nonexistent agents for the whole connection.
// `plan_subscribe_refusal` cannot measure this (it takes `&HashMap`, so the type already blocks it — a revived defect breaks compilation rather than turning the test red); it is measured through the outer entry point holding `&mut`, where `entry().or_default()` compiles. One event represents the rule of the three read spots (next-generation re-subscription · expiry marker · refusal marker) (`known_epoch`).
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
        "a read site does not create entries — empty entries of vanished agents would pile up for the whole connection lifetime"
    );
}

// ── The junction from event to window (the branching the select! arm held inline) ─────────────
// This suite's reason to exist: with the refusal arm inline, deleting it left every gate green. The three tests catch that junction (event recognition → slot release → marker to the window → next-generation Subscribe).
#[test]
fn a_refusal_delivers_a_failure_marker_to_the_windows_showing_that_agent() {
    let mut fs = flight();
    let mut subs = Subs::new();
    let agent = AgentId::new_v4();
    let router = router_showing(agent);
    let now = Instant::now();

    let first = fs.request_replay(agent, now);
    assert!(
        first.send_now,
        "precondition: this generation went out on the wire"
    );

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

    assert_eq!(out.0.len(), 1, "one refusal = one marker");
    let (labels, frame) = &out.0[0];
    assert_eq!(
        labels.as_slice(),
        &[MAIN_WINDOW_LABEL.to_string()],
        "goes to the windows where that agent is visible (router.targets)"
    );
    let (got_agent, _epoch, marker) = decode_marker(frame);
    assert_eq!(got_agent, agent, "the marker is addressed to its own agent");
    assert_eq!(
        marker.generation, first.generation,
        "what is released is that generation"
    );
    assert!(
        marker.failed,
        "refusal = failure marker (so the view does not wait 10 s)"
    );
    assert!(
        matches!(follow, ReplayFollowUp::Handled(None)),
        "no queue means no next Subscribe"
    );

    assert!(
        fs.request_replay(agent, now).send_now,
        "the slot is released and a new Subscribe goes out again (this is the heart of the defect)"
    );
}

// The merged generation must come out as a command to go out on the wire — else the slot is released but no one asks again, the zombie symptom.
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
    assert!(
        ack_out.0.is_empty(),
        "the Ack itself sends nothing to the window"
    );
    assert!(matches!(ack_follow, ReplayFollowUp::Handled(None)));

    // That Ack was from a time when nothing was in-flight, so it does not create a slot — now issue the two real requests.
    let first = fs.request_replay(agent, now);
    let waiter = fs.request_replay(agent, now);
    assert!(!waiter.send_now, "in flight, so merged");

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
    assert_eq!(
        marker.generation, first.generation,
        "what is released is the earlier generation"
    );
    assert_eq!(
        epoch, 7,
        "carries the last known epoch (not an authoritative value)"
    );
    assert!(
        !marker.continues_conversation,
        "the earlier Ack (flag true) sat in no slot and the refused slot never received an Ack — bit2 is 0"
    );

    match follow {
        ReplayFollowUp::Handled(Some(AgentCommand::Subscribe {
            agent_id,
            epoch,
            after_seq,
        })) => {
            assert_eq!(agent_id, agent);
            assert_eq!(
                epoch,
                Some(7),
                "re-subscribes with the currently known epoch"
            );
            assert_eq!(
                after_seq, None,
                "a re-replay is always in full (from-oldest)"
            );
        }
        other => panic!("the merged generation's Subscribe must come out: {other:?}"),
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

    assert_eq!(out.0.len(), 1, "one completion = one marker");
    let (labels, frame) = &out.0[0];
    assert_eq!(labels.as_slice(), &[MAIN_WINDOW_LABEL.to_string()]);
    let (_, epoch, marker) = decode_marker(frame);
    assert_eq!(epoch, 3, "carries the completed replay's epoch");
    assert_eq!(marker.generation, first.generation);
    assert!(!marker.failed, "completion = success marker");
    assert!(
        marker.truncated,
        "the Ack's truncated propagates to the marker"
    );
    assert!(
        !marker.continues_conversation,
        "the Ack said it is not a continuing incarnation"
    );
    assert!(matches!(follow, ReplayFollowUp::Handled(None)), "no queue");
}

// The Ack's resume marker reaches bit2 of the bytes going to the window (ADR-0226) — the webview uses this bit to draw loading instead of the first screen. truncated · the marker value are flipped relative to the test right above, so a swapped wiring turns one of the two red.
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

    assert_eq!(out.0.len(), 1, "one completion = one marker");
    let (labels, frame) = &out.0[0];
    assert_eq!(labels.as_slice(), &[MAIN_WINDOW_LABEL.to_string()]);
    assert_eq!(frame.len(), MARKER_FRAME_LEN, "length stays 38");
    assert_eq!(frame[29], 0b0000_0100, "success + continuation = bit2 only");
    let (_, _, marker) = decode_marker(frame);
    assert!(marker.continues_conversation);
    assert!(!marker.truncated && !marker.failed);
}

// The Ack's replay head reaches the marker bytes going to the window (ADR-0231) — the old wiring threw this field away with the pattern `..`. The view starts the success flush with it (`max(last+1, replay_from)`), so missing, a seq 0 arriving late after the marker would be skipped in a new incarnation with an empty replay. A nonzero value blocks the shape where "0 even if not carried" happens to match.
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

    assert_eq!(out.0.len(), 1, "one completion = one marker");
    let (_, _, marker) = decode_marker(&out.0[0].1);
    assert!(!marker.failed);
    assert_eq!(
        marker.replay_from, 40,
        "the Ack's replay head reaches the marker"
    );
}

// Events outside the replay family must not be swallowed by this function — swallowed, inbound commands and broadcast would vanish entirely (their only entrance is this function's `NotHandled` branch).
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
    assert!(out.0.is_empty(), "sends nothing to the window");
    assert!(matches!(follow, ReplayFollowUp::NotHandled));
}
