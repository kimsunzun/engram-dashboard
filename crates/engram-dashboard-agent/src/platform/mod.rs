//! 플랫폼 질의 넷 — 자식 프로세스 그룹 정리(Windows Job Object) · 그 무리의 멤버를 하나씩 다루는 중립 손잡이
//! ([`process_group`] — 명단 · 검증 · 끝내기) · 「이 파일을 지금 누가 열고 있나」([`file_holders`]) · 「이 PID
//! 아래 무엇이 살아 있나」([`process_tree`]).
//!
//! PID liveness 헬퍼는 여기 없다 — 소비자가 이 crate 밖에 셋이라 `engram-dashboard-base` 의
//! `platform` 으로 이사했다(ADR-0175 결정 1). ★여기 있는 넷은 그 조건을 못 채운다★ — 소비자가 전부 이
//! crate 안이다: Job Object 래퍼는 `transport::pty`·`transport::stdio`, `file_holders` 는 codex 세션 id
//! 회수(ADR-0218 결정 11), `process_tree` 는 그 회수와 claude 끊기 뒤 잔여물 정리, `process_group` 은 그
//! 정리 하나다(ADR-0257).
//!
//! ★Job Object 래퍼 말고는 crate 밖으로 안 나간다★ — 소비자가 전부 이 crate 안이다.

pub(crate) mod file_holders;
pub(crate) mod process_group;
pub(crate) mod process_tree;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::JobObjectHandle;
