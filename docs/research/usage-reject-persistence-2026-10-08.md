# Usage fetch 429 rejection deadline — is it persisted, and where does it go (2026-10-08)

- Status: research complete (light) · awaiting decision
- Method: `/research light` — collector 1 (read the source directly) · main-session grounding spot-check (checked against orca `service.ts:91` · Claude-Code-Usage-Monitor `api_usage.py` — supported) · no cross-family adversarial review (light)
- Confidence legend: certain = confirmed in code · likely = partly confirmed in code / evidence of absence · uncertain = search only

## Question

When the daemon gets a 429 (`Retry-After` of about 1 hour observed) while fetching claude · codex usage, it stores the rejection deadline separately in `daemon\state\usage_rejects.json` (`crates/engram-dashboard-daemon/src/usage_service/reject_store.rs` · TRD S21-usage-limit-slot §3 #30 — a default that reflected the 「재시작 거절」 (rejection on restart) point from PRD review round 2, and it was not a user decision). The user's questions: isn't memory alone enough · if it is persisted, shouldn't it go in one place rather than a file per feature?

## Conclusion

- **Peers split.** Of the four tools that call a usage endpoint and handle 429, two carry the deadline across restarts (CodexBar · Claude-Code-Usage-Monitor) and two keep it only in memory (orca · ClaudeBar). Two other tools do not handle 429 separately at all (paseo · t3code).
- **No one has a dedicated file that holds only the deadline.** The two that persist use either ① a general-purpose key-value store with one key per gate (CodexBar — macOS UserDefaults) or ② one field in the same file as the last-value cache (Claude-Code-Usage-Monitor — `retry_after_epoch` in `latest.json`).
- orca knows the same 1-hour window from a comment (`src/main/rate-limits/service.ts:91`), yet keeps the deadline only in memory and chose to show the old value for up to 24 hours during a 429.

## Cases

| Tool | 429 handling | Deadline location | How small state is stored | Confidence | Basis |
|---|---|---|---|---|---|
| CodexBar | yes (5 minutes if absent) · user refresh bypasses it | **persisted** — UserDefaults key (`claudeOAuthUsageRateLimitBlockedUntilV2.<토큰 해시>` etc.) | gate = UserDefaults key · large snapshots = per-feature JSON | certain | `ClaudeOAuthUsageRateLimitGate.swift:4-6,28-41` · `ClaudeCLIRateLimitGate.swift:4-5,13,30-34` (HEAD `5891c0b`) |
| Claude-Code-Usage-Monitor | yes → answers with the cached value | **persisted** — `retry_after_epoch` in the value-cache file `~/.claude-monitor/api/latest.json` (tmp + `os.replace`) | value + deadline = one file | certain | `src/claude_monitor/output/api_usage.py:33,150-165,196` |
| orca | yes · a forced fetch bypasses it · keeps the old value for 24 hours | **memory** | one app-state file `orca-data.json` + only the frequently changing cache separately (`orca-github-cache.json` — to avoid rewriting a large file) | memory = likely (no reference in the persistence layer) · storage method = certain | `src/main/rate-limits/service.ts:91,170-180` · `src/main/persistence/loading-store/user-data-path.ts:16,29-32` (`df7460af`) |
| ClaudeBar | yes (5 minutes if absent) | **memory** — `UsageMemory.retryAt` | memory | certain | `Modules/DataSources/Sources/DataSource.swift:100-111,367-394` |
| paseo | none (401 · 403 only) | none — 5-minute memory cache | memory | certain | `packages/server/src/services/quota-fetcher/providers/claude.ts:466-474` |
| t3code | none separately (non-2xx = generic error) | none | not investigated | likely | `apps/server/src/usage/cliproxyApi.ts:174-176` |
| ccusage · herdr · Claude Squad | do not call a usage endpoint | — | — | likely ~ uncertain (search only) | — |

Backoff in general HTTP clients and CLIs (Google API clients · gh CLI · Octokit throttling) is almost all process memory — we did not re-verify this in code this time (uncertain ~ likely).

## Our options

| | Content | Cost |
|---|---|---|
| **A. Memory only** | remove `usage_rejects.json` and `reject_store.rs` | If the daemon restarts within the rejection deadline, one more fetch goes out and gets a 429 again. We do not know whether that re-fetch extends the rejection. The daemon outlives the shell, so restarts are rare |
| **B. Keep as is** | per-feature files | no peer precedent · a file is added every time a new piece of small state appears |
| **C. Consolidate in one place** | gather the daemon's small runtime state in one file (or one key-value store), with the rejection deadline as one field inside it | a shared storage layer has to be built new — right now there is no other tenant for that file |

## Gaps

- Whether fetching again early extends Claude's rejection period — no peer has a measurement.
- Whether CodexBar · ClaudeBar cache the usage values themselves on disk — not looked at.
- The judgment that orca keeps it in memory came from there being no reference in the persistence layer, and we did not trace the whole startup path.
