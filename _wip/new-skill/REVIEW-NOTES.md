# new-skill 개선 검토 (research 재설계 결을 "새 스킬 기본값"으로 심기)

> 검토 대상: `_wip/new-skill/{SKILL.md, references/flow.md, references/formats/*.template.md, feedback.md}`
> 품질 기준(설계 DNA 실물): `_wip/research/{SKILL.md, references/flow.md, REDESIGN-SPEC-v3.md}`
> 규약: `core/claude-global-shared/rules/markdown-format.md`, `apps/engram-dashboard/CLAUDE.md`
> 산출: **개선 제안**(rewrite 아님).

## 1. 현재 스킬 요약

new-skill은 **새 스킬을 만드는 메타-스킬**이다. 흐름은 `발견 인터뷰(§1) → 인터페이스 설계+1회 승인(§2) → 파일 생성(§3) → 후속 안내(§4)`(`_wip/new-skill/references/flow.md:12-153`). 위치(`--global/--project`)는 사용자 확인 전 자동 확정 금지(`SKILL.md:15`), 파일 생성 전 `formats/*.template.md` 필수 Read(`flow.md:70-77`), 생성물은 `SKILL.md + references/flow.md (+선택 bindings/study-notes)`.

**자기 몸(new-skill 자신)의 DNA 준수는 양호하다:** 정직한 `## ⚠️ 검증 상태`("신규 스킬 — 한 번도 실행되지 않았다", `SKILL.md:45-47`), 자기개선 feedback 루프 + 실제 항목 1건(`feedback.md`), flow.md Read 경고=SSOT 짝(`SKILL.md:34`), bindings 개념 보유(`SKILL.md:36-43`).

**문제는 "메타" 층이다** — new-skill이 *만들어내는 모든 미래 스킬*에 research가 몸으로 보여준 DNA를 **디폴트로 심어주는가**? 여기서 갈린다. 아래는 그 관점(가장 중요) 중심.

## 2. 갭·이슈 (우선순위·심각도) — "설계 DNA를 디폴트로 강제하는가"

### G1. 역할→모델 배정표(Fable-ready) 미전파 — 템플릿이 **안티패턴을 가르침** [심각도: 큼]
research DNA의 1급 원칙: "본문은 역할 슬롯으로만 말하고 모델명은 배정표 한 곳에만 둔다 — 신모델 나오면 배정표만 교체"(`_wip/research/SKILL.md:20`, `_wip/research/references/flow.md:18-33`). 이건 프로젝트 #1 불변식("특정 모델에 코드를 묶지 않는다", `CLAUDE.md ★아키텍처 원칙`)의 스킬판이다.
그런데 `flow.template.md:24`는 스폰 블록 예시에서 **`- **<역국명> = opus/sonnet 서브에이전트**`** 로 모델명을 *본문에 인라인 하드코딩*한다. research가 폐기한 바로 그 모양이다. 두 템플릿 어디에도 "역할→모델 배정표" 섹션이 없다. → new-skill이 만드는 모든 멀티에이전트 스킬은 모델을 flow 본문에 못 박아 교체성 불변을 깨는 방향으로 태어난다. **가장 고레버리지 갭**(미래 스킬 전부가 상속).

### G2. 리서치 게이트(§1.5) 부재 — **자기 feedback.md가 이미 지목** [심각도: 큼]
`_wip/new-skill/feedback.md:5`: "비자명 스킬(설계 갈림길·OSS 선례가 결과를 바꾸는 경우)은 §2 설계 전에 'OSS 조사 → 근거 있는 선택지 제시'가 들어가야 한다 … 메인이 사용자에게 설계 질문을 cold로 던지는 안티패턴." research가 흡수한 "설계-결정 모드"(`_wip/research/SKILL.md:25`, `_wip/research/references/flow.md:152-157`)가 정확히 이 자리를 메운다. 현 flow는 인터뷰→설계로 바로 가서 근거 없는 설계 선택을 유발. **자기식별 결함 = 강한 증거.** 개선: §1.5 "비자명·설계갈림길이면 `/research`(설계-결정 모드) 위임 → 선택지 회수 → §2 브리핑 입력." 이건 CLAUDE.md "순서 불변: 컨설/선택지 → 사용자 결정"과도 정합.

