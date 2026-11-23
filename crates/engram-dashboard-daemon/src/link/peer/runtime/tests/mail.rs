//! Mail and the agent directory between two PCs in one process — the link side only: the daemon's mail host is a fake [`MailInbox`]
//!   (what the kernel does with the mail is pinned in the mail host's own tests).

use std::collections::HashSet;
use std::sync::{Condvar, Mutex as StdMutex};
use std::time::Instant;

use tokio::sync::mpsc as tmpsc;

use super::*;
use crate::link::peer::directory::DirectoryBell;
use crate::link::peer::mail::{
    MailEnd, MailInbox, Refusal, MAX_MAIL_ROWS, MAX_OVERDUE_RUNNING, MAX_OVERDUE_WAITING,
};
use crate::link::peer::wire::{
    AgentState, DirAgent, DirSnapshot, Mail, MailFrom, MailRow, Outcome, VerdictRow,
};

/// A gate the fake inbox waits at while it is closed.
#[derive(Default)]
struct Gate {
    closed: StdMutex<bool>,
    changed: Condvar,
}

impl Gate {
    fn set(&self, closed: bool) {
        *self.closed.lock().unwrap() = closed;
        self.changed.notify_all();
    }

    fn pass(&self) {
        let mut closed = self.closed.lock().unwrap();
        while *closed {
            closed = self.changed.wait(closed).unwrap();
        }
    }
}

/// The daemon's mail host, faked: delivers every row to one recipient id, records what reached it.
#[derive(Default)]
struct FakeInbox {
    directory: StdMutex<Vec<DirAgent>>,
    /// Ids of "this PC's" agents — a directory row with one is dropped.
    local_ids: StdMutex<HashSet<String>>,
    /// `local_ids` as of the last directory read, and its version — as the daemon's host counts it.
    seen_ids: StdMutex<(HashSet<String>, u64)>,
    gate: Gate,
    /// (from, this PC's name for it, mail id) per mail that reached the host.
    mails: StdMutex<Vec<(PeerId, String, String)>>,
    overdue: StdMutex<Vec<(PeerId, String)>>,
    /// Overdue reports wait here after they are recorded.
    overdue_gate: Gate,
}

/// A mail with this id makes the fake host panic (a broken kernel lock).
const PANICS: &str = "m-panic";

const RECIPIENT: &str = "6f1c2a90-0000-4000-8000-0000000000b0";

impl MailInbox for FakeInbox {
    fn mail(
        &self,
        from: PeerId,
        pc_name: &str,
        mail: &Mail,
        _arrived: Instant,
    ) -> Result<Vec<VerdictRow>, Refusal> {
        self.mails
            .lock()
            .unwrap()
            .push((from, pc_name.to_string(), mail.id.clone()));
        self.gate.pass();
        assert_ne!(mail.id, PANICS, "the fake host fails on purpose");
        Ok(mail
            .rows
            .iter()
            .map(|row| VerdictRow {
                tag: row.tag,
                outcome: Outcome::Delivered,
                recipient_id: Some(RECIPIENT.to_string()),
                code: None,
                reason: None,
            })
            .collect())
    }

    fn reply_overdue(&self, from: PeerId, request_id: &str) {
        self.overdue
            .lock()
            .unwrap()
            .push((from, request_id.to_string()));
        self.overdue_gate.pass();
    }

    fn mail_id_ok(&self, id: &str) -> bool {
        !id.is_empty() && id.len() <= 64
    }

    fn directory(&self) -> Vec<DirAgent> {
        let now = self.local_ids.lock().unwrap().clone();
        let mut seen = self.seen_ids.lock().unwrap();
        if seen.0 != now {
            *seen = (now, seen.1 + 1);
        }
        self.directory.lock().unwrap().clone()
    }

    fn local_ids_version(&self) -> u64 {
        self.seen_ids.lock().unwrap().1
    }

    fn row_problems(&self, rows: &[&DirAgent]) -> Vec<Option<&'static str>> {
        let local = self.local_ids.lock().unwrap();
        rows.iter()
            .map(|row| {
                if local.contains(&row.id) {
                    Some("a local agent id")
                } else {
                    row.name.is_empty().then_some("a bad name")
                }
            })
            .collect()
    }
}

struct Hosted {
    link: PeerLink,
    id: PeerId,
    addr: SocketAddr,
    inbox: Arc<FakeInbox>,
    bell: DirectoryBell,
}

async fn hosted(dir: &TempDir, directory: Vec<DirAgent>) -> Hosted {
    let inbox = Arc::new(FakeInbox::default());
    *inbox.directory.lock().unwrap() = directory;
    let bell = DirectoryBell::new();
    let host = Host {
        mail: inbox.clone(),
        bell: bell.clone(),
    };
    let (link, id, addr) = test_pc(dir, Some(host)).await;
    Hosted {
        link,
        id,
        addr,
        inbox,
        bell,
    }
}

/// A and B linked (B dials A).
async fn hosted_pair(
    da: &TempDir,
    db: &TempDir,
    a_dir: Vec<DirAgent>,
    b_dir: Vec<DirAgent>,
) -> (Hosted, Hosted) {
    let a = hosted(da, a_dir).await;
    let b = hosted(db, b_dir).await;
    a.link.add(entry(b.id, "bee", None)).unwrap();
    b.link.add(entry(a.id, "ay", Some(a.addr))).unwrap();
    wait_online(&a.link, &b.id).await;
    wait_online(&b.link, &a.id).await;
    (a, b)
}

