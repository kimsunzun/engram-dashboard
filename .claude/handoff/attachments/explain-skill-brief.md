# 스킬 팩토리 인계 — 전역 스킬 `explain` 신설 요청

작성 2026-09-08 · 출처 세션 = engram-dashboard-wt1 (S21 codex 백엔드 작업 중 파생)

## 0. 한 줄

사람에게 설명하는 **작성 규율**(덱·문서·그림)을 전역 스킬 `explain` 으로 세운다. 규율 본문은 아래 §5 에 완성돼 있다 — **새로 조사하지 않는다.**

## 1. 무엇을 만드나

- **위치:** `~/.claude/skills/explain/`
- **구성:** 기존 전역 스킬 관례를 따른다(`SKILL.md` + `references/flow.md` + 필요하면 `feedback.md`). **프로젝트 바인딩은 필요 없다** — 이 규율은 스택·프로젝트와 무관하다.
- **트리거 어휘(제안):** 브리핑 · 덱 · 슬라이드 · ppt · 프레젠테이션 · 「문서로 정리」 · 「설명 문서」 · 「이해되게 써줘」.
- **강도 축:** 초안은 강도 없이 규율만 갖는다. 필요 여부는 팩토리 판단.
- **언어:** 규율 본문 한국어(engram 계열 관례).

## 2. 왜 — 이 스킬이 생긴 경위

- **발단:** codex 백엔드 작업을 HTML 덱으로 브리핑했다가 사용자가 반려했다. 그 자리의 말: 「야 이게 PPT야? 너 발표할때 이렇게 발표하냐? 뺄꺼는 과감하게 빼고 전달할건 확실히 전달하고.」
- **진단:** 분량이 문제가 아니라 **한 장이 여러 일을 했다.** 슬라이드 하나에 설명 문단 + 그림 + 경고 박스 + 근거 앵커 + 표를 함께 넣었다. 슬라이드 제목도 「override 가 먼저, 그다음 capability」처럼 **주제 나열**이었다.
- **사용자 지시:** 「이런 ppt 문서 스킬같은거 숙지하고 적는게 좋아보이는데? 그런 스킬들 혹시 오픈소스로 따로 있는지 확인해봐.」 그래서 조사했다.
- **조사 결론: 공신력 있는 「스킬」은 없다. 권위는 방법론 쪽에 있다.**
  - Anthropic 공식 `skills/pptx` — **오픈소스가 아니다**(source-available). 내용도 렌더러·디자인·QA 위주로 **서사·구조 지침이 없다.**
  - `zarazhangrui/frontend-slides`(MIT, 별 다수) — 쓸 만한 것은 **발표자 중심(1~3 불릿) / 독서 중심(4~8 불릿)을 모드로 갈라 놓은 것** 하나뿐이고 **방법론 인용이 0**이다.
  - 나머지(open-slide, marp·slidev 계열)는 렌더러 지향.
- **그래서 이 스킬의 값어치 = 방법론을 채택해 규율로 박는 것.** 차용 대상 = assertion-evidence(덱) · Diátaxis(문서 종류 판정) · Google 개발자 문서 스타일(그림) · Mayer/Sweller(측정된 효과).

## 3. 경계 — 넣지 말 것

- ★**채팅(사용자 대면 보고) 규율은 이 스킬에 넣지 않는다.**★ 그건 전역 `user-facing-rules.md` 소관이다. 스킬은 그 파일을 **가리키기만** 한다 — 같은 규칙을 두 곳에 두면 갈린다.
- 같은 결정으로 `user-facing-rules.md` 「보고 규율」 절에 **세 줄이 따로 추가된다**(사용자가 직접 반영): ① 항목 배열은 사용자가 물을 순서로 ② 규칙 충돌 우선순위 ③ 터미널 표기 밀도(강조를 쌓지 않는다). **스킬이 이 세 줄을 복사하지 않는다.**
- **렌더러를 규율에 박지 않는다**(Marp·Slidev·reveal.js·pptxgenjs 등). 이번 실물은 단일 HTML + mermaid 였고, 형식은 소비처가 정한다.
- 아래 §5 규율에 **없는 조언을 보태지 않는다.** 출처 없는 작성 팁이 섞이면 이 스킬의 유일한 특징(전부 출처 있음)이 사라진다.

