//! Mail between agents on different PCs through the whole daemon path — the real mail kernel on each side, the real peer links on
//!   127.0.0.1 (helpers off), [`PeerMail`] between them; only the agents are faked (a `DeliveryPort` over in-memory lists that records
//!   what it injects).

use std::sync::Condvar;

use engram_dashboard_messaging::envelope::{new_msg_id, Entrance};
use engram_dashboard_messaging::service::{RecipientResult, SendMeta, SendStatus, MAX_REMOTE_ROWS};
use engram_dashboard_messaging::SenderIdentity;

use super::*;
use crate::link::peer::directory::DirectoryBell;
use crate::link::peer::runtime::Host;
use crate::link::peer::testing::{entry, test_pc, TempDir};

const PATIENCE: Duration = Duration::from_secs(30);

/// Agents of one PC, faked: who is live, who is asleep, and what reached them. Injection can be held at a gate.
#[derive(Default)]
struct Agents {
    roster: Mutex<Vec<LiveAgent>>,
    dormant: Mutex<Vec<DormantAgent>>,
    injected: Mutex<Vec<(PeerId, String)>>,
    gate: Mutex<bool>,
    gate_changed: Condvar,
    peer: OnceLock<Arc<OnceLock<PeerLink>>>,
    /// Listed PCs besides the peer link's.
    extra_pcs: Mutex<Vec<RemotePc>>,
}

/// A remote port that only keeps what it is handed.
#[derive(Default)]
struct Captured {
    mails: Mutex<Vec<OutboundMail>>,
}

impl RemoteDeliveryPort for Captured {
    fn enqueue(&self, mail: OutboundMail) {
        self.mails.lock().unwrap().push(mail);
    }
    fn send_reply_overdue(&self, _node: NodeRef, _request_id: &str) {}
    fn is_local_agent_id(&self, _id: PeerId) -> bool {
        false
    }
}

impl Agents {
    fn live(&self, name: &str) -> PeerId {
        let id = uuid::Uuid::new_v4();
        self.roster.lock().unwrap().push(LiveAgent {
            id,
            name: name.to_string(),
            epoch: 1,
            turn_signal: false,
        });
        id
    }

    fn asleep(&self, name: &str) -> PeerId {
        let id = uuid::Uuid::new_v4();
        self.dormant.lock().unwrap().push(DormantAgent {
            id,
            name: name.to_string(),
        });
        id
    }

    fn hold(&self, held: bool) {
        *self.gate.lock().unwrap() = held;
        self.gate_changed.notify_all();
    }

    fn injected_to(&self, id: PeerId) -> Vec<String> {
        self.injected
            .lock()
            .unwrap()
            .iter()
            .filter(|(to, _)| *to == id)
            .map(|(_, text)| text.clone())
            .collect()
    }

    fn local_ids(&self) -> std::collections::HashSet<PeerId> {
        let mut ids: std::collections::HashSet<PeerId> =
            self.roster.lock().unwrap().iter().map(|a| a.id).collect();
        ids.extend(self.dormant.lock().unwrap().iter().map(|a| a.id));
        ids
    }
}

impl DeliveryPort for Agents {
    fn inject(&self, to_id: PeerId, bytes: &[u8]) -> Result<InjectReceipt, String> {
        let mut held = self.gate.lock().unwrap();
        while *held {
            held = self.gate_changed.wait(held).unwrap();
        }
        drop(held);
        self.injected
            .lock()
            .unwrap()
            .push((to_id, String::from_utf8_lossy(bytes).into_owned()));
        Ok(InjectReceipt {
            bytes_requested: bytes.len(),
            bytes_written: bytes.len(),
            msg_uuid: uuid::Uuid::new_v4(),
            epoch: 1,
        })
    }

    fn live_agents(&self) -> Vec<LiveAgent> {
        self.roster.lock().unwrap().clone()
    }

    fn addressing_sources(&self) -> AddressingSources {
        AddressingSources {
            roster: self.roster.lock().unwrap().clone(),
            dormant: self.dormant.lock().unwrap().clone(),
            pcs: self
                .peer
                .get()
                .and_then(|slot| slot.get())
                .map(remote_pcs)
                .unwrap_or_default()
                .into_iter()
                .chain(self.extra_pcs.lock().unwrap().clone())
                .collect(),
        }
    }

