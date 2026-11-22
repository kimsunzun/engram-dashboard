//! Approvals — the trust changes an agent may only request and the questions a human answers about the peer state, asked through the
//!   daemon's one "ask the user" seam ([`crate::ask_user`]).
//!
//! What is held here (memory only — a restart drops every request):
//! - ★Pending requests★: a gated change ([`Change`]) an agent asked for, with the agent, the time and a best-effort origin ("after mail
//!   from `<pc>`" — a hint for the human, not a security boundary). At most [`MAX_PENDING`]; a request identical to a pending one is not
//!   added twice; each expires after [`REQUEST_TTL`]. After a deny the same (agent, kind) is refused for [`DENY_COOLDOWN`]; "deny and stop
//!   asking" refuses that agent's gated calls until the daemon restarts. A network change holds only the fields asked for and is merged
//!   with the settings of the moment it is applied, under the store lock, so approving it later never undoes a change to another field;
//!   a named field that moved since the request is reported ([`Applied::NetworkMoved`]).
//! - ★System questions★ derived from the peer state: the trusted-list accept question (changed outside Engram, or damaged) and the
//!   adopt question (this account's identity under another machine id — one machine id at a time, so planted files cannot open a dialog
//!   each). They stand while the state needs them. The accept question: "not now" holds it back for [`DENY_COOLDOWN`], "not this run"
//!   until the daemon restarts. The adopt question: deny = "not this run" for the question as a whole, its third answer = "start fresh"
//!   (a new identity instead of the saved one).
//!
//! ★Shutdown★ ([`Approvals::stop`]): every answer holds the read side of the stop gate from its consume through its apply; `stop` first
//!   lets nothing new through, then waits for the write side — an answer being applied — at most [`STOP_WAIT`], and drops every request.
//!
//! ★Every answer consumes its request atomically before anything is applied★ — a second answer, or one after expiry, finds nothing and is
//!   ignored. Answers come from a human only: `peer.approve` / `peer.deny` from the app (a live shell connection — the command gate keeps
//!   agents out) or the OS dialog. The change is applied after the consume, outside this module's lock (the trust operations are blocking).
//!
//! Lock order: this module's lock, then the seam's (`AskUser::ask` / `withdraw`); the seam calls back without holding its lock. The
//!   peer state ([`TrustOps::snapshot`]) is read before this module's lock is taken.
//!
//! The question text is built here in every UI language ([`text`]) from the state at the time it is asked.

// ADR-0394, ADR-0389, ADR-0372

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock, TryLockError, Weak};
use std::time::{Duration, Instant, SystemTime};

use engram_dashboard_base::sync::{lock, read};
use engram_dashboard_base::time::Clock;
use tokio::sync::watch;

use super::id::PeerId;
use super::runtime::{PeerLink, PeerSnapshot};
use super::store::{PeerEdit, PeerError, PeerStatus, Regenerated, Removal};
pub(crate) use super::trusted::NetworkPatch;
use super::trusted::{NetworkSettings, TrustedPeer};
use crate::ask_user::{Answer, AppRoute, AskUser, Dialogs, LanguageSource, Question, QuestionId};

pub(crate) mod text;

pub(crate) use text::quote;

/// A pending request expires after this.
pub(crate) const REQUEST_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// At most this many agent requests wait at once.
pub(crate) const MAX_PENDING: usize = 8;
/// [`Approvals::stop`] waits at most this for an answer being applied.
const STOP_WAIT: Duration = Duration::from_secs(3);
/// After a deny, the same (agent, kind) is refused this long without asking; a system question held back by "not now" returns after it.
pub(crate) const DENY_COOLDOWN: Duration = Duration::from_secs(10 * 60);

/// Who calls a `peer.*` command — the typed caller as the peer module sees it (no agent-crate types here).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PeerCaller {
    /// A live shell connection — a human.
    Human,
    Agent(Requester),
}

/// The agent behind a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Requester {
    /// The agent id (its token's identity).
    pub(crate) id: String,
    /// Its roster name when the call came in.
    pub(crate) name: Option<String>,
}

/// A trust change an agent may only request (ADR-0394 decision 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Change {
    Add(TrustedPeer),
    SetNetwork(NetworkPatch),
    /// A new identity key — or, while the adopt question is open, a fresh one instead of adopting.
    RegenerateKey,
    /// A rename (with an address change, if one came along).
    Edit {
        id: PeerId,
        edit: PeerEdit,
    },
}

