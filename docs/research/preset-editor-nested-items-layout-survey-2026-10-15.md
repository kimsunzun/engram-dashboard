# Preset edit pane — how to show what was added directly versus what comes from presets nested inside (peer survey)

- **Status:** version reflecting the adversarial review (FIX) · awaiting decision (an early idea before the preset PRD — user rule 「레이아웃은 관례 기준으로」 (layouts follow convention). The 「담은 것 | 최종 구성」 (added | final composition) tab the session proposed was a session idea with no peer check)
- **Date:** 2026-10-15
- **Method:** research medium · design-decision mode. 2 collection tracks (AI agents · developer tools · content tools with inheritance/nesting) · main-session grounding · 1 cross-family adversarial review (below).
- **Confidence legend:** certain = original text + independent cross-confirmation · likely = original text (single) or partial support · uncertain = memory · summary · original text not checked.

## The questions

A preset holds instructions · skills · settings · other presets (nesting). In the edit pane:

1. How do we tell apart, and show, what was added directly and what came in through a nested preset — a separate 「최종(적용 결과)」 (final, applied result) view? A source marker within one list? An expandable tree? An on/off button?
2. Do we split into per-kind panes, or mix everything in one list?
3. Does the user reorder — only some kinds?
4. Is adding done per pane (kind), or with a single button?

## Conclusions (top level)

1. **Split by kind — the practice that looks most uniform in this sample.** Continue `config.yaml` has top-level sections per kind: `models` · `rules` · `prompts` · `context` · `mcpServers`, etc. · VS Code profiles are chosen and opened per unit of settings · keybindings · MCP servers · snippets · tasks · extensions · Zed agent profiles have default tools · MCP tools · default model · the Claude Code plugin detail has commands · agents · skills · hooks · MCP/LSP servers under 「Will install」 · Chrome DevTools Computed has 「Group」 (a collapsible grouping, optional). I found no mixed single list in the sample. **Likely** (Continue · Claude · Chrome original text · VS Code partial original text).
2. **Direct vs inherited — in content editors the mainstream is 「한 목록에 함께 + 줄마다 표시」 (together in one list + a marker on each line).**
   - **Unity prefab instances** — inherited and changed items are together in one inspector, and the changed ones get a **blue line on the left + a bold name**, and added or removed components get a **+/− badge** on the icon. Apply/Revert from the 「Overrides」 menu (all · selected · one by one). **Likely** (original text).
   - **Unreal Blueprints** — turning on 「Show Inherited Variables」 shows the parent's variables together with the class's own (an on/off button). **Uncertain** (forum snippet).
   - **Chrome DevTools Styles** — shows all applicable rules (including overridden ones), and groups the inherited ones separately in 「Inherited from <요소>」 (Inherited from <element>) sections. **Likely** (original text).
   - Godot inherited scenes · Figma instances (override · Reset) — **uncertain** (snippets · memory).
3. **There is one precedent for splitting 「내가 정한 것」 (what I set) and 「실제 적용 결과」 (the actual applied result) into two screens — the Styles vs Computed tabs of Chrome DevTools.** Computed = 「실제로 적용되는 CSS 만」 (only the CSS that is actually applied). **Likely** (original text). On the AI-tool side the applied result is seen **only through commands** — `docker compose config` (prints the merged result · uncertain — search summary) · `claude plugin list` (dependency status). I did not find an AI tool whose GUI shows, line by line, 「무엇 때문에 들어왔나」 (what caused this to come in).
4. **There is no precedent in this sample for the user reordering by dragging.** The Claude plugin list is sorting · grouping (disabled ones under a collapsed header at the bottom), not user ordering · Continue does not write rule order · priority in its docs. **Gap.**
5. **Adding is mostly per kind · per target.** Continue has a `uses:` line per section · Zed has a toggle per kind · Claude Code has plugin discovery (the Discover tab) and `/plugin install`. I did not find a GUI with a `+` button on each section. **Uncertain.**

## Carrying this over to Engram (candidates — not decisions)

- **Per-kind panes** — matches practice (conclusion 1). Already a user decision (2026-10-15).
- **Items that come from a nested preset** — practice has two branches.
  - (A) **One list + a marker on each line + on/off** (Unity · Unreal · DevTools Styles): inside each kind pane the inherited lines are shown together, dimmed · marked 「← 공용 규칙」 (← shared rule), and toggled on and off with 「안에 든 것 보기」 (view nested items) above the pane. An inherited line cannot be removed with ×  (to remove it, open that preset).
  - (B) **Two screens** (DevTools Styles vs Computed): the 「담은 것 | 최종 구성」 tab the session drew.
  - Content editors (the side closer to our library slots) mostly use (A).
