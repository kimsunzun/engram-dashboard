//! Tauri adapter of the window chrome — holds [`book::ChromeBook`] under a leaf lock and applies it to the OS window (TRD S26-window-chrome §2-2 · §4-4 · §5-1 · §7-7): the OS window title · the web-reported minimum window size (OS minimum size · widen-and-pull-in once on opening).
//! The title's source of truth is the tab notification payload (`title` · `version` of `layout::apply::WindowTabsPayload`); the notification port's Tauri implementation (`commands::layout::TauriEvents`) calls [`push_title`] after the emit — notifications are outside the layout lock, so several threads may see their order reversed; one main-thread line + the ledger's version comparison discards an old version.
//! The minimum size's source of truth is the web (policy constants live in the screen — precedent ADR-0227). Reports (invoke `report_window_min_width` → [`report_min_size`]) · the opening mark ([`mark_opening`]) · the lift before a new place ([`lift_min`]) · size changes ([`on_resized`]) all look at the ledger and apply inside a single main-thread job — what is applied and when is in the `book` head.

mod book;

pub(crate) use book::MinOrder;

use std::sync::Mutex;

use engram_dashboard_base::sync;
use tauri::{AppHandle, LogicalSize, Manager, Window};

use crate::layout::WindowTabsPayload;
use crate::state::placement::{self, MonitorArea};

#[derive(Default)]
pub(crate) struct ChromeState(Mutex<book::ChromeBook>);

/// Hand the job applying the title of tab notification `tabs` to that OS window to the main thread without waiting. Called on the main thread it runs on the spot (tauri-runtime-wry 2.11.3 `send_user_message`) — do not call it holding any lock.
pub(crate) fn push_title(app: &AppHandle, tabs: &WindowTabsPayload) {
    let handle = app.clone();
    let label = tabs.label.clone();
    let version = tabs.version;
    let title = tabs.title.clone();
    let queued = app.run_on_main_thread(move || {
        let Some(state) = handle.try_state::<ChromeState>() else {
            tracing::debug!(label, "chrome ledger is not registered; title not applied");
            return;
        };
        book::apply_title(
            &state.0,
            &label,
            version,
            &title,
            handle.get_webview_window(&label),
            |window, text| window.set_title(text).map_err(|e| e.to_string()),
        );
    });
    if let Err(e) = queued {
        tracing::warn!(label = %tabs.label, version, "failed to hand the OS window title job to the main thread: {e}");
    }
}

/// The shell has just opened window `label` — record the opening mark so widen-and-pull-in is decided once (TRD §7-7 — a window port's fresh build · confirmation of an entrusted prewarmed window · boot-restore window build · copy acceptance's hidden pop-out build · placing copy acceptance's main slot). If that window's minimum size has not been received the next report decides; if received (a window not rebuilt) this job decides and applies (`book::ChromeBook::mark_opening`). Call it after building the window (after applying the place for a window not rebuilt). Recording is handed to a main-thread job without waiting — it records only after seeing that the window exists (`book` head) — and runs before that window's page reports: the main thread is a single line and the report comes on it after the page loads. Called on the main thread it runs on the spot — do not call it holding any lock.
pub(crate) fn mark_opening(app: &AppHandle, label: &str) {
    let handle = app.clone();
    let owned = label.to_owned();
    let queued = app.run_on_main_thread(move || {
        let Some(window) = handle
            .get_webview_window(&owned)
            .map(|window| window.as_ref().window())
        else {
            tracing::debug!(label = owned, "no OS window; opening mark not recorded");
            return;
        };
        let Some(state) = handle.try_state::<ChromeState>() else {
            return;
        };
        // The size state is read outside the ledger lock (a leaf lock — no OS call while holding it).
        let frame = frame_state(&window);
        let plan = sync::lock(&state.0).mark_opening(&owned, frame);
        book::apply_min(&state.0, &owned, plan, &window);
    });
    if let Err(e) = queued {
        tracing::warn!(
            label,
            "failed to hand the opening-mark job to the main thread (that window is not widened or pulled in): {e}"
        );
    }
}

