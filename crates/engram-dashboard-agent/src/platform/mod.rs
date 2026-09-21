//! 플랫폼 질의 셋 — 자식 프로세스 그룹 정리(Windows Job Object) · 「이 파일을 지금 누가 열고 있나」
//! ([`file_holders`]) · 「이 PID 아래 무엇이 살아 있나」([`process_tree`]).
//!
//! PID liveness 헬퍼는 여기 없다 — 소비자가 이 crate 밖에 셋이라 `engram-dashboard-base` 의
//! `platform` 으로 이사했다(ADR-0175 결정 1). ★여기 있는 셋은 그 조건을 못 채운다★ — Job Object 래퍼는
//! 소비자가 `transport::pty`·`transport::stdio` 둘뿐이고, 나머지 둘은 codex 세션 id 회수
//! 하나뿐이다(ADR-0218 결정 11).
//!
//! ★셋 다 crate 밖으로 안 나간다★ — 소비자가 전부 이 crate 안이고, 형제인 Job Object 래퍼가
//! 이미 그 모양이다.

pub(crate) mod file_holders;
pub(crate) mod process_tree;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::JobObjectHandle;
