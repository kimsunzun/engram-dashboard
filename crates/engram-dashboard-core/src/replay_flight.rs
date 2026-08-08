//! single-flight replay 채번/펜스 상태기계 + replay 경계 마커 인코딩 (ADR-0046 M1).
//!
//! ## 무엇 / 왜 (load-bearing)
//! src-tauri 는 미러 버퍼를 버리고(ADR-0046) 무상태 통과 라우터가 됐다. remount/리로드/새 창은 데몬 ring
//! 을 **전량 재replay** 받고, 그 replay 의 경계(끝)를 뷰가 알 수 있게 **에이전트당 single-flight** 로
//! wire `Subscribe`↔`ReplayComplete` 를 1:1 대응시켜 `gen`(세대)을 각인한다. 뷰는 자기 `requestReplay` 가
//! 돌려받은 gen **이상**의 성공 마커에만 sort+dedup flush 한다(gen 펜스 — 남의/구세대 replay 조기 flush 차단).
//!
//! ## ★셈 기반 결합 금지(재검증 실증)★
//! gen↔Complete 를 카운팅/마지막값 각인으로 추론하면 desync 한다(마지막값=조기 flush 유실, FIFO=Complete
//! 누락 실패 경로 영구 desync). 그래서 **동시 in-flight 는 정확히 1개**로 강제한다 — in-flight 중 도착한
//! 요청은 전부 "다음 1회 Subscribe" 로 병합(coalesce)하고, 그 Subscribe 는 현 in-flight 가 해소될 때 보낸다.
//!
//! ## ★수명·불변식(TRD rev4 §2 + FIX round — zombie 의미론)★
//! - in_flight 수명 = sent → (SubscribeAck) → acked → (ReplayComplete) → 성공 마커. **슬롯의 유일한 해제
//!   경로 = resolution(Ack 뒤 Complete) 또는 disconnect** — deadline 초과는 슬롯을 해제하지 않는다.
//! - 같은 gen 에 실패 마커 뒤 성공 마커가 붙어도 안전하다: 재요청한 뷰는 더 높은 myGen 을 들고 이걸
//!   무시하고, 아직 대기 중인 뷰는 완전한 버퍼를 flush 한다(뷰는 실패 마커에 버퍼를 유지한다). 진행 기반
//!   deadline 아래 late-resolution 은 empty/near-empty replay 를 함의하고, 흘렀던 frame 은 뷰가 이미 버퍼했다.
//! - **agent-gone(데몬 Error 만 오고 Ack/Complete 영영 안 옴):** 슬롯은 disconnect 까지 좀비로 남는다. 실패
//!   마커는 최초 만료 때 이미 나갔고, UX 는 뷰의 bounded 재요청 사다리 + agent-list teardown 이 처리한다(수용).
//!
//! ## ★순수성(테스트 격리 — ADR-0012/0003)★
//! 소켓·tokio·Tauri·protocol 의존 0 — core crate 에 산다(agentId 는 `uuid::Uuid`, src-tauri 의 `AgentId` 는
//! 그 alias 라 통과 전달). 시간은 `Instant` 를 인자로 받아 결정론 단위테스트가 가능하다(부작용=마커 실제
//! 송신·wire Subscribe 는 호출자=연결 task 가 수행). 이 위치 덕에 단위테스트가 `cargo test
//! -p engram-dashboard-core`(WebView2 DLL 없는 headless)에서 **실행**된다(src-tauri 테스트는 이 환경 미실행).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use uuid::Uuid;

/// 에이전트 식별자(core 는 protocol 무의존 — `AgentId = uuid::Uuid` alias 를 그대로 받는다, ADR-0003).
type AgentId = Uuid;

/// replay 경계 마커의 wire tag. 데몬 codec(tag0/tag1)엔 없는 **src-tauri↔웹뷰 Channel 내부 계약** 값 —
/// 프론트 decodeOutputFrame 이 미지 tag 로 조용히 skip(전방 호환, M0)하고 M2 가 정식 소비한다.
pub const MARKER_TAG: u8 = 255;

