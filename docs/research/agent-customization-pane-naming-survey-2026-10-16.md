# Naming practice for the agent-customization bundle and its pane — light survey

| Item | Value |
|---|---|
| Status | survey done · decision = slot name 「라이브러리 1~4」 (nothing appended — generic · user 2026-10-16) |
| Date | 2026-10-16 |
| Method | `/research` light — 1 collector (4 searches) · 2 main grounding spot-checks (WebFetch) · no adversarial review |
| Question | What do we call the slot for browsing instructions · skills · settings · presets (bundles) together. Candidates = library · asset · preset browser · customization · block |
| Confidence legend | certain = independent cross-confirmation · likely = single source backed by grounding · uncertain = unconfirmed |

## Conclusion

- **The closest precedent among AI coding tools is Cursor's 「Customize」 page** — it gathers plugins · skills · MCP · rules · subagents · commands · hooks in one place and manages them at user · team · workspace scope. (Likely — the main agent checked the original text.) ★But the use is different★ — it is a collection of settings applied in bulk to every agent in that scope, not a place where each agent picks and attaches its own bundle, as ours does (the only overlapping material = rules↔instructions · skills · plugins↔presets). So it is weak as a basis for the name.
- **We did not find any AI coding tool that uses 「에셋」 as the umbrella term for this bundle.** (Uncertain — within the range of 4 searches)
- **We did not find anyone using 「라이브러리」 as the name of this pane either, in this survey.** List UIs looked like a 「마켓 · 허브」 (market · hub), as with Discover and the marketplace in Claude Code `/plugin`.
- The name for a bundle (our preset) differs by tool — Claude Code = plugin. We did not find anyone using 「preset」.

## Findings

| Tool | Umbrella term (original) | Pane · UI name (original) | Source | Confidence · grounding |
|---|---|---|---|---|
| Cursor | "Plugins, skills, and MCPs let you customize Cursor" | 「Customize」 page in the sidebar — plugins · skills · MCP · rules · subagents · commands · hooks | https://cursor.com/docs/customize-cursor | likely · main check = supported |
| VS Code + Copilot | customization (doc path `copilot-customization` — user instructions · prompt files · chat modes) | name of a dedicated panel unknown | https://code.visualstudio.com/docs/copilot/copilot-customization | likely · not checked by main |
| Claude Code | plugin (bundles skills · agents · hooks · MCP and more into one unit) | Discover tab of `/plugin` · marketplace | https://code.claude.com/docs/en/plugins/overview | likely · not checked by main |
| Continue | collector report = blocks (pieces) · assistants (bundles) · Hub | collector report = Hub | https://docs.continue.dev/reference | **uncertain — main check = not supported** (that page has no definition of blocks · assistants · Hub) |
| Windsurf | Rules · customization icon (secondary source) | unknown | secondary source (platzi) | uncertain |

## Gaps

- Cline · JetBrains AI · OpenAI Codex were not searched.
- The panel name of VS Code's latest chat-customization UI and the Windsurf panel name are unknown.
- The 「에셋 · 브라우저」 (asset · browser) practice of game · 3D tools (Unreal Content Browser · Blender Asset Browser) is outside the scope of this survey (from session memory — unconfirmed).
