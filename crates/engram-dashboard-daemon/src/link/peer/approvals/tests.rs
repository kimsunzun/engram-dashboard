//! Pending requests and the questions the peer state raises — over fake trust operations, a fake dialog and a manual clock.

use std::sync::mpsc::Receiver;

use engram_dashboard_base::time::ManualClock;
use engram_dashboard_platform::approval::ApprovalAnswer;

use super::*;
use crate::ask_user::fakes::{
    next_dialog, no_dialog, FakeApp, FakeDialogs, FakeLanguage, Shown, PATIENCE,
};
use crate::ask_user::NoAppRoute;
use crate::link::peer::testing::{pc, source, FakeOps};
use crate::link::peer::trusted::{Helper, Permission};

struct Rig {
    approvals: Arc<Approvals>,
    ops: Arc<FakeOps>,
    dialogs: Arc<FakeDialogs>,
    shown: Receiver<Shown>,
    clock: Arc<ManualClock>,
}

fn rig_with_app(app: Arc<dyn AppRoute>) -> Rig {
    let ops = Arc::new(FakeOps::default());
    let (dialogs, shown) = FakeDialogs::new();
    let clock = Arc::new(ManualClock::new());
    let approvals = Approvals::start(
        source(ops.clone()),
        dialogs.clone(),
        app,
        Arc::new(FakeLanguage::default()),
        clock.clone(),
    );
    Rig {
        approvals,
        ops,
        dialogs,
        shown,
        clock,
    }
}

fn rig() -> Rig {
    rig_with_app(Arc::new(NoAppRoute))
}

fn agent(id: &str) -> Requester {
    Requester {
        id: id.to_string(),
        name: Some("lead".to_string()),
    }
}

fn add(byte: u8, name: &str) -> Change {
    Change::Add(TrustedPeer {
        id: pc(byte),
        name: name.to_string(),
        address: None,
        permission: Permission::Mail,
    })
}

/// Waits until `check` holds (the dialog thread applies answers).
fn eventually(what: &str, check: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + PATIENCE;
    while !check() {
        assert!(std::time::Instant::now() < deadline, "{what}");
        std::thread::yield_now();
    }
}

/// ADR-0394 d4 · d6: an agent's request waits, the dialog says who asks and what it allows — the agent quoted with its id prefix, the
///   other PC's full ID grouped, no short fingerprint — and the dialog's approve applies it.
#[test]
fn an_agent_request_is_asked_in_the_dialog_and_its_approval_applies_it() {
    let rig = rig();
    let requested = rig
        .approvals
        .request(add(0x11, "office"), agent("0123456789abcdef"))
        .unwrap();
    assert!(!requested.existing);
    assert_eq!(
        rig.ops.calls(),
        Vec::<String>::new(),
        "nothing applied before the human answers"
    );
    let pending = rig.approvals.pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, requested.id);
    assert_eq!(pending[0].kind, Kind::Add);
    assert_eq!(pending[0].expires_in, Some(REQUEST_TTL));

    let shown = next_dialog(&rig.shown);
    assert_eq!(shown.heading, "An agent asks to trust another PC");
    assert!(
        shown
            .body
            .starts_with("An agent (id 01234567…) asks. Its name: \"lead\""),
        "the system-written id comes first: {}",
        shown.body
    );
    assert!(
        shown.body.contains(&pc(0x11).grouped()),
        "the full grouped ID: {}",
        shown.body
    );
    assert!(shown.body.contains("PC name: \"office\""), "{}", shown.body);
    assert!(
        shown.body.contains(&format!("Request {}", requested.id)),
        "{}",
        shown.body
    );
    assert_eq!(shown.labels[2], "Deny and stop asking this agent");

    shown.answer(ApprovalAnswer::Approve);
    eventually("the approval was applied", || {
        rig.ops.calls() == ["add office"]
    });
    assert!(
        rig.approvals.pending().is_empty(),
        "the request was consumed"
    );
}

/// Every answer consumes the request before applying it: a second answer finds nothing, and the dialog showing it is dismissed.
#[test]
fn an_answer_consumes_the_request_and_a_second_one_is_ignored() {
    let rig = rig();
    let requested = rig
        .approvals
        .request(add(0x11, "office"), agent("a1"))
        .unwrap();
    let _shown = next_dialog(&rig.shown);

    let answered = rig
        .approvals
        .answer(&requested.id, Decision::Approve)
        .unwrap();
    assert_eq!(answered, Answered::Applied(Applied::Done));
    assert_eq!(rig.ops.calls(), ["add office"]);
    assert_eq!(
        rig.approvals.answer(&requested.id, Decision::Approve),
        Err(NoRequest::Unknown)
    );
    assert_eq!(rig.ops.calls(), ["add office"], "applied once");
    eventually("the dialog left open was dismissed", || {
        rig.dialogs.dismissed() == 1
    });
}

