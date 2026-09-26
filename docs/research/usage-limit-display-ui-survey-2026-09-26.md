# Claude·Codex 사용량 한도(5시간·주간) 표시 UI — 선례 서베이

| 항목 | 값 |
|---|---|
| **상태** | 확정(적대 리뷰 반영) — cross-family 리뷰 판정 BLOCK → 지적 8건 중 7건 반영·1건 부분 반영(§8). 레이아웃 등 선택은 **사용자 결정 대기**(§6) |
| **날짜** | 2026-09-26 |
| **방법** | 수집 4갈래(공식 제품 · 서드파티 모니터 · 에이전트 매니저/IDE · 응답 필드) + 실측 2건(codex app-server `account/rateLimits/read` · `claude -p` stream-json) + 메인 grounding 스팟체크(§7) + cross-family 적대 리뷰 1회(codex-cli 0.156.1, effort high, 웹검색 켬 — §8) |
| **확신도 범례** | **확실** = 독립 출처 둘 이상(실측+소스, 수집자+리뷰어 독립 재확인 등) · **가능성 높음** = 출처 하나가 지지 · **불확실** = 미지지·검색 스니펫뿐 |
| **왜 이 조사를 했나** | 웹뷰(프론트) 슬롯에 새 타입을 추가해 Claude·Codex 의 5시간·주간 한도를 작게 보여 주려 한다. 사용자가 원하는 레이아웃이 아직 없어서, 남들이 **무엇을 보여 주고 · 어떻게 갱신하고 · 어떤 모양인지**부터 본다 |
| **범위 밖** | 벤더별 데이터 출처 **선택**(사용자 결정 2026-09-26: "백엔드별 데이터는 나중에"). §4 는 "요청하면 무엇이 오나"만 적는다 |
| **이 서베이로 난 사용자 결정** | 아래 「사용자 결정」 절 (2026-09-26~27 인터뷰) |

## 사용자 결정 (2026-09-26~27 인터뷰)

- **퍼센트 방향 = 남은 양**(Codex 방식 — 쓸수록 막대가 줄어든다). Claude 는 사용률을 주므로 100 에서 뺀다.
- **작은 표시(상시)** = 회사별로 두 창(5시간·주간) 모두 · 막대 + % + **남은 시간**. 슬롯 폭에 따라 회사당 한 줄 → 창마다 한 줄 → 숫자만으로 줄어든다. **Claude·Codex 각각 켜고 끌 수 있다.** 수동 새로고침 ⟳ 를 둔다.
- **클릭 팝업(상세)** = 전부 보인다 · **목표 시각**(리셋 시각) · 플랜(값이 있을 때만) · **각 사 사용량 설정 링크**(Codex `chatgpt.com/codex/settings/usage` · Claude `claude.ai/settings/usage`) · ⟳. **소진 예측은 넣지 않는다.** 팝업은 슬롯 밖으로 뜰 수 있다(기존 슬롯 우클릭 메뉴와 같은 `position: fixed` 방식).
- **갱신** = 대화 중에 오는 한도 정보를 줍고, **두 창이 다 새로 들어오면 쿨타임을 초기화**, 쿨타임이 지나면 자동 조회, 수동 ⟳. **쿨타임 5분.**
  - 줍는 곳 = **Claude JSON 모드의 `rate_limit_event` · Codex app-server 알림**뿐(이미 읽는 스트림 — 추가 비용 0). **터미널 모드는 두 회사 모두 줍지 않는다** — Claude statusline(메시지마다 프로세스가 뜬다)과 Codex 세션 기록 파일 읽기(ADR-0203 이 읽지 않기로 한 파일)를 둘 다 뺐다.
- **링크 이름 = 「사용량 페이지 ↗」** + 마우스를 올리면 목적지 설명. **로컬라이징 전제** — 문구는 전부 `t(키)`(ADR-0069), 날짜·시간은 `Intl` 로케일 포맷, 폭 줄이기는 글자 수가 아니라 실제 그려진 폭 기준.
- **색 기준 = 서드파티 다수 방식**(남은 양 기준): 50% 초과 초록 · 50~20% 노랑 · 20% 미만 빨강 + ⚠. e-ink 는 노랑 = 빗금, 빨강 = 검정 채움 + 굵은 테두리 + ⚠.
- **켜고 끄기 = 슬롯 우클릭 메뉴에만**(⟳ 새로고침 · ☑ Claude 표시 · ☑ Codex 표시). 팝업은 보기 전용. 둘 다 끄면 작은 표시에 "우클릭해서 표시할 항목 고르기" 안내 한 줄.
- **저장 = 지금은 하지 않는다.** 켜고 끈 상태는 슬롯 내용 안에 넣어 레이아웃과 같은 수명으로 둔다(레이아웃은 아직 디스크 영속이 없다 — CLAUDE.md 「LLM-우선 제어」). 레이아웃 영속이 생기면 따라서 저장된다.
- **Claude 능동 조회 = 잠정 B**(Claude Code 토큰 파일을 읽어 비공개 사용량 주소 호출 — 피어 8곳 중 7곳의 방식, 2026-09-27). A(CLI 에 `get_usage`)는 탐침으로 되는지만 확인한다. 근거·정책 인용 = `claude-usage-query-method-survey-2026-09-27.md`. ★B 는 정책 문장과 모양이 겹친다 — 출시 전 법무·보안 확인 필요(이 문서는 판단하지 않는다)★.
- 남은 결정: Claude 쿨타임 5분(위 결정) ↔ 15분(피어 하한 — 조회 제한 증거) · 탐침 결과를 본 뒤 A/B 확정.

