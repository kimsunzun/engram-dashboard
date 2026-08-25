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
//! - in_flight 수명 = sent → (SubscribeAck) → acked → (ReplayComplete) → 성공 마커. **슬롯의 해제 경로는
//!   셋뿐 — resolution(Ack 뒤 Complete) · 거절([`ReplayFlightSet::on_refused`]) · disconnect.** deadline
//!   초과는 슬롯을 해제하지 않는다.
//! - 같은 gen 에 실패 마커 뒤 성공 마커가 붙어도 안전하다: 재요청한 뷰는 더 높은 myGen 을 들고 이걸
//!   무시하고, 아직 대기 중인 뷰는 완전한 버퍼를 flush 한다(뷰는 실패 마커에 버퍼를 유지한다). 진행 기반
//!   deadline 아래 late-resolution 은 empty/near-empty replay 를 함의하고, 흘렀던 frame 은 뷰가 이미 버퍼했다.
//! - **agent-gone 은 더 이상 좀비가 아니다:** 데몬이 그 거절을 `AgentEvent::SubscribeFailed` 로 내고
//!   ([`ReplayFlightSet::on_refused`]) 슬롯이 그 자리에서 풀린다. 옛 배선은 주인을 식별할 필드가 없는
//!   `Error` 뿐이라 클라가 어느 슬롯을 풀지 몰랐고, 그래서 여기 "수용된 한계" 로 적혀 있었다(되살리지 마라).
//! - **그래도 좀비가 남는 형태 하나:** 데몬이 `Ack` 만 주고 그 뒤로 `ReplayComplete` 도 거절도 영영 안 보내는
//!   경우(응답이 아직 올 수 있으므로 만료가 슬롯을 못 푼다). 그 슬롯은 disconnect 까지 남는다. 실패 마커는
//!   최초 만료 때 이미 나갔고 UX 는 뷰의 bounded 재요청 사다리가 처리한다(수용). ★데몬이 `Complete` 를
//!   **큐 포화로 흘리는** 갈래는 여기 들지 않는다★ — 그쪽은 데몬이 그 자리에서 연결을 닫아 disconnect 로
//!   귀결시킨다(daemon `handle_subscribe` 의 ReplayComplete enqueue 실패 분기).
//!
//! ## ★단일 outstanding — 이 파일 밖에서 지켜지는 전제(load-bearing)★
//! [`ReplayFlightSet::on_ack`]·[`ReplayFlightSet::on_complete`]·[`ReplayFlightSet::on_refused`] 는 도착한
//! 응답의 세대를 **대조하지 않는다**(wire 에 세대 필드가 없다 — 데몬은 gen 을 모른다). 대신 "그 에이전트의
//! Subscribe 는 wire 에 많아야 하나뿐이고 그것이 곧 현 `in_flight`" 라는 전제에 기댄다. 그래서 이 파일은
//! **추적을 잃은 채 wire 로 나간 Subscribe 를 만드는 진입점을 두지 않는다** — 그런 함수가 하나라도 있으면
//! 구세대의 Ack/Complete/거절이 신세대 슬롯에 오각인돼 replay 가 돌지 않은 gen 에 성공 마커가 붙는다
//! (gen 펜스 붕괴). 호출자 쪽 대응 규율은 `src-tauri/src/daemon_client/connection.rs` 의 replay Subscribe
//! 송신 실패 처리(연결을 끊는다)다.
//!
//! ## ★순수성(테스트 격리 — ADR-0012/0003)★
//! 소켓·tokio·Tauri·protocol 의존 0 — agent crate 에 산다(agentId 는 `uuid::Uuid`, src-tauri 의 `AgentId` 는
//! 그 alias 라 통과 전달). 시간은 `Instant` 를 인자로 받아 결정론 단위테스트가 가능하다(부작용=마커 실제
//! 송신·wire Subscribe 는 호출자=연결 task 가 수행). 이 위치 덕에 단위테스트가 `cargo test
//! -p engram-dashboard-agent`(WebView2 DLL 없는 headless)에서 **실행**된다(src-tauri 테스트는 이 환경 미실행).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use uuid::Uuid;

/// 에이전트 식별자(agent 는 protocol 무의존 — `AgentId = uuid::Uuid` alias 를 그대로 받는다, ADR-0003).
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