/// The kind of a request — the cooldown key and the dialog rate key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    Add,
    SetNetwork,
    RegenerateKey,
    Rename,
    AcceptList,
    Adopt,
}

impl Kind {
    pub(crate) fn word(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::SetNetwork => "setNetwork",
            Self::RegenerateKey => "regenerateKey",
            Self::Rename => "rename",
            Self::AcceptList => "acceptList",
            Self::Adopt => "adopt",
        }
    }
}

impl Change {
    pub(crate) fn kind(&self) -> Kind {
        match self {
            Self::Add(_) => Kind::Add,
            Self::SetNetwork(_) => Kind::SetNetwork,
            Self::RegenerateKey => Kind::RegenerateKey,
            Self::Edit { .. } => Kind::Rename,
        }
    }
}

/// The trust operations an approved change applies to — production = [`PeerLink`]. ★Blocking★ (disk, DPAPI): call off the async tasks.
pub(crate) trait TrustOps: Send + Sync {
    fn snapshot(&self) -> PeerSnapshot;
    fn ensure_identity(&self) -> Result<PeerId, PeerError>;
    fn add(&self, peer: TrustedPeer) -> Result<(), PeerError>;
    fn edit(&self, id: &PeerId, edit: PeerEdit) -> Result<(), PeerError>;
    fn remove(&self, id: &PeerId) -> Result<Removal, PeerError>;
    /// Merges, checks and saves under the store lock; returns the settings it found.
    fn patch_network(&self, patch: &NetworkPatch) -> Result<NetworkSettings, PeerError>;
    fn regenerate(&self) -> Result<Regenerated, PeerError>;
    fn accept_list(&self) -> Result<(), PeerError>;
    fn adopt(&self, machine: &str) -> Result<(), PeerError>;
    fn start_fresh(&self) -> Result<PeerId, PeerError>;
}

impl TrustOps for PeerLink {
    fn snapshot(&self) -> PeerSnapshot {
        PeerLink::snapshot(self)
    }
    fn ensure_identity(&self) -> Result<PeerId, PeerError> {
        PeerLink::ensure_identity(self)
    }
    fn add(&self, peer: TrustedPeer) -> Result<(), PeerError> {
        PeerLink::add(self, peer)
    }
    fn edit(&self, id: &PeerId, edit: PeerEdit) -> Result<(), PeerError> {
        PeerLink::edit(self, id, edit)
    }
    fn remove(&self, id: &PeerId) -> Result<Removal, PeerError> {
        PeerLink::remove(self, id)
    }
    fn patch_network(&self, patch: &NetworkPatch) -> Result<NetworkSettings, PeerError> {
        PeerLink::patch_network(self, patch)
    }
    fn regenerate(&self) -> Result<Regenerated, PeerError> {
        PeerLink::regenerate(self)
    }
    fn accept_list(&self) -> Result<(), PeerError> {
        PeerLink::accept_list(self)
    }
    fn adopt(&self, machine: &str) -> Result<(), PeerError> {
        PeerLink::adopt(self, machine)
    }
    fn start_fresh(&self) -> Result<PeerId, PeerError> {
        PeerLink::start_fresh(self)
    }
}

/// Where the trust operations come from — the daemon's peer link once it has started (it starts after the command table exists).
pub(crate) trait TrustSource: Send + Sync {
    fn ops(&self) -> Option<&dyn TrustOps>;
}

impl<T: TrustOps> TrustSource for OnceLock<T> {
    fn ops(&self) -> Option<&dyn TrustOps> {
        self.get().map(|ops| ops as &dyn TrustOps)
    }
}

/// What an applied change did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Applied {
    Done,
    /// A new key replaced the old one — every other PC must remove the old ID and pair again.
    Regenerated(Regenerated),
    /// A fresh identity instead of adopting another machine's.
    Fresh(PeerId),
    /// An approved network request was applied, but these fields it names had changed since it was asked — its values replaced them.
    NetworkMoved(Vec<&'static str>),
}

