//! Test doubles for the peer module's callers: trust operations over an in-memory peer state ([`FakeOps`]) that record what was applied.

use std::collections::{HashMap, HashSet};
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use engram_dashboard_platform::secret::UnsealError;
use engram_dashboard_transport::SystemClock;

use super::approvals::{TrustOps, TrustSource};
use super::book::Online;
use super::gate::Knock;
use super::id::PeerId;
use super::os::{LockAttempt, PeerOs};
use super::runtime::{EndpointView, Host, LinkSettings, PeerLink, PeerRow, PeerSnapshot};
use super::store::{PeerEdit, PeerError, PeerStatus, PeerStore, Regenerated, Removal};
use super::trusted::{Helper, NetworkPatch, NetworkSettings, Permission, TrustedPeer};
use super::wire::DirAgent;

/// A PC ID from one repeated byte.
pub(crate) fn pc(byte: u8) -> PeerId {
    PeerId::from_bytes([byte; 32])
}

/// The peer state the fake holds.
#[derive(Debug, Clone)]
pub(crate) struct FakeState {
    pub(crate) status: PeerStatus,
    pub(crate) id: Option<PeerId>,
    pub(crate) peers: Vec<TrustedPeer>,
    pub(crate) network: NetworkSettings,
    pub(crate) endpoint: EndpointView,
    pub(crate) knocks: Vec<Knock>,
    /// The next list save is refused.
    pub(crate) save_fails: bool,
    /// The settings are not known (the list file could not be read) — the snapshot has none.
    pub(crate) network_unknown: bool,
    /// Link states other than offline, per PC.
    pub(crate) online: HashMap<PeerId, Online>,
    /// Directory copies, per PC (none = unknown).
    pub(crate) agents: HashMap<PeerId, Vec<DirAgent>>,
}

impl Default for FakeState {
    fn default() -> Self {
        Self {
            status: PeerStatus::Ready,
            id: Some(pc(0xAA)),
            peers: Vec::new(),
            network: NetworkSettings::default(),
            endpoint: EndpointView::Off,
            knocks: Vec::new(),
            save_fails: false,
            network_unknown: false,
            online: HashMap::new(),
            agents: HashMap::new(),
        }
    }
}

/// Trust operations over [`FakeState`]; every applied operation is recorded as a word.
#[derive(Default)]
pub(crate) struct FakeOps {
    pub(crate) state: Mutex<FakeState>,
    calls: Mutex<Vec<String>>,
    /// Set: `add` waits for a message on it before it applies.
    pub(crate) hold_add: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    /// An `add` is waiting on `hold_add`.
    pub(crate) add_entered: std::sync::atomic::AtomicBool,
}

impl FakeOps {
    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }

    pub(crate) fn with(&self, change: impl FnOnce(&mut FakeState)) {
        change(&mut self.state.lock().unwrap());
    }
}

/// A started peer link holding `ops` — the [`TrustSource`] the approvals read.
pub(crate) fn source(ops: Arc<FakeOps>) -> Arc<OnceLock<Arc<FakeOps>>> {
    let slot = Arc::new(OnceLock::new());
    let _ = slot.set(ops);
    slot
}

impl TrustOps for Arc<FakeOps> {
    fn snapshot(&self) -> PeerSnapshot {
        let state = self.state.lock().unwrap();
        PeerSnapshot {
            status: state.status.clone(),
            id: state.id,
            lock_note: None,
            network: (!state.network_unknown).then(|| state.network.clone()),
            endpoint: state.endpoint.clone(),
            peers: state
                .peers
                .iter()
                .map(|peer| PeerRow {
                    peer: peer.clone(),
                    online: state
                        .online
                        .get(&peer.id)
                        .copied()
                        .unwrap_or(Online::Offline),
                    warning: None,
                    agents: state.agents.get(&peer.id).cloned(),
                })
                .collect(),
            knocks: state.knocks.clone(),
        }
    }

    fn ensure_identity(&self) -> Result<PeerId, PeerError> {
        self.record("ensureIdentity".to_string());
        let mut state = self.state.lock().unwrap();
        if state.status == PeerStatus::NoIdentityYet {
            state.status = PeerStatus::Ready;
            state.id = Some(pc(0xAA));
        }
        state
            .id
            .ok_or_else(|| PeerError::Blocked("no identity".to_string()))
    }

    fn add(&self, peer: TrustedPeer) -> Result<(), PeerError> {
        // A test can hold the add on this thread (an answer being applied).
        if let Some(release) = self.hold_add.lock().unwrap().as_ref() {
            self.add_entered
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let _ = release.recv();
        }
        self.record(format!("add {}", peer.name));
        self.state.lock().unwrap().peers.push(peer);
        Ok(())
    }

    fn edit(&self, id: &PeerId, edit: PeerEdit) -> Result<(), PeerError> {
        self.record(format!("edit {id}"));
        let mut state = self.state.lock().unwrap();
        let peer = state
            .peers
            .iter_mut()
            .find(|p| p.id == *id)
            .ok_or(PeerError::NotFound(*id))?;
        if let Some(name) = edit.name {
            peer.name = name;
        }
        if let Some(address) = edit.address {
            peer.address = address;
        }
        Ok(())
    }

