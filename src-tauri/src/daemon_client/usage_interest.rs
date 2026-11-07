//! The shell's usage interest — a state machine that computes the daemon usage subscription set from the layout and keeps that subscription in line with the daemon
//! (TRD S21 usage-limit-slot §1-7).
//!
//! ## What it owns
//! - **Interest** = the union of the companies turned on by the usage slots of the **active tab** of every window that has no hidden mark
//!   ([`interest_from_layout`]). Minimize is not tracked by the shell, so it counts as visible (user decision 2026-09-27). A view that is
//!   not any window's tab (the temporary view of a slot move) is excluded. A window left only in the model (a window left in the model without an OS window — the unresolved issue noted in
//!   the `WindowHost` doc of `layout::apply`) is counted as visible.
//! - **Windows to receive broadcasts** = windows that have a usage slot in any tab, hidden windows included ([`usage_slot_windows`]) — when shown again it is
//!   up to date right away.
//! - [`UsageInterest`] — the last values of those two · the set last sent on this socket (`sent`) · the reduction delay · the snapshot
//!   cache received on this socket.
//!
//! ## ★Pure — knows nothing of Tauri · sockets · tokio · clocks★
//! Time is supplied by the caller (`now`). Methods only return results, and socket writes · emit · timer starts are the caller's job —
//! so tests are deterministic with a fake time (`cargo test -p engram-dashboard --test lib_unit`).
//!
//! ## ★One serial path — only the connection task takes out the set to send★
//! The side that changes interest (layout recompute · hide · reduction timer) puts in only a nudge with no set, and when the connection task takes it out it
//! reads **the then-current** interest with [`UsageInterest::sync`] and sends — read order = send order, so the last one
//! on the socket is always the latest interest. The shape where the caller reads the set inside the lock and sends outside the lock lets the old set arrive last if two changes
//! cross between the read and the send — do not revive it (TRD §3 #74). So [`InterestAction`] does not
//! carry a set, and the connection-task-only methods (`sync`·`on_socket_open`·`on_socket_lost`·`on_snapshot`) are not opened outside
//! `daemon_client`.
//!
//! ## ★Socket epoch guard★
//! Every method the connection driver calls carries its socket epoch (`socket_epoch` — the transport connection
//! generation · ADR-0195 · `0` = no socket) and does nothing unless it is the current one (an open does nothing unless
//! it is greater than the largest seen) — so a late call for an older connection cannot overwrite the new one's
//! `sent` or cache. Ordering by size means something because generations never rewind (the transport mints them from
//! one counter per peer name within a registry), so it holds within one shell process.
//!
//! ## Lock order
//! The lock around this state comes after `ViewManager` (`super::SharedUsageInterest` doc). This file knows no lock.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use engram_dashboard_protocol::{
    AgentBackendKind, UsageLimitSnapshot, UsageVendorState, UsageWindow,
};

use crate::layout::manager::WindowLabel;
use crate::layout::{LayoutNode, SlotContent, ViewManager};

/// The time to wait before sending to the daemon when interest shrinks — a window to avoid sending to the daemon reductions that soon come back (going to another tab and returning · toggling a display switch
/// off and on · hiding to the tray and reviving · closing a slot·tab and putting it back within 1.5 seconds).
/// Growth is not waited for. ★Moving a slot to another window does not rely on this delay★ — the source slot
/// remains until the step that attaches it to the new window and is closed inside the same lock as that attaching, so interest does not shrink in the first place
/// (`layout::apply::move_slot_to_window`).
pub const USAGE_INTEREST_SHRINK_DELAY: Duration = Duration::from_millis(1500);

/// The union of the companies turned on by the usage slots of the **active tab** of every window that has no hidden mark.
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

/// Windows that have a usage slot in any tab (inactive tabs · hidden windows included) — slots with no company turned on are included too.
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

/// The generation of one reduction delay. If interest changes again during the delay, a new generation stands or the delay becomes invalid, and an old
/// timer that wakes late gets `false` in [`UsageInterest::deferral_elapsed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShrinkGen(u64);

