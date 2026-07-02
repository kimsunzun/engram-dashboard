# adr 개선 검토 (research 재설계 결 적용 — 단 deterministic 스킬 특성 반영)

> 검토 대상: `_wip/adr/SKILL.md` · `_wip/adr/references/flow.md` · `_wip/adr/references/bindings/engram.md` · `_wip/adr/feedback.md`
> 품질 기준: `_wip/research/{SKILL.md, references/flow.md, REDESIGN-SPEC-v3.md}`
> 규약: `core/claude-global-shared/rules/markdown-format.md` · `CLAUDE.md`(ADR 절·rot 방지·핸드오프 체크리스트)
> 실체 대조(grounding): `scripts/adr.mjs`(존재 확인) · `docs/decisions/README.md`(상태범례·템플릿·인덱스·레거시 0016/0024/0020/0023/0027 대조) · `.claude/skills/_shared/self-improvement-feedback.md`

## 1. 현재 스킬 요약

adr은 **결정적 스크립트(`scripts/adr.mjs`) + 얇은 LLM 판단** 하이브리드다. 서기 일(채번·스캐폴드·인덱스 재생성·supersede 양방향 링크·lint)은 스크립트가 결정적으로, 판단 일(입력검증·전체/부분 폐기 판단·본문 prose·보고)만 LLM이 한다. 1차 축은 **오퍼레이션(new/supersede/lint)** 이고, 강도(light/medium/deep) 축은 **의도적으로 없다** — "ADR 기록은 결정적이라 깊이 트레이드오프가 없다"를 스킬이 명시(`SKILL.md:18-20`). 프로젝트 실체는 골격(`flow.md`)에서 빼내 `bindings/engram.md`가 채운다.

핵심 판정: **이 스킬은 research 재설계의 DNA(SSOT+rot방지·정직한 검증상태·seam 명확성·날조금지·self-improvement·마크다운 핀셋)를 이미 잘 인코딩했다.** research 고유 개념(cross-family 적대·calibration·grounding·강도축·모델배정표)은 deterministic 스킬 특성상 대부분 해당 없고, 스킬은 그걸 억지로 넣지 않고 정직히 뺐다(§5).

## 2. 갭·이슈 (우선순위·심각도)

전부 **저~중** 심각도의 미세 갭이다. thesis급 재설계 근거는 없다.

- **[중] full/partial supersede 판단에 구체 예시(`<examples>`) 없음.** "전체 폐기 vs 부분 폐기"는 이 스킬 **유일의 비자명 LLM 판단**인데(`flow.md:44` "★LLM의 핵심 판단"), SKILL/flow 어디에도 "이건 full·이건 partial" 사례가 없다. `markdown-format.md`는 룰을 *보여주는* 사례를 `<examples><bad><good>`로 분리하라고 권하고, CLAUDE.md 자신이 "혼동 쌍"에 그 패턴을 쓴다. full/partial은 전형적 혼동 쌍 — 실데이터 0016/0024(부분)가 산 반례다. 예시 1쌍이 오판(살아있는 조항을 통째 폐기)을 줄인다.
- **[저~중] "보장" 근거가 스크립트 검증에 앵커돼 있지 않음(정직성).** `SKILL.md:44` "이건 스크립트가 결정적으로 검사·강제한다"는 **adr.mjs 정확성에 전적으로 의존**하는 주장인데, 그 근거(스크립트 회귀/실데이터 검증)를 ⚠️ 절이 인용하지 않는다. feedback.md에는 근거가 있다(`feedback.md:16` "실데이터 32 ADR 무손실 검증, index --write idempotent"). research가 ⚠️ 절에서 "실측된 것(2026-07-01, 30문항)"을 못 박듯, adr도 "이 정합성 보장은 adr.mjs에 의존하며 실데이터 32 ADR idempotent 검증이 그 근거" 한 줄을 박으면 과청구가 아니라 grounded claim이 된다.
- **[저] `index`(재생성) 오퍼레이션이 트리거·추정규칙에 안 노출됨.** 바인딩·flow는 `index --write`/`--check`를 별도 명령으로 쓰지만(`bindings/engram.md:21-23`), 트리거는 `/adr [new|supersede|lint]` 3개뿐. "인덱스만 재생성해줘"는 어디로도 안 매핑된다(lint=read-only `--check`라 write 아님). 실제로는 new/supersede의 하위단계라 단독 노출이 불필요할 수 있으나, 사용자 자연어 요청("인덱스 다시 만들어") 매핑 부재는 작은 seam 구멍.
- **[저] flow §60의 `index --write` 서술이 lint 절 안에 섞여 lint가 write하는 듯 읽힘.** `flow.md:60`은 lint("보고 전용·자동수정 안 함") 절 안에서 "인덱스 재생성(`index`)만 …다시 쓰되"를 설명해, index --write(실은 new/supersede 하위단계·별도 명령)를 lint의 일부처럼 오해할 여지. 바인딩(`engram.md:22`)은 명확(lint=read-only, index --write=별도)하나 골격 prose 배치가 약간 흐림.
- **[저] bare `/adr`(오퍼레이션·요청 둘 다 없음) 기본값 표현이 두 파일에서 미묘하게 다름.** `SKILL.md:28`은 "'점검'·인자 없음=lint", `flow.md:18`은 "인자 없음 → 요청 내용으로 추정"(요청이 있다는 전제). 완전-빈 호출의 안전 기본값(read-only인 lint가 맞다)이 flow §0엔 crisp하지 않다. 저영향(lint가 안전 기본이라 사고는 안 남).
- **[저·선택] research의 "실행 중 자기보고" 소절에 대응하는 게 없음.** research는 실행 중 명세 문제를 즉시 보고 vs 사소는 feedback.md로 모으는 규율을 별도 절로 둔다(`research/SKILL.md:46-47`). adr은 feedback.md 포인터만 있고 "실행 중 발견 처리"를 명시 소절로 두지 않는다 — lint/supersede 중 명세 모순 발견 시 경로가 약간 암묵. flow §60 "의미 있는 drift → 사용자 판단"이 부분 커버라 저심각.