---

## 0. 결론

1. **핵심 정보는 어디나 같다 — 창 둘(5시간·주간) × {사용률 %, 리셋 시각}.** 모델별 주간 한도·플랜·크레딧은 **있을 때만 덧붙는 부가 행**이다. (확실)
2. **퍼센트 방향이 벤더마다 반대다.** Claude Code `/usage` 는 "% used"(CHANGELOG 에 "Switched `/usage` back to "% used"" — 다른 방향을 시도했다 되돌린 흔적), Codex `/status` 는 "% left". 한 화면에 둘을 섞으면 같은 막대가 반대 뜻이 된다. 매니저·서드파티는 **하나로 통일하거나 Used/Remaining 토글**을 둔다. (확실 — 리뷰어 독립 재확인)
3. **작은 상시 표시 + 상세는 hover/click, 2단 구조가 가장 흔하다.** 상시 쪽은 숫자·미니 막대·단일 게이지, 상세(창별 행·절대 리셋 시각·갱신 시각·오류)는 툴팁·팝오버에 연다. **click 으로 고정**해 읽게 하는 곳도 있다(TokenTelemetry). (가능성 높음 — 조사한 제품 기준)
4. **갱신 수치는 한 값으로 모이지 않는다.** 실제 조회 주기는 **60초(Codex TUI·claude-usage-widget·CodexMeter 배경) ~ 15분(Orca·CodeZeno) ~ 적응형 30분(CodexBar)**까지 퍼져 있고, 조회를 **열 때만** 하는 곳(Paseo·ClaudeBar 기본값)도 있다. 공통점은 수치가 아니라 구조다 — **조회 주기 · 캐시 신선도 · 카운트다운 재렌더 · push 병합이 서로 다른 축**으로 따로 돈다. (가능성 높음)
5. **오래된 값은 지우지 않고 표시해 둔다.** 마지막 정상값을 유지하고 "N분 전"·"Cached"·⚠·흐림으로 알린다. 리셋 시각이 지나면 옛 % 를 남기지 않는다(Claude Code 가 그 버그를 고쳤다). (확실)
6. **분할 레이아웃의 한 칸(슬롯)에 두는 선례는 조사한 제품에서 찾지 못했다.** 가장 가까운 것은 **사이드바 하단의 상시 게이지**(TokenTelemetry — 모든 에이전트의 창 중 가장 빠듯한 것으로 색을 칠하고 hover 에 전체를 펼친다)와 상태바 칸(Orca)이다. 슬롯은 크기가 사용자 분할에 따라 바뀌므로 **폭에 따라 줄어드는 형태**(Orca verbose→compact→narrow, Codex `/status` 가 좁으면 막대를 버리고 % 를 남김)를 빌려 와야 한다. (가능성 높음 — 해석)
7. **데이터 모양도 두 벤더가 거의 같다** — 창마다 {사용률, 리셋 시각}. 단 단위가 소스마다 다르다(Claude 는 0–1 과 0–100, ISO 와 epoch 가 섞인다). Codex 의 `primary`/`secondary` 는 **항상 5시간/주간이 아니다** — 창 길이(`windowDurationMins`)로 분류해야 한다(실측). (확실)

---

## 1. 무엇을 보여 주나

### 1-1. 핵심 넷

| 값 | 공식 | 서드파티·매니저 |
|---|---|---|
| 5시간 창 사용률 | Claude `/usage` "Current session", Codex `/status` "5h limit" | 전부 |
| 주간 창 사용률 | Claude "Current week (all models)", Codex "Weekly limit" | 전부 |
| 각 창 리셋 시각 | **절대 시각** — Codex `resets 14:30` / 다른 날이면 `14:30 on 3 Oct`, Claude `Resets 9:40pm (TZ)` | **카운트다운이 기본**(`resets in 3h 54m`, ClaudeBar `2d`/`4:40`/`45m`/`soon`), 절대 시각은 툴팁 2차 줄(t3code `Resets <abs> · in 2h`, Jean `Fri 2:00 PM`) |

### 1-2. 방향 — used vs left

| 제품 | 방향 | 비고 |
|---|---|---|
| Claude Code `/usage` | used | CHANGELOG "Switched `/usage` back to "% used"" |
| Codex CLI `/status`·footer | left | `[████░░…] 72% left (resets 14:30)` — 20칸 막대가 **남은 양**을 채운다 |
| Orca | used 기본 + 토글 | 남은 쪽을 보여도 **막대 색은 used 기준** |
| t3code | left | "as Codex does" |
| Paseo · Jean | used | |
| AgentLimits | 토글(앱·위젯 공유) | |
| ClaudeBar · CodeZeno · ccstatusline · claude-hud · Usage4Claude | 토글 | 서드파티 다수가 사용자 선택으로 둔다 |

### 1-3. 부가 정보 (있을 때만 · 대개 툴팁 안)

