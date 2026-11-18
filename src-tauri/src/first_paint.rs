//! Hide a window until its first paint: main and pop-outs are built hidden ([`hold_window`]) and shown when the page reports it drew the real first screen
//! (`webview_painted` · `src/theme/firstPaint.ts`), so the window appears together with its content.
//!
//! - The webview itself is not hidden (`Webview::hide`): WebView2 once showed a not-yet-drawn surface after a hidden webview was shown again.
//! - Building hidden is also needed because tao 0.35.3 shows a visible window at the end of creation, before the `WM_ERASEBKGND` background subclass is attached, so the first frame was unpainted. A show after building erases with the background color.
//! - A window is shown once per hold: whichever comes first of the notification and [`FALLBACK_REVEAL`]. A notification for a window with no hold (main of a `--hidden` boot, a prewarmed window before entrusting, a reload of a shown window) does nothing.
//! - Whether to show is decided on the main thread right before showing: notification, deadline and restore carry only the hold's ticket, and the handed-over job shows only if that ticket's hold still remains ([`FirstPaintGate::release`]). Otherwise a hide between hand-over and run (end of `--hidden` boot setup, tray hide) would be followed by a show of a hidden window.
//! - Three ways of showing: held-and-built and restore windows use [`crate::state::placement::show_placed`] (activated; maximized without a normal-size frame if the wish is true); a window newly built without focus (I30) and an entrusted prewarmed window are shown after raising only z while hidden (only the entrusted one activates — [`new_window_show`]). After every show the recorded maximize wish is applied (`state::placement::apply_deferred_maximize`); a missing path leaves the wish and stops place recording for that window.
//! - [`is_held`]: a window-state command checks it; a held window is not revealed by the command (this gate shows it).
//! - Two-stage holds: a runtime-restore pop-out ([`hold_window_unarmed`] then [`reveal_held`]) and an entrusted prewarmed window ([`SparePaintGate`] then [`reveal_if_held`]) first only keep the notification. The deadline starts when decided to show, so a window that may still be reverted is not revealed early.
//! - The entrusted window accepts only a notification carrying its claim's adoption number (I26): an unnumbered or different number comes from the pre-adoption frame (waiting page, `bootFailed`) and would show it at once. Other paths accept notifications with or without a number. When an entrusted window is shown, the pool's rebuild completion is called (`SpareState::on_shown`).
//! - The registry lock is a leaf: no logging, window call or other lock under it. Lock order: layout → pool → first-paint gate (`crate::spare::SpareState`).
//! - Hiding withdraws the hold ([`cancel_hold`]): tray hide (`tray::actions::hide_main_ui`, ADR-0229), restore keep-hidden, tray show and `Destroyed` all call it. Otherwise a later notification or deadline re-shows the window (a `--hidden` boot would show a pop-out while main stays in the tray).
//! - [unverified — GUI] Whether WebView2 stops drawing a page while its parent window is hidden; if so the background color shows briefly after the show.
// ADR-0299 (TRD S26-window-chrome §5-2; TRD S21-slot-docking §3-9)

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use engram_dashboard_base::sync;
use tauri::{AppHandle, Manager, WebviewWindow};

/// The upper bound on waiting for the first-paint notification; if exceeded, the window is shown without it. The value is a judgment ("treat as broken"), not a measurement (a dev-server launch takes 0.8~1.1 s).
// ADR-0305 — failure before first paint
pub(crate) const FALLBACK_REVEAL: Duration = Duration::from_millis(1500);

/// The registry of hidden windows waiting to be shown (label → held state), one per shell, managed in the builder before any window is created. Showing is done on the main thread by the side that takes out that ticket's entry (module head).
#[derive(Default)]
pub(crate) struct FirstPaintGate {
    held: Mutex<HashMap<String, Hold>>,
    next: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hold {
    /// Shown at the notification or the deadline. `ticket` is drawn fresh at every hold (monotonic across the registry); a handed-over show takes the entry out only with its own ticket, so an old job cannot show a window that was withdrawn and held again. `since` / `opened` feed the logs (`held_ms`, `reveal_ms`).
    Armed {
        ticket: u64,
        reveal: Reveal,
        since: Instant,
        opened: Option<Instant>,
    },
    /// Only keeps the notification — whether to show is decided by [`FirstPaintGate::arm`] (restore) · [`FirstPaintGate::arm_adopted`] (commit ③).
    Unarmed { painted: Painted },
}

/// The notification a held entry accepts and how it is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reveal {
    /// A built-and-held window · a restore window — accepts any notification without looking at the number. Activated when shown.
    Placed,
    /// A window the window port newly built without focus (I30) — does not look at the number. Raises only z while hidden and is shown without activation.
    Raised,
    /// An entrusted prewarmed window (commit ③) — accepts only a notification that carries adoption number `generation` (I26). `activate` = that claim
    /// was opened as active (I5).
    Adopted { generation: u32, activate: bool },
}

/// A notification that came to a keeping entry. The restore path ([`FirstPaintGate::arm`]) treats `Numberless` and `Generation` alike; commit ③ ([`FirstPaintGate::arm_adopted`]) only the matching `Generation`. One true/false would either lose the restore window's notification or show the pre-adoption frame at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Painted {
    Not,
    Numberless,
    Generation(u32),
}

/// The answer of [`FirstPaintGate::arm`] · [`FirstPaintGate::arm_adopted`] — both put that window under hold with this ticket.
#[derive(Debug, PartialEq, Eq)]
enum Arm {
    /// The notification has already come — hand over the show now.
    Painted(u64),
    /// Wait for the notification or the deadline.
    Waiting(u64),
}

