//! 연결 하나의 상태 기계 — ★순수 층★.
//!
//! 입력 = (지금 시각, 사건) · 출력 = 행동 목록. ★future 도 런타임도 I/O 도 없다★ — 그래서 백오프 일정과
//! 예산 판정을 **실제로 기다리지 않고** 잰다. 게이트가 그 순수성을 지킨다(정본 = `lib.rs` 헤더).
//!
//! ★이 파일이 이 crate 의 유일한 「나중에 리팩터 안 하게」 장치다★ — 한 crate 안에서 sans-IO 순수성은
//! 컴파일러가 강제하지 않아 관례로만 버티는데, 이 저장소엔 그 약점을 메우는 선례가 이미 있다
//! (`src-tauri/src/daemon_client/replay_flight.rs` 의 순수성 게이트, ADR-0175 결정 3).
//!
//! ★행동을 **집행하지 않는다**★ — 통로를 열고 닫고 기다리는 것은 전부 [`crate::peer`] 의 감독 태스크다.
//! 여기서 늘어나는 것은 [`Action`]·[`Input`] 의 variant 뿐이고 그것은 경계를 흔들지 않는다.

use std::time::{Duration, Instant};

use crate::frame::CloseCode;
use crate::link::LinkError;
use crate::policy::Policy;

/// 연결 세대. 단조 증가하고 재사용이 없다. 운영 단계에 들어설 때마다 +1 이다.
///
/// ★같은 이름표를 명부에 다시 올려도 되감기지 않는다★ — [`Machine::new`] 가 **바닥값**을 받고, 명부가
/// 옛 감독의 마지막 세대를 그 바닥으로 넘긴다. 안 그러면 소비자의 평범한 방어(「최대 세대 미만은
/// 무시」)가 **새 연결을 통째로 버린다**.
///
/// ★스트림의 화신 표식([`crate::StreamMark::generation`], u32)과 다른 것이다★ — 이쪽은 **우리 연결이
/// 몇 번째인가**이고 저쪽은 **상대의 그 스트림이 몇 번째 화신인가**다. 폭도 비교 규칙도 다르다
/// (이쪽은 대소 비교가 뜻을 갖고, 저쪽은 일치/불일치만 본다).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Generation(pub u64);

/// 상대 하나의 지금 상태. [`crate::Peer::state`] 로 밖에 노출된다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerState {
    Idle,
    Dialing,
    Handshaking,
    Live,
    Backoff,
    GaveUp(GaveUpReason),
    /// 명시 종료 — 다시 붙지 않는다.
    Closed,
}

impl PeerState {
    pub fn is_live(self) -> bool {
        matches!(self, PeerState::Live)
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, PeerState::GaveUp(_) | PeerState::Closed)
    }
}

/// 왜 포기했나. ★셸이 팝업 자격을 판정하는 입력이다★(ADR-0180 결정 2) — 둘 다 「사용자가 행동하지
/// 않으면 영영 복구되지 않는 상태」라 자격을 갖지만, 사용자가 그 자리에서 할 수 있는 일이 달라 화면
/// 모양이 갈린다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GaveUpReason {
    /// 붙었는데 거절당했다. 재시도해도 결과가 같다.
    Rejected,
    /// 재연결 예산을 다 썼다.
    BudgetExhausted,
}

/// 왜 끊겼나. ★오늘 코드에는 이 구분이 없다 — 전부 「끊김」 한 덩어리다★.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectCause {
    /// 상대가 정상 종료했다.
    PeerClosed,
    /// 통로가 오류를 냈다.
    LinkError,
    /// 상대가 `idle_timeout` 동안 조용했다.
    Idle,
    /// 한 프레임을 미는 데 `write_deadline` 을 넘겼다.
    WriteDeadline,
    /// 큐가 찼고 정책이 [`crate::QueueFullPolicy::Disconnect`] 였다.
    QueueFull,
    /// 우리가 명시로 닫았다.
    Explicit,
}

impl DisconnectCause {
    /// 이 사유로 끊을 때 실을 닫기 코드.
    pub fn close_code(self) -> CloseCode {
        match self {
            DisconnectCause::Idle => CloseCode::KEEPALIVE_TIMEOUT,
            DisconnectCause::WriteDeadline => CloseCode::WRITE_DEADLINE,
            DisconnectCause::QueueFull => CloseCode::QUEUE_FULL,
            _ => CloseCode::GOING_AWAY,
        }
    }
}

/// 연결이 못 선 이유. ★이 두 갈래가 예산을 가른다★(ADR-0180 결정 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectFailure {
    /// 통로 자체가 안 열렸다 → 예산을 쓴다.
    Unreachable(LinkError),
    /// 통로는 열렸고 상대가 안 받아 줬다 → ★예산을 안 쓰고 첫 거절에서 즉시 포기★.
    Rejected {
        code: Option<CloseCode>,
        reason: String,
    },
}

