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
//! [`peer`] is the separate daemon ↔ daemon link (its own head) — it shares nothing with the shell link above.
// ADR-0310, ADR-0389

mod auth;
mod handshake;
// The daemon starts and stops the peer link; its trust operations, snapshot and the wire's mail and directory frames wait for their callers
//   (the `peer.*` commands, mail and the directory — the next pieces).
#[allow(dead_code)]
pub(crate) mod peer;
pub(crate) mod server;
mod wire;
