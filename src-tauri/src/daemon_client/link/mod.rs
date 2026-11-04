//! The shell's daemon link on the transport crate (ADR-0177 · ADR-0310 · TRD S21 trd-3 §6-2 U2).
//!
//! This module is the shell's edge of the transport: the packet vocabulary ([`wire::ShellWire`]) and, in later
//! units, the handshake · dialer · endpoint source · event routing. The connection machine itself (dial · backoff ·
//! keepalive · pending replies · per-peer lossless inbox) is the transport crate's; this module only adapts it.
//!
//! ★Nothing in production calls this yet★ — 3-2 U3 builds a new driver on it behind a constructor choice and 3-2 U4
//! switches production over. Items are `pub` so the unused stretch needs no lint allowances.
//!
//! ★Vocabulary wall★ — no `tauri` path and no agent crate in this directory (CI `shell gate 4`). Frontend emits reach
//! this module only through the ports in the parent module (`events::DaemonEvents` · `AgentNamesSink`), never as
//! Tauri types.
// ADR-0310

pub mod wire;

use engram_dashboard_transport::PeerId;

/// Which daemon a link talks to — the shell's key for one daemon in the transport registry (D4 ① — the shell is
/// built for N daemons and runs one in stage 3).
///
/// It is a label the shell chooses, not a daemon identity: two keys may point at the same daemon, and the same key
/// may reach a different daemon after a restart (TRD §3-2 — `DaemonId` is 3-3 U7).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DaemonKey(PeerId);

impl DaemonKey {
    pub fn new(name: impl AsRef<str>) -> Self {
        Self(PeerId::new(name))
    }

    /// The registry label this key uses.
    pub fn peer_id(&self) -> &PeerId {
        &self.0
    }
}

/// Names one `Registry::add` of a key. ★Intended contract — not enforced yet★: the event routing (3-2 U2 commit C)
/// mints one per add and then makes the field private behind a constructor; until then nothing mints tokens and the
/// field is open.
///
/// ★Why the shell will mint it★: transport has no handle identity on `Peer`, and re-adding the same key starts a new
/// supervisor while the old one may still drain its own inbox (`registry.rs` — overlap is not zero). Events are to be
/// filtered by "is this the key's current token" (TRD §3-3 rule ①), so the token must differ for every add of one key.
/// Order carries no meaning — only equality does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HandleToken(pub u64);