    fn is_agent_live(&self, id: PeerId) -> bool {
        self.roster.lock().unwrap().iter().any(|a| a.id == id)
    }
}

/// One daemon: its agents, its mail kernel, its peer link.
struct Daemon {
    agents: Arc<Agents>,
    messaging: Arc<MessagingService>,
    mail: Arc<PeerMail>,
    link: PeerLink,
    id: PcId,
    addr: std::net::SocketAddr,
    bell: DirectoryBell,
    _dir: TempDir,
}

async fn daemon(agents: Arc<Agents>) -> Daemon {
    let slot: Arc<OnceLock<PeerLink>> = Arc::new(OnceLock::new());
    let _ = agents.peer.set(slot.clone());
    let local = agents.clone();
    let layers = agents.clone();
    let mail = Arc::new(PeerMail::new(
        slot.clone(),
        Arc::new(move || AddressingSources {
            pcs: Vec::new(),
            ..layers.addressing_sources()
        }),
        Arc::new(move || local.local_ids()),
        Arc::new(ControlRegistry::new()),
        None,
    ));
    // No flush trigger: notices of a verdict are injected inline, inside the peer link's verdict gate — fine for these fakes (their
    //   injection never blocks); the daemon's assembly wires the trigger (`messaging_with_peers`).
    let messaging = Arc::new(
        MessagingService::new(agents.clone(), Arc::new(ControlRegistry::new()))
            .with_remote(mail.clone(), REMOTE_LIMITS),
    );
    mail.attach(&messaging);
    mail.refresh_known_ids();
    let bell = DirectoryBell::new();
    let dir = TempDir::new();
    let host = Host {
        mail: mail.clone(),
        bell: bell.clone(),
    };
    let (link, id, addr) = test_pc(&dir, Some(host)).await;
    let _ = slot.set(link.clone());
    Daemon {
        agents,
        messaging,
        mail,
        link,
        id,
        addr,
        bell,
        _dir: dir,
    }
}

/// `x` lists `y` as `name`; `y` lists `x` as `back` and dials it.
async fn pair(x: &Daemon, name: &str, y: &Daemon, back: &str) {
    x.link.add(entry(y.id, name, None)).unwrap();
    y.link.add(entry(x.id, back, Some(x.addr))).unwrap();
    wait_for("linked", || {
        online(x, &y.id) == Online::Online && online(y, &x.id) == Online::Online
    })
    .await;
}

fn online(d: &Daemon, pc: &PcId) -> Online {
    d.link
        .rows()
        .into_iter()
        .find(|r| r.peer.id == *pc)
        .map_or(Online::Offline, |r| r.online)
}

