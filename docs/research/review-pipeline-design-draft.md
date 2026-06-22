# Engram 검증 파이프라인 설계 (제안 → 일부 적용)

- 작성: 2026-06-22 · 상태: **CLAUDE.md 반영(2026-06-22)** + 본 문서가 상세 레퍼런스. ADR 박제는 미결(§6).
- 근거: `docs/research/review-methodology-research-2026-06-22.md` (각 항목 Fn 표기)
- 전제(사용자 결정 완료): 심판 모델 = **opus + Codex만**, sonnet = 하위 코더 전용, 웹 consult 폐기, 최종 판정자 = 사용자.

---

## 0. 원칙 (리서치에서 도출)

1. **편향은 익명화가 아니라 루브릭으로 잡는다.** 단계별 체크리스트 + 거친 스케일(PASS/FIX/BLOCK). 미세 점수 금지. (F6)
2. **2인 리뷰어의 힘은 "다른 family"에서.** opus + Codex. 같은 family 2개는 효용 약함. (F8)
3. **구조는 고정 변증법 쌍 = Advocate vs Adversary.** 매번 즉석 발명하지 않는 재사용 골격(devil's advocacy / dialectical inquiry). task-agnostic + 본질적 적대.
4. **성능은 단계별 "특화 역할 픽스"에서.** generic 한 쌍보다, 단계 유형마다 전용 기법을 든 역할을 *미리 한 번* 골라 박는 게 결함을 더 잡는다(PBR의 핵심 실증). "단계별 픽스 ≠ 매번 즉석 발명". (F1/F2)
5. **블라인드는 선택적.** 앵커링 위험 큰 단계(발산)만 ON. 맥락 상실로 합의↓·불변식 누락 비용 인지. (F4/F5)
6. **자기선호는 상수가 아니다 → 사람이 최종 백스톱.** 불일치는 메인이 임의 판정 말고 사용자에게. (F7)
7. **순서 편향 실재 → 취합은 순서·라벨 무관.** (리서치 기각표 역증)

---

## 1. 2층 구조 (피로 없이 성능 — 이게 핵심)

- **골격(고정·영구):** 모든 리뷰 쌍은 **Advocate(옹호·강화) vs Adversary(공격·대척)**. 적대 스탠스는 항상 보장.
- **특화(성능·단계별 1회 픽스):** 알려진 단계마다 *전용 역할+기법*을 미리 박는다(§2 표). 한 번 정하면 그 뒤론 꺼내 쓸 뿐 — 즉석 발명 0.
- **fallback:** 미리 안 박은 새 artifact만 generic Advocate/Adversary로.

**모델 매핑 원칙(고정 기본):** ADR/프로젝트 맥락이 필요한 역할 → **opus(doc-aware)**. 신선한 blind 판단이 이득인 역할 → **Codex(blind)**. (코드 게이트는 양쪽 다 적대 성향이라 예외 — 아래 표 주석.)

---

## 2. ★ 단계별 특화 역할 픽스 표 ★ (매번 안 만든다 — 여기서 꺼내 쓴다)

| 단계 | Advocate (옹호·강화) | Adversary (공격·대척) | 블라인드 | 체크리스트 출처 |
|---|---|---|---|---|
| **PRD / 요구·발산** | **User 렌즈** (Codex) — use-case로 진짜 needs·완결성 옹호 | **Tester 렌즈** (opus) — equivalence/boundary·실패 시나리오·**놓친 대안** 공격 | **ON** (결정 근거 숨김 → 앵커링 차단) | PBR: User(use-case) + Tester(equivalence-class) |
| **TRD / 설계** | **Designer 렌즈** (Codex) — 인터페이스·구조·교체성 건전성·더 단순안 | **Architect-breaker** (opus, doc-aware) — 불변식 위반·결합·기존 ADR 깨기·lifetime 공격 | **OFF** (opus=ADR 자동주입 / Codex엔 관련 ADR 묶음 명시 제공) | PBR: Designer + 우리 불변식·seam·capability·교체성 |
| **코드 (게이트)** | **correctness·단순성 옹호** — 목표 동작 충족·더 단순/명확한 구현 | **adversarial breaker** — race·lifetime·off-by-one·회귀·보안 공격 | **비대칭(실험)** — Codex=코드+계약만(blind 신선 breaker) / opus=doc-aware(불변식) | ODC 결함타입 + 우리 불변식(kill·finalize·락순서·epoch·replay) |
| **문서 정리** | **cut-advocate** (Codex, blind) — 중복·죽은 참조·군더더기 더 쳐내라 | **load-bearing 수호** (opus, doc-aware) — 삭제가 불변식·"왜"·안티패턴 경고·교차참조를 떨구나 | Codex=근거 숨김(blind) / opus=코드·ADR 접근 | 삭제-안전 체크(load-bearing 의미·교차참조 보존) |
| **(fallback) 미지정** | 목표 달성했나·더 나은/간결한 버전·빠진 것 | 뭐가 깨지나·안 적힌 가정·worst input/race·뭘 조용히 위반하나 | Adversary=doc-aware / Advocate=blind 기본 | (역할 일반 질문, 전용 체크리스트 없음) |

> **코드 게이트 주석:** 코드 리뷰는 본질이 적대라 두 역할이 모두 공격 성향이다(opus=불변식 doc-aware breaker / Codex=엣지·프로토콜 blind breaker). "Advocate(맞다·단순하다)"는 약한 보조 체크로 접는다. 이 비대칭 blind는 **리서치 실증이 없는 우리 가설**(§4) — 효과 측정 전까진 "옵션".

---

## 3. 공통 규약 (전 단계)

- **판정 스케일:** `PASS` / `FIX(항목 ≤N)` / `BLOCK`. 점수화 금지. (F6)
- **불일치 처리:** Advocate·Adversary가 갈리면(특히 cut vs keep, 채택 vs 위험) 메인이 종합하되 **임의 확정 금지 → 사용자에게 쟁점 보고**. (F7)
- **취합:** 두 결과를 라벨(A/B) 익명·순서 무관하게 합친다(순서 편향 차단).
- **effort:** 메인 세션 = **xhigh**(영구 천장; 그 위 ultracode는 세션 한정·effort↑ 아니라 워크플로우 자동화), 코더·리뷰어 = **high**(Codex는 medium 기본, 동시성·lifetime 치명 변경만 high). 무가드 통합 노드인 메인에 검수보다 effort를 싣는다.
- **코더:** opus(복잡)/sonnet(단순). 메인 직접 구현 금지(문서는 인라인 예외).
- **QA:** 리뷰와 별개 실측 게이트 — build/test + GUI(`scripts/cdp.mjs`). 변경 없음.

---

## 4. 정직한 표시 (근거 강도)

- **단단함(근거 있음):** 거친 스케일+체크리스트 우선(F6) · family 다양성(F8) · 특화 역할의 결함 커버리지(F1/F2) · 발산 블라인드의 앵커링 감소(F4) · 고정 Advocate/Adversary(devil's advocacy/dialectical inquiry — 단 SW 리뷰 직접 증거 아닌 전략의사결정 연구라 방향성).
- **약함/미검증:** 코드 단계 **비대칭 blind/doc-aware**(실증 0, 우리 가설) · PBR 관점의 *코드/문서 단계* 유효성(외삽) · "특화가 *항상* generic보다 낫다"(PBR 연구가 "perspectives가 항상 다르진 않더라"도 보임 — 특화는 방향성 우위지 절대선 아님).
- **버리지 말 것(역증):** 순서 편향 실재 → 취합 순서 무관 유지.
- **rot 주의:** 특화 픽스 표의 체크리스트(불변식·ADR·ODC)는 코드/ADR이 바뀌면 갱신해야 한다. 표는 박되 주기 점검.

## 5. 첫 적용 (dogfood)
**CLAUDE.md 문구 정리**를 위 "문서 정리" 행으로 실행: 메인이 cut 제안 작성 → Advocate(Codex, blind, "더 잘라") + Adversary(opus, doc-aware, "load-bearing 지켜") 병렬 → 불일치 항목은 사용자 판정 → 승인분만 편집.

## 6. 미결정 (사용자 비준 필요)
1. 코드 단계 비대칭 blind를 "강제"로 둘지 "실험 옵션"으로 둘지.
2. 각 단계 체크리스트의 구체 항목 확정(현재 출처만 지정).
3. 이 설계를 ADR로 박을지(거부 대안=웹 3패밀리 consult, generic-only 리뷰) + CLAUDE.md 추가 반영.
