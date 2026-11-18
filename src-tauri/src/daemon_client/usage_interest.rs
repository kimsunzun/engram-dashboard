//! The shell's usage interest — a state machine computing the daemon usage subscription set from the layout and keeping that subscription in line with the daemon (TRD S21 usage-limit-slot §1-7).
//!
//! ## What it owns
//! - Interest = the union of companies turned on by the usage slots of the active tab of every window with no hidden mark ([`interest_from_layout`]). Minimize is not tracked by the shell, so it counts as visible (user decision 2026-09-27). A view that is no window's tab (the temporary view of a slot move) is excluded. A window left only in the model without an OS window (the unresolved issue in the `WindowHost` doc of `layout::apply`) counts as visible.
//! - Windows to receive broadcasts = windows with a usage slot in any tab, hidden ones included ([`usage_slot_windows`]) — up to date right away when shown again.
//! - [`UsageInterest`] — the last values of those two · the set last sent on this socket (`sent`) · the reduction delay · the snapshot cache received on this socket.
//!
//! ## Pure — knows nothing of Tauri · sockets · tokio · clocks
//! Time is supplied by the caller (`now`); methods only return results, and socket writes · emit · timer starts are the caller's job, so tests are deterministic with a fake time (`cargo test -p engram-dashboard --test lib_unit`).
//!
//! ## One serial path — only the connection task takes out the set to send
//! The side changing interest (layout recompute · hide · reduction timer) puts in only a set-less nudge; when the connection task takes it out it reads the then-current interest with [`UsageInterest::sync`] and sends — read order = send order, so the last frame on the socket is always the latest interest. A caller reading the set inside the lock and sending outside lets the old set arrive last when two changes cross between read and send — do not revive it (TRD §3 #74). So [`InterestAction`] carries no set, and the connection-task-only methods (`sync` · `on_socket_open` · `on_socket_lost` · `on_snapshot`) are not opened outside `daemon_client`.
//!
//! ## Socket epoch guard
//! Every method the connection driver calls carries its socket epoch (`socket_epoch` — the transport connection generation · ADR-0195 · `0` = no socket) and does nothing unless it is the current one (an open does nothing unless greater than the largest seen), so a late call for an older connection cannot overwrite the new one's `sent` or cache. Ordering by size is meaningful because generations never rewind (the transport mints them from one counter per peer name within a registry), so it holds within one shell process.
//!
//! ## Lock order
//! The lock around this state comes after `ViewManager` (`super::SharedUsageInterest` doc); this file knows no lock.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use engram_dashboard_protocol::{
    AgentBackendKind, UsageLimitSnapshot, UsageVendorState, UsageWindow,
};

use crate::layout::manager::WindowLabel;
use crate::layout::{LayoutNode, SlotContent, ViewManager};

/// Wait before sending a shrink to the daemon — avoids sending reductions that soon come back (switching to another tab and returning · toggling a display switch off and on · hiding to the tray and reviving · closing a slot·tab and putting it back within 1.5 s). Growth is not waited for. Moving a slot to another window does not rely on this delay: the source slot remains until the step that attaches it to the new window and is closed inside the same lock, so interest does not shrink at all (`layout::apply::move_slot_to_window`).
pub const USAGE_INTEREST_SHRINK_DELAY: Duration = Duration::from_millis(1500);

pub fn interest_from_layout(
    mgr: &ViewManager,
    hidden: &BTreeSet<WindowLabel>,
) -> BTreeSet<AgentBackendKind> {
    let mut interest = BTreeSet::new();
    for (label, tabs) in &mgr.windows {
        if hidden.contains(label) {
            continue;
        }
        let Some(view) = mgr.views.get(&tabs.active) else {
            continue;
        };
        for_each_usage_slot(&view.layout, &mut |show_claude, show_codex| {
            if show_claude {
                interest.insert(AgentBackendKind::Claude);
            }
            if show_codex {
                interest.insert(AgentBackendKind::Codex);
            }
        });
    }
    interest
}

pub fn usage_slot_windows(mgr: &ViewManager) -> BTreeSet<WindowLabel> {
    mgr.windows
        .iter()
        .filter(|(_, tabs)| {
            tabs.tabs.iter().any(|view| {
                mgr.views.get(view).is_some_and(|view| {
                    let mut found = false;
                    for_each_usage_slot(&view.layout, &mut |_, _| found = true);
                    found
                })
            })
        })
        .map(|(label, _)| label.clone())
        .collect()
}

fn for_each_usage_slot(node: &LayoutNode, visit: &mut dyn FnMut(bool, bool)) {
    match node {
        LayoutNode::Slot {
            content:
                SlotContent::Usage {
                    show_claude,
                    show_codex,
                },
            ..
        } => visit(*show_claude, *show_codex),
        LayoutNode::Slot { .. } => {}
        LayoutNode::Split { a, b, .. } => {
            for_each_usage_slot(a, visit);
            for_each_usage_slot(b, visit);
        }
    }
}

/// The generation of one reduction delay. If interest changes again during the delay a new generation stands or the delay becomes invalid, and an old timer waking late gets `false` in [`UsageInterest::deferral_elapsed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShrinkGen(u64);

/// What recomputation tells the caller to do — the result of comparing with `sent` (the set last sent on this socket). Discarded, growth never reaches the daemon or the reduction never does.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterestAction {
    /// Nothing to do — interest = `sent` (a waiting reduction became invalid), or a reduction to the same interest is already waiting (its generation · deadline · timer still valid).
    Unchanged,
    Nudge,
    Defer { gen: ShrinkGen, deadline: Instant },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SnapshotOutcome {
    /// The windows to deliver that snapshot to (= the last value of [`usage_slot_windows`]), regardless of revision — telling them apart is the webview's job. Empty if it is not a snapshot of the current socket.
    pub labels: Vec<WindowLabel>,
    /// The `UsageSubscribe` set to write straight to the same socket — stands only when the subscription set the daemon carried differs from the set the shell wants on the daemon; it stands every time it differs (no suppression — [`UsageInterest::on_snapshot`]).
    pub resend: Option<BTreeSet<AgentBackendKind>>,
}