#[test]
fn a_request_expires_after_a_day_and_a_late_answer_is_ignored() {
    let rig = rig();
    let requested = rig
        .approvals
        .request(add(0x11, "office"), agent("a1"))
        .unwrap();
    let _shown = next_dialog(&rig.shown);
    rig.clock.advance(REQUEST_TTL - Duration::from_secs(1));
    rig.approvals.expire();
    assert_eq!(rig.approvals.pending().len(), 1, "not yet");

    rig.clock.advance(Duration::from_secs(1));
    rig.approvals.expire();
    assert!(rig.approvals.pending().is_empty());
    eventually("the expired request's dialog was dismissed", || {
        rig.dialogs.dismissed() == 1
    });
    assert_eq!(
        rig.approvals.answer(&requested.id, Decision::Approve),
        Err(NoRequest::Unknown)
    );
    assert!(rig.ops.calls().is_empty());
}

#[test]
fn at_most_eight_requests_wait_and_an_identical_one_is_not_added_twice() {
    let rig = rig();
    let first = rig.approvals.request(add(1, "pc1"), agent("a1")).unwrap();
    let again = rig.approvals.request(add(1, "pc1"), agent("a2")).unwrap();
    assert_eq!(
        again,
        Requested {
            id: first.id.clone(),
            existing: true
        }
    );
    for byte in 2..=MAX_PENDING as u8 {
        rig.approvals
            .request(add(byte, &format!("pc{byte}")), agent("a1"))
            .unwrap();
    }
    assert_eq!(rig.approvals.pending().len(), MAX_PENDING);
    assert_eq!(
        rig.approvals
            .request(add(0x40, "one-too-many"), agent("a1")),
        Err(Refusal::Full)
    );
}

/// After a deny the same (agent, kind) is refused for 10 min; another kind or another agent is not.
#[test]
fn a_deny_cools_down_the_same_agent_and_kind() {
    let rig = rig();
    let requested = rig.approvals.request(add(1, "pc1"), agent("a1")).unwrap();
    assert_eq!(
        rig.approvals.answer(&requested.id, Decision::Deny),
        Ok(Answered::Denied)
    );

    rig.clock.advance(DENY_COOLDOWN - Duration::from_secs(60));
    assert!(matches!(
        rig.approvals.request(add(2, "pc2"), agent("a1")),
        Err(Refusal::CoolingDown(_))
    ));
    assert!(
        rig.approvals
            .request(Change::RegenerateKey, agent("a1"))
            .is_ok(),
        "another kind"
    );
    assert!(
        rig.approvals.request(add(2, "pc2"), agent("a2")).is_ok(),
        "another agent"
    );

    rig.clock.advance(Duration::from_secs(60));
    assert!(
        rig.approvals.request(add(3, "pc3"), agent("a1")).is_ok(),
        "after 10 min"
    );
    assert!(rig.ops.calls().is_empty());
}

/// "Deny and stop asking" refuses that agent's gated calls for the rest of the run and drops its other requests.
#[test]
fn deny_and_stop_refuses_that_agent_for_the_run() {
    let rig = rig();
    let first = rig.approvals.request(add(1, "pc1"), agent("a1")).unwrap();
    rig.approvals
        .request(Change::RegenerateKey, agent("a1"))
        .unwrap();
    let other = rig.approvals.request(add(2, "pc2"), agent("a2")).unwrap();

    rig.approvals
        .answer(&first.id, Decision::DenyAndStop)
        .unwrap();
    let left: Vec<String> = rig.approvals.pending().into_iter().map(|r| r.id).collect();
    assert_eq!(left, [other.id], "only the other agent's request stays");
    rig.clock.advance(REQUEST_TTL);
    assert_eq!(
        rig.approvals.request(Change::RegenerateKey, agent("a1")),
        Err(Refusal::Stopped)
    );
}

/// Best-effort origin: after a remote mail reached the agent this run, its request names that PC.
#[test]
fn a_request_after_remote_mail_names_the_origin_pc() {
    let rig = rig();
    rig.approvals.note_mail_from("a1", "home");
    rig.approvals.request(add(1, "pc1"), agent("a1")).unwrap();
    let pending = rig.approvals.pending();
    assert_eq!(pending[0].origin.as_deref(), Some("home"));
    let shown = next_dialog(&rig.shown);
    assert!(
        shown.body.contains("It asks after mail from PC \"home\"."),
        "{}",
        shown.body
    );

    rig.approvals.request(add(2, "pc2"), agent("a2")).unwrap();
    assert_eq!(
        rig.approvals.pending()[1].origin,
        None,
        "another agent got no remote mail"
    );
}

/// While several wait, the dialog answers only the one it shows.
#[test]
fn a_dialog_answer_applies_only_to_the_request_shown() {
    let rig = rig();
    rig.approvals.request(add(1, "pc1"), agent("a1")).unwrap();
    let first = next_dialog(&rig.shown);
    let second = rig.approvals.request(add(2, "pc2"), agent("a1")).unwrap();
    first.answer(ApprovalAnswer::Approve);
    eventually("the shown request was applied", || {
        rig.ops.calls() == ["add pc1"]
    });
    let left: Vec<String> = rig.approvals.pending().into_iter().map(|r| r.id).collect();
    assert_eq!(left, [second.id]);
}