fn agent(id: &str, name: &str, state: AgentState) -> DirAgent {
    DirAgent {
        id: id.to_string(),
        name: name.to_string(),
        state,
    }
}

fn mail(id: &str, to: &[&str], body: &str) -> Mail {
    Mail {
        id: id.to_string(),
        from: MailFrom {
            agent_id: "6f1c2a90-0000-4000-8000-0000000000a0".to_string(),
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
        body: body.to_string(),
        reply_to: None,
        request: None,
    }
}

/// Sends a mail and gives back where its one end arrives.
fn send(link: &PeerLink, to: PeerId, mail: Mail) -> tmpsc::UnboundedReceiver<MailEnd> {
    let (tx, rx) = tmpsc::unbounded_channel();
    link.send_mail(
        to,
        mail,
        Box::new(move |end| {
            let _ = tx.send(end);
        }),
    );
    rx
}

async fn end_of(mut ends: tmpsc::UnboundedReceiver<MailEnd>) -> MailEnd {
    let end = tokio::time::timeout(PATIENCE, ends.recv())
        .await
        .expect("the mail ended within the patience")
        .expect("the callback ran");
    assert!(
        ends.recv().await.is_none(),
        "exactly one end per mail — the callback is gone after it"
    );
    end
}

fn row_of(link: &PeerLink, id: &PeerId) -> PeerRow {
    link.rows().into_iter().find(|r| r.peer.id == *id).unwrap()
}

async fn wait_for(what: &str, check: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while !check() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "{what} did not happen within {PATIENCE:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_mail_reaches_the_other_pc_and_its_verdict_comes_back() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, b) = hosted_pair(&da, &db, Vec::new(), Vec::new()).await;
    let end = end_of(send(&a.link, b.id, mail("m-1", &["bob", "carol"], "hi"))).await;
    let MailEnd::Verdict(rows) = end else {
        panic!("{end:?}")
    };
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().enumerate().all(|(i, r)| r.tag == i as u32
        && r.outcome == Outcome::Delivered
        && r.recipient_id.as_deref() == Some(RECIPIENT)));
    assert_eq!(
        *b.inbox.mails.lock().unwrap(),
        [(a.id, "ay".to_string(), "m-1".to_string())],
        "the host got it from the link's PC, named with B's name for it"
    );
    a.link.stop().await;
    b.link.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_mail_to_a_pc_without_a_live_link_is_not_sent() {
    let da = TempDir::new();
    let a = hosted(&da, Vec::new()).await;
    a.link.add(bystander(70)).unwrap();
    wait_until(&a.link, "endpoint up", PATIENCE, is_up).await;
    let end = end_of(send(
        &a.link,
        PeerId::from_bytes([70; 32]),
        mail("m-1", &["bob"], "hi"),
    ))
    .await;
    assert_eq!(end, MailEnd::Offline);
    a.link.stop().await;
}

/// ADR-0390: the sender checks the encoded size — its own oversized frame never closes the link.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_oversized_mail_is_not_sent_and_the_link_stays_up() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, b) = hosted_pair(&da, &db, Vec::new(), Vec::new()).await;
    // Control characters are written `\u00XX` — six bytes each on the link: over the 4 MiB frame cap.
    let body = "\u{1}".repeat(700 * 1024);
    let end = end_of(send(&a.link, b.id, mail("m-big", &["bob"], &body))).await;
    assert_eq!(end, MailEnd::TooLarge);
    assert!(b.inbox.mails.lock().unwrap().is_empty());
    let end = end_of(send(&a.link, b.id, mail("m-small", &["bob"], "hi"))).await;
    assert!(matches!(end, MailEnd::Verdict(_)), "{end:?}");
    assert_eq!(live_links(&a.link, &b.id), 1, "the link stayed");
    a.link.stop().await;
    b.link.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mail_over_the_in_flight_cap_is_not_sent_until_verdicts_free_it() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, b) = hosted_pair(&da, &db, Vec::new(), Vec::new()).await;
    a.link.set_in_flight_cap(2);
    b.inbox.gate.set(true);
    let first = send(&a.link, b.id, mail("m-1", &["bob"], "1"));
    let second = send(&a.link, b.id, mail("m-2", &["carol"], "2"));
    wait_for("both mails reached B", || {
        b.inbox.mails.lock().unwrap().len() == 2
    })
    .await;
    assert_eq!(
        end_of(send(&a.link, b.id, mail("m-3", &["dan"], "3"))).await,
        MailEnd::Limit
    );
    b.inbox.gate.set(false);
    assert!(matches!(end_of(first).await, MailEnd::Verdict(_)));
    assert!(matches!(end_of(second).await, MailEnd::Verdict(_)));
    let after = end_of(send(&a.link, b.id, mail("m-4", &["dan"], "4"))).await;
    assert!(
        matches!(after, MailEnd::Verdict(_)),
        "the verdicts freed the cap: {after:?}"
    );
    a.link.stop().await;
    b.link.stop().await;
}

/// ADR-0393: the link dropped after the mail was handed to it — it may have arrived.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_link_lost_after_the_mail_was_sent_leaves_it_unconfirmed() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, b) = hosted_pair(&da, &db, Vec::new(), Vec::new()).await;
    b.inbox.gate.set(true);
    let ends = send(&a.link, b.id, mail("m-1", &["bob"], "hi"));
    wait_for("the mail reached B", || {
        !b.inbox.mails.lock().unwrap().is_empty()
    })
    .await;
    b.link.stop().await;
    assert_eq!(end_of(ends).await, MailEnd::Unconfirmed);
    b.inbox.gate.set(false);
    a.link.stop().await;
}

