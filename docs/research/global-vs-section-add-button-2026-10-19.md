# Keep both a top-level 「전체 추가」 and a per-section `+`? (2026-10-19)

| Item | Value |
|---|---|
| **Status** | Research complete · keep the current design as is (PRD R23 — the 「+ 추가」 at the top of the edit pane = search across everything · the `+` on a section title = that kind only) |
| **Method** | `/research` light — collector 1 (search + read the official docs) + main grounding (checked the Epic docs source text) |
| **Question** | In an edit screen split into fixed-kind sections (instructions · skills · settings · presets), is there precedent for having both a top-level add-all and a per-section `+`? |

## Conclusion

- **There is precedent for having both — the Unreal My Blueprint panel** (certain — main checked the Epic docs): an 「Add New」 button at the top of the panel + a `+` shortcut button at the end of each section title (Graphs · Functions · Macros · Variables · Event Dispatchers). The sections are fixed kinds, as in ours. The difference = Unreal's 「Add New」 is a menu for creating something new (the docs say only that it has the same items as the right-click menu — they do not say whether one menu lists every kind), while ours searches existing items and attaches them.
- **Places with only one** — only an add-all: Unity Inspector 「Add Component」 (likely · snippet) · JetBrains Run/Debug Configurations toolbar `+` (certain) — both are open-ended lists of kinds. Only a per-section `+`: Figma properties sections Fill · Stroke · Effects (likely · snippet · not checked whether an add-all is absent).
- Weak pattern (interpretation): fixed-kind sections get a per-section `+`, an open single list gets one add-all. No design-system guidance was looked up.

## Peers

| Peer | Add-all | Per-section `+` | Sections | Source | Confidence |
|---|---|---|---|---|---|
| Unreal My Blueprint | Yes · 「Add New」 at the top | Yes · at the end of each section title | Fixed | https://dev.epicgames.com/documentation/unreal-engine/my-blueprint-panel-in-the-blueprints-visual-scripting-editor-for-unreal-engine | Certain (main checked the source text) |
| Unity Inspector | Yes · 「Add Component」 | None | Open | https://docs.unity.com/en-us/unity-studio/develop/ui-refs/inspector-ref | Likely (snippet) |
| JetBrains Run/Debug Configurations | Yes · toolbar `+` / Alt+Insert → template list | None | Open (grouped by folder) | https://www.jetbrains.com/help/idea/run-debug-configuration.html | Certain (collector read it) |
| Figma properties sections | Not checked | Yes · Fill · Stroke · Effects | Fixed kinds | https://help.figma.com/hc/en-us/articles/360049283914 | Likely (snippet) |

## Limits

- light — no adversarial review. Notion · VS Code · Postman · Blender were not looked at.
