//! Window chrome ledger — per window, the OS title last applied and its version · the minimum window size reported by the web and the value applied to the OS · the opening mark (TRD
//! S26-window-chrome §4-4 · §5-1 · §7-7). It knows nothing of Tauri — the adapter (`crate::chrome`) holds it under a leaf lock and calls
//! [`apply_title`] · [`report_min`] · [`apply_min`] from inside a single main-thread job.
//!
//! ★A window entry is created only after seeing that the OS window exists★ — checking the window is the caller's job, and if it is absent the ledger is not touched.
//! This is so that a late title job arriving after `Destroyed` has forgotten the entry does not revive it. The check and the write are in the same main-thread
//! job and the `Destroyed` handler is also on the main thread, so forgetting cannot come in between. A label is not reused
//! within one run, so no tombstone is kept.
//!
//! ★The OS minimum size is applied only to normal-size windows — for maximized · minimized windows it is neither applied nor lifted★. The minimum-size
//! setter of tao 0.35.3 (`set_min_inner_size`) re-applies the current size (`set_inner_size`) so that the OS sees the new limit again, and that setter
//! lowers the maximized flag and returns a maximized window to its original size with `SW_RESTORE` — for a hidden window it becomes visible and activated (tao
//! `platform_impl/windows/window.rs` `set_min_inner_size` · `set_inner_size` → `window_state.rs` `apply_diff` · source
//! reading). So reports that arrive during maximize · minimize are recorded, and applied when the window becomes normal size (`Resized` — [`ChromeBook::on_frame`]).
//! That is why the minimum size is not lifted during maximize — the maximized rectangle is the work area (tao's frameless window
//! `WM_NCCALCSIZE`) and the minimum size the web reports (a few hundred px) is smaller than that, so it never gets in the way (screens whose work area is narrower than that
//! are measured by QA S8).
//!
//! ★The opening mark is decided once after the window's minimum size has been received★([`ChromeBook::mark_opening`]) — if already received, right
//! where the mark is written (a window that is not rebuilt — main in runtime copy acceptance · that page does not report again if the value is unchanged), if not yet, at
//! the next accepted report. The web does not report before it receives the window number (the prewarmed window's waiting · loading · boot failure) — so the mark written when
//! a prewarmed window is taken in cannot be withdrawn first by a report made while waiting (`titleRow/TitleRow.tsx`). When applying a new place to a window that is not rebuilt,
//! the OS minimum size applied before that is lifted and the mark is raised ([`ChromeBook::lift_min`]) — if not lifted, the OS grows a narrow place to the minimum size
//! keeping its left edge fixed, and widen-and-pull-in has nothing left to do. ★Writing the lift in the ledger and the OS lift are a single main-thread job★
//! ([`lift`]) — if the two split, a report arriving between them has its applied value erased by the late OS lift, leaving the ledger saying 「입혔다」 (applied) while the OS has no
//! minimum size (U8b review 2). While the mark is raised, a size change does not apply the OS minimum size ([`ChromeBook::on_frame`]) — so that a size change
//! emitted by the lift does not immediately re-apply it. The mark of a window that has a received minimum size always re-applies that value
//! ([`ChromeBook::mark_opening`]) — even if the ledger and the OS have drifted apart, it is fixed on opening.

use std::collections::HashMap;
use std::sync::Mutex;

use engram_dashboard_base::sync;

// ADR-0303 — chrome-style title bar
#[derive(Debug, Default)]
pub(crate) struct ChromeBook {
    windows: HashMap<String, WindowChrome>,
}

#[derive(Debug, Default)]
struct WindowChrome {
    title: Option<TitleMark>,
    min: Option<MinMark>,
    // Opening mark — raised when the shell opens that window ([`ChromeBook::mark_opening`] · [`ChromeBook::lift_min`] before a new place) and
    //   withdrawn by the first decision that receives the minimum size (right there if already received when the mark is written · otherwise the next report). Widen-and-pull-in is done
    //   only in the decision that withdraws this mark (once on opening — TRD §7-7 · PRD R20 · AC34). While it is up, a size change does not apply the OS minimum size.
    opening: bool,
    // The minimum size last applied to the OS window — `None` = not yet applied, or applying failed.
    applied: Option<MinSize>,
}