/// ADR-0390 d4: a verdict is matched in the transport, past the per-frame trust check — one that came back just before the PC was
///   removed, and waited for the blocking thread that hands it on, is not applied (trust is judged on that thread).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_verdict_that_arrives_as_the_pc_is_removed_is_not_applied() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, b) = hosted_pair(&da, &db, Vec::new(), Vec::new()).await;
    let (reached, got_answer) = oneshot::channel();
    let (release, hold) = oneshot::channel();
    a.link.hold_next_verdict(reached, hold);
    let ends = send(&a.link, b.id, mail("m-1", &["bob"], "hi"));
    got_answer.await.expect("the verdict came back");
    a.link.remove(&b.id).unwrap();
    release.send(()).unwrap();
    assert_eq!(end_of(ends).await, MailEnd::Unconfirmed);
    a.link.stop().await;
    b.link.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_overdue_report_reaches_the_requesters_pc_both_ways() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, b) = hosted_pair(&da, &db, Vec::new(), Vec::new()).await;
    a.link.send_reply_overdue(b.id, "m-a");
    b.link.send_reply_overdue(a.id, "m-b");
    wait_for("both reports arrived", || {
        b.inbox.overdue.lock().unwrap().len() == 1 && a.inbox.overdue.lock().unwrap().len() == 1
    })
    .await;
    assert_eq!(
        b.inbox.overdue.lock().unwrap()[0],
        (a.id, "m-a".to_string())
    );
    assert_eq!(
        a.inbox.overdue.lock().unwrap()[0],
        (b.id, "m-b".to_string())
    );
    a.link.stop().await;
    b.link.stop().await;
}

/// R16: each PC gets the other's directory when they link, a change as it happens, and rows this PC refuses are left out of its copy.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn each_pc_holds_the_others_directory_and_follows_its_changes() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let alice = agent("id-alice", "alice", AgentState::Live);
    let bob = agent("id-bob", "bob", AgentState::Sleeping);
    let mine = agent("id-mine", "copied", AgentState::Live);
    let a = hosted(&da, vec![alice.clone(), mine.clone()]).await;
    let b = hosted(&db, vec![bob.clone()]).await;
    b.inbox.local_ids.lock().unwrap().insert(mine.id.clone());
    a.link.add(entry(b.id, "bee", None)).unwrap();
    b.link.add(entry(a.id, "ay", Some(a.addr))).unwrap();

    wait_for("B holds A's directory", || {
        row_of(&b.link, &a.id).agents == Some(vec![alice.clone()])
    })
    .await;
    wait_for("A holds B's directory", || {
        row_of(&a.link, &b.id).agents == Some(vec![bob.clone()])
    })
    .await;

    // A change on A: alice goes to sleep (live ↔ asleep) and carol is created.
    let alice_asleep = agent("id-alice", "alice", AgentState::Sleeping);
    let carol = agent("id-carol", "carol", AgentState::Live);
    *a.inbox.directory.lock().unwrap() = vec![alice_asleep.clone(), carol.clone(), mine];
    a.bell.ring();
    wait_for("B follows A's change", || {
        row_of(&b.link, &a.id).agents == Some(vec![alice_asleep.clone(), carol.clone()])
    })
    .await;

    // B goes away: A keeps B's copy (shown disconnected) — the names still resolve, to a PC that is offline.
    b.link.stop().await;
    wait_until(&a.link, "B offline", PATIENCE, |s| {
        online_of(s, &b.id) == Some(Online::Offline)
    })
    .await;
    assert_eq!(row_of(&a.link, &b.id).agents, Some(vec![bob]));

    // Removing B drops its copy: listed again, it is unknown until it links.
    a.link.remove(&b.id).unwrap();
    a.link.add(entry(b.id, "bee", None)).unwrap();
    assert_eq!(row_of(&a.link, &b.id).agents, None);
    a.link.stop().await;
}

fn count(n: &std::sync::atomic::AtomicU64) -> u64 {
    n.load(std::sync::atomic::Ordering::Acquire)
}

/// A coalesced burst of rings is one recompute; a ring with no change sends no directory frame at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rings_without_a_change_send_no_directory_frame() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let alice = agent("id-alice", "alice", AgentState::Live);
    let (a, b) = hosted_pair(&da, &db, vec![alice.clone()], Vec::new()).await;
    wait_for("B holds A's directory", || {
        row_of(&b.link, &a.id).agents == Some(vec![alice.clone()])
    })
    .await;
    let sent = count(&a.link.core.dir_sent);
    let publishes = count(&a.link.core.publishes);
    for _ in 0..5 {
        a.bell.ring();
    }
    wait_for("A recomputed its directory", || {
        count(&a.link.core.publishes) > publishes
    })
    .await;
    assert_eq!(count(&a.link.core.dir_sent), sent, "no change, no frame");
    a.link.stop().await;
    b.link.stop().await;
}

/// B's chosen link to A — to write raw PeerWire frames as a broken or hostile B would.
fn raw(from: &Hosted, to: &PeerId) -> Peer<PeerWire> {
    lock(&from.link.core.routes)
        .get(to)
        .expect("a route to that PC")
        .peer
        .clone()
}

