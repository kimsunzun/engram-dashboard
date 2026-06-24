# Review — 실행 절차

`$ARGUMENTS` = 단계 `prd`|`trd`|`code`|`doc` [+ 강도 `self`|`light`|`full`|`deep`] — 둘 다 옵션이다. 파싱·추정은 §0-1. 어떤 단계·강도로 도는지 호출 시 사용자에게 한 줄 명시한다.

## 0. 강도(intensity) 정하기

강도는 **단계와 무관한 공통 인원·깊이 스케일**이다. 단계가 *어느 렌즈*를 쓸지 정하고, 강도가 *그 렌즈를 몇 명·몇 회* 돌릴지 정한다.

| 강도 | 리뷰어 | 깊이 | 언제 |
|---|---|---|---|
| **self** | 0인 (코더 self 체크리스트만 — 역할 렌즈 X) | QA build/test만 | 1~2줄·문서 오타·자명 |
| **light** | 1인 — 그 단계 **Adversary 렌즈만** (§2 표의 blind/doc-aware 규칙대로) | 단일 패스 | 국소·저위험·단일 관심사(위험 영역 사전 배제) |
| **full**(기본) | 2인 — 단계 역할표 **Advocate + Adversary** | 단일 패스 병렬 | 비자명 변경(CLAUDE.md 기본 게이트) |
| **deep** | 2인 + 다관점/다회 | 같은 단계를 여러 렌즈 또는 반복 | 고위험 — 동시성·kill·lifetime·보안·공개 API·마이그레이션·핫패스 |

- **self는 리뷰어 0인** — 역할 렌즈를 안 쓴다. self의 단계 인자는 *렌즈*가 아니라 **self 체크리스트·QA 범위**를 고른다(code=diff+테스트 / doc=링크·중복 / trd=ADR·불변식 / prd=요구 누락). 리뷰어 렌즈는 light부터 켜진다. self도 QA(build/test, 해당하면 GUI 실측)는 돈다 — 리뷰어 스폰만 생략이다.
- **light = §2 표의 그 단계 Adversary 1인** — 해당 행의 blind/doc-aware 규칙을 그대로 따른다(PRD면 opus blind, trd/code/doc면 opus doc-aware). Advocate(옹호) 아닌 Adversary(공격)를 남기는 이유 = 저위험 변경에선 "안전 수호" 렌즈가 우선이다.
- **light 경고:** light는 위험 영역이 *사전 배제*된 변경에만 쓴다 — 단일 렌즈라 **그 렌즈가 놓친 위험은 승격 트리거가 안 당겨진다**(2인 교차 백스톱이 없다). 위험 영역이 의심되면 full로 시작한다.
- escalation-only: 시작 강도가 **하한**이다. 도중 위험 트리거를 발견하면 상위로 자동 승격하고 사용자에게 알린다. 임의로 낮추지 않는다.
- 강도 선택 트리거: 변경 LOC·위험 영역(동시성·보안·공개 API·마이그레이션·핫패스·kill/lifetime)·신규 vs 리팩터·자동생성/trivial. 무거울수록 위. 애매하면 full.

## 0-1. 인자 파싱·기본값 (트리거 추정 규칙)

`/review [prd|trd|code|doc] [self|light|full|deep]` — 단계·강도 둘 다 옵션이다. 인자로 들어온 토큰을 보고 추정한다:
- **강도 토큰만** 오면 단계는 **대상으로 추정**(코드 diff=code / 설계 문서=trd / 요구·발산=prd / 일반 정리=doc).
- **단계 토큰만** 오면 강도는 **변경 무게로 추정**하되 비자명 기본 = full.
- **둘 다 없으면** 단계·강도 둘 다 추정한다.
- **모호하면 사용자에게 한 줄 확인.** 특히 대상이 prd↔trd(요구 ↔ 설계) 양쪽에 걸치면 기본 = **사용자 확인**(임의 추정하지 않고 묻는다).

## 1. 대상·단계 확정 (Lead = 메인 오케스트레이터)

- 리뷰 대상(diff·문서·spec)을 확정하고 단계 렌즈를 고른다(§2). 단계가 애매하면 사용자에게 한 줄 확인.
- **단일 단계로 안 떨어지는 복합 변경**(코드+설계+요구가 섞임)은 §2 fallback 역할로 돌리거나, 사용자에게 **단계별 분리 여부를 확인**한다.
- 강도를 정한다(§0). 변경 무게·위험 영역을 보고 추정하되 비자명=full.
- 단계가 doc-aware 렌즈를 포함하면(TRD·코드·문서) **관련 ADR·불변식 묶음을 미리 추려** opus 리뷰어에게 줄 준비를 한다(blind 렌즈에는 주지 않는다).

## 2. 단계별 특화 역할 픽스 표 (매번 발명하지 않는다 — 여기서 꺼내 쓴다)