#[derive(Debug, Clone, PartialEq)]
/// What a webview pull gets — the whole cache (after handoff correction, in company order) and the current socket marker; with no socket, `0` and an empty list.
pub struct WebviewUsageSnapshot {
    pub socket_epoch: u64,
    pub snapshots: Vec<UsageLimitSnapshot>,
}

#[derive(Debug)]
struct Cached {
    snapshot: UsageLimitSnapshot,
    received: Instant,
}

/// The usage interest state of one shell. The shell opens one daemon connection, so the connection field of the cache key (connection, company) is "the current socket", i.e. `socket_epoch`; the cache is emptied when the socket changes.
#[derive(Debug, Default)]
pub struct UsageInterest {
    hidden: BTreeSet<WindowLabel>,
    interest: BTreeSet<AgentBackendKind>,
    usage_windows: BTreeSet<WindowLabel>,
    socket_epoch: u64,
    max_seen_socket_epoch: u64,
    sent: BTreeSet<AgentBackendKind>,
    cache: BTreeMap<AgentBackendKind, Cached>,
    shrink_seq: u64,
    /// Invariant: while standing, `interest ⊊ sent` and `interest` is exactly the interest that set up that reduction (setting up and cancelling a reduction are only recompute · send · socket open/close, and anything changing `sent` meanwhile cancels it).
    shrink_pending: Option<ShrinkGen>,
}

impl UsageInterest {
    pub fn new() -> Self {
        Self::default()
    }

    /// After layout · visibility changes, recompute the interest and the windows to receive broadcasts — called inside the ViewManager lock. The hidden mark is not erased here: a mark for a label not yet in the model is kept too — a slot move to a new window opens the OS window first (phase B), releases the lock, then adds the model window (phase C), so erasing the tray hide in that gap would count a hidden pop-out as visible. Only window destruction erases it ([`Self::forget_window`]).
    pub(super) fn recompute(&mut self, mgr: &ViewManager, now: Instant) -> InterestAction {
        let before = std::mem::replace(&mut self.interest, interest_from_layout(mgr, &self.hidden));
        self.usage_windows = usage_slot_windows(mgr);
        if !self.interest.is_subset(&self.sent) {
            self.shrink_pending = None;
            InterestAction::Nudge
        } else if self.shrink_pending.is_some() && self.interest == before {
            // The same reduction is already waiting — leave the generation · deadline so an unrelated recompute cannot postpone it (TRD §1-7 「recompute」 · §3 #91 — same interest draws no new generation).
            InterestAction::Unchanged
        } else if self.interest != self.sent {
            self.shrink_seq += 1;
            let gen = ShrinkGen(self.shrink_seq);
            self.shrink_pending = Some(gen);
            InterestAction::Defer {
                gen,
                deadline: now + USAGE_INTEREST_SHRINK_DELAY,
            }
        } else {
            self.shrink_pending = None;
            InterestAction::Unchanged
        }
    }

    /// Window `label` was destroyed (inside the same lock in which popup destruction cleanup erased it from the model) — erase the hidden mark and act as [`Self::recompute`]. A label is not reused, so a left mark leaks one entry per pop-out closed while hidden.
    pub(super) fn forget_window(
        &mut self,
        label: &str,
        mgr: &ViewManager,
        now: Instant,
    ) -> InterestAction {
        self.hidden.remove(label);
        self.recompute(mgr, now)
    }

    pub(super) fn set_visible(
        &mut self,
        label: &str,
        visible: bool,
        mgr: &ViewManager,
        now: Instant,
    ) -> InterestAction {
        if visible {
            self.hidden.remove(label);
        } else {
            self.hidden.insert(label.to_owned());
        }
        self.recompute(mgr, now)
    }

    /// The reduction timer calls it at the deadline; `true` = that reduction was still valid → one nudge. The wait ends at the deadline, not at delivery — even if that nudge disappears (command channel saturated), the next snapshot's cross-check compares with interest and fixes it ([`Self::on_snapshot`]).
    pub(super) fn deferral_elapsed(&mut self, gen: ShrinkGen) -> bool {
        if self.shrink_pending == Some(gen) {
            self.shrink_pending = None;
            true
        } else {
            false
        }
    }

    /// The `UsageSubscribe` set the connection driver writes on connection `socket_epoch` — `None` = nothing to write.
    /// - `force` = right before a ⟳ refresh frame — always gives the interest (the daemon is idempotent for the same set).
    /// - When a nudge is taken (`force = false`): with a pending shrink, only if the interest has a vendor outside `sent` (a pure shrink is the deadline's job); otherwise only if interest ≠ `sent`.
    /// `Some` means `sent := interest` and the pending shrink is void. Based on `sent`, repeated calls give `None` from the second on (idempotent — the nudge right after a connection opens relies on it). Not the current connection → `None`.
    pub(super) fn sync(
        &mut self,
        socket_epoch: u64,
        force: bool,
    ) -> Option<BTreeSet<AgentBackendKind>> {
        if !self.is_current(socket_epoch) {
            self.log_foreign("sync", socket_epoch);
            return None;
        }
        let send = force
            || if self.shrink_pending.is_some() {
                !self.interest.is_subset(&self.sent)
            } else {
                self.interest != self.sent
            };
        if !send {
            return None;
        }
        self.sent = self.interest.clone();
        self.shrink_pending = None;
        Some(self.interest.clone())
    }

    /// Called after the new socket opened the command endpoint, before reading the command channel — `Some` = one frame of that set directly to the new socket. It moves only on a marker larger than the largest seen (otherwise `None`, nothing changes); then it empties the cache, `sent := interest`, a waiting reduction becomes invalid, and an empty interest sends nothing (`None` — a new connection's daemon-side subscription is the empty set).
    pub(super) fn on_socket_open(
        &mut self,
        socket_epoch: u64,
    ) -> Option<BTreeSet<AgentBackendKind>> {
        if socket_epoch <= self.max_seen_socket_epoch {
            tracing::debug!(
                socket_epoch,
                max_seen = self.max_seen_socket_epoch,
                "usage interest: open of an already seen socket marker — ignored (task displaced by succession)"
            );
            return None;
        }
        self.max_seen_socket_epoch = socket_epoch;
        self.socket_epoch = socket_epoch;
        self.cache.clear();
        self.sent = self.interest.clone();
        self.shrink_pending = None;
        (!self.interest.is_empty()).then(|| self.interest.clone())
    }

