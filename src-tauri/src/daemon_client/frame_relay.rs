//! Relay decision for one daemon output binary frame — decode → progress signal → incarnation marker filter → pass to the viewing window.
//!
//! ## ★Do not skip seq (ADR-0231, load-bearing)★
//! In live, a view holds `seq > 마지막+1` (seq > last+1) with no deadline and releases only when `마지막+1` (last+1) arrives. So if this relay
//! silently discards one frame, the frames after it pile up to the hold cap — forever for an idle agent — and that view stalls. A frame whose decoding
//! was rejected is split in two:
//! - **Unknown tag (`UnknownTag`) → make a placeholder and flow it on the usual path.** The agent id · incarnation marker · seq are at fixed positions
//!   independent of the tag, so they can be read as long as the header length is present — from those fields it makes the tag1 `Error` placeholder **the same as the daemon sink's**
//!   (`engram_dashboard_protocol::placeholder_error_frame`). It is one error row in that agent's chat view and
//!   other agents · connections are untouched. ★Why it does not disconnect★ — the cause is usually deterministic, so disconnecting makes the full replay bring the same
//!   frame again, and every view of that shell repeats disconnect · replay endlessly.
//! - **Short frame (`TooShort`) → disconnect.** seq cannot be read so a placeholder cannot be made. The re-attached views
//!   come back through a full replay. All of the daemon's binary output frames pass through an encoder that writes the header first, so this shape arises only from a
//!   producer that bypassed that encoder, and today there is no such producer. If deterministic, reconnects repeat — we chose the visible side over a silent
//!   stall (no count · deadline cap is set, since it would be a magic number that decides the answer — ADR-0038).
//!
//! Other filters (a different incarnation marker · no viewing window) do not make a hole in the seq stream that view will receive — a view whose incarnation changed
//! asks for a full replay on re-attach, and a window that is not viewing asks for replay when it attaches.
//!
//! ## ★Why it was detached from the connection loop★
//! This decision lived inside the connection task's `select!` and that place requires a real socket. Lowered here, it can be driven
//! one frame at a time without a socket · Tauri (the tests below). The reason `deliver` is an argument is the same as `connection::apply_replay_event` — the real
//! delivery port requires `tauri::ipc::Channel`, but **to which window · with what bytes** it sends is decided here.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use engram_dashboard_protocol::{
    decode_frame, peek_frame_header, placeholder_error_frame, AgentId, CodecError,
};

use super::protocol_state::{self, EpochDecision, SubState};
use super::replay_flight::ReplayFlightSet;
use crate::output_router::{OutputRouter, WindowLabel};

/// The outcome of [`relay_binary_frame`] — what the connection loop does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameRelay {
    /// Handled (one of pass · placeholder · filtered). The loop keeps running.
    Continue,
    /// Disconnect — the loop exits with `Disconnected` and becomes a target of backoff reconnection.
    Disconnect,
}

/// The (agent, unknown tag) pairs already warned on this connection — if the cause is deterministic the same tag comes in every frame, so
/// only the first is warned and the rest are lowered to debug. The connection loop makes it afresh per connection and lends it as `&mut` (after a reconnect
/// it warns once again).
#[derive(Debug, Default)]
pub struct UnknownTagLog {
    warned: HashSet<(AgentId, u8)>,
}

impl UnknownTagLog {
    // Cap — when exceeded, clear and count again. The pairs cannot exceed (number of agents × 256) during one connection, but the number of agents
    //   is unbounded, so the cap is kept. Clearing only makes the warning appear again; it does not hide anything.
    const CAP: usize = 4096;

    /// `true` if this pair is seen for the first time (= time to warn).
    pub fn first_sighting(&mut self, agent: AgentId, tag: u8) -> bool {
        if self.warned.len() >= Self::CAP && !self.warned.contains(&(agent, tag)) {
            self.warned.clear();
        }
        self.warned.insert((agent, tag))
    }
}

