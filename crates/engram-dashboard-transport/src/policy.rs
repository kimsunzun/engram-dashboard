//! 정책 값 — ★전부 crate 기본값이 있고 소비자가 덮는다★(ADR-0177 결정 4).
//!
//! [`crate::Registry::new`] 가 명부 기본값을, [`crate::Registry::add_with_policy`] 가 상대별 덮어쓰기를
//! 받는다.
//!
//! ★근거 없이 고른 값에는 그 자리에 「미검」을 적어 둔다★ — 적어 두지 않으면 다음 세션이 추측을 실측으로
//! 읽는다. 미검인 것: `request_timeout` · `write_deadline` · `reconnect.jitter` · `inbound_queue` ·
//! `frame_buffer` · `retired_tags` · `max_streams` · `resume_timeout` · `live_dwell`. 나머지는 현행
//! 코드값 보존이다.

use std::time::Duration;

/// 큐가 찼을 때. ★나가는 큐에만 적용된다★ — 들어오는 한 줄은 상대들이 **함께 쓰는** 자원이라 그쪽의
/// 포화로 연결을 끊으면 시끄러운 상대가 조용한 상대를 죽인다(ADR-0180 결정 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueFullPolicy {
    /// 그 연결을 끊는다(현행 유지 — ADR-0177 결정 6). 다시 붙어 처음부터 맞추는 쪽이 낫다.
    ///
    /// ★메시지 단위로 떨구는 안(drop oldest/newest)은 터미널 바이트에 못 쓴다★ — 중간을 버리면 UTF-8 이
    /// 깨지고 ANSI 파서 상태가 어긋나 화면이 망가진 채로 계속 그려진다.
    Disconnect,
    /// 그 메시지만 버리고 [`crate::TransportEvent::Dropped`] 로 신고한다. ★조용한 유실을 안 만드는 것이
    /// 이 갈래의 전부다★ — 세는 것을 그만두면 `Disconnect` 보다 나쁜 선택이 된다.
    DropAndReport,
}

/// [`crate::Wire::decode`] 가 실패했을 때.
///
/// ★기본이 [`DecodeFailurePolicy::DropFrame`] 인 것은 사용자 체감 판단이다★ — 프레임 하나가 깨졌다고
/// 화면이 통째로 끊기지 않게 한다. **대가 = 프로토콜이 어긋난 채로 계속 도는 조합이 생긴다**(TRD §12-11
/// 이 연 질문이고, 그 답이 정해지면 기본값이 바뀔 수 있다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeFailurePolicy {
    DropFrame,
    Disconnect,
}

/// 재연결 예산과 백오프 일정.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReconnectPolicy {
    /// ★예산 회복 뒤 허용하는 **재시도 횟수**★. 이 수만큼의 백오프 대기가 있고, 그 뒤 다시 실패하면
    /// [`crate::GaveUpReason::BudgetExhausted`] 다.
    pub max_attempts: u32,
    pub base: Duration,
    pub cap: Duration,
    pub multiplier: f64,
    /// 대기 시간에 섞는 흔들림의 비율(0.2 = ±20%). ★미검★ — gRPC 가 스스로 *"Proposed Backoff
    /// Algorithm"* 이라 제목을 단 문서의 예시값을 차용했다. **오늘 코드에는 지터가 아예 없다.**
    pub jitter: f64,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            // ADR-0180 결정 4(사용자 결정). 오늘 코드는 5다 — 두 층이 갈리는 것은 알려진 상태이고
            //   맞추는 일은 이 crate 를 붙이는 단계 몫이다.
            max_attempts: 3,
            // 현행값 보존 — `connection.rs` 의 BACKOFF_BASE.
            base: Duration::from_millis(500),
            // 현행값 보존 — `connection.rs` 의 BACKOFF_CAP.
            cap: Duration::from_secs(10),
            // 현행값 보존 — 3회 = 500ms→1s→2s = 합계 3.5초(ADR-0180 이 든 그 수치).
            multiplier: 2.0,
            jitter: 0.2,
        }
    }
}