- 모델별 주간 한도 — Claude(Sonnet only · Fable 등), Codex(추가 limit 버킷). claude-usage-widget 은 API 가 모델별 창을 보고할 때만 셋째 막대를 **자동으로 붙였다 뗀다**.
- 플랜 이름 — Codex `planType` 은 모든 스냅숏에 있다, Claude 는 일부 출처에만(§4).
- 추가 사용량·크레딧 — Claude Extra usage, Codex credits · 무료 리셋 쿠폰(`rateLimitResetCredits`).
- **페이스(pace)** — "창 경과 시간 대비 사용률"로 **다 쓰기 전에 리셋되나**를 답한다. 성숙한 도구 여럿이 싣는다:
  - t3code — 막대 위 시간 경과 헤어라인 + ahead/on/under 글리프(±5pt).
  - AgentLimits — "Pacemaker": 경과 % 와 비교해 초록(제때·앞섬) · 주황(약간 초과) · 빨강(10%p 이상 초과), 시간 막대를 5시간 = 5칸 · 주간 = 7칸으로 나눈다.
  - ClaudeBar — 소모율 1.5× 경고. CodexBar — "On pace / X% in deficit". claude-pace — `⇣15%`/`⇡15%`.
- 갱신 시각 — "Updated Xm ago"(Orca·Paseo), "Last updated: just now"(claude.ai), "Cached"(TokenTelemetry).

### 1-4. 빈 상태·오류 상태

- **인증 없음/미지원 계정** — 막대 대신 문구: Codex "not available for this account", Orca "Sign in to see usage", t3code "This account has no subscription limits.". TokenTelemetry 는 **로그아웃을 오류로 치지 않는다**(주황 경고는 조회가 실제로 깨졌을 때만).
- **아직 데이터 없음** — Codex "data not available yet", ClaudeBar `—` + "Waiting for quota data…", Orca `···` 펄스.
- **실패했지만 이전 값 있음** — 이전 값 유지 + 표시: Orca ⚠ "Refresh failed — showing cached data", CodexBar 아이콘 흐림, Claude Code "as of" 주석.
- t3code 는 **"미지원(unsupported — 막대 비움)"과 "조회 실패(probeFailed — 이전 막대 유지)"를 구분**한다. TokenTelemetry 도 "보고할 것 없음 / 갱신 필요(실패) / 한도 없음"을 나눈다.
- **창 하나만 오는 경우** — AgentLimits: Codex 가 주간만 돌려주면 5시간 칸을 "없음"으로 두고 값을 주간 칸에 넣는다. CodexBar: 선택한 창이 없으면 다른 창으로 대체하지 않고 `—`.
- **0% 행도 그린다** — 빠뜨리면 "없는 한도"로 읽힌다(Claude Code VS Code 미터 버그 수정 — 수집자 보고, 버전 번호 미대조).

---

## 2. 어떻게 갱신하나

★**아래 표는 네 축을 가른다**★ — 조회 주기(실제로 가져오는 간격) · 캐시 신선도(그 안이면 재조회를 건너뜀) · push(에이전트 트래픽에서 오는 갱신) · 재렌더(카운트다운 틱). 한 칸의 숫자를 다른 칸과 비교하지 말 것.

| 제품 | 조회 주기 | 캐시·신선도 | push | 기타 |
|---|---|---|---|---|
| Codex CLI TUI | 60초 → 75%↑ 30초 · 90%↑ 15초 · 99%↑ 5초 | 15분 지나면 "limits may be stale" | app-server `account/rateLimits/updated`(희소 — 병합, null 은 지우지 않음) | |
| claude-usage-widget | 60초, 레이트리밋 시 300초까지 백오프 | — | — | 깨끗한 조회 뒤 원래 간격으로 복귀 |
| CodexMeter | 배경 60초 · 메뉴 열림 10초 | — | Codex 레이트리밋 갱신 시 | 실패 시 최대 5분 백오프, 이전 값 stale 표시 |
| TokenTelemetry | 게이지가 1분마다 읽음 | 서버 캐시 5분(`quotas.json`) | — | Refresh 가 캐시를 건너뛴다 |
| AgentLimits | 1–10분(사용자 설정) | — | — | Refresh Now |
| Usage4Claude | 스마트: 변화 중 1분 → 정지 시 3/5/10분 | — | — | 깨어날 때·창 열 때, 수동 클릭 10초 디바운스 |
| t3code | 5분 | Usage 페이지 열 때 5분 하한 | Claude `rate_limit_event`·Codex `account/rateLimits/updated` 를 **창 id 로 병합** | |
| Jean | 5분(스냅숏 `fetchedAt` 기준) | — | — | 카운트다운은 1분마다 |
| Orca | 15분(최소 30초로 clamp) | 재요청 디바운스 5분 · 30분 지나면 stale 값 폐기(레이트리밋이면 24시간) | Claude statusLine `rate_limits`(패널당 15초·30초 dedup) | 창 포커스 시 갱신, 숨김·최소화 중 중단, 실패 시 30초→15분 백오프 |
| CodeZeno(Windows) | 15분 기본(1분/5분/15분/1시간) | `Retry-After` 쿨다운 최대 24시간 | — | |
| CodexBar | 적응형 2/5/15/30분(메뉴를 최근에 열었을수록 짧게) | — | — | 저전력·발열 시 30분, "Refresh now" 상시 |
| Paseo | **주기 없음 — 툴팁을 열 때만** | 클라 5분 stale time + 서버 5분 캐시 | — | |
| ClaudeBar | 기본 꺼짐(팝오버 열 때만) | API 모드 캐시 ≥15분 | — | 429 시 `Retry-After`(없으면 5분) |
| Claude Code statusline | (선택) `refreshInterval` | — | 어시스턴트 메시지마다 · 창 `resets_at` 도달 시 | 300ms 디바운스, 리셋 지난 창은 뺀다 |

