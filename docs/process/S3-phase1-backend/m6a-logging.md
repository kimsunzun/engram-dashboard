# 모듈 6a — logging/mod.rs (core) 브리핑 (담당: dcs24, Sonnet)

발신: ed12 (매니저)
근거: `docs/backend-lld-stage1.md` §14 (로깅).

## 범위 (이번엔 core만)

`src-tauri/src/logging/mod.rs` 의 **초기화 + 레벨 토글 코어**만. 
headless 테스트(LogSink/examples)는 Phase 1(pty 모듈 전체) 완료 후 별도로 한다 — 지금은 손대지 말 것.
이 모듈은 pty/ 와 독립이라 병렬로 진행 가능.

## 목표

```rust
/// tracing-subscriber 전역 초기화. 앱 부팅 시 1회 호출.
/// 기본 레벨은 환경변수 RUST_LOG 우선, 없으면 "warn" (= 평상시 거의 무출력 = 기본 OFF).
pub fn init_logging();

/// 런타임 로그 레벨 토글. "trace"|"debug"|"info"|"warn"|"error"|"off".
/// EnvFilter reload handle 로 재설정한다.
pub fn set_log_level(level: &str) -> Result<(), String>;
```

핵심 포인트:
- `tracing_subscriber::fmt()` + `EnvFilter`. 기본값 `EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"))`.
- `set_log_level` 을 위해 **reload layer** (`tracing_subscriber::reload`) 사용 — handle을 static(OnceLock 등)에 보관해 런타임 재설정.
- init은 멱등(중복 호출 안전). 이미 init됐으면 무시하거나 에러 대신 no-op.

## 규칙

- 이 파일은 tauri import 0개 (Tauri command 래퍼는 Phase 2 commands/ 에서 별도). logging core는 순수 tracing.
- 기본 레벨이 OFF(warn)임을 주석으로 명시 — "릴리스 기본 OFF" 요구사항.
- 함수/주요 분기에 한국어 주석. cargo fmt 통과.

## 연결 & 보고

- `lib.rs` 에 `mod logging;` 추가 (최소 연결).
- `cargo build` 통과 확인 후: `orch 12 "⟁dcs24 logging core 완료 — init_logging/set_log_level, build OK"`

막히면 30분 내 중간보고.
