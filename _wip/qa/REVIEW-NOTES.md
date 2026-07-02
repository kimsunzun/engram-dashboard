# qa 개선 검토 (research 재설계 결 적용)

> 검토 대상 = `_wip/qa/`(SKILL.md · references/flow.md · references/bindings/engram.md · feedback.md). 기준 = `_wip/research/`(SKILL.md · flow.md · REDESIGN-SPEC-v3.md · feedback.md)의 재설계 DNA + `core/.../rules/markdown-format.md` + CLAUDE.md 게이트 규약.
> 참고: `_wip/qa/`는 현재 배포본 `.claude/skills/qa/`와 **동일**(diff = IDENTICAL). 즉 이건 "미개선 현행본"이고, 산출은 **개선 제안**이다(rewrite 아님).

## 1. 현재 스킬 요약

- **정체성이 명확하고 건강하다.** qa = "실제로 도나"(기계적 게이트), review = "맞나"(적대 판단)의 후행 짝(`SKILL.md:8`). 강도 quick/standard/full = **게이트 범위** 축(단계 아님, review 강도와 평행).
- **바인딩 분리가 이 스킬의 최강점.** `flow.md`는 스택을 전혀 모르는 범용 골격(cargo/cdp/npm 0회 — 실측 `flow.md` 전체에 engram 명령 없음). engram 전용 실명령(cargo·rg·cdp·npm)은 `bindings/engram.md`에 깔끔히 격리. research가 목표한 "범용 엔진 + bindings/<project>" 형태를 **이미 달성**.
- **정직 라벨 존재.** cdp 실측의 Windows 전용·플랫폼 제약이 `bindings/engram.md:55,70`·`flow.md:69,90`에 명시, "동작 미확인" 열화 보고 경로 있음(`flow.md:90`).
- **SSOT 지향 선언 있음.** `SKILL.md:14`·`flow.md:5,7`·`bindings/engram.md:5`가 "정본은 flow.md §0 / CLAUDE.md, 여기 복붙 금지"를 반복 선언.
- **feedback 루프 정합.** `SKILL.md:45-46`이 `feedback.md` + `usage-log.md` + 공용 규약(`_shared/self-improvement-feedback.md`)을 가리킴 — research와 동일 구조.
- **과청구 자제 미덕.** `bindings/engram.md:27`이 "프론트 lint는 정본에 없음 — 임의 추가 금지", clippy 미추가 등 SSOT(CLAUDE.md·package.json)를 넘겨 게이트를 발명하지 않음.

**결론:** research가 겪은 thesis 전복(v2→v3 대재설계)은 qa엔 불필요. qa는 이미 DNA 대부분을 갖췄고, **표적 수정 몇 건**만 남는다.

## 2. 갭·이슈 (우선순위·심각도)

### G1. 강도표가 SKILL.md ↔ flow.md §0에 near-verbatim 이중화 (심각도: 중)
- `SKILL.md:16-20`의 강도표(강도 | 게이트(범주) | 언제)와 `flow.md:13-17`의 강도표(강도 | 게이트 범위 | 언제)가 **사실상 동일 문구**다. quick 행 예: SKILL "격리 대상 모듈 닿으면 격리, 프론트 닿으면 타입체크" ≈ flow "격리 대상 모듈이 닿으면 격리도, 프론트가 닿으면 타입체크도". standard·full 행은 문구까지 일치.
- `SKILL.md:14`가 "정본 = flow.md §0 강도표 … 아래는 강도·범주·언제만"이라 **정본을 flow로 선언하고도 같은 입도(범주)로 표를 다시 그린다**. 두 표는 수동 동기화라 한쪽만 고치면 rot.
- **research 대조:** research는 이 함정을 피했다 — `research/SKILL.md:28`이 "정량(수집자 수·검색량·리뷰 범위)은 flow.md 강도표가 정본, 아래 표는 '언제·무엇'만(정량 베끼지 않는다)"라고 선언하고, `research/SKILL.md:30-34` 표에서 **정량 열을 실제로 드롭**한다(언제/무엇/산출만). qa는 정본 선언만 있고 열 드롭이 안 됐다.

### G2. review 바인딩이 qa 명령을 "복붙 금지" 선언 후 재수록 (심각도: 중 · 교차 스킬)
- `.claude/skills/review/references/bindings/engram.md:24`가 "강도별 실명령은 qa 스킬 바인딩이 정본 — **여기 베끼지 않는다**"라고 선언하고, 바로 다음 `:26-27`에서 `cargo test`·`cargo test -p engram-dashboard-core`·`-p engram-dashboard-protocol`·`cargo build`·`scripts/cdp.mjs`를 **그대로 재수록**한다. 선언과 실물이 자기모순 — 명령 정본이 **qa 바인딩 한 곳이 아니라 양쪽에 복붙**됐다.
- 지금은 두 사본이 우연히 일치하지만, qa 바인딩(예: 프론트 게이트 확정 절차)이 갱신되면 review 사본이 갈린다 = 전형적 rot. task가 물은 "명령 정본이 qa 바인딩 한 곳인가"의 답은 **아니오(부분 복붙)**.
- 방향(제안): review 바인딩 §"QA 실측 게이트 명령"은 **명령 나열을 지우고 qa 바인딩을 가리키기만** 한다("build/test·GUI 실측 실명령 = qa 바인딩 §강도별 실명령"). "무관하게 항상 돈다"·"PASS ≠ 동작 보장" 같은 *정책*만 남긴다. (review 스킬 파일 수정 = 조정 필요, qa 내부 수정 아님.)

