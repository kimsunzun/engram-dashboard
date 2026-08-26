//! 플랫폼별 프로세스 그룹 정리(Windows Job Object).
//!
//! PID liveness 헬퍼는 여기 없다 — 소비자가 이 crate 밖에 셋이라 `engram-dashboard-base` 의
//! `platform` 으로 이사했다(ADR-0175 결정 1). 여기 남은 Job Object 래퍼는 소비자가 `transport::pty`·
//! `transport::stdio` 둘뿐이라 그 조건을 못 채운다.

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::JobObjectHandle;
