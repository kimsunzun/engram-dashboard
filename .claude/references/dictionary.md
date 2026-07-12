# 전역 사전 (모든 공용 용어·바인딩의 정본 — 필요할 때 Read)

스킬·규약 본문은 용어·역할명으로만 쓰고, 그 정의·바인딩은 이 사전이 유일 정본이다 — 교체·추가는 여기 한 곳, 카테고리별 섹션으로 누적한다.

## 주도/비주도 family (스위치 — 워커·리뷰어 슬롯 한정, 사용자 결정 2026-07-11)

| 스위치 | family | 상급 슬롯 (호출) | 경량 슬롯 (호출) |
|---|---|---|---|
| **주도** | Claude | Agent `subagent_type: worker-senior` — model·effort 실값은 프리셋 정의가 정본(현재 opus·xhigh) | Agent `model: sonnet` (effort 기본값) |
| **비주도** | GPT | `mcp__codex__codex` (모델 기본값 추종 · **effort high 명시**) | (미정의 — flip 결정 시 함께 정한다) |

- **flip = 이 표 두 행의 family·호출 값 교환** — 역할표·스킬 flow는 무수정. 불변식: 주도 ≠ 비주도(cross-family 자동 성립).
- 메인 오케스트레이터는 스위치 밖(세션 하네스 소속 — flip해도 불변).
- effort 실값 = 이 표 호출 열이 정본(양 family "최상단 바로 아래" 균형 — Claude xhigh/GPT high). codex는 effort **미명시 시 none**으로 떨어진다 — 호출 시 반드시 명시한다.
- Claude 상급의 effort 명시 = `worker-senior` 프리셋(`~/.claude/agents/` — Agent 툴에 effort 파라미터가 없어 프리셋이 유일 수단). 경량은 `general-purpose + model`(effort 기본값).

## 역할 → 슬롯

| 역할 | 슬롯 |
|---|---|
| 메인 오케스트레이터 | 메인 세션 모델(상속) — 스위치 밖 |
| 코더(복잡) | 주도 상급 |
| doc-aware 리뷰어 | 주도 상급 (프로젝트 전용 리뷰어 에이전트가 있으면 그 프로젝트 바인딩이 오버라이드) |
| 코더(단순) · 경량 실행 | 주도 경량 |
| 조사 수집자 | 주도 경량 |
| cross-family(blind) 리뷰어 · 수집자 | 비주도 상급 |

- 스폰 시 `model`(또는 프리셋 타입) 명시 강제 — 미명시 = 조용한 다운그레이드.
- Fable는 워커 배정 금지 — 메인 대화 전용(사용자 결정 2026-07-02).
- **합성·grounding(클레임↔출처 함의 검증)은 역할 슬롯이 아니라 메인 오케스트레이터 소유다**(research 스킬 계약 — 2026-07-03 포트폴리오 리뷰로 모호 해소). doc-aware 리뷰어는 리뷰 슬롯일 뿐 grounding 소유자가 아니다.

## 용어

- **blind (두 뜻 — 문맥 한정 필수):**
  - **수집 blind(collection-blind)** — 병렬 수집자들이 서로 결과를 못 본다(공유 시 앵커링으로 교차 효과 소멸). research 스킬의 BLIND 축.
  - **근거 blind(rationale-blind)** — 리뷰어에게 결정 근거·ADR을 주지 않는다(신선한 판단 확보). review 스킬의 blind 슬롯. 리뷰 대상 산출물 자체는 본다.

