# Review — 실행 절차

`$ARGUMENTS` = 단계 `prd`|`trd`|`code`|`doc` [+ 강도 `self`|`light`|`full`|`deep`] — 둘 다 옵션. 어떤 단계·강도로 도는지 호출 시 사용자에게 한 줄 명시한다.

역할→모델·effort = 전역 사전(`I:\Engram\core\claude-global-shared\references\dictionary.md`) 참조. 본문·절차는 역할명으로만 말한다. 두 리뷰어 슬롯 = **doc-aware 리뷰어**(맥락·ADR 주입) · **cross-family(blind) 리뷰어**(결정 근거 없이 신선 판단). 두 슬롯은 다른 family다.

## 0. cross-family(blind) 리뷰어 가용성 확인 (스폰 전 체크)

`mcp__codex__codex` MCP가 이 세션에 연결됐는지 확인한다. 리뷰어 구성에 cross-family(blind) 슬롯이 포함될 때 필요하다 — full·deep은 항상, light는 §2 표에서 그 단계 Adversary가 blind인 경우(prd)만.

**미연결이면 즉시 사용자에게 알리고 진행 여부를 확인한다:**
> "cross-family(blind) 리뷰어가 연결되지 않았습니다 — 단독 family 리뷰만 가능합니다(교차검증 불성립). 계속 진행할까요, 연결 후 재실행할까요?"

사용자 확인 없이 **조용히 단독 family로 대체하지 않는다** — 교차검증이 없었는데 리뷰가 완료된 것처럼 보인다.

---

## 강도표 (강도별 인원·깊이 정본)

강도는 **단계와 무관한 공통 인원·깊이 스케일**이다. 단계가 *어느 렌즈*를 쓸지 정하고, 강도가 *몇 명·몇 회* 돌릴지 정한다. **인원·깊이 정량은 이 표가 유일 정본** — 다른 절은 강도 이름으로 참조만 한다(rot 방지).

| 강도 | 리뷰어 | 깊이 | 언제 |
|---|---|---|---|
| **self** | 0인 (코더 self 체크리스트만 — 역할 렌즈 X) | QA build/test만 | 1~2줄·문서 오타·자명 |
| **light** | 1인 — 그 단계 **Adversary 렌즈만** (§2 표의 blind/doc-aware 규칙대로) | 단일 패스 | 국소·저위험·단일 관심사 (위험 영역 사전 배제) |
| **full**(기본) | 2인 — 단계 역할표 **Advocate + Adversary** | 단일 패스 병렬 | 비자명 변경 (프로젝트 기본 게이트) |
| **deep** | 2인 + 추가 렌즈/반복(다관점·다회) | 같은 단계를 여러 렌즈 또는 반복 | 고위험 — 동시성·kill·lifetime·보안·공개 API·마이그레이션·핫패스 |

- **self는 리뷰어 0인.** 단계 인자는 *렌즈*가 아니라 self 체크리스트·QA 범위를 고른다(code=diff+테스트 / doc=링크·중복 / trd=ADR·불변식 / prd=요구 누락). **self도 QA(build/test, 해당하면 GUI 실측)는 돈다 — 리뷰어 스폰만 생략이다.**
- **light = Adversary 렌즈 1인.** Advocate(옹호) 아닌 Adversary(공격)를 남기는 이유 = 저위험 변경에선 "안전 수호" 렌즈가 우선이다. 단일 렌즈라 **그 렌즈가 놓친 위험은 승격 트리거가 안 당겨진다**(2인 교차 백스톱 없음) — 위험 영역이 의심되면 full로 시작한다.
- **deep = full 2인 + 추가.** 같은 단계를 추가 렌즈(보안/성능/마이그레이션 등)로 또는 반복해 한 번 더 때린다.
- **escalation-only:** 시작 강도가 하한이다. 도중 위험 트리거(동시성·kill·lifetime·보안·공개 API·마이그레이션·핫패스)를 발견하면 상위로 자동 승격 + 사용자 알림. 임의 하향 금지. 발견은 코더 self든 리뷰어 단계든 어디서든 트리거된다.
- 강도 선택 트리거 = 변경 LOC·위험 영역·신규 vs 리팩터·자동생성/trivial. 무거울수록 위. 애매하면 full.

## 0-1. 인자 파싱·기본값

`/review [prd|trd|code|doc] [self|light|full|deep]` — 둘 다 옵션. 인자 토큰으로 추정한다:
- **강도 토큰만** → 단계는 대상으로 추정(코드 diff=code / 설계 문서=trd / 요구·발산=prd / 일반 정리=doc).
- **단계 토큰만** → 강도는 변경 무게로 추정하되 비자명 기본 = full.
- **둘 다 없음** → 단계·강도 둘 다 추정.
- **모호하면 사용자에게 한 줄 확인.** 특히 대상이 prd↔trd(요구↔설계)에 걸치면 임의 추정하지 않고 묻는다.