/// Applies a change (a human's direct call, or an approved request) to the state of this moment. A network change is merged with the
///   settings of that moment, checked and saved in one store step (`PeerStore::patch_network`) — a concurrent change to another field
///   is kept.
pub(crate) fn apply(ops: &dyn TrustOps, change: &Change) -> Result<Applied, PeerError> {
    match change {
        Change::Add(peer) => ops.add(peer.clone()).map(|()| Applied::Done),
        Change::SetNetwork(patch) => ops.patch_network(patch).map(|_| Applied::Done),
        Change::Edit { id, edit } => ops.edit(id, edit.clone()).map(|()| Applied::Done),
        Change::RegenerateKey => match ops.snapshot().status {
            PeerStatus::AdoptOrFresh { .. } => ops.start_fresh().map(Applied::Fresh),
            _ => ops.regenerate().map(Applied::Regenerated),
        },
    }
}

/// Why an agent's request was not recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The user said "deny and stop asking" for this agent this run.
    Stopped,
    /// Denied this long ago (under [`DENY_COOLDOWN`]).
    CoolingDown(Duration),
    /// [`MAX_PENDING`] requests already wait.
    Full,
    /// The daemon is shutting down.
    Stopping,
    /// A network change while the current settings are not known.
    NetworkUnknown,
}

/// A recorded request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Requested {
    pub(crate) id: String,
    /// An identical request was already pending — that one stands.
    pub(crate) existing: bool,
}

/// A human's decision on a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decision {
    Approve,
    /// The adopt question only: a fresh identity instead of the saved one.
    StartFresh,
    Deny,
    DenyAndStop,
}

/// What answering a request did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Answered {
    Applied(Applied),
    /// Approved, but the change was refused when applied — the request is consumed.
    Failed(PeerError),
    Denied,
}

/// Why an answer found nothing to answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NoRequest {
    /// No pending request has that id — answered already, expired, or never made.
    Unknown,
    /// The peer link has not started, so nothing can be applied (the request stays).
    NotRunning,
    /// "Start fresh" answers only the adopt question (the request stays).
    NotAdopt,
}

/// One pending request as `peer.list` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingRow {
    pub(crate) id: String,
    pub(crate) kind: Kind,
    /// One line saying what is asked (outside strings quoted and made visible).
    pub(crate) summary: String,
    /// `None` for a system question.
    pub(crate) agent: Option<Requester>,
    pub(crate) origin: Option<String>,
    pub(crate) asked_at: SystemTime,
    /// `None` = stands while the state needs it.
    pub(crate) expires_in: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum SystemKey {
    AcceptList,
    Adopt(String),
}

#[derive(Debug, Clone)]
enum Asked {
    Change {
        change: Change,
        agent: Requester,
        origin: Option<String>,
        /// The network settings the question showed as "from" — compared again when a network request is applied.
        network_then: Option<NetworkSettings>,
    },
    System(SystemKey),
}

/// What a "not now" / "not this run" holds back — the adopt question as a whole, not one machine id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Hold {
    AcceptList,
    Adopt,
}

impl SystemKey {
    fn hold(&self) -> Hold {
        match self {
            SystemKey::AcceptList => Hold::AcceptList,
            SystemKey::Adopt(_) => Hold::Adopt,
        }
    }
}

#[derive(Debug, Clone)]
struct Request {
    id: String,
    question: QuestionId,
    asked: Asked,
    summary: String,
    at: Instant,
    at_wall: SystemTime,
}

impl Request {
    fn expired(&self, now: Instant) -> bool {
        matches!(self.asked, Asked::Change { .. }) && now >= self.at + REQUEST_TTL
    }
}

#[derive(Default)]
struct Inner {
    /// Oldest first.
    pending: Vec<Request>,
    next_question: QuestionId,
    /// (agent id, kind) → when the user denied it.
    denied: HashMap<(String, Kind), Instant>,
    /// Agents whose gated calls are refused until the daemon restarts.
    stopped: HashSet<String>,
    /// System questions held back: until when (`None` = this run).
    held_back: HashMap<Hold, Option<Instant>>,
    /// Agent id → the PC whose mail most recently reached that agent this run.
    origins: HashMap<String, String>,
    /// Lowercase PC name → the ID that held it, as seen this run — an add of that name for another ID says so.
    seen_names: HashMap<String, PeerId>,
}

impl Inner {
    fn see(&mut self, snapshot: &PeerSnapshot) {
        for row in &snapshot.peers {
            self.seen_names
                .insert(row.peer.name.to_lowercase(), row.peer.id);
        }
    }
}

