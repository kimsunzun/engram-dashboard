//! The link book — per trusted PC, which links are up, which one is chosen, what the dialing handle is doing, and the duplicate-identity
//!   evidence. ★Pure★: no transport, no tasks, time comes in as an argument; it answers with [`Action`]s the runtime carries out. Keys
//!   ([`LinkKey`]) are drawn here, so a dialing handle's key is known before the runtime starts it.
//!
//! Rules it keeps (ADR-0388 decision 7, ADR-0389 decision 8):
//! - ★Every listed PC has one dialing handle, except while an accepted link from it is live★ — then the handle is closed and a new one is
//!   started when the last link of that PC ends (transport has no pause). A re-arm trigger that fires meanwhile has nothing to do: the new
//!   handle comes anyway. The handle is closed as soon as it is between attempts (backing off, idle, gave up — the runtime says so);
//!   ★only a handle that is mid-attempt (dialing, handshaking) is left to finish★, because the other PC may be accepting that very
//!   connection and keep it by the tie-break — closing it here would drop both links. If it comes up, the tie-break picks one.
//! - Two live links of one run (same `Hello` instance) happen only when both PCs dial at once. Up within [`DIALED_TOGETHER`] of each
//!   other → keep the one dialed by the smaller ID (both PCs pick the same connection); later → the newer replaces the older (the older
//!   may be left over from an abrupt restart). The dropped one is closed going-away. Directory frames go on the chosen link only.
//! - Duplicate identity, from fresh evidence only: (a) a link of another run while one is live → `Probe` on the older; a `ProbeAck` within
//!   [`PROBE_WINDOW`] proves both alive → the newer link is refused and both get `Duplicate`; no answer → the older was stale and is
//!   closed (a plain restart); (b) a run that was replaced, or refused as a duplicate, coming back within [`DUPLICATE_MEMORY`] is refused
//!   at its handshake ([`LinkBook::admit`]); (c) the same run replacing its own link ★in the same direction★ [`FLAP_COUNT`] times
//!   within [`FLAP_WINDOW`] → a "suspected duplicate" warning, no refusal. A running clone that shares the run replaces in the same
//!   direction; a replacement across directions is one PC's dial meeting the other's (most ordinary reconnects) and is not counted.
//!   Residual: when the other PC notices a loss first and redials before ours idles out, its new link replaces the half-open one in the
//!   same direction and counts — three such long drops within the window still warn. A late frame of an old run proves nothing — only a
//!   `ProbeAck` with the nonce asked on that link counts.
//! - ★The duplicate evidence lives for the daemon run★ — replaced and refused runs, the flap history and the warnings survive an
//!   endpoint restart ([`LinkBook::restart`] resets only links and dialing).
//! - A terminal refusal from the other side stops the dialing handle until a re-arm: a link of that PC ending, a trusted-list add
//!   ([`LinkBook::rearm_all`]), daemon start, and for "not paired" a slow retry every [`NOT_PAIRED_RETRY`].

// ADR-0388, ADR-0389, ADR-0390

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::id::PeerId;
use super::wire::{DuplicateKind, Instance, PeerMsg};

/// Two links that come up this close together were dialed at once (Reason Network `DIALED_TOGETHER`).
pub(crate) const DIALED_TOGETHER: Duration = Duration::from_secs(5);
/// How long the older link of another run has to answer `Probe`.
pub(crate) const PROBE_WINDOW: Duration = Duration::from_secs(10);
/// How long a replaced or refused run is remembered — its return within this is a duplicate.
pub(crate) const DUPLICATE_MEMORY: Duration = Duration::from_secs(5 * 60);
/// The flap rule's window and count.
pub(crate) const FLAP_WINDOW: Duration = Duration::from_secs(10 * 60);
pub(crate) const FLAP_COUNT: usize = 3;
/// A "not paired" refusal is retried this often — the other PC may add this one later.
pub(crate) const NOT_PAIRED_RETRY: Duration = Duration::from_secs(10 * 60);

/// One transport handle — a dialing handle (its links come and go) or one accepted link.
pub(crate) type LinkKey = u64;

/// Who opened a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dir {
    Dialed,
    Accepted,
}

/// Why the other side ended our dialing for good (a terminal refusal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Not on its trusted list (or refused for no stated reason).
    NotPaired,
    VersionMismatch,
    Duplicate(DuplicateKind),
}