/// `d` holds a copy of `pc`'s directory that lists `name`.
fn copy_lists(d: &Daemon, pc: &PcId, name: &str) -> bool {
    d.link.rows().into_iter().any(|r| {
        r.peer.id == *pc
            && r.agents
                .as_ref()
                .is_some_and(|agents| agents.iter().any(|a| a.name == name))
    })
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

fn plain() -> SendMeta {
    SendMeta {
        request: false,
        reply_by_raw: None,
        reply_by: None,
        reply_to: None,
        to_attr: None,
    }
}

/// What the control entrance does: send, then wait for the other PCs' outcomes until `wait` (on the blocking pool).
async fn send(
    d: &Daemon,
    from: PeerId,
    name: &str,
    to: &[&str],
    meta: SendMeta,
    wait: Duration,
) -> (String, Vec<RecipientResult>) {
    let messaging = d.messaging.clone();
    let name = name.to_string();
    let to: Vec<String> = to.iter().map(|t| t.to_string()).collect();
    tokio::task::spawn_blocking(move || {
        let id = new_msg_id();
        let arrival = Instant::now();
        let from = SenderIdentity {
            peer_id: from,
            epoch: 1,
            node: None,
        };
        let mut rows = messaging
            .handle_send(&id, from, &name, &to, "hello", Entrance::Mcp, &meta)
            .unwrap();
        messaging.await_remote_answer(&id, &mut rows, arrival + wait);
        (id, rows)
    })
    .await
    .unwrap()
}

const ANSWER: Duration = Duration::from_secs(7);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mail_to_name_at_pc_reaches_that_agent_and_the_sender_gets_delivered() {
    let a = daemon(Arc::default()).await;
    let b = daemon(Arc::default()).await;
    let alice = a.agents.live("alice");
    let bob = b.agents.live("bob");
    pair(&a, "bee", &b, "ay").await;

    let (_, rows) = send(&a, alice, "alice", &["bob@bee"], plain(), ANSWER).await;
    assert_eq!(rows[0].status, SendStatus::Delivered, "{rows:?}");
    let got = b.agents.injected_to(bob);
    assert_eq!(got.len(), 1);
    assert!(
        got[0].contains("alice@ay"),
        "the envelope names the sender as B can reply to it: {}",
        got[0]
    );
}

/// ADR-0391: a bare name not found locally goes to the one PC whose directory lists it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bare_name_resolves_through_the_directory_copy() {
    let a = daemon(Arc::default()).await;
    let b = daemon(Arc::default()).await;
    let alice = a.agents.live("alice");
    let bob = b.agents.live("bob");
    b.bell.ring();
    pair(&a, "bee", &b, "ay").await;
    wait_for("A's copy of B lists bob", || copy_lists(&a, &b.id, "bob")).await;

    let (_, rows) = send(&a, alice, "alice", &["bob"], plain(), ANSWER).await;
    assert_eq!(rows[0].status, SendStatus::Delivered, "{rows:?}");
    assert_eq!(b.agents.injected_to(bob).len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bare_name_on_two_pcs_is_ambiguous_with_name_at_pc_candidates() {
    let a = daemon(Arc::default()).await;
    let b = daemon(Arc::default()).await;
    let c = daemon(Arc::default()).await;
    let alice = a.agents.live("alice");
    b.agents.live("bob");
    c.agents.asleep("bob");
    pair(&a, "bee", &b, "ay").await;
    pair(&a, "sea", &c, "ay").await;
    wait_for("A holds both copies", || {
        copy_lists(&a, &b.id, "bob") && copy_lists(&a, &c.id, "bob")
    })
    .await;

    let (_, rows) = send(&a, alice, "alice", &["bob"], plain(), ANSWER).await;
    assert_eq!(rows[0].status, SendStatus::Failed);
    assert_eq!(rows[0].code, Some(FailCode::RecipientAmbiguous));
    let hint = rows[0].hint.clone().unwrap_or_default();
    assert!(
        hint.contains("bob@bee") && hint.contains("bob@sea"),
        "{hint}"
    );
    assert!(b.agents.injected.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_asleep_recipient_is_held_and_an_unknown_one_is_not_found() {
    let a = daemon(Arc::default()).await;
    let b = daemon(Arc::default()).await;
    let alice = a.agents.live("alice");
    b.agents.asleep("dora");
    pair(&a, "bee", &b, "ay").await;

    let (_, rows) = send(
        &a,
        alice,
        "alice",
        &["dora@bee", "zed@bee"],
        plain(),
        ANSWER,
    )
    .await;
    assert_eq!(rows[0].status, SendStatus::Pending, "{rows:?}");
    assert!(
        rows[0]
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("Held on PC 'bee'")),
        "{rows:?}"
    );
    assert_eq!(rows[1].status, SendStatus::Failed);
    assert_eq!(rows[1].code, Some(FailCode::RecipientNotFound));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mail_to_a_listed_pc_without_a_link_is_peer_offline() {
    let a = daemon(Arc::default()).await;
    let alice = a.agents.live("alice");
    a.link
        .add(entry(PcId::from_bytes([7; 32]), "far", None))
        .unwrap();
    let (_, rows) = send(&a, alice, "alice", &["bob@far"], plain(), ANSWER).await;
    assert_eq!(rows[0].status, SendStatus::Failed);
    assert_eq!(rows[0].code, Some(FailCode::PeerOffline));
}

/// ADR-0393: no verdict within the agent-facing wait → unconfirmed; the verdict that comes later reaches the sender as one notice.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_verdict_after_the_answer_comes_as_one_notice() {
    let a = daemon(Arc::default()).await;
    let b = daemon(Arc::default()).await;
    let alice = a.agents.live("alice");
    b.agents.live("bob");
    pair(&a, "bee", &b, "ay").await;
    b.agents.hold(true);

    let (id, rows) = send(
        &a,
        alice,
        "alice",
        &["bob@bee"],
        plain(),
        Duration::from_millis(500),
    )
    .await;
    assert_eq!(rows[0].status, SendStatus::Pending);
    assert!(
        rows[0]
            .hint
            .as_deref()
            .is_some_and(|h| h.starts_with("Unconfirmed")),
        "{rows:?}"
    );
    b.agents.hold(false);
    wait_for("the late notice reached alice", || {
        !a.agents.injected_to(alice).is_empty()
    })
    .await;
    let notices = a.agents.injected_to(alice);
    assert_eq!(notices.len(), 1);
    assert!(
        notices[0].contains(&id) && notices[0].contains("delivered"),
        "{}",
        notices[0]
    );
}

/// ADR-0393: the contract lives on the receiving PC; the recipient's reply across PCs closes it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reply_across_pcs_closes_the_request_on_the_receiving_pc() {
    let a = daemon(Arc::default()).await;
    let b = daemon(Arc::default()).await;
    let alice = a.agents.live("alice");
    let bob = b.agents.live("bob");
    pair(&a, "bee", &b, "ay").await;

    let request = SendMeta {
        request: true,
        reply_by_raw: Some("10m".to_string()),
        reply_by: Some(Duration::from_secs(600)),
        ..plain()
    };
    let (id, rows) = send(&a, alice, "alice", &["bob@bee"], request, ANSWER).await;
    assert_eq!(rows[0].status, SendStatus::Delivered, "{rows:?}");
    assert_eq!(
        b.messaging.contract_outcome_for_test(&id, bob),
        Some("awaiting_reply")
    );

    let reply = SendMeta {
        reply_to: Some(id.clone()),
        ..plain()
    };
    let (_, rows) = send(&b, bob, "bob", &["alice@ay"], reply, ANSWER).await;
    assert_eq!(rows[0].status, SendStatus::Delivered, "{rows:?}");
    assert_eq!(
        b.messaging.contract_outcome_for_test(&id, bob),
        Some("replied")
    );
    assert_eq!(
        a.agents.injected_to(alice).len(),
        1,
        "the reply reached alice"
    );
}

fn wire_mail(from: PeerId, to: &str) -> Mail {
    Mail {
        id: new_msg_id(),
        from: MailFrom {
            agent_id: from.to_string(),
            name: "alice".to_string(),
        },
        rows: vec![MailRow {
            tag: 0,
            to: to.to_string(),
        }],
        body: "hi".to_string(),
        reply_to: None,
        request: None,
    }
}

/// ADR-0404 d6: this PC's own entrance checks run on a frame from another PC before the kernel sees it; R8: a sender id that is also
///   local is refused (a copied data folder).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_ingress_checks_and_r8_refuse_a_bad_frame_whole() {
    let b = daemon(Arc::default()).await;
    let bob = b.agents.live("bob");
    let from = PcId::from_bytes([3; 32]);
    let stranger = uuid::Uuid::new_v4();
    let check = |mail: Mail| b.mail.mail(from, "ay", &mail, Instant::now());

    let mut bad_deadline = wire_mail(stranger, "bob");
    bad_deadline.request = Some(MailRequest {
        reply_by_raw: Some("soon".to_string()),
    });
    let refusal = check(bad_deadline).unwrap_err();
    assert_eq!(refusal.code, "REMOTE_REFUSED");
    assert!(refusal.reason.contains("reply_by"), "{}", refusal.reason);

    let mut both = wire_mail(stranger, "bob");
    both.request = Some(MailRequest { reply_by_raw: None });
    both.reply_to = Some("m-1".to_string());
    assert_eq!(check(both).unwrap_err().code, "REMOTE_REFUSED");

    let copied = check(wire_mail(bob, "bob")).unwrap_err();
    assert_eq!(copied.code, "REMOTE_REFUSED");
    assert!(
        copied.reason.contains("copied data folder"),
        "{}",
        copied.reason
    );

    let mut not_an_id = wire_mail(stranger, "bob");
    not_an_id.from.agent_id = "bob".to_string();
    assert_eq!(check(not_an_id).unwrap_err().code, "REMOTE_REFUSED");

    assert!(
        b.agents.injected.lock().unwrap().is_empty(),
        "nothing was delivered"
    );
    let rows = check(wire_mail(stranger, "bob")).unwrap();
    assert_eq!(rows[0].outcome, Outcome::Delivered);
    assert_eq!(rows[0].recipient_id, Some(bob.to_string()));
}

/// R16: the published directory is what local resolution sees — live readers live, dormant profiles sleeping — minus names another PC
///   would refuse; an arriving row with a local id, a bad name or no agent id is dropped.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_directory_is_the_local_resolution_set_and_arriving_rows_are_checked() {
    let a = daemon(Arc::default()).await;
    let alice = a.agents.live("alice");
    let dora = a.agents.asleep("dora");
    a.agents.live("bell\u{7}");
    let mut directory = a.mail.directory();
    directory.sort_by(|x, y| x.name.cmp(&y.name));
    assert_eq!(
        directory,
        vec![
            DirAgent {
                id: alice.to_string(),
                name: "alice".to_string(),
                state: AgentState::Live
            },
            DirAgent {
                id: dora.to_string(),
                name: "dora".to_string(),
                state: AgentState::Sleeping
            },
        ]
    );
    let row = |id: String, name: &str| DirAgent {
        id,
        name: name.to_string(),
        state: AgentState::Live,
    };
    let rows = [
        row(alice.to_string(), "x"),
        row("bob".to_string(), "bob"),
        row(uuid::Uuid::new_v4().to_string(), ""),
        row(uuid::Uuid::new_v4().to_string(), "bob"),
    ];
    let problems = a.mail.row_problems(&rows.iter().collect::<Vec<_>>());
    assert!(problems[0].is_some(), "a local agent's id");
    assert!(problems[1].is_some(), "not an agent id");
    assert!(problems[2].is_some(), "a name another PC would refuse");
    assert!(problems[3].is_none());
}