## 3. 제안 재설계 thesis (한 줄 + 근거)

**"큰 재설계 불요 — 하이브리드 seam이 이미 정답이고 잘 인코딩됨. 미세 개선(예시·정직 앵커·트리거 갭)만."**

근거: (1) seam 명확성(§3축)은 스킬의 존재 이유로 표·골격·가드레일에 삼중 인코딩됨(`SKILL.md:22-26` 표 "스크립트(기계)/스킬(판단)" · `flow.md:7-9` · `flow.md:98` 가드레일). (2) SSOT+rot방지(§1축)는 오히려 research보다도 강하다 — 바인딩이 "나는 스냅샷일 뿐, 충돌하면 스크립트/README/CLAUDE 따르고 나를 고쳐라"를 명문화하고(`bindings/engram.md:5`), 인덱스를 본문에서 파생(`bindings/engram.md:45`)해 CLAUDE.md "상태는 ADR 헤더에만·손으로 베끼는 리스트 금지"를 곧이곧대로 구현. (3) 날조금지(§4축)는 frontmatter·본문·flow·가드레일 4중 박제(`SKILL.md:46` · `flow.md:71-76` · `flow.md:96`) + Equal Experts 리서치 근거. → 손댈 곳은 구조가 아니라 예시·정직 문구·트리거 edge뿐.

## 4. 개선 항목 표

| 항목 | 무엇 | 왜 | 사용자결정? | research 전이? | 심각도 |
|---|---|---|---|---|---|
| A. full/partial 예시 | supersede 절에 `<examples>` 전체 vs 부분 1쌍(0016/0024식) | 유일 비자명 LLM 판단인데 예시 0 · markdown-format.md·CLAUDE.md가 혼동쌍엔 examples 권장 | No(메인 판단) | ✗ (research無, 마크다운 규약 전이) | 중 |
| B. 정직 앵커 | ⚠️ 절에 "정합성 보장은 adr.mjs 정확성에 의존 — 실데이터 32 ADR idempotent 검증이 근거" 한 줄 | 스크립트 과청구 방지 · research ⚠️ "실측된 것"의 adr판 | No | ✓ (research ⚠️ 검증상태 honesty) | 저~중 |
| C. index 노출 갭 | "인덱스만 재생성" 자연어→명령 매핑 추가(트리거 각주 또는 추정규칙) | 사용자가 인덱스 재생성만 원할 때 경로 부재(lint=read-only) | No | ✗ | 저 |
| D. flow §60 배치 | index --write 설명을 lint 절에서 분리(별도 명령임을 명확화) | lint가 write하는 듯 오독 방지 | No | ✗ | 저 |
| E. bare `/adr` 기본값 | flow §0에 완전-빈 호출=lint(안전 기본) crisp화 · SKILL과 표현 정합 | 두 파일 미묘 불일치 해소 | No | ✗ | 저 |
| F. 실행 중 자기보고 | (선택) "실행 중 명세문제 즉시보고 vs 사소는 feedback.md" 소절 | supersede/lint 중 발견 경로 명시 | No | △ (부분 전이) | 저 |

