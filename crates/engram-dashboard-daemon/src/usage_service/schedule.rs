//! Usage scheduler — one thread per daemon (`usage-scheduler`) wakes at the earliest of the subscribed vendors' automatic query deadline · reset time · refusal end ·
//! coalescing deadline and carries the plan of [`UsageBook::plan_tick`](super::book::UsageBook::plan_tick)
//! (TRD §1-4 「스케줄러」 (scheduler) · §3 #58).
//!
//! ★The thread holds the service only as a `Weak` — while asleep it has no strong reference★. So when the service disappears the wake sender
//!   disappears too (`Disconnected`) and the thread ends by itself — even with a blocked query in flight · the shutdown procedure has no step.
//! ★The decision after waking is always measured by the service's clock★ — the wait timer's clock may not count sleep time (TRD §6 #2).
//!   The sleep cap [`SCHEDULE_MAX_SLEEP`] bounds lateness after resume or a wall-clock shift.
//! Entry point = [`spawn_scheduler`] — the assembly calls it once with the receiver [`UsageService::new`] gave.
// ADR-0006

use std::io;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::book::SCHEDULE_MAX_SLEEP;
use super::UsageService;

/// Wait floor — a wait shorter than this is raised to this (that deadline is late by this much at most). Purpose = so that even if a 0 leaks
/// in (a plan with a broken contract · a deadline passing while carrying) this thread does not hammer the book lock without rest.
const SCHEDULE_MIN_SLEEP: Duration = Duration::from_millis(10);

/// Launch the scheduler thread. `wakes` = the receiver [`UsageService::new`] returned together — if another receiver is passed, it cannot hear the service's
/// wakes (subscription replacement · query end · pickup) and even on service drop it ends late up to the sleep cap.
///
/// If the handle is dropped it runs detached — there is nothing to wait for (it ends by itself when the service disappears). `Err` = the thread could not be launched —
/// the service runs on but everything time drives stops: automatic queries · emission that goes out when a deadline is reached · collecting debt of emission not repaid
/// (TRD §3 #86). Requests (⟳ · bus) · pickup · subscription replacement run on. What to leave behind is the caller's share.
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
        // Drop it before sleeping — if this thread is the last owner the service disappears here, and the sender disappears with it so the wait below
        //   sees `Disconnected` at once.
        drop(strong);
        match wakes.recv_timeout(next.wait()) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    tracing::debug!("사용량 서비스가 사라졌다 — 스케줄러를 끝낸다");
}

/// The result of one pass — wake after `sleep` counted from the moment the plan read the time (`planned`).
struct Tick {
    planned: Instant,
    sleep: Duration,
}

impl Tick {
    /// The time to wait from now — subtract what has flowed since the plan (including carrying) and clamp to [`SCHEDULE_MIN_SLEEP`, `SCHEDULE_MAX_SLEEP`].
    /// So a deadline that passed while carrying is planned again right after the floor.
    fn wait(&self) -> Duration {
        bounded(self.sleep.saturating_sub(self.planned.elapsed()))
    }
}

/// One scheduler pass — take a plan and carry it. Lock order = registry {union copy} → book {plan} → emission · query launch without any lock
/// (TRD §1-4 path table 「스케줄러 `tick`」 (scheduler tick)).
fn tick(service: &UsageService) -> Tick {
    let subscribed = service.watch.union();
    let (plan, planned) = {
        let mut desk = service.desk();
        // The time is read inside the book lock — so the application order and time order of other paths do not go out of step.
        let now = service.now();
        // Elapsed time is measured not by the service clock but by the clock the wait (`recv_timeout`) measures.
        let planned = Instant::now();
        (desk.book.plan_tick(&subscribed, now), planned)
    };
    Tick {
        planned,
        sleep: service.carry_out(plan),
    }
}