/// Decide on one binary frame sent by the daemon and hand the passing ones to `deliver`.
// ADR-0231
pub fn relay_binary_frame(
    bytes: &[u8],
    flight: &mut ReplayFlightSet,
    subs: &mut HashMap<AgentId, SubState>,
    router: &OutputRouter,
    unknown_tags: &mut UnknownTagLog,
    now: Instant,
    deliver: &mut dyn FnMut(&[WindowLabel], &[u8]),
) -> FrameRelay {
    let placeholder;
    let (agent_id, epoch, out): (AgentId, u32, &[u8]) = match decode_frame(bytes) {
        Ok(frame) => (frame.agent_id, frame.epoch, bytes),
        Err(CodecError::UnknownTag(tag)) => {
            // `decode_frame` rejects the tag only after checking the header length, so the header can always be read.
            let Ok(header) = peek_frame_header(bytes) else {
                return FrameRelay::Disconnect;
            };
            if unknown_tags.first_sighting(header.agent_id, tag) {
                tracing::warn!(
                    agent = %header.agent_id,
                    seq = header.seq,
                    tag,
                    "모르는 tag 의 출력 프레임 — 같은 seq 의 자리채움으로 대신 흘린다(이 짝의 다음부터는 debug)"
                );
            } else {
                tracing::debug!(
                    agent = %header.agent_id,
                    seq = header.seq,
                    tag,
                    "모르는 tag 의 출력 프레임 — 자리채움으로 대신 흘린다"
                );
            }
            placeholder = placeholder_error_frame(header.agent_id, header.epoch, header.seq);
            (header.agent_id, header.epoch, &placeholder)
        }
        Err(CodecError::TooShort { len }) => {
            tracing::warn!(
                len,
                "헤더보다 짧은 출력 프레임 — seq 를 못 읽어 연결을 끊는다"
            );
            return FrameRelay::Disconnect;
        }
    };
    // ★Progress signal (deadline reset)★: when that agent's frame arrives the replay is alive and in progress → reset the single-flight deadline
    //   (no false positives on a healthy-slow replay). Reset before the epoch filter — even a stale frame is a signal that the daemon is alive.
    flight.note_progress(agent_id, now);
    // ★epoch filter (ADR-0046 T5)★: if it mismatches SubState.epoch (updated by SubscribeAck) (= a leftover of an old session), drop
    //   before passing. If epoch is None (before the first Ack), pass (prevents losing early output — an internal convention of decide_epoch). A placeholder passes
    //   the same filter.
    let st = subs.entry(agent_id).or_default();
    if protocol_state::decide_epoch(st, epoch) == EpochDecision::DropEpochMismatch {
        return FrameRelay::Continue;
    }
    // ★Pass by targets∩registered★: router.targets is a hot path with 0 locks (ArcSwap). If no window is viewing (labels empty), the
    //   delivery port returns early, and an unregistered label is skipped inside it. A normal frame goes out with its original bytes unchanged.
    deliver(&router.targets(agent_id), out);
    FrameRelay::Continue
}

#[cfg(test)]
mod tests {
    //! The TRD §7-1 「셸 중계」 row — short frame = disconnect · unknown tag = a same-seq placeholder passes · normal pass ·
    //! filtering of a different incarnation marker (placeholder included).
    use std::time::Duration;

    use engram_dashboard_protocol::{
        decode_frame, encode_structured_frame, encode_terminal_frame, placeholder_error_frame,
        FRAME_HEADER_LEN, FRAME_TAG_STRUCTURED_EVENT,
    };

    use super::*;
    use crate::layout::{tree, ViewManager, MAIN_WINDOW_LABEL};

    // A real routing table — if the test invents the destination, it ends up verifying the invented value.
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

    struct Harness {
        flight: ReplayFlightSet,
        subs: HashMap<AgentId, SubState>,
        router: OutputRouter,
        unknown_tags: UnknownTagLog,
        delivered: Vec<(Vec<WindowLabel>, Vec<u8>)>,
    }

    impl Harness {
        fn new(agent: AgentId) -> Self {
            Self {
                flight: ReplayFlightSet::new(Duration::from_secs(10)),
                subs: HashMap::new(),
                router: router_showing(agent),
                unknown_tags: UnknownTagLog::default(),
                delivered: Vec::new(),
            }
        }

        fn feed(&mut self, bytes: &[u8]) -> FrameRelay {
            let delivered = &mut self.delivered;
            let mut deliver = |labels: &[WindowLabel], b: &[u8]| {
                delivered.push((labels.to_vec(), b.to_vec()));
            };
            relay_binary_frame(
                bytes,
                &mut self.flight,
                &mut self.subs,
                &self.router,
                &mut self.unknown_tags,
                Instant::now(),
                &mut deliver,
            )
        }

        fn delivered_seqs(&self) -> Vec<u64> {
            self.delivered
                .iter()
                .map(|(_, b)| decode_frame(b).expect("통과분은 정상 프레임").seq)
                .collect()
        }
    }

    fn with_tag(mut frame: Vec<u8>, tag: u8) -> Vec<u8> {
        frame[0] = tag;
        frame
    }

    #[test]
    fn a_normal_frame_passes_through_unchanged() {
        let agent = AgentId::new_v4();
        let mut h = Harness::new(agent);
        let frame = encode_terminal_frame(agent, 3, 5, b"hi");
        assert_eq!(h.feed(&frame), FrameRelay::Continue);
        assert_eq!(h.delivered.len(), 1);
        assert_eq!(h.delivered[0].0, vec![MAIN_WINDOW_LABEL.to_string()]);
        assert_eq!(h.delivered[0].1, frame, "원본 bytes 그대로");
    }

    #[test]
    fn a_short_frame_disconnects() {
        let agent = AgentId::new_v4();
        let mut h = Harness::new(agent);
        let frame = encode_terminal_frame(agent, 3, 5, b"");
        assert_eq!(
            h.feed(&frame[..FRAME_HEADER_LEN - 1]),
            FrameRelay::Disconnect,
            "seq 를 못 읽으면 연결을 끊는다"
        );
        assert!(h.delivered.is_empty());
    }

