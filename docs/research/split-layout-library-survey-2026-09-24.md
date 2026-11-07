# Split layout renderer — keeping allotment vs replacing it: peer survey

- Status: confirmed survey · **decision = for now, block only the squashing with A (ratio `defaultSizes`) and merge · the structural replacement that removes pane remounts is a separate topic, starting from design · that structural replacement = D (flat leaves + percent placement, implemented ourselves) — user decision 2026-09-24** · date: 2026-09-24 · depth: medium · design-decision mode
- Method: 3 main-family (Claude) collector tracks (react-resizable-panels deep dive / allotment defect history / peer apps and candidate libraries) + main grounding (react-resizable-panels 4.13.3 published source · allotment 1.20.5 installed copy · React 19 `react-dom-client.development.js` · react-mosaic `MosaicRoot.tsx`·`BoundingBox.ts`·`package.json` originals · local clones of orca·paseo·vibe-kanban code · issue status and repository figures from the GitHub API and npm) + one cross-family (codex, effort high, web search) adversarial review — verdict REFUTE-PARTIAL, all 11 findings addressed (2 overstatements · 1 missing derivation · 1 causal claim without evidence · 1 missing source context · 4 omissions · 1 date error · 1 integration gap)
- Confidence legend: certain = 2 or more independent sources, or direct code check / likely = supported by a single source / uncertain = unsupported or inferred

## Questions

1. When a pane is closed and splits merge, a squashed screen shows briefly (user measurement 2026-09-24). What is the cause and the root fix?
2. If we switch the split library to react-resizable-panels, is this class of problem blocked at the source? What new problems appear instead?
3. How did peers solve the same problem (an externally owned split tree · a terminal inside a pane)?

## Our current state (code checked)

- The layout tree (split `id`·`dir`·`ratio`·two children) is owned by the shell backend, and the frontend only draws the snapshot (CLAUDE.md 「LLM-우선 제어」 (LLM-first control) · ADR-0035).
- The renderer draws the tree as a **recursive component** — one `Allotment` per split, with two child renderers inside it (`src/components/layout/ViewLayoutRenderer.tsx:300-310`). This is the only file that uses allotment, and its CSS is imported by `src/main.tsx:8`.
- Split instance key = `${id}:${dir}` (ADR-0223). At the time of the survey, the initial ratio was given as a percent string in the first pane's `preferredSize`, and `defaultSizes` was not used (comment in the same file: an old measurement that giving a ratio collapsed the pane to ~1px). Option A of this survey put `defaultSizes` in (same day — step-log).
- Dragged sizes are not written back to the backend (the renderer comment puts this outside the scope of ADR-0063, but the body of ADR-0063 has no such clause — corrected in ADR-0227).
- **Tabs already have no remount** — one tab = one view (it has its own split tree and focus), and the screens of all tabs in a window are stacked with absolute positioning, with only the active tab shown through `display` (`src/components/layout/WindowLayout.tsx:182-197` · ADR-0056 — code checked). A hidden tab gives back only its WebGL seat, and its terminal instance stays alive. So the flat structure only needs to apply **inside a single view**. A popout, which detaches a pane into another window, gets a new pane id and a new webview, so no structure can avoid a remount there (`src-tauri/src/layout/apply.rs:561-600` — investigation subagent report).
- **Closing a pane remounts the whole surviving sibling subtree** — if the sibling is a split, that includes a new `Allotment`, slots, xterm, re-subscription and replay; even if the sibling is a slot, the tree depth changes and that slot is remounted (cause-investigation subagent report · the latter predates ADR-0223). The 「승격된 서브트리 안 죽은 에이전트 보존 화면 소실」 (loss of the preserved screen of a dead agent inside a promoted subtree) in the 「영향」 (Impact) of ADR-0223 is a consequence of this remount.

## Findings

### F1. Cause of the squashing — a newly mounted `Allotment` paints one frame before it decides the pane sizes — likely

- Without `defaultSizes`, allotment builds the SplitView with zero views at mount, and views are added only in the **React re-render after** the ResizeObserver callback changes state (`node_modules/allotment/dist/modern.mjs:1004-1060`, `:1132` — code checked). That re-render is on the default lane, so it runs after paint — in between, the pane container has no left/width and is bunched toward the main axis.
- **This is a known unresolved defect upstream** — issue #820 「Content not full-size on first paint」 (open, 2024-09). PR #526 (v1.18.0), which added a size initialization before the first paint, was reverted by PR #805 (v1.20.2) (status checked through the GitHub API — certain). #805 links to defect #583, where `preferredSize` has no effect — this suggests the revert may have been caused by that regression, but there is no full statement of the maintainer's reason.
- That this flicker became newly visible on close is because of the key in ADR-0223 — before that, the same instance was reused, so the size was wrong instead of flickering. When a split is newly created, it was probably there from the start (inference — uncertain).
- It is **not** the case that several updates arrive on one close and intermediate states get painted — the backend sends `layout_updated` once and the frontend store also updates once (`src-tauri/src/layout/apply.rs:325-342` · `src/store/viewStore.ts:235-251` — cause-investigation report, certain).
- Unverified: no painted frame was captured in a real WebView2.