impl WindowChrome {
    // If the recorded minimum size differs from the value applied to the OS, that value — it is recorded as applied (if it could not be applied the caller calls
    //   [`ChromeBook::min_failed`]).
    fn next_min(&mut self) -> Option<MinSize> {
        let want = self.min.as_ref()?.size;
        if self.applied == Some(want) {
            return None;
        }
        self.applied = Some(want);
        Some(want)
    }
}

/// The window's size state when deciding whether to apply the minimum size — the adapter reads it inside the same main-thread job as the decision (module head).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameState {
    /// Normal size (visible or hidden — a hidden window's deferred maximize is still normal size) — the minimum size is applied.
    Normal,
    /// Maximized · minimized · unreadable — the OS minimum size is not touched now (module head).
    Other,
}

/// Minimum-size decision — the adapter applies it with [`apply_min`] outside the ledger lock.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct MinPlan {
    /// Widen-and-pull-in — if the normal size is narrower than this width, widen it and pull it into the work area (`state::placement::fit_min_width`).
    pub(crate) widen: Option<MinSize>,
    /// The value to apply as the OS minimum size — `None` = do not touch the OS.
    pub(crate) set_min: Option<MinSize>,
}

/// The answer of [`ChromeBook::on_min_report`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MinReport {
    pub(crate) verdict: MinVerdict,
    pub(crate) plan: MinPlan,
}

/// The minimum window size the web measured for that window's title row (CSS px = logical px · based on the normal state — TRD S26-window-chrome §7-7). Both values
/// are `1..=MIN_SIZE_MAX` ([`MinSize::new`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MinSize {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

// Generously larger than the largest value of the web measurement (a three-digit number marker · three buttons · area minimum cap 1000) — anything beyond that is seen as a measurement error. If applied,
//   the window would become larger than the screen.
const MIN_SIZE_MAX: u32 = 4096;

impl MinSize {
    /// `Err` (that message) if either one is outside `1..=MIN_SIZE_MAX` (4096).
    pub(crate) fn new(width: u32, height: u32) -> Result<Self, String> {
        let range = 1..=MIN_SIZE_MAX;
        if !range.contains(&width) || !range.contains(&height) {
            return Err(format!(
                "최소 창 크기 {width}x{height} — 둘 다 1~{MIN_SIZE_MAX} 이어야 한다"
            ));
        }
        Ok(Self { width, height })
    }
}

/// The order of one minimum-size report (TRD S26-window-chrome §7-7) — `epoch` = the wall-clock ms at which the web page of that window was loaded (web
/// `Math.floor(performance.timeOrigin)` — the later it is reloaded, the larger. Two pages loaded in the same ms are mixed into one page — not
/// handled) · `seq` = a sequence number that rises by one on every send within that page (retries included) (from 1).
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

/// The decision of [`ChromeBook::record_min`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MinVerdict {
    /// Accepted and different from the previous value (including that window's first report).
    Changed,
    /// Accepted and the same as the previous value.
    Same,
    /// Has an `epoch` older than the accepted one, or, in the same `epoch`, a sequence number no newer than the already accepted `seq` — an old attempt the web
    /// left on a deadline (including one from the page before reload) arrived late. It is not recorded.
    Stale,
}

#[derive(Debug)]
struct TitleMark {
    version: u64,
    // `None` = applying that version failed — the next new version is applied even if its text is the same.
    text: Option<String>,
}