/// A PC's link state as `peer.list` and the app show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Online {
    /// At least one live link.
    Online,
    Offline,
    /// The other PC refused this one ("not paired").
    Refused,
    VersionMismatch,
    DuplicateIdentity,
    /// The endpoint could not start (a refused helper URL, the port in use) — set by the runtime, never by the book.
    EndpointError,
}

/// A duplicate-identity warning on a PC: detected here, or reported by that PC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Warning {
    pub(crate) kind: DuplicateKind,
    pub(crate) reported: bool,
}

/// How a link is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloseAs {
    /// Not a refusal — a dialer redials later.
    GoingAway,
    /// Terminal for a dialer still in its handshake; during operation it reads as a loss.
    Refused,
}

/// What the runtime does for the book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Action {
    /// Close this handle after what was sent before. A closed dialing handle is not reused — a new one comes with [`Action::AddDial`].
    Close {
        key: LinkKey,
        how: CloseAs,
        reason: &'static str,
    },
    /// Send on this handle's current link.
    Send { key: LinkKey, msg: PeerMsg },
    /// Start a dialing handle for `node` under `key`.
    AddDial { node: PeerId, key: LinkKey },
    /// Dial again now with the handle that gave up.
    Redial { key: LinkKey },
}

#[derive(Debug, Clone, Copy)]
struct Link {
    key: LinkKey,
    dir: Dir,
    instance: Instance,
    up_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dial {
    /// The dialing handle exists and is dialing, backing off or live.
    Active(LinkKey),
    /// It received a terminal refusal; the handle waits for a re-arm.
    GaveUp {
        key: LinkKey,
        why: Refusal,
        at: Instant,
    },
    /// Closed while an accepted link of this PC is live; a new one starts when the last link ends.
    Removed,
}

#[derive(Debug, Clone, Copy)]
struct Probe {
    nonce: u64,
    old: LinkKey,
    new: LinkKey,
    deadline: Instant,
    /// An answer was already handed on ([`LinkBook::claim_probe_answer`]).
    claimed: bool,
}

#[derive(Debug)]
struct Node {
    /// Live links, in the order they came up.
    links: Vec<Link>,
    chosen: Option<LinkKey>,
    dial: Dial,
    /// The run of the latest chosen link.
    last_instance: Option<Instance>,
    /// Runs replaced by another run, and runs refused as duplicates — with when.
    replaced: Vec<(Instance, Instant)>,
    /// Same-run replacements of a live link.
    flaps: Vec<Instant>,
    probe: Option<Probe>,
    warning: Option<Warning>,
}

impl Node {
    fn new(dial_key: LinkKey) -> Self {
        Self {
            links: Vec::new(),
            chosen: None,
            dial: Dial::Active(dial_key),
            last_instance: None,
            replaced: Vec::new(),
            flaps: Vec::new(),
            probe: None,
            warning: None,
        }
    }

    fn prune(&mut self, now: Instant) {
        self.replaced
            .retain(|(_, at)| now.saturating_duration_since(*at) < DUPLICATE_MEMORY);
        self.flaps
            .retain(|at| now.saturating_duration_since(*at) < FLAP_WINDOW);
    }

    fn link(&self, key: LinkKey) -> Option<Link> {
        self.links.iter().copied().find(|l| l.key == key)
    }

    fn chosen_link(&self) -> Option<Link> {
        self.chosen.and_then(|key| self.link(key))
    }

    fn dial_key(&self) -> Option<LinkKey> {
        match self.dial {
            Dial::Active(key) | Dial::GaveUp { key, .. } => Some(key),
            Dial::Removed => None,
        }
    }

    /// Takes `key` out of the live links; the chosen link moves to the latest one left.
    fn drop_link(&mut self, key: LinkKey) {
        self.links.retain(|l| l.key != key);
        if self.chosen == Some(key) {
            self.chosen = self.links.last().map(|l| l.key);
        }
    }

    /// Closes `link` because another one is kept; a closed dialing handle becomes `Removed`.
    fn close(&mut self, link: Link, how: CloseAs, reason: &'static str, out: &mut Vec<Action>) {
        self.drop_link(link.key);
        if self.dial_key() == Some(link.key) {
            self.dial = Dial::Removed;
        }
        out.push(Action::Close {
            key: link.key,
            how,
            reason,
        });
    }

