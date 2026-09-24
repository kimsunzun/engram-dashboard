# research 서브클래스 — engram (프로젝트-로컬)

이 파일은 ADR-0004 규약으로 **소비처 프로젝트 트리**에 산다 — research 골격(SKILL.md 조합 절차)이 cwd-상대로 자동 Read한다(인자 불필요). Base(flow.md)의 🕳HOLE만 채운다 — 🔒SEALED 불변식은 안 건드린다. 프로젝트 사실의 정본은 이 파일이 아니라 아래 가리키는 파일들이다(여긴 "어디를 보라"만, 하드코딩 금지).

## HOLE 채움

- **제약 문서 (flow §1 · §7 제약 추출원):** 설계·기술 조사면 먼저 읽어 반영한다 — (경로 = 이 프로젝트 루트 상대)
  - `CLAUDE.md` (프로젝트 헌법·기술스택·불변식)
  - `docs/decisions/` ADR (`README.md` 인덱스 → 관련 ADR)
  - 관련 모듈 코드·기존 spike/spec 문서.
- **출처 우선순위 (flow §2):** 위 프로젝트 내부 문서(ADR·CLAUDE.md·코드)를 **1차 출처로** 우선한다(외부 웹보다 프로젝트 결정이 먼저). 외부는 그다음, SEO 콘텐츠팜 배제는 Base대로.
- **산출 저장 (flow §6):** ★**조사 결과는 전부 적립한다 — 묻지 않는다**★(사용자 결정 2026-09-24: "리서칭은 그냥 꾸준하게 적립해야되는거"). **light 의 출처 포함 요약도 같은 곳에 파일로 남긴다** — 세션 산출(스크래치)로만 두고 끝내지 않는다. 위치 = `docs/README.md` 「새 내용을 어디에 넣나」가 정하는 폴더 — 기본 `docs/research/<주제-슬러그>-<YYYY-MM-DD>.md`, 리팩토링 경계 실측·구조 부채 계측이면 `docs/refactoring/`(그 폴더 `README.md` 목록에 한 줄 추가). **고아 금지** — step-log·tracking·ADR 중 한 곳에서 링크한다(`docs/README.md`). 결정이 나면 보고서 상태 줄에 그 결정을 적고, 굵은 설계 결정으로 이어지면 `/adr` 거부 대안으로 넘긴다(Base §7).

## 이 서브클래스가 못 하는 것 (SEALED 재확인)

cross-family 리뷰 게이트(§0)·calibration(§2)·grounding 상시(§3)·fresh cross-family 리뷰어(§4)는 Base 봉인 — engram이라고 끄거나 same-family로 못 바꾼다.
