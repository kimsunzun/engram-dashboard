//! Policy values — ★every one has a crate default and the consumer overrides it★ (ADR-0177 decision 4).
//!
//! [`crate::Registry::new`] takes the registry defaults and [`crate::Registry::add_with_policy`] takes the per-peer
//! overrides.
//!
//! ★Where a value was chosen without grounds, write 「미검」 (unverified) at that spot★ — if not written, the next session
//! reads a guess as a measurement. The unverified ones: `request_timeout` · `write_deadline` · `reconnect.jitter` · `inbound_queue` ·
//! `frame_buffer` · `retired_tags` · `max_streams` · `resume_timeout` · `live_dwell` · `flush_timeout`.
//! The rest preserve current code values.

use std::time::Duration;

/// When the queue is full. ★It applies only to the outbound queue★ — the upward line filling means the consumer cannot keep up with that
/// peer, not that the connection is at fault, and the disposition was decided as backpressure, not cutting (ADR-0310 decision 17).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueFullPolicy {
    /// it cuts that connection (kept as is — ADR-0177 decision 6). It is better to re-attach and sync from the start.
    ///
    /// ★The plan of dropping per message (drop oldest/newest) cannot be used on terminal bytes★ — dropping the middle breaks UTF-8
    /// and desyncs the ANSI parser state, so the screen keeps being drawn broken.
    Disconnect,
    /// Drop only that message and report it as [`crate::TransportEvent::Dropped`]. ★Not creating a silent loss is the whole of
    /// this branch★ — if it stopped counting, it would be a worse choice than `Disconnect`.
    DropAndReport,
}

/// When [`crate::Wire::decode`] fails.
///
/// ★The default being [`DecodeFailurePolicy::DropFrame`] is a judgment about how the user feels★ — one broken frame
/// should not cut the whole screen. **Cost = a combination arises where it keeps running with the protocol out of sync** (TRD §12-11
/// is the open question, and once its answer is settled the default may change).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeFailurePolicy {
    DropFrame,
    Disconnect,
}

/// Reconnect budget and backoff schedule.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReconnectPolicy {
    /// ★The **number of retries** allowed after the budget recovers★. There are this many backoff waits, and if it fails again after that
    /// it is [`crate::GaveUpReason::BudgetExhausted`].
    pub max_attempts: u32,
    pub base: Duration,
    pub cap: Duration,
    pub multiplier: f64,
    /// The proportion of wobble mixed into the wait time (0.2 = ±20%). ★Unverified★ — borrowed from the example value of a document gRPC itself titled *"Proposed Backoff
    /// Algorithm"*. **Today's code has no jitter at all.**
    pub jitter: f64,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            // ADR-0180 decision 4 (user decision). Today's code is 5 — the two layers diverging is a known state and
            //   aligning them is the job of the stage that attaches this crate.
            max_attempts: 3,
            // Current value preserved — BACKOFF_BASE in `connection.rs`.
            base: Duration::from_millis(500),
            // Current value preserved — BACKOFF_CAP in `connection.rs`.
            cap: Duration::from_secs(10),
            // Current value preserved — 3 times = 500ms→1s→2s = total 3.5 seconds (the figure ADR-0180 cited).
            multiplier: 2.0,
            jitter: 0.2,
        }
    }
}

