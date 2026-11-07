//! ts-rs binding generation. `cargo test --test ts_export` writes the .ts files into `bindings/`.
//! In phase 0 it exports only to the crate-local `bindings/` (connecting the frontend `src/api/generated/` is phase 1).
//!
//! ts-rs makes an export method for each #[ts(export)] type. export_all_to gathers them into one directory.

use engram_dashboard_protocol::{AgentCommand, AgentEvent, StructuredEvent};
use ts_rs::TS;

#[test]
fn export_typescript_bindings() {
    let out = concat!(env!("CARGO_MANIFEST_DIR"), "/bindings/");
    // Calling export_all_to on only the two top-level envelopes makes the transitive dependencies (AgentInfo/AgentStatus/OutputChunk/…)
    // all follow into the same directory.
    AgentCommand::export_all_to(out).expect("AgentCommand 바인딩 export 실패");
    AgentEvent::export_all_to(out).expect("AgentEvent 바인딩 export 실패");
    // ★ADR-0045 tag1★: StructuredEvent is not carried by any AgentEvent variant (it flows only as the binary frame tag1 payload
    //   — not a JSON envelope), so it does not follow in the transitive dependencies above. The frontend JSON.parses the tag1 payload and
    //   tells variants apart with this type, so the binding is exported separately (frontend consumer code is module 6 — here only the type is generated).
    StructuredEvent::export_all_to(out).expect("StructuredEvent 바인딩 export 실패");
}
