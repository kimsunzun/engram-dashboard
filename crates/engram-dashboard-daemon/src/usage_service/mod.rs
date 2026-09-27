//! 사용량 한도 서비스 — 벤더 중립.
//!
//! ★벤더 이름·키 목록·정책(쿨타임·시한)을 여기 두지 않는다★ — 전부
//! `engram_dashboard_agent::backend::usage_probes()` 에서 받는다. 벤더 match·벤더 리터럴이 이 모듈에 생기면
//! 「백엔드 확장」 위반이다.
// ADR-0004

pub mod clock;
pub mod reject_store;

/// 요청의 답이 그 요청이 기다린 조회에서 나온 값인가.
///
/// ★데몬 안에서만 쓴다 — wire 스냅숏은 싣지 않는다★: wire 스냅숏은 방송과 셸 캐시로만 흐르고(⟳ 답은
///   `Ack`) 거기선 늘 `Cached` 라 뜻이 없다(TRD §3 #50·#81). 싣는 곳은 버스 행이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageServed {
    /// 이 요청이 기다린 조회가 방금 받아 온 값이다.
    Fresh,
    /// 이번 요청 동안 새로 받은 값이 없다(조회 안 함 · 실패 · 기다림 상한 초과) — 서비스가 들고 있던 값이다.
    Cached,
}
