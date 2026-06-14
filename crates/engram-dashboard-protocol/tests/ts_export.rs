//! ts-rs 바인딩 생성. `cargo test --test ts_export` 로 `bindings/` 에 .ts 파일을 쓴다.
//! phase 0 에선 crate-local `bindings/` 로만 내보낸다(프론트 `src/api/generated/` 연결은 phase 1).
//!
//! ts-rs 는 #[ts(export)] 타입마다 export 메서드를 만든다. export_all_to 로 한 디렉토리에 모음.

use engram_dashboard_protocol::{AgentCommand, AgentEvent};
use ts_rs::TS;

#[test]
fn export_typescript_bindings() {
    let out = concat!(env!("CARGO_MANIFEST_DIR"), "/bindings/");
    // 최상위 두 envelope 만 export_all_to 하면 전이 의존(AgentInfo/AgentStatus/OutputChunk/…)이
    // 모두 같은 디렉토리로 따라온다.
    AgentCommand::export_all_to(out).expect("AgentCommand 바인딩 export 실패");
    AgentEvent::export_all_to(out).expect("AgentEvent 바인딩 export 실패");
}