## 4. 미결 — 사용자 결정 대기

**md 한 파일이 「상세 정본」과 「사용자 대면 설명」을 겸하는 문제.** 선택지 둘과 각 대가가 규율 본문 §3-2 에 적혀 있다. ★스킬을 만들면서 이것을 임의로 고르지 않는다 — 그 절의 금지 문구를 그대로 옮긴다.★

## 5. 규율 본문 — 이 아래 「설명 규율 초안」 전체를 그대로 옮긴다 (그 문서의 절 번호는 자체 번호다)

---

# 설명 규율 초안 — 근거 있는 것만

> 이 초안의 모든 규칙에는 출처가 붙는다. 출처 없이 좋아 보이는 조언은 넣지 않았다.
> 조사 = tier medium(수집 3갈래 · grounding 메인 외부 검증 · 적대 리뷰 1회 반영).
> **표기 규약:** ★출처 세부 미확인★ = 판·쪽·실험 대조가 이 초안에 없다는 표시다 — 채우기 전엔 근거로 인용하지 않는다. **경험칙** = 재현 가능한 합격 기준이 없는 규칙 — 게이트로 쓰지 않는다.

---

## 0. 규칙이 충돌할 때 — 우선순위

**정확성 > 판단에 필요한 전제 > 간결 > 형식 취향.**

- 앞의 것이 뒤의 것을 항상 이긴다. 간결이 전제를 지우면 위반이고, 형식 취향(그림 밀도·불릿 개수·줄 수)이 간결을 이기는 일은 없다.
- ★**이 순서는 오케스트레이터 결정이지 출처 있는 규칙이 아니다**★ — 아래 어느 절도 이 순서를 측정하지 않았다. 근거로 인용하지 말고 결정으로 인용한다.
- 뒤집으려면 새 결정으로 뒤집는다.

## 1. 먼저 독자를 정한다 — 이게 나머지 전부를 결정한다

**전문가 역전 효과(expertise reversal):** 초심자에게 효과적인 설명 방식이 숙련자에게는 효과를 잃거나 **해가 된다**. 직격 사례 = 응집성을 높이려 추가한 설명문이 **저지식 독자에게만 이득이고 고지식 독자는 원문이 더 나았다**(McNamara 1996, Kalyuga et al. 2003 이 인용). ★출처 세부 미확인★(실험 대조·성과 지표)

기제: 같은 글이 초심자에겐 여러 조각으로, 숙련자에겐 한 덩이로 처리된다(element interactivity). **글의 부담은 글의 속성이 아니라 독자와의 관계다.**

★**역전을 가르는 축은 「관련 사전지식 + 제시 조건」이다 — 문서가 어디로 가느냐가 아니다.**★ 사용자용/세션용이라는 목적지는 독자의 숙련도를 정해 주지 않는다.

- → **규칙: 각 문서는 머리에 「가정하는 사전지식」과 「독자의 과업」을 명시한다.** 그 둘이 적혀 있지 않으면 어느 규칙을 적용할지 정할 수 없다.
- → **규칙(독자를 모를 때): 짧은 진입 설명을 먼저 두고 상세는 뒤로 미룬다.** 진입 설명은 사전지식을 가정하지 않고, 상세는 아는 독자가 건너뛸 수 있게 배치한다.
- → 이 프로젝트의 기존 구분(step-log·TRD = 세션용 / `reference/structure/` = 사용자용)은 **설계 결정이다** — 위 효과가 그 구분을 지지한다는 근거가 아니다.

출처: Kalyuga·Ayres·Chandler·Sweller, *Educational Psychologist* 38(1):23–31, 2003 · Sweller·van Merriënboer·Paas, *Educational Psychology Review* 31:261–292, 2019.

## 2. 그다음 네 갈래로 가른다 — 조회 / 절차 안내 / 학습 / 개념 이해

