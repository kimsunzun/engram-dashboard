//! Experiment infrastructure — the pure logic of the ADR-0090 Stage 2 context saturation pilot.
//!
//! ## Role
//! Holds **all the pure logic** that can be unit-tested without a real claude: deterministic filler generation (`filler`) · probe
//! grading + compaction detection (`probe`) · JSONL records + raw stream-json parser (`record`) · CLI parsing (`cli`) ·
//! transcript tap parser (`transcript` — best-effort experiment measurement, respecting the ADR-0008 boundary).
//! The driver loop that launches a real claude is performed by `bin/saturation_pilot.rs` (thin), which assembles these modules.
//!
//! ## Gating (ADR-0090 invariant)
//! **All modules are behind `#[cfg(feature = "test-harness")]`.** A production build does not compile this code at all
//! — if experiment code flows into a release artifact it violates ADR-0090 (the same family as the existing obs_seam / test-harness
//! feature). The bin (`saturation-pilot`) also has `required-features = ["test-harness"]`, so a normal
//! workspace build touches neither this module nor the bin.
// ADR-0090

pub mod cli;
pub mod filler;
pub mod probe;
pub mod record;
pub mod transcript;
