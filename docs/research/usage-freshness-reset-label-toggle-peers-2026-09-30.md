# 사용량 표시 — 「몇 분 전」 · 리셋 표기 · 켜고 끄기 피어 조사 (2026-09-30)

- **상태:** medium 으로 시작했으나 **적대 리뷰 생략 — cross-family 검증 없음**(세션 컨텍스트 한도로 중단, 수집자 결과 + 메인 grounding 스팟체크만). 결정은 이 조사 위에서 사용자가 내렸다(아래 「결정」).
- **방법:** 수집자 4갈래(① 신선도 표기 ② 리셋 표기 낱말 vs 아이콘 ③ 신선도·새로고침 합치기 관용성 ④ 막대 숨김 vs 상세 표시 스위치). 로컬 클론(`I:\Engram_Workspace\opensource\`) 소스와 GitHub 원본을 읽었다.
- **확신도:** 확실 = 소스 줄까지 확인 · 가능성 높음 = 문서·README·일부 소스 · 불확실 = 기억·검색 조각.
- **선행 조사:** [수동 새로고침 피어](usage-manual-refresh-peers-2026-09-29.md) · [리셋 시각 형식 피어](usage-reset-time-format-peers-2026-09-29.md).

## ① 값의 신선도(「1분 전」)

- **줄마다 붙이는 곳은 없다.** 회사(공급자)마다 한 번, 옅은 글자로 둔다 — CodexBar(`Sources/CodexBar/MenuCardView.swift:322-360`) · Orca(`src/renderer/src/components/status-bar/tooltip.tsx:34-45`)는 회사 머리 · Paseo 는 카드 아래(`footerText` — `packages/app/src/provider-usage/card.tsx:32-38` · 2026-09-30 정정: 처음엔 「머리」로 적었다). 확실(확인한 약 7개 한정).
- **평소엔 숨기고 오래됐을 때만 경고하는 곳도 있다.** Codex CLI `/status` = 15분 넘으면 흐린 경고 한 줄(`codex-rs/tui/src/status/rate_limits.rs:57-66`) · CodexBar Linux = `max(10분, 간격×2)` 넘으면 「Usage is out of date」(`Integrations/Linux/DesktopController.cpp:167-173`). 확실.
- **최근은 상대 시간, 24시간 넘으면 절대 시각**(CodexBar `UsageFormatter.swift:186-215`). 확실.
- **갱신 중:** CodexBar 는 그 자리 글자를 「Refreshing…」으로 바꾼다. 새로고침 단추는 따로다. 확실.

## ② 신선도와 새로고침을 한 요소로 합치나

- **합친 예(누르면 새로 받는 「⟳ 2분 전」)는 찾지 못했다.** 나란히 두는 예: AWS Cloudscape(<https://cloudscape.design/patterns/general/loading-and-refreshing/> — 새로고침 단추 + 완료 뒤 타임스탬프, 갱신 중에도 값 유지) · Smashing 「Data Freshness Indicator」(<https://www.smashingmagazine.com/2025/09/ux-strategies-real-time-dashboards/>). 가능성 높음(예시 수 적음 · 주요 디자인 시스템 대부분 「모름」).
- **호박색 기준에 대한 디자인 시스템 지침은 없다.** 선례는 위 CodexBar Linux `간격×2` 와 Codex CLI 15분.

## ③ 리셋 표기 — 낱말 vs 아이콘

- **새로고침 단추(RefreshCw)가 있는 도구는 낱말을 쓴다** — Orca(`github-rate-limit-display.tsx:164-216`) · Paseo(`window-bar.tsx:39-40`) · Codex CLI · Claude Code. 확실.
- **↻ 를 리셋에 쓰는 곳**은 새로고침 단추가 없거나(t3code 모바일 `UsageLimitsPooled.tsx:152,174` — 당겨서 새로고침) 같은 화살표라 섞인다(Usage4Claude `UsageDetailView.swift:403-426,505-515`). 확실.
- **낱말 없이 시각만:** CodexBar 메뉴바 「in 4d 1h」, 「Resets」 는 접근성 이름에만(`MenuBarLayoutRenderer.swift:939-950`). 확실.
- **모래시계는 「남은 시간」 표시에 쓰였고 리셋 시각 표시로 쓴 곳은 없다**(Usage4Claude 「⏳ 1h48m ↻ 15:07」 — 코드 주석 기준 · 가능성 높음).
- 화살표 없는 lucide 후보(1.23.0 에 실재 확인): Hourglass · AlarmClock · CalendarClock · ClockArrowUp. 피할 것: RotateCcw · RotateCw · History(RotateCcw 와 같은 선 + 시곗바늘) · Repeat.

## ④ 막대에서 숨김 vs 상세 표시 · 조회

- **막대에서 숨긴 회사를 팝업에만 남기는 도구는 없다.** CodexBar = `enabled` 하나(끄면 모든 곳에서 사라지고 조회 중단 · `UsageStore.swift:633-647` · 가능성 높음) · Orca = 막대와 팝업이 같은 목록(`StatusBar.tsx:2133-2162,2202-2211,2285-2308` · 확실), 조회는 계속하는 것으로 보임(가능성 높음).
- **스위치를 팝업 안에 둔 도구도 없다.** CodexBar·Stats = 설정 화면 · Orca = 상태 막대 우클릭 메뉴(`StatusBar.tsx:2440-2545`). Paseo·t3code 는 회사별 스위치 자체가 없다.

## 결정 (사용자 · 2026-09-30)

- 「몇 분 전」 = 회사 머리에 한 번, **시계 아이콘 없이 「1분 전 ⟳」**(아이콘 둘이 어수선 — ⟳ 가 시간 옆이면 새로고침으로 읽힌다). 팝업 ⟳ = **회사마다**(작은 표시의 전역 ⟳ 하나는 그대로).
- 리셋 = **모래시계 아이콘으로 통일**(작은 표시·팝업) · 툴팁·스크린리더는 「리셋 11:29」.
- 켜고 끄기 = 팝업 「슬롯에 표시」 유지 · 끈 회사는 **팝업에서도 사라진다**(관용과 같음).
- 호박색(오래됨) 기준 = **30분**(자동 간격 15분 × 2 · 코드 `STALE_AFTER_SECS` 와 같음 · CodexBar Linux 선례).
- 팝업을 열 때 = **새로 받지 않는다**(Orca 방식). 근거 — 주기 조회가 있는 도구는 열 때 안 받고(Orca: 열기 처리는 기록·포커스뿐 `StatusBar.tsx:2232-2238`, 다른 계정 펼칠 때만 받음 `:770-775` · 확실), 주기 조회가 없는 도구만 열 때 받는다(Paseo `components/context-window-meter.tsx:112-125` · Claude 웹 사용량 페이지는 추정). 우리는 주기 조회 쪽이다.
- 시안: `.claude/handoff/attachments/usage-options-mockup-2026-09-30.html` · `usage-reset-options-mockup-2026-09-30.html`.

## 공백

- 미확인: ClaudeBar · CodexMeter · GitHub Copilot 사용량 UI · 클라우드 콘솔 · Material/Fluent/Carbon/Primer 등 디자인 시스템 원문.
- 적대 리뷰 미실시 — 위 확신도는 수집자 자기보고 + 메인 스팟체크다.
