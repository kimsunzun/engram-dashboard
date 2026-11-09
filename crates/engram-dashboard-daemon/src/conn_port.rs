//! Connection port contract between the network layer and the daemon's upper layers.
//!
//! Upper layers name the contract only through this module, so swapping the network layer underneath does not touch them.
//! For now it re-exports net's `frame_port` as is; the contract's documentation lives at that definition.
// TRD S21 stage 3, 3-3 U1 (re-export) · U6 (becomes its own definition)

pub use engram_dashboard_net::frame_port::*;