/// The question stays in the app while it has a PC list slot open; there the human answers by id.
#[test]
fn with_the_app_showing_the_slot_the_request_waits_there() {
    let app = Arc::new(FakeApp::default());
    app.set_present(true);
    let rig = rig_with_app(app.clone());
    let requested = rig.approvals.request(add(1, "pc1"), agent("a1")).unwrap();
    no_dialog(&rig.shown);
    assert_eq!(
        rig.approvals.answer(&requested.id, Decision::Approve),
        Ok(Answered::Applied(Applied::Done))
    );
    app.set_present(false);
    rig.approvals.route_changed();
    no_dialog(&rig.shown);
}

/// No desktop: the request waits until the app answers it or it expires.
#[test]
fn without_a_desktop_a_request_waits_until_it_expires() {
    let rig = rig();
    rig.dialogs.without_desktop();
    rig.approvals.request(add(1, "pc1"), agent("a1")).unwrap();
    eventually("the dialog was tried", || rig.dialogs.asked() == 1);
    assert_eq!(rig.approvals.pending().len(), 1);
    rig.clock.advance(REQUEST_TTL);
    assert!(rig.approvals.pending().is_empty());
}

/// ADR-0394 d7 · ADR-0389: a list changed outside Engram raises the accept question (every entry with its full ID, the network
///   settings); accepting re-signs it; a state that no longer needs it withdraws it.
#[test]
fn a_changed_list_raises_the_accept_question_and_accepting_applies_it() {
    let rig = rig();
    rig.ops.with(|state| {
        state.status = PeerStatus::ListChangedOutside;
        state.peers.push(TrustedPeer {
            id: pc(0x22),
            name: "laptop".to_string(),
            address: Some("10.0.0.5:47120".parse().unwrap()),
            permission: Permission::Mail,
        });
    });
    rig.approvals.sync_state(&rig.ops.snapshot());
    rig.approvals.sync_state(&rig.ops.snapshot());
    let pending = rig.approvals.pending();
    assert_eq!(pending.len(), 1, "asked once");
    assert_eq!(pending[0].kind, Kind::AcceptList);
    assert_eq!(pending[0].agent, None);
    assert_eq!(
        pending[0].expires_in, None,
        "stands while the state needs it"
    );

    let shown = next_dialog(&rig.shown);
    assert_eq!(shown.heading, "The trusted list was changed outside Engram");
    assert!(shown.body.contains(&pc(0x22).grouped()), "{}", shown.body);
    assert!(shown.body.contains("PC name: \"laptop\""), "{}", shown.body);
    assert!(
        shown
            .body
            .contains("Relay server: n0's public servers (default)"),
        "{}",
        shown.body
    );
    shown.answer(ApprovalAnswer::Approve);
    eventually("the list was accepted", || {
        rig.ops.calls() == ["acceptList"]
    });

    rig.approvals.sync_state(&rig.ops.snapshot());
    assert!(rig.approvals.pending().is_empty(), "ready — nothing to ask");
}

#[test]
fn a_question_the_state_no_longer_needs_is_withdrawn() {
    let rig = rig();
    rig.ops
        .with(|state| state.status = PeerStatus::ListCorrupt("bad".to_string()));
    rig.approvals.sync_state(&rig.ops.snapshot());
    assert_eq!(
        next_dialog(&rig.shown).heading,
        "The trusted list file is damaged"
    );
    rig.ops.with(|state| state.status = PeerStatus::Ready);
    rig.approvals.sync_state(&rig.ops.snapshot());
    assert!(rig.approvals.pending().is_empty());
    eventually("its dialog was dismissed", || rig.dialogs.dismissed() == 1);
}

/// "Not now" holds a system question back for 10 min; "not this run" until the daemon restarts.
#[test]
fn a_declined_system_question_returns_after_the_cooldown_unless_stopped() {
    let rig = rig();
    rig.ops
        .with(|state| state.status = PeerStatus::ListChangedOutside);
    rig.approvals.sync_state(&rig.ops.snapshot());
    next_dialog(&rig.shown).answer(ApprovalAnswer::Deny);
    eventually("the deny was taken", || rig.approvals.pending().is_empty());
    rig.approvals.sync_state(&rig.ops.snapshot());
    assert!(rig.approvals.pending().is_empty(), "held back");

    rig.clock.advance(DENY_COOLDOWN);
    rig.approvals.sync_state(&rig.ops.snapshot());
    let again = rig.approvals.pending();
    assert_eq!(again.len(), 1, "asked again after the cooldown");
    assert!(rig.ops.calls().is_empty(), "a deny leaves the feature off");

    rig.approvals
        .answer(&again[0].id, Decision::DenyAndStop)
        .unwrap();
    rig.clock.advance(REQUEST_TTL);
    rig.approvals.sync_state(&rig.ops.snapshot());
    assert!(rig.approvals.pending().is_empty(), "not again this run");
}

