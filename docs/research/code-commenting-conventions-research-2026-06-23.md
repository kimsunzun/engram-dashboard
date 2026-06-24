# 코드 주석 관행 리서치 — 에이전트 시대의 주석 (상세 기록)

> **상태:** **채택: 선택지 B (ADR-0032, 캐논 `docs/reference/commenting-conventions.md`). 코드는 점진 적용(boy-scout).** Stage 1·1.5·2 완료.
> **단계:** PRD/컨설 (조사 → 선택지 → 사용자 결정) 채택 완료. 코드는 boy-scout 점진.
> **방법:** Claude(Sonnet) 4 + Codex 4 **갈래별 대칭 독립 조사**(Stage 1) → Opus 적대 검증 2(Stage 1.5) → Opus 종합·교차대조(Stage 2).
> **날짜:** 2026-06-23 · **plan:** `~/.claude/plans/goofy-conjuring-walrus.md`
> **확신도 표기:** (확실) 1차 출처·다수 수렴 · (가능성 높음) 정황·단일 출처 · (불확실) 미확인/추정.

---

## 0. 왜 (Context)

**사용자 질문/가설:** 에이전트 방식 개발로 사용자가 코드 전체를 직접 읽지 않게 되니, "주석이 더 자세히 작성되어야 사용자가 그 부분만 보고 **한눈에 이해**할 수 있다"는 가설. 이게 맞는지, 그리고 업계 주석 관행이 에이전트 시대에 어떻게 바뀌는지 확인.

**engram 현황 진단 (사전 Explore):**
- 동시성·kill·race 핵심(`output_core.rs`/`reaper.rs`/`session.rs`)은 주석 밀도 22~28%로 양호. `// ADR-NNNN` 앵커 36개, 파일 헤더(`//!`)·"왜" 한국어 주석으로 CLAUDE.md 컨벤션 ~85% 준수.
- 약점: ADR 앵커가 부분적(load-bearing 지점 대비), `session_tracker.rs` 내부 분기(resolve/poll/PID shim)와 일부 큰 테스트는 "처음 보는 사람" 기준으로 헤맴.
- 주석 전용 ADR 없음. 원칙은 CLAUDE.md "컨벤션" 절에만(why-only·자명한 건 금지·load-bearing 의도 박기·`ENGRAM_DATA_DIR` 오삭제 사례).

이 리서치의 결론 줄기 → engram 주석 컨벤션을 어떻게 다듬을지 **선택지+트레이드오프**를 사용자 결정용으로 정리(임의 채택 금지).

---

## 1. Stage 1 — 경험적 조사 (Claude×Codex 교차)

> 조사 설계: 갈래1·2 = 사실 분담(각 family 1명), 갈래3 = 회의론 vs 옹호 대치(각 family 2명). 두 family가 서로를 못 보게 독립 조사 → §1.D에서 교차대조.

### A. 갈래1 — 전통 주석 관행 분류 (taxonomy)

두 family가 **거의 완전히 수렴**. 핵심 분류:

