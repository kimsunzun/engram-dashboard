# Agent preset component survey — what gets bundled and how it is assembled (2026-10-11)

- **Status:** survey done · before the decision (before the skill-preset PRD — preceding survey = `external-skill-dir-loading-claude-codex-2026-10-10.md`)
- **Method:** `/research` extension · collector 1 (docs · local clones) · main cross-check (paseo source lines) · **cross-family adversarial review skipped** (degraded — it is a survey of the component list, so the conclusion does not stand in for the user's decision)
- **Confidence:** certain = official docs · direct source reading · likely = secondary sources · uncertain = unchecked

## Component kind × tool

| Component | Claude Code | Codex CLI | Injected on every run |
|---|---|---|---|
| Instructions | `--append-system-prompt(-file)` · CLAUDE.md · output style | `developer_instructions` · AGENTS.md | both work (certain · the codex limit is in the preceding report) |
| Skills | SKILL.md (Agent Skills standard) | same format | claude `--add-dir`/`--plugin-dir` · codex app-server `extraRoots` (certain) |
| MCP server | `--mcp-config` · `.mcp.json` · plugin | `[mcp_servers.<id>]` · plugin `mcp.json` | works (certain) |
| Hooks | settings `hooks` · plugin · agent frontmatter | `hooks` · plugin | works via claude `--settings` (certain) |
| Subagent definition | `.claude/agents/*.md` · `--agents` JSON | not found in the docs (uncertain) | claude works |
| Slash command · prompt | `commands/` (skills recommended) | custom prompts (uncertain) | files · plugins only |
| Model · reasoning effort | `--model` · `--effort` | `model` · `model_reasoning_effort` | works (certain) |
| Permissions · sandbox · tools | `--permission-mode` · `--allowedTools`/`--disallowedTools` | `approval_policy` · `sandbox_mode` | works (certain) |
| Environment variables | process env | process env · `shell_environment_policy` | works |
| Extra folders | `--add-dir` | `--add-dir` (likely) | works |
| Start prompt | positional argument · agent `initialPrompt` | positional argument | works |
| Bundle (plugin) | `plugin.json`: skills · commands · agents · hooks · MCP · LSP · output styles and more · `--plugin-dir` is session-only | `plugin.json`: skills · `mcp.json` · apps · hooks | claude session-only · codex has no run argument (uncertain) |

- **Core common to both backends (collector's summary):** instructions · skills · MCP · model · reasoning effort · permission level · environment variables · working folder/extra folders · start prompt. **Backend-specific:** hooks · output styles · subagents · plugins.

## How bundles are assembled

- **Flat fields in one file:** claude subagent frontmatter · Copilot `.agent.md` (YAML fields + a body prompt). https://code.claude.com/docs/en/sub-agents
- **Layering:** codex = system < managed < user < project < profile < CLI `-c`. claude = managed > CLI > local > project > user. https://learn.chatgpt.com/docs/config-file/config-reference
- **Composite bundle:** plugins (claude · codex · Gemini) = component folders + a manifest. https://code.claude.com/docs/en/plugins-reference
- **Inheritance · backend-neutral run record (the closest peer):** paseo `ProviderOverride` (`extends` · `label` · `command` · `env` · `params` · `models` · `disallowedTools` · `enabled` · `order` — `packages/protocol/src/provider-config.ts:47-59`) and the backend-neutral `AgentSessionConfig` (`systemPrompt` · `daemonAppendSystemPrompt` · `modeId` · `model` · `thinkingOptionId` · `featureValues` · `toolPolicy` · `mcpServers` · `env` … — `packages/server/src/server/agent/agent-sdk-types.ts:575-612` · each backend maps it into its own fields). Certain (main read it directly).
- orca has only trust presets · for t3code · vibe-kanban we found no profile concept (grep · uncertain). The internals of Zed · Continue · Cursor · Kiro · Cline/Roo are unchecked.

## Cross-tool standards

- **Agent Skills** (agentskills.io) — SKILL.md (`name` · `description` · `license` · `compatibility` · `metadata` · experimental `allowed-tools`) · both claude and codex read it (certain).
- **Agent Plugins 1.0** (one secondary source says it was announced 2026-08-06 · no official spec found · likely) — `plugin.json` + `skills/` + `mcp.json`. The codex docs confirm a portable root `plugin.json`.
- AGENTS.md · MCP · ACP — not rechecked. ACP is a session protocol, not a component bundle.

## Addendum (2026-10-12) — UI screen survey · source-matching practice · claude model list

Collected while making the UI briefing (the artifact 「Engram 프리셋 화면」). No adversarial review (degraded).

- **UI patterns (docs · source; we did not see screen captures):** Claude Code `/plugin` = tabs (Discover · Installed · Marketplaces · Errors) + the 「Will install」 list in the detail window and the context cost (tokens per turn / per call) · scope user/project/local · managed items are shown locked. The `/agents` wizard was retired in v2.1.198 and replaced by files + 「에이전트에게 시키기」 (docs). Cursor Rules = 4 ways of applying per item + layer order Team > Project > User (no expanded-result view). Zed = a profile picker in the panel + a 「Configure」 modal (create · fork · delete · model · tools) — the only reuse is fork. VS Code Profiles = create from empty / clone / template, with per-kind on · off · kinds not turned on inherit Default · applied per window. Copilot custom agents = `.agent.md` · chat dropdown. Roo modes = a form (name · description · role · tool group · instructions) · a project mode with the same slug overrides the global mode wholesale. paseo = agent profiles on the settings page (provider + model + mode + thinking + feature values) · even if the referenced target is gone, the list row is drawn with the saved id (`packages/app/src/agent-profiles/`). orca = a search combo + 「Manage agents」 + right-click default. t3code = a per-provider bundled model picker (`components/chat/ModelPickerContent.tsx`). **We found no tool that has nested bundles + drag ordering + an expanded-result view together (likely).**
- **Source-matching practice (based on the researcher's memory · docs unverified):** the recurring shape = name · id inside the folder, path in the PC settings (Claude marketplace `marketplace.json` name + `plugin@마켓` · Go `go.mod` + `go.work` · Bazel `MODULE.bazel` + `local_path_override` · Syncthing folder ID). A counter-example to avoid = Obsidian (the id exists only in the PC app settings, so PCs do not match each other). URL identification (submodule · flake · Terraform) is weak at telling forks · mirrors · subfolders apart. The researcher's recommendation = a human-readable id inside the folder (`engram-source.toml`) + the PC settings map id → path.
- **claude model list (measured · 2.1.284):** on the JSON channel (`-p --input-format stream-json --output-format stream-json --verbose`), send one line `{"type":"control_request","request_id":"…","request":{"subtype":"initialize"}}` before the first message → `response.response.models` of the `control_response` (5 entries — `default` · `opus` · `claude-fable-5-1[1m]` · `sonnet` · `haiku`). Per model `supportedEffortLevels` (low~max) · `supportsEffort` · `supportsFastMode` and so on. Zero tokens (no user message). The same response contains account information (`account`) — do not leave it in records · logs. The Agent SDK's `supportedModels()` reads this response (`@anthropic-ai/claude-agent-sdk` 0.3.296). The list may differ by account · plan. For codex, app-server `model/list` (per-model supported efforts · default effort — 0.156.1 schema).

## Addendum (2026-10-12 afternoon) — putting into a bundle · tags · common + per-target settings

Researcher 1 (web) · only VS Code Profiles and the codex settings order were opened as docs; the rest is **memory-based and unverified** (the Linear labels doc 404 · the Unreal Collections body is empty). No adversarial review.

- **Putting in:** search-style pickers and an item-side 「…에 추가」 menu hold up to hundreds of items, while move-between-lists · drag-and-drop are right for dozens (interpretation). For items already in, a check · dimming · an 「N개에 포함」 badge is the common pattern. Examples with two-way entry = Spotify · Unreal Collections (unverified). The researcher's recommendation = both a search picker and an item right-click, with cycles disabled in the picker.
- **Tags:** Obsidian nested tags (`a/b` · in the file · git-portable) · Gmail nested labels (stored by the app) · Lightroom hierarchical keywords + keyword sets · Linear label groups (only one within a group — uncertain) · Unreal Static/Dynamic collections (unverified). Storage location = in the file / a sidecar file / the app DB — a matter for the user to decide.
- **Common + per-target settings:** VS Code per-language blocks (`"[python]": {…}`) · EditorConfig sections · docker-compose override · codex = default `config.toml` < profile overlay < project `.codex/config.toml` < CLI `-c` (docs confirmed — https://learn.chatgpt.com/docs/config-file/config-advanced). Common to UIs = a common tab + per-target tabs + an 「덮임」 marker + a raw-text field.

## Addendum (2026-10-12) — is a full-snapshot push the app practice

Researcher 1 (web) · no adversarial review. **Conclusion: with a small list + low frequency + local IPC, a full-snapshot push is the practice (certain).** Partial updates (deltas) are an optimization for when one of size · frequency (tens per second or more) · remote bandwidth · concurrent editing is the problem (likely · no numeric threshold found).
- React docs (read directly): same position · same key keeps state, a changed key recreates it — flicker is a drawing problem, not a transport method (stable id keys · keep the previous data during an update · do not recreate). https://react.dev/learn/preserving-and-resetting-state
- TanStack Query (read directly): if the response has new data, swap the cache for a new object (no direct mutation). https://tanstack.com/query/latest/docs/framework/react/guides/updates-from-mutation-responses
- Kubernetes (read directly): list (full) + watch (deltas after that, resourceVersion) — if the deltas break (410) it falls back to the full list. Even the delta approach keeps the full snapshot as its recovery path. https://kubernetes.io/docs/reference/using-api/api-concepts/
- Implication for Engram: the current way of discarding old snapshots with a monotonic number = the same idea as resourceVersion. The only place where deltas fit is a high-frequency stream like terminal output, which already uses seq/replay. The LSP · Redux · VS Code webview · CRDT items are unverified.

## Addendum (2026-10-12) — the 「모음에 추가」 (add to collection) popover

Researcher 1 (web) · opened 5 official docs, but they had almost no UI detail (checkboxes · search · when it applies) — **most details are memory-based and unverified.** Confirmed from docs = YouTube 「Save」 (pick an existing playlist or Create new · a toast after adding) · Gmail (several labels on one item) · Unreal (right-click Add to Collection submenu · new collection · drag-in mentioned). The researcher's recommendation = apply on check immediately + a toast undo · places already holding it are checked + moved up · search → recent → all → 「+ 새로」 (if there is no search term, `"검색어"로 만들기`) · places that cannot hold it are disabled with a reason instead of hidden · keyboard (on open focus the search · ↑↓ · Space toggles · Esc).
