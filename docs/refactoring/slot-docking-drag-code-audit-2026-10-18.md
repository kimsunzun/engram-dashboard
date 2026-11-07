# Structure audit — slot-docking drag code (9c working tree · 2026-10-18)

- **Status:** measured (one read-only audit · reviewer preset) · waiting for the redesign direction decision — companion research = [../research/slot-docking-chrome-desktop-drag-2026-10-18.md](../research/slot-docking-chrome-desktop-drag-2026-10-18.md).
- **Trigger:** user 「코드 대충짜지말고 재활용할거 재활용하고 함수화할거 함수화하고 잘묶어주고 해야된다. 이거 뭔가 불안하네」 (2026-10-18 · after the 9c mouse measurement).
- **Scope:** `src/components/layout/{TabBar,WindowLayout}.tsx` · `tabStripLayout.ts` · `src/components/layout/dock/*` · `src-tauri/src/dock/*` · `src-tauri/src/layout/chrome.rs` (test files only for structural problems).
- **Line numbers are against the uncommitted 9c tree** — they drift after a commit or the redesign.

## One-line conclusion

The pure geometry (docking bands · tab-bar width · layout application) survives. Most of the code volume — the shell ledger `book.rs` (about 2,660 production lines) and `dockDrag.ts` (about 1,230 lines) — exists to get the races of HTML5/OLE drag events right, so going the Chrome way means writing it anew, not reworking it.

## Defects (most painful for maintenance first)

