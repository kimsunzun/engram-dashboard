//! The shell's daemon link on the transport crate (ADR-0177 · ADR-0310 · TRD S21 trd-3 §6-2 U2): the shell's edge of the transport — the packet vocabulary ([`wire::ShellWire`]), the handshake ([`handshake::ShellHandshake`]), the dialer ([`dialer::LocalDaemonDialer`]), the endpoint source ([`endpoint::EndpointSource`]), the check of the listener's OS account before the token goes out ([`account`]), the inbox consumer ([`pump`]), the routing of what it delivers ([`route`]), the failure texts callers see ([`errors`]) and the one-shot stop client ([`stop`]). The connection machine itself (dial · backoff · keepalive · pending replies · per-peer lossless inbox) is the transport crate's; this module only adapts it. The shell's connection driver (`daemon_client::connection`) runs on it in every build; the one item whose signature names a crate-private port (`route::emit_broadcast` — `events::DaemonEvents`) is `pub(crate)`.
//! Vocabulary wall: no `tauri` path and no agent crate in this directory (CI `shell gate 4`); frontend emits reach this module only through the ports in the parent module (`events::DaemonEvents` · `AgentNamesSink`), never as Tauri types.
// ADR-0310

pub mod account;
pub mod dialer;
pub mod endpoint;
pub mod errors;
pub mod handshake;
pub mod pump;
pub mod route;
pub mod stop;
#[cfg(test)]
pub(crate) mod test_log;
pub mod wire;

use std::sync::atomic::{AtomicU64, Ordering};

use engram_dashboard_transport::PeerId;

/// Which daemon a link talks to — the shell's key for one daemon in the transport registry (D4 ① — built for N daemons, one runs in stage 3). A label the shell chooses, not a daemon identity: two keys may point at one daemon, and one key may reach a different daemon after a restart (TRD §3-2 — `DaemonId` is 3-3 U7).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DaemonKey(PeerId);

impl DaemonKey {
    pub fn new(name: impl AsRef<str>) -> Self {
        Self(PeerId::new(name))
    }

    pub fn peer_id(&self) -> &PeerId {
        &self.0
    }
}

/// Names one `Registry::add` of a key: mint one per add ([`HandleToken::mint`]) — no two mints in a process are equal, and the field is private, so a token cannot be made up. The shell mints it because transport has no handle identity on `Peer`, and re-adding the same key starts a new supervisor while the old one may still drain its own inbox (`registry.rs` — overlap is not zero); items are filtered by "is this the key's current token" (TRD §3-3 rule ① — [`route`]), so the token must differ for every add of one key. Order carries no meaning — only equality does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HandleToken(u64);

impl HandleToken {
    pub fn mint() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}
