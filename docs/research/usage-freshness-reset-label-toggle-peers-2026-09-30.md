# Usage display — 「몇 분 전」 (minutes ago) · reset label · on/off toggle peer survey (2026-09-30)

- **Status:** started at medium but **adversarial review skipped — no cross-family verification** (stopped at the session context limit; only collector results + a main-session grounding spot-check). The user made the decision on top of this survey (「결정」 (Decision) below).
- **Method:** 4 collector tracks (① freshness label ② reset label: words vs icon ③ how conventional it is to merge freshness and refresh ④ switch: hide from the bar vs show in detail). We read the local-clone sources (`I:\Engram_Workspace\opensource\`) and the GitHub originals.
- **Confidence:** certain = verified down to the source line · likely = docs · README · part of the source · uncertain = memory · search fragments.
- **Earlier surveys:** [manual refresh peers](usage-manual-refresh-peers-2026-09-29.md) · [reset time format peers](usage-reset-time-format-peers-2026-09-29.md).

## ① Freshness of the value (「1분 전」 (1 minute ago))

- **No one attaches it per row.** It is placed once per provider (company), in faint text — CodexBar (`Sources/CodexBar/MenuCardView.swift:322-360`) · Orca (`src/renderer/src/components/status-bar/tooltip.tsx:34-45`) put it in the provider header · Paseo puts it below the card (`footerText` — `packages/app/src/provider-usage/card.tsx:32-38` · corrected 2026-09-30: first written as 「머리」 (header)). Certain (limited to the roughly 7 checked).
- **Some hide it normally and warn only when the value is old.** Codex CLI `/status` = one dim warning line after 15 minutes (`codex-rs/tui/src/status/rate_limits.rs:57-66`) · CodexBar Linux = 「Usage is out of date」 once past `max(10분, 간격×2)` (`Integrations/Linux/DesktopController.cpp:167-173`). Certain.
- **Recent values use relative time; past 24 hours, absolute time** (CodexBar `UsageFormatter.swift:186-215`). Certain.
- **While refreshing:** CodexBar replaces the text in that spot with 「Refreshing…」. The refresh button is separate. Certain.

## ② Whether freshness and refresh are merged into one element

- **We found no example that merges them (a 「⟳ 2분 전」 (⟳ 2 minutes ago) that fetches anew when pressed).** Examples that place them side by side: AWS Cloudscape (<https://cloudscape.design/patterns/general/loading-and-refreshing/> — refresh button + timestamp after completion, value kept during refresh) · Smashing 「Data Freshness Indicator」 (<https://www.smashingmagazine.com/2025/09/ux-strategies-real-time-dashboards/>). Likely (few examples · most major design systems are 「모름」 (unknown)).
- **There is no design-system guidance on the amber threshold.** The precedents are CodexBar Linux `간격×2` above and Codex CLI's 15 minutes.

## ③ Reset label — words vs icon

- **Tools that have a refresh button (RefreshCw) use words** — Orca (`github-rate-limit-display.tsx:164-216`) · Paseo (`window-bar.tsx:39-40`) · Codex CLI · Claude Code. Certain.
- **Places that use ↻ for reset** either have no refresh button (t3code mobile `UsageLimitsPooled.tsx:152,174` — pull to refresh) or get mixed up because it is the same arrow (Usage4Claude `UsageDetailView.swift:403-426,505-515`). Certain.
- **Time only, no words:** CodexBar menu bar 「in 4d 1h」, with 「Resets」 only in the accessible name (`MenuBarLayoutRenderer.swift:939-950`). Certain.
- **The hourglass has been used to show 「남은 시간」 (time remaining), and no one uses it to show the reset time** (Usage4Claude 「⏳ 1h48m ↻ 15:07」 — based on a code comment · likely).
- lucide candidates without an arrow (confirmed to exist in 1.23.0): Hourglass · AlarmClock · CalendarClock · ClockArrowUp. Avoid: RotateCcw · RotateCw · History (the same line as RotateCcw + clock hands) · Repeat.

## ④ Hide from the bar vs show in detail · fetching

- **No tool hides a provider from the bar but keeps it only in the popup.** CodexBar = a single `enabled` (turning it off removes it everywhere and stops fetching · `UsageStore.swift:633-647` · likely) · Orca = the bar and the popup share the same list (`StatusBar.tsx:2133-2162,2202-2211,2285-2308` · certain), and fetching appears to continue (likely).
- **No tool puts the switch inside the popup either.** CodexBar·Stats = settings screen · Orca = status-bar right-click menu (`StatusBar.tsx:2440-2545`). Paseo·t3code have no per-provider switch at all.

## Decision (user · 2026-09-30)

- 「몇 분 전」 = once in the provider header, **「1분 전 ⟳」 without a clock icon** (two icons are cluttered — a ⟳ next to the time reads as refresh). Popup ⟳ = **per provider** (the one global ⟳ in the compact display stays).
- Reset = **unified on the hourglass icon** (compact display · popup) · tooltip · screen reader say 「리셋 11:29」 (reset 11:29).
- On/off = keep 「슬롯에 표시」 (show in slot) in the popup · a provider that is turned off **also disappears from the popup** (same as the convention).
- Amber (stale) threshold = **30 minutes** (automatic interval 15 minutes × 2 · same as the code's `STALE_AFTER_SECS` · CodexBar Linux precedent).
- When the popup opens = **do not fetch anew** (Orca's way). Reason — tools that have periodic fetching do not fetch on open (Orca: open handling is only logging · focus `StatusBar.tsx:2232-2238`, and it fetches only when another account is expanded `:770-775` · certain), and only tools without periodic fetching fetch on open (Paseo `components/context-window-meter.tsx:112-125` · for the Claude web usage page this is presumed). We are on the periodic-fetching side.
- Mockups: `.claude/handoff/attachments/usage-options-mockup-2026-09-30.html` · `usage-reset-options-mockup-2026-09-30.html`.

## Gaps

- Unchecked: ClaudeBar · CodexMeter · GitHub Copilot usage UI · cloud consoles · original texts of design systems such as Material/Fluent/Carbon/Primer.
- No adversarial review was done — the confidence labels above are the collectors' self-report + a main-session spot-check.