/// Record and apply the minimum window size the web reported for window `label` — called by invoke `report_window_min_width`. Out of range → `Err`, previous value kept (`book::MinSize::new`). If that OS window is absent or `order` is not newer than the sequence accepted for it, nothing is recorded and `Ok` (`book::ChromeBook::record_min`). For a normal-size window the OS minimum is applied, widening and pulling in first if the opening mark is up (`book::ChromeBook::on_min_report`). Checking the window · reading the state · recording · applying must be inside one main-thread job so the forgetting on `Destroyed` and size changes cannot come between (`book` head) — a non-async Tauri command runs on the main thread (Tauri 2 docs 「Calling Rust from the Frontend」, which this shell's synchronous commands rely on).
pub(crate) fn report_min_size(
    app: &AppHandle,
    label: &str,
    width: u32,
    height: u32,
    order: MinOrder,
) -> Result<(), String> {
    let state = app
        .try_state::<ChromeState>()
        .ok_or_else(|| "chrome ledger is not registered".to_owned())?;
    let window = app
        .get_webview_window(label)
        .map(|window| window.as_ref().window());
    let frame = window.as_ref().map(frame_state);
    let Some(report) = book::report_min(&state.0, label, width, height, order, frame)? else {
        return Ok(());
    };
    if report.verdict == book::MinVerdict::Changed {
        tracing::debug!(label, width, height, "minimum window size report changed");
    }
    if let Some(window) = window {
        book::apply_min(&state.0, label, report.plan, &window);
    }
    Ok(())
}

/// The window's size changed (`WindowEvent::Resized` — main thread): if it returned to normal size from maximize · minimize and the recorded minimum has not been applied to the OS, apply it (`book::ChromeBook::on_frame`). Do not call it holding any lock.
// ADR-0303
pub(crate) fn on_resized(window: &Window) {
    let Some(state) = window.app_handle().try_state::<ChromeState>() else {
        return;
    };
    let label = window.label();
    let frame = frame_state(window);
    let plan = sync::lock(&state.0).on_frame(label, frame);
    book::apply_min(&state.0, label, plan, window);
}

/// Just before the shell applies a new normal place to window `label`, which it does not rebuild — lift the applied OS minimum size and raise the opening mark (lifted only at normal size — `book::ChromeBook::lift_min`). After applying the place call [`mark_opening`] — that job widens, pulls in and re-applies. Used now by main in runtime copy acceptance (`state::placement::TauriRestoreWindows::place_main` — a rebuilt window is new, so nothing to lift).
/// Hand reading the size state · writing the ledger · lifting the OS to a single main-thread job and do not wait (`book::lift`): writing on the caller's thread with only the OS lift queued would let a report of that window in between (a main-thread synchronous command) have its applied value erased by the late lift, leaving only the ledger "applied". If the caller is off the main thread, this job · the place setter after it · [`mark_opening`]'s job run in the order handed over — all three on the same event-loop line (tauri-runtime-wry 2.11.3 `send_user_message` → `proxy.send_event`). Even if a report slips between applying the place and the opening mark, the mark widens and pulls in again with the stored value and applies it (`book::ChromeBook::mark_opening`). Called on the main thread it runs on the spot — do not call it holding any lock.
// ADR-0303
pub(crate) fn lift_min(app: &AppHandle, label: &str) {
    let handle = app.clone();
    let owned = label.to_owned();
    let queued = app.run_on_main_thread(move || {
        let Some(window) = handle
            .get_webview_window(&owned)
            .map(|window| window.as_ref().window())
        else {
            tracing::debug!(label = owned, "no OS window; minimum size not lifted");
            return;
        };
        let Some(state) = handle.try_state::<ChromeState>() else {
            return;
        };
        let frame = frame_state(&window);
        book::lift(&state.0, &owned, frame, &window);
    });
    if let Err(e) = queued {
        tracing::warn!(
            label,
            "failed to hand the pre-placement lift job to the main thread (a narrow copy's place can stay widened to the OS minimum size): {e}"
        );
    }
}

