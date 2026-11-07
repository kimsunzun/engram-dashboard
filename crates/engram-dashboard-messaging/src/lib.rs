//! engram-dashboard-messaging — the S18 messaging kernel crate (ADR-0103/0104/0110).
//!
//! ★Role★: holds the policy "make messages between agents reliably go through while the daemon is alive".
//!
//! ★Complete mutual ignorance (load-bearing — ADR-0110 decision 2)★: this crate **depends on no crate in the
//! workspace** (agent included). The only external dependencies are `uuid` and `tracing`. So it **does not know, even as a type,** real host
//! objects like the agent manager, output sink and control registry — those names do not appear
//! in this crate at all (isolation grep gate), and the joints are cut only through contracts
//! (port traits) this crate owns: `service::DeliveryPort` (the real delivery) ·
//! `service::ControlPlanePort` (envelope format lookup · delivery observation record) · `service::FlushTrigger` (doorbell) ·
//! `busy::TurnFacts` (turn observation fact lookup) · `busy::IdleNotifier` (idle notification). The real adapters are owned by the host (daemon).
//! If you feel like adding a workspace dependency here, that is a sign "a port must be dug", not a reason to
//! break the wall — the compiler-enforced wall is the point of this structure (structure, not convention).
//!
//! ★Purity invariant (load-bearing — ADR-0104 seam isolation, ADR-0012)★: `mailbox`·`ledger`·`groups` do
//! not use tokio or threads (concurrency is owned by the upper service), **never call `Instant::now()`/
//! `SystemTime::now()` inside the logic** (time-dependent methods **take `now: Instant` as an
//! argument** — clock injection: TTL boundaries and timeouts can be asserted in deterministic unit tests, and the clock is manipulated by
//! pushing now by hand), and hold no global mutable state. If this invariant breaks, unit tests become
//! dependent on real-time sleeps (slow, flaky) and seam isolation collapses.
//!
//! ★Time type = `std::time::Instant`★: it is **internal data** not exposed in the envelope (spec §5 — the state-transition time is
//!   the reply/send time, but it is not exposed in the envelope), so a monotonic `Instant` is enough (if wall-clock display becomes needed, a
//!   `SystemTime` axis will be added non-breakingly in v2). TTL and reply-by are expressed as `Duration` offsets and compared with the injected now.
// ADR-0103
// ADR-0104
// ADR-0110

pub mod busy;
pub mod envelope;
pub mod groups;
pub mod ledger;
pub mod mailbox;
pub mod service;

/// ★Why the agent's `AgentId` is not used (load-bearing)★: the moment it is used this crate depends on a workspace crate
///   and complete ignorance breaks. The base type is the same `uuid::Uuid`, so the host (daemon) `AgentId` value
///   **passes through the boundary unconverted** — i.e. the cost of raising the wall is practically 0 (ADR-0110 basis: ~66 boundary
///   call sites compile unmodified). Even if non-agent participants (an external mail service · a person) appear in the future, this neutral name
///   holds as is.
pub type PeerId = uuid::Uuid;

/// Deterministic id for tests — the same name always gives the same id. It makes an agent relaunched under the same name read as 「같은 에이전트의
/// 새 화신」 (a new incarnation of the same agent) (ADR-0309 — the mail key is the id). Same-name entities that must be different do not use this.
/// It is an FNV-1a 128 using only std — bringing in the uuid `v5` feature would change the dependency ceiling (ADR-0110).
#[cfg(test)]
pub(crate) fn tid(name: &str) -> PeerId {
    const OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;
    let hash = name
        .bytes()
        .fold(OFFSET, |h, b| (h ^ u128::from(b)).wrapping_mul(PRIME));
    PeerId::from_u128(hash)
}

/// ★Why the daemon's `BoundIdentity` is not used★: that is a control-channel token binding concept (a product of host authentication), so
///   the messaging kernel has no reason to know it. The kernel uses only two fields, "who sent it and from which generation". The host
///   converts at the boundary with a 2-field copy (the daemon-side `From<BoundIdentity>`).
///
/// ★Why epoch is part of identity (load-bearing)★: even the same participant loses context when re-spawned. Reply
///   tracking and stale decisions must look at epoch together so a send from a dead generation is not mistaken for one from the live generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SenderIdentity {
    pub peer_id: PeerId,
    pub epoch: u32,
}
