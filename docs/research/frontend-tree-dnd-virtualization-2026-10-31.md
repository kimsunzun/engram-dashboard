# Frontend tree · drag · virtualization · table · marquee — library survey (client-stage material)

- **Status:** medium · 2026-10-31 · collector 1 + main grounding + codex adversarial review (effort high) — verdict FIX. TRD `docs/process/S25-preset-library/trd.md` §5-1 step 0a (external library survey per implementation part).
- **★Client-stage material — server-first rule (G20)★:** The choices below are **checked again** at the client stage after the server stage is done. The current values are provisional. The 4 spikes to run at that check are in 「공백 · 실측 필요」.
- **Trigger:** The TRD picked `@dnd-kit/core` (memo 113) for the drag system (B6), `react-selecto 1.26.3` for the marquee, `@tanstack/react-virtual 3.14.19` for virtualization, and `@tanstack/react-table 9.3.1` for the table. Check whether those choices fit the current versions and the already-locked `react-arborist 3.9.0`.
- **Confidence legend:** certain = primary source + independent cross-confirmation (main grounding or codex recheck agrees) · likely = a single primary source, no rebuttal · uncertain = estimate/unchecked · contested = the review rebutted it with counter-evidence.

## Conclusion

The input has no MAIN DECISION line. Below are survey conclusions, not settled choices.

| Need | Provisional choice | One-line reason | Next best |
|---|---|---|---|
| B6 one drag system | **keep react-dnd + HTML5Backend** (built into arborist) — but ★**with the corrections below applied**★ | arborist accepts only react-dnd (certain). It cannot be driven with dnd-kit | `@atlaskit/pragmatic-drag-and-drop 4.0.0` · `@dnd-kit/react` (evaluate separately) |
| Virtualization (list) | `@tanstack/react-virtual 3.14.24` | React 19 · MIT · 2026-10-29. Minor difference from the TRD's 3.14.19 | `react-window 2.5.2` |
| Table | `@tanstack/react-table 9.3.1` | Matches the TRD. Whether the v9 API fits the plan is unchecked | — |
| Marquee | **undecided — the claim to replace Selecto is withdrawn** | No ground for replacing it before comparing the coupling with virtualization | `react-selecto 1.26.3` (candidate to keep) · `@viselect/vanilla 3.10.1` · `@air/react-drag-to-select 5.0.11` |

### Corrections the review forced (applied)

- **「공유 react-dnd manager 하나로 트리 · 목록 · 폴더 행이 다 섬」 → withdrawn · corrected.** arborist accepts the drop type `"NODE"`, but takes the ID to be moved **from its own tree state**, and the drag source hook initializes **only the tree it started from**. Even if the manager is shared, list→tree and tree→tree moves do not work by themselves. **A custom drop target/adapter is needed and both directions must be verified.** (the review rebutted it with code → spike 2)
- **「react-dnd 16 추천」 → the compatible combination must be fixed first.** The repository locks arborist 3.9.0 → react-dnd 14 (`package-lock.json:7196`). arborist 4.0.1 needs react-dnd/backend 16. 14 and 16 must not be mixed. Since 「3.x 에 머문다」 (TRD §1-12) is still unresolved, **a concrete combination that includes the existing AgentList must be decided.** (spike 3)
- **「AgentList 가 WebView2 에서 HTML5 드래그 작동을 입증」 → overstated, marked unverified.** `AgentListReparent.test.tsx:49` mocks arborist. It proves only the wiring, not WebView2 native drag. Unverified unless there is a separate runtime observation.
- **「dnd-kit 부적합」 → overgeneralized, scope narrowed.** The basis for the rejection is only the legacy `@dnd-kit/core` (pointer based · single document). The new `@dnd-kit/react` has a native HTML DragSensor (`packages/dom/src/core/sensors/drag/DragSensor.ts` · the dndkit.com migration guide). It must be evaluated separately, and cross-window drag is still unproven.
- **「viselect 가 더 낫다」 → no basis, withdrawn.** viselect looks at the **rects of the mounted DOM** and their intersections. Rows unmounted by virtualization need extra logic (stable ID selection · remount · auto scroll). This must be compared before swapping Selecto out. (spike 4)
- ★**Conflicts with TRD memo 113 (`@dnd-kit/core`)**★ — the survey concludes that dnd-kit cannot drive arborist (certain, `.d.ts`). With the correction above, only the legacy core is eliminated, not all of dnd-kit. TRD memo 113 must be decided again at the client stage.

