# Problem display when several items fail to launch · reopening after closing (2026-10-21)

| Item | Value |
|---|---|
| **Status** | survey done · **handed to the next topic (agent system) 2026-10-21** (user — what to do when there are several block windows · when closing and reopening · and relaunch will be decided then) |
| **Method** | `/research` light — collector 1 (5 searches · 1 source text) + main grounding (the JetBrains source text was read by the collector · the rest are snippets) |
| **Question** | When a launch is blocked on several items, ① is it one window per item, or are they grouped into one shared panel ② what is used to reopen after closing ③ does it update immediately while fixing · is there a relaunch in that place |

## Conclusion

1. **The practice is to group them in one shared panel, not one window per item** (likely) — VS Code Problems (per file) · JetBrains Problems (tabs · groups) · Unreal Message Log (per category) · GitHub Actions run page (per job). No case that opens a window per failure was found.
2. **The way to reopen = an always-present entry point** (likely) — menu · shortcut · error count in the status bar · icon beside the item. The entry point stays even when there are no problems.
3. **Immediate update · relaunch is in or beside that panel** (JetBrains update = certain · source text 「IDE continuously checks … updated as you switch files」 / GitHub Actions 「Re-run failed jobs」 = certain · official documentation snippet / JetBrains Run window rerun = likely).

## Cases

| Case | Several failures | Reopen | Update · relaunch | Source | Confidence |
|---|---|---|---|---|---|
| JetBrains Problems | tabs (Current File · Project · Inspection Results) · groups | tool window bar · View menu (estimate) | continuous update · quick fix | https://jetbrains.com/help/clion/problems-tool-window.html | certain (source text) |
| JetBrains Run window | a tab per run configuration | — | toolbar rerun · rerun failed | https://www.jetbrains.com/help/idea/2018.2/rerunning-tests.html | likely |
| VS Code Problems | one panel · per file | View > Problems · Ctrl+Shift+M · error count in the status bar | no concept of rerun | secondary article snippet | likely |
| Unreal Message Log | one window · per-category log | Window > Developer Tools > Message Log | unchecked | https://unrealcommunity.wiki/message-log-4wzqj97j (community) | likely |
| GitHub Actions | one run page · per job | — | Re-run failed jobs · all · a specific job | https://docs.github.com/en/actions/how-tos/manage-workflow-runs/re-run-workflows-and-jobs | certain (official snippet) |

## Gaps

- Visual Studio Error List · Xcode · Unity Console · Docker Desktop · Task Scheduler were not examined.
- The 「항목 배지를 눌러 그 항목의 문제를 다시 연다」 shape could not be confirmed (only that an entry point exists).
