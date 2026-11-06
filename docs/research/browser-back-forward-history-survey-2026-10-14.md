# Back / forward (navigation history) in the library slot — practice summary

- **Status:** light · awaiting decision (a pre-preset-PRD idea — returning after following a 「쓰는 곳」 (used-in) link in the library slot)
- **Date:** 2026-10-14
- **Method:** research light. 1 collector · main grounding (original checked for the 1 key claim) · no adversarial review (light).

## Conclusions

1. **Back/forward in file · asset browsers is web-browser style, recording only the 「위치(폴더)」 (location (folder)).** Unreal Content Browser original: 「웹 브라우저의 Back · Forward 처럼 동작한다 — 최근 파일 경로 사이를 오간다」 (it behaves like the Back · Forward of a web browser — it moves between recent file paths). Windows Explorer · macOS Finder also record only the folder location (from memory — unconfirmed). **Likely.**
2. **Edit undo/redo and navigation history are separate commands.** In editors too (VS Code Go Back/Forward = 「파일과 편집 위치 사이 이동」 (move between files and edit locations) · JetBrains = caret positions), navigation history is separate from undo (the separate description is from memory — only the key bindings are from the original). **Likely.**
3. **The scope of the history is commonly per window · panel** (the browser tab model). Unreal gives each Content Browser a history manager (`FHistoryManager`) and has an API that exposes the history list as a menu (API docs — whether per instance is unconfirmed). **Uncertain.**
4. **Controls:** ← → buttons · Alt+←/→ (VS Code Windows · Explorer) · mouse side buttons · long-pressing or right-clicking a button gives the history list. **Likely** (some from memory).

## If moved to Engram (interpretation — the decision is the user's)

- A new history entry = when the place changes (folder move · a 「쓰는 곳」 (used-in) link · a grouped view). A click inside the same folder does not add an entry and only rewrites that entry's 「마지막 선택」 (last selection) → back = the previous place + the last selection at that time. 「마지막 선택 복원」 (restoring the last selection) goes one step further than the browsers above (location only).
- Edits (adding · removing) do not go into the history — conclusion 2.
- Separate per slot · ← → buttons + Alt+←/→ + mouse side buttons. A history list dropdown comes later.

## grounding

| Claim | Source | Decision |
|---|---|---|
| Unreal Content Browser History Back/Forward = moves between recent file paths like a web browser | https://dev.epicgames.com/documentation/unreal-engine/content-browser-interface-in-unreal-engine | supported (main checked the original) |
| Unreal `FHistoryManager` · the history menu item API | https://dev.epicgames.com/documentation/unreal-engine/API/Editor/ContentBrowser/FHistoryManager | collector checked · main did not re-check |
| VS Code Go Back/Forward keys (Windows Alt+←/→) | https://code.visualstudio.com/docs/editor/editingevolved | collector checked · main did not re-check |
| JetBrains back/forward = caret position · Ctrl+Alt+←/→ | https://www.jetbrains.com/help/idea/navigating-through-the-source-code.html | collector checked · main did not re-check |

## Limits

- Light, so no adversarial review. Explorer · Finder are memory-based. Blender not surveyed.
