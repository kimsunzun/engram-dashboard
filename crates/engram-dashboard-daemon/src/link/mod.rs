//! The daemon's shell link on the transport crate (ADR-0177 · ADR-0310 · TRD S21 trd-3 §6-3 U4).
//!
//! This module is the daemon's edge of the transport: the packet vocabulary ([`wire::DaemonWire`] — raw pass-through),
//! the first-frame auth decision ([`auth`]), the handshake ([`handshake`]) and the accepting side that implements the
//! `conn_port` contract over accepted peers ([`server`]). The connection machine itself (WS upgrade · Origin · Host ·
//! auth deadline · single writer · keepalive · lossless per-peer inbox) is the transport crate's.
//!
//! ★Vocabulary wall★ — the agent vocabulary reaches this module only through the `conn_port` contract
//! (`ConnectionHandlerFactory::handshake_error_frame` · the handler calls); nothing here decodes a command.
//!
//! [`peer`] is the separate daemon ↔ daemon link (its own head; so far identity and trusted list only) — it shares nothing with the
//! shell link above.
// ADR-0310, ADR-0389

mod auth;
mod handshake;
// Nothing outside its tests calls it yet — the peer link's endpoint and commands are the next pieces to use it.
#[allow(dead_code)]
pub(crate) mod peer;
pub(crate) mod server;
mod wire;
