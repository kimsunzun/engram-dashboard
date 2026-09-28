//! 사용량 스케줄러 — 데몬마다 스레드 하나(`usage-scheduler`)가 구독된 벤더의 자동 조회 기한·리셋 시각·거절 끝·
//! 합침 기한 중 가장 이른 때 깨어 [`UsageBook::plan_tick`](super::book::UsageBook::plan_tick) 의 계획을 나른다
//! (TRD §1-4 「스케줄러」 · §3 #58).
//!
//! ★스레드는 서비스를 `Weak` 로만 쥔다 — 잠든 동안엔 강한 참조가 없다★. 그래서 서비스가 사라지면 깨우기 송신단도
//!   함께 사라져(`Disconnected`) 스레드가 스스로 끝난다 — 막힌 조회가 진행 중이어도 · 종료 절차에 단계가 없다.
//! ★깨어난 뒤의 판정은 늘 서비스의 시계로 잰다★ — 대기 타이머의 시계는 절전 시간을 안 셀 수 있다(TRD §6 #2).
//!   잠 상한 [`SCHEDULE_MAX_SLEEP`] 이 복귀·벽시계 이동 뒤의 늦음을 묶는다.
//! 진입점 = [`spawn_scheduler`] — 조립이 [`UsageService::new`] 가 준 수신단으로 한 번 부른다.
// ADR-0006

use std::io;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::book::SCHEDULE_MAX_SLEEP;
use super::UsageService;

/// 기다림 하한 — 이보다 짧은 기다림은 이만큼으로 올린다(그 기한은 많아야 이만큼 늦는다). 목적 = 0 이 새어
/// 들어와도(계약이 깨진 계획 · 나르는 동안 기한이 지나감) 이 스레드가 책 락을 쉼 없이 두드리지 않게.
const SCHEDULE_MIN_SLEEP: Duration = Duration::from_millis(10);

/// 스케줄러 스레드를 띄운다. `wakes` = [`UsageService::new`] 가 함께 돌려준 수신단 — 다른 수신단을 넘기면 서비스의
/// 깨움(구독 교체·조회 끝·줍기)을 못 듣고 서비스 drop 에도 잠 상한까지 늦게 끝난다.
///
/// 핸들을 놓으면 분리된 채 돈다 — 기다릴 일이 없다(서비스가 사라지면 스스로 끝난다). `Err` = 스레드를 못 띄웠다 —
/// 서비스는 그대로 돌지만 시간이 이끄는 일이 전부 멈춘다: 자동 조회 · 기한이 차서 나가는 발행 · 못 갚은 발행 빚의
/// 회수(TRD §3 #86). 요청(⟳·버스)·줍기·구독 교체는 그대로 돈다. 남기는 것은 부른 쪽 몫이다.
pub fn spawn_scheduler(
    service: &Arc<UsageService>,
    wakes: Receiver<()>,
) -> io::Result<JoinHandle<()>> {
    let service = Arc::downgrade(service);
    std::thread::Builder::new()
        .name("usage-scheduler".to_owned())
        .spawn(move || run(service, wakes))
}