## 5. 정직 노트 (research에서 전이 안 되는 것이 많음 — 무엇이 왜 해당 없는지 · adr 고유 제약)

**해당 없음(억지 이식 금지 — 스킬이 정확히 뺐음):**
- **cross-family 적대 리뷰(Codex 리뷰어).** adr은 결정을 *박제*할 뿐 *타당성 검증*을 안 한다 — 산출물에 "confident-wrong"이 없다(형식 정합만 결정적으로 검사). 스킬이 `SKILL.md:45`에서 "빈약하거나 틀린 결정도 형식만 맞으면 기록은 PASS"를 정직히 선언, 결정 옳음은 review/prd 몫으로 위임(`SKILL.md:47`). 리뷰어 개념이 없으니 BLIND·레벨 사다리·tiebreaker·생산자≠리뷰어도 전부 N/A.
- **calibration(기권/확신도) · grounding(claim↔source 함의).** 사실 *수집*이 아니라 결정 *기록*이라 확신도·출처함의 축 자체가 없다. adr은 이걸 명시적 out-of-scope로 긋는다(⚠️ "보장하지 않는 것 = 결정의 옳음").
- **강도 축(light/medium/deep).** 스킬이 `SKILL.md:18-20`·`flow.md:15`에서 "결정적 작업이라 깊이 트레이드오프가 없다 — 가짜 강도를 끼우지 않는다"를 명문 거부. review의 *단계* 축과 동형인 오퍼레이션 축으로 대체 — research DNA의 *올바른* 번역이지 누락이 아님.
- **역할→모델 배정표(Fable-ready).** adr은 결정적 스크립트 + 단일 LLM 판단이라 여러 모델 슬롯이 없다 — 배정표 불필요. 특정 모델에 절차를 안 묶는 원칙은 "스크립트 vs LLM" seam으로 이미 충족.
- **⚠️ "미검(정직)" 가설 섹션.** research는 "적대리뷰 값어치=근거있는 가설(미검)"을 다는데, adr은 미검 가설이 아니라 **결정적 보장**이라 hypothesis-honesty 대신 **scope-honesty**(보장하는 것=기록 정합성 / 보장 안 하는 것=결정 옳음)를 쓴다(`SKILL.md:40-47`). 이게 deterministic 스킬의 올바른 정직성 형태 — research 형식을 복사하지 않은 게 정답.

**전이됨(공통 DNA — 이미 잘 적용):** SSOT+rot방지(§1, 오히려 더 강함) · scope-honesty형 ⚠️ 검증상태(§2) · seam 명확성(§3, 스킬의 존재 이유) · 날조금지(§4, 4중 박제+리서치 근거) · self-improvement feedback(feedback.md + `_shared` 규약, usage-log 언급도 `_shared:12-20`과 정합) · 마크다운 핀셋(prose+헤더+표, XML 오남용 0).

**adr 고유 제약(research엔 없는 것):**
- **스크립트 의존.** 스킬 품질의 하한이 `scripts/adr.mjs` 정확성에 묶인다 — research는 순수 LLM 절차라 이 의존이 없다. → 개선항목 B(보장 근거를 스크립트 검증에 앵커)가 여기서 나온다.
- **실데이터 레거시 부채.** 0016/0024(부분폐기 본문에 Amends 링크 없음, 인덱스 단서가 본문보다 풍부) · 0020/0023/0027(단방향 자연어 supersede) — `docs/decisions/README.md:66,70,73,74,77`에서 실측 확인. 스킬은 이걸 **"보존 + advisory 플래그"** 로 다루되(`bindings/engram.md:47` · `flow.md:60`), 정형 키워드 마이그레이션 여부는 **미결로 정직히 열어둠**(`feedback.md:16` "검토 대기" 3건 — 메인/사용자 결정 대기). 이 "안 정한 채 정직히 열어둠"은 research의 mode-aware 에스컬레이션과 결이 같다(승자 강요 X).
