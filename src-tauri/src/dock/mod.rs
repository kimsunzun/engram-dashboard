//! 슬롯 도킹의 끌기 쪽(TRD S21-slot-docking §1-3 · §2) — 레이아웃 권위(`crate::layout` · ADR-0035)를 부르는
//! 손님이다. 레이아웃은 여기서 바꾸지 않는다 — 바꾸는 길은 적용 서비스(`layout::apply`) 하나다.

pub mod book;
pub mod hit;
pub mod regions;
pub mod snap;
pub mod types;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use book::{Book, Snap};
use snap::StartMaterial;

use crate::layout::ViewManager;

/// 셸에 하나인 끌기 상태 — 장부(잎 락 · 규율 = `book` 머리)와 레이아웃 락 안에서 쓰는 두 셈.
///
/// - 스냅숏 번호는 1 부터 오르고 0 을 내지 않는다(장부는 마지막으로 먹은 번호 이하를 버린다 — 처음 값이 0). 번호를
///   다는 길은 [`Self::snapshot`] · [`Self::start_material`] 둘뿐이고 둘 다 `&ViewManager` 를 받는다 — 레이아웃 락
///   가드 밖에서는 그 참조가 없으니 번호와 재료가 같은 락 구간에서 뜬다(TRD §1-3 「스냅숏 번호」).
/// - 세션 세대: 0 = 세션 없음. 시작의 레이아웃 락 안에서 [`Self::begin_generation`] 으로 세우고, 끝에서
///   [`Self::end_generation`] 으로 자기 세대일 때만 내린다(TRD §1-3 「시작」 2 — 무조건 내리면 그 사이 선 다음
///   세션의 깃발까지 내려 그 세션의 계기가 끊긴다).
/// - 실패 셈: 실행기(`commands::dock`)의 emit · 창 호출 · 게터 실패 수 — 끝 줄의 `errors`(TRD §1-7).
#[derive(Debug, Default)]
pub struct DockState {
    pub book: Mutex<Book>,
    snap_seq: AtomicU64,
    generation: AtomicU64,
    gen_seq: AtomicU64,
    errors: AtomicU64,
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

    /// 실행기 실패 하나를 센다 — 셈을 비운 뒤 첫 실패면 `true`(그것 하나만 warn 하고 나머지는 끝 줄 셈으로 —
    /// TRD §1-7).
    pub fn note_error(&self) -> bool {
        self.errors.fetch_add(1, Ordering::SeqCst) == 0
    }

    /// 센 실패 수를 돌려주고 비운다 — 끝 줄이 읽고, 받아들인 시작이 새 세션 몫으로 비운다. 세션 밖(시작 전의 이른
    /// 신호 등)에서 난 실패는 다음 세션 몫으로 섞일 수 있다(셈은 진단용이다).
    pub fn take_errors(&self) -> u64 {
        self.errors.swap(0, Ordering::SeqCst)
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
        assert!(dock.note_error());
        assert!(!dock.note_error());
        assert_eq!(dock.take_errors(), 2);
        assert_eq!(dock.take_errors(), 0);
        assert!(dock.note_error(), "비운 뒤의 첫 실패");
    }
}
