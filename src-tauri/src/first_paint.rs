//! Hide a window until its first paint — main · pop-outs are built hidden ([`hold_window`]), and when the page reports it has drawn the actual first screen (tab bar + active tab)
//! (`webview_painted` · `src/theme/firstPaint.ts`), the window is shown. User decision (2026-10-19): the window appears together with its content —
//! the frame is not shown first (the native frame is due to be removed).
//!
//! - ★The webview is not hidden — let it draw inside the hidden window★: when a hidden webview (`Webview::hide`) was shown again, WebView2 once showed a surface it had not yet
//!   drawn (user observation 2026-10-19 「창이 뜨고 좀 있다가 깜박임」).
//! - ★A window is shown only once per window put under hold★ — whichever comes first of the notification and [`FALLBACK_REVEAL`]. A notification for a window with no hold (main of a `--hidden`
//!   boot · a prewarmed window before entrusting · a reload of an already shown window) does nothing.
//! - ★Whether to show is decided on the main thread, right before showing★ — a notification · deadline · restore carries only the hold's ticket to the main thread,
//!   and when the handed-over job runs, if the hold of that ticket remains, it is taken out and shown ([`FirstPaintGate::release`]). If taken out before handing over, then
//!   after the hand-over · before it runs, the main thread's hide (the end of `--hidden` boot setup · tray hide) finds nothing to withdraw and the handed-over job shows
//!   a hidden window. There are three ways of showing — a built-and-held window · a restore window use [`crate::state::placement::show_placed`] (activated when shown ·
//!   if the maximize wish is true, shown maximized without a normal-size frame), and a window the window port newly built without focus (I30) and an entrusted prewarmed window are
//!   shown after raising only z to the top while hidden (only the entrusted window is activated under the condition below — [`new_window_show`]). ★In all three, after showing, the
//!   recorded maximize wish is applied★ (`state::placement::apply_deferred_maximize` — a window-state command that came to a waiting window records that wish ·
//!   TRD S26-window-chrome §5-2). If a path is missing, that wish is left alone and that window's place recording stops.
//! - A window-state command checks with [`is_held`] whether that window is held at this gate — a held window is not revealed (that gate shows it).
//! - A pop-out of runtime restore acceptance has its whether-to-show decided after it enters the model, so it is held in two stages — when built it only keeps the notification
//!   ([`hold_window_unarmed`]), and when decided to show, it is shown ([`reveal_held`] — immediately if the notification has already come, otherwise at the notification · deadline).
//!   The deadline is measured from when it was decided to show — if shown before deciding, a window that might still be reverted is revealed. The way of showing is the same as a window
//!   built and held at build time — it activates.
//! - An entrusted prewarmed window (TRD S21-slot-docking §3-9) is also held in two stages — at entrusting, the prewarmed window pool puts up a hold that only keeps the notification
//!   ([`SparePaintGate`]), and commit ③ decides to show only if that hold remains ([`reveal_if_held`] — the deadline starts here).
//!   ★That entry accepts only a notification that carries the adoption number of that claim★ (I26) — a notification with no number or a different number is one sent by the pre-adoption frame (waiting page ·
//!   `bootFailed`), and obeying it would show that frame at once. A notification kept before ③ is also shown by ③ immediately only if it has the same number. The number
//!   is looked at only on this path — a built-and-held window and the restore path accept a notification with or without a number. ★If the entrusted window is shown
//!   (by either notification or deadline), the prewarmed window pool's rebuild completion is called★ (`SpareState::on_shown` — other completions = window
//!   `Destroyed` · safety deadline).
//! - ★The registry lock is a leaf★ — while holding it, no logging · window call · other locks. The prewarmed window pool puts up and withdraws holds while holding its own lock
//!   (order = layout → pool → first-paint gate — `crate::spare::SpareState`).
//! - ★Hiding withdraws the hold ([`cancel_hold`])★ — the tray hide path (`tray::actions::hide_main_ui` — ADR-0229) and restore's
//!   keep-hidden call it. If not withdrawn, a notification · deadline arriving after hiding shows the window again: in a `--hidden` boot, main stays
//!   in the tray while only the pop-out appears (and even takes the foreground), and usage interest treats that window as hidden. Tray show withdraws it too —
//!   that show shows the window and decides the order among windows (main last · frontmost). It is also withdrawn when a window disappears (`Destroyed`) — a window
//!   not decided to show has no deadline, so there is no other place to withdraw it.
//! - There is one more reason for building hidden — tao 0.35.3 shows a visible window at the end of window creation (`init`), but the subclass that
//!   paints the window background (`WM_ERASEBKGND`) is attached after that, so the first frame's inside was unpainted and what was behind the window showed through (user video
//!   2026-10-19 · `platform_impl/windows/window.rs` — source reading). A show after building erases with the background color.
//! - Whether the page actually draws while hidden (whether WebView2 sees the parent window's hiding as occlusion and stops drawing) is [unverified — GUI]. If it stops,
//!   the background color shows for a moment right after being shown.
// ADR-0299

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use engram_dashboard_base::sync;
use tauri::{AppHandle, Manager, WebviewWindow};

