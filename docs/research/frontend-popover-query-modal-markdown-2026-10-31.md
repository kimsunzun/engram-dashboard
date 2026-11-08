# Frontend external libraries — detail popover · query cache · modal · markdown (survey · medium)

- **Status:** medium · 2026-10-31 · collector 1 + main grounding + codex adversarial review (effort high) — verdict FIX. This is **client-stage material** for step 0a of TRD `docs/process/S25-preset-library/trd.md` §5-1 (external library survey per implementation part) — under the server-first rule (G20) it is material to be used only after the server side is settled, and nothing is settled here.
- **Trigger:** Choose the external libraries to build on for the DetailFlyout of TRD §1-10 (a detail card shown at the window layout root · flip D11 then clip) · the blocked launch window · the library data cache (T1) · the preview markdown.
- **Confidence legend:** certain = primary source + independent cross-confirmation (main grounding or codex recheck agrees) · likely = a single primary source, no rebuttal · uncertain = estimate / unchecked · contested = the review rebutted it with counter-evidence.
- **Repository measurement (collector):** Radix direct dependencies = react-context-menu · react-dropdown-menu · react-scroll-area · react-tooltip (1.2.9). No react-popover · hover-card · dialog. `@floating-ui/{core,dom,react-dom 2.1.8,utils}` comes in via Radix. No `@tanstack/react-query`. Markdown = react-markdown ^9.1.0 + remark-gfm · remark-math · rehype-katex · rehype-highlight · katex. A hand-written `role="dialog"` exists in `UsageSlot.tsx` (about 991).

## Conclusion

| Need | Choice | One-line reason | Next best | Correction the review forced |
|---|---|---|---|---|
| Detail card (DetailFlyout) | **`@floating-ui/react-dom` directly** (already in the lockfile at 2.1.8, latest 2.1.9) | The flip→shift→clip ladder of TRD §1-10 (right → left → overlap above the anchor) requires choosing the middleware order directly | `@radix-ui/react-popover` 1.3.1 — if the ladder is proven | ★correction★ The collector's first recommendation was Radix Popover, but **the review refuted it** (the installed popper applies shift before flip). Unless the ladder is proven first, do not use Radix |
| Blocked launch window | **keep a separate Tauri window as in the TRD** | An in-page modal cannot go outside the small main window | — | The owned window (`parent` option) was reconfirmed by the review |
| Data cache (T1) | **`@tanstack/react-query` 5.105.0** | A T1 decision the TRD already settled (L563 rejects the hand-written Zustand cache) | hand-written Zustand + eventBus — rejected by the TRD | ★correction★ It is **settled**, not 「필요하면」, and is confined inside `libraryQueries.ts`. For local bus queries, **`networkMode: 'always'`** is required |
| Modal | **`@radix-ui/react-dialog` 1.3.1** | focus trap · aria · Esc · portal container | native `<dialog>` | ★correction★ Native `<dialog>` also has `cancel`/`close` events and CSS enter/exit animation (「훅이 없다」 was wrong). Need to check the Radix stacked-layer Esc regression |
| Markdown preview | **reuse react-markdown ^9.1.0, without rehype-raw** | Raw HTML is escaped by default | upgrade to 10.1.0 (optional) | ★correction★ The safety decision treats not only raw HTML but also **the configured plugins · components · URL transforms** as a trust boundary |

## Detail card (flip→shift→clip)

