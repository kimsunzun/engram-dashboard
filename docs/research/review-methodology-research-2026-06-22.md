# 리뷰/검증 방법론 리서치 (다단계·다중리뷰어)

- 작성: 2026-06-22
- 출처: deep-research 하네스 (5각도 팬아웃 → 26 소스 페치 → 115 주장 추출 → 상위 25 적대검증 → 19 confirmed / 6 killed)
- 목적: "어떤 모델을 쓰냐"가 아니라 **검토 품질을 구조로 끌어올리는 방법론** 조사 — 단계별 관점, 블라인드, LLM-judge 편향 완화, 멀티에이전트 패턴.
- 주의: 각 발견에 **확신 수준**과 **한정**을 같이 둔다. 단일 수치를 보편 법칙으로 인용 금지.

---

## 1. 검증된 발견 (confirmed)

### F1. 관점/렌즈 기반 리뷰 = Perspective-Based Reading (PBR) — [확실]
리뷰어마다 **다른 이해관계자 시점**(tester / developer / user)과 **전용 reading scenario**(user=use-case, tester=equivalence partitioning, designer=structured analysis)를 배정하는 검토 설계. 전제: 서로 다른 관점이 **겹치지 않는(non-overlapping) 결함**을 찾아 합집합으로 커버리지를 넓힌다.
- 근거: Basili 원논문(peer-reviewed) verbatim — "members of a review team read a document from a particular perspective… combination of different perspectives provides better coverage than the same number of readers using their usual technique." (3-0)
- 출처: cs.umd.edu/~mvz/handouts/emp_pbr.pdf · link.springer.com/article/10.1007/BF00368702 · springer 978-3-642-29044-2_13

### F2. PBR 팀 효과는 실증됨 — [확실, 단 한정 큼]
관점별 1인 팀이 비구조적 리뷰보다 통계적으로 유의하게 높은 결함 커버리지(1995 run: 일반 문서 p=0.0007, NASA p=0.0390). 메커니즘 = 관점 간 non-overlapping coverage.
- **한정:** 이 "팀"은 실험 후 permutation으로 만든 **SIMULATED 팀**(실제 협업 팀 아님). 개인 단위는 혼재(일반 p=0.0019 유의 / NASA p=0.4755 비유의). NASA 문서는 관점 간 겹침이 커 팀 이득이 약했음.

### F3. PBR vs CBR(체크리스트) — 보편적 승자 없음, 문맥 의존 — [확실]
- 한 통제 실험(59명, UML/OO 설계): 개인 탐지율 PBR ~69% / CBR ~70% (거의 동일). CBR가 cost-per-defect 낮고 3인 가상팀에선 CBR 우세. 반대로 Laitenberger(2001)는 PBR 우세.
- "Are the Perspectives Really Different?"(Lanubile & Visaggio): 세 관점 간 탐지율·시간·커버리지에 **유의차 없음** — PBR의 차별화 전제를 반증형으로 검증.
- **결함 분산의 최대 요인은 reading 기법이 아니라 specification 문서 자체였다.**
- 함의: 관점 배정은 쓰되 "관점이 곧 우월"로 과신 금지.
- 출처: academia.edu/53008777 · thescipub.com/pdf/jcssp.2017.470.495.pdf · springer 978-3-642-29044-2_13

### F4. Double-blind는 prestige(앵커링) 편향을 *적당히* 줄임 — [확실]
ICLR 5,027편 자연실험(2017 single→2018 double-blind). 최상위 피인용 저자군 점수가 유의하게 하락. 단 효과가 작아 **합격률을 바꿀 정도는 아님**.
- 출처: asistdl.onlinelibrary.wiley.com/doi/full/10.1002/asi.24582 (Sun et al., JASIST 2022, arXiv:2101.02701)

### F5. 블라인드의 부작용 = 합의(reliability) 하락 — [가능성 높음]
저자 신원을 가리자 **같은 논문에 대한 리뷰어 간 평점 분산이 유의하게 증가**(맥락/공유단서 상실). 
- **비만장일치:** Bornmann et al.(PLOS One) 메타분석은 blinding의 inter-rater reliability 효과 null. → "한 대형 CS 컨퍼런스 연구의 증거"로 다룰 것.

### F6. ★루브릭/스케일 설계가 블라인드보다 강한 debiasing 수단★ — [가능성 높음]
같은 ICLR 데이터셋: **평점 척도 10점→4점(거칠게) 변경이 double-blind보다 prestige 편향을 약 4배 더 줄임**(prestige의 합격 영향 ~3%→~1.7%).
- **한정:** 단일 venue 관찰연구, 두 개입이 다른 연도(temporal confounding).
- 함의(우리 설계의 핵심): **거친 판정 스케일(PASS/FIX/BLOCK) + 잘 짠 체크리스트**가 익명화보다 우선순위 높은 편향 통제 수단.