### F2. Fixing it inside allotment (ratio `defaultSizes`) — by static analysis the gap closes (conditional) · the remount remains

- `defaultSizes` is **normalized as a ratio**, not as pixels — the sum becomes the total size, and `layout(e)` places each pane at `round(비율 × e)` (`modern.mjs:575-583, 655-666` — code checked). So the explanation in the old comment (「`defaultSizes` 는 픽셀이라 0.2px 로 먹는다」 (defaultSizes is pixels, so it takes effect as 0.2px)) is wrong.
- **The cause of the old 「~1px 붕괴」 (~1px collapse) observation is unknown — uncertain.** At the collection stage it was seen as 「당시 CSS import 누락과 겹친 오염」 (contamination overlapping with the CSS import that was missing at the time), but the review refuted that causal link: the `defaultSizes` path skips `relayout()` through `addView(…, true)` (`:575-581`, `:634`), and placement waits for `layoutViews()` (`:839-847`), so nothing in the code says that a 0.5px pane gets painted. The CSS is imported now. **Whether it reproduces is decided only by measurement.**
- Giving `defaultSizes` makes the views be created synchronously in the mount layout effect, so **one re-render is saved** (certain).
- **Is there a frame left over — the static derivations of both model families agree on 「없다」 (none) (conditional · likely):** zustand's `useSyncExternalStore` update is SyncLane (lane 2), and React 19 flushes the passive effects of a commit with lane `& 3` synchronously at the end of the commit (`react-dom-client.development.js:18323` — code checked). So the ResizeObserver that allotment sets in `useEffect` (`modern.mjs:230-249`) is also registered before the first paint, and under the ResizeObserver spec the first observation is delivered before paint, and its callback calls `layout` right away (`:1125-1133`). The cause investigation (Claude) and the adversarial review (GPT) independently reached the same conclusion. The opposing view (the allotment collector — 「RO 가 `useEffect` 라 한 프레임은 가능」 (since RO is in useEffect, one frame is possible)) did not look at this lane behavior.
  - **Conditions:** the container size is not 0 · the update arrives through the SyncLane path · the RO timing of WebView2 follows the spec. If any one of the three breaks (a hidden pane, another scheduling path), one frame may remain. **This is a static result, not screen evidence → confirm by measurement.**
