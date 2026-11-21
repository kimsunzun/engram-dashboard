//! The peer link — daemons of one user on different PCs (beside the shell link in `link/`): this PC's identity and the trusted list
//!   ([`id`], [`identity`], [`trusted`], [`mac`], [`os`], [`store`]), the PeerWire vocabulary ([`wire`]), the `Hello` handshake
//!   ([`handshake`]), the per-PC link book — tie-break, re-arm, duplicate identity ([`book`]) — the refused-knock list ([`gate`]) and the
//!   running link ([`runtime::PeerLink`]: trust operations, endpoint lifecycle, accept gate, registry). Approvals, the `peer.*` commands,
//!   mail and the directory build on [`runtime::PeerLink`].
//!
//! ★Link adapter types never come here★ — the key scheme is reached only through transport's raw-bytes `iroh` functions and the endpoint
//!   is built from plain values, so a replaced adapter keeps every stored ID.

// ADR-0388, ADR-0389

pub(crate) mod book;
pub(crate) mod gate;
pub(crate) mod handshake;
pub(crate) mod id;
pub(crate) mod identity;
pub(crate) mod mac;
pub(crate) mod os;
pub(crate) mod runtime;
pub(crate) mod store;
pub(crate) mod trusted;
pub(crate) mod wire;