- **why-not-what 원칙 (확실)** — 코드는 *how*를 보여주고 주석은 *why*(의도·배경·제약)를 말한다. 출처: Atwood "Code Tells You How, Comments Tell You Why"; Google C++ Style Guide(self-documenting names 우선 + tricky/non-obvious·"why chose this implementation" 요구).
- **self-documenting code 논쟁 (확실)** — "코드가 곧 문서"이나 ≠"코드만 문서". Martin Fowler "CodeAsDocumentation": 코드가 primary documentation일 수 있지만 supplementary documentation 필요. 극단적 자기문서화는 추상화 과잉·흐름 단절 역효과.
- **"주석은 실패의 변명" 계열 (확실)** — *Clean Code*(Martin) "Don't comment bad code—rewrite it". 단 이는 **과잉 `what` 주석을 겨눈 경고**이지 API 계약·ADR·불변식·동시성 제약·역사적 이유까지 지우라는 뜻이 아님(오해 시 위험).
- **comment rot / staleness (확실)** — 컴파일러가 주석 갱신을 강제하지 않아 코드와 발산. 대응: 주석 최소화가 아니라 코드 근처 배치 + 테스트가능 예제(doctest) + lint·review로 갱신 압력. 출처: arxiv 2403.00251(CoCC).
- **literate programming (확실)** — Knuth(1984): 프로그램은 인간 독자에게 하는 통신. weave/tangle. 현대 계승: Jupyter, Quarto, doctest. LLM 시대 NL Outline으로 부활(§B).
- **doc-comment 시스템 (확실)** — rustdoc(`///`,`//!`, 첫 줄 요약·Markdown·doctest), JSDoc(`@param`/`@returns`, TS `checkJs` 연동), Python docstring(PEP 257, 런타임 `__doc__`), Javadoc(IDE 경고·사실상 강제), Doxygen(C/C++ 다언어). 공통 추세: signature와 중복되는 타입 설명은 줄이고 contract·examples·panics/errors/safety에 집중.
- **ADR (확실)** — Nygard(2011): Title/Status/Context/Decision/Consequences. 핵심은 **거부한 대안 + 이유**. 코드 옆 repo 버전관리. 사소한 결정까지 ADR화하면 잡음.
- **Diátaxis (확실)** — Procida: tutorial/how-to/reference/explanation 4유형 분리. 코드 주석 taxonomy가 아니라 사용자 문서 정보구조이나, "한 문서가 여러 목적 혼재 → 품질 저하" 진단이 주석에도 적용 가능.
- **주석 종류 분류 (확실)** — 목적별(설명/라이선스/메타데이터/TODO·FIXME/디버그/지시문/문서화), 위치 기반 규칙은 lint 쉬우나 "좋은 내용"은 자동판별 어려움. TODO/FIXME는 issue tracker 연결 안 하면 방치.

**Codex의 종합 프레이밍 (가능성 높음, 유용):** 좋은 관행은 "주석을 줄인다"가 아니라 **"주석의 책임을 좁힌다"** — `what`은 이름/타입/구조/테스트가 맡고, `why·contract·edge case·decision rationale·user task`를 각각 **local comment → doc-comment → ADR → Diátaxis 문서** 4층위로 분리. (이 층위 모델이 §3.1 권고의 뼈대)

### B. 갈래2 — 에이전트/LLM 시대 변화 (2024~2026)

두 family 강하게 수렴. 무게중심이 **"코드 설명"에서 "사람+AI agent 공용 intent/context/guardrail"로 이동**.

- **(i) 사람이 AI 코드를 리뷰하는 환경:**
  - 커밋 코드의 ~42%가 AI 지원(Stack Overflow 2025 Survey), 그러나 AI 신뢰 43%(2024)→33%(2025). (확실)
  - **verification debt** 부상: 개발자 96%가 AI 코드 correctness를 완전 신뢰 안 함, 상당수가 AI 코드 검토가 동료 코드보다 오래 걸린다고 응답(Sonar). (가능성 높음)
  - 리뷰 부담이 "작성"→"검증·이해"로 이동. Salesforce Engineering(2026): AI로 프로덕션 코드 30%↑ → 리뷰를 "intent reconstruction"으로 재설계. **diff만으로는 intent를 알 수 없다.** (확실)
  - Addy Osmani: PR 최소기준 "Intent in 1-2 sentences" — 설명 못 하면 커밋 불가. (가능성 높음)
  - **NL Outlines (확실, FSE 2025 채택)** — arxiv 2408.04820: docstring(계약)·inline(산발 근거)과 구분되는 **별도 계층** = 구현 전략을 함수/블록 단위 NL 요약으로. 코드 대비 ~절반 분량, "very helpful" 63%, 양방향 자동 동기화로 staleness 극복.