**구조적 공통점(수치 아님):**

- **push 가 있으면 병합한다**(t3code·Orca·Codex TUI) — 희소 업데이트라 **덮어쓰기가 아니라 창 단위 병합**이다.
- **카운트다운 재렌더는 데이터 갱신과 따로** 돈다 — Orca 는 다음 단위 경계(분)에 맞춰 틱을 예약한다(`src/shared/rate-limit-reset-format.ts`), Jean 은 1분마다.
- **실패하면 백오프하고 이전 값을 유지**한다(Orca·CodexMeter·claude-usage-widget·ClaudeBar·CodeZeno).
- **리셋 시각이 지나면 옛 % 를 치운다** — Claude Code CHANGELOG "Fixed the status line `rate_limits` fields and `/usage` still showing a rate-limit window's pre-reset usage percentage after the window reset while the session was idle".
- Anthropic 사용량 엔드포인트가 429 뒤 오래 거절한다는 주장이 있다(ClaudeBar README — **벤더 확인 안 됨**). 데이터 출처를 고를 때 다시 볼 것.

---

## 3. 어떤 모양인가

### 3-1. 놓는 자리

| 자리 | 예 |
|---|---|
| 사이드바 하단 상시 게이지 | TokenTelemetry(모든 페이지, 알림 벨 위) |
| 앱 크롬 상태바 — 벤더당 한 칸 | Orca(하단 상태바 `ProviderSegment`), VS Code Codex "Rate limits remaining" |
| 툴바/독 배지 | Jean `9|41%`(세션|주간) |
| 다른 계기의 툴팁 안 | Paseo(컨텍스트 창 링 툴팁 — **활성 에이전트의 벤더만**), Conductor(컨텍스트 사용량 hover card) |
| 사이드바 섹션 | Claude Code VS Code "ACCOUNT & USAGE" 미터 + "View details" |
| 전용 페이지·탭 | t3code `/usage`, Superset Usage 탭, Paseo 설정 페이지, claude.ai Settings → Usage |
| 데스크톱 오버레이(항상 위) | claude-usage-widget OSD — click 하면 상세 팝업 |
| 메뉴바·트레이·위젯 | CodexBar, ClaudeBar, Usage4Claude, AgentLimits(메뉴바 2줄째 `X% / Y%`), CodeZeno(작업표시줄 위젯 2행) |

**분할 레이아웃의 한 칸(슬롯)에 두는 선례는 조사한 제품에서 찾지 못했다**(조사 범위 = 위 표 + §9 의 미확인 목록). 에이전트마다 한도 게이지를 다는 곳도 없다(Paseo·Conductor 가 활성 에이전트의 벤더로 거르는 것이 가장 가깝다).

### 3-2. 두 창을 작은 자리에 넣는 법

| 방식 | 예 | 성격 |
|---|---|---|
| 창별 행 스택(라벨·가는 막대·%·리셋) | Orca 툴팁 `ProviderPanel`(6px 막대), Codex `/status`, t3code 행 그리드(6px), Paseo 카드(4px), claude-usage-widget Bars 보기(Codex 를 켜면 막대가 아래로 늘어남) | 가장 흔한 **상세** 형태 |
| 한 줄에 두 창 | ClaudeBar `5h 62% · 4:40 | 7d 34% · 2d`(창마다 색, 합친 라벨은 더 나쁜 쪽 색), Jean `session|weekly%`, AgentLimits `X% / Y%`, Codex footer `5h 72% left` `weekly 40% left` | 가장 흔한 **작은** 형태 |
| 단일 게이지 — 가장 빠듯한 창으로 색칠 | TokenTelemetry(**모든 에이전트·모든 창 중** 가장 한도에 가까운 것), Orca 상태바(벤더별 가장 빠듯한 창, 48×6px 막대 + %), CodexMeter | 가장 작다 — 나머지는 hover |
| 주간은 위험할 때만 | claude-hud(7d 는 80% 이상일 때만) | 조건부 노출 |
| 링 | CodexMeter 동심원(바깥 = 남은 한도, 안 = 남은 시간), Usage4Claude, claude-usage-widget Gauge 보기(Claude+Codex = **2×2 링 격자**) | 정사각 자리용 |

**폭에 따라 줄이기:** Orca = verbose(아이콘 + 막대 + 글자) → compact(아이콘 + %) → narrow(한 글자 배지). Codex `/status` = 좁으면 막대를 버리고 % 를 남기며 **리셋 시각은 자르지 않고 다음 줄로 감싼다**("the actionable part of this row").

### 3-3. 색 임계값