impl ReconnectPolicy {
    /// `attempt` 번째(0-기반) 재시도 앞의 대기.
    ///
    /// `r` 은 [0,1) 의 흔들림 눈금이고 부르는 쪽이 만든다(이 crate 는 난수 crate 를 들이지 않는다 —
    /// 난수원의 정본은 [`crate::machine`]).
    ///
    /// ★값이 망가져 있어도 여기서 죽지 않는다★ — 워크스페이스 릴리즈 프로필이 `panic = "abort"` 라
    /// `Duration::from_secs_f64(NaN)` 한 번이 **프로세스 전체**를 내린다. 정책은 소비자가 파일에서
    /// 읽어 넣는 값이므로 NaN·음수·1 이상 지터가 실제로 도달할 수 있다고 보고 전부 여기서 접는다.
    ///
    /// ★상한을 지터 **뒤에** 한 번 더 문다★ — 그래서 `cap` 부근에서는 위쪽 흔들림이 잘려 분포가
    /// 한쪽으로 눌린다. 지터의 목적이 동시 재시도를 흩는 것이라 상한 초과를 허용하는 것보다 낫다고 봤다.
    pub fn delay(&self, attempt: u32, r: f64) -> Duration {
        // ★상한이 하한보다 작으면 **하한을 내린다**★ — 옛 판은 `cap.max(base)` 로 상한을 조용히
        //   버려서 `base=10s, cap=1s` 가 10초를 돌려줬다(선언한 상한의 열 배). 이름이 「상한」이면
        //   그것을 넘지 않는 쪽이 덜 놀랍다. `check()` 는 이 조합을 아예 거절한다.
        let floor = MIN_BACKOFF.as_secs_f64();
        let cap = sane_secs(self.cap.as_secs_f64(), 10.0).max(floor);
        let base = sane_secs(self.base.as_secs_f64(), 0.5).clamp(floor, cap);
        // 1 미만 배율은 백오프를 **줄여** dial 열루프가 된다.
        let multiplier = if self.multiplier.is_finite() && self.multiplier >= 1.0 {
            self.multiplier
        } else {
            1.0
        };
        // 1 이상 지터는 대기를 0 으로 만들 수 있어 같은 열루프가 된다.
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

        // ★`attempt as i32` 로 바로 넘기지 말 것★ — `u32::MAX` 가 `-1` 이 되어 지수가 뒤집히고
        //   백오프가 base 보다 **짧아진다**(실측: 500ms → 250ms). 어차피 상한에서 포화하므로 자른다.
        let grown = base * multiplier.powi(attempt.min(1024) as i32);
        let capped = if grown.is_finite() {
            grown.min(cap)
        } else {
            cap
        };
        let jittered = capped * (1.0 + jitter * (2.0 * nudge - 1.0));
        if jittered.is_finite() {
            // ★바닥이 있어야 한다★ — 「0이 아니다」로는 부족하다: 1ns 백오프는 dial 열루프다.
            Duration::from_secs_f64(jittered.clamp(floor, cap))
        } else {
            Duration::from_secs_f64(cap)
        }
    }
}

/// 어떤 정책 값이 와도 백오프가 이보다 짧아지지 않는다. 이보다 잦은 재시도는 대기가 아니라 열루프다.
pub const MIN_BACKOFF: Duration = Duration::from_millis(1);

fn sane_secs(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}

