# research 스킬 재설계 스펙 (작업본 — 승인 후 원본 덮어쓰기)

> 이 파일은 `_wip/research/` 스크래치 작업본. 검증(/review doc + 시험 리서치) 통과 후
> `.claude/skills/research/`에 덮어쓴다. 그 전엔 라이브 원본 불변.
> 근거·문헌은 대화 세션(2026-07-01, "research" 세션) 참조.

## 문제 진단 (왜 고치나)

현 스킬은 **cross-family 독립 + 적대검증 + "만장일치≠정답"** 으로 신뢰도 축은 SOTA 상위.
하지만 **Codex가 구조적으로 약한 다리**라 핵심 메커니즘이 깨진다:

| 축 | Claude | Codex | 문제 |
|---|---|---|---|
| 수집 너비 | 갈래별 서브에이전트 N명 | 1회 | Codex 침묵이 "안 찾음"인지 "없음"인지 불명 |
| 강도 스케일 | tier 따라 증가 | medium 고정 | deep이어도 Codex 안 세짐 |
| 역할 | 조사만 | 조사만 | 심판이 Opus 단독 = Claude 집안 자기편향 |

**급소:** Codex 커버리지가 얕으면 "한쪽만 있는 항목 = 환각 의심" 논리가 무너진다
(Codex의 누락이 오답 신호가 아니라 게으름일 뿐). 교차검증의 핵심 가치가 이 항목에 있는데.

## 변경 목록

### C1. Codex 수집 대칭화 (급함 — 위 급소 복구)
- **medium:** Codex 1명이되 검색량·effort를 Claude 팬아웃 **총 커버리지에 맞춤**(인원 대신 깊이).
- **deep:** Codex도 **갈래별 병렬 팬아웃**(메인이 Codex N개 병렬 호출 — Codex는 자기 서브를 못 낳으니 오케스트레이터가 띄움).
- **light:** 지금처럼 Codex 1명 경량 유지.
- **[열린 결정]** medium을 (a) Codex 깊이↑ 로 갈지 (b) Codex도 갈래 팬아웃으로 갈지 —
  기본 추천 = **medium=(a), deep=(b)**. Codex MCP가 느려 medium 팬아웃은 무거움.

### C2. Codex effort tier 스케일
- light=medium / medium=medium / **deep=high**. (`model_reasoning_effort`)
- 현 §3 "medium 고정" 버그 동시 해소.

### C3. Codex를 공동 심판으로 승격 (Opus 자기편향 차단)
- 불일치 항목을 **Opus + Codex가 각자 독립 판정**(같은 근거, 서로 안 보고).
- 심판끼리 또 토론 금지(sycophancy 수렴). 갈리면 → 메인이 **판결 말고 쟁점 정리만** → 에스컬레이션(C4).
- 이건 자매 `review` 스킬이 이미 쓰는 구조(opus+Codex, PASS/FIX/BLOCK, 불일치→사용자)를 research에 이식.
- 주의: GPT×Claude도 오차 상관 있음 → 2번째 심판은 "표 2배"가 아니라 **불일치 탐지기**로 취급.

### C4. presence-aware 에스컬레이션
| 사용자 | 남은 심판 불일치 처리 |
|---|---|
| 없음(자율) | 메인이 근거로 판정 + **판정·근거 로그** 남겨 사후 검수 |
| 있음 | **굵은 쟁점만** 질문. 마이너(결론 영향 X)는 메인 자율 |
- 마이너/굵음 게이트 = 기존 tier 트리거(되돌리기 비용·불확실성·환각 피해·출처 분쟁) 재활용.

### C5. 심판을 evidence-grounded + blind + 순서스왑으로
- 판정은 "어느 보고서가 더 좋나"(주관) 금지 → **"이 클레임이 인용 출처로 검증되나"**(근거)로만.
  (self-preference는 근거기반 판정에선 거의 사라짐 — 문헌.)
- 접전 클레임은 **authorship 블라인드**(어느 게 Claude/Codex 건지 가림) + **순서 스왑**.

### C6. 인용-근거 검증 패스 (환각 직격)
- 종합 후, 핵심 클레임마다 **인용한 출처가 실제로 그 주장을 뒷받침하나** 확인(ALCE/CitationAgent식).
- medium=핵심 스팟체크 / deep=전수.