**첫 컷은 네 갈래다**(Diátaxis 사분면): **조회**(reference) · **절차 안내**(how-to) · **학습**(tutorial) · **개념 이해**(explanation). ★일하는 중이라는 사실만으로 조회 문서가 되지 않는다★ — 일하는 중 필요한 것이 절차 안내일 수 있다.

Diátaxis 의 판정문(직접 확인):

> "is this something someone would turn to **while working**, that is, while actually getting something done, executing a task? Or is it something they'd need once they have **stepped away from the work**, and want to think about it?"

**이 판정문의 적용 범위 = 레퍼런스↔설명 한 쌍뿐이다.** 네 갈래 전체의 분류기가 아니다. 네 갈래를 먼저 가르고, 「물러나 생각하는」 쪽으로 넘어간 문서 안에서 이 문장으로 레퍼런스↔설명을 재확인한다.

- **레퍼런스(조회).** 중립 기술이 명령이다("Neutral description is the key imperative"). 표·목록. "One hardly *reads* reference material; one *consults* it."
- **설명(개념 이해).** 왜·대안·맥락·역사. "It's documentation that it makes sense to read while away from the product itself."
- **경험칙(게이트 아님):** 지루하고 기억에 안 남으면 레퍼런스다 · 목록과 표는 레퍼런스 · 욕조에서 읽을 만하면 설명이다. **검증 가능한 대체 기준 = 「이 문서로 지정한 조회 과업을 끝까지 수행할 수 있나」** — 대표 과업을 미리 적어 두고 실제로 시켜 본다.
- ★**섞으면 양쪽이 다 망가진다**★ — 설명이 레퍼런스에 스며들면 레퍼런스는 곁길에 끊기고, 설명은 제 일을 할 자리를 못 얻는다.

출처: https://diataxis.fr/reference-explanation/ · /reference/ · /explanation/ (CC BY-SA 4.0, Daniele Procida)

## 3. 형식별 규칙

### 3-1. 채팅(사용자 대면 보고)

- **결론 먼저, 그리고 잘라내기는 지정된 경계에서만** — 역피라미드는 중요한 것을 앞세워 **뒤에서부터 덜 중요한 것을 걷어내기 쉽게** 배열하는 것이다. ★어떤 앞부분이든 완결된 문서가 된다는 보장은 아니다★ — 주장 뒤에 붙은 유보 한 줄이 그 앞까지만 읽은 독자의 판단을 곧바로 깨뜨린다.
- **규칙: 자기완결적 도입부를 만든다** — 결론 + 판단에 필요한 전제(유보·경고·적용 조건)를 도입부 안에 함께 넣는다. 유보를 주장 뒤로 미루지 않는다.
- **규칙: 끊어도 되는 지점을 문서가 표시한다** — 아무 줄에서나 끊는 게 아니라 「여기까지 읽으면 판단이 선다」고 표시된 경계에서만 유효하다. 기존 규율의 「어느 줄에서 멈춰도 판단이 서야 한다」는 그 경계 표시를 요구하는 것으로 읽는다.
- **독자가 물을 순서로 배열한다** — 연방 지침 원문: "Think through the questions your audience is likely to ask and then organize your material in the order they'd ask them."
- **문장 하나에 한 생각** — "Express only one idea in each sentence."
- **숫자 상한은 출처를 갈라 쓴다** — 문장 평균 15단어·단락 10줄은 **미 육군 AR 25-50**, 문장 25단어 상한은 **GOV.UK**. ★연방 지침에는 숫자가 없다★(질적 규칙만).
- **능동태로 누가 무엇을 하는지 드러낸다** — "Not 'It must be done,' but 'You must do it.'"

출처: AR 25-50(2020-10-10) para 1-38·1-39 · Federal Plain Language Guidelines(2011) · https://insidegovuk.blog.gov.uk/2014/08/04/sentence-length-why-25-words-is-our-limit/ · Purdue OWL 역피라미드

### 3-2. md(상세 정본)

