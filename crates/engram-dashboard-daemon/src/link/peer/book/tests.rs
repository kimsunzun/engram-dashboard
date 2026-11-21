//! The link book on synthetic events and instants — tie-break, re-arm, the dialing handle's removal and the duplicate rules.

use super::*;

const A: PeerId = PeerId::from_bytes([1; 32]);
const B: PeerId = PeerId::from_bytes([2; 32]);

fn run(n: u128) -> Instance {
    Instance::from_u128(n)
}

fn secs(base: Instant, s: u64) -> Instant {
    base + Duration::from_secs(s)
}

/// A book for `own` tracking `other`; returns the dialing handle's key.
fn book(own: PeerId, other: PeerId) -> (LinkBook, LinkKey) {
    let mut book = LinkBook::new(own, 100);
    let actions = book.ensure(other);
    let [Action::AddDial { node, key }] = actions.as_slice() else {
        panic!("{actions:?}")
    };
    assert_eq!(*node, other);
    (book, *key)
}

fn closes(actions: &[Action]) -> Vec<(LinkKey, CloseAs)> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::Close { key, how, .. } => Some((*key, *how)),
            _ => None,
        })
        .collect()
}

fn sends(actions: &[Action]) -> Vec<(LinkKey, PeerMsg)> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::Send { key, msg } => Some((*key, msg.clone())),
            _ => None,
        })
        .collect()
}

fn added(actions: &[Action]) -> Option<LinkKey> {
    actions.iter().find_map(|a| match a {
        Action::AddDial { key, .. } => Some(*key),
        _ => None,
    })
}

fn probe_nonce(actions: &[Action]) -> u64 {
    sends(actions)
        .into_iter()
        .find_map(|(_, msg)| match msg {
            PeerMsg::Probe { nonce } => Some(nonce),
            _ => None,
        })
        .expect("a probe")
}

#[test]
fn each_listed_pc_gets_one_dialing_handle() {
    let (mut book, _) = book(A, B);
    assert!(book.ensure(B).is_empty(), "a second ensure adds nothing");
    assert_eq!(book.online(&B), Online::Offline);
}

#[test]
fn a_dialed_link_is_chosen_and_its_loss_leaves_the_handle_to_redial() {
    let t = Instant::now();
    let (mut book, dial) = book(A, B);
    assert!(book
        .link_up(B, dial, Dir::Dialed, run(1), false, t)
        .is_empty());
    assert_eq!(book.online(&B), Online::Online);
    assert_eq!(book.chosen(&B), Some(dial));
    assert!(
        book.link_down(dial, secs(t, 1)).is_empty(),
        "the dialing handle redials by itself"
    );
    assert_eq!(book.online(&B), Online::Offline);
    assert_eq!(book.chosen(&B), None);
}

#[test]
fn both_pcs_keep_the_same_link_when_they_dial_together() {
    let t = Instant::now();
    // A < B. Link X = A dials B (A: dialed, B: accepted); link Y = B dials A (A: accepted, B: dialed).
    for (a_first_x, b_first_x) in [(true, true), (true, false), (false, true), (false, false)] {
        let (mut a, a_dial) = book(A, B);
        let (mut b, b_dial) = book(B, A);
        let a_acc = a.accepted_key();
        let b_acc = b.accepted_key();
        let mut a_out = Vec::new();
        let mut b_out = Vec::new();
        let (a_x, a_y) = ((a_dial, Dir::Dialed), (a_acc, Dir::Accepted));
        let (b_x, b_y) = ((b_acc, Dir::Accepted), (b_dial, Dir::Dialed));
        let a_order = if a_first_x { [a_x, a_y] } else { [a_y, a_x] };
        let b_order = if b_first_x { [b_x, b_y] } else { [b_y, b_x] };
        for (i, (key, dir)) in a_order.into_iter().enumerate() {
            a_out.extend(a.link_up(B, key, dir, run(2), false, secs(t, i as u64 * 2)));
        }
        for (i, (key, dir)) in b_order.into_iter().enumerate() {
            b_out.extend(b.link_up(A, key, dir, run(1), false, secs(t, i as u64 * 3)));
        }
        // Both keep X, the link dialed by the smaller ID.
        assert_eq!(a.chosen(&B), Some(a_dial));
        assert_eq!(b.chosen(&A), Some(b_acc));
        assert_eq!(closes(&a_out), [(a_acc, CloseAs::GoingAway)]);
        assert_eq!(
            closes(&b_out),
            [(b_dial, CloseAs::GoingAway)],
            "B drops its own dialing handle"
        );
        assert_eq!(a.warning(&B), None);
        // When X ends, B starts a new dialing handle; A's own handle redials by itself.
        assert!(a.link_down(a_dial, secs(t, 60)).is_empty());
        let readd = b.link_down(b_acc, secs(t, 60));
        assert!(added(&readd).is_some_and(|key| key != b_dial), "{readd:?}");
    }
}

