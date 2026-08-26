//! # engram-dashboard-agent — 에이전트 코어 (Tauri import 0)
//!
//! Embedded(src-tauri)와 daemon(별도 bin)이 공통으로 의존하는 lib.
//! 출력/상태는 `types::{OutputSink, StatusSink}` trait 으로만 흐른다(전송 방식 불가지).
//!
//! ## 격리 게이트(불변): `rg "^\s*use tauri" src/` → 0줄. (import 라인 앵커 — 이 주석 같은 자기 인용 오탐 방지)

// ADR-0175: 옛 `agent/` 하위 모듈을 crate 루트로 접어 올렸다 — 경로에서 crate 이름과 모듈 이름이
//   `agent` 로 겹쳐 읽히던 것을 없앤다. 아래 첫 묶음이 그 이사분이다.
pub mod backend;
// ADR-0155
pub mod commands;
// ADR-0172
pub mod failure;
pub mod manager;
// ADR-0101
pub mod name;
pub mod output_core;
pub mod platform;
pub mod preset;
pub mod profile;
pub mod reaper;
pub mod session;
pub mod session_tracker;
pub mod transport;
pub mod turn;
pub mod types;

pub mod persistence;
