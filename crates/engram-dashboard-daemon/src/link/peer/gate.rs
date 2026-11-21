//! Refused knocks — PCs that tried to link without being on the trusted list. The accept gate records each refusal here: the list is
//!   state for `peer.list` and the app (so a human can add a PC that keeps knocking), and the refusal log is throttled.
//!
//! ★One time-window budget for every refusal, not one per PC★ (ADR-0310 D9 per-kind batches, ADR-0346 d5 repeated identical failures):
//!   the first refusal in a [`LOG_WINDOW`] is logged, the rest of that window are counted and folded into the next line with how many
//!   PCs they came from. A stranger rotating keys, or many strangers, therefore gives at most one line per window — the network cannot
//!   drive the log. The runtime ticks its change signal on the same budget: on each line, and once at the end of a window that folded
//!   anything ([`Knocks::trailing_due`]), so the list a watcher reads is never more than one window behind. Memory is bounded too: at most [`MAX_KNOCKS`] PCs are kept
//!   in the list (the one heard from least recently goes first) and at most [`MAX_FOLDED_PCS`] distinct PCs are counted per window.

// ADR-0390

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant, SystemTime};

use super::id::PeerId;

/// Refusals within this after a logged one are folded into the next line.
pub(crate) const LOG_WINDOW: Duration = Duration::from_secs(60);
/// PCs remembered in the list.
pub(crate) const MAX_KNOCKS: usize = 32;
/// Distinct PCs counted per window — past it the count stops growing, so the logged number is a lower bound.
pub(crate) const MAX_FOLDED_PCS: usize = 1024;

/// One refused PC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Knock {
    pub(crate) id: PeerId,
    /// Refusals this run.
    pub(crate) count: u64,
    pub(crate) first_at: SystemTime,
    pub(crate) last_at: SystemTime,
}

#[derive(Debug)]
struct Entry {
    knock: Knock,
    last: Instant,
}

/// What to do for one refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LogLine {
    /// Log now (and tick the change signal); `folded` = refusals of the previous window not logged, from `pcs` distinct PCs.
    Now { folded: u64, pcs: usize },
    /// Counted for the next line; `first` = the first fold of this window (the runtime then schedules its trailing tick).
    Fold { first: bool },
}

#[derive(Debug)]
struct Window {
    start: Instant,
    folded: u64,
    pcs: HashSet<PeerId>,
    /// The trailing change tick for this window's folds went out.
    ticked: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Knocks {
    by_id: HashMap<PeerId, Entry>,
    window: Option<Window>,
}

impl Knocks {
    /// Records a refusal of `id`.
    pub(crate) fn knock(&mut self, id: PeerId, now: Instant, wall: SystemTime) -> LogLine {
        self.remember(id, now, wall);
        match &mut self.window {
            Some(window) if now.saturating_duration_since(window.start) < LOG_WINDOW => {
                window.folded += 1;
                if window.pcs.len() < MAX_FOLDED_PCS {
                    window.pcs.insert(id);
                }
                LogLine::Fold {
                    first: window.folded == 1,
                }
            }
            _ => {
                let (folded, pcs) = self
                    .window
                    .take()
                    .map_or((0, 0), |w| (w.folded, w.pcs.len()));
                self.window = Some(Window {
                    start: now,
                    folded: 0,
                    pcs: HashSet::new(),
                    ticked: false,
                });
                LogLine::Now { folded, pcs }
            }
        }
    }

    fn remember(&mut self, id: PeerId, now: Instant, wall: SystemTime) {
        if let Some(entry) = self.by_id.get_mut(&id) {
            entry.knock.count += 1;
            entry.knock.last_at = wall;
            entry.last = now;
            return;
        }
        if self.by_id.len() >= MAX_KNOCKS {
            let oldest = self
                .by_id
                .iter()
                .min_by_key(|(_, e)| e.last)
                .map(|(id, _)| *id);
            if let Some(oldest) = oldest {
                self.by_id.remove(&oldest);
            }
        }
        self.by_id.insert(
            id,
            Entry {
                knock: Knock {
                    id,
                    count: 1,
                    first_at: wall,
                    last_at: wall,
                },
                last: now,
            },
        );
    }

    /// When the trailing change tick of the current window is due — only for a window that folded something and has not ticked.
    pub(crate) fn trailing_due(&self) -> Option<Instant> {
        self.window
            .as_ref()
            .filter(|w| w.folded > 0 && !w.ticked)
            .map(|w| w.start + LOG_WINDOW)
    }