/// Waits until A's drain has handled every frame B sent before this call — a mail B sends after them reaches A's host only then.
async fn flushed(b: &Hosted, a: &Hosted) {
    let id = format!("m-flush-{}", a.inbox.mails.lock().unwrap().len());
    let _end = send(&b.link, a.id, mail(&id, &["flush"], "-"));
    wait_for("A handled what came before", || {
        a.inbox
            .mails
            .lock()
            .unwrap()
            .iter()
            .any(|(_, _, m)| *m == id)
    })
    .await;
}

/// ADR-0393: a burst of overdue reports (one per overdue request, sent once by the other PC's sweep) is kept and handed to the host in
///   full, at most 4 at a time; the same id waiting twice is one report; a malformed id never reaches the host.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_overdue_burst_is_handled_in_full_and_ids_are_checked() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, b) = hosted_pair(&da, &db, Vec::new(), Vec::new()).await;
    let to_a = raw(&b, &a.id);
    a.inbox.overdue_gate.set(true);
    to_a.notify(PeerMsg::ReplyOverdue {
        request_id: "x".repeat(100),
    })
    .unwrap();
    for n in 0..50 {
        to_a.notify(PeerMsg::ReplyOverdue {
            request_id: format!("m-{n}"),
        })
        .unwrap();
    }
    // A repeat of one still waiting.
    to_a.notify(PeerMsg::ReplyOverdue {
        request_id: "m-49".to_string(),
    })
    .unwrap();
    flushed(&b, &a).await;
    a.inbox.overdue_gate.set(false);
    wait_for("all 50 reports reached the host", || {
        a.inbox.overdue.lock().unwrap().len() >= 50
    })
    .await;
    flushed(&b, &a).await;
    let got = a.inbox.overdue.lock().unwrap().clone();
    assert_eq!(got.len(), 50, "each report once, none dropped");
    assert!(
        got.iter().all(|(_, id)| id.len() < 100),
        "the malformed id never reached the host"
    );
    a.link.stop().await;
    b.link.stop().await;
}

/// A flood of overdue reports is bounded: 4 handed on, 256 waiting, the rest dropped.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_overdue_flood_is_bounded() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, b) = hosted_pair(&da, &db, Vec::new(), Vec::new()).await;
    let to_a = raw(&b, &a.id);
    a.inbox.overdue_gate.set(true);
    for n in 0..400 {
        to_a.notify(PeerMsg::ReplyOverdue {
            request_id: format!("m-{n}"),
        })
        .unwrap();
    }
    flushed(&b, &a).await;
    let bound = MAX_OVERDUE_RUNNING + MAX_OVERDUE_WAITING;
    a.inbox.overdue_gate.set(false);
    wait_for("the kept reports reached the host", || {
        a.inbox.overdue.lock().unwrap().len() >= bound
    })
    .await;
    flushed(&b, &a).await;
    assert_eq!(a.inbox.overdue.lock().unwrap().len(), bound);
    a.link.stop().await;
    b.link.stop().await;
}

/// ADR-0390 d4: between `peer.remove` and the actor handling it, nothing more goes to the removed PC — no snapshot from a re-read
///   route, no directory change, no overdue report.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nothing_goes_to_a_pc_once_it_is_removed() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let alice = agent("id-alice", "alice", AgentState::Live);
    let (a, b) = hosted_pair(&da, &db, vec![alice.clone()], Vec::new()).await;
    wait_for("B holds A's directory", || {
        row_of(&b.link, &a.id).agents == Some(vec![alice.clone()])
    })
    .await;
    // Stall A's actor, so the removal is published but not yet carried out.
    let (release, stall) = oneshot::channel();
    a.link.core.tx.send(Msg::Stall(stall)).ok().unwrap();
    let sent = count(&a.link.core.dir_sent);
    a.link.core.revoke(&b.id);
    a.link.send_reply_overdue(b.id, "m-1");
    *a.inbox.directory.lock().unwrap() = vec![agent("id-carol", "carol", AgentState::Live)];
    let publishes = count(&a.link.core.publishes);
    a.bell.ring();
    wait_for("A recomputed its directory", || {
        count(&a.link.core.publishes) > publishes
    })
    .await;
    release.send(()).unwrap();
    wait_until(&a.link, "B's links cut", PATIENCE, |_| {
        live_links(&a.link, &b.id) == 0
    })
    .await;
    assert_eq!(
        count(&a.link.core.dir_sent),
        sent,
        "no directory frame went to B"
    );
    assert!(b.inbox.overdue.lock().unwrap().is_empty());
    a.link.stop().await;
    b.link.stop().await;
}

/// ADR-0391: directory frames count only from the PC's chosen link; agent ids are kept in one spelling; a removal between a frame's
///   trust check and its write does not bring the copy back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_the_chosen_link_writes_a_copy_and_a_removed_pc_gets_none_back() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, b) = hosted_pair(&da, &db, Vec::new(), Vec::new()).await;
    wait_for("A holds B's directory", || {
        row_of(&a.link, &b.id).agents.is_some()
    })
    .await;
    let chosen = lock(&a.link.core.book).chosen(&b.id).unwrap();
    let copy = lock(&a.link.core.copies).get(&b.id).cloned().unwrap();
    let reply = Reply {
        peer: raw(&a, &b.id),
        generation: Generation(0),
    };
    let upper = "6F1C2A90-0000-4000-8000-0000000000C0";
    let snapshot = |rev| {
        Routed::Snapshot(DirSnapshot {
            run: copy.run,
            rev,
            agents: vec![agent(upper, "carol", AgentState::Live)],
        })
    };
    on_directory(
        &a.link.core,
        b.id,
        chosen + 1000,
        &reply,
        snapshot(copy.rev + 1),
    );
    assert_eq!(
        lock(&a.link.core.copies).get(&b.id),
        Some(&copy),
        "a frame on another link changed nothing"
    );
    on_directory(&a.link.core, b.id, chosen, &reply, snapshot(copy.rev + 1));
    assert_eq!(
        row_of(&a.link, &b.id).agents.unwrap()[0].id,
        upper.to_lowercase(),
        "one spelling per id"
    );
    a.link.core.revoke(&b.id);
    on_directory(&a.link.core, b.id, chosen, &reply, snapshot(copy.rev + 2));
    assert!(
        lock(&a.link.core.copies).get(&b.id).is_none(),
        "a removed PC's copy does not come back"
    );
    a.link.stop().await;
    b.link.stop().await;
}