/// The upper bound on waiting for the first-paint notification — if exceeded, it is shown without a notification. What it covers = page load · bundle execution · first mount · tab list and active
/// tab layout query. The value is not measured but a judgment of 「넘으면 깨진 것으로 본다」 (if exceeded, treat it as broken) (a window launched with the dev server takes 0.8~1.1 s — user
/// observation 2026-10-19). If exceeded, it is shown before first paint.
// ADR-0305 — failure before first paint
pub(crate) const FALLBACK_REVEAL: Duration = Duration::from_millis(1500);

/// The registry of hidden windows waiting to be shown — label → held state. Showing is done on the main thread by the one side that takes out that ticket's entry
/// (module head). It is one per shell (managed in the builder — it must exist before any place that creates a window).
#[derive(Default)]
pub(crate) struct FirstPaintGate {
    held: Mutex<HashMap<String, Hold>>,
    next: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hold {
    /// Shown at the notification or the deadline. `ticket` = the hold's ticket — drawn fresh at every hold (monotonic across the whole registry), and a handed-over show takes out
    /// only when it is its own ticket. This keeps an old job from showing a window that was withdrawn and then held again. `since` = when held (log `held_ms`) · `opened` =
    /// when the window port began opening that window (log `reveal_ms` — `None` for a window held outside the window port).
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

/// A notification that came to a keeping entry — the restore path ([`FirstPaintGate::arm`]) sees both `Numberless` and `Generation` as 「알림 옴」 (notification arrived),
/// and commit ③ ([`FirstPaintGate::arm_adopted`]) sees only the `Generation` of that adoption number. If lumped into one true/false, either the restore
/// window loses the notification (if a number is required) or the pre-adoption frame is shown at once (if the number is not looked at).
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

    // If a hold already exists, leave it as is — overwriting loses the first-paint notification already kept for that window, so the later `arm` · `arm_adopted` cannot
    //   show immediately and wait for the deadline (re-arming at tray show — `spare::SpareState::on_tray_show`).
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
                // A notification with no number does not erase a kept number — if a late-handled notification of the pre-adoption frame overwrote a notification that arrived before ③ with the same number,
                //   ③ could not show immediately and would wait for the deadline.
                match generation {
                    Some(got) => *painted = Painted::Generation(got),
                    None if *painted == Painted::Not => *painted = Painted::Numberless,
                    None => {}
                }
                Signal::Recorded
            }
        }
    }

    // Looked at before handing over — a taken-out ticket never comes again (tickets are monotonic), so 「없다」 (absent) stays the same even after handing over. 「있다」 (present) is
    //   decided again by the handed-over job ([`Self::release`]).
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
        // Treated the same as 「거둬졌다」 (was withdrawn) — it becomes a model window while hidden and the next tray show shows it.
        self.gate.cancel(label);
        tracing::warn!(
            module = "first_paint",
            label,
            generation,
            "앱 손잡이가 꽂히기 전의 확정이라 맡긴 창을 첫 화면에 올리지 않는다"
        );
        false
    }
}

fn gate(app: &AppHandle) -> Option<Arc<FirstPaintGate>> {
    app.try_state::<Arc<FirstPaintGate>>()
        .map(|gate| gate.inner().clone())
}