    /// While the chosen link is an accepted one, the dialing handle is closed (no dial to a PC with a live link — ADR-0388 d7) — at once
    ///   when it gave up, and when it is `idle` (between attempts: backing off, or it just failed or lost its link). ★A handle that may be
    ///   mid-attempt is left alone★ (module head): if it comes up, the tie-break picks one; if it fails, it is closed then.
    fn settle_dial(&mut self, idle: Option<LinkKey>, out: &mut Vec<Action>) {
        let Some(chosen) = self.chosen_link() else {
            return;
        };
        if chosen.dir != Dir::Accepted {
            return;
        }
        let close = match self.dial {
            Dial::GaveUp { key, .. } => Some(key),
            Dial::Active(key) if idle == Some(key) => Some(key),
            Dial::Active(_) | Dial::Removed => None,
        };
        if let Some(key) = close {
            self.dial = Dial::Removed;
            out.push(Action::Close {
                key,
                how: CloseAs::GoingAway,
                reason: "linked by the other side",
            });
        }
    }

    fn warn(&mut self, kind: DuplicateKind, reported: bool) {
        // A proven duplicate is never lowered to a suspicion.
        if self
            .warning
            .is_some_and(|w| w.kind == DuplicateKind::Duplicate)
            && kind == DuplicateKind::Suspected
        {
            return;
        }
        self.warning = Some(Warning { kind, reported });
    }
}

/// The book (module head).
#[derive(Debug)]
pub(crate) struct LinkBook {
    own: PeerId,
    nodes: HashMap<PeerId, Node>,
    next_key: LinkKey,
    next_nonce: u64,
}

impl LinkBook {
    /// `nonce_seed` starts the probe nonces (any value; the runtime draws a random one).
    pub(crate) fn new(own: PeerId, nonce_seed: u64) -> Self {
        Self {
            own,
            nodes: HashMap::new(),
            next_key: 1,
            next_nonce: nonce_seed,
        }
    }

    /// A new endpoint in the same daemon run: every link and dialing handle of the old one is gone, so each tracked PC gets a new dialing
    ///   handle; the duplicate evidence — replaced and refused runs, flap history, warnings, the last run seen — is kept. Keys keep
    ///   counting, so a late event of the old endpoint never names a new handle. `own` = this PC's ID now (a regenerate changes it).
    pub(crate) fn restart(&mut self, own: PeerId) -> Vec<Action> {
        self.own = own;
        let mut out = Vec::new();
        let mut next_key = self.next_key;
        for (node, n) in &mut self.nodes {
            n.links.clear();
            n.chosen = None;
            n.probe = None;
            n.dial = Dial::Active(next_key);
            out.push(Action::AddDial {
                node: *node,
                key: next_key,
            });
            next_key += 1;
        }
        self.next_key = next_key;
        out
    }

    /// The PC's current dialing handle, if it has one.
    pub(crate) fn dial_of(&self, node: &PeerId) -> Option<LinkKey> {
        self.nodes.get(node).and_then(Node::dial_key)
    }

    /// Whether a `ProbeAck` on `key` with `nonce` answers the probe open on that link, the first time only — the runtime hands on at most
    ///   one answer per probe, so a peer repeating it (or guessing nonces) cannot fill the actor's queue. [`LinkBook::probe_ack`] decides.
    pub(crate) fn claim_probe_answer(&mut self, key: LinkKey, nonce: u64) -> bool {
        let probe = self
            .nodes
            .values_mut()
            .filter_map(|n| n.probe.as_mut())
            .find(|p| p.old == key && p.nonce == nonce && !p.claimed);
        match probe {
            Some(probe) => {
                probe.claimed = true;
                true
            }
            None => false,
        }
    }

    /// A key for a new accepted link.
    pub(crate) fn accepted_key(&mut self) -> LinkKey {
        self.draw_key()
    }

    fn draw_key(&mut self) -> LinkKey {
        let key = self.next_key;
        self.next_key += 1;
        key
    }

    /// Starts tracking a listed PC — its dialing handle starts at once.
    pub(crate) fn ensure(&mut self, node: PeerId) -> Vec<Action> {
        if self.nodes.contains_key(&node) {
            return Vec::new();
        }
        let key = self.draw_key();
        self.nodes.insert(node, Node::new(key));
        vec![Action::AddDial { node, key }]
    }

