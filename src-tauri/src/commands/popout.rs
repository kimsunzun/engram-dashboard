//! Slot popup detach (move_slot_to_window) invoke shell + runtime window label·build·destroy (= the apply service's OS
//! window port · label issuing port adapter, `crate::layout::apply`) — tab-owning model (ADR-0057).
//!
//! ★§5 LLM control surface★: a human right-click (`slot.popout` of the frontend registry → `viewStore.moveSlotToWindow`
//! invoke) and the LLM (`slot.popout` of the command bus) land on the **same apply service** — there is no MOVE logic
//! in this file (`apply::move_slot_to_window`).
//! What remains here is only translation into the Tauri world: window build·destroy·existence check, label·tab name issuing, window cleanup.
//!
//! ★The two paths join **only in the backend — before that they still differ**★: before the invoke the frontend clears that slot's render mode
//! override (`clearRenderMode` of `src/store/viewStore.ts` — the slot disappears, so this prevents a leak). The bus
//! path has no such step, so the override of a dead slot id remains in the webview. There are three more of the same kind —
//! `slot.close`·`slot.assignAgent`·`layout.setSlotContent` also do the same cleanup on the frontend side and do not
//! on the bus side. Resolving it is the job of a follow-up step that lifts webview-owned state into the backend (do not imitate it here — the shell does not
//! hold that state).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, State, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};
use uuid::Uuid;

use crate::commands::layout::{resolve_window, RouterSubs, TauriEvents};
use crate::daemon_client::DaemonClient;
use crate::first_paint::NewWindowShow;
use crate::layout::tab_name::window_title;
use crate::layout::{
    apply, LabelSource, LayoutState, MoveFailed, SlotMove, SpareClaim, WindowHost, WindowPlace,
    MAIN_WINDOW_LABEL,
};
use crate::output_router::{DropCause, OutputRouter};
use crate::spare::pool::Opening;
use crate::state::placement::{self, Fallback};
use crate::webview_env::{Backdrop, WebviewEnv};

// Popup/runtime window label prefix. Paired with the `"slot-popup-*"` glob of capabilities/popup.json·usage-links.json (when changed,
// sync them together).
// ★Meaning extension (ADR-0057/G8)★: "popup" → "runtime window" (including create_window). The prefix value is unchanged
// (reuse of the Destroyed cleanup gate is_popup_label — with a different label cleanup is skipped → routing/subscription/Channel leak).
const POPUP_LABEL_PREFIX: &str = "slot-popup-";

// Run marker — 8 lowercase hex characters drawn once per process (TRD S26-window-chrome §4-5 · main decision M8). The sequence number restarts from 1
//   on every run, so without the marker a bus caller holding a label from a previous run would pick a different window of this run. The probability of the same marker as a previous run
//   (2⁻³²) is accepted.
fn run_marker() -> &'static str {
    static RUN: OnceLock<String> = OnceLock::new();
    RUN.get_or_init(|| Uuid::new_v4().simple().to_string()[..8].to_string())
}

/// The label of the `n`-th popup/runtime window of this run — `slot-popup-<실행 표식>-<n>` (the bracketed part is the run marker). [`PopupCounter`] issues it in this form.
pub fn popup_label(n: u64) -> String {
    format!("{POPUP_LABEL_PREFIX}{}-{n}", run_marker())
}

// Monotonic counter for issuing popup/runtime window labels. app-level shared (app.manage). ★No-reuse invariant★: fetch_add
// only increases monotonically and is not rolled back even when a window is closed (avoids a closed-label re-build error).
#[derive(Default)]
pub struct PopupCounter(pub AtomicU64);

impl PopupCounter {
    // ADR-0302 — window state command (no reuse of pop-out labels)
    fn next_label(&self) -> String {
        popup_label(self.0.fetch_add(1, Ordering::Relaxed) + 1)
    }
}

impl LabelSource for PopupCounter {
    fn next_label(&self) -> String {
        PopupCounter::next_label(self)
    }
}

// The apply service's OS window port (`crate::layout::apply`) — window build·destroy·existence check is the Tauri technique this file owns.
pub(crate) struct TauriWindowHost<'a> {
    pub app: &'a AppHandle,
}

impl WindowHost for TauriWindowHost<'_> {
    fn open(&self, label: &str) -> Result<(), String> {
        open_new_window(self.app, label, None, true)
    }

    fn open_at(&self, label: &str, at: &WindowPlace, activate: bool) -> Result<(), String> {
        open_new_window(self.app, label, Some(at), activate)
    }

    // ADR-0299
    fn claim_spare(&self) -> Option<SpareClaim> {
        crate::commands::spare::claim(self.app)
    }

    fn committed(&self, claim: &SpareClaim) {
        crate::commands::spare::commit(self.app, claim);
    }

    fn release_spare(&self, claim: &SpareClaim) {
        crate::commands::spare::release(self.app, claim);
    }

    fn close(&self, label: &str) {
        destroy_window(self.app, label);
    }

    fn is_open(&self, label: &str) -> bool {
        self.app.get_webview_window(label).is_some()
    }

    fn move_window(&self, label: &str, at: &WindowPlace) -> Result<(), MoveFailed> {
        placement::move_window(self.app, label, at)
    }

    fn record_placement(&self, label: &str) {
        if let Some(window) = self.app.get_webview_window(label) {
            crate::state::placement::record(&window.as_ref().window());
        }
    }
}

// The window port's open new window — an entrusted prewarmed window is not built but placed hidden, and a label unrelated to the pool is built anew. `at = None` =
//   `open` (staircase place · default size). A newly built window decides at open — if it is an activating request and the foreground is ours, it is built receiving focus as it is now,
//   otherwise it is built without focus and shown in a raise-only way (I30). Activation of an entrusted window is decided right before showing (I5).
//   The `window_open` line = material that tells prewarmed window · fresh-build latency apart (TRD S21-slot-docking §3-9 「로그」 · M14 — open → visible is
//   `reveal_ms` of the first-paint release line · waiting during build is `wait_ms` of the `spare_claim` line).
// ADR-0299
fn open_new_window(
    app: &AppHandle,
    label: &str,
    at: Option<&WindowPlace>,
    activate: bool,
) -> Result<(), String> {
    let started = Instant::now();
    match crate::commands::spare::opening(app, label, activate) {
        Opening::Claimed => {
            let placed = place_claimed(app, label, at)?;
            tracing::info!(
                module = "spare",
                label,
                via = "spare",
                activate,
                place_ms = crate::commands::spare::elapsed_ms(started),
                scale_from = placed.scale_from,
                scale_to = ?placed.scale_to,
                "창 포트 — 새 창을 열었다(window_open)"
            );
            Ok(())
        }
        Opening::Unavailable => {
            tracing::warn!(
                module = "spare",
                label,
                "창 포트 — 맡긴 예열 창이 사라졌다 · 그 label 로 새로 짓지 않는다"
            );
            Err(format!(
                "맡긴 예열 창 {label} 이 사라져 그 label 로 새로 짓지 않는다"
            ))
        }
        Opening::Fresh => {
            let focused =
                crate::first_paint::new_window_show(activate, placement::foreground_is_ours(app))
                    == NewWindowShow::RaiseAndActivate;
            match at {
                Some(at) => open_runtime_window_at(app, label, at, focused, started)?,
                None => {
                    let window = build_window(app, label, None, focused)?;
                    crate::first_paint::hold_built(&window, focused, started);
                }
            }
            // A newly built window — that window's first minimum-size report widens and pulls in (TRD S26-window-chrome §7-7). For an entrusted prewarmed window, the commit
            //   raises the mark (`commands::spare::commit`).
            crate::chrome::mark_opening(app, label);
            tracing::info!(
                module = "spare",
                label,
                via = "fresh",
                activate,
                focused,
                build_ms = crate::commands::spare::elapsed_ms(started),
                "창 포트 — 새 창을 열었다(window_open)"
            );
            Ok(())
        }
    }
}