### G3. review §5 게이트 vs `/qa` 스킬의 운영 경계 모호 (심각도: 중 · 사용자 결정)
- review `flow.md:86-90` §5는 "리뷰 판정과 무관하게 build/test를 돈다 … self에서도 생략 X"로 **review가 QA 게이트를 직접 돈다**고 읽힌다. 한편 CLAUDE.md 구현 실행 규약은 "QA = `/qa` 스킬로 build/test + GUI 실측"으로 **별도 /qa 호출**을 정본으로 둔다. qa `SKILL.md:25-27`은 순서(qa는 후행, review self여도 최소 quick)만 정할 뿐 **누가 게이트를 도느냐(review 내부 인라인 vs /qa 스폰)**·**한 번인가 두 번인가**를 확정하지 않는다.
- 역할 분리("맞나" vs "도나")는 명시됐으나 **운영 경계**(게이트 실행 주체·중복 실행 회피)가 비어 있다. 이중 실행(review §5 + /qa)로 낭비하거나, 반대로 둘 다 "상대가 돌겠지"로 누락될 여지.
- **research 대조:** research는 단독 스킬이라 이 경계가 없다 — 전이라기보단 qa 고유 갭. "경계를 명확히 문서화" 원칙만 전이.

### G4. 정직 ⚠️: 핫패스(race/lifetime)는 full의 cdp 실측 1회 통과로도 미증명 (심각도: 저~중)
- `bindings/engram.md:21`은 핫패스가 "test PASS만으론 race·lifetime 동작을 보장 못 한다"까진 정직하게 적지만, **그 위 대책인 cdp 실측 1회 통과도 race-free를 증명 못 한다**는 건 안 짚는다. `flow.md:71`·`bindings/engram.md:68`은 "이게 통과해야 동작 확인 = 완료"라고 단언 톤이다.
- research는 이 종류의 과청구를 정직 톤으로 눌렀다("라이브 1회 적출은 존재 증거지 효과 크기 증거 아님", `research/SKILL.md:58`). qa의 등가물: **cdp eval 1회 통과 = smoke(존재 증거)지 exhaustive/race-free 증명 아님**. SKILL §검증상태나 binding 핫패스 절에 한 줄 보강 가치.

### G5. "실행 중 명세/바인딩 드리프트 자기보고" 결여 (심각도: 저)
- research `SKILL.md:46-47` "실행 중 자기보고"는 실행 도중 발견한 명세 문제를 조용히 우회 말고 사용자에게 surface하라고 강제한다. qa `flow.md:74-82`(실패 처리)는 **게이트 FAIL**만 보고하고, **바인딩↔정본 드리프트**(예: `bindings/engram.md:27`대로 `npx tsc --noEmit`을 쓰는데 package.json에 이미 `typecheck` 스크립트가 생김, 또는 명령이 CLAUDE.md와 갈림)를 만났을 때 surface하라는 지시가 없다. `bindings/engram.md:5`는 "충돌하면 CLAUDE.md 따르고 이 파일을 고친다"까진 있으나 **사용자에게 알리라**가 빠졌다 — 조용히 우회 위험.

## 3. 제안 재설계 thesis (한 줄 + 근거)

**qa는 research DNA(바인딩 분리·정직 라벨·SSOT·feedback)를 이미 대체로 갖췄다 — research식 대재설계는 불필요하고, 필요한 건 (a) 강도표 SKILL↔flow 이중화 제거(정본 일원화·research식 열 드롭), (b) review와의 게이트 명령·경계 정합(복붙 제거 + 게이트 실행 주체 확정), (c) 핫패스/실측 한계 정직 note 보강 — 세 표적 수정뿐.** 근거: `_wip/qa`가 배포본과 동일(현행)이고, flow.md에 engram 명령 0회(바인딩 분리 달성)이며, 남은 결함은 전부 국소적 rot·경계·정직 문구다(구조·thesis 전복 아님).

## 4. 개선 항목 표

