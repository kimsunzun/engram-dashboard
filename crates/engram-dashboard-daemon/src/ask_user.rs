//! The one "ask the user" seam — every question a human must answer goes through here: trust requests, the trusted-list accept question,
//!   later kinds. ★It knows nothing of what is asked★: the asker writes every word of the question, in each UI language, and applies the
//!   answer; this module only routes and shows.
//!
//! Route, in order: the app while it is attached with a PC list slot open, on screen or not ([`AppRoute`], production = the connection
//!   layer's `peer_state::PeerStateHub`; ADR-0410) → the OS's native approval dialog (platform `approval`, behind [`Dialogs`]) → otherwise the
//!   question waits until the app answers it or the asker withdraws it (expiry). The app does not answer through this module: it answers
//!   through the asker's commands, and the asker then [`AskUser::withdraw`]s the question.
//!
//! Rules (who guarantees what):
//! - One dialog at a time, on this module's own thread — never on the daemon's async tasks (the platform call blocks until the user ends
//!   it). The asker's answer callback runs on that thread, outside this module's lock — never after [`AskUser::stop`] began.
//! - While the app takes the questions no new dialog opens, however long a question waits there (ADR-0410 — one buried behind other
//!   windows is the user's to find); a dialog already open stays — whichever answer comes first wins, because the asker consumes its
//!   question atomically. When the app stops taking them (the app calls [`AskUser::route_changed`], which goes through this module's
//!   lock so the wake-up cannot fall between the dialog thread's check and its wait), open questions go to the dialog.
//! - At most one new dialog per question kind per [`DIALOG_RATE`]; the rest queue. A dialog shows the oldest question it may show and says
//!   how many others wait; its answer applies to the question shown only.
//! - A dialog whose question is withdrawn (answered in the app, expired, no longer asked) is dismissed; where the platform cannot close
//!   it, it is left stale and its answer is ignored.
//! - No desktop (session 0, a non-interactive window station): no dialog is tried again this run; questions wait. A dialog that cannot
//!   be shown is retried after [`DIALOG_RATE`]; the first failure of a run is a warning, the ones after it until a dialog shows are not.
//! - Closing the dialog without choosing counts as its default choice, deny.
//! - The language is read when a dialog is about to show ([`LanguageSource`] — the user's `ui.language` as the shell last told the
//!   daemon; English until it does), so a change applies to the next dialog.

// ADR-0394, ADR-0372, ADR-0410

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use engram_dashboard_base::sync::lock;
use engram_dashboard_base::time::Clock;
use engram_dashboard_platform::approval::{
    ask_approval, ApprovalAnswer, ApprovalQuestion, DialogCloser,
};

/// At most one new dialog per question kind this often.
pub(crate) const DIALOG_RATE: Duration = Duration::from_secs(60);
/// [`AskUser::stop`] waits at most this for the dialog thread to end (it may be applying an answer the user gave just before).
const STOP_WAIT: Duration = Duration::from_secs(3);

/// The asker's id for a question — unique among the questions it has open.
pub(crate) type QuestionId = u64;

/// A UI language of the daemon's own text — the values of the shell's `ui.language` setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Lang {
    #[default]
    En,
    Ko,
}

impl Lang {
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ko => "ko",
        }
    }

    pub(crate) fn from_code(code: &str) -> Option<Lang> {
        [Lang::En, Lang::Ko]
            .into_iter()
            .find(|lang| lang.code() == code.trim())
    }
}

/// The language the user reads now. ★Called on the dialog thread, outside this module's lock★.
pub(crate) trait LanguageSource: Send + Sync {
    fn language(&self) -> Lang;
}

/// One language's text of a question. Agent-supplied strings must already be quoted and made visible (the asker's job).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QuestionText {
    pub(crate) title: String,
    pub(crate) heading: String,
    pub(crate) body: String,
    /// The three choice labels; `&` marks an access key in the dialog (write `&&` for a literal `&`).
    pub(crate) approve: String,
    pub(crate) deny: String,
    pub(crate) deny_and_stop: String,
}

/// One question, every word written by the asker in each UI language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Question {
    /// The rate-limit key — one new dialog per kind per [`DIALOG_RATE`].
    pub(crate) kind: &'static str,
    pub(crate) en: QuestionText,
    pub(crate) ko: QuestionText,
}

