# How a folder list follows changes on disk — watching vs. re-walking on actions

- **Status:** medium · design-decision mode · **decision (user 2026-10-14) = no watching · the client requests (pull) on its own actions · no periodic requests while visible either — 「화면 보고 있는 동안 바로 보여 줄 필요 없음」 (no need to show changes immediately while the screen is being looked at)** · adversarial review FIX → applied (「적대 리뷰」 (adversarial review) below) · no re-review after applying.
- **Date:** 2026-10-14
- **Method:** research medium. 3 collectors (① game-engine editors · asset managers ② IDEs · note apps · file managers ③ architectures where the server holds the files and the client attaches) · main grounding (source text · sources compared directly — table below) · one cross-family adversarial review (codex · effort high · level 2–3).
- **Starting point:** `docs/research/library-browser-building-blocks-survey-2026-10-14.md` — its sample (Spacedrive · Yazi · broot · Zed · watchexec · rust-analyzer · mdBook · Zola) all used watching, and the recommendation was 「notify + 다시 훑기 안전망」 (notify + a re-walk safety net). This survey widens the sample to see **whether there is a mature precedent that walks only on actions, without watching**.
- **Confidence legend:** certain = primary source + independent confirmation · likely = one primary source (main checked) · uncertain = only a collector · reviewer report · user posts · search summaries.

## Conclusions

1. **「언제 목록을 고치나(계기)」 (when is the list fixed (trigger)) and 「무엇으로 바뀐 걸 아나(감지)」 (how is a change known (detection)) go separately — game-engine editors tie the trigger to app focus.**
   - **Unity** — trigger = **when the editor regains focus** (Auto-Refresh on) · menu Refresh · `AssetDatabase.Refresh()`. Detection = on Windows, the 「Directory Monitoring」 setting can **watch directories instead of scanning all files** ("Monitors directories instead of scanning all project files to detect asset changes."). That is, watching is **a device that makes the walk at focus time cheap**, not a device that fixes the screen in real time (interpretation). **Likely** (source text of two manual pages — main checked). ★The draft's 「Unity 는 감시를 안 쓴다」 (Unity does not use watching) was wrong (found by the review)★.
   - **Godot** — trigger = **app focus** (`NOTIFICATION_APPLICATION_FOCUS_IN` → `EditorFileSystem::scan_changes()` · source). If the setting `interface/editor/behavior/import_resources_when_unfocused` is turned on, it (re)imports even when the window is out of focus · minimized — if off, "resources are only (re)imported when the editor window is focused". The default is not written in the XML. **Likely** (source · settings XML text — main checked). The reviewer's claim that this setting exists for CPU cost could not be confirmed in the source text.
   - **Total Commander** — it re-reads the directory every time you return to the window. FAQ text: "How do I keep Total Commander from re-reading the directories … each time I switch to Total Commander? With network drives this can take quite some time!" → a per-drive setting to turn it off (`Noreread=`) · manual with F2. **Likely** (official FAQ — main checked). That a watching option also exists separately is only from forum summaries (uncertain).
   - **Blender asset browser** — it has a manual 「Refresh File List」 button. That alone does not allow concluding there is no automatic detection (found by the review). **Uncertain**.
   - **Adobe Bridge** — user posts saying outside changes are not reflected automatically, so F5 is used. **Uncertain**.
   - **No vendor document stating 「왜 감시 대신 포커스 훑기인가」 (why focus walking instead of watching) was found.** The closest is Total Commander's 「네트워크 드라이브에서 다시 읽기가 느리다」 (re-reading is slow on network drives) (a cost-side reason).
2. **Even places that use watching narrow its scope or let it be turned off.**
   - **Unreal** — automatic re-import watching looks only at the 「Directories to Monitor」 the user wrote and is turned off wholesale with the single 「Monitor Content Directories」 (doc text — main checked). ★But this is a setting for **re-importing original source files** and is not the basis of the content-browser list★ — that the asset registry hooks directory-change callbacks separately was cited by the reviewer from a mirror repository's source and is not checked by main (**uncertain**).
   - **Eclipse** — 「Refresh using native hooks or polling」 · 「Refresh on access」 are both **Off by default** (current help — main checked · the description text was read truncated). **Likely**.
   - **VS Code** — watching (ParcelWatcher · `fs.watch`) + `files.watcherExclude`. In remote development, watching runs **on the target file system side** (reviewer · official wiki). **Uncertain** (not checked by main).
   - **Eagle** — watches only one designated import folder · advises avoiding LAN/NAS. **Uncertain** (collector).
