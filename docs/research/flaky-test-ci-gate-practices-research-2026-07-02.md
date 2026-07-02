# Flaky·타이밍 민감 테스트의 CI 게이트 관행 — 조사 보고서

> **상태:** medium 완주 (주계열 수집 3갈래 + 메인 grounding + cross-family 적대 리뷰 1회)
> **날짜:** 2026-07-02 · **방법:** `/research` medium — 수집자(sonnet)×3 병렬(검색 각 8~12회) → 메인 grounding(load-bearing 전수, 인용문 기반 함의 판정) → Codex 적대 리뷰(effort=medium·레벨 2~3·web_search)
> **확신도 범례:** 확실 = 독립 교차확증 / 가능성 높음 = grounding 지지 + 리뷰 통과(단일 출처) / 불확실 = 미지지·보류 / contested = 반증 존재
> **질문:** GUI/E2E 실측 1회 통과는 race-free를 보장하지 못한다 — 업계 CI 게이트는 flaky·타이밍 민감 테스트를 어떻게 다루나.
> **용도(발견 체인):** qa 스킬 재설계의 "1회 PASS ≠ race-free" 정직 note 근거. `docs/reference/debugging-conventions.md`(flaky 발화 지점)와 연결.

## 핵심 결론

1. **"1회 통과는 신뢰 근거가 아니다"는 업계 정설이다.** 비결정 테스트는 실패해도 버그인지 모르고 통과해도 결함 부재를 증명하지 않는다(Fowler: "useless + virulent infection"). race는 인터리빙의 확률적 발현이라 단일 실행은 가능한 스케줄 중 하나만 본다. [가능성 높음]
   - 단 통계적 뉘앙스: "1회 = 무정보"는 수사이고, 정보량은 실패확률 사전분포에 달렸다 — 희귀 flake(p≈1e-4)는 신뢰 있는 검출에 수만 회 실행이 필요할 수 있다(Wilson interval 논의, arXiv 2512.18088). **즉 "N회 돌리면 안전" 같은 마법 숫자도 없다.** [가능성 높음]
2. **탐지 표준 = rerun 기반.** 같은 코드에서 재실행해 pass/fail이 갈리면 flaky. Microsoft(fail→retry-pass 1회면 판정), Develocity(빌드 내 + 빌드 간 2축), CircleCI(같은 커밋 14일 윈도우). 고급형은 통계/ML(Atlassian Bayesian 다중신호, Fitbit autoencoder)과 무재실행 탐지(DeFlaker 커버리지 차분, 96% 탐지·오탐 1.5%). [가능성 높음]
3. **게이트 표준 = presubmit(차단) / postsubmit(관찰) 분리 + quarantine 자동화.** flaky 판정 테스트는 머지 게이트에서 빼되 계속 실행하며 관찰한다 — Microsoft는 "항상 실행하되 결과만 suppress", Dropbox는 "presubmit 무시·postsubmit 계속", Buildkite는 quarantine=soft-fail. 격리엔 수리 의무가 따른다(Fowler: 최대 8개·1주 캡 / Microsoft: flaky 버그 10개+ 누적 시 PR 차단 같은 패널티). [가능성 높음]
4. **retry는 "판정 도구"지 "통과 도구"가 아니다.** Dropbox는 판정 목적으로만 최대 10회 재시도. 통과 목적 retry는 신호를 파괴하고 진짜 버그를 은폐한다(Fowler). 도구들의 자동 retry 기본값은 0이고, 설정해도 2~3회 수준의 낮은 캡이 예시로 제시된다("2~3회가 표준 권고"라고까지 말하는 건 과장 — 리뷰 보정). [가능성 높음]
5. **예방 표준(타이밍/race) = 조건부 대기 + 결정성 확보.** 고정 sleep 금지 → Selenium explicit wait / Playwright auto-wait(actionability 5체크: Visible·Stable·Receives Events·Enabled·Editable) / Cypress retry-ability. 시스템 클록은 wrap해서 fake timer로 대체(단 Promise microtask는 별도 처리), 테스트 간 상태 공유 제거, 외부 의존은 hermetic 격리. [가능성 높음 — 프레임워크 권고 자체는 공식 docs 확인]
6. **⚠ 테스트 단위 quarantine의 숨은 비용.** flaky 테스트가 진짜 회귀 결함의 1/3 이상을 드러낸다는 연구가 있다 — flaky 예측기 정밀도 99.2%여도 회귀 결함의 ~76%를 놓칠 수 있음(Chromium CI 연구, arXiv 2302.10594). "flaky = 무조건 격리"가 아니라 **실패 인스턴스 분류**(이번 실패가 flake인지 진짜인지)가 더 정확한 프레임. [가능성 높음 — 리뷰어 누락 탐침 발견]

## 발견 상세