/// The decision of [`ChromeBook::accept_title`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TitleVerdict {
    /// A new version and the text changed — apply it.
    Set,
    /// A new version but the same as the text last applied — the OS is not called.
    Same,
    /// Not newer than the version already accepted for that window — an old notification that arrived late because the order got reversed.
    Stale,
}

impl ChromeBook {
    /// Decide whether to apply title `title` of version `version` to window `label`, and record it. A version is accepted only when it is greater than the last version accepted for that window
    /// (the same version is an old version), and when accepted the version is recorded even if it is `Same` — so that a later old version cannot slip in
    /// behind the same text. If `Set`, the caller applies it right away (if it could not, [`Self::title_failed`]). If there is no entry it is created —
    /// whether that OS window exists now is checked by the caller first (module head).
    // ADR-0301 — tab name · OS title
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

    /// The title of `label` accepted as `Set` could not be applied — the version is kept and only the text is cleared, so the next new version is applied even if its text is the same.
    pub(crate) fn title_failed(&mut self, label: &str) {
        if let Some(mark) = self
            .windows
            .get_mut(label)
            .and_then(|window| window.title.as_mut())
        {
            mark.text = None;
        }
    }

    /// Record the minimum-size report of window `label`. An `epoch` older than the accepted one is discarded, within the same `epoch` it is accepted only when greater than the last accepted
    /// `seq`, and with a larger `epoch` (that window's page was reloaded) the sequence is counted afresh — so regardless of the order in which reports arrive, the value
    /// sent last by the newest page remains. If there is no entry it is created (checking the window is the caller's job — module head).
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

    /// The shell has just opened window `label` (TRD §7-7 — a window port's fresh build · confirmation of an entrusted prewarmed window · boot-restore window build · the
    /// hidden pop-out build of copy acceptance · placing the main slot of copy acceptance) — decide widen-and-pull-in once. If that window's minimum size has not yet been
    /// received, raise the mark so that the next accepted report decides ([`Self::on_min_report`]), and return an empty decision. If already received, decide now with `state`
    /// and withdraw the mark — if normal size, widen and pull in, and ★always re-apply that value to the OS★ (even if it equals what is recorded as applied —
    /// this decision fixes a ledger/OS drift · TRD §7-7 · U8b review 2 fix). If maximized · minimized, neither widen nor apply (once on
    /// opening · module head). If there is no entry it is created (checking the window is the caller's job — module head). `state` is read after the place is applied.
    // ADR-0303 — chrome-style title bar
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

    /// The shell is about to apply a new normal place to window `label`, which it does not rebuild (main in runtime copy acceptance) — raise the opening mark, and if it is normal
    /// size and the OS minimum size has been applied, forget the applied value. `true` = the caller lifts the OS minimum size (module head —
    /// if not lifted, the OS grows a narrow place and widen-and-pull-in has nothing to do). Maximized · minimized windows are not lifted (the lifting setter
    /// releases the maximize — module head). After the place is applied, [`Self::mark_opening`] widens and pulls in and re-applies. Body = [`lift`] (a single main-thread
    /// job). It does not create an entry — if there is no entry there is nothing to lift.
    // ADR-0303 — chrome-style title bar
    pub(crate) fn lift_min(&mut self, label: &str, state: FrameState) -> bool {
        let Some(window) = self.windows.get_mut(label) else {
            return false;
        };
        window.opening = true;
        state == FrameState::Normal && window.applied.take().is_some()
    }

    /// One minimum-size report — record it ([`Self::record_min`]) and decide what to apply. If the sequence is old (`Stale`), nothing is applied and
    /// the opening mark is not withdrawn either. An accepted report withdraws the opening mark, and widen-and-pull-in happens only if that mark was up and the window is now normal size
    /// (a window opened during maximize · minimize is not widened — once on opening). At normal size, the OS minimum size is applied if it differs from the applied value
    /// (otherwise [`Self::on_frame`] applies it when the window becomes normal size — module head).
    // ADR-0303 — chrome-style title bar
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

