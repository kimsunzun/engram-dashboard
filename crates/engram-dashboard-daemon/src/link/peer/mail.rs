//! Mail over the peer link — what the link carries for the mail kernel, in PeerWire terms only (the kernel's adapters are the daemon's
//!   messaging host; this module knows no kernel type).
//!
//! Sending (A side, [`super::runtime::PeerLink::send_mail`]): one `Mail` request per (send, PC) on that PC's chosen link, under the
//!   registry's 120 s request deadline (ADR-0388 decision 8). ★Exactly one [`MailEnd`] per mail★, handed to the caller's callback: not
//!   sent (no live link, the in-flight cap [`IN_FLIGHT_CAP`], the encoded frame over the cap), the verdict, or unconfirmed (deadline, the
//!   link dropped after the mail was handed to it, or the verdict came from a link this PC no longer trusts — [`MailEnd::of`]).
//!
//! Receiving (B side): the drain task hands each `Mail` to that PC's receive queue ([`Inbound`]) and never runs it itself (the host injects
//!   inline, which blocks). ★Order is kept per (PC, recipient token)★: a frame waits only for earlier frames of that PC that share a `to`
//!   token, so one stuck recipient does not stall the PC's other mail; at most [`MAX_RUNNING`] frames of one PC run at once and at most
//!   [`MAX_QUEUED`] wait or run — a frame arriving at a full queue is refused whole with `REMOTE_LIMIT`, and one with more than
//!   [`MAX_MAIL_ROWS`] rows or more than [`MAX_MAIL_BYTES`] of text ([`mail_bytes`]) before it is queued, so a queue holds at most
//!   [`MAX_QUEUED`] × [`MAX_MAIL_BYTES`] of mail text whatever the frame cap (plus the copies of row tokens the queue keeps per job). ★A frame whose 120 s transport deadline passed while it waited — for the queue or for a
//!   blocking thread — is dropped, counted from its arrival on this PC★ (ADR-0404 decision 5 — the PCs share no clock; the sender has
//!   reported it unconfirmed by then). ★Every answer stays under the frame cap★ (a refusal answers at most [`MAX_MAIL_ROWS`] rows).

// ADR-0390, ADR-0393, ADR-0404

use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

use engram_dashboard_transport::RequestError;

use super::id::PeerId;
use super::wire::{DirAgent, Mail, PeerMsg, VerdictRow};

/// `Mail` requests in flight to one PC at once — the transport's pending table has no cap, and with a 120 s deadline a dead PC whose
///   connection has not idled out yet would pile up waiters.
pub(crate) const IN_FLIGHT_CAP: usize = 256;
/// Frames of one PC running at once.
pub(crate) const MAX_RUNNING: usize = 4;
/// Frames of one PC waiting or running.
pub(crate) const MAX_QUEUED: usize = 64;
/// Rows of one inbound mail — the kernel's `MAX_REMOTE_ROWS` (pinned equal in the mail host's tests); a frame with more is refused
///   before it is queued, with a bounded answer.
pub(crate) const MAX_MAIL_ROWS: usize = 64;
/// The text one inbound mail may carry ([`mail_bytes`]) — checked before it is queued, as the 4 MiB frame cap is sized for directories
///   (ADR-0411), not mail. A mail the kernel would accept stays far under it (a 64 KiB body, at most [`MAX_MAIL_ROWS`] short tokens); the
///   kernel still checks the body against its own limit when the mail runs.
pub(crate) const MAX_MAIL_BYTES: usize = 512 << 10;
/// `ReplyOverdue` reports of one PC handed to the host at once; more wait.
pub(crate) const MAX_OVERDUE_RUNNING: usize = 4;
/// `ReplyOverdue` reports of one PC waiting (each request id once) — past it a report is dropped with a warning. A legitimate burst is
///   one report per overdue request in one sweep of the other PC.
pub(crate) const MAX_OVERDUE_WAITING: usize = 256;
/// The `Mail` request's transport deadline — the registry's request timeout, and the age past which an inbound frame is dropped.
pub(crate) const MAIL_DEADLINE: Duration = Duration::from_secs(120);