1. **`createDockDrag` is one giant closure** — `dockDrag.ts:318-1119` · about 35 mutable variables + 4 snapshot/notify pairs · eight concerns (folding the session record · target decision and reporting · hover-transition timer · swallowing Esc · detecting a lost drag signal with the pointer · source mode and leave timer · the reorder animation state machine · blocking file drops). 9c put the whole source-side state machine (L462-605) into this target-side controller — although the file head says the source side is the job of `TabBar`.
2. **The shape of the 「그린 것」 report is repeated about 7 times** — Rust `DragOverReport::Over` · `DropReport` · `DrawnReport` (`types.rs:265-318`) + `book::Report` and two identical `From` impls (`book.rs:113-146`) · three hand-assembled TS copies (`dockDrag.ts:708-716` · `907-928` · `992-1000`).
3. **`EndLog` copies `EndCause` field by field** — 16 fields (`book.rs:395-444` against `787-827`) · and again at `2182-2212`.
4. **Scroll plumbing that is always 0** — about 120 places in 11 production files (`scroll` in `DockChrome` · `TabBarRegion` · `ReorderRegion` · `StripStart` · three reports · `Report` · `DockGuide` · `ChromeMeasure` · `Drawn` · `dockTabBarScrolled` · `onScroll` · `same_bar` · `sameChrome` · `pick(scroll)`). Edition 5.8 set this aside as 「잠듦 — 배관만 남긴다」 (TRD §7 #64).
5. **The guards of the TS ↔ Rust mirrored logic are all different** — decision rules (`dockPick.ts` ↔ `hit.rs`) = golden JSON · time-limit constants = a test that reads the Rust source with a regex · new-window size (`dragImage.ts:28` ↔ `POPOUT_DEFAULT_SIZE`) = a comment only · `displayedTabOf` ↔ `visible_tab` = none. Inside Rust too, `book::display_active` (`book.rs:2611`) re-implements `regions::visible_tab` (`regions.rs:75`).
6. **Two tab-bar DOM measurers** — `TabBar.measureStrip` (`101-125`) · `WindowLayout.measureDockChrome` (`321-335`) + `dockChromeOf`/`contentRect` (`dockChromeReport.ts:50-70`) · `rectOf` (`TabBar:127`) — all pick by `data-testid`. `measureDockChrome` does not subtract the transform, so `WindowLayout` has two refs attached only to get around that (`reorderStandingRef` · `seenChrome` — L76-83 · L217-227).
7. **The reorder geometry is in four places** — `hit.rs:319-346` · `dockPick.ts:77-98` tied by the mirror · golden, plus `dockDrag.ts:521-531` · `TabBar.tsx:140-155` with no tests (implicitly holding 「`[index, dragged)` 이웃이 한 폭씩 밀린다」).
8. **A copied reporter that sends one at a time** — `dockChromeReport.ts:72-205` ≈ `windowCanvasReport.ts` · two 100 ms debounce constants written as 「같은 값」 (`WindowLayout.tsx:345` · `dockChromeReport.ts:42`).
9. **Small things** — `inner_origin` · `scale` · `label` of `hit::WindowFrame` are used only by tests (production is always 0 · 1 · "" — `regions.rs:190`) · `pick` rebuilds `DockChrome` from `DockRegions` and calls `locate` · the unused `dragImage` `slotLabel` · the predicate 「이 창이 도킹 모드의 출처」 three times (`dockDrag.ts:187` · `:464` · `WindowLayout.tsx:97`) · six hand-written equality checks · magic numbers (the 28 px tab-bar height in three places · `TAB_MIN_ACTIVE_W = 42` derived by hand from padding) · tests: about 3,900 lines inside `book.rs` · the 1,523 lines of `dockDrag.test.ts` pin down the OLE timing races — the only tests that carry over to the redesign are `pick_golden.json` · the `hit.rs` geometry · `tabStripLayout`.

## Classification — under a Chrome-style redesign

| Piece | Decision | Reason |
|---|---|---|
| `layout::apply::dock_tab` · `DockDest` · `DockGuard` · `lease_tab` · `docked_tree` · `incoming_rect` · `succession` | keep | model change · guard — the same however the input arrives |
| `hit.rs` docking bands (`outer_strips` · `valid_bands` · `landing_rect` · `choose_band` · `choose_seam`) | keep | pure geometry (but split attach area < preview — research C) |
| `hit.rs` `locate` tab-bar side · `reorder_zone` · `REORDER_SLACK_PX` | keep | `REORDER_SLACK_PX` 15 = Chrome `kVerticalDetachMagnetism` · attaching before the drop is decided in screen coordinates, so the `WindowFrame` origin · scale are used again — do not delete them, use them |
| `regions.rs` `regions_for` · `slot_regions` · `outer_bands` | keep | building per-window targets |
| `regions.rs` `rev` · `bar_rev` · `bumps` · `Expect` | unclear | for 「비동기 보고 뒤 그린 것 확인」 — can be reduced if one process decides |
| `snap.rs` · `DockState` | keep | snapshot under the layout lock · session generation |
| `commands/dock.rs` ports (`DockPorts` · `DockSpawn` · syncing hide/show on the main thread) | keep | a testable seam · also needed for the window that follows the cursor |
| `commands/dock.rs` command set (`dock_drag_over` · `drop` · `end` · `pointer_seen` · `pull`) | drop | HTML5 event protocol |
| `book.rs` | drop (about 85%) | wait stages · engine cancel · six race timers — only the skeleton of session id · lease · apply once · end log (about 15%) is reused |
| `types.rs` `PxRectWire` · `Band` · `OuterBand` · `SlotRegion` · `Picked` | keep | geometry wire |
| `types.rs` `DragOverReport` · `DropReport` · `DrawnReport` · `DragEffect` · `StripStart` | drop | OLE report protocol |
| `layout/chrome.rs` · `dockChromeReport.ts` | keep | attaching before the drop needs other windows' tab-bar rectangles |
| `tabStripLayout.ts` | keep | Chromium width · hide rules |
| `DockOverlay.tsx` | mostly keep | seam · band drawing · the insertion-slot bar is replaced by a real insertion |
| `dockPick.ts` | unclear | the mirror may go away — inside the tab bar = TS only · between windows = Rust only |
| `dockDrag.ts` listeners · swallowing Esc · pointer sighting · `lastDrawn` · leave timer | drop | HTML5 events · races |
| `dockDrag.ts` blocking file · URL drops (`272-277` · `833-844` · `877-881`) | keep | always needed (AC22) — do not delete it along with the file, split it out |
| `dragImage.ts` | drop | the new model has no OS drag image |
| `TabBar.tsx` | split | renaming · layout = keep · `draggable` · `dragstart`/`dragend` · `ENGINE_CANCEL_MS` · `setTimeout(0)` session opening = drop · `reorderStyle` = keep the concept |
| `WindowLayout.tsx` docking wiring | unclear | to be pulled out into a hook — the redesign decides the content |

## Proposed module structure (where the redesign lands)

- **Rust `src-tauri/src/dock/`** — `geometry.rs` (docking bands · preview · seam) · `strip.rs` (insertion slot · attach slot · detach slack) · `targets.rs` (snapshot + chrome → per-window targets · `target_at(화면 점)`) · `session.rs` (press → reorder → detached and following → attached to W → end — a single event source) · `follow.rs` (the port for the window that follows the cursor — create · move · show/hide) · `exec.rs` (ports · main-thread syncing · apply — the slimmed-down `commands/dock.rs`) · `wire.rs` · the one mutation path is `layout::apply` (add `move_tab` for reorder · attach before the drop).
- **TS `src/components/layout/tabStrip/`** — `layout.ts` (today's `tabStripLayout`) · `measure.ts` (one DOM measurer that subtracts the transform) · `reorderGeometry.ts` (following · slot · insertion x · neighbor shift) · `useTabDrag.ts` (threshold · pointer capture · moving the real element · detach slack · Esc — talks to the shell only on detach · attach · end).
- **TS elsewhere** — `TabBar.tsx` (drawing · renaming only) · `dock/dockSession.ts` · `dock/DockOverlay.tsx` (draws the guidance the shell pushes) · `dock/chromeReport.ts` (on top of `util/latestValueReporter.ts`) · `dock/fileDropGuard.ts` (always on) · `useDockWindow.ts` (the wiring pulled out of `WindowLayout`).
