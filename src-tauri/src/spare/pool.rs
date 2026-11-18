//! Prewarmed window pool (TRD S21-slot-docking §3-9 「pool」) — a pure state machine of one field: only data and decisions, it neither creates nor touches windows and knows nothing of locks · condition variables · the first-paint gate (that part = [`super::SpareState`]).
//! 「Is it a prewarmed window」 is the current state, not a property of the label, so this pool decides it, not a prefix — an adopted window usually becomes a popout and its label cannot change.
//! - A window of the label the field holds (being built · built · handed over · closing) is excluded from tray show · hide · `--hidden` and placement recording ([`SparePool::contains`]); after handover and before commit, even if that label is already in the model, the pool's predicate wins.
//! - That window's webview measurement report is recorded here instead of the model ([`SparePool::record`]) — the commit moves it to the model.
//! - A build is one per field — the place is secured first ([`SparePool::reserve`]) and the popout number taken after (I13 — a rejected build does not burn a number).
//! - While an adopted window awaits show (`awaiting_reveal`) no build happens — a rebuild is kicked off only by that window's show completion (I25).
// ADR-0295 · ADR-0299

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use crate::layout::manager::{check_metrics, reported_canvas, WindowLabel};
use crate::layout::{CanvasPx, LayoutError, SpareClaim, UiMetrics};

pub const LISTEN_FAILURE_LIMIT: u8 = 3;

