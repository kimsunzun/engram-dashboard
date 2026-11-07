# What to build the library slot (folder browser) on — building-blocks survey

- **Status:** version with the adversarial review (BLOCK) applied · awaiting decision (a sketch from before the preset PRD — implementation forks are settled in the TRD)
- **Date:** 2026-10-14
- **Method:** research medium · design-decision mode. 3 tracks (① what already exists in this repository ② React tree · list components ③ Rust folder-walking · change-detection crates) · main grounding (npm registry measured · code checked) · one cross-family adversarial review (below).
- **Confidence legend:** certain = source text/measurement + cross-confirmation · likely = a single source text/measurement · uncertain = from memory.
- **Constraints (user decision 2026-10-14):** the data (library walking · preset contents · where they are used · re-linking) is held by the daemon, and the client only receives it and draws it (view state = folder · selection · sort · back/forward · personal bundles are on the client). The client does not read the disk directly. Stack = React 19 · Tailwind v4 + shadcn/ui · Zustand · Tauri v2 · Windows first.

## Conclusions (top level)

1. **What already exists in this repository (code checked).**
   - Tree: `react-arborist ^3.9.0` is in the dependencies and is used in `src/components/agent/AgentList.tsx:588-599` with controlled data · custom row rendering · drag-to-move · right-click (multi-select is turned off). There is no wrapped reusable component.
   - The path for sending the whole list at once: `PresetListUpdated` (daemon `crates/engram-dashboard-daemon/src/connection_core.rs` `broadcast_preset_list` → shell `src-tauri/src/daemon_client/events.rs` → frontend store). It is the path of the old 「프리셋(작업 폴더 북마크)」 (preset (working-folder bookmark)) that is to be removed, but the new one can follow its shape as it is.
   - Folder-picker dialog: `@tauri-apps/plugin-dialog` is wired up. Opening a file · opening in the file explorer: `@tauri-apps/plugin-opener` is present, but its permissions are tied to two URLs, so the permission settings must be changed.
   - **Absent:** column sorting · multi-select list · back/forward · a folder-walking/change-detection crate (`walkdir` · `notify` 0 hits).