근거·체크리스트 상세·왜 이 매핑인지는 `docs/research/review-pipeline-design-draft.md` §2가 정본이다. 이 표는 실행용 요약 — 상세 복붙으로 rot 만들지 않는다.

| 단계 | Advocate (옹호·강화) | Adversary (공격·대척) | 블라인드 | 체크리스트 출처 |
|---|---|---|---|---|
| **prd** (요구·발산) | **User 렌즈** (Codex) — use-case로 진짜 needs·완결성 옹호 | **Tester 렌즈** (opus) — equivalence/boundary·실패 시나리오·**놓친 대안** 공격 | **ON** (결정 근거 숨김 → 앵커링 차단) | PBR: User(use-case) + Tester(equivalence-class) |
| **trd** (설계) | **Designer 렌즈** (Codex) — 인터페이스·구조·교체성 건전성·더 단순안 | **Architect-breaker** (opus, doc-aware) — 불변식 위반·결합·기존 ADR 깨기·lifetime 공격 | **OFF** (opus=ADR 자동주입 / Codex엔 관련 ADR 묶음 명시 제공) | PBR: Designer + seam·capability·교체성 |
| **code** (게이트) | **correctness·단순성 옹호** — 목표 동작 충족·더 단순/명확한 구현 | **adversarial breaker** — race·lifetime·off-by-one·회귀·보안 공격 | **비대칭(실험)** — Codex=코드+계약만(blind 신선 breaker) / opus=doc-aware(불변식) | ODC 결함타입 + 우리 불변식(프로젝트 통합 절) |
| **doc** (문서 정리) | **cut-advocate** (Codex, blind) — 중복·죽은 참조·군더더기 더 쳐내라 | **load-bearing 수호** (opus, doc-aware) — 삭제가 불변식·"왜"·안티패턴 경고·교차참조를 떨구나 | Codex=근거 숨김(blind) / opus=코드·ADR 접근 | 삭제-안전 체크(load-bearing 의미·교차참조 보존) |
| **(fallback) 추정 실패/복합 대상** | 목표 달성했나·더 나은/간결한 버전·빠진 것 | 뭐가 깨지나·안 적힌 가정·worst input/race·뭘 조용히 위반하나 | Adversary=doc-aware / Advocate=blind 기본 | (전용 체크리스트 없음 — 역할 일반 질문) |

- **모델 매핑 원칙(고정 기본):** ADR/프로젝트 맥락이 필요한 역할 → **opus(doc-aware)**. 신선한 blind 판단이 이득인 역할 → **Codex(blind)**.
- **code 단계 주석:** 코드 리뷰는 본질이 적대라 두 역할이 모두 공격 성향이다(opus=불변식 doc-aware breaker / Codex=엣지·프로토콜 blind breaker). "Advocate(맞다·단순하다)"는 약한 보조 체크로 접는다. 이 **비대칭 blind는 실증 없는 우리 가설** — 효과 측정 전까진 옵션.
- **light 강도:** 위 표에서 **Adversary 렌즈 1인만** 돌린다 — 그 단계 행의 blind/doc-aware 규칙을 그대로 따른다(PRD면 opus blind, trd/code/doc면 opus doc-aware). Advocate 생략. light 한계는 §0 경고 참조.
- **deep 강도:** 위 2인에 더해 같은 단계를 추가 렌즈로 또는 반복해 검증한다(예: 코드 게이트를 보안 렌즈로 한 번 더).

## 3. 리뷰어 스폰 (병렬 · 강도에 맞게)

- full 이상: Advocate·Adversary를 **한 메시지에 동시 스폰**한다(병렬 실행). Codex=`mcp__codex__codex`, opus=Agent 서브에이전트. (Agent 서브에이전트가 불가한 환경에서만 fallback으로 메인 opus 별도 패스.)
- blind 렌즈(표의 블라인드=ON 또는 Codex blind)에는 **결정 근거·관련 ADR을 주지 않는다** — 신선한 판단이 그 렌즈의 가치다. doc-aware 렌즈(opus)에는 §1에서 추린 ADR·불변식을 함께 준다.
- 각 리뷰어는 **판정 + 항목별 근거만 구조화 반환**한다(verbose 덤프 회수 금지). 메인은 결론만 회수해 취합한다.
- **effort:** 리뷰어 = high(Codex는 medium 기본, 동시성·lifetime 치명 변경만 high). 메인 세션 = xhigh(무가드 통합 노드라 검수보다 effort를 싣는다).

각 리뷰어 스폰 프롬프트 최소 골격(빠짐없이 채운다):
- **역할** — §2 표의 그 단계 Advocate/Adversary 렌즈(예: "trd Architect-breaker — 불변식·결합·ADR 위반 공격").
- **대상** — 리뷰할 diff·문서·spec 범위.
- **제공 컨텍스트** — doc-aware면 §1에서 추린 ADR·불변식 묶음.
- **금지 컨텍스트** — blind 렌즈면 "결정 근거·관련 ADR을 주지 않는다"를 명시(앵커링 차단).
- **출력 형식** — `PASS / FIX / BLOCK` 판정 + `findings[]`(항목마다 근거 한 줄). verbose 덤프 금지.

