//! The seam's routing with a fake dialog and a fake app route: what goes to the dialog and when, what comes back, and what is withdrawn.

use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use engram_dashboard_base::time::{ManualClock, SystemClock};
use engram_dashboard_platform::approval::ApprovalAnswer;

use super::fakes::{
    next_dialog, no_dialog, FakeApp, FakeDialogs, FakeLanguage, Shown, PATIENCE, QUIET,
};
use super::*;

fn text(heading: &str, lang: &str) -> QuestionText {
    QuestionText {
        title: "Engram — test".to_string(),
        heading: heading.to_string(),
        body: format!("body of {heading} ({lang})"),
        approve: format!("Approve ({lang})"),
        deny: "Deny".to_string(),
        deny_and_stop: "Deny and stop asking this agent".to_string(),
    }
}

fn question(kind: &'static str, heading: &str) -> Question {
    Question {
        kind,
        en: text(heading, "en"),
        ko: text(heading, "ko"),
    }
}

struct Rig {
    ask: AskUser,
    dialogs: Arc<FakeDialogs>,
    shown: Receiver<Shown>,
    answers: Receiver<(QuestionId, Answer)>,
    app: Arc<FakeApp>,
    language: Arc<FakeLanguage>,
}

fn rig_with(clock: Arc<dyn Clock>) -> Rig {
    let (dialogs, shown) = FakeDialogs::new();
    let app = Arc::new(FakeApp::default());
    let language = Arc::new(FakeLanguage::default());
    let (tx, answers) = mpsc::channel();
    let tx = Mutex::new(tx);
    let ask = AskUser::start(
        dialogs.clone(),
        app.clone(),
        language.clone(),
        clock,
        Box::new(move |id, answer| {
            let _ = tx.lock().unwrap().send((id, answer));
        }),
    );
    Rig {
        ask,
        dialogs,
        shown,
        answers,
        app,
        language,
    }
}

fn rig() -> Rig {
    rig_with(Arc::new(SystemClock))
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.ask.stop();
    }
}

fn next_answer(rig: &Rig) -> (QuestionId, Answer) {
    rig.answers
        .recv_timeout(PATIENCE)
        .expect("an answer should have come back")
}

fn no_answer(rig: &Rig) {
    if let Ok(answer) = rig.answers.recv_timeout(QUIET) {
        panic!("no answer should come back, but {answer:?} did");
    }
}

#[test]
fn without_the_app_a_question_goes_to_the_dialog_and_each_choice_comes_back() {
    let rig = rig();
    for (id, click, wanted) in [
        (1, ApprovalAnswer::Approve, Answer::Approve),
        (2, ApprovalAnswer::Deny, Answer::Deny),
        (3, ApprovalAnswer::DenyAndStop, Answer::DenyAndStop),
    ] {
        // One kind per question, so the per-kind dialog rate does not hold the next one back.
        let kind: &'static str = ["a", "b", "c"][id as usize - 1];
        rig.ask.ask(id, question(kind, &format!("q{id}")));
        let shown = next_dialog(&rig.shown);
        assert_eq!(shown.heading, format!("q{id}"));
        assert_eq!(shown.title, "Engram — test");
        assert_eq!(
            shown.labels,
            [
                "Approve (en)".to_string(),
                "Deny".to_string(),
                "Deny and stop asking this agent".to_string()
            ]
        );
        shown.answer(click);
        assert_eq!(next_answer(&rig), (id, wanted));
    }
}

/// ADR-0394 d6: deny is the default — closing the dialog without a choice is a deny.
#[test]
fn closing_the_dialog_without_a_choice_is_a_deny() {
    let rig = rig();
    rig.ask.ask(1, question("a", "q1"));
    next_dialog(&rig.shown).answer(ApprovalAnswer::Closed);
    assert_eq!(next_answer(&rig), (1, Answer::Deny));
}

#[test]
fn while_the_app_shows_the_slot_no_dialog_opens_and_hiding_it_reroutes_to_the_dialog() {
    let rig = rig();
    rig.app.set_present(true);
    rig.ask.ask(1, question("a", "q1"));
    no_dialog(&rig.shown);

    rig.app.set_present(false);
    rig.ask.route_changed();
    let shown = next_dialog(&rig.shown);
    assert_eq!(shown.heading, "q1", "the open question moved to the dialog");
    shown.answer(ApprovalAnswer::Approve);
    assert_eq!(next_answer(&rig), (1, Answer::Approve));
}

/// Answered in the app (the asker withdraws it) while the dialog is open → the dialog is dismissed and nothing comes back from it.
#[test]
fn a_withdrawn_question_dismisses_its_dialog_and_gives_no_answer() {
    let rig = rig();
    rig.ask.ask(1, question("a", "q1"));
    let _shown = next_dialog(&rig.shown);
    rig.ask.withdraw(1);
    no_answer(&rig);
    assert_eq!(rig.dialogs.dismissed(), 1, "the open dialog was dismissed");
    assert_eq!(rig.ask.showing(), None);
}