impl ReconnectPolicy {
    /// The wait before the `attempt`-th (0-based) retry.
    ///
    /// `r` is a wobble scale in [0,1) and the caller makes it (this crate does not bring in a random-number crate —
    /// the canonical source of randomness is [`crate::machine`]).
    ///
    /// ★It does not die here even if the values are broken★ — an operating process with a panic hook ends at once on an uncontained panic
    /// (ADR-0292), so one `Duration::from_secs_f64(NaN)` takes down the **whole process**. The policy is
    /// a value the consumer reads in from a file, so we consider NaN · negative · jitter of 1 or more actually reachable and fold them all
    /// here.
    ///
    /// ★The cap is bitten once more **after** the jitter★ — so near `cap` the upward wobble is cut and the distribution
    /// is pressed to one side. Since the purpose of jitter is to scatter simultaneous retries, we judged this better than allowing the cap to be exceeded.
    pub fn delay(&self, attempt: u32, r: f64) -> Duration {
        // ★If the cap is smaller than the floor, **lower the floor**★ — the old edition quietly discarded the cap with `cap.max(base)`
        //   so `base=10s, cap=1s` returned 10 seconds (ten times the declared cap). If the name is 「상한」 (cap),
        //   not exceeding it is less surprising. `check()` rejects this combination outright.
        let floor = MIN_BACKOFF.as_secs_f64();
        let cap = sane_secs(self.cap.as_secs_f64(), 10.0).max(floor);
        let base = sane_secs(self.base.as_secs_f64(), 0.5).clamp(floor, cap);
        // A multiplier under 1 **shrinks** the backoff and becomes a dial busy loop.
        let multiplier = if self.multiplier.is_finite() && self.multiplier >= 1.0 {
            self.multiplier
        } else {
            1.0
        };
        // A jitter of 1 or more can make the wait 0 and becomes the same busy loop.
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

        // ★Do not pass `attempt as i32` directly★ — `u32::MAX` becomes `-1`, the exponent flips and
        //   the backoff becomes **shorter** than base (measured: 500ms → 250ms). It saturates at the cap anyway, so we truncate.
        let grown = base * multiplier.powi(attempt.min(1024) as i32);
        let capped = if grown.is_finite() {
            grown.min(cap)
        } else {
            cap
        };
        let jittered = capped * (1.0 + jitter * (2.0 * nudge - 1.0));
        if jittered.is_finite() {
            // ★There must be a floor★ — 「0이 아니다」 (not zero) is not enough: a 1ns backoff is a dial busy loop.
            Duration::from_secs_f64(jittered.clamp(floor, cap))
        } else {
            Duration::from_secs_f64(cap)
        }
    }
}

/// Whatever policy value comes, the backoff never gets shorter than this. Retrying more often than this is a busy loop, not a wait.
pub const MIN_BACKOFF: Duration = Duration::from_millis(1);

fn sane_secs(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}

