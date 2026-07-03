//! ts-rs 바인딩 생성. `cargo test --test ts_export` 로 `bindings/` 에 .ts 파일을 쓴다.
//! phase 0 에선 crate-local `bindings/` 로만 내보낸다(프론트 `src/api/generated/` 연결은 phase 1).
//!
//! ts-rs 는 #[ts(export)] 타입마다 export 메서드를 만든다. export_all_to 로 한 디렉토리에 모음.

use engram_dashboard_protocol::{AgentCommand, AgentEvent, StructuredEvent};
use ts_rs::TS;

#[test]
fn export_typescript_bindings() {
    let out = concat!(env!("CARGO_MANIFEST_DIR"), "/bindings/");
    // 최상위 두 envelope 만 export_all_to 하면 전이 의존(AgentInfo/AgentStatus/OutputChunk/…)이
    // 모두 같은 디렉토리로 따라온다.
    AgentCommand::export_all_to(out).expect("AgentCommand 바인딩 export 실패");
    AgentEvent::export_all_to(out).expect("AgentEvent 바인딩 export 실패");
    // ★ADR-0045 tag1★: StructuredEvent 는 어느 AgentEvent variant 에도 안 실려(binary frame tag1 payload
    //   로만 흐름 — JSON envelope 아님) 위 전이 의존에 안 따라온다. 프론트가 tag1 payload 를 JSON.parse
    //   후 이 타입으로 판별하므로 바인딩을 별도 export 한다(프론트 소비 코드는 모듈6 — 여기선 타입만 생성).
    StructuredEvent::export_all_to(out).expect("StructuredEvent 바인딩 export 실패");
}
