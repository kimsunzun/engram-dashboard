//! 토큰 핸드셰이크 프레임 — 모양 · golden 의 집은 protocol `handshake` 다(ADR-0310 결정 9). 여기는 다시
//! 내보내기만 한다: net 이 지워질 때까지(TRD 3단계 3-3 U6) `auth::AuthFrame` 경로를 쓰는 `ws` 가 그대로 선다.
//!
//! ★이 crate 가 파싱하는 프레임은 여전히 이것 하나다★ — 모양을 소유하지 않을 뿐, 에이전트 어휘를 타입으로
//! 모르는 성질(lib.rs 헤더 게이트 4)은 그대로다. 재수출 밖에서 protocol 의 명령 어휘를 부르지 말 것.

pub use engram_dashboard_protocol::handshake::AuthFrame;
