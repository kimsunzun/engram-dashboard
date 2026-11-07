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

// ★Progress-based replay deadline (ADR-0046)★. If a single-flight in-flight makes zero progress (frame/Ack) for this long,
// it is terminated with a failure marker (paths where Ack/Complete never come at all because the agent vanished or subscribe failed). A healthy-slow replay
// (debug build · contention) is reset by every frame/Ack and never trips (progress-based). Around 10s in production.
pub(super) const REPLAY_DEADLINE: Duration = Duration::from_secs(10);

// The deadline sweep period — the granularity at which expired in-flight replays are swept (the driver arms it only
// while a replay waits).
pub(super) const REPLAY_DEADLINE_TICK: Duration = Duration::from_secs(1);

/// Outcome of [`apply_replay_event`].
#[derive(Debug)]
pub enum ReplayFollowUp {
    /// Not a replay-family event — the caller passes it on to its own branch (inbound command / broadcast).
    NotHandled,
    /// Handled. `Some(cmd)` = the **next-generation `Subscribe`** that must go out on the wire now (the merged request).
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
        // ★Subscription ack★: update SubState.epoch (basis of the binary arm's decide_epoch) + move in-flight to acked +
        //   remember truncated · continues_conversation (ADR-0226) · the replay head `replay_from` (ADR-0231) (propagated to the success
        //   marker) + progress (deadline reset).
        //   ★ADR-0046: no buffer/cursor reset★ —
        //   epoch-transition resubscribe is handled by the frontend's authoritative roster observation (observeRoster) (ADR-0164 decision 8) —
        //   the subscription deps `[viewId, agentId]` deliberately exclude the incarnation marker.
        // ★The returned bool (epoch_changed) is deliberately ignored★: the old wiring used this value to reset the window's render_seq, but with the mirror
        //   buffer removed (ADR-0046) no progress state is left in src-tauri → there is nothing to reset. Adopting the epoch is done by the frontend
        //   with the success marker's epoch (gen fence).
        // ★Do not drop `replay_from` (ADR-0231)★: the view starts the success flush at `max(마지막+1, replay_from)`.
        //   Using the buffer's minimum seq instead skips a seq 0 that arrives late after the marker in an empty replay (new incarnation).
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
        // ★Replay boundary stamp (ADR-0046 M1)★: resolve the acked in-flight with a success marker (a Complete that arrives before Ack =
        //   orphan of the previous generation → Ignore). If there is a queue, return the merged next Subscribe to the caller.
        AgentEvent::ReplayComplete { agent_id, epoch } => {
            match flight.on_complete(*agent_id, now) {
                Resolution::Ignore => ReplayFollowUp::Handled(None),
                Resolution::Emit { marker, send_next } => {
                    let frame = replay_flight::encode_marker_frame(*agent_id, *epoch, marker);
                    deliver(&router.targets(*agent_id), &frame);
                    // The marker's epoch (= that of the replay that just ended) and the next resubscribe's epoch are separate — the next replay
                    //   resubscribes in full with the **currently known** epoch.
                    ReplayFollowUp::Handled(send_next.then(|| AgentCommand::Subscribe {
                        agent_id: *agent_id,
                        epoch: known_epoch(subs, *agent_id),
                        after_seq: None,
                    }))
                }
            }
        }
        // ★Refusal = the trigger to release the single-flight slot★. A refused Subscribe is followed by neither Ack nor Complete
        //   (the daemon's `handle_subscribe` returns before emitting Ack), so releasing the slot now leaves no response to be misattributed —
        //   this difference is why expiry (zombie) release was not chosen (the authoritative rationale = `replay_flight::check_deadlines`).
        //   ★What happens if it is not released★: that agent's Subscribe can never go out again, so even after a re-spawn output stays cut off in every
        //   window permanently (measured 2026-08-19).
        // ★Keep the warn★: the refusal has left `Error`, so the error arm of `emit_broadcast` can no longer catch it. This
        //   line carries on that diagnostic (the reason sentence is what the daemon wrote).
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

/// The **last known** epoch for that agent (`None` if unknown).
///
/// ★A read does not create an entry — this is the whole growth rule of this map★: there are only two places that **insert** an entry into `subs` —
/// the `SubscribeAck` stamp and the epoch gate of the binary frame. Both hold evidence that "that agent exists".
/// The reading spots (next-generation resubscribe · expiry marker · refusal marker) have no such evidence — using
/// `entry().or_default()` there would pile up empty entries of vanished agents over the whole connection lifetime. All reading spots are routed
/// through this function so the rule does not diverge by site.
pub(super) fn known_epoch(subs: &HashMap<AgentId, SubState>, agent_id: AgentId) -> Option<u32> {
    subs.get(&agent_id).and_then(|st| st.epoch)
}

/// What one refusal produces — the failure marker frame for the windows and the next generation's `Subscribe` for the
/// wire. The caller (the connection driver) sends them.
#[derive(Debug)]
pub struct RefusalPlan {
    /// tag=255 marker frame. `None` = there was no pool slot, or this generation's failure marker already went out once (no duplicates).
    pub marker_frame: Option<Vec<u8>>,
    /// `Subscribe` of the merged queue. `None` = no queue.
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
            // The failure marker cuts the view's wait now (otherwise it would wait for the 10-second deadline).
            marker_frame: marker
                .map(|m| replay_flight::encode_marker_frame(agent_id, epoch.unwrap_or(0), m)),
            // The next replay resubscribes with the **currently known epoch** (separate from the marker's epoch).
            next_subscribe: send_next.then_some(AgentCommand::Subscribe {
                agent_id,
                epoch,
                after_seq: None,
            }),
        },
    }
}