/// A broken PC asking for snapshots over and over gets at most one per second per link, not one per request.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resync_requests_are_answered_at_most_once_a_second() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, b) = hosted_pair(
        &da,
        &db,
        vec![agent("id-alice", "alice", AgentState::Live)],
        Vec::new(),
    )
    .await;
    wait_for("B holds A's directory", || {
        row_of(&b.link, &a.id).agents.is_some()
    })
    .await;
    let sent = count(&a.link.core.dir_sent);
    let to_a = raw(&b, &a.id);
    for _ in 0..20 {
        to_a.notify(PeerMsg::DirResync).unwrap();
    }
    wait_for("the first request answered", || {
        count(&a.link.core.dir_sent) > sent
    })
    .await;
    tokio::time::sleep(RESYNC_INTERVAL + Duration::from_millis(500)).await;
    let answered = count(&a.link.core.dir_sent) - sent;
    assert!(
        (1..=2).contains(&answered),
        "{answered} snapshots for 20 requests"
    );
    a.link.stop().await;
    b.link.stop().await;
}

/// ADR-0390: a mail naming more agents than one mail may is refused before it is queued, with a bounded answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_mail_with_too_many_rows_is_refused_before_it_is_queued() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, b) = hosted_pair(&da, &db, Vec::new(), Vec::new()).await;
    let names: Vec<String> = (0..1000).map(|n| format!("agent{n}")).collect();
    let to: Vec<&str> = names.iter().map(String::as_str).collect();
    let end = end_of(send(&a.link, b.id, mail("m-many", &to, "hi"))).await;
    let MailEnd::Verdict(rows) = end else {
        panic!("{end:?}")
    };
    assert_eq!(
        rows.len(),
        MAX_MAIL_ROWS,
        "the answer is bounded; the rest settle unconfirmed"
    );
    assert!(rows
        .iter()
        .all(|r| r.outcome == Outcome::Refused && r.code.as_deref() == Some("REMOTE_REFUSED")));
    assert!(b.inbox.mails.lock().unwrap().is_empty());
    a.link.stop().await;
    b.link.stop().await;
}

/// A host that panics on one mail does not keep that recipient's slot — the next mail to it still runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_host_panic_gives_the_queue_slot_back() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, b) = hosted_pair(&da, &db, Vec::new(), Vec::new()).await;
    let lost = send(&a.link, b.id, mail(PANICS, &["bob"], "hi"));
    let end = end_of(send(&a.link, b.id, mail("m-2", &["bob"], "hi"))).await;
    assert!(matches!(end, MailEnd::Verdict(_)), "{end:?}");
    drop(lost);
    a.link.stop().await;
    b.link.stop().await;
}

/// ADR-0404 d5: a frame that waited past its deadline for a blocking thread is dropped when its job starts — the host never sees it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_frame_that_waited_past_its_deadline_for_a_thread_is_dropped() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, b) = hosted_pair(&da, &db, Vec::new(), Vec::new()).await;
    let Some(arrived) = Instant::now().checked_sub(MAIL_DEADLINE + Duration::from_secs(1)) else {
        eprintln!("skipped: this host's monotonic clock started less than the mail deadline ago");
        return;
    };
    let mut queue: Inbound<Reply> = Inbound::new(77);
    queue
        .push(
            mail("m-late", &["bob"], "hi"),
            arrived,
            Reply {
                peer: raw(&b, &a.id),
                generation: Generation(0),
            },
        )
        .unwrap();
    // Started when it was still in time; run only now.
    let job = queue.start(arrived).run.pop().unwrap();
    let core = b.link.core.clone();
    tokio::task::spawn_blocking(move || run_job(core, a.id, job))
        .await
        .unwrap();
    assert!(b.inbox.mails.lock().unwrap().is_empty());
    a.link.stop().await;
    b.link.stop().await;
}

/// ADR-0390 d4: a verdict that passed its trust check is applied before `peer.remove` returns — the removal waits for it; one that
///   checks after the removal is not applied (`a_verdict_that_arrives_as_the_pc_is_removed_is_not_applied`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_removal_waits_for_a_verdict_being_applied() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, b) = hosted_pair(&da, &db, Vec::new(), Vec::new()).await;
    let (reached, checked) = oneshot::channel();
    let (release, hold) = oneshot::channel();
    a.link.hold_next_commit(reached, hold);
    let ends = send(&a.link, b.id, mail("m-1", &["bob"], "hi"));
    checked.await.expect("the verdict passed its trust check");
    let link = a.link.clone();
    let b_id = b.id;
    let removing = tokio::task::spawn_blocking(move || link.remove(&b_id));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !removing.is_finished(),
        "the removal waits for the verdict being applied"
    );
    release.send(()).unwrap();
    assert!(matches!(end_of(ends).await, MailEnd::Verdict(_)));
    removing.await.unwrap().unwrap();
    a.link.stop().await;
    b.link.stop().await;
}

