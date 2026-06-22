# ADR-0031: 검수 체계 — opus + Codex 2자 적대 리뷰 (웹 consult 폐기, 단계별 특화 역할)

- 상태: 확정 (2026-06-22, 근거: `docs/research/` 방법론 리서치 + 본 세션 합의)
- 관련: CLAUDE.md 「구현 실행 규약 · 리뷰어 역할 표」 · `docs/research/review-pipeline-design-draft.md`(상세 설계) · `docs/research/review-methodology-research-2026-06-22.md`(근거)

## 맥락
Codex(GPT CLI, `mcp__codex__codex`)를 도입했다. 그전까지 굵은 설계 교차검증·리뷰는 **웹 consult**(GPT·Gemini·Claude-opus 3종에 동일 프롬프트 → 블라인드 judge → correctness-merge)와 fable 1순위 LLD 리뷰어로 했다. Codex가 생긴 김에 "검수를 어떻게 구성하는 게 옳은가"를 재검토했다.

## 결정
웹 consult 폐기. 비자명 코드 변경·굵은 설계의 검수를 **opus + Codex 2자 적대 리뷰**로 통일한다.
- **구조(고정):** 모든 리뷰 쌍은 **Advocate(옹호·강화) vs Adversary(공격·대척)** — devil's advocacy/dialectical 쌍. 즉석 발명 금지.
- **특화(단계별 1회 픽스):** PRD=User/Tester · TRD=Designer/Architect-breaker · 코드=correctness/breaker · 문서정리=cut-advocate/load-bearing 수호. (운영 표 = CLAUDE.md, 상세 = 설계 문서)
- **모델 매핑:** 맥락(ADR·불변식) 필요 역할=opus(doc-aware), 신선 blind 역할=Codex.
- **판정:** PASS/FIX/BLOCK(점수화 금지), 취합 순서·라벨 무관. **불일치 → 메인 임의 판정 금지, 사용자에게.**
- **effort:** 메인 세션 xhigh / 코더·리뷰어 high(Codex medium 기본, 동시성·lifetime 치명 변경만 high).
- sonnet=하위 코더 전용(심판 아님), haiku=게이트 제외.

## 거부한 대안
- **웹 3패밀리 consult(GPT+Gemini+Claude blind judge) 유지** — 입력 다양성(3패밀리)은 더 크나, **판단 노드가 끝까지 Claude**(blind judge도 종합도 Claude opus)라 정작 신경 쓰는 편향(메인=Claude가 Claude식 답을 옳게 봄)을 *구조적으로 못 잡는다*. 비-Claude 판단(Codex)에 구속력(teeth)을 주는 게 그 축에선 더 효과적. 부수적으로 운영비용(web-runner 패널·브라우저 로그인·~12분 폴링·orch 취약)도 큼. 폐기로 잃는 건 *입력 다양성 중 Gemini 한 패밀리*뿐 — 편향은 어차피 consult도 못 잡고 있었다.
- **단일 리뷰어(fable/opus only)** — 교차 패밀리 부재 → 자기맹점. 적대 쌍·다양성 이점 없음.
- **generic Advocate/Adversary만(단계 특화 없음)** — 전용 기법을 든 특화 역할이 결함 커버리지가 더 크다(PBR). "단계별 픽스 ≠ 매번 즉석 발명"이라 피로도 늘지 않음. generic은 미지정 artifact의 fallback으로만.
- **블라인드(익명화)를 주 편향장치로** — 익명화는 앵커링을 *적당히*만 줄이고 합의(reliability)를 떨군다. 거친 루브릭(PASS/FIX/BLOCK)+체크리스트가 더 강한 debiasing.

## 근거
방법론 리서치(`docs/research/review-methodology-research-2026-06-22.md`, 적대검증 통과 항목):
- **루브릭/거친 스케일 > 익명화** (ICLR 자연실험: 4점 척도가 double-blind보다 prestige 편향 ~4× 더 감소).
- **다중 judge는 *다양성*이 활성 성분** (PoLL — 작은 다양한 패널 > 단일 대형 judge; 후속 연구는 *상관된 같은-family 패널은 효용 작음*을 보강). → opus+opus 무의미, opus+Codus(다른 family) 유의.
- **self-preference는 모델별 비균일**(부호 뒤집힘) → 단단한 반편향 백스톱은 블라인드가 아니라 **사람(사용자) 최종 판정**.
- **PBR** — 전용 reading 기법을 든 특화 관점이 비구조적 읽기보다 커버리지↑(단 "관점이 항상 다르진 않다"는 한정도 있음 → 특화는 방향성 우위지 절대선 아님).

## 영향 / 불변식
- 비자명 코드 변경마다 이 2자 리뷰가 게이트(스킵 금지). 코더·리뷰어 분리(메인은 오케스트레이션).
- **불일치는 사용자가 최종** — 메인(Claude 계열)이 임의 판정하면 반편향이 깨진다.
- 새 단계 유형 추가 시 특화 역할은 *한 번* 픽스(CLAUDE.md 표에 추가), 즉석 발명 금지.
- **미검증(실험 옵션):** 코드 단계 "Codex blind / opus doc-aware" 비대칭은 실증 근거 없는 가설 — 효과 측정 전까진 강제 아님.
