# 에이전트 대화에서 오는 한도 정보를 피어들이 줍고 합치는 방식 (light)

| 항목 | 값 |
|---|---|
| **상태** | light — 수집 1명 + 메인 스팟체크 3건. 적대 리뷰 없음 |
| **날짜** | 2026-09-26 |
| **앞 문서** | `docs/research/usage-limit-display-ui-survey-2026-09-26.md` §2(갱신) · §4(데이터 모양)의 후속 |
| **왜** | 사용자 갱신 설계(2026-09-26): 대화에서 오는 한도 정보를 최대한 주워 갱신하고, 일정 시간 안 오면 자동 조회, 수동 버튼은 심플 표시·팝업 둘 다. 그 "줍기"를 남들이 어떻게 하는지 본다 |
| **확신도** | 확실 = 소스 직접 확인(수집자 + 메인 스팟체크) · 가능성 높음 = 수집자 단독 · 불확실 = 일부만 읽음 |

## 결론

1. **부분 갱신은 창 단위로 합친다.** 이벤트에 없는 창은 이전 값을 유지한다 — t3code(`apps/server/src/provider/providerUsageLimits.ts:60` `applyUsageLimitsUpdate`), Orca(statusLine 수신 시 빠진 창 유지), Claude-Code-Usage-Monitor(창별로 공식 값이 추정치를 덮음). 예외 agent-deck 은 매번 통째로 바꾸는데, Claude statusline 이 리셋 지난 창을 빼 주기 때문이라고 적었다. (확실 — t3code 스팟체크)
2. **조회 값과 주운 값의 시각을 비교하는 곳은 없다 — 나중에 쓴 쪽이 이긴다.** 단 **실패한 조회는 좋은 값을 덮지 않는다**(t3code·Orca). t3code 주석은 창별 시각 장부를 "막아 주는 1초 미만의 퇴행보다 코드가 더 많다"며 거부했다(`providerUsageLimits.ts:121`). (확실 — 스팟체크)
3. **주운 값으로 자동 조회를 미루는 곳은 Orca 하나다.** 대화에서 받은 값이 5분 안이면 자동 Claude 조회를 건너뛴다(`src/main/rate-limits/service.ts:1451-1461` — `isLiveClaudeUsageFresh`·`shouldSkipAutomatedClaudeFetch`). 15분 주기 자체는 그대로 돌고, 그 틱을 건너뛰는 방식이다. 사용자 설계("안 오면 그때 조회")와 같은 모양의 유일한 선례. (확실 — 스팟체크)
4. **Codex 쪽은 알림 내용을 화면 값으로 쓰지 않는 곳이 있다.**
   - Codex CLI: 흘러오는 알림은 경고·복구에만 쓰고 표시 값은 전체 조회(`account/rateLimits/read`)만 쓴다 — 주석 "must not overwrite status data"(`codex-rs/tui/src/chatwidget/rate_limits.rs:375`). 희소 알림에서 빠진 크레딧·플랜은 직전 전체 조회에서 채운다(`:248`). (확실 — 스팟체크)
   - CodexMeter: 알림이 오면 내용은 버리고 **전체 조회를 한 번 더 한다**(`CodexUsageService.swift:466-468`). (가능성 높음)
5. **Claude 0–1 값은 ×100 후 0–100 으로 자른다**(t3code). **`unifiedWindows` 는 t3code·Orca 둘 다 안 쓴다.** 쓰는 곳 하나(yourpapai/papai)가 "CLI 2.1.251 은 `rateLimitType` 없이 `unifiedWindows` 에만 수치를 싣는다 — CLI 자체 스키마에서 `@internal`"이라 적었고, 다른 곳(RobertIlisei/MARVIN)은 옛 CLI 에는 그 필드가 없었다고 적었다. → **버전마다 모양이 다르니 두 모양을 다 느슨하게 받아야 한다.** (가능성 높음 — 서드파티 주장)
6. **Codex 세션 기록 파일에서 한도를 줍는 곳은 드물다** — 작은 저장소 둘(aiexpedite-local-terminal · supermatrix). Orca 는 그 파일을 토큰 집계에만 읽는다. (불확실 — 검색 적중률 낮음)
7. **Claude 터미널 모드에서 줍는 방법 = 사용자 `~/.claude/settings.json` 의 statusLine 에 스크립트를 거는 것**(Orca·agent-deck·Usage Monitor). Orca 는 사용자가 이미 statusLine 을 쓰고 있으면 건드리지 않고, 한 번 설치한 뒤 사용자가 비우면 다시 깔지 않는다. (확실 — 수집자)
8. **오래됨 기준:** Usage Monitor 10분(지나면 추정치로), agent-deck 15분(표시 유지 + `(stale)`), Codex CLI 15분(문구), Orca 30분(폐기 · 레이트리밋 중이면 24시간).

## 한계

- light — 적대 리뷰 없음. 스팟체크는 결론 2·3·4 의 셋만.
- Orca 의 조회 성공 시 교체 규칙(`applyStalePolicy`)은 끝까지 읽지 않았다. Codex CLI 의 "알림이 조회 기한을 안 미룬다"는 호출부를 전부 보지 않아 가능성 높음.
- 세션 기록 파일 검색은 적중률이 낮아 "드물다"는 불확실하다.
- 로컬 클론 기준: t3code `5378f87f9`(2026-09-18) · Orca `df7460af`(2026-08-20). Codex 는 2026-09-26 master.
