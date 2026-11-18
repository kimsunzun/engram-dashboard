//! ts-rs binding generation: `cargo test --test ts_export` writes the .ts files into the crate-local `bindings/` (the frontend
//!   `src/api/generated/` is not wired yet).
//! `export_all_to` gathers a type and its transitive dependencies into one directory.

use engram_dashboard_protocol::{AgentCommand, AgentEvent, StructuredEvent};
use ts_rs::TS;

#[test]
fn export_typescript_bindings() {
    let out = concat!(env!("CARGO_MANIFEST_DIR"), "/bindings/");
    // The two top-level envelopes pull in their transitive dependencies (AgentInfo/AgentStatus/OutputChunk/…).
    AgentCommand::export_all_to(out).expect("AgentCommand binding export failed");
    AgentEvent::export_all_to(out).expect("AgentEvent binding export failed");
    // ★ADR-0045 tag1★: no AgentEvent variant carries StructuredEvent (it flows only as the binary tag1 payload), so it is not a transitive
    //   dependency; the frontend
    // tells payload variants apart with it, so export it separately.
    StructuredEvent::export_all_to(out).expect("StructuredEvent binding export failed");
}