    /// Socket `socket_epoch` ended (any reason). Only if it is the current socket: empty the cache, `sent := ∅`, no socket. A waiting reduction is invalid too — the `sent` to reduce is gone, and a new socket sends the interest in full on open.
    pub(super) fn on_socket_lost(&mut self, socket_epoch: u64) {
        if !self.is_current(socket_epoch) {
            self.log_foreign("on_socket_lost", socket_epoch);
            return;
        }
        self.cache.clear();
        self.sent.clear();
        self.shrink_pending = None;
        self.socket_epoch = 0;
    }

    /// One snapshot arrived on socket `socket_epoch` — `subscribed` = the subscription set of that connection the daemon carried in that snapshot. Not the current socket → nothing (empty result · cache unchanged).
    /// - Cache = highest revision per company — a smaller revision arriving late does not change it, an equal or larger one does (receipt time = `now`). The daemon sends the first snapshot outside the registry lock, so the last to arrive may not be the highest.
    /// - Cross-check: wanted set = `sent` if a reduction is waiting, else interest. If `subscribed` differs, `resend` = the wanted set; with a reduction waiting the wait · `sent` are left alone (a stale snapshot inside the 1.5 s window must not pull the reduction forward), else `sent := interest`.
    /// - Resend on every mismatch — do not suppress: suppressing the same pair let a stale record from an earlier toggle round trip and a rejected correction resend bring back the same pair and block a needed resend (TRD §3 #84). The feedback is finite because the daemon's same-set replacement emits no first snapshot. One `debug` line per mismatch.
    pub(super) fn on_snapshot(
        &mut self,
        socket_epoch: u64,
        snapshot: &UsageLimitSnapshot,
        subscribed: &[AgentBackendKind],
        now: Instant,
    ) -> SnapshotOutcome {
        if !self.is_current(socket_epoch) {
            self.log_foreign("on_snapshot", socket_epoch);
            return SnapshotOutcome::default();
        }
        let older = self
            .cache
            .get(&snapshot.vendor)
            .is_some_and(|cached| cached.snapshot.revision > snapshot.revision);
        if !older {
            self.cache.insert(
                snapshot.vendor,
                Cached {
                    snapshot: snapshot.clone(),
                    received: now,
                },
            );
        }
        let subscribed: BTreeSet<AgentBackendKind> = subscribed.iter().copied().collect();
        let shrink_pending = self.shrink_pending.is_some();
        let desired = if shrink_pending {
            self.sent.clone()
        } else {
            self.interest.clone()
        };
        let resend = if subscribed == desired {
            None
        } else {
            tracing::debug!(
                ?subscribed,
                ?desired,
                shrink_pending,
                socket_epoch,
                "usage subscription mismatch — resending the wanted set"
            );
            if !shrink_pending {
                self.sent = desired.clone();
            }
            Some(desired)
        };
        SnapshotOutcome {
            labels: self.usage_windows.iter().cloned().collect(),
            resend,
        }
    }

    /// Called by a webview pull — the whole cache with handoff correction (no daemon round trip). Handoff correction shifts relative fields by the whole seconds `d` since receipt: `age_secs + d` per window (per model included), `Failed.next_attempt_in_secs` · `Rejected.retry_in_secs` get `− d` (saturating at 0). Without it an old cache (up to 15 minutes) would show values that much younger · waits that much longer.
    pub fn snapshot_for_webview(&self, now: Instant) -> WebviewUsageSnapshot {
        WebviewUsageSnapshot {
            socket_epoch: self.socket_epoch,
            snapshots: self
                .cache
                .values()
                .map(|cached| {
                    handed_off(
                        &cached.snapshot,
                        now.saturating_duration_since(cached.received).as_secs(),
                    )
                })
                .collect(),
        }
    }

    fn is_current(&self, socket_epoch: u64) -> bool {
        socket_epoch != 0 && socket_epoch == self.socket_epoch
    }

    fn log_foreign(&self, call: &'static str, socket_epoch: u64) {
        tracing::debug!(
            call,
            socket_epoch,
            current = self.socket_epoch,
            "usage interest: marker is not the current socket — ignored"
        );
    }
}