- **(ii) AI가 소비할 문서/주석:**
  - **에이전트 지침 파일 생태계 (확실)** — `AGENTS.md`(60k+ repos, 30+ agents), `CLAUDE.md`, `.cursor/rules`, `.kiro/steering/`. Princeton 실측: AGENTS.md 있을 때 런타임 28.6%↓·토큰 16.6%↓.
  - **경고 (확실):** LLM이 자동생성한 AGENTS.md는 README 중복으로 역효과(성공률 2%↓·비용 23%↑). **50줄 핀포인트 > 1000줄 망라**. 500줄 초과는 대부분 무시.
  - **llms.txt (확실 채택 / 불확실 실효)** — 2024-09 제안, 84만+ 사이트(Stripe·Cloudflare·Anthropic). 단 주요 LLM이 랭킹 신호로 쓴다는 공식 확인 없음.
  - **spec-driven development (가능성 높음)** — "the spec is the prompt". GitHub Spec Kit, AWS Kiro(Vibe↔Spec). Karpathy도 "vibe coding 시대 종료, agentic engineering"으로 선회. 단 "스펙 vs 코드 — 무엇이 진실의 원천인가"는 미합의.
  - **문서 = soft context, 강제 아님 (확실)** — Claude 문서: `CLAUDE.md`는 enforced configuration이 아니라 context. 실제 차단은 hooks/CI/CODEOWNERS 같은 **hard guardrail** 필요. (engram §5 LLM-우선 제어 설계와 직결)
  - **인라인 `// ADR-NNNN` 앵커 (가능성 높음)** — 에이전트 가이드레일로 확산. 단 arxiv 2602.04445: ADR은 아키텍처 결정만 잡고 *구현 수준 결정*(훨씬 많고 가장 잘 손실됨)은 못 잡는 갭 존재.

### C. 갈래3 — 회의론 vs 옹호 (대치)

| | 회의론(skeptic) | 옹호(pro) |
|---|---|---|
| 핵심 명제 | 코드만이 실행되는 진실. 주석은 검증 안 되고 rot한다 | 코드는 *how*만 보존, *why·rationale·invariant*는 주석만 보존 |
| 최강 권위 | Clean Code(Martin), Kernighan&Pike | Ousterhout *A Philosophy of Software Design* |
| 실증 무기 | eye-tracking(주석이 comprehension에 무영향)·comment rot 대규모 연구·link decay | 이해비용 50~70%·load-bearing 삭제방지·ABF 주석이 LLM 버그수정 개선 |
| 대안 | 좋은 네이밍·작은 함수·타입 시스템·테스트가 문서 | comments-first(설계 도구)·designNotes·NL outline |
| 에이전트 시대 | AI 생성 주석은 redundant·hallucination·검증부채 → 테스트/타입/static analysis로 | 사람이 안 짠 코드일수록 intent 주석이 유일한 맥락 채널·리뷰 기준선 |

두 입장 모두 **"`what` 주석은 나쁘다"에는 동의** — 진짜 다툼은 "*why*까지 줄여야 하나(회의론) vs *why*는 본질이라 키워야 하나(옹호)". 이 단층선 위 핵심 실증 주장을 §2에서 적대 검증.

### D. Claude ↔ Codex 교차검증 표 (수렴/발산)

| 갈래 | 수렴(두 family 일치) | 발산/보완 |
|---|---|---|
| 1 전통분류 | 8개 분류 항목·doc-comment 시스템·ADR·Diátaxis 전부 일치 | Codex만 "책임을 좁혀라" 4층위 모델 명시(발산 아닌 보강) |
| 2 에이전트시대 | intent로 이동·AGENTS.md/CLAUDE.md/llms.txt·spec-driven·NL Outline 일치 | Claude=NL Outline·Princeton 수치 강조 / Codex="문서=soft context, hard guardrail 필요" 강조 |
| 3a 회의론 | rot·self-documenting·Clean Code·types>주석·tests as docs 일치 | Codex만 link decay(arxiv 1901.07440)·"주석 품질 평가난" SLR 추가 |
| 3b 옹호 | Ousterhout·이해비용·load-bearing·literate·NL outline 일치 | Codex만 ABF(arxiv 2601.23059) "주석이 LLM 버그수정 ~3배" 추가 |

