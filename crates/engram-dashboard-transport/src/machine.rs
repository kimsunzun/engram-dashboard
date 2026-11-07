//! State machine of one connection — ★pure layer★.
//!
//! Input = (current time, event) · output = list of actions. ★No future, no runtime, no I/O★ — so the backoff schedule and
//! the budget decision are measured **without really waiting**. The gate keeps that purity (canonical = `lib.rs` header).
//!
//! ★This file is this crate's only 「나중에 리팩터 안 하게」 (so we never refactor later) device★ — within one crate the compiler does not enforce sans-IO purity,
//! so it holds only by convention, and this repo already has a precedent that fills that weakness
//! (the purity gate of `src-tauri/src/daemon_client/replay_flight.rs`, ADR-0175 decision 3).
//!
//! ★It does **not execute** actions★ — opening, closing and waiting on links are all the supervisor task of [`crate::peer`].
//! What grows here is only variants of [`Action`] · [`Input`], and that does not shake the boundary.

use std::time::{Duration, Instant};

use crate::frame::CloseCode;
use crate::link::{LinkError, RejectKind};
use crate::mask::mask_reason;
use crate::policy::Policy;

/// Connection generation. Monotonically increasing and never reused. It is +1 every time it enters the operating phase.
///
/// ★It does not rewind even if the same label is put on the registry again★ — [`Machine::new`] takes a **floor value**, and the registry
/// passes the old supervisor's last generation as that floor. Otherwise the consumer's ordinary defense (「최대 세대 미만은
/// 무시」 (ignore anything below the max generation)) **throws away the new connection wholesale**.
///
/// ★It differs from the stream's incarnation marker ([`crate::StreamMark::generation`], u32)★ — this one is **which of our connections
/// this is**, while that one is **which incarnation of the peer's stream this is**. The width and the comparison rule differ
/// (here ordering comparison has meaning, there only match/mismatch is looked at).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Generation(pub u64);

/// The current state of one peer. It is exposed outside only through [`crate::Peer::state`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerState {
    Idle,
    Dialing,
    Handshaking,
    Live,
    Backoff,
    GaveUp(GaveUpReason),
    /// Explicit exit — it does not connect again.
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

/// Why it gave up. ★This is the input from which the shell decides popup eligibility★ (ADR-0180 decision 2) — both are 「사용자가 행동하지
/// 않으면 영영 복구되지 않는 상태」 (a state that never recovers unless the user acts) and so qualify, but what the user can do on the spot differs, so the screen
/// shape splits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GaveUpReason {
    /// Received a terminal rejection ([`RejectKind::Terminal`]) — it connected and was rejected, or the dialer knew it before dialing
    /// ([`crate::DialFailure::Refused`]). Retrying gives the same result.
    Rejected,
    /// Used up the reconnect budget.
    BudgetExhausted,
}

/// Why it disconnected. ★Today's code has no such distinction — everything is one lump of 「끊김」 (disconnected)★.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectCause {
    /// The peer exited normally.
    PeerClosed,
    /// The reading side ended — link reading produced an error, or it disconnected because what was read could not be read (the policy of disconnecting on decode failure).
    /// ★Telling it apart from [`DisconnectCause::WriteFailed`] is the point★ — the daemon adapter tells the read end (finishes the remaining commands)
    /// from the write end (disconnects right away) by this value (TRD 3 §3-0 ①).
    ReadFailed,
    /// The writing side ended — a frame · keepalive write produced an error (a deadline overrun is [`DisconnectCause::WriteDeadline`]).
    WriteFailed,
    /// The peer was quiet for `idle_timeout`. An accepted peer ([`crate::Registry::attach`]) failing to finish the handshake within the auth deadline is this too
    /// (the `cause` of `TransportEvent::Closed`).
    Idle,
    /// Exceeded `write_deadline` pushing one frame — for an accepted peer's handshake write, it exceeded the auth deadline.
    WriteDeadline,
    /// The queue was full and the policy was [`crate::QueueFullPolicy::Disconnect`].
    QueueFull,
    /// We closed it explicitly — for an accepted peer, our handshake rejecting it is this too.
    Explicit,
}

