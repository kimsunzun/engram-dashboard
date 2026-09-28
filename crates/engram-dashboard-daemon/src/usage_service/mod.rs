//! 사용량 한도 서비스 — 벤더 중립.
//!
//! ★벤더 이름·키 목록·정책(쿨타임·시한)을 여기 두지 않는다★ — 전부
//! `engram_dashboard_agent::backend::usage_probes()` 에서 받는다. 벤더 match·벤더 리터럴이 이 모듈에 생기면
//! 「백엔드 확장」 위반이다.
//!
//! [`UsageService`] 가 책([`book::UsageBook`])·구독 명부([`watch::UsageWatch`])·시계·인코더·스케줄러 깨우기
//! 송신단을 쥐고 줍기 적용·구독 교체·발행을 몬다(TRD §1-4).
//! ★락 순서 = 명부 → 책 · 책은 잎이다★ — 책을 쥔 채 명부·출구·인코더·깨우기를 부르지 않는다.
//! ★발행은 「적용 먼저, 대상 열거 나중」이다★ — 책 락을 놓은 **뒤** 명부에서 대상을 뜨고, 어느 락도 없이
//!   나른다([`watch::deliver`]). 구독 교체의 「등록 먼저, 스냅숏 나중」과 짝을 이뤄 겹친 교체·발행이 그 연결에
//!   책의 값 이상을 닿게 한다(TRD §3 #57).
//! ★한 연결에 닿는 차례는 경로끼리 보장하지 않는다★ — 보내기가 락 밖이라 두 경로(줍기·교체·조회·스케줄러)의
//!   한 장이 뒤바뀌어 작은 revision 이 늦게 닿거나 같은 한 장이 두 번 닿을 수 있다. 받는 쪽이 칸마다 큰 revision
//!   만 남기는 것(TRD §1-7·§1-8)에 기댄 설계다.
// ADR-0004
// ADR-0006

pub mod book;
pub mod clock;
pub mod observe;
pub mod reject_store;
pub mod watch;

use std::collections::BTreeSet;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use engram_dashboard_agent::usage::{
    UsageAccountKey, UsageKey, UsageObservation, UsagePolicy, UsageVendorKey,
};
use engram_dashboard_net::frame_port::ConnId;
use engram_dashboard_protocol::UsageLimitSnapshot;

use book::{Coalesce, Now, UsageBook};
use clock::UsageClock;
use reject_store::RejectEntry;
use watch::{deliver, Replaced, UsageEncoder, UsageOutlet, UsageWatch};

/// 요청의 답이 그 요청이 기다린 조회에서 나온 값인가.
///
/// ★데몬 안에서만 쓴다 — wire 스냅숏은 싣지 않는다★: wire 스냅숏은 방송과 셸 캐시로만 흐르고(⟳ 답은
///   `Ack`) 거기선 늘 `Cached` 라 뜻이 없다(TRD §3 #50·#81). 싣는 곳은 버스 행이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageServed {
    /// 이 요청이 기다린 조회가 방금 받아 온 값이다.
    Fresh,
    /// 이번 요청 동안 새로 받은 값이 없다(조회 안 함 · 실패 · 기다림 상한 초과) — 서비스가 들고 있던 값이다.
    Cached,
}

/// 데몬마다 하나.
///
/// ★메서드는 전부 논블록이다★ — 잡는 락은 책·명부의 짧은 메모리 구간뿐이고, 보내기는 출구의 `try_send` ·
///   깨우기는 채널 `try_send` 다. 그래서 pump 스레드(줍기)와 연결 태스크(구독 교체)에서 곧장 부른다.
pub struct UsageService {
    book: Mutex<UsageBook>,
    watch: UsageWatch,
    clock: Arc<dyn UsageClock>,
    encoder: Arc<dyn UsageEncoder>,
    /// 스케줄러 깨우기 — 용량 1. ★사본을 밖(조회 스레드 등)에 주지 않는다★ — 주면 서비스가 사라져도 받는 쪽이
    /// `Disconnected` 를 못 봐 스케줄러가 끝나지 않는다.
    wake: SyncSender<()>,
}