### G3. `/review trd`가 강제 아닌 "권장" — 자기 스킬을 자기가 통과 [심각도: 중~큼]
`flow.md:150-152`·`SKILL.md`는 `/review trd`를 §4 후속의 "강력 권장"으로만 둔다. 그러나 flow.md 자신이 "스킬 파일 작성은 … 기본적으로 비자명하다"(`flow.md:144`)고 인정하고, CLAUDE.md는 비자명 변경에 "리뷰 스킵 절대 금지" + "생산자 ≠ 리뷰어(fresh)"를 못 박는다. research도 medium+에서 cross-family 적대 리뷰를 **하드 게이트**로 건다(`_wip/research/references/flow.md:92-96`). → new-skill이 방금 초안한 스킬을 적대 검증 없이 통과시킬 여지. 개선: 비자명 스킬은 `/review trd`를 **핵심 설계/가드레일의 게이트**로 승격(권장→강제), "생산자(new-skill)≠리뷰어(fresh, cross-family)" 명시.

### G4. 정량 SSOT 분리 미전파 (rot 방지 핵심 패턴) [심각도: 중]
research의 최다 반복 rot방지 장치: "정량(수·검색량·effort)은 flow의 정본 표 **한 곳에만**, 다른 절은 tier 이름으로만 참조"(`_wip/research/SKILL.md:28`, `_wip/research/references/flow.md:37-53`). new-skill 템플릿은 이 분업을 안 가르친다 — `SKILL.template.md:21-28`의 1차축 표는 "언제 | 깊이/범위"를 **한 표에 혼재**시켜, SKILL(언제/무엇) ↔ flow 정본표(정량)의 SSOT 분리를 지시하지 않는다. 생성 스킬이 숫자를 SKILL·flow 양쪽에 적어 rot할 소지.

### G5. 하우스 마크다운 핀셋 규약 미참조 [심각도: 중]
`markdown-format.md`(핀셋 패턴 — 룰=prose, 다른 결=XML `<examples>/<scope>` 등)를 new-skill의 **템플릿·체크리스트가 아예 언급하지 않는다**(grep 확인: `_wip/new-skill` 전체에 `markdown-format`/`핀셋` 0건). flow.md §3의 "SKILL.md 작성 규칙"(`flow.md:111-129`)·"flow.md 작성 규칙"(`flow.md:130-140`)에도 없음. → 생성 스킬이 하우스 마크다운으로 수렴하지 않는다. task가 직접 물은 "new-skill이 새 스킬 초안에 이걸 강제하나?"의 답 = **아니오**.

### G6. study-notes/ 위치 = SSOT 심링크 안티패턴 (research가 교정한 것을 회귀) [심각도: 중]
`flow.md:99-107`은 학습용 변형에서 **`skills/<스킬명>/study-notes/`** 로 스킬 폴더 *안*에 런타임 노트를 두라 가르친다. research는 정반대 교훈을 박제: "저장 위치 = 스킬 폴더가 아니라 프로젝트/데이터 경로 — 이 디렉터리는 SSOT에서 심링크로 배포된 읽기전용이라 스킬 폴더에 쓰면 실패하거나 SSOT를 오염"(`_wip/research/SKILL.md:52`). new-skill 템플릿이 이 하드윈 교훈을 미전파→회귀. 개선: "런 로그·학습 노트·usage-log는 스킬 폴더가 아니라 프로젝트 데이터 경로(프로젝트 통합 지점이 지정)"를 기본으로.