/// [`ReplayFlightSet::on_refused`] 결정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalOutcome {
    /// 풀 슬롯이 없다 — disconnect 로 이미 청소됐거나, 우리가 낸 적 없는 구독의 거절(stray).
    Ignore,
    /// 슬롯 해제 완료.
    Released {
        /// 실패 마커. 이미 좀비로 1회 발행됐으면 `None`(중복 발행 금지 — 뷰의 재요청 사다리를 두 번 민다).
        marker: Option<Marker>,
        /// `true` = 병합된 다음 요청의 Subscribe 를 지금 보낸다.
        send_next: bool,
    },
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

    // ★송신 실패 롤백 진입점은 의도적으로 없다(되살리지 마라)★: 옛 `abort_in_flight` 는 send_now
    //   Subscribe 의 wire 송신 실패 시 슬롯만 비우고 **연결은 계속 폴링**했다. 그 조합은 이 파일의 단일
    //   outstanding 전제(모듈 헤더)를 호출자 쪽 가정 위에 올려놓는다 — "송신 실패 = 상대가 못 받았다"가
    //   참일 때만 안전한데, 그건 tungstenite/TCP 의 오류 의미론이지 우리가 강제하는 성질이 아니다. 대신
    //   호출자가 그 자리에서 **연결을 끊고**(`connection.rs` Subscribe(replay) 송신 실패 분기)
    //   [`Self::on_disconnect`] 가 일괄 청소한다 — 롤백보다 넓게 지우지만 전제를 코드로 만든다.

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
    /// 슬롯 해제는 resolution(late Complete) · **거절**([`Self::on_refused`]) · disconnect 로만 일어난다.
    ///
    /// ★거절은 왜 해제해도 되나 — 타임아웃은 왜 안 되나(위 hazard 를 푸는 근거)★: 이 둘의 차이는
    /// **응답이 아직 올 수 있는가** 하나다. 타임아웃은 "10초째 조용하다"는 관측일 뿐이라 늦은 Ack/Complete
    /// 가 여전히 올 수 있고, 그래서 슬롯을 붙잡아 오귀속을 구조적으로 막아야 한다. 반면 거절
    /// (`AgentEvent::SubscribeFailed`)은 데몬이 **그 Subscribe 를 처리하지 않기로 끝냈다는 통보**이고,
    /// 그 뒤로 그 구독에 대한 Ack 도 Complete 도 발행되지 않는다(데몬 `handle_subscribe` 는 Ack 발행 전에
    /// 조기 return 한다 — 그 실패는 `get_session` 조회 실패뿐이라 구조적으로 `on_ready` 앞이다).
    /// 오귀속될 응답 자체가 존재하지 않으므로 즉시 해제가 안전하다.
    /// 이 차이 때문에 "만료된 좀비를 해제한다"는 손쉬운 변형을 **택하지 않았다**(실측 2026-08-19).
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

    /// 데몬이 이 에이전트의 `Subscribe` 를 **거절**했다(`AgentEvent::SubscribeFailed`). 슬롯을 해제하고,
    /// 아직 실패 마커를 안 냈으면 지금 내고(뷰가 10초 deadline 을 기다리지 않게), 병합된 대기열을 전진시킨다.
    ///
    /// ★해제가 안전한 이유★ = [`Self::check_deadlines`] 의 "거절은 왜 해제해도 되나" 문단(그 hazard 를
    /// 푸는 근거가 거기 있다). 요약: 거절된 구독엔 뒤따를 Ack/Complete 가 없어 오귀속될 응답이 없다.
    ///
    /// ★왜 세대를 대조하지 않나 — "푸는 슬롯이 곧 거절당한 그 슬롯"인 근거★: wire 거절
    /// (`AgentEvent::SubscribeFailed`)엔 세대 필드가 없다(데몬은 gen 을 모른다). 그래도 현 `in_flight` 를
    /// 풀어도 되는 건 **그 에이전트의 Subscribe 가 wire 에 많아야 하나뿐**이기 때문이다 — 모듈 헤더의 단일
    /// outstanding 전제. 새 Subscribe 는 (a) idle 요청 · (b) [`Self::on_complete`]/이 함수의 `send_next`
    /// 로만 나가고, 둘 다 **직전 세대가 해소된 뒤에** 나간다. 그래서 "거절이 도착했는데 그 대상은 이미
    /// 해소된 구세대이고 현 슬롯은 다른 세대" 라는 상태가 성립하지 않는다. ★이 전제를 깨는 유일한 형태 =
    /// 추적을 잃은 채 wire 에 남은 Subscribe★ 이고, 그런 진입점을 만들지 않는다(위 "송신 실패 롤백 진입점은
    /// 의도적으로 없다" 주석).
    ///
    /// ★acked 슬롯은 해제하지 않는다(방어)★: Ack 를 받은 구독은 데몬이 **받아들인** 것이라 거절과 공존할
    /// 수 없다. 그래도 막는 이유 = 해제해 버리면 뒤따라오는 `ReplayComplete` 가 빈 슬롯을 만나
    /// [`Self::on_complete`] 의 acked 게이트에서 `Ignore` 로 떨어지고, 그 replay 를 기다리던 뷰는 성공
    /// 마커를 영영 못 받는다. 잘못 발화한 거절 하나가 **정상 replay 를 죽이는** 경로라 값싸게 닫는다.
    ///
    /// ★거절 사유가 일시적이어도 해제가 맞다★: 이 함수는 "재시도해도 될까"를 판정하지 않는다 — 재요청
    /// 구동자는 뷰의 bounded 사다리다. 해제는 그 재요청이 **wire 로 나갈 수 있게** 만들 뿐이고, 붙잡아
    /// 두면 일시적 실패조차 영구 두절이 된다(이 결함의 실제 모습).
    pub fn on_refused(&mut self, agent: AgentId, now: Instant) -> RefusalOutcome {
        let deadline = self.deadline;
        let Some(f) = self.agents.get_mut(&agent) else {
            return RefusalOutcome::Ignore;
        };
        match f.in_flight.as_ref() {
            None => return RefusalOutcome::Ignore,
            Some(inf) if inf.acked => return RefusalOutcome::Ignore,
            Some(_) => {}
        }
        let inf = f.in_flight.take().expect("바로 위에서 Some 확인");
        let marker = if inf.failed {
            None
        } else {
            Some(Marker {
                generation: inf.generation,
                truncated: inf.truncated,
                failed: true,
            })
        };
        let send_next = advance_next(f, now, deadline);
        RefusalOutcome::Released { marker, send_next }
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

    // ── 거절(SubscribeFailed) — 슬롯 해제 ──────────────────────────────────────────────
    // ★회귀 대상(실측 2026-08-19)★: 데몬 재기동 직후엔 세션이 없어(부팅 자동 복원 OFF) 재연결 replay 의
    //   Subscribe 가 "agent not found" 로 거절된다. 거절엔 Ack/Complete 가 없어 슬롯이 좀비로 남았고,
    //   그 뒤 **그 에이전트의 Subscribe 가 두 번 다시 나가지 못해**(모든 요청이 병합) 재spawn 해도 출력이
    //   모든 창에서 영구 두절됐다. 아래 첫 케이스가 정확히 그 "두 번 다시"를 깬다.
    #[test]
    fn refusal_releases_slot_so_next_request_sends_again() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        let first = fs.request_replay(a, now);
        assert_eq!((first.generation, first.send_now), (1, true));

        match fs.on_refused(a, now) {
            RefusalOutcome::Released { marker, send_next } => {
                let m = marker.expect("첫 거절은 실패 마커를 낸다(뷰가 10초를 안 기다리게)");
                assert_eq!(m.generation, 1);
                assert!(m.failed);
                assert!(!send_next, "대기열이 없으면 다음 Subscribe 없음");
            }
            other => panic!("거절은 슬롯을 해제해야: {other:?}"),
        }

        // ★이 단언이 결함의 핵심★ — 고치기 전엔 send_now=false 가 영원히 반복됐다.
        let next = fs.request_replay(a, now);
        assert_eq!(next.generation, 2, "gen 단조");
        assert!(next.send_now, "해제됐으니 새 Subscribe 가 즉시 나간다");
    }

    #[test]
    fn refusal_advances_coalesced_waiter() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        fs.request_replay(a, now); // gen1 in-flight
        let w = fs.request_replay(a, now); // gen2 병합
        assert_eq!((w.generation, w.send_now), (2, false));

        match fs.on_refused(a, now) {
            RefusalOutcome::Released { marker, send_next } => {
                assert_eq!(marker.expect("실패 마커").generation, 1, "해제되는 건 gen1");
                assert!(send_next, "병합된 gen2 Subscribe 를 지금 발사");
            }
            other => panic!("해제여야: {other:?}"),
        }
        // gen2 가 in-flight 로 올라섰다 — 정상 Ack/Complete 로 성공 마커까지 간다.
        fs.on_ack(a, false, now);
        assert!(matches!(
            fs.on_complete(a, now),
            Resolution::Emit {
                marker: Marker {
                    generation: 2,
                    failed: false,
                    ..
                },
                ..
            }
        ));
    }

    // ★단일 outstanding 전제의 박제★: 거절엔 세대 필드가 없으므로 "지금 풀리는 게 방금 거절당한 그 세대"는
    //   순서로만 성립한다 — 연속 거절이 각각 **그때 wire 에 나가 있던** 세대를 낸다. 이게 어긋나면 gen 펜스가
    //   무너진다(모듈 헤더 · on_refused 문단).
    #[test]
    fn consecutive_refusals_release_the_then_outstanding_generation() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        fs.request_replay(a, now); // gen1 = wire 로 나감
        let w = fs.request_replay(a, now); // gen2 병합(아직 안 나감)
        assert_eq!((w.generation, w.send_now), (2, false));

        match fs.on_refused(a, now) {
            RefusalOutcome::Released { marker, send_next } => {
                assert_eq!(marker.expect("실패 마커").generation, 1, "첫 거절 = gen1");
                assert!(send_next, "이제 gen2 가 wire 로 나간다");
            }
            other => panic!("해제여야: {other:?}"),
        }
        // 두 번째 거절이 도착 — 그 시점 outstanding 은 gen2 하나뿐이다.
        match fs.on_refused(a, now) {
            RefusalOutcome::Released { marker, send_next } => {
                assert_eq!(marker.expect("실패 마커").generation, 2, "둘째 거절 = gen2");
                assert!(!send_next, "대기열 소진");
            }
            other => panic!("해제여야: {other:?}"),
        }
        assert!(
            fs.request_replay(a, now).send_now,
            "슬롯이 비어 재발사 가능"
        );
    }

    // 좀비(만료로 실패 마커 1회 발행됨)가 뒤늦게 거절을 받으면: 해제는 하되 마커는 **다시 내지 않는다**.
    #[test]
    fn refusal_on_expired_zombie_releases_without_duplicate_marker() {
        let mut fs = ReplayFlightSet::new(Duration::from_millis(100));
        let a = aid(1);
        let start = t0();
        fs.request_replay(a, start);
        let expired = fs.check_deadlines(start + Duration::from_millis(200));
        assert_eq!(expired.len(), 1, "만료 실패 마커 1회");

        match fs.on_refused(a, start + Duration::from_millis(300)) {
            RefusalOutcome::Released { marker, send_next } => {
                assert!(
                    marker.is_none(),
                    "이미 실패 마커가 나간 gen — 중복 발행 금지"
                );
                assert!(!send_next);
            }
            other => panic!("해제여야: {other:?}"),
        }
        assert!(
            fs.request_replay(a, start + Duration::from_millis(400))
                .send_now,
            "좀비도 거절로 풀린다"
        );
    }

    // ★방어★: Ack 를 받은(=데몬이 받아들인) 슬롯은 거절로 해제하지 않는다. 해제하면 뒤따라올
    //   ReplayComplete 가 빈 슬롯을 만나 Ignore 로 떨어져 그 replay 의 성공 마커가 영영 안 나간다.
    #[test]
    fn refusal_never_releases_an_acked_healthy_slot() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        fs.request_replay(a, now);
        fs.on_ack(a, false, now);

        assert_eq!(
            fs.on_refused(a, now),
            RefusalOutcome::Ignore,
            "acked 슬롯은 거절로 안 풀린다"
        );
        // 정상 완료 경로가 그대로 살아 있어야 한다.
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
    fn refusal_without_inflight_is_ignored() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        assert_eq!(fs.on_refused(a, now), RefusalOutcome::Ignore, "미지 agent");
        fs.request_replay(a, now);
        fs.on_refused(a, now);
        assert_eq!(
            fs.on_refused(a, now),
            RefusalOutcome::Ignore,
            "이미 해제된 슬롯의 두 번째 거절은 무시(중복 마커·오해제 금지)"
        );
    }

    // ── 송신 실패 = 단절로 청소(옛 abort_in_flight 롤백을 대체) ────────────────────────
    // ★회귀 대상★: 옛 코드는 wire 송신 실패에 슬롯만 비우고 연결을 계속 폴링했다 — 추적을 잃은 Subscribe 가
    //   wire 에 남을 수 있으면 뒤늦은 거절/Ack 가 신세대 슬롯에 오각인된다(gen 펜스 붕괴). 지금은 호출자가
    //   그 자리에서 연결을 끊고 이 경로가 일괄 청소한다. 재요청이 다시 나갈 수 있어야 한다는 결과는 같다.
    #[test]
    fn send_failure_path_clears_via_disconnect_and_next_request_sends() {
        let mut fs = ReplayFlightSet::new(dl());
        let a = aid(1);
        let now = t0();
        let first = fs.request_replay(a, now);
        assert_eq!((first.generation, first.send_now), (1, true));
        fs.on_disconnect(); // 송신 실패 → 호출자가 연결을 끊는다.
        let next = fs.request_replay(a, now);
        assert_eq!(next.generation, 2, "gen 단조(1 소진 → 2)");
        assert!(next.send_now, "단절 청소로 슬롯 비어 즉시 재발사");
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