#[test]
fn a_later_second_link_of_the_same_run_replaces_the_older_one() {
    let t = Instant::now();
    let (mut book, dial) = book(B, A);
    let acc = book.accepted_key();
    book.link_up(A, dial, Dir::Dialed, run(1), false, t);
    let out = book.link_up(A, acc, Dir::Accepted, run(1), false, secs(t, 6));
    assert_eq!(book.chosen(&A), Some(acc), "the newer one");
    assert_eq!(closes(&out), [(dial, CloseAs::GoingAway)]);
    assert_eq!(book.warning(&A), None, "one replacement is no suspicion");
}

#[test]
fn replacements_across_directions_are_ordinary_reconnects_and_raise_no_suspicion() {
    let t = Instant::now();
    // Each round: our dialing handle (mid-attempt when the other side's link came) connects late and replaces the accepted link —
    //   an outage, a sleep and wake, a restart of either side, three times in ten minutes.
    let (mut book, dial) = book(B, A);
    for round in 0..4u64 {
        let base = round * 120;
        let acc = book.accepted_key();
        book.link_up(A, acc, Dir::Accepted, run(1), false, secs(t, base));
        let out = book.link_up(A, dial, Dir::Dialed, run(1), false, secs(t, base + 6));
        assert_eq!(book.chosen(&A), Some(dial), "round {round}: the newer link");
        assert_eq!(closes(&out), [(acc, CloseAs::GoingAway)], "round {round}");
        assert!(sends(&out).is_empty(), "round {round}: no Duplicate sent");
        assert!(book.link_down(dial, secs(t, base + 60)).is_empty());
    }
    assert_eq!(book.warning(&A), None);
}

#[test]
fn a_dialing_handle_between_attempts_is_closed_when_an_accepted_link_comes_up() {
    let t = Instant::now();
    let (mut book, dial) = book(B, A);
    let acc = book.accepted_key();
    // The handle is backing off (the runtime reads it from the handle) — closed at once, so it cannot connect late and churn.
    let out = book.link_up(A, acc, Dir::Accepted, run(1), true, t);
    assert_eq!(closes(&out), [(dial, CloseAs::GoingAway)]);
    assert_eq!(book.chosen(&A), Some(acc));
    // When the accepted link ends a new dialing handle starts.
    let fresh = added(&book.link_down(acc, secs(t, 1))).expect("a new dialing handle");
    assert_ne!(fresh, dial);
}

#[test]
fn the_duplicate_evidence_survives_an_endpoint_restart() {
    let t = Instant::now();
    let (mut book, _) = book(B, A);
    let old = book.accepted_key();
    let new = book.accepted_key();
    book.link_up(A, old, Dir::Accepted, run(1), false, t);
    let nonce = probe_nonce(&book.link_up(A, new, Dir::Accepted, run(9), false, secs(t, 1)));
    book.probe_ack(old, nonce, secs(t, 2));
    assert_eq!(
        book.admit(&A, run(9), secs(t, 3)),
        Err(DuplicateKind::Duplicate)
    );

    // The endpoint restarts (e.g. new network settings): links and dialing reset, evidence kept.
    let out = book.restart(B);
    let fresh = added(&out).expect("a new dialing handle per tracked PC");
    assert!(fresh > new, "keys keep counting");
    assert_eq!(book.live_links(&A), 0);
    assert_eq!(book.chosen(&A), None);
    assert_eq!(book.online(&A), Online::Offline);
    assert_eq!(
        book.warning(&A),
        Some(Warning {
            kind: DuplicateKind::Duplicate,
            reported: false
        })
    );
    assert_eq!(
        book.admit(&A, run(9), secs(t, 60)),
        Err(DuplicateKind::Duplicate)
    );
    assert!(book.ensure(A).is_empty(), "still tracked");
}

