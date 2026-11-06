# Four felt problems after slot docking OS dragging — practice and measurements (2026-10-21)

> **Status:** Investigation finished · decision (user 2026-10-21 — end of the appendix) — ① warm-up window = adopted (「예비창을 띄워줘」 (put up a spare window) · always keep one warmed from the start) · ② prohibition cursor = undecided (awaiting a judgment in appendix 2 — if blocked, leave it as is) · ③ obscuring = leave as is (you cannot drop on an obscured spot) · ④ tab color = ADR-0298. Trigger = the user's measurement after U3 (2026-10-21): ① the new window from a desktop drop is slow ② a prohibition cursor outside our windows ③ the dragged source window and other apps' windows cover the window to drop on ④ the active tab is not visible.
> **Strength:** medium — 4 collectors (Claude · document type) + main synthesis · grounding + 1 codex adversarial review (FIX 7 — folded in). Measurements = reading the mouse button state (no input injection) · app logs · CDP Navigation/Resource timing on the new window page (read only).
> **Links:** TRD `docs/process/S21-slot-docking/trd.md` §9 · ADR-0296 · ADR-0297 · old warm-up investigation `.claude/handoff/attachments/slot-docking-popout-prewarm-research-2026-10-19.md` (untracked).

## Conclusion

1. **Most of the new-window delay (≈ 0.6 s in the dev build) is window building + page loading — the OS share is small.** Most of the 0.16–0.19 s (3 runs) from detaching to the shell apply log appears to be **building the WebView2 window and controller** — the apply log is printed after the window has been fully built. That span also mixes in reading input · source geometry · monitors and placing the position, so it is not a value for building alone (≈ 0.16 s measured from the navigation start time · codex review finding) (at `layout/apply.rs:1242` `open_at` → `:1276` `record_placement`, a getter waits on the main thread for the build to finish). Apply → visible ≈ 0.40 s = dev server module loading and execution ≈ 0.27 s + mount · serial IPC ≈ 0.10 s + first-paint gate ≈ 0.03 s. Detach → dragend is estimated at ≤ ~0.03 s (unchecked). **Confidence: likely** (span boundaries = measurement of sample 1 + agreement of 3 logs · release figures are estimates).
2. **The main way to reduce it = adopt a hidden warm-up window** (Electron `show:false` · Chrome spare renderer · the same conclusion as the old investigation of 2026-10-19). Old spike S8 「pool」 = visible p50 35.8 ms · flicker 0 — but that is a value from another flow that moved over an already-drawn off-screen window, and **the first show of a hidden window has never been measured** (`.claude/handoff/attachments/spike-chrome-drag-2026-10-18.md:78-96`). After adoption, the real figures are measured by implementing it. The current new drag path **does not use** the spare window built at boot (`slot-popup-1`) and builds a new one every time (`reused=false`). **Confidence: certain** (log · code).
3. **The industry also leaves the prohibition cursor as is.** Chromium returns `DRAGDROP_S_USEDEFAULTCURSORS` from `GiveFeedback`, leaving the cursor to the OS, and the OS shows the prohibition cursor when the target answers `DROPEFFECT_NONE`. OLE itself allows a drag source to supply the cursor directly (the `IDropSource::GiveFeedback` contract), but no path was found by which WebView2 or Chromium hands that handle to the page — it is not 「불가능」 (impossible) but 「우리가 닿는 길 없음」 (no path that reaches us). Peers that make a new window outside the window with an HTML5 drag (the Infragistics Dock Manager Electron example · `zcbenz/dnd-drag-hint`) use `dropEffect === 'none'` of dragend as the 「새 창」 (new window) signal (those documents do not describe the cursor the user sees). If we carry a format the desktop accepts (a file, etc.), the cursor changes, but dropping it creates a real file or shortcut on the desktop, so it cannot be used (inference — unchecked). **Confidence: certain** (Chromium source) / peer practice = likely.
4. **Obscuring — raising the window under the cursor during a drag is OS shell-level practice** (Windows taskbar: hovering over a button during a drag raises that window to the front 「잠시 뒤」 (after a moment) · KDE Plasma 5.24 「1–2 초 머물면 올림」 (raises after hovering 1–2 seconds) · macOS spring loading is for folders). No case was found of a tab docking app doing it itself (VS Code · JetBrains · Visual Studio — unknown). (The Metacity 「뗄 때 올림 기각」 (raise-on-release rejected) that the old edition wrote here is **about postponing the click-raise of the source window**, so it is not a basis for this conclusion — codex review finding · raising the target window during a drag is covered separately by GNOME bug 112308.) Peers with their own drag (Chrome · KDDockWidgets · AvalonDock) have the dragged window follow the cursor and exclude it from the target search (z walk) — whether we, being an OS drag, can use the same path depends on whether OLE skips `WS_EX_TRANSPARENT` layered windows, and that is unchecked (the docs only say that a layered window + transparent passes the mouse below). Cases solved with translucent · click-through · hidden: 0. **Confidence: likely** (mostly issues · search summaries — the delay ms not checked against the source text).
5. **Active tab distinction — peers where 「활성 = 내용과 같은 면」 (active = the same surface as the content) keep the tab bar at a different brightness from the content.** Zed Ayu Dark: active = content `#0D1016` · tab bar = inactive `#1F2127` (the active one is the darkest and the rest are lighter — ΔL* ≈ 9, an estimate) · Zed One Dark the same direction (ΔL* ≈ 3) · VS Code Dark Modern is the opposite direction (tab bar · inactive `#181818` < active · content `#1F1F1F`) + a 1 px highlight line above the active tab `#0078D4` · text `#FFFFFF` vs `#9D9D9D` · JetBrains Darcula underline 3 px `#4A88C7`. Ours is tab bar `#000000` · active = content `#0a0a0a`, so ΔL* ≈ 2.7 (computed — the 「≈ 2」 first written was main's estimate) and it is not visible. **Confidence: certain** (original theme values — via a tool summary) / ΔL* = main's estimate.

## 1. New window delay — span table (sample `slot-popup-9` · dev build · vite 1420)

| Span | ms | Basis | Status |
|---|---|---|---|
| detach → dragend · IPC sent | ≤ ~30 | the difference between button release → apply 164–187 (3 runs) and dragend → apply 197 | estimate |
| before the shell builds (input · source geometry · monitor) | < 5 | `commands/tab_drop.rs:51-99` | estimate |
| WebView2 window and controller build → Navigate | ≈ 160 | `performance.timeOrigin` − IPC sent = 166 · old spike S8 fresh 163–222 | measured (1) |
| shell after build (position · dock · getter) → apply log | 31 | navigation start → apply log | measured |
| HTML | 7 | Navigation timing | measured |
| dev module waterfall (180 requests · 9.4 MB · 11 levels of static import) | ≈ 185 | Resource timing — the popup path is not lazily loaded (`App.tsx:4` · `:20`) | measured |
| module execution → DOMContentLoaded | ≈ 80 | | measured |
| React first commit (dev · StrictMode) | ≈ 23 | | measured |
| serial data chain (three listens → `window_role` → `list_tabs` → `get_view` → first paint) | ≈ 77 | endpoints measured · the chain is from code (`dockSession.ts:151` · `WindowLayout.tsx:97` · `:142` · `:179` · `:199`) | measured + code |
| first-paint gate (double rAF → `webview_painted` → show) | ≈ 33 | `firstPaint.ts:34` · the 100 ms cap did not trigger | measured |

- Other runs: apply → visible 375–410 ms (once 476). The menu 「팝업으로 분리」 (detach to popup) = 375 ms — the same path.
- **Release estimate (unmeasured):** build ≈ 160 as is · module loading and execution → 100–150 (bundle 1.68 MB · stand-in measurement of V8 compilation 43 ms · whether the custom scheme has a code cache is unchecked) · chain 40–60 · gate as is → **dragend → visible ≈ 330–420 ms**.

### Ways to reduce it

| Option | Gain | Cost |
|---|---|---|
| A adopt a warm-up window (new window = put the tab into a hidden spare window and show it · right after, build the next spare window again) | build · load · mount drop out — the figure is a hypothesis (S8 is another flow · conclusion 2 above) | memory of one always-hidden WebView2 · ADR-0296 decision 5 · conflicts with TRD U4a (removing the spare window) → a decision is needed · consecutive drags within 0.5 s fall back to building anew · the first show of a window hidden with SW_HIDE is unmeasured |
| B build ahead when the drag leaves the source window | the same as A if the drag lasts 0.5 s or more · no idle window | a wasted build if dropped on our window · the build holds the main thread for ~160 ms, so synchronous commands of other windows stall during the drag |
| C send the materials for the first paint along when building (role · tab · snapshot) | 30–60 ms (dev) · 15–30 (release estimate) | a second data path |
| D per-route code splitting | 50–120 ms (dev) · 10–25 (release estimate) | Suspense boundary · content fills in after it is visible |
| E first-paint gate with a single rAF | ≈ 16 ms | risk of showing a frame not yet drawn |

### Also seen

- The name of the apply log misleads about the stage (it is printed after the build) — leaving the build time line (`build_ms`) separately lets this table be read from the log alone.
- The spare window and the guide window are still built at boot and left unused (ADR-0296 decided to remove them — U4a), and are loaded again with every full vite reload.
- When a popout is re-attached and the window disappears, `PostMessage failed … 0x80070578` (wry dev build output — sent to a closed window) — unrelated to performance.

## 2. Prohibition cursor

- Chromium `ui/base/dragdrop/drag_source_win.cc` — `GiveFeedback` returns `DRAGDROP_S_USEDEFAULTCURSORS` (certain · source text). MS `IDropSource::GiveFeedback` docs — it is called with `DROPEFFECT_NONE` when leaving the target (certain).
- VS Code #10121 (prohibition cursor when dragging outside the window · dropping on Explorer copies the file) → 1.85 (2023-11) made a floating window on a desktop drop. The cursor shape after that is unknown.
- Firefox 1598915 — other apps (VS Code · Discord, etc.) only accepted with `DROPEFFECT_COPY` and did nothing, so the tab did not become a new window; 132 fixed it as 「자기 탭 형식은 바깥 앱의 받음 응답을 무시」 (its own tab format ignores an outside app's accept response) (130's fix had the condition inverted, so 130 · 131 are affected). The same direction as our Q39 (a new window even if another app accepts) (certain · bug tracker summary).

## 3. Obscuring

| Peer | During the drag | How to reach the obscured target | Confidence |
|---|---|---|---|
| Windows taskbar | hovering over a button during a drag brings that window to the front (the Windows 11 22557 announcement is for **file** dragging) | via the button — whether it works for our tab drag is unchecked | behavior certain · delay ms unknown (400 ms is a general hover value — unchecked) |
| KDE Plasma 5.24 | raises after hovering 1–2 seconds (bug 176582 FIXED) | hovering | likely |
| GNOME / Metacity | no raising of the target during a drag · the discussion of raising the target = bug 112308 (unresolved) | — | likely |
| Chromium | the dragged window follows the cursor · the dragged window is excluded from the target search (`GetLocalProcessWindowAtPoint(…, ignore)`) | own z decision | likely (Windows implementation not read) |
| KDDockWidgets · AvalonDock | the floating window follows the cursor · z walk excluding the dragged window · the guide window lets the mouse through | own z decision | certain (source) |
| ChromeOS | there is a `tab_drag_drop_windows_hider` (judging by the name, it hides other windows) | — | uncertain (not read) |
| VS Code · JetBrains · Visual Studio | no basis | — | unknown |

- Whether OLE's target window selection skips `WS_EX_TRANSPARENT` · alpha 0 · hidden windows — not in the MS docs (unknown). Wine wrote it separately with `ChildWindowFromPointEx` instead of `WindowFromPoint` (search summary — not read).
- A spike was done with an external script that turned the source popout translucent + click-through during the drag, but it stopped before the result was judged (user observation: translucency also applied to title-bar drags — the script reacted to every drag inside the popout). **No result.**

## 4. Active tab

| Peer | Tab bar | Active | Inactive | Content | Marker | Confidence |
|---|---|---|---|---|---|---|
| VS Code Dark Modern | `#181818` | `#1F1F1F` | `#181818` | `#1F1F1F` | 1 px above `#0078D4` · text `#FFF`/`#9D9D9D` | certain |
| Zed Ayu Dark | `#1F2127` | `#0D1016` | `#1F2127` | `#0D1016` | — | certain (tool summary) |
| Zed One Dark | `#2F343E` | `#282C33` | `#2F343E` | `#282C33` | — (`#464B57` is a general border value) | certain (tool summary) |
| JetBrains Darcula | `#3C3F41` | `#4E5254` | `#3C3F41` | — | underline 3 px `#4A88C7` (application to editor tabs is inference) | likely |
| Firefox | — | — | — | — | shadow · outline in high contrast | marker certain · value unknown |
| Chrome · Windows Terminal · JetBrains New UI | unknown | | | | | |

- No guideline for the brightness difference between active and inactive surfaces (ΔL · contrast ratio) was found. WCAG 1.4.11 (non-text 3:1) is from memory — unchecked.

## Limits

- The collection tool could not read some source texts (Chromium `window_finder` Windows implementation · Raymond Chen's original · the Windows Terminal spec · WinUI resources) — those rows are based on commit messages and search summaries.
- The delay spans are the boundary measurement of sample 1 + 3 logs. All release figures are estimates.
- The translucent · click-through spike stopped before judgment — no conclusion.

## Appendix — second investigation of the obscured target (source level · 2026-10-21) · decision

To answer the user's questions 「A 창 왼쪽에 붙이고 싶은데 다른 창이 가리면?」 (I want to attach to the left of window A, but what if another window covers it?) · 「다 가이드 방식이야?」 (is it all the guide method?), two collectors read the source directly (Claude document type · this appendix did not go through a codex review).

| Target | Handling of an obscured target | Basis |
|---|---|---|
| KDDockWidgets | **topmost guide** — if it is probably obscured (`DockRegistry::isProbablyObscured`), it shows the outer guides again. Comment: 「there might be another window obscuring our target, so it's useful to show the outer indicators」 (`src/core/DropIndicatorOverlay.cpp` 183–191) · Win32 z walk (`DragController.cpp` `qtTopLevelUnderCursor`) | source |
| Qt-ADS · AvalonDock · DockPanel Suite | topmost guide (a separate top-level window) + exclude the dragged window from the decision | source |
| Dear ImGui (docking) | the dragged viewport as no-input · guides are inside the host window · admits in a comment that it does not handle 「non-imgui windows … in-between」 (`imgui.cpp` 17192) | source |
| Eclipse e4 | punches a hole (Region) at the cursor position in the dragged shell and decides on what is below | source |
| NetBeans | excludes an obscured window from the targets (cannot drop) | source |
| wxAUI · Qt QDockWidget | looks only at geometry (ignores obscuring) | source |
| Chromium | excludes the dragged window and goes through our process's windows in z order · **does not drop on an obscured window** — 「tab drag drop shouldn't drop on an occluded browser window」 (`ui/display/win/local_process_window_finder_win.cc`) | source |
| IntelliJ | looks only at the screen rectangle and does not see obscuring (a drop works even on an invisible window) · an empty floating window gets alpha 0.5 during the drag (`DockManagerImpl.kt`) | source |
| Unreal Slate | a cursor decorator window (0.45 opacity) · decides inside the window the OS determines · does not see covering by another app's window | source |
| VS Code (HTML5 drag like ours) | no handling — events do not reach an obscured spot, so it cannot be dropped there | code summary |
| Raising the target window | **0 places** | — |

- Count: topmost guide 4 (+ImGui in part) · excluding the dragged window 7 (frameworks) · raising the target 0 · cannot drop on an obscured spot = Chromium · VS Code · NetBeans.
- **Decision (user 2026-10-21): leave it as is — you cannot drop on an obscured spot (the same as VS Code · Chromium).** No topmost guide · no raising the window during the drag (Q36) either. Reconfirmed 2026-10-23 — user 「언리얼 유니티 관행보고 걔네들도 안하면 그냥 안하는 방향으로.」 (look at Unreal and Unity practice, and if they do not do it either, just go the way of not doing it.) → session check: confirmed that Unreal Slate is among the 13 places of the table above (12 = source · VS Code = code summary — raising the target window is 0 places) and does not raise · Unity not investigated.
- **Decision (user 2026-10-21): warm-up window — always keep one hidden window when the app starts, and for a new window from a desktop drop, use that window and build one again right after.**
- **Decision (user 2026-10-21): prohibition cursor — investigate more** (shell-side means · result = next session).
- **Decision (user 2026-10-21): tab color — dark tab bar `#242424` · active = content `#0a0a0a` (Zed Ayu shape) · separator 1×13 (Chrome 16/34 ratio) · end separator · × · + = 22 px (user decision) · icon 13 = Chrome ratio · dark inactive text `#999` (「업계 기준으로 추천하는대로」 (as you recommend by industry standard) · ADR-0298).**

## Appendix 2 — ways to change the prohibition cursor (2026-10-21 · 1 collector · no review)

- **It cannot be changed through WebView2 · Chromium settings · flags · DataTransfer** — `DragSourceWin::GiveFeedback` is fixed at `DRAGDROP_S_USEDEFAULTCURSORS` (`ui/base/dragdrop/drag_source_win.cc` · certain). Carrying a format the desktop accepts changes the cursor, but on drop a `.url` or downloaded file is created and the `none` decision of dragend breaks too (likely). The composition controller's drag API is for the receiving side only (likely · wry does not use visual hosting).
- **Remaining ways (all hacks · no precedent):**
  1. **Receiver window** — only during the drag, lay down a transparent · `WS_EX_NOACTIVATE` top-level window on each monitor with `RegisterDragDrop` and answer `DROPEFFECT_MOVE`. The cursor takes the move shape (likely). Cost = it must keep the z spot **right below our windows · above other apps** (if it sat above our windows it would cover our drops) · it intercepts drops over other apps (the same result as Q39, so not a problem) · the dragend effect becomes `move` instead of `none`, so the 「새 창」 signal has to move to the receiver window's Drop · the shell has to know the drag start (an exception to the 「끄는 동안 셸 호출 0」 (zero shell calls during a drag) of ADR-0296/0297).
  2. **Swapping with `SetSystemCursor(OCR_NO)`** — system-wide · if the app dies it stays until restored · even whether ole32 uses that system cursor is uncertain. Not recommended.
  3. **Drop the OS drag and use our own drag loop** (the Chrome · Windows Terminal shape — our old engine · ADR-0295) — clean, but it reverses ADR-0296.
- VS Code 1.85 solved it on macOS · Wayland by fixing Chromium (WebView2 has no such path). Which cursor VS Code shows over the desktop on Windows is unknown.
- **Awaiting judgment (user):** whether to measure the receiver window (1) with a spike or leave it as is.
