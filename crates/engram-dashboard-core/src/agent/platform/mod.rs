//! 플랫폼별 프로세스 그룹 정리(Windows Job Object) + PID liveness 헬퍼.

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::JobObjectHandle;

mod process;

pub use process::{
    child_pids, current_process_start_time, pid_alive, pid_alive_with_start_time,
    process_creation_time,
};