/// Put a window built hidden under hold so that it is shown at the first-paint notification (or [`FALLBACK_REVEAL`]) — activated when shown. ★The window must be hidden
/// and its place · deferred maximize must already be applied★ — the show appears at that place as it is. If the registry is not registered, it is shown
/// right away (a hidden window with no way to be shown is not made).
pub(crate) fn hold_window(window: &WebviewWindow) {
    let Some(gate) = gate(window.app_handle()) else {
        tracing::warn!(
            module = "first_paint",
            label = window.label(),
            "첫 화면 명부가 등록돼 있지 않아 창을 곧바로 보인다"
        );
        show_as(window, Reveal::Placed);
        return;
    };
    let ticket = gate.hold(window.label());
    reveal_when_overdue_on_main_thread(window.app_handle(), gate, window.label(), ticket);
}

/// The window-port edition of [`hold_window`] — a pop-out newly built by the window port. `focused` = built while receiving focus (activated when shown —
/// the same as [`hold_window`]) · `false` = built without focus (raises only z while hidden and is shown without activation — I30). `opened` = when the window
/// port began opening that window (the show log `reveal_ms`).
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
            "첫 화면 명부가 등록돼 있지 않아 창을 곧바로 보인다"
        );
        show_as(window, reveal);
        return;
    };
    let ticket = gate.hold_as(window.label(), reveal, Some(opened));
    reveal_when_overdue_on_main_thread(window.app_handle(), gate, window.label(), ticket);
}

/// Only keeps the first-paint notification of a window built hidden — whether to show is decided later by [`reveal_held`] (it is shown) · [`cancel_hold`] (it is kept
/// hidden). Until decided, neither the notification nor the deadline shows the window. ★If that window already has a hold, leave it as is★ — a hold already decided to show
/// (armed) also stays, so that window is shown at the notification · deadline even after this call (a kept notification is not erased either). If the registry is not registered,
/// nothing is done (then [`reveal_held`] shows it immediately).
pub(crate) fn hold_window_unarmed(window: &WebviewWindow) {
    if let Some(gate) = gate(window.app_handle()) {
        gate.hold_unarmed(window.label());
    }
}