/// 마커 프레임 총 길이 = tag(1) + agentId(16) + epoch(4) + gen(8) + flags(1) = 30바이트.
pub const MARKER_FRAME_LEN: usize = 1 + 16 + 4 + 8 + 1;

/// replay 경계 마커의 논리 내용(gen 펜스 + 플래그). agentId·epoch 는 인코딩 시 붙인다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marker {
    pub generation: u64,
    /// 데몬이 ring 하한 초과 과거를 잘랐음(SubscribeAck.truncated 전파) — 뷰가 경고 표면화.
    pub truncated: bool,
    /// 실패 종결(진행 기반 deadline 초과) — 뷰는 flush 금지, 재요청 사다리(M2).
    pub failed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestOutcome {
    /// 이 요청에 배정된 세대 — 호출자가 프론트로 반환한다.
    pub generation: u64,
    /// `true` = 지금 wire `Subscribe{after_seq:None}` 를 보낸다(idle 이라 즉시 발사).
    /// `false` = in-flight 중이라 병합됨(Subscribe 는 현 in-flight 해소 시 [`Resolution::send_next`] 로 발사).
    pub send_now: bool,
}

/// [`ReplayFlightSet::on_complete`] 결정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Ack 전 도착 Complete(전대 고아) 또는 in_flight 없는 stray.
    Ignore,
    Emit {
        marker: Marker,
        /// `true` = 병합된 다음 요청의 Subscribe 를 지금 보낸다(현 in-flight 를 그 gen 으로 교체 완료).
        send_next: bool,
    },
}

/// 한 에이전트의 single-flight 부기. `gen_counter` 는 절대 리셋 안 한다(단조 — 구세대 마커 오인 방지).
#[derive(Default)]
struct AgentFlight {
    gen_counter: u64,
    in_flight: Option<InFlight>,
    /// in-flight 중 도착한 요청들이 공유하는 "다음 1회 Subscribe" 의 gen. `Some` ⟹ in_flight `Some`(불변식).
    next_gen: Option<u64>,
}

struct InFlight {
    generation: u64,
    acked: bool,
    truncated: bool,
    /// ★좀비 플래그(FIX-1)★: 실패 마커를 이미 1회 발행했음(재발행 금지 표시). 근거는
    ///   [`ReplayFlightSet::check_deadlines`].
    failed: bool,
    /// 진행 기반 deadline — frame/Ack 마다 갱신.
    deadline: Instant,
}

impl InFlight {
    fn fresh(generation: u64, now: Instant, deadline: Duration) -> Self {
        InFlight {
            generation,
            acked: false,
            truncated: false,
            failed: false,
            deadline: now + deadline,
        }
    }
}

/// 연결 task(actor)가 `&mut` 로 소유해 직렬 조작한다 — 내부 락 없음.
pub struct ReplayFlightSet {
    agents: HashMap<AgentId, AgentFlight>,
    deadline: Duration,
}

impl ReplayFlightSet {
    /// `deadline` = 진행 기반 무진행 상한(운영 10s급).
    pub fn new(deadline: Duration) -> Self {
        Self {
            agents: HashMap::new(),
            deadline,
        }
    }