/// The one final verdict call per mail, for every way a mail can end (ADR-0393) — observed in the kernel's answer rows.
#[test]
fn every_mail_end_settles_every_tag_in_the_kernel() {
    let recipient = uuid::Uuid::new_v4();
    let verdict = |outcome, recipient_id: Option<String>, code: Option<&str>| {
        MailEnd::Verdict(vec![VerdictRow {
            tag: 0,
            outcome,
            recipient_id,
            code: code.map(str::to_string),
            reason: Some("why".to_string()),
        }])
    };
    let cases: Vec<(MailEnd, SendStatus, Option<FailCode>)> = vec![
        (
            MailEnd::Offline,
            SendStatus::Failed,
            Some(FailCode::PeerOffline),
        ),
        (
            MailEnd::Limit,
            SendStatus::Failed,
            Some(FailCode::RemoteLimit),
        ),
        (
            MailEnd::TooLarge,
            SendStatus::Failed,
            Some(FailCode::BodyTooLarge),
        ),
        (MailEnd::Unconfirmed, SendStatus::Pending, None),
        (
            verdict(Outcome::Delivered, Some(recipient.to_string()), None),
            SendStatus::Delivered,
            None,
        ),
        // Held without a recipient id cannot be trusted either way — unconfirmed.
        (
            verdict(Outcome::Held, None, None),
            SendStatus::Pending,
            None,
        ),
        (
            verdict(Outcome::Refused, None, Some("MAILBOX_FULL")),
            SendStatus::Failed,
            Some(FailCode::MailboxFull),
        ),
        (
            verdict(Outcome::UnknownRecipient, None, None),
            SendStatus::Failed,
            Some(FailCode::RecipientNotFound),
        ),
        // A tag the verdict leaves out is settled unconfirmed by the kernel.
        (MailEnd::Verdict(Vec::new()), SendStatus::Pending, None),
    ];
    for (end, status, code) in cases {
        let what = format!("{end:?}");
        let agents = Arc::new(Agents::default());
        let alice = agents.live("alice");
        *agents.extra_pcs.lock().unwrap() = vec![RemotePc {
            node: NodeRef::from_bytes([9; 32]),
            name: "far".to_string(),
            link: PcLink::Connected,
            directory: Some(Vec::new()),
        }];
        let captured = Arc::new(Captured::default());
        let messaging = Arc::new(
            MessagingService::new(agents.clone(), Arc::new(ControlRegistry::new()))
                .with_remote(captured.clone(), REMOTE_LIMITS),
        );
        let from = SenderIdentity {
            peer_id: alice,
            epoch: 1,
            node: None,
        };
        let id = new_msg_id();
        let mut rows = messaging
            .handle_send(
                &id,
                from,
                "alice",
                &["bob@far".to_string()],
                "hi",
                Entrance::Mcp,
                &plain(),
            )
            .unwrap();
        let mail = captured
            .mails
            .lock()
            .unwrap()
            .pop()
            .expect("handed to the port");
        let tags = mail.rows.iter().map(|r| r.tag).collect();
        verdict_call(Some(Arc::downgrade(&messaging)), mail.id, mail.node, tags)(end);
        messaging.await_remote_answer(&id, &mut rows, Instant::now());
        assert_eq!(rows[0].status, status, "{what}: {rows:?}");
        assert_eq!(rows[0].code, code, "{what}: {rows:?}");
        if status == SendStatus::Pending {
            assert!(
                rows[0]
                    .hint
                    .as_deref()
                    .is_some_and(|h| h.starts_with("Unconfirmed")),
                "{what}: {rows:?}"
            );
        }
    }
}

