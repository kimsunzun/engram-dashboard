//! Test doubles for the peer module's callers: trust operations over an in-memory peer state ([`FakeOps`]) that record what was applied.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use super::approvals::{TrustOps, TrustSource};
use super::book::Online;
use super::gate::Knock;
use super::id::PeerId;
use super::runtime::{EndpointView, PeerRow, PeerSnapshot};
use super::store::{PeerEdit, PeerError, PeerStatus, Regenerated, Removal};
use super::trusted::{NetworkPatch, NetworkSettings, TrustedPeer};

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
                    online: Online::Offline,
                    warning: None,
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