### A. 탐지·정량화
| 발견 | 확신도 | 출처 |
|---|---|---|
| Microsoft CloudBuild/CloudTest: fail→retry-pass 1회면 flaky 판정 → 자동 quarantine + 버그 자동 생성, close 시 자동 해제. ~49,000개 식별·100+ 팀 | 가능성 높음 (원문 직독) | [MS devblogs](https://devblogs.microsoft.com/engineering-at-microsoft/improving-developer-productivity-via-flaky-test-management/) |
| Google TAP: 실패 테스트 10회 재실행, 1회라도 통과 시 flaky — **단 이 수치는 Luo FSE 2014가 John Micco 개인 커뮤니케이션으로 인용한 전언**(1차 문서 미확인) | 가능성 높음 (전언 주의) | [Luo et al. FSE 2014](https://mir.cs.illinois.edu/marinov/publications/LuoETAL14FlakyTestsAnalysis.pdf) |
| Develocity: within-build(fail→pass) + cross-build(같은 입력 pass/fail 혼재) 2축 탐지 | 가능성 높음 (공식 docs) | [Develocity docs](https://docs.gradle.com/develocity/flaky-test-detection/) |
| CircleCI Test Insights: 같은 커밋 14일 윈도우 pass/fail 혼재 → flaky | 가능성 높음 | [CircleCI blog](https://circleci.com/blog/introducing-test-insights-with-flaky-test-detection/) |
| Atlassian Flakinator: Bayesian + 다중신호(duration 변동·환경·retry 빈도) 0~1 점수. master 실패 중 flaky 기인 21%(FE)/15%(BE) | 가능성 높음 (원문 직독) | [Atlassian blog](https://www.atlassian.com/blog/atlassian-engineering/taming-test-flakiness-how-we-built-a-scalable-tool-to-detect-and-manage-flaky-tests) |
| DeFlaker: 커버리지 차분으로 재실행 없이 탐지 — rerun 대비 96% 탐지, 오탐 1.5% | 가능성 높음 | [DeFlaker (ICSE 2018)](https://www.cs.cornell.edu/~legunsen/pubs/BellETAL18DeFlaker.pdf) |
| Chromium Findit: 원인 커밋 추적에 최대 400회 재실행(탐지용 아님) | 가능성 높음 | [Findit CAT](https://sites.google.com/chromium.org/cat/findit) |

### B. 규모 수치
| 발견 | 확신도 | 출처 |
|---|---|---|
| Google(2016): 전체 테스트 *실행*의 ~1.5%가 flaky 결과 · 전체 *테스트*의 ~16%("1 in 7")가 어느 정도 flaky. ~~30일 윈도우~~(원문에 없음 — 리뷰 보정으로 제거) | 가능성 높음 | [Google Testing Blog 2016](https://testing.googleblog.com/2016/05/flaky-tests-at-google-and-how-we.html) |
| Google(2016): pass→fail 전이의 84%가 flaky(진짜 회귀 아님) — 수집 단계에선 "인용 세탁 의심"이었으나 **적대 리뷰가 원문 실재를 확인** | 가능성 높음 | 위와 동일 |
| Luo et al. FSE 2014 원인 분포(51개 프로젝트·201 fix-commit): **원수치 async wait 74건 / concurrency 32 / order dependency 19 (201 기준 ≈37%/16%/9%)**. 널리 인용되는 45%/20%/12%는 분류 가능분 기준으로 보임 — 분모 주의 (리뷰 보정) | 가능성 높음 | [Luo et al.](https://mir.cs.illinois.edu/marinov/publications/LuoETAL14FlakyTestsAnalysis.pdf) |
| GitHub Actions(1,960개 Java OSS): 빌드 3.2%가 재실행, 그중 67.73%가 flaky 행동 — 단 "flaky *builds*" 연구(테스트 단위 아님) | 가능성 높음 | [arXiv 2602.02307](https://arxiv.org/abs/2602.02307) |
| flaky의 75%는 첫 커밋부터 flaky (Lam et al. OOPSLA 2020) | 가능성 높음 (원문 미독·요약 기반) | [OOPSLA 2020](https://dl.acm.org/doi/10.1145/3428270) |

### C. 게이트 정책
| 발견 | 확신도 | 출처 |
|---|---|---|
| Fowler: 비결정 테스트 = "useless + virulent infection" · bare sleep 금지 · quarantine 최대 8개 또는 1주 캡 · quarantine은 메인 파이프라인 밖으로 | 확실 (원문 다중 확인 + 리뷰 통과) | [martinfowler.com](https://martinfowler.com/articles/nonDeterminism.html) |
| Meta: PFS(Probabilistic Flakiness Score) 기반 — 열화 테스트는 change-based testing 대상에서 제외 | 가능성 높음 (원문 직독) | [engineering.fb](https://engineering.fb.com/2020/12/10/developer-tools/probabilistic-flakiness/) |
| Dropbox Athena: postsubmit 수시간 내 2회+ fail → "noisy" → presubmit 무시·postsubmit 계속. 판정용 재시도 최대 10회(통과용 아님) | 가능성 높음 (원문 직독) | [dropbox.tech](https://dropbox.tech/infrastructure/athena-our-automated-build-health-management-system) |
| Buildkite: quarantine = soft-fail(파이프라인 결과에 무영향) | 가능성 높음 (공식 docs) | [Buildkite docs](https://buildkite.com/docs/test-engine/test-state-and-quarantine) |
| Atlassian: flaky 감지 → Jira 티켓 자동 생성 + 오너십 할당, 일정 기간 healthy면 자동 재입장 | 가능성 높음 (원문 직독) | Atlassian blog (위) |
| Spotify "Master Guardian": fail→retrigger-pass면 flaky → 티켓, open 동안 pre-merge skip | **불확실** — 2차 정리만, 1차 출처 미확인(리뷰어도 기권) | [2차: engineering.atspotify 정리글](https://engineering.atspotify.com/2019/11/test-flakiness-methods-for-identifying-and-dealing-with-flaky-tests) |
| GitHub Actions: 네이티브 flaky 자동 격리 기능은 문서상 부재(step retry·수동 re-run만) — 부재 *증명*은 아님 | 가능성 높음 | [GH docs](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/re-run-workflows-and-jobs) |

### D. 예방 (타이밍/race)
| 발견 | 확신도 | 출처 |
|---|---|---|
| Selenium: sleep 기피 → explicit wait(조건 명시), implicit+explicit 혼용 금지("unpredictable wait times") | 확실 (공식 docs 직독 + 리뷰 통과) | [Selenium Waits](https://www.selenium.dev/documentation/webdriver/waits/) |
| Playwright: 모든 액션 전 auto-wait actionability 5체크(Visible·Stable[2프레임 bbox 동일]·Receives Events·Enabled·Editable) + auto-retrying assertions | 확실 (공식 docs) | [Playwright Actionability](https://playwright.dev/docs/actionability) |
| Cypress: retry-ability — 조건 달성 즉시 진행, 하드코딩 대기 제거가 1차 방어선 | 확실 (공식 docs) | [Cypress Retry-ability](https://docs.cypress.io/app/core-concepts/retry-ability) |
| 시스템 클록 wrap + fake timer(macrotask 제어, Promise microtask는 async 변형 필요) · 상태 공유 제거 · hermetic 격리 | 가능성 높음 | [Jest](https://jestjs.io/docs/timer-mocks) · [Sinon](https://sinonjs.org/releases/latest/fake-timers/) · Fowler(위) · [Hermetic Servers](https://testing.googleblog.com/2012/10/hermetic-servers.html) |

## 적대 리뷰 결과 (cross-family 실효 — 이번 실행에서 잡은 것)

리뷰어(Codex, 레벨 2~3)가 수집·합성의 오류를 반증 출처와 함께 보정:
1. **방향 반전:** "84% 전이" 수치를 우리는 인용 세탁으로 의심(불확실 태그) → 리뷰어가 Google 원문에서 실재 확인(지지로 승격).
2. **분모 보정:** Luo 45%/20%/12%는 분류 가능분 기준 — 원수치는 74/32/19 of 201(≈37%/16%/9%).
3. **과정밀 제거:** Google 16%에 붙어 다니는 "30일 윈도우"는 원문에 없음.
4. **출처 격 강등:** "TAP 10회"는 Google 1차 문서가 아니라 논문의 개인 커뮤니케이션 인용(전언).
5. **과장 완화:** "retry 2~3회 = 지배적 권고" → 도구 기본값은 0, 2~3은 설정 예시/상한.

## engram 함의 (qa 스킬 재설계 근거)

- **cdp 1회 실측 PASS = "기능이 존재하고 이 인터리빙에서 동작함" 증명이지 race-free 증명이 아니다** — qa 스킬 정직 note의 근거가 이 보고서. 마법의 "N회 통과 = 안전" 숫자도 없다(사전확률 의존).
- 우리 규모(1인·데스크톱 앱)에 대기업식 quarantine 인프라는 과함. 현실적 이식: **① 타이밍 민감 테스트는 조건부 대기로 작성(고정 sleep 금지 — ADR-0038 5ms→50ms 사례와 정합) ② flaky 의심 시 같은 코드에서 N회 재실행해 판정(통과 목적 retry 금지) ③ 판정된 flaky는 게이트에서 빼되 조용히 지우지 않고 기록**.
- 실패 인스턴스 분류 관점(결론 6)은 향후 qa가 "이번 실패가 flake인가"를 물을 때의 프레임.

## 쟁점/한계

- Spotify Master Guardian: 1차 출처 미확인 → 불확실 유지(수집자·리뷰어 모두 기권).
- TAP 10회: 전언 기반 — Google 1차 문서 확인 실패(수집자 PDF 접근 불가 반복).
- 링크 생사: 수집·리뷰 중 실제 fetch로 대부분 확인, 별도 전수 pass는 생략(medium 범위 판단).
- 미조사 공백: UI 특화 원인 분포 심화(arXiv 2103.02669) · 정적 예측 계열(Flakify/FlakeFlagger, arXiv 2112.12331) · 소규모 팀 경량 정책 · 보안 테스트 quarantine 예외 · quarantine의 커버리지 drift 정량.
- 시점: Google/Fowler 자료는 2011~2016 — 방향은 유효하나 수치는 노후.