    /// 항상 정확히 1개의 마커로 종결되는 계약의 진입점.
    pub fn request_replay(&mut self, agent: AgentId, now: Instant) -> RequestOutcome {
        let deadline = self.deadline;
        let f = self.agents.entry(agent).or_default();
        if f.in_flight.is_none() {
            f.gen_counter += 1;
            let generation = f.gen_counter;
            f.in_flight = Some(InFlight::fresh(generation, now, deadline));
            RequestOutcome {
                generation,
                send_now: true,
            }
        } else {
            // ★coalesce 안전조건★: 요청은 항상 뷰 SubState 등록 *후* 도착하므로 그 뒤에 발사될 Subscribe 에
            //   병합해도 안전하다(rev2 안전조건이 이 구조로 충족). N뷰 동시 remount ≤ 2회 replay.
            //   ★좀비 슬롯도 여기로 병합★ — 좀비가 late Complete/disconnect 로 풀릴 때 이 next_gen 이 나간다.
            let generation = match f.next_gen {
                Some(g) => g,
                None => {
                    f.gen_counter += 1;
                    f.next_gen = Some(f.gen_counter);
                    f.gen_counter
                }
            };
            RequestOutcome {
                generation,
                send_now: false,
            }
        }
    }

    /// ★send 실패 롤백(FIX-2)★: send_now Subscribe 가 wire 송신 실패했을 때 방금 만든 in-flight 를 롤백한다
    /// (마커 미발행 — 아무 것도 wire 로 안 나갔다). 호출자가 send 실패를 감지한 *직후* 부른다(actor
    /// 직렬이라 그 사이 in-flight 는 방금 만든 그 세대 그대로).
    pub fn abort_in_flight(&mut self, agent: AgentId) {
        if let Some(f) = self.agents.get_mut(&agent) {
            f.in_flight = None;
            f.next_gen = None;
        }
    }

    /// single-flight 라 도착하는 Ack 는 항상 유일 outstanding in-flight 의 것이다(좀비 포함 — 만료
    /// 세대의 late Ack 도 그 슬롯을 가리킨다). 그래서 gen 대조 없이 현 슬롯에 그대로 각인한다.
    pub fn on_ack(&mut self, agent: AgentId, truncated: bool, now: Instant) {
        let deadline = self.deadline;
        if let Some(f) = self.agents.get_mut(&agent) {
            if let Some(inf) = f.in_flight.as_mut() {
                inf.acked = true;
                inf.truncated = truncated;
                inf.deadline = now + deadline;
            }
        }
    }

    /// binary frame 등 그 에이전트의 진행 신호 — deadline 리셋(healthy-slow replay 무오탐).
    pub fn note_progress(&mut self, agent: AgentId, now: Instant) {
        let deadline = self.deadline;
        if let Some(f) = self.agents.get_mut(&agent) {
            if let Some(inf) = f.in_flight.as_mut() {
                inf.deadline = now + deadline;
            }
        }
    }

    /// **acked 상태의 in_flight 에만** 성공 마커를 각인한다. wire 순서가 `[Ack_k]…[Complete_k][Ack_{k+1}]`
    /// 이므로 Ack 전에 도착한 Complete 는 증명 가능하게 전대(前代)의 고아 → Ignore(오귀속 원천 차단).
    /// `now` = 새 in-flight 의 deadline 기점.
    ///
    /// ★좀비도 여기서 해제된다(FIX-1)★: deadline 으로 좀비(`failed=true`)가 된 슬롯도 late Ack→late Complete
    /// 면 acked 게이트를 통과해 **같은 gen 의 성공 마커**로 해제된다(replay 가 실제로 완료됐다는 증거).
    pub fn on_complete(&mut self, agent: AgentId, now: Instant) -> Resolution {
        let deadline = self.deadline;
        let Some(f) = self.agents.get_mut(&agent) else {
            return Resolution::Ignore;
        };
        let acked = matches!(&f.in_flight, Some(inf) if inf.acked);
        if !acked {
            return Resolution::Ignore;
        }
        let inf = f.in_flight.take().expect("acked 이면 in_flight 는 Some");
        // ★같은 gen 성공 마커★: 좀비였든(late Complete) 정상이든 replay 완료 = 성공(failed:false).
        let marker = Marker {
            generation: inf.generation,
            truncated: inf.truncated,
            failed: false,
        };
        let send_next = advance_next(f, now, deadline);
        Resolution::Emit { marker, send_next }
    }

