# ADR-0034: 문서 아키텍처 — 개발 플로우 중심 frame + CLAUDE.md 라우터화

- 상태: 확정 (2026-06-26, 근거: documentation-architecture-research)
- 관련: `docs/handbook/documentation-system.md` · `docs/README.md` · CLAUDE.md · `docs/research/documentation-architecture-research-2026-06-24.md` · ADR-0032(주석 컨벤션)

## 맥락
docs/ 문서 종류(README·decisions·process/step-log·research·reference)가 늘면서 "전체 문서 시스템이 어떻게 한 세트로 엮이나"가 CLAUDE.md와 docs/README에 흩어졌다. CLAUDE.md는 ~12k 토큰(always-load 권고 20~80줄을 크게 초과)으로 항상 로딩 비용이 컸다. 문서를 개별 타입이 아니라 **개발 플로우의 한 세트**로 보는 큰 그림이 필요했다.

## 결정
- **플로우 중심 frame** — 문서를 개발 플로우(리서치→ADR→TRD→코드) **단계별 산출/게이트/기록**으로 매핑한 frame을 `docs/handbook/documentation-system.md`에 둔다. 플로우 *정의*는 CLAUDE.md를 링크(비복제, SSoT).
- **불변식 4개** — SSoT(복사 말고 링크) · 고아 금지 · soft(문서)+hard(도구) 짝 · 수명(불변=ADR·step-log / living=README·reference).
- **CLAUDE.md = 라우터** — 상세 설명·요약은 정본 포인터로(리뷰표→review 스킬, 참조구현→ADR-0013/14, 매트릭스→ADR-0002/30, 모듈맵 왜→코드). 규칙·핵심 불변식·빌드명령·§5 LLM-우선은 유지(238→206줄, 보수 슬림).
- **핸드오프는 문서 아키텍처 밖** — `.ccb/` 일회성 세션 인계 도구, 영구 기록 노드 아님.

## 거부한 대안
- **doc-type 분류만(타입별 표)** — 일의 흐름이 안 보여 "새 문서 어디 넣나"가 헷갈린다. 플로우 중심으로 전환.
- **CLAUDE.md에 상세 유지** — always-load 비용↑ + 한쪽만 갱신돼 rot. 라우터화로 상세를 정본에 위임.
- **핵심 불변식까지 reference로 이전(공격적 슬림, 옵션 B)** — 회귀를 막는 안전망이라 always-load에 유지(보수안 A). B는 앵커 커버리지 충분해지면 재고.
- **step-log → CHANGELOG 개명** — CHANGELOG(Keep a Changelog)는 릴리스·버전용이라 내부 작업 타임라인인 step-log와 의미 불일치. step-log 유지.
- **핸드오프를 문서 노드로 박기** — 일회성이라 영구 기록 불요.

## 근거
- **documentation-architecture-research(2026-06-24)** — Diátaxis·docs-as-code·SSoT·고아방지(빌드실패)·GitLab/MS handbook 패턴·"always-load 짧게" 권고가 3-family(Claude×2+Codex) 수렴(실질 충돌 0).

## 영향 / 불변식
- 새 문서는 frame의 단계에 매핑 + 발견 체인(README·tracking·코드앵커)에 연결한다(고아 금지).
- CLAUDE.md는 라우터로 유지 — 상세를 다시 본문에 늘리지 않는다(정본 포인터).
- **미적용(미래):** CLAUDE.md 추가 슬림(옵션 B) · 문서 CI(orphan/link/freshness를 hard 게이트로).