impl UsageService {
    /// 칸 = 받은 벤더마다 기본 계정 하나([`UsageBook::new`] — wire 벤더로 못 바꾸는 벤더는 칸이 없다).
    ///
    /// 함께 돌려주는 수신단은 스케줄러 스레드 몫이다 — 깸(`()`)은 많아야 하나가 대기하고, 서비스가 drop 되면
    /// `Disconnected` 가 된다. 받는 쪽이 없어도(버렸음) 서비스는 그대로 돈다 — 깨우기만 허공에 간다.
    pub fn new(
        vendors: &[(UsageVendorKey, UsagePolicy)],
        clock: Arc<dyn UsageClock>,
        encoder: Arc<dyn UsageEncoder>,
    ) -> (Arc<Self>, Receiver<()>) {
        let (wake, wakes) = mpsc::sync_channel(1);
        let service = Arc::new(Self {
            book: Mutex::new(UsageBook::new(vendors)),
            watch: UsageWatch::new(),
            clock,
            encoder,
            wake,
        });
        (service, wakes)
    }

    /// 저장된 거절 기한을 되살린다 — 조립이 연결을 받기 전에 한 번 부른다. 불러온 수와 되살린 수를 info 로 남기고
    /// 되살린 수를 돌려준다. 되살린 칸의 발행은 빚으로 남는다 — 구독되면 스케줄러가 낸다.
    pub fn restore_rejects(&self, entries: &[RejectEntry]) -> usize {
        let restored = {
            let mut book = self.book();
            let now = self.now();
            book.restore_rejects(entries, now)
        };
        tracing::info!(
            loaded = entries.len(),
            restored,
            "저장된 사용량 거절 기한을 되살렸다"
        );
        restored
    }

    /// 줍기 관측 한 건 — pump 스레드가 그 자리에서 부른다(쌓지 않는다). 바뀐 것이 없으면 발행도 깨우기도 없다.
    /// 책에 칸이 없는 벤더면 아무것도 안 한다.
    pub fn observe(&self, obs: &UsageObservation) {
        let key = account_key(obs.vendor);
        let (applied, coalesce) = {
            let mut book = self.book();
            // 시각은 책 락 안에서 읽는다 — 두 pump 스레드의 적용 순서와 시각 순서가 어긋나지 않는다.
            let now = self.now();
            let applied = book.apply_passive(obs, now);
            let coalesce = if applied.changed {
                book.coalesce_passive(&key, now)
            } else {
                None
            };
            (applied, coalesce)
        };
        let deferred = matches!(coalesce, Some(Coalesce::Deferred));
        if let Some(Coalesce::PublishNow(sheet)) = coalesce {
            self.publish(key.vendor, &sheet);
        }
        if applied.next_auto_changed || deferred {
            self.wake_scheduler();
        }
    }

    /// `UsageSubscribe` — 그 연결의 구독 집합을 통째로 바꾼다(빈 집합 = 해제). 한 연결의 교체는 도착순으로 부른다.
    ///
    /// 차례: ① 요청 집합의 벤더마다 시각 평가 — 새긴 것(래치·거절 끝)이 있으면 그 벤더의 **지금** 구독자 전부에게
    /// 한 장 ② 명부 교체 — 새로 든 벤더마다 첫 한 장을 이 연결에만 ③ 스케줄러 깨우기(첫 한 장은 빚을 갚지 않고,
    /// 새로 든 벤더의 자동 조회 기한이 이미 지났을 수 있다). 끊긴 뒤 늦게 든 교체는 버린다 — 칸을 되살리지 않고
    /// 그 연결에는 아무것도 보내지 않는다(① 의 래치 발행은 그 벤더의 다른 구독자에게 나간다). 책에 칸이 없는
    /// 벤더는 명부 집합에만 들고 한 장이 없다.
    pub fn replace_subscription(&self, conn: ConnId, vendors: BTreeSet<UsageVendorKey>) {
        let latched: Vec<(UsageVendorKey, UsageLimitSnapshot)> = {
            let mut book = self.book();
            let now = self.now();
            vendors
                .iter()
                .filter_map(|&vendor| {
                    let key = account_key(vendor);
                    if !book.eval_time(&key, now) {
                        return None;
                    }
                    book.broadcast_sheet(&key, now).map(|sheet| (vendor, sheet))
                })
                .collect()
        };
        for (vendor, sheet) in &latched {
            self.publish(*vendor, sheet);
        }

        // 명부 락 안에서 불린다(명부 → 책) — 책의 메모리 일만 한다.
        let replaced = self.watch.replace(conn, vendors, |added| {
            let mut book = self.book();
            let now = self.now();
            added
                .iter()
                .filter_map(|&vendor| {
                    let key = account_key(vendor);
                    book.eval_time(&key, now);
                    book.snapshot(&key, now)
                })
                .collect()
        });
        let Some(Replaced {
            first_sheets,
            target,
        }) = replaced
        else {
            tracing::debug!(conn, "끊긴 연결의 사용량 구독 교체가 늦게 왔다 — 버린다");
            return;
        };
        for sheet in &first_sheets {
            deliver(sheet, vec![target.clone()], self.encoder.as_ref());
        }
        self.wake_scheduler();
    }