/// This account's identity under another machine id → one adopt question per machine; approving adopts that one. Never adopted silently.
#[test]
fn the_adopt_question_adopts_the_machine_it_names() {
    let rig = rig();
    rig.ops.with(|state| {
        state.status = PeerStatus::AdoptOrFresh {
            machines: vec!["old-machine".to_string()],
        }
    });
    rig.approvals.sync_state(&rig.ops.snapshot());
    assert!(rig.ops.calls().is_empty(), "never adopted without a human");
    let pending = rig.approvals.pending();
    assert_eq!(pending[0].kind, Kind::Adopt);
    let shown = next_dialog(&rig.shown);
    assert!(shown.body.contains("\"old-machine\""), "{}", shown.body);
    assert_eq!(shown.labels[0], "Adopt");
    drop(shown);
    assert_eq!(
        rig.approvals.answer(&pending[0].id, Decision::Approve),
        Ok(Answered::Applied(Applied::Done))
    );
    assert_eq!(rig.ops.calls(), ["adopt old-machine"]);
}

/// A regenerate while the adopt question is open makes a fresh identity instead (the human's "start fresh").
#[test]
fn regenerate_while_the_adopt_question_is_open_starts_fresh() {
    let ops = Arc::new(FakeOps::default());
    ops.with(|state| {
        state.status = PeerStatus::AdoptOrFresh {
            machines: vec!["m".to_string()],
        }
    });
    assert_eq!(
        apply(&ops, &Change::RegenerateKey),
        Ok(Applied::Fresh(pc(0xDD)))
    );
    assert_eq!(ops.calls(), ["startFresh"]);
    assert!(matches!(
        apply(&ops, &Change::RegenerateKey),
        Ok(Applied::Regenerated(_))
    ));
}

#[test]
fn quoted_strings_are_one_line_and_capped() {
    assert_eq!(quote("a\nb\tc"), "\"a\u{FFFD}b\u{FFFD}c\"");
    let long = "x".repeat(200);
    let quoted = quote(&long);
    assert_eq!(quoted.chars().count(), text::QUOTE_CAP + 3, "{quoted}");
    assert!(quoted.ends_with("…\""));
}

#[test]
fn an_approval_without_a_running_peer_link_leaves_the_request() {
    let (dialogs, _shown) = FakeDialogs::new();
    let approvals = Approvals::start(
        crate::link::peer::testing::not_started(),
        dialogs,
        Arc::new(FakeApp::default()),
        Arc::new(FakeLanguage::default()),
        Arc::new(ManualClock::new()),
    );
    let requested = approvals.request(add(1, "pc1"), agent("a1")).unwrap();
    assert_eq!(
        approvals.answer(&requested.id, Decision::Approve),
        Err(NoRequest::NotRunning)
    );
    assert_eq!(approvals.pending().len(), 1);
}

// ── question text (ADR-0394 d6) ──

/// Strings from outside cannot imitate or reorder the system-written text: quotes are escaped; line separators, bidi overrides and
///   zero-width characters show as a visible mark.
#[test]
fn quoted_strings_escape_quotes_and_show_hidden_characters() {
    assert_eq!(quote("le\"ad"), "\"le\\\"ad\"");
    assert_eq!(quote("a\u{2028}b"), "\"a\u{FFFD}b\"", "line separator");
    assert_eq!(
        quote("x\u{202E}y"),
        "\"x\u{FFFD}y\"",
        "right-to-left override"
    );
    assert_eq!(quote("p\u{200B}q"), "\"p\u{FFFD}q\"", "zero-width space");
    assert_eq!(quote("back\\slash"), "\"back\\\\slash\"");
}

/// An agent that names itself like the system label cannot push the real one away: the system-written id comes first.
#[test]
fn a_forged_agent_name_stays_inside_its_quotes_after_the_real_id() {
    let rig = rig();
    let forged = Requester {
        id: "9d8e7f6a-0000-4000-8000-000000000009".to_string(),
        name: Some("lead\" (id 01234567…)\u{202E}".to_string()),
    };
    rig.approvals.request(add(0x11, "office"), forged).unwrap();
    let shown = next_dialog(&rig.shown);
    let first_line = shown.body.lines().next().unwrap().to_string();
    assert_eq!(
        first_line,
        "An agent (id 9d8e7f6a…) asks. Its name: \"lead\\\" (id 01234567…)\u{FFFD}\""
    );
}

/// A long helper host is shown whole; only the path is capped.
#[test]
fn a_helper_url_shows_its_whole_host() {
    let host = format!("use1-1.relay.n0.iroh.link.{}.evil.example", "a".repeat(60));
    let url = format!("https://{host}:8443/{}", "p".repeat(100));
    let shown = text::url_text(&url);
    assert!(
        shown.starts_with(&format!("https://{host}:8443 ")),
        "{shown}"
    );
    assert!(shown.contains("evil.example"), "{shown}");
    assert!(shown.ends_with("…\""), "the path is capped: {shown}");
    assert_eq!(
        text::url_text("https://relay.example.com/"),
        "https://relay.example.com"
    );
}

