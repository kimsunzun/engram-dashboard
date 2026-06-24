---
name: review
description: 변경물을 Advocate(옹호·강화) vs Adversary(공격·대척) 2인 적대 리뷰로 검증한다. 단계(prd/trd/code/doc)가 어느 역할 렌즈를 쓸지 고르고, 강도(self/light/full/deep)가 인원·깊이를 고른다. 다른 family(opus doc-aware + Codex blind)로 편향을 가르고, 판정은 PASS/FIX/BLOCK, 불일치는 사용자에게 에스컬레이션한다. 비자명 코드·설계·문서 변경 검증에 사용. 트리거 /review [prd|trd|code|doc] [self|light|full|deep].
---

# Review

변경물을 **Advocate(옹호·강화) vs Adversary(공격·대척)** 고정 2인 골격으로 적대 검증한다. 두 리뷰어는 **다른 family**(opus + Codex)라야 학습 편향이 갈려 교차검증이 성립한다 — 같은 family 둘은 효용이 약하다.

핵심은 "더 많이 보기"가 아니라 **단계마다 전용 역할 렌즈를 미리 박아** 결함 커버리지를 올리고, **불일치를 사용자에게 떠넘겨** 자기편 편향(메인=Claude 계열)을 차단하는 것이다.

## 핵심 설계 — 2축 (단계 × 강도, 직교)

review는 두 축이 직교한다. 한 축이 다른 축을 정하지 않는다. 이 골격(Advocate/Adversary 쌍 · 강도 4-tier · 단계 렌즈 · PASS/FIX/BLOCK 판정 · 불일치→사용자)이 **범용 리뷰 엔진**이다 — 어느 프로젝트에서나 동일하게 쓴다.

- **단계(무엇을 보나)** = `prd` | `trd` | `code` | `doc` | (fallback). 단계가 **어느 역할 렌즈(Advocate/Adversary)·블라인드·체크리스트**를 쓸지 고른다.
- **강도(얼마나)** = `self` | `light` | `full`(기본) | `deep`. 강도가 **리뷰어 인원·검증 깊이**를 고른다. 단계와 무관한 공통 스케일이다.
- 예: `code` 단계를 `full`로 = 코드 역할표의 2인(correctness 옹호 + breaker), `light`로 = breaker 1인만. 같은 단계라도 강도가 인원을 가른다.
- **self 예외:** self는 리뷰어 0인이라 역할 렌즈를 안 쓴다. 이때 단계 인자는 *렌즈*가 아니라 **self 체크리스트·QA 범위**를 고른다(code=diff+테스트 / doc=링크·중복 / trd=ADR·불변식 / prd=요구 누락). 리뷰어 렌즈는 light부터 켜진다.

## 강도 tier — 언제

> 정본 = `references/flow.md` §0 강도표(누가·깊이). 아래는 "강도·언제"만 — 여기 안 베낀다(rot 방지).

| 강도 | 언제 |
|---|---|
| **self** | 1~2줄·문서 오타·자명한 변경 |
| **light** | 국소·저위험·단일 관심사 변경 (위험 영역 사전 배제) |
| **full**(기본) | 비자명 변경 (CLAUDE.md 기본 게이트) |
| **deep** | 고위험 — 동시성·kill·lifetime·보안·공개 API·마이그레이션·핫패스 |

강도 신호 = **LOC · 위험 영역 · 신규/리팩터 · 자동생성**, 무거울수록 위 강도. 상세 기준은 flow §0. 애매하면 full.

## escalation 규칙 (강도는 올라가기만 한다)

light/full로 시작했어도 리뷰 도중 **위험 트리거(동시성·kill·lifetime·보안·공개 API·마이그레이션 등)를 발견하면 상위 강도로 자동 승격하고 사용자에게 알린다**. escalation-only — 강도를 임의로 낮추지 않는다(시작 강도가 하한). 발견은 코더 self 단계든 리뷰어 단계든 어디서든 트리거된다.

## 트리거

`/review [prd|trd|code|doc] [self|light|full|deep]`. 단계·강도 둘 다 옵션이다. 파싱·기본값(단계/강도 추정, 모호하면 사용자 확인)은 `references/flow.md` §0-1. 호출 시 **"어느 단계·어느 강도로 도는지" 한 줄을 사용자에게 명시**한다(예: "code 단계 / full 강도로 검증합니다").

전체 실행 절차·단계 역할표·공통 규약·가드레일은 `references/flow.md`를 따른다.

## engram 통합 (스킬 밖 바인딩)

engram 특화 — **코드 게이트 체크리스트의 우리 불변식**(kill 인과·finalize 1회·락 순서·epoch 재구독·replay→live), 보고서/커밋 게이트 위치, ADR 연동 — 은 그 프로젝트의 통합 지점(CLAUDE.md)과 `references/flow.md`의 "프로젝트 통합" 절에서 바인딩한다. 스킬 골격에 하드코딩하지 않는다.

## ⚠️ 검증 상태 (정직한 표시)

근거 강도를 섞지 않는다. 출처·상세는 `docs/research/review-pipeline-design-draft.md`.

- **단단함(근거 있음):** 거친 판정 스케일+체크리스트로 편향 차단(익명화로는 못 잡는다, F6) · 다른 family 다양성(opus+Codex, F8) · 단계별 특화 역할의 결함 커버리지(F1/F2) · 발산(PRD) 단계 블라인드의 앵커링 감소(F4) · 자기선호는 상수가 아니라(모델마다 달라) 사람 백스톱이 필요(F7) · 고정 Advocate/Adversary 골격(devil's advocacy / dialectical inquiry — 단 SW 리뷰 직접 증거가 아니라 전략의사결정 연구라 방향성).
- **약함/미검증:** 코드 단계의 **비대칭 blind/doc-aware(Codex blind breaker / opus doc-aware breaker)는 실증 0, 우리 가설** — 효과 측정 전까진 옵션으로 둔다 · **PBR 관점의 코드·문서 단계 적용은 요구/설계 인스펙션 실증의 외삽 — 그 단계에선 미검증** · "특화 역할이 *항상* generic보다 낫다"는 단정 금지(PBR 연구도 "perspectives가 항상 다르진 않더라"를 보임 — 방향성 우위지 절대선 아님).

이 스킬이 단일 모델·기존 방식 대비 실제로 더 나은 결함 검출을 내는지는 아직 대조 검증되지 않았다. 그 전까지 "근거 있는 가설"로 취급한다.