#[test]
fn the_same_run_replacing_its_link_three_times_in_ten_minutes_is_a_suspected_duplicate() {
    let t = Instant::now();
    let (mut book, _) = book(B, A);
    let first = book.accepted_key();
    book.link_up(A, first, Dir::Accepted, run(1), false, t);
    let mut last = Vec::new();
    for i in 1..=3u64 {
        let key = book.accepted_key();
        last = book.link_up(A, key, Dir::Accepted, run(1), false, secs(t, i * 60));
        if i < 3 {
            assert_eq!(book.warning(&A), None);
        }
    }
    assert_eq!(
        book.warning(&A),
        Some(Warning {
            kind: DuplicateKind::Suspected,
            reported: false
        })
    );
    let chosen = book.chosen(&A).unwrap();
    assert!(sends(&last).contains(&(
        chosen,
        PeerMsg::Duplicate {
            kind: DuplicateKind::Suspected
        }
    )));
    assert_eq!(
        book.online(&A),
        Online::Online,
        "links are not refused for it"
    );
}

#[test]
fn replacements_spread_over_more_than_ten_minutes_raise_nothing() {
    let t = Instant::now();
    let (mut book, _) = book(B, A);
    let first = book.accepted_key();
    book.link_up(A, first, Dir::Accepted, run(1), false, t);
    for i in 1..=3u64 {
        let key = book.accepted_key();
        book.link_up(A, key, Dir::Accepted, run(1), false, secs(t, i * 6 * 60));
    }
    assert_eq!(book.warning(&A), None);
}

#[test]
fn two_runs_both_answering_the_probe_refuse_the_newer_link_and_warn_both() {
    let t = Instant::now();
    let (mut book, _) = book(B, A);
    let old = book.accepted_key();
    let new = book.accepted_key();
    book.link_up(A, old, Dir::Accepted, run(1), false, t);
    let asked = book.link_up(A, new, Dir::Accepted, run(9), false, secs(t, 1));
    assert!(
        closes(&asked).is_empty(),
        "nothing is decided before the answer"
    );
    assert_eq!(book.chosen(&A), Some(old));
    let nonce = probe_nonce(&asked);
    assert_eq!(sends(&asked), [(old, PeerMsg::Probe { nonce })]);

    let out = book.probe_ack(old, nonce, secs(t, 5));
    let duplicate = PeerMsg::Duplicate {
        kind: DuplicateKind::Duplicate,
    };
    assert_eq!(closes(&out), [(new, CloseAs::Refused)]);
    assert!(sends(&out).contains(&(new, duplicate.clone())));
    assert!(sends(&out).contains(&(old, duplicate)));
    assert_eq!(book.chosen(&A), Some(old));
    assert_eq!(
        book.warning(&A),
        Some(Warning {
            kind: DuplicateKind::Duplicate,
            reported: false
        })
    );
    // The refused run is refused at its next handshake.
    assert_eq!(
        book.admit(&A, run(9), secs(t, 30)),
        Err(DuplicateKind::Duplicate)
    );
    assert_eq!(book.admit(&A, run(1), secs(t, 30)), Ok(()));
}

