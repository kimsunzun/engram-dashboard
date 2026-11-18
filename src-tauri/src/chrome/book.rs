//! Window chrome ledger — per window: the OS title last applied and its version, the minimum window size reported by the web and the value applied to the OS, and the opening mark (TRD S26-window-chrome §4-4 · §5-1 · §7-7). Knows nothing of Tauri: the adapter (`crate::chrome`) holds it under a leaf lock and calls [`apply_title`] · [`report_min`] · [`apply_min`] from inside a single main-thread job.
//!
//! A window entry is created only after seeing that the OS window exists (the caller checks; if absent the ledger is untouched), so a late title job arriving after `Destroyed` forgot the entry does not revive it. Check and write are in the same main-thread job and `Destroyed` is handled on the main thread too, so forgetting cannot come between. A label is not reused within one run, so no tombstone is kept.
//!
//! The OS minimum size is applied only to normal-size windows — for maximized · minimized windows it is neither applied nor lifted. tao 0.35.3's `set_min_inner_size` re-applies the current size (`set_inner_size`) so the OS sees the new limit, which lowers the maximized flag and returns a maximized window to its original size with `SW_RESTORE` — a hidden window becomes visible and activated (tao `platform_impl/windows/window.rs` `set_min_inner_size` · `set_inner_size` → `window_state.rs` `apply_diff`, source reading). So reports arriving during maximize · minimize are recorded and applied when the window becomes normal size (`Resized` — [`ChromeBook::on_frame`]). The minimum is not lifted during maximize because the maximized rectangle is the work area (tao's frameless `WM_NCCALCSIZE`) and the reported minimum (a few hundred px) is smaller (narrower work areas are measured by QA S8).
//!
//! The opening mark is decided once after the window's minimum size has been received ([`ChromeBook::mark_opening`]) — right where the mark is written if already received (a window not rebuilt — main in runtime copy acceptance; its page does not report again if unchanged), else at the next accepted report. The web does not report before receiving the window number (prewarmed window waiting · loading · boot failure), so the mark written when a prewarmed window is taken in cannot be withdrawn first by a report made while waiting (`titleRow/TitleRow.tsx`). Applying a new place to a window not rebuilt lifts the previously applied OS minimum size and raises the mark ([`ChromeBook::lift_min`]) — unlifted, the OS grows a narrow place to the minimum keeping its left edge fixed and widen-and-pull-in has nothing left to do. The ledger's lift write and the OS lift are a single main-thread job ([`lift`]): split, a report arriving between them would have its applied value erased by the late OS lift, leaving the ledger saying "applied" with no OS minimum (U8b review 2). While the mark is raised a size change does not apply the OS minimum ([`ChromeBook::on_frame`]), so the size change the lift emits does not immediately re-apply it. The mark of a window with a received minimum always re-applies that value ([`ChromeBook::mark_opening`]) — fixing any ledger/OS drift on opening.

use std::collections::HashMap;
use std::sync::Mutex;

use engram_dashboard_base::sync;

// ADR-0303
#[derive(Debug, Default)]
pub(crate) struct ChromeBook {
    windows: HashMap<String, WindowChrome>,
}

#[derive(Debug, Default)]
struct WindowChrome {
    title: Option<TitleMark>,
    min: Option<MinMark>,
    // Opening mark — raised when the shell opens the window ([`ChromeBook::mark_opening`] · [`ChromeBook::lift_min`] before a new place) and withdrawn by the first decision that receives the minimum size (right there if already received when the mark is written, else the next report). Widen-and-pull-in happens only in the decision that withdraws this mark (once on opening — TRD §7-7 · PRD R20 · AC34). While up, a size change does not apply the OS minimum size.
    opening: bool,
    applied: Option<MinSize>,
}

