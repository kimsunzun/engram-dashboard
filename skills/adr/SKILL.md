---
name: adr
description: ADR(설계 결정 기록)를 박제한다 — 새 결정·번복(supersede)·정합성 점검(lint)·인덱스 재생성. 트리거 /adr [new|supersede|lint|index].
---

# ADR

**실행 전 `references/flow.md`를 반드시 Read — 안 읽고 채번/스크립트 호출 금지.**

ADR(설계 결정 기록 — Architecture Decision Record)의 **기계적 일**을 자동화한다. ADR = "왜 이렇게 정했나 + 거부한 대안"을 시점 무관하게 박아 다음 세션의 재론(re-litigation)을 막는 영구 못. 이 스킬은 **결정을 만들지 않는다** — 호출자가 준 결정을 받아 구조화·파일링·교차링크할 뿐이다.

## 불변 — 결정적 스크립트 + 얇은 판단

서기 일(채번·스캐폴드·인덱스 재생성·supersede 양방향 링크·형식 lint)은 **결정적 스크립트**가 한다. 스킬(LLM)은 기계가 못 하는 것만 한다: ① 입력 수령·검증 ② 전체/부분 폐기 판단 ③ 본문 prose ④ 보고. 이 경계가 스킬의 존재 이유다 — LLM이 서기 일을 손으로 하면 번호 충돌·한쪽 누락·거짓 lint가 난다.

## 오퍼레이션 (1차 축 — 강도 아님)

ADR 기록은 결정적 작업이라 "대충/철저히" 강도 축이 없다.

| 오퍼레이션 | 무엇 | 스크립트(기계) | 스킬(판단) |
|---|---|---|---|
| **new** | 새 결정 1건 박제 | 채번 → 스캐폴드 → 인덱스 재생성 | 본문 prose |
| **supersede** | 결정 번복(전체/부분) | 새 ADR + 옛 ADR 양방향 링크 | **전체 vs 부분 판단** + prose |
| **lint** | 정합성 점검(read-only) | 번호·양방향·상태어휘·앵커고아 검사 | error/advisory 해석·보고 |
| **index** | 인덱스 단독 재생성 | 본문 파생 표 갱신(큐레이션 셀 보존) | — |

## 트리거

`/adr [new|supersede|lint|index]`. 오퍼레이션은 옵션 — 미지정이면 요청으로 추정(새 결정=new / "폐기"·"바뀜"=supersede / "점검"·인자 없음=lint / "인덱스 다시"=index). 파싱·추정 규칙 = `references/flow.md §0`. 호출 시 **어느 오퍼레이션을 도는지 한 줄을 사용자에게 명시**한다.

## 프로젝트 바인딩

바인딩 정본·서기 스크립트 위치 = `references/flow.md` 바인딩 절. 스캐폴드 템플릿 실체는 `references/formats/`(dashboard = `adr.template.md`, 경량 = `adr-light.template.md`).

## 자기개선 피드백

결함·개선점은 그 자리서 고치지 말고 작업 종료 후 `feedback.md`에 누적한다(검증 상태도 그쪽이 정본). 전체 규약 = `../_shared/self-improvement-feedback.md`.
