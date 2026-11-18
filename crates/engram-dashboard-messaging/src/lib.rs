//! engram-dashboard-messaging — the S18 messaging kernel crate (ADR-0103/0104/0110).
//!
//! ★Role★: the policy "make messages between agents reliably go through while the daemon is alive".
//! ★Complete mutual ignorance (load-bearing — ADR-0110 decision 2)★: this crate **depends on no workspace crate** (agent included);
//!   external deps are `uuid` and `tracing`. Host objects (agent manager, output sink,
//! control registry) do not appear in it at all (isolation grep gate); joints are cut only through contracts it owns:
//!   `service::DeliveryPort` (real delivery), `service::ControlPlanePort` (envelope format lookup,
//! delivery observation), `service::FlushTrigger` (doorbell), `busy::TurnFacts` (turn observation), `busy::IdleNotifier` (idle
//!   notification); real adapters are the host's (daemon). Feeling the urge to add a
//! workspace dependency means "a port must be dug", not "break the wall" — the compiler-enforced wall is the point.
//!
//! ★Purity invariant (load-bearing — ADR-0104 seam isolation, ADR-0012)★: `mailbox`·`ledger`·`groups` use no tokio or threads (concurrency
//!   is the upper service's), **never call `Instant::now()`/`SystemTime::now()`
//! in logic** (time-dependent methods take `now: Instant` — clock injection, so TTL and timeouts are asserted deterministically) and hold
//!   no global mutable state; breaking it makes tests depend on real sleeps
//! (slow, flaky) and collapses seam isolation.
//! ★Time type = `std::time::Instant`★: internal data not exposed in the envelope (spec §5), so a monotonic `Instant` suffices (wall-clock
//!   display, if needed, adds a `SystemTime` axis non-breakingly in v2); TTL and
//! reply-by are `Duration` offsets compared with the injected now.
// ADR-0103, ADR-0104, ADR-0110

pub mod busy;
pub mod envelope;
pub mod groups;
pub mod ledger;
pub mod mailbox;
pub mod service;

/// ★Why not the agent's `AgentId` (load-bearing)★: using it would make this crate depend on a workspace crate and break complete
///   ignorance. The base type is the same `uuid::Uuid`, so the host `AgentId` value passes the boundary
///   unconverted (ADR-0110: ~66 call sites compile unmodified); non-agent participants (external mail service, a person) fit this neutral
///   name too.
pub type PeerId = uuid::Uuid;

/// Deterministic id for tests — the same name always gives the same id, so an agent relaunched under one name reads as "a new incarnation
///   of the same agent" (ADR-0309 — the mail key is the id); same-name entities that must differ
/// do not use this. FNV-1a 128 using only std — the uuid `v5` feature would change the dependency ceiling (ADR-0110).
#[cfg(test)]
pub(crate) fn tid(name: &str) -> PeerId {
    const OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;
    let hash = name
        .bytes()
        .fold(OFFSET, |h, b| (h ^ u128::from(b)).wrapping_mul(PRIME));
    PeerId::from_u128(hash)
}

/// ★Why not the daemon's `BoundIdentity`★: that is a control-channel token binding (a product of host authentication) the kernel has no
///   reason to know; it uses only "who sent it and from which generation", and the host converts
///   with a 2-field copy (daemon-side `From<BoundIdentity>`). ★Why epoch is part of identity (load-bearing)★: a re-spawned participant
///   loses context; reply tracking and stale decisions must see epoch so a send from a dead
///   generation is not mistaken for the live one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SenderIdentity {
    pub peer_id: PeerId,
    pub epoch: u32,
}