- **Order** — there is no precedent. For instructions · settings the order of concatenation changes the result, so a way to handle order is needed — note that drag-to-reorder comes from our own need, not from practice.
- **A `+` per pane** — no precedent verified. The user decided it is convenient.

## Grounding (main-session external check)

| Claim | Source | Verdict |
|---|---|---|
| Unity blue line · +/− badge · Overrides Apply/Revert | https://docs.unity3d.com/Manual/PrefabInstanceOverrides.html (original text quoted via a summarizer) | supported |
| DevTools Styles = everything including overridden · 「Inherited from」 · Computed = actually applied · Group checkbox | https://developer.chrome.com/docs/devtools/css/reference (original text quoted via a summarizer) | supported |
| Claude Code bundle = a plugin that has only `dependencies` · detail 「Will install」 per kind · disabled plugins collapsed at the bottom | https://code.claude.com/docs/en/plugin-dependencies (main session directly from the original — the bundle part) · install docs (collector) | partially supported (bundle = checked by the main session · UI wording = collector) |
| Continue `config.yaml` per-kind top-level sections · `uses:` | https://docs.continue.dev/reference (collector) | not verified (not cross-checked by the main session) |
| VS Code profiles per kind · Zed profile kinds | collector (partial original text) | not verified |
| Unreal Show Inherited Variables · Godot · Figma · docker compose config | collector (snippets · memory) | uncertain |

## Adversarial review

codex (cross-family) · effort high · level 2 · web search · decision **FIX**. Addressed (this section takes priority over the conclusions above):

1. **Conclusion 2 is an exaggeration (high).** Unity marks 「내가 바꾼 것」 (what I changed) and Chrome is a bundle of CSS inherited from DOM ancestors, so neither is a precedent for a **per-line source marker for items that a nested bundle put in**. Both are **inheritance** from a single parent — our preset **includes** several presets (the user pointed out the same thing: 「프리셋을 펼쳐 보는 게 아니라 각 칸에 묻히는 거냐」 (so it is not that we expand the preset to view it, but that it is buried in each pane?)). → (A) is not used as 「관례상 주류」 (mainstream by convention). The closer peer is VS Code profiles (inheriting wholesale from the default profile, per kind).
2. **Precedent for two screens added (medium)** — Argo CD ApplicationSet UI: an editor + a preview of the child apps that will be created (reviewer's report · not checked by the main session).
3. **Missing mixed single list · nesting GUI (medium)** — Ableton Racks: a Rack inside a Rack (nesting — main-session original-text check: 「you will create a Rack within a Rack」), and a chain holds several kinds of devices mixed together · a Rack is folded and unfolded in place (「Fold」 — main-session original-text check). It is a **precedent for unfolding a containment relation in place**.
4. **Conclusion 4 「순서 바꾸기 선례 없음」 (no precedent for reordering) is refuted (high)** — Ableton reorders the Rack chain list · devices, and that order changes the result (reviewer's report). The main-session original-text check confirmed only 「Racks, chains and devices can be freely dragged into and out of other Racks」 — the sentence on chain order was not confirmed → **partially supported**.
5. **Conclusion 5 「칸마다 `+` 선례 미확인」 (no verified precedent for a plus button per pane) is refuted (medium)** — **Unreal My Blueprint panel: a create button on each pane title + 「Add New」 at the top** (main-session original-text check: 「shortcut buttons (… at the end of each heading) for creating new variables, functions, macros…」 · 「the Add New button at the top of the My Blueprint Panel」). It is the same shape as our decision (a `+` per pane + 「+ 담기」 (+ add) at the top).
6. The source of the Claude 「Will install」 wording is the install docs (misattribution — low).

## Gaps · limits

- No precedent for reordering · a `+` per pane.
- Places not looked at: Cursor rules screen · Goose recipe editor · Dify · Flowise · LangSmith · Copilot user agents · JetBrains · Kustomize · Blender library overrides · Ableton Racks.
- GUI screen wording · screenshots were almost never obtained (docs rarely describe the screen).