## 1. 대상·단계 확정 (Lead = 메인 오케스트레이터)

- 리뷰 대상(diff·문서·spec)을 확정하고 단계 렌즈를 고른다(§2). 애매하면 사용자에게 한 줄 확인.
- **복합 변경**(코드+설계+요구가 섞임)은 §2 fallback 역할로 돌리거나 사용자에게 단계별 분리 여부를 확인한다.
- 강도를 정한다(강도표). 비자명=full.
- 단계가 doc-aware 렌즈를 포함하면(trd·code·doc) **관련 ADR·불변식 묶음을 미리 추려** doc-aware 리뷰어에게 줄 준비를 한다(blind 렌즈엔 주지 않는다).

## 2. 단계별 특화 역할표 (매번 발명 X — 여기서 꺼내 쓴다)

| 단계 | Advocate (옹호·강화) | Adversary (공격·대척) | 블라인드 | 체크리스트 |
|---|---|---|---|---|
| **prd** (요구·발산) | **User 렌즈** (blind) — use-case로 진짜 needs·완결성 옹호 | **Tester 렌즈** (blind) — equivalence/boundary·실패 시나리오·놓친 대안 공격 | **둘 다 ON** (결정 근거 숨김 → 앵커링 차단) | PBR: User(use-case) + Tester(equivalence-class) |
| **trd** (설계) | **Designer 렌즈** (blind) — 인터페이스·구조·교체성·더 단순안 | **Architect-breaker** (doc-aware) — 불변식 위반·결합·기존 ADR 깨기·lifetime 공격 | 비대칭 (doc-aware=ADR 주입 / blind=근거 없이) | PBR: Designer + seam·capability·교체성 |
| **code** (게이트) | **correctness·단순성** (약한 보조 — 아래 주석) | **adversarial breaker** — doc-aware=불변식 위반 / blind=race·lifetime·off-by-one·회귀·보안·계약 | 비대칭 (doc-aware=불변식 / blind=코드+계약만) | ODC 결함타입 + 프로젝트 코드 불변식(바인딩) |
| **doc** (문서 정리) | **cut-advocate** (blind) — 중복·죽은 참조·군더더기 더 쳐내라 | **load-bearing 수호** (doc-aware) — 삭제가 불변식·"왜"·안티패턴 경고·교차참조를 떨구나 | 비대칭 (blind=근거 숨김 / doc-aware=코드·ADR 접근) | 삭제-안전 체크(load-bearing 의미·교차참조 보존) |
| **(fallback) 추정 실패/복합** | 목표 달성했나·더 나은/간결한 버전·빠진 것 | 뭐가 깨지나·안 적힌 가정·worst input/race·뭘 조용히 위반하나 | 비대칭 (Adversary=doc-aware / Advocate=blind) | (전용 체크리스트 없음 — 역할 일반 질문) |

**슬롯 → 역할 배정 (고정):** **blind 슬롯 = cross-family(blind) 리뷰어 · doc-aware 슬롯 = doc-aware 리뷰어.** 근거 = ADR/맥락이 필요한 역할엔 doc-aware, 신선한 판단이 이득인 역할엔 blind. **단 prd 예외 — 둘 다 blind**라, 평소 doc-aware를 맡는 리뷰어도 prd에선 결정 근거 없이 blind 모드로 돈다(발산 단계 앵커링 차단).

- **code 단계 주석:** 코드 리뷰는 본질이 적대라 **두 역할이 모두 공격 성향**이다(doc-aware=불변식 breaker / blind=엣지·계약 breaker). "Advocate(맞다·단순하다)"는 약한 보조 체크로 접는다. 이 **비대칭 blind는 실증 없는 가설** — 효과 측정 전까진 옵션(검증 상태는 feedback.md).
- **light 강도:** 위 표에서 **Adversary 렌즈 1인만** 돌린다 — 그 단계 행의 blind/doc-aware 규칙 그대로(prd면 blind, trd/code/doc면 doc-aware). Advocate 생략.
- **deep 강도:** 위 2인 + 같은 단계를 추가 렌즈로 또는 반복 검증(강도표).

## 3. 리뷰어 스폰 (병렬 · 강도에 맞게)

- full 이상: Advocate·Adversary를 **한 메시지에 동시 스폰**(병렬). cross-family(blind) 리뷰어 = `mcp__codex__codex`, doc-aware 리뷰어 = Agent 서브에이전트. (Agent 서브에이전트 불가 환경에서만 fallback으로 메인이 doc-aware 별도 패스 — 단 메인이 생산에 관여했으면 fresh 위반이니 주의.)
- **blind 렌즈에는 결정 근거·관련 ADR을 주지 않는다** — 신선한 판단이 그 렌즈의 가치다(주면 앵커링으로 신선도가 죽는다). doc-aware 렌즈에는 §1에서 추린 ADR·불변식을 함께 준다.
- 각 리뷰어는 **판정 + 항목별 근거만 구조화 반환**한다(verbose 덤프 회수 금지). 메인은 결론만 회수해 취합한다.
- **effort** = 전역 사전 참조(리뷰어 high). 메인 세션 = xhigh(무가드 통합 노드). cross-family(blind) 리뷰어는 medium 기본 — 동시성·lifetime 치명 변경만 high.

