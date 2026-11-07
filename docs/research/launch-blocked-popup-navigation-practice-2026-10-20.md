# What appears when a launch is blocked · where clicking an item inside it leads (2026-10-20)

| Item | Value |
|---|---|
| **Status** | Research complete · **decided 2026-10-20** — when a launch issued by a person is blocked, show that agent's preset edit pane in a separate floating window next to the tree row · it does not close on an outside click, only with × · Esc (the persistent-panel convention — Unreal Compiler Results) · fixing happens inside it · the library opens only on double-click · via the magnifier (user: 「ㅇㅋㅇㅋ … 최종 적용」 — PRD E4) |
| **Method** | `/research` light — collector 1 (4 searches · no direct reading of source texts) + main grounding (checked the Unreal docs source text · checked the VS Code issue source text → not supported) |
| **Question** | When a run · play · compile is blocked by a configuration problem: ① what appears (a popover next to the target · a notification · a modal · a panel) ② does clicking an item go to its origin · does it open the target's editor if that is closed ③ what happens to that display after navigating |

## Conclusion

1. **No mature tool was found that shows the problem in a popover attached next to the target that was launched** (uncertain — absence was not exhaustively swept). The mainstream = a **persistent panel** that opens by itself (Unreal Compiler Results — certain) or a modal · a transient notification (VS Code · Visual Studio · JetBrains — from memory · snippets, unverified).
2. **Clicking an item goes to the problem location** (certain — Unreal source text 「Click on a message to jump the graph view directly to the problematic node」). Only Unity was found to open the target editor when it is closed (double-clicking in the console → brings up the external script editor · snippet), and no official doc stating it was found.
3. **The display persists** (likely): the Unreal panel · the Unity console stay after navigating. That a modal closes and a balloon disappears is an estimate from memory.

## Cases

| Case | Display at launch | On click | After navigating | Source | Confidence |
|---|---|---|---|---|---|
| Unreal Blueprint | The Compiler Results panel **opens by itself** on errors · warnings (usually below the Graph panel) | Message · link → to that node | Panel stays | https://dev.epicgames.com/documentation/en-us/unreal-engine/compiler-results-in-the-blueprints-visual-scripting-editor-for-unreal-engine | Certain (main checked the source text) |
| Unity | On Play, the console shows 「All compiler errors have to be fixed before you can enter playmode!」 | Double-click in the console → the external script editor opens at that line | Console stays | https://support.unity.com/hc/en-us/articles/205930539 · https://discussions.unity.com/t/all-compilers-errors-have-to-be-fixed-before-you-can-enter-playmode/503684 | Likely (snippet) |
| VS Code | preLaunchTask fails → modal (Debug Anyway · Show Errors · Abort) | Show Errors → Problems panel · item → file tab | — | https://github.com/microsoft/vscode/issues/54397, which the collector cited, does not cover that dialog (main checked) | Uncertain (from memory) |
| JetBrains | 「Error running '<config>'」 balloon + a link to edit the settings | Link → Run/Debug Configurations window | Balloon disappears · stays in the notification log | None | Uncertain (from memory) |
| Visual Studio | 「There were build errors…」 modal + Error List | — | — | None | Uncertain (from memory) |

## Gaps

- The popover-next-to-the-target form (GitHub Actions · Docker Desktop, etc.) was not looked at. Xcode was not looked at either.
- No official doc states 「편집기가 닫혀 있을 때 누르면 새 창을 연다」.
