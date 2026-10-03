//! 트리 창(`agent-tree`)의 화면 속성 칸 — 테마 · 자리 · 최대화(TRD S21-storage §6-3). 트리 창은 레이아웃 모델
//! (`ViewManager`) 밖이라 따로 들고 자기 변경 번호를 갖는다.
//!
//! - ★보임 여부는 싣지 않는다(사용자 결정 F9)★ — 트리 창은 부팅마다 숨은 채 뜬다. 최대화 여부는 싣는다 — 자리의
//!   일부다(보통 자리로 놓은 뒤 최대화 — F8).
//! - ★락은 잎이다★ — 쥔 채 다른 락을 잡지 않고 OS 를 부르지 않는다. 창 게터는 부르는 쪽이 락 밖에서 읽어 값으로
//!   넘긴다.
//! - 번호는 바뀐 쓰기마다 +1 이고 기록기는 같은지만 본다(§6-4) — ★되돌리거나 초기화하지 않는다★. 칸을 통째로
//!   갈아끼울 때(부팅 채우기 · 런타임 수락)도 [`TreeAttrs::set`] 으로 올린다.

use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::layout::{WindowAttrs, WindowPlacement};
use crate::ui_settings::UiTheme;

#[derive(Default)]
pub struct TreeAttrs {
    cell: Mutex<Cell>,
}

#[derive(Default)]
struct Cell {
    attrs: WindowAttrs,
    rev: u64,
}

impl TreeAttrs {
    pub fn attrs(&self) -> WindowAttrs {
        self.lock().attrs
    }

    pub fn rev(&self) -> u64 {
        self.lock().rev
    }

    pub fn set(&self, attrs: WindowAttrs) {
        self.update(|current| {
            let changed = *current != attrs;
            *current = attrs;
            changed
        });
    }

    /// `None` = 창별 테마를 지운다(전역 테마를 따른다).
    pub fn set_theme(&self, theme: Option<UiTheme>) {
        self.update(|attrs| attrs.set_theme(theme));
    }

    /// 규칙은 [`WindowAttrs::observe`].
    pub fn observe_placement(&self, seen: WindowPlacement) {
        self.update(|attrs| attrs.observe(seen));
    }

    fn update(&self, change: impl FnOnce(&mut WindowAttrs) -> bool) {
        let mut cell = self.lock();
        if change(&mut cell.attrs) {
            cell.rev += 1;
        }
    }

    // 독 든 락은 되살린다 — 칸이 값 셋뿐이라 반쯤 바뀐 상태가 해롭지 않고, 기록기 포트는 패닉하면 안 된다
    // (릴리스는 `panic = "abort"` — `state/saver.rs` 의 `SnapshotSource`).
    fn lock(&self) -> MutexGuard<'_, Cell> {
        self.cell.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::TreeAttrs;
    use crate::layout::{WindowAttrs, WindowBounds, WindowPlacement};
    use crate::ui_settings::UiTheme;

    fn bounds(x: f64) -> WindowBounds {
        WindowBounds::new(x, 20.0, 280.0, 600.0).unwrap()
    }

    fn normal(x: f64) -> WindowPlacement {
        WindowPlacement {
            minimized: false,
            maximized: false,
            bounds: Some(bounds(x)),
        }
    }

    #[test]
    fn starts_empty_at_revision_zero() {
        let tree = TreeAttrs::default();
        assert_eq!(tree.attrs(), WindowAttrs::default());
        assert_eq!(tree.rev(), 0);
    }

    #[test]
    fn each_change_bumps_the_revision_once_and_a_repeat_does_not() {
        let tree = TreeAttrs::default();
        tree.set_theme(Some(UiTheme::Light));
        assert_eq!(tree.rev(), 1);
        tree.set_theme(Some(UiTheme::Light));
        assert_eq!(tree.rev(), 1);

        tree.observe_placement(normal(10.0));
        assert_eq!(tree.rev(), 2);
        tree.observe_placement(normal(10.0));
        assert_eq!(tree.rev(), 2);
        assert_eq!(
            tree.attrs(),
            WindowAttrs {
                theme: Some(UiTheme::Light),
                bounds: Some(bounds(10.0)),
                maximized: false,
            }
        );

        tree.set_theme(None);
        assert_eq!((tree.attrs().theme, tree.rev()), (None, 3));
    }

    #[test]
    fn set_replaces_everything_and_bumps_only_when_different() {
        let tree = TreeAttrs::default();
        let restored = WindowAttrs {
            theme: Some(UiTheme::EInk),
            bounds: Some(bounds(-1920.0)),
            maximized: true,
        };
        tree.set(restored);
        assert_eq!((tree.attrs(), tree.rev()), (restored, 1));
        tree.set(restored);
        assert_eq!(tree.rev(), 1);
        tree.set(WindowAttrs::default());
        assert_eq!((tree.attrs(), tree.rev()), (WindowAttrs::default(), 2));
    }

    #[test]
    fn minimized_and_maximized_follow_the_window_rule() {
        let tree = TreeAttrs::default();
        tree.observe_placement(normal(10.0));
        tree.observe_placement(WindowPlacement {
            minimized: false,
            maximized: true,
            bounds: Some(bounds(0.0)),
        });
        assert_eq!(
            (tree.attrs().bounds, tree.attrs().maximized),
            (Some(bounds(10.0)), true)
        );
        let rev = tree.rev();
        tree.observe_placement(WindowPlacement {
            minimized: true,
            maximized: false,
            bounds: Some(bounds(-32000.0)),
        });
        assert_eq!(tree.rev(), rev, "최소화는 아무것도 바꾸지 않는다");
    }

    #[test]
    fn a_poisoned_lock_is_recovered() {
        let tree = TreeAttrs::default();
        tree.set_theme(Some(UiTheme::Dark));
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = tree.cell.lock().unwrap();
            panic!("독");
        }));
        assert!(tree.cell.is_poisoned());
        assert_eq!(tree.attrs().theme, Some(UiTheme::Dark));
        tree.set_theme(None);
        assert_eq!(tree.rev(), 2);
    }
}
