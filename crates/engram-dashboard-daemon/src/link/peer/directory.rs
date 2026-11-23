//! The agent directory across PCs — this PC's published directory ([`Publisher`]) and one read-only copy per trusted PC ([`Copies`]).
//!   ★Pure★: no transport, no tasks; the runtime feeds it frames and the directory of the moment and sends what it answers.
//!
//! Rules (ADR-0391, ADR-0388 decision 7):
//! - ★Each PC writes only its own directory★: directory frames carry no node, so the copy a frame changes is the one of the link's PC;
//!   copies are never re-published (no gossip, no forwarding through a third PC).
//! - ★A row carries id, name and state only★ — nothing else about an agent (no folder, parent, backend, activity) and nothing about this
//!   PC (no addresses). The host omits names that break the remote name rule before they reach the publisher.
//! - The publisher counts `rev` per daemon run (`run` = this run's `Hello` instance): a change is one `DirDelta` with `rev` + 1; no change
//!   = no frame; ★a delta that would exceed the frame cap goes as a snapshot instead★ (frames are sent under the publisher's lock, so
//!   their order is the publisher's).
//! - ★A snapshot is one frame★ (ADR-0411): a directory that does not fit [`super::wire::MAX_FRAME_BYTES`] goes with the rows that fit (by
//!   id) and the rest left out, warned once per change — never split, never a reason to close the link. The cut is not kept: later
//!   deltas are diffed against the whole directory and may still carry left-out rows, and a copy is not repaired by a snapshot when the
//!   directory shrinks back under the cap (accepted — ADR-0411 d3).
//! - A copy applies a delta only when its `run` equals the copy's and its `rev` is the copy's + 1; otherwise the holder asks for a fresh
//!   snapshot (`DirResync`). A snapshot of the copy's own run with a lower `rev` is ignored (an old snapshot never replaces a newer copy);
//!   a snapshot of another run replaces it (the publisher restarted).
//! - ★Only the PC's chosen link writes its copy★: the actor records each PC's chosen link here ([`Copies::set_chosen`]) and a frame from
//!   any other link is dropped — checked under this lock alone, so applying a frame never holds the link book.
//! - ★Rows are checked on arrival★ by the host ([`RowCheck`]): an id that is a local agent's, a name that breaks the remote name rule, an
//!   id that is not an agent id — dropped and counted ([`Report`], logged by the runtime at most once a minute per PC). An id another PC's copy also lists is kept in both (logged): dropping one would make
//!   the winner depend on which link came up first.
//!
//! Not here: when frames are sent (the runtime: a snapshot on each newly chosen link and as the answer to `DirResync`, a delta per change
//!   on every chosen link), and whether a copy's PC is linked (`connected` / `disconnected` is read from the link book).

// ADR-0391, ADR-0390, ADR-0411

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use super::book::LinkKey;
use super::id::PeerId;
use super::wire::{encoded_len, DirAgent, DirDelta, DirSnapshot, Instance, PeerMsg};

/// The host's check of one arriving row — `Some(why)` drops it.
pub(crate) type RowCheck<'a> = &'a dyn Fn(&DirAgent) -> Option<&'static str>;

/// Rings the publisher: something that may change this PC's directory happened (a profile saved, a session started or ended). Cheap and
///   never blocking; rings while the publisher works coalesce into one more recompute.
#[derive(Clone, Default)]
pub(crate) struct DirectoryBell(Arc<tokio::sync::Notify>);

impl DirectoryBell {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn ring(&self) {
        // `notify_one` keeps one permit when nobody waits, so a ring before the publisher waits is not lost.
        self.0.notify_one();
    }

    pub(crate) async fn rung(&self) {
        self.0.notified().await;
    }
}

/// What a directory change goes out as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Publish {
    Delta(DirDelta),
    /// The delta did not fit one frame — the whole directory instead.
    Snapshot(DirSnapshot),
}

/// This PC's published directory (module head).
pub(crate) struct Publisher {
    run: Instance,
    rev: u64,
    /// The directory as last published, by id.
    agents: BTreeMap<String, DirAgent>,
    /// Frame payload cap.
    cap: usize,
    /// The snapshot of the current `rev`, built at its first use — once per change however many links get it, so a directory over the
    ///   cap is warned about once per change.
    snapshot: Option<DirSnapshot>,
}

impl Publisher {
    pub(crate) fn new(run: Instance, cap: usize) -> Self {
        Self {
            run,
            rev: 0,
            agents: BTreeMap::new(),
            cap,
            snapshot: None,
        }
    }