    /// 연결이 붙었다(`on_connect`) — 빈 집합의 명부 칸([`UsageWatch::attach`]).
    pub fn attach(&self, conn: ConnId, outlet: impl UsageOutlet + 'static) {
        self.watch.attach(conn, outlet);
    }

    /// 연결이 끊겼다(`on_disconnect`) — 명부 칸을 지우고 그 출구를 **반환 전에** 거둔다([`UsageWatch::detach`]).
    /// 반환 = 칸이 있었다.
    pub fn detach(&self, conn: ConnId) -> bool {
        self.watch.detach(conn)
    }

    /// 빚을 갚은 한 장을 그 벤더의 지금 구독자 전부에게 나른다. ★어느 락도 쥐지 않은 채 부른다★.
    fn publish(&self, vendor: UsageVendorKey, sheet: &UsageLimitSnapshot) {
        deliver(sheet, self.watch.targets(vendor), self.encoder.as_ref());
    }

    fn wake_scheduler(&self) {
        // `Full` = 깸 하나가 이미 대기 중이라 같은 뜻이다 · `Disconnected` = 받는 스케줄러가 없다. 어느 쪽이든
        //   깨우는 쪽(pump 스레드 포함)을 막지 않는다.
        let _ = self.wake.try_send(());
    }

    fn now(&self) -> Now {
        Now {
            mono: self.clock.mono(),
            wall: self.clock.wall(),
        }
    }