    /// ★진행 기반 deadline sweep(FIX-1 — zombie 의미론)★. 무진행으로 만료된 in-flight 를 **실패 마커로 1회
    /// 발행**하고 좀비(`failed=true`)로 표시하되 **슬롯은 유지하고 대기열은 전진시키지 않는다**. 반환:
    /// `(agent, 실패 마커)` 목록(호출자가 마커만 송신 — 다음 Subscribe 는 여기서 절대 안 나간다).
    ///
    /// ★왜 큐를 전진 안 시키나(cross-family 리뷰어 적출 — load-bearing)★: 타임아웃이 즉시 다음 Subscribe 로
    /// 넘어가 슬롯을 교체하면, 만료 세대의 *늦은* Ack/Complete 가 도착했을 때 그게 새 in-flight 에 오각인돼
    /// **replay 가 아직 안 돈 새 gen 에 성공 마커**가 붙는다(gen 펜스 붕괴). 좀비로 슬롯을 붙잡아 두면
    /// Ack/Complete 는 구조적으로 유일 outstanding Subscribe(=이 좀비)만 가리킬 수 있어 오귀속이 불가능하다.
    /// 슬롯 해제는 오직 resolution(late Complete) 또는 disconnect 로만 일어난다.
    pub fn check_deadlines(&mut self, now: Instant) -> Vec<(AgentId, Marker)> {
        let mut out = Vec::new();
        for (agent, f) in self.agents.iter_mut() {
            if let Some(inf) = f.in_flight.as_mut() {
                if !inf.failed && now >= inf.deadline {
                    inf.failed = true;
                    out.push((
                        *agent,
                        Marker {
                            generation: inf.generation,
                            truncated: inf.truncated,
                            failed: true,
                        },
                    ));
                }
            }
        }
        out
    }

    /// 연결 단절 — 내부 클리어만, **마커는 발행하지 않는다**(재요청 구동자는 프론트 connected 전이 단독).
    /// agent-gone 으로 disconnect 까지 남은 좀비도 여기서 최종 청소된다.
    pub fn on_disconnect(&mut self) {
        for f in self.agents.values_mut() {
            f.in_flight = None;
            f.next_gen = None;
        }
    }
}

fn advance_next(f: &mut AgentFlight, now: Instant, deadline: Duration) -> bool {
    match f.next_gen.take() {
        Some(g) => {
            f.in_flight = Some(InFlight::fresh(g, now, deadline));
            true
        }
        None => false,
    }
}

