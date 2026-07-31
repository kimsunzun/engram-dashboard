# clean-comment 바인딩 — engram-dashboard

> **ADR-0004 컨벤션:** 소비처 프로젝트 트리 파일. clean-comment 골격(flow §0 · SKILL §소비 계약)이 cwd-상대로 Read해 실값을 꺼낸다. 골격 🔒SEALED는 이 파일이 못 덮는다.

## 양식 오버라이드 (항목 단위 — 명시 안 한 항목은 style-base 유지)

- **파일 헤더** (style-base "안 쓴다"의 오버라이드): load-bearing 파일(동시성·kill·보안·세션복원 등)은 모듈 overview 헤더(`//!` — 역할·불변식·진입점·시그니처로 안 보이는 의미)를 **권장**한다. soft·boy-scout(만질 때 곁다리로) — 게이트 아님.
- **프로젝트 상세 캐논** = `docs/reference/commenting-conventions.md`(ADR-0032) — rules.md와 같은 계열(why/invariant/load-bearing). 스킬 🔒SEALED가 우선하고, 그 아래 상세(층위 모델·헤더 형식·점진 정리 권고)는 캐논이 채운다.

## 보호 패턴 추가 (add-only)

(현재 없음 — Base 보호(ADR 앵커 `ADR-NNNN` 토큰 · 도구 지시 범주)로 충분. `#[cfg]`·`#[allow]` 등 attribute는 코드라 이 목록 불요.)

## 제외 경로 (스윕 대상 제외 — 골격 기본에 추가)

(없음 — 골격 기본(git 추적만 · vendor류 제외) 그대로.)

## retrofit-완료 (밀도 정합 활성 범위 — 루트-상대 디렉터리 · 구획 경계 전방 일치)

- `src-tauri/src/tray` (2026-07-31 파일럿 — review doc light PASS + qa standard PASS)
- `src-tauri/src/daemon_client` (2026-07-31 확대 — 워커 3 병렬, review doc full PASS(재작성 오류 1건 게이트 적출·정정) + qa standard PASS)