/// ADR-0391: with no chosen link a directory frame is dropped, and every newly chosen link — the first one too — asks for the snapshot.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_frame_before_any_chosen_link_is_dropped_and_the_first_link_asks_again() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, b) = hosted_pair(&da, &db, Vec::new(), Vec::new()).await;
    // B asked A for its snapshot on B's first chosen link, and A answered on that link.
    let key = lock(&a.link.core.book).chosen(&b.id).unwrap();
    wait_for("A answered B's resync", || {
        lock(&a.link.core.resyncs)
            .links
            .get(&key)
            .is_some_and(|r| r.last.is_some())
    })
    .await;
    // A PC listed but never linked: nothing is chosen, so a frame (from a link key it never had) is dropped.
    let far = PeerId::from_bytes([71; 32]);
    a.link.add(entry(far, "far", None)).unwrap();
    let reply = Reply {
        peer: raw(&a, &b.id),
        generation: Generation(0),
    };
    let frame = Routed::Snapshot(DirSnapshot {
        run: Instance::from_u128(5),
        rev: 1,
        agents: vec![agent("id-x", "x", AgentState::Live)],
    });
    on_directory(&a.link.core, far, key, &reply, frame);
    assert_eq!(row_of(&a.link, &far).agents, None);
    a.link.stop().await;
    b.link.stop().await;
}

/// Rows a copy admitted before an agent of this PC took their id are dropped at the next directory read — the copies are re-checked only
///   when this PC's ids changed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn copy_rows_that_now_name_a_local_agent_are_dropped() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let twin = agent("id-twin", "twin", AgentState::Live);
    let (a, b) = hosted_pair(&da, &db, Vec::new(), vec![twin.clone()]).await;
    wait_for("A holds B's directory", || {
        row_of(&a.link, &b.id).agents == Some(vec![twin.clone()])
    })
    .await;
    // A directory read that leaves this PC's ids as they were does not re-check the copies.
    let rechecks = count(&a.link.core.rechecks);
    let publishes = count(&a.link.core.publishes);
    a.bell.ring();
    wait_for("A recomputed its directory", || {
        count(&a.link.core.publishes) > publishes
    })
    .await;
    assert_eq!(
        count(&a.link.core.rechecks),
        rechecks,
        "the same ids — no re-check"
    );
    a.inbox.local_ids.lock().unwrap().insert(twin.id.clone());
    a.bell.ring();
    wait_for("the row is gone", || {
        row_of(&a.link, &b.id).agents == Some(Vec::new())
    })
    .await;
    assert_eq!(
        count(&a.link.core.rechecks),
        rechecks + 1,
        "one re-check for the changed ids"
    );
    a.link.stop().await;
    b.link.stop().await;
}

/// An endpoint restart takes the resync state of its links with it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restart_clears_the_old_links_resync_state() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, b) = hosted_pair(&da, &db, Vec::new(), Vec::new()).await;
    wait_for("resync traffic", || {
        !lock(&a.link.core.resyncs).links.is_empty()
    })
    .await;
    let old: Vec<LinkKey> = lock(&a.link.core.resyncs).links.keys().copied().collect();
    let before = a
        .link
        .core
        .bring_ups
        .load(std::sync::atomic::Ordering::Acquire);
    a.link.set_network(quiet(free_port())).unwrap();
    wait_restarted(&a.link, before).await;
    let left = lock(&a.link.core.resyncs);
    assert!(
        old.iter().all(|key| !left.links.contains_key(key)),
        "no state of the retired links is left"
    );
    drop(left);
    a.link.stop().await;
    b.link.stop().await;
}

/// G1: a resync request read while the answer to an earlier one is being sent gets one more answer, an interval after that one ended —
///   never a second worker at once, and the earlier answer's end does not swallow the newer request.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_resync_request_during_an_answer_gets_one_more_answer_an_interval_later() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, b) = hosted_pair(
        &da,
        &db,
        vec![agent("id-alice", "alice", AgentState::Live)],
        Vec::new(),
    )
    .await;
    let key = lock(&a.link.core.book).chosen(&b.id).unwrap();
    let idle = || {
        lock(&a.link.core.resyncs)
            .links
            .get(&key)
            .is_some_and(|r| r.last.is_some() && !r.running)
    };
    wait_for("A answered B's first resync and is idle", idle).await;
    wait_for("A published its directory and B holds it", || {
        count(&a.link.core.publishes) > 0
            && row_of(&b.link, &a.id).agents.is_some_and(|r| r.len() == 1)
    })
    .await;
    let (reached, sending) = oneshot::channel();
    let (release, hold) = oneshot::channel();
    *lock(&a.link.core.resync_hold) = Some((reached, hold));
    let sent = count(&a.link.core.dir_sent);
    let to_a = raw(&b, &a.id);
    to_a.notify(PeerMsg::DirResync).unwrap();
    sending.await.expect("the first answer was sent");
    assert_eq!(count(&a.link.core.dir_sent), sent + 1);
    to_a.notify(PeerMsg::DirResync).unwrap();
    flushed(&b, &a).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        count(&a.link.core.dir_sent),
        sent + 1,
        "no second answer while the first one's worker runs"
    );
    let released = tokio::time::Instant::now();
    release.send(()).unwrap();
    wait_for("the newer request answered", || {
        count(&a.link.core.dir_sent) == sent + 2
    })
    .await;
    assert!(
        released.elapsed() >= RESYNC_INTERVAL,
        "answered {:?} after the first ended — not an interval",
        released.elapsed()
    );
    wait_for("A is idle again", idle).await;
    assert_eq!(
        count(&a.link.core.dir_sent),
        sent + 2,
        "each request answered once"
    );
    a.link.stop().await;
    b.link.stop().await;
}

