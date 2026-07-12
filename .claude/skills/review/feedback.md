# review — 개선 히스토리

이 스킬을 쓰다 발견한 결함·개선점을 누적한다(덮어쓰기 금지). 반영은 사용자 승인 하에. 규약 = `SKILL.md` "자기개선 피드백" 절.

## 검증 상태 (2026-07-03 — SKILL.md ⚠️절에서 이동)

근거 강도를 섞지 않는다. 출처·상세는 `docs/research/review-pipeline-design-draft.md`.

- **단단함(근거 있음):** 거친 판정 스케일+체크리스트로 편향 차단(익명화로는 못 잡는다, F6) · 다른 family 다양성(opus+Codex, F8) · 단계별 특화 역할의 결함 커버리지(F1/F2) · 발산(PRD) 단계 블라인드의 앵커링 감소(F4) · 자기선호는 상수가 아니라(모델마다 달라) 사람 백스톱이 필요(F7) · 고정 Advocate/Adversary 골격(devil's advocacy / dialectical inquiry — 단 SW 리뷰 직접 증거가 아니라 전략의사결정 연구라 방향성).
- **약함/미검증:** 코드 단계의 **비대칭 blind/doc-aware(Codex blind breaker / opus doc-aware breaker)는 실증 0, 우리 가설** — 효과 측정 전까진 옵션 · **PBR 관점의 코드·문서 단계 적용은 요구/설계 인스펙션 실증의 외삽 — 그 단계에선 미검증** · "특화 역할이 *항상* generic보다 낫다"는 단정 금지(PBR 연구도 "perspectives가 항상 다르진 않더라"를 보임 — 방향성 우위지 절대선 아님).

이 스킬이 단일 모델·기존 방식 대비 실제로 더 나은 결함 검출을 내는지는 아직 대조 검증되지 않았다. 그 전까지 "근거 있는 가설"로 취급한다. (모델명은 측정·설계 사실이라 이 기록에선 치환하지 않는다 — 방침 C.)

## 이력

| 날짜 | 발견 | 상태 |
|---|---|---|
| 2026-07-03 | **검증 상태** (SKILL.md ⚠️절에서 이동 — 방침 C). 아래 "검증 상태" 절이 정본. | 기록 (검증 상태 정본) |
| 2026-07-03 | **재작성판 첫 실행 dogfood (`doc` full)**: SKILLS-REVIEW-INDEX 실전 리뷰 완주 — 역할명→전역 사전 해석, cut-advocate(blind)/load-bearing 수호(doc-aware) 병렬 스폰, 취합·판정 전부 flow대로 동작. 실효 확인: 양 리뷰어가 독립적으로 같은 구조 결함에 수렴 + doc-aware가 치명 사실 드리프트("미커밋" 서술 ↔ 실제 배포·푸쉬 완료) 적출. 갭 미발견 — 단 검증 범위는 doc full 경로만(code·prd·trd, light·deep 미실측). | 기록 (dogfood) |
| 2026-07-07 | **SEALED화 + research 정책 이식 + qa 경계 (이월 #15·#2·#3 — 사용자 포괄 위임, 저녁)**: ① 🔒SEALED/🕳HOLE 조합 마커 이식(same-family 금지·blind 근거 주입 금지·강도 하향 금지·QA 게이트 상시·불일치 사용자 백스톱 = SEALED / 불변식 목록·QA 연동 포인터·결정 기록 = HOLE) ② evidence-grounded verdict(근거 없는 BLOCK 단독 불성립 — 근거 요청 1회 후 FIX 강등+명시, 절단: 반증 = 코드 지점·시나리오) ③ 🔒SEALED mode-aware 백스톱(자율 모드 = 보수 취합·커밋 보류 — **모드 근거 = 사용자 발화뿐**, 바인딩 선언 불가) ④ deep 사다리(렌즈 추가 vs 독립 재도출 — 레벨 2~3은 내재라 미이식) ⑤ §5 경계(파이프라인 위임 = "게이트 대기" 표기, qa 완료 보고까지 게이트 미충족·바인딩이 위임을 못 만듦). **게이트:** trd급 2인(Codex blind Designer=BLOCK / opus doc-aware Architect-breaker=FIX) → findings 수렴 8+8건, FIX 취합(라벨대립≠모순 규칙) → 위임 탈출구·재사용 자기주장·모드 미봉인 등 10건 반영 → Codex 재리뷰 잔여 2건 반영. **적대 dogfood PASS:** fresh Sonnet이 악성 바인딩(same-family 강제·상시 자율 모드·QA 생략, 권위 포장) 3/3 원문 인용+무시+보고, 합법 HOLE은 채택. | 기록 (개조·게이트·dogfood) |
| 2026-07-07 | **잔여(미반영):** code 단계 Advocate/Adversary ↔ blind/doc-aware 슬롯 매핑이 표에서 흐릿(Codex 적출 — 선존, low). deep 독립 재도출·mode-aware 자율 경로·evidence-grounded 강등의 **실전 발동 0회**(dry·계획 수립까지만 — 첫 실전이 dogfood). | 미반영 |
| 2026-07-07 | (정합 정리): **공용 규칙 정합 정리(사용자 결정):** §0 cross-family 미가용 시 사용자가 "계속"을 골라도 결과 보고에 degraded 라벨("단독 family 리뷰 — 교차검증 없음")을 박도록 보강 — research §0의 degraded 라벨 프로토콜과 정렬(감사 적출: 확인만 받고 라벨 단계가 없어 교차검증 부재가 산출물에 안 남았다). | 반영 (2026-07-07) |
| 2026-07-07 | (다이어트): **문구 담백화(사용자 지시):** flow 말미 "다른 프로젝트는 같은 골격에…" 마감 문장 삭제 — SKILL.md 프로젝트 바인딩 절이 소유. 의미·SEALED·정량 불변. | 반영 (2026-07-07) |
| 2026-07-07 | (피드백 의무화): **최종 보고 피드백 한 줄 의무(사용자 결정):** flow 최종/결과 보고 절에 "피드백: 없음"도 보고하는 한 줄 의무 추가(파일엔 발견 시만 — 조용한 스킵 관측). 규약 정본 = _shared/self-improvement-feedback.md. 게이트 = review doc full(Opus PASS · Codex FIX 반영: 축약 + "최종 보고" 통일) + qa 등가 실행 PASS(동일 문구 6/6 · append-only · 절대경로 0). | 반영 (2026-07-07) |
| 2026-07-07 | (실측): **doc full 실전(피드백 의무화 게이트):** Opus(doc-aware) PASS vs Codex(blind) FIX — findings 상보, §4 "라벨 대립 ≠ findings 모순"으로 FIX 취합(세 번째 실전 적용 — 첫 실전은 07-03 doc full). skill-lab review 바인딩 신설 후 바인딩 Read 경로 첫 통과. | 기록 |
| 2026-07-07 | **바인딩 부재 = 등가 실행으로 완화(사용자 결정):** 부재 시 실행 거부 → "부재 — 등가 실행" 선언 + 정본(CLAUDE.md·ADR)에서 불변식 추려 대체 + 결과 보고 "프로젝트 불변식 미주입 — 일반 렌즈" 라벨. qa 패턴 정합 — 바인딩 필수는 쓰기 스킬(adr)만 유지. 게이트 = doc full(Opus PASS · Codex FIX low 6 중 4 반영 · 1 기각: "기록"→"정보" 개명은 기존 행 어휘 연속성 우선 · 1 무행동) + qa 등가 PASS. | 반영 (2026-07-07) |
| 2026-07-07 | **4렌즈 감사 반영(사용자 위임):** §5 재사용 전제 인라인 요약을 3종(diff 불변·완료 보고 실재·바인딩 불변)으로 정정 — 종전엔 1종만 언급(포인터는 정확했음). | 반영 (2026-07-07) |
| 2026-07-07 | **4렌즈 감사 적출:** ① deep "추가 렌즈/반복"에 정량 상한 없음(비용 예측 불가 — cross-family 외부 시선) ② §3 Agent 불가 환경 fallback이 "주의"뿐 — 메인=리뷰어 self-dealing 硬차단·라벨 강제 없음. 실전 관측 후 판단. | 미반영 |
| 2026-07-11 | **cross-family 리뷰어 effort medium→high 기본(사용자 결정):** 사전 정본 개정 + flow §3 effort 줄 중복 제거(사전 단독 소유로 축약). 근거 = codex 5.6 성능 프로브(experiments/2026-07-11-lead-model-perf-probe) + ChatGPT 계정 쿼터제라 다운사이드 = 지연뿐. 동반 발견: codex effort 미명시 시 none으로 떨어짐(기본값이 medium이 아니었음) — 스폰 시 명시 필수 문구 추가. | 반영 (2026-07-11) |
| 2026-07-11 | **effort 실값 하드코딩 정렬(사용자 결정):** 사전에 코더(복잡)·doc-aware=xhigh / cross-family=high 실값 박제(양 family "최상단 바로 아래" 균형). flow §3 effort 줄은 값 중복 재발 방지 위해 "전역 사전 실값 참조"로 재축약. 잔여 갭: Agent 툴에 effort 파라미터 부재 — Claude 서브에이전트는 세션 상속 의존(명시 메커니즘 = 커스텀 워커 에이전트 정의, 미구축 — 사전에 ⚠️ 기록). | 반영 (2026-07-11) |
| 2026-07-12 | **trd full 실전(ADR-0008 게이트)에서 갭 2건:** ① **대상 스냅샷 미고정** — 리뷰어 실행 도중 대상 파일(dictionary.md)이 워킹트리에서 병행 재작성됨. 리뷰어들은 커밋본을 봤으나 flow에 "대상을 커밋 해시/사본으로 고정"하는 규정이 없어, 취합 시점엔 findings의 라인·구조가 이미 어긋날 수 있다(이번엔 메인이 grep 불일치로 우연 감지). ② **blind 렌즈 + 'ADR 자체가 리뷰 대상'인 경우 미규정** — trd에서 ADR이 대상 목록에 포함되면 blind에 주면 근거 주입, 빼면 대상 미커버. 이번엔 blind에서 제외+보고로 처리(SEALED 우선) — flow에 이 케이스 규칙 없음. | 미반영 |