| Name | Version | Date | License | Already in the lockfile? | Fit | Confidence | Source |
|---|---|---|---|---|---|---|---|
| `@floating-ui/react-dom` | 2.1.9 (lockfile 2.1.8) | not stated | not stated | yes (via Radix) | chooses the middleware order directly → fits the ladder | likely (collector + main grounding's check of the dependency chain) | npm registry |
| `@radix-ui/react-popover` | 1.3.1 | 2026-10-14 | MIT | no (no direct dependency) | **does not fit the ladder (contested)** | contested — codex pointed out that in the installed `@radix-ui/react-popper` `dist/index.mjs:139`, `shift()` is applied **before** `flip()`, cross-axis shift is turned off, and the available-size variables are exported but the limit is not applied | npm registry · codex review |
| `@floating-ui/react` | 0.27.20 | — | — | no | overlaps with Radix | likely | collector |
| `radix-ui` umbrella package · hover-card | — | — | — | — | the repository depends per package · toggles on click, so it is not a hover-card | likely | collector |

- **Main grounding:** confirmed the chain `@radix-ui/react-popover` 1.3.1 → `@radix-ui/react-popper` 1.4.0 → `@floating-ui/react-dom` ^2.0.0 (supported) — since it is the same family, the collector's benefit that no new copy of floating-ui is added is right. But the benefit does not outweigh the ladder mismatch.
- Radix exposes only props (side · align · avoidCollisions · collisionPadding · collisionBoundary · sticky · hideWhenDetached · onPlaced) and cannot take user middleware. The 「정확한 미들웨어 순서」 the collector wrote was a guess made without reading the source, and **the review refuted it with the source**.
- The portal container = the window layout root (the same whether through the Radix `container` prop or a direct render).

## Blocked launch window

- Keep a **separate Tauri window** as in the TRD (owned by main · via `WebviewEnv::finish`). An in-page modal cannot display outside the small main window.
- Cost: capability JSON · route · label cleanup. The owned window (`parent`) was reconfirmed by codex (holds).

## Data cache

| Name | Version | Date | License | Already in the lockfile? | Fit | Confidence | Source |
|---|---|---|---|---|---|---|---|
| `@tanstack/react-query` | 5.105.0 | 2026-10-26 | MIT | no | `queryFn` can be any Promise · `invalidateQueries`/`setQueryData` on a bus push | certain (collector + codex recheck + main's peer `react ^18\|\|^19` check) | npm registry · TanStack docs |
| Zustand + eventBus by hand | — | — | — | yes | rejected by TRD L563 | — | TRD |
| SWR | — | — | — | — | unchecked | uncertain | — |

- **Settled (no MAIN DECISION — TRD T1 already settled it, the review confirmed):** TanStack Query is not optional. The scope is limited to inside `libraryQueries.ts`.
- ★`networkMode: 'always'`★ — the default `'online'` pauses queries when offline. A local bus query is not HTTP and needs this mode (tanstack.com network-mode guide).
- Other: `QueryClientProvider` is needed · being non-HTTP, turn off window-focus refetch · retry. The tension with the repository rule 「두 번째 상태 소유자」 (agentClient + Zustand) is treated as a design T1 has already decided.

## Modal

| Name | Version | Date | License | Already in the lockfile? | Fit | Confidence | Source |
|---|---|---|---|---|---|---|---|
| `@radix-ui/react-dialog` | 1.3.1 | not stated | not stated | no | focus trap · aria · Esc · portal container | likely | collector |
| native `<dialog>` | — | — | — | — | `showModal` holds in WebView2 (Chromium) | certain (collector · no rebuttal from the review). 「닫힘 훅 · 애니메이션 없음」 is **contested** — the HTML spec has `cancel`/`close` events and the Chrome blog has CSS enter/exit animation | HTML spec · Chrome blog |

- The repository's hand-written `role="dialog"` (`UsageSlot.tsx` ~991) is recorded only as a replacement candidate.
- ★Stacked-layer Esc★ — `radix-ui/primitives#4143` (DismissableLayer 1.1.19 closes two stacked dialogs together). Check the installed version and test 「나중에 연 것이 먼저 닫힘」 of TRD D11. Since Popover is not chosen, the affected combinations are modal ↔ modal · modal ↔ other Radix layers.

## Markdown preview

- Reuse react-markdown ^9.1.0, without `rehype-raw` — raw HTML is escaped by default (certain — collector + codex recheck). The latest 10.1.0 (2025-03-07) is an optional upgrade. `rehype-sanitize` 6.0.0 only when raw HTML is turned on. Excerpt = string truncation.
- ★correction★ Security is not closed by raw HTML alone — plugins · `components` · URL transforms are also trust boundaries (react-markdown#security). Check **the configured plugin set** (remark-gfm · remark-math · rehype-katex · rehype-highlight · the `components` the preview uses · link URL handling) separately.

## Adversarial review result (codex · effort high · FIX)

| # | Severity | Finding | Handling |
|---|---|---|---|
| 1 | HIGH | Radix Popover cannot give the D11 ladder (shift before flip · cross-axis shift off · available-size limit not applied — popper `dist/index.mjs:139`) | **applied** — DetailFlyout uses `@floating-ui/react-dom` directly. Radix only if the ladder is proven |
| 2 | MED | TanStack Query is a settled T1 decision, not optional | **applied** — described as settled · limited to `libraryQueries.ts` |
| 3 | HIGH (omission) | Local bus queries need `networkMode: 'always'` | **applied** — raised to required in the conclusion |
| 4 | LOW | Native `<dialog>` has `cancel`/`close` · CSS animation | **applied** — corrected the collector's description |
| 5 | MED | react-markdown plugins · components · URL transforms are also a security boundary | **applied** — added a check of the configured set to the gaps |
| 6 | MED | Radix stacked-layer Esc regression (#4143) | **left** — checking the installed version · testing is the implementation pilot |
| — | — | Reconfirmed: Promise `queryFn` · raw HTML escape · owned Tauri window (`parent`) | agrees |

## Gaps · measurements needed (implementation pilot)

- Build the DetailFlyout ladder (right → left → overlap above the anchor → clip) with `@floating-ui/react-dom` middleware order and measure the edge cases in a real window. To use Radix, prove it with the same test.
- No WebView2 measurement (modal · portal · window) · bundle size not measured.
- The Tauri capability JSON of the blocked launch window is unverified.
- Radix Dialog stacked-layer Esc (#4143) — test on the installed version.
- Set `networkMode: 'always'` and turning off window-focus refetch · retry as `QueryClient` defaults, and test.
- Security check of the markdown plugin set. SWR unchecked (low need because T1 is settled).

## Sources

- npm registry (`npm view` — collector + main rerun): `@radix-ui/react-popover` 1.3.1 → `@radix-ui/react-popper` 1.4.0 → `@floating-ui/react-dom` ^2.0.0 · `@tanstack/react-query` 5.105.0 (peer react ^18||^19) · `@floating-ui/react-dom` 2.1.9 · `@floating-ui/react` 0.27.20 · `@radix-ui/react-dialog` 1.3.1 · react-markdown 10.1.0 · rehype-sanitize 6.0.0
- Repository: `package.json` · `node_modules` · `node_modules/@radix-ui/react-popper/dist/index.mjs:139` (codex) · `UsageSlot.tsx` ~991
- TRD `docs/process/S25-preset-library/trd.md` L401 · 418–423 · 511 · 563
- tanstack.com network-mode guide · react-markdown#security · radix-ui/primitives#4143 · HTML spec (`<dialog>` `cancel`/`close`) · Chrome blog (`<dialog>` animation) — the input had only the names of the URLs, so full addresses cannot be written
