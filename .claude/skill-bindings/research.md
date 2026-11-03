# research 바인딩 — engram-dashboard

경로는 이 프로젝트 루트 기준이다.

## 기존 조사 (flow §1 착수 전 훑기)

- 훑을 디렉터리 = `docs/research/` · `docs/refactoring/`.

## 제약 문서 (flow §1 · design-survey 제약 추출원)

설계·기술 조사면 착수 전에 읽는다.

- `CLAUDE.md` 「설계 원칙」 · 「모듈」.
- `docs/reference/architecture.md` — 모듈 연결 · 불변식.
- `docs/decisions/README.md` 인덱스 → 관련 ADR.
- 그 작업 폴더 `docs/process/<작업>/`의 PRD · TRD.

## 출처 우선순위 (flow §2)

- 프로젝트 내부 문서(ADR · `CLAUDE.md` · 코드)를 외부 웹보다 먼저 1차 출처로 쓴다.
- 로컬 참조 클론 = `I:\Engram_Workspace\opensource\`. 클론별 상태(체크아웃 위치 · 깊이)는 그 폴더 `README.md`를 본다. repo 밖이라 없으면 그때 clone한다.
- 피어 목록 = `docs/research/reference-implementations.md`. 클론을 새로 받으면 그 문서에 겹치는 문제와 함께 한 줄 추가한다.

## 산출 저장 (flow §6)

- 조사 결과는 묻지 않고 전부 파일로 남긴다 — light 요약도 같다.
- 기본 경로 = `docs/research/<주제-슬러그>-<YYYY-MM-DD>.md`.
- 리팩토링 경계 실측 · 구조 부채 계측 = `docs/refactoring/`에 두고 그 폴더 `README.md` 목록에 한 줄 추가한다.
- 새 보고서는 ADR · `docs/process/<작업>/`의 PRD·TRD · `docs/tracking.md` · `docs/process/step-log.md` 중 한 곳에서 링크한다.
- 결정이 나면 보고서 상태 줄에 그 결정을 적는다.