### G7. calibration/evidence-grounded "가설 표기" 얕음 [심각도: 경미]
`SKILL.template.md:42-47`의 ⚠️ 섹션은 "단단함/약함·미검증" 2분만 가르친다. research의 더 깊은 캘리브레이션 — "근거 있는 가설"로 명시, "라이브 1회 적출 = 존재 증거지 효과 크기 증거 아님"(`_wip/research/SKILL.md:20,58`) — 은 씨앗만 있음. 멀티모델·실증 주장 스킬엔 한 줄 가이드 추가 여지(과도하면 생략 가능).

### G8. mode-aware(자율 vs 대화) 미언급 [심각도: 경미]
new-skill §1 인터뷰·§2 위치확인은 자율("진행 쭉해") 모드에서도 사용자 결정을 강제해야 하는지 무언급. 실제로는 강제가 맞다(위치=사용자 결정, `CLAUDE.md`)지만 flow에 안 박혀 있어, 자율 모드에서 메인이 임의 확정할 여지. research의 mode-aware 에스컬레이션(`_wip/research/references/flow.md:136-143`)만큼 정교할 필요는 없으나, "인터뷰·위치확정은 자율 모드에서도 블록" 한 줄 권장.

**잘 심긴 축(칭찬 — 유지):** 정직한 `## ⚠️ 검증 상태`는 필수 섹션으로 강제됨(`SKILL.template.md:42-47`, `flow.md:118`) · 자기개선 feedback + _shared 포인터 + "빈 feedback.md 사전생성 금지"(`SKILL.template.md:49-55`, `flow.md:128,163`) · flow.md Read 경고=SSOT 짝(`flow.md:140`) · bindings 개념·"실 명령은 bindings에"(`flow.template.md:66`). 이 4개는 research DNA를 이미 디폴트로 전파 중.

## 3. 제안 재설계 thesis (한 줄 + 근거)

**thesis:** new-skill은 "파일 골격 생성기"에서 **"research 설계 DNA를 모든 미래 스킬에 자동 상속시키는 전파 장치"**로 격상해야 한다 — 정직한 ⚠️·feedback·bindings는 이미 전파하나, **역할→모델 배정표(G1)·리서치 게이트(G2)·강제 /review trd(G3)·정량 SSOT(G4)·마크다운 핀셋(G5)·런타임 산출 위치(G6)** 6개 핵심 DNA가 템플릿/체크리스트에서 빠졌거나(대부분) 안티패턴으로 역행(G1·G6)한다.
**근거:** 이 6개는 research 재설계가 *가장 비싸게 배운* 불변들이고(모델 교체성·rot방지·생산자≠리뷰어·SSOT 심링크 제약), new-skill의 레버리지는 "1회 실행 품질"이 아니라 "생성하는 스킬 N개 전부의 기본값"이라 미전파 비용이 N배로 복리된다.

## 4. 개선 항목 표