impl DisconnectCause {
    /// The close code to carry when disconnecting for this reason.
    pub fn close_code(self) -> CloseCode {
        match self {
            DisconnectCause::Idle => CloseCode::KEEPALIVE_TIMEOUT,
            DisconnectCause::WriteDeadline => CloseCode::WRITE_DEADLINE,
            DisconnectCause::QueueFull => CloseCode::QUEUE_FULL,
            _ => CloseCode::GOING_AWAY,
        }
    }
}

/// Why a connection could not stand. ★What splits the budget is not the branch but 「종결 거절인가」 (is it a terminal rejection)★ (ADR-0180 decision 5 · TRD 3 §3-0 ⑤) —
/// only an [`RejectKind::Terminal`] rejection spends no budget and gives up at the first rejection; the rest spend budget and dial again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectFailure {
    /// The link itself did not open → spends budget. ★The text may be text the peer gave too★ — a consumer [`crate::Dialer`] may carry the peer's
    /// response text over, so this text carried by an event that passed through the machine has also gone through [`mask_reason`].
    Unreachable(LinkError),
    /// The peer (or this side's handshake · the dialer) said it will not accept. `kind` splits the budget and `code` is the close code of that rejection
    /// that crossed the link (for the dialer's [`crate::DialFailure::Refused`], the code it chose).
    ///
    /// ★`reason` may be text the peer gave — neither decide on it nor use it as popup text★ (TRD 3 §3-0 ⑤ · §5 19). The screen
    /// text is the consumer's fixed message chosen by `code`, and this text is for details · logs. When carrying it onto an LLM surface, mark it as 「상대가 준
    /// 데이터」 (data the peer gave). This text carried by an event that passed through the machine has gone through [`mask_reason`].
    Rejected {
        kind: RejectKind,
        code: CloseCode,
        reason: String,
    },
}

impl ConnectFailure {
    /// Masks the text before carrying it in an event — both the rejection text and the could-not-connect text.
    fn masked(self) -> Self {
        match self {
            ConnectFailure::Rejected { kind, code, reason } => ConnectFailure::Rejected {
                kind,
                code,
                reason: mask_reason(&reason),
            },
            ConnectFailure::Unreachable(err) => {
                ConnectFailure::Unreachable(LinkError::new(mask_reason(&err.message)))
            }
        }
    }
}

/// What the supervisor task feeds into the machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// It was put on the registry.
    Start,
    /// The link opened — now the handshake.
    Dialed,
    /// The handshake finished. ★The supervisor draws the generation from the **shared counter per peer label** and carries it here★ —
    /// if the machine did +1 itself, an old and a new supervisor of the same label could emit the same number (D1).
    Connected(Generation),
    /// The dial or the handshake failed.
    ConnectFailed(ConnectFailure),
    /// Lost the link while operating.
    LinkLost(DisconnectCause),
    /// The backoff wait ended.
    BackoffElapsed,
    /// ★「지금 다시 해라」 (do it again now)★ — resets the budget and tries at once (ADR-0180 decision 7).
    ReconnectNow,
    /// Explicit exit.
    Close,
}

/// An event the machine instructs to raise upward. [`crate::peer`] attaches a source marker to it and turns it into
/// a [`crate::TransportEvent`].
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
        /// ★The **retry** budget used so far★ (1-based). The first connection attempt is not a retry and is not counted, so
        /// if `retry_in` is `Some`, read it as "now do the `attempt`th retry after that long", and if `None`, read it as
        /// "used up `attempt`/`of`".
        attempt: u32,
        of: u32,
        cause: ConnectFailure,
        /// `None` = the budget is used up so it waits no more — that is all it means. A terminal rejection does not raise this event
        /// (it goes as [`MachineEvent::Rejected`] · [`MachineEvent::GaveUp`]).
        retry_in: Option<Duration>,
    },
    /// ★Raises only a terminal rejection ([`RejectKind::Terminal`])★ — a rejection that is retried goes as the `cause` of [`MachineEvent::ConnectFailed`].
    /// `reason` is masked text ([`ConnectFailure::Rejected`]).
    Rejected {
        code: CloseCode,
        reason: String,
    },
    GaveUp {
        reason: GaveUpReason,
    },
}

