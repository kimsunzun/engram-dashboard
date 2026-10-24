//! 창 크롬 장부 — 창마다 마지막으로 입힌 OS 제목과 그 판(TRD S26-window-chrome §4-4 · §5-1). Tauri 를 모른다 — 어댑터
//! (`crate::chrome`)가 잎 락으로 쥐고 메인 스레드 일 하나 안에서 [`apply_title`] 로 부른다.
//!
//! ★창 항목은 그 OS 창이 있는 것을 본 뒤에만 만든다★ — 창 확인은 부르는 쪽 몫이고, 없으면 장부를 만지지 않는다.
//! `Destroyed` 가 항목을 잊은 뒤 늦게 온 제목 일이 항목을 되살리지 않게 하려는 것이다. 확인과 적기가 같은 메인 스레드
//! 일 안이고 `Destroyed` 처리기도 메인 스레드라 그 사이에 잊기가 끼어들 수 없다. label 은 한 실행 안에서 다시 쓰이지
//! 않아 묘비를 두지 않는다.

use std::collections::HashMap;
use std::sync::Mutex;

use engram_dashboard_base::sync;

#[derive(Debug, Default)]
pub(crate) struct ChromeBook {
    windows: HashMap<String, WindowChrome>,
}

#[derive(Debug, Default)]
struct WindowChrome {
    title: Option<TitleMark>,
}

#[derive(Debug)]
struct TitleMark {
    version: u64,
    // `None` = 그 판을 입히다 실패했다 — 다음 새 판은 글자가 같아도 입힌다.
    text: Option<String>,
}

/// [`ChromeBook::accept_title`] 의 판정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TitleVerdict {
    /// 새 판이고 글자가 바뀌었다 — 입힌다.
    Set,
    /// 새 판이지만 마지막으로 입힌 글자와 같다 — OS 를 부르지 않는다.
    Same,
    /// 그 창에 이미 받아들인 판보다 새 판이 아니다 — 차례가 뒤바뀌어 늦게 온 옛 알림이다.
    Stale,
}

impl ChromeBook {
    /// 창 `label` 에 판 `version` 의 제목 `title` 을 입힐지 정하고 적는다. 판은 그 창에 받아들인 마지막 판보다 클 때만
    /// 받아들이고(같은 판은 옛 판이다), 받아들이면 `Same` 이어도 판을 적는다 — 뒤에 온 옛 판이 같은 글자 뒤에 숨어
    /// 들어오지 못하게. `Set` 이면 부르는 쪽이 곧 입힌다(못 입혔으면 [`Self::title_failed`]). 항목이 없으면 만든다 —
    /// 그 OS 창이 지금 있는지는 부르는 쪽이 먼저 본다(모듈 머리).
    pub(crate) fn accept_title(&mut self, label: &str, version: u64, title: &str) -> TitleVerdict {
        let window = self.windows.entry(label.to_owned()).or_default();
        let Some(mark) = &mut window.title else {
            window.title = Some(TitleMark {
                version,
                text: Some(title.to_owned()),
            });
            return TitleVerdict::Set;
        };
        if version <= mark.version {
            return TitleVerdict::Stale;
        }
        mark.version = version;
        if mark.text.as_deref() == Some(title) {
            return TitleVerdict::Same;
        }
        mark.text = Some(title.to_owned());
        TitleVerdict::Set
    }

    /// `Set` 으로 받아들인 `label` 의 제목을 못 입혔다 — 판은 두고 글자만 지워, 다음 새 판은 글자가 같아도 입힌다.
    pub(crate) fn title_failed(&mut self, label: &str) {
        if let Some(mark) = self
            .windows
            .get_mut(label)
            .and_then(|window| window.title.as_mut())
        {
            mark.text = None;
        }
    }

    /// 창 `label` 의 항목을 잊는다 — 그 OS 창의 `Destroyed`. 없으면 무동작.
    pub(crate) fn forget(&mut self, label: &str) {
        self.windows.remove(label);
    }
}