fn listed(rig: &Rig, byte: u8, name: &str) {
    rig.ops.with(|state| {
        state.peers.push(TrustedPeer {
            id: pc(byte),
            name: name.to_string(),
            address: Some("10.0.0.5:47120".parse().unwrap()),
            permission: Permission::Mail,
        })
    });
}

/// A rename shows the current name → the new one, and which `name@pc` addresses stop working.
#[test]
fn the_rename_question_shows_the_old_and_the_new_name() {
    let rig = rig();
    listed(&rig, 0x22, "home");
    let edit = PeerEdit {
        name: Some("office".to_string()),
        address: None,
    };
    rig.approvals
        .request(Change::Edit { id: pc(0x22), edit }, agent("a1"))
        .unwrap();
    let shown = next_dialog(&rig.shown);
    assert!(
        shown.body.contains("PC name: \"home\" → \"office\""),
        "{}",
        shown.body
    );
    assert!(
        shown.body.contains("name@home stop resolving"),
        "{}",
        shown.body
    );
    assert!(shown.body.contains(&pc(0x22).grouped()), "{}", shown.body);
}

/// The add question marks a PC that knocked uninvited, and a name another ID held earlier this run.
#[test]
fn the_add_question_marks_a_knocker_and_a_reused_name() {
    let rig = rig();
    listed(&rig, 0x22, "office");
    rig.approvals.sync_state(&rig.ops.snapshot());
    rig.ops.with(|state| {
        state.peers.clear();
        state
            .knocks
            .push(crate::link::peer::testing::knock(pc(0x33)));
    });
    rig.approvals
        .request(add(0x33, "Office"), agent("a1"))
        .unwrap();
    let shown = next_dialog(&rig.shown);
    assert!(
        shown
            .body
            .contains("that PC tried to link to this one while it was not on the trusted list"),
        "{}",
        shown.body
    );
    assert!(
        shown.body.contains(&format!(
            "belonged to another PC (ID {})",
            pc(0x22).grouped()
        )),
        "{}",
        shown.body
    );
}

/// A network request holds only the fields asked for, shows from → to, and approving it later keeps a change made since.
#[test]
fn a_network_request_merges_with_the_settings_of_the_moment_it_is_approved() {
    let rig = rig();
    let patch = NetworkPatch {
        mdns: Some(false),
        ..NetworkPatch::default()
    };
    let requested = rig
        .approvals
        .request(Change::SetNetwork(patch), agent("a1"))
        .unwrap();
    let shown = next_dialog(&rig.shown);
    assert!(
        shown
            .body
            .contains("Local network discovery (mDNS): on → off"),
        "{}",
        shown.body
    );
    assert!(
        !shown.body.contains("Relay server"),
        "unchanged fields are not listed: {}",
        shown.body
    );
    // The human changes the relay meanwhile.
    rig.ops.with(|state| state.network.relay = Helper::None);
    rig.approvals
        .answer(&requested.id, Decision::Approve)
        .unwrap();
    let network = rig.ops.state.lock().unwrap().network.clone();
    assert!(!network.mdns);
    assert_eq!(network.relay, Helper::None, "the later relay change stays");
}

/// The accept-list question lists a bounded number of PCs and counts the rest.
#[test]
fn the_accept_question_lists_a_bounded_number_of_pcs() {
    let rig = rig();
    for byte in 1..=(text::LISTED_PCS as u8 + 3) {
        listed(&rig, byte, &format!("pc{byte}"));
    }
    rig.ops
        .with(|state| state.status = PeerStatus::ListChangedOutside);
    rig.approvals.sync_state(&rig.ops.snapshot());
    let shown = next_dialog(&rig.shown);
    assert!(
        shown
            .body
            .starts_with(&format!("Trusted PCs: {}", text::LISTED_PCS + 3)),
        "{}",
        shown.body
    );
    assert_eq!(shown.body.matches("PC name:").count(), text::LISTED_PCS);
    assert!(
        shown.body.contains("3 more — review them in the app"),
        "{}",
        shown.body
    );
    assert_eq!(
        shown.labels[1], "Not now",
        "the default is still not to accept"
    );
}

/// Several machine ids → one adopt question at a time, which counts the others; its third answer starts fresh.
#[test]
fn adopt_asks_one_machine_at_a_time_and_its_third_answer_starts_fresh() {
    let rig = rig();
    rig.ops.with(|state| {
        state.status = PeerStatus::AdoptOrFresh {
            machines: vec!["m-b".to_string(), "m-a".to_string(), "m-c".to_string()],
        }
    });
    rig.approvals.sync_state(&rig.ops.snapshot());
    let pending = rig.approvals.pending();
    assert_eq!(pending.len(), 1, "one question, not one per machine id");
    let shown = next_dialog(&rig.shown);
    assert!(
        shown.body.contains("\"m-a\""),
        "the first by name: {}",
        shown.body
    );
    assert!(shown.body.contains("2 other machine ids"), "{}", shown.body);
    assert_eq!(shown.labels[2], "Start fresh");
    shown.answer(ApprovalAnswer::DenyAndStop);
    eventually("started fresh", || rig.ops.calls() == ["startFresh"]);
}