3. **If watching is used, a re-walk safety net is kept alongside — watching is treated as 「최선 노력」 (best effort).**
   - **Syncthing** — watching on by default + a full walk every hour. Text: "Even with watcher enabled it is advised to keep regular full scans enabled, as it is possible that some changes aren't picked up by it." **Likely** (doc text — main checked).
   - **JetBrains IDE** — 「Sync external changes」 = re-reads changed files **when returning to the IDE window · when opening an editor tab** (the second option = at 15 seconds of inactivity · experimental). **Likely** (settings text — main checked). The relation to watching · the reason are only from forum summaries (uncertain).
   - **Obsidian** — watching only · staff answer "The file watcher doesn't work on network drives." → network drives unsupported (collector · **uncertain**). **Logseq** — watching + manual Refresh · Re-index (user forum · **uncertain**).
4. **Precedents for architectures where the server holds the data and the client attaches.**
   - **LSP** — watching is done by **one place, the client**, and the server asks only for the patterns it needs through **dynamic registration**. Text: "file system watching is not for free especially if the implementation uses some sort of polling and keeps a file system tree in memory to compare time stamps" · "If every server runs its own file system watching it can become a CPU or memory problem." → **a precedent for 「필요하다고 등록한 것만 감시」 (watch only what is registered as needed)**. **Likely** (spec text — main checked).
   - **JupyterLab file browser** — it **polls every 10 seconds while visible and stops when hidden** (`DEFAULT_REFRESH_INTERVAL = 10000` · `standby: 'when-hidden'` · backoff up to 5 minutes). It has the shape of 「보이는 화면만 주기적으로 다시 요청」 (re-request periodically only the visible screen) without watching. **Likely** (source — main checked). ★The draft's 「설정했을 때만 폴링」 (polls only when configured) was wrong — polling is the default (found by the review)★.
   - **Nextcloud notify_push** — server push is best effort: "updates might happen without a notification being sent and a notification can be sent even if no update has actually happened" · "Clients are advised to still perform periodic checks for updates on their own, although these can be run on a much lower frequency." **Likely** (README text — main checked).
   - **Theia** — it creates a watcher per workspace root and drops the watcher when that root is removed (watcher lifetime = lifetime of the open target). **Uncertain** (reviewer's source link · not checked by main).
   - No documents were found on catching up on missed notifications by sequence number · re-walking on reconnect · or a mobile-battery rationale.

## One-line pattern per track

| Kind | Trigger | Detection | Representative |
|---|---|---|---|
| Game-engine editors | **app focus** · button | walking, or watching (to make walking cheap) | Unity · Godot |
| IDEs | watching + **sync on returning to the window**, or watching off by default | watching · walking | JetBrains · Eclipse · VS Code |
| File managers | **re-read on window activation** (turned off for network drives) · watching option | re-reading · watching | Total Commander · Explorer |
| Note apps | watching | watching (+ manual) | Obsidian · Logseq |
| Server-client | watching only for what is registered · **polling only while visible** · push is a signal + low-frequency check | watching · polling | LSP · JupyterLab · Nextcloud · Syncthing |

## If carried over to Engram (session interpretation — the decision is the user's)

- **The common trigger across the precedents is 「앱 창 활성화」 (app window activation).** Because the moment of returning after editing outside (file explorer · agent shell) is right after an outside change (interpretation). The current provisional plan (slot open · slot focus · refresh · folder move · just before launch) is **missing Engram window activation**.
- **A gap the review pointed out — changes that happen without leaving the window.** Engram is a tool where agents on the same PC write files, so the library can change while a person is looking at Engram (interpretation · no frequency measured). The activation trigger alone cannot catch that case.
- Options:
  - **A. No watching · walk on activation** = the Godot · Total Commander model. No registration · no broadcast. Cost = outside changes made while looking are not visible until a refresh.
  - **A′. No watching · walk on activation + periodic requests while the slot is visible** = the JupyterLab model (the client requests every N seconds only while visible · stops when hidden). The daemon still answers only requests — no registration · broadcast · notify. Cost = walks of (number of visible slots × period) (the cost of walking thousands of entries has not been measured).
  - **B. Watch only while registered + re-walk safety net** = LSP registration + Theia lifetime + Syncthing safety net. Changes show immediately. Cost = register · unregister · sending to the registrant + notify (8.2.0 Windows overflow drop → safety net mandatory).
- **A → A′ → B are all additive** — the path 「요청하면 훑어 답한다」 (walk and answer when asked) is used as it is by all three.

## grounding (main source text · source comparison)

| Claim | Source | Verdict |
|---|---|---|
| Unity refreshes on focus return · Auto-Refresh | docs.unity3d.com Manual/AssetDatabaseRefreshing | supported |
| Unity Directory Monitoring (Windows) = watch directories instead of a full walk | docs.unity3d.com Manual/preferences-asset-pipeline | supported |
| Godot app focus → `scan_changes()` | `godotengine/godot` master `editor/editor_node.cpp` | supported |
| Godot `import_resources_when_unfocused` description | `godotengine/godot` master `doc/classes/EditorSettings.xml` | supported · CPU reason not supported |
| Total Commander re-reads on activation · turn off for network drives | ghisler.com/efaquser.htm | supported |
| JetBrains 「Sync external changes」 two options | jetbrains.com/help/idea/system-settings.html | supported |
| Eclipse two options Off by default | help.eclipse.org … ref-9.htm | supported (description truncated) |
| Syncthing watching + hourly full walk · safety-net advice | docs.syncthing.net/users/syncing | supported |
| LSP the client watches · cost reason | LSP 3.17 didChangeWatchedFiles text | supported |
| notify_push best effort · periodic-check advice | github.com/nextcloud/notify_push | supported · notification content not stated |
| JupyterLab default 10-second polling · stops when hidden | `jupyterlab/jupyterlab` main `packages/filebrowser/src/model.ts` | supported |
| Unreal automatic re-import watching = designated folders only · turn off | dev.epicgames.com automatic re-import doc | supported · unrelated to the content browser |
| Unreal asset registry directory callback · Theia watcher lifetime · VS Code remote watching | reviewer links | not checked by main |
| Blender · Obsidian · Logseq · Eagle · Bridge | collector reports | not checked by main |

## Adversarial review

codex (effort high · web search on) — verdict **FIX**. Findings and handling:

| Finding | Severity | Handling |
|---|---|---|
| Citing Unity as a 「감시 없음」 (no watching) precedent is an overstatement — Windows Directory Monitoring exists | major | main confirmed with the manual → conclusion 1 changed to 「계기 · 감지 분리」 (trigger · detection separated) |
| JupyterLab polls every 10 seconds by default, not only when configured | major | main confirmed with the source → fixed · reflected as option A′ |
| The Unreal auto re-import setting is not a basis for the content-browser list | major | accepted · the registry-callback claim is marked unchecked |
| Rebuttal of 「포커스 훑기 이유의 벤더 문서 없음」 (no vendor document for the focus-walk reason) — Godot · Total Commander | major | the Total Commander FAQ is confirmed (cost reason) · the Godot CPU reason was not found in the source text → partly accepted |
| A daemon-client watcher-lifetime peer (Theia) is missing | major | added (marked unchecked) |
| A logic gap in making client focus the freshness boundary — changes while looking | major | accepted · the gap and A′ added to 「Engram 에 옮기면」 (If carried over to Engram) |
| Blender: inferring no automatic detection from the button alone | minor | accepted · lowered to uncertain |

## Limits

- No document was found in which a vendor states 「왜 포커스 훑기인가」 (why focus walking) as a design reason (Total Commander's cost remark is the closest).
- Main did not check the actual update paths of the Unreal content-browser list · the VS Code explorer · macOS Finder.
- No documents were found on how remote · mobile clients handle missed notifications (sequence numbers · re-connect walk).
- The cost of walking the Engram library (thousands of entries) has not been measured — it is needed to set A′'s period · B's safety-net period.
- Candidates not checked: code-server · JetBrains Gateway · filebrowser · WebDAV sync-collection (RFC 6578) · Dropbox `list_folder/longpoll` · Lightroom 「폴더 동기화」 (folder sync) · Directory Opus · Sublime Text.