    /// Takes the directory as it is now. `None` = nothing changed (nothing is sent).
    pub(crate) fn update(&mut self, now: Vec<DirAgent>) -> Option<Publish> {
        let mut next: BTreeMap<String, DirAgent> = BTreeMap::new();
        for agent in now {
            next.entry(agent.id.clone()).or_insert(agent);
        }
        let upsert: Vec<DirAgent> = next
            .values()
            .filter(|agent| self.agents.get(&agent.id) != Some(*agent))
            .cloned()
            .collect();
        let remove: Vec<String> = self
            .agents
            .keys()
            .filter(|id| !next.contains_key(*id))
            .cloned()
            .collect();
        if upsert.is_empty() && remove.is_empty() {
            return None;
        }
        self.rev += 1;
        self.agents = next;
        self.snapshot = None;
        let delta = DirDelta {
            run: self.run,
            rev: self.rev,
            upsert,
            remove,
        };
        let fits = encoded_len(&PeerMsg::DirDelta(delta.clone())).is_ok_and(|n| n <= self.cap);
        Some(if fits {
            Publish::Delta(delta)
        } else {
            Publish::Snapshot(self.snapshot())
        })
    }

    /// The whole directory at the current `rev` as one frame — or, over the frame cap, the rows that fit in id order (ADR-0411).
    pub(crate) fn snapshot(&mut self) -> DirSnapshot {
        if let Some(snapshot) = &self.snapshot {
            return snapshot.clone();
        }
        let (snapshot, omitted) = self.fitting();
        if omitted > 0 {
            tracing::warn!(
                omitted,
                sent = snapshot.agents.len(),
                cap_bytes = self.cap,
                "this PC's agent directory does not fit one frame — the rows past the cap (in id order) are left out of the snapshot other PCs receive, so they may not reach those agents by name (later deltas can still carry some of them)"
            );
        }
        self.snapshot = Some(snapshot.clone());
        snapshot
    }

    /// The snapshot of the rows that fit the frame cap, in id order, and how many rows were left out.
    pub(crate) fn fitting(&self) -> (DirSnapshot, usize) {
        // The frame around the rows, with the widest `rev` it can carry.
        let mut size = encoded_len(&PeerMsg::DirSnapshot(DirSnapshot {
            run: self.run,
            rev: u64::MAX,
            agents: Vec::new(),
        }))
        .unwrap_or(0);
        let mut agents = Vec::with_capacity(self.agents.len());
        for agent in self.agents.values() {
            // A row and the comma before it.
            let row = serde_json::to_string(agent).map_or(0, |text| text.len()) + 1;
            if size + row > self.cap {
                break;
            }
            size += row;
            agents.push(agent.clone());
        }
        let omitted = self.agents.len() - agents.len();
        let snapshot = DirSnapshot {
            run: self.run,
            rev: self.rev,
            agents,
        };
        (snapshot, omitted)
    }
}

/// A trusted PC's directory as this PC holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Copy {
    pub(crate) run: Instance,
    pub(crate) rev: u64,
    /// Sorted by name, then id.
    pub(crate) agents: Vec<DirAgent>,
}

/// What a directory frame did to a copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Applied {
    /// The copy changed (or was replaced by an equal one).
    Updated,
    /// An old snapshot of the copy's own run — left as it is.
    Ignored,
    /// The frame does not follow the copy — ask the PC for a fresh snapshot.
    Resync,
}

/// The copies of the trusted PCs' directories (module head). Kept in memory only: after a restart a PC has no copy until it links again.
#[derive(Default)]
pub(crate) struct Copies {
    copies: HashMap<PeerId, Copy>,
    /// Each linked PC's chosen link as the actor last read it (module head).
    chosen: HashMap<PeerId, LinkKey>,
    /// What the last frame dropped or found shared — the runtime logs it, rate-limited per PC ([`Copies::take_report`]).
    report: Report,
}

/// What applying one frame found worth a warning.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Report {
    /// Rows the host refused, and the first reason.
    pub(crate) dropped: usize,
    pub(crate) first: Option<&'static str>,
    /// Rows whose id another PC's copy also lists.
    pub(crate) shared: usize,
}

impl Copies {
    pub(crate) fn get(&self, node: &PeerId) -> Option<&Copy> {
        self.copies.get(node)
    }

    /// Drops a PC's copy — it was removed from the trusted list.
    pub(crate) fn drop_node(&mut self, node: &PeerId) {
        self.copies.remove(node);
        self.chosen.remove(node);
    }

    /// Records each linked PC's chosen link — the actor, after every event.
    pub(crate) fn set_chosen(&mut self, chosen: HashMap<PeerId, LinkKey>) {
        self.chosen = chosen;
    }