/// The approvals (module head).
pub(crate) struct Approvals {
    inner: Mutex<Inner>,
    /// The stop gate. Every answer and request holds the read side from its check through its apply; [`Approvals::stop`] raises
    ///   `stopping` first (nothing new gets through), then waits for the write side — an answer being applied — at most [`STOP_WAIT`].
    stopped: RwLock<()>,
    stopping: AtomicBool,
    ask: AskUser,
    ops: Arc<dyn TrustSource>,
    clock: Arc<dyn Clock>,
    changes: watch::Sender<u64>,
}

impl Approvals {
    /// Starts the seam's dialog thread with answers routed back here; `language` = the user's UI language for the dialogs.
    pub(crate) fn start(
        ops: Arc<dyn TrustSource>,
        dialogs: Arc<dyn Dialogs>,
        app: Arc<dyn AppRoute>,
        language: Arc<dyn LanguageSource>,
        clock: Arc<dyn Clock>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me: &Weak<Self>| {
            let me = me.clone();
            let ask = AskUser::start(
                dialogs,
                app,
                language,
                clock.clone(),
                Box::new(move |question, answer| {
                    if let Some(approvals) = me.upgrade() {
                        approvals.answer_question(question, answer);
                    }
                }),
            );
            Self {
                inner: Mutex::new(Inner::default()),
                stopped: RwLock::new(()),
                stopping: AtomicBool::new(false),
                ask,
                ops,
                clock,
                changes: watch::channel(0).0,
            }
        })
    }

    /// The trust operations, once the peer link runs.
    pub(crate) fn ops(&self) -> Option<&dyn TrustOps> {
        self.ops.ops()
    }

    /// Ticks whenever the pending requests change — the app route broadcasts it beside [`PeerLink::changes`].
    pub(crate) fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// The app started or stopped showing the PC list slot.
    pub(crate) fn route_changed(&self) {
        self.ask.route_changed();
    }

    /// Daemon shutdown, before the peer link stops: from now no answer or request gets through; waits at most [`STOP_WAIT`] for an
    ///   answer being applied (past it, that apply finishes on its own — the peer link refuses new trust operations once it stops), then
    ///   drops every request and ends the dialog thread (an open dialog is dismissed, bounded too). Not to be called from an answer.
    pub(crate) fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
        let deadline = Instant::now() + STOP_WAIT;
        loop {
            match self.stopped.try_write() {
                Ok(_) | Err(TryLockError::Poisoned(_)) => break,
                Err(TryLockError::WouldBlock) if Instant::now() >= deadline => {
                    tracing::warn!(
                        wait_ms = STOP_WAIT.as_millis() as u64,
                        "an approved request was still being applied at shutdown — not waited for any longer"
                    );
                    break;
                }
                Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        let questions: Vec<QuestionId> = lock(&self.inner)
            .pending
            .drain(..)
            .map(|r| r.question)
            .collect();
        for question in questions {
            self.ask.withdraw(question);
        }
        self.ask.stop();
        self.bump();
    }

    /// Records that a mail from `pc` reached the agent `agent_id` this run — the origin a later request of that agent names.
    pub(crate) fn note_mail_from(&self, agent_id: &str, pc: &str) {
        lock(&self.inner)
            .origins
            .insert(agent_id.to_string(), pc.to_string());
    }

    /// Records an agent's request and asks the user — answered at once, never waiting for the human. The question is written from the
    ///   peer state of this moment.
    pub(crate) fn request(&self, change: Change, agent: Requester) -> Result<Requested, Refusal> {
        self.request_seen(change, agent, self.ops().map(|ops| ops.snapshot()))
    }

    /// [`Approvals::request`] with the peer state the caller checked the change against — the question and a network request's
    ///   "from" (`network_then`) are written from exactly that state, never a later read. A network request whose state has no known
    ///   settings is refused.
    pub(crate) fn request_seen(
        &self,
        change: Change,
        agent: Requester,
        snapshot: Option<PeerSnapshot>,
    ) -> Result<Requested, Refusal> {
        if matches!(change, Change::SetNetwork(_))
            && snapshot.as_ref().and_then(|s| s.network.as_ref()).is_none()
        {
            return Err(Refusal::NetworkUnknown);
        }
        let now = self.clock.now();
        let kind = change.kind();
        let stopped = read(&self.stopped);
        if self.stopping.load(Ordering::Acquire) {
            return Err(Refusal::Stopping);
        }
        let mut inner = lock(&self.inner);
        self.expire_locked(&mut inner, now);
        if let Some(snapshot) = &snapshot {
            inner.see(snapshot);
        }
        if inner.stopped.contains(&agent.id) {
            return Err(Refusal::Stopped);
        }
        if let Some(at) = inner.denied.get(&(agent.id.clone(), kind)) {
            let ago = now.saturating_duration_since(*at);
            if ago < DENY_COOLDOWN {
                return Err(Refusal::CoolingDown(ago));
            }
        }
        if let Some(same) = inner
            .pending
            .iter()
            .find(|r| matches!(&r.asked, Asked::Change { change: c, .. } if *c == change))
        {
            return Ok(Requested {
                id: same.id.clone(),
                existing: true,
            });
        }
        let agent_requests = inner
            .pending
            .iter()
            .filter(|r| matches!(r.asked, Asked::Change { .. }))
            .count();
        if agent_requests >= MAX_PENDING {
            return Err(Refusal::Full);
        }
        let origin = inner.origins.get(&agent.id).cloned();
        let id = request_id(&inner);
        let question = next_question(&mut inner);
        let text = change_question(
            &change,
            snapshot.as_ref(),
            &inner.seen_names,
            &agent,
            origin.as_deref(),
            &id,
        );
        let summary = change_summary(&change);
        tracing::info!(request = %id, kind = kind.word(), agent = %agent.id, "an agent asked for a trust change — waiting for the user's approval");
        inner.pending.push(Request {
            id: id.clone(),
            question,
            asked: Asked::Change {
                change,
                agent,
                origin,
                network_then: snapshot.as_ref().and_then(|s| s.network.clone()),
            },
            summary,
            at: now,
            at_wall: SystemTime::now(),
        });
        self.ask.ask(question, text);
        drop(inner);
        drop(stopped);
        self.bump();
        Ok(Requested {
            id,
            existing: false,
        })
    }

    /// A human's answer by request id (`peer.approve` / `peer.deny`). Consumes the request before applying it.
    pub(crate) fn answer(&self, id: &str, decision: Decision) -> Result<Answered, NoRequest> {
        let now = self.clock.now();
        let stopped = read(&self.stopped);
        if self.stopping.load(Ordering::Acquire) {
            return Err(NoRequest::Unknown);
        }
        let request = {
            let mut inner = lock(&self.inner);
            self.expire_locked(&mut inner, now);
            let Some(at) = inner.pending.iter().position(|r| r.id == id) else {
                return Err(NoRequest::Unknown);
            };
            if decision == Decision::StartFresh
                && !matches!(inner.pending[at].asked, Asked::System(SystemKey::Adopt(_)))
            {
                return Err(NoRequest::NotAdopt);
            }
            if matches!(decision, Decision::Approve | Decision::StartFresh) && self.ops().is_none()
            {
                return Err(NoRequest::NotRunning);
            }
            self.consume(&mut inner, at, decision, now)
        };
        self.ask.withdraw(request.question);
        self.bump();
        let answered = self.carry_out(&request, decision);
        drop(stopped);
        Ok(answered)
    }

    /// The dialog's answer to the question it showed — the same consume, then the same application. The adopt question's third
    ///   choice is "start fresh".
    fn answer_question(&self, question: QuestionId, answer: Answer) {
        let now = self.clock.now();
        let stopped = read(&self.stopped);
        if self.stopping.load(Ordering::Acquire) {
            return;
        }
        let (request, decision) = {
            let mut inner = lock(&self.inner);
            self.expire_locked(&mut inner, now);
            let Some(at) = inner.pending.iter().position(|r| r.question == question) else {
                return;
            };
            let decision = match (answer, &inner.pending[at].asked) {
                (Answer::Approve, _) => Decision::Approve,
                (Answer::Deny, _) => Decision::Deny,
                (Answer::DenyAndStop, Asked::System(SystemKey::Adopt(_))) => Decision::StartFresh,
                (Answer::DenyAndStop, _) => Decision::DenyAndStop,
            };
            if matches!(decision, Decision::Approve | Decision::StartFresh) && self.ops().is_none()
            {
                // Cannot happen once questions exist (the peer link starts before any is asked); the request stays for the app.
                tracing::warn!(request = %inner.pending[at].id, "an approval came while the peer link is not running — the request stays");
                return;
            }
            (self.consume(&mut inner, at, decision, now), decision)
        };
        self.bump();
        let answered = self.carry_out(&request, decision);
        drop(stopped);
        match answered {
            Answered::Failed(e) => {
                tracing::warn!(request = %request.id, "an approved request could not be applied: {e}")
            }
            outcome => {
                tracing::info!(request = %request.id, ?outcome, "the user answered a request in the approval dialog")
            }
        }
    }

    /// Removes the request at `at` and records what a deny means — under the lock.
    fn consume(&self, inner: &mut Inner, at: usize, decision: Decision, now: Instant) -> Request {
        let request = inner.pending.remove(at);
        match (&request.asked, decision) {
            (_, Decision::Approve | Decision::StartFresh) => {}
            (Asked::Change { change, agent, .. }, Decision::Deny) => {
                inner.denied.insert((agent.id.clone(), change.kind()), now);
            }
            (Asked::Change { agent, .. }, Decision::DenyAndStop) => {
                inner.stopped.insert(agent.id.clone());
                // The user asked not to hear from this agent again — its other requests go too.
                let theirs: Vec<QuestionId> = inner
                    .pending
                    .iter()
                    .filter(
                        |r| matches!(&r.asked, Asked::Change { agent: a, .. } if a.id == agent.id),
                    )
                    .map(|r| r.question)
                    .collect();
                inner.pending.retain(|r| !theirs.contains(&r.question));
                for question in theirs {
                    self.ask.withdraw(question);
                }
            }
            // The adopt question's third answer is "start fresh", so its deny is "not this run" (asked again at the next start).
            (Asked::System(SystemKey::Adopt(_)), Decision::Deny | Decision::DenyAndStop) => {
                inner.held_back.insert(Hold::Adopt, None);
            }
            (Asked::System(key), Decision::Deny) => {
                inner
                    .held_back
                    .insert(key.hold(), Some(now + DENY_COOLDOWN));
            }
            (Asked::System(key), Decision::DenyAndStop) => {
                inner.held_back.insert(key.hold(), None);
            }
        }
        request
    }

    /// Applies an approved request (outside the lock).
    fn carry_out(&self, request: &Request, decision: Decision) -> Answered {
        if matches!(decision, Decision::Deny | Decision::DenyAndStop) {
            tracing::info!(request = %request.id, "the user denied a request");
            return Answered::Denied;
        }
        let Some(ops) = self.ops() else {
            return Answered::Failed(PeerError::Blocked(
                "the peer link is not running".to_string(),
            ));
        };
        let applied = match (&request.asked, decision) {
            (Asked::System(SystemKey::Adopt(_)), Decision::StartFresh) => {
                ops.start_fresh().map(Applied::Fresh)
            }
            (
                Asked::Change {
                    change: Change::SetNetwork(patch),
                    network_then,
                    ..
                },
                _,
            ) => ops.patch_network(patch).map(|found| {
                // Without the settings the question showed, every named field counts as moved — never a silent `Done`.
                let moved = match network_then {
                    Some(then) => patch.moved(then, &found),
                    None => patch.named(),
                };
                if moved.is_empty() {
                    Applied::Done
                } else {
                    tracing::warn!(request = %request.id, ?moved, "an approved network request replaced settings changed since it was asked");
                    Applied::NetworkMoved(moved)
                }
            }),
            (Asked::Change { change, .. }, _) => apply(ops, change),
            (Asked::System(SystemKey::AcceptList), _) => ops.accept_list().map(|()| Applied::Done),
            (Asked::System(SystemKey::Adopt(machine)), _) => {
                ops.adopt(machine).map(|()| Applied::Done)
            }
        };
        match applied {
            Ok(applied) => Answered::Applied(applied),
            Err(e) => Answered::Failed(e),
        }
    }

    /// The pending requests, oldest first (expired ones dropped).
    pub(crate) fn pending(&self) -> Vec<PendingRow> {
        let now = self.clock.now();
        let mut inner = lock(&self.inner);
        self.expire_locked(&mut inner, now);
        inner
            .pending
            .iter()
            .map(|r| {
                let (kind, agent, origin, expires_in) = match &r.asked {
                    Asked::Change {
                        change,
                        agent,
                        origin,
                        ..
                    } => (
                        change.kind(),
                        Some(agent.clone()),
                        origin.clone(),
                        Some((r.at + REQUEST_TTL).saturating_duration_since(now)),
                    ),
                    Asked::System(SystemKey::AcceptList) => (Kind::AcceptList, None, None, None),
                    Asked::System(SystemKey::Adopt(_)) => (Kind::Adopt, None, None, None),
                };
                PendingRow {
                    id: r.id.clone(),
                    kind,
                    summary: r.summary.clone(),
                    agent,
                    origin,
                    asked_at: r.at_wall,
                    expires_in,
                }
            })
            .collect()
    }

    /// Drops expired requests (their dialogs are dismissed). The daemon calls it on a timer; every read and answer calls it first.
    pub(crate) fn expire(&self) {
        let now = self.clock.now();
        let mut inner = lock(&self.inner);
        self.expire_locked(&mut inner, now);
    }

    fn expire_locked(&self, inner: &mut Inner, now: Instant) {
        let expired: Vec<QuestionId> = inner
            .pending
            .iter()
            .filter(|r| r.expired(now))
            .map(|r| r.question)
            .collect();
        if expired.is_empty() {
            return;
        }
        inner.pending.retain(|r| !r.expired(now));
        for question in expired {
            self.ask.withdraw(question);
        }
        tracing::info!("a trust-change request expired unanswered");
        self.bump();
    }

    /// Asks or withdraws the system questions the peer state needs now: accept the trusted list while it was changed outside Engram or
    ///   is damaged; adopt while this account's identity is found under another machine id — one question, the first machine id
    ///   (sorted), unless the user put the adopt question off for this run. The daemon calls it on every peer-state change and on its timer (a held-back question returns after its
    ///   cooldown).
    pub(crate) fn sync_state(&self, snapshot: &PeerSnapshot) {
        let now = self.clock.now();
        let mut inner = lock(&self.inner);
        inner.see(snapshot);
        inner
            .held_back
            .retain(|_, until| until.map_or(true, |until| until > now));
        let wanted: Vec<(SystemKey, usize)> = match &snapshot.status {
            PeerStatus::ListChangedOutside | PeerStatus::ListCorrupt(_) => {
                vec![(SystemKey::AcceptList, 0)]
            }
            PeerStatus::AdoptOrFresh { machines } => {
                let mut machines = machines.clone();
                machines.sort();
                // A pending adopt question stays the one asked while its machine is still offered.
                let asked = inner.pending.iter().find_map(|r| match &r.asked {
                    Asked::System(SystemKey::Adopt(m)) if machines.contains(m) => Some(m.clone()),
                    _ => None,
                });
                let pick = asked.or_else(|| {
                    (!inner.held_back.contains_key(&Hold::Adopt))
                        .then(|| machines.first().cloned())
                        .flatten()
                });
                pick.map(|m| (SystemKey::Adopt(m), machines.len() - 1))
                    .into_iter()
                    .collect()
            }
            _ => Vec::new(),
        };
        let mut changed = false;
        let stale: Vec<QuestionId> = inner
            .pending
            .iter()
            .filter(
                |r| matches!(&r.asked, Asked::System(key) if !wanted.iter().any(|(k, _)| k == key)),
            )
            .map(|r| r.question)
            .collect();
        if !stale.is_empty() {
            inner.pending.retain(|r| !stale.contains(&r.question));
            for question in stale {
                self.ask.withdraw(question);
            }
            changed = true;
        }
        for (key, others) in wanted {
            let asked = inner
                .pending
                .iter()
                .any(|r| matches!(&r.asked, Asked::System(k) if *k == key));
            if asked || inner.held_back.contains_key(&key.hold()) {
                continue;
            }
            let id = request_id(&inner);
            let question = next_question(&mut inner);
            let (text, summary) = system_question(&key, snapshot, others, &id);
            tracing::info!(request = %id, kind = ?key, "the peer state needs a human's answer — asking");
            inner.pending.push(Request {
                id,
                question,
                asked: Asked::System(key),
                summary,
                at: now,
                at_wall: SystemTime::now(),
            });
            self.ask.ask(question, text);
            changed = true;
        }
        drop(inner);
        if changed {
            self.bump();
        }
    }

    fn bump(&self) {
        self.changes.send_modify(|n| *n = n.wrapping_add(1));
    }
}