**결론: 두 family 간 실질 충돌 없음(수렴).** 차이는 전부 한쪽이 더 길게 다룬 *보완*. → 교차검증이 landscape의 신뢰도를 높임(확실). 단, Codex가 단독 제시한 두 실증 주장(eye-tracking 반대편의 ABF, link decay)과 회의론 최강 무기는 단일 출처라 §2에서 적대 검증 필요.

---

## 2. Stage 1.5 — 적대 검증 (Opus 2인, 웹 1차 출처 재확인)

### 회의론 실증 주장 — **요약보다 약함**

- **주장1 (eye-tracking, "주석이 comprehension에 무영향") — 판정: 부분사실 / 요약은 과장.**
  - 실재: Abdelsalam et al., *Empirical Software Engineering* 2025, DOI 10.1007/s10664-025-10721-2.
  - 실제: **n=20 학생**, ~20줄 단순 Java 12개, 주석 유무 이분(what/why 구분 **없음**). correctness에서 주석 효과 유의한 snippet은 **12개 중 1개**뿐. 그러나 time(efficiency)은 6/12 유의(일부는 주석이 *방해*).
  - 저자 본인 결론: *"the efficacy of comments is highly contextual — 어떤 경우 향상, 어떤 경우 방해."* → "주석 무용"이 **아님**. "readability vs comprehension" 이분 자체가 논문 측정과 어긋남(논문이 잰 건 visual attention·reading linearity). 저자가 학생 표본·단순 코드 일반화 주의 명시.
  - 회의론이 정직하게 인용 가능한 명제: "*이 좁은 조건에서* 주석 유무가 정답률을 유의하게 높이지 못함(11/12 비유의)" — 거기까지. (확실)
- **주장2 (comment rot 대규모 입증) — 판정: 사실확인, 단 끌어낸 결론은 비약.**
  - 출처·수치 정확: Wen et al. 2019(ICPC, 1,500 Java 프로젝트, "대부분 함께 진화 안 함"), CoCC arxiv 2403.00251(탐지기), 9.6M Links arxiv 1901.07440(~10% dead link). rot의 *존재*는 견고(확실).
  - **비약:** "rot 있음 → 자세한 주석 쓰지 마라"는 데이터 초과. 동일 데이터가 "주석을 코드와 함께 갱신/자동동기화하라"를 똑같이 지지(실제 후속연구 방향 = 탐지·자동수정). rot는 주석 *길이*가 아니라 *갱신 누락*의 함수.
- **종합:** 회의론은 "주석이 비용·실패모드를 가진다"까지는 데이터로 단단하나, "그러므로 줄여라/이해를 못 돕는다"의 마지막 한 걸음에서 두 주장 모두 증거를 초과.

### 옹호 실증 + 사용자 가설 — **방향 지지, 형태 수정**

- **주장1 (주석이 LLM 버그수정 ~3배) — 판정: 조건부지지.**
  - 실재: Vitale et al., arxiv 2601.23059. "3배"는 **CodeT5+ 220M** fine-tuned가 2011–2017 Java에서 3.26%→9.80%. **GPT-4.1은 6.11%→8.63% = ~1.4배**(대형 모델에선 소폭). oracle 누수(주석을 *수정된* 코드에서 생성) — 저자가 "실무 적용 불가" 명시.
  - **견고한 정성 결론:** 주석 5범주 중 **"How(구현 의도/메커니즘)"가 최강, "What"이 최약**(두 모델 공통). (확실)
