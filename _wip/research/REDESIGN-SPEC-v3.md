# research 스킬 v3-final — 실증 기반 재설계 (2026-07-01)

> 30문항 실측(SimpleQA web/no-web) + 이 세션의 Codex 설계리뷰로 확정. v2(cross-family 수집 이중화 + 2심판)를 폐기·전환한다.

## 실측이 뒤집은 것 (근거)
1. **cross-family "수집 이중화"는 사실조회에서 군더더기** — 웹으로 찾아지는 사실은 단일 Claude ~100%, 둘 다 같은 웹 참조 → 교차 이득 0·비용 2배·오경보 1. (Codex 비판 "같은 웹→같은 오차" 실측 확인)
2. **cross-family의 진짜 가치 = "수집"이 아니라 "적대 리뷰"** — confident-wrong(모델이 확신에 차 틀리고 자기 못 잡음)은 다른 family 리뷰만 잡는다. 이 세션에서 Codex 리뷰가 메인의 confident-wrong(설계 결함·과장 인용)을 실제로 적출 = 라이브 증거.
3. **모델은 이미 잘 calibrated** — obscure 사실에 확신-환각 대신 기권/저확신. "모르면 기권 + 저확신 불신" 단일 규칙이 사실조회 교차의 대부분을 대체.
4. **가치는 라우팅에 있다** — "findable-fact냐(→싸게) vs 종합·논쟁·confident-wrong 위험이냐(→적대리뷰)" 판별이 스킬 핵심 산출.

## 핵심 구조 (v3)
- **수집 = 단일 주계열(Claude) + 정직한 확신도/기권.** 병렬 cross-family 수집 제거(deep 옵션으로만).
- **cross-family = 적대 리뷰어(Codex).** 합성된 주장/보고서를 때린다: 근거 없는 주장·과장·오귀속·논리공백·confident-wrong. (= `/review` 패턴을 research 산출물에 적용)
- **grounding = 메인 외부 체크(셀프 금지) — 상시.**
- **calibration = 1급 규칙** — 수집자는 모르면 기권/저확신 명시. 저확신·근거없음은 불신.

## 티어 (라우팅 = 핵심)
- **light** (findable fact·저위험): 단일 수집 + grounding + 기권 → 출처 포함 요약. Codex 리뷰 없음.
- **medium** (종합·비교·논쟁·confident-wrong 위험): + **Codex 적대 리뷰** of 산출물. 리뷰 지적 → 수정 or 불확실 태그.
- **deep** (고위험·비가역): + gap 반복(loop-until-dry) + 사람 백스톱. (deep에서만 cross-family 병렬수집도 옵션)

**라우팅 가이드 (스킬에 명시 — 핵심):**
- 단발 사실·웹서 바로 찾아짐 → **light** (교차 불필요)
- 여러 주장 종합·해석·논쟁적·1차자료 희소·"확신하는데 틀리면 치명" → **medium+** (Codex 리뷰)
- 비가역·고비용·안전치명 → **deep**

## 판정/종결
- 클레임별: grounding{지지/미지지/불확실} + (medium+) Codex 리뷰 판정.
- **abstention ≠ contradiction (v2 결함 수정):** 한쪽 기권 + 다른쪽 근거있는 답 → grounding 통과 시 채택(정답 안 버림). **서로 다른 답을 낼 때만** contested.
- **확신도 = grounding + 리뷰 결과** (자기신고 저확신은 강등). **"수집자 합의=확신"은 폐기**(같은 웹, 연출).
- contested/미지지 → **mode-aware 에스컬레이션** (interactive 질문 / autonomous 태그+로그+진행).

## 모델→역할 배정표 (Fable-ready · 한 곳만 모델명)
| 슬롯 | 모델 |
|---|---|
| 주계열 수집자 | Claude Sonnet |
| 주계열 합성·grounding | Claude Opus (← 신모델 나오면 여기 교체) |
| cross-family 리뷰어 | Codex (gpt) |
불변: 리뷰어는 주계열과 다른 family 강제.

## 폐기된 v2 요소 (실측/리뷰 근거)
- cross-family 병렬 수집(사실) → 제거(군더더기, deep 옵션만)
- "collector agreement = confidence" → 폐기(같은 웹, 연출)
- 2 blind 심판 전수판정 → 과함 → Codex=리뷰어 단일화
- 3rd-family tiebreaker 불변식 → 3rd family 미보유 → 가용 시에만(조건부)

## ⚠️ 미검 (정직)
- 종합형 과제(DeepResearch Bench)에서 Codex 적대리뷰의 값어치는 아직 정량 미검. 사실조회 판정만 실측됨. "근거 있는 가설"로 표기 유지.