impl Question {
    pub(crate) fn text(&self, lang: Lang) -> &QuestionText {
        match lang {
            Lang::En => &self.en,
            Lang::Ko => &self.ko,
        }
    }
}

/// A human's answer to a question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Answer {
    Approve,
    Deny,
    /// The third choice — its meaning is the asker's (stop asking this agent; for some questions another action).
    DenyAndStop,
}

/// The app's route: whether the app takes these questions right now (attached with a PC list slot open — ADR-0410). ★Called under this
///   module's lock — answer from a value at hand, never block or call back into the asker★.
pub(crate) trait AppRoute: Send + Sync {
    fn takes_questions(&self) -> bool;
}

/// No app route — every question goes to the dialog (or waits where there is no desktop). Tests only; production plugs in the
///   connection layer's peer-state hub (`peer_state`).
#[cfg(test)]
pub(crate) struct NoAppRoute;

#[cfg(test)]
impl AppRoute for NoAppRoute {
    fn takes_questions(&self) -> bool {
        false
    }
}

/// The OS dialog — production = [`OsDialogs`]; tests answer through a fake.
pub(crate) trait Dialogs: Send + Sync {
    /// Shows `question` and blocks until it ends; `closer` ends it from another thread.
    fn ask(&self, question: &ApprovalQuestion<'_>, closer: &Closer) -> io::Result<ApprovalAnswer>;
}

/// The platform's native approval dialog.
pub(crate) struct OsDialogs;

impl Dialogs for OsDialogs {
    fn ask(&self, question: &ApprovalQuestion<'_>, closer: &Closer) -> io::Result<ApprovalAnswer> {
        ask_approval(question, &closer.platform)
    }
}

/// Ends one dialog from another thread.
#[derive(Debug, Default)]
pub(crate) struct Closer {
    platform: DialogCloser,
    dismissed: AtomicBool,
}

impl Closer {
    fn dismiss(&self) -> io::Result<()> {
        self.dismissed.store(true, Ordering::Release);
        self.platform.dismiss()
    }

    /// Whether the question was withdrawn while its dialog was asked for — a fake dialog's view of [`DialogCloser::dismiss`] (the
    ///   platform dialog reads its own closer).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn is_dismissed(&self) -> bool {
        self.dismissed.load(Ordering::Acquire)
    }
}

/// The asker's callback for a human's answer from the dialog — runs on this module's thread, outside its lock.
pub(crate) type OnAnswer = Box<dyn Fn(QuestionId, Answer) + Send + Sync>;

/// The seam (module head). Cheap to clone.
#[derive(Clone)]
pub(crate) struct AskUser {
    shared: Arc<Shared>,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    /// Signalled when the dialog thread has ended.
    ended: Condvar,
    dialogs: Arc<dyn Dialogs>,
    app: Arc<dyn AppRoute>,
    language: Arc<dyn LanguageSource>,
    clock: Arc<dyn Clock>,
    on_answer: OnAnswer,
}

#[derive(Default)]
struct State {
    /// Open questions, oldest first (ids grow).
    open: BTreeMap<QuestionId, Question>,
    /// The question on screen and its closer.
    shown: Option<(QuestionId, Arc<Closer>)>,
    /// When a new dialog of each kind was last shown.
    last_dialog: HashMap<&'static str, Instant>,
    no_desktop: bool,
    /// A dialog failed to show and no dialog has shown since — later failures are not warned again.
    failing: bool,
    stopped: bool,
    /// The dialog thread is not running (ended, or never started).
    ended: bool,
}

enum Next {
    Show {
        id: QuestionId,
        question: Question,
        waiting: usize,
        closer: Arc<Closer>,
    },
    WaitFor(Duration),
    Idle,
}

