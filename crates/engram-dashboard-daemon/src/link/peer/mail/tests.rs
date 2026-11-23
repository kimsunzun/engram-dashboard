//! The receive queue's order, caps and deadline, and how a request's end maps — pure, no link.

use engram_dashboard_transport::{DisconnectCause, Generation};

use super::*;
use crate::link::peer::wire::{MailFrom, MailRequest, MailRow, MailVerdict, Outcome};

fn mail(id: &str, to: &[&str]) -> Mail {
    Mail {
        id: id.to_string(),
        from: MailFrom {
            agent_id: "a-1".to_string(),
            name: "writer".to_string(),
        },
        rows: to
            .iter()
            .enumerate()
            .map(|(i, to)| MailRow {
                tag: i as u32,
                to: to.to_string(),
            })
            .collect(),
        body: "hi".to_string(),
        reply_to: None,
        request: None,
    }
}

/// ADR-0411: the bound before queueing counts every text field, not only the body — a frame with a small body and a huge name is over.
#[test]
fn the_mail_size_counts_every_text_field() {
    let mut m = mail("m1", &["bob", "carol"]);
    m.reply_to = Some("m0".to_string());
    m.request = Some(MailRequest {
        reply_by_raw: Some("10m".to_string()),
    });
    assert_eq!(mail_bytes(&m), "m1a-1writerhim0bobcarol10m".len());
    m.from.name = "n".repeat(MAX_MAIL_BYTES);
    assert!(mail_bytes(&m) > MAX_MAIL_BYTES);
}

fn ids<T>(jobs: &[Job<T>]) -> Vec<&str> {
    jobs.iter().map(|j| j.mail.id.as_str()).collect()
}

#[test]
fn a_stuck_recipient_does_not_hold_up_mail_to_another_one() {
    let mut queue: Inbound<()> = Inbound::new(0);
    let t = Instant::now();
    queue.push(mail("m1", &["bob"]), t, ()).unwrap();
    queue.push(mail("m2", &["bob"]), t, ()).unwrap();
    queue.push(mail("m3", &["carol"]), t, ()).unwrap();
    let started = queue.start(t);
    assert_eq!(
        ids(&started.run),
        ["m1", "m3"],
        "m2 waits for m1 (same recipient); m3 does not"
    );
    assert!(ids(&queue.start(t).run).is_empty(), "m2 still waits");
    queue.finish(started.run[0].tokens());
    assert_eq!(ids(&queue.start(t).run), ["m2"]);
}

#[test]
fn a_frame_waits_for_an_earlier_waiting_frame_that_shares_a_token() {
    let mut queue: Inbound<()> = Inbound::new(0);
    let t = Instant::now();
    queue.push(mail("m1", &["bob"]), t, ()).unwrap();
    let first = queue.start(t);
    // m2 (bob, carol) waits for m1; m3 (carol) must not overtake m2.
    queue.push(mail("m2", &["bob", "carol"]), t, ()).unwrap();
    queue.push(mail("m3", &["carol"]), t, ()).unwrap();
    assert!(ids(&queue.start(t).run).is_empty());
    queue.finish(first.run[0].tokens());
    let next = queue.start(t);
    assert_eq!(ids(&next.run), ["m2"]);
    queue.finish(next.run[0].tokens());
    assert_eq!(ids(&queue.start(t).run), ["m3"]);
}

#[test]
fn at_most_four_frames_of_a_pc_run_and_sixty_four_wait_or_run() {
    let mut queue: Inbound<()> = Inbound::new(0);
    let t = Instant::now();
    for n in 0..MAX_QUEUED {
        queue
            .push(mail(&format!("m{n}"), &[&format!("agent{n}")]), t, ())
            .unwrap();
    }
    let started = queue.start(t);
    assert_eq!(started.run.len(), MAX_RUNNING);
    let refused = queue.push(mail("over", &["x"]), t, ());
    assert!(
        refused.is_err(),
        "the queue is full — refused whole (REMOTE_LIMIT)"
    );
    queue.finish(started.run[0].tokens());
    assert_eq!(queue.start(t).run.len(), 1, "one ends, one more runs");
    assert!(queue.push(mail("again", &["x"]), t, ()).is_ok());
}

/// ADR-0404 d5: a frame that waited past the 120 s transport deadline (since it arrived here) is dropped, not run late.
#[test]
fn a_frame_past_its_deadline_since_arrival_is_dropped() {
    let mut queue: Inbound<()> = Inbound::new(0);
    let t = Instant::now();
    queue.push(mail("old", &["bob"]), t, ()).unwrap();
    queue
        .push(mail("fresh", &["carol"]), t + Duration::from_secs(60), ())
        .unwrap();
    let started = queue.start(t + MAIL_DEADLINE + Duration::from_secs(1));
    assert_eq!(ids(&started.expired), ["old"]);
    assert_eq!(ids(&started.run), ["fresh"]);
    let mut queue: Inbound<()> = Inbound::new(0);
    queue.push(mail("edge", &["bob"]), t, ()).unwrap();
    assert_eq!(
        ids(&queue.start(t + MAIL_DEADLINE).run),
        ["edge"],
        "at the deadline it still runs"
    );
}

#[test]
fn a_whole_frame_refusal_answers_every_tag() {
    let refusal = Refusal {
        code: "REMOTE_LIMIT",
        reason: "receiver busy".to_string(),
    };
    let rows = refuse_all(&mail("m", &["a", "b"]), &refusal);
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().enumerate().all(|(i, r)| r.tag == i as u32
        && r.outcome == Outcome::Refused
        && r.code.as_deref() == Some("REMOTE_LIMIT")));
}

/// The sender-side result table (ADR-0393) for what the link reports.
#[test]
fn a_request_end_maps_to_one_mail_end() {
    let verdict = || {
        Ok(PeerMsg::MailVerdict(MailVerdict {
            id: "m".to_string(),
            rows: Vec::new(),
        }))
    };
    let gone = |sent| {
        Err(RequestError::Disconnected {
            generation: Generation(1),
            cause: DisconnectCause::PeerClosed,
            sent,
        })
    };
    assert_eq!(
        MailEnd::of(verdict(), || true),
        MailEnd::Verdict(Vec::new())
    );
    assert_eq!(
        MailEnd::of(verdict(), || false),
        MailEnd::Unconfirmed,
        "a verdict from a PC no longer trusted is not applied"
    );
    assert_eq!(MailEnd::of(gone(false), || true), MailEnd::Offline);
    assert_eq!(MailEnd::of(gone(true), || true), MailEnd::Unconfirmed);
    assert_eq!(
        MailEnd::of(
            Err(RequestError::TimedOut {
                after: MAIL_DEADLINE
            }),
            || true
        ),
        MailEnd::Unconfirmed
    );
    assert_eq!(
        MailEnd::of(Err(RequestError::QueueFull), || true),
        MailEnd::Limit
    );
}