pub(crate) fn forget(app: &AppHandle, label: &str) {
    if let Some(state) = app.try_state::<ChromeState>() {
        sync::lock(&state.0).forget(label);
    }
}

// Tell apart by tao's maximize · minimize flags (the very flags the OS minimum-size setter touches — `book` head). If unreadable, it is on the side of not touching.
fn frame_state(window: &Window) -> book::FrameState {
    match (window.is_maximized(), window.is_minimized()) {
        (Ok(false), Ok(false)) => book::FrameState::Normal,
        _ => book::FrameState::Other,
    }
}

impl book::MinWindow for Window {
    // Normal place = physical outer position · logical inner size (`state::placement` head). Work area = that of the monitor the window sits on (a hidden window uses the nearest) · scale = that window's scale (what unfolds a logical width to physical — as tao's size setter). Outer width − inner width = the invisible resize border that remains even with the OS frame removed (tao 0.35.3 `calculate_insets_for_dpi`).
    fn widen_to(&self, min_w: u32) -> Result<bool, String> {
        let scale = self.scale_factor().map_err(|e| e.to_string())?;
        let at = self.outer_position().map_err(|e| e.to_string())?;
        let inner = self.inner_size().map_err(|e| e.to_string())?;
        let outer = self.outer_size().map_err(|e| e.to_string())?;
        let bounds = placement::saved_bounds(at.x, at.y, inner.width, inner.height, scale)
            .ok_or_else(|| {
                format!("window size read as an unusable value ({inner:?} · scale {scale})")
            })?;
        let monitor = self
            .current_monitor()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "could not read the monitor the window sits on".to_owned())?;
        let work = monitor.work_area();
        let area = MonitorArea {
            x: work.position.x,
            y: work.position.y,
            w: work.size.width,
            h: work.size.height,
            scale,
        };
        let frame = f64::from(outer.width.saturating_sub(inner.width));
        let fitted = placement::fit_min_width(bounds, f64::from(min_w), area, frame);
        if fitted == bounds {
            return Ok(false);
        }
        // QA material (AC27 · AC34 — compare the widened width · pulled-in place with the work area).
        tracing::debug!(
            module = "chrome",
            label = self.label(),
            from = ?bounds,
            to = ?fitted,
            work = ?area,
            frame,
            "widen and pull in"
        );
        // Apply the position only when the position changes — widening only the size leaves the left edge as is (the size setter leaves the position and changes only the size).
        let pos = (fitted.x() != bounds.x())
            .then(|| (fitted.x().round() as i32, fitted.y().round() as i32));
        placement::move_steps(self, pos, Some((fitted.w(), fitted.h())))
            .map_err(|failed| format!("{:?}: {}", failed.step, failed.error))?;
        Ok(true)
    }

    // Add the invisible border (outer − inner) to the setter's argument — tao's minimum tracking size leaves it out, so the inner area shrinks that much more (the reason for `book::os_min_size`). Called only for normal-size windows (`book` head), so the measured difference is that window's border.
    fn apply_min_size(&self, min: book::MinSize) -> Result<(), String> {
        let scale = self.scale_factor().map_err(|e| e.to_string())?;
        let inner = self.inner_size().map_err(|e| e.to_string())?;
        let outer = self.outer_size().map_err(|e| e.to_string())?;
        let (width, height) = book::os_min_size(
            min,
            outer.width.saturating_sub(inner.width),
            outer.height.saturating_sub(inner.height),
            scale,
        );
        self.set_min_size(Some(LogicalSize::new(width, height)))
            .map_err(|e| e.to_string())
    }

    fn clear_min_size(&self) -> Result<(), String> {
        self.set_min_size(None::<LogicalSize<f64>>)
            .map_err(|e| e.to_string())
    }
}