| # | 무엇 | 왜 | 사용자 결정? | research 전이? | 심각도 |
|---|---|---|---|---|---|
| G1 | flow.template.md 스폰 블록의 인라인 `opus/sonnet` 제거 → **역할 슬롯 + "역할→모델 배정표" 섹션**을 멀티에이전트 스킬 기본 산출로 | 모델 교체성(#1 불변) 위반 방향으로 스킬 생성. 신모델(Fable) 대응 불가 | 아니오(불변 정합, 메인 보고) | `research/references/flow.md:18-33` 배정표 통째 | 큼 |
| G2 | §1.5 리서치 게이트 — 비자명·설계갈림길이면 `/research`(설계-결정 모드) 위임 후 §2 입력 | cold 설계질문 안티패턴(자기 feedback.md 지목). "순서 불변" 정합 | 예(선택지→사용자) | `research/SKILL.md:25`, `flow.md:152-157` | 큼 |
| G3 | 비자명 스킬 `/review trd`를 권장→**강제 게이트**(핵심설계·가드레일), "생산자≠리뷰어(cross-family fresh)" 명시 | 스킬정의=비자명변경, "리뷰 스킵 금지"·self-통과 방지 | 아니오(규약 강제) | `research/references/flow.md:92-96,164` | 중~큼 |
| G4 | 템플릿에 "정량은 flow 정본표 한 곳, SKILL·본문은 이름 참조" 지시 + 1차축 표를 언제/무엇 ↔ 정량으로 분리 | 숫자 양쪽 기재→rot | 아니오 | `research/SKILL.md:28`, `flow.md:37-53` | 중 |
| G5 | flow.md 작성 규칙에 `markdown-format.md` 핀셋 규약 참조 강제(예시·정체성·권한만 XML) | 생성 스킬이 하우스 마크다운 미수렴 | 아니오 | `markdown-format.md`(전이 대상=규약) | 중 |
| G6 | study-notes/·usage-log·런로그를 **스킬 폴더 밖 프로젝트 데이터 경로**로 (SSOT 심링크 읽기전용) | research가 교정한 교훈 회귀 | 예(경로=프로젝트 통합) | `research/SKILL.md:52` | 중 |
| G7 | ⚠️ 템플릿에 "근거 있는 가설/라이브1회=존재증거" 한 줄(멀티모델·실증 스킬) | 과청구 경계 얕음 | 아니오 | `research/SKILL.md:20,58` | 경미 |
| G8 | 인터뷰·위치확정은 자율 모드에서도 블록 명시(간이 mode-aware) | 자율 모드 임의 확정 방지 | 아니오 | `research/references/flow.md:136-143`(축소판) | 경미 |

## 5. 정직 노트 (과청구 경계 · new-skill 고유 제약)

- **research DNA를 통째로 이식하면 안 된다(오버엔지니어링 경계).** research의 grounding·cross-family 수집·강도 사다리·mode-aware 에스컬레이션은 *사실조사 도메인*에 특화된 것이다. new-skill이 심을 것은 **원칙의 형태**(역할슬롯+배정표, 정량SSOT, 생산자≠리뷰어, 정직⚠️, 핀셋)지 research의 도메인 로직이 아니다. G1·G4는 "멀티에이전트 스킬일 때만", G7은 "실증 주장 스킬일 때만" 조건부로 넣어야 단일-선형 스킬에 군더더기가 안 붙는다.
- **new-skill 고유 제약 — 대칭 리뷰가 아니다.** research의 cross-family는 *같은 산출을 두 family가*지만, new-skill의 검증은 "초안 → 다른 스킬(`/review trd`)로 넘김"이라 대칭이 아니라 **위임형**이다. 따라서 G3은 "new-skill 안에 리뷰어를 스폰"이 아니라 "review 스킬로 넘기는 게이트를 강제"가 맞다(중복 파이프라인 신설 금지).
- **자기 몸은 대체로 건강하다.** §2의 갭은 "new-skill이 나쁘다"가 아니라 "전파 커버리지가 부분적"이다 — 정직⚠️·feedback·bindings·flow SSOT 짝은 이미 잘 전파 중(§2 말미). 재설계는 *추가 전파*지 재작성이 아니다.
- **G2 vs 오버헤드 트레이드오프(사용자 판단).** 모든 스킬 생성에 `/research`를 끼우면 사소한 스킬엔 과함 — "비자명·설계갈림길" 게이트 판정 기준을 어디에 둘지는 §스킬화 가치 기준과 묶어 **사용자 결정**으로 남기는 게 맞다(임의 확정 금지).
- 이 검토는 **정적 대조**(파일 읽기)만 근거다 — new-skill 실행 실측은 0(자기 ⚠️도 미실행 인정). 갭의 *실사용 마찰* 크기는 실행 후 feedback.md로 재검증돼야 한다.

---
**전체 판정: 큼** — 자기 몸은 양호하나, 메타-스킬의 본질(미래 스킬 전부에 DNA 상속)에서 6개 핵심 DNA가 미전파(G4·G5) 또는 안티패턴 역행(G1·G6)이라 레버리지가 크다. G1(인라인 모델)은 프로젝트 #1 불변을 깨는 방향이라 단독으로도 재설계 근거.