| 제품 | 기준 | 구간 |
|---|---|---|
| Orca | used | <60 **무채색**(상시 크롬을 조용히) · 60–79 노랑 · ≥80 빨강 |
| Paseo | used | ≥70 경고 · >90 위험 |
| ClaudeBar · claude-codex-battery | remaining | ≥50 정상 · 20–50 경고 · <20 위험 (= used 50/80) |
| cfranci 확장 · "Claude Code Limits" 확장 | used | <50 초록 · 50–80 주황 · >80 빨강 |
| AgentLimits · t3code | **페이스** | 고정 임계값 대신 경과 시간 대비(AgentLimits 10%p 초과 = 빨강 · t3code ±5pt 글리프, 긴급 색 없음) |
| Codex CLI | used(알림) | 50/75/90/95% 에서 경고 문구, 90% 에서 모델 전환 제안 · 막대 자체는 임계 색 없음 |
| Claude Code | used(알림) | 공개된 일반 임계값은 확인 못 함. CHANGELOG 한 줄이 "주간 리셋 뒤 낮은 사용률에서 경고가 뜨던 것을 고침 — 이제 70% 사용이 필요"라고 적는다 |

**빨강 선은 80 근처로 모인다.** 경고 시작은 50·60·70 으로 갈린다. 고정 임계값 대신 **페이스로 색을 정하는 쪽**이 따로 있다. **색만으로 구분하지 않는 장치**: CodexMeter("without relying on color alone"), Usage4Claude(흑백 모드에서 창별 모양), ClaudeBar(고대비 팔레트 4.5:1) — 우리 e-ink 테마와 직결된다.

### 3-4. 빌려 올 만한 세부

- 막대와 라벨이 같은 반올림 함수를 쓴다 — 어긋나면 "막대는 다 찼는데 99%" 가 된다(Orca `src/shared/usage-percentage-display.ts`).
- 배경 재조회 중에도 정상 칩을 그대로 둔다 — 실패하는 벤더가 매 주기 `…→오류`로 깜빡이지 않게(Orca).
- hover 로 펼치고 **click 으로 고정**한다 — 읽거나 스크롤할 수 있게(TokenTelemetry). 아래 접근성 항목과도 맞물린다.

### 3-5. 접근성 (리뷰가 짚은 누락)