    /// The size state of window `label` changed (`Resized` — including return from maximize · minimize) — if it became normal size and the recorded minimum size
    /// has not yet been applied to the OS, have that value applied (no widen-and-pull-in). If the opening mark is up, it is not applied — the decision
    /// that withdraws that mark applies it after widening and pulling in (module head). It does not create an entry.
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

    /// The OS minimum size the decision said to apply could not be applied — forget the applied value so the next report · size change applies it again.
    pub(crate) fn min_failed(&mut self, label: &str) {
        if let Some(window) = self.windows.get_mut(label) {
            window.applied = None;
        }
    }

    /// Forget the entry of window `label` — the `Destroyed` of that OS window. A no-op if absent.
    pub(crate) fn forget(&mut self, label: &str) {
        self.windows.remove(label);
    }
}

/// The body of one minimum-size report — `window` = the current size state of the OS window of that label (`None` = no window · the caller reads it inside the same main-thread
/// job — module head). If out of range it is `Err` and the ledger is not touched (the sequence is not recorded either). If there is no window nothing is recorded
/// and `Ok(None)`. If the sequence is old it is not recorded and the decision is empty. The decision is [`ChromeBook::on_min_report`] — applying is done by the caller outside the ledger
/// lock with [`apply_min`].
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
            "OS 창이 없어 최소 크기 보고를 적지 않는다"
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
            "옛 순번의 최소 크기 보고 — 버린다"
        );
    }
    Ok(Some(report))
}

/// The OS operations on one window to which the minimum size is applied — a seam cut so that [`apply_min`] can be tested without a window. The real thing = the Tauri window of `crate::chrome`.
pub(crate) trait MinWindow {
    /// If the normal size is narrower than `min_w` (logical px), widen it to that width and pull it into the work area (`state::placement::fit_min_width`).
    /// `Ok(true)` = moved · `Ok(false)` = there was nothing to widen.
    fn widen_to(&self, min_w: u32) -> Result<bool, String>;
    /// Apply the OS minimum size — `min` = the logical inner px the web reported. Keeping the inner size from shrinking below that value is the real implementation's job
    /// ([`os_min_size`]).
    fn apply_min_size(&self, min: MinSize) -> Result<(), String>;
    /// Lift the OS minimum size ([`lift`]).
    fn clear_min_size(&self) -> Result<(), String>;
}

/// The body of the lift job before a new place (`crate::chrome::lift_min` hands it over as a main-thread job) — `state` · `window` = the OS window of that label
/// and its size state (the caller sees that the window exists and reads it inside the same job). Write the lift in the ledger ([`ChromeBook::lift_min`]) and,
/// if there was a value to lift, lift the OS minimum size after releasing the ledger lock. ★Because writing and lifting are inside one main-thread job, that window's report (a main-thread
/// synchronous command) cannot come in between★ (module head). If lifting failed, only a warn is left — the ledger recorded it as lifted, and the decision of the opening
/// mark re-applies the stored value ([`ChromeBook::mark_opening`] — an old value remaining in the OS could block widen-and-pull-in).
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
            "새 자리 앞에 OS 최소 창 크기를 걷지 못했다: {e}"
        );
    }
}

/// The logical size to pass to the OS minimum-size setter (Tauri `set_min_size` → tao `set_min_inner_size`) — to the inner minimum size `min` the web reported, add
/// that window's invisible border (`frame_w` · `frame_h` = outer − inner width · height · physical px), converted to logical with scale `scale`.
/// If the scale is not a finite positive number, nothing is added.
///
/// Reason (tao 0.35.3 · source reading): the setter's name says 「안쪽」 (inner), but the `WM_GETMINMAXINFO` handling (`platform_impl/windows/event_loop.rs`)
/// computes the minimum tracking size from that value with `util::adjust_size`, and that computation (`util.rs` `adjust_window_rect`), for a frameless window,
/// removes `WS_CAPTION` · `WS_SIZEBOX` and calls `AdjustWindowRectEx`, so no border is added. But for a frameless window with a shadow,
/// `WM_NCCALCSIZE` shrinks the inner area by the resize border on left · right · bottom and by one or two px on top (Windows 11) (`util.rs`
/// `calculate_insets_for_dpi`) — so if passed as is, the inner area is dragged narrower than the reported value by that border (about 16 wide · 9 high px at 100 %).
/// The size setter of the same version (`set_inner_size`) adds that difference (`undecorated_with_shadows`). The border is proportional to the scale,
/// so the logical value stays almost the same across monitor moves — only the rounding share of the top border may be off by up to 1 px.
// ADR-0303 — chrome-style title bar
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

