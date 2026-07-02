---
name: research
description: 조사 수집자(주계열)가 수집·합성하고 메인이 grounding으로 검증한 뒤, 종합·논쟁·confident-wrong 위험이 있으면 cross-family(blind) 리뷰어가 산출물을 적대 리뷰해 출처 단 보고서를 만든다. 깊은 사실 조사·업계 관행·기술 비교처럼 한 모델만 믿기 불안한 리서치에 사용 — 단발 사실은 light, 종합·논쟁은 medium+, 고위험·비가역은 deep. 트리거 /research "<주제>" [light|medium|deep].
---

# Research

조사 수집자(주계열) 팬아웃이 수집·합성하고 메인이 grounding으로 검증한 뒤, 종합·논쟁·confident-wrong 위험이 큰 산출물만 cross-family(blind) 리뷰어가 적대 리뷰해 출처 단 보고서를 만든다. 핵심은 라우팅 — 강도(light/medium/deep)로 싸게 단일 수집으로 끝낼지 vs 적대 리뷰로 검증할지를 가른다.

## 핵심 설계 (불변)

- **라우팅 = 핵심 산출.** 강도가 "싸게 단일 수집이냐 vs 적대 리뷰냐"를 가른다: findable-fact → light, 종합·논쟁·confident-wrong 위험 → medium+, 비가역·고위험 → deep. (정량 = `references/flow.md`)
- **수집 = 단일 조사 수집자(주계열) + calibration.** 수집자는 모르면 기권하고 honest 확신도를 단다. 이 규칙이 옛 이중 수집 교차의 사실검증 몫을 대부분 대체한다 — 현대 모델은 obscure 사실에 확신-환각 대신 기권으로 self-calibrate하므로. cross-family 병렬 수집은 deep 옵션으로만 둔다.
- **grounding = 메인 외부 체크, 상시 ON.** 클레임↔출처 함의는 수집자 셀프 체크가 아니라 메인이 외부에서 검증한다 — 모든 tier(light 포함). medium↑에선 cross-family(blind) 리뷰어도 load-bearing 함의를 스팟 재검증한다(같은 family는 메인 오독을 못 잡음 — 2차 방어선).
- **cross-family(blind) 리뷰어 = 적대 리뷰어(수집자 아님).** medium↑에서 합성 산출물을 때린다 — 근거 없는 주장·과장·오귀속·논리공백·confident-wrong·완전성/누락. 값어치는 수집 이중화가 아니라 합성 결과 적대 리뷰다 — 같은 family는 self-consistency로 confident-wrong을 무르게 본다. 누락은 부분적으로만 되찾으니 누락이 결론을 가르면 deep 독립 수집이 백스톱. 레벨 사다리·반박=반증 강제는 `references/flow.md §4`.
- **abstention ≠ contradiction.** 한쪽이 근거 있는 답, 다른 쪽이 단지 기권이면 grounding 통과 시 채택 — 정면으로 다른 답을 낼 때만 contested. (`references/flow.md §5`)
- **확신도 = grounding + 리뷰에서 파생.** 자기보고 %도, 수집자 합의도도 근거가 아니다. 최상위 '확실'은 독립 교차확증(제2 1차자료/deep 독립 수집 합의)이 있어야 주고, 리뷰가 공격 안 한 단일 출처는 '가능성 높음'까지. (`references/flow.md §5`)
- **mode-aware 에스컬레이션.** 남는 load-bearing contested/미지지는 대화 모드 = 사용자 질문 / 자율 모드 = 태그 유지 + 로그. 마이너는 메인 자율. (`references/flow.md §5`)
- **BLIND — 병렬 수집 축에만.** deep에서 cross-family 병렬 수집을 켜면 두 family는 서로 결과를 안 본다(공유하면 앵커링으로 교차 효과 소멸). 적대 리뷰는 반대로 산출물을 봐야 때린다 — BLIND는 병렬 수집에만 건다.

