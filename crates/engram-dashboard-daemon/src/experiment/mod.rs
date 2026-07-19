//! 실험 인프라 — ADR-0090 Stage 2 컨텍스트 포화 파일럿의 순수 로직.
//!
//! ## 역할
//! 실 claude 없이 단위 테스트 가능한 **순수 로직 전부**를 담는다: 결정적 필러 생성(`filler`)·프로브
//! 채점 + compaction 감지(`probe`)·JSONL 레코드 + raw stream-json 파서(`record`)·CLI 파싱(`cli`)·
//! 트랜스크립트 탭 파서(`transcript` — best-effort 실험 측정, ADR-0008 경계 준수).
//! 실 claude 를 띄우는 드라이버 루프는 `bin/saturation_pilot.rs`(thin) 가 이 모듈들을 조립해 수행한다.
//!
//! ## 게이팅(ADR-0090 불변식)
//! **전 모듈이 `#[cfg(feature = "test-harness")]` 뒤에 있다.** 운영(production) 빌드는 이 코드를 아예
//! 컴파일하지 않는다 — 실험 코드가 릴리즈 아티팩트에 유입되면 ADR-0090 위반(기존 obs_seam / test-harness
//! feature 와 같은 계열). bin(`saturation-pilot`)도 `required-features = ["test-harness"]` 라 통상
//! 워크스페이스 빌드는 이 모듈도 bin 도 만지지 않는다.
//!
//! ## 진입점
//! - `cli::parse_args` → `PilotConfig`
//! - `filler::filler_doc` / `filler::doc_title`
//! - `probe::score_probe` / `probe::detect_suspected_compaction`
//! - `record::Record`(+ 하위) / `record::sha256_hex` / raw 파서들
//! - `transcript::locate_transcript` / `transcript::parse_transcript` / `RealUsage`
// ADR-0090

pub mod cli;
pub mod filler;
pub mod probe;
pub mod record;
pub mod transcript;
