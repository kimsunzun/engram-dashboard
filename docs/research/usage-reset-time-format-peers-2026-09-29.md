# 사용량 표시 도구의 리셋 시각 표기 — 하루 넘게 남은 창(주간) (피어 실측)

- **상태:** light 조사(출처 포함 요약) · 2026-09-29 · 적대 리뷰 없음(light) · **결정 = 날짜**(사용자 결정 2026-09-29 — 작은 화면 「리셋 10/1 01:48」 · 영어 「resets Oct 1 01:48」)
- **방법:** 수집자 1명(Sonnet)이 소스를 읽었다. 메인이 Orca · Paseo · t3code 포맷 함수를 로컬 클론에서 재확인했다(grounding 스팟체크).
- **짝 조사:** `docs/research/usage-manual-refresh-peers-2026-09-29.md`(수동 새로고침) · 앞선 서베이 `docs/research/usage-limit-display-ui-survey-2026-09-26.md:49`

## 결론

- **날짜가 가장 흔하다** — 벤더 도구 둘 다 날짜(Codex CLI · Claude Code), 서드파티는 날짜 2(t3code · CodexBar) · 요일 1(Jean) · 남은 시간만 2(Orca · Paseo). 날짜+요일은 없다. 날짜를 쓰는 곳은 **항상 시각을 함께** 쓴다.
- **어순:** Codex 만 시각 먼저(`14:30 on 3 Oct`)이고 나머지(CodexBar · t3code · Claude Code · Jean)는 날짜·요일 먼저, 시각 뒤다.
- **요일의 함정:** 주간 창은 최대 7일이라 다음 주 같은 요일의 리셋이 「오늘」로 읽힌다 — 날짜는 그렇지 않다(메인 추론, 피어 근거 아님).

## 제품별

| 제품 | 표면 | 표기(원문) | 출처 | 확신도 |
|---|---|---|---|---|
| Codex CLI | `/status` | 같은 날 `14:30` · 다른 날 `14:30 on 3 Oct` · `(resets …)` | `openai/codex` `codex-rs/tui/src/status/helpers.rs` `format_reset_timestamp` · `status/card.rs` | 확실 |
| Claude Code | `/usage` | `Resets: Apr 28, 1:59pm (America/Los_Angeles)` · 세션 창 `12pm (…)` | `anthropics/claude-code` issue #52466(사용자 붙여넣기) | 가능성 높음 |
| t3code | 웹 툴팁 | `Resets {절대} · in 2h` — 오늘 = 시각 · 내일 = `tomorrow at …` · 그 뒤 = `8/13 12:34 PM`(숫자 날짜 + 시각) | `pingdotgg/t3code` `apps/web/src/timestampFormat.ts:165-186`(메인 재확인) · `apps/web/src/components/usage/UsageLimits.tsx:138-141` | 확실 |
| t3code | 모바일 작은 표시 | `↻ 6d 7h`(남은 시간) | `apps/mobile/src/features/usage/UsageLimitsPooled.tsx:147-174` | 확실 |
| CodexBar | 메뉴 리셋 줄 | 같은 날 시각 · 내일 「tomorrow」+시각 · 그 뒤 월 약칭+일+시각(로케일 의존) · 남은 시간 모드로 전환 가능 | `steipete/CodexBar` `Sources/CodexBarCore/UsageFormatter.swift` `resetDescription` | 확실(구조) · 문자열은 추론 |
| Jean | 사용량 | 24시간 안 `2:00 PM` · 그 밖 `Fri 2:00 PM`(요일+시각) | `src/lib/usage-format.ts:47-57` | 확실 |
| Orca | 상태바 툴팁 | `Resets in 6d 7h`(남은 시간만) | `stablyai/orca` `src/shared/rate-limit-reset-format.ts:28-31`(메인 재확인) | 확실 |
| Paseo | 작은 막대 | `resets 6d`(가장 큰 단위 하나) | `getpaseo/paseo` `packages/app/src/provider-usage/format.ts:12-28`(메인 재확인) | 확실 |

## 공백

ClaudeBar · Usage4Claude · claude-usage-widget · CodexMeter · AgentLimits · claude.ai / chatgpt.com 웹 사용량 페이지 · Claude Code statusline 은 보지 않았다.
