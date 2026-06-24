---
name: qa
description: 코드 변경의 빌드·테스트·실측 게이트(기계적). review(적대 판단)와 짝이며 후행이다 — review가 "맞나"를 보면 qa는 "실제로 도나"를 본다. 강도(quick/standard/full)가 게이트 범위를 고른다 — quick=영향 crate만, standard(기본)=workspace 전회귀+격리, full=standard+GUI 실측(cdp). 코드(test/tsc)가 통과해도 실제 화면 동작 확인 전엔 미완. 비자명 코드 변경 검증에 사용. 트리거 /qa [quick|standard|full].
---

# QA

코드 변경의 **빌드·테스트·실측 게이트**(기계적 검증 "실제로 도나"). review(적대 판단 "맞나")의 후행 짝이다.

핵심 철학(CLAUDE.md): **코드(test/tsc)가 통과해도 실제 화면에서 동작 확인 전엔 미완으로 본다.** 그래서 최상위 강도(full)는 빌드·테스트를 넘어 실제 앱을 띄워 GUI로 동작을 확인한다.

## 강도 (review와 평행 — quick/standard/full)

> 정본 = `references/flow.md` §0 강도표 + §2(강도별 실명령). 아래는 "강도·범주·언제"만 — 실명령은 여기 안 베낀다(rot 방지).

| 강도 | 게이트(범주) | 언제 |
|---|---|---|
| **quick** | 영향 crate만 회귀(+ core 닿으면 격리, 프론트 닿으면 타입체크) | 국소 변경·단일 crate |
| **standard**(기본) | workspace 전회귀 + 격리 + 프론트(테스트·타입체크) | 일반 비자명 변경 |
| **full** | standard + GUI 실측(cdp) | UI·핫패스·릴리스·실제 동작 확인 필요 |

- 선택 신호: 단일 crate 국소=quick / 다중·비자명=standard / UI·핫패스·릴리스=full. 애매하면 standard. 상세 판정·escalation은 `references/flow.md` §1.
- **UI/프론트가 닿으면 무조건 full** — test/tsc만으론 화면 동작을 보장 못 한다(아래 검증 상태).

## review와의 연결

review의 `self` 강도여도 qa는 **최소 quick은 반드시 돈다**(review가 self여도 빌드·테스트 게이트는 생략 X). 상세 계약은 review 스킬.

## 트리거

`/qa [quick|standard|full]`. 강도는 옵션이다 — 미지정이면 변경 범위로 추정하되 **기본 standard**. 파싱·추정 규칙은 `references/flow.md` §0-1. 호출 시 **"어느 강도로 도는지 + 어떤 게이트를 도는지" 한 줄을 사용자에게 명시**한다(예: "standard 강도 / workspace 전회귀 + 격리 + 프론트로 검증합니다").

전체 실행 절차·게이트별 실명령·실패 처리·결과 보고·가드레일은 `references/flow.md`를 따른다.

## 프로젝트 통합

실행 절차·프로젝트 명령·가드레일은 `references/flow.md`가 정본이다(명령 출처는 CLAUDE.md "빌드·검증 명령" 절).

## ⚠️ 검증 상태 (정직한 표시)

qa는 기계적 게이트라 review/research 같은 "미검증 가설" 성격은 약하다(명령이 PASS/FAIL을 직접 낸다). 단 하나의 핵심 경고:

- **test/tsc PASS ≠ 동작 보장.** UI·핫패스는 full의 cdp `eval`로 실제 통과시켜야 동작 확인 = 완료다(비-Windows는 cdp 불가 → standard까지 한계 + "동작 미확인" 정직 보고). 구체 절차는 `references/flow.md` full 절.