/// 감독 태스크가 기계에 넣는 것.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// 명부에 올랐다.
    Start,
    /// 통로가 열렸다 — 이제 핸드셰이크다.
    Dialed,
    /// 핸드셰이크가 끝났다. ★세대는 감독이 **상대 이름표별 공용 계수기**에서 뽑아 여기 실어 준다★ —
    /// 기계가 스스로 +1 하면 같은 이름표의 옛 감독과 새 감독이 같은 번호를 발행할 수 있다(D1).
    Connected(Generation),
    /// dial 이나 핸드셰이크가 실패했다.
    ConnectFailed(ConnectFailure),
    /// 운영 중 통로를 잃었다.
    LinkLost(DisconnectCause),
    /// 백오프 대기가 끝났다.
    BackoffElapsed,
    /// ★「지금 다시 해라」★ — 예산을 초기화하고 즉시 시도한다(ADR-0180 결정 7).
    ReconnectNow,
    /// 명시 종료.
    Close,
}

/// 기계가 위로 올리라고 지시하는 사건. [`crate::peer`] 가 여기에 출처 표식을 붙여
/// [`crate::TransportEvent`] 로 만든다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MachineEvent {
    Connected {
        generation: Generation,
    },
    Disconnected {
        generation: Generation,
        cause: DisconnectCause,
    },
    ConnectFailed {
        /// ★이제까지 쓴 **재시도** 예산★(1-기반). 첫 연결 시도는 재시도가 아니라 세지 않으므로,
        /// `retry_in` 이 `Some` 이면 "이제 `attempt`번째 재시도를 그만큼 뒤에 한다" 로 읽고 `None` 이면
        /// "`attempt`/`of` 를 다 썼다" 로 읽는다.
        attempt: u32,
        of: u32,
        cause: ConnectFailure,
        /// `None` = 더 안 기다린다(예산 소진 또는 거절).
        retry_in: Option<Duration>,
    },
    Rejected {
        code: Option<CloseCode>,
        reason: String,
    },
    GaveUp {
        reason: GaveUpReason,
    },
}

/// 기계가 시키는 일. 집행은 감독 태스크가 한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// 통로를 연다. `deadline` 은 **dial 과 핸드셰이크를 함께** 감싼다(현행 `HANDSHAKE_TIMEOUT` 보존).
    Dial { deadline: Instant },
    /// 열린 통로 위에서 핸드셰이크를 돌린다. `deadline` 은 [`Action::Dial`] 이 잡은 그것 그대로다.
    Handshake { deadline: Instant },
    /// 지금 통로를 닫고 버린다. 실을 닫기 코드가 `close` 다.
    DropLink { code: CloseCode },
    /// 이 시각까지 기다렸다가 [`Input::BackoffElapsed`] 를 넣는다.
    Wait { until: Instant },
    /// 대기 중인 요청 전원을 이 사유로 깨운다.
    FailPending(DisconnectCause),
    /// 위로 올릴 사건.
    Emit(MachineEvent),
}

/// 연결 하나의 수명.
#[derive(Debug)]
pub struct Machine {
    state: PeerState,
    generation: u64,
    /// ★못 붙어서 쓴 예산★ — 마지막 성공 연결(또는 [`Input::ReconnectNow`]) 이후 예약한 백오프 대기 수.
    /// 연결에 성공하면 0 으로 돌아간다.
    retries: u32,
    /// ★붙었다가 금세 끊긴 연속 횟수★ — `live_dwell` 을 못 채운 연결이 이어진 수. 오래 버틴 연결
    /// 하나가 0 으로 되돌린다.
    ///
    /// 축이 둘인 이유: 하나로 합치면 **둘 중 하나가 반드시 깨진다**. 붙을 때마다 예산을 다 채우면
    /// 붙자마자 끊기는 상대가 영원히 되풀이해 [`GaveUpReason::BudgetExhausted`] 에 닿지 않고(팝업이
    /// 안 뜬다), 안 채우면 **재시도를 한 번도 안 해 보고** 포기하는 자리가 생긴다 — 데몬이 4초 걸려
    /// 재시작하는 동안 예산을 다 태우고, 붙은 뒤 10초 만에 한 번 끊기면 그 즉시 포기하는 모양이다.
    /// 그 상태는 「사용자가 행동하지 않으면 영영 복구되지 않는」이 아니라 **한 번만 더 걸면 붙는**
    /// 상태라 팝업 자격이 없다(ADR-0180 결정 2).
    flaps: u32,
    policy: Policy,
    /// 지금 시도의 dial+핸드셰이크 마감. 두 구간이 한 예산을 나눠 쓰므로 기계가 들고 있다.
    connect_deadline: Instant,
    /// 지금 연결이 운영 단계에 들어선 시각. 예산을 되채울 자격을 이것으로 잰다.
    live_since: Option<Instant>,
    /// 지터 눈금의 시간 축. `Instant` 는 내부 표현을 안 내주므로 기준점과의 차이를 쓴다.
    epoch: Instant,
    /// 상대마다 다른 눈금이 나오게 하는 씨앗. 부르는 쪽이 `PeerId` 에서 뽑는다.
    jitter_seed: u64,
}

