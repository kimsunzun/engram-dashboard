//! # engram-dashboard-core — 에이전트 코어 (Tauri import 0)
//!
//! src-tauri 에서 그대로 이동한 3개 모듈. 내부의 `crate::agent`/`crate::persistence`/
//! `crate::logging` 참조는 여기서도 top-level 모듈이라 무수정으로 유효하다.
//!
//! Embedded(src-tauri)와 미래 daemon(별도 bin)이 공통으로 의존하는 lib.
//! 출력/상태는 `agent::types::{OutputSink, StatusSink}` trait 으로만 흐른다(전송 방식 불가지).
//!
//! ## 격리 게이트(불변): `rg "use tauri" src/` → 0줄.

pub mod agent;
pub mod logging;
pub mod persistence;
// ADR-0046 M1: single-flight replay 채번/펜스 상태기계 + replay 경계 마커 인코딩(순수 — 소켓/Tauri/protocol
//   의존 0, agentId=uuid::Uuid). src-tauri 에서 이관 — 단위테스트를 headless(`cargo test -p …-core`)에서
//   실행하기 위함(src-tauri 테스트는 WebView2 DLL 로 이 환경 미실행). src-tauri 는 얇은 re-export 로 소비.
pub mod replay_flight;
