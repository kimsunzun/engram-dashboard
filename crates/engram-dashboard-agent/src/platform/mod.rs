//! 플랫폼 질의 둘 — 자식 프로세스 그룹 정리(Windows Job Object 래퍼 + Job 에 넣을 때까지 멈춰 띄운
//! 프로세스를 깨우는 `resume_suspended_process`) · 그 무리의 중립 손잡이([`process_group`] — 명단 ·
//! 멤버 붙들기(사실 · 끝내기) · 가입 알림 · 물러남 표시).
//!
//! PID liveness 헬퍼 · 「이 파일을 지금 누가 열고 있나」(`file_holders`) · 「이 PID 아래 무엇이 살아
//! 있나」(`process::subtree`)는 여기 없다 — OS 층 crate `engram-dashboard-platform` 에 있다(ADR-0266).
//! 여기 남은 둘은 소비자가 전부 이 crate 안이다: Job Object 래퍼는 `transport::pty`·`transport::stdio`·
//! `backend::codex::transport`·`usage::process`(깨우기 헬퍼는 `usage::process` 하나), `process_group` 은
//! claude 끊기 뒤 잔여물 정리다(ADR-0262).
//!
//! ★Job Object 래퍼 말고는 crate 밖으로 안 나간다★ — 소비자가 전부 이 crate 안이다.

pub(crate) mod process_group;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub(crate) use windows::resume_suspended_process;
#[cfg(windows)]
pub use windows::JobObjectHandle;
