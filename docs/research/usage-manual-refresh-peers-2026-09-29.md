# 사용량 표시 도구의 수동 새로고침 — 위치 · 범위 · 쿨타임 (피어 실측)

- **상태:** light 조사(출처 포함 요약) · 2026-09-29 · 적대 리뷰 없음(light) · **결정 = ADR-0257 결정 1·3**(사용자 결정 2026-09-29 — ⟳ 는 작은 표시의 전역 버튼 하나 · 수동 30초 간격 삭제, 진행 중 합류와 거절 중 차단만 남김)
- **방법:** 수집자 1명(Sonnet — Opus 주간 한도로 대체)이 소스·README 를 읽었다. 메인이 Orca 핵심 클레임을 로컬 클론에서 재확인했다(grounding 스팟체크).
- **확신도:** 확실 = 소스에서 확인 · 가능성 높음 = 소스 일부 + 추론 · 불확실 / README 주장 = 소스 미확인
- **앞선 조사:** `docs/research/usage-limit-display-ui-survey-2026-09-26.md`(피어 목록 · 조회 주기 표 — 수동 새로고침 축은 거기 없었다)

## 결론

- **위치:** 아래 표 8개 중 상시 보이는 작은 표시에서 바로 누르는 것은 둘(Usage4Claude = 링·그래프 클릭 · Orca = 상태바 아이콘 버튼)이다. 셋은 한 번 더 펼친 메뉴·팝오버·우클릭 메뉴 안(CodexBar · CodexMeter · claude-usage-widget), 둘은 설정 화면 안(Paseo · AgentLimits)이다. 남는 하나(t3code — 모바일 사용량 화면의 당겨서 새로고침)는 이 셋 어디에도 넣지 않았다(2 + 3 + 2 = 7 · 분류 밖 1). 단축키는 찾지 못했다(따로 찾지는 않았다).
- **범위:** **전체 한 번에가 기본**이다(Orca · Paseo · AgentLimits · CodexMeter · Usage4Claude · claude-usage-widget — 여섯. 단 CodexMeter 는 Codex 한 회사뿐이라 「전체」 = 그 한 회사다). 회사별 버튼까지 있는 것은 CodexBar 하나만 확인했다(전체 + 회사별 메뉴 항목). t3code 는 범위를 확인하지 않았다(8 = 6 + 1 + 미확인 1). Orca 는 내부에 회사별 새로고침 통로가 있지만 화면 버튼은 전체다.
- **막기:** **조회 중 재누름 차단(버튼 비활성·무시·진행 중 요청 공유)은 확인한 곳엔 다 있다** — 소스로 확인 4(Orca · CodexBar · Paseo 서비스 · CodexMeter) · 가능성 높음 2(AgentLimits · claude-usage-widget) · 불확실 1(t3code — 새로고침 중 두 번째 호출 무시). Usage4Claude 는 README 의 10초 디바운스만 확인했고 조회 중 차단은 따로 확인하지 않았다 — 그래서 「8곳 전부」라고는 말하지 않는다. **시간 기반 쿨타임은 드물다** — Usage4Claude 10초 디바운스(README) 하나가 분명하고, Orca 는 상수 주석이 「수동 새로고침 몰림을 5분으로 디바운스」라 적지만 강제 조회가 그것을 건너뛰는지는 미확인이다.
- **429 중 수동:** 갈린다. Orca = 사용자가 직접 누른 조회만 Retry-After 를 넘는다(자동은 지킨다). Usage4Claude = 수동은 되지만 반복하면 대기가 늘어난다(README). 나머지는 미확인.

## 제품별

| 제품 | 위치 | 범위 | 막기 · 429 | 출처 | 확신도 |
|---|---|---|---|---|---|
| Orca | 상시 상태바의 RefreshCw 아이콘 버튼 | 전체(Claude+Codex+감지된 에이전트). 회사별·계정별 통로는 내부에만 | 조회 중 무시 · `force` 가 자동 조회 간격과 Retry-After 를 넘는다(「user-directed (force) fetches may bypass a provider's Retry-After gate」) · 자동 = 기본 15분, 최소 30초, `MIN_REFETCH_MS` 5분(「debounce resume/manual refresh bursts」) | `stablyai/orca` `src/renderer/src/components/status-bar/StatusBar.tsx:2090-2103` · `src/main/rate-limits/service.ts:75-91,1110` (로컬 클론, 메인 재확인) | 확실(위치·범위·조회 중 무시·force 우회) · 5분 디바운스의 수동 적용은 불확실 |
| CodexBar | 메뉴바 메뉴 안 「Refresh now」 + 회사별 메뉴 항목 | 전체 + 회사별 | 조회 중이면 되돌아간다 · 시간 쿨타임 없음 · 문서 「always available」 · 429 미확인 | `steipete/CodexBar` `Sources/CodexBar/StatusItemController+Actions.swift:138-195` · `docs/refresh-loop.md:20-23` | 확실(429 제외) |
| Paseo | 설정의 사용량 절 머리 버튼 | 전체 | `forceRefresh` 가 캐시를 넘는다 · 진행 중 요청 공유 · 429 미확인 | `getpaseo/paseo` `packages/server/src/services/quota-fetcher/service.ts:42-58` · `packages/app/src/provider-usage/settings-section.tsx:20-35` | 확실(서비스) · 버튼이 force 를 부르는지 미확인 |
| t3code | 모바일 사용량 화면 당겨서 새로고침 | 미확인 | 새로고침 중 두 번째 호출 무시 · 그 밖 미확인 | `pingdotgg/t3code` `apps/mobile/src/features/usage/UsageRouteScreen.tsx:125-149,246-249` | 불확실 |
| AgentLimits | 메인 설정 창 「Refresh now」 | 전체 | 조회 중 비활성 | `AgentLimits/Usage/ContentView.swift:198-201` | 가능성 높음 |
| CodexMeter | 팝오버 안 새로고침 버튼 | Codex 전용 | 조회 중 비활성 + 서비스도 겹침 방지 · 429 미확인 | `CodexMeter/ContentView.swift:176-188` · `CodexUsageService.swift:215-228` | 확실(막기) · 팝오버 위치는 가능성 높음 |
| Usage4Claude | 작은 위젯의 링·그래프 클릭 | 전체 | 수동 10초 디바운스 · 레이트리밋 백오프 · 수동 반복이 백오프를 늘린다 | README 100-106, 239 | README 주장 |
| claude-usage-widget | 우클릭 메뉴 「↻ Refresh」 | 전체 | 조회 중 플래그 · 자동 조회는 429 에 지수 백오프 · 수동 429 미확인 | `claude_usage/widget.py:1036,1125-1131,1533` | 가능성 높음 |

## 공백

- 안 본 것: Jean · ClaudeBar 화면(서비스엔 `refreshAll` 과 `refresh(providerId:)` 가 둘 다 있다 — 화면 호출 지점 미확인) · CodeZeno(README 에 새로고침 조작이 없다).
- CodexBar 의 429 처리 · t3code 웹 화면 · Orca Codex 경로의 Retry-After 우회 세부는 읽지 않았다.
