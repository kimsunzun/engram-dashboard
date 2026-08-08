//! # engram-dashboard-core — 에이전트 코어 (Tauri import 0)
//!
//! Embedded(src-tauri)와 daemon(별도 bin)이 공통으로 의존하는 lib.
//! 출력/상태는 `agent::types::{OutputSink, StatusSink}` trait 으로만 흐른다(전송 방식 불가지).
//!
//! ## 격리 게이트(불변): `rg "^\s*use tauri" src/` → 0줄. (import 라인 앵커 — 이 주석 같은 자기 인용 오탐 방지)

pub mod agent;
pub mod logging;
pub mod persistence;
// ADR-0046
pub mod replay_flight;