    /// Whether `key` is `node`'s chosen link — the only one whose directory frames count.
    pub(crate) fn is_chosen(&self, node: &PeerId, key: LinkKey) -> bool {
        self.chosen.get(node) == Some(&key)
    }

    pub(crate) fn snapshot(
        &mut self,
        node: PeerId,
        snap: DirSnapshot,
        check: RowCheck<'_>,
    ) -> Applied {
        if self
            .copies
            .get(&node)
            .is_some_and(|copy| copy.run == snap.run && snap.rev < copy.rev)
        {
            tracing::debug!(pc = %node, rev = snap.rev, "an older directory snapshot of the same run — ignored");
            return Applied::Ignored;
        }
        let mut agents = dedup(checked(snap.agents, check, &mut self.report));
        sort(&mut agents);
        self.copies.insert(
            node,
            Copy {
                run: snap.run,
                rev: snap.rev,
                agents,
            },
        );
        self.count_shared_ids(&node);
        Applied::Updated
    }

    pub(crate) fn delta(&mut self, node: PeerId, delta: DirDelta, check: RowCheck<'_>) -> Applied {
        let Some(copy) = self.copies.get_mut(&node) else {
            tracing::debug!(pc = %node, "a directory delta without a copy — asking for a snapshot");
            return Applied::Resync;
        };
        if copy.run != delta.run || copy.rev.checked_add(1) != Some(delta.rev) {
            tracing::debug!(pc = %node, copy_rev = copy.rev, rev = delta.rev, "a directory delta that does not follow the copy — asking for a fresh snapshot");
            return Applied::Resync;
        }
        let gone: HashSet<&str> = delta
            .remove
            .iter()
            .map(String::as_str)
            .chain(delta.upsert.iter().map(|a| a.id.as_str()))
            .collect();
        copy.agents
            .retain(|agent| !gone.contains(agent.id.as_str()));
        // A dropped upsert row also takes the old row of that id away — a rename to a refused name must not leave the old name resolving.
        copy.agents
            .extend(dedup(checked(delta.upsert, check, &mut self.report)));
        sort(&mut copy.agents);
        copy.rev = delta.rev;
        self.count_shared_ids(&node);
        Applied::Updated
    }

    /// Logs ids `node`'s copy shares with another PC's copy — both are kept; a send to such an id is refused as ambiguous.
    fn count_shared_ids(&mut self, node: &PeerId) {
        let Some(copy) = self.copies.get(node) else {
            return;
        };
        let others: HashSet<&str> = self
            .copies
            .iter()
            .filter(|(other, _)| *other != node)
            .flat_map(|(_, theirs)| theirs.agents.iter().map(|a| a.id.as_str()))
            .collect();
        self.report.shared += copy
            .agents
            .iter()
            .filter(|a| others.contains(a.id.as_str()))
            .count();
    }

    /// What the frames applied since the last call found (then cleared).
    pub(crate) fn take_report(&mut self) -> Report {
        std::mem::take(&mut self.report)
    }

    /// Drops every copy row `problems` refuses (one answer per row, in order) — how many each PC that lost any lost.
    pub(crate) fn drop_refused(
        &mut self,
        problems: &dyn Fn(&[&DirAgent]) -> Vec<Option<&'static str>>,
    ) -> Vec<(PeerId, usize)> {
        let mut dropped = Vec::new();
        for (node, copy) in &mut self.copies {
            let refs: Vec<&DirAgent> = copy.agents.iter().collect();
            let refused: HashSet<String> = refs
                .iter()
                .zip(problems(&refs))
                .filter(|(_, why)| why.is_some())
                .map(|(row, _)| row.id.clone())
                .collect();
            if refused.is_empty() {
                continue;
            }
            let before = copy.agents.len();
            copy.agents.retain(|a| !refused.contains(&a.id));
            dropped.push((*node, before - copy.agents.len()));
        }
        dropped
    }
}

/// The rows of `rows` the host accepts; the others are counted in `report` (a count and the first reason — not their text, which is
///   agent content).
fn checked(rows: Vec<DirAgent>, check: RowCheck<'_>, report: &mut Report) -> Vec<DirAgent> {
    rows.into_iter()
        .filter(|row| match check(row) {
            None => true,
            Some(why) => {
                report.dropped += 1;
                report.first.get_or_insert(why);
                false
            }
        })
        .collect()
}

/// One row per id (the first kept).
fn dedup(rows: Vec<DirAgent>) -> Vec<DirAgent> {
    let mut seen = HashSet::new();
    rows.into_iter()
        .filter(|row| seen.insert(row.id.clone()))
        .collect()
}

/// By name, then id — the order a copy keeps.
pub(crate) fn sort(rows: &mut [DirAgent]) {
    rows.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
}

#[cfg(test)]
mod tests;
