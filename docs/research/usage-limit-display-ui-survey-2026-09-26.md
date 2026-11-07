# Claude·Codex usage limit (5-hour · weekly) display UI — precedent survey

| Item | Value |
|---|---|
| **Status** | Final (adversarial review applied) — cross-family review verdict BLOCK → of 8 points, 7 applied and 1 partially applied (§8). Choices such as the layout are **awaiting a user decision** (§6) |
| **Date** | 2026-09-26 |
| **Method** | 4 collection tracks (official products · third-party monitors · agent managers/IDEs · response fields) + 2 measurements (codex app-server `account/rateLimits/read` · `claude -p` stream-json) + main-session grounding spot-check (§7) + 1 round of cross-family adversarial review (codex-cli 0.156.1, effort high, web search on — §8) |
| **Confidence legend** | **certain** = two or more independent sources (measurement + source, collector + reviewer independently re-checking, etc.) · **likely** = one source supports it · **uncertain** = unsupported · search snippets only |
| **Why we did this survey** | We want to add a new type to a webview (frontend) slot to show the Claude·Codex 5-hour·weekly limits in small form. The user does not yet have a layout in mind, so we first look at what others **show · how they refresh · what shape it takes** |
| **Out of scope** | **Choosing** the data source per vendor (user decision 2026-09-26: "per-backend data later"). §4 only writes "what comes back if you ask" |
| **User decisions made from this survey** | The 「사용자 결정」 (User decisions) section below (2026-09-26~27 interview) |

## User decisions (2026-09-26~27 interview)

- **Percentage direction = the remaining amount** (the Codex way — the bar shrinks as you use). Claude gives utilization, so subtract it from 100.
- **Compact display (always visible)** = per provider, both windows (5-hour·weekly) · bar + % + **remaining time**. Depending on the slot width it shrinks from one line per provider → one line per window → numbers only. **Claude·Codex can each be turned on and off.** A manual refresh ⟳ is provided.
- **Click popup (detail)** = shows everything · **target time** (reset time) · plan (only when there is a value) · **each provider's usage settings link** (Codex `chatgpt.com/codex/settings/usage` · Claude `claude.ai/settings/usage`) · ⟳. **No exhaustion forecast is included.** The popup may float outside the slot (the same `position: fixed` approach as the existing slot right-click menu).
- **Refresh** = pick up the limit information that arrives during the conversation, **reset the cooldown when both windows come in fresh**, auto-fetch when the cooldown has passed, and a manual ⟳. **Cooldown 5 minutes** (Claude 15 minutes — 2026-09-27).
  - Where we pick up = **only the `rate_limit_event` in Claude JSON mode · the Codex app-server notifications** (streams we already read — extra cost 0). **Terminal mode picks up for neither provider** — we dropped both the Claude statusline (a process starts for every message) and reading the Codex session log file (the file ADR-0203 decided not to read).
- **Link name = 「사용량 페이지 ↗」** (usage page ↗) + a description of the destination on mouse hover. **Localization premise** — all text is `t(키)` (ADR-0069), dates · times use `Intl` locale formatting, and shrinking the width is based on the actual rendered width, not the character count.
- **Color criteria = the third-party majority way** (based on the remaining amount): above 50% green · 50~20% yellow · below 20% red + ⚠. For e-ink, yellow = hatching, red = black fill + thick border + ⚠.
- **On/off = only in the slot right-click menu** (⟳ refresh · ☑ Claude shown · ☑ Codex shown). The popup is view-only. If both are off, the compact display shows one line of guidance, "우클릭해서 표시할 항목 고르기" (right-click to choose what to show).
- **Storage = not for now.** The on/off state goes inside the slot content so it has the same lifetime as the layout (the layout does not yet have disk persistence — CLAUDE.md 「LLM-우선 제어」 (LLM-first control)). When layout persistence arrives, it will be stored along with it. ★**Correction 2026-10-04 — layout persistence now exists**★ (`shell\state\state.json` — TRD `docs/process/S21-storage/trd.md` §6 · storage management P3b3 `f9db35a`): the on/off state (`SlotContent::Usage { show_claude, show_codex }`) is carried together with the slot content (TRD §6-0). The preceding sentence was the fact at the time of the survey.
- **Claude active fetch = A confirmed (2026-09-27 — CLI `get_usage`, probe behavior confirmed · B if there is a problem)** · the earlier tentative option = B (read the Claude Code token file and call the non-public usage address — the way 7 of 8 peers do it, 2026-09-27). Basis · policy quotes = `claude-usage-query-method-survey-2026-09-27.md`. ★B overlaps in shape with a policy sentence — legal · security confirmation is needed before release (this document does not judge)★.
- Remaining decisions: none — the source of truth for requirements is `docs/process/S21-usage-limit-slot/prd.md`.