## 4. 취합 + 판정 (메인, 순서·라벨 무관)

- 두(또는 그 이상) 결과를 **라벨·순서 무관하게 합친다** — 누가 먼저인지, A/B 어느 쪽인지로 가중하지 않는다(순서 편향 차단).
- 판정 스케일은 거친 3단(점수화·미세 등급 금지, F6): **`PASS` / `FIX`(독립 수정 항목 1~5) / `BLOCK`**(핵심 불변식 위반·방향 재검토). FIX 항목이 5를 넘으면 변경이 너무 커 BLOCK 쪽으로 분리·재검토를 검토한다.
- **불일치 처리:** Advocate·Adversary가 갈리면(특히 cut vs keep, 채택 vs 위험, FIX vs BLOCK) 메인이 종합하되 **임의 확정 금지 → 사용자에게 쟁점을 보고**해 판정을 받는다(F7). 메인=Claude 계열이라 자기편(opus) 편향이 있으므로 사람이 백스톱이다.
- light는 단일 렌즈라 대립 구조가 없다 — 그 렌즈 판정을 그대로 보고하되, BLOCK·위험 트리거가 나오면 full 이상으로 승격을 제안한다.

## 5. QA 실측 게이트 (리뷰와 별개 · 항상)

- 리뷰 판정과 무관하게 build/test를 돌린다(`cargo test` 워크스페이스 루트 등 프로젝트 명령).
- 화면·동작이 걸린 변경은 GUI 실측까지 한다(`scripts/cdp.mjs` eval/shot). **코드(test/tsc)가 통과해도 실제 화면 확인 전엔 미완**으로 본다.
- self 강도에서도 이 게이트는 생략하지 않는다.

## 6. 결과 보고 + 후속 (결정권 = 사용자)

- 메인이 단계·강도·판정(PASS/FIX/BLOCK)·미해결 쟁점을 사용자에게 보고한다. 불일치는 선택지로 제시(임의 채택 금지).
- 커밋은 게이트(리뷰 PASS/FIX 반영 + QA) 통과 후에만. ADR·step-log 기록은 프로젝트 관례에 위임한다(이 스킬이 직접 쓰지 않는다).

## 프로젝트 통합 (스킬 밖 — engram 바인딩)

이 스킬은 **범용 리뷰 엔진**이다. engram에 쓸 때의 바인딩만 여기 둔다(골격에 하드코딩 X):

- **code 단계 체크리스트의 우리 불변식** — Adversary(opus doc-aware breaker)는 다음 불변식 위반을 공격 표면으로 삼는다: kill 인과(2동사: shutdown → join_pump) · finalize 1회(swap AcqRel) · 락 순서(Arc clone 후 해제, status lock 보유 중 외부호출 금지) · epoch 재구독(맵 교체 +1) · replay→live(subscribers lock 보유 중 replay + seq dedup) · 코어 tauri import 0. 근거·상세는 CLAUDE.md "핵심 불변식"과 각 ADR. **이 목록은 코드·ADR이 바뀌면 rot한다 — 정본은 코드의 `// ADR-` 앵커, 여기는 리뷰 포인터일 뿐.**
- **QA 명령** — `cargo test -p engram-dashboard-core` / `-p engram-dashboard-protocol` / `cargo build`(루트) + `scripts/cdp.mjs`(WebView2 GUI).
- **결정 기록** — 굵은 설계 결정은 ADR(`docs/decisions/`), 흐름은 step-log. 스킬은 기록하지 않고 메인이 처리.

## 가드레일 (앞 절에 없는 금지만)

앞 절에 이미 박힌 규약(QA 생략 금지·점수화 금지·불일치 사용자 에스컬레이션·순서/라벨 가중 금지·리뷰 스킵 금지)은 여기서 반복하지 않는다. 이 절은 **다른 곳에 안 적힌 금지**만 모은다:

- **same-family 2인 금지** — Advocate/Adversary는 다른 family(opus + Codex). Claude 둘로 대체하면 편향이 안 갈린다(F8).
- **즉석 역할 발명 금지** — 알려진 단계는 §2 표에서 꺼내 쓴다. 표에 없는 새 artifact만 fallback generic.
- **blind 렌즈에 근거 주입 금지** — 발산(PRD) Advocate·Codex blind 렌즈에 결정 근거·ADR을 주면 앵커링으로 신선도가 죽는다(F4).
- **강도 하향 금지** — escalation-only. 시작 강도가 하한, 위험 발견 시 승격만(§0).
