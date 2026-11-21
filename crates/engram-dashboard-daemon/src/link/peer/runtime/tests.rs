//! The running peer link between PCs in one process — real link-adapter endpoints on 127.0.0.1 with relay, address lookup and mDNS off
//!   (nothing leaves the machine), the peer state in temp folders and the OS behind a fake (sealing tags the blob; the identity lock is a
//!   shared in-memory set). Waits are "notification or a short poll, with a cap"; nothing passes by sleeping.
//!
//! Not here (two real PCs — U5): NAT traversal, relay and lookup servers, mDNS, the firewall prompt, the 30 s QUIC idle-out of a frozen
//!   peer (measured in the adapter's own suite).

use std::collections::HashSet;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use engram_dashboard_platform::secret::UnsealError;

use super::*;
use crate::link::peer::mac;
use crate::link::peer::os::{LockAttempt, PeerOs};
use crate::link::peer::wire::DuplicateKind;

const ACCOUNT: &str = "S-1-5-21-10-20-30-1001";
const MACHINE: &str = "aaaa-1111";
const PATIENCE: Duration = Duration::from_secs(30);

/// A relay on this machine that is not there — a network setting that differs without contacting anything outside.
const LOCAL_RELAY: &str = "http://127.0.0.1:1";

/// A URL whose form the trusted list accepts but the link adapter refuses (invalid punycode).
const ADAPTER_REFUSES: &str = "https://xn--a.example";

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("engram-peer-link-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn key_file(&self) -> PathBuf {
        self.0.join(format!("peer-key.{ACCOUNT}.{MACHINE}"))
    }

    fn list_file(&self) -> PathBuf {
        self.0.join(format!("peers.{ACCOUNT}.{MACHINE}.json"))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

type Locks = Arc<Mutex<HashSet<String>>>;

/// The OS fake: a sealed blob is `T` + the secret; unsealing can be made to fail transiently.
struct FakeOs {
    locks: Locks,
    transient: Arc<AtomicBool>,
}

struct Release {
    name: String,
    locks: Locks,
}

impl Drop for Release {
    fn drop(&mut self) {
        self.locks.lock().unwrap().remove(&self.name);
    }
}

impl PeerOs for FakeOs {
    fn seal(&self, secret: &[u8]) -> io::Result<Vec<u8>> {
        let mut blob = vec![b'T'];
        blob.extend_from_slice(secret);
        Ok(blob)
    }

    fn unseal(&self, blob: &[u8]) -> Result<Vec<u8>, UnsealError> {
        if self.transient.load(Ordering::Acquire) {
            return Err(UnsealError::Transient(io::Error::other(
                "master key not ready",
            )));
        }
        match blob {
            [b'T', rest @ ..] if rest.len() == 32 => Ok(rest.to_vec()),
            _ => Err(UnsealError::Unreadable(io::Error::new(
                io::ErrorKind::InvalidData,
                "bad data",
            ))),
        }
    }

    fn lock(&self, name: &str) -> LockAttempt {
        let mut locks = self.locks.lock().unwrap();
        if !locks.insert(name.to_string()) {
            return LockAttempt::HeldByThisAccount;
        }
        LockAttempt::Held(Box::new(Release {
            name: name.to_string(),
            locks: self.locks.clone(),
        }))
    }

    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

async fn start_with(dir: &Path, locks: &Locks, transient: &Arc<AtomicBool>) -> PeerLink {
    let dir = dir.to_path_buf();
    let os = Arc::new(FakeOs {
        locks: locks.clone(),
        transient: transient.clone(),
    });
    PeerLink::start_loading(
        move || PeerStore::load(dir, Ok(ACCOUNT.to_string()), Ok(MACHINE.to_string()), os),
        LinkSettings {
            dev: true,
            loopback_only: true,
        },
        Arc::new(SystemClock),
    )
    .await
    .unwrap()
}

async fn start(dir: &Path) -> PeerLink {
    start_with(dir, &Locks::default(), &Arc::default()).await
}

fn free_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Network settings that contact nothing but the typed addresses.
fn quiet(port: u16) -> NetworkSettings {
    NetworkSettings {
        port: Some(port),
        relay: Helper::None,
        address_lookup: Helper::None,
        mdns: false,
    }
}

/// A started PC with its key made and its helpers off — its ID and the address it will listen on.
async fn pc(dir: &TempDir) -> (PeerLink, PeerId, SocketAddr) {
    let link = start(dir.path()).await;
    let port = free_port();
    link.set_network(quiet(port)).unwrap();
    let id = link.ensure_identity().unwrap();
    (link, id, SocketAddr::from(([127, 0, 0, 1], port)))
}

fn entry(id: PeerId, name: &str, address: Option<SocketAddr>) -> TrustedPeer {
    TrustedPeer {
        id,
        name: name.to_string(),
        address,
        permission: Permission::Mail,
    }
}

/// A listed PC that never answers — keeps an endpoint up.
fn bystander(n: u8) -> TrustedPeer {
    entry(PeerId::from_bytes([n; 32]), &format!("bystander{n}"), None)
}

async fn wait_until(
    link: &PeerLink,
    what: &str,
    patience: Duration,
    ready: impl Fn(&PeerSnapshot) -> bool,
) -> PeerSnapshot {
    let mut changes = link.changes();
    let deadline = tokio::time::Instant::now() + patience;
    loop {
        let snapshot = link.snapshot();
        if ready(&snapshot) {
            return snapshot;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{what} did not happen within {patience:?}: {snapshot:?}"
        );
        tokio::select! {
            _ = changes.changed() => {}
            () = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
}

fn online_of(snapshot: &PeerSnapshot, id: &PeerId) -> Option<Online> {
    snapshot
        .peers
        .iter()
        .find(|p| p.peer.id == *id)
        .map(|p| p.online)
}

fn is_up(snapshot: &PeerSnapshot) -> bool {
    matches!(snapshot.endpoint, EndpointView::Up { .. })
}

fn live_links(link: &PeerLink, id: &PeerId) -> usize {
    lock(&link.core.book).live_links(id)
}

async fn wait_online(link: &PeerLink, id: &PeerId) {
    wait_until(link, "online", PATIENCE, |s| {
        online_of(s, id) == Some(Online::Online)
    })
    .await;
}

/// Two PCs linked by B dialing A (A has no address for B).
async fn linked_pair(
    da: &TempDir,
    db: &TempDir,
) -> (PeerLink, PeerId, SocketAddr, PeerLink, PeerId) {
    let (a, a_id, a_addr) = pc(da).await;
    let (b, b_id, _) = pc(db).await;
    a.add(entry(b_id, "bee", None)).unwrap();
    a.add(bystander(90)).unwrap();
    b.add(entry(a_id, "ay", Some(a_addr))).unwrap();
    wait_online(&a, &b_id).await;
    wait_online(&b, &a_id).await;
    (a, a_id, a_addr, b, b_id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_endpoint_runs_only_while_a_pc_is_listed() {
    let dir = TempDir::new();
    let (a, _, _) = pc(&dir).await;
    assert_eq!(a.snapshot().status, PeerStatus::Ready);
    assert_eq!(
        a.snapshot().endpoint,
        EndpointView::Off,
        "no PC listed — no endpoint"
    );

    a.add(bystander(7)).unwrap();
    wait_until(&a, "endpoint up", PATIENCE, is_up).await;

    let removal = a.remove(&PeerId::from_bytes([7; 32])).unwrap();
    assert!(removal.saved.is_ok());
    wait_until(&a, "endpoint down", PATIENCE, |s| {
        s.endpoint == EndpointView::Off
    })
    .await;
    a.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_pcs_exchange_hellos_and_link_once() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, a_id, _, b, b_id) = linked_pair(&da, &db).await;
    assert_eq!(live_links(&a, &b_id), 1);
    assert_eq!(live_links(&b, &a_id), 1);
    let row =
        |s: &PeerSnapshot, id: &PeerId| s.peers.iter().find(|p| p.peer.id == *id).cloned().unwrap();
    assert_eq!(row(&a.snapshot(), &b_id).warning, None);
    assert_eq!(row(&b.snapshot(), &a_id).warning, None);
    a.stop().await;
    b.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unlisted_pc_is_refused_shows_as_a_knock_and_a_later_add_re_arms_it() {
    let (da, dc) = (TempDir::new(), TempDir::new());
    let (a, a_id, a_addr) = pc(&da).await;
    let (c, c_id, _) = pc(&dc).await;
    a.add(bystander(91)).unwrap();
    wait_until(&a, "endpoint up", PATIENCE, is_up).await;

    c.add(entry(a_id, "ay", Some(a_addr))).unwrap();
    wait_until(&c, "refused as not paired", PATIENCE, |s| {
        online_of(s, &a_id) == Some(Online::Refused)
    })
    .await;
    let knocked = wait_until(&a, "the knock recorded", PATIENCE, |s| {
        s.knocks.iter().any(|k| k.id == c_id)
    })
    .await;
    assert!(knocked.peers.iter().all(|p| p.peer.id != c_id));
    assert_eq!(live_links(&a, &c_id), 0);

    // A lists C (without an address, so only C can dial); C's own add re-arms its refused dialing handle.
    a.add(entry(c_id, "cee", None)).unwrap();
    assert!(
        a.snapshot().knocks.iter().all(|k| k.id != c_id),
        "an added PC leaves the knock list"
    );
    c.add(bystander(92)).unwrap();
    wait_online(&c, &a_id).await;
    wait_online(&a, &c_id).await;
    a.stop().await;
    c.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn removing_a_pc_mid_link_cuts_it_within_a_second_and_refuses_it_after() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, a_id, _, b, b_id) = linked_pair(&da, &db).await;
    let started = Instant::now();
    let removal = a.remove(&b_id).unwrap();
    assert!(removal.saved.is_ok());
    assert!(a.snapshot().peers.iter().all(|p| p.peer.id != b_id));
    wait_until(&b, "the link cut", Duration::from_secs(1), |s| {
        online_of(s, &a_id) != Some(Online::Online)
    })
    .await;
    assert!(started.elapsed() <= Duration::from_secs(1));
    assert_eq!(live_links(&a, &b_id), 0);
    // B redials and is refused at the gate.
    wait_until(&b, "refused", PATIENCE, |s| {
        online_of(s, &a_id) == Some(Online::Refused)
    })
    .await;
    a.stop().await;
    b.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn when_both_pcs_dial_exactly_one_link_survives() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, a_id, a_addr) = pc(&da).await;
    let (b, b_id, b_addr) = pc(&db).await;
    // Each lists the other without an address, so neither dial can land (and neither is refused as not paired) ...
    a.add(entry(b_id, "bee", None)).unwrap();
    b.add(entry(a_id, "ay", None)).unwrap();
    wait_until(&a, "endpoint up", PATIENCE, is_up).await;
    wait_until(&b, "endpoint up", PATIENCE, is_up).await;
    // ... then both addresses at once: each waiting handle dials now (or at its next attempt, a couple of seconds on). Whichever link
    //   lands first closes the other side's handle if it is between attempts, so the two dials need not meet (see below).
    a.edit(
        &b_id,
        PeerEdit {
            name: None,
            address: Some(Some(b_addr)),
        },
    )
    .unwrap();
    b.edit(
        &a_id,
        PeerEdit {
            name: None,
            address: Some(Some(a_addr)),
        },
    )
    .unwrap();
    wait_online(&a, &b_id).await;
    wait_online(&b, &a_id).await;
    // Settled: one link on each side, still online after another dial would have come and gone.
    let settled = |link: &PeerLink, id: PeerId| {
        let link = link.clone();
        async move {
            let deadline = tokio::time::Instant::now() + PATIENCE;
            while live_links(&link, &id) != 1 {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "links never settled to one"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    };
    settled(&a, b_id).await;
    settled(&b, a_id).await;
    // Past the together window and another dial attempt: still one link on each side, and both sides kept the same connection (one
    //   holds it as dialed, the other as accepted). Which one wins when both land together is pinned by the book's own test
    //   (`both_pcs_keep_the_same_link_when_they_dial_together`) — here a dial that is between attempts when the other side's link comes
    //   is closed first, so two dials landing together cannot be forced.
    tokio::time::sleep(BACKOFF_BASE * 3).await;
    assert_eq!(live_links(&a, &b_id), 1);
    assert_eq!(live_links(&b, &a_id), 1);
    let a_dir = lock(&a.core.book).chosen_dir(&b_id);
    let b_dir = lock(&b.core.book).chosen_dir(&a_id);
    assert!(
        matches!(
            (a_dir, b_dir),
            (Some(Dir::Dialed), Some(Dir::Accepted)) | (Some(Dir::Accepted), Some(Dir::Dialed))
        ),
        "{a_dir:?} / {b_dir:?}"
    );
    assert_eq!(online_of(&a.snapshot(), &b_id), Some(Online::Online));
    assert_eq!(online_of(&b.snapshot(), &a_id), Some(Online::Online));
    a.stop().await;
    b.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_graceful_stop_closes_the_links_and_a_lost_link_reconnects() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, a_id, _, b, _) = linked_pair(&da, &db).await;
    a.stop().await;
    assert_eq!(a.snapshot().endpoint, EndpointView::Off);
    // Well inside QUIC's 30 s idle-out: the close itself told B.
    wait_until(&b, "B sees A gone", Duration::from_secs(5), |s| {
        online_of(s, &a_id) == Some(Online::Offline)
    })
    .await;
    drop(a);
    // A runs again on the same folder and port; B's dialing handle finds it.
    let a2 = start(da.path()).await;
    wait_until(&a2, "endpoint up again", PATIENCE, is_up).await;
    wait_online(&b, &a_id).await;
    a2.stop().await;
    b.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_running_copy_of_one_identity_is_refused_and_both_sides_warn() {
    let (da, db1, db2) = (TempDir::new(), TempDir::new(), TempDir::new());
    let (a, a_id, _, b1, b_id) = linked_pair(&da, &db1).await;
    // A copy of B's folder (same key, same list), run under its own identity lock (as on a cloned machine).
    std::fs::copy(db1.key_file(), db2.key_file()).unwrap();
    std::fs::copy(db1.list_file(), db2.list_file()).unwrap();
    let b2 = start(db2.path()).await;
    assert_eq!(b2.snapshot().id, Some(b_id));
    b2.set_network(quiet(free_port())).unwrap();

    let warned = wait_until(&a, "A detects the duplicate", PATIENCE, |s| {
        s.peers.iter().any(|p| {
            p.peer.id == b_id
                && p.warning
                    .is_some_and(|w| w.kind == DuplicateKind::Duplicate)
        })
    })
    .await;
    assert_eq!(
        online_of(&warned, &b_id),
        Some(Online::Online),
        "the older link is kept"
    );
    wait_until(&b2, "the copy is refused as a duplicate", PATIENCE, |s| {
        online_of(s, &a_id) == Some(Online::DuplicateIdentity)
    })
    .await;
    wait_until(&b1, "the original is told", PATIENCE, |s| {
        s.peers.iter().any(|p| {
            p.peer.id == a_id
                && p.warning
                    == Some(Warning {
                        kind: DuplicateKind::Duplicate,
                        reported: true,
                    })
        })
    })
    .await;
    assert_eq!(online_of(&b1.snapshot(), &a_id), Some(Online::Online));
    assert_eq!(live_links(&a, &b_id), 1);

    // A's endpoint restarts (new network settings): the warning stays and the copy's run is still refused.
    let before = bring_ups(&a);
    let network = a.snapshot().network.unwrap();
    a.set_network(NetworkSettings {
        relay: Helper::Url(LOCAL_RELAY.to_string()),
        ..network
    })
    .unwrap();
    wait_restarted(&a, before).await;
    wait_online(&b1, &a_id).await;
    wait_online(&a, &b_id).await;
    let row = a
        .snapshot()
        .peers
        .into_iter()
        .find(|p| p.peer.id == b_id)
        .unwrap();
    assert_eq!(row.warning.map(|w| w.kind), Some(DuplicateKind::Duplicate));
    assert_eq!(
        lock(&a.core.book).admit(&b_id, b2.core.instance, std::time::Instant::now()),
        Err(DuplicateKind::Duplicate)
    );
    a.stop().await;
    b1.stop().await;
    b2.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restart_onto_a_port_still_held_retries_quickly_without_showing_an_error() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, _, _, b, b_id) = linked_pair(&da, &db).await;
    let before = bring_ups(&a);
    // Another socket holds the new port for a moment, as a closing endpoint can.
    let port = free_port();
    let held = UdpSocket::bind(("127.0.0.1", port)).unwrap();
    a.set_network(quiet(port)).unwrap();
    let watched = tokio::time::Instant::now() + Duration::from_secs(1);
    while tokio::time::Instant::now() < watched {
        let snapshot = a.snapshot();
        assert!(
            !matches!(snapshot.endpoint, EndpointView::Error(_)),
            "an error shown during the quick retries: {snapshot:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        bring_ups(&a),
        before,
        "the held port kept the endpoint down"
    );
    drop(held);
    // Within the quick retries (about 3 s), far under the 60 s bind retry.
    let up = wait_until(
        &a,
        "endpoint up on the freed port",
        Duration::from_secs(4),
        |s| is_up(s) && s.network.as_ref().is_some_and(|n| n.port == Some(port)),
    )
    .await;
    let EndpointView::Up { addrs } = up.endpoint else {
        unreachable!()
    };
    assert!(addrs.iter().any(|a| a.port() == port), "{addrs:?}");
    assert!(bring_ups(&a) > before);
    assert_eq!(live_links(&a, &b_id), 0, "B dials the old port");
    a.stop().await;
    b.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stop_past_its_deadline_still_cuts_the_links_and_closes_the_endpoint() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, a_id, a_addr, b, _) = linked_pair(&da, &db).await;
    // The runtime is stuck — it will not handle the stop.
    let (release, stall) = oneshot::channel();
    assert!(a.core.tx.send(Msg::Stall(stall)).is_ok());
    let started = Instant::now();
    a.stop().await;
    assert!(
        started.elapsed() >= STOP_DEADLINE,
        "it was cut, not stopped gracefully"
    );
    assert_eq!(a.snapshot().endpoint, EndpointView::Off);
    // Cut, not left open: B sees the link go, and A's port is free again (the endpoint closed).
    wait_until(&b, "B sees A gone", Duration::from_secs(5), |s| {
        online_of(s, &a_id) != Some(Online::Online)
    })
    .await;
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while UdpSocket::bind(a_addr).is_err() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "A's port was never freed"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    drop(release);
    b.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dialing_handle_between_attempts_is_closed_when_the_other_pc_links_first() {
    let (da, db) = (TempDir::new(), TempDir::new());
    let (a, a_id, a_addr) = pc(&da).await;
    let (b, b_id, _) = pc(&db).await;
    // A has no address for B: its dialing handle fails at once and waits out its backoff (about 2 s) — B dials only once it does.
    a.add(entry(b_id, "bee", None)).unwrap();
    wait_until(&a, "endpoint up", PATIENCE, is_up).await;
    let deadline = tokio::time::Instant::now() + PATIENCE;
    loop {
        let (answer, state) = oneshot::channel();
        assert!(a.core.tx.send(Msg::DialState(b_id, answer)).is_ok());
        let state = tokio::time::timeout_at(deadline, state)
            .await
            .expect("the runtime did not answer")
            .unwrap();
        if state == Some(PeerState::Backoff) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "A's dialing handle never backed off"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    b.add(entry(a_id, "ay", Some(a_addr))).unwrap();
    wait_online(&a, &b_id).await;
    // Closed as soon as B's link came — not left to connect later and churn.
    assert_eq!(lock(&a.core.book).dial_of(&b_id), None);
    a.stop().await;
    b.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn identity_states_that_keep_the_feature_off_keep_the_endpoint_down() {
    let dir = TempDir::new();
    let locks = Locks::default();
    let transient = Arc::new(AtomicBool::new(false));
    let first = start_with(dir.path(), &locks, &transient).await;
    first.set_network(quiet(free_port())).unwrap();
    first.add(bystander(95)).unwrap();
    wait_until(&first, "endpoint up", PATIENCE, is_up).await;

    // A second copy of this identity on this machine and account: refused locally, endpoint never up.
    let second = start_with(dir.path(), &locks, &transient).await;
    reconciled(&second).await;
    let snapshot = second.snapshot();
    assert_eq!(snapshot.status, PeerStatus::IdentityInUse);
    assert_eq!(snapshot.endpoint, EndpointView::Off);
    second.stop().await;
    drop(second);
    first.stop().await;
    drop(first);
    wait_until_unlocked(&locks).await;

    // The key cannot be read for now: off, retried, and back up with the same ID once it reads.
    transient.store(true, Ordering::Release);
    let retrying = start_with(dir.path(), &locks, &transient).await;
    reconciled(&retrying).await;
    assert!(matches!(
        retrying.snapshot().status,
        PeerStatus::KeyTransient(_)
    ));
    assert_eq!(retrying.snapshot().endpoint, EndpointView::Off);
    transient.store(false, Ordering::Release);
    let back = wait_until(&retrying, "recovered by the retry", PATIENCE, is_up).await;
    assert_eq!(back.status, PeerStatus::Ready);
    retrying.stop().await;
    drop(retrying);
    wait_until_unlocked(&locks).await;

    // The list was changed outside Engram: off until a human accepts it.
    let text = std::fs::read_to_string(dir.list_file()).unwrap();
    std::fs::write(dir.list_file(), text.replace("bystander95", "bystander96")).unwrap();
    let changed = start_with(dir.path(), &locks, &transient).await;
    reconciled(&changed).await;
    assert_eq!(changed.snapshot().status, PeerStatus::ListChangedOutside);
    assert_eq!(
        changed.snapshot().peers.len(),
        1,
        "the list still names a PC"
    );
    assert_eq!(changed.snapshot().endpoint, EndpointView::Off);
    changed.accept_list().unwrap();
    wait_until(&changed, "up once accepted", PATIENCE, is_up).await;
    changed.stop().await;
    drop(changed);
    wait_until_unlocked(&locks).await;

    // An unreadable key: off.
    std::fs::write(dir.key_file(), b"garbage").unwrap();
    let unreadable = start_with(dir.path(), &locks, &transient).await;
    reconciled(&unreadable).await;
    assert!(matches!(
        unreadable.snapshot().status,
        PeerStatus::KeyUnreadable(_)
    ));
    assert_eq!(unreadable.snapshot().endpoint, EndpointView::Off);
    unreadable.stop().await;
}

fn bring_ups(link: &PeerLink) -> u64 {
    link.core
        .bring_ups
        .load(std::sync::atomic::Ordering::Acquire)
}

/// Waits until the endpoint was brought up again after `before` — a restart judged after it happened, not on the old endpoint.
async fn wait_restarted(link: &PeerLink, before: u64) {
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while bring_ups(link) <= before {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the endpoint never restarted"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Waits until the runtime has looked at the state once — a fresh link changes nothing before its actor's first reconcile.
async fn reconciled(link: &PeerLink) {
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while *link.changes().borrow() == 0 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the runtime never reconciled"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The identity lock goes when the last holder of the stopped link's state lets go (its tasks end shortly after the stop).
async fn wait_until_unlocked(locks: &Locks) {
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while !locks.lock().unwrap().is_empty() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the identity lock was never released"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stored_helper_url_the_adapter_refuses_is_an_endpoint_error_never_a_fallback() {
    let dir = TempDir::new();
    let (a, _, _) = pc(&dir).await;
    a.add(bystander(97)).unwrap();
    wait_until(&a, "endpoint up", PATIENCE, is_up).await;
    a.stop().await;
    drop(a);
    // Re-sign a body carrying that URL, as a list saved before a stricter adapter would be.
    let secret = {
        let blob = std::fs::read(dir.key_file()).unwrap();
        Secret::from_bytes(<[u8; 32]>::try_from(&blob[1..]).unwrap())
    };
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.list_file()).unwrap()).unwrap();
    let mut body: serde_json::Value = serde_json::from_str(doc["body"].as_str().unwrap()).unwrap();
    body["network"]["relay"] = serde_json::Value::from(ADAPTER_REFUSES);
    let body = serde_json::to_string(&body).unwrap();
    let tag = mac::sign(&secret, &body).unwrap();
    doc["body"] = serde_json::Value::from(body);
    doc["mac"] = serde_json::Value::from(engram_dashboard_base::text::hex_lower(&tag));
    std::fs::write(dir.list_file(), serde_json::to_string(&doc).unwrap()).unwrap();

    let a = start(dir.path()).await;
    let failed = wait_until(&a, "endpoint error", PATIENCE, |s| {
        matches!(
            s.endpoint,
            EndpointView::Error(EndpointError::HelperRefused(_))
        )
    })
    .await;
    assert_eq!(failed.status, PeerStatus::Ready);
    assert_eq!(
        online_of(&failed, &PeerId::from_bytes([97; 32])),
        Some(Online::EndpointError)
    );
    // A change of the port alone keeps the refused URL — refused too.
    let network = a.snapshot().network.unwrap();
    let port_only = NetworkSettings {
        port: Some(free_port()),
        ..network.clone()
    };
    assert!(matches!(
        a.set_network(port_only),
        Err(PeerError::Invalid(_))
    ));
    assert_eq!(a.snapshot().network, Some(network.clone()));
    // Settings that pass bring it up.
    a.set_network(NetworkSettings {
        relay: Helper::None,
        ..network
    })
    .unwrap();
    wait_until(&a, "endpoint up", PATIENCE, is_up).await;
    a.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_start_clears_temp_files_a_crashed_write_left() {
    let dir = TempDir::new();
    let leftover = dir.path().join(format!(
        "peers.{ACCOUNT}.{MACHINE}.json.tmp{}.9",
        std::process::id()
    ));
    std::fs::write(&leftover, b"half").unwrap();
    let unrelated = dir.path().join("notes.txt.tmp1.1");
    std::fs::write(&unrelated, b"keep").unwrap();
    let a = start(dir.path()).await;
    assert!(!leftover.exists());
    assert!(unrelated.exists(), "other names are left alone");
    a.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_frame_is_checked_against_the_list_and_stray_answers_are_dropped() {
    let dir = TempDir::new();
    let (a, _, _) = pc(&dir).await;
    let listed = PeerId::from_bytes([98; 32]);
    let stranger = PeerId::from_bytes([99; 32]);
    a.add(bystander(98)).unwrap();
    wait_until(&a, "endpoint up", PATIENCE, is_up).await;
    let core = &a.core;
    let probe = || PeerMsg::Probe { nonce: 5 };

    assert_eq!(
        route(core, &listed, 1000, probe()),
        Route::Reply(PeerMsg::ProbeAck { nonce: 5 })
    );
    assert_eq!(route(core, &stranger, 1000, probe()), Route::NotTrusted);
    // A ProbeAck nobody asked for stays out of the actor's queue; one for the probe open on that link goes through.
    assert_eq!(
        route(core, &listed, 1000, PeerMsg::ProbeAck { nonce: 5 }),
        Route::Drop
    );
    let nonce = {
        let mut book = lock(&core.book);
        let now = std::time::Instant::now();
        book.link_up(
            listed,
            1000,
            Dir::Accepted,
            Instance::from_u128(1),
            false,
            now,
        );
        let asked = book.link_up(
            listed,
            1001,
            Dir::Accepted,
            Instance::from_u128(2),
            false,
            now,
        );
        asked
            .iter()
            .find_map(|a| match a {
                Action::Send {
                    msg: PeerMsg::Probe { nonce },
                    ..
                } => Some(*nonce),
                _ => None,
            })
            .unwrap()
    };
    // While the probe is open: a wrong nonce, or the right one on the other link, is dropped; the first right answer goes through,
    //   and repeats of it are dropped (one answer per probe reaches the actor).
    assert_eq!(
        route(
            core,
            &listed,
            1000,
            PeerMsg::ProbeAck {
                nonce: nonce.wrapping_add(1)
            }
        ),
        Route::Drop
    );
    assert_eq!(
        route(core, &listed, 1001, PeerMsg::ProbeAck { nonce }),
        Route::Drop
    );
    assert_eq!(
        route(core, &listed, 1000, PeerMsg::ProbeAck { nonce }),
        Route::Forward(LinkEvent::ProbeAck(nonce))
    );
    for _ in 0..3 {
        assert_eq!(
            route(core, &listed, 1000, PeerMsg::ProbeAck { nonce }),
            Route::Drop
        );
    }
    // A Duplicate warning is taken once; repeats change nothing.
    let duplicate = || PeerMsg::Duplicate {
        kind: DuplicateKind::Duplicate,
    };
    assert_eq!(route(core, &listed, 1000, duplicate()), Route::Drop);
    assert_eq!(
        lock(&core.book).warning(&listed),
        Some(Warning {
            kind: DuplicateKind::Duplicate,
            reported: true
        })
    );
    assert!(!lock(&core.book).duplicate_reported(&listed, DuplicateKind::Duplicate));
    // Revoked: the very next frame from it is refused.
    core.revoke(&listed);
    assert_eq!(route(core, &listed, 1000, probe()), Route::NotTrusted);
    a.stop().await;
}