    // ★Key★ — a same-seq placeholder stands in the place of the unknown-tag frame, so there is no hole in the view's seq stream.
    #[test]
    fn an_unknown_tag_frame_becomes_a_placeholder_at_the_same_seq() {
        let agent = AgentId::new_v4();
        let mut h = Harness::new(agent);
        assert_eq!(
            h.feed(&encode_terminal_frame(agent, 3, 5, b"a")),
            FrameRelay::Continue
        );
        assert_eq!(
            h.feed(&with_tag(encode_structured_frame(agent, 3, 6, b"{}"), 0x07)),
            FrameRelay::Continue,
            "끊지 않는다 — 결정적 원인이면 끊김·replay 가 끝없이 되풀이된다"
        );
        assert_eq!(
            h.feed(&encode_terminal_frame(agent, 3, 7, b"b")),
            FrameRelay::Continue
        );

        assert_eq!(h.delivered_seqs(), vec![5, 6, 7], "구멍 없는 seq 줄기");
        let (labels, bytes) = &h.delivered[1];
        assert_eq!(labels, &vec![MAIN_WINDOW_LABEL.to_string()], "보는 창으로");
        assert_eq!(decode_frame(bytes).unwrap().tag, FRAME_TAG_STRUCTURED_EVENT);
        assert_eq!(
            bytes,
            &placeholder_error_frame(agent, 3, 6),
            "데몬 sink 의 자리채움과 바이트가 같다(한 벌)"
        );
    }

    #[test]
    fn a_stale_epoch_is_dropped_without_disconnecting_placeholder_included() {
        let agent = AgentId::new_v4();
        let mut h = Harness::new(agent);
        // Current incarnation marker = 3 (what SubscribeAck stamped).
        protocol_state::apply_subscribe_ack(h.subs.entry(agent).or_default(), 3);

        assert_eq!(
            h.feed(&encode_terminal_frame(agent, 2, 5, b"old")),
            FrameRelay::Continue
        );
        assert_eq!(
            h.feed(&with_tag(encode_terminal_frame(agent, 2, 6, b"old"), 0x09)),
            FrameRelay::Continue,
            "옛 화신의 모르는 tag 도 끊지 않고 거른다"
        );
        assert!(h.delivered.is_empty(), "옛 화신은 자리채움까지 거른다");

        assert_eq!(
            h.feed(&with_tag(encode_terminal_frame(agent, 3, 7, b"new"), 0x09)),
            FrameRelay::Continue
        );
        assert_eq!(h.delivered_seqs(), vec![7], "현 화신의 자리채움은 통과");
    }

    // An unknown tag is also a progress signal — it is evidence that the daemon is alive and flowing that agent's replay, so it postpones the no-progress expiry
    //   (the placeholder passes through the usual path as is).
    #[test]
    fn an_unknown_tag_frame_still_counts_as_replay_progress() {
        let agent = AgentId::new_v4();
        let mut h = Harness::new(agent);
        h.flight = ReplayFlightSet::new(Duration::from_millis(100));
        let start = Instant::now();
        h.flight.request_replay(agent, start);

        let mut deliver = |_: &[WindowLabel], _: &[u8]| {};
        relay_binary_frame(
            &with_tag(encode_terminal_frame(agent, 3, 0, b""), 0x07),
            &mut h.flight,
            &mut h.subs,
            &h.router,
            &mut h.unknown_tags,
            start + Duration::from_millis(80),
            &mut deliver,
        );
        assert!(
            h.flight
                .check_deadlines(start + Duration::from_millis(150))
                .is_empty(),
            "80ms 의 진행 신호로 만료가 180ms 로 밀렸다"
        );
    }

    // The same (agent, tag) is warned only once per connection — since with a deterministic cause it comes with every frame.
    #[test]
    fn an_unknown_tag_warns_once_per_agent_and_tag() {
        let (a, b) = (AgentId::new_v4(), AgentId::new_v4());
        let mut log = UnknownTagLog::default();
        assert!(log.first_sighting(a, 7), "첫 번째는 warn");
        assert!(!log.first_sighting(a, 7), "같은 짝의 두 번째는 debug");
        assert!(log.first_sighting(a, 9), "다른 tag 는 따로 센다");
        assert!(log.first_sighting(b, 7), "다른 에이전트는 따로 센다");
    }

    #[test]
    fn repeated_unknown_tag_frames_still_each_become_placeholders() {
        let agent = AgentId::new_v4();
        let mut h = Harness::new(agent);
        for seq in 5..8 {
            assert_eq!(
                h.feed(&with_tag(encode_terminal_frame(agent, 3, seq, b""), 0x07)),
                FrameRelay::Continue
            );
        }
        assert_eq!(
            h.delivered_seqs(),
            vec![5, 6, 7],
            "경고를 줄여도 자리채움은 매번"
        );
        assert!(
            !h.unknown_tags.first_sighting(agent, 0x07),
            "첫 장에서 이미 셌다"
        );
    }
}
