//! 슬롯 도킹의 끌기 쪽(TRD S21-slot-docking §1-3 · §2) — 레이아웃 권위(`crate::layout` · ADR-0035)를 부르는
//! 손님이다. 레이아웃은 여기서 바꾸지 않는다 — 바꾸는 길은 적용 서비스(`layout::apply`) 하나다.

pub mod book;
pub mod geometry;
pub mod hit;
pub mod parked;
pub mod ports;
pub mod regions;
pub mod session;
pub mod sizing;
pub mod snap;
pub mod strip;
pub mod targets;
pub mod types;
pub mod units;
pub mod wire;
pub mod world;

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use engram_dashboard_base::sync;

use book::{Book, Snap};
use parked::{Measure, Parked};
use snap::StartMaterial;
use wire::WindowRole;

use crate::layout::{LayoutError, ViewManager};

/// 셸에 하나인 끌기 상태 — 장부(잎 락 · 규율 = `book` 머리) · 레이아웃 락 안에서 쓰는 두 셈 · 대기 창 명부.
///
/// - 스냅숏 번호는 1 부터 오르고 0 을 내지 않는다(장부는 마지막으로 먹은 번호 이하를 버린다 — 처음 값이 0). 번호를
///   다는 길은 [`Self::snapshot`] · [`Self::start_material`] 둘뿐이고 둘 다 `&ViewManager` 를 받는다 — 레이아웃 락
///   가드 밖에서는 그 참조가 없으니 번호와 재료가 같은 락 구간에서 뜬다(TRD §1-3 「스냅숏 번호」).
/// - 세션 세대: 0 = 세션 없음. 시작의 레이아웃 락 안에서 [`Self::begin_generation`] 으로 세우고, 끝에서
///   [`Self::end_generation`] 으로 자기 세대일 때만 내린다(TRD §1-3 「시작」 2 — 무조건 내리면 그 사이 선 다음
///   세션의 깃발까지 내려 그 세션의 계기가 끊긴다).
/// - 실패 셈: 실행기(`commands::dock`)의 emit · 창 호출 · 게터 실패 수 — 끝 줄의 `errors`(TRD §1-7). 받아들인 시작이
///   그 세대 몫으로 열고([`Self::open_errors`]) 그 끝 줄이 비우며 닫는다([`Self::take_errors`]). 세대를 단 실패
///   ([`Self::note_error_of`])는 그 세대 셈이 열려 있을 때만 센다 — 끝 줄 뒤의 메인 스레드 실패는 세지 않고 세대마다
///   한 번만 알린다([`Self::first_late_failure`]).
/// - 맞추기가 숨긴 창: 실행기의 숨기기 · 보이기 맞추기가 스스로 숨긴 창 label — 맞추기는 이 창만 되보인다(트레이 등
///   다른 길이 숨긴 창을 끌기 끝이 드러내지 않게). 잎 락이고 메인 스레드 맞추기 · 창 소멸만 쓴다.
/// - 대기 창 명부([`parked`] · TRD §2-5): 잎 락 — 쥔 채 레이아웃 락 · emit · 창 호출을 하지 않는다. 레이아웃 락을 쥔
///   채 잡는 것은 된다(차례 = 레이아웃 → 명부 · 세계 뜨기 [`world::snapshot`] 가 둘을 함께 본다).
/// - 앱 종료 표지: 종료 요청을 받으면 선다([`Self::begin_exit`]) — 그 뒤 사라진 예비 창 · 안내 창은 다시 짓지 않는다
///   (TRD §2-5 「사라지면」). 내리지 않는다.
#[derive(Debug, Default)]
pub struct DockState {
    pub book: Mutex<Book>,
    snap_seq: AtomicU64,
    generation: AtomicU64,
    gen_seq: AtomicU64,
    errors: Mutex<ErrorTally>,
    late_warned: AtomicU64,
    own_hidden: Mutex<HashSet<String>>,
    parked: Mutex<Parked>,
    exiting: AtomicBool,
}