/// The dialog thread ends with the approvals (it holds only a weak handle back). No wait here: the last handle may drop on that thread.
impl Drop for Approvals {
    fn drop(&mut self) {
        self.ask.stop_now();
    }
}

/// How often [`follow`] drops expired requests and brings back held-back questions without a state change.
const FOLLOW_TICK: Duration = Duration::from_secs(60);

/// Keeps the system questions in step with the peer state and drops expired requests — on every change of the state and on a timer;
///   runs until its task is aborted (daemon shutdown).
pub(crate) async fn follow(link: PeerLink, approvals: Arc<Approvals>) {
    let mut changes = link.changes();
    let mut tick = tokio::time::interval(FOLLOW_TICK);
    loop {
        approvals.sync_state(&link.snapshot());
        approvals.expire();
        tokio::select! {
            changed = changes.changed() => {
                if changed.is_err() {
                    return;
                }
            }
            _ = tick.tick() => {}
        }
    }
}

fn next_question(inner: &mut Inner) -> QuestionId {
    inner.next_question += 1;
    inner.next_question
}

/// A request id — 12 random hex digits, so an answer meant for a request of an earlier run (or a stale screen) does not hit a new one.
fn request_id(inner: &Inner) -> String {
    loop {
        let mut bytes = [0u8; 6];
        let id = match getrandom::getrandom(&mut bytes) {
            Ok(()) => engram_dashboard_base::text::hex_lower(&bytes),
            // No OS randomness: still unique within this run; only the cross-run property is lost.
            Err(_) => format!("{:012x}", inner.next_question + 1),
        };
        if !inner.pending.iter().any(|r| r.id == id) {
            return id;
        }
    }
}