/// What the machine instructs. The supervisor task carries it out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Opens a link. `deadline` wraps **dial and handshake together** (preserves the current `HANDSHAKE_TIMEOUT`).
    Dial { deadline: Instant },
    /// Runs the handshake over the open link. `deadline` is exactly the one [`Action::Dial`] set.
    Handshake { deadline: Instant },
    /// Closes and discards the current link. The close code to carry is `close`.
    DropLink { code: CloseCode },
    /// Waits until this time and then feeds in [`Input::BackoffElapsed`].
    Wait { until: Instant },
    /// Wakes all waiting requests with this reason.
    FailPending(DisconnectCause),
    /// The event to raise upward.
    Emit(MachineEvent),
}

/// The lifetime of one connection.
#[derive(Debug)]
pub struct Machine {
    state: PeerState,
    generation: u64,
    /// ★Budget spent on failing to connect★ — the number of backoff waits reserved since the last successful connection (or [`Input::ReconnectNow`]).
    /// It returns to 0 when a connection succeeds.
    retries: u32,
    /// ★Consecutive count of connecting and soon disconnecting★ — the number of connections in a row that failed to fill `live_dwell`. One
    /// connection that lasted long resets it to 0.
    ///
    /// Why there are two axes: merge them into one and **one of the two is bound to break**. If the budget is refilled completely on every connect,
    /// a peer that disconnects as soon as it connects repeats forever and never reaches [`GaveUpReason::BudgetExhausted`] (no popup
    /// appears), and if it is not refilled, there is a place where it gives up **without ever having retried** — the shape where the daemon takes 4 seconds to
    /// restart and burns the whole budget during that time, and then one disconnect 10 seconds after connecting makes it give up at once.
    /// That state is not 「사용자가 행동하지 않으면 영영 복구되지 않는」 (never recovers unless the user acts); it is one that **connects if tried just once more**,
    /// so it has no popup eligibility (ADR-0180 decision 2).
    flaps: u32,
    policy: Policy,
    /// The dial+handshake deadline of the current attempt. The machine holds it because the two stretches share one budget.
    connect_deadline: Instant,
    /// The time the current connection entered the operating phase. Eligibility to refill the budget is measured by it.
    live_since: Option<Instant>,
    /// The time axis of the jitter scale. `Instant` does not expose its internal representation, so the difference from a reference point is used.
    epoch: Instant,
    /// A seed that makes each peer get a different scale. The caller draws it from `PeerId`.
    jitter_seed: u64,
}

impl Machine {
    /// `generation_floor` = this peer's generation continues from here (the value left by an old supervisor of the same label).
    /// A peer being set up for the first time has 0.
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

    /// Budget spent on failing to connect.
    pub fn attempt(&self) -> u32 {
        self.retries
    }

    /// Consecutive count of connecting and soon disconnecting.
    pub fn flaps(&self) -> u32 {
        self.flaps
    }