★**미해결 — 사용자 결정 대기**★ 한 md 파일이 **상세 정본**과 **사용자 대면 설명**을 동시에 맡으라는 요구는 §2 와 충돌한다. markdown 은 형식이지 문서 목적이 아니므로 「md 니까 레퍼런스」로는 해결되지 않는다. 선택지 둘:

- **① 파일을 쪼갠다** — 설명 문서와 레퍼런스 문서를 따로 두고 링크로 잇는다. **비용** = 파일·링크 유지보수 증가, 두 곳이 어긋날 위험, 독자가 두 번 이동.
- **② 한 파일 안에서 라벨로 가른다** — 「설명」 절과 「레퍼런스」 절을 명시 라벨로 나누고, 이것이 Diátaxis 에 대한 **지역 예외**임을 그 파일에 적는다. **비용** = 예외가 관례로 번져 다시 섞일 위험, 판정이 절 경계 유지에 의존.
- ★**둘 중 하나를 세션이 임의로 고르지 않는다.**★ 결정 전까지 기존 파일은 그대로 두고, 이 규칙을 근거로 새 파일을 만들지 않는다.

규칙(결정과 무관하게 유효):

- **austere·중립·표/목록.** 설명은 본문에 섞지 말고 **링크**한다(Diátaxis 반드리프트 규칙) — 위 결정이 ②로 나면 「링크」 자리에 「라벨된 별개 절」이 들어간다.
- **구조는 대상의 구조를 따른다** — 이야기 순서가 아니라 기계의 순서.
- **문장 하나·둘로 끝나는 절은 만들지 않는다**(GitLab CTRT).
- **경험칙(게이트 아님): 스캔되게 쓴다** — "Save your readers' time by writing like a newspaper instead of a novel."(Write the Docs) **검증 가능한 대체 기준 = 대표 조회 과업 하나를 정해 독자가 그것을 끝낼 수 있는지 본다.**

출처: diataxis.fr/reference/ · https://docs.gitlab.com/development/documentation/topic_types/ · https://www.writethedocs.org/guide/writing/docs-principles/

### 3-3. 덱(프레젠테이션)

- **제목 = 완결된 주장문, 본문 = 시각 근거.** Alley 원문: "build your presentations on succinct messages (assertions), rather than phrase topics… you provide **visual evidence rather than bulleted lists**." 그가 지목한 최대 실수 둘 = 구절형 제목은 요점을 전달하지 못한다 · 불릿 목록은 글이 넘쳐 시선이 어디로 갈지 모르게 만든다.
- **경험칙(게이트 아님): 한 그림에 한 문단 분량을 넘기지 않는다** — Google 기술문서 강의 원문: "don't put more than one paragraph's worth of information in a single diagram"(대안 경험칙 = 설명에 불릿 다섯 개가 필요하면 그 그림은 과하다). ★「한 문단 분량」은 정의된 길이가 아니다★ — **검증 가능한 대체 기준 = 그 그림이 답해야 하는 질문 한 개를 미리 적고, 그림만 보고 그 질문에 답할 수 있는지 본다.**
- **캡션을 먼저 쓰고 그 캡션에 맞는 그림을 만든다**(같은 출처).
- **그림은 말로 표현하기 어려운 것에만 쓴다** — "Use images only when they provide useful visual explanations of information that is otherwise difficult to express with words."
- 복잡하면 큰 그림 먼저, 그다음 하위 그림으로 쪼갠다(같은 출처).

**교수설계 실험에서 측정된 효과**(Mayer, *Cambridge Handbook of Multimedia Learning* 중위 d) ★출처 세부 미확인★(판·쪽·실험 대조·성과 지표):

- 불필요한 것 제거(coherence) **0.86**
- 말과 그림을 물리적으로 붙이기(spatial contiguity) **1.10**
- 같은 내용을 두 형태로 동시 제시하지 않기(redundancy) **0.86**
- 독자 속도로 끊어 주기(segmenting) **0.79**
- 제목으로 구조를 신호하기(signaling) **0.41**