/// 제목 일 하나의 몸통 — 메인 스레드에서 돈다. `window` = 그 label 의 OS 창(없으면 `None` — 닫혔다). 창이 없으면 장부를
/// 만지지 않고 끝낸다. 판정은 `book` 락 안이고, `set_title` 은 그 락을 놓은 뒤 부른다(잎 락 — 쥔 채 OS 를 부르지
/// 않는다 · TRD S26-window-chrome §5-1). 실패는 warn 을 남기고 장부에 적는다([`ChromeBook::title_failed`]).
pub(crate) fn apply_title<W>(
    book: &Mutex<ChromeBook>,
    label: &str,
    version: u64,
    title: &str,
    window: Option<W>,
    set_title: impl FnOnce(&W, &str) -> Result<(), String>,
) {
    let Some(window) = window else {
        tracing::debug!(label, version, "OS 창이 없어 제목을 입히지 않는다");
        return;
    };
    let verdict = sync::lock(book).accept_title(label, version, title);
    match verdict {
        TitleVerdict::Set => {}
        TitleVerdict::Same => return,
        TitleVerdict::Stale => {
            tracing::debug!(label, version, "옛 판의 제목 — 버린다");
            return;
        }
    }
    if let Err(e) = set_title(&window, title) {
        tracing::warn!(
            label,
            version,
            "OS 창 제목을 입히지 못했다(다음 판이 다시 입힌다): {e}"
        );
        sync::lock(book).title_failed(label);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const POPUP: &str = "slot-popup-1";

    // ── accept_title ──

    #[test]
    fn the_first_title_of_a_window_is_set() {
        let mut book = ChromeBook::default();
        assert_eq!(
            book.accept_title(POPUP, 3, "빈 칸 - Engram:2"),
            TitleVerdict::Set
        );
    }

    #[test]
    fn an_older_or_equal_version_is_stale_whatever_its_text() {
        let mut book = ChromeBook::default();
        book.accept_title(POPUP, 5, "a - Engram:2");

        assert_eq!(
            book.accept_title(POPUP, 4, "b - Engram:2"),
            TitleVerdict::Stale
        );
        assert_eq!(
            book.accept_title(POPUP, 5, "b - Engram:2"),
            TitleVerdict::Stale
        );
        assert_eq!(
            book.accept_title(POPUP, 6, "b - Engram:2"),
            TitleVerdict::Set,
            "버린 옛 판은 적히지 않았다 — 다음 새 판은 마지막으로 입힌 a 와 견준다"
        );
    }

    #[test]
    fn a_newer_version_with_the_same_text_skips_the_os_but_moves_the_version() {
        let mut book = ChromeBook::default();
        book.accept_title(POPUP, 5, "a - Engram:2");

        assert_eq!(
            book.accept_title(POPUP, 7, "a - Engram:2"),
            TitleVerdict::Same
        );
        assert_eq!(
            book.accept_title(POPUP, 6, "b - Engram:2"),
            TitleVerdict::Stale,
            "7 을 적었다 — 늦게 온 6 이 같은 글자 뒤로 끼어들지 못한다"
        );
    }

    #[test]
    fn windows_keep_their_own_versions() {
        let mut book = ChromeBook::default();
        book.accept_title("main", 9, "a - Engram:1");

        assert_eq!(
            book.accept_title(POPUP, 4, "b - Engram:2"),
            TitleVerdict::Set
        );
    }

    #[test]
    fn a_failed_title_is_set_again_by_the_next_version_even_with_the_same_text() {
        let mut book = ChromeBook::default();
        book.accept_title(POPUP, 5, "a - Engram:2");
        book.title_failed(POPUP);

        assert_eq!(
            book.accept_title(POPUP, 5, "a - Engram:2"),
            TitleVerdict::Stale
        );
        assert_eq!(
            book.accept_title(POPUP, 6, "a - Engram:2"),
            TitleVerdict::Set
        );
    }

    #[test]
    fn a_forgotten_window_starts_over() {
        let mut book = ChromeBook::default();
        book.accept_title(POPUP, 5, "a - Engram:2");
        book.forget(POPUP);

        assert_eq!(
            book.accept_title(POPUP, 1, "a - Engram:2"),
            TitleVerdict::Set
        );
        book.forget("never-seen");
    }

    // ── apply_title(메인 스레드 일) ──

    struct FakeWindow;

    // 부른 제목 · 그때 장부 락이 놓여 있었나.
    fn run(
        book: &Mutex<ChromeBook>,
        version: u64,
        title: &str,
        window: Option<FakeWindow>,
        fail: bool,
    ) -> Vec<(String, bool)> {
        let calls = RefCell::new(Vec::new());
        apply_title(book, POPUP, version, title, window, |_, text| {
            calls
                .borrow_mut()
                .push((text.to_owned(), book.try_lock().is_ok()));
            if fail {
                Err("창 핸들 무효(시험)".to_owned())
            } else {
                Ok(())
            }
        });
        calls.into_inner()
    }

    #[test]
    fn the_os_title_is_set_after_the_book_lock_is_released() {
        let book = Mutex::new(ChromeBook::default());

        let calls = run(&book, 1, "a - Engram:2", Some(FakeWindow), false);

        assert_eq!(calls, [("a - Engram:2".to_owned(), true)]);
    }

    #[test]
    fn only_new_versions_with_new_text_reach_the_os() {
        let book = Mutex::new(ChromeBook::default());
        run(&book, 5, "a - Engram:2", Some(FakeWindow), false);

        assert!(run(&book, 4, "b - Engram:2", Some(FakeWindow), false).is_empty());
        assert!(run(&book, 6, "a - Engram:2", Some(FakeWindow), false).is_empty());
        assert_eq!(
            run(&book, 7, "b - Engram:2", Some(FakeWindow), false).len(),
            1
        );
    }

    // `Destroyed` 가 잊은 뒤 늦게 온 일 — 창이 없으니 항목을 되살리지 않는다(TRD S26-window-chrome §4-4).
    #[test]
    fn a_late_job_after_forget_finds_no_window_and_leaves_no_entry() {
        let book = Mutex::new(ChromeBook::default());
        run(&book, 5, "a - Engram:2", Some(FakeWindow), false);
        book.lock().unwrap().forget(POPUP);

        assert!(run(&book, 9, "b - Engram:2", None, false).is_empty());

        assert!(
            book.lock().unwrap().windows.is_empty(),
            "잊은 창의 항목이 되살아났다"
        );
    }

    #[test]
    fn a_failed_set_is_retried_by_the_next_version_with_the_same_text() {
        let book = Mutex::new(ChromeBook::default());

        assert_eq!(
            run(&book, 5, "a - Engram:2", Some(FakeWindow), true).len(),
            1
        );

        assert!(run(&book, 5, "a - Engram:2", Some(FakeWindow), false).is_empty());
        assert_eq!(
            run(&book, 6, "a - Engram:2", Some(FakeWindow), false),
            [("a - Engram:2".to_owned(), true)]
        );
    }
}
