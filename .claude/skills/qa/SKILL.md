---
name: qa
description: 코드 변경의 빌드·테스트·실측 게이트(기계적). review(적대 판단)와 짝이며 후행이다 — review가 "맞나"를 보면 qa는 "실제로 도나"를 본다. 강도(quick/standard/full)가 게이트 범위를 고른다 — quick=영향 모듈만, standard(기본)=전체 회귀+격리, full=standard+GUI/실제 동작 실측. 코드 테스트·타입체크가 통과해도 실제 화면 동작 확인 전엔 미완. 비자명 코드 변경 검증에 사용. 트리거 /qa [quick|standard|full].
---

# QA

코드 변경의 **빌드·테스트·실측 게이트**(기계적 검증 "실제로 도나"). review(적대 판단 "맞나")의 후행 짝이다.

핵심 철학: **코드 테스트·타입체크가 통과해도 실제 화면에서 동작 확인 전엔 미완으로 본다.** 그래서 최상위 강도(full)는 빌드·테스트를 넘어 실제 앱을 띄워 GUI로 동작을 확인한다.

## 강도 (review와 평행 — quick/standard/full)

> 정본 = `references/flow.md` §0 강도표. 강도별 실명령은 바인딩 파일(`references/bindings/<project>.md`). 아래는 "강도·범주·언제"만 — 실명령은 여기 안 베낀다(rot 방지).

| 강도 | 게이트(범주) | 언제 |
|---|---|---|
| **quick** | 영향 모듈만 회귀(+ 격리 대상 모듈 닿으면 격리, 프론트 닿으면 타입체크) | 국소 변경·단일 모듈 |
| **standard**(기본) | 전체 회귀 + 격리 + 프론트(테스트·타입체크) | 일반 비자명 변경 |
| **full** | standard + GUI/실제 동작 실측 | UI·핫패스·릴리스·실제 동작 확인 필요 |

- 선택 신호: 단일 모듈 국소=quick / 다중·비자명=standard / UI·핫패스·릴리스=full. 애매하면 standard. 상세 판정·escalation은 `references/flow.md` §1.
- **UI/프론트가 닿으면 무조건 full** — 코드 테스트·타입체크만으론 화면 동작을 보장 못 한다(아래 검증 상태).

## review와의 연결

review의 `self` 강도여도 qa는 **최소 quick은 반드시 돈다**(review가 self여도 빌드·테스트 게이트는 생략 X). 상세 계약은 review 스킬.

## 트리거

`/qa [quick|standard|full]`. 강도는 옵션이다 — 미지정이면 변경 범위로 추정하되 **기본 standard**. 파싱·추정 규칙은 `references/flow.md` §0-1. 호출 시 **"어느 강도로 도는지 + 어떤 게이트를 도는지" 한 줄을 사용자에게 명시**한다(예: "standard 강도 / 전체 회귀 + 격리 + 프론트로 검증합니다").

**실행 전 `references/flow.md`를 반드시 Read 한다 — 안 읽고 게이트 진행 금지.** 전체 실행 절차·실패 처리·결과 보고·가드레일이 거기 있다(범용 골격). 강도별 실명령은 바인딩 파일을 쓴다.

## 프로젝트 바인딩

전용 명령·체크리스트는 `references/bindings/<project>.md`. 현재 engram = `references/bindings/engram.md`. 다른 프로젝트는 같은 골격(`references/flow.md`)에 바인딩 파일만 추가한다.

## ⚠️ 검증 상태 (정직한 표시)

qa는 기계적 게이트라 review/research 같은 "미검증 가설" 성격은 약하다(명령이 PASS/FAIL을 직접 낸다). 단 하나의 핵심 경고:

- **코드 테스트·타입체크 PASS ≠ 동작 보장.** UI·핫패스는 full의 실측으로 실제 통과시켜야 동작 확인 = 완료다(실측이 불가한 플랫폼에선 standard까지 한계 + "동작 미확인" 정직 보고 — 바인딩이 제약을 명시). 구체 절차는 `references/flow.md` full 절 + 바인딩.

## 자기개선 피드백 · 사용 로그
결함·개선점은 **그 자리서 고치지 말고** 작업 종료 후 이 폴더의 `feedback.md`에 누적(반영은 관련 주제 재등장 시 사용자 승인 하에). **추가로(권장·강제 아님)** 정상 사용 기록은 `usage-log.md`에 한 줄 — 학습용. 전체 규약 = `../_shared/self-improvement-feedback.md`.