#[test]
fn start_fresh_answers_only_the_adopt_request() {
    let rig = rig();
    let requested = rig.approvals.request(add(1, "pc1"), agent("a1")).unwrap();
    assert_eq!(
        rig.approvals.answer(&requested.id, Decision::StartFresh),
        Err(NoRequest::NotAdopt)
    );
    assert_eq!(rig.approvals.pending().len(), 1, "the request still waits");
    rig.ops.with(|state| {
        state.status = PeerStatus::AdoptOrFresh {
            machines: vec!["m".to_string()],
        }
    });
    rig.approvals.sync_state(&rig.ops.snapshot());
    let adopt = rig
        .approvals
        .pending()
        .into_iter()
        .find(|row| row.kind == Kind::Adopt)
        .unwrap();
    assert_eq!(
        rig.approvals.answer(&adopt.id, Decision::StartFresh),
        Ok(Answered::Applied(Applied::Fresh(pc(0xDD))))
    );
}

/// While the adopt question is open, an agent's regenerate request says it abandons the saved identity.
#[test]
fn a_regenerate_request_during_adopt_says_it_abandons_the_saved_identity() {
    let rig = rig();
    rig.ops.with(|state| {
        state.status = PeerStatus::AdoptOrFresh {
            machines: vec!["m".to_string()],
        }
    });
    rig.approvals
        .request(Change::RegenerateKey, agent("a1"))
        .unwrap();
    let shown = next_dialog(&rig.shown);
    assert!(
        shown.heading.contains("instead of adopting"),
        "{}",
        shown.heading
    );
    assert!(
        shown.body.contains("abandons the identity"),
        "{}",
        shown.body
    );
}

/// ADR-0372: every question is written in Korean too, and the Korean text is not the English one.
#[test]
fn every_question_has_a_korean_text() {
    let q = text::change_question(
        &text::Asked::RegenerateKey {
            instead_of_adopting: false,
        },
        "a1",
        Some("lead"),
        None,
        "r1",
    );
    for (en, ko) in [
        (&q.en.title, &q.ko.title),
        (&q.en.heading, &q.ko.heading),
        (&q.en.body, &q.ko.body),
        (&q.en.approve, &q.ko.approve),
        (&q.en.deny, &q.ko.deny),
        (&q.en.deny_and_stop, &q.ko.deny_and_stop),
    ] {
        assert!(!ko.is_empty() && ko != en, "{en:?} / {ko:?}");
    }
    let accept = text::accept_list_question(false, &[], None, "r2");
    assert_ne!(accept.en.heading, accept.ko.heading);
    let adopt = text::adopt_question("m", 0, "r3");
    assert_ne!(adopt.en.deny_and_stop, adopt.ko.deny_and_stop);
    assert!(
        q.en.body.ends_with("Request r1."),
        "no expiry frozen in the dialog: {}",
        q.en.body
    );
}

// ── round 2 ──

/// A network request lists every field it names, also one equal to the current value — the human sees each value approving writes.
#[test]
fn a_network_request_lists_every_named_field_even_an_equal_one() {
    let rig = rig();
    let patch = NetworkPatch {
        relay: Some(Helper::Default),
        mdns: Some(false),
        ..NetworkPatch::default()
    };
    rig.approvals
        .request(Change::SetNetwork(patch), agent("a1"))
        .unwrap();
    let shown = next_dialog(&rig.shown);
    assert!(
        shown.body.contains(
            "Relay server: n0's public servers (default) → n0's public servers (default)"
        ),
        "{}",
        shown.body
    );
    assert!(shown.body.contains("(mDNS): on → off"), "{}", shown.body);
    assert!(
        !shown.body.contains("Listen port"),
        "an unnamed field is not listed: {}",
        shown.body
    );
}

/// A named field changed between the request and the approval: the request's value wins (that is what the human approved), and the
///   answer says which field it replaced.
#[test]
fn an_approved_network_request_reports_a_named_field_that_moved_since() {
    let rig = rig();
    let patch = NetworkPatch {
        relay: Some(Helper::Default),
        ..NetworkPatch::default()
    };
    let requested = rig
        .approvals
        .request(Change::SetNetwork(patch), agent("a1"))
        .unwrap();
    rig.ops.with(|state| state.network.relay = Helper::None);
    assert_eq!(
        rig.approvals.answer(&requested.id, Decision::Approve),
        Ok(Answered::Applied(Applied::NetworkMoved(vec!["relay"])))
    );
    assert_eq!(rig.ops.state.lock().unwrap().network.relay, Helper::Default);
}