/// One line for `peer.list`.
fn change_summary(change: &Change) -> String {
    match change {
        Change::Add(peer) => format!("add PC {} ({})", quote(&peer.name), peer.id),
        Change::SetNetwork(_) => "change the network settings".to_string(),
        Change::RegenerateKey => "make a new identity key for this PC".to_string(),
        Change::Edit { id, edit } => match &edit.name {
            Some(name) => format!("rename PC {id} to {}", quote(name)),
            None => format!("change PC {id}"),
        },
    }
}

/// The question for an agent's change, from the state at request time.
fn change_question(
    change: &Change,
    snapshot: Option<&PeerSnapshot>,
    seen_names: &HashMap<String, PeerId>,
    agent: &Requester,
    origin: Option<&str>,
    request: &str,
) -> Question {
    let current_network = snapshot.and_then(|s| s.network.clone()).unwrap_or_default();
    let asked = match change {
        Change::Add(peer) => text::Asked::Add {
            peer,
            knocked: snapshot.is_some_and(|s| s.knocks.iter().any(|k| k.id == peer.id)),
            name_was: seen_names
                .get(&peer.name.to_lowercase())
                .copied()
                .filter(|held| *held != peer.id),
        },
        Change::SetNetwork(patch) => text::Asked::SetNetwork {
            from: &current_network,
            patch,
        },
        Change::RegenerateKey => text::Asked::RegenerateKey {
            instead_of_adopting: snapshot
                .is_some_and(|s| matches!(s.status, PeerStatus::AdoptOrFresh { .. })),
        },
        Change::Edit { id, edit } => {
            let row = snapshot.and_then(|s| s.peers.iter().find(|row| row.peer.id == *id));
            text::Asked::Rename {
                id: *id,
                from_name: row.map(|row| row.peer.name.as_str()),
                to_name: edit.name.as_deref(),
                from_address: row.and_then(|row| row.peer.address),
                to_address: edit.address,
            }
        }
    };
    text::change_question(&asked, &agent.id, agent.name.as_deref(), origin, request)
}

fn system_question(
    key: &SystemKey,
    snapshot: &PeerSnapshot,
    others: usize,
    request: &str,
) -> (Question, String) {
    match key {
        SystemKey::AcceptList => {
            let peers: Vec<TrustedPeer> =
                snapshot.peers.iter().map(|row| row.peer.clone()).collect();
            (
                text::accept_list_question(
                    matches!(snapshot.status, PeerStatus::ListCorrupt(_)),
                    &peers,
                    snapshot.network.as_ref(),
                    request,
                ),
                "accept the trusted list as it is now".to_string(),
            )
        }
        SystemKey::Adopt(machine) => (
            text::adopt_question(machine, others, request),
            format!(
                "adopt the identity saved under machine id {} (or start fresh)",
                quote(machine)
            ),
        ),
    }
}

#[cfg(test)]
mod tests;