/// The answer of [`FirstPaintGate::painted`].
#[derive(Debug, PartialEq, Eq)]
enum Signal {
    /// A held entry received it — hand over the show with this ticket.
    Release(u64),
    /// Recorded in the keeping entry.
    Recorded,
    /// The entrusted prewarmed window's entry discarded a notification with a different (absent) number (I26).
    Mismatch { expected: u32, got: Option<u32> },
    /// There is no entry for that label.
    Ignored,
}

/// The entry taken out by [`FirstPaintGate::release`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Released {
    reveal: Reveal,
    since: Instant,
    opened: Option<Instant>,
}

impl FirstPaintGate {
    fn ticket(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    fn hold(&self, label: &str) -> u64 {
        self.hold_as(label, Reveal::Placed, None)
    }

    fn hold_as(&self, label: &str, reveal: Reveal, opened: Option<Instant>) -> u64 {
        let ticket = self.ticket();
        let since = Instant::now();
        sync::lock(&self.held).insert(
            label.to_owned(),
            Hold::Armed {
                ticket,
                reveal,
                since,
                opened,
            },
        );
        ticket
    }

    // Keep an existing hold: overwriting would lose the notification already kept, so `arm` · `arm_adopted` could not show immediately (re-arming at tray show — `spare::SpareState::on_tray_show`).
    fn hold_unarmed(&self, label: &str) {
        sync::lock(&self.held)
            .entry(label.to_owned())
            .or_insert(Hold::Unarmed {
                painted: Painted::Not,
            });
    }

    // A window not in the registry (including a withdrawn window) is also waited for from now — a notification that came before that is not kept, so it is shown at the deadline.
    fn arm(&self, label: &str) -> Arm {
        let ticket = self.ticket();
        let since = Instant::now();
        let mut held = sync::lock(&self.held);
        let painted = matches!(
            held.get(label),
            Some(Hold::Unarmed { painted }) if *painted != Painted::Not
        );
        held.insert(
            label.to_owned(),
            Hold::Armed {
                ticket,
                reveal: Reveal::Placed,
                since,
                opened: None,
            },
        );
        if painted {
            Arm::Painted(ticket)
        } else {
            Arm::Waiting(ticket)
        }
    }

    // If there is no entry (withdrawn), do nothing — if it waited from now like `arm`, a UI hide would revive the withdrawn window.
    fn arm_adopted(
        &self,
        label: &str,
        generation: u32,
        activate: bool,
        opened: Option<Instant>,
    ) -> Option<Arm> {
        let ticket = self.ticket();
        let since = Instant::now();
        let mut held = sync::lock(&self.held);
        let painted = match held.get(label)? {
            Hold::Unarmed {
                painted: Painted::Generation(got),
            } => *got == generation,
            Hold::Unarmed { .. } | Hold::Armed { .. } => false,
        };
        held.insert(
            label.to_owned(),
            Hold::Armed {
                ticket,
                reveal: Reveal::Adopted {
                    generation,
                    activate,
                },
                since,
                opened,
            },
        );
        Some(if painted {
            Arm::Painted(ticket)
        } else {
            Arm::Waiting(ticket)
        })
    }

    fn painted(&self, label: &str, generation: Option<u32>) -> Signal {
        let mut held = sync::lock(&self.held);
        let Some(hold) = held.get_mut(label) else {
            return Signal::Ignored;
        };
        match hold {
            Hold::Armed {
                ticket,
                reveal: Reveal::Placed | Reveal::Raised,
                ..
            } => Signal::Release(*ticket),
            Hold::Armed {
                ticket,
                reveal:
                    Reveal::Adopted {
                        generation: expected,
                        ..
                    },
                ..
            } => {
                if generation == Some(*expected) {
                    Signal::Release(*ticket)
                } else {
                    Signal::Mismatch {
                        expected: *expected,
                        got: generation,
                    }
                }
            }
            Hold::Unarmed { painted } => {
                // A numberless notification must not erase a kept number, or ③ could not show immediately.
                match generation {
                    Some(got) => *painted = Painted::Generation(got),
                    None if *painted == Painted::Not => *painted = Painted::Numberless,
                    None => {}
                }
                Signal::Recorded
            }
        }
    }

    // A taken-out ticket never comes again (tickets are monotonic), so "absent" stays true after hand-over; "present" is re-decided by the handed-over job ([`Self::release`]).
    fn is_waiting(&self, label: &str, ticket: u64) -> bool {
        matches!(
            sync::lock(&self.held).get(label),
            Some(Hold::Armed { ticket: t, .. }) if *t == ticket
        )
    }

    /// Decided when the handed-over show runs on the main thread — if that window remains held with this ticket, take it out and return it (it is shown).
    /// `None` = already shown or withdrawn or held again.
    fn release(&self, label: &str, ticket: u64) -> Option<Released> {
        let mut held = sync::lock(&self.held);
        match held.get(label).copied() {
            Some(Hold::Armed {
                ticket: t,
                reveal,
                since,
                opened,
            }) if t == ticket => {
                held.remove(label);
                Some(Released {
                    reveal,
                    since,
                    opened,
                })
            }
            _ => None,
        }
    }

    fn cancel(&self, label: &str) -> bool {
        sync::lock(&self.held).remove(label).is_some()
    }

    fn is_held(&self, label: &str) -> bool {
        sync::lock(&self.held).contains_key(label)
    }
}

/// The first-paint gate handle the prewarmed window pool holds (`crate::spare::PaintGate` — I17 · I26). It is made in the builder before the pool, and the app handle
/// to which commit ③'s show is handed over is plugged in by setup with [`Self::attach`] — before that there is no window, so nothing to entrust.
pub(crate) struct SparePaintGate {
    gate: Arc<FirstPaintGate>,
    app: OnceLock<AppHandle>,
}

impl SparePaintGate {
    pub(crate) fn new(gate: Arc<FirstPaintGate>) -> Self {
        Self {
            gate,
            app: OnceLock::new(),
        }
    }