impl AskUser {
    /// Starts the dialog thread. If the thread cannot start, questions wait for the app (logged).
    pub(crate) fn start(
        dialogs: Arc<dyn Dialogs>,
        app: Arc<dyn AppRoute>,
        language: Arc<dyn LanguageSource>,
        clock: Arc<dyn Clock>,
        on_answer: OnAnswer,
    ) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            ended: Condvar::new(),
            dialogs,
            app,
            language,
            clock,
            on_answer,
        });
        let worker = shared.clone();
        if let Err(e) = std::thread::Builder::new()
            .name("ask-user".to_string())
            .spawn(move || worker.run())
        {
            tracing::error!(
                "the approval dialog thread could not start — questions wait for the app: {e}"
            );
            lock(&shared.state).ended = true;
        }
        Self { shared }
    }

    /// Opens a question. Re-asking an open id replaces its text (a dialog already showing it keeps the old text).
    pub(crate) fn ask(&self, id: QuestionId, question: Question) {
        let mut state = lock(&self.shared.state);
        state.open.insert(id, question);
        drop(state);
        self.shared.wake.notify_all();
    }

    /// Closes a question that needs no answer any more — answered in the app, expired, or no longer asked. Its dialog, if open, is
    ///   dismissed.
    pub(crate) fn withdraw(&self, id: QuestionId) {
        let mut state = lock(&self.shared.state);
        state.open.remove(&id);
        if let Some((shown, closer)) = &state.shown {
            if *shown == id {
                if let Err(e) = closer.dismiss() {
                    tracing::warn!(
                        question = id,
                        "an approval dialog whose question was answered elsewhere could not be closed — it stays open as stale and its answer is ignored: {e}"
                    );
                }
            }
        }
        drop(state);
        self.shared.wake.notify_all();
    }

    /// The app started or stopped taking the questions — routes the open questions again (to the dialog when it stopped). Recorded
    ///   under the lock, so the dialog thread cannot read the old route and then miss this wake-up.
    pub(crate) fn route_changed(&self) {
        let state = lock(&self.shared.state);
        self.shared.wake.notify_all();
        drop(state);
    }

    /// Ends the dialog thread: an open dialog is dismissed, no answer is handed on from now, and this waits up to [`STOP_WAIT`] for the
    ///   thread (it may be applying an answer the user gave just before). Not to be called from the answer callback.
    pub(crate) fn stop(&self) {
        self.stop_now();
        let state = lock(&self.shared.state);
        let (state, waited) = self
            .shared
            .ended
            .wait_timeout_while(state, STOP_WAIT, |state| !state.ended)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if waited.timed_out() && !state.ended {
            tracing::warn!(
                wait_ms = STOP_WAIT.as_millis() as u64,
                "the approval dialog thread did not end in time — left behind"
            );
        }
    }

    /// [`AskUser::stop`] without the wait — for a drop that may run on the dialog thread itself.
    pub(crate) fn stop_now(&self) {
        let mut state = lock(&self.shared.state);
        state.stopped = true;
        if let Some((_, closer)) = &state.shown {
            let _ = closer.dismiss();
        }
        drop(state);
        self.shared.wake.notify_all();
    }

    /// Whether a dialog failed to show and none has shown since (tests).
    #[cfg(test)]
    pub(crate) fn is_failing(&self) -> bool {
        lock(&self.shared.state).failing
    }

    /// Whether a stop has begun (tests).
    #[cfg(test)]
    pub(crate) fn is_stopping(&self) -> bool {
        lock(&self.shared.state).stopped
    }

    /// The question a dialog shows now (tests).
    #[cfg(test)]
    pub(crate) fn showing(&self) -> Option<QuestionId> {
        lock(&self.shared.state).shown.as_ref().map(|(id, _)| *id)
    }
}

impl Shared {
    fn run(self: Arc<Self>) {
        self.serve();
        lock(&self.state).ended = true;
        self.ended.notify_all();
    }

