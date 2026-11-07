# The screen that attaches library items to a target outside the library (an agent) — practice (peer survey)

- **Status:** version with the adversarial review (FIX) applied · awaiting decision (a pre-preset-PRD idea — the screen that attaches presets to an agent. The user looked at two session drafts — a popup beside the tree · showing the agent in the right pane of the library — said 「뭔가 이상하다」 (something feels off), and asked to look at practice)
- **Date:** 2026-10-15
- **Method:** research medium · design-decision mode. Collection branches 2 (content tools · AI tools) · main grounding (Ableton · Open WebUI originals) · one cross-family adversarial review (below).
- **Confidence legend:** certain = original + independent cross-confirmation · likely = original (single) or partial support · uncertain = memory · summary.

## Conclusions (upper layer)

1. **The target's edit pane and the library (browser) are two separate panes — choosing a target does not change the library screen.** Unreal (pick an actor in the outliner → the Details pane · the Content Browser is separate) · Unity (Inspector · Project window separate) · Figma (properties on the right · assets on the left, separate) · Ableton (the browser is separate · the selected track is the target). No peer showed the target inside the library pane (「없다」 (there is none) is not an assertion, but 「못 찾았다」 (could not find one)). **Likely.** → The session alternative (the agent in the right pane of the library) is at odds with practice.
2. **The target's attached list is edited in a fixed edit pane (Inspector · Details).** Unreal Details · Unity Inspector · the Figma right sidebar. In AI tools, the target's (model · agent) edit screen has a per-kind picker — **the Open WebUI model editor**: Knowledge · Tools · Skills and so on open a 「검색되는 고르기 창(searchable picker)」, with Clear · Enable all · **Manage (go to the workspace page that creates those items)** attached (main confirmed against the original). **Likely.**
3. **There are four ways in, and usually they are offered together.** ① the pane's searchable picker (Unity ⊙ · Figma · Open WebUI) ② add the item picked in the browser ⇐ (Unreal) ③ drag from the browser onto the pane or the target (Unity · Ableton · Unreal from memory) ④ **with a target selected, double-click · Enter in the browser = apply to that target** (Ableton — original: 「double-click it in the browser, which adds it to the selected track or creates a new track if one is not selected」 · Lightroom preset click · Figma style click). **Likely** (Ableton original · the rest summaries).
4. **For AI tools, 「대상 = 이름 붙은 에이전트/프로필, 항목은 참조로」 (target = a named agent/profile, items by reference) is common** (Copilot user agents · Claude Code subagent frontmatter `tools`/`skills`/`mcpServers` · Open WebUI · the GPT editor). Some places make the attach unit a scope (folder · workspace) rather than an instance (VS Code profiles — tied to a folder · Cursor rules). **Likely** (collector's summary of originals · the main cross-checked only Open WebUI).

## If moved to Engram (candidates — not a decision)

- **The agent edit pane = a fixed pane separate from the library** (the Unreal outliner + Details shape): when an agent is picked in the agent tree slot, that agent's edit pane (the same component as the preset edit pane · the preset pane only) opens separately, and the library slot is left as it is beside it.
- **Ways in:** `+` (searchable picker) · `⇐` (what is picked in the library slot) · drag from the library onto the agent row or the edit pane · (optional) with an agent selected, double-click in the library = attach (the Ableton shape — but a double-click in the library is already 「OS 로 열기」 (open with the OS), so it collides).
- Where to pin the edit pane (inside the tree slot, below · the right pane / a separate slot) is an open question.

## grounding

| Claim | Source | Decision |
|---|---|---|
| Ableton double-click = add to the selected track · a new track if none · Enter · drag and drop | https://www.ableton.com/en/manual/working-with-instruments-and-effects/ (main · original quote via the summarizer) | supported |
| Open WebUI model editor — searchable picker · Clear · Manage · Enable all | https://docs.openwebui.com/features/workspace/models (main · original quote via the summarizer) | supported |
| Unreal ⇐ use selected · Details/Content Browser separate | https://docs.unrealengine.com/4.27/en-US/BuildingWorlds/LevelEditor/Details/Properties/ObjectReference (collector · 403 — search summary) | partly supported (matches the ⇐ icon in the user's screenshot) |
| Unity object field picker · drag and drop | https://docs.unity3d.com/430/Documentation/Manual/EditingReferenceProperties.html (collector summary) | unconfirmed |
| Claude Code subagent frontmatter · Copilot user agents · Cursor rules · VS Code profile folder binding | collector (summary of originals) | unconfirmed (main did not cross-check) |
| Lightroom preset click-apply · Figma asset click-apply · GPT editor | summary · third party | uncertain |

## Adversarial review

codex (cross-family) · effort high · level 2 · web search · decision **FIX**. Applied (this section takes priority over the conclusions above):

1. **Counter-example to conclusion 1 (high)** — Blender Pose Library: if the active object is an armature in pose mode, a Pose Library panel appears in the asset browser → there is a case where the target changes the browser screen. Conclusion 1 is lowered to 「표본 대부분은 따로 둔다」 (most of the sample keeps them separate) (reviewer's report · main unconfirmed).
2. **「라이브러리 칸 안에 대상을 띄우면 관행과 어긋난다」 (showing the target inside the library pane is at odds with practice) is an exaggeration (high)** — the reviewer also could not find an exact peer of that shape. Not 「어긋난다」 (is at odds) but 「선례를 못 찾았다」 (could not find a precedent).
3. **A missing counter-example to conclusion 2 (high)** — the n8n AI Agent connects tool nodes as a graph, and the Zed agent profile picks tools in a modal → a fixed inspector is one of several shapes (reviewer's report · main unconfirmed).
4. **Conclusion 3 「넷을 보통 함께 준다」 (the four are usually offered together) is an unsupported synthesis (medium)** — only that each product offers different ways in is confirmed.
5. **Gap in the analogy for conclusion 4 (medium)** — Open WebUI · Zed are saved model/profile settings, not edits of a running agent instance.

Reviewer spot grounding: Ableton · Open WebUI · Unity Object Picker · Unreal 「Use Selected Asset」 supported.

## Gaps

- Blender · Photoshop libraries · Dify · Flowise · JetBrains · LibreChat · how Zed profiles are edited · the Continue hub block-adding flow.