/// What interest recomputation tells the caller to do — the result of comparing with `sent` (the set last sent on this socket). If discarded, growth does not
/// reach the daemon or the reduction never does.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterestAction {
    /// Nothing to do — interest = `sent` (a waiting reduction became invalid), or a reduction to the same interest is already waiting
    /// (its generation · deadline · timer are still valid).
    Unchanged,
    /// Interest has a company outside `sent` — one nudge right away.
    Nudge,
    /// Interest ⊊ `sent` — call [`UsageInterest::deferral_elapsed`]`(gen)` at `deadline`, and if `true`, one nudge.
    Defer { gen: ShrinkGen, deadline: Instant },
}

/// The result of [`UsageInterest::on_snapshot`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SnapshotOutcome {
    /// The windows to deliver that snapshot to (= the last value of [`usage_slot_windows`]). It is delivered regardless of revision — telling them apart is
    /// the webview's job. It is empty if it is not a snapshot of the current socket.
    pub labels: Vec<WindowLabel>,
    /// The `UsageSubscribe` set to write straight to the same socket — it stands only when the subscription set the daemon carried differs from the
    /// set the shell wants to have on the daemon. It stands every time it differs (no suppression — [`UsageInterest::on_snapshot`]).
    pub resend: Option<BTreeSet<AgentBackendKind>>,
}

/// What to hand to a webview pull — the whole cache (after handoff correction, in company order) and the current socket marker. If there is no socket, `0` and an empty list.
#[derive(Debug, Clone, PartialEq)]
pub struct WebviewUsageSnapshot {
    pub socket_epoch: u64,
    pub snapshots: Vec<UsageLimitSnapshot>,
}

#[derive(Debug)]
struct Cached {
    snapshot: UsageLimitSnapshot,
    received: Instant,
}

/// The usage interest state of one shell. The shell opens one daemon connection, so the connection field of the cache key (connection, company) is 「지금 소켓」 (the current socket),
/// that is `socket_epoch` — when the socket changes the cache is emptied.
#[derive(Debug, Default)]
pub struct UsageInterest {
    /// Windows the shell hid (tray). The label of a destroyed window is erased by [`UsageInterest::forget_window`].
    hidden: BTreeSet<WindowLabel>,
    interest: BTreeSet<AgentBackendKind>,
    usage_windows: BTreeSet<WindowLabel>,
    socket_epoch: u64,
    max_seen_socket_epoch: u64,
    sent: BTreeSet<AgentBackendKind>,
    cache: BTreeMap<AgentBackendKind, Cached>,
    shrink_seq: u64,
    /// ★Invariant — while standing, `interest ⊊ sent` and `interest` is exactly the interest that set up that reduction★ (setting up a reduction
    /// and cancelling it are only recompute · send · socket open/close, and any place that changes `sent` in the meantime cancels the reduction).
    shrink_pending: Option<ShrinkGen>,
}

impl UsageInterest {
    pub fn new() -> Self {
        Self::default()
    }

