# Multi-Window Minimize Linkage — Peer Practice · Win32 Owned Windows · Tauri v2

- 상태: **research done · decision made → ADR-0229** (user decision 2026-09-24: minimize works per window on its own · the main X hides all windows to the tray · the tray icon revives all windows — options A–D below are all rejected, the reason is in the ADR) · Date: 2026-09-24
- Method: `/research` **medium** (design survey — the user called it light, but it was promoted by the design-survey rule) · main track collection 2 (peer practice · mechanism) + code-fact collection 1 · main grounding (「grounding」 below) · one cross-family adversarial review (「적대 리뷰」 below)
- Question (user, 2026-09-24): 「최소화했다가 아이콘을 누르면 복원되는데, 창을 여러 개로 분리했을 때는 메인 창만 적용된다. 메인 창이 최소화되면 다른 창도 다 최소화돼야 하지 않나.」
- Confidence legend: **certain** = primary source quoted + independently cross-confirmed · **likely** = one primary source or local source confirmed · **uncertain** = weak source · inference · **unknown** = no source

## Conclusion

- **There are two linkage means, and the means decides the behavior.**
  - **Owned window** (Win32 owner — Tauri `parent`/`owner`): when the main is minimized, **the OS** hides the popouts and revives them on restore. The price: a popout is **always above the main** and cannot go behind it. (certain)
  - **Independent window + event synchronization:** keep the current structure (independent top-level windows) as it is, detect the main's minimize and restore, and have the other windows follow. The z-order and taskbar buttons stay as they are now. Tauri has no minimize event, so detection and loop prevention must be written by hand. (likely)
- **Mature desktop IDEs offer both and let the user choose.** Visual Studio has a 3-level setting and its **default is 「도구 창은 메인 소유」** (= minimize together). JetBrains lets you choose per tool window between Float (together with the main) and Window (independent). (certain)
- **Web-family popouts (VS Code auxiliary windows · Chrome DevTools · Teams) are on the independent side.** VS Code had a complaint that 「보조 창이 메인 뒤로 가라앉는다」, and a per-window 「항상 위」 mode came in later — there is no source for a design judgment connecting the two (that ownership was considered and dropped). (likely)
- **We found no peer that states 「메인 복원 시 독립 팝아웃도 같이 복원」 (when the main is restored, independent popouts are restored too)** — the OS does it for owned windows, and independent-window peers have no documentation. No peer documentation covers the tray restore path either. (unknown)

## Current code (confirmed by reading — not run)

- **The tray is a single left click** (on button release), not a double click — `src-tauri/src/tray/mod.rs:111-120`. A comment at `:106` says it was changed from double to single at the user's request, and the 「좌클릭 더블=UI 열기」 comment at `:103` is stale.
- The restore function `show_main_ui` does `show → unminimize → set_focus` for **the main alone** (`src-tauri/src/tray/actions.rs:55-61`, the order is load-bearing because of Windows focus stealing — `:52`). The single-instance callback (`src-tauri/src/lib.rs:36-37`) and the `show_main_ui` command (`src-tauri/src/commands/tray.rs:10`) call the same function.
- A popout window is an **independent top-level window** — the builder at `src-tauri/src/commands/popout.rs:110-120` has no `parent`, `owner`, `skip_taskbar` or `always_on_top`. That is why each window has a taskbar button and can go behind the main.
- Window event handling is in one place only, `src-tauri/src/lib.rs:178-210` — main `CloseRequested` → `prevent_close` + `hide` (hide to the tray, ADR-0026), popup `Destroyed` → cleanup. **There is no minimize-detection code in the shell or on the screen.**
- **No behavior spreads from the main to other windows** — hiding the main (close · `hide_main_ui` · `--hidden` startup) also hides only the main.
- ADRs: no ADR decides cross-window minimize linkage or the popout's owner/taskbar setting. ADR-0023 (replaced by 0026) · 0026 · 0027 decide only close = hide to the tray and raising the existing window on relaunch.

## Platform facts

### Win32 owned windows (primary source — main checked against the source text)

[Window Features — Owned Windows · Visibility](https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features):