/// 한 상대에게 걸리는 정책 전부.
#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    /// 답장을 기다리는 상한. 명령 종류와 무관한 공용 값 하나다(ADR-0181 결정 1).
    ///
    /// ★미검 — 에이전트 spawn 실소요를 잰 적이 없다★. 그 ADR 이 8초의 유일한 실질 위험으로 적은 자리다.
    pub request_timeout: Duration,
    /// dial 과 핸드셰이크를 **함께** 감싸는 상한(현행 `HANDSHAKE_TIMEOUT` 보존).
    pub connect_timeout: Duration,
    /// 한 프레임을 미는 데 허용하는 상한. ★미검 — 업계 참고값뿐★(NATS 10s · nginx 60s · gRPC keepalive
    /// timeout 20s). 불변식: `write_deadline < ping_interval` — 안 그러면 쓰기 하나가 keepalive 를
    /// 통째로 굶긴다(오늘 `net` 에 있는 실물 결함이 정확히 그 모양이다).
    pub write_deadline: Duration,
    /// 능동 ping 주기(현행 데몬값 보존).
    pub ping_interval: Duration,
    /// 상대가 이만큼 조용하면 끊는다(현행 데몬값 보존 — ping 주기의 2.5배).
    pub idle_timeout: Duration,
    /// ★이만큼 운영 단계에 머문 연결만 재연결 예산을 되채운다★.
    ///
    /// 없으면 붙자마자 끊기는 상대가 **영원히** 500ms 마다 재연결을 되풀이해
    /// [`crate::GaveUpReason::BudgetExhausted`] 에 절대 닿지 않고, 그러면 ADR-0180 결정 7 의 팝업이
    /// 「사용자가 행동하지 않으면 영영 복구되지 않는」 바로 그 상태에서 **뜨지 않는다**.
    /// ★미검 — 실제 재연결 소요를 잰 적이 없다★. 오늘 셸에는 이 개념 자체가 없다(성공 즉시 리셋).
    pub live_dwell: Duration,
    pub reconnect: ReconnectPolicy,
    /// 나가는 큐 길이(현행 셸값 보존).
    pub outbound_queue: usize,
    /// ★상대 전부가 함께 쓰는★ 위로 올라가는 한 줄의 길이. ★미검 — 오늘 대응물이 없다★(셸은 Tauri
    /// Channel 로 직행해 이 큐 자체가 없다). 즉 이 crate 는 **새 실패 모드를 하나 만든다**(TRD §12-12).
    pub inbound_queue: usize,
    /// 연결 하나의 읽기 앞잡이 버퍼. ★[`Policy::inbound_queue`] 와 **다른 자원**이다★ — 이쪽이 차면
    /// 읽기 태스크가 멈춰 전송 계층 배압이 걸릴 뿐이고, 저쪽이 차면 다른 상대까지 영향을 받는다.
    /// 하나로 합치면 「상대 하나를 위해 고른 값」이 「전체를 위해 고른 값」을 덮어쓴다. ★미검★.
    pub frame_buffer: usize,
    pub full_queue: QueueFullPolicy,
    pub decode_failure: DecodeFailurePolicy,
    /// 만료된 짝짓기 번호를 몇 개까지 기억하나. ★미검★ — [`crate::Pending`] 의 유계 링 길이다.
    pub retired_tags: usize,
    /// 순번을 추적하는 스트림 수 상한. ★이 crate 에서 **크기를 상대가 정하는 유일한 컬렉션**이라
    /// 상한이 없으면 새 키를 계속 보내는 상대가 메모리를 무한히 밀어 올린다★. 넘치면 가장 오래
    /// 등록된 것부터 잊고, 잊힌 스트림은 중복 제거 장부를 처음부터 다시 쌓는다. ★미검★.
    pub max_streams: usize,
    /// 이어받기를 청한 뒤 그 답(=`after+1` 번 조각)을 기다리는 상한.
    ///
    /// ★이 값이 「상대가 못 준다」와 「아직 안 왔다」를 가른다★ — 그 사이에 도착하는 순서 밖 조각은
    /// 상대가 재요청을 받기 전에 이미 띄운 것일 수 있으므로 버리고 답을 기다린다. 이 시한을 넘겨야
    /// [`crate::TransportEvent::StreamTruncated`] 를 올린다. ★미검★.
    pub resume_timeout: Duration,
    /// 재요청의 답을 기다리는 동안 스트림 하나가 **붙들어 두는** 조각 수 상한.
    ///
    /// 순서를 지키려고 붙드는 것이라 버리면 안 되지만, 상한이 없으면 상대가 이 버퍼로 우리 메모리를
    /// 민다. 차면 창을 그 자리에서 닫고 붙든 것을 순서대로 내보낸다. ★미검★.
    pub resume_buffer: usize,
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
        }
    }
}

impl Policy {
    /// 값들이 서로 모순되지 않나. 어긋난 항목의 사람이 읽는 이름을 돌려준다.
    ///
    /// ★이것이 통과하지 않아도 crate 는 죽지 않는다★ — [`ReconnectPolicy::delay`] 가 망가진 값을 스스로
    /// 접는다. 이 함수는 **소비자가 설정을 읽을 때 시끄럽게 실패하라고** 있는 것이고, 명부는
    /// `debug_assert` 로만 본다.
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
    }

    #[test]
    fn write_deadline_may_not_starve_keepalive() {
        let mut p = Policy::default();
        p.write_deadline = p.ping_interval;
        assert!(p.check().is_err());
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

    // ── F2: 상한이 하한보다 작은 조합 ──
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

    // ── F3: 열루프가 되는 바닥값 ──
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

    // ── 망가진 정책 값이 프로세스를 내리지 않는다 ──
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
    fn the_shared_line_and_the_per_link_buffer_are_two_values() {
        let p = Policy {
            inbound_queue: 8,
            frame_buffer: 4096,
            ..Policy::default()
        };
        assert_eq!(p.check(), Ok(()));
        assert_ne!(p.inbound_queue, p.frame_buffer);
    }
}