- On this route, **the sibling subtree remount on close (the last item of the current state) stays as it is** — even if the squashing goes away, the remounted terminal looks empty until the replay arrives.
- allotment is in low-activity maintenance — latest 1.20.5 (2025-12-19), only dependency-update commits since, about 100 issues and PRs open, the requests to support direction change (#341·#457) have been open since 2022, and the upstream's recommended workaround is a `key={vertical}` remount (checked through the GitHub API — certain).

### F3. react-resizable-panels (v4) — blocks the first-paint and direction classes · the remount depends on the renderer's shape — likely

- **First paint:** panes are drawn from the first render by CSS (`flexBasis` = `defaultSize`, or `flexGrow:1` if absent), and the actual measuring and placement finish inside **useLayoutEffect**, before paint (published `dist/react-resizable-panels.js:3` (import `useLayoutEffect as et`) · `:1703` (`V = window ? et : Se`) · `:2011` (group registration is `V(() => …)`) · `:2338-2372` (pane style fallback) — code checked). However, if the group size is measured as 0 (a hidden subtree), it waits for the RO (`:1661` `defaultLayoutDeferred`). There is no screen measurement.
- **Direction:** `orientation` is a dependency of the group registration effect (`:1911` default · `:2073-2080` dependency array — code checked), so when it changes, only the group is registered again, without remounting the React children. Whether sizes are preserved was not run (likely).
- **Remount — if the renderer is a recursive one that mirrors the backend tree as it is, it cannot be prevented (likely):** when a sibling is promoted, the React ancestor path of the surviving subtree changes, and React does not preserve the state of an element whose ancestor path changed (the position rule of the React docs 「Preserving and Resetting State」). Fixing the type and key at the destination does not help, since the whole ancestor path must be the same.
  - **However, 「중첩 Group 으로는 불가능」 (impossible with nested Groups) is an overstatement (review finding addressed):** it can be preserved by leaving the old ancestor `Group`/`Panel` as pass-through nodes so that the ancestor path is kept. The cost = the React tree on screen drifts from the backend tree, and the renderer has to manage separately the cleanup of pass-through nodes, the size computation and the mapping to the backend (not implemented — likely). This variant applies to allotment too, by the same reasoning.
- **Synchronization with the backend source of truth is not free:** `defaultSize` is only an initial value, so when the backend ratio changes `groupRef.setLayout()` must be called explicitly, and the renderer has to build stable pane ids and a 「프로그램 변경 vs 사용자 드래그」 (programmatic change vs user drag) callback policy (`onLayoutChanged(layout, {isUserInteraction})`) (the API exists — collector report · review finding).
- Maturity: 4.13.3 (2026-09-23) · MIT · single maintainer (Brian Vaughn) · 5.4k★ · 6 open issues · about 26.8 million weekly downloads (allotment about 190 thousand) · peer `react ^18 || ^19` (npm·GitHub API — certain). v4 (2025-12) was a large breaking change (`PanelGroup`→`Group`, `direction`→`orientation`, etc.) and regressions continued for a while (performance #531·#572, conditional panes #565·#691·#717, stuck resize #729 — mostly closed · collector report).
- Units: accepts px·%·em·rem·vh·vw (`.d.ts` — collector report).
- The current Resizable of shadcn/ui wraps react-resizable-panels **^4** and is styled with Tailwind v4 (shadcn repository `new-york-v4/ui/resizable.tsx` — collector report).

### F4. Peers — apps that handle a terminal split tree mostly use their own engine, and use a library for the flat app shell — certain (what they use) / uncertain (why)

| App | Approach | Source |
|---|---|---|
| orca (Electron) | own imperative pane manager · ratio = `style.flex = "<비율> 1 0%"` | local clone `orca/src/renderer/src/lib/pane-manager/pane-divider-drag.ts:88-89` (checked) |
| paseo | own `split-container.tsx` · ratio → `flexGrow` + `flexBasis: 0` | local clone `paseo/packages/app/src/components/split-container.tsx:852-860` (checked) |
| t3code | own flex layout · not a nested tree but flat terminal groups + one direction per group | collector report |
| vibe-kanban | react-resizable-panels ^4.12.4 — workspace shell | local clone `packages/web-core/package.json:83` (checked) |
| Wave Terminal | blocks use its own TileLayout, only the workspace shell uses react-resizable-panels | collector report |
| Hyper · Tabby · VS Code · Zed | all implemented themselves (Hyper `split-pane.tsx` · Tabby `splitTab.component.ts` percent placement · VS Code gridview/splitview · Zed GPUI `pane_group.rs`) | collector report |
| Theia · JupyterLab | Lumino (imperative widget toolkit) | collector report |

- No peer wrote down its reason for the choice — 「터미널 트리엔 라이브러리가 안 맞아서」 (because libraries do not fit a terminal tree) is an interpretation.

### F5. There is a shape without remounts — **a flat list of leaves + percent absolute placement computed from the tree** — certain (react-mosaic) / likely (Tabby)

- react-mosaic walks the tree, computes a bounding box for each leaf, and renders the leaves as **a single flat list keyed by leaf id**. Each leaf is placed absolutely with `top/right/bottom/left: X%`. The source comment states it directly: 「트리 어디로 옮겨도 재마운트도 DOM 이동도 없다(iframe 이 다시 로드되지 않는다)」 (back-translated: wherever it is moved in the tree, there is neither a remount nor a DOM move (the iframe is not reloaded)) (`libs/react-mosaic-component/src/lib/MosaicRoot.tsx:55-63, 83-90` · `util/BoundingBox.ts:90-98` — original checked).
- With this shape, three classes disappear **structurally:** ① first paint — it draws with CSS percentages only, without measuring ② direction change — direction is only an input to the bounding box computation ③ remount of surviving panes on close·split·promotion — the key of the leaf and the parent (the root) do not change. ③ also solves the loss of the preserved screen of a dead agent in the 「영향」 of ADR-0223 (inference — likely).
- **Cost:**
  - DOM order stops being layout order (tab focus·screen reader order — a trade-off the react-mosaic comment states itself).
  - **Divider drag and minimum size are an algorithm (review finding addressed):** a leaf's CSS minimum cannot stop the tree dividers. We must build the following: for each split, roll descendant minimums up and merge them, compute the draggable range and divider thickness, handle pointer and keyboard operation, and sync the ratio — work that allotment does for us now (`modern.mjs:713-777, 784-875`). There are headless parts that provide only the divider interaction (for example Zag splitter), but the whole-tree constraints are still ours.
  - **Terminal size refitting still happens (review finding addressed):** percent boundaries become fractional pixels, and FitAddon rounds pixels down to decide rows and columns (`node_modules/@xterm/addon-fit/lib/addon-fit.js` · `src/components/slot/TerminalSlot.tsx:93-119`). Even when leaf identity is stable, the reflow across cell boundaries and the PTY resize traffic on resize are not prevented — this happens in every approach and is on a different axis from remounting.
- react-mosaic itself: 7.1.0 (2026-09-10) · Apache-2.0 (LICENSE original checked — GitHub's automatic detection is `NOASSERTION`) · 4.8k★ · takes the tree as a controlled value (`value` + `onChange`) (collector report) · peer `react 16 - 19` (`package.json` original checked).
  - **The dependencies are heavy:** five from the `react-dnd` family (`react-dnd`·`dnd-core` pinned at 16.0.1·html5/touch/multi backends) + `lodash-es`·`uuid`·`immutability-helper` (`package.json` original checked). **There has been no new release of `react-dnd` since 16.0.1 (2022-06)** (npm checked). They come along into the install graph even if drag-and-drop is not used.
  - The window decoration (`MosaicWindow`) is optional — `MosaicRoot` draws the leaves it is given as they are (`MosaicRoot.tsx:83-90`). Blueprint is optional, but the package stylesheet is needed to show the layout controls (README·theme guide — review report).

### F6. Other candidates

- Do not fit an externally owned tree (likely — collector report): dockview (port of the VS Code gridview · owns its own model · imperative) · golden-layout (imperative · last release 2022-09) · Lumino (non-React imperative toolkit) · FlexLayout (its own JSON model is the source of truth).
- Stagnant: Split.js 1.6.5 · react-split 2.0.14 — the last release of both was about 5 years ago (npm — review report. The draft's 「2018 이후」 (since 2018) was wrong).
- **react-split-pane v3 (3.2.0) was not evaluated** — the docs state controlled size, nesting, custom dividers and keyboard resize (review report). Its shape is nested, so it likely shares the remount problem of F3, but this was not checked. The internals of rc-dock were also not checked.

## Options (fit to constraints)

Constraints: ⓐ the backend is the source of truth for the tree, the frontend only draws ⓑ the first paint is right ⓒ in-place direction change ⓓ do not remount surviving panes (terminals) on close·split ⓔ the path toward writing dragged sizes back ⓕ stack (React 19·Tailwind v4·shadcn) and dependencies ⓖ scale of the change

| Option | ⓐ | ⓑ | ⓒ | ⓓ | ⓔ | ⓕ | ⓖ |
|---|---|---|---|---|---|---|---|
| A. allotment + ratio `defaultSizes` | ○ | ○ static analysis (measurement needed) | ✕ key remount stays | ✕ | △ imperative resize needed | ○ | one file · one attribute |
| B. react-resizable-panels v4 nested | ○ | ○ | ○ (likely) | ✕ if it mirrors as is / △ if old ancestors are left as pass-through nodes | △ `setLayout` sync must be built | ○ shadcn wraps it | one renderer file + rewriting 12 allotment harness cases |
| C. react-mosaic | ○ controlled value | ○ | ○ | ○ | ○ `onChange` | △ React 19 peer ○ · brings in the stagnant `react-dnd` family | tree conversion + harness rewrite |
| D. own implementation (flat leaves + percent, orca·Tabby·mosaic shape) | ○ | ○ | ○ | ○ | build it ourselves | ○ zero dependencies | renderer rewrite + divider drag·tree minimum size algorithm built ourselves |

- **A is a local fix that only reduces this symptom.** B blocks the squashing and direction classes, but it blocks remounts only if the renderer is built to diverge from the backend tree. **The ones that block remounts by their very shape are C·D.**
- Rejected candidates (candidates for the ADR's rejected alternatives): dockview·golden-layout·Lumino·FlexLayout — they own their own model or are imperative, so conflict with ⓐ · Split.js·react-split — stagnant.

## Open issues and limits

- No option had a painted frame captured in WebView2 — the 「틈이 닫힌다」 (the gap closes) of A is a conditional static result.
- The cause of the old 「~1px 붕괴」 is unknown (F2).
- Size preservation after react-resizable-panels changes direction, the pass-through node variant of B, and the internals of react-split-pane v3·rc-dock are unchecked.
- The reasons for the peers' choices are not written down anywhere.