/// A dialog the platform could not close stays as stale; a click on it after the question was withdrawn is ignored.
#[test]
fn a_click_on_a_stale_dialog_is_ignored() {
    let rig = rig();
    rig.dialogs.ignoring_dismiss();
    rig.ask.ask(1, question("a", "q1"));
    let shown = next_dialog(&rig.shown);
    rig.ask.withdraw(1);
    shown.answer(ApprovalAnswer::Approve);
    no_answer(&rig);
}

/// One dialog at a time; it says how many others wait, and its answer applies to the question shown only.
#[test]
fn one_dialog_at_a_time_counts_the_others_and_answers_only_the_one_shown() {
    let rig = rig();
    rig.ask.ask(1, question("a", "q1"));
    let first = next_dialog(&rig.shown);
    rig.ask.ask(2, question("b", "q2"));
    rig.ask.ask(3, question("c", "q3"));
    no_dialog(&rig.shown);

    first.answer(ApprovalAnswer::Approve);
    assert_eq!(next_answer(&rig), (1, Answer::Approve));
    let second = next_dialog(&rig.shown);
    assert_eq!(second.heading, "q2", "oldest first");
    assert!(
        second
            .body
            .ends_with("1 more request is waiting — requests are shown one at a time."),
        "{}",
        second.body
    );
    second.answer(ApprovalAnswer::Deny);
    assert_eq!(next_answer(&rig), (2, Answer::Deny));
    let third = next_dialog(&rig.shown);
    assert_eq!(third.body, "body of q3 (en)", "nothing else waits");
    third.answer(ApprovalAnswer::Deny);
    assert_eq!(next_answer(&rig), (3, Answer::Deny));
}

#[test]
fn the_waiting_count_reads_in_the_plural() {
    let rig = rig();
    rig.app.set_present(true);
    for id in 1..=3 {
        rig.ask.ask(id, question("a", &format!("q{id}")));
    }
    rig.app.set_present(false);
    rig.ask.route_changed();
    let shown = next_dialog(&rig.shown);
    assert!(
        shown
            .body
            .ends_with("2 more requests are waiting — requests are shown one at a time."),
        "{}",
        shown.body
    );
}

/// At most one new dialog per kind per minute; another kind is not held back.
#[test]
fn one_new_dialog_per_kind_per_minute() {
    let clock = Arc::new(ManualClock::new());
    let rig = rig_with(clock.clone());
    rig.ask.ask(1, question("add", "q1"));
    next_dialog(&rig.shown).answer(ApprovalAnswer::Deny);
    assert_eq!(next_answer(&rig).0, 1);

    rig.ask.ask(2, question("add", "q2"));
    no_dialog(&rig.shown);
    rig.ask.ask(3, question("setNetwork", "q3"));
    let other_kind = next_dialog(&rig.shown);
    assert_eq!(
        other_kind.heading, "q3",
        "another kind is not rate-limited by the first"
    );
    other_kind.answer(ApprovalAnswer::Deny);
    assert_eq!(next_answer(&rig).0, 3);
    no_dialog(&rig.shown);

    clock.advance(DIALOG_RATE);
    rig.ask.route_changed();
    assert_eq!(
        next_dialog(&rig.shown).heading,
        "q2",
        "a minute later the queued one shows"
    );
}

/// No desktop: nothing is tried again this run; questions wait (for the app or their expiry).
#[test]
fn without_a_desktop_questions_wait_and_no_dialog_is_tried_again() {
    let rig = rig();
    rig.dialogs.without_desktop();
    rig.ask.ask(1, question("a", "q1"));
    let deadline = std::time::Instant::now() + PATIENCE;
    while rig.dialogs.asked() == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "the dialog was never tried"
        );
        std::thread::yield_now();
    }
    rig.ask.ask(2, question("b", "q2"));
    rig.ask.route_changed();
    no_answer(&rig);
    assert_eq!(rig.dialogs.asked(), 1, "no second try after no desktop");
}

#[test]
fn stopping_dismisses_the_open_dialog() {
    let rig = rig();
    rig.ask.ask(1, question("a", "q1"));
    let _shown = next_dialog(&rig.shown);
    rig.ask.stop();
    let deadline = std::time::Instant::now() + PATIENCE;
    while rig.dialogs.dismissed() == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "the dialog stayed open"
        );
        std::thread::yield_now();
    }
    no_answer(&rig);
}

