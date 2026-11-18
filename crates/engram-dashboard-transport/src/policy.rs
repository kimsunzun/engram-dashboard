//! Policy values — ★every one has a crate default and the consumer overrides it★ (ADR-0177 decision 4): [`crate::Registry::new`] takes the registry defaults, [`crate::Registry::add_with_policy`] per-peer
//! overrides. ★A value chosen without grounds is marked 「unverified」 at its spot★ (else a guess reads as a measurement): `request_timeout` · `write_deadline` · `reconnect.jitter` · `inbound_queue` ·
//! `frame_buffer` · `retired_tags` · `max_streams` · `resume_timeout` · `live_dwell` · `flush_timeout`. The rest preserve current code values.

use std::time::Duration;

/// When the queue is full — ★applies only to the outbound queue★ (a full upward line means the consumer is slow, handled as backpressure — ADR-0310 decision 17).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueFullPolicy {
    /// Cuts that connection (kept as is — ADR-0177 decision 6); better to re-attach and sync from the start. ★Per-message dropping (drop oldest/newest) cannot be used on terminal bytes★ — a dropped middle
    /// breaks UTF-8 and desyncs the ANSI parser, so the screen stays garbled.
    Disconnect,
    /// Drops only that message and reports [`crate::TransportEvent::Dropped`]. ★Not creating a silent loss is the whole of this branch★ — not counting would be worse than `Disconnect`.
    DropAndReport,
}

/// When [`crate::Wire::decode`] fails. ★Default [`DecodeFailurePolicy::DropFrame`] is a judgment about how the user feels★ — one broken frame should not cut the whole screen; **cost = it can keep running with
/// the protocol out of sync** (TRD §12-11 is the open question; the default may change with its answer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeFailurePolicy {
    DropFrame,
    Disconnect,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReconnectPolicy {
    /// ★The **number of retries** allowed after the budget recovers★ — that many backoff waits, then [`crate::GaveUpReason::BudgetExhausted`].
    pub max_attempts: u32,
    pub base: Duration,
    pub cap: Duration,
    pub multiplier: f64,
    /// Wobble proportion mixed into the wait (0.2 = ±20%). ★Unverified★ — borrowed from gRPC's *"Proposed Backoff Algorithm"* example; the old code had no jitter.
    pub jitter: f64,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            // ADR-0180 decision 4 (user decision). The old code is 5 — the layers diverging is known; aligning is the job of the stage that attaches this crate.
            max_attempts: 3,
            // Preserved from `BACKOFF_BASE` in `connection.rs`.
            base: Duration::from_millis(500),
            // Preserved from `BACKOFF_CAP` in `connection.rs`.
            cap: Duration::from_secs(10),
            // Preserved: 3 retries = 500ms→1s→2s = 3.5 s (the figure ADR-0180 cited).
            multiplier: 2.0,
            jitter: 0.2,
        }
    }
}