    fn remove(&self, id: &PeerId) -> Result<Removal, PeerError> {
        self.record(format!("remove {id}"));
        let mut state = self.state.lock().unwrap();
        let at = state
            .peers
            .iter()
            .position(|p| p.id == *id)
            .ok_or(PeerError::NotFound(*id))?;
        let removed = state.peers.remove(at);
        let saved = if state.save_fails {
            Err(PeerError::Io("the disk is full".to_string()))
        } else {
            Ok(())
        };
        Ok(Removal { removed, saved })
    }

    fn patch_network(&self, patch: &NetworkPatch) -> Result<NetworkSettings, PeerError> {
        self.record("setNetwork".to_string());
        let mut state = self.state.lock().unwrap();
        let before = state.network.clone();
        let merged = patch.applied_to(&before);
        merged.check_for_endpoint().map_err(PeerError::Invalid)?;
        state.network = merged;
        Ok(before)
    }

    fn regenerate(&self) -> Result<Regenerated, PeerError> {
        self.record("regenerate".to_string());
        let mut state = self.state.lock().unwrap();
        let old = state.id;
        state.id = Some(pc(0xBB));
        state.status = PeerStatus::ListChangedOutside;
        Ok(Regenerated {
            old,
            id: pc(0xBB),
            backup: None,
        })
    }

    fn accept_list(&self) -> Result<(), PeerError> {
        self.record("acceptList".to_string());
        self.state.lock().unwrap().status = PeerStatus::Ready;
        Ok(())
    }

    fn adopt(&self, machine: &str) -> Result<(), PeerError> {
        self.record(format!("adopt {machine}"));
        let mut state = self.state.lock().unwrap();
        state.status = PeerStatus::Ready;
        state.id = Some(pc(0xCC));
        Ok(())
    }

    fn start_fresh(&self) -> Result<PeerId, PeerError> {
        self.record("startFresh".to_string());
        let mut state = self.state.lock().unwrap();
        state.status = PeerStatus::Ready;
        state.id = Some(pc(0xDD));
        Ok(pc(0xDD))
    }
}

/// A refused knock now.
pub(crate) fn knock(id: PeerId) -> Knock {
    Knock {
        id,
        count: 2,
        first_at: SystemTime::now(),
        last_at: SystemTime::now(),
    }
}

/// Nothing started yet — the approvals' source before the peer link lands.
pub(crate) fn not_started() -> Arc<dyn TrustSource> {
    Arc::new(OnceLock::<Arc<FakeOps>>::new())
}

// ── a peer link in one process (shared by the runtime tests and the mail host tests) ──

pub(crate) const ACCOUNT: &str = "S-1-5-21-10-20-30-1001";
pub(crate) const MACHINE: &str = "aaaa-1111";

pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub(crate) fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("engram-peer-link-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }

    pub(crate) fn key_file(&self) -> PathBuf {
        self.0.join(format!("peer-key.{ACCOUNT}.{MACHINE}"))
    }

    pub(crate) fn list_file(&self) -> PathBuf {
        self.0.join(format!("peers.{ACCOUNT}.{MACHINE}.json"))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(crate) type Locks = Arc<Mutex<HashSet<String>>>;

/// The OS fake: a sealed blob is `T` + the secret; unsealing can be made to fail transiently.
pub(crate) struct FakeOs {
    pub(crate) locks: Locks,
    pub(crate) transient: Arc<AtomicBool>,
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

/// Starts a peer link over a temp folder with the OS behind [`FakeOs`], bound to 127.0.0.1 only.
pub(crate) async fn start_test_link(
    dir: &Path,
    locks: &Locks,
    transient: &Arc<AtomicBool>,
    host: Option<Host>,
) -> PeerLink {
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
        host,
    )
    .await
    .unwrap()
}

pub(crate) fn free_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Network settings that contact nothing but the typed addresses.
pub(crate) fn quiet(port: u16) -> NetworkSettings {
    NetworkSettings {
        port: Some(port),
        relay: Helper::None,
        address_lookup: Helper::None,
        mdns: false,
    }
}

/// A started PC with its key made and its helpers off — its ID and the address it will listen on.
pub(crate) async fn test_pc(dir: &TempDir, host: Option<Host>) -> (PeerLink, PeerId, SocketAddr) {
    let link = start_test_link(dir.path(), &Locks::default(), &Arc::default(), host).await;
    let port = free_port();
    link.set_network(quiet(port)).unwrap();
    let id = link.ensure_identity().unwrap();
    (link, id, SocketAddr::from(([127, 0, 0, 1], port)))
}

pub(crate) fn entry(id: PeerId, name: &str, address: Option<SocketAddr>) -> TrustedPeer {
    TrustedPeer {
        id,
        name: name.to_string(),
        address,
        permission: Permission::Mail,
    }
}