역할→모델 = 전역 사전(`I:\Engram\core\claude-global-shared\references\dictionary.md`) 참조. 불변 = 리뷰어는 주계열과 다른 family여야 confident-wrong을 가른다.

**실행 전 `references/flow.md`를 반드시 Read 한다 — 안 읽고 조사/리뷰 에이전트 스폰 금지.** 전체 절차·강도표·라우팅 가이드·가드레일이 거기 있다. `$ARGUMENTS` = 조사 주제(+선택적 강도). 없으면 사용자에게 묻고, 강도 미지정이면 라우팅 가이드로 추정(기본 medium).

**설계-결정 모드:** 설계 착수 전 "OSS는 이 문제를 어떻게 풀었나 → 우리 뭐로 갈까" 서베이도 이 스킬이 한다 — 기본 조사에 제약 적합도 표 + 거부후보→ADR 거부대안을 더해 선택지로 끝낸다(`references/flow.md §7`).

## 티어 = 라우팅 (핵심)

강도(light/medium/deep)가 "싸게 단일 수집이냐 vs 적대 리뷰냐"를 가른다. **정량(수집자 수·검색량·리뷰 범위)은 `references/flow.md` 강도표가 정본** — 아래 표는 "언제·무엇"만 담는다.

| tier | 언제 쓰나 (라우팅) | 무엇을 한다 | 산출 |
|---|---|---|---|
| **light** | 단발 사실 · 웹서 바로 찾아짐 · 결정 영향 작음 | 단일 수집 + grounding + 기권. 적대 리뷰 없음 | 출처 포함 요약 |
| **medium**(기본) | 여러 주장 종합·해석 · 논쟁적 · 1차자료 희소 · "확신하는데 틀리면 치명" | + 적대 리뷰 of 산출물 | 출처 단 보고서 + 리뷰 결과 |
| **deep** | 비가역 · 고비용 · 안전치명 · 출처 분쟁 예상 | + gap 반복(loop-until-dry) + 사람 백스톱 (+옵션 cross-family 병렬수집) | 인용 보고서 + 쟁점 판정 |

**라우팅 가이드 (오분류가 비용/누락을 가른다):**
- **findable-fact**(단발·웹서 즉답·저위험) → **light**. 리뷰는 낭비(둘 다 같은 웹 → 교차 이득 0·비용 2배).
- **종합·비교·논쟁·confident-wrong 위험** → **medium+**.
- **비가역·고위험·안전치명** → **deep**. 사람 백스톱 필수 (+옵션 cross-family 병렬 수집).
- 애매하면 medium. tier 트리거 = 되돌리기 비용·불확실성·confident-wrong 가능성·출처 분쟁 가능성(클수록 위 tier, escalation-only). 'findable-fact로 보이지만 확신-오류 위험이 커 light 금지'인 부류(버전·법규·통계 등)의 전체 목록은 `references/flow.md` 라우팅 가이드가 정본.

## 실행 중 자기보고

grounding·적대 리뷰 단계에서 현재 산출에 영향을 주는 문제(클레임 모순·못 믿을 출처·도구 오작동·명세 모순)는 지체 없이 사용자에게 보고하고 조용히 우회하지 않는다. 산출에 영향 없는 사소한 명세 개선점은 최종 보고의 "명세 개선 메모"로 모은다. (리서치 내용의 미해결 쟁점(contested)은 별개 — 보고서의 쟁점/한계 섹션, `references/flow.md §5`.)

## 자기개선 피드백

이 스킬을 쓰다 발견한 결함·개선점은 그 자리서 고치지 말고 작업 종료 후 이 폴더 `feedback.md`에 한 줄 누적한다(반영은 관련 주제 재등장 시 사용자 승인 하에). 위 "명세 개선 메모"도 같은 `feedback.md`로. 전체 규약 = `../_shared/self-improvement-feedback.md`.
