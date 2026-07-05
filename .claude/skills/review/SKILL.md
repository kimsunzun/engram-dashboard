---
name: review
description: 변경물을 Advocate(옹호·강화) vs Adversary(공격·대척) 2인 적대 리뷰로 검증한다. 단계(prd/trd/code/doc)가 역할 렌즈를, 강도(self/light/full/deep)가 인원·깊이를 고른다. 두 리뷰어는 다른 family라 학습 편향이 갈린다. 판정 PASS/FIX/BLOCK, 불일치는 사용자에게 에스컬레이션한다. 비자명 코드·설계·문서 변경 검증에 사용. 트리거 /review [prd|trd|code|doc] [self|light|full|deep].
---

# Review

변경물을 **Advocate(옹호·강화) vs Adversary(공격·대척)** 2인으로 적대 검증한다. 단계가 전용 역할 렌즈를 박아 결함 커버리지를 올리고, 불일치를 사용자에게 넘겨 자기편 편향을 차단한다.

**실행 전 `references/flow.md`를 반드시 Read 한다 — 안 읽고 리뷰어 스폰 금지.** 전체 절차·강도표·단계 역할표·가드레일이 거기 있다. `$ARGUMENTS` = 단계 `prd`|`trd`|`code`|`doc` [+ 강도 `self`|`light`|`full`|`deep`] — 둘 다 옵션. 파싱·추정은 `references/flow.md §0-1`.

## 핵심 설계 (불변)

- **2인 적대 = Advocate + Adversary.** 두 리뷰어는 **다른 family**라야 학습 편향이 갈려 교차검증이 성립한다 — 같은 family 둘은 편향이 안 갈린다.
- **2축 직교 — 단계 × 강도.** 단계(prd|trd|code|doc|fallback)가 **어느 역할 렌즈·블라인드·체크리스트**를 쓸지 고르고, 강도(self|light|full|deep)가 **리뷰어 인원·깊이**를 고른다. 한 축이 다른 축을 정하지 않는다.
- **self 예외 — 리뷰어 0인.** self는 역할 렌즈를 안 쓴다. 단계 인자는 *렌즈*가 아니라 self 체크리스트·QA 범위를 고른다. 리뷰어 렌즈는 light부터 켜진다.
- **판정 = 거친 3단 PASS/FIX/BLOCK.** 점수화·미세 등급 금지.
- **불일치 → 사용자.** Advocate·Adversary가 정면으로 갈리면 메인이 임의 확정하지 않고 사용자에게 쟁점을 보고한다 — 메인은 자기 family 편향이 있어 사람이 백스톱이다.
- **escalation-only.** 시작 강도가 하한이다. 도중 위험 트리거를 발견하면 상위로만 승격하고 알린다 — 임의 하향 금지.

## 강도 (언제)

> 정본 = `references/flow.md` 강도표(리뷰어 인원·깊이). 아래는 "강도·언제"만 — 여기 안 베낀다(rot 방지).

| 강도 | 언제 |
|---|---|
| **self** | 1~2줄·문서 오타·자명한 변경 |
| **light** | 국소·저위험·단일 관심사 변경 (위험 영역 사전 배제) |
| **full**(기본) | 비자명 변경 (프로젝트 기본 게이트) |
| **deep** | 고위험 — 동시성·kill·lifetime·보안·공개 API·마이그레이션·핫패스 |

강도 신호 = LOC·위험 영역·신규/리팩터·자동생성. 무거울수록 위. 애매하면 full. 상세·escalation은 `references/flow.md`.

## 트리거

`/review [prd|trd|code|doc] [self|light|full|deep]`. 호출 시 **"어느 단계·어느 강도로 도는지" 한 줄을 사용자에게 명시**한다(예: "code 단계 / full 강도로 검증합니다").

## 프로젝트 바인딩

전용 체크리스트·명령·연동(code 단계 코드 불변식, QA 실측 명령, 결정 기록)은 `references/bindings/<project>.md`. 현재 engram = `references/bindings/engram.md`. 다른 프로젝트는 같은 골격에 바인딩 파일만 추가한다. 골격에 특정 스택·불변식을 하드코딩하지 않는다.

## 자기개선 피드백

이 스킬을 쓰다 발견한 결함·개선점은 그 자리서 고치지 말고 작업 종료 후 이 폴더 `feedback.md`에 한 줄 누적한다(반영은 관련 주제 재등장 시 사용자 승인 하에). 검증 상태도 `feedback.md`가 정본이다. 전체 규약 = `../_shared/self-improvement-feedback.md`.