/// The daemon's side of mail and the directory, as the peer link calls it. ★Blocking calls★ — the link runs them on the blocking pool.
pub(crate) trait MailInbox: Send + Sync {
    /// Runs one inbound mail from the PC `from` (this PC's name for it: `pc_name`) through this PC's checks and the mail kernel's remote
    ///   entrance. `arrived` = when the frame reached this PC. A row per request row, or a refusal of the whole frame.
    fn mail(
        &self,
        from: PeerId,
        pc_name: &str,
        mail: &Mail,
        arrived: Instant,
    ) -> Result<Vec<VerdictRow>, Refusal>;
    /// The PC `from` reports this PC's request `request_id` overdue there.
    fn reply_overdue(&self, from: PeerId, request_id: &str);
    /// Whether `id` is a mail id another PC may use — checked before an overdue report is handed on. Not blocking.
    fn mail_id_ok(&self, id: &str) -> bool;
    /// This PC's directory now — the agents local name resolution sees, names that may travel only.
    fn directory(&self) -> Vec<DirAgent>;
    /// Why each arriving directory row is dropped (a local agent's id, a name that breaks the remote name rule, not an agent id) — `None`
    ///   keeps it; one answer per row, in order. ★Not blocking★ — called on the link's drain task, so the host answers from what it
    ///   already holds (its local ids as of its last directory read).
    fn row_problems(&self, rows: &[&DirAgent]) -> Vec<Option<&'static str>>;
    /// Changes each time the local ids [`MailInbox::row_problems`] checks against change (at a directory read); bumped only after the new
    ///   ids are in place. Not blocking.
    fn local_ids_version(&self) -> u64;
}

/// A whole frame refused — `code` from the closed set (`REMOTE_REFUSED`, `REMOTE_LIMIT`), `reason` this PC's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Refusal {
    pub(crate) code: &'static str,
    pub(crate) reason: String,
}

/// How one sent mail ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MailEnd {
    Verdict(Vec<VerdictRow>),
    /// Nothing was handed to that PC: no live link, or the link dropped before it was.
    Offline,
    /// [`IN_FLIGHT_CAP`] requests to that PC are already waiting (or its outbound queue is full) — not sent.
    Limit,
    /// The encoded frame would exceed the link's cap — not sent, the link stays up.
    TooLarge,
    /// No answer will come — the deadline passed, the link dropped after the mail was handed over, or the answer is not trusted.
    Unconfirmed,
}

impl MailEnd {
    /// The end of a request. `trusted` = the PC is still trusted and still the run the mail went to — read right before the end is
    ///   handed on (on the blocking thread that applies it): a verdict is matched inside the transport, past the per-frame trust check,
    ///   so it is checked here (a verdict read just before a revocation is not applied).
    // ADR-0390 d4
    pub(crate) fn of(
        result: Result<PeerMsg, RequestError>,
        trusted: impl FnOnce() -> bool,
    ) -> Self {
        match result {
            Ok(PeerMsg::MailVerdict(verdict)) if trusted() => Self::Verdict(verdict.rows),
            Ok(PeerMsg::MailVerdict(_)) => {
                tracing::info!("a mail verdict from a PC no longer trusted (or another run of it) — not applied, the mail stays unconfirmed");
                Self::Unconfirmed
            }
            Ok(other) => {
                tracing::warn!(
                    kind = other.kind(),
                    "a mail request answered by something other than a verdict — unconfirmed"
                );
                Self::Unconfirmed
            }
            Err(RequestError::Disconnected { sent: false, .. }) => Self::Offline,
            Err(RequestError::QueueFull) => Self::Limit,
            Err(
                e @ (RequestError::Disconnected { sent: true, .. } | RequestError::TimedOut { .. }),
            ) => {
                tracing::debug!("a mail request ended without a verdict: {e}");
                Self::Unconfirmed
            }
            Err(e) => {
                tracing::warn!("a mail request failed in the link: {e} — unconfirmed");
                Self::Unconfirmed
            }
        }
    }
}

/// One inbound frame waiting or running.
pub(crate) struct Job<T> {
    /// The queue it was started from — a finish counts only against that queue ([`Inbound::gen`]).
    pub(crate) gen: u64,
    pub(crate) mail: Mail,
    pub(crate) arrived: Instant,
    /// What the runtime needs to answer on the frame's own connection.
    pub(crate) reply: T,
    tokens: Vec<String>,
}

impl<T> Job<T> {
    pub(crate) fn tokens(&self) -> &[String] {
        &self.tokens
    }
}

impl<T> std::fmt::Debug for Job<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Job")
            .field("mail", &self.mail)
            .finish_non_exhaustive()
    }
}