impl Machine {
    /// `generation_floor` = 이 상대의 세대가 여기서부터 이어진다(같은 이름표의 옛 감독이 남긴 값).
    /// 처음 세우는 상대는 0 이다.
    pub fn new(policy: Policy, jitter_seed: u64, created: Instant, generation_floor: u64) -> Self {
        Self {
            state: PeerState::Idle,
            generation: generation_floor,
            retries: 0,
            flaps: 0,
            connect_deadline: created,
            live_since: None,
            policy,
            epoch: created,
            jitter_seed,
        }
    }

    pub fn state(&self) -> PeerState {
        self.state
    }

    pub fn generation(&self) -> Generation {
        Generation(self.generation)
    }

    /// 못 붙어서 쓴 예산.
    pub fn attempt(&self) -> u32 {
        self.retries
    }

    /// 붙었다가 금세 끊긴 연속 횟수.
    pub fn flaps(&self) -> u32 {
        self.flaps
    }

    /// 사건 하나를 먹이고 할 일을 받는다.
    pub fn on(&mut self, now: Instant, input: Input) -> Vec<Action> {
        if self.state == PeerState::Closed {
            return Vec::new();
        }
        match input {
            Input::Close => {
                let was_live = self.state.is_live();
                let generation = Generation(self.generation);
                self.state = PeerState::Closed;
                self.live_since = None;
                // 대기자를 먼저 깨운다 — DropLink 는 쓰기 시한만큼 늦어질 수 있고, 그 사이 사유를
                //   묻는 호출자에게 낡은 답이 나간다.
                let mut acts = vec![Action::FailPending(DisconnectCause::Explicit)];
                if was_live {
                    acts.push(Action::Emit(MachineEvent::Disconnected {
                        generation,
                        cause: DisconnectCause::Explicit,
                    }));
                }
                acts.push(Action::DropLink {
                    code: CloseCode::GOING_AWAY,
                });
                acts
            }
            Input::Start => match self.state {
                PeerState::Idle => {
                    self.retries = 0;
                    self.flaps = 0;
                    self.dial(now)
                }
                _ => Vec::new(),
            },
            Input::ReconnectNow => {
                // ★대기자를 여기서 깨우지 않으면 아무도 안 깨운다★ — 다음 dial 이 예산을 태우고
                //   GaveUp 으로 앉으면 이 기계는 다시는 시한을 볼 일이 없고, 그 슬롯은 영구 대기가 된다.
                //   Close·LinkLost·거절이 전부 FailPending 을 내는데 이 갈래만 안 내던 것이 결함이었다.
                let was_live = self.state.is_live();
                let generation = Generation(self.generation);
                self.retries = 0;
                self.flaps = 0;
                self.live_since = None;
                let mut acts = vec![Action::FailPending(DisconnectCause::Explicit)];
                if was_live {
                    acts.push(Action::Emit(MachineEvent::Disconnected {
                        generation,
                        cause: DisconnectCause::Explicit,
                    }));
                }
                acts.push(Action::DropLink {
                    code: CloseCode::GOING_AWAY,
                });
                acts.extend(self.dial(now));
                acts
            }
            Input::BackoffElapsed => match self.state {
                PeerState::Backoff => self.dial(now),
                _ => Vec::new(),
            },
            Input::Dialed => match self.state {
                PeerState::Dialing => {
                    self.state = PeerState::Handshaking;
                    vec![Action::Handshake {
                        deadline: self.connect_deadline,
                    }]
                }
                _ => Vec::new(),
            },
            Input::Connected(generation) => match self.state {
                PeerState::Handshaking => {
                    // ★붙었으면 「못 붙는다」 예산은 다시 찬다★ — 그 축이 재는 것은 도달 가능성이고,
                    //   방금 도달했다. 붙자마자 끊기는 상대를 잡는 것은 아래 `flaps` 축이다.
                    self.retries = 0;
                    self.live_since = Some(now);
                    self.generation = generation.0;
                    self.state = PeerState::Live;
                    vec![Action::Emit(MachineEvent::Connected { generation })]
                }
                _ => Vec::new(),
            },
            Input::ConnectFailed(cause) => match self.state {
                PeerState::Dialing | PeerState::Handshaking => self.on_connect_failed(now, cause),
                _ => Vec::new(),
            },
            Input::LinkLost(cause) => match self.state {
                PeerState::Live => {
                    let generation = Generation(self.generation);
                    match self.live_since.take() {
                        // 오래 버틴 연결 하나가 「금세 끊긴다」 기록을 지운다.
                        Some(since)
                            if now.saturating_duration_since(since) >= self.policy.live_dwell =>
                        {
                            self.flaps = 0;
                        }
                        Some(_) => self.flaps = self.flaps.saturating_add(1),
                        None => {}
                    }
                    let mut acts = vec![
                        Action::FailPending(cause),
                        Action::Emit(MachineEvent::Disconnected { generation, cause }),
                        Action::DropLink {
                            code: cause.close_code(),
                        },
                    ];
                    acts.extend(self.schedule_retry(now, None));
                    acts
                }
                _ => Vec::new(),
            },
        }
    }