2. **Daemon (Rust): `walkdir` + `notify` 8.x — use the events only as a 「바뀌었다」 (it changed) signal, and keep a re-walk that runs even without events as the safety net.** Gather events for 0.2–0.5 seconds, then re-walk the library and compare with the previous list (with thousands of entries a re-walk is cheap). The Windows `ReadDirectoryChangesW` buffer is a fixed 16 KiB. ★**`notify` 8.2.0 does not emit a 「다시 훑어라」 (`Rescan`) (re-walk it) signal even when the buffer overflows**★ — that handling (`ERROR_NOTIFY_ENUM_DIR` · 0-byte completion → `Rescan`) exists only on `main` (the 9.0 RC line) (main compared the sources of the two tags directly — an error in the draft · refuted by the review). So with 8.2 the overflowed changes **vanish silently** → keep a re-walk when the slot opens · a re-walk when it gains focus + a periodic re-walk as the safety net (switch to `Rescan` handling when 9.0 stable is out). Pairing up renames · moves must not rely on events and is done by our own comparison (file identity · name). Other pitfalls (review findings · not checked by main): renaming a whole folder can deliver an incomplete path · deleting the watched root folder itself can be noticed only by watching its parent · OneDrive 「온라인 전용」 (online-only) files · Windows long paths. **Likely.**
3. **Screen (React) — two tracks.** For the tree, the already-used react-arborist (keep 3.9 — 4.x, per a review finding · not checked by main, adds Node ≥22.12 and ESM-only on top of the `react-dnd` ^16 peer).
   - **(a) React Aria Components (Adobe · Apache-2.0 · 1.22.1 · React 19 peer — measured):** `Table` gives header-click sorting (`allowsSorting` · `onSortChange`) · multi-select (`selectionMode`) · keyboard navigation · row actions (`onRowAction` — the slot for double-click · Enter) (source text). It has no styles, and there are Tailwind examples. `GridList` · `Tree` are in the same family. **The draft was wrong to say 「탐색기식 다중 선택 · 키보드는 어느 라이브러리도 안 준다」 (no library gives explorer-style multi-select · keyboard) (refuted by the review)** — Shift · Ctrl range selection was not stated explicitly in the docs (going by that page's summary — unchecked).
   - **(b) TanStack Table v9 + Virtual (MIT · headless):** columns · sorting · selection state · controlled data. Keyboard · range selection must be built by hand. According to the review, the shadcn data-table guide is already based on v9, and v9 has a different API from v8, such as `useTable` · a feature-registration style (not checked by main).
   - Either way, **the back/forward · ★ · where-used · not-found rows are built by hand.** **Likely.**
4. **A whole-widget package (SVAR React File Manager etc.) is not recommended.** It gives tree + list + detail + keyboard in full, but its own CSS · event-integration design means the internals would have to be pierced to layer on the daemon snapshot · custom rows · back/forward (docs not read closely — uncertain). Chonky has been effectively abandoned since 2022-04. Commercial widgets (Syncfusion · DevExtreme · Kendo) are excluded because the licence judgment belongs to legal.

## Candidate table (npm registry measured 2026-10-14)

| Candidate | Part it covers | Version · updated | Licence | React 19 | Notes |
|---|---|---|---|---|---|
| react-arborist | tree (virtualized · custom rows · dragging) | 4.0.1 · 2026-10-11 (the repository has ^3.9.0) | MIT | peer `>= 18` | 4.x requires `react-dnd` · `react-dnd-html5-backend` ^16 as peers — whether to stay on 3.x or move up is a TRD matter |
| @headless-tree/react | tree (headless · multi-select · keyboard · dragging) | 1.7.0 · 2026-05-17 | MIT | peer `*` | an alternative if arborist falls short |
| react-complex-tree | tree (multi-select · keyboard · dragging) | 2.6.4 · 2026-08-24 | MIT | peer `>=16.0.0` | no virtualization (from memory) |
| @tanstack/react-table | list columns · sorting · selection state | 9.3.1 · 2026-10-12 | MIT | peer `>=18` | v9 — the shadcn data-table precedent may be based on v8, so API differences need checking (unverified) |
| @tanstack/react-virtual | virtualizing long lists | 3.14.19 · 2026-10-13 | MIT | peer ~19 | with a few hundred entries it may not be needed |
| react-aria-components | list (Table · GridList) · tree (Tree) — sorting · multi-select · keyboard · row actions | 1.22.1 · 2026-10-09 | Apache-2.0 | peer ~19 | found by the review · measured by main · part of the source text checked |
| SVAR React File Manager | whole widget | 2.6.0 · 2026-06 (collector) | MIT | 18 · 19 (README) | not recommended (conclusion 4) |

| Crate | Part it covers | Version | Licence | Notes |
|---|---|---|---|---|
| walkdir | folder walking | 2.5.0 | Unlicense/MIT | inside `spawn_blocking` |
| notify | change detection | 8.2.0 stable · 9.0.0-rc | CC0-1.0 | used by rust-analyzer · Zed · Deno · watchexec (README) |
| notify-debouncer-full | stitching renames together | 0.8.0-rc.2 | MIT/Apache-2.0 | optional · it is an RC, so pinning is needed |
| ignore | .gitignore-style exclusion walking | 0.4.33 | Unlicense/MIT | only when exclusion rules are needed |

## grounding

| Claim | Evidence | Verdict |
|---|---|---|
| react-arborist ^3.9.0 is a dependency · `disableMultiSelection` in AgentList | `package.json:31` · `src/components/agent/AgentList.tsx:588-599` | supported (main checked) |
| `broadcast_preset_list` is called on every create · delete | `crates/engram-dashboard-daemon/src/connection_core.rs:1799,1808,1818` | supported (main checked) |
| walkdir · notify are absent from the repository Cargo.toml | `rg` 0 hits | supported (main checked) |
| npm metadata (version · licence · peer) | `npm view` measured | supported (main checked) |
| notify Windows 16 KiB buffer | `notify/src/windows.rs` in `notify-8.2.0` · `main` | supported (main checked) |
| `Rescan` on overflow | `completion_rescan_event` (ERROR_NOTIFY_ENUM_DIR) only on `main` — not in `notify-8.2.0` | **not supported for 8.2** → conclusion 2 revised |
| React Aria Table = sorting · multi-select · keyboard · `onRowAction` | https://react-aria.adobe.com/Table | supported (Shift/Ctrl range is not in the summary) |
| TanStack Table = headless columns · sorting · row selection | collector | not re-checked by main |

## Adversarial review

codex (cross-family · effort high · level 2 · web search), once. Verdict **BLOCK**. What was applied:

| Finding | Severity | Applied |
|---|---|---|
| `notify` 8.2 does not emit `Rescan` on overflow — the draft used `main` source as the basis for recommending 8.2 | critical | main confirmed it by comparing the sources of the two tags · re-walk safety net added to conclusion 2 |
| React Aria Table · GridList · Tree missing — they give sorting · multi-select · keyboard · row actions | high (omission) | conclusion 3 (a) · table · npm measurement · part of the source text checked |
| the shadcn data-table is already based on TanStack v9 · v9 API changes | high | reflected in conclusion 3 (b) · not checked by main |
| react-arborist 4 = Node ≥22.12 · ESM-only · deep imports removed | medium | reflected in conclusion 3 (keep 3.9) · not checked by main |
| watching pitfalls: folder rename gives an incomplete path · root deletion · OneDrive online-only · long paths | high | listed in conclusion 2 · not checked by main |
| omissions: Ark UI Tree View · Tauri v2 fs watch · watchexec | — | recorded only — given the daemon boundary (the daemon does the watching), the Tauri fs plugin is on the shell side and does not fit (interpretation) |

- No re-review was run after the changes (medium single pass).

## Limits

- SVAR's controlled-mode behavior · the headless-tree feature list · the TanStack Table v9 changes: docs not read closely.
- Collector's suggestion: build a minimal prototype of tree + list + multi-select as one pilot to measure the implementation cost.

## Addendum (2026-10-14) — how mature apps actually combined them

On the user's request 「다른 곳의 라이브러리 조합을 보고 따라가자」 (look at how others combine libraries and follow them), 2 more collection tracks were run (reading sources · manifests directly · light — no adversarial review). Main re-checked the key points against source text.

**The closest peer = Spacedrive** (39.2k★ · commit 2026-10-13) — a separate core daemon + a Tauri shell that only relays (`invoke("daemon_request")` · `listen("core-event")`), so it has the same shape as Engram.

| Area | What Spacedrive uses | Checked |
|---|---|---|
| Watching | `notify = "8.0.0"` (`crates/fs-watcher/Cargo.toml`) · no debouncer crate | main checked |
| Walking | a hand-written parallel `read_dir` worker (`core/src/ops/indexing/phases/discovery.rs`) | collector |
| List | `@tanstack/react-table ^8.21.3` + `@tanstack/react-virtual ^3.13.12` | main checked (`packages/interface/package.json`) |
| Multi-select | `react-selecto ^1.26.3` (box · picking several) | main checked |
| Drag and drop | `@dnd-kit/core` · `sortable` | main checked |
| State | `zustand ^5` · `@tanstack/react-query ^5` | main checked |
| Tree | no dedicated library — appears to be hand-written | uncertain (source not opened) |

Others: Yazi (42.8k★) · broot (13.1k★) · Zed also use `notify` for watching (8.x · Zed uses a fork). For walking: `ignore` (Zed) · hand-written (Spacedrive · broot · Yazi) · `walkdir` (small apps · rust-analyzer). No mature app using react-arborist · React Aria for a file browser was found in this sample.

**What happens after a change signal arrives (source checked).**
- Small folders: **gather, then rebuild everything** — mdBook `notify_debouncer_mini::new_debouncer(Duration::from_secs(1))` (`src/cmd/watch/native.rs:26`) → rebuild the whole book · Zola `notify_debouncer_full` default 1000 ms → for deletes · folder renames etc., `recreate_site()` (`src/cmd/serve.rs`). Renames · overflow need no separate handling (everything is redone wholesale).
- Large trees: **fix only the changed paths, and if unclear re-walk just below that** — Zed (100 ms) · watchexec (50 ms), both handle `Rescan` directly. rust-analyzer has per-path updates · no gathering · no overflow handling.

→ Engram's scale (hundreds to thousands) fits the mdBook · Zola side (gather, then re-walk wholesale + compare), and for the on-screen list, following the Spacedrive combination (TanStack Table + Virtual + a multi-select library) — the closest peer — comes closest to 「성숙한 곳 따라하기」 (following mature apps) (interpretation).