/// ADR-0390: at most 64 agents of one PC per send — the rows past it fail on this PC with their own wording; the first 64 go.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rows_past_sixty_four_to_one_pc_fail_here_and_the_rest_go() {
    let a = daemon(Arc::default()).await;
    let b = daemon(Arc::default()).await;
    let alice = a.agents.live("alice");
    pair(&a, "bee", &b, "ay").await;
    let names: Vec<String> = (0..=MAX_REMOTE_ROWS)
        .map(|n| format!("agent{n}@bee"))
        .collect();
    let to: Vec<&str> = names.iter().map(String::as_str).collect();
    let (_, rows) = send(&a, alice, "alice", &to, plain(), ANSWER).await;
    assert_eq!(rows.len(), MAX_REMOTE_ROWS + 1);
    assert!(
        rows[..MAX_REMOTE_ROWS]
            .iter()
            .all(|r| r.code == Some(FailCode::RecipientNotFound)),
        "the first 64 reached B, which has none of them: {rows:?}"
    );
    let over = &rows[MAX_REMOTE_ROWS];
    assert_eq!(over.code, Some(FailCode::RemoteLimit));
    assert!(
        over.hint
            .as_deref()
            .is_some_and(|h| h.contains("separate send")),
        "{over:?}"
    );
}