fn handed_off(snapshot: &UsageLimitSnapshot, elapsed_secs: u64) -> UsageLimitSnapshot {
    let mut out = snapshot.clone();
    let age = |window: &mut UsageWindow| {
        window.age_secs = window.age_secs.saturating_add(elapsed_secs);
    };
    out.five_hour.iter_mut().for_each(age);
    out.weekly.iter_mut().for_each(age);
    out.model_scoped
        .iter_mut()
        .for_each(|scoped| age(&mut scoped.window));
    // No wildcard arm on purpose — adding a state that carries relative seconds must break the compile here.
    match &mut out.state {
        UsageVendorState::Failed {
            next_attempt_in_secs,
            ..
        } => *next_attempt_in_secs = next_attempt_in_secs.saturating_sub(elapsed_secs),
        UsageVendorState::Rejected { retry_in_secs, .. } => {
            *retry_in_secs = retry_in_secs.saturating_sub(elapsed_secs)
        }
        UsageVendorState::Ready
        | UsageVendorState::NotInstalled { .. }
        | UsageVendorState::NeedsLogin { .. }
        | UsageVendorState::Unavailable { .. } => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use engram_dashboard_protocol::UsageScopedWindow;
    use uuid::Uuid;

    use super::*;
    use crate::layout::manager::ViewId;
    use crate::layout::{tree, SplitDir, MAIN_WINDOW_LABEL};

    use AgentBackendKind::{Claude, Codex};

    const POPUP: &str = "slot-popup-1";

    fn set(vendors: &[AgentBackendKind]) -> BTreeSet<AgentBackendKind> {
        vendors.iter().copied().collect()
    }

    fn labels(names: &[&str]) -> BTreeSet<WindowLabel> {
        names.iter().map(|n| n.to_string()).collect()
    }

    fn usage(show_claude: bool, show_codex: bool) -> SlotContent {
        SlotContent::Usage {
            show_claude,
            show_codex,
        }
    }

    fn root_slot(mgr: &ViewManager, view: ViewId) -> Uuid {
        tree::first_slot_id(&mgr.views[&view].layout)
    }

    fn put(mgr: &mut ViewManager, view: ViewId, content: SlotContent) -> Uuid {
        let slot = root_slot(mgr, view);
        mgr.set_slot_content(view, slot, content).unwrap();
        slot
    }

    fn main_active(mgr: &ViewManager) -> ViewId {
        mgr.windows[MAIN_WINDOW_LABEL].active
    }

    fn main_with(show_claude: bool, show_codex: bool) -> (ViewManager, Uuid) {
        let mut mgr = ViewManager::new();
        let view = main_active(&mgr);
        let slot = put(&mut mgr, view, usage(show_claude, show_codex));
        (mgr, slot)
    }

    fn window(age_secs: u64) -> UsageWindow {
        UsageWindow {
            used_pct: Some(10.0),
            resets_at: Some(1_900_000_000),
            age_secs,
            expired: false,
        }
    }

    fn snap(vendor: AgentBackendKind, revision: u64) -> UsageLimitSnapshot {
        UsageLimitSnapshot {
            vendor,
            account_key: "default".to_string(),
            five_hour: Some(window(10)),
            weekly: Some(window(20)),
            model_scoped: vec![UsageScopedWindow {
                label: "opus".to_string(),
                window: window(30),
            }],
            plan: None,
            in_flight: false,
            state: UsageVendorState::Ready,
            revision,
        }
    }

    fn connected(mgr: &ViewManager, t0: Instant) -> UsageInterest {
        let mut ui = UsageInterest::new();
        let _ = ui.recompute(mgr, t0);
        ui.on_socket_open(1);
        ui
    }

    fn defer_gen(action: InterestAction) -> ShrinkGen {
        match action {
            InterestAction::Defer { gen, .. } => gen,
            other => panic!("should be a deferred reduction: {other:?}"),
        }
    }

    // ── layout → interest ──────────────────────────────────────────────────────────

    #[test]
    fn the_active_tab_counts_and_an_inactive_tab_does_not() {
        let (mut mgr, _) = main_with(true, false);
        let usage_tab = main_active(&mgr);
        assert_eq!(interest_from_layout(&mgr, &labels(&[])), set(&[Claude]));

        mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        assert_eq!(interest_from_layout(&mgr, &labels(&[])), set(&[]));
        assert_eq!(
            usage_slot_windows(&mgr),
            labels(&[MAIN_WINDOW_LABEL]),
            "a usage slot in a non-active tab still receives broadcasts"
        );

        mgr.switch_tab(MAIN_WINDOW_LABEL, usage_tab).unwrap();
        assert_eq!(interest_from_layout(&mgr, &labels(&[])), set(&[Claude]));
    }

    #[test]
    fn only_the_hidden_mark_drops_a_window() {
        let (mgr, _) = main_with(true, true);
        assert_eq!(
            interest_from_layout(&mgr, &labels(&[])),
            set(&[Claude, Codex])
        );
        assert_eq!(
            interest_from_layout(&mgr, &labels(&[MAIN_WINDOW_LABEL])),
            set(&[])
        );
        assert_eq!(usage_slot_windows(&mgr), labels(&[MAIN_WINDOW_LABEL]));
    }

    #[test]
    fn two_windows_union_their_vendors() {
        let (mut mgr, _) = main_with(true, false);
        let popup_view = mgr.create_window(POPUP).unwrap();
        put(&mut mgr, popup_view, usage(false, true));
        assert_eq!(
            interest_from_layout(&mgr, &labels(&[])),
            set(&[Claude, Codex])
        );
        assert_eq!(
            interest_from_layout(&mgr, &labels(&[POPUP])),
            set(&[Claude])
        );
        assert_eq!(
            usage_slot_windows(&mgr),
            labels(&[MAIN_WINDOW_LABEL, POPUP])
        );
    }

    #[test]
    fn two_slots_in_one_tab_union_and_a_toggle_off_drops_its_vendor() {
        let (mut mgr, slot) = main_with(true, false);
        let view = main_active(&mgr);
        let other = mgr.split_slot(view, slot, SplitDir::LeftRight).unwrap();
        mgr.set_slot_content(view, other, usage(false, true))
            .unwrap();
        assert_eq!(
            interest_from_layout(&mgr, &labels(&[])),
            set(&[Claude, Codex])
        );

        mgr.set_slot_content(view, other, usage(false, false))
            .unwrap();
        assert_eq!(interest_from_layout(&mgr, &labels(&[])), set(&[Claude]));
        assert_eq!(
            usage_slot_windows(&mgr),
            labels(&[MAIN_WINDOW_LABEL]),
            "a usage slot with both switched off is still a usage slot"
        );
    }

    #[test]
    fn a_view_owned_by_no_window_drops_out() {
        let (mut mgr, slot) = main_with(true, true);
        let view = main_active(&mgr);
        mgr.prepare_detached_view(view, slot).unwrap();
        mgr.set_slot_content(view, slot, SlotContent::Empty)
            .unwrap();
        assert_eq!(interest_from_layout(&mgr, &labels(&[])), set(&[]));
        assert_eq!(usage_slot_windows(&mgr), labels(&[]));
    }

    #[test]
    fn a_closed_popup_drops_out_and_its_hidden_mark_is_forgotten() {
        let t0 = Instant::now();
        let mut mgr = ViewManager::new();
        let popup_view = mgr.create_window(POPUP).unwrap();
        put(&mut mgr, popup_view, usage(true, false));
        let mut ui = connected(&mgr, t0);
        assert_eq!(ui.sent, set(&[Claude]));

        assert!(matches!(
            ui.set_visible(POPUP, false, &mgr, t0),
            InterestAction::Defer { .. }
        ));
        assert!(ui.hidden.contains(POPUP));

        mgr.close_window(POPUP).unwrap();
        let _ = ui.recompute(&mgr, t0);
        assert!(ui.hidden.contains(POPUP), "recount does not erase hidden marks");
        let _ = ui.forget_window(POPUP, &mgr, t0);
        assert!(ui.hidden.is_empty(), "the hidden mark of a destroyed window remained");
        assert_eq!(ui.interest, set(&[]));
        assert!(ui.usage_windows.is_empty());
    }

    /// Moving to a new window opens the OS window first, releases the lock, then adds the model window — the tray hide in that gap must survive so the pop-out still counts as hidden once the model window stands.
    #[test]
    fn a_hide_mark_before_the_model_window_exists_survives_until_forgotten() {
        let t0 = Instant::now();
        let mut mgr = ViewManager::new();
        let mut ui = connected(&mgr, t0);

        assert_eq!(
            ui.set_visible(POPUP, false, &mgr, t0),
            InterestAction::Unchanged
        );
        let _ = ui.recompute(&mgr, t0);
        assert!(
            ui.hidden.contains(POPUP),
            "the mark of a label not in the model was erased"
        );

        let popup_view = mgr.create_window(POPUP).unwrap();
        put(&mut mgr, popup_view, usage(true, true));
        assert_eq!(ui.recompute(&mgr, t0), InterestAction::Unchanged);
        assert_eq!(ui.interest, set(&[]), "a hidden pop-out was counted as visible");
        assert_eq!(ui.usage_windows, labels(&[POPUP]));

        mgr.close_window(POPUP).unwrap();
        let _ = ui.forget_window(POPUP, &mgr, t0);
        assert!(ui.hidden.is_empty());
    }

    // ── growth · shrinkage (fake time) ─────────────────────────────────────────────────

    #[test]
    fn growth_nudges_at_once_and_sync_sends_it_once() {
        let t0 = Instant::now();
        let mut mgr = ViewManager::new();
        let mut ui = connected(&mgr, t0);
        assert_eq!(ui.sync(1, false), None, "empty interest = nothing to send");

        let view = main_active(&mgr);
        put(&mut mgr, view, usage(true, false));
        assert_eq!(ui.recompute(&mgr, t0), InterestAction::Nudge);
        assert_eq!(ui.sync(1, false), Some(set(&[Claude])));
        assert_eq!(ui.sent, set(&[Claude]));
        assert_eq!(ui.sync(1, false), None, "based on sent, so nothing from the second call on (idempotent)");
    }

    #[test]
    fn shrink_waits_for_its_deadline_and_then_sends_once() {
        let t0 = Instant::now();
        let (mut mgr, slot) = main_with(true, true);
        let mut ui = connected(&mgr, t0);
        let view = main_active(&mgr);

        mgr.set_slot_content(view, slot, usage(true, false))
            .unwrap();
        let action = ui.recompute(&mgr, t0);
        let InterestAction::Defer { gen, deadline } = action else {
            panic!("should be a deferred reduction: {action:?}");
        };
        assert_eq!(deadline, t0 + USAGE_INTEREST_SHRINK_DELAY);
        assert_eq!(USAGE_INTEREST_SHRINK_DELAY, Duration::from_millis(1500));
        assert_eq!(ui.sync(1, false), None, "before the deadline = nothing sent");
        assert_eq!(ui.sent, set(&[Claude, Codex]));

        assert!(ui.deferral_elapsed(gen));
        assert_eq!(ui.sync(1, false), Some(set(&[Claude])));
        assert!(!ui.deferral_elapsed(gen), "a generation ends only once");
        assert_eq!(ui.sync(1, false), None);
    }

    /// If it returns to `sent` within the deadline, like hiding to the tray and reviving · closing a slot and soon putting it back, nothing goes to the daemon.
    #[test]
    fn returning_to_sent_within_the_delay_cancels_the_shrink() {
        let t0 = Instant::now();
        let (mut mgr, _) = main_with(true, false);
        let mut ui = connected(&mgr, t0);

        let gen = defer_gen(ui.set_visible(MAIN_WINDOW_LABEL, false, &mgr, t0));
        let later = t0 + Duration::from_millis(700);
        assert_eq!(
            ui.set_visible(MAIN_WINDOW_LABEL, true, &mgr, later),
            InterestAction::Unchanged
        );
        assert_eq!(ui.shrink_pending, None);
        assert!(!ui.deferral_elapsed(gen), "late deadline of a pending reduction that became invalid");
        assert_eq!(ui.sync(1, false), None);

        // The layout side is the same — if a slot is emptied (reduction) and put back in another window within 1.5 seconds (restored), nothing goes.
        let view = main_active(&mgr);
        let slot = root_slot(&mgr, view);
        mgr.set_slot_content(view, slot, SlotContent::Empty)
            .unwrap();
        let gen = defer_gen(ui.recompute(&mgr, later));
        let popup_view = mgr.create_window(POPUP).unwrap();
        put(&mut mgr, popup_view, usage(true, false));
        assert_eq!(ui.recompute(&mgr, later), InterestAction::Unchanged);
        assert!(!ui.deferral_elapsed(gen));
        assert_eq!(ui.sync(1, false), None);
    }

    #[test]
    fn a_newer_shrink_supersedes_the_older_timer() {
        let t0 = Instant::now();
        let (mut mgr, slot) = main_with(true, true);
        let mut ui = connected(&mgr, t0);
        let view = main_active(&mgr);

        mgr.set_slot_content(view, slot, usage(true, false))
            .unwrap();
        let first = defer_gen(ui.recompute(&mgr, t0));
        mgr.set_slot_content(view, slot, usage(false, false))
            .unwrap();
        let later = t0 + Duration::from_secs(1);
        let rearmed = ui.recompute(&mgr, later);
        assert!(
            matches!(rearmed, InterestAction::Defer { deadline, .. } if deadline == later + USAGE_INTEREST_SHRINK_DELAY),
            "if it shrinks further it re-arms with a new deadline: {rearmed:?}"
        );
        let second = defer_gen(rearmed);
        assert_ne!(first, second);
        assert!(!ui.deferral_elapsed(first));
        assert!(ui.deferral_elapsed(second));
        assert_eq!(ui.sync(1, false), Some(set(&[])));
    }

    /// A recompute yielding the same interest as a waiting reduction (an unrelated layout change) leaves the generation · deadline alone — successive recomputes cannot postpone the reduction.
    #[test]
    fn unrelated_recomputes_during_a_pending_shrink_do_not_postpone_it() {
        let t0 = Instant::now();
        let (mut mgr, slot) = main_with(true, true);
        let mut ui = connected(&mgr, t0);
        let view = main_active(&mgr);

        mgr.set_slot_content(view, slot, usage(true, false))
            .unwrap();
        let gen = defer_gen(ui.recompute(&mgr, t0));
        for step in 1..=3u64 {
            mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
            mgr.switch_tab(MAIN_WINDOW_LABEL, view).unwrap();
            assert_eq!(
                ui.recompute(&mgr, t0 + Duration::from_millis(500 * step)),
                InterestAction::Unchanged,
                "recount {step}"
            );
        }
        assert_eq!(ui.shrink_pending, Some(gen));
        assert!(ui.deferral_elapsed(gen), "the first generation ends at the deadline as is");
        assert_eq!(ui.sync(1, false), Some(set(&[Claude])));
    }

    #[test]
    fn growth_during_a_pending_shrink_cancels_it_and_nudges() {
        let t0 = Instant::now();
        let (mut mgr, slot) = main_with(true, false);
        let mut ui = connected(&mgr, t0);
        let view = main_active(&mgr);

        mgr.set_slot_content(view, slot, usage(false, false))
            .unwrap();
        let gen = defer_gen(ui.recompute(&mgr, t0));
        mgr.set_slot_content(view, slot, usage(false, true))
            .unwrap();
        assert_eq!(ui.recompute(&mgr, t0), InterestAction::Nudge);
        assert!(!ui.deferral_elapsed(gen));
        assert_eq!(ui.sync(1, false), Some(set(&[Codex])));
    }

    #[test]
    fn hiding_defers_but_keeps_the_window_in_labels_and_showing_nudges() {
        let t0 = Instant::now();
        let (mgr, _) = main_with(true, false);
        let mut ui = connected(&mgr, t0);

        let gen = defer_gen(ui.set_visible(MAIN_WINDOW_LABEL, false, &mgr, t0));
        assert_eq!(ui.interest, set(&[]));
        let out = ui.on_snapshot(1, &snap(Claude, 1), &[Claude], t0);
        assert_eq!(
            out.labels,
            vec![MAIN_WINDOW_LABEL.to_string()],
            "a hidden window still receives broadcasts"
        );
        assert!(ui.deferral_elapsed(gen));
        assert_eq!(ui.sync(1, false), Some(set(&[])));

        assert_eq!(
            ui.set_visible(MAIN_WINDOW_LABEL, true, &mgr, t0),
            InterestAction::Nudge
        );
        assert_eq!(ui.sync(1, false), Some(set(&[Claude])));
    }

    /// One serial path ① — even if the nudges of two recomputes are handled crossed, the last one on the socket = the last interest.
    #[test]
    fn crossed_nudges_still_leave_the_latest_interest_last() {
        let t0 = Instant::now();
        let mut mgr = ViewManager::new();
        let mut ui = connected(&mgr, t0);
        let view = main_active(&mgr);

        let slot = put(&mut mgr, view, usage(true, false));
        assert_eq!(ui.recompute(&mgr, t0), InterestAction::Nudge); // nudge A
        mgr.set_slot_content(view, slot, usage(true, true)).unwrap();
        assert_eq!(ui.recompute(&mgr, t0), InterestAction::Nudge); // nudge B

        let mut wire = Vec::new();
        // In whatever order the connection task takes out the two nudges, what it reads is the then-current interest.
        wire.extend(ui.sync(1, false));
        wire.extend(ui.sync(1, false));
        assert_eq!(wire, vec![set(&[Claude, Codex])]);
    }

    // ── cross-check (subscribed carried by the daemon) ───────────────────────────────────────────

    #[test]
    fn a_mismatch_without_a_pending_shrink_resends_the_interest() {
        let t0 = Instant::now();
        let (mut mgr, slot) = main_with(true, false);
        let mut ui = connected(&mgr, t0);
        let view = main_active(&mgr);
        mgr.set_slot_content(view, slot, usage(true, true)).unwrap();
        assert_eq!(ui.recompute(&mgr, t0), InterestAction::Nudge);
        // The nudge disappeared (command channel saturated) — the daemon still subscribes only to claude.
        let out = ui.on_snapshot(1, &snap(Claude, 1), &[Claude], t0);
        assert_eq!(out.resend, Some(set(&[Claude, Codex])));
        assert_eq!(ui.sent, set(&[Claude, Codex]));
        assert_eq!(ui.sync(1, false), None, "the reconcile already sent it");
    }

    #[test]
    fn a_match_resends_nothing() {
        let t0 = Instant::now();
        let (mgr, _) = main_with(true, true);
        let mut ui = connected(&mgr, t0);
        let out = ui.on_snapshot(1, &snap(Codex, 1), &[Codex, Claude], t0);
        assert_eq!(
            out.resend, None,
            "first snapshot of a newly added company (subscribed = interest)"
        );
        assert_eq!(out.labels, vec![MAIN_WINDOW_LABEL.to_string()]);
    }

    /// One serial path ② — a broadcast of a company about to drop out during the reduction delay is legitimate, since the daemon still subscribes to the old `sent`.
    #[test]
    fn during_a_pending_shrink_subscribed_equal_to_sent_is_left_alone() {
        let t0 = Instant::now();
        let (mut mgr, slot) = main_with(true, true);
        let mut ui = connected(&mgr, t0);
        mgr.set_slot_content(main_active(&mgr), slot, usage(true, false))
            .unwrap();
        let gen = defer_gen(ui.recompute(&mgr, t0));

        let out = ui.on_snapshot(1, &snap(Codex, 2), &[Claude, Codex], t0);
        assert_eq!(out.resend, None);
        assert_eq!(ui.shrink_pending, Some(gen));
        assert_eq!(ui.sent, set(&[Claude, Codex]));
    }

    /// A stale snapshot that arrives inside the 1.5-second window only resends `sent` and does not pull the reduction forward.
    #[test]
    fn during_a_pending_shrink_a_mismatch_resends_sent_and_keeps_waiting() {
        let t0 = Instant::now();
        let (mut mgr, slot) = main_with(true, true);
        let mut ui = connected(&mgr, t0);
        mgr.set_slot_content(main_active(&mgr), slot, usage(true, false))
            .unwrap();
        let gen = defer_gen(ui.recompute(&mgr, t0));

        let out = ui.on_snapshot(1, &snap(Claude, 1), &[Claude], t0);
        assert_eq!(out.resend, Some(set(&[Claude, Codex])));
        assert_eq!(ui.shrink_pending, Some(gen), "the pending reduction stays");
        assert_eq!(ui.sent, set(&[Claude, Codex]));
        assert!(ui.deferral_elapsed(gen));
        assert_eq!(ui.sync(1, false), Some(set(&[Claude])));
    }

    /// The reduction nudge disappeared after the deadline — the next snapshot of the company about to drop out fixes it.
    #[test]
    fn a_lost_shrink_nudge_is_healed_by_the_next_snapshot() {
        let t0 = Instant::now();
        let (mut mgr, slot) = main_with(true, true);
        let mut ui = connected(&mgr, t0);
        mgr.set_slot_content(main_active(&mgr), slot, usage(true, false))
            .unwrap();
        let gen = defer_gen(ui.recompute(&mgr, t0));
        assert!(ui.deferral_elapsed(gen)); // One nudge → discarded by channel saturation (no sync)

        let out = ui.on_snapshot(1, &snap(Codex, 3), &[Claude, Codex], t0);
        assert_eq!(out.resend, Some(set(&[Claude])));
        assert_eq!(ui.sent, set(&[Claude]));
    }

    /// Persistent mismatch — if the same `subscribed` comes three times, it is resent three times, with one debug line per snapshot.
    #[test]
    fn a_persistent_mismatch_resends_every_time_and_logs_each() {
        let t0 = Instant::now();
        let (mgr, _) = main_with(true, true);
        let mut ui = connected(&mgr, t0);

        let lines = Arc::new(Mutex::new(Vec::new()));
        let mismatches = |lines: &Mutex<Vec<String>>| {
            lines
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m.contains("usage subscription mismatch"))
                .count()
        };
        let resends = tracing::subscriber::with_default(DebugLines(lines.clone()), || {
            // The callsite interest is a process-global cache: if another test thread registered the same callsite before this subscriber stood, the cache stays "not received" after that registration ends (0 lines in a parallel run). So rebuild the cache and re-poke until one line is caught, then start counting.
            for rev in 1..=100 {
                tracing::callsite::rebuild_interest_cache();
                ui.on_snapshot(1, &snap(Claude, rev), &[Claude], t0);
                if mismatches(&lines) > 0 {
                    break;
                }
            }
            assert!(mismatches(&lines) > 0, "cannot catch the debug line");
            lines.lock().unwrap().clear();
            // The three counted ones also rebuild the cache each time — the race above can occur again even after one has been caught.
            (101..=103)
                .map(|rev| {
                    tracing::callsite::rebuild_interest_cache();
                    ui.on_snapshot(1, &snap(Claude, rev), &[Claude], t0).resend
                })
                .collect::<Vec<_>>()
        });
        assert_eq!(resends, vec![Some(set(&[Claude, Codex])); 3]);
        assert_eq!(mismatches(&lines), 3);
    }

    #[test]
    fn a_snapshot_of_an_older_socket_is_not_reconciled_or_cached() {
        let t0 = Instant::now();
        let (mgr, _) = main_with(true, false);
        let mut ui = connected(&mgr, t0);
        assert!(ui.on_socket_open(2).is_some());

        let out = ui.on_snapshot(1, &snap(Claude, 9), &[], t0);
        assert_eq!(out, SnapshotOutcome::default());
        assert!(ui.cache.is_empty());
    }

    // ── ⟳ force ────────────────────────────────────────────────────────────────

    #[test]
    fn force_sends_even_when_sent_matches_and_cancels_a_pending_shrink() {
        let t0 = Instant::now();
        let (mut mgr, slot) = main_with(true, true);
        let mut ui = connected(&mgr, t0);
        assert_eq!(ui.sync(1, true), Some(set(&[Claude, Codex])));

        mgr.set_slot_content(main_active(&mgr), slot, usage(false, true))
            .unwrap();
        let gen = defer_gen(ui.recompute(&mgr, t0));
        assert_eq!(ui.sync(1, true), Some(set(&[Codex])));
        assert_eq!(ui.sent, set(&[Codex]));
        assert!(!ui.deferral_elapsed(gen));
        assert_eq!(ui.sync(2, true), None, "no force either when it is not the current socket");
    }

    // ── cache · handoff correction ─────────────────────────────────────────────────────

    #[test]
    fn the_cache_keeps_the_highest_revision_and_equal_replaces() {
        let t0 = Instant::now();
        let (mgr, _) = main_with(true, false);
        let mut ui = connected(&mgr, t0);

        let mut newer = snap(Claude, 5);
        newer.plan = Some("max".to_string());
        ui.on_snapshot(1, &newer, &[Claude], t0);
        let out = ui.on_snapshot(1, &snap(Claude, 4), &[Claude], t0);
        assert_eq!(
            out.labels,
            vec![MAIN_WINDOW_LABEL.to_string()],
            "delivery is independent of revision"
        );
        assert_eq!(ui.snapshot_for_webview(t0).snapshots, vec![newer]);

        let t1 = t0 + Duration::from_secs(40);
        let mut same = snap(Claude, 5);
        same.plan = Some("pro".to_string());
        ui.on_snapshot(1, &same, &[Claude], t1);
        assert_eq!(
            ui.snapshot_for_webview(t1).snapshots,
            vec![same],
            "same revision = newly received (received time renewed too)"
        );
    }

    #[test]
    fn the_handoff_moves_relative_seconds_by_the_time_since_receipt() {
        let t0 = Instant::now();
        let (mgr, _) = main_with(true, true);
        let mut ui = connected(&mgr, t0);

        let mut claude = snap(Claude, 1);
        claude.state = UsageVendorState::Failed {
            next_attempt_in_secs: 60,
            detail: None,
        };
        let mut codex = snap(Codex, 1);
        codex.state = UsageVendorState::Rejected {
            retry_in_secs: 600,
            detail: None,
        };
        ui.on_snapshot(1, &claude, &[Claude, Codex], t0);
        ui.on_snapshot(1, &codex, &[Claude, Codex], t0);

        let pulled = ui.snapshot_for_webview(t0 + Duration::from_millis(90_900));
        assert_eq!(pulled.socket_epoch, 1);
        let [c, x] = pulled.snapshots.as_slice() else {
            panic!("two companies: {pulled:?}");
        };
        assert_eq!(c.vendor, Claude);
        assert_eq!(c.five_hour.as_ref().unwrap().age_secs, 100);
        assert_eq!(c.weekly.as_ref().unwrap().age_secs, 110);
        assert_eq!(c.model_scoped[0].window.age_secs, 120);
        assert_eq!(
            c.state,
            UsageVendorState::Failed {
                next_attempt_in_secs: 0,
                detail: None
            },
            "60 - 90 saturates to 0"
        );
        assert_eq!(x.vendor, Codex);
        assert_eq!(
            x.state,
            UsageVendorState::Rejected {
                retry_in_secs: 510,
                detail: None
            }
        );
        assert_eq!(
            ui.snapshot_for_webview(t0).snapshots[0]
                .five_hour
                .as_ref()
                .unwrap()
                .age_secs,
            10,
            "the cache original is not worn down by the correction"
        );
    }

    #[test]
    fn without_a_socket_the_pull_is_empty_with_epoch_zero() {
        let t0 = Instant::now();
        let (mgr, _) = main_with(true, false);
        let mut ui = UsageInterest::new();
        assert_eq!(
            ui.snapshot_for_webview(t0),
            WebviewUsageSnapshot {
                socket_epoch: 0,
                snapshots: vec![]
            }
        );

        let _ = ui.recompute(&mgr, t0);
        ui.on_socket_open(1);
        ui.on_snapshot(1, &snap(Claude, 1), &[Claude], t0);
        ui.on_socket_lost(1);
        assert_eq!(
            ui.snapshot_for_webview(t0),
            WebviewUsageSnapshot {
                socket_epoch: 0,
                snapshots: vec![]
            }
        );
    }

    // ── socket marker ─────────────────────────────────────────────────────────────

    #[test]
    fn a_new_socket_clears_the_cache_and_sends_the_interest() {
        let t0 = Instant::now();
        let (mgr, _) = main_with(true, false);
        let mut ui = connected(&mgr, t0);
        ui.on_snapshot(1, &snap(Claude, 7), &[Claude], t0);

        assert_eq!(ui.on_socket_open(3), Some(set(&[Claude])));
        assert_eq!(ui.socket_epoch, 3);
        assert_eq!(ui.sent, set(&[Claude]));
        assert!(ui.cache.is_empty());

        let empty = ViewManager::new();
        let mut idle = UsageInterest::new();
        let _ = idle.recompute(&empty, t0);
        assert_eq!(idle.on_socket_open(1), None, "empty interest has nothing to send");
        assert_eq!(idle.socket_epoch, 1);
    }

    #[test]
    fn an_open_not_above_the_highest_seen_epoch_changes_nothing() {
        let t0 = Instant::now();
        let (mgr, _) = main_with(true, false);
        let mut ui = UsageInterest::new();
        let _ = ui.recompute(&mgr, t0);
        assert!(ui.on_socket_open(5).is_some());
        ui.on_snapshot(5, &snap(Claude, 1), &[Claude], t0);
        ui.on_socket_lost(5);
        assert!(ui.on_socket_open(6).is_some());
        ui.on_snapshot(6, &snap(Claude, 2), &[Claude], t0);
        let sent_before = ui.sent.clone();

        for stale in [5, 6, 0] {
            assert_eq!(ui.on_socket_open(stale), None, "marker {stale}");
        }
        assert_eq!(ui.socket_epoch, 6);
        assert_eq!(ui.sent, sent_before);
        assert_eq!(ui.cache.len(), 1);
    }

    #[test]
    fn calls_carrying_an_older_socket_epoch_do_nothing() {
        let t0 = Instant::now();
        let (mut mgr, slot) = main_with(true, false);
        let mut ui = connected(&mgr, t0);
        assert!(ui.on_socket_open(2).is_some());
        ui.on_snapshot(2, &snap(Claude, 1), &[Claude], t0);

        mgr.set_slot_content(main_active(&mgr), slot, usage(true, true))
            .unwrap();
        let _ = ui.recompute(&mgr, t0);
        assert_eq!(ui.sync(1, false), None);
        assert_eq!(ui.sync(1, true), None);
        ui.on_socket_lost(1);
        assert_eq!(ui.socket_epoch, 2);
        assert_eq!(ui.cache.len(), 1);
        assert_eq!(ui.sent, set(&[Claude]));
        assert_eq!(ui.sync(2, false), Some(set(&[Claude, Codex])));
    }

    #[test]
    fn losing_the_socket_clears_the_cache_and_sent() {
        let t0 = Instant::now();
        let (mut mgr, slot) = main_with(true, true);
        let mut ui = connected(&mgr, t0);
        ui.on_snapshot(1, &snap(Codex, 1), &[Claude, Codex], t0);
        mgr.set_slot_content(main_active(&mgr), slot, usage(true, false))
            .unwrap();
        let gen = defer_gen(ui.recompute(&mgr, t0));

        ui.on_socket_lost(1);
        assert_eq!(ui.socket_epoch, 0);
        assert!(ui.cache.is_empty());
        assert!(ui.sent.is_empty());
        assert!(!ui.deferral_elapsed(gen), "the sent to reduce vanished");
        assert_eq!(
            ui.recompute(&mgr, t0),
            InterestAction::Nudge,
            "while disconnected, interest is the increase itself (the nudge is a no-op as the window is closed)"
        );
        assert_eq!(ui.sync(0, false), None, "does not match no-socket (0)");
        assert_eq!(ui.on_socket_open(2), Some(set(&[Claude])));
    }

    // ── log capture ─────────────────────────────────────────────────────────────

    /// This thread's debug line messages.
    struct DebugLines(Arc<Mutex<Vec<String>>>);

    struct Message<'a>(&'a mut String);

    impl tracing::field::Visit for Message<'_> {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                *self.0 = format!("{value:?}");
            }
        }
    }

    impl tracing::Subscriber for DebugLines {
        fn register_callsite(
            &self,
            _: &'static tracing::Metadata<'static>,
        ) -> tracing::subscriber::Interest {
            tracing::subscriber::Interest::sometimes()
        }
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            if *event.metadata().level() == tracing::Level::DEBUG {
                let mut message = String::new();
                event.record(&mut Message(&mut message));
                self.0.lock().unwrap().push(message);
            }
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }
}