    fn book(&self) -> MutexGuard<'_, UsageBook> {
        // poison 을 견딘다 — 여기서 패닉하면 부른 pump 스레드가 죽는다. 책의 메서드는 패닉하지 않으므로 poison 은
        //   debug·시험에서만 선다(릴리즈는 `panic = "abort"`).
        self.book.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// 벤더의 기본 계정 칸 — 계정 낱말은 지금 늘 기본값이다(`UsageAccountKey`).
fn account_key(vendor: UsageVendorKey) -> UsageKey {
    UsageKey {
        vendor,
        account: UsageAccountKey::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage_service::clock::ManualUsageClock;
    use crate::usage_service::watch::tests::{recording, words, Counting, Log};
    use crate::usage_service::watch::UsageFrame;
    use engram_dashboard_agent::backend::usage_probes;
    use engram_dashboard_agent::usage::{UsageSource, WindowObs};
    use engram_dashboard_protocol::{AgentBackendKind, UsageVendorState};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Barrier, OnceLock, Weak};
    use std::time::Duration;

    const H: i64 = 3_600;
    const T0: i64 = 1_900_000_000;

    // 조회기 키로만 칸을 만든다 — 시험에도 벤더 리터럴을 두지 않는다.
    fn vendor(i: usize) -> UsageVendorKey {
        usage_probes()[i].key()
    }

    fn key(i: usize) -> UsageKey {
        account_key(vendor(i))
    }

    fn set(indices: &[usize]) -> BTreeSet<UsageVendorKey> {
        indices.iter().map(|&i| vendor(i)).collect()
    }

    fn all_vendors() -> Vec<(UsageVendorKey, UsagePolicy)> {
        usage_probes()
            .iter()
            .map(|p| (p.key(), p.policy()))
            .collect()
    }

    pub(super) struct Rig {
        pub(super) service: Arc<UsageService>,
        wakes: Receiver<()>,
        encoder: Arc<Counting>,
    }

    fn rig_with(clock: Arc<dyn UsageClock>, vendors: &[(UsageVendorKey, UsagePolicy)]) -> Rig {
        let encoder = Arc::new(Counting::default());
        let (service, wakes) = UsageService::new(vendors, clock, encoder.clone());
        Rig {
            service,
            wakes,
            encoder,
        }
    }

    pub(super) fn rig() -> (Rig, Arc<ManualUsageClock>) {
        let clock = Arc::new(ManualUsageClock::new(Duration::from_secs(100), T0));
        (rig_with(clock.clone(), &all_vendors()), clock)
    }

    impl Rig {
        /// 붙이고 구독까지 — 첫 한 장과 깸은 비워 둔다.
        pub(super) fn subscriber(&self, conn: ConnId, indices: &[usize]) -> Arc<Log> {
            let (outlet, log) = recording();
            self.service.attach(conn, outlet);
            self.service.replace_subscription(conn, set(indices));
            log.frames.lock().unwrap().clear();
            self.drain_wakes();
            log
        }

        fn drain_wakes(&self) -> usize {
            self.wakes.try_iter().count()
        }

        fn revision(&self, i: usize) -> u64 {
            self.service.book().revision(&key(i)).expect("아는 키")
        }

        fn wire(&self, i: usize) -> AgentBackendKind {
            let now = self.service.now();
            self.service
                .book()
                .snapshot(&key(i), now)
                .expect("아는 키")
                .vendor
        }
    }

    pub(super) fn five(i: usize, pct: f64, reset: i64) -> UsageObservation {
        UsageObservation {
            vendor: vendor(i),
            five_hour: Some(WindowObs {
                used_pct: Some(pct),
                resets_at: Some(reset),
            }),
            weekly: None,
            model_scoped: None,
            plan: None,
            source: UsageSource::Passive,
            limits_unavailable: None,
        }
    }

    pub(super) fn far_reset() -> i64 {
        T0 + 5 * H
    }

    fn both(i: usize, five_pct: f64, weekly_pct: f64) -> UsageObservation {
        UsageObservation {
            weekly: Some(WindowObs {
                used_pct: Some(weekly_pct),
                resets_at: Some(T0 + 100 * H),
            }),
            ..five(i, five_pct, far_reset())
        }
    }

    fn frames(log: &Log) -> Vec<UsageFrame> {
        log.frames.lock().unwrap().clone()
    }

    pub(super) fn last_pct(log: &Log) -> Option<f64> {
        frames(log)
            .last()
            .and_then(|f| f.snapshot.five_hour.as_ref())
            .and_then(|w| w.used_pct)
    }

    // ── 줍기 ──

    #[test]
    fn an_observed_change_goes_to_that_vendors_subscribers_only_and_a_repeat_sends_nothing() {
        let (rig, _clock) = rig();
        let a = rig.subscriber(1, &[0]);
        let b = rig.subscriber(2, &[1]);
        let c = rig.subscriber(3, &[]);

        rig.service.observe(&five(0, 40.0, far_reset()));
        let got = frames(&a);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].snapshot.revision, rig.revision(0));
        assert_eq!(&*got[0].json, words(&set(&[0])));
        assert_eq!((b.count(), c.count()), (0, 0));