/// replay 경계 마커를 wire 프레임 bytes 로 인코딩(Channel 내부 계약, ADR-0046). 레이아웃(M2 파서 계약):
/// `[tag=255:1][agentId:16][epoch:4 BE][gen:8 BE][flags:1]`.
///
/// ★엔디안(FIX-4 — 마커 프레임 전체 BE 통일)★: agentId 는 RFC4122 network order(frame 헤더 동형), epoch·gen
/// 은 모두 **big-endian**(binary frame 헤더가 uniformly BE 인 것과 동일 규약 — M2 파서가 한 규약으로 읽게).
/// flags bit0=truncated, bit1=failed.
pub fn encode_marker_frame(agent_id: AgentId, epoch: u32, marker: Marker) -> Vec<u8> {
    let mut buf = Vec::with_capacity(MARKER_FRAME_LEN);
    buf.push(MARKER_TAG);
    buf.extend_from_slice(agent_id.as_bytes());
    buf.extend_from_slice(&epoch.to_be_bytes());
    buf.extend_from_slice(&marker.generation.to_be_bytes());
    let mut flags = 0u8;
    if marker.truncated {
        flags |= 0b0000_0001;
    }
    if marker.failed {
        flags |= 0b0000_0010;
    }
    buf.push(flags);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aid(n: u128) -> AgentId {
        Uuid::from_u128(n)
    }

    fn t0() -> Instant {
        Instant::now()
    }

    fn dl() -> Duration {
        Duration::from_secs(10)
    }

    // ── gen 단조 + idle 요청 = Subscribe 1회 ───────────────────────────────────────────
    #[test]
    fn idle_request_allocates_gen_and_sends_subscribe() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        let out = fs.request_replay(a, now);
        assert_eq!(out.generation, 1, "첫 gen=1");
        assert!(out.send_now, "idle 이면 즉시 Subscribe 송신");
    }

    #[test]
    fn gen_counter_monotonic_across_cycles_never_resets() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        assert_eq!(fs.request_replay(a, now).generation, 1);
        fs.on_ack(a, false, now);
        match fs.on_complete(a, now) {
            Resolution::Emit { marker, send_next } => {
                assert_eq!(marker.generation, 1);
                assert!(!send_next, "대기열 없으면 다음 Subscribe 없음");
            }
            other => panic!("성공 마커여야: {other:?}"),
        }
        assert_eq!(fs.request_replay(a, now).generation, 2, "gen 단조(리셋 0)");
        fs.on_ack(a, false, now);
        assert!(matches!(
            fs.on_complete(a, now),
            Resolution::Emit {
                marker: Marker { generation: 2, .. },
                ..
            }
        ));
    }

    // ── coalescing ────────────────────────────────────────────────────────────────────
    #[test]
    fn coalesces_waiters_to_single_next_gen() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        let first = fs.request_replay(a, now);
        assert_eq!((first.generation, first.send_now), (1, true));
        let w1 = fs.request_replay(a, now);
        let w2 = fs.request_replay(a, now);
        assert_eq!(w1.generation, 2, "첫 대기자 = 다음 gen 2");
        assert_eq!(w2.generation, 2, "둘째 대기자 = 같은 다음 gen 2(공유)");
        assert!(
            !w1.send_now && !w2.send_now,
            "in-flight 중이라 즉시 발사 안 함"
        );
        fs.on_ack(a, false, now);
        match fs.on_complete(a, now) {
            Resolution::Emit { marker, send_next } => {
                assert_eq!(marker.generation, 1, "해소되는 건 현 in-flight(gen1)");
                assert!(send_next, "병합된 대기열 → 다음 Subscribe 1회 발사");
            }
            other => panic!("성공 마커여야: {other:?}"),
        }
        fs.on_ack(a, false, now);
        match fs.on_complete(a, now) {
            Resolution::Emit { marker, send_next } => {
                assert_eq!(marker.generation, 2);
                assert!(!send_next, "대기열 소진 — 더는 Subscribe 없음");
            }
            other => panic!("성공 마커여야: {other:?}"),
        }
    }

    // ── acked 게이트 ──────────────────────────────────────────────────────────────────
    #[test]
    fn complete_before_ack_is_orphan_ignored() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        fs.request_replay(a, now);
        assert_eq!(
            fs.on_complete(a, now),
            Resolution::Ignore,
            "Ack 전 Complete 무시"
        );
        fs.on_ack(a, false, now);
        assert!(matches!(
            fs.on_complete(a, now),
            Resolution::Emit {
                marker: Marker {
                    generation: 1,
                    failed: false,
                    ..
                },
                ..
            }
        ));
    }

    #[test]
    fn complete_without_inflight_is_ignored() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        assert_eq!(
            fs.on_complete(a, t0()),
            Resolution::Ignore,
            "in-flight 없으면 stray Complete 무시"
        );
    }

    // ── FIX-1 (a): 좀비 late-resolution 시퀀스 ────────────────────────────────────────
    #[test]
    fn zombie_late_ack_complete_emits_success_same_gen_then_advances() {
        let mut fs = ReplayFlightSet::new(Duration::from_millis(100));
        let a = aid(1);
        let start = t0();
        assert_eq!(fs.request_replay(a, start).generation, 1);
        let w = fs.request_replay(a, start);
        assert_eq!((w.generation, w.send_now), (2, false), "gen2 대기열 병합");
        let expired = fs.check_deadlines(start + Duration::from_millis(200));
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].0, a);
        assert_eq!(expired[0].1.generation, 1, "실패 마커 gen1");
        assert!(expired[0].1.failed, "무진행 초과 = 실패 마커");
        assert!(
            fs.check_deadlines(start + Duration::from_millis(300))
                .is_empty(),
            "좀비는 실패 마커 재발행 없음"
        );
        fs.on_ack(a, false, start + Duration::from_millis(400));
        match fs.on_complete(a, start + Duration::from_millis(500)) {
            Resolution::Emit { marker, send_next } => {
                assert_eq!(marker.generation, 1, "★성공 마커는 gen1 — gen2 아님★");
                assert!(!marker.failed, "late Complete = 성공(replay 실제 완료)");
                assert!(send_next, "이제서야 병합된 gen2 Subscribe 발사");
            }
            other => panic!("late Complete 는 성공 마커: {other:?}"),
        }
        fs.on_ack(a, false, start + Duration::from_millis(600));
        match fs.on_complete(a, start + Duration::from_millis(700)) {
            Resolution::Emit { marker, send_next } => {
                assert_eq!(marker.generation, 2);
                assert!(!marker.failed);
                assert!(!send_next, "대기열 소진");
            }
            other => panic!("성공 마커여야: {other:?}"),
        }
    }

    // ── FIX-1 (b): 좀비 미해제 + disconnect ───────────────────────────────────────────
    #[test]
    fn zombie_unresolved_then_disconnect_clears_no_markers_counter_monotonic() {
        let mut fs = ReplayFlightSet::new(Duration::from_millis(100));
        let a = aid(1);
        let start = t0();
        fs.request_replay(a, start); // gen1
        fs.request_replay(a, start); // gen2 대기열
                                     // agent-gone 시나리오: 이후 Ack/Complete 가 영영 안 온다.
        let expired = fs.check_deadlines(start + Duration::from_millis(200));
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].1.generation, 1);
        assert!(expired[0].1.failed);
        let w = fs.request_replay(a, start + Duration::from_millis(300));
        assert!(!w.send_now, "좀비가 슬롯 점유 → 새 요청 병합(즉시 발사 X)");
        fs.on_disconnect();
        let after = fs.request_replay(a, start + Duration::from_millis(400));
        assert_eq!(after.generation, 3, "gen_counter 단조(좀비 후에도 리셋 0)");
        assert!(after.send_now, "disconnect 로 슬롯 비어 즉시 발사");
    }

    // ── 진행 기반 deadline ────────────────────────────────────────────────────────────
    #[test]
    fn missing_complete_expires_to_failure_marker_then_zombie_resolves() {
        let mut fs = ReplayFlightSet::new(Duration::from_millis(100));
        let a = aid(1);
        let start = t0();
        fs.request_replay(a, start); // gen1 in-flight
        let expired = fs.check_deadlines(start + Duration::from_millis(200));
        assert_eq!(expired.len(), 1);
        let (agent, marker) = expired[0];
        assert_eq!(agent, a);
        assert_eq!(marker.generation, 1);
        assert!(marker.failed, "무진행 초과 = 실패 마커");
        assert!(
            !fs.request_replay(a, start + Duration::from_millis(250))
                .send_now,
            "좀비 슬롯 점유로 새 요청은 병합"
        );
        fs.on_ack(a, false, start + Duration::from_millis(300));
        assert!(matches!(
            fs.on_complete(a, start + Duration::from_millis(300)),
            Resolution::Emit {
                marker: Marker {
                    generation: 1,
                    failed: false,
                    ..
                },
                send_next: true
            }
        ));
    }

    #[test]
    fn progress_resets_deadline_healthy_slow_replay_no_false_positive() {
        let mut fs = ReplayFlightSet::new(Duration::from_millis(100));
        let a = aid(1);
        let start = t0();
        fs.request_replay(a, start);
        fs.on_ack(a, false, start);
        let mut now = start;
        for _ in 0..5 {
            now += Duration::from_millis(80); // deadline(100ms) 전에 진행.
            fs.note_progress(a, now);
            assert!(fs.check_deadlines(now).is_empty(), "진행 중이면 만료 0");
        }
        assert!(matches!(
            fs.on_complete(a, now),
            Resolution::Emit {
                marker: Marker { failed: false, .. },
                ..
            }
        ));
    }

    // ── FIX-2: send 실패 롤백 ─────────────────────────────────────────────────────────
    #[test]
    fn send_failure_releases_slot_next_request_works() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        let first = fs.request_replay(a, now);
        assert_eq!((first.generation, first.send_now), (1, true));
        fs.abort_in_flight(a);
        let next = fs.request_replay(a, now);
        assert_eq!(next.generation, 2, "gen 단조(1 소진 → 2)");
        assert!(next.send_now, "롤백으로 슬롯 비어 즉시 재발사");
        fs.on_ack(a, false, now);
        assert!(matches!(
            fs.on_complete(a, now),
            Resolution::Emit {
                marker: Marker { generation: 2, .. },
                ..
            }
        ));
    }

    // ── 단절 ──────────────────────────────────────────────────────────────────────────
    #[test]
    fn disconnect_clears_inflight_and_waiters_keeps_counter() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        fs.request_replay(a, now); // gen1 in-flight
        fs.request_replay(a, now); // gen2 대기열
        fs.on_disconnect();
        let out = fs.request_replay(a, now);
        assert_eq!(out.generation, 3, "gen_counter 단조 유지(2까지 소진 → 3)");
        assert!(out.send_now, "단절로 in-flight 비어 즉시 발사");
    }

    // ── truncated 플래그 전파 ──────────────────────────────────────────────────────────
    #[test]
    fn truncated_flag_propagates_to_success_marker() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        fs.request_replay(a, now);
        fs.on_ack(a, true, now); // 데몬이 하한 초과 과거를 잘랐음.
        match fs.on_complete(a, now) {
            Resolution::Emit { marker, .. } => {
                assert!(marker.truncated, "SubscribeAck.truncated 가 마커로 전파");
                assert!(!marker.failed);
            }
            other => panic!("성공 마커여야: {other:?}"),
        }
    }

    // ── 마커 인코딩 규격 ──────────────────────────────────────────────────────────────
    #[test]
    fn marker_frame_layout_is_fixed() {
        let a = aid(0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10);
        let marker = Marker {
            generation: 0x1122_3344_5566_7788,
            truncated: true,
            failed: false,
        };
        let buf = encode_marker_frame(a, 7, marker);
        assert_eq!(buf.len(), MARKER_FRAME_LEN, "30바이트");
        assert_eq!(buf[0], MARKER_TAG, "tag=255");
        assert_eq!(&buf[1..17], a.as_bytes(), "agentId 16바이트");
        assert_eq!(&buf[17..21], &7u32.to_be_bytes(), "epoch BE");
        assert_eq!(
            &buf[21..29],
            &0x1122_3344_5566_7788u64.to_be_bytes(),
            "gen BE(FIX-4 — 프레임 전체 BE 통일)"
        );
        assert_eq!(buf[29], 0b0000_0001, "flags: truncated=bit0");
    }

    #[test]
    fn marker_failed_flag_encodes_bit1() {
        let a = aid(1);
        let buf = encode_marker_frame(
            a,
            0,
            Marker {
                generation: 1,
                truncated: false,
                failed: true,
            },
        );
        assert_eq!(buf[29], 0b0000_0010, "flags: failed=bit1");
    }
}