impl WindowChrome {
    // If the recorded minimum differs from the value applied to the OS, that value — recorded as applied (if it could not be applied the caller calls [`ChromeBook::min_failed`]).
    fn next_min(&mut self) -> Option<MinSize> {
        let want = self.min.as_ref()?.size;
        if self.applied == Some(want) {
            return None;
        }
        self.applied = Some(want);
        Some(want)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameState {
    Normal,
    Other,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct MinPlan {
    pub(crate) widen: Option<MinSize>,
    pub(crate) set_min: Option<MinSize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MinReport {
    pub(crate) verdict: MinVerdict,
    pub(crate) plan: MinPlan,
}

/// The minimum window size the web measured for that window's title row (CSS px = logical px, normal-state based — TRD S26-window-chrome §7-7). Both values are `1..=MIN_SIZE_MAX` ([`MinSize::new`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MinSize {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

// Generously above the largest web measurement (a three-digit number marker · three buttons · area minimum cap 1000); beyond it is a measurement error — applied, the window would exceed the screen.
const MIN_SIZE_MAX: u32 = 4096;

impl MinSize {
    pub(crate) fn new(width: u32, height: u32) -> Result<Self, String> {
        let range = 1..=MIN_SIZE_MAX;
        if !range.contains(&width) || !range.contains(&height) {
            return Err(format!(
                "minimum window size {width}x{height} — both must be 1..={MIN_SIZE_MAX}"
            ));
        }
        Ok(Self { width, height })
    }
}

/// The order of one minimum-size report (TRD S26-window-chrome §7-7): `epoch` = wall-clock ms at which that window's web page loaded (web `Math.floor(performance.timeOrigin)` — a later reload is larger; two pages loaded in the same ms mix into one page, not handled); `seq` = a sequence rising by one on every send within that page (retries included), from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MinOrder {
    pub(crate) epoch: u64,
    pub(crate) seq: u32,
}

#[derive(Debug)]
struct MinMark {
    order: MinOrder,
    size: MinSize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MinVerdict {
    Changed,
    Same,
    /// An older `epoch` than the accepted one, or in the same `epoch` a `seq` no newer than the accepted one — an old attempt the web left on a deadline (including from the page before a reload) arrived late. Not recorded.
    Stale,
}

#[derive(Debug)]
struct TitleMark {
    version: u64,
    text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TitleVerdict {
    Set,
    Same,
    Stale,
}

impl ChromeBook {
    /// Decide whether to apply title `title` of version `version` to window `label`, and record it. A version is accepted only when greater than the last accepted for that window (the same version counts as old); once accepted the version is recorded even if `Same`, so a later old version cannot slip in behind the same text. `Set` → the caller applies it at once (on failure [`Self::title_failed`]). A missing entry is created — the caller checks that the OS window exists first (module head).
    // ADR-0301
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

    pub(crate) fn title_failed(&mut self, label: &str) {
        if let Some(mark) = self
            .windows
            .get_mut(label)
            .and_then(|window| window.title.as_mut())
        {
            mark.text = None;
        }
    }

    /// Record the minimum-size report of window `label`. An older `epoch` is discarded; within the same `epoch` it is accepted only when `seq` is greater than the last accepted; a larger `epoch` (page reloaded) counts the sequence afresh — so whatever the arrival order, the value sent last by the newest page remains. A missing entry is created (the caller checks the window — module head).
    pub(crate) fn record_min(&mut self, label: &str, order: MinOrder, min: MinSize) -> MinVerdict {
        let window = self.windows.entry(label.to_owned()).or_default();
        let verdict = match &window.min {
            Some(mark) if (order.epoch, order.seq) <= (mark.order.epoch, mark.order.seq) => {
                return MinVerdict::Stale;
            }
            Some(mark) if mark.size == min => MinVerdict::Same,
            _ => MinVerdict::Changed,
        };
        window.min = Some(MinMark { order, size: min });
        verdict
    }

    /// The shell has just opened window `label` (TRD §7-7 — a window port's fresh build · confirmation of an entrusted prewarmed window · boot-restore window build · copy acceptance's hidden pop-out build · placing copy acceptance's main slot) — decide widen-and-pull-in once. If that window's minimum size has not been received, raise the mark so the next accepted report decides ([`Self::on_min_report`]) and return an empty decision. If received, decide now with `state` (read after the place is applied) and withdraw the mark: normal size → widen and pull in and always re-apply that value to the OS (even if equal to the recorded applied value — this fixes ledger/OS drift, TRD §7-7 · U8b review 2); maximized · minimized → neither widen nor apply (once on opening — module head). A missing entry is created (module head).
    // ADR-0303
    pub(crate) fn mark_opening(&mut self, label: &str, state: FrameState) -> MinPlan {
        let window = self.windows.entry(label.to_owned()).or_default();
        let Some(min) = window.min.as_ref().map(|mark| mark.size) else {
            window.opening = true;
            return MinPlan::default();
        };
        window.opening = false;
        if state != FrameState::Normal {
            return MinPlan::default();
        }
        window.applied = Some(min);
        MinPlan {
            widen: Some(min),
            set_min: Some(min),
        }
    }

    /// The shell is about to apply a new normal place to window `label`, which it does not rebuild (main in runtime copy acceptance): raise the opening mark and, if normal size and the OS minimum was applied, forget the applied value. `true` = the caller lifts the OS minimum (module head — unlifted, the OS grows a narrow place and widen-and-pull-in has nothing to do). Maximized · minimized windows are not lifted (the lifting setter releases the maximize). After the place is applied, [`Self::mark_opening`] widens, pulls in and re-applies. Body = [`lift`] (one main-thread job). No entry is created — nothing to lift without one.
    // ADR-0303
    pub(crate) fn lift_min(&mut self, label: &str, state: FrameState) -> bool {
        let Some(window) = self.windows.get_mut(label) else {
            return false;
        };
        window.opening = true;
        state == FrameState::Normal && window.applied.take().is_some()
    }

    /// One minimum-size report — record it ([`Self::record_min`]) and decide what to apply. A `Stale` sequence applies nothing and does not withdraw the opening mark. An accepted report withdraws the mark, and widen-and-pull-in happens only if the mark was up and the window is now normal size (a window opened during maximize · minimize is not widened — once on opening). At normal size the OS minimum is applied if it differs from the applied value (else [`Self::on_frame`] applies it when the window becomes normal size — module head).
    // ADR-0303
    pub(crate) fn on_min_report(
        &mut self,
        label: &str,
        order: MinOrder,
        min: MinSize,
        state: FrameState,
    ) -> MinReport {
        let verdict = self.record_min(label, order, min);
        if verdict == MinVerdict::Stale {
            return MinReport {
                verdict,
                plan: MinPlan::default(),
            };
        }
        let window = self.windows.entry(label.to_owned()).or_default();
        let opening = std::mem::take(&mut window.opening);
        let normal = state == FrameState::Normal;
        MinReport {
            verdict,
            plan: MinPlan {
                widen: (opening && normal).then_some(min),
                set_min: if normal { window.next_min() } else { None },
            },
        }
    }

    /// The size state of window `label` changed (`Resized` — including return from maximize · minimize): if it became normal size and the recorded minimum has not been applied to the OS, have it applied (no widen-and-pull-in). With the opening mark up nothing is applied — the decision that withdraws the mark applies it after widening and pulling in (module head). No entry is created.
    pub(crate) fn on_frame(&mut self, label: &str, state: FrameState) -> MinPlan {
        let set_min = match (state, self.windows.get_mut(label)) {
            (FrameState::Normal, Some(window)) if !window.opening => window.next_min(),
            _ => None,
        };
        MinPlan {
            widen: None,
            set_min,
        }
    }

    pub(crate) fn min_failed(&mut self, label: &str) {
        if let Some(window) = self.windows.get_mut(label) {
            window.applied = None;
        }
    }

    pub(crate) fn forget(&mut self, label: &str) {
        self.windows.remove(label);
    }
}

/// The body of one minimum-size report — `window` = the current size state of that label's OS window (`None` = no window; the caller reads it inside the same main-thread job — module head). Out of range → `Err`, ledger untouched (the sequence is not recorded either). No window → nothing recorded, `Ok(None)`. An old sequence is not recorded and the decision is empty. The decision is [`ChromeBook::on_min_report`]; the caller applies it outside the ledger lock with [`apply_min`].
pub(crate) fn report_min(
    book: &Mutex<ChromeBook>,
    label: &str,
    width: u32,
    height: u32,
    order: MinOrder,
    window: Option<FrameState>,
) -> Result<Option<MinReport>, String> {
    let min = MinSize::new(width, height)?;
    let Some(state) = window else {
        tracing::debug!(
            label,
            width,
            height,
            "no OS window; minimum size report not recorded"
        );
        return Ok(None);
    };
    let report = sync::lock(book).on_min_report(label, order, min, state);
    if report.verdict == MinVerdict::Stale {
        tracing::debug!(
            label,
            width,
            height,
            epoch = order.epoch,
            seq = order.seq,
            "minimum size report with an old sequence number — discarded"
        );
    }
    Ok(Some(report))
}

pub(crate) trait MinWindow {
    /// If the normal size is narrower than `min_w` (logical px), widen it to that width and pull it into the work area (`state::placement::fit_min_width`). `Ok(true)` = moved · `Ok(false)` = nothing to widen.
    fn widen_to(&self, min_w: u32) -> Result<bool, String>;
    /// Apply the OS minimum size — `min` = the logical inner px the web reported; keeping the inner size from shrinking below it is the real implementation's job ([`os_min_size`]).
    fn apply_min_size(&self, min: MinSize) -> Result<(), String>;
    fn clear_min_size(&self) -> Result<(), String>;
}

/// The body of the lift job before a new place (`crate::chrome::lift_min` hands it over as a main-thread job); `state` · `window` = that label's OS window and size state (the caller checks the window exists and reads them inside the same job). Writes the lift in the ledger ([`ChromeBook::lift_min`]) and, if there was a value to lift, lifts the OS minimum after releasing the ledger lock. Writing and lifting share one main-thread job, so that window's report (a main-thread synchronous command) cannot come between (module head). A failed lift only warns — the ledger recorded it as lifted and the opening mark's decision re-applies the stored value ([`ChromeBook::mark_opening`]; an old value remaining in the OS could block widen-and-pull-in).
pub(crate) fn lift(
    book: &Mutex<ChromeBook>,
    label: &str,
    state: FrameState,
    window: &impl MinWindow,
) {
    if !sync::lock(book).lift_min(label, state) {
        return;
    }
    if let Err(e) = window.clear_min_size() {
        tracing::warn!(
            module = "chrome",
            label,
            "failed to lift the OS minimum window size before the new place: {e}"
        );
    }
}

/// The logical size to pass to the OS minimum-size setter (Tauri `set_min_size` → tao `set_min_inner_size`): the web's inner minimum `min` plus that window's invisible border (`frame_w` · `frame_h` = outer − inner width · height, physical px), converted to logical with `scale`. A scale that is not a finite positive number adds nothing.
/// Why (tao 0.35.3, source reading): despite the name "inner", the `WM_GETMINMAXINFO` handling (`platform_impl/windows/event_loop.rs`) computes the minimum tracking size with `util::adjust_size`, which for a frameless window removes `WS_CAPTION` · `WS_SIZEBOX` and calls `AdjustWindowRectEx`, adding no border. But for a frameless window with a shadow `WM_NCCALCSIZE` shrinks the inner area by the resize border on left · right · bottom and by one or two px on top (Windows 11; `util.rs` `calculate_insets_for_dpi`) — passed as is, the inner area is dragged narrower than reported by that border (≈16 wide · 9 high px at 100 %). The same version's `set_inner_size` adds that difference (`undecorated_with_shadows`). The border scales with the scale factor, so the logical value stays almost constant across monitor moves — only the top border's rounding may be off by up to 1 px.
// ADR-0303
pub(crate) fn os_min_size(min: MinSize, frame_w: u32, frame_h: u32, scale: f64) -> (f64, f64) {
    let (width, height) = (f64::from(min.width), f64::from(min.height));
    if !(scale.is_finite() && scale > 0.0) {
        return (width, height);
    }
    (
        width + f64::from(frame_w) / scale,
        height + f64::from(frame_h) / scale,
    )
}

/// Apply decision `plan` to window `label` — called outside the ledger lock (a leaf lock; no OS call while holding it). Widen-and-pull-in comes first: applying the OS minimum first would let the OS widen to the right keeping the left edge, push the window out of the work area and only then pull it back. Failures warn; an OS minimum that could not be applied is recorded in the ledger ([`ChromeBook::min_failed`]).
pub(crate) fn apply_min(
    book: &Mutex<ChromeBook>,
    label: &str,
    plan: MinPlan,
    window: &impl MinWindow,
) {
    if let Some(min) = plan.widen {
        match window.widen_to(min.width) {
            Ok(true) => tracing::info!(
                module = "chrome",
                label,
                min_width = min.width,
                "opened window was narrower than the minimum width; widened and pulled in"
            ),
            Ok(false) => {}
            Err(e) => tracing::warn!(
                module = "chrome",
                label,
                min_width = min.width,
                "failed to widen the opened window to the minimum width (only the OS minimum size applies): {e}"
            ),
        }
    }
    if let Some(min) = plan.set_min {
        match window.apply_min_size(min) {
            // QA material (S8 — the log tells apart "not applied during maximize" from "applied once normal").
            Ok(()) => tracing::debug!(
                module = "chrome",
                label,
                width = min.width,
                height = min.height,
                "OS minimum window size applied"
            ),
            Err(e) => {
                tracing::warn!(
                    module = "chrome",
                    label,
                    width = min.width,
                    height = min.height,
                    "failed to apply the OS minimum window size (the next report or size change applies it again): {e}"
                );
                sync::lock(book).min_failed(label);
            }
        }
    }
}

/// The body of one title job — runs on the main thread. `window` = that label's OS window (`None` if absent/closed): with no window it finishes without touching the ledger. The decision is inside the `book` lock and `set_title` is called after releasing it (leaf lock; no OS call while holding it — TRD S26-window-chrome §5-1). A failure warns and is recorded in the ledger ([`ChromeBook::title_failed`]).
pub(crate) fn apply_title<W>(
    book: &Mutex<ChromeBook>,
    label: &str,
    version: u64,
    title: &str,
    window: Option<W>,
    set_title: impl FnOnce(&W, &str) -> Result<(), String>,
) {
    let Some(window) = window else {
        tracing::debug!(label, version, "no OS window; title not applied");
        return;
    };
    let verdict = sync::lock(book).accept_title(label, version, title);
    match verdict {
        TitleVerdict::Set => {}
        TitleVerdict::Same => return,
        TitleVerdict::Stale => {
            tracing::debug!(label, version, "title of an old version — discarded");
            return;
        }
    }
    if let Err(e) = set_title(&window, title) {
        tracing::warn!(
            label,
            version,
            "failed to apply the OS window title (the next version applies it again): {e}"
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
            book.accept_title(POPUP, 3, "blank - Engram:2"),
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
            "the discarded old version was not recorded — the next new version is compared with the last applied `a`"
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
            "7 was recorded — a late 6 cannot slip in behind the same text"
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

    // ── apply_title (main-thread job) ──

    struct FakeWindow;

    // The title called · whether the ledger lock was released at that time.
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
                Err("window handle invalid (test)".to_owned())
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

    // A job that arrived late after `Destroyed` forgot it — there is no window, so the entry is not revived (TRD S26-window-chrome §4-4).
    #[test]
    fn a_late_job_after_forget_finds_no_window_and_leaves_no_entry() {
        let book = Mutex::new(ChromeBook::default());
        run(&book, 5, "a - Engram:2", Some(FakeWindow), false);
        book.lock().unwrap().forget(POPUP);

        assert!(run(&book, 9, "b - Engram:2", None, false).is_empty());

        assert!(
            book.lock().unwrap().windows.is_empty(),
            "the forgotten window's entry came back to life"
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

    // ── minimum-size report ──

    const EPOCH: u64 = 1_761_000_000_000;

    fn nth(seq: u32) -> MinOrder {
        MinOrder { epoch: EPOCH, seq }
    }

    fn size(width: u32, height: u32) -> MinSize {
        MinSize::new(width, height).unwrap()
    }

    fn min_of(book: &Mutex<ChromeBook>, label: &str) -> Option<MinSize> {
        book.lock()
            .unwrap()
            .windows
            .get(label)
            .and_then(|w| w.min.as_ref())
            .map(|m| m.size)
    }

    // One report of a normal-size window — `open` = that OS window exists. `Ok(true)` = recorded and different from the previous report.
    fn changed(
        book: &Mutex<ChromeBook>,
        label: &str,
        width: u32,
        height: u32,
        order: MinOrder,
        open: bool,
    ) -> Result<bool, String> {
        report_min(
            book,
            label,
            width,
            height,
            order,
            open.then_some(FrameState::Normal),
        )
        .map(|report| report.is_some_and(|r| r.verdict == MinVerdict::Changed))
    }

    #[test]
    fn a_min_report_is_recorded_per_window_and_says_whether_it_changed() {
        let book = Mutex::new(ChromeBook::default());

        assert_eq!(changed(&book, POPUP, 286, 64, nth(1), true), Ok(true));
        assert_eq!(changed(&book, POPUP, 286, 64, nth(2), true), Ok(false));
        assert_eq!(changed(&book, POPUP, 295, 64, nth(3), true), Ok(true));
        assert_eq!(changed(&book, "main", 120, 64, nth(1), true), Ok(true));

        assert_eq!(
            min_of(&book, POPUP),
            Some(MinSize {
                width: 295,
                height: 64
            })
        );
        assert_eq!(
            min_of(&book, "main"),
            Some(MinSize {
                width: 120,
                height: 64
            })
        );
    }

    #[test]
    fn an_out_of_range_min_report_is_refused_and_keeps_the_last_one_and_its_seq() {
        let book = Mutex::new(ChromeBook::default());
        changed(&book, POPUP, 286, 64, nth(1), true).unwrap();

        for (i, (w, h)) in [
            (0, 64),
            (286, 0),
            (MIN_SIZE_MAX + 1, 64),
            (286, MIN_SIZE_MAX + 1),
        ]
        .into_iter()
        .enumerate()
        {
            let seq = 10 + i as u32;
            assert!(
                changed(&book, POPUP, w, h, nth(seq), true).is_err(),
                "{w}x{h}"
            );
        }
        assert_eq!(min_of(&book, POPUP), Some(size(286, 64)));
        assert_eq!(
            changed(&book, POPUP, MIN_SIZE_MAX, MIN_SIZE_MAX, nth(2), true),
            Ok(true),
            "sequence numbers (10..13) of the rejected reports were not recorded — 2 is accepted"
        );
        assert_eq!(min_of(&book, POPUP), Some(size(MIN_SIZE_MAX, MIN_SIZE_MAX)));
    }

    // The case where an attempt the web left on a deadline reaches the shell after a newer attempt (TRD S26-window-chrome §7-7).
    #[test]
    fn an_older_or_equal_seq_of_the_same_epoch_is_dropped() {
        let book = Mutex::new(ChromeBook::default());
        changed(&book, POPUP, 295, 64, nth(2), true).unwrap();

        assert_eq!(changed(&book, POPUP, 286, 64, nth(1), true), Ok(false));
        assert_eq!(changed(&book, POPUP, 286, 64, nth(2), true), Ok(false));
        assert_eq!(min_of(&book, POPUP), Some(size(295, 64)));

        assert_eq!(changed(&book, POPUP, 286, 64, nth(3), true), Ok(true));
        assert_eq!(min_of(&book, POPUP), Some(size(286, 64)));
    }

    #[test]
    fn record_min_tells_changed_same_and_stale_apart() {
        let mut book = ChromeBook::default();

        assert_eq!(
            book.record_min(POPUP, nth(1), size(286, 64)),
            MinVerdict::Changed
        );
        assert_eq!(
            book.record_min(POPUP, nth(2), size(286, 64)),
            MinVerdict::Same
        );
        assert_eq!(
            book.record_min(POPUP, nth(2), size(295, 64)),
            MinVerdict::Stale
        );
        assert_eq!(
            book.record_min(POPUP, nth(3), size(286, 64)),
            MinVerdict::Same,
            "the discarded 295 was not recorded"
        );
    }

    // If the page of the same window is reloaded, the web's sequence starts again from 1.
    #[test]
    fn a_new_epoch_restarts_the_seq() {
        let book = Mutex::new(ChromeBook::default());
        changed(&book, POPUP, 295, 64, nth(5), true).unwrap();
        let reloaded = |seq| MinOrder {
            epoch: EPOCH + 1,
            seq,
        };

        assert_eq!(changed(&book, POPUP, 286, 64, reloaded(1), true), Ok(true));
        assert_eq!(changed(&book, POPUP, 120, 64, reloaded(1), true), Ok(false));
        assert_eq!(min_of(&book, POPUP), Some(size(286, 64)));
    }

    // The case where page A's attempt is left on a deadline, the report of the reloaded page B is accepted first, and A's arrives late.
    #[test]
    fn a_late_report_of_an_older_epoch_after_a_newer_one_is_dropped() {
        let book = Mutex::new(ChromeBook::default());
        let page_a = |seq| MinOrder { epoch: EPOCH, seq };
        let page_b = |seq| MinOrder {
            epoch: EPOCH + 5_000,
            seq,
        };
        changed(&book, POPUP, 295, 64, page_a(1), true).unwrap();

        assert_eq!(changed(&book, POPUP, 286, 64, page_b(1), true), Ok(true));
        assert_eq!(
            changed(&book, POPUP, 120, 64, page_a(9), true),
            Ok(false),
            "an old page's larger sequence number cannot overwrite the new page's value"
        );
        assert_eq!(min_of(&book, POPUP), Some(size(286, 64)));
        assert_eq!(
            book.lock()
                .unwrap()
                .record_min(POPUP, page_a(10), size(120, 64)),
            MinVerdict::Stale
        );
    }

    #[test]
    fn windows_keep_their_own_seqs() {
        let book = Mutex::new(ChromeBook::default());
        changed(&book, "main", 286, 64, nth(9), true).unwrap();

        assert_eq!(changed(&book, POPUP, 295, 64, nth(1), true), Ok(true));
    }

    // A report that arrived late after `Destroyed` forgot it — there is no window, so the entry is not revived (module head).
    #[test]
    fn a_min_report_without_the_window_leaves_no_entry() {
        let book = Mutex::new(ChromeBook::default());

        assert_eq!(changed(&book, POPUP, 286, 64, nth(1), false), Ok(false));

        assert!(book.lock().unwrap().windows.is_empty());
    }

    #[test]
    fn forgetting_a_window_drops_its_min_report_and_seq_too() {
        let book = Mutex::new(ChromeBook::default());
        changed(&book, POPUP, 286, 64, nth(5), true).unwrap();
        book.lock().unwrap().forget(POPUP);

        assert_eq!(min_of(&book, POPUP), None);
        assert_eq!(changed(&book, POPUP, 286, 64, nth(1), true), Ok(true));
    }

    // ── minimum-size apply · widen-and-pull-in decision (TRD S26-window-chrome §7-7) ──

    use FrameState::{Normal, Other};

    fn plan(book: &mut ChromeBook, seq: u32, min: MinSize, state: FrameState) -> MinPlan {
        book.on_min_report(POPUP, nth(seq), min, state).plan
    }

    fn set_only(min: MinSize) -> MinPlan {
        MinPlan {
            widen: None,
            set_min: Some(min),
        }
    }

    #[test]
    fn a_normal_window_gets_each_new_min_once() {
        let mut book = ChromeBook::default();

        assert_eq!(
            plan(&mut book, 1, size(286, 64), Normal),
            set_only(size(286, 64))
        );
        assert_eq!(
            plan(&mut book, 2, size(286, 64), Normal),
            MinPlan::default(),
            "the same value is not applied again"
        );
        assert_eq!(
            plan(&mut book, 3, size(295, 64), Normal),
            set_only(size(295, 64))
        );
    }

    // If the OS minimum size is applied to a maximized · minimized window, tao returns it to its original size (module head) — apply it when it becomes normal size.
    #[test]
    fn a_maximized_or_minimized_window_gets_its_min_when_it_is_normal_again() {
        let mut book = ChromeBook::default();

        assert_eq!(plan(&mut book, 1, size(286, 64), Other), MinPlan::default());
        assert_eq!(book.on_frame(POPUP, Other), MinPlan::default());
        assert_eq!(book.on_frame(POPUP, Normal), set_only(size(286, 64)));
        assert_eq!(
            book.on_frame(POPUP, Normal),
            MinPlan::default(),
            "a value applied once is not applied again on every size change"
        );
    }

    #[test]
    fn maximizing_and_restoring_leave_an_applied_min_alone() {
        let mut book = ChromeBook::default();
        plan(&mut book, 1, size(286, 64), Normal);

        assert_eq!(
            book.on_frame(POPUP, Other),
            MinPlan::default(),
            "not lifted by maximizing"
        );
        assert_eq!(book.on_frame(POPUP, Normal), MinPlan::default());
    }

    #[test]
    fn a_frame_change_without_a_report_does_nothing_and_makes_no_entry() {
        let mut book = ChromeBook::default();

        assert_eq!(book.on_frame(POPUP, Normal), MinPlan::default());

        assert!(book.windows.is_empty());
    }

    // Widen-and-pull-in is once on opening — while a person drags the window to a narrower monitor, only the OS minimum size applies (PRD R20 · AC34).
    #[test]
    fn only_the_first_report_after_an_opening_mark_widens() {
        let mut book = ChromeBook::default();
        assert_eq!(
            book.mark_opening(POPUP, Normal),
            MinPlan::default(),
            "a mark made before the minimum size arrived waits for the next report"
        );

        assert_eq!(
            plan(&mut book, 1, size(286, 64), Normal),
            MinPlan {
                widen: Some(size(286, 64)),
                set_min: Some(size(286, 64)),
            }
        );
        assert_eq!(plan(&mut book, 2, size(295, 64), Normal).widen, None);
    }

    #[test]
    fn a_report_without_an_opening_mark_never_widens() {
        let mut book = ChromeBook::default();

        assert_eq!(
            plan(&mut book, 1, size(286, 64), Normal),
            set_only(size(286, 64))
        );
    }

    #[test]
    fn an_opening_mark_spent_by_a_maximized_report_never_widens() {
        let mut book = ChromeBook::default();
        book.mark_opening(POPUP, Normal);

        assert_eq!(plan(&mut book, 1, size(286, 64), Other), MinPlan::default());
        assert_eq!(
            plan(&mut book, 2, size(295, 64), Normal),
            set_only(size(295, 64)),
            "a window that was maximized when opened is not widened or pulled in even after returning to its original size"
        );
    }

    // A window not rebuilt (main in runtime copy acceptance): its page does not report again if the value is unchanged, so the decision is made where the mark is written (module head).
    #[test]
    fn marking_a_window_whose_min_is_known_widens_at_once_and_spends_the_mark() {
        let mut book = ChromeBook::default();
        plan(&mut book, 1, size(286, 64), Normal);

        assert_eq!(
            book.mark_opening(POPUP, Normal),
            MinPlan {
                widen: Some(size(286, 64)),
                set_min: Some(size(286, 64)),
            },
            "even for the same value it tries widening and applies again — fixes a ledger/OS mismatch too"
        );
        assert_eq!(
            plan(&mut book, 2, size(295, 64), Normal),
            set_only(size(295, 64)),
            "the mark was withdrawn"
        );
    }

    #[test]
    fn marking_a_maximized_window_whose_min_is_known_never_widens() {
        let mut book = ChromeBook::default();
        plan(&mut book, 1, size(286, 64), Normal);

        assert_eq!(book.mark_opening(POPUP, Other), MinPlan::default());
        assert_eq!(plan(&mut book, 2, size(295, 64), Normal).widen, None);
    }

    // The OS minimum lifted before a new place is re-applied by the decision that withdraws the mark, after widen-and-pull-in; size changes in between (emitted by lifting · placing) are not applied (module head).
    #[test]
    fn a_lifted_min_is_held_until_the_mark_widens_and_applies_it_again() {
        let mut book = ChromeBook::default();
        plan(&mut book, 1, size(286, 64), Normal);

        assert!(book.lift_min(POPUP, Normal), "lifts the applied value");
        assert_eq!(book.on_frame(POPUP, Normal), MinPlan::default());
        assert_eq!(
            book.mark_opening(POPUP, Normal),
            MinPlan {
                widen: Some(size(286, 64)),
                set_min: Some(size(286, 64)),
            }
        );
        assert_eq!(book.on_frame(POPUP, Normal), MinPlan::default());
    }

    // A sequence where that window's report slipped in between the lift and the opening mark (U8b review 2): even if the report withdrew the mark and applied a new value, the opening mark re-applies it. Lift and report are one main-thread job each, so no OS lift can slip between them now (module head). But if the place setter was afterwards caught on the OS minimum and grew a narrow place keeping its left edge, the width is already at the minimum so `fit_min_width` does not pull it in — it may remain spilled past the right of the work area (rare · accepted · TRD §7-7).
    #[test]
    fn a_report_between_the_lift_and_the_mark_still_ends_with_the_min_applied() {
        let mut book = ChromeBook::default();
        plan(&mut book, 1, size(286, 64), Normal);
        assert!(book.lift_min(POPUP, Normal));

        assert_eq!(
            plan(&mut book, 2, size(295, 64), Normal),
            MinPlan {
                widen: Some(size(295, 64)),
                set_min: Some(size(295, 64)),
            },
            "a report withdraws the lift's mark"
        );
        assert_eq!(
            book.mark_opening(POPUP, Normal),
            MinPlan {
                widen: Some(size(295, 64)),
                set_min: Some(size(295, 64)),
            },
            "applies again even when the same value is recorded as applied"
        );
        assert_eq!(book.on_frame(POPUP, Normal), MinPlan::default());
    }

    #[test]
    fn a_maximized_window_keeps_its_os_min_through_a_lift() {
        let mut book = ChromeBook::default();
        plan(&mut book, 1, size(286, 64), Normal);

        assert!(
            !book.lift_min(POPUP, Other),
            "the lifting setter would un-maximize — not lifted"
        );
        assert_eq!(book.mark_opening(POPUP, Other), MinPlan::default());
        assert_eq!(
            book.on_frame(POPUP, Normal),
            MinPlan::default(),
            "the value left on the OS is not applied again"
        );
    }

    #[test]
    fn lifting_a_window_never_seen_makes_no_entry() {
        let mut book = ChromeBook::default();

        assert!(!book.lift_min(POPUP, Normal));

        assert!(book.windows.is_empty());
    }

    #[test]
    fn a_stale_report_leaves_a_lifted_mark_for_the_mark_to_spend() {
        let mut book = ChromeBook::default();
        plan(&mut book, 5, size(286, 64), Normal);
        book.lift_min(POPUP, Normal);

        assert_eq!(
            plan(&mut book, 4, size(120, 64), Normal),
            MinPlan::default()
        );
        assert_eq!(book.mark_opening(POPUP, Normal).widen, Some(size(286, 64)));
    }

    #[test]
    fn a_failed_min_is_applied_again_by_the_next_frame_change() {
        let mut book = ChromeBook::default();
        plan(&mut book, 1, size(286, 64), Normal);

        book.min_failed(POPUP);

        assert_eq!(book.on_frame(POPUP, Normal), set_only(size(286, 64)));
        book.min_failed("never-seen");
    }

    #[test]
    fn forgetting_a_window_drops_its_opening_mark_and_applied_min() {
        let mut book = ChromeBook::default();
        book.mark_opening(POPUP, Normal);
        book.forget(POPUP);

        assert_eq!(
            plan(&mut book, 1, size(286, 64), Normal),
            set_only(size(286, 64)),
            "the opening mark is forgotten too"
        );
        book.forget(POPUP);
        assert_eq!(
            plan(&mut book, 1, size(286, 64), Normal),
            set_only(size(286, 64)),
            "the applied value is forgotten too"
        );
    }

    // ── invisible border added to the OS minimum size (`os_min_size`) ──

    #[test]
    fn the_os_min_adds_the_invisible_frame_in_logical_px() {
        assert_eq!(
            os_min_size(size(286, 64), 16, 9, 1.0),
            (302.0, 73.0),
            "100 % — left/right 8 + 8 · top 1 + bottom 8"
        );
        let (w, h) = os_min_size(size(286, 64), 24, 13, 1.5);
        assert_eq!(w, 302.0, "150 % — physical 24 = logical 16");
        assert!((h - (64.0 + 13.0 / 1.5)).abs() < 1e-9, "{h}");
    }

    #[test]
    fn an_unusable_scale_adds_no_frame() {
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                os_min_size(size(286, 64), 16, 9, scale),
                (286.0, 64.0),
                "{scale}"
            );
        }
    }

    #[test]
    fn a_report_without_the_window_neither_plans_nor_spends_anything() {
        let book = Mutex::new(ChromeBook::default());

        assert_eq!(report_min(&book, POPUP, 286, 64, nth(1), None), Ok(None));
        assert_eq!(
            report_min(&book, POPUP, 0, 64, nth(2), None),
            Err(format!(
                "minimum window size 0x64 — both must be 1..={MIN_SIZE_MAX}"
            )),
            "the range check comes before the window check"
        );
    }

    // ── apply_min (apply sequence) ──

    // The job called · whether the ledger lock was released at that time.
    struct FakeMinWindow<'a> {
        book: &'a Mutex<ChromeBook>,
        calls: RefCell<Vec<(String, bool)>>,
        widen: Result<bool, String>,
        set: Result<(), String>,
    }

    impl<'a> FakeMinWindow<'a> {
        fn new(book: &'a Mutex<ChromeBook>) -> Self {
            Self {
                book,
                calls: RefCell::new(Vec::new()),
                widen: Ok(true),
                set: Ok(()),
            }
        }

        fn log(&self, call: String) {
            let free = self.book.try_lock().is_ok();
            self.calls.borrow_mut().push((call, free));
        }
    }

    impl MinWindow for FakeMinWindow<'_> {
        fn widen_to(&self, min_w: u32) -> Result<bool, String> {
            self.log(format!("widen {min_w}"));
            self.widen.clone()
        }

        fn apply_min_size(&self, min: MinSize) -> Result<(), String> {
            self.log(format!("min {}x{}", min.width, min.height));
            self.set.clone()
        }

        fn clear_min_size(&self) -> Result<(), String> {
            self.log("clear".to_owned());
            self.set.clone()
        }
    }

    // ── lift (lift job before a new place) ──

    #[test]
    fn a_lift_clears_the_os_min_outside_the_book_lock_only_when_one_was_applied() {
        let book = Mutex::new(ChromeBook::default());
        book.lock()
            .unwrap()
            .on_min_report(POPUP, nth(1), size(286, 64), Normal);
        let window = FakeMinWindow::new(&book);

        lift(&book, POPUP, Normal, &window);
        lift(&book, POPUP, Normal, &window);

        assert_eq!(
            window.calls.into_inner(),
            [("clear".to_owned(), true)],
            "the second lift has no value to lift"
        );
        assert_eq!(
            book.lock().unwrap().mark_opening(POPUP, Normal),
            MinPlan {
                widen: Some(size(286, 64)),
                set_min: Some(size(286, 64)),
            },
            "the opening mark withdraws the mark the lift raised"
        );
    }

    #[test]
    fn a_lift_of_a_maximized_or_unseen_window_clears_nothing() {
        let book = Mutex::new(ChromeBook::default());
        let window = FakeMinWindow::new(&book);

        lift(&book, POPUP, Normal, &window);
        book.lock()
            .unwrap()
            .on_min_report(POPUP, nth(1), size(286, 64), Normal);
        lift(&book, POPUP, Other, &window);

        assert!(window.calls.into_inner().is_empty());
    }

    // Even if lifting failed, the ledger stays as lifted and the opening mark re-applies the stored value.
    #[test]
    fn a_failed_lift_is_repaired_by_the_mark() {
        let book = Mutex::new(ChromeBook::default());
        book.lock()
            .unwrap()
            .on_min_report(POPUP, nth(1), size(286, 64), Normal);
        let mut window = FakeMinWindow::new(&book);
        window.set = Err("window handle invalid (test)".to_owned());

        lift(&book, POPUP, Normal, &window);

        assert_eq!(window.calls.into_inner().len(), 1);
        assert_eq!(
            book.lock().unwrap().mark_opening(POPUP, Normal).set_min,
            Some(size(286, 64))
        );
    }

    #[test]
    fn widening_goes_before_the_os_min_and_both_run_outside_the_book_lock() {
        let book = Mutex::new(ChromeBook::default());
        let window = FakeMinWindow::new(&book);

        apply_min(
            &book,
            POPUP,
            MinPlan {
                widen: Some(size(286, 64)),
                set_min: Some(size(286, 64)),
            },
            &window,
        );

        assert_eq!(
            window.calls.into_inner(),
            [
                ("widen 286".to_owned(), true),
                ("min 286x64".to_owned(), true)
            ]
        );
    }

    #[test]
    fn a_failed_widen_still_applies_the_os_min() {
        let book = Mutex::new(ChromeBook::default());
        let mut window = FakeMinWindow::new(&book);
        window.widen = Err("could not read the monitor (test)".to_owned());

        apply_min(
            &book,
            POPUP,
            MinPlan {
                widen: Some(size(286, 64)),
                set_min: Some(size(286, 64)),
            },
            &window,
        );

        assert_eq!(window.calls.into_inner().len(), 2);
    }

    #[test]
    fn a_failed_os_min_is_forgotten_so_the_next_frame_change_applies_it_again() {
        let book = Mutex::new(ChromeBook::default());
        let planned = {
            let mut book = book.lock().unwrap();
            book.on_min_report(POPUP, nth(1), size(286, 64), Normal)
                .plan
        };
        let mut window = FakeMinWindow::new(&book);
        window.set = Err("window handle invalid (test)".to_owned());

        apply_min(&book, POPUP, planned, &window);

        assert_eq!(
            book.lock().unwrap().on_frame(POPUP, Normal),
            set_only(size(286, 64))
        );
    }

    #[test]
    fn an_empty_plan_touches_nothing() {
        let book = Mutex::new(ChromeBook::default());
        let window = FakeMinWindow::new(&book);

        apply_min(&book, POPUP, MinPlan::default(), &window);

        assert!(window.calls.into_inner().is_empty());
    }
}