---

## 0. Conclusion

1. **The core information is the same everywhere — two windows (5-hour · weekly) × {utilization %, reset time}.** Per-model weekly limits · plan · credits are **extra rows that are added only when present.** (certain)
2. **The percentage direction is opposite by vendor.** Claude Code `/usage` is "% used" (the CHANGELOG has "Switched `/usage` back to "% used"" — a trace of trying another direction and reverting), and Codex `/status` is "% left". Mixing the two on one screen makes the same bar mean the opposite. Managers · third parties **unify on one or provide a Used/Remaining toggle.** (certain — the reviewer independently re-checked)
3. **A small always-visible display + detail on hover/click, a two-tier structure, is the most common.** The always-visible part is numbers · a mini bar · a single gauge, and the detail (per-window rows · absolute reset time · refresh time · errors) opens in a tooltip · popover. Some places let you **pin it with a click** so it can be read (TokenTelemetry). (likely — based on the products surveyed)
4. **The refresh figures do not converge on one value.** The actual fetch intervals are spread from **60 seconds (Codex TUI · claude-usage-widget · CodexMeter background) ~ 15 minutes (Orca · CodeZeno) ~ adaptive 30 minutes (CodexBar)**, and some fetch **only on open** (Paseo · ClaudeBar default). What they have in common is the structure, not a figure — **fetch interval · cache freshness · countdown re-render · push merge run separately on different axes.** (likely)
5. **Stale values are marked, not erased.** They keep the last good value and signal it with "N minutes ago" · "Cached" · ⚠ · dimming. After the reset time has passed, the old % is not left (Claude Code fixed that bug). (certain)
6. **We found no precedent for placing it in one cell (slot) of a split layout in the products surveyed.** The closest are the **always-visible gauge at the bottom of the sidebar** (TokenTelemetry — colored by the tightest of all agents' windows, with the whole list opening on hover) and a status-bar cell (Orca). Since a slot's size changes with the user's split, we must borrow a **form that shrinks with width** (Orca verbose→compact→narrow, Codex `/status` dropping the bar and keeping the % when narrow). (likely — interpretation)
7. **The data shape is also almost the same for the two vendors** — per window {utilization, reset time}. But the unit differs by source (Claude mixes 0–1 and 0–100, ISO and epoch). Codex's `primary`/`secondary` are **not always 5-hour/weekly** — they must be classified by the window length (`windowDurationMins`) (measured). (certain)

---

## 1. What is shown

### 1-1. The core four

| Value | Official | Third-party · managers |
|---|---|---|
| 5-hour window utilization | Claude `/usage` "Current session", Codex `/status` "5h limit" | all |
| Weekly window utilization | Claude "Current week (all models)", Codex "Weekly limit" | all |
| Reset time of each window | **absolute time** — Codex `resets 14:30` / on another day `14:30 on 3 Oct`, Claude `Resets 9:40pm (TZ)` | **countdown is the default** (`resets in 3h 54m`, ClaudeBar `2d`/`4:40`/`45m`/`soon`), absolute time as a second tooltip line (t3code `Resets <abs> · in 2h`, Jean `Fri 2:00 PM`) |

### 1-2. Direction — used vs left

| Product | Direction | Note |
|---|---|---|
| Claude Code `/usage` | used | CHANGELOG "Switched `/usage` back to "% used"" |
| Codex CLI `/status` · footer | left | `[████░░…] 72% left (resets 14:30)` — the 20-cell bar fills with the **remaining amount** |
| Orca | used by default + toggle | even when showing the remaining side, the **bar color is based on used** |
| t3code | left | "as Codex does" |
| Paseo · Jean | used | |
| AgentLimits | toggle (shared by app · widget) | |
| ClaudeBar · CodeZeno · ccstatusline · claude-hud · Usage4Claude | toggle | many third parties leave it to the user's choice |

### 1-3. Additional information (only when present · mostly inside the tooltip)

- Per-model weekly limits — Claude (Sonnet only · Fable, etc.), Codex (extra limit bucket). claude-usage-widget **automatically attaches and detaches** a third bar only when the API reports a per-model window.
- Plan name — Codex `planType` is in every snapshot; Claude has it only in some sources (§4).
- Extra usage · credits — Claude Extra usage, Codex credits · free reset coupons (`rateLimitResetCredits`).
- **Pace** — answers "will it reset before it is used up" by "utilization relative to the elapsed window time". Several mature tools carry it:
  - t3code — a time-elapsed hairline above the bar + an ahead/on/under glyph (±5pt).
  - AgentLimits — "Pacemaker": compared with the elapsed %, green (on time · ahead) · orange (slightly over) · red (over by 10%p or more), and the time bar is split into 5 cells for 5 hours · 7 cells for weekly.
  - ClaudeBar — a burn-rate 1.5× warning. CodexBar — "On pace / X% in deficit". claude-pace — `⇣15%`/`⇡15%`.
- Refresh time — "Updated Xm ago" (Orca · Paseo), "Last updated: just now" (claude.ai), "Cached" (TokenTelemetry).

### 1-4. Empty and error states

- **No auth / unsupported account** — text instead of a bar: Codex "not available for this account", Orca "Sign in to see usage", t3code "This account has no subscription limits.". TokenTelemetry **does not treat being logged out as an error** (the orange warning only when the fetch is actually broken).
- **No data yet** — Codex "data not available yet", ClaudeBar `—` + "Waiting for quota data…", Orca a `···` pulse.
- **Failed but a previous value exists** — keep the previous value + indicate it: Orca ⚠ "Refresh failed — showing cached data", CodexBar dims the icon, Claude Code an "as of" note.
- t3code **tells "unsupported (bar emptied)" apart from "fetch failed (probeFailed — previous bar kept)"**. TokenTelemetry also separates "nothing to report / refresh needed (failed) / no limit".
- **Only one window arrives** — AgentLimits: if Codex returns only the weekly one, the 5-hour cell is left as "none" and the value goes into the weekly cell. CodexBar: if the selected window is absent, it does not substitute another window and shows `—`.
- **A 0% row is drawn too** — leaving it out reads as a "limit that does not exist" (Claude Code VS Code meter bug fix — collector report, version number not cross-checked).

---

## 2. How it is refreshed

★**The table below separates four axes**★ — fetch interval (the actual interval at which it is fetched) · cache freshness (within it, re-fetching is skipped) · push (updates that come from agent traffic) · re-render (the countdown tick). Do not compare the number in one cell with another cell.

| Product | Fetch interval | Cache · freshness | push | Other |
|---|---|---|---|---|
| Codex CLI TUI | 60 seconds → 75%+ 30 seconds · 90%+ 15 seconds · 99%+ 5 seconds | "limits may be stale" after 15 minutes | app-server `account/rateLimits/updated` (sparse — merged, null does not erase) | |
| claude-usage-widget | 60 seconds, backs off up to 300 seconds on a rate limit | — | — | returns to the original interval after a clean fetch |
| CodexMeter | background 60 seconds · menu open 10 seconds | — | on a Codex rate-limit update | backs off up to 5 minutes on failure, previous value shown as stale |
| TokenTelemetry | the gauge reads every minute | server cache 5 minutes (`quotas.json`) | — | Refresh skips the cache |
| AgentLimits | 1–10 minutes (user setting) | — | — | Refresh Now |
| Usage4Claude | smart: 1 minute while changing → 3/5/10 minutes when still | — | — | on wake · window open, manual click has a 10-second debounce |
| t3code | 5 minutes | 5-minute floor when the Usage page is opened | Claude `rate_limit_event` · Codex `account/rateLimits/updated` **merged by window id** | |
| Jean | 5 minutes (based on the snapshot `fetchedAt`) | — | — | the countdown every minute |
| Orca | 15 minutes (clamped to a minimum of 30 seconds) | re-request debounce 5 minutes · stale values discarded after 30 minutes (24 hours if rate-limited) | Claude statusLine `rate_limits` (15-second · 30-second dedup per panel) | refreshes on window focus, stops while hidden · minimized, backoff 30 seconds→15 minutes on failure |
| CodeZeno (Windows) | 15 minutes by default (1 minute/5 minutes/15 minutes/1 hour) | `Retry-After` cooldown up to 24 hours | — | |
| CodexBar | adaptive 2/5/15/30 minutes (shorter the more recently the menu was opened) | — | — | 30 minutes under low power · heat, "Refresh now" always available |
| Paseo | **no interval — only when the tooltip is opened** | client 5-minute stale time + server 5-minute cache | — | |
| ClaudeBar | off by default (only when the popover is opened) | API mode cache ≥15 minutes | — | on a 429 `Retry-After` (5 minutes if absent) |
| Claude Code statusline | (optional) `refreshInterval` | — | on every assistant message · when a window's `resets_at` is reached | 300ms debounce, windows past their reset are left out |

**Structural commonalities (not figures):**

- **If there is push, it is merged** (t3code · Orca · Codex TUI) — updates are sparse, so it is **a per-window merge, not an overwrite**.
- **The countdown re-render runs separately from the data refresh** — Orca schedules the tick at the next unit boundary (the minute) (`src/shared/rate-limit-reset-format.ts`), and Jean does it every minute.
- **On failure it backs off and keeps the previous value** (Orca · CodexMeter · claude-usage-widget · ClaudeBar · CodeZeno).
- **When the reset time passes, the old % is cleared** — Claude Code CHANGELOG "Fixed the status line `rate_limits` fields and `/usage` still showing a rate-limit window's pre-reset usage percentage after the window reset while the session was idle".
- There is a claim that the Anthropic usage endpoint rejects for a long time after a 429 (ClaudeBar README — **not confirmed by the vendor**). Look at it again when choosing the data source.

---

## 3. What shape it takes

### 3-1. Where it is placed

| Place | Examples |
|---|---|
| Always-visible gauge at the bottom of the sidebar | TokenTelemetry (all pages, above the notification bell) |
| App chrome status bar — one cell per vendor | Orca (bottom status bar `ProviderSegment`), VS Code Codex "Rate limits remaining" |
| Toolbar/dock badge | Jean `9|41%` (session|weekly) |
| Inside another gauge's tooltip | Paseo (the context window ring tooltip — **only the active agent's vendor**), Conductor (context usage hover card) |
| Sidebar section | Claude Code VS Code "ACCOUNT & USAGE" meter + "View details" |
| Dedicated page · tab | t3code `/usage`, Superset Usage tab, Paseo settings page, claude.ai Settings → Usage |
| Desktop overlay (always on top) | claude-usage-widget OSD — click for a detail popup |
| Menu bar · tray · widget | CodexBar, ClaudeBar, Usage4Claude, AgentLimits (second line of the menu bar `X% / Y%`), CodeZeno (taskbar widget, 2 rows) |