    fn dial(&mut self, now: Instant) -> Vec<Action> {
        self.state = PeerState::Dialing;
        self.connect_deadline = now + self.policy.connect_timeout;
        vec![Action::Dial {
            deadline: self.connect_deadline,
        }]
    }

    fn on_connect_failed(&mut self, now: Instant, cause: ConnectFailure) -> Vec<Action> {
        match cause {
            ConnectFailure::Rejected { code, reason } => {
                self.state = PeerState::GaveUp(GaveUpReason::Rejected);
                self.live_since = None;
                vec![
                    Action::FailPending(DisconnectCause::PeerClosed),
                    Action::Emit(MachineEvent::Rejected { code, reason }),
                    Action::Emit(MachineEvent::GaveUp {
                        reason: GaveUpReason::Rejected,
                    }),
                    Action::DropLink {
                        code: CloseCode::HANDSHAKE_REJECTED,
                    },
                ]
            }
            unreachable => {
                let mut acts = vec![Action::DropLink {
                    code: CloseCode::GOING_AWAY,
                }];
                acts.extend(self.schedule_retry(now, Some(unreachable)));
                acts
            }
        }
    }

    /// 다음 시도를 잡거나 예산 소진을 선언한다. `cause` 가 있으면 `ConnectFailed` 를 함께 올린다.
    fn schedule_retry(&mut self, now: Instant, cause: Option<ConnectFailure>) -> Vec<Action> {
        let of = self.policy.reconnect.max_attempts;
        // ★두 축 중 어느 쪽이 바닥나도 포기다★ — `retries` 는 「못 붙는다」, `flaps` 는 「붙어도 안
        //   버틴다」를 잰다. 어느 쪽도 사용자가 손대지 않으면 안 풀리는 상태다.
        if self.retries >= of || self.flaps >= of {
            self.state = PeerState::GaveUp(GaveUpReason::BudgetExhausted);
            let mut acts = Vec::new();
            if let Some(cause) = cause {
                acts.push(Action::Emit(MachineEvent::ConnectFailed {
                    attempt: self.retries,
                    of,
                    cause,
                    retry_in: None,
                }));
            }
            acts.push(Action::Emit(MachineEvent::GaveUp {
                reason: GaveUpReason::BudgetExhausted,
            }));
            return acts;
        }
        let delay = self
            .policy
            .reconnect
            .delay(self.retries, self.jitter_nudge(now));
        let attempt = self.retries + 1;
        self.retries = attempt;
        self.state = PeerState::Backoff;
        let mut acts = Vec::new();
        if let Some(cause) = cause {
            acts.push(Action::Emit(MachineEvent::ConnectFailed {
                attempt,
                of,
                cause,
                retry_in: Some(delay),
            }));
        }
        acts.push(Action::Wait { until: now + delay });
        acts
    }