    fn serve(&self) {
        let mut state = lock(&self.state);
        loop {
            if state.stopped {
                return;
            }
            match self.next(&mut state) {
                Next::Show {
                    id,
                    question,
                    waiting,
                    closer,
                } => {
                    drop(state);
                    let lang = self.language.language();
                    let text = question.text(lang);
                    let body = with_waiting(&text.body, waiting, lang);
                    let shown = ApprovalQuestion {
                        title: &text.title,
                        heading: &text.heading,
                        body: &body,
                        approve: &text.approve,
                        deny: &text.deny,
                        deny_and_stop: &text.deny_and_stop,
                    };
                    let result = self.dialogs.ask(&shown, &closer);
                    state = lock(&self.state);
                    state.shown = None;
                    if state.stopped {
                        // A click that landed just before the stop is not handed on — the daemon is shutting down.
                        return;
                    }
                    if result.is_ok() {
                        state.failing = false;
                    }
                    let answer = match result {
                        Ok(ApprovalAnswer::Approve) => Some(Answer::Approve),
                        // ADR-0394 d6: deny is the default — closing without a choice is a deny.
                        Ok(ApprovalAnswer::Deny | ApprovalAnswer::Closed) => Some(Answer::Deny),
                        Ok(ApprovalAnswer::DenyAndStop) => Some(Answer::DenyAndStop),
                        Ok(ApprovalAnswer::Dismissed) => None,
                        Ok(ApprovalAnswer::NoDesktop) => {
                            tracing::info!("no desktop to show an approval dialog on — questions wait for the app this run");
                            state.no_desktop = true;
                            None
                        }
                        Err(e) => {
                            // ADR-0346 d5: a failure that repeats is not logged again until a dialog shows.
                            if state.failing {
                                tracing::debug!(
                                    question = id,
                                    "the approval dialog still cannot be shown: {e}"
                                );
                            } else {
                                tracing::warn!(
                                    question = id,
                                    retry_in_secs = DIALOG_RATE.as_secs(),
                                    "the approval dialog could not be shown — the question waits (retried; repeats are not logged until a dialog shows): {e}"
                                );
                                state.failing = true;
                            }
                            None
                        }
                    };
                    // A withdrawn question's late click is ignored — the asker consumed it elsewhere.
                    if let Some(answer) = answer.filter(|_| state.open.remove(&id).is_some()) {
                        drop(state);
                        (self.on_answer)(id, answer);
                        state = lock(&self.state);
                    }
                }
                Next::WaitFor(wait) => state = wait_timeout(&self.wake, state, wait),
                Next::Idle => state = wait(&self.wake, state),
            }
        }
    }

    /// What the dialog thread does next — decided under the lock (the app route is read here too).
    fn next(&self, state: &mut State) -> Next {
        if state.shown.is_some() || state.no_desktop || state.open.is_empty() {
            return Next::Idle;
        }
        // `route_changed` wakes this thread on every change of the app's answer.
        if self.app.takes_questions() {
            return Next::Idle;
        }
        let now = self.clock.now();
        let mut earliest: Option<Instant> = None;
        let mut later = |at: Instant| earliest = Some(earliest.map_or(at, |e: Instant| e.min(at)));
        let pick =
            state
                .open
                .iter()
                .find(|(_, question)| match state.last_dialog.get(question.kind) {
                    Some(last) if *last + DIALOG_RATE > now => {
                        later(*last + DIALOG_RATE);
                        false
                    }
                    _ => true,
                });
        let Some((&id, question)) = pick else {
            return match earliest {
                Some(at) => Next::WaitFor(at.saturating_duration_since(now)),
                None => Next::Idle,
            };
        };
        let question = question.clone();
        let waiting = state.open.len() - 1;
        let closer = Arc::new(Closer::default());
        state.last_dialog.insert(question.kind, now);
        state.shown = Some((id, closer.clone()));
        Next::Show {
            id,
            question,
            waiting,
            closer,
        }
    }
}

/// The body with the count of other open questions — they are shown one at a time.
fn with_waiting(body: &str, waiting: usize, lang: Lang) -> String {
    let line = match (lang, waiting) {
        (_, 0) => return body.to_string(),
        (Lang::En, 1) => {
            "1 more request is waiting — requests are shown one at a time.".to_string()
        }
        (Lang::En, n) => {
            format!("{n} more requests are waiting — requests are shown one at a time.")
        }
        (Lang::Ko, n) => format!("요청 {n}건이 더 기다립니다 — 요청은 하나씩 보여 줍니다."), // hangul-ok
    };
    format!("{body}\n\n{line}")
}

fn wait<'a>(wake: &Condvar, state: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
    // The state is plain fields written in single steps; a panic elsewhere cannot leave it half-written.
    wake.wait(state)
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn wait_timeout<'a>(
    wake: &Condvar,
    state: MutexGuard<'a, State>,
    timeout: Duration,
) -> MutexGuard<'a, State> {
    wake.wait_timeout(state, timeout)
        .map(|(state, _)| state)
        .unwrap_or_else(|poisoned| poisoned.into_inner().0)
}

#[cfg(test)]
pub(crate) mod fakes;
#[cfg(test)]
mod tests;
