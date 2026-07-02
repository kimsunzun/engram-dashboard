# review 개선 검토 (research 재설계 결 적용)

> 작성 2026-07-02. 대상 = `_wip/review/`(SKILL.md · flow.md · bindings/engram.md · feedback.md). 기준 = `_wip/research/` v3.2가 도달한 설계 DNA. **이 문서는 개선 검토(제안)이지 rewrite가 아니다** — 굵은 재설계는 사용자 승인 후 실행한다. 근거는 파일·줄로 인용한다.

## 1. 현재 스킬 요약 (뭐하는 스킬·구조·핵심 설계축)

review는 변경물을 **Advocate(옹호·강화) vs Adversary(공격·대척)** 고정 2인으로 적대 검증하는 **범용 리뷰 엔진**이다. 판정은 거친 3단(`PASS`/`FIX`/`BLOCK`), 불일치는 사용자에게 에스컬레이션한다.

핵심 설계축 두 개가 **직교**한다(SKILL.md:12-19):
- **단계(무엇을 보나)** = `prd`|`trd`|`code`|`doc`|fallback → 어느 역할 렌즈·블라인드·체크리스트를 쓸지 고른다.
- **강도(얼마나)** = `self`|`light`|`full`(기본)|`deep` → 리뷰어 인원·깊이를 고른다.

구조는 research와 **같은 저자 계열의 설계 언어**를 이미 공유한다 — cross-family 적대(opus+Codex, 다른 family 강제), PASS/FIX/BLOCK 거친 스케일(점수화 금지, F6), 순서·라벨 무관 취합, `references/flow.md` 실행 정본 분리, `bindings/<project>.md` 프로젝트 분리, `feedback.md` 자기개선 누적, ⚠️ 검증 상태 정직 라벨. **research의 DNA 상당수를 이미 만족한다** — 이 검토는 from-scratch rewrite 후보가 아니라 **표적 패치** 판단이다.