/// The link's copies of the kernel's bounds stay equal to them (`link/` may not name the kernel — CI daemon gate 1).
#[test]
fn the_links_mail_bounds_are_the_kernels() {
    assert_eq!(
        crate::link::peer::mail::MAIL_DEADLINE,
        engram_dashboard_messaging::ledger::REMOTE_TRANSPORT_DEADLINE,
        "ADR-0404 d2 · d5: the overdue grace is built on the transport deadline"
    );
    assert_eq!(crate::link::peer::mail::MAX_MAIL_ROWS, MAX_REMOTE_ROWS);
    // ADR-0411: the link's size bound before queueing never refuses a mail the kernel would take (a full body and room for the rest).
    assert!(REMOTE_LIMITS.max_body_bytes * 2 <= crate::link::peer::mail::MAX_MAIL_BYTES);
}

/// ADR-0393: the control entrance waits for other PCs' outcomes until its request's arrival + `CommandDeliveries::DEFAULT_DEADLINE`,
///   then answers what has none unconfirmed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_control_entrance_waits_from_its_arrival_and_answers_unconfirmed() {
    use crate::command_delivery::CommandDeliveries;
    use crate::control::ingress::{handle_send_at, ControlCommand, ControlResult, SendContract};
    use crate::control::registry::BoundIdentity;

    let a = daemon(Arc::default()).await;
    let b = daemon(Arc::default()).await;
    let alice = a.agents.live("alice");
    b.agents.live("bob");
    pair(&a, "bee", &b, "ay").await;
    b.agents.hold(true);

    let manager = test_manager();
    let messaging = a.messaging.clone();
    let (result, waited) = tokio::task::spawn_blocking(move || {
        let registry = Arc::new(ControlRegistry::new());
        let cmd = ControlCommand {
            from: BoundIdentity {
                agent_id: alice,
                epoch: 1,
            },
            to: vec!["bob@bee".to_string()],
            body: "hi".to_string(),
            contract: SendContract::default(),
        };
        // The request came in almost the whole wait ago — only the rest of it is left.
        let arrival =
            Instant::now() - CommandDeliveries::DEFAULT_DEADLINE + Duration::from_millis(300);
        let started = Instant::now();
        let result = handle_send_at(&manager, &registry, &messaging, Entrance::Mcp, cmd, arrival);
        (result, started.elapsed())
    })
    .await
    .unwrap();
    b.agents.hold(false);

    assert!(
        waited < Duration::from_secs(3),
        "the wait counts from the arrival, not from now: {waited:?}"
    );
    let ControlResult::Ok { results, .. } = result else {
        panic!("{:?}", result.to_json())
    };
    assert_eq!(results[0].status, "pending");
    assert!(
        results[0]
            .hint
            .as_deref()
            .is_some_and(|h| h.starts_with("Unconfirmed")),
        "the entrance sealed the answer: {:?}",
        results[0]
    );
}

