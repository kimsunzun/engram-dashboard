//! Test doubles for the "ask the user" seam: a dialog the test answers (or that finds no desktop) and an app route the test turns on
//!   or off. Waits are "a message or a cap"; nothing passes by sleeping.

use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use engram_dashboard_platform::approval::{ApprovalAnswer, ApprovalQuestion};

use super::{AppRoute, Closer, Dialogs, Lang, LanguageSource};

/// How long a positive wait may take before the test fails.
pub(crate) const PATIENCE: Duration = Duration::from_secs(10);
/// How long a negative check watches for something that must not happen.
pub(crate) const QUIET: Duration = Duration::from_millis(300);

/// One dialog on the fake screen.
pub(crate) struct Shown {
    pub(crate) title: String,
    pub(crate) heading: String,
    pub(crate) body: String,
    pub(crate) labels: [String; 3],
    answer: Sender<ApprovalAnswer>,
}

impl Shown {
    /// Clicks / closes it. A dialog already dismissed has returned; the click is then lost, as on screen.
    pub(crate) fn answer(&self, answer: ApprovalAnswer) {
        let _ = self.answer.send(answer);
    }
}

/// The OS dialog, answered by the test.
pub(crate) struct FakeDialogs {
    shown: Mutex<Sender<Shown>>,
    no_desktop: AtomicBool,
    /// Keep waiting for a click after a dismissal — a dialog the platform could not close.
    ignores_dismiss: AtomicBool,
    asked: AtomicUsize,
    dismissed: AtomicUsize,
    /// The next this many dialogs fail to show.
    failures: AtomicUsize,
}

impl FakeDialogs {
    pub(crate) fn new() -> (Arc<Self>, Receiver<Shown>) {
        let (tx, rx) = mpsc::channel();
        (
            Arc::new(Self {
                shown: Mutex::new(tx),
                no_desktop: AtomicBool::new(false),
                ignores_dismiss: AtomicBool::new(false),
                asked: AtomicUsize::new(0),
                dismissed: AtomicUsize::new(0),
                failures: AtomicUsize::new(0),
            }),
            rx,
        )
    }

    pub(crate) fn without_desktop(&self) {
        self.no_desktop.store(true, Ordering::SeqCst);
    }

    /// The next `n` dialogs fail to show.
    pub(crate) fn failing(&self, n: usize) {
        self.failures.store(n, Ordering::SeqCst);
    }

    pub(crate) fn ignoring_dismiss(&self) {
        self.ignores_dismiss.store(true, Ordering::SeqCst);
    }

    /// Dialogs asked for so far (shown or not).
    pub(crate) fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }

    /// Dialogs that ended dismissed.
    pub(crate) fn dismissed(&self) -> usize {
        self.dismissed.load(Ordering::SeqCst)
    }
}

impl Dialogs for FakeDialogs {
    fn ask(&self, question: &ApprovalQuestion<'_>, closer: &Closer) -> io::Result<ApprovalAnswer> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        if self.no_desktop.load(Ordering::SeqCst) {
            return Ok(ApprovalAnswer::NoDesktop);
        }
        if self
            .failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(io::Error::other("the dialog could not be created"));
        }
        let (tx, rx) = mpsc::channel();
        let shown = Shown {
            title: question.title.to_string(),
            heading: question.heading.to_string(),
            body: question.body.to_string(),
            labels: [
                question.approve.to_string(),
                question.deny.to_string(),
                question.deny_and_stop.to_string(),
            ],
            answer: tx,
        };
        if self.shown.lock().unwrap().send(shown).is_err() {
            return Ok(ApprovalAnswer::Closed);
        }
        loop {
            if closer.is_dismissed() && !self.ignores_dismiss.load(Ordering::SeqCst) {
                self.dismissed.fetch_add(1, Ordering::SeqCst);
                return Ok(ApprovalAnswer::Dismissed);
            }
            match rx.recv_timeout(Duration::from_millis(5)) {
                Ok(answer) => return Ok(answer),
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    // The test dropped the dialog without answering: wait for a dismissal (or the end).
                    if closer.is_dismissed() {
                        self.dismissed.fetch_add(1, Ordering::SeqCst);
                        return Ok(ApprovalAnswer::Dismissed);
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }
}

/// The app's route — a PC list slot open or not, as the test sets it.
#[derive(Default)]
pub(crate) struct FakeApp {
    present: AtomicBool,
}

impl FakeApp {
    pub(crate) fn set_present(&self, present: bool) {
        self.present.store(present, Ordering::SeqCst);
    }
}

impl AppRoute for FakeApp {
    fn takes_questions(&self) -> bool {
        self.present.load(Ordering::SeqCst)
    }
}

/// The next dialog, or the test fails.
pub(crate) fn next_dialog(rx: &Receiver<Shown>) -> Shown {
    rx.recv_timeout(PATIENCE)
        .expect("a dialog should have been shown")
}

/// No dialog within [`QUIET`].
pub(crate) fn no_dialog(rx: &Receiver<Shown>) {
    if let Ok(shown) = rx.recv_timeout(QUIET) {
        panic!("no dialog should open, but one did: {:?}", shown.heading);
    }
}

/// The user's language, set by the test.
#[derive(Default)]
pub(crate) struct FakeLanguage {
    korean: AtomicBool,
}

impl FakeLanguage {
    pub(crate) fn set(&self, lang: Lang) {
        self.korean.store(lang == Lang::Ko, Ordering::SeqCst);
    }
}

impl LanguageSource for FakeLanguage {
    fn language(&self) -> Lang {
        if self.korean.load(Ordering::SeqCst) {
            Lang::Ko
        } else {
            Lang::En
        }
    }
}
