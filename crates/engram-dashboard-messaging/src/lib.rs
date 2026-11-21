//! engram-dashboard-messaging — the S18 messaging kernel crate (ADR-0103/0104/0110).
//!
//! ★Role★: the policy "make messages between agents reliably go through while the daemon is alive".
//! ★Complete mutual ignorance (load-bearing — ADR-0110 decision 2)★: this crate **depends on no workspace crate** (agent included);
//!   external deps are `uuid` and `tracing`. Host objects (agent manager, output sink,
//! control registry) do not appear in it at all (isolation grep gate); joints are cut only through contracts it owns:
//!   `service::DeliveryPort` (real delivery), `service::ControlPlanePort` (envelope format lookup,
//! delivery observation), `service::FlushTrigger` (doorbell), `busy::TurnFacts` (turn observation), `busy::IdleNotifier` (idle
//!   notification), `service::RemoteDeliveryPort` (mail to other PCs); real adapters are the host's (daemon). Feeling the urge to add a
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
//!
//! ★Remote branch (S37 — mail to agents on the user's other PCs)★: the kernel resolves addresses across PCs from the one
//!   `AddressingSources` snapshot (it reads the directory copies the host keeps; it does not own or sync them), hands remote rows to
//! `service::RemoteDeliveryPort` (non-blocking, outside the lock) and moves them only through the verdict entry; inbound mail enters
//!   through the remote entrance (validation, per-PC backstop, local-only resolution — no forwarding). The link, the wire vocabulary
//! and the directory sync are the host's. A PC is known here only as an opaque `NodeRef`.
// ADR-0103, ADR-0104, ADR-0110
// ADR-0390, ADR-0391, ADR-0393

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
/// ★`node` = the PC the sender is on (`None` = this PC)★: a display / routing qualifier only — keys stay agent ids (ids are unique across
///   PCs), so no ledger or mailbox key carries it. The remote entrance fills it from the authenticated link, never from the frame.
// ADR-0393
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SenderIdentity {
    pub peer_id: PeerId,
    pub epoch: u32,
    pub node: Option<NodeRef>,
}

/// ★One PC (daemon) as the kernel sees it — the 32 raw bytes of its node key, opaque★. A fixed-size value, not a `String`, so
///   `SenderIdentity` stays `Copy`; not a uuid alias — the node identity is a key, not an agent id. The ID text format is the host's.
// ADR-0393
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeRef([u8; 32]);

impl NodeRef {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Lowercase hex of the key — the kernel's own encoding (park payload, logs); not the host's ID text.
    pub(crate) fn to_hex(self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub(crate) fn from_hex(s: &str) -> Option<Self> {
        if s.len() != 64 || !s.is_ascii() {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
        }
        Some(Self(out))
    }
}

impl std::fmt::Debug for NodeRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NodeRef({})", self.to_hex())
    }
}