    /// Plug in the app handle — listened to only once (from the second on, ignored).
    pub(crate) fn attach(&self, app: AppHandle) {
        let _ = self.app.set(app);
    }
}

// `hold_unarmed` · `cancel` are called inside the pool lock — they take only the registry's leaf lock (module head).
impl crate::spare::PaintGate for SparePaintGate {
    fn hold_unarmed(&self, label: &str) {
        self.gate.hold_unarmed(label);
    }

    fn cancel(&self, label: &str) {
        self.gate.cancel(label);
    }

    fn reveal_if_held(
        &self,
        label: &str,
        generation: u32,
        activate: bool,
        opened: Option<Instant>,
    ) -> bool {
        if let Some(app) = self.app.get() {
            return reveal_if_held(app, &self.gate, label, generation, activate, opened);
        }
        // Treated the same as 「was withdrawn」 — it becomes a model window while hidden and the next tray show shows it.
        self.gate.cancel(label);
        tracing::warn!(
            module = "first_paint",
            label,
            generation,
            "confirmed before the app handle was plugged in; the handed-over window is not raised for first paint"
        );
        false
    }
}

fn gate(app: &AppHandle) -> Option<Arc<FirstPaintGate>> {
    app.try_state::<Arc<FirstPaintGate>>()
        .map(|gate| gate.inner().clone())
}

/// Put a window built hidden under hold so it is shown at the first-paint notification (or [`FALLBACK_REVEAL`]), activated. ★The window must be hidden with its place and deferred maximize already applied★. If the registry is not registered it is shown right away.
pub(crate) fn hold_window(window: &WebviewWindow) {
    let Some(gate) = gate(window.app_handle()) else {
        tracing::warn!(
            module = "first_paint",
            label = window.label(),
            "first-paint gate is not registered; showing the window right away"
        );
        show_as(window, Reveal::Placed);
        return;
    };
    let ticket = gate.hold(window.label());
    reveal_when_overdue_on_main_thread(window.app_handle(), gate, window.label(), ticket);
}

/// The window-port edition of [`hold_window`]. `focused` = built receiving focus (activated when shown); `false` = built without focus (raises only z while hidden, shown without activation — I30). `opened` = when the window port began opening it (log `reveal_ms`).
// ADR-0299
pub(crate) fn hold_built(window: &WebviewWindow, focused: bool, opened: Instant) {
    let reveal = if focused {
        Reveal::Placed
    } else {
        Reveal::Raised
    };
    let Some(gate) = gate(window.app_handle()) else {
        tracing::warn!(
            module = "first_paint",
            label = window.label(),
            "first-paint gate is not registered; showing the window right away"
        );
        show_as(window, reveal);
        return;
    };
    let ticket = gate.hold_as(window.label(), reveal, Some(opened));
    reveal_when_overdue_on_main_thread(window.app_handle(), gate, window.label(), ticket);
}

/// Only keeps the first-paint notification of a window built hidden; [`reveal_held`] decides to show, [`cancel_hold`] keeps it hidden. ★An existing hold, armed or not, is left as is★ (a kept notification is not erased). Does nothing if the registry is not registered ([`reveal_held`] then shows it immediately).
pub(crate) fn hold_window_unarmed(window: &WebviewWindow) {
    if let Some(gate) = gate(window.app_handle()) {
        gate.hold_unarmed(window.label());
    }
}

/// Decide to show that window at first paint: immediately if the notification already came, otherwise at the notification or [`FALLBACK_REVEAL`] from now; activated. A window not kept (including a withdrawn one) is also waited for from now. Callable from any thread.
pub(crate) fn reveal_held(app: &AppHandle, label: &str) {
    let Some(gate) = gate(app) else {
        tracing::warn!(
            module = "first_paint",
            label,
            "first-paint gate is not registered; showing the window right away"
        );
        reveal_on_main_thread(app, label, |_| Some(Reveal::Placed));
        return;
    };
    match gate.arm(label) {
        Arm::Painted(ticket) => {
            release_on_main_thread(app, gate, label, ticket, Cause::PaintedBeforeArming)
        }
        Arm::Waiting(ticket) => reveal_when_overdue_on_main_thread(app, gate, label, ticket),
    }
}

// Commit ③ — `true` = the remaining hold was put up. Callable from any thread. (TRD S21-slot-docking §3-9)
fn reveal_if_held(
    app: &AppHandle,
    gate: &Arc<FirstPaintGate>,
    label: &str,
    generation: u32,
    activate: bool,
    opened: Option<Instant>,
) -> bool {
    match gate.arm_adopted(label, generation, activate, opened) {
        None => false,
        Some(Arm::Painted(ticket)) => {
            release_on_main_thread(app, gate.clone(), label, ticket, Cause::PaintedBeforeArming);
            true
        }
        Some(Arm::Waiting(ticket)) => {
            reveal_when_overdue_on_main_thread(app, gate.clone(), label, ticket);
            true
        }
    }
}

/// Withdraw that window's hold: a later notification or deadline, and an already handed-over show, will not show it. No-op if never held. Hide and tray-show paths call it per window (module head).
pub(crate) fn cancel_hold(app: &AppHandle, label: &str) {
    if gate(app).is_some_and(|gate| gate.cancel(label)) {
        tracing::debug!(
            module = "first_paint",
            label,
            "withdrawing the raise of the window waiting for first paint — notification and deadline will not show that window"
        );
    }
}

/// Whether that window is held at this gate (decided to show, or only keeping the notification); false if the registry is not registered. A momentary value: read off the main thread, this gate's show can pass between read and write (a window-state command reads inside a single main-thread job). (TRD S26-window-chrome §5-2)
pub(crate) fn is_held(app: &AppHandle, label: &str) -> bool {
    gate(app).is_some_and(|gate| gate.is_held(label))
}

/// That window's first-paint notification (`generation` = the adoption number it carries): shown if held, kept if the entry only keeps, otherwise nothing. An entrusted prewarmed window accepts only its own adoption number.
pub(crate) fn reveal_painted(window: &WebviewWindow, generation: Option<u32>) {
    let app = window.app_handle();
    let Some(gate) = gate(app) else {
        return;
    };
    let label = window.label();
    match gate.painted(label, generation) {
        Signal::Release(ticket) => release_on_main_thread(app, gate, label, ticket, Cause::Painted),
        Signal::Mismatch { expected, got } => tracing::debug!(
            module = "first_paint",
            label,
            expected,
            got = ?got,
            "discarding a first-paint notification with a different adoption number — it came from the pre-adoption frame"
        ),
        Signal::Recorded | Signal::Ignored => {}
    }
}

/// What decided the handed-over show — the log's share.
#[derive(Debug, Clone, Copy)]
enum Cause {
    Painted,
    PaintedBeforeArming,
    Overdue,
}

impl Cause {
    // `via` · `reveal_ms` compare the open → visible delay of a prewarmed window and a fresh build (M14).
    // ADR-0299 (TRD S21-slot-docking §3-9)
    fn log(self, label: &str, released: &Released) {
        let held_ms = crate::spare::elapsed_ms(released.since);
        let reveal_ms = released.opened.map(crate::spare::elapsed_ms);
        let via = match released.reveal {
            Reveal::Adopted { .. } => "spare",
            Reveal::Placed | Reveal::Raised => "fresh",
        };
        match self {
            Self::Painted => tracing::info!(
                module = "first_paint",
                label,
                cause = "painted",
                via,
                held_ms,
                reveal_ms = ?reveal_ms,
                "showing the window on the first-paint notification"
            ),
            Self::PaintedBeforeArming => tracing::info!(
                module = "first_paint",
                label,
                cause = "painted_before_arming",
                via,
                held_ms,
                reveal_ms = ?reveal_ms,
                "the first-paint notification arrived earlier; showing the window right away"
            ),
            Self::Overdue => tracing::warn!(
                module = "first_paint",
                label,
                cause = "overdue",
                via,
                held_ms,
                reveal_ms = ?reveal_ms,
                "no first-paint notification within the deadline; showing the window"
            ),
        }
    }
}

fn reveal_when_overdue_on_main_thread(
    app: &AppHandle,
    gate: Arc<FirstPaintGate>,
    label: &str,
    ticket: u64,
) {
    let app = app.clone();
    let label = label.to_owned();
    tauri::async_runtime::spawn(async move {
        let reveal = {
            let gate = gate.clone();
            let label = label.clone();
            move || release_on_main_thread(&app, gate, &label, ticket, Cause::Overdue)
        };
        queue_when_overdue(&gate, &label, ticket, FALLBACK_REVEAL, reveal).await;
    });
}

async fn queue_when_overdue(
    gate: &FirstPaintGate,
    label: &str,
    ticket: u64,
    after: Duration,
    queue: impl FnOnce(),
) {
    tokio::time::sleep(after).await;
    if gate.is_waiting(label, ticket) {
        queue();
    }
}

fn release_on_main_thread(
    app: &AppHandle,
    gate: Arc<FirstPaintGate>,
    label: &str,
    ticket: u64,
    cause: Cause,
) {
    reveal_on_main_thread(app, label, move |label| {
        let released = gate.release(label, ticket);
        match &released {
            Some(released) => cause.log(label, released),
            None => tracing::debug!(
                module = "first_paint",
                label,
                ?cause,
                "before the handed-over show ran, that raise was shown or withdrawn, so nothing is shown"
            ),
        }
        released.map(|released| released.reveal)
    });
}

// `decide` runs on the main thread right before showing (module head) — `None` = not shown.
fn reveal_on_main_thread(
    app: &AppHandle,
    label: &str,
    decide: impl FnOnce(&str) -> Option<Reveal> + Send + 'static,
) {
    let handle = app.clone();
    let owned = label.to_owned();
    let queued = app.run_on_main_thread(move || {
        let Some(reveal) = decide(&owned) else {
            return;
        };
        match handle.get_webview_window(&owned) {
            Some(window) => show_as(&window, reveal),
            None => {
                tracing::debug!(module = "first_paint", label = %owned, "the window closed before it was shown")
            }
        }
    });
    if let Err(e) = queued {
        tracing::warn!(
            module = "first_paint",
            label,
            "failed to hand the show-hidden-window job to the main thread: {e}"
        );
    }
}

// Called on the main thread (module head).
fn show_as(window: &WebviewWindow, reveal: Reveal) {
    match reveal {
        Reveal::Placed => crate::state::placement::show_placed(&window.as_ref().window()),
        Reveal::Raised => raise_and_show(window, false),
        Reveal::Adopted { activate, .. } => show_adopted(window, activate),
    }
}

// Show of an entrusted window: the foreground decision is right before showing, and rebuild completion is called after it (even if it could not be shown); notification and deadline both come here.
// ADR-0299 (TRD S21-slot-docking §3-9)
fn show_adopted(window: &WebviewWindow, activate: bool) {
    let app = window.app_handle();
    let show = new_window_show(activate, crate::state::placement::foreground_is_ours(app));
    raise_and_show(window, show == NewWindowShow::RaiseAndActivate);
    crate::commands::spare::on_adopted_window_shown(app, window.label());
}

// ⓐ raise only z to the top while hidden → ⓑ show and apply the recorded maximize wish → ⓒ activate (only for `activate`). ⓐ comes before ⓑ so a long-hidden window is not seen even for one frame at its old z place (I29).
// ★ⓑ is Tauri `show()`★: shown with raw Win32 the tao visibility flag stays false and tray hide does nothing (see the prewarmed-window building head of `commands::spare`). tao shows a window built without focus without activating it.
// [unmeasured — M22] whether ⓐ alone puts it above another app's foreground window, and whether the z raised while hidden survives ⓑ. [unmeasured] whether applying a maximize wish (`SW_MAXIMIZE` · `SW_RESTORE` double as activation) moves the foreground for a window built without focus.
// ADR-0299 (TRD S26-window-chrome §10-5)
fn raise_and_show(window: &WebviewWindow, activate: bool) {
    let label = window.label();
    let os = window.as_ref().window();
    raise_without_activation(&os);
    if let Err(e) = window.show() {
        tracing::warn!(module = "first_paint", label, error = %e, "could not show the hidden window");
        return;
    }
    // TRD S26-window-chrome §5-2
    crate::state::placement::apply_deferred_maximize(&os);
    if activate {
        if let Err(e) = window.set_focus() {
            tracing::debug!(module = "first_paint", label, error = %e, "could not focus the shown window");
        }
    }
}

/// Raise only that window's z to the top of its band without activation; a hidden window stays hidden (I29). Failure is only logged. ★Main thread only★.
// ADR-0299
pub(crate) fn raise_without_activation(window: &tauri::Window) {
    let label = window.label();
    if let Err(e) = engram_dashboard_platform::window::place_window(
        window,
        None,
        engram_dashboard_platform::window::ZOrder::Top,
        false,
        false,
    ) {
        // Whether the raise ran is M22 diagnostic material — only the absence of means on this OS is left quiet.
        match e {
            engram_dashboard_platform::window::WindowError::Unsupported => {
                tracing::debug!(module = "first_paint", label, error = %e, "could not raise the window's z-order; moving on");
            }
            engram_dashboard_platform::window::WindowError::NoWindow
            | engram_dashboard_platform::window::WindowError::Os(_) => {
                tracing::warn!(module = "first_paint", label, error = %e, "could not raise the window's z-order; moving on");
            }
        }
    }
}

/// Show of a new window (an entrusted prewarmed window or a window newly built by the window port): both are shown after raising z while hidden (I29).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NewWindowShow {
    /// Leaves focus as it is.
    Raise,
    /// Activates when shown — an entrusted window is given focus after being shown, and a newly built window is built while receiving focus.
    RaiseAndActivate,
}