    /// Feeds in one event and receives the work to do.
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
                // Wake the waiters first — DropLink can be delayed by as much as the write deadline, and in that time a stale answer goes out to
                //   a caller asking for the reason.
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
                // ★If the waiters are not woken here, nobody wakes them★ — if the next dial burns the budget and
                //   lands on GaveUp, this machine never looks at a deadline again, and that slot becomes a permanent wait.
                //   Close · LinkLost · rejection all emit FailPending, and only this branch did not — that was the defect.
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
                    // ★If it connected, the 「못 붙는다」 (cannot connect) budget fills again★ — what that axis measures is reachability,
                    //   and it was just reached. Catching a peer that disconnects as soon as it connects is the `flaps` axis below.
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
                        // One long-lasting connection clears the 「금세 끊긴다」 (soon disconnects) record.
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
            ConnectFailure::Rejected {
                kind: RejectKind::Terminal,
                code,
                reason,
            } => {
                self.state = PeerState::GaveUp(GaveUpReason::Rejected);
                self.live_since = None;
                vec![
                    Action::FailPending(DisconnectCause::PeerClosed),
                    Action::Emit(MachineEvent::Rejected {
                        code,
                        reason: mask_reason(&reason),
                    }),
                    Action::Emit(MachineEvent::GaveUp {
                        reason: GaveUpReason::Rejected,
                    }),
                    Action::DropLink {
                        code: CloseCode::HANDSHAKE_REJECTED,
                    },
                ]
            }
            // A rejection that is retried takes the same path as failing to connect — it spends budget (TRD 3 §3-0 ⑤).
            retry => {
                let mut acts = vec![Action::DropLink {
                    code: CloseCode::GOING_AWAY,
                }];
                acts.extend(self.schedule_retry(now, Some(retry.masked())));
                acts
            }
        }
    }

    /// Sets the next attempt or declares the budget exhausted. If there is a `cause`, it raises `ConnectFailed` too.
    fn schedule_retry(&mut self, now: Instant, cause: Option<ConnectFailure>) -> Vec<Action> {
        let of = self.policy.reconnect.max_attempts;
        // ★If either of the two axes runs out, it gives up★ — `retries` measures 「못 붙는다」 (cannot connect), `flaps` measures 「붙어도 안
        //   버틴다」 (connects but does not last). Either is a state that does not clear unless the user steps in.
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

    /// A jitter scale in [0,1). ★No new dependency is brought in★ — it mixes the per-peer seed and the generation into the clock's low-order nanoseconds
    /// (precedent = the place where `profile.rs` of `agent` uses uuid v4 bytes instead of a random-number crate).
    ///
    /// **Two limits:** ① it is not a cryptographic random number (not needed for jitter) ② under a manual clock it becomes deterministic —
    /// a property the harness **wants**, but that is why the *variance* of jitter can be observed only on a real socket.
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
    use crate::mask::REASON_MAX_BYTES;

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

    /// Mimics what the supervisor does — ★the generation is assigned **from outside**, not by the machine★ (D1).
    fn connect(m: &mut Machine, now: Instant) -> Vec<Action> {
        let minted = Generation(m.generation().0 + 1);
        m.on(now, Input::Connected(minted))
    }

    /// Pushes Idle → Dialing → Handshaking → Live in one go.
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
                kind: RejectKind::Terminal,
                code: CloseCode::HANDSHAKE_REJECTED,
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

    fn retryable(reason: &str) -> ConnectFailure {
        ConnectFailure::Rejected {
            kind: RejectKind::Retryable,
            code: CloseCode::HANDSHAKE_RETRYABLE,
            reason: reason.into(),
        }
    }

    #[test]
    fn a_retryable_rejection_spends_budget_and_redials_like_unreachability() {
        let (mut m, t0) = machine();
        m.on(t0, Input::Start);
        m.on(t0, Input::Dialed);
        let acts = m.on(t0, Input::ConnectFailed(retryable("stale token")));
        assert_eq!(m.state(), PeerState::Backoff);
        assert_eq!(m.attempt(), 1, "예산을 쓴다");
        assert_eq!(waits(&acts, t0), vec![Duration::from_millis(500)]);
        let evs = events(&acts);
        assert!(
            !evs.iter().any(|e| matches!(
                e,
                MachineEvent::Rejected { .. } | MachineEvent::GaveUp { .. }
            )),
            "다시 거는 거절은 포기하지 않는다: {evs:?}"
        );
        assert!(evs.iter().any(|e| matches!(
            e,
            MachineEvent::ConnectFailed {
                cause: ConnectFailure::Rejected {
                    kind: RejectKind::Retryable,
                    ..
                },
                retry_in: Some(_),
                ..
            }
        )));
        assert!(matches!(
            m.on(t0 + Duration::from_millis(500), Input::BackoffElapsed)
                .as_slice(),
            [Action::Dial { .. }]
        ));
    }

    #[test]
    fn retryable_rejections_end_in_budget_exhaustion_not_in_a_rejection() {
        let (mut m, t0) = machine();
        m.on(t0, Input::Start);
        let mut acts = Vec::new();
        for _ in 0..4 {
            m.on(t0, Input::Dialed);
            acts = m.on(t0, Input::ConnectFailed(retryable("stale token")));
            m.on(t0, Input::BackoffElapsed);
        }
        assert_eq!(m.state(), PeerState::GaveUp(GaveUpReason::BudgetExhausted));
        assert!(events(&acts).iter().any(|e| matches!(
            e,
            MachineEvent::GaveUp {
                reason: GaveUpReason::BudgetExhausted
            }
        )));
    }

    #[test]
    fn both_rejection_paths_mask_the_reason_before_it_leaves_the_machine() {
        let noisy = format!(
            "x\u{1b}]0;pwn\u{7}\n\u{2028}\u{e0041}{}",
            "y".repeat(REASON_MAX_BYTES)
        );
        let reason_of = |kind: RejectKind| {
            let (mut m, t0) = machine();
            m.on(t0, Input::Start);
            m.on(t0, Input::Dialed);
            let acts = m.on(
                t0,
                Input::ConnectFailed(ConnectFailure::Rejected {
                    kind,
                    code: kind.close_code(),
                    reason: noisy.clone(),
                }),
            );
            events(&acts)
                .into_iter()
                .find_map(|e| match e {
                    MachineEvent::Rejected { reason, .. }
                    | MachineEvent::ConnectFailed {
                        cause: ConnectFailure::Rejected { reason, .. },
                        ..
                    } => Some(reason.clone()),
                    _ => None,
                })
                .expect("거절 문구를 실은 사건")
        };
        for kind in [RejectKind::Terminal, RejectKind::Retryable] {
            assert_eq!(reason_of(kind), mask_reason(&noisy), "{kind:?}");
        }
    }

    #[test]
    fn an_unreachable_message_is_masked_on_every_connect_failed_event() {
        // A failure to connect where a consumer dialer carried the peer's response text — masks both the retry event and the budget-exhausted event.
        let noisy = format!(
            "502 from proxy\r\n\u{2028}\u{e0069}\u{200b}\u{1b}[2J{}",
            "z".repeat(REASON_MAX_BYTES)
        );
        let (mut m, t0) = machine();
        m.on(t0, Input::Start);
        let mut seen = Vec::new();
        for _ in 0..4 {
            let acts = m.on(
                t0,
                Input::ConnectFailed(ConnectFailure::Unreachable(LinkError::new(noisy.clone()))),
            );
            seen.extend(events(&acts).into_iter().filter_map(|e| match e {
                MachineEvent::ConnectFailed {
                    cause: ConnectFailure::Unreachable(err),
                    retry_in,
                    ..
                } => Some((err.message.clone(), retry_in.is_some())),
                _ => None,
            }));
            m.on(t0, Input::BackoffElapsed);
        }
        assert_eq!(m.state(), PeerState::GaveUp(GaveUpReason::BudgetExhausted));
        assert_eq!(
            seen.iter().map(|(_, retry)| *retry).collect::<Vec<_>>(),
            vec![true, true, true, false]
        );
        for (message, _) in seen {
            assert_eq!(message, mask_reason(&noisy));
        }
    }

    #[test]
    fn rejection_and_unreachability_land_in_different_states_from_the_same_attempt_count() {
        let (mut rejected, t0) = machine();
        rejected.on(t0, Input::Start);
        rejected.on(t0, Input::Dialed);
        rejected.on(
            t0,
            Input::ConnectFailed(ConnectFailure::Rejected {
                kind: RejectKind::Terminal,
                code: CloseCode::HANDSHAKE_REJECTED,
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

    // ── N8: the budget fills by 「버텼다」 (it lasted), not by 「붙었다」 (it connected) ──
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
        // Repeats disconnecting as soon as it connects — never once fills the dwell.
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

    // ── B4: a peer that could not connect for a long time and finally connected must be tried once more ──
    #[test]
    fn a_peer_that_reconnected_after_a_long_outage_still_gets_a_retry() {
        let (mut m, t0) = machine();
        let mut now = t0;
        m.on(now, Input::Start);
        // The daemon restart is long and burns the whole 「못 붙는다」 (cannot connect) budget.
        for _ in 0..3 {
            m.on(now, Input::ConnectFailed(unreachable()));
            assert_eq!(m.state(), PeerState::Backoff);
            m.on(now, Input::BackoffElapsed);
        }
        assert_eq!(m.attempt(), 3);
        // Then it finally connects.
        m.on(now, Input::Dialed);
        connect(&mut m, now);
        assert_eq!(m.state(), PeerState::Live);
        assert_eq!(m.attempt(), 0, "붙었으면 「못 붙는다」 예산은 다시 찬다");

        // It fails to fill dwell (30s) and disconnects once.
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

    // ── C1: whichever branch drops the connection, wake the waiters ──
    #[test]
    fn every_branch_that_abandons_a_link_wakes_the_waiters() {
        let abandons = [
            Input::Close,
            Input::ReconnectNow,
            Input::LinkLost(DisconnectCause::Idle),
            Input::ConnectFailed(ConnectFailure::Rejected {
                kind: RejectKind::Terminal,
                code: CloseCode::HANDSHAKE_REJECTED,
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

    // ── D1: the generation is not built by the machine but assigned to it ──
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
                kind: RejectKind::Terminal,
                code: CloseCode::HANDSHAKE_REJECTED,
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
