# ADR-0282: 2-2 discovery 나누기의 세부 — 실행 파일 위치는 platform 한 곳, 쓰기 프로브는 base, 셸 사본은 같은 경로 시험으로 묶는다

- 상태: 제안 (2026-10-07, 근거: 번호 선점 — 본문은 2-2 착지 때 채운다 · 정본 설계 = `docs/process/S21-crate-boundaries/trd-2-2-discovery-split.md` §7)
- 관련: ADR-0269(결정 7 입주 조건 — 아래 결정 2) · Amends ADR-0271 (결정 2의 실행 파일 위치 계산 자리)

## 맥락
TODO — 무슨 문제를 풀어야 했나.

## 결정
(사용자 위임 2026-10-07 아래의 메인 결정 · 2-2 착지 때 나머지를 채운다 — 지금은 U1 이 기대는 둘만 적는다)

1. **실행 파일 위치 계산은 platform `env::sibling_exe` 한 곳이다** — ADR-0271 결정 2 가 이것을 「데몬으로」 두었던 자리를 고친다. 데몬의 CLI 찾기 · 셸의 데몬 찾기 · `roundtrip_smoke` 가 모두 그것을 부른다(데몬과 셸은 서로의 코드를 못 나눠 쓰고, 둘 다 platform 에는 닿는다).
2. **쓰기 프로브 도우미는 base `writable` 이다 — 입주를 2-2 U1 로 당긴다.** ADR-0269 결정 7 의 「사본 둘」 조건은 같은 단계 안의 U2 · U3 에서 성립한다. 당긴 이유 = 임시 사본을 만들지 않는다 · 프로브 이름 카운터(`probe_path` 의 static)를 데몬 판과 셸 판이 한 프로세스에서 공유해야 이름이 안 겹친다. 옮기면서 그 정리 경고의 tracing target 이 `engram_dashboard_discovery` → `engram_dashboard_base::writable` 로 바뀐다(문구 · 레벨 그대로 · 저장소 안에 그 target 을 쓰는 `RUST_LOG` 지시 0건).

## 거부한 대안
- TODO 대안 A — 왜 버렸나.
- TODO 대안 B — 왜 버렸나.

## 근거
TODO — 실측·리뷰 등 결정의 뒷받침.

## 영향 / 불변식
TODO — 이 결정이 묶는 코드·게이트. 어기면 무엇이 깨지나.