/// Apply decision `plan` to window `label` — called outside the ledger lock (a leaf lock — the OS is not called while holding it). ★Widen-and-pull-in
/// comes first★ — if the OS minimum size were applied first, the OS would widen it to the right keeping the left edge, push it out of the work area, and only then pull it back. What failed
/// gets a warn, and an OS minimum size that could not be applied is recorded in the ledger ([`ChromeBook::min_failed`]).
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
                "연 창이 최소 폭보다 좁아 넓혀 당겼다"
            ),
            Ok(false) => {}
            Err(e) => tracing::warn!(
                module = "chrome",
                label,
                min_width = min.width,
                "연 창을 최소 폭으로 넓히지 못했다(OS 최소 크기만 걸린다): {e}"
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
                "OS 최소 창 크기를 입혔다"
            ),
            Err(e) => {
                tracing::warn!(
                    module = "chrome",
                    label,
                    width = min.width,
                    height = min.height,
                    "OS 최소 창 크기를 입히지 못했다(다음 보고 · 크기 바뀜이 다시 입힌다): {e}"
                );
                sync::lock(book).min_failed(label);
            }
        }
    }
}

/// The body of one title job — runs on the main thread. `window` = the OS window of that label (`None` if absent — it was closed). If there is no window it finishes without
/// touching the ledger. The decision is inside the `book` lock, and `set_title` is called after releasing that lock (a leaf lock — the OS is not called while
/// holding it · TRD S26-window-chrome §5-1). A failure leaves a warn and is recorded in the ledger ([`ChromeBook::title_failed`]).
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

    // A job that arrived late after `Destroyed` forgot it — there is no window, so the entry is not revived (TRD S26-window-chrome §4-4).
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
            "거절한 보고의 순번(10~13)은 적지 않았다 — 2 가 받아들여진다"
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
            "버린 295 는 적히지 않았다"
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
            "옛 페이지의 순번이 더 커도 새 페이지 값을 덮지 못한다"
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
            "같은 값은 다시 입히지 않는다"
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
            "한 번 입힌 값은 크기가 바뀔 때마다 다시 입히지 않는다"
        );
    }

    #[test]
    fn maximizing_and_restoring_leave_an_applied_min_alone() {
        let mut book = ChromeBook::default();
        plan(&mut book, 1, size(286, 64), Normal);

        assert_eq!(
            book.on_frame(POPUP, Other),
            MinPlan::default(),
            "최대화로 걷지 않는다"
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
            "최소 크기를 받기 전의 표시는 다음 보고를 기다린다"
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
            "열 때 최대화였던 창은 나중에 원래 크기로 돌아와도 넓혀 당기지 않는다"
        );
    }

    // A window that is not rebuilt (main in runtime copy acceptance) — its page does not report again if the value is unchanged, so the decision is made
    //   at the place where the mark is written (module head).
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
            "같은 값이어도 넓혀 보고 다시 입힌다 — 장부와 OS 가 어긋났어도 고친다"
        );
        assert_eq!(
            plan(&mut book, 2, size(295, 64), Normal),
            set_only(size(295, 64)),
            "표시는 거뒀다"
        );
    }

    #[test]
    fn marking_a_maximized_window_whose_min_is_known_never_widens() {
        let mut book = ChromeBook::default();
        plan(&mut book, 1, size(286, 64), Normal);

        assert_eq!(book.mark_opening(POPUP, Other), MinPlan::default());
        assert_eq!(plan(&mut book, 2, size(295, 64), Normal).widen, None);
    }

    // The OS minimum size lifted before a new place is re-applied by the decision that withdraws the mark, after widen-and-pull-in — size changes in between (emitted by lifting · placing)
    //   are not applied (module head).
    #[test]
    fn a_lifted_min_is_held_until_the_mark_widens_and_applies_it_again() {
        let mut book = ChromeBook::default();
        plan(&mut book, 1, size(286, 64), Normal);

        assert!(book.lift_min(POPUP, Normal), "입혀 둔 값을 걷는다");
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

    // A sequence where that window's report slipped in between the lift and the opening mark (U8b review 2) — even if the report withdrew the mark and applied a new
    //   value, the opening mark re-applies that value. The lift and the report are one main-thread job each, so there is now no OS lift that can slip between them (module head). But if afterwards
    //   the place setter was caught on the OS minimum size and grew a narrow place keeping its left edge, the width is already at the minimum so `fit_min_width` does not
    //   pull it in — it may remain spilled past the right of the work area (rare · accepted · TRD §7-7).
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
            "보고가 걷기의 표시를 거둔다"
        );
        assert_eq!(
            book.mark_opening(POPUP, Normal),
            MinPlan {
                widen: Some(size(295, 64)),
                set_min: Some(size(295, 64)),
            },
            "입힌 값으로 적힌 같은 값이어도 다시 입힌다"
        );
        assert_eq!(book.on_frame(POPUP, Normal), MinPlan::default());
    }

    #[test]
    fn a_maximized_window_keeps_its_os_min_through_a_lift() {
        let mut book = ChromeBook::default();
        plan(&mut book, 1, size(286, 64), Normal);

        assert!(
            !book.lift_min(POPUP, Other),
            "걷는 세터가 최대화를 푼다 — 걷지 않는다"
        );
        assert_eq!(book.mark_opening(POPUP, Other), MinPlan::default());
        assert_eq!(
            book.on_frame(POPUP, Normal),
            MinPlan::default(),
            "OS 에 남은 값을 다시 입히지 않는다"
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
            "열기 표시를 함께 잊는다"
        );
        book.forget(POPUP);
        assert_eq!(
            plan(&mut book, 1, size(286, 64), Normal),
            set_only(size(286, 64)),
            "입힌 값을 함께 잊는다"
        );
    }

    // ── invisible border added to the OS minimum size (`os_min_size`) ──

    #[test]
    fn the_os_min_adds_the_invisible_frame_in_logical_px() {
        assert_eq!(
            os_min_size(size(286, 64), 16, 9, 1.0),
            (302.0, 73.0),
            "100 % — 좌우 8 + 8 · 위 1 + 아래 8"
        );
        let (w, h) = os_min_size(size(286, 64), 24, 13, 1.5);
        assert_eq!(w, 302.0, "150 % — 물리 24 = 논리 16");
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
                "최소 창 크기 0x64 — 둘 다 1~{MIN_SIZE_MAX} 이어야 한다"
            )),
            "범위 검사가 창 확인보다 먼저다"
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
            "두 번째 걷기는 걷을 값이 없다"
        );
        assert_eq!(
            book.lock().unwrap().mark_opening(POPUP, Normal),
            MinPlan {
                widen: Some(size(286, 64)),
                set_min: Some(size(286, 64)),
            },
            "걷기가 세운 표시를 열기 표시가 거둔다"
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
        window.set = Err("창 핸들 무효(시험)".to_owned());

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
        window.widen = Err("모니터를 못 읽었다(시험)".to_owned());

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
        window.set = Err("창 핸들 무효(시험)".to_owned());

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
