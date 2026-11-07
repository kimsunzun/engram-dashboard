# Ways to actively query Claude usage — peer survey (medium)

| Item | Value |
|---|---|
| **Status** | Edition with adversarial review (FIX) applied — application record in §6 · A probe result = §7 (2026-09-27) |
| **Date** | 2026-09-27 |
| **Method** | 2 collection tracks (peer code · official docs/policy) + main grounding spot checks (§5) + one cross-family adversarial review (§6) |
| **Preceding doc** | `docs/research/usage-limit-display-ui-survey-2026-09-26.md` 「사용자 결정」 — refresh = pick up during conversation + automatic query after the cooldown (5 min) + manual ⟳. This document looks at **what to use for that automatic query on Claude** |
| **Confidence** | certain = two or more independent sources · likely = one source · uncertain = unsupported · inference |
| **Policy caution** | This document only **quotes** terms/policy sentences. Interpretation and applicability are for the legal and security department |

## 0. Conclusion

1. **Most peers call a private OAuth usage address directly (B).** `GET https://api.anthropic.com/api/oauth/usage` + `Authorization: Bearer <토큰>` + `anthropic-beta: oauth-2025-04-20`. Orca · Paseo · CodexBar · jean · claude-usage-widget · CodeZeno use it as first choice, and ClaudeBar as a secondary. The token is **the one Claude Code stored, which the app reads directly** (`~/.claude/.credentials.json`, macOS Keychain, `CLAUDE_CODE_OAUTH_TOKEN`). Several imitate `User-Agent: claude-code/2.1.x` (Orca `src/main/rate-limits/claude-fetcher.ts:46-48,358-360`). Some (ClaudeBar · jean · Orca managed accounts) even **refresh the token themselves** with Claude Code's public client_id. (certain — Orca spot check)
2. **t3code uses the SDK `get_usage` (A).** It starts a claude SDK session that never sends a prompt, finishes only initialization, and calls `usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET()` — comment (back-translated, not the original wording): "no user message is ever written to stdin … so it does not start an API request to Anthropic" (`apps/server/src/provider/Layers/ClaudeProvider.ts:318-367`). The login is done by the claude process itself, so **the app does not touch the token.** 5-minute cache. (certain — spot check) However, t3code calls **through the SDK library** — it is not evidence for a path that sends directly to the CLI without the SDK (§1). Also, "no API request" means **no inference (model) request**; the usage value itself comes "from the claude.ai usage endpoint", as the SDK type comment says (`sdk.d.ts:4232`) — a network call does happen.
3. **The usage address has strict query limits — it acts like a "penalty box".** ClaudeBar (quotes in this item back-translated, not the original wording): "a single call after a quiet period once triggered a 1-hour `Retry-After` … even a 5-minute cache got caught" → success values cached for 15 minutes · API-mode floor 15 minutes (`docs/providers/claude/design.md:61-62`). **This 15 minutes is an operating choice, not an interval proven safe.** Orca: default 15-minute polling, and while rate-limited it keeps the previous value for 24 hours. Claude Code itself wrote in its CHANGELOG "when the usage endpoint is rate-limited, it shows the last value as 'as of'" (2.1.208) and "sessions on one machine share a read made within the last 1 minute" (2.1.275). User issues include a report that even a 10-minute interval kept getting 429 within an hour with no `Retry-After` (anthropics/claude-code#31637) and a report that 429 continued with `Retry-After: 0` (#30930) (there is no official figure). (certain — that query limits exist / uncertain — the exact limit and the safe interval)
4. **There are several pieces of indirect evidence that `get_usage` (A) shares the same limit as B — the exact bucket is unconfirmed.** ① The SDK type says the value's source is the "claude.ai usage endpoint" (`sdk.d.ts:4232`) ② ClaudeBar sees CLI `/usage` and its own direct call as the same backend, so it does not fall back to the CLI after a direct-call 429 (`docs/providers/claude/design.md` line 18 — reviewer's quote) ③ CLI 2.1.275 shares the last 1 minute of reads across sessions on one machine. None of them proves a server-side bucket. That 1-minute shared cache benefits only the A side. (likely — sharing / uncertain — the bucket)
5. **Policy sentences (quotes only — back-translated here, not the original wording):** The Claude Code legal and compliance document (`code.claude.com/docs/en/legal-and-compliance`) says OAuth authentication "is designed for ordinary use of Claude Code and other native Anthropic apps", and "developers may not collect, store, or broker Claude.ai credentials or session tokens". **The same item says, earlier than that, that "developers building products that interact with Claude's capabilities, including those using the Agent SDK, should use API key authentication (should)", and goes on to say that third-party developers are not allowed to relay requests with Free · Pro · Max plan credentials on behalf of users.** The same document also says "it does not prevent end users from logging in to an **unmodified Claude Code binary** with their own subscription". B (the app reads the token directly and calls) has the same shape the credential sentence names, and the API key sentence names the Agent SDK, so A also falls within the review scope. Which side A falls under, this document does not decide — **subject to legal and security confirmation.** (certain — all five sentences exist, main checked against the source text 2026-09-27)
6. **There is no public official query API.** The documented routes are only the statusline's `rate_limits` (TUI only — not used, per the preceding doc's decision) and the experimental `get_usage`. (likely — not a proof of absence)
7. **C (short prompt) · D (scraping the `/usage` screen) · E (claude.ai cookie) are used only as secondaries.** C only by CodeZeno (`max_tokens: 1` Haiku — it consumes usage), D is ClaudeBar's default and Orca/CodexBar's secondary (fragile, e.g. a fixed 2-second startup wait), E only by CodexBar.

