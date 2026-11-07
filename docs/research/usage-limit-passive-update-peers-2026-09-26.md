# How peers pick up and merge limit information that arrives in agent conversations (light)

| Item | Value |
|---|---|
| **Status** | light — 1 collector + 3 main-session spot-checks. No adversarial review |
| **Date** | 2026-09-26 |
| **Previous document** | follow-up to `docs/research/usage-limit-display-ui-survey-2026-09-26.md` §2 (refresh) · §4 (data shape) |
| **Why** | User refresh design (2026-09-26): pick up as much limit information as possible from the conversation to refresh, auto-fetch if none arrives within a set time, and a manual button in both the simple display and the popup. We look at how others do that "picking up" |
| **Confidence** | certain = source checked directly (collector + main-session spot-check) · likely = collector alone · uncertain = only partly read |

## Conclusion

1. **Partial updates are merged per window.** A window that is not in the event keeps its previous value — t3code (`apps/server/src/provider/providerUsageLimits.ts:60` `applyUsageLimitsUpdate`), Orca (windows missing from a received statusLine are kept), Claude-Code-Usage-Monitor (per window, the official value overwrites the estimate). The exception, agent-deck, replaces everything every time, and wrote that this is because the Claude statusline omits windows that are past their reset. (Certain — t3code spot-check)
2. **No one compares the timestamps of a fetched value and a picked-up value — the later write wins.** But **a failed fetch does not overwrite a good value** (t3code · Orca). A t3code comment rejected a per-window timestamp ledger on the grounds that it has more code than the sub-1-second regression it would guard against (`providerUsageLimits.ts:121`; back-translated from the Korean source). (Certain — spot-check)
3. **Orca is the only one that defers automatic fetching because of picked-up values.** If a value received from the conversation is within 5 minutes, the automatic Claude fetch is skipped (`src/main/rate-limits/service.ts:1451-1461` — `isLiveClaudeUsageFresh` · `shouldSkipAutomatedClaudeFetch`). The 15-minute cycle itself keeps running, and the method is to skip that tick. The only precedent with the same shape as the user's design ("fetch then, if none arrives"). (Certain — spot-check)
4. **On the Codex side, some places do not use the notification content as the displayed value.**
   - Codex CLI: streamed notifications are used only for warnings and recovery, and the displayed values come only from the full fetch (`account/rateLimits/read`) — comment "must not overwrite status data" (`codex-rs/tui/src/chatwidget/rate_limits.rs:375`). Credits and plan missing from a sparse notification are filled in from the previous full fetch (`:248`). (Certain — spot-check)
   - CodexMeter: when a notification arrives, it discards the content and **does one more full fetch** (`CodexUsageService.swift:466-468`). (Likely)
5. **Claude 0–1 values are multiplied by 100 and clamped to 0–100** (t3code). **Neither t3code nor Orca uses `unifiedWindows`.** The one place that does (yourpapai/papai) wrote (back-translated from the Korean source) that CLI 2.1.251 carries the numbers only in `unifiedWindows`, without `rateLimitType` — `@internal` in the CLI's own schema; another (RobertIlisei/MARVIN) wrote that older CLIs did not have that field. → **The shape differs by version, so both shapes must be accepted loosely.** (Likely — third-party claim)
6. **Picking up limits from Codex session log files is rare** — two small repositories (aiexpedite-local-terminal · supermatrix). Orca reads that file only for token totals. (Uncertain — low search hit rate)
7. **The way to pick up in Claude terminal mode = hook a script into statusLine in the user's `~/.claude/settings.json`** (Orca · agent-deck · Usage Monitor). Orca does not touch it if the user already uses a statusLine, and after installing once, it does not reinstall if the user clears it. (Certain — collector)
8. **Staleness thresholds:** Usage Monitor 10 minutes (after that, falls back to an estimate), agent-deck 15 minutes (keeps the display + `(stale)`), Codex CLI 15 minutes (wording), Orca 30 minutes (discarded · 24 hours while rate-limited).

## Limits

- light — no adversarial review. Spot-checks covered only three of the conclusions: 2, 3 and 4.
- Orca's replacement rule on a successful fetch (`applyStalePolicy`) was not read to the end. Codex CLI's "a notification does not postpone the fetch deadline" was not checked across all call sites, so it is likely.
- The session-log-file search had a low hit rate, so "rare" is uncertain.
- Local clone basis: t3code `5378f87f9` (2026-09-18) · Orca `df7460af` (2026-08-20). Codex is 2026-09-26 master.