// 실패 셈과 그 몫인 세대를 한 락 아래 쥔다 — 세대를 보고 세는 것과 끝 줄이 비우며 닫는 것 사이에 다른 세션의 끝 ·
//   시작이 끼지 못하게. `generation` 0 = 닫힘.
#[derive(Debug, Default)]
struct ErrorTally {
    generation: u64,
    count: u64,
}

impl DockState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 새 번호를 단 모델 스냅숏 — 레이아웃 락을 쥔 채 부른다(재료 = [`snap::snapshot`]).
    pub fn snapshot(&self, mgr: &ViewManager) -> Snap {
        snap::snapshot(mgr, self.next_snap_number())
    }

    /// 새 번호를 단 시작 재료 — `lease_tab` 과 같은 레이아웃 락 구간에서 부른다(재료 · `None` 의 뜻 =
    /// [`snap::start_material`]).
    pub fn start_material(&self, mgr: &ViewManager) -> Option<StartMaterial> {
        snap::start_material(mgr, self.next_snap_number())
    }

    fn next_snap_number(&self) -> u64 {
        self.snap_seq.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// 새 세대(0 이 아니고 한 부팅 안에서 겹치지 않는다)를 세우고 돌려준다 — 남아 있던 옛 세대는 덮는다.
    pub fn begin_generation(&self) -> u64 {
        let generation = self.gen_seq.fetch_add(1, Ordering::SeqCst) + 1;
        self.generation.store(generation, Ordering::SeqCst);
        generation
    }

    /// 지금 세대가 `generation` 일 때만 0 으로 내린다 — 내렸으면 `true`. 0 은 내릴 세대가 아니다(`false`).
    pub fn end_generation(&self, generation: u64) -> bool {
        generation != 0
            && self
                .generation
                .compare_exchange(generation, 0, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
    }

    /// 세션 깃발이 섰나 — 계기가 스냅숏을 뜰지.
    pub fn session_live(&self) -> bool {
        self.generation.load(Ordering::SeqCst) != 0
    }

    /// 받아들인 시작 — 셈을 비우고 그 세대 몫으로 연다(남아 있던 셈은 버린다).
    pub fn open_errors(&self, generation: u64) {
        *sync::lock(&self.errors) = ErrorTally {
            generation,
            count: 0,
        };
    }

    /// 실행기 실패 하나를 지금 셈에 센다(세대를 보지 않는다) — 셈을 비운 뒤 첫 실패면 `true`(그것 하나만 warn 하고
    /// 나머지는 끝 줄 셈으로 — TRD §1-7). 셈이 닫혀 있으면 다음 시작이 버린다 — 세션 밖(시작 전의 이른 신호 등) 몫은
    /// 다음 세션에 섞일 수 있다(셈은 진단용이다).
    pub fn note_error(&self) -> bool {
        let mut tally = sync::lock(&self.errors);
        tally.count += 1;
        tally.count == 1
    }

    /// 세대 `generation` 몫의 실패 — 그 세대 셈이 열려 있으면 세고 `Some(첫 실패인가)`, 아니면 세지 않고 `None`
    /// (그 끝 줄이 이미 닫았거나 다음 세션이 열었다). 확인과 셈이 한 락 안이라 끝 줄이 비운 뒤에는 세지 못한다.
    pub fn note_error_of(&self, generation: u64) -> Option<bool> {
        let mut tally = sync::lock(&self.errors);
        (generation != 0 && tally.generation == generation).then(|| {
            tally.count += 1;
            tally.count == 1
        })
    }

    /// 끝 줄 — 그 세대 셈이 열려 있으면 센 수를 돌려주고 닫는다. 다른 세대가 열려 있으면 0 이고 그 셈은 그대로다(옛
    /// 세션의 늦은 끝 줄이 새 세션 셈을 가져가지 않는다).
    pub fn take_errors(&self, generation: u64) -> u64 {
        let mut tally = sync::lock(&self.errors);
        if generation == 0 || tally.generation != generation {
            return 0;
        }
        std::mem::take(&mut *tally).count
    }

    #[cfg(test)]
    pub fn booked_errors(&self) -> u64 {
        sync::lock(&self.errors).count
    }

    /// 끝 줄 뒤 실패의 알림 — 그 세대를 처음 알리면 `true`(셈은 늘지 않는다). 이미 알린 세대 이하면 `false` — 세대는
    /// 오르기만 하니 더 옛 세대의 늦은 실패는 알리지 않는다(debug 몫).
    pub fn first_late_failure(&self, generation: u64) -> bool {
        self.late_warned.fetch_max(generation, Ordering::SeqCst) < generation
    }

    pub fn mark_own_hidden(&self, label: &str) {
        sync::lock(&self.own_hidden).insert(label.to_owned());
    }

    pub fn owns_hidden(&self, label: &str) -> bool {
        sync::lock(&self.own_hidden).contains(label)
    }

    pub fn forget_own_hidden(&self, label: &str) {
        sync::lock(&self.own_hidden).remove(label);
    }

    /// 명부 가드 — 문장 하나 안에서 쓰고 놓는다(쥔 채 할 수 없는 것 = 구조체 문서).
    pub fn parked(&self) -> MutexGuard<'_, Parked> {
        sync::lock(&self.parked)
    }

    pub fn is_parked(&self, label: &str) -> bool {
        self.parked().contains(label)
    }

    /// 명부의 창이 보낸 재기를 적는다 — `None` = 명부에 없다(부르는 쪽이 모델에 적는다 · 뜻 = [`Parked::record`]).
    /// 명부 락은 돌아오기 전에 놓는다 — 부르는 쪽이 이어서 레이아웃 락을 잡는다.
    pub fn record_parked(&self, label: &str, measure: Measure) -> Option<Result<(), LayoutError>> {
        self.parked().record(label, measure)
    }

    /// 앱이 끝나는 중이다 — 사라진 예비 창 · 안내 창을 다시 짓지 않는다. 한 번 서면 내리지 않는다.
    pub fn begin_exit(&self) {
        self.exiting.store(true, Ordering::SeqCst);
    }

    pub fn exiting(&self) -> bool {
        self.exiting.load(Ordering::SeqCst)
    }

    /// 창 `label` 의 역할 — 명부의 창이면 `Parked`, 아니면 `in_model` 로 `Model` · `Unknown`. `in_model` 은 명부 락을
    /// 놓은 뒤 부르므로 레이아웃 락을 잡아도 된다.
    pub fn window_role(&self, label: &str, in_model: impl FnOnce() -> bool) -> WindowRole {
        if self.is_parked(label) {
            WindowRole::Parked
        } else if in_model() {
            WindowRole::Model
        } else {
            WindowRole::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_numbers_start_at_one_and_rise_across_both_doors() {
        let dock = DockState::new();
        let mut mgr = ViewManager::new();
        let view = mgr.windows[crate::layout::MAIN_WINDOW_LABEL].active;
        assert_eq!(dock.snapshot(&mgr).number, 1);
        mgr.lease_tab(view).unwrap();
        assert_eq!(dock.start_material(&mgr).unwrap().snap.number, 2);
        assert_eq!(dock.snapshot(&mgr).number, 3);
    }

    #[test]
    fn generations_are_non_zero_and_distinct() {
        let dock = DockState::new();
        assert!(!dock.session_live());
        let first = dock.begin_generation();
        let second = dock.begin_generation();
        assert_ne!(first, 0);
        assert!(second > first);
        assert!(dock.session_live());
    }

    #[test]
    fn an_older_generation_cannot_lower_a_newer_one() {
        let dock = DockState::new();
        let old = dock.begin_generation();
        let new = dock.begin_generation();
        assert!(!dock.end_generation(old), "옛 세션의 끝");
        assert!(dock.session_live());
        assert!(dock.end_generation(new));
        assert!(!dock.session_live());
        assert!(!dock.end_generation(new), "이미 내려갔다");
        assert!(!dock.end_generation(0));
        assert!(!dock.session_live());
    }

    #[test]
    fn only_the_first_failure_after_a_take_is_first() {
        let dock = DockState::new();
        dock.open_errors(1);
        assert!(dock.note_error());
        assert!(!dock.note_error());
        assert_eq!(dock.take_errors(1), 2);
        assert_eq!(dock.take_errors(1), 0);
        assert!(dock.note_error(), "비운 뒤의 첫 실패");
    }

    #[test]
    fn a_failure_of_a_generation_counts_only_while_its_tally_is_open() {
        let dock = DockState::new();
        assert_eq!(dock.note_error_of(1), None, "연 셈이 없다");
        dock.open_errors(1);
        assert_eq!(dock.note_error_of(1), Some(true));
        assert_eq!(dock.note_error_of(1), Some(false));
        assert_eq!(dock.note_error_of(2), None, "다른 세대 몫");
        assert_eq!(dock.take_errors(1), 2);
        assert_eq!(dock.note_error_of(1), None, "끝 줄이 닫았다");
        assert_eq!(dock.booked_errors(), 0);

        dock.open_errors(2);
        assert_eq!(dock.note_error_of(1), None);
        assert_eq!(
            dock.take_errors(1),
            0,
            "옛 세션의 끝 줄이 새 세션 셈을 가져가지 않는다"
        );
        assert!(dock.note_error());
        assert_eq!(dock.take_errors(2), 1);
    }

    #[test]
    fn a_late_failure_is_warned_once_per_generation() {
        let dock = DockState::new();
        assert!(dock.first_late_failure(1));
        assert!(dock.first_late_failure(2));
        assert!(!dock.first_late_failure(1), "A, B, A — A 는 한 번");
        assert!(!dock.first_late_failure(2));
        assert!(dock.first_late_failure(3));
    }

    #[test]
    fn the_exit_mark_stays_once_set() {
        let dock = DockState::new();
        assert!(!dock.exiting());
        dock.begin_exit();
        dock.begin_exit();
        assert!(dock.exiting());
    }

    #[test]
    fn a_parked_window_is_parked_without_asking_the_model() {
        let dock = DockState::new();
        dock.parked().reserve_spare("slot-popup-1".to_owned());
        let role = dock.window_role("slot-popup-1", || {
            panic!("명부의 창은 모델을 묻지 않는다");
        });
        assert_eq!(role, WindowRole::Parked);
    }

    #[test]
    fn a_window_outside_the_registry_is_model_or_unknown() {
        let dock = DockState::new();
        assert_eq!(dock.window_role("main", || true), WindowRole::Model);
        assert_eq!(
            dock.window_role("dock-overlay", || false),
            WindowRole::Unknown
        );
    }

    #[test]
    fn the_model_question_runs_with_the_registry_unlocked() {
        let dock = DockState::new();
        let role = dock.window_role("main", || {
            assert!(
                dock.parked.try_lock().is_ok(),
                "명부 락을 쥔 채 묻지 않는다"
            );
            true
        });
        assert_eq!(role, WindowRole::Model);
    }

    #[test]
    fn a_measure_for_a_window_outside_the_registry_is_not_taken() {
        let dock = DockState::new();
        assert_eq!(
            dock.record_parked("main", Measure::Canvas { w: 800, h: 600 }),
            None
        );
        dock.parked().reserve_spare("slot-popup-1".to_owned());
        assert_eq!(
            dock.record_parked("slot-popup-1", Measure::Canvas { w: 800, h: 600 }),
            Some(Ok(()))
        );
    }
}
