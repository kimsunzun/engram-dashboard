# Survey — data sources for the agent slot's bottom display (context usage · model · effort)

- **Status:** survey banked — adversarial review (FIX) applied. **No decision** (the PRD choice is for the next session — user decision 2026-09-27: 「조사만 해 놔, 다음 핸드오프 때」).
- **Requirement (user, 2026-09-27):** like the Claude desktop app, show **context capacity · current model · effort** under the input box (that app shows 「Opus 5.5 · 엑스트라 · 작은 링」).
- **Method:** research medium · design-survey mode — collected three branches (peers · Claude · Codex) → main grounding (「grounding」 below) → cross-family adversarial review.
- **Target versions:** Claude Code CLI **2.1.280** (installed binary checked directly) · codex-cli **0.156.1** (app-server schema extracted) · openai/codex source HEAD `41f9084b3081` (difference from the tag unconfirmed).
- **Confidence legend:** certain = two or more primary sources agree · likely = one primary source · uncertain = inference or unmeasured.

## 0. Conclusion

1. **GUI peers (Paseo · t3code · vibe-kanban) converge on one small ring/gauge beside the input box + a tooltip with used/window** — the CLI side (Claude statusline · Codex TUI · ccusage) is text. **The color thresholds differ from one to the next** (Paseo 70/90 · vibe-kanban 50/75/90 · ccusage default 50/80). The model and effort are usually the **value the product itself passed when it spawned**. (likely — ★the draft that wrote 「전부 같은 모양」 (all the same shape) was caught by the adversarial review as an exaggeration★)
2. **For Claude (JSON mode), the raw data we need already flows in the stream but we throw it away.** Usage = the `message.usage` of the most recent `assistant` message · window size = `result.modelUsage[<모델>].contextWindow` at turn end · model = `system/init.model`. We read only the two fields input and output of `result.usage` and `rate_limit_event`, and skip `system/init` entirely (`crates/engram-dashboard-agent/src/backend/claude/mod.rs:863`). (certain)
3. **For Codex (app-server), a single notification gives both usage and window size** — `tokenUsage.last.totalTokens` and `tokenUsage.modelContextWindow` (nullable) of `thread/tokenUsage/updated`. The model and effort are `model` and `reasoningEffort` in the thread-start response. We deserialize only the two fields input and output of `last` from that notification and throw the rest away (`crates/engram-dashboard-agent/src/backend/codex/protocol.rs:634-640`). (certain)
4. **Neither company has a 「실제로 쓰인 값」 (actually used value) signal for effort** — only the configured value comes back. For Claude, `init.effort` is present **only when the host supplies it**, and whether it is supplied under `-p` is unmeasured. (likely)
5. **Terminal mode has effectively no channel a machine can read** — Claude has the statusline command and the transcript file, Codex has only the session record file, and both are in the class this repository has already decided not to use (statusline = a process per message · Codex record file = ADR-0203). Both TUIs already draw these values on their own screens. (certain — the channel list · the decision is the PRD's job)

## 1. Peers (direct survey)

| Product | What is shown | Source of usage · window size | Model · effort | Source · confidence |
|---|---|---|---|---|
| Claude Code CLI statusline | the command receives stdin JSON and draws | `context_window.{used_percentage, context_window_size, current_usage…}` — doc original "calculated from input tokens only: `input_tokens + cache_creation_input_tokens + cache_read_input_tokens`. It does not include `output_tokens`" | `model.{id,display_name}` · `effort.level` (only for models that support effort, reflects `/effort` changes) | code.claude.com/docs/en/statusline + binary builder · certain |
| Codex CLI TUI | status-line items such as "Context N% left" | `last_token_usage.total_tokens` · `model_context_window` (the config value if absent) · **normalization that subtracts `BASELINE_TOKENS = 12000` from both numerator and denominator** | status-line item `ModelWithReasoning` | openai/codex tag `rust-v0.156.1` `codex-rs/protocol/src/protocol.rs:2244-2286` · `tui/src/token_usage.rs` · (at HEAD `41f9084b3081`: `protocol.rs:2417,2451-2462` · `tui/src/chatwidget/status_controls.rs:405-427`) · certain |
| Paseo | 14px SVG ring, >90% red · ≥70% amber | Claude: stream `message_start` · `message_delta` usage (3 input kinds + output) · window = the max of `result.modelUsage[*].contextWindow`, the catalog value before that · Codex: `last.totalTokens`/`modelContextWindow` | its own configured value · Codex `reasoningEffort` | local clone `paseo/packages/app/src/components/context-window-meter.tsx:67-70` · `server/.../codex-app-server-agent.ts:928-930` · certain |
| t3code | meter beside the composer | Claude (Agent SDK): 3 input kinds + output, `min(used, max)` · window = `result.modelUsage`, the last value if absent, initial value = catalog (`standard 200k / expanded 1M`) · Codex: `last.totalTokens`/`modelContextWindow` | its own configured value | local clone `t3code/apps/server/src/provider/Layers/ClaudeAdapter.ts:706-759,2665-2677` — ★「Avoid getContextUsage because its token-count fallback can make extra model requests」(`:2677`)★ · certain |
| vibe-kanban | gauge in 4 steps <50/<75/<90/above · tooltip "N% · used/total" | ★**parses the CLI stream-json directly — closest to our structure. But do not copy that calculation**★ — `message_delta.usage` 3 input kinds + output (subagent events excluded). The `message_delta` example in Anthropic's streaming docs carries **only `output_tokens`**, and since the missing input fields are filled with 0 and the sum is replaced, it can **under-count by counting only output** (`claude.rs:1772` · platform.claude.com/docs/en/build-with-claude/streaming — adversarial-review refutation) · window default `200_000`, 1M if `system/init.model` has `[1m]`, corrected by `result.modelUsage[주 모델].contextWindow` · Codex `last.total_tokens`/`model_context_window` | — | local clone `vibe-kanban/crates/executors/src/executors/claude.rs:723,1264` · certain (the `README` announces the service shutdown — we look at the design only) |
| Orca | the statusline hook is for collecting `rate_limits` | an implementation of the context meter was **not found** by grep (that does not mean it does not exist) | — | local clone `src/main/claude/statusline-script.ts:14-16` · uncertain |
| herdr | no such feature | — | — | local grep · likely |
| ccstatusline · ccusage | statusline script (text) · ccusage default thresholds 50/80 | statusline stdin JSON (documents an alternative context calculation) | statusline stdin | github.com/sirmalloc/ccstatusline `docs/USAGE.md` · github.com/ccusage/ccusage `docs/guide/statusline.md` · ★**the adversarial review caught these as an omission and we only added the names — the bodies were not read**★ (uncertain) |
| Claude desktop/web | model · effort · small ring | source and thresholds **unknown** (public material not surveyed) | — | — |

**Two branches in summing 「사용량」 (usage):** the CLI itself and the statusline use **the 3 input kinds only** (output excluded) · Paseo, t3code and vibe-kanban use **3 input kinds + output**. The basis is always **the last single API response**, and none of them uses the cumulative `result.usage` values. (certain)

## 2. Claude Code CLI 2.1.280

- **The CLI's own formula (binary checked):** `r = input_tokens + cache_creation_input_tokens + cache_read_input_tokens` · `pct = round(r / window × 100)` → clamped to 0–100. `e` is the first usage met when scanning the messages from the end — **the most recent API response**. Output tokens are reported separately and are not put into the %. (certain — binary + statusline doc)
- **Window size:** the only place in the stream that carries the window size is **`result.modelUsage[<모델>].contextWindow`** (schema `contextWindow: int`). It is not in the stream before the first turn ends. The CLI's decision rules: `[1m]` in the model id (env `CLAUDE_CODE_DISABLE_1M_CONTEXT` turns it off) · the `context-1m` beta + a supported model · a model that is 1M natively → 1,000,000 · an unknown model uses `CLAUDE_CODE_MAX_CONTEXT_TOKENS` · default 200,000. (rule code = certain · the meaning of each helper function = likely) ★**This value is 「CLI 가 믿는 한도」 (the limit the CLI believes), not a verified provider limit**★ — Anthropic documented that on compatible gateways Claude Code cannot confirm native 1M, so without the `[1m]` selector it reports and budgets as 200K (github.com/anthropics/claude-code-action `docs/configuration.md` — adversarial review).
- **Model:** `system/init.model` (certain) · each `assistant.message.model` (the standard API field — likely). The display name (`display_name`) exists only in the statusline. A mid-session switch is the control request `set_model`. Whether `init` comes again after a switch is **unmeasured**.
- **effort:** how to set it = `--effort <level>` · env `CLAUDE_CODE_EFFORT_LEVEL` · setting `effortLevel` (per model possible) · `/effort`. Where it is returned = statusline `effort.level` · **`init.effort` (carried only when the value is defined — binary `if(e.effort!==void 0)…effort=e.effort`, schema description "absent on hosts that do not publish it … newest frame wins")** · control request `get_settings` → `applied.effort`. Whether it is actually carried in the `init` of the `-p` stream is **unmeasured**. (likely)
- **Compaction:** `compact_metadata.{trigger, pre_tokens, post_tokens?}` of `system/compact_boundary`. (certain — schema)
- **What our code reads:** `result.usage.{input_tokens,output_tokens}` → `OutputEvent::Usage` (cumulative values, so not occupancy · cache fields ignored — `mod.rs:802-825`) · `rate_limit_event`. `assistant.message.usage` · `message.model` · `system/init` · `modelUsage` · `compact_boundary` are thrown away (`mod.rs:785-800,863`). (certain)

## 3. codex-cli 0.156.1 (app-server)

- **Usage:** `thread/tokenUsage/updated` `{threadId, turnId, tokenUsage:{last, total, modelContextWindow?}}` · the `last`/`total` fields = `inputTokens · cachedInputTokens · outputTokens · reasoningOutputTokens · totalTokens` (+ `cacheWriteInputTokens` default 0). `last.totalTokens` = the latest active context size, `total` = the session cumulative (upstream TUI comment `tui/src/token_usage.rs:37-38` — the schema has no description). `modelContextWindow` **may be null** (upstream `TODO make this not optional`). (certain — schema + paseo · t3code · vibe-kanban read the same field)
- **The TUI's %:** `BASELINE_TOKENS = 12000` ("prompts, tools and space to call compact") — 0 if the window ≤ 12000 · otherwise `eff = window − 12000`, `used = max(last.totalTokens − 12000, 0)`, `left% = round(clamp((eff − used)/eff × 100))`. So while `last.totalTokens ≤ 12000` (usually right after the first prompt) it shows 100% left — lower if the first prompt is large. (certain — tag `rust-v0.156.1` `codex-rs/protocol/src/protocol.rs:2244-2286` + `codex-rs/tui/src/token_usage.rs`, the same formula at HEAD)
- **Model:** top-level `model` · `reasoningEffort` of the `thread/start` · `thread/resume` · `thread/fork` responses (schema checked) · mid-session change = overriding with `model`/`effort` of `turn/start` · change notification = `thread/settings/updated` (`threadSettings.{model, effort}`) · if the server changes the executing model, `model/rerouted{fromModel,toModel,reason}`. The schema states that all of these are **configured values** and not per-turn execution records. (likely — the collector's schema reading · the main agent could not recheck the schema location of `thread/settings/updated`)
- **Format of the effort value:** not a closed enum but 「모델이 광고한 비지 않은 문자열」 (a non-empty string the model advertised). `model/list` gives each model's `defaultReasoningEffort` · `supportedReasoningEfforts` (it does not give the window size). (likely)
- **Compaction:** thread item `type: "contextCompaction"` (the old `thread/compacted` is to be discarded) · manual `thread/compact/start` · the automatic threshold is config `model_auto_compact_token_limit` (not carried in a notification). (likely)
- **What our code reads:** `TokenUsageBreakdown` declares only the two fields `input_tokens` · `output_tokens` → `total` · `modelContextWindow` · cache · reasoning · `totalTokens` are dropped at deserialization (`protocol.rs:634-640`) · `thread/settings/updated` · `model/rerouted` · `thread/compacted` are on the ignore list (`decoder.rs:171,184,205`) · the `Thread` struct and the start response do not read `model` · `reasoningEffort` · `turn/start` does not override the model or effort. (certain)

## 4. To choose in the PRD (not a decision — the next session raises it to the user)

1. **Target mode** — JSON mode (chat slots) only / up to terminal mode. For the terminal-side channels see §0-5.
2. **Claude usage sum** — the 3 input kinds only, like the CLI / include output, like the peers.
3. **Codex % formula** — the value with the 12000 baseline subtracted, like the TUI / the raw value (`last.totalTokens / window`). The question is whether the numbers diverge when the same conversation is viewed in the TUI and in the dashboard.
4. **Window size before the first turn** — for Claude the stream has no window size before the first `result` → leave blank as 「—」 / estimate by the CLI rule (`[1m]` · 200k).
5. **Source of effort** — the value we passed at spawn / `init.effort` (if carried) / Claude control request `get_settings` / Codex start response · settings notification. What do we show if we did not pass it and nothing is reported.
6. **Threshold colors** — whether to reuse the usage slot's color tokens, or follow the peers' 70 · 90% thresholds.

## 5. Rejected candidates (so the next session does not bring them up again)

- **Claude Agent SDK `getContextUsage`** — ★the reason for rejection is 「우리 경로가 SDK 가 아니라 CLI stream-json 이라서」 (our path is the CLI stream-json, not the SDK)★. t3code avoided it saying 「토큰 수 대체 계산이 모델 요청을 더 만든다」 (the token-count fallback makes extra model requests) (`ClaudeAdapter.ts:2677`), but the SDK changelog gained `getContextUsage({ detail: 'summary' })`, which uses only the last response's usage and a local estimate with no extra request (github.com/anthropics/claude-agent-sdk-typescript `CHANGELOG.md` — adversarial review). So do not repeat 「추가 요청 때문」 (because of the extra request) as the reason.
- **Computing occupancy from the cumulative `result.usage` values** — they are turn · session cumulative, so they are not context occupancy (none of the peers uses them · our current `OutputEvent::Usage` is exactly this value).
- **Reading the statusline command · transcript · Codex record file for terminal mode** — this repository has already rejected the same class (ADR-0203 and others). To revive it, reopen that decision first.

## grounding (main's external cross-check — 2026-09-27)

| Claim | What was cross-checked | Decision |
|---|---|---|
| CLI % formula = 3 input kinds / window | binary `…input_tokens+e.cache_creation_input_tokens+e.cache_read_input_tokens,s=Math.round(r/n*100),g=Math.min(100,Math.max(0,s))…` | supported |
| `init.effort` only when defined | binary `if(e.effort!==void 0)i.effort=e.effort` | supported |
| `modelUsage.contextWindow` schema | binary `costUSD:k(),contextWindow:k().int(),maxOutputTokens:k().int()` | supported |
| our Claude decoder skips `system/init` | `backend/claude/mod.rs:863` comment "system/init·thinking_tokens 등 메타 라인 … skip" | supported |
| vibe-kanban 200k default · `[1m]` → 1M | `claude.rs:723` · `:1264` | supported |
| t3code 「Avoid getContextUsage」 | `ClaudeAdapter.ts:2677` | supported |
| paseo Codex = `last.totalTokens` · `modelContextWindow` · ring 90/70 | `codex-app-server-agent.ts:928,930` · `context-window-meter.tsx:67,70` | supported |
| Codex notification has `modelContextWindow` | extracted schema `ThreadTokenUsageUpdatedNotification.json` | supported |
| Codex TUI `BASELINE_TOKENS = 12000` | received upstream `protocol.rs:2417,2452` | supported |
| start response has `model` · `reasoningEffort` · `turn/start` has `effort` | extracted schema `ThreadStartResponse.json` · `TurnStartParams.json` | supported |
| our Codex deserialization has only two fields | `backend/codex/protocol.rs:634-640` | supported |
| ignore list has `model/rerouted` · `thread/settings/updated` | `backend/codex/decoder.rs:171,205` | supported |
| statusline doc original (input only) | the collector's quote — main did not open the web doc directly (the binary formula supports the same content) | partly supported |

## Adversarial review (cross-family · codex · 2026-09-27)

Decision **FIX** → all 7 items applied (only those with a refuting source attached were adopted — each place applied is marked 「적대 리뷰」).

| Finding | Defect type | Handling |
|---|---|---|
| 「피어는 전부 같은 모양·문턱」 (peers all share one shape · threshold) | exaggeration | narrowed §0-1 to GUI peers and corrected the thresholds to differ |
| held up vibe-kanban's `message_delta` sum as a model | confident-wrong · logic gap | under-count warning in the table · 「본받지 말 것」 (do not copy) |
| trusting `modelUsage.contextWindow` as the denominator | missing vendor caveat | §2 now says 「CLI 가 믿는 한도 ≠ 공급자 한도」 (the limit the CLI believes ≠ the provider limit) |
| the reason for rejecting `getContextUsage` is stale | exaggeration · omission | replaced the §5 reason with 「CLI 경로라서」 (because it is the CLI path) |
| Codex 「첫 프롬프트 직후 100%」 (100% right after the first prompt) | exaggeration | made conditional (`last.totalTokens ≤ 12000`) |
| HEAD line numbers for 0.156.1 behavior | misattributed citation | replaced with tag-pinned line numbers (HEAD in parentheses) |
| ccstatusline · ccusage omitted | completeness | added only the names to the table (bodies unread — uncertain) · left in the gaps |

**Reviewer re-check (spot):** the statusline formula (3 input kinds · output excluded) = supported · Codex notification `last`/`total`/nullable `modelContextWindow` (0.156.1 schema) = supported · Codex 12000 baseline (0.156.1 TUI) = supported (the first-prompt conclusion is conditional) · our decoder skipping `system/init` = supported · vibe-kanban `message_delta` = partly supported.

## Gaps · limits

- Whether `effort` is carried in the `init` of the `-p` stream · whether `init` comes again after `set_model` — **needs confirmation with one real run**.
- When the first Codex `thread/tokenUsage/updated` arrives (before the first turn? right after resume?) · when `thread/settings/updated` fires — unmeasured. The footer's initial value depends on this.
- Whether the Codex TUI applies the `effective_context_window_percent` (default 95) correction to the window size it uses — partly traced.
- The Claude desktop app's source and thresholds · the bodies of ccstatusline · ccusage (the alternative context calculation) · the threshold of Claude's built-in auto-compaction warning — not surveyed.
- ★**Reading Claude usage from `assistant.message.usage` (a complete set) looks safe but is unmeasured**★ — the stream's `message_delta` may carry only output (the vibe-kanban warning above). Confirm with one real run whether the `assistant` line in our JSON mode carries all 3 input kinds.
- We do not know whether our Codex test fixture value `modelContextWindow: 272000` (`decoder.rs:1424`) is a measured value.
- Upstream openai/codex is as of HEAD — difference from the 0.156.1 tag unconfirmed.