fn bounded(sleep: Duration) -> Duration {
    // Do not use `clamp` — it panics if floor > cap (outside the containment mark so it ends the daemon · ADR-0292).
    sleep.max(SCHEDULE_MIN_SLEEP).min(SCHEDULE_MAX_SLEEP)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage_service::book::RequestKind;
    use crate::usage_service::clock::{ManualUsageClock, UsageClock};
    use crate::usage_service::tests::{
        active, answered, bus_request, far_reset, five, frames, key, last_pct, ok, pct, rig,
        rig_with, set, wait_until, Rig, T0,
    };
    use crate::usage_service::watch::tests::recording;
    use crate::usage_service::watch::{UsageFrame, UsageOutlet};
    use crate::usage_service::UsageServed;
    use engram_dashboard_agent::usage::{ProbeError, UsageObservation};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Mutex};

    /// Stamp one finished query onto the cell — the automatic query deadline is pushed past the cooldown so `tick` does not open a query. It also repays the debt.
    fn settle(rig: &Rig, i: usize, obs: UsageObservation) {
        let now = rig.service.now();
        let mut desk = rig.service.desk();
        assert!(desk.book.begin_probe(&key(i)));
        desk.book.finish_probe(&key(i), Ok(obs), now);
        desk.book.broadcast_sheet(&key(i), now);
    }

    /// Launch the scheduler — take out the frame's wake receiver and pass it (the frame cannot count wakes after that).
    fn launch(rig: &mut Rig) -> JoinHandle<()> {
        let wakes = std::mem::replace(&mut rig.wakes, mpsc::sync_channel(1).1);
        spawn_scheduler(&rig.service, wakes).expect("스케줄러 스레드")
    }

    /// It must end within the cap — far shorter than the sleep cap (60 seconds), so if it ends here a wake or a drop ended it.
    fn join_within(handle: JoinHandle<()>, what: &str) {
        wait_until(what, || handle.is_finished());
        handle.join().expect("스케줄러 스레드는 패닉하지 않는다");
    }

    // ── tick ──

    #[test]
    fn a_tick_without_subscribers_starts_nothing_publishes_nothing_and_sleeps_the_cap() {
        let (rig, _clock) = rig();
        let idle = rig.subscriber(1, &[]);
        {
            // A cell that left a debt — stamp a query that ended in refusal without emitting.
            let now = rig.service.now();
            let mut desk = rig.service.desk();
            assert!(desk.book.begin_probe(&key(1)));
            desk.book.finish_probe(
                &key(1),
                Err(ProbeError::RateLimited {
                    retry_after: Some(Duration::from_secs(30)),
                }
                .into()),
                now,
            );
        }

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

    /// Write down the moment received and stay briefly — a time measured after that is later than the written moment.
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

    // ── Driver thread ──

    /// A hand clock that counts reads — before a connection subscribes, the only one reading the clock is the scheduler's `tick`.
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

    /// One cell · the frame of the counting clock.
    fn counted_rig() -> (Rig, Arc<Counted>) {
        let clock = Arc::new(Counted {
            inner: ManualUsageClock::new(Duration::from_secs(100), T0),
            reads: AtomicUsize::new(0),
        });
        (rig_with(clock.clone(), 1), clock)
    }

    /// Until it finishes the first `tick` and goes to sleep — it read the clock and dropped the strong reference. If there is no subscription that sleep is the cap.
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
        // The 「갱신 중」 (refreshing) sheet arrives at carrying (outside the lock), not at the plan (inside the book lock) — wait for the sheet that arrived.
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
        // The prober blocks until it receives a script — the script sender stays in `rig` and nobody answers to the end.
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
        // A sender outside the service — even if the service disappears it is not `Disconnected`, so it must be woken to see the upgrade failure.
        let (wake, wakes) = mpsc::sync_channel(1);
        let handle = spawn_scheduler(&rig.service, wakes).expect("스케줄러 스레드");
        wait_asleep(&rig, &clock);

        drop(rig);
        assert!(!handle.is_finished(), "아직 상한 잠 안이다");
        wake.try_send(()).expect("잠든 스케줄러가 받는다");
        join_within(handle, "깨어 서비스가 없음을 봤다");
    }
}
