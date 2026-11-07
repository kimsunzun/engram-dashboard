# Guidance for relinking missing references · overlap with a floating error popup (practice survey)

- **Status:** light survey · 2026-10-18 · 1 collector (document-type) + main-session grounding (checked against the Microsoft InfoBar original text) · no adversarial review
- **Questions:** ① how does a tool guide the user to relink file references that went missing because they were moved from outside (list · how to search · does fixing one fix the rest · is name matching automatic) ② when another window is opened from a floating error popup, what happens to the popup · the way back to the remaining items.

## Conclusions

- **① The missing items are shown in one place, and when the user points at one file (or a folder to search), the tool fixes the rest by name** — Premiere Pro (「미디어 연결」 (link media) · when one is found, the rest in the same or a similar folder are relinked automatically) · After Effects (replace one, and it searches the same folder: 「X additional missing items have been found」) · DaVinci Resolve (give a folder, and it searches down into subfolders by file name) · Blender (「Find Missing Files」 — searches the folder down into subfolders and restores everything it finds). I found no peer that asks for per-item confirmation in the automatic linking of the rest. — likely (secondary sources · the Adobe official page returned 403)
- **I found no peer that links automatically by name alone, without the user pointing first** — all the peers above match by name after the user starts 「찾기」 (find). Unreal keeps references by leaving a redirector when moving inside the editor (a relink screen for outside moves was not found), and Unity shows only 「Missing (Mono Script)」 and has no relink screen. — likely
- **② For an error that changed the app's state, Microsoft recommends an inline notification (it stays until closed and does not cover other content)** — 「By default the notification will remain in the content area until closed by the user but will not necessarily break user flow.」 · 「It will not cover up other content or float on top of it.」 Information in the context of a specific UI element goes in a Flyout (no persistence needed) · what must be read, or must block the whole app, goes in a ContentDialog (checked against the original text). — certain
- **A brief notification plus a retained record is the common shape** — JetBrains keeps both a balloon (disappears soon · a sticky balloon = until closed) and a record in the notifications tool window. — likely (search summary)
- **I found no source that describes what happens to the popup after another window is opened from it by a link.**

## Sources

| Peer | Source | Confidence |
|---|---|---|
| Premiere Pro link media | https://www.adobe.com/learn/premiere-pro/web/relink-offline-media · https://larryjordan.com/articles/premiere-pro-cc-relink-missing-media/ | likely |
| After Effects replace footage | https://aejuice.com/blog/how-to-relink-footage-in-after-effects | likely |
| DaVinci Resolve relink | Resolve 18 manual mirror · https://jayaretv.com/workflow/relink-offline-media-in-davinci-resolve/ | likely |
| Blender Find Missing Files | docs.blender.org 2.79 manual (summary) | likely |
| Unreal redirectors | https://dev.epicgames.com/documentation/en-us/unreal-engine/asset-redirectors-in-unreal-engine | likely |
| Microsoft InfoBar | https://learn.microsoft.com/en-us/windows/apps/design/controls/infobar | certain (checked against original text) |
| JetBrains notifications | https://www.jetbrains.com/help/idea/notifications.html | likely |

## Gaps

- Reading the Adobe official help directly failed (403). Not checked: whether the old name is pre-filled in the search window · whether Resolve confirms per item · whether VS Code notifications persist.
