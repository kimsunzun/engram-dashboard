//! Usage scheduler — one thread per daemon (`usage-scheduler`) wakes at the earliest of the subscribed vendors' automatic query deadline · reset time · refusal end · coalescing deadline and carries out the plan of [`UsageBook::plan_tick`](super::book::UsageBook::plan_tick) (TRD §1-4 「scheduler」 · §3 #58).
//!
//! ★The thread holds the service only as a `Weak` — while asleep it has no strong reference★: when the service disappears the wake sender disappears too (`Disconnected`) and the thread ends by itself, even with a blocked query in flight; the shutdown procedure has no step.
//! ★The decision after waking is always measured by the service's clock★ — the wait timer's clock may not count sleep time (TRD §6 #2). The sleep cap [`SCHEDULE_MAX_SLEEP`] bounds lateness after resume or a wall-clock shift.
//! Entry point = [`spawn_scheduler`] — the assembly calls it once with the receiver [`UsageService::new`] gave.
// ADR-0006

use std::io;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::book::SCHEDULE_MAX_SLEEP;
use super::UsageService;

/// Wait floor — a shorter wait is raised to this (that deadline is late by at most this much), so a leaked 0 (a plan with a broken contract · a deadline passing while carrying out) does not make this thread hammer the book lock.
const SCHEDULE_MIN_SLEEP: Duration = Duration::from_millis(10);

/// Launches the scheduler thread. `wakes` = the receiver [`UsageService::new`] returned together — another receiver cannot hear the service's wakes (subscription replacement · query end · pickup), and even on service drop the thread ends late by up to the sleep cap.
///
/// A dropped handle runs detached (it ends by itself when the service disappears). `Err` = the thread could not be launched — the service runs on but everything time drives stops: automatic queries · deadline-reached emission · collecting the unrepaid emission debt (TRD §3 #86). Requests (⟳ · bus) · pickup · subscription replacement run on. What to leave behind is the caller's share.
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
        // Drop it before sleeping — if this thread is the last owner the service disappears here and the sender with it, so the wait below sees `Disconnected` at once.
        drop(strong);
        match wakes.recv_timeout(next.wait()) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    tracing::debug!("the usage service is gone — ending the scheduler");
}

/// Result of one pass — wake after `sleep` counted from the moment the plan read the time (`planned`).
struct Tick {
    planned: Instant,
    sleep: Duration,
}

impl Tick {
    /// Time to wait from now — subtract what has flowed since the plan (including carrying out) and clamp to [`SCHEDULE_MIN_SLEEP`, `SCHEDULE_MAX_SLEEP`]; a deadline that passed meanwhile is planned again right after the floor.
    fn wait(&self) -> Duration {
        bounded(self.sleep.saturating_sub(self.planned.elapsed()))
    }
}

/// One scheduler pass — take a plan and carry it out. Lock order = registry {union copy} → book {plan} → emission · query launch without any lock (TRD §1-4 path table 「scheduler `tick`」).
fn tick(service: &UsageService) -> Tick {
    let subscribed = service.watch.union();
    let (plan, planned) = {
        let mut desk = service.desk();
        // Time is read inside the book lock so other paths' application order and time order stay consistent.
        let now = service.now();
        // Elapsed time is measured by the clock the wait (`recv_timeout`) measures, not the service clock.
        let planned = Instant::now();
        (desk.book.plan_tick(&subscribed, now), planned)
    };
    Tick {
        planned,
        sleep: service.carry_out(plan),
    }
}