impl ReconnectPolicy {
    /// The wait before the `attempt`-th (0-based) retry; `r` = a wobble scale in [0,1) made by the caller (no random-number crate here; see [`crate::machine`]).
    /// ★Broken values never die here★ — a process with a panic hook ends on an uncontained panic (ADR-0292), and `Duration::from_secs_f64(NaN)` would take down the **whole process**; the policy is
    /// read from a file, so NaN · negative · jitter ≥ 1 are reachable and folded here. ★The cap is applied again **after** the jitter★ — near `cap` the upward wobble is cut and the distribution leans
    /// one way, judged better than exceeding the cap.
    pub fn delay(&self, attempt: u32, r: f64) -> Duration {
        // ★If the cap is below the floor, **lower the floor**★ — `cap.max(base)` discarded the cap (`base=10s, cap=1s` gave 10 s); not exceeding a thing named 「cap」 is less surprising. `check()` rejects it outright.
        let floor = MIN_BACKOFF.as_secs_f64();
        let cap = sane_secs(self.cap.as_secs_f64(), 10.0).max(floor);
        let base = sane_secs(self.base.as_secs_f64(), 0.5).clamp(floor, cap);
        // A multiplier under 1 **shrinks** the backoff into a dial busy loop.
        let multiplier = if self.multiplier.is_finite() && self.multiplier >= 1.0 {
            self.multiplier
        } else {
            1.0
        };
        // A jitter ≥ 1 can make the wait 0 — the same busy loop.
        let jitter = if self.jitter.is_finite() {
            self.jitter.clamp(0.0, 0.99)
        } else {
            0.0
        };
        let nudge = if r.is_finite() {
            r.clamp(0.0, 1.0)
        } else {
            0.5
        };

        // ★Do not pass `attempt as i32` directly★ — `u32::MAX` becomes `-1`, flipping the exponent and making the backoff **shorter** than base (measured: 500ms → 250ms); it saturates at the cap anyway.
        let grown = base * multiplier.powi(attempt.min(1024) as i32);
        let capped = if grown.is_finite() {
            grown.min(cap)
        } else {
            cap
        };
        let jittered = capped * (1.0 + jitter * (2.0 * nudge - 1.0));
        if jittered.is_finite() {
            // ★A floor is required★ — 「not zero」 is not enough: a 1ns backoff is a dial busy loop.
            Duration::from_secs_f64(jittered.clamp(floor, cap))
        } else {
            Duration::from_secs_f64(cap)
        }
    }
}

/// Whatever the policy, the backoff never gets shorter than this (faster retry is a busy loop, not a wait).
pub const MIN_BACKOFF: Duration = Duration::from_millis(1);