/// Sends `n` overdue reports from B to A, A's host holding them, and waits until 4 are being handed on and the rest queued.
async fn overdue_held(a: &Hosted, b: &Hosted, n: usize) {
    let to_a = raw(b, &a.id);
    a.inbox.overdue_gate.set(true);
    for i in 0..n {
        to_a.notify(PeerMsg::ReplyOverdue {
            request_id: format!("m-{i}"),
        })
        .unwrap();
    }
    flushed(b, a).await;
    wait_for("4 reports handed on", || {
        a.inbox.overdue.lock().unwrap().len() == MAX_OVERDUE_RUNNING
    })
    .await;
}

/// G2 · ADR-0390 d4: overdue reports of a PC still waiting when it is removed are never handed to the host.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn overdue_reports_still_waiting_are_dropped_when_the_pc_is_removed() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, b) = hosted_pair(&da, &db, Vec::new(), Vec::new()).await;
    overdue_held(&a, &b, 10).await;
    a.link.remove(&b.id).unwrap();
    a.inbox.overdue_gate.set(false);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        a.inbox.overdue.lock().unwrap().len(),
        MAX_OVERDUE_RUNNING,
        "nothing waiting reached the host after the removal"
    );
    assert!(lock(&a.link.core.overdue).is_empty());
    a.link.stop().await;
    b.link.stop().await;
}

/// G2: a stop drops every overdue report still waiting.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn overdue_reports_still_waiting_are_dropped_by_a_stop() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, b) = hosted_pair(&da, &db, Vec::new(), Vec::new()).await;
    overdue_held(&a, &b, 10).await;
    a.link.stop().await;
    a.inbox.overdue_gate.set(false);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        a.inbox.overdue.lock().unwrap().len(),
        MAX_OVERDUE_RUNNING,
        "nothing waiting reached the host after the stop"
    );
    assert!(lock(&a.link.core.overdue).is_empty());
    b.link.stop().await;
}

/// G3 · ADR-0391: a frame whose rows were checked before this PC's ids changed, and committed after the copies were re-checked, is
///   checked again under the copies' lock — a row naming the new local agent does not slip in.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_row_checked_before_this_pcs_ids_changed_is_checked_again_when_applied() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let twin = agent("id-twin", "twin", AgentState::Live);
    let (a, b) = hosted_pair(&da, &db, Vec::new(), Vec::new()).await;
    wait_for("A holds B's directory", || {
        row_of(&a.link, &b.id).agents == Some(Vec::new())
    })
    .await;
    let (reached_tx, reached) = std::sync::mpsc::channel();
    let (release, hold) = std::sync::mpsc::channel();
    *lock(&a.link.core.dir_hold) = Some((twin.id.clone(), reached_tx, hold));
    // B creates twin: its change reaches A, which checks the row (not an id of A's yet) and is held before the copies' lock.
    *b.inbox.directory.lock().unwrap() = vec![twin.clone()];
    b.bell.ring();
    wait_for("the frame is held", || reached.try_recv().is_ok()).await;
    // Meanwhile an agent of A takes twin's id and A reads its directory: the copies are re-checked, before the row is in them.
    let rechecks = count(&a.link.core.rechecks);
    a.inbox.local_ids.lock().unwrap().insert(twin.id.clone());
    a.bell.ring();
    wait_for("A re-checked its copies", || {
        count(&a.link.core.rechecks) > rechecks
    })
    .await;
    release.send(()).unwrap();
    flushed(&b, &a).await;
    assert_eq!(
        row_of(&a.link, &b.id).agents,
        Some(Vec::new()),
        "the row was checked again when it was applied"
    );
    a.link.stop().await;
    b.link.stop().await;
}

/// G4: a resync request read by a drain of an endpoint already torn down adds no state back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_resync_request_read_by_a_retired_endpoints_drain_adds_no_state() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, b) = hosted_pair(&da, &db, Vec::new(), Vec::new()).await;
    let route = lock(&a.link.core.routes).get(&b.id).cloned().unwrap();
    let before = a
        .link
        .core
        .bring_ups
        .load(std::sync::atomic::Ordering::Acquire);
    a.link.set_network(quiet(free_port())).unwrap();
    wait_restarted(&a.link, before).await;
    let reply = Reply {
        peer: route.peer.clone(),
        generation: Generation(0),
    };
    on_resync(&a.link.core, route.epoch, b.id, route.key, reply);
    assert!(
        !lock(&a.link.core.resyncs).links.contains_key(&route.key),
        "the retired link got no resync state"
    );
    a.link.stop().await;
    b.link.stop().await;
}