    /// Stops tracking a PC no longer listed; the runtime closes its handles.
    pub(crate) fn forget(&mut self, node: &PeerId) {
        self.nodes.remove(node);
    }

    pub(crate) fn nodes(&self) -> impl Iterator<Item = &PeerId> {
        self.nodes.keys()
    }

    /// The handshake's check of a PC's run (rule (b)): a run that was replaced, or refused as a duplicate, within [`DUPLICATE_MEMORY`]
    ///   is refused — and warned about here.
    pub(crate) fn admit(
        &mut self,
        node: &PeerId,
        instance: Instance,
        now: Instant,
    ) -> Result<(), DuplicateKind> {
        let Some(n) = self.nodes.get_mut(node) else {
            return Ok(());
        };
        n.prune(now);
        if n.replaced.iter().any(|(seen, _)| *seen == instance) {
            n.warn(DuplicateKind::Duplicate, false);
            return Err(DuplicateKind::Duplicate);
        }
        Ok(())
    }

    /// A link of `node` reached the operating phase with the run `instance`. `dial_idle` = the PC's dialing handle is between attempts
    ///   (backing off, idle or gave up), not mid-attempt — the runtime reads it from the handle.
    pub(crate) fn link_up(
        &mut self,
        node: PeerId,
        key: LinkKey,
        dir: Dir,
        instance: Instance,
        dial_idle: bool,
        now: Instant,
    ) -> Vec<Action> {
        let own = self.own;
        let nonce = self.next_nonce;
        let mut out = Vec::new();
        let Some(n) = self.nodes.get_mut(&node) else {
            // Not tracked (removed meanwhile) — the per-frame check would drop it anyway.
            out.push(Action::Close {
                key,
                how: CloseAs::Refused,
                reason: "not paired",
            });
            return out;
        };
        n.prune(now);
        if n.dial_key() == Some(key) {
            // A dialing handle that connects is dialing again (a re-arm already set it active).
            n.dial = Dial::Active(key);
        }
        let new = Link {
            key,
            dir,
            instance,
            up_at: now,
        };
        if n.probe.is_some() {
            // One question at a time; the other side redials after its backoff.
            out.push(Action::Close {
                key,
                how: CloseAs::GoingAway,
                reason: "a duplicate check is running",
            });
            if n.dial_key() == Some(key) {
                n.dial = Dial::Removed;
            }
            return out;
        }
        match n.chosen_link() {
            None => {
                if let Some(last) = n.last_instance.filter(|last| *last != instance) {
                    n.replaced.push((last, now));
                }
                n.last_instance = Some(instance);
                n.links.push(new);
                n.chosen = Some(key);
            }
            Some(old) if old.instance == instance => {
                n.links.push(new);
                let together =
                    now.saturating_duration_since(old.up_at) <= DIALED_TOGETHER && old.dir != dir;
                let (keep, drop) = if together {
                    // The link dialed by the smaller ID: ours when our ID is smaller.
                    let keep_dir = if own < node {
                        Dir::Dialed
                    } else {
                        Dir::Accepted
                    };
                    if dir == keep_dir {
                        (new, old)
                    } else {
                        (old, new)
                    }
                } else {
                    // ADR-0389 d8 (c): only a same-direction replacement counts — across directions it is an ordinary reconnect.
                    if old.dir == dir {
                        n.flaps.push(now);
                    }
                    (new, old)
                };
                n.chosen = Some(keep.key);
                n.close(drop, CloseAs::GoingAway, "duplicate link", &mut out);
                if n.flaps.len() >= FLAP_COUNT {
                    n.flaps.clear();
                    n.warn(DuplicateKind::Suspected, false);
                    out.push(Action::Send {
                        key: keep.key,
                        msg: PeerMsg::Duplicate {
                            kind: DuplicateKind::Suspected,
                        },
                    });
                }
            }
            Some(old) => {
                n.links.push(new);
                n.probe = Some(Probe {
                    nonce,
                    old: old.key,
                    new: key,
                    deadline: now + PROBE_WINDOW,
                    claimed: false,
                });
                self.next_nonce = nonce.wrapping_add(1);
                out.push(Action::Send {
                    key: old.key,
                    msg: PeerMsg::Probe { nonce },
                });
                return out;
            }
        }
        let idle = if dial_idle { n.dial_key() } else { None };
        n.settle_dial(idle, &mut out);
        out
    }