fn sane_secs(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    /// Upper limit on waiting for a reply; one value for every command kind (ADR-0181 decision 1). ★Unverified★ — real agent spawn time never measured (the ADR's only real risk, 8 s).
    pub request_timeout: Duration,
    /// The limit wrapping the dial and the handshake **together** (as the old `HANDSHAKE_TIMEOUT`). ★For an accepted peer ([`crate::Registry::attach`]) it is the auth deadline★, measured from the end of the
    /// upgrade ([`crate::Accepted::upgraded_at`], TRD 3 §5 3); an accepted peer does not redial, so it ignores [`Policy::reconnect`] · [`Policy::live_dwell`].
    pub connect_timeout: Duration,
    /// Limit for pushing one frame. ★Unverified — only industry reference values★ (NATS 10s · nginx 60s · gRPC keepalive 20s). Invariant: `write_deadline < ping_interval`, else one write starves keepalive
    /// (the deleted `net` crate did).
    pub write_deadline: Duration,
    /// Active ping interval (current daemon value preserved).
    pub ping_interval: Duration,
    /// If the peer is this quiet, cut (current daemon value preserved — 2.5 times the ping interval).
    pub idle_timeout: Duration,
    /// ★Only a connection that stayed operating this long refills the reconnect budget★ — without it a peer cut right after attaching reconnects every 500 ms **forever** and the ADR-0180 decision 7 popup
    /// **never appears**. ★Unverified — real reconnect time never measured★; the old shell has no such concept.
    pub live_dwell: Duration,
    pub reconnect: ReconnectPolicy,
    /// Outbound queue length (current shell value preserved).
    pub outbound_queue: usize,
    /// ★One handle's★ receiver-end ([`crate::Inbox`]) length (count): every [`crate::Registry::add`] · [`crate::Registry::attach`] sets up a line of this length (re-raising a name tag is a new line). Total held =
    /// the sum over live receiver ends, including old ones whose supervisor has not finished teardown. ★Count only★ — bytes are the supervisor's crate-internal budget (16 MiB; `budget.rs`, `lib.rs` 「known
    /// limits」). Full = that supervisor holds in its ordering area and stops reading that peer (no drop). ★Unverified — no counterpart today★ (the shell goes straight to a Tauri Channel); this crate **creates one new
    /// failure mode** (TRD §12-12).
    pub inbound_queue: usize,
    /// One connection's read-ahead buffer (count). ★A **different resource** from [`Policy::inbound_queue`]★: this one full stops the read task (transport backpressure); that one full makes the supervisor
    /// hold events · packets in its ordering area and stop draining this one, so this soon fills too. Bytes: see `inbound_queue`. ★Unverified★.
    pub frame_buffer: usize,
    pub full_queue: QueueFullPolicy,
    pub decode_failure: DecodeFailurePolicy,
    /// How many expired pairing numbers to remember — the length of [`crate::Pending`]'s bounded ring. ★Unverified★.
    pub retired_tags: usize,
    /// Limit on streams whose sequence numbers are tracked — ★the only collection here whose size the peer decides★ (unbounded, a peer inventing keys would grow memory without bound). Overflow forgets the
    /// oldest registered; a forgotten stream rebuilds its dedup ledger from scratch. ★Unverified★.
    pub max_streams: usize,
    /// Upper limit on waiting for the answer (piece `after+1`) after a resume request. ★Separates 「the peer cannot give it」 from 「not come yet」★ — an out-of-order piece meanwhile may be one the peer sent before
    /// the re-request, so it is dropped and we wait; only past this deadline is [`crate::TransportEvent::StreamTruncated`] raised. ★The clock stops while that peer's reading is stopped by backpressure★
    /// (`lib.rs` 「Receiver-side invariants」 2). ★Unverified★.
    pub resume_timeout: Duration,
    /// Upper limit on pieces one stream **holds back** while awaiting a re-request's answer (held to keep order, not droppable; unbounded the peer would push our memory). Full = the window closes at once and
    /// held pieces go out in order. In bytes, the supervisor's window cap (8 MiB — half the byte budget, summed over all streams) closes it the same way; a new hole then opens no window and truncates at once
    /// (`lib.rs` 「known limits」). ★Unverified★.
    pub resume_buffer: usize,
    /// The deadline one [`crate::Peer::close_after_flush`] uses — from the supervisor's notice it covers the writes before the marker · keepalive · close frame **together**; past it, it closes without writing the
    /// rest. ★Not replaceable by [`Policy::write_deadline`]★ (per write — a slow reader could hold the close for queue length × write deadline; TRD 3 §3-0 ④). ★Unverified★ — TRD 3 §3-0 ④'s draft value.
    pub flush_timeout: Duration,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(8),
            connect_timeout: Duration::from_secs(10),
            write_deadline: Duration::from_secs(5),
            ping_interval: Duration::from_secs(20),
            idle_timeout: Duration::from_secs(50),
            live_dwell: Duration::from_secs(30),
            reconnect: ReconnectPolicy::default(),
            outbound_queue: 512,
            inbound_queue: 512,
            frame_buffer: 512,
            full_queue: QueueFullPolicy::Disconnect,
            decode_failure: DecodeFailurePolicy::DropFrame,
            retired_tags: 256,
            max_streams: 1024,
            resume_timeout: Duration::from_secs(2),
            resume_buffer: 256,
            flush_timeout: Duration::from_secs(2),
        }
    }
}

