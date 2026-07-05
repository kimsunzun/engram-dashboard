//! single-flight replay 상태기계 — **core 로 이관된 순수 머신의 얇은 re-export**(ADR-0046 M1, FIX-5).
//!
//! 실제 로직·불변식·단위테스트는 `engram_dashboard_core::replay_flight` 에 산다(소켓/Tauri/protocol 의존 0,
//! agentId=`uuid::Uuid`). 이 파일은 connection.rs 의 `super::replay_flight::…` 경로를 그대로 유지하는
//! adapter 일 뿐이라 배선 변경이 없다. 왜 core 로 옮겼나: src-tauri 테스트는 이 환경에서 WebView2 DLL 로
//! 실행되지 않아(STATUS_ENTRYPOINT_NOT_FOUND) 상태기계 회귀가 안 돌았다 — core 로 옮겨
//! `cargo test -p engram-dashboard-core`(headless)에서 **실행**되게 한다.
//!
//! ★타입 통과★: src-tauri 의 `AgentId` 는 `uuid::Uuid` 의 alias 라, core API 의 `Uuid` 시그니처에 그대로
//! 전달된다(`.as_bytes()` 표현 보존). core 는 protocol 을 런타임 의존하지 않는다(ADR-0003 격리).

pub use engram_dashboard_core::replay_flight::{
    encode_marker_frame, Marker, ReplayFlightSet, RequestOutcome, Resolution, MARKER_FRAME_LEN,
    MARKER_TAG,
};