- 게이지는 **프로그램이 읽을 수 있는 이름과 값**을 가져야 한다 — W3C ARIA `meter` 패턴(https://www.w3.org/WAI/ARIA/apg/patterns/meter/). 「LLM-우선 제어」 관점에서도 값이 DOM 텍스트로 있어야 LLM 이 읽는다.
- hover 로 드러나는 내용은 **focus 로도 열리고 · 닫을 수 있고 · 포인터를 올려도 사라지지 않아야** 한다 — WCAG 2.2 SC 1.4.13(https://www.w3.org/WAI/WCAG22/Understanding/content-on-hover-or-focus.html). **hover 전용 상세는 이 조건을 못 채운다** → click 고정 또는 focus 열림이 필요하다.

---

## 4. 요청하면 무엇이 오나 (데이터 모양만 — 출처 선택은 범위 밖)

### 4-1. Codex — 한 가지 모델이 네 통로로 온다

**모델 = `RateLimitSnapshot`**: `limitId` · `limitName` · `normalModelSlug` · `primary` · `secondary` · `credits{hasCredits, unlimited, balance: string}` · `individualLimit` · `spendControlReached` · `planType` · `rateLimitReachedType`. **창 = `{usedPercent 0–100, windowDurationMins, resetsAt epoch 초}`**.

| 통로 | 방식 | 비고 |
|---|---|---|
| app-server `account/rateLimits/read` | 요청(파라미터 없음, `initialize` 뒤) | 응답에 `rateLimitsByLimitId`(버킷별)·`rateLimitResetCredits`·`accountId` 가 더 온다. **실측 성공** |
| app-server `account/rateLimits/updated` | push | 희소 — 직전 read 에 병합(null 은 지우지 않음). 이번 실측에서는 도착하지 않았다(턴을 안 돌림). **우리 데몬은 지금 이것을 받고 버린다**(`crates/engram-dashboard-agent/src/backend/codex/decoder.rs:125`) |
| 세션 기록 JSONL `token_count` 이벤트 | 파일 | snake_case(`window_minutes`, `used_percent` f64). **실측 샘플 확인** |
| 백엔드 HTTP `wham/usage` | 요청(ChatGPT OAuth 토큰 필요) | 창 길이가 **초**(`limit_window_seconds`), `reset_after_seconds` 도 있음. 비공개 — 소스로만 확인, 호출 안 함 |

**실측(2026-09-26, codex-cli 0.156.1, Plus 플랜 — 식별자 제거):**

```json
"primary":   { "usedPercent": 37, "windowDurationMins": 300,   "resetsAt": 1790424706 },
"secondary": { "usedPercent": 58, "windowDurationMins": 10080, "resetsAt": 1790739378 },
"planType": "plus",
"rateLimitsByLimitId": { "base_model_inference": {
  "limitName": "gpt-reserve", "normalModelSlug": "gpt-5.6-luna",
  "primary": { "usedPercent": 0, "windowDurationMins": 10080, "resetsAt": 1791018755 },
  "secondary": null } }
```

★**`primary` 가 곧 5시간이 아니다**★ — 추가 버킷 `base_model_inference` 는 `primary` 가 주간(10080분)이다. 5시간/주간 분류는 **슬롯 위치가 아니라 `windowDurationMins`** 로 해야 한다.

### 4-2. Claude — 출처가 다섯이고 단위가 섞인다

| 통로 | 방식 | 창 | 사용률 단위 | 리셋 형식 | 문서화 |
|---|---|---|---|---|---|
| stream-json `rate_limit_event` | push(한도 정보가 바뀔 때) | 문서화된 최상위 필드는 `rateLimitType` 이 가리키는 **한 창만**(그 필드들도 전부 optional) | **0–1** | epoch 초 | 타입은 SDK 에 공개 |
| statusline stdin `rate_limits` | push(Claude Code 세션 안에서만) | `five_hour` · `seven_day` | 0–100(`used_percentage`) | epoch 초 | 공식 문서 |
| SDK 제어 요청 `get_usage` | 요청 | + `seven_day_opus`/`_sonnet`/`_oauth_apps`, `model_scoped[]`, `extra_usage`, `subscription_type` | 0–100 | ISO 8601 | SDK 에 공개 — 메서드 이름이 `…_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET` |
| OAuth `api/oauth/usage` | 요청(OAuth 토큰) | `get_usage` 와 같은 모양 + 새 `limits[]` | 0–100 | ISO 8601 | **비공개** — 서드파티 코드로만 확인, 호출 안 함 |
| 응답 헤더 `anthropic-ratelimit-unified-*` | 프록시만 볼 수 있음 | 5h · 7d | 0–1 | epoch 초 | 비공개 — 서드파티 |

**실측(2026-09-26, Claude Code 2.1.280 — 식별자 제거):**

```json
{"type":"rate_limit_event","rate_limit_info":{
  "status":"allowed_warning","rateLimitType":"seven_day",
  "utilization":0.83,"resetsAt":1790856000,"surpassedThreshold":0.75,"isUsingOverage":false,
  "unifiedWindows":{"five_hour":{"utilization":0.67,"resetsAt":1790425800},
                    "seven_day":{"utilization":0.83,"resetsAt":1790856000}}}}
```

★**`unifiedWindows` 는 캡처 한 건에서 관측됐을 뿐이다**★ — SDK 타입 선언(`sdk.d.ts`)에 없고, 다음 버전에서 사라져도 이상하지 않다. **"stream 한 줄로 두 창을 다 받는다"를 전제로 삼지 말 것.** 우리 저장소 테스트 픽스처(`claude_text.jsonl:3` 등)의 `rate_limit_event` 는 이보다 얇은 옛 모양(`utilization` 없음)이다. 데몬은 이 이벤트를 **건너뛴다**(`crates/engram-dashboard-agent/src/backend/claude/mod.rs:743`).

### 4-3. 나란히

| | Claude | Codex |
|---|---|---|
| 5시간 창 | `five_hour` | 창 길이 300분인 창 |
| 주간 창 | `seven_day` | 창 길이 10080분인 창 |
| 창 길이 | 이름으로 암시(분 필드 없음) | 명시(분 · 백엔드는 초) |
| 사용률 | 0–1(stream·헤더) / 0–100(statusline·`get_usage`·엔드포인트) | 0–100(app-server·백엔드는 정수, 기록 파일은 실수) |
| 리셋 | epoch 초 / ISO 8601 | epoch 초 |
| 플랜 | `get_usage.subscription_type` 에만 | 모든 스냅숏 `planType` |
| push | stream `rate_limit_event`(문서상 한 창씩), statusline | app-server `account/rateLimits/updated`, 기록 파일 |
| pull | `get_usage`, OAuth 엔드포인트 | app-server `account/rateLimits/read`, `wham/usage` |

---

## 5. 우리 제약 적합도

**제약:** ① 슬롯 크기가 사용자 분할에 따라 바뀐다(가로로 긴 띠일 수도, 세로로 긴 기둥일 수도) ② 테마 dark/light/**e-ink** — CSS 변수 토큰(CLAUDE.md) ③ 「LLM-우선 제어」 — 슬롯 생성은 기존 `layout.setSlotContent` 경로로 흡수, 새 전역 핸들 금지, 값은 DOM 텍스트로 ④ 사용자 요구: 작게 한 곳, 상세는 툴팁, 막대는 작게, 지금은 5시간·주간만 ⑤ 접근성 — 상세가 focus·click 으로도 열려야 한다(§3-5).

**아래 "네 값 상시 노출"은 요구가 아니라 트레이드오프다** — 사용자 요구 ④는 상세를 툴팁에 두는 것을 허용한다.

| 형태 | ① 크기 변화 | ② e-ink | 네 값 상시 노출 | 선례 |
|---|---|---|---|---|
| A. 4행 스택(벤더×창 한 줄씩) | 세로 기둥에 강함 · 좁으면 막대를 버리고 % 로 | 글자 + 막대 길이라 색 없이 읽힘 | 예 | Orca 툴팁 · Codex `/status` · claude-usage-widget Bars |
| B. 벤더 2행 × 막대 둘 나란히 | 가로 띠에 강함 · 세로 기둥엔 넘침 | 같음 | 예 | ClaudeBar 2줄 · Jean 배지 · AgentLimits `X% / Y%` |
| C. 벤더당 가장 빠듯한 창 하나 | 작은 자리에 들어감 | 같음 | 아니오 — 벤더당 하나만, 나머지는 툴팁 | Orca 상태바 · CodexMeter |
| E. 단일 게이지(전체 중 가장 빠듯한 창으로 색칠) | 가장 작은 자리에도 들어감 | **색이 주 신호라 e-ink 에서 약함** — 숫자 병기 필요 | 아니오 — 전부 툴팁 | TokenTelemetry |
| D. 링(2×2 격자 등) | 정사각 자리에 적합 | 모양으로 창 구분 — 작은 크기에서 약함 | 예(2×2 격자) | claude-usage-widget Gauge · 메뉴바 앱 |

---

## 6. 선택지 (사용자 결정 대상)

1. **레이아웃** — A / B / C / E (위 표). 폭 줄이기(막대 → % → 한 글자)는 어느 쪽이든 붙일 수 있다.
2. **퍼센트 방향** — used 통일 / left 통일 / 벤더 관례 그대로 / 토글. 벤더 관례 그대로는 한 슬롯에 반대 방향 막대가 섞인다.
3. **리셋 표시** — 슬롯엔 카운트다운 + 툴팁에 절대 시각(매니저 관례) / 절대 시각만(공식 관례).
4. **색** — 고정 임계값(60/80 무채색 시작 · 50/80 · 70/90) / 페이스 기준(AgentLimits·t3code). e-ink 용 비색 표지 필요 여부.
5. **페이스 표시** — 막대 위 시간 경과 헤어라인(t3code) 넣기 / 안 넣기.
6. **상세 여는 방식** — hover + focus / hover + click 고정. hover 전용은 §3-5 조건을 못 채운다.
7. **갱신** — 데이터 출처 결정과 묶인다(범위 밖). 표시 쪽의 **카운트다운 분 단위 재렌더 · stale 표시 · 리셋 경과 시 옛 % 제거 · 실패 시 이전 값 유지**는 조사한 선례에 널리 있어 결정 대상이 아니라 기본값 후보다.

**거부 후보 (근거 = 이 서베이 · 최종 판정은 사용자):**

- **D. 링** — 2×2 링 격자 선례(claude-usage-widget)는 있다. 그러나 작은 슬롯에서 링 네 개는 숫자를 넣을 자리가 좁고, 창 구분을 모양에 기대 e-ink 에서 약하다. 매니저·IDE 선례는 조사 범위에서 0.
- **추정치(로컬 토큰 로그 합산)** — ccusage·옛 Claude-Code-Usage-Monitor 방식. 벤더가 계산한 실제 % 가 이미 오는데 추정은 틀릴 수 있다. agent-deck 이 근거를 적어 두었다: "being wrong about a quota is worse than not showing one".

---

## 7. grounding (메인 대조)

| 클레임 | 대조 | 판정 |
|---|---|---|
| Codex app-server read 응답 모양·`base_model_inference` 버킷 | 실측 캡처 파일 직접 열람 | 지지 |
| Claude `rate_limit_event` 의 `unifiedWindows`·0–1 단위 | 실측 캡처 파일 직접 열람 | 지지(단 캡처 1건 — §4-2 단서) |
| Orca 15분 폴링·5분 디바운스·30분/24시간 stale·색 60/80 | 로컬 클론 `src/main/rate-limits/service.ts:76-92`, `src/renderer/src/components/status-bar/tooltip.tsx:190-198` | 지지 |
| Codex stale 15분 | openai/codex `codex-rs/tui/src/status/rate_limits.rs:66` | 지지 |
| Claude Code: statusline `rate_limits` 추가 · `/usage` "% used" 복귀 · "as of" 주석 · 리셋 뒤 옛 % 버그 수정 · 70% 경고의 맥락 | anthropics/claude-code CHANGELOG 본문 grep | 지지(버전 번호는 수집자 보고 — 미대조) |
| 리뷰어가 가져온 새 선례(TokenTelemetry·claude-usage-widget·AgentLimits) | 문서 페이지 · 각 README 직접 열람 | 지지 |
| 서드파티 임계값·간격(ClaudeBar·CodexBar·Usage4Claude 등), Paseo·t3code·Jean 상수 | 메인 미대조 — 수집자 보고 | 부분지지(단일 출처) |
| OAuth 엔드포인트 응답 모양 | 서드파티 코드뿐 | 가능성 높음 |

---

## 8. 적대 리뷰 결과

**리뷰어:** codex-cli 0.156.1(cross-family), effort high, 웹검색 켬, 레벨 2~3. **판정: BLOCK** — "선택지가 직접 관련된 작은 표시 선례를 빠뜨렸고, 한 후보가 보고서 자신의 요구를 못 채운다".

| # | 지적 | 유형 · 심각도 | 처리 |
|---|---|---|---|
| 1 | C 가 §5 의 "네 값 한눈에"를 못 채우는데 후보로 남았다 | 논리 공백 · high | **부분 반영** — 그 열은 사용자 요구가 아니라 내가 넣은 기준이었다. 사용자 요구는 상세를 툴팁에 두는 것을 허용하므로 C 를 빼지 않고, 열을 "트레이드오프"로 고쳐 적었다(§5) |
| 2 | TokenTelemetry(사이드바 상시 게이지 + hover 전체 + click 고정)·claude-usage-widget(막대 4개 / 2×2 링) 누락 | 누락 · high | **반영** — 둘 다 원문 확인. 형태 E 신설, D 에 선례 추가(§3·§5) |
| 3 | "5분이 규범, 5~15분이 기본"은 조회 주기와 캐시 신선도를 섞은 과장 | 논리 공백·과장 · high | **반영** — 표를 네 축으로 가르고 규범 주장을 철회(§0-4·§2) |
| 4 | `unifiedWindows` 로 두 창을 받는다는 서술이 과장 — SDK 타입에 없다 | 과장 · medium | **반영** — 캡처 1건·비문서·사라질 수 있음으로 명시(§4-2) |
| 5 | "슬롯 선례 없음"이 조사 범위를 넘는 주장 — 사이드바 게이지가 더 가깝다 | 누락 · medium | **반영** — "조사한 제품에서"로 좁히고 사이드바 게이지를 최근접 선례로(§0-6·§3-1) |
| 6 | "Claude Code 는 70% 에서 경고"는 과장 — 주간 리셋 뒤 버그 수정의 맥락이다 | 과장 · medium | **반영** — CHANGELOG 원문 확인(맥락 그대로 인용, §3-3) |
| 7 | 작은 게이지·hover 상세의 접근성 기준 누락(ARIA meter · WCAG 1.4.13) | 누락 · medium | **반영** — §3-5 신설, 제약 ⑤·선택지 6 추가 |
| 8 | 머리말은 리뷰를 했다는데 §8 이 비어 있다 | 근거 없음 · low | **반영** — 리뷰 전 초안의 머리말이었다. 이 절을 채움 |

**리뷰어의 독립 재확인(지지):** Claude `/usage` "% used" · Codex "% left" 와 남은 양을 채우는 막대 · Codex 적응형 폴링 60/30/15/5초와 경고 50/75/90/95 · Codex `primary` 가 5시간이 아닐 수 있음 · Orca 60/80·15분. **부분지지:** `unifiedWindows`(캡처엔 있고 타입엔 없음).

---

## 9. 한계·공백

- 스크린샷을 보지 않았다 — 레이아웃 서술은 README·문서·소스 텍스트 기준이다.
- claude.ai Settings → Usage · ChatGPT Codex 사용량 페이지 · Claude 데스크톱 앱은 로그인이 필요해 모양을 확인하지 못했다.
- `account/rateLimits/updated` 가 실제로 언제·얼마나 자주 오는지 모른다(실측에서 도착 안 함).
- Claude `rate_limit_event` 가 창마다 한 번씩 오는지, 대표 창 하나만 오는지 모른다.
- claude-squad · Superset 소스 · Windsurf · Warp 는 확인하지 못했다(검색 한도). opcode·Crystal·claudecodeui·emdash 는 코드 검색에서 흔적이 없었을 뿐이라 "없다"는 불확실하다. 리뷰어가 수집자들이 놓친 선례 셋을 찾았으므로 **누락이 더 있을 수 있다**.
- Codex 파일:줄 포인터는 2026-09-26 `main` 기준(SHA 고정 아님). Orca 는 로컬 클론 `df7460af` 기준.

## 출처 (주요)

- Claude Code statusline 문서 — https://code.claude.com/docs/en/statusline
- Claude Code CHANGELOG — https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md
- Codex TUI status — https://github.com/openai/codex/tree/main/codex-rs/tui/src/status · rate limit 경고 — https://github.com/openai/codex/blob/main/codex-rs/tui/src/chatwidget/rate_limits.rs
- Codex app-server 프로토콜 — https://github.com/openai/codex/tree/main/codex-rs/app-server-protocol
- Orca — https://github.com/stablyai/orca (로컬 클론 `I:\Engram_Workspace\opensource\orca`)
- TokenTelemetry — https://tokentelemetry.com/docs/features/plan-limits/
- t3code — https://github.com/pingdotgg/t3code · Paseo — https://github.com/getpaseo/paseo · Jean — https://github.com/coollabsio/jean · agent-deck — https://github.com/asheshgoplani/agent-deck
- CodexBar — https://github.com/steipete/CodexBar · ClaudeBar — https://github.com/tddworks/ClaudeBar · Usage4Claude — https://github.com/f-is-h/Usage4Claude · CodeZeno — https://github.com/CodeZeno/Claude-Code-Usage-Monitor · CodexMeter — https://github.com/raycalrui/CodexMeter · AgentLimits — https://github.com/Nihondo/AgentLimits · claude-usage-widget — https://github.com/bozdemir/claude-usage-widget
- ccstatusline — https://github.com/sirmalloc/ccstatusline · claude-hud — https://github.com/jarrodwatts/claude-hud · claude-pace — https://github.com/Astro-Han/claude-pace · ccusage — https://github.com/ccusage/ccusage
- Claude Agent SDK 타입 — `@anthropic-ai/claude-agent-sdk` `sdk.d.ts`
- OAuth 엔드포인트 모양(서드파티) — https://github.com/trickv/hass-claude-usage · https://github.com/lobehub/lobehub
- 접근성 — https://www.w3.org/WAI/ARIA/apg/patterns/meter/ · https://www.w3.org/WAI/WCAG22/Understanding/content-on-hover-or-focus.html