/// G5: applying a directory frame does not take the link book — a frame is applied while the book is held elsewhere.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_directory_frame_is_applied_while_the_link_book_is_held() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, b) = hosted_pair(&da, &db, Vec::new(), Vec::new()).await;
    wait_for("A holds B's directory", || {
        row_of(&a.link, &b.id).agents.is_some()
    })
    .await;
    let chosen = lock(&a.link.core.book).chosen(&b.id).unwrap();
    let copy = lock(&a.link.core.copies).get(&b.id).cloned().unwrap();
    let reply = Reply {
        peer: raw(&a, &b.id),
        generation: Generation(0),
    };
    let x = agent("id-x", "x", AgentState::Live);
    let frame = Routed::Snapshot(DirSnapshot {
        run: copy.run,
        rev: copy.rev + 1,
        agents: vec![x.clone()],
    });
    let core = a.link.core.clone();
    let book = lock(&a.link.core.book);
    let applying =
        tokio::task::spawn_blocking(move || on_directory(&core, b.id, chosen, &reply, frame));
    let done = tokio::time::timeout(Duration::from_secs(2), applying).await;
    drop(book);
    done.expect("applied without waiting for the link book")
        .unwrap();
    assert_eq!(
        lock(&a.link.core.copies).get(&b.id).unwrap().agents,
        vec![x]
    );
    a.link.stop().await;
    b.link.stop().await;
}

/// `count` directory rows whose JSON text is about `width` bytes, in id order.
fn many(count: usize, width: usize) -> Vec<DirAgent> {
    (0..count)
        .map(|n| {
            let name = format!("{n:06}-{}", "n".repeat(width.saturating_sub(48)));
            agent(&format!("id-{n:06}"), &name, AgentState::Live)
        })
        .collect()
}

/// ADR-0411: a directory far over the old 512 KiB frame reaches the other PC whole, and a snapshot frame just under the 4 MiB cap is
///   taken in by the link adapter.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_large_directory_crosses_whole_and_a_frame_up_to_the_cap_is_taken_in() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let big = many(20_000, 100);
    let (a, b) = hosted_pair(&da, &db, big.clone(), Vec::new()).await;
    wait_for("B holds all of A's directory", || {
        row_of(&b.link, &a.id).agents.map_or(0, |rows| rows.len()) == big.len()
    })
    .await;

    // B writes A a snapshot just under the cap.
    wait_for("A holds B's directory", || {
        row_of(&a.link, &b.id).agents.is_some()
    })
    .await;
    let copy = lock(&a.link.core.copies).get(&b.id).cloned().unwrap();
    let row = serde_json::to_string(&many(1, 100)[0]).unwrap().len() + 1;
    let rows = many((MAX_FRAME_BYTES - 1024) / row, 100);
    let frame = PeerMsg::DirSnapshot(DirSnapshot {
        run: copy.run,
        rev: copy.rev + 1,
        agents: rows.clone(),
    });
    let size = encoded_len(&frame).unwrap();
    assert!(
        size > MAX_FRAME_BYTES - (64 << 10) && size <= MAX_FRAME_BYTES,
        "{size} bytes"
    );
    raw(&b, &a.id).notify(frame).unwrap();
    wait_for("A took the whole frame in", || {
        row_of(&a.link, &b.id).agents.map_or(0, |r| r.len()) == rows.len()
    })
    .await;
    assert_eq!(live_links(&a.link, &b.id), 1, "the link stayed");
    a.link.stop().await;
    b.link.stop().await;
}

/// ADR-0411: a directory that does not fit one frame goes cut to the rows that fit — the link stays and mail still flows.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_directory_over_the_cap_goes_cut_and_the_link_stays() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let huge = many(60_000, 100);
    let (a, b) = hosted_pair(&da, &db, huge.clone(), Vec::new()).await;
    wait_for("A published its directory", || {
        count(&a.link.core.publishes) > 0
    })
    .await;
    let (fit, omitted) = {
        let mut publisher = lock(&a.link.core.publisher);
        let snapshot = publisher.snapshot();
        (snapshot.agents.len(), publisher.fitting().1)
    };
    assert!(omitted > 0 && fit > 0, "{fit} rows fit, {omitted} left out");
    assert_eq!(fit + omitted, huge.len());
    wait_for("B holds the rows that fit", || {
        row_of(&b.link, &a.id).agents.map_or(0, |rows| rows.len()) == fit
    })
    .await;
    assert_eq!(
        row_of(&b.link, &a.id).agents.unwrap(),
        huge[..fit],
        "the rows that fit, in id order"
    );
    assert_eq!(live_links(&a.link, &b.id), 1, "the link stayed");
    let end = end_of(send(&a.link, b.id, mail("m-1", &["bob"], "hi"))).await;
    assert!(matches!(end, MailEnd::Verdict(_)), "{end:?}");
    a.link.stop().await;
    b.link.stop().await;
}

/// ADR-0411: a mail larger than one mail may be is refused before it is queued — the host never sees it, nothing of it waits in the
///   receive queue, and the link stays.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_oversized_mail_is_refused_before_it_is_queued() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, b) = hosted_pair(&da, &db, Vec::new(), Vec::new()).await;
    let body = "x".repeat(1 << 20);
    let end = end_of(send(
        &a.link,
        b.id,
        mail("m-huge", &["bob", "carol"], &body),
    ))
    .await;
    let MailEnd::Verdict(rows) = end else {
        panic!("{end:?}")
    };
    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .all(|r| r.outcome == Outcome::Refused && r.code.as_deref() == Some("REMOTE_REFUSED")));
    assert!(
        b.inbox.mails.lock().unwrap().is_empty(),
        "the host never saw it"
    );
    assert!(
        !lock(&b.link.core.inbound).contains_key(&a.id),
        "nothing of it was queued"
    );
    let end = end_of(send(&a.link, b.id, mail("m-small", &["bob"], "hi"))).await;
    assert!(matches!(end, MailEnd::Verdict(_)), "{end:?}");
    assert_eq!(live_links(&a.link, &b.id), 1, "the link stayed");
    a.link.stop().await;
    b.link.stop().await;
}