/// The scale after placing an entrusted prewarmed window — the log's share that tells apart an adoption moved to a monitor with a different scale (I9 · M16).
struct SparePlaced {
    scale_from: f64,
    scale_to: Option<f64>,
}

// Place an entrusted prewarmed window hidden ([`spare_spot`] · position → size — reason = [`placement::place`]). It is not visible — showing is the
//   first paint after commit. If it cannot be placed, withdraw that window and `Err` (window port `open_at` contract — there is no window left up · its `Destroyed` ends
//   the entrusting and rebuilds). If the window is already gone, there is nothing to withdraw and `Err` — it is not rebuilt with that label (I20).
// ADR-0299
fn place_claimed(
    app: &AppHandle,
    label: &str,
    at: Option<&WindowPlace>,
) -> Result<SparePlaced, String> {
    let Some(window) = app.get_webview_window(label) else {
        return Err(format!(
            "맡긴 예열 창 {label} 이 없다 — 그 label 로 새로 짓지 않는다"
        ));
    };
    let window = window.as_ref().window();
    let placed = window
        .scale_factor()
        .map_err(|e| format!("배율을 못 읽었다: {e}"))
        .and_then(|scale_from| {
            let monitors = match at.and_then(|at| at.pos) {
                Some(_) => placement::monitors(app),
                None => Vec::new(),
            };
            let (pos, size) = spare_spot(label, at, &monitors, scale_from);
            placement::move_steps(&window, Some(pos), Some(size))
                .map_err(|failed| format!("{:?}: {}", failed.step, failed.error))?;
            Ok(SparePlaced {
                scale_from,
                scale_to: window.scale_factor().ok(),
            })
        });
    placed.map_err(|reason| {
        tracing::warn!(
            module = "spare",
            label,
            error = %reason,
            "맡긴 예열 창을 놓지 못해 거둔다"
        );
        let gone = match try_destroy_window(app, label) {
            Ok(()) => "거뒀다".to_string(),
            Err(e) => {
                crate::commands::spare::abandon_claimed(app, label);
                format!("거두기도 실패: {e}")
            }
        };
        format!("맡긴 예열 창 자리를 못 입혔다({reason}) — {gone}")
    })
}

// The place of an entrusted prewarmed window — physical outer top-left · logical inner size. The same computation as a newly built window ([`open_runtime_window_at`] · [`build_window`])
//   — if `at` straddles some monitor, that landing; otherwise the staircase place of the label unfolded by that window's current scale `scale` (the same unfolding as Tauri's
//   logical position setter) · size = `at.size` (the default if absent or unusable). A returned prewarmed window may have been moved and resized by an earlier placement,
//   so `open` (`at = None`) also re-applies the staircase place · default size (I18).
fn spare_spot(
    label: &str,
    at: Option<&WindowPlace>,
    monitors: &[placement::MonitorArea],
    scale: f64,
) -> ((i32, i32), (f64, f64)) {
    let landed = at.and_then(|at| {
        placement::resolve_place(
            label,
            at,
            POPOUT_DEFAULT_SIZE,
            monitors,
            Fallback::DefaultPlace,
        )
    });
    if let Some((bounds, landing)) = landed {
        return ((landing.x, landing.y), (bounds.w(), bounds.h()));
    }
    let size = at
        .and_then(|at| placement::usable_size(at.size))
        .unwrap_or(POPOUT_DEFAULT_SIZE);
    let (x, y) = cascade_position(label);
    (
        ((x * scale).round() as i32, (y * scale).round() as i32),
        size,
    )
}

// Used by the lib.rs Destroyed arm to distinguish it from main.
pub fn is_popup_label(label: &str) -> bool {
    label.starts_with(POPUP_LABEL_PREFIX)
}

// ★URL key = `?window=<label>` (ADR-0057/§3-3)★: the popup page draws "this window's active tab", not a "fixed view"
// (the active tab is authoritative in the backend `windows[label].active`).
fn window_url(label: &str) -> String {
    format!("index.html#/popup?window={label}")
}

