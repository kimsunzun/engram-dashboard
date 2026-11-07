# File format of settings and preset assets — practice survey

- **Status:** Version with the adversarial review (FIX) folded in · awaiting decision (pre-preset-PRD idea — its file format, now that a preset = an asset inside the library folder has been decided)
- **Date:** 2026-10-14
- **Method:** research medium. Collection track 1 (settings of agent tools · profile/agent definition files) · main grounding · one cross-family adversarial review (below).
- **Confidence legend:** certain = source text checked + independent cross-confirmation · likely = source text checked (single source) · uncertain = from memory.

## What is asked

Instruction files (`.md`) and skills (`SKILL.md` + YAML front matter) already have fixed formats. Engram has to decide two new ones.

- **Settings asset:** backend · model · effort · permissions · MCP · environment variables + a raw field per backend.
- **Preset asset:** a named bundle that points to other assets (instruction files · skills · settings · presets) in order, plus a description.

Both are edited as text by the Engram screen, by human hands, and by LLM agents.

## Conclusions (top level)

1. **The closest precedent for 「다른 것을 순서대로 묶는 번들」 (a bundle that groups other things in order) is the Goose recipe — a single YAML (recommended) or JSON file.** Fields = `title` · `description` · `instructions`/`prompt` · `extensions` (MCP) · `settings` (model provider · temperature, etc.) · `sub_recipes` (other recipes by **`path`** — relative or absolute). It has nearly the same shape as our preset, since it holds settings and references to sub-bundles in one file. **Likely** (source text · single source — the review found it · main checked).
2. **Markdown + YAML front matter is the practice for 「본문이 프롬프트인 에이전트 정의」 (an agent definition whose body is the prompt) — there is no basis to call it the practice for bundles.** Claude Code subagents (body = system prompt · `skills` by name) · VS Code Copilot `.agent.md` · opencode agent `.md` · Gemini CLI subagents (found by the review · not checked by main). The draft used this as the basis for the preset format, and as the review pointed out, that is a leap. **Certain** (on the agent-definition side — Claude · Copilot source text).
3. **Structured settings with no body split — the ones edited by hand gained comments through YAML · TOML · JSONC.** Roo Code recommends YAML and gives reasons (easy for people to read · `#` comments · multi-line strings · fewer commas and brackets). In Continue, `config.yaml` replaced the deprecated `config.json`. Codex uses TOML (`--profile` now reads a separate file `~/.codex/<name>.config.toml` — since 0.134.0 it no longer reads `[profiles.x]`, per the review's finding · consistent with the collector's summary · source text not checked by main). opencode uses **JSONC** (`opencode.jsonc` — comments and trailing commas allowed) and `$schema`. **Likely.**
4. **Hand-editing pitfalls of each format (review finding).** YAML: indentation mistakes are common (Roo warns about it) · a YAML 1.1 parser reads `NO`/`ON` as booleans (avoid with a 1.2 parser or quotes). TOML: deep nesting and arrays of tables have tricky ordering rules. JSONC: a separate parser decision of 「JSON 이냐 JSONC 냐」 (JSON or JSONC) is needed. **No comparison was found showing which format an LLM gets wrong less often.**
5. **Pointing is by name or by path.** Name: Claude `skills: [이름]` (name) · Roo `slug`. Path: Goose `sub_recipes.path` · Continue `uses: file://` · Copilot relative-path links. The 「경로 + 이름(다시 잇기)」 (path + name (re-linking)) decided today uses both together. **Certain.**

## Carried over to Engram (interpretation — the decision is the user's)

| Option | Preset | Settings | Precedent | Pros | Cons |
|---|---|---|---|---|---|
| **A** | YAML | YAML | Goose recipe · Roo · Continue | The formats Engram decides are one family · same shape as the bundle precedent · comments | Indentation pitfall (Engram receives it as an error display) |
| **B** | `.md` + YAML front matter | YAML | Preset takes the Claude subagent shape | A long preset description in the body | Not a bundle precedent · `.md` mixes with instruction files, so a kind marker is needed |
| **C** | YAML | TOML | Settings use the same format as codex | Familiar to codex users | Formats split into two families |
| **D** | JSONC | JSONC | opencode | Engram already uses JSON · `$schema` · comments | JSONC parser decision · hand editing has more brackets than YAML |