#[test]
fn an_older_link_that_does_not_answer_was_stale_and_the_newer_run_takes_over() {
    let t = Instant::now();
    let (mut book, _) = book(B, A);
    let old = book.accepted_key();
    let new = book.accepted_key();
    book.link_up(A, old, Dir::Accepted, run(1), false, t);
    let asked = book.link_up(A, new, Dir::Accepted, run(2), false, secs(t, 1));
    assert_eq!(
        book.next_deadline(),
        Some(secs(t, 11) + Duration::from_millis(1))
    );
    assert!(book.tick(secs(t, 10)).is_empty(), "still within the window");
    let out = book.tick(secs(t, 12));
    assert_eq!(closes(&out), [(old, CloseAs::GoingAway)]);
    assert_eq!(book.chosen(&A), Some(new));
    assert_eq!(book.warning(&A), None, "a plain restart");
    // A late answer from the old link proves nothing now.
    assert!(book
        .probe_ack(old, probe_nonce(&asked), secs(t, 13))
        .is_empty());
    assert_eq!(book.warning(&A), None);
    // The replaced run coming back within five minutes is a duplicate; later it is not.
    assert_eq!(
        book.admit(&A, run(1), secs(t, 4 * 60)),
        Err(DuplicateKind::Duplicate)
    );
    let (mut later, _) = self::book(B, A);
    later.link_up(A, 50, Dir::Accepted, run(1), false, t);
    later.link_down(50, secs(t, 1));
    later.link_up(A, 51, Dir::Accepted, run(2), false, secs(t, 2));
    assert_eq!(later.admit(&A, run(1), secs(t, 6 * 60)), Ok(()));
}

#[test]
fn a_wrong_nonce_a_late_answer_or_an_answer_on_the_other_link_proves_nothing() {
    let t = Instant::now();
    for case in 0..3 {
        let (mut book, _) = book(B, A);
        let old = book.accepted_key();
        let new = book.accepted_key();
        book.link_up(A, old, Dir::Accepted, run(1), false, t);
        let nonce = probe_nonce(&book.link_up(A, new, Dir::Accepted, run(2), false, secs(t, 1)));
        let out = match case {
            0 => book.probe_ack(old, nonce.wrapping_add(1), secs(t, 2)),
            1 => book.probe_ack(old, nonce, secs(t, 12)),
            _ => book.probe_ack(new, nonce, secs(t, 2)),
        };
        assert!(out.is_empty(), "case {case}: {out:?}");
        assert_eq!(book.warning(&A), None, "case {case}");
    }
}

#[test]
fn the_older_link_ending_before_it_answers_hands_over_to_the_newer_run() {
    let t = Instant::now();
    let (mut book, _) = book(B, A);
    let old = book.accepted_key();
    let new = book.accepted_key();
    book.link_up(A, old, Dir::Accepted, run(1), false, t);
    book.link_up(A, new, Dir::Accepted, run(2), false, secs(t, 1));
    book.link_down(old, secs(t, 3));
    assert_eq!(book.chosen(&A), Some(new));
    assert_eq!(book.next_deadline(), None, "the probe is over");
    assert_eq!(book.warning(&A), None);
}

#[test]
fn a_plain_restart_raises_nothing_but_the_old_run_coming_back_is_a_duplicate() {
    let t = Instant::now();
    let (mut book, dial) = book(B, A);
    book.link_up(A, dial, Dir::Dialed, run(1), false, t);
    book.link_down(dial, secs(t, 1));
    assert!(book.admit(&A, run(2), secs(t, 2)).is_ok());
    assert!(closes(&book.link_up(A, dial, Dir::Dialed, run(2), false, secs(t, 2))).is_empty());
    assert_eq!(book.warning(&A), None);
    // A late frame of run 1 is not a handshake; run 1 completing a handshake again is.
    assert_eq!(
        book.admit(&A, run(1), secs(t, 60)),
        Err(DuplicateKind::Duplicate)
    );
    assert_eq!(
        book.warning(&A).map(|w| w.kind),
        Some(DuplicateKind::Duplicate)
    );
}