## B6 — one drag system

- **arborist's coupling (certain, checked in 4.0.1 `.d.ts`):** `getDndProviderProps` accepts only a react-dnd manager (`dndManager`) or backend (`dndBackend`). The internal `useDrag`/`useDrop` are react-dnd. To use dnd-kit, after `disableDrag` + `disableDrop` the tree drop targets must be rebuilt directly.
- **Tauri fit (certain — code + review recheck):** `src-tauri/tauri.conf.json:21` `dragDropEnabled false`, `webview_env.rs:196` `disable_drag_drop_handler`. This must be off for HTML5 DnD to fire on Windows (codex rechecked with the Tauri configuration reference).
- **react-dnd status:** 16.0.1 · 2022-06-25 · peer react >=16.14 · no explicit React 19 (uncertain). The repository already runs react-dnd 14 under arborist 3.9.0 on React 19. Dormant. No sortable helper.
- **arborist 4.x:** exposes `dndManager` · `dndRootElement` (rechecked). Full interoperability is not proven.
- **Cross-window pop-out drag:** dnd-kit (legacy) is pointer based · single document, so it is not possible (likely). HTML5 DnD is an OS drag session, so it might work, but **uncertain · untested.** The behavior of a custom MIME `dataTransfer` between WebView2 windows is unknown. Risk: a bug report that when `dragDropEnabled false`, the source is not redrawn until the drag ends (Tauri issue #9445 · #13761 snippets — uncertain).

| Name | Version | Date | License | Already locked? | Fit | Confidence | Source |
|---|---|---|---|---|---|---|---|
| react-dnd (+ HTML5Backend) | 16.0.1 (the lock has 14) | 2022-06-25 | — (not in the input) | 14 locked (`package-lock.json:7196`) | built into arborist. A shared manager alone cannot move between list and tree | version · dormant = certain (npm + main) / React 19 = uncertain | npm registry |
| @atlaskit/pragmatic-drag-and-drop | 4.0.0 | 2026-09-24 | Apache-2.0 | no | native HTML5 · hitbox package. Cannot be attached to arborist. Coexistence with react-dnd unknown | likely | npm registry |
| @dnd-kit/core (legacy) | 6.3.1 | 2024-12 | — | no | pointer based · single document · cannot drive arborist | certain (cannot drive) | npm registry · `.d.ts` |
| @dnd-kit/react (new) | 0.5.x | — | — | no | has a native HTML DragSensor. Not evaluated | uncertain (not evaluated) | dndkit.com migration guide · `DragSensor.ts` (review citation) |

## Virtualization

- The tree is virtualized by arborist (react-window 1.x). The list is separate.
- `@tanstack/react-virtual` 3.14.24 · 2026-10-29 · MIT · React 19 → minor difference from the TRD's 3.14.19 (likely — a single npm source, no rebuttal). Whether it is locked is not in the input.
- Next best `react-window 2.5.2`.

## Table

- `@tanstack/react-table` 9.3.1 · 2026-10-12 · MIT · react >=18 → matches the TRD. **Whether the v9 API fits the plan is unchecked** (uncertain).

## Marquee (area drag selection)

| Name | Version | Date | License | Already locked? | Fit | Confidence | Source |
|---|---|---|---|---|---|---|---|
| react-selecto (TRD choice) | 1.26.3 | 2023-12-03 | — | not in the input | last updated 2 years ago. No explicit React 19 | version · date = certain (main checked) / React 19 = uncertain | npm registry |
| @viselect/vanilla | 3.10.1 | 2026-09-04 | MIT | no | framework independent. Intersects only mounted DOM rects → virtual rows need separate handling | virtualization limit = certain (the review cited the source) / 「더 낫다」 = contested | npm · `packages/vanilla/src/index.ts` |
| @air/react-drag-to-select | 5.0.11 | 2026-10-09 | — | no | React 16–19 | likely | npm registry |

- Arbitration needed: pressing on a row = drag, pressing on empty space = marquee.

## react-arborist

- 4.0.1 · 2026-10-11 · peer react-dnd ^16 · react >=18 · Node >=22.12 · **ESM only** — the factual basis of the TRD §1-12 「3.x 유지」 is confirmed (codex recheck = matches the arborist 4 manifest). The benefit of moving up to 4 is unknown (changelog unchecked). Currently `package.json` is `^3.9.0` (main checked).

## Adversarial review result (codex · effort high · FIX)

| # | Level | Finding | Handling |
|---|---|---|---|
| 1 | HIGH | The claim that a shared manager makes tree · list · folder rows all work — arborist takes the move ID from its own tree state and the source hook initializes only the tree it started from (`src/dnd/drop-hook.ts` · `interfaces/tree-api.ts`) | applied — claim withdrawn, corrected to a custom adapter + bidirectional verification · spike 2 |
| 2 | MED | react-dnd 16 is recommended but 「3.x 유지」 is unresolved — the lock has 14 · arborist 4 needs 16 | applied — fixing the compatible combination is a prerequisite task · spike 3 |
| 3 | MED | the viselect 「더 낫다」 claim has no basis · virtualization omitted | applied — replacement claim withdrawn, turned into a comparison of the coupling with virtualization · spike 4 |
| 4 | MED | the dnd-kit rejection applies only to the legacy core — the new `@dnd-kit/react` has a native DragSensor | applied — scope narrowed · separate evaluation needed (left) · cross-window is still unproven |
| 5 | MED | overstatement that AgentList proves WebView2 native drag — the test mocks arborist (`AgentListReparent.test.tsx:49`) | applied — marked unverified |
| — | — | Rechecked: the Tauri `dragDropEnabled` requirement confirmed · arborist 4 manifest confirmed · `dndManager` exists, full interoperability unproven | applied — in the body above |

## Gaps · measurements needed (공백 · 실측 필요) (client-stage spikes)

1. **Cross-window drag:** with two Tauri windows (`dragDropEnabled false`), does HTML5 DnD drag between pop-out windows work · does the custom MIME `dataTransfer` get through · does the bug where the source is not redrawn until the drag ends reproduce.
2. **Shared react-dnd manager + custom adapter:** when tree · list · folder rows share one manager, do list→tree and tree→tree moves work in both directions (working around arborist's ID source problem with an adapter).
3. **Compatible combination:** pick one of react-dnd 14 vs 16 / arborist 3 vs 4, including the existing AgentList, and confirm that it actually runs on React 19 (including checking the changelog for the benefit of arborist 4).
4. **Marquee vs virtualization:** compare Selecto · viselect · `@air/react-drag-to-select` on a virtual list (selecting unmounted rows · stable IDs · auto scroll · arbitration with row drag).
- Other: `@dnd-kit/react` 0.5.x not evaluated · coexistence of react-dnd + pragmatic unknown · react-table v9 API vs the plan unchecked · package size is by unpacked size · no WebView2 measurement of AgentList.

## Sources

- npm registry (`npm view` · the `.d.ts` from `npm pack react-arborist@4.0.1`): react-arborist · react-dnd · @dnd-kit/core · @atlaskit/pragmatic-drag-and-drop · @tanstack/react-virtual · @tanstack/react-table · react-selecto · @viselect/vanilla · @air/react-drag-to-select · react-window
- Code: `src-tauri/tauri.conf.json:21` · `src-tauri/src/webview_env.rs:196` · `package.json` (react-arborist ^3.9.0) · `package-lock.json:7196` · `AgentListReparent.test.tsx:49`
- Review-cited sources: react-arborist `src/dnd/drop-hook.ts` · `interfaces/tree-api.ts` · viselect `packages/vanilla/src/index.ts` · dnd-kit `packages/dom/src/core/sensors/drag/DragSensor.ts` · dndkit.com migration guide · Tauri configuration reference (tauri.app)
- Snippets (not read closely): Tauri issue #9445 · #13761 · dev.to puckeditor top-5 dnd · pkgpulse 2026 guide