    /// A `ProbeAck` arrived on `key`.
    pub(crate) fn probe_ack(&mut self, key: LinkKey, nonce: u64, now: Instant) -> Vec<Action> {
        let mut out = Vec::new();
        let Some(n) = self
            .nodes
            .values_mut()
            .find(|n| n.probe.is_some_and(|p| p.old == key))
        else {
            return out;
        };
        let Some(probe) = n.probe.filter(|p| p.nonce == nonce && now <= p.deadline) else {
            return out;
        };
        n.probe = None;
        let Some(newer) = n.link(probe.new) else {
            return out;
        };
        // Both runs are alive: the newer is refused, both are told, and its run is refused at its next handshake.
        n.replaced.push((newer.instance, now));
        n.warn(DuplicateKind::Duplicate, false);
        let duplicate = PeerMsg::Duplicate {
            kind: DuplicateKind::Duplicate,
        };
        out.push(Action::Send {
            key: newer.key,
            msg: duplicate.clone(),
        });
        n.close(newer, CloseAs::Refused, "duplicate identity", &mut out);
        out.push(Action::Send {
            key: probe.old,
            msg: duplicate,
        });
        out
    }

    /// The other PC sent a `Duplicate` warning.
    /// Returns whether the PC's warning changed.
    pub(crate) fn duplicate_reported(&mut self, node: &PeerId, kind: DuplicateKind) -> bool {
        let Some(n) = self.nodes.get_mut(node) else {
            return false;
        };
        let before = n.warning;
        n.warn(kind, true);
        n.warning != before
    }

    /// A live link ended (or the handle did). Idempotent.
    pub(crate) fn link_down(&mut self, key: LinkKey, now: Instant) -> Vec<Action> {
        let mut out = Vec::new();
        let Some(node) = self.find(|n| n.link(key).is_some()) else {
            return out;
        };
        let Some(n) = self.nodes.get_mut(&node) else {
            return out;
        };
        let was = n.link(key);
        n.drop_link(key);
        if let Some(probe) = n.probe {
            if probe.old == key {
                // The older run's link died before it answered — the newer one is the run now.
                n.probe = None;
                if let Some(old) = was {
                    n.replaced.push((old.instance, now));
                }
                if let Some(newer) = n.link(probe.new) {
                    n.chosen = Some(newer.key);
                    n.last_instance = Some(newer.instance);
                }
            } else if probe.new == key {
                n.probe = None;
            }
        }
        // The dialing handle whose link just ended is between attempts now.
        n.settle_dial(Some(key), &mut out);
        if n.links.is_empty() && n.dial == Dial::Removed {
            let fresh = self.next_key;
            self.next_key += 1;
            n.dial = Dial::Active(fresh);
            out.push(Action::AddDial { node, key: fresh });
        }
        out
    }

    /// The handle `key` ended for good (its inbox closed). A dialing handle the book did not close is replaced.
    pub(crate) fn handle_ended(&mut self, key: LinkKey, now: Instant) -> Vec<Action> {
        let mut out = self.link_down(key, now);
        let Some(node) = self.find(|n| n.dial_key() == Some(key)) else {
            return out;
        };
        let Some(n) = self.nodes.get_mut(&node) else {
            return out;
        };
        if n.links.is_empty() {
            let fresh = self.next_key;
            self.next_key += 1;
            n.dial = Dial::Active(fresh);
            out.push(Action::AddDial { node, key: fresh });
        } else {
            n.dial = Dial::Removed;
        }
        out
    }

    fn find(&self, pred: impl Fn(&Node) -> bool) -> Option<PeerId> {
        self.nodes.iter().find(|(_, n)| pred(n)).map(|(id, _)| *id)
    }

    /// The dialing handle `key` failed an attempt and is backing off — closed now if an accepted link of its PC is the chosen one.
    pub(crate) fn dial_failed(&mut self, key: LinkKey) -> Vec<Action> {
        let mut out = Vec::new();
        if let Some(n) = self
            .nodes
            .values_mut()
            .find(|n| n.dial == Dial::Active(key))
        {
            n.settle_dial(Some(key), &mut out);
        }
        out
    }

