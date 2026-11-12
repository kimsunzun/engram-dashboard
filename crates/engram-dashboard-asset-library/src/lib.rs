//! # engram-dashboard-asset-library — the preset library
//!
//! The library holds instruction, skill, settings and preset files under one root folder. This crate reads, checks and
//! resolves them and makes the launch materials; the daemon creates the service, plugs its ports in and passes it values,
//! nothing more (ADR-0311).
//!
//! ## Dependency rule (invariant)
//!
//! - Among workspace crates exactly agent · platform · command · base (ADR-0311 decision 2). agent defines the ports this
//!   crate implements or calls; agent does not know this crate. This crate calls no daemon internals.
//! - No test feature (ADR-0311 decision 4): fakes are `#[cfg(test)]` inside the crate.
//! - No network client, no process runner — settings are not schema-checked, so nothing is fetched or probed
//!   (ADR-0359).
//! - No OS `cfg`: OS behaviour comes from platform (the platform gates 4 · 6 file lists do not name this crate).
//!
//! ## Backend extensibility (invariant — TRD S25 step-7 §1-1)
//!
//! Adding a backend to agent (its `AgentCommand` variant, backend word and settings check in the registry) must make
//! tags, raw settings fields, the selection of what an agent loads, the hand-over checks and the notices work with no
//! change here. So:
//!
//! - A backend tag and a raw-field name are strings, accepted only when the registry knows the word (ASCII
//!   case-insensitive). There is no backend enum here.
//! - The registry is reached only through [`backends::Backends`]; [`backends::AgentBackends`] is the only code here that
//!   calls agent's settings-check registry.
//! - Every per-backend difference comes from a port answer; the code never branches on a backend word, and no backend
//!   name appears in the source outside tests, doc comments describing today's port answers and built-in content.
//!
//! ## Gates
//!
//! `.github/workflows/ci.yml` holds the commands and expected values (canonical — not copied here), as steps
//! `asset-library gate 1`–`4` (ADR-0311 decision 6 numbering; gate 3 is the shared messaging step): ① the test line
//! `cargo test -p engram-dashboard-asset-library` · ② the direct workspace dependency name set {agent · asset-library ·
//! base · command · platform} and no direct `tauri*` package · ③ `asset_library` in the messaging isolation regex ·
//! ④ zero `use tauri` lines under `src/`.
//!
//! Nothing calls this crate yet — the daemon wiring is step 8.
// ADR-0311

pub mod backends;
pub mod catalog;
pub mod excerpt;
pub mod format;
pub mod path;
pub mod refs;
pub mod store;
pub mod validate;