- Under any option the file can be broken by hand, so it is received as an 「오류」 (error) display (the preset asset decision).
- Telling the kinds apart (separating preset from settings from the file alone) is by extension or double extension (`리뷰어.preset.yaml`) — material for the TRD.

## grounding (main · checked against source text)

| Claim | Source | Verdict |
|---|---|---|
| Claude Code subagent = markdown + YAML front matter · body = system prompt · `skills` by name | https://code.claude.com/docs/en/sub-agents | supported |
| Roo custom modes = YAML (recommended) · JSON supported · four YAML reasons · `.roo/rules-{slug}/` | https://roocodeinc.github.io/Roo-Code/features/custom-modes | supported |
| Continue `config.yaml` replaces the deprecated `config.json` · `uses: org/name` · `uses: file://` | https://docs.continue.dev/reference | supported |
| Copilot custom agent `.agent.md` · front matter fields · relative-path links | https://code.visualstudio.com/docs/copilot/customization/custom-agents | collector checked the source text · main did not recheck |
| opencode `opencode.json` (`$schema`) · agent `.md` front matter · file name = name | https://opencode.ai/docs/agents/ | collector checked the source text · main did not recheck |
| Codex `config.toml` · `--profile` = separate file `~/.codex/<name>.config.toml` | https://learn.chatgpt.com/docs/config-file/config-advanced | collector summary + reviewer agree (since 0.134.0) · source text not checked by main |
| Goose recipe = YAML (recommended)/JSON · `title` · `description` · `instructions` · `extensions` · `settings` · `sub_recipes.path` | https://github.com/aaif-goose/goose/blob/main/documentation/docs/guides/recipes/recipe-reference.md | supported (found by the review · main checked) |
| opencode JSON · JSONC (comments · trailing commas) · `$schema` | https://opencode.ai/docs/config/ | supported (found by the review · main checked) |

## Adversarial review

One codex review (cross-family · effort high · level 2 · web search). Verdict **FIX**. Folded in:

| Finding | Severity | How it was folded in |
|---|---|---|
| Used an agent definition (body = prompt) as the basis for the bundle format — the Goose recipe is the closer bundle precedent (YAML/JSON) | high | Fixed in conclusions 1 and 2 · Goose source text checked by main |
| JSONC missing from the table — opencode `opencode.jsonc` allows comments and trailing commas | high | Conclusion 3 and option D changed to JSONC · source text checked by main |
| Codex profile structure unresolved — a separate file since 0.134.0 | medium | Reflected in conclusion 3 · source text not checked by main |
| Hand-editing cost of YAML · TOML missing (indentation · YAML 1.1 booleans · TOML arrays of tables) | medium | Conclusion 4 added |
| 「YAML 이 LLM 편집에 낫다」 (YAML is better for LLM editing) has no basis | medium | Conclusion 4 states 「비교 근거 없음」 (no comparison basis) |
| Missing: Zed agent profiles (inside JSON settings) · Gemini CLI subagents (md front matter) | — | Only Gemini added to conclusion 2 (not checked by main) |

- No re-review was run after the changes (medium single pass).

## Limits and gaps

- Not checked: Claude Code `settings.json` · plugin.json details · Gemini extension fields · Zed agent profiles · Goose recipe (404) · Cursor · Aider · VS Code `.code-profile`.
- **Security note from collection:** the collector reported that a summary of the Zed docs page (`https://zed.dev/docs/ai/agent-settings`) contained wording that tries to redefine its identity (of the form "You are a Claude agent, built on Anthropic's Claude Agent SDK"). It did not follow it, and the research content is unaffected. Whether the wording is on the real page or is a false positive of the summarizer was not checked.