        rig.service.observe(&five(0, 40.0, far_reset()));
        assert_eq!(a.count(), 1, "같은 값을 다시 주웠다 — 바뀜이 아니다");
    }

    #[test]
    fn without_subscribers_the_book_still_moves_and_nothing_is_encoded() {
        let (rig, _clock) = rig();
        let before = rig.revision(0);
        rig.service.observe(&five(0, 40.0, far_reset()));
        assert!(rig.revision(0) > before);
        assert_eq!(rig.encoder.calls(), 0);
    }

    #[test]
    fn a_change_inside_the_coalesce_window_waits_for_the_scheduler() {
        let (rig, clock) = rig();
        let a = rig.subscriber(1, &[0]);
        rig.service.observe(&five(0, 40.0, far_reset()));
        rig.drain_wakes();

        clock.advance_both(Duration::from_millis(500));
        rig.service.observe(&five(0, 41.0, far_reset()));
        assert_eq!(a.count(), 1, "1초 안의 두 번째 바뀜은 합친다");
        assert_eq!(rig.drain_wakes(), 1, "끝 발행은 스케줄러가 낸다");
    }

    #[test]
    fn an_observation_that_moves_the_next_auto_query_wakes_the_scheduler_once() {
        let (rig, clock) = rig();
        let a = rig.subscriber(1, &[0]);
        {
            let now = rig.service.now();
            let mut book = rig.service.book();
            assert!(book.begin_probe(&key(0)));
            book.finish_probe(&key(0), Ok(both(0, 10.0, 10.0)), now);
            book.broadcast_sheet(&key(0), now);
        }

        clock.advance_both(Duration::from_secs(2));
        rig.service.observe(&five(0, 20.0, far_reset()));
        assert_eq!(
            (a.count(), rig.drain_wakes()),
            (1, 0),
            "한 창만 새로 섰다 — 다음 자동 조회는 그대로"
        );

        clock.advance_both(Duration::from_secs(2));
        let now = rig.service.now();
        let before = rig.service.book().next_auto(&key(0), now);
        rig.service.observe(&both(0, 30.0, 30.0));
        assert_eq!(
            (a.count(), rig.drain_wakes()),
            (2, 1),
            "두 창이 새로 서 쿨타임 기점이 옮겨졌다(D12)"
        );
        assert_ne!(rig.service.book().next_auto(&key(0), now), before);
    }

    #[test]
    fn an_observation_still_applies_after_the_book_lock_is_poisoned() {
        let (rig, _clock) = rig();
        let a = rig.subscriber(1, &[0]);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _held = rig.service.book.lock().unwrap();
            panic!("시험 — 책 락을 쥔 채 패닉");
        }));
        assert!(panicked.is_err());
        assert!(rig.service.book.is_poisoned(), "시험이 poison 갈래를 탔다");

        rig.service.observe(&five(0, 40.0, far_reset()));
        assert_eq!(last_pct(&a), Some(40.0));
    }

    // 조회 시작(곧바로 갚을 빚)을 아직 안 냈으면 1초 안의 줍기가 그것까지 곧바로 낸다.
    #[test]
    fn a_passive_right_after_an_unpublished_probe_start_publishes_at_once() {
        let (rig, clock) = rig();
        let a = rig.subscriber(1, &[0]);
        rig.service.observe(&five(0, 40.0, far_reset()));
        assert!(rig.service.book().begin_probe(&key(0)));

        clock.advance_both(Duration::from_millis(500));
        rig.service.observe(&five(0, 42.0, far_reset()));
        let got = frames(&a);
        assert_eq!(got.len(), 2);
        assert!(got[1].snapshot.in_flight);
        assert_eq!(last_pct(&a), Some(42.0));
        assert_eq!(got[1].snapshot.revision, rig.revision(0));
    }

    #[test]
    fn a_vendor_without_a_cell_gets_no_sheet_and_its_observation_does_nothing() {
        let clock = Arc::new(ManualUsageClock::new(Duration::from_secs(100), T0));
        let rig = rig_with(clock, &all_vendors()[..1]);
        let (outlet, log) = recording();
        rig.service.attach(1, outlet);
        rig.service.replace_subscription(1, set(&[0, 1]));
        let got = frames(&log);
        assert_eq!(got.len(), 1, "칸이 있는 벤더만 첫 한 장");
        assert_eq!(&*got[0].json, words(&set(&[0, 1])), "집합은 요청 그대로");
        rig.drain_wakes();

        rig.service.observe(&five(1, 40.0, far_reset()));
        assert_eq!(log.count(), 1);
        assert_eq!(rig.drain_wakes(), 0);
    }

    // ── 구독 교체 ──

    #[test]
    fn a_latch_due_at_subscription_is_published_to_existing_subscribers_too() {
        let (rig, clock) = rig();
        let a = rig.subscriber(1, &[0]);
        rig.service.observe(&five(0, 40.0, T0 + 10));
        assert_eq!(a.count(), 1);
        clock.set_wall(T0 + 20);

        let (outlet, b) = recording();
        rig.service.attach(2, outlet);
        rig.service.replace_subscription(2, set(&[0]));

        let latched = frames(&a);
        assert_eq!(latched.len(), 2, "기존 구독자에게 래치 한 장");
        let sheet = &latched[1].snapshot;
        assert!(sheet.five_hour.as_ref().expect("창").expired);
        assert_eq!(sheet.revision, rig.revision(0));
        let first = frames(&b);
        assert_eq!(first.len(), 1, "새 구독자는 첫 한 장만");
        assert_eq!(first[0].snapshot, *sheet);
        assert_eq!(rig.drain_wakes(), 1);

        rig.service.replace_subscription(1, set(&[0]));
        rig.service.replace_subscription(2, set(&[0]));
        assert_eq!(
            (a.count(), b.count()),
            (2, 1),
            "같은 집합 교체 — 한 장도 없다"
        );
    }

    #[test]
    fn a_first_sheet_goes_to_the_new_subscriber_only_and_wakes_the_scheduler() {
        let (rig, _clock) = rig();
        let a = rig.subscriber(1, &[0]);
        rig.service.observe(&five(0, 40.0, far_reset()));
        let (outlet, b) = recording();
        rig.service.attach(2, outlet);
        rig.drain_wakes();

        rig.service.replace_subscription(2, set(&[0, 1]));
        assert_eq!(a.count(), 1);
        let first = frames(&b);
        assert_eq!(first.len(), 2, "새로 든 벤더마다 한 장");
        assert!(first.iter().all(|f| &*f.json == words(&set(&[0, 1]))));
        assert_eq!(rig.drain_wakes(), 1);
    }

    #[test]
    fn a_late_replace_after_detach_is_dropped() {
        let (rig, _clock) = rig();
        let (outlet, log) = recording();
        rig.service.attach(1, outlet);
        assert!(rig.service.detach(1));
        assert_eq!(log.revokes(), 1);
        rig.service.observe(&five(0, 40.0, far_reset()));
        rig.drain_wakes();

        rig.service.replace_subscription(1, set(&[0]));
        assert_eq!(log.count(), 0);
        assert_eq!(Arc::strong_count(&log), 1, "출구가 되살아나지 않았다");
        assert!(rig.service.watch.union().is_empty());
        assert_eq!(rig.drain_wakes(), 0);
        assert_eq!(rig.encoder.calls(), 0);
    }

    #[test]
    fn a_late_replace_after_detach_still_publishes_a_due_latch_to_the_other_subscribers() {
        let (rig, clock) = rig();
        let other = rig.subscriber(2, &[0]);
        let (outlet, gone) = recording();
        rig.service.attach(1, outlet);
        assert!(rig.service.detach(1));
        rig.service.observe(&five(0, 40.0, T0 + 10));
        clock.set_wall(T0 + 20);

        rig.service.replace_subscription(1, set(&[0]));
        let got = frames(&other);
        assert_eq!(got.len(), 2, "줍기 한 장 + 래치 한 장");
        assert!(got[1].snapshot.five_hour.as_ref().expect("창").expired);
        assert_eq!(gone.count(), 0, "끊긴 연결에는 아무것도 없다");
    }

    #[test]
    fn adding_a_vendor_while_another_latches_sends_the_latch_first_then_the_first_sheet() {
        let (rig, clock) = rig();
        let a = rig.subscriber(1, &[0]);
        rig.service.observe(&five(0, 40.0, T0 + 10));
        a.frames.lock().unwrap().clear();
        clock.set_wall(T0 + 20);

        rig.service.replace_subscription(1, set(&[0, 1]));
        let got = frames(&a);
        let order: Vec<_> = got
            .iter()
            .map(|f| (f.snapshot.vendor, f.json.to_string()))
            .collect();
        assert_eq!(
            order,
            [
                (rig.wire(0), words(&set(&[0]))),
                (rig.wire(1), words(&set(&[0, 1]))),
            ],
            "래치(교체 전 집합) 다음 첫 한 장(교체 뒤 집합)"
        );
        assert!(got[0].snapshot.five_hour.as_ref().expect("창").expired);
    }

    // ── 락 ──

    /// 처음 받을 때 서비스를 다시 부르는 출구 — 줍기·교체·자기 해제.
    struct CallsBack {
        service: Arc<OnceLock<Weak<UsageService>>>,
        calls: Arc<AtomicUsize>,
    }

    impl UsageOutlet for CallsBack {
        fn send(&self, _frame: &UsageFrame) {
            if self.calls.fetch_add(1, Ordering::SeqCst) > 0 {
                return;
            }
            let service = self.service.get().and_then(Weak::upgrade).expect("서비스");
            service.observe(&five(0, 77.0, far_reset()));
            service.replace_subscription(1, set(&[0, 1]));
            service.detach(1);
        }

        fn revoke(&self) {}
    }

    #[test]
    fn an_outlet_may_call_back_into_the_service() {
        // 교착이면 매달리는 대신 실패하도록 다른 스레드에서 돌린다.
        let (done_tx, done_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (rig, _clock) = rig();
            let slot = Arc::new(OnceLock::new());
            slot.set(Arc::downgrade(&rig.service)).expect("한 번");
            let calls = Arc::new(AtomicUsize::new(0));
            rig.service.attach(
                1,
                CallsBack {
                    service: slot,
                    calls: calls.clone(),
                },
            );
            rig.service.replace_subscription(1, set(&[0]));
            let union = rig.service.watch.union();
            done_tx.send((calls.load(Ordering::SeqCst), union)).unwrap();
        });
        let (calls, union) = done_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("교착 없이 끝난다");
        assert!(calls >= 1);
        assert!(union.is_empty(), "출구 안에서 부른 해제가 칸을 지웠다");
    }

    // ── 동시성 ──

    /// 읽을 때마다 단조 시간이 2초 흐르는 시계 — 바뀜마다 합침 창이 이미 닫혀 곧바로 나간다(이 시험엔 끝 발행을
    /// 낼 스케줄러가 없다). 벽시계는 멈춰 있어 래치가 서지 않는다.
    struct Ticking(AtomicU64);

    impl UsageClock for Ticking {
        fn mono(&self) -> Duration {
            Duration::from_secs(self.0.fetch_add(2, Ordering::SeqCst))
        }

        fn wall(&self) -> i64 {
            T0
        }
    }

    /// 연결 둘이 구독을 바꾸는 동안 줍기 둘이 두 벤더를 바꾸는 판 하나 — 겹침은 판마다 달라지고 일의 양은 정해져
    /// 있다. 연결 0 은 정해진 만큼 바꾸다 비우고 판이 끝난 뒤 다시 구독한다. 연결 1 은 정해진 만큼 바꾼 뒤 비우고,
    /// 줍기 0 의 마지막 바뀜 둘과 **같은 순간에** 다시 구독한다 — 마지막 값이 발행으로 닿을지 첫 한 장으로 닿을지는
    /// 그 겹침이 정한다(TRD §3 #57 의 두 차례가 맞물리는 자리). 돌려주는 것 = 어긋난 칸마다 한 줄.
    fn race_trial(trial: usize) -> Vec<String> {
        const OBSERVERS: usize = 2;
        const OBSERVATIONS: usize = 16;
        const REPLACES: usize = 8;
        const CHURN: usize = 24;
        let rig = rig_with(Arc::new(Ticking(AtomicU64::new(100))), &all_vendors());
        let logs: Vec<Arc<Log>> = (0..2)
            .map(|conn| {
                let (outlet, log) = recording();
                rig.service.attach(conn, outlet);
                log
            })
            .collect();
        let cycle: [&[usize]; 5] = [&[0], &[0, 1], &[1], &[], &[1, 0]];
        let barrier = Arc::new(Barrier::new(2 + OBSERVERS));
        let endgame = Arc::new(Barrier::new(2));

        let mut threads = Vec::new();
        {
            let (service, barrier) = (rig.service.clone(), barrier.clone());
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                for round in 0..CHURN {
                    service.replace_subscription(0, set(cycle[round % cycle.len()]));
                }
                service.replace_subscription(0, set(&[]));
            }));
        }
        {
            let (service, barrier, endgame) =
                (rig.service.clone(), barrier.clone(), endgame.clone());
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                for round in 0..REPLACES {
                    service.replace_subscription(1, set(cycle[(round + 1) % cycle.len()]));
                }
                service.replace_subscription(1, set(&[]));
                endgame.wait();
                service.replace_subscription(1, set(&[0, 1]));
            }));
        }
        for observer in 0..OBSERVERS {
            let (service, barrier, endgame) =
                (rig.service.clone(), barrier.clone(), endgame.clone());
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                for n in 0..OBSERVATIONS {
                    let pct = ((observer * 7_919 + n * 13) % 1_000) as f64 / 10.0;
                    service.observe(&five(n % 2, pct, far_reset()));
                }
                if observer == 0 {
                    // 앞의 값은 모두 100 아래라 늘 바뀜이다.
                    endgame.wait();
                    for i in 0..2 {
                        service.observe(&five(i, 100.0, far_reset()));
                    }
                }
            }));
        }
        for thread in threads {
            thread.join().expect("시험 스레드");
        }
        rig.service.replace_subscription(0, set(&[0, 1]));

        let mut misses = Vec::new();
        for i in 0..2 {
            let (wire, last) = (rig.wire(i), rig.revision(i));
            for (conn, log) in logs.iter().enumerate() {
                let highest = frames(log)
                    .iter()
                    .filter(|f| f.snapshot.vendor == wire)
                    .map(|f| f.snapshot.revision)
                    .max();
                if last == 0 || highest != Some(last) {
                    misses.push(format!(
                        "판 {trial} · 연결 {conn} · 벤더 {i}: 받은 최고 {highest:?} · 책 {last}"
                    ));
                }
            }
        }
        misses
    }

    /// 판마다 그 연결이 받은 최고 revision = 책의 마지막 revision. 올바른 코드에서는 겹침과 무관하게 선다 — 시계가
    /// 읽을 때마다 2초 흘러 바뀜마다 곧바로 나가고, 벽시계가 멈춰 래치가 없다.
    #[test]
    fn racing_replaces_and_observations_leave_each_subscriber_at_the_final_revision() {
        const TRIALS: usize = 2_000;
        // 락 차례가 깨져 교착하면 매달리는 대신 실패하도록 다른 스레드에서 돌린다.
        let (done_tx, done_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let misses: Vec<String> = (0..TRIALS).flat_map(race_trial).collect();
            done_tx.send(misses).unwrap();
        });
        let misses = done_rx
            .recv_timeout(Duration::from_secs(120))
            .expect("시험 스레드가 끝나지 않았다(교착 또는 패닉)");
        let total = misses.len();
        assert!(
            misses.is_empty(),
            "{total}건 어긋남 — 처음: {:?}",
            misses.first()
        );
    }

    // ── 거절 복원 ──

    #[test]
    fn restored_rejects_are_counted_and_the_cell_reads_rejected() {
        let (rig, _clock) = rig();
        let entries = [
            RejectEntry {
                key: key(0),
                until_epoch_s: T0 + 600,
            },
            RejectEntry {
                key: key(1),
                until_epoch_s: T0 - 1,
            },
        ];
        assert_eq!(
            rig.service.restore_rejects(&entries),
            1,
            "지난 항목은 버린다"
        );
        let now = rig.service.now();
        let sheet = rig.service.book().snapshot(&key(0), now).expect("아는 키");
        assert!(matches!(sheet.state, UsageVendorState::Rejected { .. }));
    }
}
