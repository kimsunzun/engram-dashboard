pub mod backend;
pub mod manager;
// ADR-0101: canonical 표시명 파생(cwd basename) — 라우팅·로스터·봉투 sender·프론트 트리가 공유하는
//   단일 규칙(WYSIWYA). 프론트 src/util/basename.ts 의 Rust 미러.
pub mod name;
pub mod output_core;
pub mod platform;
pub mod preset;
pub mod profile;
pub mod reaper;
pub mod session;
pub mod session_tracker;
pub mod transport;
pub mod types;
