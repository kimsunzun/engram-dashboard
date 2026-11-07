//! Token handshake frame — the home of the shape · golden is protocol `handshake` (ADR-0310 decision 9). Here it is only
//! re-exported: until net is removed (TRD stage 3 3-3 U6), `ws`, which uses the `auth::AuthFrame` path, stays standing as is.
//!
//! ★The frame this crate parses is still this one alone★ — it merely does not own the shape; the property of not
//! knowing agent vocabulary even as a type (lib.rs header gate 4) is unchanged. Do not call protocol's command vocabulary outside the re-export.

pub use engram_dashboard_protocol::handshake::AuthFrame;