★**이 숫자는 멀티미디어 학습 자료에서 측정된 것이다 — 산문·덱 규칙으로의 전이는 우리 적응이지 측정된 것이 아니다.**★ 「과감하게 빼라」의 일반 근거로 이 숫자를 들지 않는다.
★**coherence 는 판단에 필요한 근거·유보를 지우는 허가가 아니다.**★ 충돌하면 §0 이 이긴다(전제 > 간결).

출처: https://www.assertion-evidence.com/ · https://developers.google.com/tech-writing/two/illustrations · https://developers.google.com/style/images · Mayer&Fiorella ch.12 · Mayer&Pilegard ch.13

## 4. 중복 판정 — md 와 덱이 같은 사실을 나르는 것은 위반인가

**같은 사실을 두 산출물이 나른다는 것 자체는 판정 근거가 아니다.** Mayer 의 redundancy 효과가 다루는 대조는 **그림+음성 내레이션 vs 그림+음성 내레이션+화면 텍스트**다 — 같은 말을 동시에 두 경로로 밀어넣는 경우다. ★출처 세부 미확인★(판·쪽)

**판정 축 = 함께 읽히는가.** 둘을 본다: ① **동시 제시**(한 화면·한 자리에서 같이 눈에 들어오나) ② **의도된 읽기 순서**(하나를 보며 다른 하나를 참조하도록 설계돼 있나).

- ★**파일이 갈렸다는 사실은 별개 소비를 증명하지 않는다**★ — 덱을 띄운 채 md 를 참조하는 사용은 흔하다. Write the Docs 의 "Multiple publications may be created from a single source" 는 **발행 관행**을 말한 것이고 인지적 면제가 아니다.
- 반대쪽도 같다 — 한 페이지 안에 있어도 서로 보완하는 두 표현은 그 자체로 해로운 중복이 아니다.
- **함께 쓰인다면:** 두 산출물의 **역할을 갈라 적고**(어느 쪽이 정본이고 어느 쪽이 요약인지), **불필요한 동시 중복만** 걷어낸다.
- **규칙(교체): 「한 페이지에서 그림과 산문이 겹치면 산문을 지운다」는 일괄 규칙을 쓰지 않는다.** 지우는 것은 **의도된 전달 조건에서 없어도 되는 텍스트**뿐이고, 필요한 라벨·설명·접근성 대체 텍스트는 남긴다.

출처: Mayer&Fiorella ch.12(redundancy) · https://www.writethedocs.org/guide/writing/docs-principles/

## 5. 확정적으로 쓰지 말 것 (과청구 방지 목록)

- **「BLUF」라는 약어는 AR 25-50 에 없다** — 본문 표현은 "(bottom line up front)"뿐(수집자 grep 확인).
- **7±2 는 낡았고, 4도 합의가 아니다** — Miller 자신이 그 수를 "pernicious, Pythagorean coincidence"라 했고, Cowan(2001)의 약 4가 이를 대체했으나 최근 추정치는 4~6.4로 갈린다. **숫자를 합의로 인용하지 않는다.**
- **NN/g 의 유명한 수치**(간결 58%·스캔가능 47%·객관 27%·셋 다 124%)는 **51명 단일 연구**다.
- **F-패턴은 관찰된 행동이지 레이아웃 목표가 아니다** — 서식이 그 행동에 영향을 줄 수는 있으나, F-패턴을 관찰한 것만으로 서식 결함이 증명되지 않는다(독자의 목표·동기·훑기 전략도 함께 작용한다). ★「나쁜 서식의 증상이다」로 단정하지 말 것★(NN/g 2017 재검토).
- **「한 장에 한 아이디어」를 Duarte 에게 귀속하지 말 것** — 확인된 것은 HBR 2012 의 3초 glance test 뿐.
- **Minto 원문에 「action titles」는 없다** — 그건 컨설팅 관행이다.
- **자기설명 프롬프트를 무조건 붙이지 말 것** — 효과는 **과업·독자·프롬프트 조건에 따라 갈린다**(보고된 g=0.55, 수학 worked example 메타분석의 음의 조절효과). ★음의 조절효과는 「효과가 0 아래로 뒤집힌다」가 아니라 「이득이 더 작다」일 수 있다★ — **「인지부하 총량이 한도를 넘으면 역전된다」는 인과 문턱은 쓰지 않는다.** ★출처 세부 미확인★(두 통계 모두 이 초안에 식별 가능한 인용이 없다)
- **문서의 목적지로 독자 숙련도를 단정하지 말 것** — 역전을 가르는 것은 관련 사전지식과 제시 조건이다(§1).
- **교수설계 효과크기를 산문·덱 규칙의 일반 근거로 인용하지 말 것** — 전이는 우리 적응이다(§3-3).
- **파일이 갈렸다는 사실을 「중복 아님」의 근거로 쓰지 말 것** — 판정 축은 동시 제시와 의도된 읽기 순서다(§4).
- **재현 가능한 합격 기준이 없는 규칙을 게이트로 쓰지 말 것** — 「지루하면 레퍼런스」·「한 문단 분량」·「스캔되게」는 경험칙이고, 대체 검증은 대표 과업 수행이다(§2 · 3-2 · 3-3).