/// Decide to show that window at first paint — immediately if the notification has already come, otherwise it is shown at the notification or at [`FALLBACK_REVEAL`] from now
/// — activated when shown. A window that was not kept (including a withdrawn window) is also waited for from now. It may be called from any thread (the show
/// is handed to the main thread).
pub(crate) fn reveal_held(app: &AppHandle, label: &str) {
    let Some(gate) = gate(app) else {
        tracing::warn!(
            module = "first_paint",
            label,
            "첫 화면 명부가 등록돼 있지 않아 창을 곧바로 보인다"
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

// Commit ③ (TRD S21-slot-docking §3-9 「`committed`」) — `true` = the remaining hold was put up. It may be called from any thread.
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

/// Withdraw that window's hold — a first-paint notification · deadline coming later and an already handed-over show will not show that window. If the window was never held,
/// nothing is done. The hide · tray show paths call it for each window (module head).
pub(crate) fn cancel_hold(app: &AppHandle, label: &str) {
    if gate(app).is_some_and(|gate| gate.cancel(label)) {
        tracing::debug!(
            module = "first_paint",
            label,
            "첫 화면을 기다리던 창의 올림을 거둔다 — 알림 · 시한이 그 창을 보이지 않는다"
        );
    }
}

/// Is that window held at this gate now — true both for a hold decided to show and for a hold that only keeps the notification (in both, this gate ·
/// its owner decides whether to show). False if the registry is not registered. It is a momentary value — if read off the main thread, this gate's show can pass
/// after the read · before the write (TRD S26-window-chrome §5-2 — a window-state command reads inside a single main-thread job).
pub(crate) fn is_held(app: &AppHandle, label: &str) -> bool {
    gate(app).is_some_and(|gate| gate.is_held(label))
}

/// That window's first-paint notification — `generation` = the adoption number the notification carries. If it is a held window it is shown (if it is a window that only keeps, it is kept ·
/// an entrusted prewarmed window accepts only a notification of that adoption number), otherwise nothing is done.
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
            "채택 번호가 다른 첫 화면 알림을 버린다 — 채택 전 틀이 보낸 알림이다"
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
    // `via` · `reveal_ms` = material to compare the open → visible delay of a prewarmed window and a fresh build (TRD S21-slot-docking §3-9 「로그」 · M14).
    // ADR-0299
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
                "첫 화면 알림에 창을 보인다"
            ),
            Self::PaintedBeforeArming => tracing::info!(
                module = "first_paint",
                label,
                cause = "painted_before_arming",
                via,
                held_ms,
                reveal_ms = ?reveal_ms,
                "첫 화면 알림이 먼저 와 있어 창을 곧바로 보인다"
            ),
            Self::Overdue => tracing::warn!(
                module = "first_paint",
                label,
                cause = "overdue",
                via,
                held_ms,
                reveal_ms = ?reveal_ms,
                "첫 화면 알림이 시한 안에 오지 않아 창을 보인다"
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
                "넘긴 보이기가 돌기 전에 그 올림이 보였거나 거둬져 보이지 않는다"
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
                tracing::debug!(module = "first_paint", label = %owned, "보이기 전에 창이 닫혔다")
            }
        }
    });
    if let Err(e) = queued {
        tracing::warn!(
            module = "first_paint",
            label,
            "숨긴 창을 보이는 일을 메인 스레드에 넘기지 못했다: {e}"
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

// Show of an entrusted window (TRD S21-slot-docking §3-9 「보이는 차례」) — the foreground decision is right before showing. After showing (even if it could not be shown),
//   it calls rebuild completion — both the notification · deadline come here.
// ADR-0299
fn show_adopted(window: &WebviewWindow, activate: bool) {
    let app = window.app_handle();
    let show = new_window_show(activate, crate::state::placement::foreground_is_ours(app));
    raise_and_show(window, show == NewWindowShow::RaiseAndActivate);
    crate::commands::spare::on_adopted_window_shown(app, window.label());
}

// ⓐ raise only z to the top while hidden → ⓑ show · apply the recorded maximize wish → ⓒ activate (only for `activate`). ⓐ is placed before ⓑ — so a long-
//   hidden window is not seen even for one frame at its old z place (I29). ★ⓑ is Tauri `show()`★ — if shown with raw Win32, the tao visibility flag stays
//   false and tray hide does nothing (the prewarmed-window building head of `commands::spare`). It is a window built without focus, so
//   tao shows it without activating. Whether ⓐ alone puts it above another app's foreground window · whether the z raised while hidden remains after ⓑ is
//   [unmeasured — M22]. `SW_MAXIMIZE` · `SW_RESTORE` of applying the wish double as activation — whether, if a window built without focus has a wish,
//   that applying moves the foreground is [unmeasured — TRD S26-window-chrome §10-5 「숨은 창 보이기의 전경」].
// ADR-0299
fn raise_and_show(window: &WebviewWindow, activate: bool) {
    let label = window.label();
    let os = window.as_ref().window();
    raise_without_activation(&os);
    if let Err(e) = window.show() {
        tracing::warn!(module = "first_paint", label, error = %e, "숨긴 창을 보이지 못했다");
        return;
    }
    // TRD S26-window-chrome §5-2 「모든 보이기 갈래가 미룬 최대화를 입힌다」
    crate::state::placement::apply_deferred_maximize(&os);
    if activate {
        if let Err(e) = window.set_focus() {
            tracing::debug!(module = "first_paint", label, error = %e, "보인 창에 포커스를 주지 못했다");
        }
    }
}

/// Raise only that window's z to the top of its band without activation — a hidden window stays hidden (platform `place_window(.., ZOrder::Top, show: false,
/// activate: false)` · I29). A failure is only logged. ★Called on the main thread★ (platform `window` head 「스레드」 (threads)).
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
                tracing::debug!(module = "first_paint", label, error = %e, "창의 z 를 올리지 못하고 넘어간다");
            }
            engram_dashboard_platform::window::WindowError::NoWindow
            | engram_dashboard_platform::window::WindowError::Os(_) => {
                tracing::warn!(module = "first_paint", label, error = %e, "창의 z 를 올리지 못하고 넘어간다");
            }
        }
    }
}

/// Show of a new window (an entrusted prewarmed window · a window the window port newly built) — both are shown after raising z to the top while hidden (I29).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NewWindowShow {
    /// Leaves focus as it is (user decision 2026-10-21 「포커스 뺏지말고, 순서만 위로올려.」).
    Raise,
    /// Activates when shown — an entrusted window is given focus after being shown, and a newly built window is built while receiving focus.
    RaiseAndActivate,
}