/// One PC's receive queue (module head). `T` = how a job is answered (the runtime's link handle and generation).
pub(crate) struct Inbound<T> {
    /// Drawn per queue — a PC removed and added again gets a new queue, and a job of the old one never finishes against it.
    gen: u64,
    waiting: VecDeque<Job<T>>,
    running: usize,
    /// Tokens of the running frames.
    busy: HashSet<String>,
}

/// What [`Inbound::start`] hands back.
pub(crate) struct Started<T> {
    pub(crate) run: Vec<Job<T>>,
    /// Frames past their deadline — dropped, logged by the caller.
    pub(crate) expired: Vec<Job<T>>,
}

impl<T> Inbound<T> {
    pub(crate) fn new(gen: u64) -> Self {
        Self {
            gen,
            waiting: VecDeque::new(),
            running: 0,
            busy: HashSet::new(),
        }
    }

    pub(crate) fn gen(&self) -> u64 {
        self.gen
    }

    /// Queues a frame; `Err` gives it back when the queue is full (refused whole).
    pub(crate) fn push(&mut self, mail: Mail, arrived: Instant, reply: T) -> Result<(), Job<T>> {
        let tokens: Vec<String> = mail.rows.iter().map(|row| row.to.clone()).collect();
        let job = Job {
            gen: self.gen,
            mail,
            arrived,
            reply,
            tokens,
        };
        if self.waiting.len() + self.running >= MAX_QUEUED {
            return Err(job);
        }
        self.waiting.push_back(job);
        Ok(())
    }

    /// Takes the frames that may run now, in arrival order: one runs when fewer than [`MAX_RUNNING`] run and none of its tokens is held
    ///   by a running frame or by an earlier waiting one. Frames past [`MAIL_DEADLINE`] since their arrival are taken out as expired.
    pub(crate) fn start(&mut self, now: Instant) -> Started<T> {
        let mut run = Vec::new();
        let mut expired = Vec::new();
        let mut held: HashSet<String> = HashSet::new();
        let mut keep = VecDeque::new();
        while let Some(job) = self.waiting.pop_front() {
            if now.saturating_duration_since(job.arrived) > MAIL_DEADLINE {
                expired.push(job);
                continue;
            }
            let blocked = job
                .tokens
                .iter()
                .any(|t| self.busy.contains(t) || held.contains(t));
            if !blocked && self.running < MAX_RUNNING {
                self.running += 1;
                self.busy.extend(job.tokens.iter().cloned());
                run.push(job);
            } else {
                held.extend(job.tokens.iter().cloned());
                keep.push_back(job);
            }
        }
        self.waiting = keep;
        Started { run, expired }
    }

    /// A running frame with these tokens finished.
    pub(crate) fn finish(&mut self, tokens: &[String]) {
        self.running = self.running.saturating_sub(1);
        for token in tokens {
            self.busy.remove(token);
        }
    }

    pub(crate) fn is_idle(&self) -> bool {
        self.waiting.is_empty() && self.running == 0
    }
}

/// The text `mail` carries — the sum of its decoded string fields; what [`MAX_MAIL_BYTES`] bounds.
pub(crate) fn mail_bytes(mail: &Mail) -> usize {
    let request = mail
        .request
        .as_ref()
        .and_then(|r| r.reply_by_raw.as_ref())
        .map_or(0, String::len);
    mail.id.len()
        + mail.from.agent_id.len()
        + mail.from.name.len()
        + mail.body.len()
        + mail.reply_to.as_ref().map_or(0, String::len)
        + request
        + mail.rows.iter().map(|row| row.to.len()).sum::<usize>()
}

/// The verdict that refuses every row of `mail` with `refusal` — at most [`MAX_MAIL_ROWS`] rows (an honest sender sends no more; the
///   sender settles the tags left out unconfirmed), so the answer to a huge frame stays small.
pub(crate) fn refuse_all(mail: &Mail, refusal: &Refusal) -> Vec<VerdictRow> {
    mail.rows
        .iter()
        .take(MAX_MAIL_ROWS)
        .map(|row| VerdictRow {
            tag: row.tag,
            outcome: super::wire::Outcome::Refused,
            recipient_id: None,
            code: Some(refusal.code.to_string()),
            reason: Some(refusal.reason.clone()),
        })
        .collect()
}

#[cfg(test)]
mod tests;
