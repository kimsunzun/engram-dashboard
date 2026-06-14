//! # engram-dashboard-core — 에이전트 코어 (Tauri import 0)
//!
//! src-tauri 에서 그대로 이동한 3개 모듈. 내부의 `crate::pty`/`crate::persistence`/
//! `crate::logging` 참조는 여기서도 top-level 모듈이라 무수정으로 유효하다.
//!
//! Embedded(src-tauri)와 미래 daemon(별도 bin)이 공통으로 의존하는 lib.
//! 출력/상태는 `pty::types::{OutputSink, StatusSink}` trait 으로만 흐른다(전송 방식 불가지).
//!
//! ## 격리 게이트(불변): `rg "use tauri" src/` → 0줄.

pub mod logging;
pub mod persistence;
pub mod pty;
