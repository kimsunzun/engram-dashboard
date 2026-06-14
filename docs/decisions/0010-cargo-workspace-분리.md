# ADR-0010: Cargo workspace 3-crate 분리

- 상태: 확정 (S12 phase 1)
- 관련: CLAUDE.md 백엔드 모듈 맵·빌드 명령 · 루트 `Cargo.toml`

## 맥락
데몬화(S12)를 하려면 코어 로직이 `src-tauri`(=tauri bin)에 묶여 있으면 안 된다. 데몬 서버 bin은 tauri 없이 코어만 의존해야 한다.

## 결정
Cargo **workspace**로 전환. 멤버 3개:
- `crates/engram-dashboard-protocol` — wire 계약(AgentCommand/AgentEvent/OutputChunk/codec, ts-rs 바인딩).
- `crates/engram-dashboard-core` — 코어(pty/persistence/logging) + examples. `src-tauri/src/`에서 git mv(history 보존).
- `src-tauri` — tauri 앱. 코어를 `engram_dashboard_core::{...}`로 re-import.

## 거부한 대안
- **src-tauri 단일 crate 유지** — 데몬 bin이 tauri 의존을 끌고 오게 됨. 코어 격리(ADR-0003) 위반.

## 근거
데몬은 core+protocol만 의존(tauri 없이 빌드). 내부 `crate::` 경로는 무수정(top-level 모듈 유지)이라 이동 비용 작음.

## 영향 / 불변식
- 빌드·테스트는 workspace 루트에서 `cargo test -p <crate>` / `cargo build`.
- 격리 게이트 경로: `rg "use tauri" crates/engram-dashboard-core/src/` → 0줄.