### F7. LLM self-preference(자기 모델 선호)는 보편 법칙이 아님 — [가능성 높음]
모델마다 부호가 뒤집힘(한 연구: Flash +0.56 vs Pro −0.22). 여러 모델에서 *음의* self-preference도 보고됨.
- **한정:** 주 소스가 single-author 비peer-review preprint(arXiv:2604.23178), sample size 불명 → 정량치 신뢰 낮음. 단 "비보편성"은 다중 소스로 robust.
- 함의: "메인=Claude라 무조건 Claude 편든다"는 **확정이 아니다**. 자기선호를 상수로 두고 설계하지 말 것.

### F8. 다중 judge 패널(PoLL): 다양성이 활성 성분, 머릿수가 아님 — [확실]
작은 **다양한** 모델 패널이 단일 대형 judge보다 인간 일치↑, intra-model bias↓, 7배 저렴(Cohere PoLL, arXiv:2404.18796). ensemble이 개별 judge 대비 인간 일치(kappa) 향상(arXiv:2408.09235).
- **보강:** 2026 후속 "Nine Judges, Two Effective Votes"(arXiv:2605.29800) — **상관된(같은 계열) judge의 naive 패널은 효용 작음, diversity가 필수**.
- 함의: opus+opus(같은 family)는 약하고, **opus+Codex(다른 family)가 의미 있는 페어**.

---

## 2. 기각된 주장 (채택 금지 — 적대검증 탈락)

| 주장 | 투표 | 비고 |
|---|---|---|
| PBR가 CBR보다 검사 *시간* 우위 | 1-2 | 시간 효율 주장 근거 부족 |
| PBR-CBR는 일률적 yield↔cost 트레이드오프 | 0-3 | 단순 트레이드오프 공식 성립 안 함 |
| reference 답을 주면 self-preference 사라짐 | 1-2 | reference-guided가 편향 제거 못 함 |
| **position(순서) 편향은 무시 가능(≤0.04)** | 0-3 | **순서 편향은 실재 → 답 위치 스왑은 유효** |
| 완화책 다 합치면(swap+CoT+rubric) 최대 이득 | 0-3 | 결합 최대이득 주장 근거 부족 |
| diverse family 패널이 self-preference를 *직접* 완화한다는 증거 | 1-2 | 패널 이점을 self-preference debiasing으로 귀속 금지 |

---

## 3. 공백 (이번 조사로 못 채운 것)

- **비대칭 리뷰어(한 명 컨텍스트 blind, 한 명 doc/불변식-aware)** — 명시적 실증 연구·실무 패턴 **확인 안 됨.** 우리 가설이며 미검증.
- **멀티에이전트 리뷰 패턴(generator-critic, self-refine/Reflexion, multi-agent debate, red-team/blue-team, devil's advocate)** 효과 비교 — 이번 검증셋에 포함 안 됨(미해결).
- **PBR 관점 차별화가 코드 리뷰 단계에서 유효한지** — 검증은 요구사항/설계 문서에 집중. 코드 단계 매핑은 외삽.
- rating-scale coarsening의 debiasing이 LLM judge의 verbosity/position bias로 이전되는지 — 증거 부재.

---

## 4. 한계 (전반)

- 인스펙션 문헌(PBR/CBR)은 1996~2017 — 성숙·안정 분야라 vintage 자체는 결격 아니나, **측정 단위(개인 vs 팀)·artifact 유형(요구사항 vs 설계 vs 코드)에 따라 결론이 갈림.**
- 블라인드 관련 결론(F5·F6)은 **단일 venue(ICLR) 관찰연구 하나**에 의존. F5는 메타분석과 충돌.
- LLM-judge 비보편성(F7) 주 소스는 약한 preprint.
- 대부분 **인간 인스펙션·학술 peer review** 증거를 LLM 코드 리뷰로 외삽한 것 — 직접 LLM 코드리뷰 실증 아님.

---

## 5. 우리 설계로의 시사점 (요약)

1. **편향 통제 1순위 = 단계별 체크리스트 + 거친 판정 스케일(PASS/FIX/BLOCK)** — 블라인드보다 강함(F6).
2. **다중 리뷰어의 활성 성분 = family 다양성** — opus+Codex가 의미, opus+opus는 약함(F8).
3. **관점(렌즈) 배정** — 단계별로 서로 다른 시점 2개(F1/F2). 단 "관점=우월"로 과신 금지(F3).
4. **블라인드는 선택적** — 앵커링 위험 큰 단계(발산)에만, reliability 비용 인지. 비대칭 blind/doc-aware는 **미검증 보너스**로 표시(공백).
5. **자기선호는 상수 아님** — 단단한 백스톱은 블라인드가 아니라 **다양성 + 거친 루브릭 + 사람(최종 판정)**(F7).
6. **순서 편향은 실재** — 리뷰 결과 취합 시 순서 무관·라벨 익명화(기각표에서 역증).
