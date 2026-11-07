# LLM control command design — query vs. command (applying T-65) — 2026-10-26

- **Status:** adversarial review applied (verdict FIX → revised) · no decision — an input to the T-65 PRD.
- **Method:** `/research` medium · subclass engram · design-decision mode. 4 collection tracks (write-command peers · query results inline/outside · output format · internal constraints) → main grounding (key quotations checked against source text) → codex adversarial review (level 2–3 · web search) → 3 review rebuttals re-checked by main against source text, then applied.
- **Confidence legend:** certain (independent cross-confirmation) · likely (evidence supports · single source) · uncertain · contested.
- **Question (user's framing 2026-10-26):** separate queries from commands. Command = finish in one call where possible. Query = ① pinpoint vs. everything ② carry the result in the context vs. send it outside (a file) — is 「보통은 외부에서 찾게 한다」 (usually have it looked up externally) the industry practice?

## Conclusions (first)

1. **Sending output outside is actually used in two shapes — nobody has measured which one is the industry 「기본값」 (default).**
   - **Size-threshold type:** when the result gets large it is handed off to a file and only the path + the beginning are given — LangChain Deep Agents (over 20,000 tokens → path + first 10 lines · regardless of tool type) · Cursor (long shell · MCP output to a file) · Claude Code (default cap of 25,000 tokens).
   - **Full state is always a file:** the Playwright CLI **returns page info and a snapshot file link with every command** (automatic · no threshold mentioned). It is a CLI where an agent operates a UI, so it is the shape most like ours.
   - So the user's intuition (「전체는 밖으로 내고 찾게, 하나하나는 핀포인트」 — send the whole outside and let it be searched, pinpoint for individual items) **has real precedent only for whole-state queries.** Confidence = likely (each precedent has source text checked · whether it is a 「보편 관행」 (universal practice) is uncertain).
2. **For commands (writes), bundling several steps into one call is the vendor recommendation, and naming the target is an option consistent with that recommendation — but the evidence does not extend to 「이름 지목이 일반 규칙」 (naming the target is the general rule).** Anthropic writes that natural-language names are better than cryptic ids, but that is mainly about the **context that is returned**, and it also allows the flow of finding by name and then acting by id (reviewer's finding · not checked by main). Confidence = likely (bundling) · uncertain (generality of naming).
3. **Pinpoint · filtered queries are the recommendation** (「넓은 검색 한 번보다 작고 겨냥한 검색 여러 번」 — several small targeted searches rather than one broad search · pagination · range · filter · truncation + sensible defaults). Confidence = likely.
4. **There is no universal winner for format — it differs by model · task.** MCP puts a structured result and a text block side by side, but that text is a **backward-compatibility JSON serialization**, so it is not evidence supporting 「구조 계약 + LLM 용 렌더」 (a structured contract + a render for the LLM) (it is only transport compatibility). Whether an LLM-oriented render is better can only be known by measuring on our data. Confidence = likely (no winner).
5. **We did not measure the size of our layout query results** — 「작으니 파일로 내면 호출만 는다」 (they are small, so sending to a file only adds calls) is an unmeasured guess. Measuring the actual size comes before deciding.

## A. Commands (writes) — peers and recommendations

| # | Claim | Source | grounding | Confidence |
|---|---|---|---|---|
| A-1 | Agents handle natural-language names far better than cryptic identifiers — "Agents also tend to grapple with natural language names, terms, or identifiers significantly more successfully than they do with cryptic identifiers." **The place of this sentence is mainly the returned context (what goes in the response)** (reviewer). | [Anthropic — Writing tools for agents](https://www.anthropic.com/engineering/writing-tools-for-agents) | sentence supported (main checked) · generalizing it to write targeting was pointed out by the reviewer as an overstatement | likely (the sentence) |
| A-2 | Bundle several steps into one tool — `schedule_event` instead of `list_users`·`list_events`·`create_event`. Amplitude merged per-endpoint tools per object (cohorts 9→1 · charts 16→4 — this is the **number of exposed tools**, not the number of execution steps). | Anthropic same article · [Amplitude](https://amplitude.com/blog/lessons-from-consolidating-our-mcp-server) | Anthropic supported · Amplitude collector summary | likely |
| A-3 | A mature precedent for naming the target = tmux: the target window is resolved in the order special token → index → id → exact name → name prefix → glob, and if several match it is an **error** (it does not guess). The `=` prefix is exact match only. | [tmux(1)](https://man.openbsd.org/tmux.1) | supported (main checked) | likely |
| A-4 | The closest peer herdr: the pane id is position-based (`w1:p2`), so it changes when moved, and after `pane move` the new id is read from `.result.move_result.pane.pane_id` (the old id is `previous_pane_id`). The agent name is an alias for as long as it is alive (`[a-z][a-z0-9_-]{0,31}` · unique among the living). | [herdr agent automation](https://herdr.dev/docs/agent-automation/) | supported (main checked) | likely |
| A-5 | The response is the minimum needed for the next step — a creating command returns the new id (herdr `pane split` → `.result.pane` · zellij `new-pane`·`new-tab`). The i3 IPC command response is only `{success}`. An example of a 「간결/상세」 (concise/detailed) enum: 72 tokens vs. 206 tokens. | Anthropic · herdr · [zellij CLI](https://zellij.dev/documentation/cli-actions.html) · [i3 IPC](https://i3wm.org/docs/ipc.html) | Anthropic·herdr supported · zellij·i3 collector summary | likely |
| A-6 | An error must say how to fix it — "prompt-engineer your error responses to clearly communicate specific and actionable improvements, rather than opaque error codes". | Anthropic | supported | likely |
| A-7 | A precedent for returning a candidate list on an ambiguous name = CeylonCharts MCP (partial · fuzzy name resolution + a JSON candidate list when ambiguous). It is a small peer and not evidence that it is widely used. | [CeylonCharts MCP docs](https://github.com/CreonSolutions/ceyloncharts-mcp-docs/blob/master/docs/mcp-tools.md) | presented by the reviewer · not checked by main | uncertain |
| A-8 | The claim that herdr `pane.move` also returns the updated layout | herdr socket API (collector summary) | not on the agent-automation page · whether it is the whole tree or a part is unchecked | uncertain |

- **Playwright MCP is on the opposite side** — it targets by snapshot ref, so a read comes before the action (collector summary · unchecked).

## B. Queries — inline vs. outside, pinpoint vs. everything

| # | Claim | Source | grounding | Confidence |
|---|---|---|---|---|
| B-1 | Claude Code limits tool responses to 25,000 tokens by default. The recommendation = a combination of pagination · range selection · filters · truncation, and sensible defaults. | Anthropic | supported | likely |
| B-2 | Instead of truncating long shell·MCP output, Cursor **writes it to a file** and has the agent look at the end with `tail` and read more if needed (because truncating loses information). It did not state a size threshold. | [Cursor — dynamic context discovery](https://cursor.com/blog/dynamic-context-discovery) | supported (main checked) | likely |
| B-3 | It recommends several small targeted searches over one broad search — "make many small and targeted searches instead of a single, broad search". | Anthropic | supported | likely |
| B-4 | LangChain Deep Agents: when a tool result exceeds 20,000 tokens it is handed to storage and replaced with a **file path + a 10-line preview of the beginning** — regardless of tool type (web · DB · API). | [Deep Agents — context engineering](https://docs.langchain.com/oss/python/deepagents/context-engineering) | supported (main checked — review rebuttal applied) | likely |
| B-5 | **Playwright CLI: every command returns page info and a snapshot file link** (a timestamped `.yml` under `.playwright-cli/`) — automatic, and no size threshold is mentioned. A precedent for always sending the whole UI state to a file. | [Playwright CLI — snapshots](https://playwright.dev/agent-cli/snapshots) | supported (main checked — an omission found by the reviewer) | likely |
| B-6 | Other export cases: Manus (「파일 시스템을 궁극의 컨텍스트로」 — back-translated: the file system as the ultimate context · recoverable compression that keeps only URLs · paths) · Playwright MCP (the `filename` argument of `browser_snapshot` — optional). | [Manus](https://manus.im/blog/Context-Engineering-for-AI-Agents-Lessons-from-Building-Manus) · [Playwright MCP](https://github.com/microsoft/playwright-mcp) | collector summary | likely |
| B-7 | **No source was found** that traded off, in numbers, the calls that exporting adds against the context it saves. Cursor's −46.9% is the share from dynamic discovery of MCP tool descriptions, not tool results. | Cursor | supported (the target of that figure checked) | — |
| B-8 | An example of filtering results before they reach the context — in a code-execution environment, filtering a 10,000-row table so 「에이전트는 10,000 행 대신 5 행을 본다」 (back-translated: the agent sees 5 rows instead of 10,000). ★The 150,000 → 2,000 tokens in the same article is the share from **loading only the needed tool definitions**, not result filtering★ (the draft's misattribution — reviewer rebuttal · main checked the source text). | [Anthropic — code execution with MCP](https://www.anthropic.com/engineering/code-execution-with-mcp) | supported (main checked) | likely |
| B-9 | A large context cuts accuracy (context rot — 18 models, a focused prompt of ~300 tokens vs. the full ~113k). It is long-context retrieval research, and carrying it over to tool results is an inference. | [Chroma — context rot](https://trychroma.com/research/context-rot) | collector summary | likely (the finding) · application is an inference |

## C. Output format — text vs. JSON

| # | Claim | Source | grounding | Confidence |
|---|---|---|---|---|
| C-1 | No universal winner — Anthropic: the response structure (XML·JSON·Markdown) affects evaluation performance and the optimum differs by task·agent, so choose by evaluation. A prompt-format paper: GPT-3.5 swings widely with format and GPT-4 less, and the best format does not transfer well across model families. | Anthropic · [arXiv 2411.10541](https://arxiv.org/abs/2411.10541) | Anthropic supported · paper collector summary | likely |
| C-2 | Tokens: for flat tabular data, compressed formats are far shorter than JSON (TOON's own benchmark: JSON 6,326 vs. TOON 2,537 vs. CSV 2,336). For nested data it **differs by data** — deep config is JSON-compact 562 < TOON 669, but nested-contact is TOON 1,444 < JSON-compact 2,357 (reviewer). Accuracy differs by a few points on strong models and by a lot on small models. | [TOON benchmark](https://toonformat.dev/guide/benchmarks) (format-promoter side) · [Improving Agents — TOON](https://www.improvingagents.com/blog/toon-benchmarks) (independent) | collector·reviewer summary · not checked by main | uncertain |
| C-3 | There is a study comparing nested-data formats — comparing JSON·YAML·XML·hierarchical Markdown on 6–7-level nested config (small model GPT-5-nano: YAML 62.1 · Markdown 54.3 · JSON 50.3 · TOON 43.1). It is not window-layout data. There is also an EACL 2026 paper measuring the handling of complex JSON tool responses on 15 models (presented by the reviewer · unchecked). | [Improving Agents — nested format](https://www.improvingagents.com/blog/best-nested-data-format) · [EACL 2026](https://aclanthology.org/2026.eacl-long.134/) | presented by the reviewer · not checked by main | uncertain |
| C-4 | MCP puts a structured result (`structuredContent`) and a text block side by side — "For backwards compatibility, a tool that returns structured content SHOULD also return the serialized JSON in a TextContent block." It is a **backward-compatibility JSON serialization**, not a render for the LLM. | [MCP 2025-06-18 tools](https://modelcontextprotocol.io/specification/2025-06-18/server/tools) | supported (main checked) | likely |
| C-5 | CLI precedents: `gh --json/--jq` · `kubectl -o` · `docker --format` — a human-oriented default screen + a machine mode. | knowledge-based (source not looked at this time) | unchecked | uncertain |

## D. Internal constraints (code measured)

- **All current layout commands target by opaque id** — tab `view_id` · slot `slot_id` · split `split_id` · window `window_id`/the same launch label (numbers are rejected · ADR-0302). No layout command takes an agent name. If a name is passed to `slot.assignAgent`, it is not rejected and the slot shows empty (the layout layer does not know about agent existence · ADR-0035). (`src-tauri/src/layout/commands.rs`)
- **Queries are small and narrow in scope** — `tab.list{window}` · `window.list` · `split.list{view_id}` · `slot.resolveSpatial{token}`. **There is no command that reads back slot contents and no whole-layout query** (measured — not in the command name list).
- **Write responses are small** — creating commands return an id (`tab.create` → `{view_id}` · `slot.split` → `{slot_id}`), window-state commands return a small state field (`window.maximize` → `deferred` · `window.raise` → `shown_all`·`focused`·`pending` — reviewer correction), the rest are `{}`.
- **The CLI prints the response body as it is** (`crates/engram-dashboard-cli/src/bin/engram.rs:264`) — no truncation · pagination · text render · file export. Only the `help` and command-list screens are drawn as text.
- **The daemon does not interpret the body of UI commands** (ADR-0081 · ADR-0156) — changing the response shape must be done in the shell handler or the CLI.
- **A name-resolution precedent already exists** — agent crate `resolve_in` (`crates/engram-dashboard-agent/src/commands.rs:1686`): id first, then exact name, a duplicate name is CONFLICT, none is NOT_FOUND (the same 「모호하면 오류」 (error if ambiguous) as tmux). **But the shell does not depend on the agent crate** (ADR-0270) — the shell has an id→name cache filled by daemon list events (`agent_names` in `layout/manager.rs`) and it is never cleared, so stale names can remain.
- **The docking apply service already takes it in one go** — `apply::dock_tab(DockRequest{view, to: DockDest{Seam{target, anchor, side, share} | TabBar{window, index} | NewWindow{..}}, focus, mv})` (`src-tauri/src/layout/apply.rs`). `NewWindow` deadlocks if called from the main thread · a synchronous handler. TRD slot docking §3-6 has planned the bus name `tab.dock` (focus kept).
- **User criterion (old memo · T-65)** = a common request is 1–2 calls · 「LLM 시나리오는 여러 번 거치지 않아야 실수가 적다」 (an LLM scenario makes fewer mistakes if it does not go through many steps). The old memo's candidates = read `layout.get` (everything) · write `tab.dock`/`tab.move`/`tab.detach` (target by agent name · if ambiguous, reject with a candidate list). The 「명령 하나 + 모드」 (one command + a mode) option (TRD v4) was pushed out because of room for combination mistakes.
- **Spatial targeting is a logical handle** (direction · neighbor · ordinal · tmux-style token), and exposing pixels was deferred (ADR-0068). Reading values is mandatory in v1 (ADR-0156 — user decision).

## Candidates × constraint fit

The call count is for the same task (a common request such as 「에이전트 X 를 Y 옆으로」 (put agent X next to Y)) and **since sizes were not measured, the context column is only a relative comparison** (reviewer's finding — a ranking can only be given by comparing the same task · the same size).

Query side:

| Candidate | Calls | Context | Fit to our constraints | Precedent |
|---|---|---|---|---|
| Q1 everything inline (JSON as is) | 1 | proportional to layout size | easy (the current CLI as is) | — |
| Q2 everything always to a file · only the path (+ a summary) inline | 1 + as many reads as needed | minimum | a file-write path must be added to the CLI·shell | Playwright CLI (B-5) |
| Q3 pinpoint query (「에이전트 X 어디」 (where is agent X) · filter argument) | 1 | minimum | possible in the shell handler | Anthropic recommendation (B-3) |
| Q4 everything inline + a cap, over the cap a file + a preview | 1 (if over, 1 + a read) | within the cap | render in the shell handler · only the overflow goes to a file | Deep Agents · Claude Code (B-1 · B-4) |

Command side:

| Candidate | Calls | Room for mistakes | Fit to our constraints |
|---|---|---|---|
| W1 name targeting + in one go (no query) · the response is what changed (including new ids) · error if ambiguous (+ candidates) | 1 | name ambiguity → blocked by an error | the shell needs name resolution (handle stale cache) |
| W2 id targeting (the current way) | query 1 + command 1 | id copy mistakes | as is |
| W3 one command + a mode (one `tab.dock` with a destination kind) | 1 | combination mistakes (old verdict) | 1:1 with `DockDest` |
| W4 several commands per destination (`tab.dock`/`tab.move`/`tab.detach`) | 1 | command-choice mistakes | a thin set of three over the same apply service |

Format side: F1 JSON as is · F2 a structured contract + a text render for the LLM (shell handler or CLI · `format` argument) · F3 a TOON-type compressed format — for all three, the evidence alone does not decide a ranking (C-1 to C-3). A small evaluation on our data is needed.

## Rejected candidates (→ material for the ADR's rejected alternatives)

- **Reading via the CDP DOM** — the old memo already rejected it (no authentication · a render, not the model · brittle · large).
- (The draft's 「Q2 를 기본으로 거부」 (reject Q2 as the default) · 「F3 일괄 거부」 (reject F3 wholesale) were withdrawn after the review — the evidence cannot support those rejections.)

## Gaps · limits

- **The actual token size of the layout query results was not measured** — it is the premise of the ranking of the Q candidates.
- The tree-data format evaluation was not run on our layout (C-3 is general config data).
- The prevalence of 「업계 기본값이 인라인이냐 파일이냐」 (whether the industry default is inline or a file) was measured by no source (conclusion 1).
- Sources presented by the reviewer whose source text main could not see: CeylonCharts (A-7) · the TOON nested-contact figures and the independent test (C-2) · the Improving Agents nested study and EACL 2026 (C-3) · Anthropic's allowance of 「이름으로 찾은 뒤 id 로 행동」 (find by name, then act by id) (conclusion 2).
- Peers not seen: Hyprland · sway · yabai · komorebi · GitHub MCP · OpenAI function-calling recommendations (collection budget).
- No numeric source for how much naming the target reduces the call count.

## Adversarial review results (codex · level 2–3 · verdict FIX)

| # | Target | Defect | Severity | Handling |
|---|---|---|---|---|
| 1 | A-1/A-2 → conclusion 2 | overstatement — the name sentence is about the response context, and the Amplitude figure is about tool count | medium | conclusion 2 · A-1 · A-2 softened |
| 2 | conclusion 1 | no basis for the generalization + a missing counterexample (Playwright CLI automatic file link) | high | **main checked the source text → conclusion 1 rewritten** · B-5 added |
| 3 | B-5 (draft) · rejecting Q2 | the limit 「끝없는 결과만」 (only endless results) is wrong — Deep Agents is regardless of tool type | medium | **main checked the source text** → B-4 · the Q2 rejection withdrawn |
| 4 | B-7 (draft) 150k→2k | misattribution — the share of tool definitions | high | **main checked the source text** → B-8 corrected |
| 5 | C-3 → conclusion 4 | the MCP text block is only for backward compatibility | medium | conclusion 4 · C-4 corrected |
| 6 | C-5 (draft) 「연구 없음」 (no study) | an excessive absence claim | medium | changed to C-3 (marked unchecked) |
| 7 | F3 wholesale rejection | over-generalization — even in nested data it differs by data | medium | rejection withdrawn · C-2 reinforced |
| 8 | candidate list has no peer | a missing counterexample (CeylonCharts) | medium | A-7 added (unchecked) |
| 9 | conclusion 5 · the Q table | unmeasured inference · Q2/Q4 call counts inconsistent | high | conclusion 5 made 「먼저 잴 것」 (what to measure first) · the Q table call counts aligned |
| 10 | D 「나머지는 `{}`」 (the rest are {}) | a code fact is wrong | low | D corrected |