// Diagonal cascade position (prevents windows from opening overlapped) — chosen by the sequence number at the end `-<n>` of the label. If the run marker were unfolded into a number too, every window
//   would stand on the same staircase (TRD S26-window-chrome §4-5).
fn cascade_position(label: &str) -> (f64, f64) {
    let n: u32 = label
        .strip_prefix(POPUP_LABEL_PREFIX)
        .and_then(|rest| rest.rsplit('-').next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let step = (n.saturating_sub(1) % 8) as f64;
    (140.0 + step * 72.0, 110.0 + step * 60.0)
}

// Build a runtime window with WebviewWindowBuilder (★call only outside the lock — avoids deadlock★). The webview environment is attached by the common finish
// (`WebviewEnv::finish`) — if omitted, a ghost window (ADR-0054).
//
// `at` = the first place to give the builder — `None` means the label sequence's staircase place · default size. Placing a saved place exactly is done by the caller
//   (`state::placement`) after building: tao unfolds the builder's logical position with the scale of the first monitor that fits when scanning monitors in enumeration order
//   (tao 0.35 `platform_impl/windows/window.rs` `init`), so if monitors with different scales are mixed it can appear on another monitor.
//
// ★Built hidden — showing is the caller's job★: after applying the place · maximize, it is shown at first paint
//   (`first_paint::hold_window`), or if the runtime restore acceptance puts it in the model and then decides to show, it is shown at first paint
//   (`first_paint::reveal_held` · TRD S21-storage §6-7 ② ④).
pub(crate) fn build_runtime_window(
    app: &AppHandle,
    label: &str,
    at: Option<(LogicalPosition<f64>, LogicalSize<f64>)>,
) -> Result<WebviewWindow, String> {
    build_window(app, label, at, true)
}

/// Spare window (TRD S21-slot-docking §3-9) — built hidden · without focus with the same builder · URL as a normal pop-out (no spare marker)
/// (so that tao's show does not activate it). It is not put in the model and its place is not recorded — recording in the pool is the caller's job
/// (spare window building of `commands::spare`).
// ADR-0295
pub(crate) fn build_spare_window(app: &AppHandle, label: &str) -> Result<WebviewWindow, String> {
    build_window(app, label, None, false)
}

/// Pop-out default size (logical) — a runtime window opened without a place, and a docking new window whose size was not chosen.
pub(crate) const POPOUT_DEFAULT_SIZE: (f64, f64) = (720.0, 500.0);

// A docking new window (window port `open_at`) — if `at` straddles some monitor, give the builder the first place and after building apply that place again
//   exactly (position first — the reason = head of [`placement::place`] · [`build_runtime_window`]). If it does not straddle or there is no position, it is the
//   label's staircase place. If the size is absent or unusable, it is the default size. `activate` = built while receiving focus (otherwise a raise-only
//   show — `first_paint::hold_built`) · `opened` = when the window port began opening.
// TRD S21-slot-docking §3-9 「`open_at`(맡긴 label)」 · 「폴백」
fn open_runtime_window_at(
    app: &AppHandle,
    label: &str,
    at: &WindowPlace,
    activate: bool,
    opened: Instant,
) -> Result<(), String> {
    let monitors = match at.pos {
        Some(_) => placement::monitors(app),
        None => Vec::new(),
    };
    match placement::resolve_place(
        label,
        at,
        POPOUT_DEFAULT_SIZE,
        &monitors,
        Fallback::DefaultPlace,
    ) {
        Some(landed) => {
            let window = build_window(app, label, Some(placement::first_place(landed)), activate)?;
            let (bounds, landing) = landed;
            // If the chosen place could not be applied, withdraw that window and answer failure — if left, docking attaches the tab to a window at the wrong place (window port
            //   `open_at` contract).
            let placed = placement::move_steps(
                &window.as_ref().window(),
                Some((landing.x, landing.y)),
                Some((bounds.w(), bounds.h())),
            );
            if let Err(failed) = placed {
                let gone = match try_destroy_window(app, label) {
                    Ok(()) => "거뒀다".to_string(),
                    Err(e) => format!("거두기도 실패: {e}"),
                };
                return Err(format!(
                    "새 창 자리를 못 입혔다({:?}: {}) — {gone}",
                    failed.step, failed.error
                ));
            }
            crate::first_paint::hold_built(&window, activate, opened);
        }
        None => {
            let (w, h) = placement::usable_size(at.size).unwrap_or(POPOUT_DEFAULT_SIZE);
            let (x, y) = cascade_position(label);
            let window = build_window(
                app,
                label,
                Some((LogicalPosition::new(x, y), LogicalSize::new(w, h))),
                activate,
            )?;
            crate::first_paint::hold_built(&window, activate, opened);
        }
    }
    Ok(())
}

// `focused = false` = build but do not take focus (the `FocusPolicy::Keep` of a docking new window · a spare window). If `true`, even while hidden
//   focus goes to this window (wry puts it into the webview as soon as it is made — see the main-build comment in `state::placement`) — the same
//   moment as before showing.
//
// ★Always built hidden — do not give the builder `visible(true)` (head of `crate::first_paint` — first-frame bleed-through)★.
fn build_window(
    app: &AppHandle,
    label: &str,
    at: Option<(LogicalPosition<f64>, LogicalSize<f64>)>,
    focused: bool,
) -> Result<WebviewWindow, String> {
    let ((x, y), (w, h)) = match at {
        Some((at, size)) => ((at.x, at.y), (size.width, size.height)),
        None => (cascade_position(label), POPOUT_DEFAULT_SIZE),
    };
    // Do not build without the webview environment — a window that skipped the finish diverges in environment from other windows (ADR-0054).
    let env = app
        .try_state::<Arc<WebviewEnv>>()
        .ok_or_else(|| "런타임 창 생성 실패: 웹뷰 환경이 등록돼 있지 않다".to_owned())?;
    env.finish(
        WebviewWindowBuilder::new(app, label, WebviewUrl::App(window_url(label).into()))
            // The title of an unnumbered window outside the model (including a hidden prewarmed window) — once it enters the model, the tab notification overwrites it (TRD S26-window-chrome §4-4).
            .title(window_title(None, None))
            .inner_size(w, h)
            .position(x, y)
            .visible(false)
            .focused(focused),
        Backdrop::for_window(app, label),
    )
    .build()
    .map_err(|e| format!("런타임 창 생성 실패: {e}"))
}

// The Destroyed event → the lib.rs Destroyed arm → cleanup_popup_window cleans up the remainder.
// ★Window closed = backend single source (§5-2/G2)★: no separate view:closed is sent to the frontend (prevents double firing · reentry).
// The registry is not touched here (Destroyed→cleanup cleans it) — so it is not taken as an argument either (F5).
pub fn destroy_window(app: &AppHandle, label: &str) {
    if let Err(e) = try_destroy_window(app, label) {
        tracing::warn!(label, "destroy_window 실패(창 이미 닫힘일 수 있음): {e}");
    }
}

/// Same as [`destroy_window`] but returns the failure — used by the side that has to retry the withdrawal or report the remaining window (runtime restore acceptance).
/// If the OS window is absent, `Ok`.
pub(crate) fn try_destroy_window(app: &AppHandle, label: &str) -> Result<(), String> {
    match app.get_webview_window(label) {
        Some(w) => w.destroy().map_err(|e| e.to_string()),
        None => {
            tracing::debug!(label, "destroy_window: OS 창 없음(이미 닫힘) — no-op");
            Ok(())
        }
    }
}

// MOVE a slot to a new tab of another window — logic · lock discipline · rollback are all owned by the apply service.
//
// ★async fn required★: avoids a WebviewWindowBuilder deadlock (a new-window target is built in the service's phase B).
#[tauri::command]
pub async fn move_slot_to_window(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    counter: State<'_, Arc<PopupCounter>>,
    client: State<'_, Arc<DaemonClient>>,
    view_id: Uuid,
    slot_id: Uuid,
    to_window: Option<String>,
) -> Result<SlotMove, String> {
    let to_window = to_window
        .map(|window| resolve_window(&state, &window))
        .transpose()?;
    apply::move_slot_to_window(
        &state,
        &RouterSubs {
            router: &router,
            client: &client,
        },
        &TauriEvents { app: &app },
        &TauriWindowHost { app: &app },
        &**counter,
        view_id,
        slot_id,
        to_window,
    )
}

// ★Window Destroyed cleanup (lifetime/leak critical — multi-tab, G1)★. When a popup/runtime window closes (titlebar close · forced destroy ·
// destroy via close_tab/close_window), the lib.rs Destroyed arm calls this. For titlebar close, the close request
// ([`close_requested_popup_window`]) already cleared the model first, so only recomputing · releasing the closing window's number · detaching the Channel
// remain here. The window number is released by this — the caller does not release the number separately (TRD S26-window-chrome §4-1).
//
// ★Current bug fix (G1)★: the old code cleaned up only a single binding, so force-closing a multi-tab popup left the remaining tabs
// + missed Unsubscribe.
//
// ★This function is not a command★ — it is called directly from the Rust event handler (on_window_event). Instead of State it takes
// the Arc references already in hand as arguments (lib.rs takes them out with app.state and passes them). `_app` is currently unused (the signature is unified
// to leave room for an immediate emit later).
pub fn cleanup_popup_window(
    _app: &tauri::AppHandle,
    label: &str,
    state: &LayoutState,
    router: &OutputRouter,
    registry: &crate::output_channel::WindowChannelRegistry,
    client: &DaemonClient,
) {
    // main is never a cleanup target (invariant 4 — hide only, no Destroyed). Skipped defensively.
    if label == MAIN_WINDOW_LABEL {
        return;
    }

    // 1) Model cleanup · window number release — inside one ViewManager lock (the function below). 0 if the close request cleared it first.
    let Some(dropped_views) =
        drop_window_in_model(label, DropCause::Destroyed, state, router, client)
    else {
        return;
    };

    // 2) Remove this label from the output Channel registry (prevents leaks — so that no dead webview Channel remains). It is the Tauri
    //   part, so a separate lock (unrelated to ViewManager) — outside the core (model · routing), so keeping it outside the lock is OK (F1).
    //   ★Remove even if poisoned (ADR-0231)★ — the old skip left dead Channels behind (canonical rationale = the `output_channel`
    //   module header).
    crate::output_channel::unregister_window(registry, label);

    tracing::info!(
        label,
        dropped_views,
        "런타임 창 소멸 정리 완료(모델 몫 · Channel)"
    );
}

/// A pop-out's close request (`CloseRequested` in `lib.rs`) — does the model share of window cleanup ([`drop_window_in_model`] · including the tombstone)
/// now, without waiting for destruction. If the following `Destroyed` cleanup ([`cleanup_popup_window`]) does the same share again, the only thing that changes
/// is releasing that window number. For main it is a no-op (main's close request is hide — the caller's branch).
/// TRD S21-slot-docking edition 5.8 §1-2 「닫히는 창」
pub(crate) fn close_requested_popup_window(
    label: &str,
    state: &LayoutState,
    router: &OutputRouter,
    client: &DaemonClient,
) {
    // 0 = the model did not yet know that window (between window opening and model commit of a new-window move) — only the tombstone stands.
    if let Some(dropped_views) =
        drop_window_in_model(label, DropCause::CloseRequested, state, router, client)
    {
        tracing::info!(
            label,
            dropped_views,
            "팝아웃 닫기 요청 — 모델에서 먼저 지우고 label 에 묘비를 단다"
        );
    }
}

// The model's share of cleaning up a popout window — drop every tab View of the window, remove its windows entry,
// recompute the routing table, send the subscription cleanup, recompute the usage interest and set the label
// tombstone, ★all under one lock★. The close request and `Destroyed` both call it; the second changes nothing but
// freeing the closing window's number (a window already gone gets only the recomputes · the tombstones are a set).
// Answer = the number of tabs removed from the model (0 when the window was already gone) · `None` = not cleaned up
// (main · a poisoned lock). It stands without windows (`AppHandle`), so the shell's unit tests call it directly.
//   ★F1 — a real concurrency bug★: computing the delta under the lock and sending `to_unsubscribe` after dropping it
//   let another command (assign_agent/spawn/move) re-add the agent in between, and the stale 1→0 unsubscribe killed
//   the live subscription just formed (it breaks `output_router::rebuild`'s call contract — that comment is canonical,
//   ★ADR-0006 has no "delta queued under the lock" clause★). So the send is under the lock too, as in the apply
//   service (`SubscriptionSync::resync`) and move_slot_to_window — unsubscribe only sends to the connection driver's
//   actor (no await, no network, no other lock), so there is no deadlock.
// Window number (TRD S26-window-chrome §4-1) — a window the close request removed holds its number until its `Destroyed`, and `Destroyed` releases that
//   number inside the same lock (immediately if it was still in the model). ★Do not move `Destroyed`'s number release out of this lock★
//   — if the model window were removed first under another lock, a window that dropped out with no tombstone would be seen in that gap and the removed tab count would always be 0.
//   Only the extreme case where this cannot be called (`lib.rs` — it has no state) has `layout::release_window_number` release the number.
pub(crate) fn drop_window_in_model(
    label: &str,
    cause: DropCause,
    state: &LayoutState,
    router: &OutputRouter,
    client: &DaemonClient,
) -> Option<usize> {
    // main is not closed (ADR-0057 ④) — if a tombstone stands, docking into main is blocked.
    if label == MAIN_WINDOW_LABEL {
        return None;
    }
    let Ok(mut mgr) = state.0.lock() else {
        tracing::warn!(
            label,
            "팝아웃 창 정리(닫기 요청 · 소멸): lock poisoned — 정리 스킵"
        );
        return None;
    };
    let dropped_views = mgr.windows.get(label).map_or(0, |w| w.tabs.len());
    // If the window is already deleted from the model, only rebuild.
    let delta = crate::output_router::cleanup_window_core(&mut mgr, router, label, cause);
    if cause == DropCause::Destroyed {
        mgr.release_closing(label);
    }
    // The tombstone is inside the same lock as the model deletion — a docking · new-window commit after this lock meets that window as 「없음」 (absent) or a tombstone and changes nothing
    //   (TRD S21-slot-docking edition 5.8 §1-2 「닫히는 창」 (c)).
    mgr.mark_closed(label);
    // An agent for which this window was the last is 1→0 → Unsubscribe (fired inside the lock — F1).
    for a in delta.to_unsubscribe {
        client.unsubscribe(a);
    }
    // ★Do not distinguish by whether the model held this window (TRD S21 usage-limit-slot §1-7 「창 소멸」)★ — the hidden mark
    //   stands separately from the model: moving a slot to a new window opens the OS window first and adds the model window later, so a window hidden and
    //   closed in that gap keeps its mark even though it is not in the model. The label is not reused (`PopupCounter`), so if not erased here it leaks.
    client.usage_window_destroyed(label, &mgr);
    Some(dropped_views)
}

// ── tests: label issuing · prefix decision (window creation itself needs a running app, so GUI verification) ──────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_counter_monotonic_never_reuses() {
        let c = PopupCounter::default();
        let a = c.next_label();
        let b = c.next_label();
        assert_eq!(a, popup_label(1));
        assert_eq!(b, popup_label(2));
        assert_ne!(a, b, "label 재사용 금지 — 단조 증가");
    }

    // TRD S26-window-chrome §4-5 · §10-1 — `slot-popup-<소문자 hex 8>-<n>` (8 lowercase hex digits) · the marker is one per run even with several issuers.
    #[test]
    fn popup_labels_carry_one_run_marker_per_process() {
        let a = PopupCounter::default().next_label();
        let b = PopupCounter::default().next_label();
        assert_eq!(
            a, b,
            "두 발급기의 첫 label 이 같다 — 표식이 프로세스에 하나"
        );
        let (run, n) = a
            .strip_prefix(POPUP_LABEL_PREFIX)
            .and_then(|rest| rest.split_once('-'))
            .expect("접두 · 표식 · 순번");
        assert_eq!(n, "1");
        assert_eq!(run.len(), 8, "{a}");
        assert!(
            run.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "소문자 hex: {a}"
        );
        assert_eq!(run, run_marker());
    }

    // Boot · runtime restore line pop-outs up by (length, characters) order to recover the issue order (`state::placement` · `state::restore`) —
    //   even with the run marker attached, that order is the issue order (including when the digit count grows, 9 → 10).
    #[test]
    fn popup_labels_sorted_by_length_then_text_come_back_in_issue_order() {
        let c = PopupCounter::default();
        let issued: Vec<String> = (0..12).map(|_| c.next_label()).collect();
        let mut sorted = issued.clone();
        sorted.reverse();
        sorted.sort_by_key(|label| (label.len(), label.clone()));
        assert_eq!(sorted, issued);
    }

    #[test]
    fn is_popup_label_matches_prefix_only() {
        assert!(is_popup_label(&popup_label(1)));
        assert!(is_popup_label("slot-popup-1"));
        assert!(is_popup_label("slot-popup-42"));
        assert!(!is_popup_label("main"));
        assert!(!is_popup_label("x-slot-popup-1"));
    }

    #[test]
    fn window_url_uses_window_key() {
        // ★URL key = ?window=<label> (ADR-0057)★ — not the old ?view=<id>.
        assert_eq!(
            window_url("slot-popup-3"),
            "index.html#/popup?window=slot-popup-3"
        );
    }

    #[test]
    fn cascade_position_offsets_by_label_index() {
        assert_eq!(cascade_position(&popup_label(1)), (140.0, 110.0));
        assert_eq!(cascade_position(&popup_label(2)), (212.0, 170.0));
        // The 9th wraps (every 8).
        assert_eq!(cascade_position(&popup_label(9)), (140.0, 110.0));
        // Unfold the trailing sequence number — a run marker made only of digits is not read as a sequence number.
        assert_eq!(cascade_position("slot-popup-12345678-2"), (212.0, 170.0));
        assert_eq!(cascade_position("slot-popup-3"), (284.0, 230.0));
    }

    // ── place of an entrusted prewarmed window (TRD S21-slot-docking §3-9 「`open_at`(맡긴 label)」 · I18) ──

    const SCREEN: placement::MonitorArea = placement::MonitorArea {
        x: 0,
        y: 0,
        w: 1920,
        h: 1080,
        scale: 1.0,
    };

    #[test]
    fn a_claimed_spare_opened_without_a_place_gets_the_cascade_and_the_default_size_again() {
        assert_eq!(
            spare_spot("slot-popup-3", None, &[SCREEN], 1.5),
            ((426, 345), POPOUT_DEFAULT_SIZE),
            "계단 자리(284, 230)를 그 창의 배율로 푼다"
        );
    }

    #[test]
    fn a_claimed_spare_lands_where_a_fresh_window_would() {
        let at = WindowPlace {
            pos: Some((300, 200)),
            size: Some((800.0, 600.0)),
        };
        assert_eq!(
            spare_spot("slot-popup-3", Some(&at), &[SCREEN], 2.0),
            ((300, 200), (800.0, 600.0)),
            "모니터에 걸친 자리는 물리 그대로 — 배율로 풀지 않는다"
        );
    }

    #[test]
    fn a_claimed_spare_whose_place_is_off_every_monitor_takes_the_cascade_and_the_asked_size() {
        let off = WindowPlace {
            pos: Some((-5000, -5000)),
            size: Some((640.0, 480.0)),
        };
        assert_eq!(
            spare_spot("slot-popup-1", Some(&off), &[SCREEN], 1.0),
            ((140, 110), (640.0, 480.0))
        );
        let unusable = WindowPlace {
            pos: None,
            size: Some((0.0, 480.0)),
        };
        assert_eq!(
            spare_spot("slot-popup-1", Some(&unusable), &[SCREEN], 1.0),
            ((140, 110), POPOUT_DEFAULT_SIZE),
            "쓸 수 없는 크기는 기본 크기"
        );
    }
}