- 「An owned window is always above its owner in the z-order.」 · 「The system automatically destroys an owned window when its owner is destroyed.」 (certain)
- 「When an owner window is minimized, the system automatically hides the associated owned windows. Similarly, when an owner window is restored, the system automatically shows the associated owned windows.」 (certain)
- 「**Hiding an owner window has no effect on the visibility state of the owned windows.**」 — our main's 「닫기 = 트레이로 숨김」 does **not** hide the popouts even if we set an owner relation. (certain)
- Ownership transfer: the same page says it cannot be transferred after creation, while the `SetWindowLongPtr` doc says `GWLP_HWNDPARENT` sets a new owner, so **the docs contradict each other**. Our path specifies it at creation, so it does not step on this issue. (uncertain — irrelevant to us)

[The Taskbar](https://learn.microsoft.com/en-us/windows/win32/shell/taskbar): 「The Shell creates a button on the taskbar whenever an application creates a window that isn't owned.」 `WS_EX_APPWINDOW` forces a button and `WS_EX_TOOLWINDOW` removes it. (likely — collector quote, not checked by main)

### Tauri v2 / tao (local source — tauri 2.11.3 · tao 0.35.3, per `Cargo.lock`)

- `WebviewWindowBuilder::parent(&main)` **sets an owner on Windows** — the doc comment (`tauri-2.11.3/src/webview/webview_window.rs:617-631`) carries the three MSDN sentences above verbatim. `#[cfg(windows)] owner(...)` and `owner_raw(HWND)` are the same. Only `parent_raw(HWND)` is `WS_CHILD` (confined to the parent's client area), which is not our use. (likely — local source)
- When there is an owner, tao sets `WS_POPUP` and does **not** set `WS_EX_APPWINDOW` (`tao-0.35.3/src/platform_impl/windows/window.rs:1159-1164`, `window_state.rs:258-277` — main checked). **But at the end of every window creation it calls `set_skip_taskbar(false)` → `ITaskbarList::AddTab`** (`window.rs:1334`, `:1529-1535` — main checked). So **a Tauri-owned popout likely also has a taskbar button** — how that button looks while the main is minimized and what pressing it does has **not been measured**. `skip_taskbar(true)` can remove the button (Tauri maintainer's answer — [discussion #8574](https://github.com/orgs/tauri-apps/discussions/8574), collector quote). (uncertain)
- **There is no minimize event** — `WindowEvent` in `tauri-runtime-2.11.3/src/window.rs` has no Minimized/Restored. On Windows, minimize arrives as `Resized(0,0)` (+ `Moved(-32000,-32000)`) and is confirmed with `is_minimized()` (= `IsIconic`). (likely — local source + [tauri #7664](https://github.com/tauri-apps/tauri/issues/7664) collector quote)
- There is a report that when `minimize()` is called programmatically, a maximized window flashes once at its restored size and then minimizes — [tao #1338](https://github.com/tauri-apps/tao/issues/1338). A felt difference on the event-synchronization path. (uncertain — a single report)

## Peer practice (summary — detailed sources are in the collection results)

| Peer | When the main is minimized | Its own taskbar button | Can it go behind the main | Implementation | Confidence |
|---|---|---|---|---|---|
| Visual Studio — Floating Windows = **Tool Windows (default)** | minimize together | none | no (above while the IDE is active) | main-owned | certain ([MS Learn](https://learn.microsoft.com/en-us/visualstudio/ide/customizing-window-layouts-in-visual-studio) checked against the source text) |
| Visual Studio — Documents and Tool Windows | together | none | no | main-owned (including document windows) | certain (same doc) |
| Visual Studio — None | **stays visible** | yes | yes | independent | certain (same doc — the purpose is 「FancyZones 스냅·다른 앱 작업 중에도 보이게」) |
| JetBrains — Float | 「visible only together with the main project window」 | no documentation | no (above the main) | unknown (owner presumed) | likely ([JetBrains](https://www.jetbrains.com/help/idea/viewing-modes.html), collector quote) |
| JetBrains — Window | independent | no documentation | yes | unknown | likely |
| VS Code auxiliary windows | weak evidence: one report that a detached terminal window stays usable after the main is minimized ([#322582](https://github.com/microsoft/vscode/issues/322582) — quoted by the adversarial reviewer, not checked by main) | knowledge-based (yes) | yes — a per-window 「항상 위」 toggle (1.100) after the complaint issues | a separate Electron window | uncertain ([#204912](https://github.com/microsoft/vscode/issues/204912) · [#246694](https://github.com/microsoft/vscode/issues/246694) collector quotes) |
| Chrome DevTools detached | unknown | yes (search summary) | knowledge-based (yes) | a separate browser window | uncertain |
| Teams · Slack · Discord popouts | unknown / weak sources | mostly yes | unknown | unknown | uncertain |
| GIMP docks | a window-manager hint setting (Normal / Utility / Keep Above) | depends on the WM | no if Keep Above | WM hint | certain (docs) · WM-dependent |

## Options (checked against engram constraints)

engram constraints: the shell (backend) owns windows and tabs (ADR-0035/0057) · a new UI feature gets an LLM call path along with it (CLAUDE.md 「LLM-우선 제어」) · a popout is a runtime-created window (ADR-0057) · close = hide to the tray (ADR-0026).

| | A. Owned window (`parent(&main)`) | B. Independent + event synchronization | C. Keep as is (independent) | D. Let a setting choose (A or B + off) |
|---|---|---|---|---|
| Main minimized → popout | the OS hides it | the shell calls `minimize()` | unchanged | per the setting |
| Main restored (taskbar · tray) → popout | the OS revives it | the shell calls `unminimize()` | unchanged | per the setting |
| Popout goes behind the main | **cannot** (always above) | can (same as now) | can | per the setting |
| Popout taskbar entry ※ | likely (tao AddTab) — **not measured** · can be turned off with `skip_taskbar` | yes (same as now) | yes | per the setting |
| Minimizing only the popout | possible — without a button it drops to a small title-bar fragment (Electron #20059 report) | possible | possible | — |
| Main close (hide to the tray) → popout | **does not hide** (MSDN 「Hiding an owner … no effect」) — if wanted, [`ShowOwnedPopups`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-showownedpopups) can hide them together and revive them (reviving only the windows that function hid) | can be added to the same hook if wanted | does not hide | — |
| Amount of work | one builder line + measurement | window event detection (`Resized` + `is_minimized`) + loop prevention + restore order and focus rules + **a memory of 「동기화가 최소화한 창만 되살린다」 (only the windows that the synchronization minimized are revived)** (so that a popout the user minimized separately before the main is not unfolded by `unminimize()` — found by the adversarial review) | 0 | A/B + settings storage and a command |
| Risk | 「항상 위」 (always on top) covers the main on the same monitor · taskbar button behavior not measured | a maximized-window flash (tao #1338) · a minimize animation per window · a rule is needed for when only a popout is restored | the phenomenon the user complained about, unchanged | decisions double · you must decide **when a setting change takes effect on an already-open popout** — A sets the owner at creation, so to apply it immediately you must recreate the window or change the owner after creation (the latter steps on the 「소유권 이전」 (ownership transfer) doc conflict above) |
| Peers | VS default · JetBrains Float | no explicit precedent found | VS None · VS Code · Chrome | VS · JetBrains · GIMP |

- ※ **Separate the taskbar 「항목」 (entry) from the 「버튼」 (button).** Windows groups windows with the same AppUserModelID into one button ([AppUserModelID](https://learn.microsoft.com/en-us/windows/win32/properties/props-system-appusermodel-id) — quoted by the adversarial reviewer). So 「있음」 (yes) in the table above means each window has a taskbar entry (a preview thumbnail), not that it is a separate button — it depends on the grouping setting.
- **LLM path:** A, B and C are window behavior, not a new user action, so no command is needed. D creates a setting, so a command to change that setting must follow (「LLM-우선 제어」).
- **The agent-tree window** (a hidden static window) is slated to be removed by ADR-0225, so it is excluded from the targets under every option.

## Unconfirmed — resolve with a spike before deciding

- **A's taskbar behavior:** whether a Tauri-owned popout really has a button, whether that button disappears while the main is minimized, and what pressing it does. A 10-minute measurement resolves it (collector's recommendation).
- Whether there are side effects on rendering and xterm when the OS hides and shows a WebView2 window — not investigated.

## grounding (main external check)

- Supported: the four MSDN owned-window sentences (hide on minimize · show on restore · always above · destroyed together) + 「Hiding an owner … no effect」 — checked against the source text by grep. The Visual Studio 3-level setting and the default and the meaning of each option — checked against the source text. tao owner → `WS_POPUP` · `WS_EX_APPWINDOW` unset · `AddTab` call — checked against local source. Current-code facts — the code collector's `파일:줄` (main did not re-check the tray and popout builder lines).
- Unchecked (collector quotes only): the JetBrains wording · VS Code issues · the Taskbar doc · tauri #7664 · tao #1338 · discussion #8574 · Electron issues.

## Adversarial review

One cross-family run (GPT, `codex exec` effort high · web search) — decision **FIX 6**, all applied. No claim was overturned by counter-evidence (contested).

- Applied: ① a logic gap where B's restore also unfolds a popout the user minimized separately → added the rule 「동기화가 최소화한 창만」 ② D's undefined timing of applying to an already-open popout → stated in the table ③ `ShowOwnedPopups` was missing from A's tray-hide limit → added ④ one weak counter-evidence on the VS Code 「모름」 (unknown) → attached to the table (not checked by main) ⑤ taskbar entry and button conflated → added the ※ note ⑥ the overstated attribution that VS Code dropped ownership → removed.
- Reviewer grounding re-confirmed (supported): MSDN owned-window behavior · the Visual Studio 3-level setting and default · Tauri `parent` = owner (`tauri-2.11.3/src/window/mod.rs:640`) · the tao `AddTab` call (actual display not measured) · the absence of a minimize event (`tauri-runtime-2.11.3/src/window.rs:30`, tao `event_loop.rs:1227`).

## Additional research — popouts when the main is closed (hide to the tray) (2026-09-24, light)

The user proposed a new plan instead of minimize linkage — **minimize works per window on its own, the main X (hide to the tray) hides all windows, and the tray icon revives all windows.** We narrowly checked peer practice for that plan (collector 1 · main spot check).

- **Conclusion: the evidence is thin, and the two apps confirmed leave the popouts — yet on one of them a user is asking for the same behavior as the new plan.** There is one sourced precedent that revives all windows on a tray restore. Discord · Slack · Teams · Steam have no official docs (unknown).
- **OBS Studio** — even after sending it to the tray, the fullscreen projector window stays: 「OBS does indeed move itself over to System Tray, but the window for the Fullscreen Projector (Preview) is still on my task bar」 · what they want: 「move this window too down to System Tray along with OBS?」 · no reply, no setting. ([forum](https://obsproject.com/forum/threads/fullscreen-projector-preview-not-moving-to-system-tray-along-with-obs.169893/) — main checked against the source text · likely, a single user report)
- **Telegram Desktop** — closing the main leaves the detached chat windows (the repro steps close the remaining chat window after closing the main). A defect was also reported where closing that chat window brings the hidden main back up. ([tdesktop #27656](https://github.com/telegramdesktop/tdesktop/issues/27656) — collector quote · likely)
- **hermes-agent** (Electron, Windows) — a tray click revives **all hidden windows**: 「for each hidden window it calls `setSkipTaskbar(false)`, then `win.restore()` if `isMinimized()`, then `win.showInactive()`」 → then focuses the main. The issue does not say what the main X does to the other windows. ([#119243](https://github.com/NousResearch/hermes-agent/issues/119243) — main checked against the source text · likely)
- **Microsoft Teams** — X = tray is the default, but the docs do not say what happens to popout windows. There is a complaint that **minimizing** the main makes the meeting pop out into a separate window that stays ([Q&A](https://learn.microsoft.com/en-us/answers/questions/4430717/how-to-disable-teams-meeting-in-pop-out-window-whe) — collector quote · likely).
- Electron's tray tutorial covers only hide/show of a single window — there is no official 「모든 창 숨김」 (hide all windows) pattern (collector · uncertain).
- **Effect on the judgment:** the new plan is behavior that most peers **do not** do, but the OBS user's request and hermes-agent's tray restore point the same way. We found no ground for rejecting it (「그렇게 하면 문제가 났다」, doing it caused a problem).