    /// Whether the trailing tick is due at `now` — then it is taken (once per window).
    pub(crate) fn take_trailing(&mut self, now: Instant) -> bool {
        match self.window.as_mut() {
            Some(w) if w.folded > 0 && !w.ticked && now >= w.start + LOG_WINDOW => {
                w.ticked = true;
                true
            }
            _ => false,
        }
    }

    /// The remembered PCs, most recent first.
    pub(crate) fn list(&self) -> Vec<Knock> {
        let mut entries: Vec<&Entry> = self.by_id.values().collect();
        entries.sort_by(|a, b| b.last.cmp(&a.last));
        entries.into_iter().map(|e| e.knock.clone()).collect()
    }

    /// Forgets a PC — it was added to the list.
    pub(crate) fn forget(&mut self, id: &PeerId) {
        self.by_id.remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> PeerId {
        PeerId::from_bytes([n; 32])
    }

    #[test]
    fn the_first_refusal_logs_and_the_rest_of_the_window_fold_into_the_next_line() {
        let mut knocks = Knocks::default();
        let t = Instant::now();
        let wall = SystemTime::now();
        assert_eq!(
            knocks.knock(id(1), t, wall),
            LogLine::Now { folded: 0, pcs: 0 }
        );
        assert_eq!(
            knocks.knock(id(1), t + Duration::from_secs(1), wall),
            LogLine::Fold { first: true }
        );
        assert_eq!(
            knocks.knock(id(2), t + Duration::from_secs(30), wall),
            LogLine::Fold { first: false },
            "another PC shares the window"
        );
        assert_eq!(
            knocks.knock(id(1), t + LOG_WINDOW, wall),
            LogLine::Now { folded: 2, pcs: 2 }
        );
        let list = knocks.list();
        assert_eq!(
            list.iter().map(|k| (k.id, k.count)).collect::<Vec<_>>(),
            [(id(1), 3), (id(2), 1)]
        );
    }

    #[test]
    fn a_hundred_distinct_keys_in_one_window_give_one_line() {
        let mut knocks = Knocks::default();
        let t = Instant::now();
        let wall = SystemTime::now();
        let lines = (0..100u8)
            .filter(|n| {
                matches!(
                    knocks.knock(id(*n), t + Duration::from_millis(u64::from(*n)), wall),
                    LogLine::Now { .. }
                )
            })
            .count();
        assert_eq!(lines, 1);
        assert_eq!(
            knocks.knock(id(200), t + LOG_WINDOW, wall),
            LogLine::Now {
                folded: 99,
                pcs: 99
            }
        );
        assert_eq!(knocks.list().len(), MAX_KNOCKS, "the list stays bounded");
    }

    #[test]
    fn a_window_that_folded_gets_one_trailing_tick_at_its_end() {
        let mut knocks = Knocks::default();
        let t = Instant::now();
        let wall = SystemTime::now();
        knocks.knock(id(1), t, wall);
        assert_eq!(knocks.trailing_due(), None, "nothing folded yet");
        knocks.knock(id(2), t + Duration::from_secs(5), wall);
        assert_eq!(knocks.trailing_due(), Some(t + LOG_WINDOW));
        assert!(!knocks.take_trailing(t + Duration::from_secs(59)));
        assert!(knocks.take_trailing(t + LOG_WINDOW));
        assert!(!knocks.take_trailing(t + LOG_WINDOW), "once per window");
        assert_eq!(knocks.trailing_due(), None);
    }

    #[test]
    fn at_most_max_knocks_are_kept_and_the_least_recent_goes_first() {
        let mut knocks = Knocks::default();
        let t = Instant::now();
        let wall = SystemTime::now();
        for n in 0..MAX_KNOCKS as u8 {
            knocks.knock(id(n), t + Duration::from_secs(u64::from(n)), wall);
        }
        // PC 0 knocks again — now PC 1 is the least recent.
        knocks.knock(id(0), t + Duration::from_secs(100), wall);
        knocks.knock(id(200), t + Duration::from_secs(101), wall);
        let list = knocks.list();
        assert_eq!(list.len(), MAX_KNOCKS);
        assert!(list.iter().any(|k| k.id == id(0)));
        assert!(!list.iter().any(|k| k.id == id(1)));
        knocks.forget(&id(200));
        assert!(!knocks.list().iter().any(|k| k.id == id(200)));
    }
}
