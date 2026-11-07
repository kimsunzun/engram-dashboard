# Manual refresh in usage-display tools — location · scope · cooldown (measured on peers)

- **Status:** light survey (summary with sources) · 2026-09-29 · no adversarial review (light) · **decision = ADR-0258 decisions 1 and 3** (user decision 2026-09-29 — ⟳ is one global button in the compact display · the manual 30-second interval is deleted, and only joining an in-flight fetch and blocking while rejected remain)
- **Method:** 1 collector (Sonnet — substituted because of the Opus weekly limit) read the sources and READMEs. The main session re-verified the core Orca claims in the local clone (grounding spot-check).
- **Confidence:** certain = verified in source · likely = part source + inference · uncertain / README claim = source not verified
- **Earlier survey:** `docs/research/usage-limit-display-ui-survey-2026-09-26.md` (peer list · fetch-interval table — the manual-refresh axis was not there)

## Conclusion

- **Location:** Of the 8 in the table below, two are pressed directly from the always-visible compact display (Usage4Claude = click on the ring/graph · Orca = status-bar icon button). Three are inside a menu · popover · right-click menu one step further in (CodexBar · CodexMeter · claude-usage-widget), and two are inside a settings screen (Paseo · AgentLimits). The one remaining (t3code — pull-to-refresh on the mobile usage screen) was not put in any of these three (2 + 3 + 2 = 7 · 1 outside the classification). No keyboard shortcut was found (we did not search for one separately).
- **Scope:** **All at once is the default** (Orca · Paseo · AgentLimits · CodexMeter · Usage4Claude · claude-usage-widget — six. But CodexMeter has only one provider, Codex, so 「전체」 (all) = that one provider). Only CodexBar was confirmed to have per-provider buttons as well (all + per-provider menu items). t3code's scope was not checked (8 = 6 + 1 + 1 unchecked). Orca has an internal per-provider refresh path, but the on-screen button is for all.
- **Blocking:** **Blocking a re-press during a fetch (button disabled · ignored · sharing the in-flight request) exists in every place we checked** — confirmed in source in 4 (Orca · CodexBar · Paseo service · CodexMeter) · likely in 2 (AgentLimits · claude-usage-widget) · uncertain in 1 (t3code — a second call during a refresh is ignored). For Usage4Claude, only the README's 10-second debounce was confirmed, and blocking during a fetch was not separately confirmed — so we do not say 「8곳 전부」 (all 8 places). **Time-based cooldowns are rare** — Usage4Claude's 10-second debounce (README) is the only clear one, and for Orca the constant's comment says 「수동 새로고침 몰림을 5분으로 디바운스」 (debounce manual-refresh bursts by 5 minutes), but whether a forced fetch skips it is unconfirmed.
- **Manual during a 429:** it splits. Orca = only fetches the user pressed directly pass the Retry-After (automatic ones respect it). Usage4Claude = manual works, but repeating it lengthens the wait (README). The rest are unconfirmed.

## By product

| Product | Location | Scope | Blocking · 429 | Source | Confidence |
|---|---|---|---|---|---|
| Orca | RefreshCw icon button in the always-visible status bar | All (Claude+Codex+detected agents). Per-provider and per-account paths are internal only | ignored during a fetch · `force` passes the automatic fetch interval and the Retry-After (「user-directed (force) fetches may bypass a provider's Retry-After gate」) · automatic = default 15 minutes, minimum 30 seconds, `MIN_REFETCH_MS` 5 minutes (「debounce resume/manual refresh bursts」) | `stablyai/orca` `src/renderer/src/components/status-bar/StatusBar.tsx:2090-2103` · `src/main/rate-limits/service.ts:75-91,1110` (local clone, main-session re-verified) | Certain (location · scope · ignored during a fetch · force bypass) · whether the 5-minute debounce applies to manual refresh is uncertain |
| CodexBar | 「Refresh now」 inside the menu-bar menu + per-provider menu items | All + per provider | returns early if a fetch is in progress · no time cooldown · docs 「always available」 · 429 unconfirmed | `steipete/CodexBar` `Sources/CodexBar/StatusItemController+Actions.swift:138-195` · `docs/refresh-loop.md:20-23` | Certain (except 429) |
| Paseo | button in the header of the usage section in settings | All | `forceRefresh` bypasses the cache · shares the in-flight request · 429 unconfirmed | `getpaseo/paseo` `packages/server/src/services/quota-fetcher/service.ts:42-58` · `packages/app/src/provider-usage/settings-section.tsx:20-35` | Certain (service) · whether the button calls force is unconfirmed |
| t3code | pull-to-refresh on the mobile usage screen | unconfirmed | a second call during a refresh is ignored · the rest unconfirmed | `pingdotgg/t3code` `apps/mobile/src/features/usage/UsageRouteScreen.tsx:125-149,246-249` | Uncertain |
| AgentLimits | 「Refresh now」 in the main settings window | All | disabled during a fetch | `AgentLimits/Usage/ContentView.swift:198-201` | Likely |
| CodexMeter | refresh button inside the popover | Codex only | disabled during a fetch + the service also prevents overlap · 429 unconfirmed | `CodexMeter/ContentView.swift:176-188` · `CodexUsageService.swift:215-228` | Certain (blocking) · popover location likely |
| Usage4Claude | click on the ring/graph of the small widget | All | manual 10-second debounce · rate-limit backoff · repeated manual presses extend the backoff | README 100-106, 239 | README claim |
| claude-usage-widget | right-click menu 「↻ Refresh」 | All | in-progress flag · automatic fetching uses exponential backoff on 429 · manual 429 unconfirmed | `claude_usage/widget.py:1036,1125-1131,1533` | Likely |

## Gaps

- Not looked at: the Jean · ClaudeBar screens (the service has both `refreshAll` and `refresh(providerId:)` — the call site from the screen is unconfirmed) · CodeZeno (the README has no refresh action).
- CodexBar's 429 handling · the t3code web screen · the details of the Orca Codex path's Retry-After bypass were not read.