/// An agent manager with no agents — the entrance reads the sender's name from it (falls back to the id).
fn test_manager() -> Arc<engram_dashboard_agent::manager::AgentManager> {
    use engram_dashboard_agent::persistence::{FilePresetStore, FileProfileStore};
    use engram_dashboard_agent::preset::PresetRegistry;
    use engram_dashboard_agent::profile::ProfileRegistry;
    use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
    use engram_dashboard_agent::types::{AgentInfo, AgentStatus};

    struct NoopSink;
    impl StatusSink for NoopSink {
        fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
        fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
    }
    let dir = |k: &str| {
        std::env::temp_dir().join(format!("engram-peer-mail-{k}-{}", uuid::Uuid::new_v4()))
    };
    Arc::new(AgentManager::new(
        Arc::new(NoopSink),
        Arc::new(ProfileRegistry::new(Arc::new(FileProfileStore::new(dir(
            "prof",
        ))))),
        Arc::new(PresetRegistry::new(Arc::new(FilePresetStore::new(dir(
            "preset",
        ))))),
        Arc::new(SessionTracker::new(
            TrackerConfig {
                enabled: false,
                poll_interval: Duration::from_secs(1),
            },
            Arc::new(|_, _| {}),
        )),
    ))
}

/// R16 triggers: a session status change and a list change ring the directory bell (live ↔ asleep changes this PC's directory).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_status_chain_rings_the_directory_bell() {
    use engram_dashboard_agent::types::{AgentInfo, AgentStatus};
    struct NoopSink;
    impl StatusSink for NoopSink {
        fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
        fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
    }
    let (flush_tx, _flush_rx) = mpsc::unbounded_channel();
    let bell = DirectoryBell::new();
    let sink =
        MessagingFlushSink::new_test(Box::new(NoopSink), flush_tx, Arc::new(IdleCoalescer::new()))
            .with_directory(bell.clone());
    let rung = || tokio::time::timeout(Duration::from_secs(5), bell.rung());

    sink.status_changed(uuid::Uuid::new_v4(), AgentStatus::Running, 1);
    assert!(rung().await.is_ok(), "a status change rings");
    sink.agent_list_updated(Vec::new());
    assert!(rung().await.is_ok(), "a list change rings");
    assert!(
        tokio::time::timeout(Duration::from_millis(200), bell.rung())
            .await
            .is_err(),
        "nothing else rang"
    );
}
