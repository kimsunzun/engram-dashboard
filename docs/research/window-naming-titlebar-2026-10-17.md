# Window name · window number · title bar — practice survey (2026-10-17)

- **Status:** survey complete · some user decisions made (「사용자 결정」 (User decisions) below) · main-session proposal not finalized
- **Method:** research skill, light — 1 collection track (public docs · forums) → main-session grounding (reopened the lines of our code `popout.rs` · checked some sources). ★No adversarial review was done★ — read the confidence labels below with that premise.
- **Confidence legend:** certain = two or more independent sources, or code · official docs checked directly · likely = supported by a single source · uncertain = weak basis · unknown = not found
- **Trigger:** the user was bothered that the popout window title showed as 「Engram — slot-popup-11」 and the popout's new tab name as a number that keeps growing, like 「Popup 6」. The question = 「다른 앱은 창 · 탭 이름과 번호를 어떻게 다루나」 (how do other apps handle window · tab names and numbers).

## Conclusion

1. **Separating the internal id from the name shown to people is the norm.** We found no mainstream app in the sample that shows an ever-growing global number to people as is. — likely
2. **For the visible number, reusing 「비어 있는 가장 작은 번호」 (the smallest free number) has precedent** (tmux window index default · Apple HIG's 「Untitled 2」). — likely
3. **A window title is 「내용 먼저, 앱 이름 뒤」 (content first, app name after) or drops the app name** (Chrome · VS Code · Obsidian). Microsoft's guidance does not prescribe an order or separator. — likely
4. **In Tauri the title can be changed independently of the label** (`setTitle()`). So the path of keeping the label monotonic and changing only the visible name is not blocked. — certain
5. **The Chrome · Windows 11 Explorer style 「탭 줄이 제목 표시줄 자리」 (the tab strip takes the place of the title bar) has to be drawn directly in Tauri** (`decorations: false` + drag region + window buttons). The Windows 11 snap layouts have to be handled separately. — likely

## 1. Our facts (code)

| Item | Content | Basis |
|---|---|---|
| Window label | monotonic counter `PopupCounter` · prefix `slot-popup-` — paired with the `"slot-popup-*"` glob in capabilities | `src-tauri/src/commands/popout.rs:35-55` |
| Label reuse is forbidden | according to the code comment, building again with a closed label produces an error, so labels are not reused — ★only the comment was read, not checked directly★ | the `PopupCounter` comment in `popout.rs` |
| Window title | `Engram — {label}` | `popout.rs:231` |
| Popout new tab name | 「Popup N」, from the number at the end of the label | `tab_name` at `popout.rs:58-61` |
| Renaming | tab = can be renamed (screen · `tab.rename`) · window = cannot | — |
| LLM's window addressing | gets a list with `window.list` and points to a window by label | — |

## 2. Practice

| # | Content | Source | Confidence |
|---|---|---|---|
| ① | tmux: each window has three separate things — a unique id (`@N`) · a position number (index — default = the smallest free number, renumbered to close gaps with `renumber-windows`) · a name (`rename-window`). Targeting accepts all three | https://mintlify.wiki/tmux/tmux/concepts/windows | likely |
| ② | a mainstream app that shows an ever-growing global number to people | not found in the sample | unknown (not a proof of absence) |
| ③-a | Chrome window title = 「<활성 탭> - Google Chrome」 (<active tab> - Google Chrome) | knowledge base | likely |
| ③-b | VS Code `window.title` default template = `${dirty}${activeEditorShort}${separator}${rootName}${separator}${profileName}${separator}${appName}` | VS Code settings docs | likely |
| ③-c | Obsidian title = note name → vault → app | https://forum.obsidian.md/t/option-to-set-window-title-format/46303 | likely |
| ③-d | Apple HIG: when titles are the same, a numeric suffix from 2 (「Untitled 2」) | https://developer.apple.com/design/human-interface-guidelines/windows | certain |
| ③-e | Microsoft's title bar guidance does not prescribe an order or separator | https://learn.microsoft.com/en-us/windows/apps/design/basics/titlebar-design | certain |
| ④ | Chrome has a window-naming feature (「Name Window…」) | https://til.codeinthehole.com/posts/about-chromes-named-windows/ | likely |
| ⑤ | Tauri: `setTitle()` sets the title independently of the label · the label is a "unique identifier" | https://v2.tauri.app/reference/javascript/api/namespacewindow/ | certain · whether a closed label can be reused is unknown |
| ⑥ | Chrome · the Windows 11 Explorer · Windows Terminal put the tab strip in the place of the OS title bar | user observation + knowledge base | likely |
| ⑥-Tauri | `decorations: false` + drag region (`data-tauri-drag-region`) + drawing the window buttons directly · the snap layout of the Windows 11 maximize button has to be handled separately · the community plugin `tauri-plugin-decorum` is known to handle this | knowledge base | likely · plugin unconfirmed |

## 3. User decisions (2026-10-17)

- **The visible window number = reuse the smallest free number among the live windows** — with 1 · 2 · 3, if 2 is closed, the new window = 2.
- **The internal label stays monotonic.**
- **Chrome · Explorer-style title bar** — the tab strip is at the very top · an empty area = window drag · the window buttons are drawn directly.
- **Tab-bar overflow = exactly like Chrome** — no scrolling, it shrinks down to the minimum width, and if it still overflows, the tabs are not visible.
- **Timing = right after slot docking 9c · included in the final merge.**

## 4. Main-session proposal (not finalized)

- Window title = 「<활성 탭 이름> - Engram」 (<active tab name> - Engram), and with no tab 「Engram 2」 (the visible number).
- Popout new tab name = the agent name of the slot that was moved, and 「새 탭」 (new tab) if empty.
- `window.list` gives the visible number and the label together and the command accepts both — the number is for interpreting human speech (「2번 창」 (window 2)), and what the agent remembers is the label (since numbers are reused).

## 5. Gaps · unconfirmed

- VS Code's 「Untitled-N」 number reuse policy
- zellij's tab · session numbering rules
- Whether a window can be created again with a closed label in Tauri (our code comment says 「에러」 (error) but we did not check)
- The exact behavior in Chrome when tabs overflow (minimum width · handling of tabs that are not visible)
