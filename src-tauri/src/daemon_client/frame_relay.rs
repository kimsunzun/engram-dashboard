//! 데몬 출력 binary frame 한 장의 중계 판정 — 디코드 → 진행 신호 → 화신 표식 거름 → 보는 창으로 통과.
//!
//! ## ★seq 를 건너뛰지 않는다(ADR-0231, load-bearing)★
//! 뷰는 live 에서 `seq > 마지막+1` 을 시한 없이 붙들고 `마지막+1` 이 와야 흘린다. 그래서 이 중계가 한 프레임을
//! 말없이 버리면 그 뒤 프레임이 붙듦 상한까지 쌓이고 — 한가한 에이전트면 영영 — 그 뷰가 멈춘다. 해독이
//! 거절된 프레임은 둘로 가른다:
//! - **모르는 tag(`UnknownTag`) → 자리채움을 만들어 평소 경로로 흘린다.** agent id · 화신 표식 · seq 는 tag
//!   와 무관한 고정 자리라 헤더 길이만 되면 읽힌다 — 그 칸으로 **데몬 sink 와 같은** tag1 `Error` 자리채움
//!   (`engram_dashboard_protocol::placeholder_error_frame`)을 만든다. 그 에이전트의 챗 뷰에 오류 행 하나이고
//!   다른 에이전트·연결은 그대로다. ★끊지 않는 이유★ — 원인은 대개 결정적이라, 끊으면 전량 replay 가 같은
//!   프레임을 다시 실어 와 그 셸의 모든 뷰가 끊김·replay 를 끝없이 되풀이한다.
//! - **짧은 프레임(`TooShort`) → 연결을 끊는다.** seq 를 읽을 수 없어 자리채움을 만들 수 없다. 다시 붙은 뷰들은
//!   전량 replay 로 돌아온다. 데몬의 이진 출력 프레임은 전부 헤더를 먼저 쓰는 인코더를 지나므로 이 모양은 그
//!   인코더를 우회한 생산자에서만 나고, 오늘 그런 생산자는 없다. 결정적이면 재연결이 되풀이된다 — 조용한
//!   멈춤 대신 드러나는 쪽을 택했다(횟수·시한 상한은 정답을 가르는 매직 넘버라 두지 않는다 — ADR-0038).
//!
//! 그 밖의 거름(다른 화신 표식 · 보는 창 없음)은 그 뷰가 받을 seq 줄기에 구멍을 내지 않는다 — 화신이 바뀐
//! 뷰는 재부착이 전량 replay 를 청하고, 안 보는 창은 붙을 때 replay 를 청한다.
//!
//! ## ★연결 루프 밖으로 뗀 이유★
//! 이 판정은 연결 태스크의 `select!` 안에 살았고 그 자리는 실 소켓을 요구한다. 여기로 내리면 소켓·Tauri 없이
//! 한 장씩 태울 수 있다(아래 시험). `deliver` 가 인자인 이유는 `connection::apply_replay_event` 와 같다 — 실
//! 배달구는 `tauri::ipc::Channel` 을 요구하지만 **어느 창으로 · 무슨 바이트를** 보내는지는 여기서 정해진다.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use engram_dashboard_protocol::{
    decode_frame, peek_frame_header, placeholder_error_frame, AgentId, CodecError,
};

use super::protocol_state::{self, EpochDecision, SubState};
use super::replay_flight::ReplayFlightSet;
use crate::output_router::{OutputRouter, WindowLabel};

/// [`relay_binary_frame`] 의 결말 — 연결 루프가 무엇을 할지.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameRelay {
    /// 처리했다(통과 · 자리채움 · 거름 중 하나). 루프는 계속 돈다.
    Continue,
    /// 연결을 끊는다 — 루프가 `Disconnected` 로 빠져 백오프 재연결 대상이 된다.
    Disconnect,
}