/// C3: `stop` waits for an answer being applied, and nothing is answered or applied after it began.
#[test]
fn stop_waits_for_an_answer_being_applied_and_nothing_applies_after_it() {
    let rig = rig();
    let (release, hold) = std::sync::mpsc::channel();
    *rig.ops.hold_add.lock().unwrap() = Some(hold);
    rig.approvals.request(add(1, "pc1"), agent("a1")).unwrap();
    let other = rig.approvals.request(add(2, "pc2"), agent("a2")).unwrap();
    next_dialog(&rig.shown).answer(ApprovalAnswer::Approve);
    eventually("the approval is being applied", || {
        rig.ops
            .add_entered
            .load(std::sync::atomic::Ordering::SeqCst)
    });

    let approvals = rig.approvals.clone();
    let stopping = std::thread::spawn(move || approvals.stop());
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        !stopping.is_finished(),
        "stop waits for the answer being applied"
    );
    release.send(()).unwrap();
    stopping.join().unwrap();
    assert_eq!(
        rig.ops.calls(),
        ["add pc1"],
        "the answer that began before the stop completed"
    );

    *rig.ops.hold_add.lock().unwrap() = None;
    assert_eq!(
        rig.approvals.answer(&other.id, Decision::Approve),
        Err(NoRequest::Unknown),
        "after the stop no request is left to answer"
    );
    assert!(rig.approvals.pending().is_empty());
    assert_eq!(
        rig.approvals.request(add(3, "pc3"), agent("a1")),
        Err(Refusal::Stopping)
    );
    assert_eq!(rig.ops.calls(), ["add pc1"]);
}

/// D1: "not this run" on the adopt question holds the question as a whole back — no other machine id is asked instead.
#[test]
fn declining_the_adopt_question_holds_it_for_the_run() {
    let rig = rig();
    rig.ops.with(|state| {
        state.status = PeerStatus::AdoptOrFresh {
            machines: vec!["m-a".to_string(), "m-b".to_string()],
        }
    });
    rig.approvals.sync_state(&rig.ops.snapshot());
    let shown = next_dialog(&rig.shown);
    assert_eq!(shown.labels[1], "Not this run");
    shown.answer(ApprovalAnswer::Deny);
    eventually("the deny was taken", || rig.approvals.pending().is_empty());
    rig.clock.advance(REQUEST_TTL);
    rig.approvals.sync_state(&rig.ops.snapshot());
    assert!(
        rig.approvals.pending().is_empty(),
        "neither m-a nor m-b is asked again this run"
    );
    assert!(rig.ops.calls().is_empty());
}

/// D3: a stack of combining marks is kept to two per character, and quote look-alikes are escaped like `"`.
#[test]
fn visible_text_caps_combining_marks_and_escapes_quote_lookalikes() {
    let stacked = format!("a{}b", "\u{0301}".repeat(40));
    assert_eq!(text::visible(&stacked, 80), "a\u{0301}\u{0301}\u{FFFD}b");
    assert_eq!(quote("\u{201C}x\u{201D}\u{FF02}"), "\"\\\"x\\\"\\\"\"");
}

// ── round 3 ──

/// E2: only stacking marks are capped — conjoining jamo and an Indic conjunct keep every code point.
#[test]
fn visible_text_keeps_every_letter_of_a_cluster() {
    let jamo = "\u{1100}\u{1161}\u{11A8}";
    assert_eq!(
        text::visible(jamo, 80),
        jamo,
        "a syllable written in conjoining jamo keeps its final consonant"
    );
    let conjunct = "\u{0915}\u{094D}\u{0937}\u{093F}";
    assert_eq!(text::visible(conjunct, 80), conjunct);
    assert_eq!(quote("\u{2018}x\u{00BB}"), "\"\\\"x\\\"\"");
}

/// E3: shutdown does not wait without end for an apply that hangs — at most `STOP_WAIT`, then it goes on; nothing new gets through.
#[test]
fn stop_waits_for_a_hanging_apply_only_for_its_bound() {
    let rig = rig();
    let (release, hold) = std::sync::mpsc::channel();
    *rig.ops.hold_add.lock().unwrap() = Some(hold);
    rig.approvals.request(add(1, "pc1"), agent("a1")).unwrap();
    let other = rig.approvals.request(add(2, "pc2"), agent("a2")).unwrap();
    next_dialog(&rig.shown).answer(ApprovalAnswer::Approve);
    eventually("the approval is being applied", || {
        rig.ops
            .add_entered
            .load(std::sync::atomic::Ordering::SeqCst)
    });
    let started = std::time::Instant::now();
    rig.approvals.stop();
    let waited = started.elapsed();
    assert!(waited >= STOP_WAIT, "it waited its bound: {waited:?}");
    assert!(
        waited < STOP_WAIT * 2 + Duration::from_secs(5),
        "and not much longer: {waited:?}"
    );
    assert_eq!(
        rig.approvals.answer(&other.id, Decision::Approve),
        Err(NoRequest::Unknown)
    );
    assert_eq!(
        rig.approvals.request(add(3, "pc3"), agent("a1")),
        Err(Refusal::Stopping)
    );
    release.send(()).unwrap();
    eventually("the hanging apply finishes on its own", || {
        rig.ops.calls() == ["add pc1"]
    });
}

// ── round 4 ──