- **주장2 (사용자 가설) — 판정: 조건부지지. naive형 반증, 다듬은형 지지.**
  - (a) 업계 합의 = "더 많이/더 자세히"가 **아니라** "책임을 좁혀라"(why/intent/invariant로). LLM 생성 주석의 redundant·hallucination 역효과 실증(arxiv 2605.13280, 2406.14836) → "전반적 상세화"는 자기파괴. → **naive 가설 반증.**
  - (b) "한눈에 이해"를 돕는 건 인라인 verbose가 아니라 **함수/파일/모듈 overview·NL outline**(별도 계층) + **자동 갱신(living doc)**으로 rot 차단(arxiv 2408.04820).
  - (c) 이해부담 증가는 견고(verification debt 다출처 수렴). 단 METR(arxiv 2507.09089, 체감 +20% / 실측 -19%)는 n=16·2025 초기 모델 한계 — "18개월 벽"의 직접 증거까진 아님(가능성 높음). **결정적:** Osmani — "주석·스펙만으로 comprehension debt를 못 푼다 … 결국 누군가는 검토해야 한다." → 가설의 암묵 전제("주석만 자세하면 안 읽어도 됨")는 **반증**.
- **다듬은 명제(증거가 지지하는 형태):** *"에이전트 개발에서는 인라인 주석을 why/invariant/load-bearing으로 **좁혀** 신뢰도를 높이고, '한눈 이해'는 자동 갱신되는 overview/outline **별도 계층**으로 분리하되, 이것이 인간 리뷰를 **대체하지 않는다**."* — 우연히 engram CLAUDE.md 기존 규약("자명한 건 금지, load-bearing만 깊게")과 일치.

---

## 3. Stage 2 — 종합

### 3.1 관행 분류표 — "책임을 좁히는" 4층위 모델

| 층위 | 형식 | 담을 것 | 피할 것 |
|---|---|---|---|
| 코드 내부 local | `//`, `/* */` | 비직관 이유·invariant·sentinel·동시성/lifetime·workaround 근거 | 코드 한 줄 직역·죽은 코드·오래된 TODO |
| API doc-comment | rustdoc `///`/`//!`, JSDoc, docstring | 사용법·contract·params/return 의미·errors/panics/safety·examples·deprecation | signature와 중복되는 타입 설명·template filler |
| 설계 결정 | ADR (+ 코드 `// ADR-NNNN` 앵커) | context·decision·**거부 대안**·consequences·status·supersede | 회의록·사소한 구현 선택 |
| 사용자/에이전트 문서 | Diátaxis 4유형 · AGENTS.md/CLAUDE.md | 학습/작업/조회/이해 분리 · 에이전트 불변식·금지·검증절차 | 한 문서에 입문+절차+API+철학 혼재 |

**+ 에이전트 시대 신규 층위(제안):** 파일/모듈 **overview(한눈 이해)** — 역할·책임·핵심 불변식·진입점을 헤더에 요약(현 engram `//!` 헤더의 강화판), 가능하면 코드 변경과 함께 갱신.

### 3.2 에이전트 시대 변화 흐름 (한 줄 요약)

주석·문서가 "코드 보조 설명"에서 **"intent infrastructure"**(사람=AI 코드 grok·리뷰 기준선 / AI=repo 규칙·의도·금지·검증 주입)로 진화. 단 문서는 soft context라 hard guardrail(테스트·CI·hooks) 동반 필요. (확실)

### 3.3 사용자 가설 판정

**조건부지지.** 방향(사람이 덜 읽으니 주석에 더 투자)은 맞다. 형태는 "전반적으로 더 자세히"가 아니라 **(1) 인라인은 why/intent/invariant/load-bearing으로 좁히고 (2) '한눈 이해'는 overview/outline 별도 계층이 담당 (3) 자동 갱신으로 rot 방지 (4) 주석이 인간 리뷰를 대체한다고 가정하지 않음**.

### 3.4 engram 적용 권고 — 선택지 (사용자 결정)

engram 현 컨벤션은 이미 증거-지지 형태와 ~85% 일치(우연이 아니라 좋은 기조). 갭은 ① 파일 overview "한눈 이해" 품질 편차 ② ADR 앵커 부분적 ③ session_tracker.rs류 내부 분기 주석 부족.