/// ADR-0372: the dialog speaks the language the shell last set — read when the dialog shows, so a change reaches the next one; the
///   waiting count follows it too.
#[test]
fn a_dialog_speaks_the_language_the_shell_set() {
    let rig = rig();
    rig.language.set(Lang::Ko);
    rig.app.set_present(true);
    rig.ask.ask(1, question("a", "q1"));
    rig.ask.ask(2, question("b", "q2"));
    rig.app.set_present(false);
    rig.ask.route_changed();
    let shown = next_dialog(&rig.shown);
    assert!(shown.body.starts_with("body of q1 (ko)"), "{}", shown.body);
    assert_eq!(shown.labels[0], "Approve (ko)");
    assert!(shown.body.ends_with("\u{c694}\u{ccad} 1\u{ac74}\u{c774} \u{b354} \u{ae30}\u{b2e4}\u{b9bd}\u{b2c8}\u{b2e4} \u{2014} \u{c694}\u{ccad}\u{c740} \u{d558}\u{b098}\u{c529} \u{bcf4}\u{c5ec} \u{c90d}\u{b2c8}\u{b2e4}."), "{}", shown.body);
    rig.language.set(Lang::En);
    shown.answer(ApprovalAnswer::Deny);
    assert_eq!(next_answer(&rig).0, 1);

    let next = next_dialog(&rig.shown);
    assert_eq!(next.body, "body of q2 (en)");
}

/// The route change goes through the lock: however the hide and the dialog thread's check interleave, the hidden slot's questions
///   reach the dialog (a lost wake-up would leave them waiting).
#[test]
fn hiding_the_slot_always_reaches_the_dialog() {
    let clock = Arc::new(ManualClock::new());
    let rig = rig_with(clock.clone());
    for round in 0..50u64 {
        // Each round's dialog is a new one of the same kind — let the rate limit pass.
        clock.advance(DIALOG_RATE);
        rig.app.set_present(true);
        rig.ask.ask(round, question("a", &format!("q{round}")));
        rig.ask.route_changed();
        rig.app.set_present(false);
        rig.ask.route_changed();
        let shown = next_dialog(&rig.shown);
        assert_eq!(shown.heading, format!("q{round}"));
        rig.ask.withdraw(round);
        let deadline = std::time::Instant::now() + PATIENCE;
        while rig.ask.showing().is_some() {
            assert!(std::time::Instant::now() < deadline, "the dialog stayed");
            std::thread::yield_now();
        }
    }
}

/// A click that lands just before the stop is not handed on — nothing is applied while the daemon shuts down.
#[test]
fn an_answer_after_stop_is_not_handed_on() {
    let rig = rig();
    rig.dialogs.ignoring_dismiss();
    rig.ask.ask(1, question("a", "q1"));
    let shown = next_dialog(&rig.shown);
    let ask = rig.ask.clone();
    let stopping = std::thread::spawn(move || ask.stop());
    // The stop is under way (it set the flag before it waits), then the user clicks.
    let deadline = std::time::Instant::now() + PATIENCE;
    while !rig.ask.is_stopping() {
        assert!(std::time::Instant::now() < deadline, "stop never began");
        std::thread::yield_now();
    }
    shown.answer(ApprovalAnswer::Approve);
    stopping.join().unwrap();
    no_answer(&rig);
}

/// ADR-0346 d5: a dialog that keeps failing to show is retried each minute, warned about once, and the flag clears when one shows.
#[test]
fn a_dialog_that_keeps_failing_is_retried_and_warned_once() {
    let clock = Arc::new(ManualClock::new());
    let rig = rig_with(clock.clone());
    rig.dialogs.failing(2);
    rig.ask.ask(1, question("a", "q1"));
    let deadline = std::time::Instant::now() + PATIENCE;
    while rig.dialogs.asked() < 1 || !rig.ask.is_failing() {
        assert!(std::time::Instant::now() < deadline, "never tried");
        std::thread::yield_now();
    }
    no_dialog(&rig.shown);
    clock.advance(DIALOG_RATE);
    rig.ask.route_changed();
    while rig.dialogs.asked() < 2 {
        assert!(std::time::Instant::now() < deadline, "not retried");
        std::thread::yield_now();
    }
    assert!(rig.ask.is_failing(), "still failing — no second warning");
    clock.advance(DIALOG_RATE);
    rig.ask.route_changed();
    let shown = next_dialog(&rig.shown);
    shown.answer(ApprovalAnswer::Deny);
    assert_eq!(next_answer(&rig), (1, Answer::Deny));
    assert!(
        !rig.ask.is_failing(),
        "a dialog showed — the next failure warns again"
    );
}

/// ADR-0410: no backstop — while the app takes the questions, one nobody answers there still never opens a dialog; closing the
///   last slot sends it to the dialog.
#[test]
fn a_question_left_unanswered_in_the_app_waits_there() {
    let clock = Arc::new(ManualClock::new());
    let rig = rig_with(clock.clone());
    rig.app.set_present(true);
    rig.ask.ask(1, question("a", "q1"));
    no_dialog(&rig.shown);

    clock.advance(DIALOG_RATE * 10);
    rig.ask.route_changed();
    no_dialog(&rig.shown);

    rig.app.set_present(false);
    rig.ask.route_changed();
    assert_eq!(next_dialog(&rig.shown).heading, "q1");
}