// ── Tests: the model's share of a popout close request (TRD S21-slot-docking 5.8판 §1-2 「닫히는 창」) — no windows ──
// The routing table and the subscription resync are production's (`OutputRouter` · `RouterSubs`); the client never
// connects, so an unsubscribe has no open connection to go out on and does nothing (how many go out is measured by the
// `daemon_client` suite). The window-event wiring in `lib.rs` is left to on-screen measurement.
#[cfg(test)]
mod close_request_tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use tokio::runtime::Handle;

    use super::*;
    use crate::layout::dock::{DockAnchor, DockSide, DOCK_NEW_SHARE};
    use crate::layout::manager::{DockError, DockMove, DockTo, FocusPolicy};
    use crate::layout::{
        tree, DockDest, DockRequest, LayoutError, LayoutEvents, SlotContent, SubscriptionSync,
        ViewManager, ViewSnapshot, WindowBounds, WindowPlacement, WindowTabsPayload,
    };

    const POPUP: &str = "slot-popup-7";

    struct NoEvents;

    impl LayoutEvents for NoEvents {
        fn layout_updated(&self, _snapshot: &ViewSnapshot) {}
        fn window_tabs_updated(&self, _tabs: &WindowTabsPayload) {}
    }

    struct Rig {
        state: LayoutState,
        router: Arc<OutputRouter>,
        client: Arc<DaemonClient>,
    }

    impl Rig {
        fn new() -> Self {
            Rig {
                state: LayoutState::new(),
                router: Arc::new(OutputRouter::new()),
                client: Arc::new(DaemonClient::unconnected(Handle::current())),
            }
        }

        fn subs(&self) -> RouterSubs<'_> {
            RouterSubs {
                router: &self.router,
                client: &self.client,
            }
        }

        // The `CloseRequested` pop-out branch of `lib.rs`.
        fn close_requested(&self, label: &str) {
            close_requested_popup_window(label, &self.state, &self.router, &self.client);
        }

        // The model share of the `Destroyed` cleanup of `lib.rs` (the branch where all state is present) — as in production, this one call also releases the window number.
        fn destroyed(&self, label: &str) -> Option<usize> {
            drop_window_in_model(
                label,
                DropCause::Destroyed,
                &self.state,
                &self.router,
                &self.client,
            )
        }

        // The model share of runtime crash copy acceptance (`state::restore` — a copy without pop-outs) — now the pop-out drops out entry and all.
        fn adopt_crash_copy(&self) {
            let mut mgr = self.state.0.lock().unwrap();
            mgr.adopt_restored(ViewManager::new()).unwrap();
            self.subs().resync(&mgr);
        }

        // Add one tab to `window` (if absent, create that window too).
        fn add_tab(&self, window: &str) -> Uuid {
            let mut mgr = self.state.0.lock().unwrap();
            let view = if mgr.windows.contains_key(window) {
                mgr.create_tab(window, None).unwrap()
            } else {
                mgr.create_window(window).unwrap()
            };
            self.subs().resync(&mgr);
            view
        }

        fn main_tab(&self) -> Uuid {
            self.state.0.lock().unwrap().windows[MAIN_WINDOW_LABEL].active
        }

        fn first_slot(&self, view: Uuid) -> Uuid {
            tree::first_slot_id(&self.state.0.lock().unwrap().views[&view].layout)
        }

        // Seat `agent` in the first slot of tab `view` — the routing table is matched too.
        fn seat(&self, view: Uuid, agent: Uuid) {
            let slot = self.first_slot(view);
            let mut mgr = self.state.0.lock().unwrap();
            mgr.assign_agent(view, slot, agent.to_string()).unwrap();
            self.subs().resync(&mgr);
        }

        fn routed(&self, agent: Uuid) -> Vec<String> {
            self.router.targets(agent).to_vec()
        }

        fn fingerprint(&self) -> (u64, Vec<String>, HashMap<Uuid, String>) {
            let mgr = self.state.0.lock().unwrap();
            assert_owned(&mgr);
            let mut windows: Vec<_> = mgr.windows.keys().cloned().collect();
            windows.sort();
            (mgr.version, windows, mgr.view_owner.clone())
        }
    }

    // ADR-0057 ① ② — no windowless View (including a temporary View the rollback could not withdraw) remains · window number invariant (TRD S26-window-chrome §10-1).
    fn assert_owned(mgr: &ViewManager) {
        let tabs: usize = mgr.windows.values().map(|w| w.tabs.len()).sum();
        assert_eq!(mgr.views.len(), tabs, "어느 창의 탭도 아닌 View 가 남았다");
        for (label, w) in &mgr.windows {
            for view in &w.tabs {
                assert_eq!(mgr.view_owner.get(view), Some(label));
            }
        }
        mgr.assert_number_invariants();
    }

    // The gap where pop-out released the lock — right after checking the window (`is_open` → open) or opening it (`open`), the user closes that window.
    //   If `adopts_on_open`, what happens right after opening is not a close but runtime crash copy acceptance.
    struct ClosingHost<'a> {
        rig: &'a Rig,
        adopts_on_open: bool,
        closed: Mutex<Vec<String>>,
        placed: Mutex<Vec<String>>,
        // A prewarmed window to be entrusted once (TRD S21-slot-docking §8-1 ⑬).
        spare: Mutex<Option<SpareClaim>>,
        // The order in which the window port was called — `"claim:"` · `"open:"` · `"committed:"` · `"release:"` · `"close:"` · `"place:"` + label.
        journal: Mutex<Vec<String>>,
        // The claim ticket returned by commit · release.
        returned: Mutex<Vec<SpareClaim>>,
    }

    impl<'a> ClosingHost<'a> {
        fn new(rig: &'a Rig) -> Self {
            ClosingHost {
                rig,
                adopts_on_open: false,
                closed: Mutex::new(Vec::new()),
                placed: Mutex::new(Vec::new()),
                spare: Mutex::new(None),
                journal: Mutex::new(Vec::new()),
                returned: Mutex::new(Vec::new()),
            }
        }

        fn adopting(rig: &'a Rig) -> Self {
            ClosingHost {
                adopts_on_open: true,
                ..ClosingHost::new(rig)
            }
        }

        fn with_spare(rig: &'a Rig, claim: SpareClaim) -> Self {
            ClosingHost {
                spare: Mutex::new(Some(claim)),
                ..ClosingHost::new(rig)
            }
        }

        fn note(&self, entry: String) {
            self.journal.lock().unwrap().push(entry);
        }

        fn user_closes(&self, label: &str) {
            assert!(
                self.rig.state.0.try_lock().is_ok(),
                "창 포트는 락 밖에서 불려야 한다 — 닫기 요청 처리기가 같은 락을 잡는다"
            );
            self.rig.close_requested(label);
        }
    }

    // A new window · a reuse move is the same gap — it is closed right after opening or moving.
    impl WindowHost for ClosingHost<'_> {
        // ★It must be before entering the model★ — all three places that open a new window open the window first and put it in the model (TRD S21-slot-docking
        // §3-9 「자리별 차례」 · ADR-0299).
        fn open(&self, label: &str) -> Result<(), String> {
            let mgr = self
                .rig
                .state
                .0
                .try_lock()
                .expect("창 포트는 락 밖에서 불려야 한다");
            assert!(
                !mgr.windows.contains_key(label),
                "open({label}) 은 창이 모델에 들기 전에 불려야 한다"
            );
            drop(mgr);
            self.note(format!("open:{label}"));
            if self.adopts_on_open {
                self.rig.adopt_crash_copy();
            } else {
                self.user_closes(label);
            }
            Ok(())
        }
        fn open_at(&self, label: &str, _at: &WindowPlace, _activate: bool) -> Result<(), String> {
            self.open(label)
        }
        fn close(&self, label: &str) {
            assert!(
                self.rig.state.0.try_lock().is_ok(),
                "close 는 락 밖에서 불려야 한다 — destroy 의 `Destroyed` 처리기가 같은 락을 잡는다"
            );
            self.note(format!("close:{label}"));
            self.closed.lock().unwrap().push(label.to_string());
        }
        fn is_open(&self, label: &str) -> bool {
            self.user_closes(label);
            true
        }
        fn move_window(&self, label: &str, _at: &WindowPlace) -> Result<(), MoveFailed> {
            self.user_closes(label);
            Ok(())
        }
        fn record_placement(&self, label: &str) {
            self.note(format!("place:{label}"));
            self.placed.lock().unwrap().push(label.to_string());
        }
        fn claim_spare(&self) -> Option<SpareClaim> {
            let claim = self.spare.lock().unwrap().take()?;
            self.note(format!("claim:{}", claim.label));
            Some(claim)
        }
        fn committed(&self, claim: &SpareClaim) {
            self.note(format!("committed:{}", claim.label));
            self.returned.lock().unwrap().push(claim.clone());
        }
        fn release_spare(&self, claim: &SpareClaim) {
            assert!(
                self.rig.state.0.try_lock().is_ok(),
                "release_spare 는 락 밖에서 불려야 한다"
            );
            self.note(format!("release:{}", claim.label));
            self.returned.lock().unwrap().push(claim.clone());
        }
    }

    // The user closes right after an empty new window opened · before it is put in the model (the `Destroyed` cleanup is also the same model share — the tombstone) — the insertion meets the tombstone and
    //   withdraws the opened window, and a pop-out entry without an OS window does not remain in the model · `window.list` (if it remained, the next boot
    //   would restore it as an empty popup). TRD S21-slot-docking §8-1 ⑬ · §3-9 「자리별 차례」
    #[tokio::test]
    async fn a_close_before_the_insert_refuses_a_created_window_and_closes_it() {
        let rig = Rig::new();
        let host = ClosingHost::new(&rig);
        let before = rig.fingerprint();

        let fresh = popup_label(1);
        let fresh = fresh.as_str();
        let err = apply::create_window(
            &rig.state,
            &rig.subs(),
            &NoEvents,
            &host,
            &PopupCounter::default(),
        )
        .unwrap_err();
        assert_eq!(err, LayoutError::WindowClosed(fresh.into()).to_string());
        assert_eq!(*host.closed.lock().unwrap(), [fresh], "방금 연 창을 거둔다");
        assert!(
            host.placed.lock().unwrap().is_empty(),
            "모델에 없는 창의 자리는 적지 않는다"
        );
        assert_eq!(rig.fingerprint(), before, "모델 무변경");
        assert!(!apply::list_windows(&rig.state)
            .unwrap()
            .iter()
            .any(|w| w == fresh));

        // That window's `Destroyed` changes nothing either.
        assert_eq!(
            rig.destroyed(fresh),
            Some(0),
            "모델이 그 창을 가진 적이 없다"
        );
        assert_eq!(rig.fingerprint(), before);
    }

    // The prewarmed-window version of the same gap — if the insertion is rejected by the tombstone, the entrusted window is not closed but handed back with that same claim ticket (no commit · no
    //   place record). TRD S21-slot-docking §8-1 ⑬
    #[tokio::test]
    async fn a_close_before_the_insert_refuses_a_prewarmed_window_and_gives_it_back() {
        let rig = Rig::new();
        let claim = SpareClaim {
            label: "slot-popup-4".into(),
            generation: 3,
        };
        let spare = claim.label.clone();
        let host = ClosingHost::with_spare(&rig, claim.clone());
        let before = rig.fingerprint();

        let err = apply::create_window(
            &rig.state,
            &rig.subs(),
            &NoEvents,
            &host,
            &PopupCounter::default(),
        )
        .unwrap_err();

        assert_eq!(err, LayoutError::WindowClosed(spare.clone()).to_string());
        assert_eq!(
            *host.journal.lock().unwrap(),
            [
                format!("claim:{spare}"),
                format!("open:{spare}"),
                format!("release:{spare}"),
            ],
            "닫기 · 확정 · 자리 기록 없음"
        );
        assert_eq!(
            *host.returned.lock().unwrap(),
            [claim],
            "맡김표 그대로 한 번"
        );
        assert_eq!(rig.fingerprint(), before, "모델 무변경");
        assert!(!apply::list_windows(&rig.state)
            .unwrap()
            .iter()
            .any(|w| *w == spare));
    }

    // Right after an empty new window opened · before it is put in the model, crash copy acceptance swaps the window set — that acceptance does not delete the window the model
    //   does not yet know (there is no tombstone either), so the insertion stands, and the window remains and becomes the model's window (the decided outcome — review 2026-10-21).
    //   TRD S21-slot-docking §3-9 「자리별 차례」
    #[tokio::test]
    async fn a_crash_copy_adopted_during_the_open_keeps_the_created_window_in_the_model() {
        let rig = Rig::new();
        let old_popout = rig.add_tab(POPUP);
        let host = ClosingHost::adopting(&rig);

        let label = apply::create_window(
            &rig.state,
            &rig.subs(),
            &NoEvents,
            &host,
            &PopupCounter::default(),
        )
        .unwrap()
        .window;

        assert_eq!(label, popup_label(1));
        assert!(
            host.closed.lock().unwrap().is_empty(),
            "연 창을 거두지 않는다"
        );
        assert_eq!(*host.placed.lock().unwrap(), [label.as_str()]);
        let mgr = rig.state.0.lock().unwrap();
        assert_owned(&mgr);
        assert!(
            !mgr.windows.contains_key(POPUP),
            "수락이 옛 팝아웃을 지웠다"
        );
        assert!(!mgr.views.contains_key(&old_popout));
        let tabs = mgr.list_tabs(&label).expect("새 창은 모델의 창이다");
        assert_eq!(tabs.tabs.len(), 1, "빈 탭 하나");
        assert_eq!(mgr.owner_of(tabs.active), Some(&label));
    }

    #[tokio::test]
    async fn a_close_request_drops_the_popout_now_and_the_later_destroyed_changes_nothing() {
        let rig = Rig::new();
        let shared = Uuid::new_v4();
        let only_popout = Uuid::new_v4();
        rig.seat(rig.main_tab(), shared);
        let q1 = rig.add_tab(POPUP);
        rig.seat(q1, only_popout);
        let q2 = rig.add_tab(POPUP);
        rig.seat(q2, shared);
        assert_eq!(rig.routed(only_popout), [POPUP]);

        rig.close_requested(POPUP);
        let after = rig.fingerprint();
        {
            let mgr = rig.state.0.lock().unwrap();
            assert!(!mgr.windows.contains_key(POPUP));
            for q in [q1, q2] {
                assert!(!mgr.views.contains_key(&q), "그 창의 탭이 전부 빠진다");
            }
        }
        assert!(
            rig.routed(only_popout).is_empty(),
            "그 창에만 보이던 에이전트는 라우팅에서 빠진다"
        );
        assert_eq!(rig.routed(shared), [MAIN_WINDOW_LABEL]);

        // The following `Destroyed` — doing the same share again changes nothing.
        assert_eq!(rig.destroyed(POPUP), Some(0), "닫기 요청이 이미 지웠다");
        assert_eq!(rig.fingerprint(), after);
        assert!(rig.routed(only_popout).is_empty());
        assert_eq!(rig.routed(shared), [MAIN_WINDOW_LABEL]);
    }

    // TRD S26-window-chrome §4-1 — a close request holds the number until that window's `Destroyed` and `Destroyed` releases it. A `Destroyed` that came without a close request
    //   (a window left in the model) releases it immediately.
    #[tokio::test]
    async fn a_close_request_holds_the_number_until_destroyed_frees_it() {
        let rig = Rig::new();
        rig.add_tab("slot-popup-1");
        rig.add_tab("slot-popup-2");
        let closing = || rig.state.0.lock().unwrap().closing_numbers();

        rig.close_requested("slot-popup-1");
        assert_eq!(closing(), [2]);
        rig.fingerprint();
        assert_eq!(rig.destroyed("slot-popup-1"), Some(0));
        assert!(closing().is_empty());

        assert_eq!(rig.destroyed("slot-popup-2"), Some(1));
        assert!(closing().is_empty());
        rig.add_tab("slot-popup-3");
        assert_eq!(
            rig.state.0.lock().unwrap().windows["slot-popup-3"].number,
            2
        );
        rig.fingerprint();
    }

    #[tokio::test]
    async fn the_model_part_answers_how_many_tabs_it_dropped() {
        let rig = Rig::new();
        rig.add_tab(POPUP);
        rig.add_tab(POPUP);
        assert_eq!(rig.destroyed(POPUP), Some(2));
        assert_eq!(rig.destroyed(POPUP), Some(0), "두 번째는 지울 것이 없다");
        // Between window opening and model commit of a new-window move — a tombstone stands even for a window the model does not yet know.
        let fresh = "slot-popup-99";
        assert_eq!(rig.destroyed(fresh), Some(0));
        let main = rig.main_tab();
        let slot = rig.first_slot(main);
        let mut mgr = rig.state.0.lock().unwrap();
        let (tmp, _) = mgr.prepare_detached_view(main, slot).unwrap();
        assert_eq!(
            mgr.attach_view_as_new_window(fresh, tmp),
            Err(LayoutError::WindowClosed(fresh.into()))
        );
    }

    #[tokio::test]
    async fn after_a_close_request_docking_into_that_window_is_refused_unchanged() {
        let rig = Rig::new();
        let source = rig.add_tab(MAIN_WINDOW_LABEL);
        let q = rig.add_tab(POPUP);
        rig.close_requested(POPUP);

        let before = rig.fingerprint();
        let mut mgr = rig.state.0.lock().unwrap();
        let version = mgr.version;
        for (to, want) in [
            (
                DockTo::Seam {
                    target: q,
                    anchor: DockAnchor::Outer,
                    side: DockSide::Left,
                    share: DOCK_NEW_SHARE,
                },
                DockError::TabNotFound(q),
            ),
            (
                DockTo::TabBar {
                    window: POPUP.into(),
                    index: None,
                    after: None,
                },
                DockError::WindowNotFound(POPUP.into()),
            ),
            (
                DockTo::NewWindow {
                    label: POPUP.into(),
                },
                DockError::WindowClosed(POPUP.into()),
            ),
        ] {
            assert_eq!(
                mgr.dock(source, to, FocusPolicy::Reveal, DockMove::default()),
                Err(want)
            );
            assert_eq!(mgr.version, version);
        }
        // The new-window phase C of a pop-out (`slot.popout`) also stops at the tombstone.
        let slot = tree::first_slot_id(&mgr.views[&source].layout);
        let (tmp, _) = mgr.prepare_detached_view(source, slot).unwrap();
        assert_eq!(
            mgr.attach_view_as_new_window(POPUP, tmp),
            Err(LayoutError::WindowClosed(POPUP.into()))
        );
        mgr.drop_detached_view(tmp);
        drop(mgr);
        // `version` rises by as much as the temporary Views made and withdrawn — only window · tab ownership is looked at.
        let after = rig.fingerprint();
        assert_eq!((after.1, after.2), (before.1, before.2));
    }

    #[tokio::test]
    async fn a_close_request_after_the_check_rolls_a_popout_into_that_window_back() {
        let rig = Rig::new();
        let agent = Uuid::new_v4();
        let src = rig.main_tab();
        rig.seat(src, agent);
        let slot = rig.first_slot(src);
        rig.add_tab(POPUP);
        let host = ClosingHost::new(&rig);

        let out = apply::move_slot_to_window(
            &rig.state,
            &rig.subs(),
            &NoEvents,
            &host,
            &PopupCounter::default(),
            src,
            slot,
            Some(POPUP.into()),
        );
        assert!(out.is_err(), "{out:?}");
        {
            let mgr = rig.state.0.lock().unwrap();
            assert_owned(&mgr);
            assert!(!mgr.windows.contains_key(POPUP), "닫힌 창이 모델에 남았다");
            assert_eq!(
                mgr.slot_content(src, slot),
                Ok(SlotContent::Agent {
                    agent_id: agent.to_string()
                }),
                "옮기려던 슬롯은 출처에 남는다"
            );
        }
        assert!(
            host.closed.lock().unwrap().is_empty(),
            "기존 창 타깃은 창을 닫지 않는다"
        );
        assert_eq!(rig.routed(agent), [MAIN_WINDOW_LABEL]);
    }

    #[tokio::test]
    async fn a_close_request_after_the_build_rolls_a_popout_into_a_new_window_back() {
        let rig = Rig::new();
        let agent = Uuid::new_v4();
        let src = rig.main_tab();
        rig.seat(src, agent);
        let slot = rig.first_slot(src);
        let labels = PopupCounter::default();
        let fresh = popup_label(1);
        let fresh = fresh.as_str();
        let host = ClosingHost::new(&rig);

        let out = apply::move_slot_to_window(
            &rig.state,
            &rig.subs(),
            &NoEvents,
            &host,
            &labels,
            src,
            slot,
            None,
        );
        assert!(out.is_err(), "{out:?}");
        let after = rig.fingerprint();
        assert!(
            !after.1.iter().any(|w| w == fresh),
            "OS 창 없는 모델 창이 남았다"
        );
        assert_eq!(
            rig.state.0.lock().unwrap().slot_content(src, slot),
            Ok(SlotContent::Agent {
                agent_id: agent.to_string()
            }),
            "옮기려던 슬롯은 출처에 남는다"
        );
        assert_eq!(*host.closed.lock().unwrap(), [fresh], "방금 연 창을 거둔다");
        assert!(
            host.placed.lock().unwrap().is_empty(),
            "모델에 없는 창의 자리는 적지 않는다"
        );
        assert_eq!(rig.routed(agent), [MAIN_WINDOW_LABEL]);

        // That window's `Destroyed` changes nothing either.
        assert_eq!(
            rig.destroyed(fresh),
            Some(0),
            "모델이 그 창을 가진 적이 없다"
        );
        assert_eq!(rig.fingerprint(), after);
    }

    #[tokio::test]
    async fn a_close_request_for_main_changes_nothing_and_main_still_takes_tabs() {
        let rig = Rig::new();
        let q = rig.add_tab(POPUP);
        rig.add_tab(POPUP);
        let before = rig.fingerprint();

        rig.close_requested(MAIN_WINDOW_LABEL);
        assert_eq!(
            rig.destroyed(MAIN_WINDOW_LABEL),
            None,
            "main 은 정리하지 않는다"
        );
        assert_eq!(rig.fingerprint(), before);
        // No tombstone stood on main.
        rig.state
            .0
            .lock()
            .unwrap()
            .dock(
                q,
                DockTo::TabBar {
                    window: MAIN_WINDOW_LABEL.into(),
                    index: None,
                    after: None,
                },
                FocusPolicy::Keep,
                DockMove::default(),
            )
            .unwrap();
    }

    fn dock_to_new_window(rig: &Rig, host: &ClosingHost, view: Uuid) -> Result<(), DockError> {
        apply::dock_tab(
            &rig.state,
            &rig.subs(),
            &NoEvents,
            host,
            &PopupCounter::default(),
            DockRequest {
                view,
                to: DockDest::NewWindow {
                    at: WindowPlace {
                        pos: Some((300, 200)),
                        size: None,
                    },
                },
                focus: FocusPolicy::Reveal,
                mv: DockMove::default(),
            },
        )
        .map(drop)
    }

    // The user closes right after a docking new window opened — phase C meets the tombstone and withdraws the opened window, and the tab stays at its source.
    #[tokio::test]
    async fn a_close_request_before_the_attach_closes_the_new_dock_window_and_keeps_the_tab() {
        let rig = Rig::new();
        let agent = Uuid::new_v4();
        let src = rig.add_tab(MAIN_WINDOW_LABEL);
        rig.seat(src, agent);
        let host = ClosingHost::new(&rig);
        let before = rig.fingerprint();

        let fresh = popup_label(1);
        let fresh = fresh.as_str();
        assert_eq!(
            dock_to_new_window(&rig, &host, src),
            Err(DockError::WindowClosed(fresh.into()))
        );
        assert_eq!(*host.closed.lock().unwrap(), [fresh], "방금 연 창을 거둔다");
        assert!(
            host.placed.lock().unwrap().is_empty(),
            "모델에 없는 창의 자리는 적지 않는다"
        );
        assert_eq!(rig.fingerprint(), before, "탭은 출처에");
        assert_eq!(rig.routed(agent), [MAIN_WINDOW_LABEL]);
    }

    // Right after moving by reuse, the user closes that window — the drop finishes as is (it is not decided again) and the model is exactly as the close left it. There is no opened window
    // so there is nothing to close.
    #[tokio::test]
    async fn a_close_request_during_a_reuse_move_lets_the_drop_finish() {
        let rig = Rig::new();
        let q = rig.add_tab(POPUP);
        rig.state
            .0
            .lock()
            .unwrap()
            .observe_window_placement(
                POPUP,
                WindowPlacement {
                    minimized: false,
                    maximized: false,
                    bounds: WindowBounds::new(10.0, 20.0, 640.0, 480.0),
                },
            )
            .unwrap();
        let host = ClosingHost::new(&rig);

        assert_eq!(dock_to_new_window(&rig, &host, q), Ok(()));
        assert!(host.closed.lock().unwrap().is_empty());
        let mgr = rig.state.0.lock().unwrap();
        assert_owned(&mgr);
        assert!(!mgr.windows.contains_key(POPUP), "닫기 요청이 지웠다");
    }
}