**We found no precedent for placing it in one cell (slot) of a split layout in the products surveyed** (survey scope = the table above + the unconfirmed list in §9). Nor is there any place that attaches a limit gauge per agent (Paseo · Conductor filtering by the active agent's vendor is the closest).

### 3-2. Fitting two windows into a small space

| Method | Examples | Character |
|---|---|---|
| Stack of per-window rows (label · thin bar · % · reset) | Orca tooltip `ProviderPanel` (6px bar), Codex `/status`, t3code row grid (6px), Paseo card (4px), claude-usage-widget Bars view (turning Codex on stretches the bars downward) | the most common **detail** form |
| Two windows on one line | ClaudeBar `5h 62% · 4:40 | 7d 34% · 2d` (a color per window, the combined label takes the color of the worse side), Jean `session|weekly%`, AgentLimits `X% / Y%`, Codex footer `5h 72% left` `weekly 40% left` | the most common **small** form |
| Single gauge — colored by the tightest window | TokenTelemetry (the one closest to its limit **among all agents · all windows**), Orca status bar (the tightest window per vendor, 48×6px bar + %), CodexMeter | the smallest — the rest on hover |
| Weekly only when dangerous | claude-hud (7d only at 80% or more) | conditional display |
| Ring | CodexMeter concentric (outer = remaining limit, inner = remaining time), Usage4Claude, claude-usage-widget Gauge view (Claude+Codex = **2×2 ring grid**) | for square spots |

**Shrinking with width:** Orca = verbose (icon + bar + text) → compact (icon + %) → narrow (one-character badge). Codex `/status` = when narrow, drops the bar and keeps the %, and **does not cut off the reset time but wraps it to the next line** ("the actionable part of this row").

### 3-3. Color thresholds

| Product | Basis | Ranges |
|---|---|---|
| Orca | used | <60 **achromatic** (keeps the always-on chrome quiet) · 60–79 yellow · ≥80 red |
| Paseo | used | ≥70 warning · >90 danger |
| ClaudeBar · claude-codex-battery | remaining | ≥50 normal · 20–50 warning · <20 danger (= used 50/80) |
| cfranci extension · "Claude Code Limits" extension | used | <50 green · 50–80 orange · >80 red |
| AgentLimits · t3code | **pace** | relative to elapsed time instead of a fixed threshold (AgentLimits: over by 10%p = red · t3code: ±5pt glyph, no urgency color) |
| Codex CLI | used (notifications) | warning text at 50/75/90/95%, a model-switch suggestion at 90% · the bar itself has no threshold color |
| Claude Code | used (notifications) | we could not confirm a generally published threshold. One CHANGELOG line says (back-translated from the Korean source) that it fixed warnings that appeared at low utilization after the weekly reset — it now requires 70% used |

**The red line clusters near 80.** Where warnings start splits among 50 · 60 · 70. There is a separate camp that **decides the color by pace** instead of a fixed threshold. **Devices that do not rely on color alone**: CodexMeter ("without relying on color alone"), Usage4Claude (per-window shapes in monochrome mode), ClaudeBar (high-contrast palette, 4.5:1) — directly tied to our e-ink theme.

### 3-4. Details worth borrowing

- The bar and the label use the same rounding function — if they disagree you get "the bar is full but it says 99%" (Orca `src/shared/usage-percentage-display.ts`).
- Keep the normal chip as it is during a background re-fetch — so a failing vendor does not flicker `…→오류` every cycle (Orca).
- Expand on hover and **pin with a click** — so it can be read or scrolled (TokenTelemetry). This also ties in with the accessibility item below.

### 3-5. Accessibility (an omission the review pointed out)

- A gauge must have a **programmatically readable name and value** — the W3C ARIA `meter` pattern (https://www.w3.org/WAI/ARIA/apg/patterns/meter/). From the 「LLM-우선 제어」 viewpoint too, the value must be DOM text for the LLM to read it.
- Content revealed on hover must be **opened by focus too · dismissible · and must not disappear when the pointer is moved over it** — WCAG 2.2 SC 1.4.13 (https://www.w3.org/WAI/WCAG22/Understanding/content-on-hover-or-focus.html). **A hover-only detail cannot meet this condition** → a click pin or a focus open is needed.

---

## 4. What comes back if you ask (data shape only — source choice is out of scope)

### 4-1. Codex — one model comes through four paths

**Model = `RateLimitSnapshot`**: `limitId` · `limitName` · `normalModelSlug` · `primary` · `secondary` · `credits{hasCredits, unlimited, balance: string}` · `individualLimit` · `spendControlReached` · `planType` · `rateLimitReachedType`. **Window = `{usedPercent 0–100, windowDurationMins, resetsAt epoch 초}`**.

| Path | Method | Note |
|---|---|---|
| app-server `account/rateLimits/read` | request (no parameters, after `initialize`) | the response also carries `rateLimitsByLimitId` (per bucket) · `rateLimitResetCredits` · `accountId`. **Measured successfully** |
| app-server `account/rateLimits/updated` | push | sparse — merged into the previous read (null does not erase). It did not arrive in this measurement (no turn was run). **Our daemon currently receives this and discards it** (`crates/engram-dashboard-agent/src/backend/codex/decoder.rs:125`) |
| session log JSONL `token_count` event | file | snake_case (`window_minutes`, `used_percent` f64). **Measured sample confirmed** |
| backend HTTP `wham/usage` | request (ChatGPT OAuth token required) | the window length is in **seconds** (`limit_window_seconds`), and `reset_after_seconds` is also there. Non-public — confirmed from source only, not called |

**Measured (2026-09-26, codex-cli 0.156.1, Plus plan — identifiers removed):**

```json
"primary":   { "usedPercent": 37, "windowDurationMins": 300,   "resetsAt": 1790424706 },
"secondary": { "usedPercent": 58, "windowDurationMins": 10080, "resetsAt": 1790739378 },
"planType": "plus",
"rateLimitsByLimitId": { "base_model_inference": {
  "limitName": "gpt-reserve", "normalModelSlug": "gpt-5.6-luna",
  "primary": { "usedPercent": 0, "windowDurationMins": 10080, "resetsAt": 1791018755 },
  "secondary": null } }
```

★**`primary` is not always 5 hours**★ — in the extra bucket `base_model_inference`, `primary` is the weekly one (10080 minutes). The 5-hour/weekly classification must be done by **`windowDurationMins`, not by the slot position.**

### 4-2. Claude — five sources and the units are mixed

| Path | Method | Windows | Utilization unit | Reset format | Documented |
|---|---|---|---|---|---|
| stream-json `rate_limit_event` | push (when the limit information changes) | the documented top-level fields cover **only one window**, the one `rateLimitType` points to (those fields are all optional too) | **0–1** | epoch seconds | types are public in the SDK |
| statusline stdin `rate_limits` | push (only inside a Claude Code session) | `five_hour` · `seven_day` | 0–100 (`used_percentage`) | epoch seconds | official docs |
| SDK control request `get_usage` | request | + `seven_day_opus`/`_sonnet`/`_oauth_apps`, `model_scoped[]`, `extra_usage`, `subscription_type` | 0–100 | ISO 8601 | public in the SDK — the method name is `…_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET` |
| OAuth `api/oauth/usage` | request (OAuth token) | the same shape as `get_usage` + a new `limits[]` | 0–100 | ISO 8601 | **non-public** — confirmed only from third-party code, not called |
| response headers `anthropic-ratelimit-unified-*` | only a proxy can see them | 5h · 7d | 0–1 | epoch seconds | non-public — third-party |

**Measured (2026-09-26, Claude Code 2.1.280 — identifiers removed):**

```json
{"type":"rate_limit_event","rate_limit_info":{
  "status":"allowed_warning","rateLimitType":"seven_day",
  "utilization":0.83,"resetsAt":1790856000,"surpassedThreshold":0.75,"isUsingOverage":false,
  "unifiedWindows":{"five_hour":{"utilization":0.67,"resetsAt":1790425800},
                    "seven_day":{"utilization":0.83,"resetsAt":1790856000}}}}
```

★**`unifiedWindows` was only observed in a single capture**★ — it is not in the SDK type declarations (`sdk.d.ts`), and it would not be strange for it to disappear in the next version. **Do not take "one stream line delivers both windows" as a premise.** The `rate_limit_event` in our repository's test fixtures (`claude_text.jsonl:3` etc.) is an older, thinner shape (no `utilization`). The daemon **skips** this event (`crates/engram-dashboard-agent/src/backend/claude/mod.rs:743`).

### 4-3. Side by side

| | Claude | Codex |
|---|---|---|
| 5-hour window | `five_hour` | the window whose length is 300 minutes |
| Weekly window | `seven_day` | the window whose length is 10080 minutes |
| Window length | implied by the name (no minutes field) | explicit (minutes · the backend uses seconds) |
| Utilization | 0–1 (stream · headers) / 0–100 (statusline · `get_usage` · endpoint) | 0–100 (an integer for the app-server · backend, a real number for the log file) |
| Reset | epoch seconds / ISO 8601 | epoch seconds |
| Plan | only in `get_usage.subscription_type` | `planType` in every snapshot |
| push | stream `rate_limit_event` (one window at a time per the docs), statusline | app-server `account/rateLimits/updated`, log file |
| pull | `get_usage`, OAuth endpoint | app-server `account/rateLimits/read`, `wham/usage` |

---

## 5. Fit with our constraints

**Constraints:** ① the slot size changes with the user's split (it may be a wide horizontal strip or a tall vertical column) ② themes dark/light/**e-ink** — CSS variable tokens (CLAUDE.md) ③ 「LLM-우선 제어」 — slot creation is absorbed into the existing `layout.setSlotContent` path, no new global handle, values as DOM text ④ user requirements: small and in one place, detail in a tooltip, a small bar, only 5-hour · weekly for now ⑤ accessibility — the detail must also open by focus · click (§3-5).

**The "always shows four values" below is not a requirement but a trade-off** — user requirement ④ allows putting the detail in the tooltip.

| Form | ① Size change | ② e-ink | Always shows four values | Precedent |
|---|---|---|---|---|
| A. 4-row stack (one line per vendor × window) | strong in a vertical column · when narrow, drop the bar and use % | readable without color since it is text + bar length | yes | Orca tooltip · Codex `/status` · claude-usage-widget Bars |
| B. Two vendor rows × two bars side by side | strong in a horizontal strip · overflows in a vertical column | same | yes | ClaudeBar 2 lines · Jean badge · AgentLimits `X% / Y%` |
| C. The one tightest window per vendor | fits a small spot | same | no — only one per vendor, the rest in the tooltip | Orca status bar · CodexMeter |
| E. Single gauge (colored by the tightest window overall) | fits even the smallest spot | **color is the main signal so it is weak on e-ink** — numbers must accompany it | no — everything in the tooltip | TokenTelemetry |
| D. Ring (2×2 grid, etc.) | suits a square spot | tells windows apart by shape — weak at small sizes | yes (2×2 grid) | claude-usage-widget Gauge · menu-bar apps |

---

## 6. Options (user decision targets)

1. **Layout** — A / B / C / E (the table above). Shrinking with width (bar → % → one character) can be attached to any of them.
2. **Percentage direction** — unify on used / unify on left / keep each vendor's convention / toggle. Keeping each vendor's convention mixes bars of opposite direction in one slot.
3. **Reset display** — countdown in the slot + absolute time in the tooltip (the managers' convention) / absolute time only (the official convention).
4. **Color** — fixed thresholds (60/80 with an achromatic start · 50/80 · 70/90) / pace basis (AgentLimits · t3code). Whether a non-color marker is needed for e-ink.
5. **Pace display** — include the time-elapsed hairline above the bar (t3code) / do not.
6. **How the detail opens** — hover + focus / hover + click pin. Hover-only cannot meet the §3-5 conditions.
7. **Refresh** — tied to the data-source decision (out of scope). On the display side, the **minute-granularity countdown re-render · stale indication · removing the old % once the reset has passed · keeping the previous value on failure** are widespread in the precedents surveyed, so they are default candidates rather than decision targets.

**Rejection candidates (basis = this survey · the final verdict is the user's):**

- **D. Ring** — there is a 2×2 ring grid precedent (claude-usage-widget). But in a small slot, four rings leave little room for numbers, and relying on shape to tell windows apart is weak on e-ink. Manager · IDE precedents are 0 in the survey scope.
- **Estimates (summing local token logs)** — the ccusage · old Claude-Code-Usage-Monitor way. The real % computed by the vendor already arrives, and an estimate can be wrong. agent-deck wrote down the reason: "being wrong about a quota is worse than not showing one".

---

## 7. grounding (main-session cross-check)

| Claim | Cross-check | Verdict |
|---|---|---|
| Codex app-server read response shape · `base_model_inference` bucket | opened the measurement capture file directly | supported |
| Claude `rate_limit_event`'s `unifiedWindows` · the 0–1 unit | opened the measurement capture file directly | supported (but a single capture — the §4-2 caveat) |
| Orca 15-minute polling · 5-minute debounce · 30-minute/24-hour stale · color 60/80 | local clone `src/main/rate-limits/service.ts:76-92`, `src/renderer/src/components/status-bar/tooltip.tsx:190-198` | supported |
| Codex stale 15 minutes | openai/codex `codex-rs/tui/src/status/rate_limits.rs:66` | supported |
| Claude Code: statusline `rate_limits` added · `/usage` return to "% used" · "as of" note · fix of the old-% bug after the reset · the context of the 70% warning | grep of the anthropics/claude-code CHANGELOG body | supported (version numbers are the collector's report — not cross-checked) |
| New precedents the reviewer brought (TokenTelemetry · claude-usage-widget · AgentLimits) | opened the documentation pages · each README directly | supported |
| Third-party thresholds · intervals (ClaudeBar · CodexBar · Usage4Claude, etc.), Paseo · t3code · Jean constants | not cross-checked by the main session — collector report | partially supported (single source) |
| OAuth endpoint response shape | third-party code only | likely |

---

## 8. Adversarial review results

**Reviewer:** codex-cli 0.156.1 (cross-family), effort high, web search on, level 2~3. **Verdict: BLOCK** — (back-translated from the Korean source) the options omitted a directly relevant small-display precedent, and one candidate fails the report's own requirement.

| # | Point | Type · severity | Handling |
|---|---|---|---|
| 1 | C fails §5's "four values at a glance" yet remains a candidate | logical gap · high | **partially applied** — that column was not a user requirement but a criterion I had added. Since the user requirement allows putting the detail in the tooltip, C was not removed, and the column was rewritten as a "trade-off" (§5) |
| 2 | TokenTelemetry (always-visible sidebar gauge + the whole list on hover + click pin) · claude-usage-widget (4 bars / 2×2 rings) omitted | omission · high | **applied** — both checked at the source. Form E created, a precedent added to D (§3 · §5) |
| 3 | "5 minutes is the norm, 5~15 minutes is the default" is an exaggeration that mixes the fetch interval and cache freshness | logical gap · exaggeration · high | **applied** — split the table into four axes and withdrew the norm claim (§0-4 · §2) |
| 4 | The description that two windows are received through `unifiedWindows` is an exaggeration — it is not in the SDK types | exaggeration · medium | **applied** — stated as a single capture · non-documented · may disappear (§4-2) |
| 5 | "No slot precedent" goes beyond the survey scope — a sidebar gauge is closer | omission · medium | **applied** — narrowed to "in the products surveyed" and made the sidebar gauge the nearest precedent (§0-6 · §3-1) |
| 6 | "Claude Code warns at 70%" is an exaggeration — it is the context of a bug fix after the weekly reset | exaggeration · medium | **applied** — checked the CHANGELOG original (quoted the context as it is, §3-3) |
| 7 | The accessibility criteria for small gauges · hover details are missing (ARIA meter · WCAG 1.4.13) | omission · medium | **applied** — §3-5 added, constraint ⑤ and option 6 added |
| 8 | The header says a review was done but §8 is empty | no basis · low | **applied** — it was the header of the pre-review draft. This section is now filled in |

**The reviewer's independent re-checks (supported):** Claude `/usage` "% used" · Codex "% left" and a bar that fills with the remaining amount · Codex adaptive polling 60/30/15/5 seconds and warnings at 50/75/90/95 · Codex `primary` may not be 5 hours · Orca 60/80 · 15 minutes. **Partially supported:** `unifiedWindows` (in the capture but not in the types).

---

## 9. Limits · gaps

- We did not look at screenshots — the layout descriptions are based on README · docs · source text.
- claude.ai Settings → Usage · the ChatGPT Codex usage page · the Claude desktop app require a login, so we could not confirm their shape.
- We do not know when and how often `account/rateLimits/updated` actually arrives (it did not arrive in the measurement).
- We do not know whether Claude `rate_limit_event` arrives once per window or only one representative window.
- We could not check claude-squad · the Superset source · Windsurf · Warp (search limit). For opcode · Crystal · claudecodeui · emdash there were just no traces in the code search, so "none" is uncertain. Since the reviewer found three precedents the collectors had missed, **there may be more omissions**.
- The Codex file:line pointers are as of `main` on 2026-09-26 (SHA not pinned). Orca is as of the local clone `df7460af`.

## Sources (main)

- Claude Code statusline docs — https://code.claude.com/docs/en/statusline
- Claude Code CHANGELOG — https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md
- Codex TUI status — https://github.com/openai/codex/tree/main/codex-rs/tui/src/status · rate limit warnings — https://github.com/openai/codex/blob/main/codex-rs/tui/src/chatwidget/rate_limits.rs
- Codex app-server protocol — https://github.com/openai/codex/tree/main/codex-rs/app-server-protocol
- Orca — https://github.com/stablyai/orca (local clone `I:\Engram_Workspace\opensource\orca`)
- TokenTelemetry — https://tokentelemetry.com/docs/features/plan-limits/
- t3code — https://github.com/pingdotgg/t3code · Paseo — https://github.com/getpaseo/paseo · Jean — https://github.com/coollabsio/jean · agent-deck — https://github.com/asheshgoplani/agent-deck
- CodexBar — https://github.com/steipete/CodexBar · ClaudeBar — https://github.com/tddworks/ClaudeBar · Usage4Claude — https://github.com/f-is-h/Usage4Claude · CodeZeno — https://github.com/CodeZeno/Claude-Code-Usage-Monitor · CodexMeter — https://github.com/raycalrui/CodexMeter · AgentLimits — https://github.com/Nihondo/AgentLimits · claude-usage-widget — https://github.com/bozdemir/claude-usage-widget
- ccstatusline — https://github.com/sirmalloc/ccstatusline · claude-hud — https://github.com/jarrodwatts/claude-hud · claude-pace — https://github.com/Astro-Han/claude-pace · ccusage — https://github.com/ccusage/ccusage
- Claude Agent SDK types — `@anthropic-ai/claude-agent-sdk` `sdk.d.ts`
- OAuth endpoint shape (third-party) — https://github.com/trickv/hass-claude-usage · https://github.com/lobehub/lobehub
- Accessibility — https://www.w3.org/WAI/ARIA/apg/patterns/meter/ · https://www.w3.org/WAI/WCAG22/Understanding/content-on-hover-or-focus.html