/// F1: marks outside the stacking blocks are bounded by the per-character limit; one U+FFFD marks what was dropped.
#[test]
fn every_cluster_is_bounded_and_whole_letters_fit() {
    for mark in ['\u{0654}', '\u{0E49}'] {
        let shown = text::visible(&format!("a{}b", mark.to_string().repeat(100)), 80);
        assert_eq!(
            shown.chars().filter(|c| *c == '\u{FFFD}').count(),
            1,
            "{shown:?}"
        );
        assert_eq!(
            shown.chars().count(),
            text::MAX_CLUSTER_CHARS + 2,
            "{shown:?}"
        );
        assert!(shown.ends_with("\u{FFFD}b"), "{shown:?}");
    }
    let jamo = "\u{1100}\u{1161}\u{11A8}";
    assert_eq!(text::visible(jamo, 80), jamo);
    // Seven code points, one grapheme (man, ZWJ, woman, ZWJ, girl, ZWJ, boy).
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    assert_eq!(family.chars().count(), 7);
    let shown = text::visible(family, 80);
    assert_eq!(
        shown.chars().count(),
        7,
        "kept whole (each ZWJ shows as U+FFFD, the invisible-character rule): {shown:?}"
    );
}

/// F2: the request is written from the state the gate checked — not a later read — and a missing "from" reports every named field.
#[test]
fn a_network_request_is_written_from_the_state_the_caller_checked() {
    let rig = rig();
    let patch = NetworkPatch {
        relay: Some(Helper::None),
        ..NetworkPatch::default()
    };
    let seen = rig.ops.snapshot();
    // The state turns unreadable after the check; the request still has the settings that were checked.
    rig.ops.with(|state| state.network_unknown = true);
    let requested = rig
        .approvals
        .request_seen(Change::SetNetwork(patch.clone()), agent("a1"), Some(seen))
        .unwrap();
    rig.ops.with(|state| {
        state.network_unknown = false;
        state.network.relay = Helper::Url("https://relay.example.com".to_string());
    });
    assert_eq!(
        rig.approvals.answer(&requested.id, Decision::Approve),
        Ok(Answered::Applied(Applied::NetworkMoved(vec!["relay"]))),
        "the drift since the checked state is reported"
    );
    rig.ops.with(|state| state.network_unknown = true);
    assert_eq!(
        rig.approvals
            .request(Change::SetNetwork(patch), agent("a1")),
        Err(Refusal::NetworkUnknown),
        "no known settings — refused"
    );
}

/// C3 QA: the stop gate's own deadline — an answer applied through `answer` on a thread of its own (not the dialog thread), held past the
///   bound: `stop` waits for it `STOP_WAIT` and not much longer. The dialog thread is idle here, so its own stop wait cannot stand in.
#[test]
fn stop_waits_its_own_bound_for_an_answer_applied_off_the_dialog_thread() {
    let rig = rig();
    let (release, hold) = std::sync::mpsc::channel();
    *rig.ops.hold_add.lock().unwrap() = Some(hold);
    let request = rig.approvals.request(add(1, "pc1"), agent("a1")).unwrap();
    let approvals = rig.approvals.clone();
    let answering = std::thread::spawn(move || approvals.answer(&request.id, Decision::Approve));
    eventually("the approval is being applied", || {
        rig.ops
            .add_entered
            .load(std::sync::atomic::Ordering::SeqCst)
    });
    let started = std::time::Instant::now();
    rig.approvals.stop();
    let waited = started.elapsed();
    assert!(waited >= STOP_WAIT, "it waited its bound: {waited:?}");
    assert!(
        waited < STOP_WAIT + Duration::from_secs(2),
        "and returned soon after it: {waited:?}"
    );
    release.send(()).unwrap();
    assert!(answering.join().unwrap().is_ok());
    assert_eq!(rig.ops.calls(), ["add pc1"]);
}

/// The app's copy of a question follows the language the shell set (ADR-0406), read anew on each `pending()` — a switch re-words
///   the cards on the next broadcast.
#[test]
fn a_pending_rows_question_is_in_the_language_the_shell_set() {
    let ops = Arc::new(FakeOps::default());
    let (dialogs, _shown) = FakeDialogs::new();
    let language = Arc::new(FakeLanguage::default());
    // The app has a PC list slot open, so no dialog takes the question.
    let app = Arc::new(FakeApp::default());
    app.set_present(true);
    let approvals = Approvals::start(
        source(ops.clone()),
        dialogs,
        app,
        language.clone(),
        Arc::new(ManualClock::new()),
    );
    approvals.request(add(0x22, "laptop"), agent("a1")).unwrap();
    let (en, ko) = {
        let inner = lock(&approvals.inner);
        let asked = &inner.pending[0].text;
        (asked.en.clone(), asked.ko.clone())
    };
    assert_ne!(en, ko);
    assert_eq!(approvals.pending()[0].text, en);
    language.set(crate::ask_user::Lang::Ko);
    assert_eq!(approvals.pending()[0].text, ko);
    approvals.stop();
}
