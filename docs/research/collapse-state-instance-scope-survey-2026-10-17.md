# ★ Collapse/expand memory — per browser (window), and what is remembered — light survey

| Item | Value |
|---|---|
| Status | Survey complete · awaiting decision (user) |
| Date | 2026-10-17 |
| Method | `/research` light — 1 collector (about 8 searches and fetches) · 1 main grounding spot check (Unreal `EExpansionPersistenceMethod` API doc, WebFetch) · no adversarial review |
| Question | In libraries 1~4 (like Unreal Content Browsers 1~4), is the collapse/expand state remembered separately per number, or is there one shared state · of the things that collapse, which are remembered and which are not |
| Previous survey | `docs/research/collapse-state-persistence-scope-survey-2026-10-17.md` (per view · per item) |

## Conclusion

- **The Unreal Details default is 「종류마다 전역 공유」 — not per window.** Original text: Default = 「Expansion will use default persistence (Global shared per struct type)」 · per window (view id) exists only as an option — TransientPerViewIdentifier (for the editor session) · PersistentPerViewIdentifier (on disk). (Close to certain — main verified the original text · single source) → This matches the user's guess (「보통은 브라우저 별로 저장할 것 같지는 않다」).
- **We could not confirm what Content Browsers 1~4 store separately per window** — the instance name and the 「저장된 설정」 concept exist (likely), and `UContentBrowserSettings` is global, 「global not per-project」 (certain — API doc). The ini section name and the fields stored per window are unconfirmed (doc returned 403 · source not public).
- **Remembered vs. not — the only confirmed case is the detail · inspector group (remembered).** Tree expansion (VS Code = restored on reopen — weak source) · column header · row expansion · menu/popover: no source found (abstained).
- **Collector rule of thumb (inference without a source):** long-lived structures (tree nodes · columns · groups) are remembered by (view id + fixed element id) · row expansion tied to content, menus and popovers are not remembered.

## Findings

| What | Per window? | Storage | Source | Confidence |
|---|---|---|---|---|
| Content Browser instance — 「global name of a content browser instance」 · 「saved settings」 can be used instead of defaults at creation | Yes (named instance · saved settings) | Unconfirmed | https://dev.epicgames.com/documentation/unreal-engine/API/Editor/ContentBrowser/IContentBrowserSingleton/CreateContentBrowser | Likely |
| `UContentBrowserSettings` (show engine folder · dock collections …) | No — global | Editor settings | https://dev.epicgames.com/documentation/unreal-engine/API/Editor/UnrealEd/UContentBrowserSettings | Certain |
| Details group expansion | Default = global shared per type · option = per view id (session · disk) | Session / disk | https://dev.epicgames.com/documentation/unreal-engine/API/Editor/PropertyEditor/FDetailsViewArgs/EExpansionPersistenceMethod (main verified) | Close to certain |
| Folder tree expansion (VS Code) | Presumed per workspace | Unconfirmed | — | Uncertain |
| Column header · row expansion · menu/popover | — | — | — | Abstained |

## What this gives our design

- Collapse/expand memory = **one shared state for 1~4 · keyed by the element's fixed id** (the Unreal Details default shape). Not per window.
- Only long-lived structures go into the single shared mechanism (element id → expansion) — the left column headers (library · favorites) · the detail card 「참조」 groups (preset · agent) · the edit column's kind column header. The preset row expansion in the edit column · menus · the 👁 card are not included. Folder tree expansion has weak grounds, so it awaits a decision.

## Gaps

- Sources for: the Unreal ini section · the fields stored per window · the VS Code explorer · column collapse storage (readable from the source `explorerView.ts`) · Obsidian row expansion · menus not being remembered.
