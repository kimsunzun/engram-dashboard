//! The peer link — daemons of one user on different PCs (beside the shell link in `link/`). So far it holds this PC's identity and the
//!   trusted list: the ID text format ([`id`]), the sealed identity key on disk ([`identity`]), the trusted list file with its MAC
//!   ([`trusted`], [`mac`]), the OS seam ([`os`]) and the one state object the rest reads and changes ([`store`]). No network yet: the
//!   wire, the links, approvals and the `peer.*` commands build on [`store::PeerStore`].
//!
//! ★Link adapter types never come here★ — the key scheme is reached only through transport's raw-bytes `iroh::public_key`, so a replaced
//!   adapter keeps every stored ID.

// ADR-0389

pub(crate) mod id;
pub(crate) mod identity;
pub(crate) mod mac;
pub(crate) mod os;
pub(crate) mod store;
pub(crate) mod trusted;