- **선택지 A (최소):** 컨벤션 문장은 그대로 두고, 진단상 약한 파일에 점진 보강만(ADR 앵커·내부 why 주석). ADR/CLAUDE.md 변경 없음.
  - 트레이드오프: 비용 최소·rot 위험 최소 / 표준이 암묵적이라 다음 세션이 "overview 품질"을 빠뜨릴 수 있음.
- **선택지 B (중간, 권고):** CLAUDE.md "컨벤션" 절에 **2계층 주석 규약**을 명문화 — (1) 인라인 = why/intent/invariant/load-bearing(기존) (2) **신규: load-bearing 파일은 "한눈 이해" overview 헤더 의무**(역할·책임·불변식·진입점) + `// ADR-NNNN` 앵커 점진 확대. 굵은 결정이면 ADR-00xx.
  - 트레이드오프: 사용자 "한눈 이해" 니즈를 증거-지지 형태로 직접 충족 / 헤더도 rot 가능 → "코드 변경과 함께 갱신" 규율 필요.
- **선택지 C (강):** B + NL-outline식 구조화 overview·자동 갱신(living doc) 툴링 지향(예: 변경 감지 시 헤더 갱신 알림). 
  - 트레이드오프: rot를 구조적으로 차단·에이전트 친화 최강 / 툴링 비용 큼, 현 단계 over-engineering 위험(저위험-장기 아닌 고비용-불확실 영역).

권고: **B**. 사용자 "한눈에 이해"를 증거가 지지하는 정확한 형태(overview 계층 분리 + 인라인 좁히기)로 충족하면서, C의 툴링 비용·rot 위험은 피함. 채택이 굵은 결정이면 ADR로 박제.

---

## 4. 적용 예시 (제안 — 코드 미적용)

> 리서치 단계라 코드엔 적용하지 않았다. 아래는 선택지 B의 "한눈 이해 overview 헤더"가 실제로 어떤 모습인지 보여주는 **예시**다(채택 시 적용).

**중요 발견:** 코어 22개 파일 중 **19개가 이미 `//!` 헤더 보유** → 이 코드베이스는 이미 선택지 B를 ~충실히 따른다. 사전 진단이 "약하다"고 본 `session_tracker.rs`도 실제론 모범(헤더·분기 why·PID shim/degraded/lock 순서 박힘)이라 손댈 필요 없다. 갭은 헤더 없는 3개 파일(`logging/mod.rs`·`types.rs`·`agent/mod.rs`) + ADR 앵커 부분성뿐.

**예시 대상:** `crates/engram-dashboard-core/src/logging/mod.rs` — load-bearing(보안 `mask_secrets`, tracking T-1/D-6)인데 1번 줄이 `use ...`로 시작, 처음 보는 사람에게 오리엔테이션 0. item별 `///`는 좋으나 모듈 overview(`//!`)만 부재.

**Before:** 첫 줄 = `use std::sync::OnceLock;`. "이 파일이 무슨 책임을 지는지", 특히 **"마스킹이 자동이 아니라 호출자 책임"** 이라는 비자명 불변식이 한눈에 안 보임 → 다음 세션이 production 로그에 PTY 텍스트를 추가하며 `mask_secrets`를 빠뜨릴 위험(= CLAUDE.md `ENGRAM_DATA_DIR` 오삭제 사고와 같은 부류).

**After (이런 `//!` 헤더를 달면):**

```rust
//! logging — tracing-subscriber 전역 설정 + 로그 비밀값 마스킹.
//!
//! ## 두 책임
//! - 로그 초기화·레벨 제어: init_logging(부팅 1회, 멱등) → RELOAD_HANDLE → set_log_level.
//! - 비밀값 마스킹: mask_secrets가 API 키·Bearer 토큰을 ***로 치환(T-1).
//!
//! ## load-bearing (시그니처만 봐선 안 보이는 의미)
//! - 마스킹은 *호출자* 책임 — 자동 적용이 아니다. 이 모듈은 mask_secrets를 제공만 한다.
//!   (tracking T-1/D-6 — production PTY 로그 추가 시 필수)
//! - 마스킹은 best-effort. AWS Secret Key·generic api_key= 는 못 잡는다. "통과 = 비밀 0" 아님.
//! - 전역 상태(OnceLock)라 init·regex 컴파일은 정확히 1회. 중복 init/set은 no-op.
```