/// 이 연결에서 이미 warn 한 (에이전트, 모르는 tag) 짝 — 원인이 결정적이면 같은 tag 가 프레임마다 오므로
/// 첫 번째만 warn 하고 나머지는 debug 로 내린다. 연결 루프가 연결마다 새로 만들어 `&mut` 로 빌려준다(재연결
/// 뒤엔 다시 한 번 warn 한다).
#[derive(Debug, Default)]
pub struct UnknownTagLog {
    warned: HashSet<(AgentId, u8)>,
}

impl UnknownTagLog {
    // 상한 — 넘으면 비우고 다시 센다. 짝은 연결 하나 동안 (에이전트 수 × 256) 을 못 넘지만 에이전트 수에
    //   바닥이 없어 둔다. 비우면 경고가 다시 날 뿐 숨지 않는다.
    const CAP: usize = 4096;

    /// 이 짝을 처음 보면 `true`(= warn 할 차례).
    pub fn first_sighting(&mut self, agent: AgentId, tag: u8) -> bool {
        if self.warned.len() >= Self::CAP && !self.warned.contains(&(agent, tag)) {
            self.warned.clear();
        }
        self.warned.insert((agent, tag))
    }
}

/// 데몬이 보낸 binary frame 한 장을 판정하고, 통과분을 `deliver` 로 넘긴다.
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
            // `decode_frame` 은 헤더 길이를 본 뒤에야 tag 를 거절하므로 헤더는 늘 읽힌다.
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
    // ★진행 신호(deadline 리셋)★: 그 agent 의 frame 이 오면 replay 가 살아 진행 중 → single-flight deadline
    //   리셋(healthy-slow replay 무오탐). epoch 필터 전에 리셋한다 — stale frame 이어도 데몬이 살아있다는 신호.
    flight.note_progress(agent_id, now);
    // ★epoch 필터(ADR-0046 T5)★: SubState.epoch(SubscribeAck 로 갱신)와 불일치(=옛 세션 잔여)면 통과 전
    //   drop. epoch None(첫 Ack 전)이면 통과(초반 출력 유실 방지 — decide_epoch 내부 규약). 자리채움도 같은
    //   거름을 지난다.
    let st = subs.entry(agent_id).or_default();
    if protocol_state::decide_epoch(st, epoch) == EpochDecision::DropEpochMismatch {
        return FrameRelay::Continue;
    }
    // ★targets∩registered 로 통과★: router.targets 는 핫패스 락 0(ArcSwap). 어느 창도 안 보면(labels 비면)
    //   배달구가 early return, 미등록 label 은 그 안에서 skip. 정상 프레임은 원본 bytes 그대로 나간다.
    deliver(&router.targets(agent_id), out);
    FrameRelay::Continue
}

#[cfg(test)]
mod tests {
    //! TRD §7-1 「셸 중계」 행 — 짧은 프레임 = 끊는다 · 모르는 tag = 같은 seq 의 자리채움 통과 · 정상 통과 ·
    //! 다른 화신 표식 거름(자리채움 포함).
    use std::time::Duration;

    use engram_dashboard_protocol::{
        decode_frame, encode_structured_frame, encode_terminal_frame, placeholder_error_frame,
        FRAME_HEADER_LEN, FRAME_TAG_STRUCTURED_EVENT,
    };

    use super::*;
    use crate::layout::{tree, ViewManager, MAIN_WINDOW_LABEL};

    // 실 라우팅 표 — 목적지를 테스트가 지어내면 지어낸 값을 검증하는 꼴이 된다.
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

    // ★핵심★ — 모르는 tag 의 프레임 자리에 같은 seq 의 자리채움이 서서 뷰의 seq 줄기에 구멍이 없다.
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
        // 현 화신 표식 = 3(SubscribeAck 가 각인한 것).
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

    // 모르는 tag 도 진행 신호다 — 데몬이 살아 그 에이전트의 replay 를 흘리고 있다는 증거라 무진행 만료를
    //   미룬다(자리채움은 평소 경로를 그대로 지난다).
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

    // 같은 (에이전트, tag) 는 한 연결에 한 번만 warn — 결정적 원인이면 프레임마다 오므로.
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
