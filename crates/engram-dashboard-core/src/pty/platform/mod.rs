//! 플랫폼별 프로세스 그룹 정리 추상화.
//!
//! 현재는 Windows(Job Object)만 구현한다. 다른 OS는 추후(예: Unix process group).
//! session.rs는 `use crate::pty::platform::JobObjectHandle` 로 참조한다.

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::JobObjectHandle;
