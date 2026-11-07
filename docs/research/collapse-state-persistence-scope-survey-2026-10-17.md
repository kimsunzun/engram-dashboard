# ★ Is the collapse/expand state remembered — per view · per item — light survey

| Item | Value |
|---|---|
| Status | Survey complete · awaiting decision (user) |
| Date | 2026-10-17 |
| Method | `/research` light — 1 collector (about 10 searches and fetches · read 1 VS Code source file) · 1 main grounding spot check (GOV.UK accordion, WebFetch) · no adversarial review |
| Question | Is the expansion state of the groups inside the detail card 「참조」 (「▾ 프리셋 3」 · 「▸ 에이전트 17」) remembered — not at all · one per view (shared across items) · per item — and where is it stored |

## Conclusion

- **It is remembered per view (column) or per kind — per item almost never.** Changing the visible item leaves the group as the user set it. VS Code = per column id (not per file) · Unreal Details = per class · Unity UI Toolkit = per element key. (Likely)
- **Remembering is the default, and the places that do not remember get complaints** — the Obsidian backlinks 「결과 접기」 (collapse results) resets in a new tab or new file, so there are forum requests and a plugin that fixes it. (Likely)
- **Storage scope:** IDE columns = long-lived per window · workspace · web accordions = for the session — GOV.UK: 「the accordion component will use session storage to remember which sections were open」 (on by default · turned off with `rememberExpanded: false`). (GOV.UK = close to certain — main verified the original text)
- **A group needs a fixed id to be remembered** — the key is the group kind (「프리셋」 · 「에이전트」), not the slot order.

## Findings

| Tool | What collapses | Remembered | Scope | Storage | Source | Confidence |
|---|---|---|---|---|---|---|
| VS Code | Columns inside a container (Explorer · Outline · Timeline) | Yes | Per column id (not per file) | Workspace | https://raw.githubusercontent.com/microsoft/vscode/main/src/vs/workbench/browser/parts/views/viewPaneContainer.ts | Likely (the file that stores the collapse flag was not read) |
| Obsidian | Backlinks 「Collapse results」 | No — resets in a new tab · file · the 「Backlink Settings」 plugin fixes it | Column instance | None | https://forum.obsidian.md/t/maintain-show-backlinks-in-document-settings-like-collapse-results-across-tabs/51506 · https://community.obsidian.md/plugins/backlink-settings | Likely |
| JetBrains | Project tree | Only while the tool window is open (forum) | Tool window | Unconfirmed | https://intellij-support.jetbrains.com/hc/en-us/community/posts/207073755 | Uncertain |
| Unity | UI Toolkit Foldout | Yes, if given a `viewDataKey` | Per element key | Unconfirmed | https://docs.unity3d.com/2023.2/Documentation/Manual/UIE-ViewData.html | Likely |
| Unity | IMGUI Foldout | No — store it yourself | — | — | https://discussions.unity.com/t/editorguilayout-foldout-no-way-to-remember-state/36422 | Likely |
| Unreal | Details panel category | Yes — across sessions too (RestoreExpansionState) | Per class | Unconfirmed | https://dev.epicgames.com/documentation/unreal-engine/API/Editor/PropertyEditor/IDetailCategoryBuilder · https://issues.unrealengine.com/issue/UE-96324 | Likely |
| macOS Finder | Get Info sections | Yes | Unconfirmed | Unconfirmed | https://macmost.com/the-three-different-types-of-get-info-window-in-the-mac-finder.html | Uncertain |
| GOV.UK | Accordion | Yes (default) | Per accordion id · page | Session storage | https://design-system.service.gov.uk/components/accordion/ (main verified) | Close to certain |
| DNB Eufemia | Accordion | Optional (rememberState · needs a unique id) | Per id | Not stated | https://app.unpkg.com/@dnb/eufemia@11.8.3/files/components/accordion/AccordionDocs.js | Likely |

## What this gives our design

- 「참조」 group expansion = remembered **per library number (1~4) · keyed by the group kind (preset · agent)** — unchanged when the item changes. It is shell screen state (like column width · sort · last folder), so it survives a restart (the IDE column shape).

## Gaps

- Unconfirmed: the VS Code settings editor groups · the references view · the exact storage key of the collapse flag · JetBrains Find Usages · where Unity and Unreal store it · whether Finder is per item · the Explorer details column.
