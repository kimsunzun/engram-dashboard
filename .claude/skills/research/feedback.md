# research — 개선 히스토리

이 스킬을 쓰다 발견한 결함·개선점을 누적한다(덮어쓰기 금지). 반영은 사용자 승인 하에. 규약 = `SKILL.md` "자기개선 피드백" 절.

| 날짜 | 발견 | 상태 |
|---|---|---|
| 2026-06-26 | `flow.md` §0 강도표(medium Codex = "blind 1회")와 §3 본문("medium 이상=갈래별")이 불일치. 표가 정본 선언이라 §3 문구를 표에 맞추거나 둘을 명시 정렬해야 함. | 반영 (2026-07-01) |
| 2026-06-27 | Codex 독립 교차가 Claude 갈래에서 암시만 된 포인트(핸드오프 = 별도 레이어)를 명확히 정리 → cross-family 효과 실관찰. deep tier의 가치 근거 사례로 study-notes에 기록. | 부분 반영 (2026-07-01 — 리뷰어: "반영"은 과잉 청구. 관찰·정리는 됐으나 실증 대조는 미완, `⚠️ 미검증` 유지) |

> 2026-07-01: C1~C6(cross-family 수집 대칭화·effort tier 스케일·공동 심판 승격·에스컬레이션·evidence-grounded blind 심판·인용-근거 검증 패스)를 `_wip/research/` 스크래치 재설계에 반영. 위 §0↔§3 불일치는 C1/C8 재작성으로 강도표에 정렬해 해소. (승인·검증 후 원본 덮어쓰기 예정 — `REDESIGN-SPEC.md`.)
> 2026-07-01: v2 재설계 적용 — 모델→역할 추상화 + 단일 배정표 · 심판 독립 대칭 수정(양 family 모두 생산자≠심판, fresh 분리) · 연구 확립 종결 알고리즘(클레임별 판정→집계, 자기보고 % 무시, 정면충돌=3심판/contested) · mode-aware 에스컬레이션(옛 presence-aware 개명, 메인 판결 제거로 BLOCK 해소).
> 2026-07-01: **v3 재설계** — Codex를 수집자→적대 리뷰어로 전환(합성 산출물을 때림, 수집 이중화는 deep 옵션만) · "collector-agreement=confidence" 폐기(단일 웹 연출) · 라우팅(light/medium/deep = 싸게냐 vs 적대리뷰냐 판별)을 스킬 핵심으로 추가 · calibration 1급 규칙 승격 · grounding = 메인 외부 상시 pass · abstention≠contradiction 결함 수정 병합 · 2심판/cross-judge 폐기(리뷰어 단일). 실증 근거 = 30문항 SimpleQA(web/no-web) + 라이브 설계리뷰. medium+ 적대리뷰 값어치는 종합형 미검("근거 있는 가설" 유지).
> 2026-07-01: **v3.1** (Codex 적대 리뷰 라운드 2) — 누락(⑥ 완전성) 렌즈 추가 + 리뷰어 web_search 누락 탐침(omission blindness 방어) · medium+ cross-family grounding 스팟 재검증(메인 오독 방어) · "light처럼 보이지만 medium+" 라우팅 제외목록(버전·법규·통계·인용귀속·최신가격·시간민감) · 확신도 상한(확실=독립 교차확증 필요, 단일 출처는 가능성높음) · deep 비용 하드 백스톱(iter/검색/토큰 상한) · study-notes/로그 sink를 스킬 폴더→프로젝트 산출 경로로 이전(스킬 dir = SSOT 심링크 읽기전용) · 열화상태 라벨(리뷰 생략 시 "medium (적대 리뷰 생략)" 명시) · 정직 톤(가설 프레이밍 — 단일 적출을 효과 크기로 과청구 X).
> 2026-07-02: **v3.2** — 적대 리뷰 강도 레벨 사다리(2~5 · 축=리뷰어 독립도: 2 검산/3 홉 독립 재도출/4 다중렌즈+대안답 랭킹/5 blind 재해결) + tier 매핑(medium=2~3/deep=4~5, 강도표 정본) + 반박 verdict 반증 출처 강제(근거 없는 의심 오경보·비용폭발 방지) 추가.
> 2026-07-02: **study-notes 폐기(사용자 결정) + 첫 full `/review doc` 게이트 통과** — 학습용 노트 개념 전면 제거(목적 빗나감 판정: 노트 15파일 삭제 · SKILL.md "(임시) 학습용 rationale 노트" 절 제거 · v3.1의 "노트 sink를 프로젝트 산출 경로로 이전"을 **번복** — 자율모드 사후감사 sink = 보고서 쟁점/한계 섹션으로 일원화 · `_shared` usage-log 면제 제거 → research도 표준 usage-log 권장 적용). 게이트 FIX 반영: SKILL 레벨매핑 SSOT 위반 제거 · grounding 범위 복사 제거 · 판정표 "부분지지+반박" 행 추가 · light+자율 load-bearing 미지지 → medium 승격 명시.
