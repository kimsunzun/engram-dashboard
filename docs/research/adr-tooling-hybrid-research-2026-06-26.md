# ADR 자동화 — "스크립트 + 판단 한 스푼" 하이브리드 업계 관행 (리서치)

- 상태: 완료 (2026-06-26) · 강도: research 스킬 **medium**
- 방법: Claude(Sonnet) 2갈래(supersede / 하이브리드 아키텍처) + Codex 1, **BLIND 독립 수집** → opus 교차 대조·적대 검증. (채번·인덱스 갈래는 6-25 라운드에서 회수.)
- 목적: engram `adr` 스킬을 "결정적 스크립트 + LLM 판단" 하이브리드로 재설계. `/review trd full`이 잡은 부분 폐기·lint·채번 구멍의 해법 탐색.
- 확신도 범례: 확실 / 가능성 높음 / 불확실

## 핵심 결론

1. **기계 vs 판단 경계는 업계 전반에 일관**(확실). 스크립트 = 서기 작업(채번·파일생성·템플릿·인덱스 재생성·supersede 링크 박기·형식 lint). 사람/LLM = 사유 작업(본문 prose·전체/부분 폐기 판단·status 해석·"ADR감인가" 트리거). 2015 adr-tools ~ 2026 adr-kit까지 동일.
2. **adr-tools(npryce)가 표준 thin 하이브리드 레퍼런스**(확실). bash 수백 줄. `adr new -s N` = supersede 양방향 자동, `-l "N:Amends:Amended by"` = 임의 양방향 관계.
3. **부분 폐기(partial supersede)는 표준 도구 지원이 없다**(확실). 전체 폐기는 status를 `superseded`로 바꾸는 게 기본. 부분은 ① adr-tools `-l Amends/Amended by` 양방향 링크 관습 ② 본문 dated note(living-doc, joelparkerhenderson) — **둘 다 옛 ADR status는 유지**하고 링크/노트만 추가.
   - → **우리 구멍의 정답:** 부분 폐기 = status 유지 + 양방향 `Amends`/`Amended by` 링크(+조항 단서). status를 통째 '폐기'로 덮지 않는다.
4. **양방향 링크가 핵심이고 자동화가 가치**(확실). 손으로 하면 한쪽 빠뜨림 → 스크립트가 양쪽 박는 게 adr-tools의 존재 이유(우리 review 지적과 동일).
5. **index/TOC는 별도 관심사, 재생성형**(확실). adr-log·pyadr `generate-toc`가 본문 스캔해 index 재생성. adr-tools 자체는 index 안 만듦.
   - → 인덱스를 본문에서 **재생성**하면 "인덱스↔본문 drift"·"제목 drift"가 원천 차단(본문=단일 출처). 우리 "상태 정본=본문 헤더" 불변식과 정합.
6. **status 어휘 표준** = proposed/accepted/deprecated/superseded(+rejected)(확실). superseded=더 나은 결정으로 대체 / deprecated=대체 없이 폐함.
7. **LLM을 ADR에 끼운 패턴은 2024–2026 등장, 주류 내장은 아직 아님**(가능성 높음). 검증된 패턴 = Equal Experts "kernel of truth"(사람이 핵심 제공 → LLM 초안 → AI 비평 → 사람 fact-check·승인), Codex CLI 거버넌스, rvdbreemen/adr-kit(MCP). 공통 경고: **LLM이 Context/Consequences를 환각**(없는 API 생성) → 사람 검증 필수.
8. **CI/훅 lint는 형성 중, 비주류**(확실). archgate·adr-kit·Codex CLI 패턴 전부 2025–2026 신생.

## 교차검증표 (Claude ↔ Codex)

| 클레임 | Claude | Codex | 판정 |
|---|---|---|---|
| 기계/판단 경계 일관 | ✓ | ✓ | 수렴·확실 |
| adr-tools = thin 레퍼런스 | ✓ | ✓ | 수렴·확실 |
| 부분 폐기 표준 미지원, Amends 링크가 유일 실용 | ✓ | ✓ | 수렴·확실 |
| 양방향 자동화는 adr-tools만 | ✓ | ✓ | 수렴·확실 |
| index 재생성 = 별도 도구 | ✓ | ✓ | 수렴·확실 |
| LLM 패턴 등장·비주류 | ✓ | ✓ | 수렴·가능성높음 |
| EventCatalog 구조화 `amends` 필드 | ✓(미확정) | — | 한쪽만·가능성높음 |
| dotnet-adr 실존 | 라운드1 확인 | 확인불가 | 약함·불확실 |

## 설계 함의 (engram adr 하이브리드)

**`scripts/adr.mjs` (결정적):**
- `new` — 채번(본문 파일 max+1, 쓰기 직전 재스캔) + 템플릿 파일 생성 + 인덱스 재생성
- `supersede` — 전체: 옛 ADR status→`폐기 (Superseded by N)` + 새 ADR `Supersedes M`(양방향) / **부분: 옛 status 유지 + `Amended by N` ↔ 새 `Amends M` 양방향 링크**
- `index` — 본문 H1·상태 스캔해 README 인덱스 **재생성**(drift 원천 차단)
- `lint` — 상태 *어휘만* 비교 / 중복·빠진 번호 / supersede 양방향 일치 / 코드 앵커 `// ADR-`(코드 경로 한정, `docs/` 제외) 고아

**`adr` 스킬 (얇게, 판단):** 사용자 결정 수령 → **전체 vs 부분 폐기 판단** → 본문 prose 정리(kernel→템플릿) → 스크립트 호출 → 결과 보고. 거부한 대안·근거는 사용자 제공(환각 금지·fact-check).

→ 이 설계가 `/review`가 잡은 6개 구멍(부분폐기·lint거짓양성·채번·rg노이즈·제목drift·상태정본)을 **구조적으로** 해소한다.

## 공백·한계
- **만장일치 주의:** 두 family 모두 adr-tools에 강하게 앵커링 — 가장 오래·잘 문서화돼 과대표집 가능. 단 "부분 폐기 표준 없음"은 다수 독립 도구 확인이라 견고.
- EventCatalog의 구조화 `amends` 필드는 스키마 미확인(추가 조사 여지).
- LLM-ADR 연구의 효과 수치는 초록 미공개 다수(정량 불확실).

## 출처
- adr-tools(npryce): https://github.com/npryce/adr-tools (`src/adr-new` 소스 직접 확인)
- adr-log: https://github.com/adr/adr-log · pyadr: https://github.com/opinionated-digital-center/pyadr
- MADR: https://adr.github.io/madr/ · log4brains: https://github.com/thomvaill/log4brains
- joelparkerhenderson ADR 모음: https://github.com/joelparkerhenderson/architecture-decision-record
- Backstage ADR: https://backstage.io/docs/architecture-decisions/ · EventCatalog: https://www.eventcatalog.dev/blog/introducing-adrs
- archgate/cli: https://github.com/archgate/cli · rvdbreemen/adr-kit: https://github.com/rvdbreemen/adr-kit · kschlt/adr-kit
- Codex CLI ADR 거버넌스: https://codex.danielvaughan.com/2026/04/28/codex-cli-architecture-decision-records-adr-automated-governance/
- Equal Experts(생성 AI + ADR): https://www.equalexperts.com/blog/our-thinking/accelerating-architectural-decision-records-adrs-with-generative-ai/
- 논문: arXiv 2403.01709(Can LLMs Generate ADDs) · arXiv 2504.08207(DRAFT-ing ADDs) · AWS Prescriptive Guidance(ADR process)