fn run(service: Weak<UsageService>, wakes: Receiver<()>) {
    loop {
        let Some(strong) = service.upgrade() else {
            break;
        };
        let next = tick(&strong);
        // 잠들기 전에 놓는다 — 이 스레드가 마지막 소유자면 서비스가 여기서 사라지고, 송신단도 함께 사라져 아래
        //   대기가 곧바로 `Disconnected` 를 본다.
        drop(strong);
        match wakes.recv_timeout(next.wait()) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    tracing::debug!("사용량 서비스가 사라졌다 — 스케줄러를 끝낸다");
}

/// 한 번의 결과 — 계획이 시각을 읽은 순간(`planned`)부터 `sleep` 뒤에 깬다.
struct Tick {
    planned: Instant,
    sleep: Duration,
}

impl Tick {
    /// 지금부터 기다릴 시간 — 계획 뒤 흐른 만큼(나르기 포함)을 빼고 [`SCHEDULE_MIN_SLEEP`, `SCHEDULE_MAX_SLEEP`]
    /// 로 자른다. 그래서 나르는 동안 지난 기한은 하한 뒤 곧바로 다시 계획한다.
    fn wait(&self) -> Duration {
        bounded(self.sleep.saturating_sub(self.planned.elapsed()))
    }
}

/// 스케줄러 한 번 — 계획을 뜨고 나른다. 락 차례 = 명부{합집합 사본} → 책{계획} → 어느 락도 없이 발행·조회 기동
/// (TRD §1-4 경로 표 「스케줄러 `tick`」).
fn tick(service: &UsageService) -> Tick {
    let subscribed = service.watch.union();
    let (plan, planned) = {
        let mut desk = service.desk();
        // 시각은 책 락 안에서 읽는다 — 다른 경로의 적용 순서와 시각 순서가 어긋나지 않는다.
        let now = service.now();
        // 흐른 시간은 서비스 시계가 아니라 기다림(`recv_timeout`)이 재는 시계로 잰다.
        let planned = Instant::now();
        (desk.book.plan_tick(&subscribed, now), planned)
    };
    Tick {
        planned,
        sleep: service.carry_out(plan),
    }
}

fn bounded(sleep: Duration) -> Duration {
    // `clamp` 를 쓰지 않는다 — 하한 > 상한이면 패닉한다(릴리즈 = abort).
    sleep.max(SCHEDULE_MIN_SLEEP).min(SCHEDULE_MAX_SLEEP)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage_service::book::RequestKind;
    use crate::usage_service::clock::{ManualUsageClock, UsageClock};
    use crate::usage_service::reject_store::RejectEntry;
    use crate::usage_service::tests::{
        active, answered, bus_request, far_reset, five, frames, key, last_pct, ok, pct, rig,
        rig_with, set, wait_until, Rig, T0,
    };
    use crate::usage_service::watch::tests::recording;
    use crate::usage_service::watch::{UsageFrame, UsageOutlet};
    use crate::usage_service::UsageServed;
    use engram_dashboard_agent::usage::UsageObservation;
    use engram_dashboard_protocol::UsageVendorState;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Mutex};

    /// 칸에 끝난 조회 하나를 새긴다 — 자동 조회 기한이 쿨타임 뒤로 밀려 `tick` 이 조회를 열지 않는다. 빚도 갚는다.
    fn settle(rig: &Rig, i: usize, obs: UsageObservation) {
        let now = rig.service.now();
        let mut desk = rig.service.desk();
        assert!(desk.book.begin_probe(&key(i)));
        desk.book.finish_probe(&key(i), Ok(obs), now);
        desk.book.broadcast_sheet(&key(i), now);
    }

    /// 스케줄러를 띄운다 — 틀의 깸 수신단을 꺼내 넘긴다(틀은 그 뒤 깸을 못 센다).
    fn launch(rig: &mut Rig) -> JoinHandle<()> {
        let wakes = std::mem::replace(&mut rig.wakes, mpsc::sync_channel(1).1);
        spawn_scheduler(&rig.service, wakes).expect("스케줄러 스레드")
    }

    /// 상한 안에 끝나야 한다 — 잠 상한(60초)보다 훨씬 짧아, 여기서 끝나면 깨움이나 drop 이 끝낸 것이다.
    fn join_within(handle: JoinHandle<()>, what: &str) {
        wait_until(what, || handle.is_finished());
        handle.join().expect("스케줄러 스레드는 패닉하지 않는다");
    }

    // ── tick ──

    #[test]
    fn a_tick_without_subscribers_starts_nothing_publishes_nothing_and_sleeps_the_cap() {
        let (rig, _clock) = rig();
        let idle = rig.subscriber(1, &[]);
        rig.service.restore_rejects(&[RejectEntry {
            key: key(1),
            until_epoch_s: T0 + 30,
        }]);

        assert_eq!(tick(&rig.service).sleep, SCHEDULE_MAX_SLEEP);
        assert!(
            !rig.in_flight(0),
            "기준점이 없어 기한이 지났어도 구독 밖이다"
        );
        assert!(!rig.in_flight(1));
        assert_eq!(idle.count(), 0);
    }

    #[test]
    fn a_due_subscribed_vendor_gets_one_probe_a_refresh_joins_and_its_start_sheet_goes_first() {
        let (rig, _clock) = rig();
        let log = rig.subscriber(1, &[0]);

        assert_eq!(
            tick(&rig.service).sleep,
            SCHEDULE_MAX_SLEEP,
            "진행 중인 칸의 기한은 안 센다"
        );
        assert!(rig.in_flight(0));
        assert!(!rig.in_flight(1), "구독 밖");
        let got = frames(&log);
        assert_eq!(got.len(), 1);
        assert!(got[0].snapshot.in_flight, "「갱신 중」 한 장");

        let joined = bus_request(&rig.service, 0, RequestKind::Refresh);
        wait_until("⟳ 가 합류했다", || rig.joiners(0) == 1);
        tick(&rig.service);
        assert_eq!(log.count(), 1, "진행 중 — 새로 연 조회도 낼 한 장도 없다");

        rig.answer(0, ok(active(0, 42.0)));
        assert_eq!(answered(&joined).served, UsageServed::Fresh);
        assert_eq!(rig.queries(0), 1, "스케줄러가 연 조회 하나에 합류했다");
        let got = frames(&log);
        assert_eq!(got.len(), 2);
        assert!(!got[1].snapshot.in_flight);
        assert_eq!(pct(&got[1].snapshot), Some(42.0));

        assert_eq!(
            tick(&rig.service).sleep,
            SCHEDULE_MAX_SLEEP,
            "다음 기한 = 쿨타임 뒤"
        );
        assert!(!rig.in_flight(0));
        assert_eq!(log.count(), 2);
    }

    #[test]
    fn a_coalesced_change_goes_out_once_at_its_deadline() {
        let (rig, clock) = rig();
        settle(&rig, 0, active(0, 10.0));
        clock.advance_both(Duration::from_secs(2));
        let log = rig.subscriber(1, &[0]);
        rig.service.observe(&five(0, 40.0, far_reset()));
        assert_eq!(log.count(), 1, "합침 창 밖 — 곧바로");
        clock.advance_both(Duration::from_millis(400));
        rig.service.observe(&five(0, 41.0, far_reset()));
        assert_eq!(log.count(), 1, "창 안 — 끝 발행은 스케줄러 몫");

        assert_eq!(tick(&rig.service).sleep, Duration::from_millis(600));
        assert_eq!(log.count(), 1, "기한 전");
        clock.advance_both(Duration::from_millis(600));
        assert_eq!(tick(&rig.service).sleep, SCHEDULE_MAX_SLEEP);
        assert_eq!(log.count(), 2);
        assert_eq!(last_pct(&log), Some(41.0));
        tick(&rig.service);
        assert_eq!(log.count(), 2, "빚은 한 번 갚는다");
    }

    #[test]
    fn a_deadline_nearer_than_the_floor_sleeps_the_floor_then_goes_out() {
        let (rig, clock) = rig();
        settle(&rig, 0, active(0, 10.0));
        clock.advance_both(Duration::from_secs(2));
        let log = rig.subscriber(1, &[0]);
        rig.service.observe(&five(0, 40.0, far_reset()));
        clock.advance_both(Duration::from_millis(997));
        rig.service.observe(&five(0, 41.0, far_reset()));

        let next = tick(&rig.service);
        assert_eq!(next.sleep, Duration::from_millis(3));
        assert_eq!(
            next.wait(),
            SCHEDULE_MIN_SLEEP,
            "하한 아래 기다림은 하한으로 올린다"
        );
        assert_eq!(log.count(), 1);
        clock.advance_both(SCHEDULE_MIN_SLEEP);
        tick(&rig.service);
        assert_eq!(log.count(), 2);
        assert_eq!(last_pct(&log), Some(41.0));
    }

    #[test]
    fn a_window_reaching_its_reset_latches_and_goes_out_at_that_tick() {
        let (rig, clock) = rig();
        settle(&rig, 0, five(0, 50.0, T0 + 30));
        let log = rig.subscriber(1, &[0]);

        assert_eq!(
            tick(&rig.service).sleep,
            Duration::from_secs(30),
            "리셋 시각까지"
        );
        assert_eq!(log.count(), 0);
        clock.advance_both(Duration::from_secs(30));
        tick(&rig.service);
        let got = frames(&log);
        assert_eq!(got.len(), 1);
        assert!(got[0]
            .snapshot
            .five_hour
            .as_ref()
            .is_some_and(|w| w.expired));
        tick(&rig.service);
        assert_eq!(log.count(), 1, "래치 한 장은 한 번");
    }

    #[test]
    fn a_restored_rejection_goes_out_on_the_first_tick_and_holds_the_probe_until_it_ends() {
        let (rig, clock) = rig();
        rig.service.restore_rejects(&[RejectEntry {
            key: key(0),
            until_epoch_s: T0 + 40,
        }]);
        let log = rig.subscriber(1, &[0]);

        assert_eq!(
            tick(&rig.service).sleep,
            Duration::from_secs(40),
            "거절 끝까지"
        );
        let got = frames(&log);
        assert_eq!(got.len(), 1, "되살린 칸의 빚을 tick 이 갚는다");
        assert!(matches!(
            got[0].snapshot.state,
            UsageVendorState::Rejected { .. }
        ));
        assert!(!rig.in_flight(0), "거절 중 — 조회 없음");

        clock.advance_both(Duration::from_secs(40));
        tick(&rig.service);
        assert!(rig.in_flight(0), "기준점 없는 칸의 기한 = 거절 끝");
        let got = frames(&log);
        assert_eq!(got.len(), 2, "거절 끝과 조회 시작이 한 장");
        assert!(got[1].snapshot.in_flight);
        assert!(!matches!(
            got[1].snapshot.state,
            UsageVendorState::Rejected { .. }
        ));
    }

    #[test]
    fn the_wait_subtracts_the_time_elapsed_since_the_plan() {
        let ago = |ms| {
            Instant::now()
                .checked_sub(Duration::from_millis(ms))
                .expect("단조 시계 기점 뒤")
        };
        let passed = Tick {
            planned: ago(100),
            sleep: Duration::from_millis(60),
        };
        assert_eq!(
            passed.wait(),
            SCHEDULE_MIN_SLEEP,
            "나르는 동안 기한이 지났다 — 하한 뒤 곧 다시 계획"
        );
        let ahead = Tick {
            planned: ago(200),
            sleep: Duration::from_secs(1),
        };
        assert!(ahead.wait() <= Duration::from_millis(800));
        let capped = Tick {
            planned: Instant::now(),
            sleep: Duration::MAX,
        };
        assert_eq!(capped.wait(), SCHEDULE_MAX_SLEEP);
    }

    /// 받은 순간을 적고 잠깐 머문다 — 그 뒤에 잰 시각은 적힌 순간보다 늦다.
    struct Stamping(Arc<Mutex<Vec<Instant>>>);

    impl UsageOutlet for Stamping {
        fn send(&self, _frame: &UsageFrame) {
            self.0.lock().unwrap().push(Instant::now());
            std::thread::sleep(Duration::from_millis(5));
        }

        fn revoke(&self) {}
    }

    #[test]
    fn a_tick_takes_its_plan_time_before_carrying_the_plan_out() {
        let (rig, _clock) = rig();
        let sent = Arc::new(Mutex::new(Vec::new()));
        rig.service.attach(1, Stamping(sent.clone()));
        rig.service.replace_subscription(1, set(&[0]));
        sent.lock().unwrap().clear();

        let next = tick(&rig.service);
        let sent = sent.lock().unwrap().clone();
        assert_eq!(sent.len(), 1, "나르기의 「갱신 중」 한 장");
        assert!(next.planned <= sent[0], "계획 시각은 나르기 전에 잰다");
    }

    #[test]
    fn the_sleep_stays_between_the_floor_and_the_cap() {
        assert_eq!(bounded(Duration::ZERO), SCHEDULE_MIN_SLEEP);
        assert_eq!(bounded(Duration::from_nanos(1)), SCHEDULE_MIN_SLEEP);
        assert_eq!(
            bounded(Duration::from_millis(500)),
            Duration::from_millis(500)
        );
        assert_eq!(bounded(Duration::MAX), SCHEDULE_MAX_SLEEP);
    }

    // ── 구동 스레드 ──

    /// 읽힌 수를 세는 손 시계 — 연결이 구독하기 전 시계를 읽는 것은 스케줄러의 `tick` 뿐이다.
    struct Counted {
        inner: ManualUsageClock,
        reads: AtomicUsize,
    }

    impl UsageClock for Counted {
        fn mono(&self) -> Duration {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.inner.mono()
        }

        fn wall(&self) -> i64 {
            self.inner.wall()
        }
    }

    /// 칸 하나 · 세는 시계의 틀.
    fn counted_rig() -> (Rig, Arc<Counted>) {
        let clock = Arc::new(Counted {
            inner: ManualUsageClock::new(Duration::from_secs(100), T0),
            reads: AtomicUsize::new(0),
        });
        (rig_with(clock.clone(), 1), clock)
    }

    /// 첫 `tick` 을 마치고 잠들 때까지 — 시계를 읽었고 강한 참조를 놓았다. 구독이 없으면 그 잠은 상한이다.
    fn wait_asleep(rig: &Rig, clock: &Counted) {
        wait_until("첫 tick 뒤 잠들었다", || {
            clock.reads.load(Ordering::SeqCst) >= 1 && Arc::strong_count(&rig.service) == 1
        });
    }

    #[test]
    fn a_wake_during_the_sleep_replans_at_once() {
        let (mut rig, clock) = counted_rig();
        let (outlet, log) = recording();
        rig.service.attach(1, outlet);
        let handle = launch(&mut rig);
        wait_asleep(&rig, &clock);

        rig.service.replace_subscription(1, set(&[0]));
        // 「갱신 중」 한 장은 계획(책 락 안)이 아니라 나르기(락 밖)에서 닿는다 — 닿은 한 장을 기다린다.
        wait_until(
            "깨어 새로 든 벤더의 「갱신 중」 한 장이 닿았다",
            || frames(&log).last().is_some_and(|f| f.snapshot.in_flight),
        );
        assert!(rig.in_flight(0));

        drop(rig);
        join_within(handle, "서비스가 사라져 끝났다");
    }

    #[test]
    fn the_asleep_scheduler_holds_no_service_so_a_drop_ends_it_even_during_a_stuck_probe() {
        let (mut rig, _clock) = rig();
        let _log = rig.subscriber(1, &[0]);
        let handle = launch(&mut rig);
        // 조회기는 대본을 받을 때까지 막힌다 — 대본 송신단은 `rig` 에 남아 끝까지 아무도 답하지 않는다.
        wait_until("조회를 열고 잠들었다", || {
            rig.queries(0) == 1 && Arc::strong_count(&rig.service) == 1
        });

        let service = rig.service;
        let weak = Arc::downgrade(&service);
        drop(service);
        assert!(
            weak.upgrade().is_none(),
            "잠든 스케줄러는 서비스를 붙들지 않는다"
        );
        join_within(handle, "막힌 조회가 도는 중에도 끝났다");
    }

    #[test]
    fn a_closed_wake_channel_ends_the_scheduler_while_the_service_lives() {
        let (rig, _clock) = rig();
        let (wake, wakes) = mpsc::sync_channel(1);
        drop(wake);
        let handle = spawn_scheduler(&rig.service, wakes).expect("스케줄러 스레드");
        join_within(handle, "송신단이 없다");
        assert_eq!(Arc::strong_count(&rig.service), 1);
    }

    #[test]
    fn a_wake_after_the_service_is_gone_ends_the_scheduler() {
        let (rig, clock) = counted_rig();
        // 서비스 밖의 송신단 — 서비스가 사라져도 `Disconnected` 가 아니므로 깨워야 upgrade 실패를 본다.
        let (wake, wakes) = mpsc::sync_channel(1);
        let handle = spawn_scheduler(&rig.service, wakes).expect("스케줄러 스레드");
        wait_asleep(&rig, &clock);

        drop(rig);
        assert!(!handle.is_finished(), "아직 상한 잠 안이다");
        wake.try_send(()).expect("잠든 스케줄러가 받는다");
        join_within(handle, "깨어 서비스가 없음을 봤다");
    }
}