/// Activation only when the request was opened as active (`activate`) and the foreground is now our process (I5 · I30): activating while the foreground is someone else's makes tao steal focus with a fake Alt (`foreground_is_ours` in `state::placement`). Decided right before showing for an entrusted window, at open for a newly built one.
pub(crate) fn new_window_show(activate: bool, foreground_is_ours: bool) -> NewWindowShow {
    if activate && foreground_is_ours {
        NewWindowShow::RaiseAndActivate
    } else {
        NewWindowShow::Raise
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::spare::PaintGate;

    fn released(signal: Signal) -> u64 {
        match signal {
            Signal::Release(ticket) => ticket,
            other => panic!("the raised entry should have received it: {other:?}"),
        }
    }

    fn reveal_of(gate: &FirstPaintGate, label: &str, ticket: u64) -> Option<Reveal> {
        gate.release(label, ticket).map(|released| released.reveal)
    }

    // ── registry ──

    #[test]
    fn the_painted_signal_reveals_a_held_window_once() {
        let gate = FirstPaintGate::default();
        let ticket = gate.hold("main");
        assert_eq!(gate.painted("main", None), Signal::Release(ticket));
        assert_eq!(reveal_of(&gate, "main", ticket), Some(Reveal::Placed));
        assert_eq!(
            gate.painted("main", None),
            Signal::Ignored,
            "the second notification shows nothing"
        );
    }

    #[test]
    fn a_signal_from_a_window_that_was_not_held_does_nothing() {
        let gate = FirstPaintGate::default();
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Ignored);
        assert_eq!(gate.painted("slot-popup-1", Some(3)), Signal::Ignored);
    }

    #[test]
    fn the_signal_and_the_deadline_reveal_the_window_once_between_them() {
        let gate = FirstPaintGate::default();
        let ticket = gate.hold("main");
        let signal = released(gate.painted("main", None));
        assert!(gate.release("main", signal).is_some());
        assert!(
            !gate.is_waiting("main", ticket),
            "the deadline does not hand it over"
        );
        assert!(gate.release("main", ticket).is_none());
    }

    #[test]
    fn the_painted_signal_does_nothing_after_the_deadline() {
        let gate = FirstPaintGate::default();
        let ticket = gate.hold("main");
        assert!(gate.release("main", ticket).is_some());
        assert_eq!(gate.painted("main", None), Signal::Ignored);
    }

    #[test]
    fn an_old_ticket_does_not_reveal_a_window_held_again_under_the_same_label() {
        let gate = FirstPaintGate::default();
        let old = gate.hold("slot-popup-1");
        assert!(gate.release("slot-popup-1", old).is_some());
        let new = gate.hold("slot-popup-1");
        assert!(gate.release("slot-popup-1", old).is_none());
        assert!(gate.release("slot-popup-1", new).is_some());
    }

    #[test]
    fn a_built_window_does_not_look_at_the_adoption_number() {
        let gate = FirstPaintGate::default();
        let ticket = gate.hold("slot-popup-1");
        assert_eq!(
            gate.painted("slot-popup-1", Some(7)),
            Signal::Release(ticket)
        );
    }

    // ── handed-over show — the decision is made when the handed-over job runs ──

    #[test]
    fn a_window_hidden_after_the_deadline_queued_its_reveal_stays_hidden() {
        let gate = FirstPaintGate::default();
        let ticket = gate.hold("slot-popup-1");
        assert!(
            gate.is_waiting("slot-popup-1", ticket),
            "the deadline hands it over"
        );
        assert!(gate.cancel("slot-popup-1"));
        assert!(gate.release("slot-popup-1", ticket).is_none());
    }

    #[test]
    fn a_window_hidden_after_the_signal_queued_its_reveal_stays_hidden() {
        let gate = FirstPaintGate::default();
        gate.hold("slot-popup-1");
        let queued = released(gate.painted("slot-popup-1", None));
        assert!(gate.cancel("slot-popup-1"));
        assert!(gate.release("slot-popup-1", queued).is_none());
    }

    #[test]
    fn a_window_hidden_after_arming_queued_its_reveal_stays_hidden() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Recorded);
        let Arm::Painted(queued) = gate.arm("slot-popup-1") else {
            panic!("if decided after the notification, it hands over right away");
        };
        assert!(gate.cancel("slot-popup-1"));
        assert!(gate.release("slot-popup-1", queued).is_none());
    }

    #[test]
    fn a_window_held_again_after_hiding_is_not_revealed_by_a_reveal_queued_before() {
        let gate = FirstPaintGate::default();
        let first = gate.hold("slot-popup-1");
        let queued = released(gate.painted("slot-popup-1", None));
        assert!(gate.cancel("slot-popup-1"));
        let again = gate.hold("slot-popup-1");
        assert!(gate.release("slot-popup-1", queued).is_none());
        assert!(gate.release("slot-popup-1", first).is_none());
        assert!(gate.release("slot-popup-1", again).is_some());
    }

    #[test]
    fn rearming_after_hiding_ignores_the_reveal_queued_before() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Recorded);
        let Arm::Painted(queued) = gate.arm("slot-popup-1") else {
            panic!("if decided after the notification, it hands over right away");
        };
        assert!(gate.cancel("slot-popup-1"));
        let Arm::Waiting(again) = gate.arm("slot-popup-1") else {
            panic!("after withdrawing, no earlier notification remains");
        };
        assert!(gate.release("slot-popup-1", queued).is_none());
        assert!(gate.release("slot-popup-1", again).is_some());
    }

    // ── withdrawing (hide · tray show) ──

    #[test]
    fn cancelling_a_window_that_was_not_held_does_nothing() {
        let gate = FirstPaintGate::default();
        assert!(!gate.cancel("main"));
    }

    // ── keeping (runtime restore) ──

    #[test]
    fn an_unarmed_window_is_not_revealed_by_the_signal_and_arming_after_it_reveals_at_once() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Recorded);
        let Arm::Painted(ticket) = gate.arm("slot-popup-1") else {
            panic!("if decided after the notification, it hands over right away");
        };
        assert_eq!(
            reveal_of(&gate, "slot-popup-1", ticket),
            Some(Reveal::Placed)
        );
        assert_eq!(
            gate.painted("slot-popup-1", None),
            Signal::Ignored,
            "a notification after showing does not show again"
        );
    }

    #[test]
    fn restoring_counts_a_numbered_signal_as_painted_too() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1", Some(4)), Signal::Recorded);
        assert!(matches!(gate.arm("slot-popup-1"), Arm::Painted(_)));
    }

    #[test]
    fn arming_before_the_signal_waits_for_the_signal() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        let Arm::Waiting(ticket) = gate.arm("slot-popup-1") else {
            panic!("if decided before the notification, it waits");
        };
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Release(ticket));
        assert!(gate.release("slot-popup-1", ticket).is_some());
        assert!(!gate.is_waiting("slot-popup-1", ticket));
    }

    #[test]
    fn an_unarmed_window_kept_hidden_is_never_revealed() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Recorded);
        assert!(gate.cancel("slot-popup-1"));
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Ignored);
    }

    #[test]
    fn arming_a_window_that_was_not_held_waits_from_now() {
        let gate = FirstPaintGate::default();
        let Arm::Waiting(ticket) = gate.arm("slot-popup-1") else {
            panic!("with no stored notification, it waits");
        };
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Release(ticket));
        assert!(gate.release("slot-popup-1", ticket).is_some());
    }

    // ── entrusted prewarmed window (commit ③ · I17 · I26) ──

    #[test]
    fn revealing_a_label_without_a_hold_does_nothing() {
        let gate = FirstPaintGate::default();
        assert_eq!(gate.arm_adopted("slot-popup-1", 1, true, None), None);
        assert_eq!(
            gate.painted("slot-popup-1", Some(1)),
            Signal::Ignored,
            "a withdrawn raise is not revived"
        );
    }

    #[test]
    fn a_hold_cancelled_before_the_commit_is_not_revealed() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert!(gate.cancel("slot-popup-1"), "UI hide withdraws it");
        assert_eq!(gate.arm_adopted("slot-popup-1", 1, true, None), None);
    }

    #[test]
    fn a_signal_before_the_commit_without_the_number_is_ignored_and_the_commit_waits() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Recorded);
        let Some(Arm::Waiting(ticket)) = gate.arm_adopted("slot-popup-1", 2, true, None) else {
            panic!("a notification from the pre-adoption frame does not show right away");
        };
        assert!(
            gate.is_waiting("slot-popup-1", ticket),
            "the deadline starts counting here"
        );
        assert_eq!(
            gate.painted("slot-popup-1", Some(2)),
            Signal::Release(ticket)
        );
    }

    #[test]
    fn a_signal_before_the_commit_with_another_number_is_ignored() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1", Some(1)), Signal::Recorded);
        assert!(matches!(
            gate.arm_adopted("slot-popup-1", 2, true, None),
            Some(Arm::Waiting(_))
        ));
    }

    #[test]
    fn a_signal_before_the_commit_with_its_number_reveals_at_the_commit() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1", Some(2)), Signal::Recorded);
        let Some(Arm::Painted(ticket)) = gate.arm_adopted("slot-popup-1", 2, true, None) else {
            panic!("if a notification with the same number came first, ③ shows right away");
        };
        assert_eq!(
            reveal_of(&gate, "slot-popup-1", ticket),
            Some(Reveal::Adopted {
                generation: 2,
                activate: true
            })
        );
    }

    #[test]
    fn a_late_numberless_signal_does_not_erase_the_number_received_before_the_commit() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1", Some(2)), Signal::Recorded);
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Recorded);
        assert!(matches!(
            gate.arm_adopted("slot-popup-1", 2, true, None),
            Some(Arm::Painted(_))
        ));
    }

    #[test]
    fn after_the_commit_only_the_signal_with_its_number_reveals() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        let Some(Arm::Waiting(ticket)) = gate.arm_adopted("slot-popup-1", 5, false, None) else {
            panic!("if decided before the notification, it waits");
        };
        assert_eq!(
            gate.painted("slot-popup-1", None),
            Signal::Mismatch {
                expected: 5,
                got: None
            }
        );
        assert_eq!(
            gate.painted("slot-popup-1", Some(4)),
            Signal::Mismatch {
                expected: 5,
                got: Some(4)
            }
        );
        assert!(
            gate.is_waiting("slot-popup-1", ticket),
            "a discarded notification does not touch the raise"
        );
        assert_eq!(
            gate.painted("slot-popup-1", Some(5)),
            Signal::Release(ticket)
        );
        assert_eq!(
            reveal_of(&gate, "slot-popup-1", ticket),
            Some(Reveal::Adopted {
                generation: 5,
                activate: false
            }),
            "the entrust's activate is carried through to the entry"
        );
    }

    #[test]
    fn an_adopted_window_is_revealed_once_between_its_signal_and_the_deadline() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        let Some(Arm::Waiting(ticket)) = gate.arm_adopted("slot-popup-1", 1, true, None) else {
            panic!("if decided before the notification, it waits");
        };
        let signal = released(gate.painted("slot-popup-1", Some(1)));
        assert!(gate.release("slot-popup-1", signal).is_some());
        assert!(
            !gate.is_waiting("slot-popup-1", ticket),
            "the deadline does not hand it over"
        );
        assert!(gate.release("slot-popup-1", ticket).is_none());
        assert_eq!(gate.painted("slot-popup-1", Some(1)), Signal::Ignored);
    }

    #[test]
    fn hiding_after_the_commit_keeps_the_adopted_window_hidden() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        let Some(Arm::Waiting(ticket)) = gate.arm_adopted("slot-popup-1", 1, true, None) else {
            panic!("if decided before the notification, it waits");
        };
        assert!(gate.cancel("slot-popup-1"));
        assert_eq!(gate.painted("slot-popup-1", Some(1)), Signal::Ignored);
        assert!(!gate.is_waiting("slot-popup-1", ticket));
        assert!(gate.release("slot-popup-1", ticket).is_none());
    }

    #[test]
    fn a_hold_put_back_by_the_tray_is_revealed_by_the_commit() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert!(gate.cancel("slot-popup-1"));
        gate.hold_unarmed("slot-popup-1");
        assert!(matches!(
            gate.arm_adopted("slot-popup-1", 1, true, None),
            Some(Arm::Waiting(_))
        ));
    }

    // A hold put up again without being withdrawn (re-arming at tray show) does not erase a notification already kept.
    #[test]
    fn holding_again_keeps_a_notice_already_received() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1", Some(1)), Signal::Recorded);
        gate.hold_unarmed("slot-popup-1");
        assert!(matches!(
            gate.arm_adopted("slot-popup-1", 1, true, None),
            Some(Arm::Painted(_))
        ));
    }

    // ── branches of showing an entrusted window (I5 · I30) ──

    #[test]
    fn a_new_window_is_activated_only_when_asked_active_and_the_foreground_is_ours() {
        for (activate, ours, show) in [
            (true, true, NewWindowShow::RaiseAndActivate),
            (true, false, NewWindowShow::Raise),
            (false, true, NewWindowShow::Raise),
            (false, false, NewWindowShow::Raise),
        ] {
            assert_eq!(
                new_window_show(activate, ours),
                show,
                "activate={activate} ours={ours}"
            );
        }
    }

    // A window the window port newly built without focus (I30) — accepted without looking at the number, and it falls into the raise-only show.
    #[test]
    fn a_window_built_without_focus_takes_any_signal_and_is_raised_without_activation() {
        let gate = FirstPaintGate::default();
        let opened = Instant::now();
        for generation in [None, Some(4)] {
            let ticket = gate.hold_as("slot-popup-2", Reveal::Raised, Some(opened));
            assert_eq!(
                gate.painted("slot-popup-2", generation),
                Signal::Release(ticket)
            );
            let released = gate.release("slot-popup-2", ticket).expect("takes it out");
            assert_eq!(
                (released.reveal, released.opened),
                (Reveal::Raised, Some(opened))
            );
        }
    }

    // The entrusted window's open time goes through the ③ entry to the show log (`reveal_ms`).
    #[test]
    fn the_open_time_of_an_adopted_window_reaches_its_release() {
        let gate = FirstPaintGate::default();
        let opened = Instant::now();
        gate.hold_unarmed("slot-popup-1");
        let Some(Arm::Waiting(ticket)) = gate.arm_adopted("slot-popup-1", 2, false, Some(opened))
        else {
            panic!("if decided before the notification, it waits");
        };
        assert_eq!(
            gate.painted("slot-popup-1", Some(2)),
            Signal::Release(ticket)
        );
        let released = gate.release("slot-popup-1", ticket).expect("takes it out");
        assert_eq!(released.opened, Some(opened));
        assert!(matches!(
            released.reveal,
            Reveal::Adopted {
                generation: 2,
                activate: false
            }
        ));
    }

    // ── the pool's handle ──

    #[test]
    fn the_pool_handle_holds_and_cancels_on_the_shared_gate() {
        let gate = Arc::new(FirstPaintGate::default());
        let handle = SparePaintGate::new(gate.clone());
        handle.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Recorded);
        handle.cancel("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Ignored);
    }

    #[test]
    fn the_pool_handle_without_the_app_treats_the_commit_as_cancelled() {
        let gate = Arc::new(FirstPaintGate::default());
        let handle = SparePaintGate::new(gate.clone());
        handle.hold_unarmed("slot-popup-1");
        assert!(!handle.reveal_if_held("slot-popup-1", 1, true, None));
        assert_eq!(gate.painted("slot-popup-1", Some(1)), Signal::Ignored);
    }

    // ── deadline ──

    fn counter() -> (Arc<Mutex<u32>>, impl FnOnce()) {
        let queued = Arc::new(Mutex::new(0));
        let sink = queued.clone();
        (queued, move || *sink.lock().unwrap() += 1)
    }

    #[tokio::test(start_paused = true)]
    async fn without_a_signal_the_deadline_queues_the_reveal_after_the_fallback() {
        let gate = Arc::new(FirstPaintGate::default());
        let ticket = gate.hold("main");
        let (queued, queue) = counter();
        let deadline = gate.clone();
        let task = tokio::spawn(async move {
            queue_when_overdue(&deadline, "main", ticket, FALLBACK_REVEAL, queue).await
        });

        tokio::time::sleep(FALLBACK_REVEAL - Duration::from_millis(1)).await;
        assert_eq!(
            *queued.lock().unwrap(),
            0,
            "does not hand over before the deadline"
        );

        task.await.unwrap();
        assert_eq!(*queued.lock().unwrap(), 1);
        assert!(gate.release("main", ticket).is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn a_reveal_before_the_deadline_leaves_the_deadline_nothing_to_queue() {
        let gate = Arc::new(FirstPaintGate::default());
        let ticket = gate.hold("main");
        let (queued, queue) = counter();
        let deadline = gate.clone();
        let task = tokio::spawn(async move {
            queue_when_overdue(&deadline, "main", ticket, FALLBACK_REVEAL, queue).await
        });

        let signal = released(gate.painted("main", None));
        assert!(gate.release("main", signal).is_some());
        task.await.unwrap();
        assert_eq!(*queued.lock().unwrap(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_window_hidden_before_the_deadline_leaves_the_deadline_nothing_to_queue() {
        let gate = Arc::new(FirstPaintGate::default());
        let ticket = gate.hold("slot-popup-1");
        let (queued, queue) = counter();
        let deadline = gate.clone();
        let task = tokio::spawn(async move {
            queue_when_overdue(&deadline, "slot-popup-1", ticket, FALLBACK_REVEAL, queue).await
        });

        assert!(gate.cancel("slot-popup-1"));
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Ignored);
        task.await.unwrap();
        assert_eq!(*queued.lock().unwrap(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn an_adopted_window_whose_signals_carry_another_number_is_revealed_by_the_deadline() {
        let gate = Arc::new(FirstPaintGate::default());
        gate.hold_unarmed("slot-popup-1");
        let Some(Arm::Waiting(ticket)) = gate.arm_adopted("slot-popup-1", 3, true, None) else {
            panic!("if decided before the notification, it waits");
        };
        let (queued, queue) = counter();
        let deadline = gate.clone();
        let task = tokio::spawn(async move {
            queue_when_overdue(&deadline, "slot-popup-1", ticket, FALLBACK_REVEAL, queue).await
        });

        assert!(matches!(
            gate.painted("slot-popup-1", None),
            Signal::Mismatch { .. }
        ));
        task.await.unwrap();
        assert_eq!(*queued.lock().unwrap(), 1);
        assert!(gate.release("slot-popup-1", ticket).is_some());
    }
}