## 6. 채팅에는 공인 규율이 없다 — 이 절은 차용이 아니라 우리가 정하는 것

조사가 확인한 공백: Diátaxis · Google · Microsoft · GitLab · Kubernetes · Write the Docs · Good Docs 어디에도 **대화형 답변의 레지스터**를 다루는 규율이 없다. 가장 가까운 전이 가능 규칙 = Microsoft 「Get to the point fast」 + Diátaxis 나침반을 질문자의 즉시 필요에 적용(단 §2 대로 네 갈래를 먼저 가른다 — 일하는 중이라는 사실만으로 레퍼런스 레지스터가 되지 않는다).

→ 그래서 3-1 은 **차용이 아니라 우리 결정**이고, 근거는 산문 규칙의 전이라고 명시해 둔다.

## 7. 자산 라이선스 (차용할 때)

- Diátaxis — CC BY-SA 4.0(공유동일조건)
- Google 스타일·기술문서 강의 — CC BY 4.0
- Good Docs 템플릿 — MIT-0(무귀속 사용 가능, 현 위치 = GitLab)
- Kubernetes 문서 — CC BY 4.0
- Write the Docs — **CC BY-NC-SA 4.0**(비영리·동일조건 — 가장 제한적)
- Microsoft 스타일 가이드 — **오픈 아님**(All rights reserved, 읽기만)
- Anthropic `skills/pptx` — **오픈소스 아님**(source-available)
- assertion-evidence 자료 — 라이선스 표기 확인 못 함
---

## 10. 이 규율이 거친 검증 — 무엇이 확인됐고 무엇이 아닌가

**거친 것:** 조사 tier medium — 수집 3갈래 병렬 · grounding(메인이 1차 출처를 직접 열어 인용 대조) · cross-family 적대 리뷰 1회.

**적대 리뷰가 FIX 10건을 냈고 전부 반영했다.** 주요 정정 여섯:

1. 전문가 역전 효과를 「문서 목적지」에 연결한 것은 과장이었다 — 그 효과의 축은 관련 사전지식과 제시 조건이다.
2. Diátaxis 를 「일하는 중 → 레퍼런스」 2분법으로 축소했다 — 실제로는 사분면이고 일하는 중에도 절차 안내가 필요하다.
3. 역피라미드의 「어떤 앞부분도 완결된 문서가 된다」는 보장이 아니다 — 우선순위 배열이고, 주장 뒤에 붙은 유보 한 줄이 그 보장을 깨뜨린다.
4. 중복(redundancy) 설명이 Sweller 전통과 Mayer 전통을 섞었다.
5. 검증 불가 규칙(「지루하면 레퍼런스」·「한 문단 분량」·「스캔되게」)을 게이트처럼 썼다 → 경험칙으로 강등 + 시험 가능한 대체 기준 부여.
6. 규칙이 충돌할 때의 우선순위가 없었다 → §0 신설.

**미확인으로 남은 것(지어내지 않고 표시만 했다):**

