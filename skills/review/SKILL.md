---
name: review
description: 변경물을 2인 적대 리뷰로 검증한다. 비자명 코드·설계·문서 변경 검증에 사용. 트리거 /review [prd|trd|code|doc] [self|light|full|deep].
---

# Review

변경물을 **Advocate(옹호·강화) vs Adversary(공격·대척)** 2인으로 적대 검증한다. 단계가 전용 역할 렌즈를 박아 결함 커버리지를 올리고, 불일치를 사용자에게 넘겨 자기편 편향을 차단한다.

**실행 전 `references/flow.md`를 반드시 Read 한다 — 안 읽고 리뷰어 스폰 금지.** 전체 절차·강도표·단계 역할표·가드레일이 거기 있다. `$ARGUMENTS` = 단계 `prd`|`trd`|`code`|`doc` [+ 강도 `self`|`light`|`full`|`deep`] — 둘 다 옵션. 파싱·추정은 `references/flow.md §0-1`.

## 핵심 설계 (불변)

- **2인 적대 = Advocate + Adversary.** 두 리뷰어는 **다른 family**라야 학습 편향이 갈려 교차검증이 성립한다 — 같은 family 둘은 편향이 안 갈린다.
- **불일치 → 사용자.** Advocate·Adversary가 정면으로 갈리면 메인이 임의 확정하지 않고 사용자에게 쟁점을 보고한다 — 메인은 자기 family 편향이 있어 사람이 백스톱이다.
- **escalation-only.** 시작 강도가 하한이다. 도중 위험 트리거를 발견하면 상위로만 승격하고 알린다 — 임의 하향 금지.

## 트리거

`/review [prd|trd|code|doc] [self|light|full|deep]`. 호출 시 **"어느 단계·어느 강도로 도는지" 한 줄을 사용자에게 명시**한다(예: "code 단계 / full 강도로 검증합니다").

## 자기개선 피드백

결함·개선점은 그 자리서 고치지 말고 작업 종료 후 `feedback.md`에 누적한다(검증 상태도 그쪽이 정본). 전체 규약 = `../_shared/self-improvement-feedback.md`.