#[test]
fn a_not_paired_refusal_is_retried_after_ten_minutes_and_on_a_re_arm() {
    let t = Instant::now();
    let (mut book, dial) = book(A, B);
    assert!(book.dial_gave_up(dial, Refusal::NotPaired, t).is_empty());
    assert_eq!(book.online(&B), Online::Refused);
    assert_eq!(book.next_deadline(), Some(t + NOT_PAIRED_RETRY));
    assert!(book.tick(secs(t, 9 * 60)).is_empty());
    assert_eq!(
        book.tick(t + NOT_PAIRED_RETRY),
        [Action::Redial { key: dial }]
    );
    assert_eq!(book.online(&B), Online::Offline);

    book.dial_gave_up(dial, Refusal::NotPaired, t);
    assert_eq!(book.rearm_all(), [Action::Redial { key: dial }]);
    assert!(book.rearm_all().is_empty(), "only a handle that gave up");
}

#[test]
fn a_version_mismatch_or_duplicate_refusal_waits_for_a_re_arm_only() {
    let t = Instant::now();
    for (why, online) in [
        (Refusal::VersionMismatch, Online::VersionMismatch),
        (
            Refusal::Duplicate(DuplicateKind::Duplicate),
            Online::DuplicateIdentity,
        ),
    ] {
        let (mut book, dial) = book(A, B);
        book.dial_gave_up(dial, why, t);
        assert_eq!(book.online(&B), online);
        assert_eq!(book.next_deadline(), None);
        assert!(book.tick(secs(t, 3600)).is_empty());
        assert_eq!(book.rearm_all(), [Action::Redial { key: dial }]);
    }
}

#[test]
fn the_dialing_handle_is_closed_while_an_accepted_link_is_live_and_restarted_when_it_ends() {
    let t = Instant::now();
    // A handle that gave up is closed at once.
    let (mut book, dial) = book(A, B);
    book.dial_gave_up(dial, Refusal::NotPaired, t);
    let acc = book.accepted_key();
    let out = book.link_up(B, acc, Dir::Accepted, run(2), false, secs(t, 1));
    assert_eq!(closes(&out), [(dial, CloseAs::GoingAway)]);
    assert_eq!(book.online(&B), Online::Online);
    // A re-arm meanwhile has nothing to redial — the new handle comes when the link ends.
    assert!(book.rearm_all().is_empty());
    let out = book.link_down(acc, secs(t, 2));
    let fresh = added(&out).expect("a new dialing handle");
    assert_ne!(fresh, dial);

    // A handle that may be mid-attempt is left alone until it fails.
    let (mut book, dial) = self::book(A, B);
    let acc = book.accepted_key();
    assert!(closes(&book.link_up(B, acc, Dir::Accepted, run(2), false, t)).is_empty());
    assert_eq!(
        closes(&book.dial_failed(dial)),
        [(dial, CloseAs::GoingAway)]
    );
    assert!(book.dial_failed(dial).is_empty());
}

#[test]
fn a_dialing_handle_that_ends_by_itself_is_replaced() {
    let t = Instant::now();
    let (mut book, dial) = book(A, B);
    let out = book.handle_ended(dial, t);
    assert!(added(&out).is_some_and(|key| key != dial));
    // One the book closed is not doubled.
    let (mut book, dial) = self::book(A, B);
    let acc = book.accepted_key();
    book.dial_gave_up(dial, Refusal::NotPaired, t);
    book.link_up(B, acc, Dir::Accepted, run(2), false, t);
    assert!(book.handle_ended(dial, t).is_empty());
}

#[test]
fn a_reported_warning_is_kept_and_a_suspicion_never_lowers_a_duplicate() {
    let (mut book, _) = book(A, B);
    book.duplicate_reported(&B, DuplicateKind::Duplicate);
    book.duplicate_reported(&B, DuplicateKind::Suspected);
    assert_eq!(
        book.warning(&B),
        Some(Warning {
            kind: DuplicateKind::Duplicate,
            reported: true
        })
    );
}

#[test]
fn a_link_of_a_pc_no_longer_tracked_is_refused() {
    let t = Instant::now();
    let (mut book, _) = book(A, B);
    book.forget(&B);
    assert_eq!(
        closes(&book.link_up(B, 77, Dir::Accepted, run(2), false, t)),
        [(77, CloseAs::Refused)]
    );
    assert_eq!(book.admit(&B, run(2), t), Ok(()));
}