- Mayer 효과크기의 판·쪽·실험 대조·성과 지표 — 본문 5곳에 ★출처 세부 미확인★.
- 「한 장에 한 아이디어」의 Duarte 귀속 — 확인된 것은 HBR 2012 의 3초 glance test 뿐.
- Tufte 『The Cognitive Style of PowerPoint』 1차 페이지 미열람.
- marp·slidev 계열 repo 다섯의 내용·라이선스 미열람(이름만 검색 색인에서 확인).
- assertion-evidence 자료의 라이선스 표기 미확인.

## 11. 출처 — 이미 확인한 것 (재조사 금지)

**덱·주장 구조**
- assertion-evidence (Michael Alley · Penn State Leonhard Center · NSF Grant 1323230) — https://www.assertion-evidence.com/
- 실증: Garner & Alley, "How the design of presentation slides affects audience comprehension", *International Journal of Engineering Education* 29(6), 2013 — https://pure.psu.edu/en/publications/how-the-design-of-presentation-slides-affects-audience-comprehens/ · 전문 PDF https://writing.engr.psu.edu/ae_comprehension.pdf
- Duarte, glance test, HBR 2012-10-22 — https://hbr.org/2012/10/do-your-slides-pass-the-glance-test
- Reynolds, signal-to-noise·slideument — https://www.garrreynolds.com/design-tips  (주의: `/preso-tips/design` 는 404)

**문서 종류 판정**
- Diátaxis (Daniele Procida, CC BY-SA 4.0) — https://diataxis.fr/reference-explanation/ · /reference/ · /explanation/ · /compass/
- GitLab CTRT 토픽 타입 — https://docs.gitlab.com/development/documentation/topic_types/
- Kubernetes 페이지 콘텐츠 타입(Diátaxis 참조 명시) — https://kubernetes.io/docs/contribute/style/page-content-types/
- Write the Docs 문서 원칙(CC BY-NC-SA 4.0 — 가장 제한적) — https://www.writethedocs.org/guide/writing/docs-principles/
- The Good Docs Project 템플릿(MIT-0, 현 위치 GitLab) — https://gitlab.com/tgdp/templates

**산문·대화 구조**
- 미 육군 AR 25-50 (2020-10-10) para 1-38·1-39 — https://armypubs.army.mil/epubs/DR_pubs/DR_a/ARN42124-AR_25-50-007-WEB-13.pdf  ★"BLUF" 약어는 본문에 없다(grep 확인)★
- Plain Writing Act of 2010 (PL 111-274) — https://www.govinfo.gov/content/pkg/PLAW-111publ274/html/PLAW-111publ274.htm
- Federal Plain Language Guidelines (2011) — https://raw.githubusercontent.com/GSA/plainlanguage.gov/master/media/FederalPLGuidelines.pdf  ★`plainlanguage.gov` 는 전부 https://digital.gov/guides/plain-language 로 리다이렉트되고 그쪽은 훨씬 얇다★
- GOV.UK 문장 25단어 상한 — https://insidegovuk.blog.gov.uk/2014/08/04/sentence-length-why-25-words-is-our-limit/
- 역피라미드 (Purdue OWL) — https://owl.purdue.edu/owl/subject_specific_writing/journalism_and_journalistic_writing/the_inverted_pyramid.html
- Gopen & Swan, "The Science of Scientific Writing", *American Scientist* 1990 — https://www.gatsby.ucl.ac.uk/~pel/misc/gopen_swan.pdf
- Haviland & Clark 1974, given-new contract, DOI 10.1016/S0022-5371(74)80003-4 (제목·저자·연도만 확인, 본문 미열람)

**그림 규칙**
- Google 기술문서 강의 Illustrating (CC BY 4.0) — https://developers.google.com/tech-writing/two/illustrations
- Google 개발자 문서 스타일 가이드 — https://developers.google.com/style/images · /style/sentence-structure · /style/headings · /style/highlights
- Microsoft Writing Style Guide (★오픈 아님 — All rights reserved★) — https://learn.microsoft.com/en-us/style-guide/top-10-tips-style-voice · /scannable-content/