impl Policy {
    /// Do the values contradict each other; returns the human-readable name of the offending item. ★The crate does not die without this passing★ ([`ReconnectPolicy::delay`] folds broken values); it exists so
    /// **the consumer fails loudly when reading its settings** — the registry only `debug_assert`s it.
    pub fn check(&self) -> Result<(), &'static str> {
        if self.write_deadline >= self.ping_interval {
            return Err("write_deadline < ping_interval");
        }
        if self.idle_timeout <= self.ping_interval {
            return Err("ping_interval < idle_timeout");
        }
        if self.outbound_queue == 0 || self.inbound_queue == 0 || self.frame_buffer == 0 {
            return Err("queue lengths must be non-zero");
        }
        if self.max_streams == 0 || self.resume_buffer == 0 {
            return Err("max_streams / resume_buffer must be non-zero");
        }
        if self.flush_timeout.is_zero() {
            return Err("flush_timeout must be non-zero");
        }
        if self.reconnect.base < MIN_BACKOFF {
            return Err("reconnect.base >= MIN_BACKOFF");
        }
        if self.reconnect.cap < self.reconnect.base {
            return Err("reconnect.base <= reconnect.cap");
        }
        if !(self.reconnect.multiplier.is_finite() && self.reconnect.multiplier >= 1.0) {
            return Err("reconnect.multiplier >= 1.0");
        }
        if !(self.reconnect.jitter.is_finite() && (0.0..1.0).contains(&self.reconnect.jitter)) {
            return Err("0.0 <= reconnect.jitter < 1.0");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_satisfy_their_own_invariants() {
        assert_eq!(Policy::default().check(), Ok(()));
    }

    #[test]
    fn defaults_are_the_values_the_trd_table_names() {
        let p = Policy::default();
        assert_eq!(p.request_timeout, Duration::from_secs(8));
        assert_eq!(p.connect_timeout, Duration::from_secs(10));
        assert_eq!(p.write_deadline, Duration::from_secs(5));
        assert_eq!(p.ping_interval, Duration::from_secs(20));
        assert_eq!(p.idle_timeout, Duration::from_secs(50));
        assert_eq!(p.outbound_queue, 512);
        assert_eq!(p.inbound_queue, 512);
        assert_eq!(p.retired_tags, 256);
        assert_eq!(p.full_queue, QueueFullPolicy::Disconnect);
        assert_eq!(p.reconnect.max_attempts, 3);
        assert_eq!(p.flush_timeout, Duration::from_secs(2));
    }

    #[test]
    fn write_deadline_may_not_starve_keepalive() {
        let mut p = Policy::default();
        p.write_deadline = p.ping_interval;
        assert!(p.check().is_err());
    }

    #[test]
    fn a_zero_flush_timeout_is_rejected() {
        let p = Policy {
            flush_timeout: Duration::ZERO,
            ..Policy::default()
        };
        assert_eq!(p.check(), Err("flush_timeout must be non-zero"));
    }

    #[test]
    fn idle_timeout_below_ping_interval_is_rejected() {
        let mut p = Policy::default();
        p.idle_timeout = p.ping_interval;
        assert!(p.check().is_err());
    }

    #[test]
    fn backoff_without_jitter_is_the_documented_schedule() {
        let policy = ReconnectPolicy {
            jitter: 0.0,
            ..ReconnectPolicy::default()
        };
        assert_eq!(policy.delay(0, 0.0), Duration::from_millis(500));
        assert_eq!(policy.delay(1, 0.0), Duration::from_millis(1000));
        assert_eq!(policy.delay(2, 0.0), Duration::from_millis(2000));
        let total: Duration = (0..3).map(|n| policy.delay(n, 0.0)).sum();
        assert_eq!(total, Duration::from_millis(3500));
    }

    // ── F2: a combination where the cap is smaller than the floor ──
    #[test]
    fn a_cap_below_the_base_is_rejected_and_never_exceeded() {
        let policy = ReconnectPolicy {
            base: Duration::from_secs(10),
            cap: Duration::from_secs(1),
            jitter: 0.0,
            ..ReconnectPolicy::default()
        };
        let p = Policy {
            reconnect: policy,
            ..Policy::default()
        };
        assert!(p.check().is_err(), "check must not accept this combination");
        for attempt in [0u32, 1, 5] {
            assert!(
                policy.delay(attempt, 0.0) <= policy.cap,
                "★a declared cap must not be silently dropped★"
            );
        }
    }

    // ── F3: a floor value that becomes a busy loop ──
    #[test]
    fn a_nanosecond_base_is_rejected_and_never_produces_a_hot_loop() {
        let policy = ReconnectPolicy {
            base: Duration::from_nanos(1),
            jitter: 0.0,
            ..ReconnectPolicy::default()
        };
        let p = Policy {
            reconnect: policy,
            ..Policy::default()
        };
        assert!(p.check().is_err());
        for attempt in [0u32, 1, 5] {
            assert!(
                policy.delay(attempt, 0.0) >= MIN_BACKOFF,
                "a 1ns backoff is not a wait but a hot dial loop"
            );
        }
    }

    #[test]
    fn every_poisoned_policy_still_respects_the_floor() {
        let poisons = [
            ReconnectPolicy {
                base: Duration::ZERO,
                ..Default::default()
            },
            ReconnectPolicy {
                base: Duration::from_nanos(3),
                cap: Duration::from_nanos(9),
                ..Default::default()
            },
            ReconnectPolicy {
                cap: Duration::ZERO,
                ..Default::default()
            },
        ];
        for policy in poisons {
            for attempt in [0u32, 3, u32::MAX] {
                assert!(policy.delay(attempt, 0.5) >= MIN_BACKOFF, "{policy:?}");
            }
        }
    }

    #[test]
    fn backoff_saturates_at_the_cap() {
        let policy = ReconnectPolicy {
            jitter: 0.0,
            ..ReconnectPolicy::default()
        };
        assert_eq!(policy.delay(30, 0.0), policy.cap);
        assert_eq!(policy.delay(u32::MAX, 0.0), policy.cap);
    }

    #[test]
    fn jitter_moves_the_delay_within_its_band() {
        let policy = ReconnectPolicy::default();
        let low = policy.delay(1, 0.0);
        let mid = policy.delay(1, 0.5);
        let high = policy.delay(1, 1.0);
        assert_eq!(mid, Duration::from_millis(1000));
        assert_eq!(low, Duration::from_millis(800));
        assert_eq!(high, Duration::from_millis(1200));
        assert!(low < mid && mid < high);
    }

    #[test]
    fn jitter_never_exceeds_the_cap() {
        let policy = ReconnectPolicy::default();
        for n in 0..40 {
            assert!(policy.delay(n, 1.0) <= policy.cap);
        }
    }

    // ── A broken policy value does not take down the process ──
    #[test]
    fn a_poisoned_policy_never_panics_and_never_yields_a_zero_wait() {
        let poisons = [
            ReconnectPolicy {
                jitter: f64::NAN,
                ..Default::default()
            },
            ReconnectPolicy {
                jitter: 1.0,
                ..Default::default()
            },
            ReconnectPolicy {
                jitter: 25.0,
                ..Default::default()
            },
            ReconnectPolicy {
                jitter: -3.0,
                ..Default::default()
            },
            ReconnectPolicy {
                multiplier: f64::NAN,
                ..Default::default()
            },
            ReconnectPolicy {
                multiplier: 0.1,
                ..Default::default()
            },
            ReconnectPolicy {
                multiplier: f64::INFINITY,
                ..Default::default()
            },
            ReconnectPolicy {
                base: Duration::ZERO,
                ..Default::default()
            },
            ReconnectPolicy {
                cap: Duration::ZERO,
                ..Default::default()
            },
        ];
        for policy in poisons {
            for attempt in [0u32, 1, 7, u32::MAX] {
                for r in [0.0, 0.5, 1.0, f64::NAN] {
                    let d = policy.delay(attempt, r);
                    assert!(d > Duration::ZERO, "{policy:?} attempt={attempt} r={r}");
                    assert!(d <= Duration::from_secs(3600));
                }
            }
        }
    }

    #[test]
    fn check_rejects_the_values_delay_has_to_defend_against() {
        for jitter in [f64::NAN, 1.0, 25.0, -3.0] {
            let p = Policy {
                reconnect: ReconnectPolicy {
                    jitter,
                    ..Default::default()
                },
                ..Policy::default()
            };
            assert!(p.check().is_err(), "jitter={jitter}");
        }
        let p = Policy {
            reconnect: ReconnectPolicy {
                multiplier: 0.5,
                ..Default::default()
            },
            ..Policy::default()
        };
        assert!(p.check().is_err());
    }

    #[test]
    fn the_upward_line_and_the_read_ahead_are_two_values() {
        let p = Policy {
            inbound_queue: 8,
            frame_buffer: 4096,
            ..Policy::default()
        };
        assert_eq!(p.check(), Ok(()));
        assert_ne!(p.inbound_queue, p.frame_buffer);
    }
}