## 1. What this means for us

- **A fits our constraints best — but a raw stream-json probe is the precondition for choosing A.** ★**→ Probe done (§7, 2026-09-27): ①②③ confirmed working (no initialization needed · responds without a first turn), ⑤ shows no trace of a model call (likely). What follows is the pre-probe record.**★ We do not read the token; we ask the official CLI we already launch. But t3code goes through the SDK, so it is not evidence for our path. What the probe should look at: ① whether our daemon (Rust) works **sending `control_request{subtype:"get_usage"}` directly to the CLI stream-json**, without the SDK library (the CLI 2.1.208 release notes say only that headless stream-json accepts control requests in general and do not list this subtype — not measured) ② whether `initialize` is needed first ③ whether it works before the first turn ④ response handling (null · omissions in §3) ⑤ whether it does not consume usage (we **infer** it won't since there is no model call — no official sentence found). On the policy side, the Agent SDK · API key sentence of §0-5 reaches A too — the applicability decision is for legal and security.
- **The Claude cooldown of 5 minutes collides with peer evidence.** Mature peers put the Claude query floor at 15 minutes (Orca · ClaudeBar). A likely shares the same limit (§0-4), so a 5-minute query period may draw 429. → **Starting Claude alone at 15 minutes** is the side that fits the evidence. However, 15 minutes is a **starting point, not a safe interval** — there is a case where one call after a quiet period got a 1-hour rejection (§0-3). Coordinate per account (only one query per account). Codex (local app-server) stays at 5 minutes.
- **429 handling does not stop at honoring `Retry-After`.** If the value is missing, broken, in the past, or `0` (#30930), use a fallback wait (ClaudeBar = 5 minutes). While rejected, **neither automatic nor manual (⟳) re-queries immediately**; the previous value is kept.
- If a JSON-mode Claude agent is already running, it is also worth checking **whether `get_usage` can be sent to that process** (no new process is launched — a TRD item to confirm).

## 2. Peer table

| Peer | Method (order) | Authentication | Period · 429 |
|---|---|---|---|
| t3code | A (+ merges `rate_limit_event` during conversation) | the claude process itself | 5-minute cache · 5-minute health refresh · no 429 handling |
| Orca | B → re-read and refresh credentials, then B → D · receives statusline | Keychain → `.credentials.json` | 15 minutes · debounce 5 minutes · honors `Retry-After` · keeps value 24 hours while rate-limited |
| Paseo | B | `.credentials.json` → Keychain · read only (refresh is the CLI's job) | 5-minute cache · unaware of 429 |
| CodexBar | B → D → E | Keychain/`.credentials.json` · refresh left to `claude /status` | adaptive · per-token 429 gate (default 5 minutes) |
| ClaudeBar | D ↔ B | `.credentials.json` → Keychain → env · refreshes itself | success cached 15 minutes · floor 15 minutes · `Retry-After` (5 minutes if absent) |
| jean | B + merges `rate_limit_event` of its own runs | Keychain → `.credentials.json` · refreshes itself | 5-minute cache · while events arrive B runs once per 30 minutes · 429 cooldown |
| claude-usage-widget | B | env → `.credentials.json` → Keychain · no refresh | 60 seconds → 300-second backoff · retries twice on 429 |
| CodeZeno (Windows) | B → C | `.credentials.json` etc. · on expiry `claude -p .` (consumes usage) | configurable · `Retry-After` up to 24 hours |

## 3. `get_usage` shape (per SDK type `sdk.d.ts`)

- Request: `{type:"control_request", request_id, request:{subtype:"get_usage", skip_behaviors?:boolean}}` — `skip_behaviors` is (back-translated) "for callers like usage meters that need only the plan limits" (skips scanning 7 days of history).
- Response: `subscription_type` (pro/max/team/enterprise/null) · `rate_limits_available` · `rate_limits` (may be null){five_hour, seven_day, seven_day_opus, seven_day_sonnet, seven_day_oauth_apps: {utilization 0–100 | null, resets_at ISO 8601 | null}, model_scoped[] (may be absent and is filtered by an allowlist — `sdk.d.ts:4285-4295`), extra_usage} · `session{…}`.
- **null · omissions are normal responses** (`sdk.d.ts:4234-4254,4303`) — do not draw a missing value as 0%.
- Label: "EXPERIMENTAL: this API is unstable and may change or be removed in any release without notice" (`sdk.d.ts:3041`).

## 4. Limits

- **Not measured:** whether A uses the same 429 budget as B (indirect evidence only — §0-4) · whether usage is consumed (only up to no trace of a model call — §7) · the path of sending to a running agent process. (Sending CLI stream-json directly · whether `initialize` is needed · behavior before the first turn were confirmed by the §7 probe.)
- The query-limit figures are only peer and user observations (no official figure). The observations disagree with each other (a report that 5 minutes held vs a report that even 10 minutes gets 429). 15 minutes is also not an interval proven safe.
- We did not interpret the terms. Press reports (the 2026-02 wording change · blocking measures) were not checked against a primary source.
- Peer source points in time: t3code `5378f87f9` (2026-09-18) · Orca `df7460af` (2026-08-20) · Paseo `bbe3f17` (2026-08-25) · the rest were the default branch on 2026-09-27.

## 5. grounding (main check)

| Claim | Check | Decision |
|---|---|---|
| t3code calls `get_usage` with a prompt-less SDK session and "does not start an API request" | local clone `ClaudeProvider.ts:318-367` source text | supported |
| Orca calls with the OAuth address + beta header + `claude-code/2.1.0` UA | local clone `claude-fetcher.ts:46-48,358-360` | supported |
| ClaudeBar's "penalty box" · 15-minute floor | `design.md:61-62` source text via `gh api` | supported |
| `get_usage` request shape · source comment · EXPERIMENTAL | `sdk.d.ts:3041,4201-4205,4232` | supported |
| The CLI shares usage reads within 1 minute | CHANGELOG source-text grep | supported |
| Three sentences of the legal and compliance document | page source-text grep | supported |
| The API key sentence of the same document · the plan-credential relay prohibition sentence (the part applied from §6) | page source-text check (2026-09-27) | supported |
| Details of the remaining peers (Paseo · CodexBar · jean · widget · CodeZeno) | not checked by main — collector's report | partially supported (single source) |
| New quotes in the part applied from §6 (ClaudeBar `design.md` lines 18 · 48 · `sdk.d.ts` null-allowed line · #30930) | checked by the reviewer — not checked by main | partially supported (single source) |

The existing `sdk.d.ts` line numbers are based on a copy with no recorded version — the lines for 0.3.276 are in §7.

## 6. Adversarial review result

Reviewer = codex (cross-family) · decision **FIX** · source text = `.claude/handoff/attachments/claude-usage-query-review-2026-09-27.md`.

| # | Finding (severity) | Applied in |
|---|---|---|
| 1 | The A recommendation rests on an unmeasured direct CLI path — make a raw stream-json probe a precondition (high) | §0-2 · §1 · §4 |
| 2 | The policy quote omits the adjacent sentence (Agent SDK users also use API key authentication) — it reaches A too (high) | §0-5 · §1 · §5 |
| 3 | 15 minutes was written as if the evidence backed the interval — it is only a starting point (medium) | §0-3 · §1 · §4 |
| 4 | The indirect evidence that A and B share a limit was left out — the bucket is unconfirmed (medium) | §0-4 · §1 · §4 |
| 5 | The null · omissions of the `get_usage` response were left out — do not draw as 0% (medium) | §3 · §1 |
| 6 | 429 handling stops at honoring `Retry-After` — fallback wait · no immediate re-query (medium) | §0-3 · §1 |

The ClaudeBar `design.md` line numbers disagree because they use different baselines — §0-3 wrote the same "penalty box" content at lines 61-62 (`gh api`), and the reviewer at lines 47-49 (raw main). The cause was not checked.

## 7. Probe result (2026-09-27)

- **Target** = raw `claude` CLI 2.1.280 — no SDK · no prompt · no `initialize`. Only one line was written to stdin: `{"type":"control_request","request_id":"<random>","request":{"subtype":"get_usage","skip_behaviors":true}}`
- **Arguments** = our backend JSON-mode arguments `--permission-mode bypassPermissions -p --input-format stream-json --output-format stream-json --replay-user-messages --verbose` + side-effect reducers `--no-session-persistence --settings {"disableAllHooks":true} --strict-mcp-config --mcp-config {"mcpServers":{}}`.
- **Result** = a `control_response` success about 1.2 seconds after startup. The whole stdout is that one line (no `system:init` · `assistant` · `result`).
- **Response shape** = `rate_limits.five_hour`/`seven_day` {utilization, resets_at} · `subscription_type` · `model_scoped`[] · windows whose key is present but value is null (`seven_day_opus` · `seven_day_sonnet` · `seven_day_oauth_apps`) · several fields not in the type.
- **No model call** — `session.total_api_duration_ms`=0 · `total_cost_usd`=0 · `model_usage`={}. (likely — not a proof)
- **Query budget** = `get_usage` used once · no 429. In A the app does not touch the token.
- **Unchecked** = our arguments exactly as they are, without the side-effect-reducing arguments · sending `get_usage` to an already running agent process · the CLI has a branch that returns "get_usage is not supported in this context" (which mode is unidentified).
- **SDK line numbers (`@anthropic-ai/claude-agent-sdk@0.3.276` — the version in t3code `pnpm-lock.yaml:1117`)** = EXPERIMENTAL method 2860-2868 · `SDKControlGetUsageRequest` 3995-4001 · response type 4006 · "claude.ai usage endpoint" comment 4027 · nullable windows 4039-4079 · `model_scoped?` 4083 · `rate_limits … | null` 4098.
- Raw log = session scratch (outside the repository, no identifiers).