    /// After layout · visibility changes, recompute the interest and the windows to receive broadcasts — called inside the ViewManager lock.
    ///
    /// ★The hidden mark is not erased here★ — a mark for a label not yet in the model is also kept: moving a slot to a new window
    /// opens the OS window first (phase B), releases the lock, and then adds the model window (phase C), so if the tray hide in that gap were erased,
    /// a hidden pop-out would be counted as visible. Only window destruction erases it ([`Self::forget_window`]).
    pub(super) fn recompute(&mut self, mgr: &ViewManager, now: Instant) -> InterestAction {
        let before = std::mem::replace(&mut self.interest, interest_from_layout(mgr, &self.hidden));
        self.usage_windows = usage_slot_windows(mgr);
        if !self.interest.is_subset(&self.sent) {
            self.shrink_pending = None;
            InterestAction::Nudge
        } else if self.shrink_pending.is_some() && self.interest == before {
            // The same reduction is already waiting — leave the generation · deadline as they are so an unrelated recompute cannot postpone the reduction
            // (TRD §1-7 「recompute」 · §3 #91 — with the same interest a new generation is not drawn).
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

    /// Window `label` was destroyed (inside the same lock in which popup destruction cleanup erased the window from the model) — erase the hidden mark and
    /// act the same as [`Self::recompute`]. A label is not reused, so if left it leaks one entry for every pop-out that closed while hidden.
    pub(super) fn forget_window(
        &mut self,
        label: &str,
        mgr: &ViewManager,
        now: Instant,
    ) -> InterestAction {
        self.hidden.remove(label);
        self.recompute(mgr, now)
    }

    /// After the shell hides (`false`) or re-shows (`true`) window `label` — fix the hidden mark and act the same as [`Self::recompute`].
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

    /// The reduction timer calls it at the deadline. `true` = that reduction was still valid → one nudge.
    ///
    /// ★The wait ends at the deadline, not at delivery★ — even if that nudge disappears (command channel saturated), the wait has already ended, so the next
    /// snapshot's cross-check compares with interest and fixes it ([`Self::on_snapshot`]).
    pub(super) fn deferral_elapsed(&mut self, gen: ShrinkGen) -> bool {
        if self.shrink_pending == Some(gen) {
            self.shrink_pending = None;
            true
        } else {
            false
        }
    }

    /// The `UsageSubscribe` set the connection driver writes on connection `socket_epoch` — `None` = nothing to write.
    ///
    /// - `force` = right before a ⟳ refresh frame — always gives the interest (the daemon is idempotent for the same
    ///   set).
    /// - When a nudge is taken (`force = false`): with a pending shrink, only if the interest has a vendor outside
    ///   `sent` (a pure shrink is the deadline's job); otherwise only if the interest ≠ `sent`.
    ///
    /// `Some` means `sent := interest` and the pending shrink is void. Being based on `sent`, repeated calls give
    /// `None` from the second on (idempotent — the nudge right after a connection opens relies on it). Not the
    /// current connection → `None`.
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

    /// Called after the new socket opened the command endpoint, before reading the command channel — if `Some`, one frame of that set directly to the new socket.
    ///
    /// It moves only on a marker larger than the largest one seen (otherwise `None` · nothing changes). When it moves it empties the cache and
    /// `sent := 관심` · a waiting reduction becomes invalid, and if interest is empty there is nothing to send so `None` (the daemon-side
    /// subscription of a new connection is the empty set).
    pub(super) fn on_socket_open(
        &mut self,
        socket_epoch: u64,
    ) -> Option<BTreeSet<AgentBackendKind>> {
        if socket_epoch <= self.max_seen_socket_epoch {
            tracing::debug!(
                socket_epoch,
                max_seen = self.max_seen_socket_epoch,
                "사용량 관심: 본 적 있는 소켓 표식의 열기 — 무시(승계로 밀려난 태스크)"
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

    /// Socket `socket_epoch` ended (regardless of reason). Only if it is the current socket, empty the cache and set `sent := ∅` · no socket.
    /// A waiting reduction is invalid too — the `sent` to reduce is gone, and a new socket sends the interest in full again on open.
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

    /// One snapshot arrived on socket `socket_epoch` — `subscribed` = the subscription set of that connection that the daemon carried in that one snapshot.
    ///
    /// - Cache = **highest** revision per company — a smaller revision arriving late does not change it, and an equal or larger one does (receipt
    ///   time = `now`). The daemon sends the first snapshot outside the registry lock, so the last to arrive may not be the highest.
    /// - ★Cross-check★: wanted set = `sent` if a reduction is waiting, otherwise interest. If `subscribed` differs from it,
    ///   `resend` = the wanted set. If a reduction is waiting, the wait · `sent` are left as they are — a stale
    ///   snapshot arriving inside the 1.5-second window must not pull the reduction forward. Otherwise `sent := 관심`.
    /// - ★Resend on every mismatch — do not suppress★: in the shape that suppressed the same pair, a stale record left by an earlier toggle round trip and
    ///   a correction resend the daemon rejected brought the same pair back and blocked a needed resend (TRD §3 #84). The feedback is finite
    ///   because the daemon's same-set replacement does not emit a first snapshot. One `debug` line per mismatch.
    ///
    /// If it is not the current socket, nothing is done (empty result · cache unchanged).
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
                "사용량 구독 어긋남 — 바라는 집합을 다시 보낸다"
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

    /// Called by a webview pull — hands over the whole cache with handoff correction (no daemon round trip).
    ///
    /// ★Handoff correction★: shifts relative fields by the whole seconds `d` elapsed since receipt — `age_secs + d` per window (per model included) ·
    /// `Failed.next_attempt_in_secs` · `Rejected.retry_in_secs` get `− d` (saturating at 0). Without it, an old cache (as long as
    /// 15 minutes) shows the value that much younger · the wait that much longer.
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
            "사용량 관심: 지금 소켓이 아닌 표식 — 무시"
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
    // No wildcard is kept — if a state that carries relative seconds is added, the compile must break here.
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

    /// main's active tab = one usage slot (`show_claude` · `show_codex`).
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

    /// A state where socket 1 is open and `sent` = interest (the interest is from `mgr`).
    fn connected(mgr: &ViewManager, t0: Instant) -> UsageInterest {
        let mut ui = UsageInterest::new();
        let _ = ui.recompute(mgr, t0);
        ui.on_socket_open(1);
        ui
    }

    fn defer_gen(action: InterestAction) -> ShrinkGen {
        match action {
            InterestAction::Defer { gen, .. } => gen,
            other => panic!("줄임 지연이어야 한다: {other:?}"),
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
            "활성 아닌 탭의 사용량 슬롯도 방송은 받는다"
        );

        mgr.switch_tab(MAIN_WINDOW_LABEL, usage_tab).unwrap();
        assert_eq!(interest_from_layout(&mgr, &labels(&[])), set(&[Claude]));
    }

    /// Only the hidden mark is subtracted — the shell does not know minimize, so a window with no hidden mark is visible (even if minimized).
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
            "둘 다 끈 사용량 슬롯도 사용량 슬롯이다"
        );
    }

    /// The temporary view of slot move phase A is not any window's tab.
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
        assert!(ui.hidden.contains(POPUP), "재계산은 숨김 표시를 안 지운다");
        let _ = ui.forget_window(POPUP, &mgr, t0);
        assert!(ui.hidden.is_empty(), "소멸한 창의 숨김 표시가 남았다");
        assert_eq!(ui.interest, set(&[]));
        assert!(ui.usage_windows.is_empty());
    }

    /// Moving to a new window opens the OS window first, releases the lock, and then adds the model window — the tray hide in that gap must survive
    /// so that the pop-out still counts as a hidden window after the model window stands.
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
            "모델에 없는 label 의 표시가 지워졌다"
        );

        let popup_view = mgr.create_window(POPUP).unwrap();
        put(&mut mgr, popup_view, usage(true, true));
        assert_eq!(ui.recompute(&mgr, t0), InterestAction::Unchanged);
        assert_eq!(ui.interest, set(&[]), "숨긴 팝아웃이 보임으로 셌다");
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
        assert_eq!(ui.sync(1, false), None, "빈 관심 = 보낼 것 없음");

        let view = main_active(&mgr);
        put(&mut mgr, view, usage(true, false));
        assert_eq!(ui.recompute(&mgr, t0), InterestAction::Nudge);
        assert_eq!(ui.sync(1, false), Some(set(&[Claude])));
        assert_eq!(ui.sent, set(&[Claude]));
        assert_eq!(ui.sync(1, false), None, "sent 기준이라 둘째부터 없음(멱등)");
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
            panic!("줄임 지연이어야 한다: {action:?}");
        };
        assert_eq!(deadline, t0 + USAGE_INTEREST_SHRINK_DELAY);
        assert_eq!(USAGE_INTEREST_SHRINK_DELAY, Duration::from_millis(1500));
        assert_eq!(ui.sync(1, false), None, "기한 전 = 송신 0");
        assert_eq!(ui.sent, set(&[Claude, Codex]));