/// The two measurements the prewarmed window's webview reported — same shape · acceptance rule as `canvas` · `metrics` of a model window entry. `None` = no report yet.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpareMeasures {
    pub canvas: Option<CanvasPx>,
    pub metrics: Option<UiMetrics>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Measure {
    Canvas { w: u32, h: u32 },
    Metrics(UiMetrics),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Slot {
    Empty,
    Building {
        label: Option<WindowLabel>,
        measures: SpareMeasures,
    },
    Built {
        label: WindowLabel,
        measures: SpareMeasures,
    },
    /// Handed over to one caller (before commit · take-back). `activate` = the `open_at` argument of that label — true right after handover · `open`. `opened_at` = when that label was last opened (dropped) — for the log.
    Claimed {
        label: WindowLabel,
        measures: SpareMeasures,
        generation: u32,
        activate: bool,
        opened_at: Option<Instant>,
    },
    /// A handed-over window decided at commit to be closed because it is not in the model — its `Destroyed` removes it and rebuilds. Meanwhile nobody can take it and no claim ticket matches (I27 — the only way back to the pool is a take-back).
    Closing {
        label: WindowLabel,
        generation: u32,
    },
}

impl Slot {
    fn label(&self) -> Option<&str> {
        match self {
            Self::Empty | Self::Building { label: None, .. } => None,
            Self::Building {
                label: Some(label), ..
            }
            | Self::Built { label, .. }
            | Self::Claimed { label, .. }
            | Self::Closing { label, .. } => Some(label),
        }
    }

    fn measures_mut(&mut self) -> Option<&mut SpareMeasures> {
        match self {
            Self::Building { measures, .. }
            | Self::Built { measures, .. }
            | Self::Claimed { measures, .. } => Some(measures),
            Self::Empty | Self::Closing { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoBuild {
    Occupied,
    AwaitingReveal,
    Exiting,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimStep {
    Claimed(SpareClaim),
    Wait {
        build: u64,
    },
    Fresh {
        build: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum CommitDecision {
    Adopt {
        measures: SpareMeasures,
        activate: bool,
        opened_at: Option<Instant>,
    },
    Close,
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gone {
    NotOurs,
    /// The field's window — the field was emptied. `claimed` = it was a handed-over window · `closing` = a window the commit decided to close (the awaited vanishing) · `rebuild` = rebuild.
    Removed {
        claimed: bool,
        closing: bool,
        rebuild: bool,
    },
    RevealDone {
        build: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Listen {
    NotPool,
    Kept,
    /// The field's window could not set up listening — the field was emptied (the caller closes that window; its `Destroyed` does not rebuild). `disabled_now` = the pool was turned off by this failure.
    Removed { claimed: bool, disabled_now: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opening {
    Claimed,
    Unavailable,
    Fresh,
}

/// TRD §3-9
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayShow {
    Rehold,
    Adopted,
    Untouched,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SparePool {
    slot: Slot,
    awaiting_reveal: Option<(WindowLabel, u32)>,
    reveal_since: Option<Instant>,
    exiting: bool,
    disabled: bool,
    listen_failures: u8,
    last_generation: u32,
    last_build: u64,
    lost: BTreeSet<WindowLabel>,
    revealed_at: Option<Instant>,
    waiting: u32,
}

impl Default for SparePool {
    fn default() -> Self {
        Self {
            slot: Slot::Empty,
            awaiting_reveal: None,
            reveal_since: None,
            exiting: false,
            disabled: false,
            listen_failures: 0,
            last_generation: 0,
            last_build: 0,
            lost: BTreeSet::new(),
            revealed_at: None,
            waiting: 0,
        }
    }
}

impl SparePool {
    pub fn slot(&self) -> &Slot {
        &self.slot
    }

    pub fn awaiting_reveal(&self) -> Option<(&str, u32)> {
        self.awaiting_reveal
            .as_ref()
            .map(|(label, generation)| (label.as_str(), *generation))
    }

    pub fn exiting(&self) -> bool {
        self.exiting
    }

    pub fn disabled(&self) -> bool {
        self.disabled
    }

    pub fn waiting(&self) -> u32 {
        self.waiting
    }

    pub fn contains(&self, label: &str) -> bool {
        self.slot.label() == Some(label)
    }

    pub fn reveal_generation(&self, label: &str) -> Option<u32> {
        self.awaiting_reveal()
            .and_then(|(l, generation)| (l == label).then_some(generation))
    }

    pub fn begin_exit(&mut self) {
        self.exiting = true;
    }

    // ── build (I13 · I16 · I25) ──

    /// Secures the build place (a label-less [`Slot::Building`]) — the number is taken after that. On success returns the previous rebuild completion time (for the log).
    // ADR-0299
    pub fn reserve(&mut self) -> Result<Option<Instant>, NoBuild> {
        if self.exiting {
            return Err(NoBuild::Exiting);
        }
        if self.disabled {
            return Err(NoBuild::Disabled);
        }
        if self.awaiting_reveal.is_some() {
            return Err(NoBuild::AwaitingReveal);
        }
        if self.slot != Slot::Empty {
            return Err(NoBuild::Occupied);
        }
        self.slot = Slot::Building {
            label: None,
            measures: SpareMeasures::default(),
        };
        self.last_build += 1;
        Ok(self.revealed_at.take())
    }

    pub fn assign(&mut self, label: WindowLabel) -> bool {
        match &mut self.slot {
            Slot::Building { label: slot, .. } if slot.is_none() => {
                *slot = Some(label);
                true
            }
            _ => false,
        }
    }

    pub fn mark_built(&mut self, label: &str) -> bool {
        match std::mem::replace(&mut self.slot, Slot::Empty) {
            Slot::Building {
                label: Some(l),
                measures,
            } if l == label => {
                self.slot = Slot::Built { label: l, measures };
                true
            }
            other => {
                self.slot = other;
                false
            }
        }
    }

    pub fn build_failed(&mut self, label: &str) -> bool {
        let ours = matches!(&self.slot, Slot::Building { label: Some(l), .. } if l == label);
        if ours {
            self.slot = Slot::Empty;
        }
        ours
    }

    // ── handover · open · take-back (I2 · I4 · I27) ──

    fn next_generation(&mut self) -> u32 {
        self.last_generation = self.last_generation.wrapping_add(1);
        self.last_generation
    }

    /// One step of handover. `awaited` = the build this handover started waiting for (the `build` of the [`ClaimStep::Wait`] then): if it failed or vanished and another build took the field, neither follow it nor take it — give `Fresh` (I2 — failure · vanishing of the awaited build gives `None` at once).
    // ADR-0299
    pub fn take_built(&mut self, awaited: Option<u64>) -> ClaimStep {
        if self.exiting || self.disabled {
            return ClaimStep::Fresh { build: false };
        }
        let replaced = awaited.is_some_and(|build| build != self.last_build);
        match std::mem::replace(&mut self.slot, Slot::Empty) {
            Slot::Built { label, measures } if !replaced => {
                let generation = self.next_generation();
                self.slot = Slot::Claimed {
                    label: label.clone(),
                    measures,
                    generation,
                    activate: true,
                    opened_at: None,
                };
                ClaimStep::Claimed(SpareClaim { label, generation })
            }
            other => {
                let step = match &other {
                    Slot::Building { .. } if !replaced => ClaimStep::Wait {
                        build: self.last_build,
                    },
                    Slot::Empty => ClaimStep::Fresh {
                        build: self.awaiting_reveal.is_none(),
                    },
                    _ => ClaimStep::Fresh { build: false },
                };
                self.slot = other;
                step
            }
        }
    }

    pub fn park(&mut self) {
        self.waiting += 1;
    }

    pub fn unpark(&mut self) {
        self.waiting -= 1;
    }

    pub fn opening(&mut self, label: &str, activate: bool, now: Instant) -> Opening {
        if let Slot::Claimed {
            label: l,
            activate: a,
            opened_at,
            ..
        } = &mut self.slot
        {
            if l == label {
                *a = activate;
                *opened_at = Some(now);
                return Opening::Claimed;
            }
        }
        if self.lost.contains(label) || self.contains(label) {
            Opening::Unavailable
        } else {
            Opening::Fresh
        }
    }

    /// Take-back before commit — only if the claim ticket is the current handover (label · adoption number), returns it to the field hidden (can be handed over again); the measurements it held stay.
    pub fn release(&mut self, claim: &SpareClaim) -> bool {
        match std::mem::replace(&mut self.slot, Slot::Empty) {
            Slot::Claimed {
                label,
                measures,
                generation,
                ..
            } if label == claim.label && generation == claim.generation => {
                self.slot = Slot::Built { label, measures };
                true
            }
            other => {
                self.slot = other;
                false
            }
        }
    }

    // ── commit (I21 · I25 · I27 · I31) ──

    /// Commit ① — if the claim ticket is the current handover, empties the field and sets `awaiting_reveal` in the same place (a second handover then kicks off no build — I25). `in_model` = whether that label has a model entry, as seen by the caller inside the layout lock.
    // ADR-0299
    pub fn commit(&mut self, claim: &SpareClaim, in_model: bool, now: Instant) -> CommitDecision {
        match std::mem::replace(&mut self.slot, Slot::Empty) {
            Slot::Claimed {
                label,
                measures,
                generation,
                activate,
                opened_at,
            } if label == claim.label && generation == claim.generation => {
                if !in_model {
                    self.slot = Slot::Closing { label, generation };
                    return CommitDecision::Close;
                }
                self.awaiting_reveal = Some((label, generation));
                self.reveal_since = Some(now);
                CommitDecision::Adopt {
                    measures,
                    activate,
                    opened_at,
                }
            }
            other => {
                self.slot = other;
                CommitDecision::Stale
            }
        }
    }

    /// The window the commit decided to close could not be closed either time — empties the field (next handover builds — I31). `false` = the field is already not that window (a `Destroyed` removed it meanwhile).
    pub fn close_failed(&mut self, claim: &SpareClaim) -> bool {
        let ours = matches!(&self.slot, Slot::Closing { label, generation }
            if *label == claim.label && *generation == claim.generation);
        if ours {
            self.slot = Slot::Empty;
        }
        ours
    }

    // ── rebuild completion · `Destroyed` (I8 · I25) ──

    /// Rebuild completion (show · the awaited label's `Destroyed` · safety deadline) — if the label awaiting show is that one, empties it and gives `Some(build)`, else `None` (the one that came first already finished it). Once per label.
    // ADR-0299
    pub fn complete_reveal(&mut self, label: &str, now: Instant) -> Option<bool> {
        self.reveal_generation(label)?;
        self.awaiting_reveal = None;
        self.reveal_since = None;
        let build = !self.exiting && !self.disabled;
        if build {
            self.revealed_at = Some(now);
        }
        Some(build)
    }

    /// If a show-wait has stood longer than `older_than`, does that rebuild completion now — a net for a wait the safety deadline (set window-side after commit) could not end; left, no build place could be secured for the whole run and prewarming stops. `Some` = the label · adoption number of the wait that ended (whether to build as [`Self::complete_reveal`] — handover's `Empty` branch kicks it off next).
    // ADR-0299
    pub fn expire_reveal(
        &mut self,
        now: Instant,
        older_than: Duration,
    ) -> Option<(WindowLabel, u32)> {
        let since = self.reveal_since?;
        if now.saturating_duration_since(since) <= older_than {
            return None;
        }
        let (label, generation) = self.awaiting_reveal.clone()?;
        self.complete_reveal(&label, now);
        Some((label, generation))
    }

    pub fn destroyed(&mut self, label: &str, now: Instant) -> Gone {
        if self.contains(label) {
            let claimed = matches!(self.slot, Slot::Claimed { .. });
            let closing = matches!(self.slot, Slot::Closing { .. });
            if claimed {
                self.lost.insert(label.to_owned());
            }
            self.slot = Slot::Empty;
            return Gone::Removed {
                claimed,
                closing,
                rebuild: !self.exiting && !self.disabled,
            };
        }
        match self.complete_reveal(label, now) {
            Some(build) => Gone::RevealDone { build },
            None => Gone::NotOurs,
        }
    }

    // ── listening failure (I28) ──

    pub fn listen(&mut self, label: &str, adoptable: bool) -> Listen {
        if !self.contains(label) {
            return Listen::NotPool;
        }
        if adoptable {
            self.listen_failures = 0;
            return Listen::Kept;
        }
        let claimed = matches!(self.slot, Slot::Claimed { .. });
        if claimed {
            self.lost.insert(label.to_owned());
        }
        self.slot = Slot::Empty;
        self.listen_failures = self.listen_failures.saturating_add(1);
        let disabled_now = !self.disabled && self.listen_failures >= LISTEN_FAILURE_LIMIT;
        if disabled_now {
            self.disabled = true;
        }
        Listen::Removed {
            claimed,
            disabled_now,
        }
    }

    pub fn tray_show(&self, label: &str) -> TrayShow {
        match &self.slot {
            Slot::Claimed { label: l, .. } if l == label => TrayShow::Rehold,
            _ if self.contains(label) => TrayShow::Untouched,
            _ => TrayShow::Adopted,
        }
    }

    // ── measurements ──

    /// Records the measurements the field's window sent — `None` = not the field's window (the caller records in the model, including a closing window). `Some(Err)` = rejected, metrics out of range (previous value kept). A zero-size canvas is ignored: `Some(Ok)`.
    pub fn record(&mut self, label: &str, measure: Measure) -> Option<Result<(), LayoutError>> {
        if !self.contains(label) {
            return None;
        }
        let m = self.slot.measures_mut()?;
        Some(match measure {
            Measure::Canvas { w, h } => {
                if let Some(canvas) = reported_canvas(w, h) {
                    m.canvas = Some(canvas);
                }
                Ok(())
            }
            Measure::Metrics(metrics) => check_metrics(&metrics).map(|()| {
                m.metrics = Some(metrics);
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::geometry::Insets;

    const P1: &str = "slot-popup-1";
    const P2: &str = "slot-popup-2";

    fn metrics() -> UiMetrics {
        UiMetrics {
            frame_insets: Insets {
                t: 1.0,
                r: 1.0,
                b: 1.0,
                l: 1.0,
            },
            min_pane_px: 50,
        }
    }

    fn full() -> SpareMeasures {
        SpareMeasures {
            canvas: Some(CanvasPx { w: 800, h: 600 }),
            metrics: Some(metrics()),
        }
    }

    fn report_all(pool: &mut SparePool, label: &str) {
        for measure in [
            Measure::Canvas { w: 800, h: 600 },
            Measure::Metrics(metrics()),
        ] {
            assert_eq!(pool.record(label, measure), Some(Ok(())));
        }
    }

    fn building(label: &str) -> SparePool {
        let mut pool = SparePool::default();
        pool.reserve().unwrap();
        assert!(pool.assign(label.to_owned()));
        pool
    }

    fn built(label: &str) -> SparePool {
        let mut pool = building(label);
        assert!(pool.mark_built(label));
        pool
    }

    fn claimed(label: &str) -> (SparePool, SpareClaim) {
        let mut pool = built(label);
        let ClaimStep::Claimed(claim) = pool.take_built(None) else {
            panic!("hands over the built window");
        };
        (pool, claim)
    }

    fn now() -> Instant {
        Instant::now()
    }

    // ── build place ──

    #[test]
    fn the_build_place_is_taken_before_the_label_and_only_once() {
        let mut pool = SparePool::default();
        assert_eq!(pool.reserve(), Ok(None));
        assert_eq!(
            pool.slot(),
            &Slot::Building {
                label: None,
                measures: SpareMeasures::default()
            }
        );
        assert!(!pool.contains(P1), "the label does not exist yet");
        assert_eq!(pool.reserve(), Err(NoBuild::Occupied));
        assert!(pool.assign(P1.to_owned()));
        assert!(!pool.assign(P2.to_owned()), "one label per place");
        assert!(pool.contains(P1));
        assert_eq!(pool.reserve(), Err(NoBuild::Occupied));
    }

    #[test]
    fn no_build_place_while_exiting_disabled_or_awaiting_a_reveal() {
        let mut pool = SparePool::default();
        pool.awaiting_reveal = Some((P1.to_owned(), 1));
        assert_eq!(pool.reserve(), Err(NoBuild::AwaitingReveal));
        pool.disabled = true;
        assert_eq!(pool.reserve(), Err(NoBuild::Disabled));
        pool.begin_exit();
        assert_eq!(pool.reserve(), Err(NoBuild::Exiting));
        assert_eq!(pool.slot(), &Slot::Empty);
    }

    #[test]
    fn a_build_failure_frees_only_its_own_place() {
        let mut pool = building(P1);
        assert!(!pool.build_failed(P2));
        assert!(pool.build_failed(P1));
        assert_eq!(pool.slot(), &Slot::Empty);
        assert!(!pool.mark_built(P1), "a failed build cannot mark it as built");
    }

    // ── handover ──

    #[test]
    fn only_a_built_spare_is_handed_out_and_each_claim_gets_a_new_generation() {
        assert_eq!(building(P1).take_built(None), ClaimStep::Wait { build: 1 });
        assert_eq!(
            SparePool::default().take_built(None),
            ClaimStep::Fresh { build: true }
        );
        let (mut pool, first) = claimed(P1);
        assert_eq!(first.label, P1);
        assert_eq!(pool.take_built(None), ClaimStep::Fresh { build: false });
        assert!(pool.release(&first));
        let ClaimStep::Claimed(second) = pool.take_built(None) else {
            panic!("hands over the returned window again");
        };
        assert_eq!(second.label, P1);
        assert!(second.generation > first.generation);
    }

    #[test]
    fn an_empty_slot_asks_for_a_build_only_without_a_pending_reveal() {
        let mut pool = SparePool::default();
        pool.awaiting_reveal = Some((P1.to_owned(), 1));
        assert_eq!(pool.take_built(None), ClaimStep::Fresh { build: false });
    }

    #[test]
    fn nothing_is_handed_out_while_exiting_or_disabled() {
        let disable = |p: &mut SparePool| p.disabled = true;
        for stop in [SparePool::begin_exit as fn(&mut SparePool), disable] {
            let mut pool = built(P1);
            stop(&mut pool);
            assert_eq!(pool.take_built(None), ClaimStep::Fresh { build: false });
            assert!(matches!(pool.slot(), Slot::Built { .. }));
        }
    }

    #[test]
    fn opening_tells_a_claimed_label_from_a_lost_or_fresh_one() {
        let (mut pool, claim) = claimed(P1);
        assert_eq!(pool.opening(P2, true, now()), Opening::Fresh);
        assert_eq!(pool.opening(P1, false, now()), Opening::Claimed);
        assert!(matches!(
            pool.slot(),
            Slot::Claimed {
                activate: false,
                opened_at: Some(_),
                ..
            }
        ));
        assert!(matches!(
            pool.destroyed(&claim.label, now()),
            Gone::Removed { claimed: true, .. }
        ));
        assert_eq!(pool.opening(P1, true, now()), Opening::Unavailable);
        let mut pool = built(P1);
        assert_eq!(
            pool.opening(P1, true, now()),
            Opening::Unavailable,
            "does not open with the label of a prewarmed window that was not handed over"
        );
    }

    // ── commit ──

    #[test]
    fn commit_empties_the_slot_and_sets_the_pending_reveal_in_one_step() {
        let (mut pool, claim) = claimed(P1);
        report_all(&mut pool, P1);
        assert!(matches!(
            pool.commit(&claim, true, now()),
            CommitDecision::Adopt { measures, activate: true, .. } if measures == full()
        ));
        assert_eq!(pool.slot(), &Slot::Empty);
        assert_eq!(pool.awaiting_reveal(), Some((P1, claim.generation)));
        assert_eq!(pool.reveal_generation(P1), Some(claim.generation));
        assert!(!pool.contains(P1), "after confirmation it is a model window");
        assert_eq!(pool.commit(&claim, true, now()), CommitDecision::Stale);
    }

    #[test]
    fn commit_for_a_label_gone_from_the_model_keeps_it_closing() {
        let (mut pool, claim) = claimed(P1);
        assert_eq!(pool.commit(&claim, false, now()), CommitDecision::Close);
        assert_eq!(
            pool.slot(),
            &Slot::Closing {
                label: P1.to_owned(),
                generation: claim.generation
            }
        );
        assert_eq!(pool.awaiting_reveal(), None);
        assert!(pool.contains(P1));
        assert_eq!(pool.take_built(None), ClaimStep::Fresh { build: false });
        assert!(!pool.release(&claim), "a window decided to be closed does not go back");
        assert_eq!(pool.commit(&claim, true, now()), CommitDecision::Stale);
        assert!(pool.close_failed(&claim));
        assert_eq!(pool.slot(), &Slot::Empty);
        assert!(!pool.close_failed(&claim));
    }

    // ── rebuild completion ──

    #[test]
    fn a_reveal_completion_while_disabled_clears_without_building() {
        let (mut pool, claim) = claimed(P1);
        pool.commit(&claim, true, now());
        pool.disabled = true;
        assert_eq!(pool.complete_reveal(P1, now()), Some(false));
        assert_eq!(pool.awaiting_reveal(), None);
        assert_eq!(pool.complete_reveal(P1, now()), None);
    }

    #[test]
    fn a_pending_reveal_older_than_the_backstop_is_completed_once_and_frees_the_build_place() {
        let (mut pool, claim) = claimed(P1);
        let at = now();
        pool.commit(&claim, true, at);
        let limit = Duration::from_secs(5);
        assert_eq!(
            pool.expire_reveal(at + limit, limit),
            None,
            "keeps up to the limit"
        );
        assert_eq!(pool.reserve(), Err(NoBuild::AwaitingReveal));
        let late = at + limit + Duration::from_millis(1);
        assert_eq!(
            pool.expire_reveal(late, limit),
            Some((P1.to_owned(), claim.generation))
        );
        assert_eq!(pool.awaiting_reveal(), None);
        assert_eq!(
            pool.complete_reveal(P1, late),
            None,
            "a late completion of a finished wait is a no-op"
        );
        assert_eq!(pool.expire_reveal(late, limit), None);
        assert_eq!(pool.take_built(None), ClaimStep::Fresh { build: true });
        assert_eq!(pool.reserve(), Ok(Some(late)));
    }

    #[test]
    fn a_completed_reveal_leaves_nothing_to_expire() {
        let (mut pool, claim) = claimed(P1);
        let at = now();
        pool.commit(&claim, true, at);
        assert_eq!(pool.complete_reveal(P1, at), Some(true));
        assert_eq!(
            pool.expire_reveal(at + Duration::from_secs(60), Duration::ZERO),
            None
        );
    }

    #[test]
    fn the_reveal_time_is_handed_to_the_next_build_once() {
        let (mut pool, claim) = claimed(P1);
        pool.commit(&claim, true, now());
        let at = now();
        assert_eq!(pool.complete_reveal(P1, at), Some(true));
        assert_eq!(pool.reserve(), Ok(Some(at)));
        pool.slot = Slot::Empty;
        assert_eq!(pool.reserve(), Ok(None));
    }

    // ── tray show ──

    #[test]
    fn a_tray_show_tells_claimed_adopted_and_untouched_apart() {
        assert_eq!(built(P1).tray_show(P1), TrayShow::Untouched);
        assert_eq!(building(P1).tray_show(P1), TrayShow::Untouched);
        let (mut pool, claim) = claimed(P1);
        assert_eq!(pool.tray_show(P1), TrayShow::Rehold);
        pool.commit(&claim, true, now());
        assert_eq!(pool.tray_show(P1), TrayShow::Adopted);
    }

    // ── receiving measurements ──

    #[test]
    fn a_label_outside_the_slot_is_left_to_the_model() {
        let mut pool = SparePool::default();
        assert_eq!(pool.record(P1, Measure::Canvas { w: 800, h: 600 }), None);
        assert_eq!(pool, SparePool::default());
        let (mut pool, claim) = claimed(P1);
        pool.commit(&claim, false, now());
        assert_eq!(
            pool.record(P1, Measure::Canvas { w: 800, h: 600 }),
            None,
            "a window being closed has nothing to hold"
        );
    }

    #[test]
    fn measures_follow_the_window_from_building_to_claimed_and_back() {
        let mut pool = building(P1);
        report_all(&mut pool, P1);
        pool.mark_built(P1);
        assert_eq!(
            pool.slot(),
            &Slot::Built {
                label: P1.to_owned(),
                measures: full()
            }
        );
        let ClaimStep::Claimed(claim) = pool.take_built(None) else {
            panic!("hands over");
        };
        assert_eq!(
            pool.record(P1, Measure::Canvas { w: 1024, h: 768 }),
            Some(Ok(()))
        );
        pool.release(&claim);
        assert!(matches!(
            pool.slot(),
            Slot::Built { measures, .. } if measures.canvas == Some(CanvasPx { w: 1024, h: 768 })
        ));
    }

    #[test]
    fn a_zero_canvas_is_ignored_and_keeps_the_previous_value() {
        let mut pool = built(P1);
        report_all(&mut pool, P1);
        for (w, h) in [(0, 600), (800, 0), (0, 0)] {
            assert_eq!(pool.record(P1, Measure::Canvas { w, h }), Some(Ok(())));
        }
        assert!(matches!(
            pool.slot(),
            Slot::Built { measures, .. } if measures.canvas == Some(CanvasPx { w: 800, h: 600 })
        ));
    }

    #[test]
    fn out_of_range_metrics_are_refused_and_keep_the_previous_value() {
        let mut pool = built(P1);
        report_all(&mut pool, P1);
        let bad = UiMetrics {
            min_pane_px: 0,
            ..metrics()
        };
        assert!(matches!(
            pool.record(P1, Measure::Metrics(bad)),
            Some(Err(LayoutError::InvalidMetrics(_)))
        ));
        assert!(matches!(
            pool.slot(),
            Slot::Built { measures, .. } if measures.metrics == Some(metrics())
        ));
    }
}