/// Activation is only when that request was opened as active (`activate`) and the foreground is now our process (I5 · I30) — if activated while the foreground is someone else's,
/// tao steals focus with a fake Alt (head of `foreground_is_ours` in `state::placement`). For an entrusted window it is decided right before
/// showing, and for a newly built window at open.
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
            other => panic!("올린 항목이 받았어야 한다: {other:?}"),
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
            "두 번째 알림은 보이지 않는다"
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
        assert!(!gate.is_waiting("main", ticket), "시한은 넘기지 않는다");
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
        assert!(gate.is_waiting("slot-popup-1", ticket), "시한이 넘긴다");
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
            panic!("알림 뒤에 정했으면 곧바로 넘긴다");
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
            panic!("알림 뒤에 정했으면 곧바로 넘긴다");
        };
        assert!(gate.cancel("slot-popup-1"));
        let Arm::Waiting(again) = gate.arm("slot-popup-1") else {
            panic!("거둔 뒤에는 그 전 알림이 남아 있지 않다");
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
            panic!("알림 뒤에 정했으면 곧바로 넘긴다");
        };
        assert_eq!(
            reveal_of(&gate, "slot-popup-1", ticket),
            Some(Reveal::Placed)
        );
        assert_eq!(
            gate.painted("slot-popup-1", None),
            Signal::Ignored,
            "보인 뒤의 알림은 다시 보이지 않는다"
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
            panic!("알림 전에 정했으면 기다린다");
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
            panic!("받아 둔 알림이 없으면 기다린다");
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
            "거둔 올림을 되살리지 않는다"
        );
    }

    #[test]
    fn a_hold_cancelled_before_the_commit_is_not_revealed() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert!(gate.cancel("slot-popup-1"), "UI 숨기기가 거둔다");
        assert_eq!(gate.arm_adopted("slot-popup-1", 1, true, None), None);
    }

    #[test]
    fn a_signal_before_the_commit_without_the_number_is_ignored_and_the_commit_waits() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Recorded);
        let Some(Arm::Waiting(ticket)) = gate.arm_adopted("slot-popup-1", 2, true, None) else {
            panic!("채택 전 틀의 알림으로 곧바로 보이지 않는다");
        };
        assert!(
            gate.is_waiting("slot-popup-1", ticket),
            "시한은 여기부터 돈다"
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
            panic!("같은 번호의 알림이 먼저 왔으면 ③ 이 곧바로 보인다");
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
            panic!("알림 전에 정했으면 기다린다");
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
            "버린 알림은 올림을 건드리지 않는다"
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
            "맡김의 activate 가 항목까지 옮겨진다"
        );
    }

    #[test]
    fn an_adopted_window_is_revealed_once_between_its_signal_and_the_deadline() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        let Some(Arm::Waiting(ticket)) = gate.arm_adopted("slot-popup-1", 1, true, None) else {
            panic!("알림 전에 정했으면 기다린다");
        };
        let signal = released(gate.painted("slot-popup-1", Some(1)));
        assert!(gate.release("slot-popup-1", signal).is_some());
        assert!(
            !gate.is_waiting("slot-popup-1", ticket),
            "시한은 넘기지 않는다"
        );
        assert!(gate.release("slot-popup-1", ticket).is_none());
        assert_eq!(gate.painted("slot-popup-1", Some(1)), Signal::Ignored);
    }

    #[test]
    fn hiding_after_the_commit_keeps_the_adopted_window_hidden() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        let Some(Arm::Waiting(ticket)) = gate.arm_adopted("slot-popup-1", 1, true, None) else {
            panic!("알림 전에 정했으면 기다린다");
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
            let released = gate.release("slot-popup-2", ticket).expect("빼낸다");
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
            panic!("알림 전에 정했으면 기다린다");
        };
        assert_eq!(
            gate.painted("slot-popup-1", Some(2)),
            Signal::Release(ticket)
        );
        let released = gate.release("slot-popup-1", ticket).expect("빼낸다");
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
        assert_eq!(*queued.lock().unwrap(), 0, "시한 전에는 넘기지 않는다");

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
            panic!("알림 전에 정했으면 기다린다");
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
