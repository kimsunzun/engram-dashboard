//! 플랫폼별 프로세스 그룹 정리 추상화.
//!
//! 현재는 Windows(Job Object)만 구현한다. 다른 OS는 추후(예: Unix process group).
//! session.rs는 `use crate::pty::platform::JobObjectHandle` 로 참조한다.
//!
//! PID liveness/creation-time 헬퍼(process_creation_time/pid_alive/pid_alive_with_start_time/
//! current_process_start_time)는 daemon(portfile)·tauri(discovery)가 공유한다 —
//! 두 crate 모두 core 에 의존하므로 여기 한 곳에 두어 사본 중복을 없앤다(DRY).

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::JobObjectHandle;

// PID liveness + creation-time 헬퍼는 별도 모듈(양쪽 OS stub 포함, 단위 테스트 동거).
mod process;

pub use process::{
    current_process_start_time, pid_alive, pid_alive_with_start_time, process_creation_time,
};
