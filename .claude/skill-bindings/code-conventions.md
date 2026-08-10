# code-conventions 바인딩 — engram-dashboard

> 소비처 프로젝트 트리 파일(ADR-0004). 스킬 골격이 cwd-상대로 Read해 실값을 꺼낸다. 골격 🔒 항목은 이 파일이 못 덮는다.

## 양식 오버라이드 (항목 단위 — 명시 안 한 항목은 골격 유지)

(없음 — 구 프로젝트 캐논(`commenting-conventions.md`, dashboard ADR-0032)의 실값(`//!` overview 헤더·동갱신·boy-scout 경계)은 골격에 승급돼 커버되고, 캐논 문서는 폐기됐다. 근거 = dashboard `docs/decisions/0032`.)

## 보호 항목 추가 (add-only)

(없음 — 골격 보호(법적 고지 · `ADR-NNNN` 앵커 · 도구 지시 범주)로 충분. `#[cfg]`·`#[allow]` 등 attribute는 주석이 아니라 코드라 목록 불요.)

- **앵커 형태** — 이 프로젝트는 `ADR-NNNN`(dashboard `docs/decisions/`)과 `FIX N`(리뷰 앵커) 둘을 쓴다.

## 제외 경로 (스윕 대상 제외 — 골격 기본에 추가)

(없음 — 골격 기본(git 추적만 · vendor·생성물 트리 제외) 그대로.)

## retrofit-완료 (소급 정리가 끝난 범위 — 루트-상대 디렉터리)

- `src/` — **프론트엔드 전량**(TS/React, 19,638줄 · 순감 −28%). review doc + qa 통과, push 완료.
- `src-tauri/`, `crates/` — **백엔드 전량**(Rust). master 머지 + CI green(2026-08-09).

> 즉 현재 추적 소스는 전량 1회 스윕됐다. 단 **개정 R4**(「한 사실은 한 집 — 반복은 지운다」)는 스윕 도중 확정돼, 초기 배치는 구 R4 기준으로 돌았다 — 신 R4 재패스는 미실행이다.