    /// The dialing handle `key` received a terminal refusal.
    pub(crate) fn dial_gave_up(&mut self, key: LinkKey, why: Refusal, now: Instant) -> Vec<Action> {
        let mut out = Vec::new();
        if let Some(n) = self
            .nodes
            .values_mut()
            .find(|n| n.dial == Dial::Active(key))
        {
            n.dial = Dial::GaveUp { key, why, at: now };
            n.settle_dial(None, &mut out);
        }
        out
    }

    /// Re-arms every dialing handle that gave up (a trusted-list add).
    pub(crate) fn rearm_all(&mut self) -> Vec<Action> {
        let mut out = Vec::new();
        for n in self.nodes.values_mut() {
            if let Dial::GaveUp { key, .. } = n.dial {
                n.dial = Dial::Active(key);
                out.push(Action::Redial { key });
            }
        }
        out
    }

    /// Timers: open probes past their window, "not paired" refusals due for the slow retry.
    pub(crate) fn tick(&mut self, now: Instant) -> Vec<Action> {
        let mut out = Vec::new();
        for n in self.nodes.values_mut() {
            n.prune(now);
            if let Some(probe) = n.probe.filter(|p| now > p.deadline) {
                // No answer — the older link was stale (a plain restart of the other side).
                n.probe = None;
                if let Some(old) = n.link(probe.old) {
                    n.replaced.push((old.instance, now));
                    n.close(old, CloseAs::GoingAway, "replaced by a newer run", &mut out);
                }
                if let Some(newer) = n.link(probe.new) {
                    n.chosen = Some(newer.key);
                    n.last_instance = Some(newer.instance);
                }
                n.settle_dial(None, &mut out);
            }
            if let Dial::GaveUp {
                key,
                why: Refusal::NotPaired,
                at,
            } = n.dial
            {
                if n.links.is_empty() && now.saturating_duration_since(at) >= NOT_PAIRED_RETRY {
                    n.dial = Dial::Active(key);
                    out.push(Action::Redial { key });
                }
            }
        }
        out
    }

    /// When [`LinkBook::tick`] next has something to do.
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.nodes
            .values()
            .flat_map(|n| {
                let probe = n.probe.map(|p| p.deadline + Duration::from_millis(1));
                let retry = match n.dial {
                    Dial::GaveUp {
                        why: Refusal::NotPaired,
                        at,
                        ..
                    } if n.links.is_empty() => Some(at + NOT_PAIRED_RETRY),
                    _ => None,
                };
                [probe, retry]
            })
            .flatten()
            .min()
    }

    /// The node a key belongs to (a live link or the dialing handle).
    pub(crate) fn node_of(&self, key: LinkKey) -> Option<PeerId> {
        self.nodes
            .iter()
            .find(|(_, n)| n.link(key).is_some() || n.dial_key() == Some(key))
            .map(|(id, _)| *id)
    }

    /// The chosen link of `node` — where pushes such as directory frames go.
    pub(crate) fn chosen(&self, node: &PeerId) -> Option<LinkKey> {
        self.nodes.get(node).and_then(|n| n.chosen)
    }

    pub(crate) fn online(&self, node: &PeerId) -> Online {
        let Some(n) = self.nodes.get(node) else {
            return Online::Offline;
        };
        if !n.links.is_empty() {
            return Online::Online;
        }
        match n.dial {
            Dial::GaveUp { why, .. } => match why {
                Refusal::NotPaired => Online::Refused,
                Refusal::VersionMismatch => Online::VersionMismatch,
                Refusal::Duplicate(_) => Online::DuplicateIdentity,
            },
            Dial::Active(_) | Dial::Removed => Online::Offline,
        }
    }

    #[cfg(test)]
    pub(crate) fn live_links(&self, node: &PeerId) -> usize {
        self.nodes.get(node).map_or(0, |n| n.links.len())
    }

    #[cfg(test)]
    pub(crate) fn chosen_dir(&self, node: &PeerId) -> Option<Dir> {
        self.nodes
            .get(node)
            .and_then(|n| n.chosen_link())
            .map(|l| l.dir)
    }

    pub(crate) fn warning(&self, node: &PeerId) -> Option<Warning> {
        self.nodes.get(node).and_then(|n| n.warning)
    }
}

#[cfg(test)]
mod tests;