**개선점:** 처음 보는 사람이 1번 줄에서 ① 두 책임(로그 제어 / 비밀 마스킹) ② load-bearing 불변식(마스킹=호출자 책임·best-effort·전역 1회)을 즉시 파악. redundant 주석(noise)이 아니라 시그니처로 안 보이는 의미를 박는 것 — why-not-what·load-bearing 정합. 이게 선택지 B의 구체적 모습이다. (실제 적용·QA는 채택 결정 후.)

---

## 5. 공백 (이번 조사로 못 채운 것)

- **engram 코드에 대한 NL-outline 자동 갱신 가능성** — living doc 툴링이 Rust/Tauri 환경에서 현실적인지 미조사(선택지 C 채택 시 별도 prior-art 필요).
- **"한눈 이해" overview의 최적 분량·형식** — 50줄 핀포인트 원칙은 AGENTS.md용이고, 코드 파일 헤더의 적정 길이는 실측 미확인.
- **국내(한국어권) 관행** — 조사 대상이 영어권 OSS·문헌 위주. 한국어 주석 자체의 효과 연구는 미확인.

## 6. 한계 (전반)

- 회의론 핵심 실증(eye-tracking)은 단일 venue·소표본. 옹호 핵심 실증(ABF 3배)은 oracle 누수로 실무 외삽 제한 — 둘 다 §2에서 좁혀 인용.
- 에이전트 시대 트렌드(2024~2026)는 빠르게 변동 — spec-vs-code 진실원천, llms.txt 실효, AGENTS.md 최적 granularity 모두 미합의(불확실).
- 두 family 모두 web 조사 기반 — 1차 논문 전문 대조는 §2의 4개 주장에 한정.

## 7. 주요 출처

- Atwood "Code Tells You How…": https://blog.codinghorror.com/code-tells-you-how-comments-tell-you-why/
- Fowler "CodeAsDocumentation": https://martinfowler.com/bliki/CodeAsDocumentation.html
- Ousterhout *A Philosophy of Software Design*: https://web.stanford.edu/~ouster/cgi-bin/book.php
- Knuth Literate Programming: https://www-cs-faculty.stanford.edu/~knuth/lp.html
- ADR (Nygard / adr.github.io): https://adr.github.io/ , https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions
- Diátaxis: https://diataxis.fr/
- rustdoc: https://doc.rust-lang.org/rustdoc/how-to-write-documentation.html
- NL Outlines (FSE 2025): https://arxiv.org/abs/2408.04820
- AGENTS.md: https://agents.md/ · Codex AGENTS.md: https://developers.openai.com/codex/guides/agents-md
- llms.txt: https://llmstxt.org/
- Spec-driven: https://kiro.dev/docs/specs/ , https://arxiv.org/abs/2602.00180
- eye-tracking (검증): https://www.se.cs.uni-saarland.de/publications/docs/APB+25.pdf (DOI 10.1007/s10664-025-10721-2)
- comment rot: Wen 2019 https://dl.acm.org/doi/10.1109/ICPC.2019.00019 · CoCC https://arxiv.org/abs/2403.00251 · link decay https://arxiv.org/abs/1901.07440
- ABF 주석 효과 (검증): https://arxiv.org/abs/2601.23059
- comprehension debt: https://addyosmani.com/blog/comprehension-debt/ · METR https://arxiv.org/abs/2507.09089 · Sonar verification gap
- LLM 주석 역효과: https://arxiv.org/html/2605.13280v1 , https://arxiv.org/abs/2406.14836
