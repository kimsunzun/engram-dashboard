# Window identification display · distinguishing the primary window — practice survey (2026-10-23)

> **Status:** survey done · adversarial review FIX applied · **decided (user decision 2026-10-23)** — a fixed marker showing only the number is always placed at the right end of the tab strip (immediately left of the window buttons) · numbers on all windows · main = 1 · OS title · taskbar · Alt+Tab = 「<탭 이름> - Engram:<n>」 (<tab name> - Engram:<n>) · distinguishing the main window is covered by number 1 + main ✕ (hide all windows) (not decorating only the main window separately is the main session's proposal) → PRD `docs/process/S21-window-chrome/prd.md` edition 0.4 R24 · R3 · Q3. The basis for T-66 (S26) PRD Q3 (where to show the number) · 「main 과 팝아웃 구분」 (distinguishing main from popout). **Method:** `/research medium` — two collection tracks (Claude) · main-session grounding (5 original-text checks — see 「메인 확인」 (main-session check) below) · 1 round of cross-family adversarial review (codex · FIX 9 items → applied). **Confidence:** certain = the main session checked the primary document · likely = a source supports it but it is single · a search summary · uncertain = weak support · unknown = not found. 「못 찾았다」 (not found) means it was not found in the sample, not a proof that it does not exist.

## Conclusion

1. **There is precedent for attaching a window number to the title bar.** For the second window of the same document, Excel always attaches `Book1.xls:2` to the title (a per-document window number — not an app-wide window number). iTerm2 puts the window number in the title through a setting and goes to that window with ⌘⌥N. Windows Terminal does not show it all the time, and shows the name · index as an overlay on request (`identifyWindow` · `identifyWindows`). — certain
2. **There is one precedent for placing a marker that tells windows apart at the left inside the tab strip — Vivaldi workspaces.** It puts a workspace button (icon + name · the name can be hidden) at the left of the tab strip, and a workspace is open in only one window at a time — in effect, that window's marker. But it is a name the user made, not a permanent window id. — certain
3. **We found no peer that always puts a numeric number badge in the tab strip.** WezTerm can build one with the tab-strip status area · the window id API (an inference that it is feasible — neither a default feature nor real-world use confirmed). — likely
4. **Window names mostly live on the OS side (title · taskbar · Alt+Tab)** — Chrome · Edge 「창 이름 지정」 (name window) shows in the taskbar · Alt+Tab · VS Code · JetBrains put the workspace · project name in the title · Notepad++ adds an identifying string to the title with `-titleAdd`. Edge workspaces create windows that have a name · theme. — likely
5. **We found no peer that tells the primary window apart by decorating it with a color · badge. Apps that have a primary window tell it apart by function · lifetime** — only the primary window has the full menu · global commands (JetBrains detached editor windows have no main menu · Unreal), and closing the primary window closes the other windows (Unreal · Blender child windows). In browsers · terminals all windows are equal. — likely
6. **Visual Studio's 「소유된 창」 (owned windows) are, by default, only tool windows** — under the default `Tool Windows`, tool windows are bound to the primary window (no taskbar button · minimized together · always on top) and document windows appear like independent windows. — certain
7. **We found no window number that appears and hides depending on the number of windows.** The only conditional display is the Chrome profile avatar (2 or more profiles). — likely

## By peer

| Peer | What · where · when | Source | Confidence |
|---|---|---|---|
| Excel | a new window of the same document = `Book1.xls:2` — 「window number」, 1 for most windows. Different from the index (position in the window collection) | https://learn.microsoft.com/en-us/office/vba/api/excel.window.windownumber (main-session checked) | certain |
| IronCAD INOVATE | tells windows of the same scene apart in the title as `My Project:1` · `:2` | https://www.ironcad.com/support/INOVATE_Help/Content/Understanding_the_Basics/Menus.htm (offered by the reviewer · not checked by the main session) | likely |
| iTerm2 | 「window titles include the window number. You can navigate to a window by pressing cmd-opt-N」 · neither the default nor number reuse is in the docs | https://iterm2.com/documentation-preferences-appearance.html (main-session checked) | certain (default unknown) |
| Windows Terminal | window id (integer) · name (optional) — `wt -w <id\|name>`. `identifyWindow` = name · index overlay on the focused window · `identifyWindows` = all windows (no default shortcut). No always-visible place found | https://learn.microsoft.com/en-us/windows/terminal/customize-settings/actions (main-session checked) | certain |
| macOS Terminal | the title options have 「Command key」 (the key that goes to that window) instead of the window number | https://support.apple.com/guide/terminal/trmlwindw/mac (search summary) | likely |
| Vivaldi | workspace button = left of the tab strip (vertical tab strip: top of the list) · name can be hidden (icon only) · 「A workspace can be open in one browser window at a time」 · title = active tab title · no window-naming feature | https://help.vivaldi.com/desktop/tabs/workspaces/ (main-session checked) · forum.vivaldi.net | certain |
| Chrome · Edge | 「창 이름 지정」 → taskbar · Alt+Tab (Mac = Window menu) · only when the user turns it on · unknown whether it shows in the tab strip · profile avatar only when there are 2 or more profiles · Edge workspace = a window that has a name · theme (reached from the upper-left menu) | androidcentral · microsoft.com/edge · https://support.microsoft.com/en-us/edge/getting-started-with-microsoft-edge-workspaces (offered by the reviewer · not checked by the main session) | likely |
| Firefox | no default window name · the extension API `titlePreface` prepends to the OS title | connect.mozilla.org (search summary) | likely |
| Notepad++ | the command line `-titleAdd` adds an identifying string to the title | npp-usermanual command-prompt (offered by the reviewer · not checked by the main session) | likely |
| WezTerm | tab-strip right status area (`set_right_status`) · window id lookup (`window_id`) — that a number badge can be built is only an inference | https://wezterm.org/config/lua/window/set_right_status.html (offered by the reviewer) | uncertain (not a default feature) |
| VS Code | `window.title` template (default `${activeEditorShort}${separator}${rootName}${separator}${appName}`) — identity = workspace name · no number · a secondary window is a light window attached to the parent instance | https://code.visualstudio.com/docs/configure/custom-layout | template certain · secondary window uncertain |
| JetBrains | title = project · file · a detached editor window has no main menu and 「원래 창을 닫으면 메뉴가 사라진다」 (the menu disappears when the original window is closed) | intellij-support forum answer (opening it directly gave 403 — search summary) | likely |
| Unreal Editor | the primary window's menu has the global items · closing the primary window closes all other windows · chrome details of floating tab windows unconfirmed | secondary docs (search summary) | likely |
| Visual Studio | 「Floating Windows」 setting — None · Tool Windows (default — owns only tool windows, document windows are independent) · Documents and Tool Windows. Owned window = no taskbar button · minimized together · always on top | https://devblogs.microsoft.com/visualstudio/take-full-control-of-your-floating-windows-in-visual-studio (main-session checked) | certain |
| Blender | a detached child window closes when the original window is closed · 「New Main Window」 is an independent parent | developer.blender.org T94346 (search summary) | likely |
| Dear ImGui | `ConfigViewportsNoDecoration` is a setting that applies to **secondary** viewports (not evidence that primary and secondary decoration are the same) | imgui.h docking (offered by the reviewer) | likely |

- **tmux · zellij were left out** — their 「window」 · 「tab」 correspond to a tab of a GUI app, so they cannot be counted as precedent for a top-level window number (review point).

## What this means for us (main-session interpretation — not peer fact)

- **Attaching the number to the title** = Excel (`:2`) · iTerm2 have precedent.
- **The shape of a window marker at the left of the tab strip** = the Vivaldi workspace button is the closest precedent (icon + hideable name). We found no precedent for a numeric badge itself.
- **Distinguishing main** is already split by function · lifetime, as with peers (closing main = hide all · global entry point) — whether to add decoration separately is the user's decision.

## Gaps

- How long the Windows Terminal overlay stays up · whether Chrome · Edge names appear in the tab strip · File Explorer tabs · iTerm2 number reuse · the VS Code secondary window's menu · title bar · Photoshop · Arc · Zed · Kitty · Ghostty.
- Of the counterexample URLs the reviewer offered, the INOVATE · Edge workspace · Notepad++ · WezTerm · ImGui header ones were not reopened by the main session.

## Review (codex · 2026-10-23 · FIX)

- 9 applied — conclusion 1 counterexamples (Excel · INOVATE) · Vivaldi omission (conclusion 2 added) · WezTerm limited to inference · Visual Studio ownership scope overstatement corrected (default = tool windows only) · ImGui citation misattribution corrected · tmux · zellij comparison-unit confusion → removed · overlay pattern limited to Windows Terminal alone · Edge workspace · Notepad++ added · 「선례 없음」 (no precedent) weakened to 「못 찾음」 (not found).
- Main-session grounding: the original texts of Windows Terminal · iTerm2 · Visual Studio · Vivaldi · Excel were checked.