**측정된 효과**
- Mayer & Fiorella ch.12 / Mayer & Pilegard ch.13, *Cambridge Handbook of Multimedia Learning* — https://www.cambridge.org/core/books/abs/cambridge-handbook-of-multimedia-learning/principles-for-reducing-extraneous-processing-in-multimedia-learning-coherence-signaling-redundancy-spatial-contiguity-and-temporal-contiguity-principles/CD5B7AE1279A9AB81F8EEBB53DBEC86E
- Sweller·van Merriënboer·Paas, *Educational Psychology Review* 31:261–292, 2019 — https://leadinglearner.me/wp-content/uploads/2019/02/sweller2019_article_cognitivearchitectureandinstru.pdf
- Kalyuga·Ayres·Chandler·Sweller, 전문가 역전, *Educational Psychologist* 38(1):23–31, 2003 — https://mrbartonmaths.com/resourcesnew/8.%20Research/Explicit%20Instruction/The%20Expertise%20Reversal%20Effect.pdf
- Dunlosky et al. 2013, 학습 기법 유용성 등급 — https://www.whz.de/fileadmin/lehre/hochschuldidaktik/docs/dunloskiimprovingstudentlearning.pdf
- Cowan 2001, 약 4 (Miller 7±2 대체) — https://www.cambridge.org/core/services/aop-cambridge-core/content/view/44023F1147D4A1D44BDC0AD226838496/S0140525X01003922a.pdf/the-magical-number-4-in-short-term-memory-a-reconsideration-of-mental-storage-capacity.pdf
- 최근 추정치 4~6.4로 갈림 — https://journalofcognition.org/articles/10.5334/joc.387
- NN/g: 79% 스캔 https://www.nngroup.com/articles/how-users-read-on-the-web/ · 수치 출처(51명 연구) https://www.nngroup.com/articles/concise-scannable-and-objective-how-to-write-for-the-web/ · F-패턴 2017 재검토 https://www.nngroup.com/articles/f-shaped-pattern-reading-web-content/

**오픈소스 자산(참고)**
- `zarazhangrui/frontend-slides` (MIT — 밀도 두 모드가 유일한 차용 가치) — https://github.com/zarazhangrui/frontend-slides
- `anthropics/skills` (★`skills/pptx` 는 source-available, 오픈소스 아님★) — https://github.com/anthropics/skills
- `1weiho/open-slide` (MIT, 프레임워크) — https://github.com/1weiho/open-slide
- 큐레이션 인덱스 (CC BY 4.0) — https://github.com/danielrosehill/AI-Presentation-Builders-Index

## 12. 이번 실물 — 규율을 적용해 만든 후보 3안 (참고)

같은 사실·같은 팔레트로 고정하고 구조 규칙만 달리해 셋을 만들었다.

- **A · 주장형**(assertion-evidence) — 제목이 완결된 주장문, 본문은 그림 하나. 10장.
- **B · 한 화면** — 스크롤 없이 표 중심 참조 시트. 그림 없음.
- **C · 질문·답형** — 질문 7개가 제목, 답이 한 문장 먼저. 연방 지침의 「물을 순서로 배열」과 일치.

**사용자 선택 = A(주장형).** 단 「A 도 아직 글이 많다」는 평이므로 장당 문장 수를 더 줄이고 근거 앵커·경고 박스는 md 로 내리는 방향.

세 파일은 세션 스크래치패드에 있다(임시 경로 — 필요하면 옮겨 둘 것):
`case-a.html` · `case-b.html` · `case-c.html`

## 13. 팩토리에 남기는 판단 요청

- `explain` 이 **하나의 스킬**로 덱·문서를 다 덮는 게 맞나, 아니면 형식별로 쪼개는 게 맞나. 지금 초안은 「설명」을 축으로 하나로 묶었다(사용자와 합의). 근거 = 실패한 것은 슬라이드 기술이 아니라 전달 판단이고, 그 판단이 채팅·문서·덱에 똑같이 걸린다.
- 트리거 정확도(오발동·미발동) 검증은 팩토리 관례를 따른다.