        assert!(ui.deferral_elapsed(gen));
        assert_eq!(ui.sync(1, false), Some(set(&[Claude])));
        assert!(!ui.deferral_elapsed(gen), "한 세대는 한 번만 끝난다");
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
        assert!(!ui.deferral_elapsed(gen), "무효가 된 대기의 뒤늦은 기한");
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
            "더 줄면 새 기한으로 다시 선다: {rearmed:?}"
        );
        let second = defer_gen(rearmed);
        assert_ne!(first, second);
        assert!(!ui.deferral_elapsed(first));
        assert!(ui.deferral_elapsed(second));
        assert_eq!(ui.sync(1, false), Some(set(&[])));
    }

    /// A recompute that yields the same interest as a waiting reduction (an unrelated layout change) leaves the generation · deadline as they are — successive
    /// recomputes cannot postpone the reduction.
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
                "재계산 {step}"
            );
        }
        assert_eq!(ui.shrink_pending, Some(gen));
        assert!(ui.deferral_elapsed(gen), "처음 세대가 그대로 기한에 끝난다");
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
            "숨은 창도 방송은 받는다"
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
        assert_eq!(ui.sync(1, false), None, "대조가 이미 보냈다");
    }

    #[test]
    fn a_match_resends_nothing() {
        let t0 = Instant::now();
        let (mgr, _) = main_with(true, true);
        let mut ui = connected(&mgr, t0);
        let out = ui.on_snapshot(1, &snap(Codex, 1), &[Codex, Claude], t0);
        assert_eq!(
            out.resend, None,
            "새로 든 회사의 첫 스냅숏(subscribed = 관심)"
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
        assert_eq!(ui.shrink_pending, Some(gen), "대기는 그대로");
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
                .filter(|m| m.contains("사용량 구독 어긋남"))
                .count()
        };
        let resends = tracing::subscriber::with_default(DebugLines(lines.clone()), || {
            // The callsite interest is a process-global cache — if another test thread is registering the same callsite with the subscriber list from before this subscriber stood,
            // after that registration ends the cache is left as 「안 받음」 (not received) (measured — 0 lines in a parallel
            // run). So rebuild the cache and re-poke until one line is caught, and only then start counting.
            for rev in 1..=100 {
                tracing::callsite::rebuild_interest_cache();
                ui.on_snapshot(1, &snap(Claude, rev), &[Claude], t0);
                if mismatches(&lines) > 0 {
                    break;
                }
            }
            assert!(mismatches(&lines) > 0, "debug 줄을 잡을 수 없다");
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
        assert_eq!(ui.sync(2, true), None, "지금 소켓이 아니면 강제도 없다");
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
            "전달은 revision 무관"
        );
        assert_eq!(ui.snapshot_for_webview(t0).snapshots, vec![newer]);

        let t1 = t0 + Duration::from_secs(40);
        let mut same = snap(Claude, 5);
        same.plan = Some("pro".to_string());
        ui.on_snapshot(1, &same, &[Claude], t1);
        assert_eq!(
            ui.snapshot_for_webview(t1).snapshots,
            vec![same],
            "같은 revision = 새로 받은 것(받은 시각도 새로)"
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
            panic!("회사 둘: {pulled:?}");
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
            "60 - 90 은 0 으로 포화"
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
            "캐시 원본은 보정에 안 닳는다"
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
        assert_eq!(idle.on_socket_open(1), None, "빈 관심은 보낼 것이 없다");
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
            assert_eq!(ui.on_socket_open(stale), None, "표식 {stale}");
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
        assert!(!ui.deferral_elapsed(gen), "줄일 sent 가 사라졌다");
        assert_eq!(
            ui.recompute(&mgr, t0),
            InterestAction::Nudge,
            "비연결 중에는 관심이 곧 늘어남이다(넛지는 창구가 닫혀 no-op)"
        );
        assert_eq!(ui.sync(0, false), None, "소켓 없음(0)과는 맞지 않는다");
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