| 항목 | 무엇 | 왜 | 사용자 결정? | research 전이? | 심각도 |
|---|---|---|---|---|---|
| **A (G1)** | `SKILL.md:16-20` 강도표에서 **게이트(범주) 열 드롭** → 강도·언제(신호)만 남기고 게이트 범주는 flow.md §0 정본 참조 | 두 표 near-verbatim 수동 동기화 rot; SKILL이 정본 선언(`:14`)과 모순 | 아니오(문서 내부 구조) | **예** — research SKILL 표가 정량 열 드롭한 것과 동형(`research/SKILL.md:28,30-34`) | 중 |
| **B (G2)** | review `bindings/engram.md:26-27`의 cargo/cdp 명령 나열 삭제 → qa 바인딩 가리키기만, 정책 문구만 잔류 | "여기 베끼지 않는다" 선언 후 복붙 = SSOT 자기모순·rot | 아니오(단 review 파일 수정 = 조정 필요) | **예** — SSOT 정본 한 곳 | 중 |
| **C (G3)** | review §5 게이트와 `/qa`의 관계 확정 — 게이트 실행 주체·중복 회피를 qa `SKILL.md` "review와의 연결"과 review §5 양쪽에 정합 서술 | 게이트를 누가/몇 번 도는지 불명 → 이중 실행 또는 누락 | **예**(정책: review 내부 인라인 게이트냐 별도 /qa 스폰이냐) | 부분 — "경계 명확화" 원칙만 | 중 |
| **D (G4)** | 핫패스/실측 정직 note — cdp eval 1회 통과 = smoke, race-free/exhaustive 증명 아님을 SKILL §검증상태 또는 binding 핫패스 절에 한 줄 | full "동작 확인 = 완료" 단언이 race/lifetime엔 과청구 | 아니오 | **예** — 정직 톤·과청구 경계(`research/SKILL.md:58`) | 저~중 |
| **E (G5)** | "실행 중 명세/바인딩 드리프트 자기보고" 한 줄 — 바인딩↔정본 괴리 발견 시 조용히 우회 말고 surface | 조용한 우회로 게이트가 엉뚱한/낡은 명령을 돌 위험 | 아니오 | **예** — research "실행 중 자기보고"(`research/SKILL.md:46-47`) | 저 |
| F (옵션) | full 사전점검(preflight) — 비-Windows/포트 점유면 게이트 시작 전 abort-and-ask(현재는 사후 degraded 보고만) | research §0 Codex 가용성 사전 체크 결 | 아니오 | 부분 — 사전 가용성 체크(`research/flow.md:5-14`) | 저 |

## 5. 정직 노트 (research에서 전이 안 되는 것 · qa 고유 제약 · 과청구 경계)

**research 고유라 qa에 해당 안 됨 (억지 이식 금지):**
- **cross-family 적대 리뷰 / Advocate·Adversary 2인 / 모델 배정표 / BLIND** → **해당 없음.** qa는 명령이 PASS/FAIL을 직접 낸다 — 판단 주체(리뷰어)도, 스폰할 에이전트도, family 편향도 없다. 배정표(모델→역할)는 qa에 무의미(명령엔 모델이 없다).
- **calibration(기권·확신도) / abstention≠contradiction / grounding(claim↔source 함의)** → **해당 없음.** qa엔 클레임도 출처도 없다. 게이트는 "지지/미지지"가 아니라 exit/매치로 이항 판정.
- **적대 강도 레벨 사다리(2~5, 축=리뷰어 독립도)** → **해당 없음.** qa 강도(quick/standard/full)는 "게이트 범위" 축이지 "리뷰어 독립도" 축이 아니다. qa의 대응물은 **escalation-only(도중 위험 발견 시 자동 상향, 하향 금지 — `flow.md:20,27`)**로 이미 충분.
- **라우팅 = 스킬 핵심 산출** → **부분만.** research는 "싸게 vs 적대리뷰"를 가르는 판별이 핵심 산출이었다. qa의 등가물은 "경로→강도 매핑"(`flow.md:35`·바인딩)인데, 이건 기계적 매핑이라 research만큼 "핵심 산출" 무게는 아니다 — 억지로 격상할 필요 없음.

**qa로 잘 전이되는 것 (이미 있음 → 강화만):**
- 바인딩 분리(강 — 이미 달성) · 정직/열화 라벨(강 — 이미 있음) · SSOT 정본 한 곳(선언은 있음, G1/G2가 실행 갭) · self-improvement feedback(강 — 이미 정합) · 실행 중 자기보고(약하게 전이 — G5, 명세 드리프트 surfacing).

**qa 고유 과청구 경계 (정직해야 할 지점):**
- **"코드 통과 = 동작 확인" 착각 금지**는 이미 잘 박혀 있다(`flow.md:112`·`SKILL.md:43`) — 유지.
- **한 겹 더(G4):** full의 cdp 실측조차 **닿은 동작 1회 통과 = smoke**다. 특히 핫패스(race/lifetime)는 1회 관찰로 race-free를 증명하지 못한다 — research가 "1회 적출=존재 증거지 효과 크기 아님"으로 눌렀듯, qa도 "1회 실측 통과=존재 증거지 exhaustive/race 증명 아님"을 정직하게 달아야 과청구를 막는다.
- **비-Windows 한계**는 정직하게 표시됨(`bindings/engram.md:70`) — 다만 시작 전 preflight가 아니라 사후 degraded 보고라, F(옵션)로 사전화 여지.