설계 근거 정본은 `docs/research/review-pipeline-design-draft.md`(F1~F8 · PBR · ODC · devil's advocacy)에 있고, review 골격은 그 §2 역할표를 실행용으로 옮긴 것이다.

## 2. 갭·이슈 (우선순위·심각도)

### G1. 역할→모델 배정표 미분리 (심각도: 높음)
research는 모델명을 flow.md §역할→모델 **배정표 한 곳에만** 두고 본문은 역할 슬롯으로만 말한다(신모델=표만 교체, research/flow.md:18-33). review는 정반대로 모델명이 **§2 역할표 셀마다 + 여러 산문에 박혀 있다**:
- flow.md:54-57 — 표의 모든 역할 셀에 모델명 하드코딩: "User 렌즈 **(Codex)**", "Tester 렌즈 **(opus)**", "Designer 렌즈 **(Codex)**", "Architect-breaker **(opus, doc-aware)**", code 행 "Codex=코드+계약만 / opus=doc-aware", doc 행 "cut-advocate **(Codex, blind)** / load-bearing 수호 **(opus, doc-aware)**".
- flow.md:60 원칙 산문, flow.md:67 스폰("Codex=`mcp__codex__codex`, opus=Agent"), SKILL.md:8·52 도 opus/Codex 직박.

Fable 등 신모델이 opus를 대체하면 **§2 표 전 행 셀 + §3 + 원칙주석 + SKILL을 다 손봐야** 한다 — research가 정확히 없앤 안티패턴이다. 교체성(아키텍처 원칙 §0)에 직접 반한다.

**정직한 복잡도 주석:** review의 family 배정은 blind/doc-aware만으로 일률 결정되지 않는다 — PRD는 Advocate=Codex·Adversary=opus인데 **둘 다 blind ON**(opus Tester가 앵커링 차단용 blind, flow.md:54). 나머지(trd/code/doc)는 **Codex=blind-fresh / opus=context-heavy(doc-aware)** 로 일관된다. 그래서 배정표는 "blind-fresh 슬롯→Codex, doc-aware 슬롯→opus, **단 PRD는 opus도 blind 모드**" 한 줄 주석이면 표현 가능하다. research 표를 그대로 복사가 아니라 **PRD 예외 한 줄 얹은 배정표**로 만드는 게 정확하다.

### G2. evidence-grounded 판정(반박=반증/구체적 파괴사례 강제) 부재 (심각도: 중간~높음)
research는 반박(refute) verdict에 **반증 출처를 강제**하고, 근거 없는 의심은 finding까진 되나 verdict는 안 된다(research/flow.md:104). 과적대 오경보·비용폭발을 막는 고삐다. review의 Adversary는 **구체적 파괴사례 없이 FIX/BLOCK을 낼 수 있다** — flow.md:77 스폰 출력형식은 "`findings[]`(항목마다 근거 한 줄)"만 요구하고, flow.md:82 판정 스케일도 "핵심 불변식 위반·방향 재검토"라 부를 뿐 **"어느 입력/레이스/줄/불변식이 어떻게 깨지나"를 명시하라는 강제가 없다.** "위험해 보인다"만으로 BLOCK이 가능 → 멀쩡한 변경을 약한 근거로 막는 오경보 위험.

**전이 형태(억지 이식 아님):** research의 "반증 출처(URL)"를 그대로 옮기지 않는다 — review의 반증 = **구체적 실패 시나리오/repro/위반 불변식+깨지는 경로/버그 줄**이다. "grounded verdict: 모든 FIX/BLOCK은 추상적 불안이 아니라 구체적 파괴 방식을 지목한다"로 번역하면 그대로 review에 붙는다.

### G3. abstention ≠ contradiction 미구분 (심각도: 중간)
review 불일치 처리(flow.md:83)는 "Advocate·Adversary가 갈리면 사용자 보고"만 있고, **세 경우를 구분하지 않는다**: ① Adversary가 근거 있는 BLOCK을 냄 vs ② Adversary가 반증 없이 불안만 표함 vs ③ Adversary가 그 지점을 아예 안 다룸(coverage 밖). research는 "단지 기권 ≠ 반박, 반증 낼 때만 contested"로 이걸 가른다(research/flow.md:131). G2와 한 몸 — evidence-grounded가 들어오면 ①만 진짜 대립으로 승격되고 ②③은 에스컬레이션 트리거가 아니게 된다. 지금은 ②(근거 없는 불안)도 "불일치→사용자"를 당길 수 있어 오경보가 사용자 시간을 먹는다.

### G4. mode-aware 에스컬레이션 부재 (심각도: 중간)
research는 남은 contested를 **모드로 가른다** — 대화 모드=사용자 질문 / 자율("진행 쭉해") 모드=태그 유지+로그+진행, 마이너는 메인 자율(research/flow.md:136-143). review는 flow.md:83에서 불일치를 **항상 "사용자에게 쟁점을 보고해 판정을 받는다"** 로만 처리한다 — 자율 모드 분기가 없다. CLAUDE.md 구현 실행 규약은 "진행 쭉해" 자율 모드에서도 review를 강제하는데, 그때 review는 매 Advocate/Adversary 불일치마다 멈춰 물어 자율 흐름을 깨거나(과차단) 동작이 미정의다. review의 "불일치→사용자"는 F7(자기편 편향 백스톱)이라 **load-bearing 대립은 물어야 맞다** — 부족한 건 **마이너/load-bearing 게이트 + 자율 모드 시 마이너는 태그+로그+진행**이다(flow.md:82의 "FIX 5↑=BLOCK 분리"류 무게 판별을 에스컬레이션에도 재활용).

### G5. deep 강도 정량 미정의 (심각도: 중간)
research는 deep을 **리뷰어 독립도 레벨 2~5 사다리**로 정량화한다(검산→홉 독립 재도출→다중렌즈+대안답 랭킹→blind 재해결, research/flow.md:107-114). review의 deep은 flow.md:25 "2인 + 다관점/다회", flow.md:63 "추가 렌즈로 또는 반복"으로 **모호**하다 — 몇 명·어떤 렌즈·언제 반복인지 미정의.

**전이 형태(적응이지 복사 아님):** research 사다리 축(수집자 답으로부터의 독립도)은 review에 부분만 맞는다 — 리뷰 대상(diff/문서)은 반드시 읽어야 해서 "답을 안 읽고 재도출"(레벨3)이 그대로는 안 된다. 다만 **"명세만 주고 올바른 동작을 리뷰어가 독립 재도출→구현과 대조"(research 레벨5 blind 재해결의 review판)** 는 유의미하다. deep을 (a) 열거된 추가 렌즈(보안/성능/마이그레이션) + (b) 옵션 blind 정정(명세→기대동작 독립 재도출) 로 **구체화**하면 된다. 5단 사다리 통째 이식은 과전이 — review deep의 핵심 축은 "독립도 심화"보다 "렌즈 폭"이라 정직하게 구분한다.

### G6. fresh(생산자≠리뷰어) 불변식 명시 약함 (심각도: 낮음~중간)
research는 "리뷰어 = 수집자·합성자·메인과 다른 fresh 인스턴스"를 배정표 옆 불변식으로 못박는다(research/flow.md:31, 가드레일:164). review는 fresh를 flow.md:67 괄호("Agent 서브에이전트가 불가한 환경에서만 fallback으로 **메인 opus 별도 패스**")로만 흘리는데, 이 fallback은 메인이 생산에 관여했으면 **fresh 위반**이다(자기 산출 자기통과). §4는 "메인=Claude 계열 자기편 편향"을 인지하나 fresh 강제로 승격되진 않았다. G1 배정표를 만들 때 fresh·same-family금지를 그 옆에 모으면 한 번에 정리된다.

### G7. 프로젝트 바인딩 누수 — 근거 출처가 골격에 하드코딩 (심각도: 낮음~중간)
골격(범용 엔진)이 **engram 전용 문서 경로를 직박**한다: flow.md:50·SKILL.md:50 "`docs/research/review-pipeline-design-draft.md` §2가 정본", SKILL.md:52 "F6/F8" 등. 다른 프로젝트가 이 엔진을 채택하면 그 문서가 없다. 방법론(PBR·ODC·devil's advocacy)은 보편이라 골격에 남겨도 되지만 **파일 경로·Fn 라벨은 bindings로 내리거나 추상 참조로** 바꿔야 교체성이 산다. research는 ⚠️를 스킬 자체 검증(30문항 실측)에 한정해 이 누수를 피했다.

### G8. SSOT 앵커 drift·§번호 오참조 (심각도: 낮음)
- SKILL.md:23이 강도표 정본을 "`flow.md` **§0** 강도표"로 가리키지만, flow.md의 강도표는 §0(=Codex 가용성 확인)이 아니라 **무번호 `## 강도 정하기` 절**에 있다. 앵커 rot.
- flow.md §2 표와 draft §2 표가 **거의 동일 복제**다(flow.md:52-58 ↔ draft.md:33-39). flow가 "실행용 요약, draft가 정본"(flow.md:50)이라 선언하나 내용이 사실상 겹쳐 한쪽만 갱신되면 rot. 어느 쪽이 무엇을 담는지 경계가 얇다.

## 3. 제안 재설계 thesis (한 줄 + 근거)

**review는 골격이 이미 research DNA와 정합하므로 rewrite가 아니라, 모델명을 배정표 한 곳으로 걷어내(교체성) + 적대 판정을 evidence-grounded로 조여(오경보 억제) + 자율 모드 에스컬레이션을 넣는(자율흐름 정합) 표적 패치로 충분하다.** 근거: cross-family·PASS/FIX/BLOCK·정직 라벨·바인딩 분리·feedback은 이미 있고(§1), 빠진 건 G1(교체성)·G2/G3(정직한 적대)·G4(mode-aware)라는 **research가 최근 추가한 델타**에 정확히 대응한다.

## 4. 개선 항목 표

| 항목 | 무엇 | 왜 | 사용자결정? | research 전이? | 심각도 |
|---|---|---|---|---|---|
| **G1 배정표 분리** | 모델명을 §2 셀·산문에서 걷어 flow.md에 역할→모델 배정표 한 곳(+PRD 둘다blind 예외 주석). 본문은 blind-fresh/doc-aware 슬롯으로만 | 신모델 교체 시 표 한 곳만 수정(교체성·아키텍처 §0). 지금은 표 전행+§3+SKILL 다 손봐야 | 메인(구조 결정 — 보고) | **전이 O** (research 배정표 패턴, 단 PRD 예외 얹음) | 높음 |
| **G2 evidence-grounded verdict** | 모든 FIX/BLOCK은 구체적 파괴사례(입력/레이스/줄/위반 불변식+깨지는 경로) 지목 강제. 추상적 불안은 finding까지, verdict 아님 | 과적대 오경보·멀쩡한 변경 차단 방지 | 사용자(적대 강도 정책 변화) | **전이 O** (반증출처→구체적 파괴사례로 번역) | 중간~높음 |
| **G3 abstention≠contradiction** | 불일치를 ①근거BLOCK ②근거없는 불안 ③미커버로 3분. ①만 대립 승격 | 근거 없는 불안이 사용자 에스컬레이션 오발 방지 | 메인(보고) — G2와 한 몸 | **전이 O** (research abstention 규칙) | 중간 |
| **G4 mode-aware 에스컬레이션** | 마이너/load-bearing 게이트 + 자율 모드는 마이너=태그+로그+진행, load-bearing만 질문 | "진행 쭉해" 자율 흐름을 매 불일치마다 안 끊음(CLAUDE.md 자율 모드 정합) | 사용자(에스컬레이션 정책) | **전이 O** (research mode-aware) | 중간 |
| **G5 deep 정량화** | deep을 열거 렌즈(보안/성능/마이그레이션) + 옵션 blind 정정(명세→기대동작 독립 재도출)으로 구체화 | "다관점/다회" 모호 제거 | 사용자(선택지 제시) | **부분 전이**(사다리 축 적응 — 5단 통째는 과전이) | 중간 |
| **G6 fresh 불변식 명시** | fresh(생산자≠리뷰어)·same-family금지를 배정표 옆 불변식으로 승격. §3 "메인 opus fallback"의 fresh 위반 경고 | 자기 산출 자기통과 차단 | 메인(보고) — G1과 함께 | **전이 O** | 낮음~중간 |
| **G7 바인딩 누수 정리** | engram 문서 경로·Fn 라벨을 bindings로 내리거나 추상 참조화 | 다른 프로젝트 채택 시 교체성 | 메인(보고) | (research가 회피한 패턴) | 낮음~중간 |
| **G8 앵커 drift 수정** | SKILL.md:23 "§0" 오참조 정정 + flow §2↔draft §2 경계 명확화 | SSOT rot 방지 | 메인(인라인) | (research SSOT 규율) | 낮음 |

## 5. 정직 노트 (뭐가 전이 안 되나 · 과청구 경계 · review 고유 제약)

**전이하면 안 되는 것(research 고유 — 억지 이식 금지):**
- **라우팅("싸게 vs 적대리뷰")** — research의 핵심 산출. review는 **항상 적대**가 정체성이라 "적대를 건너뛸지" 판별이 없다. review의 강도는 "적대의 깊이/인원"이지 "적대 여부"가 아니다 → research의 "라우팅=핵심 산출" 프레이밍을 이식하지 말 것. review의 2축은 이미 옳다.
- **grounding(claim↔source 함의)** — 사실 환각 방어용 research 고유 pass. review는 "변경이 의도대로 도나/뭐가 깨지나"를 보지 인용↔출처 함의를 안 본다. **G2의 evidence-grounded는 "grounding"이 아니라 "grounded verdict"** — 이름·개념을 혼동해 grounding pass를 review에 달지 말 것.
- **BLIND(병렬 수집 독립)** — research BLIND는 두 family가 서로 안 보고 **수집**하는 축. review엔 수집 단계가 없고, review의 "blind"는 **리뷰어에게 결정 근거·ADR을 안 주는(앵커링 차단)** 전혀 다른 축이다(flow.md:54-57 블라인드 열). 두 blind를 합치지 말 것 — review blind는 이미 §2 표에 잘 정의됨.
- **확신도 태그(확실/가능성높음/불확실)** — research 산출 형태. review 산출은 PASS/FIX/BLOCK이고 그게 review의 calibrated 스케일(F6 점수화 금지)이다. 확신도 태그를 verdict에 덧대지 말 것. "확신도=파생" DNA는 review에선 "verdict는 자기보고 확신이 아니라 구체적 파괴사례에서 나온다"(=G2)로만 흡수한다.

**과청구 경계(현재 review가 이미 정직한 부분 — 보존할 강점):**
- SKILL.md:52-55 ⚠️ 검증 상태가 **근거 강도를 이미 분리**한다 — 단단함(F1/F2/F4/F6/F7/F8) vs 약함/미검(비대칭 blind=실증0 가설·PBR 코드/문서 외삽·"특화가 항상 낫다" 단정 금지). research의 정직 톤과 동급이다. G5 deep 정량화 시 새 레벨을 "실측"으로 과청구하지 말고 이 라벨 규율을 이어갈 것.

**review 고유 제약(research엔 없는 것):**
- **리뷰 대상은 반드시 읽어야 함** — research 리뷰어는 답을 안 읽고 독립 재도출(레벨3)이 가능하지만, review 리뷰어는 diff/문서를 안 읽을 수 없다. deep 사다리(G5)가 research를 통째 복사 못 하는 이유 — "명세만 주고 blind 재도출"(레벨5 아날로그)만 부분 유효.
- **"불일치→사용자"는 review의 F7 코어** — research보다 사용자 백스톱 성향이 강하다(메인=Claude 자기편 편향 차단). G4 mode-aware는 이걸 **완화가 아니라 자율 모드에서만 마이너를 태그+로그로 미루는 것** — load-bearing 대립은 여전히 물어야 F7이 산다.