    /// [0,1) 의 흔들림 눈금. ★새 의존을 들이지 않는다★ — 시계 하위 나노초에 상대별 씨앗과 세대를 섞는다
    /// (선례 = `agent` 의 `profile.rs` 가 난수 crate 대신 uuid v4 바이트를 쓰는 자리).
    ///
    /// **한계 둘:** ① 암호학적 난수가 아니다(지터엔 필요 없다) ② 수동 시계 아래에서는 결정적이 된다 —
    /// 하네스가 **원하는** 성질이지만, 그래서 지터의 *분산*은 실소켓에서만 관측된다.
    fn jitter_nudge(&self, now: Instant) -> f64 {
        let nanos = now.saturating_duration_since(self.epoch).subsec_nanos() as u64;
        let mixed = nanos
            ^ self.jitter_seed.rotate_left(17)
            ^ self.generation.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        (mixed % 1_000_000) as f64 / 1_000_000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> Policy {
        Policy {
            reconnect: crate::policy::ReconnectPolicy {
                jitter: 0.0,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn machine() -> (Machine, Instant) {
        let t0 = Instant::now();
        (Machine::new(policy(), 0, t0, 0), t0)
    }

    fn unreachable() -> ConnectFailure {
        ConnectFailure::Unreachable(LinkError::new("refused"))
    }

    fn waits(acts: &[Action], t0: Instant) -> Vec<Duration> {
        acts.iter()
            .filter_map(|a| match a {
                Action::Wait { until } => Some(until.duration_since(t0)),
                _ => None,
            })
            .collect()
    }

    /// 감독이 하는 일을 흉내낸다 — ★세대는 기계가 아니라 **밖에서** 배정한다★(D1).
    fn connect(m: &mut Machine, now: Instant) -> Vec<Action> {
        let minted = Generation(m.generation().0 + 1);
        m.on(now, Input::Connected(minted))
    }

    /// Idle → Dialing → Handshaking → Live 를 한 번에 민다.
    fn go_live(m: &mut Machine, t0: Instant) {
        m.on(t0, Input::Start);
        m.on(t0, Input::Dialed);
        connect(m, t0);
    }

    fn events(acts: &[Action]) -> Vec<&MachineEvent> {
        acts.iter()
            .filter_map(|a| match a {
                Action::Emit(e) => Some(e),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn start_dials_with_the_connect_budget_attached() {
        let (mut m, t0) = machine();
        let acts = m.on(t0, Input::Start);
        assert_eq!(m.state(), PeerState::Dialing);
        assert_eq!(
            acts,
            vec![Action::Dial {
                deadline: t0 + Duration::from_secs(10)
            }]
        );
    }

    #[test]
    fn reaching_live_mints_the_next_generation() {
        let (mut m, t0) = machine();
        m.on(t0, Input::Start);
        assert_eq!(m.state(), PeerState::Dialing);
        m.on(t0, Input::Dialed);
        assert_eq!(m.state(), PeerState::Handshaking);
        assert_eq!(m.generation(), Generation(0));
        let acts = connect(&mut m, t0);
        assert_eq!(m.state(), PeerState::Live);
        assert_eq!(m.generation(), Generation(1));
        assert_eq!(
            events(&acts),
            vec![&MachineEvent::Connected {
                generation: Generation(1)
            }]
        );
    }

    #[test]
    fn unreachable_burns_budget_and_the_schedule_is_500_1000_2000() {
        let (mut m, t0) = machine();
        m.on(t0, Input::Start);

        let a1 = m.on(t0, Input::ConnectFailed(unreachable()));
        assert_eq!(m.state(), PeerState::Backoff);
        assert_eq!(waits(&a1, t0), vec![Duration::from_millis(500)]);

        m.on(t0, Input::BackoffElapsed);
        let a2 = m.on(t0, Input::ConnectFailed(unreachable()));
        assert_eq!(waits(&a2, t0), vec![Duration::from_millis(1000)]);

        m.on(t0, Input::BackoffElapsed);
        let a3 = m.on(t0, Input::ConnectFailed(unreachable()));
        assert_eq!(waits(&a3, t0), vec![Duration::from_millis(2000)]);
        assert_eq!(m.attempt(), 3);
    }

    #[test]
    fn a_fourth_failure_exhausts_the_three_attempt_budget() {
        let (mut m, t0) = machine();
        m.on(t0, Input::Start);
        for _ in 0..3 {
            m.on(t0, Input::ConnectFailed(unreachable()));
            m.on(t0, Input::BackoffElapsed);
        }
        let acts = m.on(t0, Input::ConnectFailed(unreachable()));
        assert_eq!(m.state(), PeerState::GaveUp(GaveUpReason::BudgetExhausted));
        assert!(waits(&acts, t0).is_empty());
        assert!(events(&acts).iter().any(|e| matches!(
            e,
            MachineEvent::GaveUp {
                reason: GaveUpReason::BudgetExhausted
            }
        )));
    }

    #[test]
    fn the_last_failure_still_reports_its_cause() {
        let (mut m, t0) = machine();
        m.on(t0, Input::Start);
        for _ in 0..3 {
            m.on(t0, Input::ConnectFailed(unreachable()));
            m.on(t0, Input::BackoffElapsed);
        }
        let acts = m.on(t0, Input::ConnectFailed(unreachable()));
        let reported = events(&acts).into_iter().find_map(|e| match e {
            MachineEvent::ConnectFailed {
                attempt,
                of,
                retry_in,
                ..
            } => Some((*attempt, *of, *retry_in)),
            _ => None,
        });
        assert_eq!(
            reported,
            Some((3, 3, None)),
            "예산이 끝나도 왜 못 붙었는지가 버려지면 안 된다"
        );
    }

    #[test]
    fn rejection_spends_no_budget_and_gives_up_at_once() {
        let (mut m, t0) = machine();
        m.on(t0, Input::Start);
        m.on(t0, Input::Dialed);
        let acts = m.on(
            t0,
            Input::ConnectFailed(ConnectFailure::Rejected {
                code: Some(CloseCode::HANDSHAKE_REJECTED),
                reason: "version 3 != 4".into(),
            }),
        );
        assert_eq!(m.state(), PeerState::GaveUp(GaveUpReason::Rejected));
        assert_eq!(m.attempt(), 0, "거절은 예산을 안 쓴다");
        assert!(waits(&acts, t0).is_empty());
        let evs = events(&acts);
        assert!(evs.iter().any(
            |e| matches!(e, MachineEvent::Rejected { reason, .. } if reason.contains("version"))
        ));
        assert!(evs.iter().any(|e| matches!(
            e,
            MachineEvent::GaveUp {
                reason: GaveUpReason::Rejected
            }
        )));
    }

    #[test]
    fn rejection_and_unreachability_land_in_different_states_from_the_same_attempt_count() {
        let (mut rejected, t0) = machine();
        rejected.on(t0, Input::Start);
        rejected.on(t0, Input::Dialed);
        rejected.on(
            t0,
            Input::ConnectFailed(ConnectFailure::Rejected {
                code: None,
                reason: "no".into(),
            }),
        );

        let (mut unreach, _) = machine();
        unreach.on(t0, Input::Start);
        unreach.on(t0, Input::ConnectFailed(unreachable()));

        assert_eq!(rejected.state(), PeerState::GaveUp(GaveUpReason::Rejected));
        assert_eq!(unreach.state(), PeerState::Backoff);
    }

    #[test]
    fn losing_a_live_link_fails_the_waiters_and_retries() {
        let (mut m, t0) = machine();
        go_live(&mut m, t0);
        let acts = m.on(t0, Input::LinkLost(DisconnectCause::Idle));
        assert_eq!(m.state(), PeerState::Backoff);
        assert_eq!(
            acts.first(),
            Some(&Action::FailPending(DisconnectCause::Idle)),
            "대기자를 먼저 깨운다"
        );
        assert!(acts.contains(&Action::DropLink {
            code: CloseCode::KEEPALIVE_TIMEOUT
        }));
        assert_eq!(
            events(&acts),
            vec![&MachineEvent::Disconnected {
                generation: Generation(1),
                cause: DisconnectCause::Idle
            }]
        );
        assert_eq!(waits(&acts, t0), vec![Duration::from_millis(500)]);
    }

    // ── N8: 예산은 「붙었다」가 아니라 「버텼다」로 찬다 ──
    #[test]
    fn a_connection_that_holds_for_the_dwell_refills_the_budget() {
        let (mut m, t0) = machine();
        m.on(t0, Input::Start);
        m.on(t0, Input::ConnectFailed(unreachable()));
        m.on(t0, Input::BackoffElapsed);
        m.on(t0, Input::Dialed);
        connect(&mut m, t0);
        assert_eq!(m.attempt(), 0, "★붙었으면 「못 붙는다」 예산은 다시 찬다★");

        let held = t0 + Policy::default().live_dwell;
        let acts = m.on(held, Input::LinkLost(DisconnectCause::PeerClosed));
        assert_eq!(
            m.flaps(),
            0,
            "오래 버틴 연결은 「금세 끊긴다」 기록을 지운다"
        );
        assert_eq!(waits(&acts, held), vec![Duration::from_millis(500)]);
    }

    #[test]
    fn a_flapping_peer_burns_its_budget_and_reaches_gave_up() {
        let (mut m, t0) = machine();
        let mut now = t0;
        m.on(now, Input::Start);
        // 붙자마자 끊기기를 되풀이한다 — dwell 을 한 번도 못 채운다.
        for flap in 1..=3u32 {
            m.on(now, Input::Dialed);
            connect(&mut m, now);
            assert_eq!(m.state(), PeerState::Live, "flap={flap}");
            now += Duration::from_millis(10);
            m.on(now, Input::LinkLost(DisconnectCause::PeerClosed));
            assert_eq!(m.flaps(), flap);
            if flap < 3 {
                assert_eq!(
                    m.state(),
                    PeerState::Backoff,
                    "★포기하기 전에 재시도를 해 봐야 한다★"
                );
                m.on(now, Input::BackoffElapsed);
            }
        }
        assert_eq!(
            m.state(),
            PeerState::GaveUp(GaveUpReason::BudgetExhausted),
            "★여기 못 닿으면 ADR-0180 결정 7 의 팝업이 영영 안 뜬다★"
        );
    }

    // ── B4: 오래 못 붙다가 겨우 붙은 상대는 한 번 더 걸어 봐야 한다 ──
    #[test]
    fn a_peer_that_reconnected_after_a_long_outage_still_gets_a_retry() {
        let (mut m, t0) = machine();
        let mut now = t0;
        m.on(now, Input::Start);
        // 데몬 재시작이 길어 「못 붙는다」 예산을 다 태운다.
        for _ in 0..3 {
            m.on(now, Input::ConnectFailed(unreachable()));
            assert_eq!(m.state(), PeerState::Backoff);
            m.on(now, Input::BackoffElapsed);
        }
        assert_eq!(m.attempt(), 3);
        // 그러고 나서 겨우 붙는다.
        m.on(now, Input::Dialed);
        connect(&mut m, now);
        assert_eq!(m.state(), PeerState::Live);
        assert_eq!(m.attempt(), 0, "붙었으면 「못 붙는다」 예산은 다시 찬다");

        // dwell(30s)을 못 채우고 한 번 끊긴다.
        now += Duration::from_secs(10);
        let acts = m.on(now, Input::LinkLost(DisconnectCause::PeerClosed));
        assert_eq!(
            m.state(),
            PeerState::Backoff,
            "★한 번만 더 걸면 붙는 상태다 — 여기서 포기하면 팝업 자격이 없는 상태에 팝업이 뜬다★"
        );
        assert_eq!(waits(&acts, now), vec![Duration::from_millis(500)]);
    }

    #[test]
    fn a_short_lived_connection_does_not_refill_the_budget() {
        let (mut m, t0) = machine();
        m.on(t0, Input::Start);
        m.on(t0, Input::ConnectFailed(unreachable()));
        m.on(t0, Input::BackoffElapsed);
        m.on(t0, Input::Dialed);
        connect(&mut m, t0);
        let brief = t0 + Duration::from_secs(1);
        m.on(brief, Input::LinkLost(DisconnectCause::PeerClosed));
        assert_eq!(m.flaps(), 1, "1초짜리 연결은 「금세 끊긴다」로 센다");
    }

    // ── C1: 어느 갈래로 연결을 버리든 대기자를 깨운다 ──
    #[test]
    fn every_branch_that_abandons_a_link_wakes_the_waiters() {
        let abandons = [
            Input::Close,
            Input::ReconnectNow,
            Input::LinkLost(DisconnectCause::Idle),
            Input::ConnectFailed(ConnectFailure::Rejected {
                code: None,
                reason: "no".into(),
            }),
        ];
        for input in abandons {
            let (mut m, t0) = machine();
            m.on(t0, Input::Start);
            m.on(t0, Input::Dialed);
            if !matches!(input, Input::ConnectFailed(_)) {
                connect(&mut m, t0);
            }
            let acts = m.on(t0, input.clone());
            assert!(
                acts.iter().any(|a| matches!(a, Action::FailPending(_))),
                "{input:?} 가 대기자를 버려 둔다"
            );
        }
    }

    #[test]
    fn reconnect_now_from_live_says_so_before_it_redials() {
        let (mut m, t0) = machine();
        go_live(&mut m, t0);
        let acts = m.on(t0, Input::ReconnectNow);
        assert_eq!(
            acts.first(),
            Some(&Action::FailPending(DisconnectCause::Explicit)),
            "대기자 깨우기가 통로 닫기보다 앞이라야 사유가 최신이다"
        );
        assert!(events(&acts).iter().any(|e| matches!(
            e,
            MachineEvent::Disconnected {
                cause: DisconnectCause::Explicit,
                ..
            }
        )));
        assert_eq!(m.state(), PeerState::Dialing);
    }

    // ── D1: 세대는 기계가 짓지 않고 배정받는다 ──
    #[test]
    fn the_machine_adopts_the_generation_it_is_handed() {
        let t0 = Instant::now();
        let mut m = Machine::new(policy(), 0, t0, 40);
        assert_eq!(m.generation(), Generation(40), "바닥값이 첫 보고값이다");
        m.on(t0, Input::Start);
        m.on(t0, Input::Dialed);
        let acts = m.on(t0, Input::Connected(Generation(41)));
        assert_eq!(m.generation(), Generation(41));
        assert_eq!(
            events(&acts),
            vec![&MachineEvent::Connected {
                generation: Generation(41)
            }],
            "★기계가 스스로 +1 하면 같은 이름표의 두 감독이 같은 번호를 낸다★"
        );
    }

    #[test]
    fn reconnect_now_clears_the_budget_from_a_gave_up_state() {
        let (mut m, t0) = machine();
        m.on(t0, Input::Start);
        for _ in 0..3 {
            m.on(t0, Input::ConnectFailed(unreachable()));
            m.on(t0, Input::BackoffElapsed);
        }
        m.on(t0, Input::ConnectFailed(unreachable()));
        assert!(m.state().is_terminal());

        let acts = m.on(t0, Input::ReconnectNow);
        assert_eq!(m.state(), PeerState::Dialing);
        assert_eq!(m.attempt(), 0);
        assert!(acts.contains(&Action::Dial {
            deadline: t0 + Duration::from_secs(10)
        }));
    }

    #[test]
    fn reconnect_now_also_works_out_of_a_rejection() {
        let (mut m, t0) = machine();
        m.on(t0, Input::Start);
        m.on(t0, Input::Dialed);
        m.on(
            t0,
            Input::ConnectFailed(ConnectFailure::Rejected {
                code: None,
                reason: "no".into(),
            }),
        );
        m.on(t0, Input::ReconnectNow);
        assert_eq!(m.state(), PeerState::Dialing);
    }

    #[test]
    fn close_is_final_and_swallows_everything_after_it() {
        let (mut m, t0) = machine();
        go_live(&mut m, t0);
        let acts = m.on(t0, Input::Close);
        assert_eq!(m.state(), PeerState::Closed);
        assert!(acts.contains(&Action::FailPending(DisconnectCause::Explicit)));

        assert!(m.on(t0, Input::ReconnectNow).is_empty());
        assert!(m.on(t0, Input::Start).is_empty());
        assert!(m.on(t0, Input::Connected(Generation(1))).is_empty());
        assert_eq!(m.state(), PeerState::Closed);
    }

    #[test]
    fn closing_before_ever_being_live_emits_no_disconnect() {
        let (mut m, t0) = machine();
        m.on(t0, Input::Start);
        let acts = m.on(t0, Input::Close);
        assert!(events(&acts).is_empty());
    }

    #[test]
    fn inputs_that_do_not_belong_to_the_current_state_are_ignored() {
        let (mut m, t0) = machine();
        assert!(m.on(t0, Input::Dialed).is_empty());
        assert!(m.on(t0, Input::Connected(Generation(1))).is_empty());
        assert!(m.on(t0, Input::BackoffElapsed).is_empty());
        assert!(m.on(t0, Input::LinkLost(DisconnectCause::Idle)).is_empty());
        assert_eq!(m.state(), PeerState::Idle);
    }

    #[test]
    fn backoff_is_capped_and_jitter_stays_inside_its_band() {
        let t0 = Instant::now();
        let policy = Policy {
            reconnect: crate::policy::ReconnectPolicy {
                max_attempts: 40,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut m = Machine::new(policy.clone(), 0xDEAD_BEEF, t0, 0);
        m.on(t0, Input::Start);
        let mut seen = Vec::new();
        for step in 0..40u32 {
            let now = t0 + Duration::from_millis(step as u64);
            let acts = m.on(now, Input::ConnectFailed(unreachable()));
            for w in waits(&acts, now) {
                seen.push(w);
            }
            m.on(now, Input::BackoffElapsed);
        }
        assert_eq!(seen.len(), 40);
        for w in &seen {
            assert!(*w <= policy.reconnect.cap, "{w:?}");
        }
        assert!(seen[20] >= policy.reconnect.cap.mul_f64(0.8));
    }

    #[test]
    fn the_handshake_inherits_the_deadline_the_dial_was_given() {
        let (mut m, t0) = machine();
        let dial = m.on(t0, Input::Start);
        let deadline = match dial.as_slice() {
            [Action::Dial { deadline }] => *deadline,
            other => panic!("{other:?}"),
        };
        let shake = m.on(t0 + Duration::from_secs(3), Input::Dialed);
        assert_eq!(shake, vec![Action::Handshake { deadline }]);
    }

    #[test]
    fn a_dial_that_never_lands_never_reaches_the_handshake() {
        let (mut m, t0) = machine();
        m.on(t0, Input::Start);
        m.on(t0, Input::ConnectFailed(unreachable()));
        assert_eq!(m.state(), PeerState::Backoff);
        assert!(m.on(t0, Input::Connected(Generation(1))).is_empty());
    }

    #[test]
    fn different_peers_get_different_jitter_from_the_same_instant() {
        let t0 = Instant::now();
        let now = t0 + Duration::from_nanos(123_456);
        let mut a = Machine::new(Policy::default(), 1, t0, 0);
        let mut b = Machine::new(Policy::default(), 2, t0, 0);
        a.on(t0, Input::Start);
        b.on(t0, Input::Start);
        let wa = waits(&a.on(now, Input::ConnectFailed(unreachable())), now);
        let wb = waits(&b.on(now, Input::ConnectFailed(unreachable())), now);
        assert_ne!(wa, wb);
    }
}