/// All the policy that applies to one peer.
#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    /// The upper limit on waiting for a reply. One shared value regardless of command kind (ADR-0181 decision 1).
    ///
    /// ★Unverified — the real time agent spawn takes has never been measured★. It is the place that ADR wrote down as the only real risk of 8 seconds.
    pub request_timeout: Duration,
    /// The limit that wraps the dial and the handshake **together** (preserving the current `HANDSHAKE_TIMEOUT`). ★For an accepted peer
    /// ([`crate::Registry::attach`]) it is the auth deadline★ — measured not from when it was attached but from the end of the upgrade ([`crate::Accepted::upgraded_at`])
    /// (TRD 3 §5 3). An accepted peer does not redial, so it does not use [`Policy::reconnect`] · [`Policy::live_dwell`].
    pub connect_timeout: Duration,
    /// The limit allowed for pushing one frame. ★Unverified — only industry reference values★ (NATS 10s · nginx 60s · gRPC keepalive
    /// timeout 20s). Invariant: `write_deadline < ping_interval` — otherwise one write starves
    /// keepalive entirely (the deleted `net` crate had exactly that shape).
    pub write_deadline: Duration,
    /// Active ping interval (current daemon value preserved).
    pub ping_interval: Duration,
    /// If the peer is this quiet, cut (current daemon value preserved — 2.5 times the ping interval).
    pub idle_timeout: Duration,
    /// ★Only a connection that stayed in the operating phase this long refills the reconnect budget★.
    ///
    /// Without it, a peer that is cut as soon as it attaches repeats reconnecting every 500ms **forever** and
    /// never reaches [`crate::GaveUpReason::BudgetExhausted`], and then the popup of ADR-0180 decision 7
    /// **does not appear** in exactly the state of 「사용자가 행동하지 않으면 영영 복구되지 않는」 (never recovers unless the user acts).
    /// ★Unverified — the real reconnect time has never been measured★. Today's shell has no such concept at all (reset on success).
    pub live_dwell: Duration,
    pub reconnect: ReconnectPolicy,
    /// Outbound queue length (current shell value preserved).
    pub outbound_queue: usize,
    /// ★One handle's★ receiver end ([`crate::Inbox`]) length (count). Each time [`crate::Registry::add`] · [`crate::Registry::attach`]
    /// raise one, a line of this length is set up (even raising the same name tag again is a new line). The total held is the sum of the live receiver-end
    /// lengths (including old receiver ends whose old supervisor has not finished teardown) and ★this value measures only count★ — bytes are measured by each supervisor's crate-internal
    /// budget (16 MiB — one count covers the read-ahead queue · the ordering area · this line · the resume window. Only a supervisor that has received a stream piece even once
    /// splits it half and half into read share · window cap, and for one that has not, the read share is the whole budget) (`lib.rs` 「알려진
    /// 한계」 (known limits)). When full, that handle's supervisor holds in its own ordering area and stops that peer's reading — it does not drop (backpressure).
    /// ★Unverified — there is no counterpart today★ (the shell goes straight to a Tauri Channel so this queue itself does not exist). That is, this crate
    /// **creates one new failure mode** (TRD §12-12).
    pub inbound_queue: usize,
    /// One connection's read-ahead buffer (count). ★A **different resource** from [`Policy::inbound_queue`]★ — if this one fills,
    /// the read task stops and transport-layer backpressure applies, and if that one fills, that peer's events · packets are held in the supervisor's ordering area
    /// and the supervisor drains this one no more — so if that one fills, this one soon fills too. Bytes are measured by the supervisor's budget separately from this value
    /// (the same sentence in `inbound_queue` above).
    /// ★Unverified★.
    pub frame_buffer: usize,
    pub full_queue: QueueFullPolicy,
    pub decode_failure: DecodeFailurePolicy,
    /// How many expired pairing numbers to remember. ★Unverified★ — it is the length of [`crate::Pending`]'s bounded ring.
    pub retired_tags: usize,
    /// The upper limit on the number of streams whose sequence numbers are tracked. ★This is **the only collection in this crate whose size the peer decides**, so
    /// without a limit a peer that keeps sending new keys pushes memory up without bound★. When it overflows it forgets from the oldest
    /// registered, and a forgotten stream rebuilds its dedup ledger from scratch. ★Unverified★.
    pub max_streams: usize,
    /// The upper limit on waiting for the answer (= piece number `after+1`) after asking for a resume.
    ///
    /// ★This value separates 「상대가 못 준다」 (the peer cannot give it) from 「아직 안 왔다」 (it has not come yet)★ — an out-of-order piece arriving in between
    /// may be one the peer had already sent before receiving the re-request, so we drop it and wait for the answer. Only after exceeding this deadline do we raise
    /// [`crate::TransportEvent::StreamTruncated`]. ★While that peer's reading is stopped by backpressure, this clock
    /// stops★ — even if the answer comes in that time we cannot read it (`lib.rs` 「받는 쪽 불변식」 (receiver invariants) 2). ★Unverified★.
    pub resume_timeout: Duration,
    /// The upper limit on the number of pieces one stream **holds back** while waiting for a re-request's answer.
    ///
    /// They are held to keep order so dropping is not allowed, but without a limit the peer pushes our memory
    /// with this buffer. When full, the window is closed on the spot and what was held is sent out in order. In bytes, each supervisor's crate-internal window cap
    /// (8 MiB — half of the byte budget · the sum of held pieces across all streams) also measures, and when exceeded it closes the same way — if it is a new hole,
    /// it does not open the window and goes straight to a truncation (`lib.rs` 「알려진 한계」 (known limits)). ★Unverified★.
    pub resume_buffer: usize,
    /// The deadline that one [`crate::Peer::close_after_flush`] uses — from when the supervisor notices that close it covers the writes before the marker ·
    /// keepalive · close frame **all together**. If exceeded, it closes without writing the rest.
    ///
    /// ★It is not replaced by [`Policy::write_deadline`]★ — that is per write, so a peer that reads slowly can hold the close for queue length ×
    /// write deadline (TRD 3 §3-0 ④). ★Unverified★ — it is the draft value of TRD 3 §3-0 ④ as is.
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
    /// Do the values contradict each other. Returns the human-readable name of the item that is off.
    ///
    /// ★The crate does not die even if this does not pass★ — [`ReconnectPolicy::delay`] folds broken values by itself.
    /// This function exists so that **the consumer fails loudly when reading its settings**, and the registry
    /// looks only with `debug_assert`.
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
        assert!(p.check().is_err(), "check 가 이 조합을 받아 주면 안 된다");
        for attempt in [0u32, 1, 5] {
            assert!(
                policy.delay(attempt, 0.0) <= policy.cap,
                "★선언한 상한을 조용히 버리면 안 된다★"
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
                "1ns 백오프는 대기가 아니라 dial 열루프다"
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
