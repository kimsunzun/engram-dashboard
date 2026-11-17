//! # engram-dashboard-agent — agent core (zero Tauri imports)
//!
//! The lib that Embedded (src-tauri) and the daemon (separate bin) both depend on.
//! Output/status flow only through the `types::{OutputSink, StatusSink}` traits (agnostic of the transport method).
//!
//! ## Isolation gate (invariant): `rg "^\s*use tauri" src/` → 0 lines. (anchored on import lines — prevents false hits on self-quotes like this comment)

// ADR-0175: the old `agent/` submodules were folded up into the crate root — removes the double `agent` reading of the crate name and
//   the module name in paths. The first group below is what moved.
pub mod backend;
// ADR-0155
pub mod commands;
// ADR-0172
pub mod failure;
// ADR-0231
pub mod inputs_pending;
pub mod manager;
// ADR-0101
pub mod name;
pub mod output_core;
pub mod preset;
pub mod profile;
// ADR-0231
pub mod queued_input;
pub mod reaper;
pub mod session;
// ADR-0226
pub(crate) mod session_id_latch;
pub mod session_tracker;
pub mod transport;
pub mod turn;
pub mod types;

pub mod persistence;
pub mod usage;
