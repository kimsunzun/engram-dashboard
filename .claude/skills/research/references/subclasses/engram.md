# research 서브클래스 — engram

`/research engram [<주제>] [강도]`로 선택(주제 생략 시 컨텍스트 추론 — 파싱 = SKILL.md 정본). Base(flow.md)의 🕳HOLE만 채운다 — 🔒SEALED 불변식은 안 건드린다. 프로젝트 사실의 정본은 이 파일이 아니라 아래 가리키는 파일들이다(여긴 "어디를 보라"만, 하드코딩 금지).

## HOLE 채움

- **제약 문서 (flow §1 · §7 제약 추출원):** 설계·기술 조사면 먼저 읽어 반영한다 —
  - `apps/engram-dashboard/CLAUDE.md` (프로젝트 헌법·기술스택·불변식)
  - `apps/engram-dashboard/docs/decisions/` ADR (`README.md` 인덱스 → 관련 ADR)
  - 관련 모듈 코드·기존 spike/spec 문서.
- **출처 우선순위 (flow §2):** 위 프로젝트 내부 문서(ADR·CLAUDE.md·코드)를 **1차 출처로** 우선한다(외부 웹보다 프로젝트 결정이 먼저). 외부는 그다음, SEO 콘텐츠팜 배제는 Base대로.
- **산출 저장 (flow §6):** 조사 보고서는 프로젝트 문서 관례를 따른다 — 설계 서베이는 관련 ADR/spec 옆 또는 `docs/`에, 순수 조사 메모는 세션 산출로. 굵은 설계 결정으로 이어지면 `/adr` 거부 대안으로 넘긴다(Base §7). **정확한 경로는 확정 전 사용자에게 한 줄 확인.**

## 이 서브클래스가 못 하는 것 (SEALED 재확인)

cross-family 리뷰 게이트(§0)·calibration(§2)·grounding 상시(§3)·fresh cross-family 리뷰어(§4)는 Base 봉인 — engram이라고 끄거나 same-family로 못 바꾼다.