각 스폰 프롬프트 최소 골격(빠짐없이 채운다):
- **역할** — §2 표의 그 단계 렌즈(예: "trd Architect-breaker — 불변식·결합·ADR 위반 공격").
- **대상** — 리뷰할 diff·문서·spec 범위.
- **제공 컨텍스트** — doc-aware면 §1에서 추린 ADR·불변식 묶음.
- **금지 컨텍스트** — blind 렌즈면 "결정 근거·관련 ADR을 주지 않는다"를 명시.
- **출력 형식** — `PASS / FIX / BLOCK` 판정 + `findings[]`(항목마다 근거 한 줄). verbose 덤프 금지.

## 4. 취합 + 판정 (메인 · 순서·라벨 무관)

- 두(또는 그 이상) 결과를 **라벨·순서 무관하게 합친다** — 누가 먼저인지, A/B 어느 쪽인지로 가중하지 않는다(순서 편향 차단).
- 판정 스케일은 거친 3단(점수화·미세 등급 금지): **`PASS` / `FIX`(독립 수정 항목 1~5) / `BLOCK`**(핵심 불변식 위반·방향 재검토). FIX 항목이 5를 넘으면 변경이 너무 커 BLOCK 쪽으로 분리·재검토를 검토한다.
- **불일치 처리:** 정면 대립(cut vs keep, 채택 vs 위험, FIX vs BLOCK)은 메인이 종합하되 **임의 확정 금지 → 사용자에게 쟁점을 보고**해 판정을 받는다. 메인은 자기 family 편향이 있어 사람이 백스톱이다.
- light는 단일 렌즈라 대립 구조가 없다 — 그 렌즈 판정을 그대로 보고하되, BLOCK·위험 트리거가 나오면 full 이상 승격을 제안한다.

## 5. QA 실측 게이트 (리뷰와 별개 · 항상)

- 리뷰 판정과 무관하게 build/test를 돌린다. **build/test·GUI 실측 실명령은 `/qa` 스킬 바인딩이 정본** — 여기·review 바인딩에 재수록하지 않는다.
- 화면·동작이 걸린 변경은 GUI/실제 동작 실측까지 한다. **코드 테스트·타입체크가 통과해도 실제 화면 확인 전엔 미완**으로 본다.
- **self 강도에서도 이 게이트는 생략하지 않는다.**

## 6. 결과 보고 + 후속 (결정권 = 사용자)

- 메인이 단계·강도·판정(PASS/FIX/BLOCK)·미해결 쟁점을 사용자에게 보고한다. 불일치는 선택지로 제시(임의 채택 금지).
- 커밋은 게이트(리뷰 PASS/FIX 반영 + QA) 통과 후에만. 결정·흐름 기록은 프로젝트 관례(바인딩 §결정 기록)에 위임한다 — 이 스킬·리뷰어가 직접 쓰지 않는다.

## 프로젝트 바인딩 (스킬 밖)

이 골격은 **스택을 모르는 범용 리뷰 엔진**이다. 프로젝트 전용은 `bindings/<project>.md`가 채운다(현재 engram = `bindings/engram.md`). 바인딩이 정의하는 것:
- **code 단계 코드 불변식** — Adversary(doc-aware) breaker가 공격 표면으로 삼는 프로젝트 불변식 목록. doc-aware 렌즈에만 주고 blind엔 주지 않는다.
- **QA 실측 명령** — build/test·GUI 실측 명령·플랫폼 제약(§5). 실명령 정본은 `/qa` 바인딩.
- **결정 기록** — 굵은 설계 결정·흐름을 어디에 남기는지(§6). 스킬·리뷰어는 기록하지 않고 메인이 처리.

다른 프로젝트는 같은 골격에 바인딩 파일만 추가한다. 골격에 특정 스택·불변식을 하드코딩하지 않는다.

## 가드레일 (앞 절에 없는 금지만)

- **same-family 2인 금지** — Advocate/Adversary는 다른 family. 같은 family 둘로 대체하면 편향이 안 갈린다.
- **즉석 역할 발명 금지** — 알려진 단계는 §2 표에서 꺼내 쓴다. 표에 없는 새 artifact만 fallback generic.
- **blind 렌즈에 근거 주입 금지** — 결정 근거·ADR을 주면 앵커링으로 신선도가 죽는다.
- **강도 하향 금지** — escalation-only. 시작 강도가 하한, 위험 발견 시 승격만.
