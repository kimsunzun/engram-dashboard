# How built-in (read-only) assets are shown in the browser (2026-10-19)

| Item | Value |
|---|---|
| **Status** | survey done · **decided 2026-10-20** — a separate read-only root 「Engram 기본 제공」 (Engram built-in) below the library tree · default = hidden (the Unreal 「Show Engine Content」 shape) · on/off = the tree header 「…」 menu (changing the folder also goes through that menu) ☐ 「기본 제공 보기」 (show built-in) · remembered per number · the real files = one `builtin\` folder in the distribution (the user's 「A 로하자」 (let's go with A) — PRD R11 · R43) |
| **Method** | `/research` light — 1 collector (search + read official docs) + main grounding (checked the Epic docs original) |
| **Questions** | When read-only assets that the app ships are shown alongside user content: ① is the default hidden? ② a separate root? · above/below? ③ handling read-only ④ even when there are only a few, are they kept separate in the tree? |

## Conclusions

1. **Keep them as a separate root · pane — do not mix them into the user's folders** (likely): Unreal = the `Engine Content` folder in the Sources panel · Unity = the Packages root · Blender = 「Essentials」 in the library dropdown · JetBrains = the bundled-template group in the settings list.
2. **Visibility splits**: Unreal shows it only after you turn on the view option 「Show Engine Content」 (certain — Epic docs 「If enabled, shows the `Engine Content` folder in the Sources panel」 · the default is not in the docs → default off is an inference, likely). Unity · JetBrains always show it. Blender only when it is chosen in the dropdown.
3. **Read-only**: Blender's bundled files = read-only (likely) · Unity registry packages = cannot be edited (likely) · JetBrains = 「Duplicate」 makes a copy you can edit (uncertain).
4. **No peer was found that keeps a few built-ins as a tree pane** (uncertain) — every peer found has large built-ins. That small built-ins are more naturally shown at the place where you choose (picker · template list) is an inference.
5. The above/below position was not confirmed (unknown).

## Peers

| Peer | Shown by default | Place | Read-only | Source | Confidence |
|---|---|---|---|---|---|
| Unreal Content Browser | shown only if the options 「Show Engine Content」 · 「Show Plugin Content」 are on (default not stated) | the `Engine Content` folder in the Sources panel | not in the docs | https://dev.epicgames.com/documentation/en-us/unreal-engine/content-browser-settings-in-unreal-engine · https://dev.epicgames.com/documentation/en-us/unreal-engine/sources-panel-reference-in-unreal-engine | certain (the option exists — main checked the original) · default off likely |
| Blender Asset Browser | the dropdown 「Essentials」 · cannot be turned off | not a tree but a library choice | bundled files read-only · used by link | https://docs.blender.org/manual/en/4.4/editors/asset_browser.html | likely (snippet) |
| Unity Project window | always shown | the Packages root | registry · Git packages cannot be edited | https://docs.unity3d.com/6000.0/Documentation/Manual/upm-inspect.html | likely |
| JetBrains Live Templates | always shown | a group in the settings list | Duplicate = a copy to edit · restoring a deleted default | https://www.jetbrains.com/help/idea/settings-live-templates.html | uncertain (snippet) |

## For us

Options: A a separate pane in the left pane · hidden by default · turned on with a view option (the Unreal shape) / B not in the tree, only at the place where you choose (the agent slot 「기본」 (default) pane · the top of the `+` search) (the current design · the one that fits small built-ins) / C a separate pane · always shown (collapsed). The user decides.

## Limits

- Light — no adversarial review. Godot · VS Code were not looked at. The Unreal default and the tree position are not in the docs, so they have to be seen in the engine directly.