fn bounded(sleep: Duration) -> Duration {
    // Do not use `clamp` — it panics if floor > cap (outside the containment mark, ending the daemon · ADR-0292).
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

    /// Stamps one finished query onto the cell — the automatic query deadline is pushed past the cooldown so `tick` does not open a query; also repays the debt.
    fn settle(rig: &Rig, i: usize, obs: UsageObservation) {
        let now = rig.service.now();
        let mut desk = rig.service.desk();
        assert!(desk.book.begin_probe(&key(i)));
        desk.book.finish_probe(&key(i), Ok(obs), now);
        desk.book.broadcast_sheet(&key(i), now);
    }

    /// Launches the scheduler — takes the frame's wake receiver and passes it (the frame cannot count wakes after that).
    fn launch(rig: &mut Rig) -> JoinHandle<()> {
        let wakes = std::mem::replace(&mut rig.wakes, mpsc::sync_channel(1).1);
        spawn_scheduler(&rig.service, wakes).expect("scheduler thread")
    }

    /// Must end within the cap — far shorter than the sleep cap (60 seconds), so ending here means a wake or a drop ended it.
    fn join_within(handle: JoinHandle<()>, what: &str) {
        wait_until(what, || handle.is_finished());
        handle.join().expect("the scheduler thread does not panic");
    }

    // ── tick ──

    #[test]
    fn a_tick_without_subscribers_starts_nothing_publishes_nothing_and_sleeps_the_cap() {
        let (rig, _clock) = rig();
        let idle = rig.subscriber(1, &[]);
        {
            // A cell that left a debt — a query that ended in refusal was stamped without emitting.
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
            "with no reference point, it is outside the subscription even past the deadline"
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
            "the deadline of an in-progress cell is not counted"
        );
        assert!(rig.in_flight(0));
        assert!(!rig.in_flight(1), "outside the subscription");
        let got = frames(&log);
        assert_eq!(got.len(), 1);
        assert!(got[0].snapshot.in_flight, "one 「refreshing」 sheet");

        let joined = bus_request(&rig.service, 0, RequestKind::Refresh);
        wait_until("⟳ joined", || rig.joiners(0) == 1);
        tick(&rig.service);
        assert_eq!(
            log.count(),
            1,
            "in progress — neither a newly opened query nor a sheet to emit"
        );

        rig.answer(0, ok(active(0, 42.0)));
        assert_eq!(answered(&joined).served, UsageServed::Fresh);
        assert_eq!(
            rig.queries(0),
            1,
            "joined the one query the scheduler opened"
        );
        let got = frames(&log);
        assert_eq!(got.len(), 2);
        assert!(!got[1].snapshot.in_flight);
        assert_eq!(pct(&got[1].snapshot), Some(42.0));

        assert_eq!(
            tick(&rig.service).sleep,
            SCHEDULE_MAX_SLEEP,
            "next deadline = after the cooldown"
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
        assert_eq!(log.count(), 1, "outside the merge window — right away");
        clock.advance_both(Duration::from_millis(400));
        rig.service.observe(&five(0, 41.0, far_reset()));
        assert_eq!(
            log.count(),
            1,
            "inside the window — the closing emit is the scheduler's job"
        );

        assert_eq!(tick(&rig.service).sleep, Duration::from_millis(600));
        assert_eq!(log.count(), 1, "before the deadline");
        clock.advance_both(Duration::from_millis(600));
        assert_eq!(tick(&rig.service).sleep, SCHEDULE_MAX_SLEEP);
        assert_eq!(log.count(), 2);
        assert_eq!(last_pct(&log), Some(41.0));
        tick(&rig.service);
        assert_eq!(log.count(), 2, "the debt is paid once");
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
            "a wait below the floor is raised to the floor"
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
            "until the reset time"
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
        assert_eq!(log.count(), 1, "one latch sheet, once");
    }

    #[test]
    fn the_wait_subtracts_the_time_elapsed_since_the_plan() {
        let ago = |ms| {
            Instant::now()
                .checked_sub(Duration::from_millis(ms))
                .expect("after the monotonic clock origin")
        };
        let passed = Tick {
            planned: ago(100),
            sleep: Duration::from_millis(60),
        };
        assert_eq!(
            passed.wait(),
            SCHEDULE_MIN_SLEEP,
            "the deadline passed while carrying — plan again soon after the floor"
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

    /// Records the moment received and stays briefly — a time measured after that is later than the recorded moment.
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
        assert_eq!(sent.len(), 1, "the one 「refreshing」 sheet from carrying");
        assert!(
            next.planned <= sent[0],
            "the plan time is measured before carrying"
        );
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

    /// A hand clock that counts reads — before a connection subscribes, the only reader is the scheduler's `tick`.
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

    /// One cell · the counting clock's frame.
    fn counted_rig() -> (Rig, Arc<Counted>) {
        let clock = Arc::new(Counted {
            inner: ManualUsageClock::new(Duration::from_secs(100), T0),
            reads: AtomicUsize::new(0),
        });
        (rig_with(clock.clone(), 1), clock)
    }

    /// Until it finishes the first `tick` and sleeps (it read the clock and dropped the strong reference). Without a subscription that sleep is the cap.
    fn wait_asleep(rig: &Rig, clock: &Counted) {
        wait_until("slept after the first tick", || {
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
        // The 「refreshing」 sheet arrives at carrying out (outside the lock), not at the plan (inside the book lock) — wait for the sheet that arrived.
        wait_until(
            "the 「refreshing」 sheet of the newly added vendor arrived after waking",
            || frames(&log).last().is_some_and(|f| f.snapshot.in_flight),
        );
        assert!(rig.in_flight(0));

        drop(rig);
        join_within(handle, "ended because the service is gone");
    }

    #[test]
    fn the_asleep_scheduler_holds_no_service_so_a_drop_ends_it_even_during_a_stuck_probe() {
        let (mut rig, _clock) = rig();
        let _log = rig.subscriber(1, &[0]);
        let handle = launch(&mut rig);
        // The prober blocks until it receives a script — the script sender stays in `rig` and nobody answers to the end.
        wait_until("opened a query and slept", || {
            rig.queries(0) == 1 && Arc::strong_count(&rig.service) == 1
        });

        let service = rig.service;
        let weak = Arc::downgrade(&service);
        drop(service);
        assert!(
            weak.upgrade().is_none(),
            "a sleeping scheduler does not hold on to the service"
        );
        join_within(handle, "ended even while a blocked query is running");
    }

    #[test]
    fn a_closed_wake_channel_ends_the_scheduler_while_the_service_lives() {
        let (rig, _clock) = rig();
        let (wake, wakes) = mpsc::sync_channel(1);
        drop(wake);
        let handle = spawn_scheduler(&rig.service, wakes).expect("scheduler thread");
        join_within(handle, "there is no sender");
        assert_eq!(Arc::strong_count(&rig.service), 1);
    }

    #[test]
    fn a_wake_after_the_service_is_gone_ends_the_scheduler() {
        let (rig, clock) = counted_rig();
        // A sender outside the service — it is not `Disconnected` even if the service disappears, so it must be woken to see the upgrade failure.
        let (wake, wakes) = mpsc::sync_channel(1);
        let handle = spawn_scheduler(&rig.service, wakes).expect("scheduler thread");
        wait_asleep(&rig, &clock);

        drop(rig);
        assert!(!handle.is_finished(), "still within the capped sleep");
        wake.try_send(())
            .expect("the sleeping scheduler receives it");
        join_within(handle, "woke and saw the service is gone");
    }
}
