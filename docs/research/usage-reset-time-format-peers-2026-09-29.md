# Reset-time notation in usage-display tools — windows with more than a day left (weekly) (measured on peers)

- **Status:** light survey (summary with sources) · 2026-09-29 · no adversarial review (light) · **decision = date** (user decision 2026-09-29 — compact display 「리셋 10/1 01:48」 (reset 10/1 01:48) · English 「resets Oct 1 01:48」)
- **Method:** 1 collector (Sonnet) read the sources. The main session re-verified the Orca · Paseo · t3code format functions in the local clones (grounding spot-check).
- **Companion survey:** `docs/research/usage-manual-refresh-peers-2026-09-29.md` (manual refresh) · the earlier survey `docs/research/usage-limit-display-ui-survey-2026-09-26.md:49`

## Conclusion

- **A date is the most common** — both vendor tools use a date (Codex CLI · Claude Code); among third-party tools, 2 use a date (t3code · CodexBar), 1 uses a weekday (Jean), and 2 use remaining time only (Orca · Paseo). None uses date + weekday. Places that use a date **always show a time with it**.
- **Order:** Only Codex puts the time first (`14:30 on 3 Oct`); the others (CodexBar · t3code · Claude Code · Jean) put the date or weekday first and the time after it.
- **The weekday trap:** a weekly window is at most 7 days, so a reset on the same weekday next week reads as 「오늘」 (today) — a date does not (main-session inference, not peer evidence).

## By product

| Product | Surface | Notation (original) | Source | Confidence |
|---|---|---|---|---|
| Codex CLI | `/status` | same day `14:30` · other day `14:30 on 3 Oct` · `(resets …)` | `openai/codex` `codex-rs/tui/src/status/helpers.rs` `format_reset_timestamp` · `status/card.rs` | certain |
| Claude Code | `/usage` | `Resets: Apr 28, 1:59pm (America/Los_Angeles)` · session window `12pm (…)` | `anthropics/claude-code` issue #52466 (user paste) | likely |
| t3code | web tooltip | `Resets {절대} · in 2h` — today = time · tomorrow = `tomorrow at …` · after that = `8/13 12:34 PM` (numeric date + time) | `pingdotgg/t3code` `apps/web/src/timestampFormat.ts:165-186` (main-session re-verified) · `apps/web/src/components/usage/UsageLimits.tsx:138-141` | certain |
| t3code | mobile compact display | `↻ 6d 7h` (remaining time) | `apps/mobile/src/features/usage/UsageLimitsPooled.tsx:147-174` | certain |
| CodexBar | menu reset line | same day: time · tomorrow: 「tomorrow」 + time · after that: month abbreviation + day + time (locale dependent) · can switch to a remaining-time mode | `steipete/CodexBar` `Sources/CodexBarCore/UsageFormatter.swift` `resetDescription` | certain (structure) · strings inferred |
| Jean | usage | within 24 hours `2:00 PM` · beyond that `Fri 2:00 PM` (weekday + time) | `src/lib/usage-format.ts:47-57` | certain |
| Orca | status-bar tooltip | `Resets in 6d 7h` (remaining time only) | `stablyai/orca` `src/shared/rate-limit-reset-format.ts:28-31` (main-session re-verified) | certain |
| Paseo | small bar | `resets 6d` (largest unit only) | `getpaseo/paseo` `packages/app/src/provider-usage/format.ts:12-28` (main-session re-verified) | certain |

## Gaps

Not looked at: ClaudeBar · Usage4Claude · claude-usage-widget · CodexMeter · AgentLimits · the claude.ai / chatgpt.com web usage pages · the Claude Code statusline.