### C7. (별건, 나중) 평가 하네스
- 루브릭(factual/citation/completeness/source quality) + 단일콜 judge 0–1+pass/fail.
- `study-notes/`를 eval셋 씨앗으로 → 단일Claude vs 이 스킬 환각률·토큰 대조.
- 스킬 본체와 분리된 측정 도구라 우선순위 뒤. `⚠️ 미검증` 태그를 이걸로 해소.

### C8. (문서 hygiene) flow.md §0 강도표 ↔ §3 본문 불일치 정리
- feedback.md에 이미 "미반영"으로 적힌 2건 함께 처리.

### C9. (별건) PRD/TRD용 binding 신설 — research/bindings/engram.md
- review처럼 프로젝트 특화를 외부화. 담을 것: 로컬 코드/ADR 리더를 1급 수집 갈래로 ·
  제약 적합도 표 적대검증 · /review prd 핸드오프 · 코드복붙 시 라이선스 게이트.
- §7 설계-결정 모드는 제네릭 shape로 유지.

## 착수 순서 (레버리지 순)
C1·C2·C3·C4·C5 (핵심 대칭+심판+에스컬레이션, 한 묶음) → C6 (인용검증) → C8 (hygiene)
→ C7 (평가, 별도) → C9 (binding, 별도)

## 게이트
load-bearing 스킬이라: 코더 서브에이전트(작업본에서) → `/review doc`(opus+Codex 적대)
→ 시험 리서치 1회 → 통과하면 원본 덮어쓰기 + 커밋.

## v2 확정 — 종결 알고리즘 (정본 · pseudo-rule)

2026-07-01 v2 재설계로 확정. `flow.md §4`(집계) + `§5`(종료 조건)에 인코딩됨. 여기에 영속.

```
# 입력: 클레임 C, 두 fresh 심판(주 심판 = 주 계열 / cross-family 심판 = Codex)
#       각 심판은 상대 family 수집을 우선 검증(cross-judge), authorship-blind + 순서 스왑

FOR each 심판 j in {주 심판, cross-family 심판}:
    verdict[j] = judge(C, cited_sources)      # ∈ {지지, 부분지지, 미지지, 불확실}
                                              # NLI/함의 근거, 출처 스팬 인용
    # 심판 자기보고 %확신도는 무시(과신 편향)

# 메인이 기계적 집계 — 판결 아님(승자 강요 X)
IF verdict[주] == verdict[cross]:                       accept;         tag = 확실       # 동일
ELIF {verdict[주], verdict[cross]} == {지지, 부분지지}: accept(약한 쪽);  tag = 가능성높음  # 인접
ELIF 불확실 in {verdict[주], verdict[cross]}:            hold;           tag = 불확실
ELIF {verdict} == {지지, 미지지}:                        # 정면 충돌 — 메인은 승자 안 고름
    IF 제3 family 가용:  spawn 3rd-family judge(≠ 두 family); accept 다수결
    ELSE (제3 없음 / 여전히 교착 / load-bearing):        mark contested; tag = 불확실; escalate()

# 확신도 태그는 합의도에서 파생(자기보고 금지): 만장일치→확실 · 인접→가능성높음 · 충돌/보류→불확실

escalate():   # mode-aware — 자율 vs 대화 감지
    IF interactive & load-bearing:  ask_user(contested claim)
    IF autonomous:                  keep tag(contested/불확실);      # 조용히 해소 X
                                    log → 보고서 쟁점/한계 + study-notes(사후감사)
                                    flag 하류 결정 that hinge on contested load-bearing claim
    IF minor(결론 영향 X):          proceed(메인 자율)                # 양 모드 공통

# 전체 보고서 종료(loop-until-dry → 검증):
#   stop when (a) 모든 sub-question 커버 AND (b) 마지막 iteration에 새 load-bearing 공백/불일치 없음
#             AND (c) 모든 클레임이 판정(불확실 포함) 보유  → THEN 인용-근거 검증 pass

# 금지: 합의로 몰아가는 토론(sycophancy) · 충돌 평균내기 · same-family tiebreak · 만장일치=증명
```
