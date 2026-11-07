# How does one click on a star (favorite) connect to the collections I made (practice survey)

- **Status:** version with the adversarial review (FIX) applied · decision pending (an idea from before the preset PRD — the library slot's ★ and favorites collections)
- **Date:** 2026-10-14
- **Method:** research medium. 2 collection tracks (general apps · professional/developer tools) · main-session grounding · 1 cross-family adversarial review (below).
- **Confidence legend:** certain = primary source checked + independent cross-confirmation · likely = primary source checked (single source) or secondary source · uncertain = from memory.
- **Earlier survey:** `docs/research/library-multi-root-display-survey-2026-10-14.md` (shape of the library list).

## What is asked

The library slot has a ★ (default collection 「즐겨찾기」) and a favorites collection the user made (e.g. 「리뷰할 때」) side by side. What happens when ★ is pressed, and what is the practice for putting an item into the user's collection?

## Conclusions (top level)

1. **Among what was checked in primary sources, the most common shape = 「누르는 즉시 기본 목록에 넣고, 그 자리에서 목적지를 바꿀 길을 준다」.** **Certain** (three or more independent sources).
   - YouTube (Android): pressing Save puts the video **straight into the last-used playlist or Watch Later**, and the **「Change」** in the notification below chooses another playlist (official help). On desktop, a chooser window picks Watch later or my playlists.
   - Zotero Connector: pressing the save button saves, and a **popup showing which collection it goes into** lets you switch to another collection · library and add tags (official docs).
   - Spotify (2023): the heart and 「재생목록에 추가」 were **merged into a single +** — pressing saves and turns into a green check, and **pressing the check changes the destination.** The reason is 「더 직관적」 (quoted from a tech outlet's report of the official announcement · the reviewer pointed to the Spotify newsroom primary source).
2. **Immediate add + moving the 「기본」 status — JetBrains · Lightroom.** **Likely.**
   - JetBrains Bookmarks: F11 = straight into the default list named after the project. **A different list can be made the default.** To another list, drag to move or right-click 「Add Bookmark to Another List」. Files · folders · packages can also be bookmarked. The old Favorites was replaced by Bookmarks (primary source).
   - Lightroom Classic: the B key = adds to the Target Collection (default = the Quick Collection, which is temporary in nature). Any collection can become the destination with 「Set as Target Collection」 (search summary · official page not checked directly). The review pointed out that removing works only with the target collection open.
   - ★ However, neither is a precedent for 「즐겨찾기라는 이름의 묶음」 (a collection named favorites) — the default list is a list named after the project · a temporary collection (review finding).
3. **Star = a marker separate from lists.** Gmail star (independent of labels) · Notion Favorites (a personal sidebar section) · Figma Starred (found by the review · not checked by main) · Adobe Bridge (Favorites = pinned folders, Collections = groups of files — found by the review · not checked by main). **Likely.**
4. **A shape with only 「누르면 먼저 고르는 창」 has weak evidence.** Pinterest · Amazon are secondary sources · memory, and Chrome's star appears to add the bookmark at the moment of pressing and then choose a folder (see 1 above — review finding · unconfirmed). **Uncertain.**
5. ~~professional tool = immediate · general app = chooser window~~ — **this split in the draft is withdrawn.** Zotero (a research tool) shows a choosing popup after saving, and Apple Music (a general app) makes it a favorite immediately on pressing (review counter-evidence · Zotero checked by main in the primary source).

- **Can folders be added too:** JetBrains · Obsidian can (primary source). Adobe Bridge · Unreal separated 「폴더 즐겨찾기」 from 「파일 컬렉션」 (review · secondary source — uncertain). We found no document that states the reason.

## If moved to Engram (interpretation — the decision is the user's)

| Option | When ★ is pressed | How to put it into 「리뷰할 때」 | Resembles |
|---|---|---|---|
| **A. Add immediately + change the default** | straight into 「즐겨찾기」 (or the collection designated as default) · pressing again removes | right-click 「묶음에 넣기」 · drag and drop · right-click a collection 「★ 기본으로」 | JetBrains · Lightroom |
| **B. A chooser window first** | collection popover (with a favorites checkbox) | the same popover | YouTube desktop · Pinterest (weak evidence) |
| **C. Add immediately, and change on the spot** | straight into 「즐겨찾기」 + notification 「★ 즐겨찾기에 넣음 · 묶음 고르기」 · pressing the filled ★ opens the collection popover (removal is also here) | the same popover + right-click · drag | YouTube Android · Zotero · Spotify |

## grounding (main · checked against primary sources)

| Claim | Source | Verdict |
|---|---|---|
| JetBrains F11 = the default list named after the project · another list as default · drag / 「Add Bookmark to Another List」 · folder bookmarks · replaced Favorites | https://www.jetbrains.com/help/idea/bookmarks.html | supported |
| YouTube desktop Save = choose Watch later or my playlist · notification below | https://support.google.com/youtube/answer/57792 | supported (multi-select is not in the docs) |
| YouTube Android Save = straight into the last playlist or Watch Later · Change in the notification | https://support.google.com/youtube/answer/57792?co=GENIE.Platform%3DAndroid&hl=en | supported (found by the review · main checked) |
| Zotero Connector = saved-collection popup after saving · other collection · tags there | https://www.zotero.org/support/adding_items_to_zotero | supported (found by the review · main checked) |
| Spotify + = save with one tap · choose destination · green check · pressing the check changes the destination · reason 「더 직관적」 | https://www.gsmarena.com/spotify_is_killing_the_heart_icon_replacing_it_with_a_plus-news-57718.php · primary https://newsroom.spotify.com/2023-02-27/save-your-favorite-songs-podcasts-and-more-with-spotifys-plus-button/ | supported (secondary outlet checked · main did not open the newsroom primary source) |
| Lightroom Target Collection · Quick Collection · B key | https://helpx.adobe.com/lightroom-classic/desktop/organize-photos-in-lightroom-classic/photo-collections.html | collector search summary · reviewer quoted the primary source — not checked by main |
| Obsidian bookmark targets include folders · groups | https://obsidian.md/help/plugins/bookmarks | collector checked primary source · not rechecked by main |
| Notion ⭐ → sidebar Favorites | https://www.notion.com/help/navigate-with-the-sidebar | collector checked primary source · not rechecked by main |
| Raindrop belongs to one collection · default Unsorted | https://help.raindrop.io/collections | collector checked primary source · not rechecked by main |

## Adversarial review

codex (cross-family · effort high · level 2 · web search) once. Verdict **FIX**. Applied:

| Finding | Severity | Applied |
|---|---|---|
| The split 「전문 도구 = 즉시 · 일반 앱 = 고르는 창」 is wrong — Zotero shows a choosing popup after saving, Apple Music makes it a favorite immediately | high | withdrawn as conclusion 5. Zotero checked by main in the primary source · Apple Music (https://support.apple.com/guide/music/mark-items-as-favorites-musf33a29ebf/mac) unconfirmed |
| YouTube was put only under 「고르는 창」 — Android saves immediately + Change | high | main checked the primary source · moved to the representative case of conclusion 1 · put in C of the table |
| JetBrains · Lightroom are not precedents for 「즐겨찾기라는 묶음의 기본 자격이 옮겨 다니는」 | high | caveat added to conclusion 2 |
| Lightroom's B key was described as a general toggle — removing works only with the target collection open | medium | applied to conclusion 2 · marked unconfirmed |
| Chrome was grouped as chooser-window-first — the star adds first | medium | moved to conclusion 4 and treated as uncertain |
| Missing: Figma Starred · Adobe Bridge · Unreal Manage Collections (a menu of checked collections — not a star press) · Apple Music | — | added to conclusions 3 · 5 as the reviewer's claims (not checked by main) |

- No re-review after applying (medium single pass).

## Limits · gaps

- The **checkbox multi-select · immediate effect** of a chooser window was not confirmed in official docs for any app.
- Material on **user complaints · reversals** after the change (such as Spotify community threads) could not be opened.
- Not seen: Apple Photos · Firefox · Eagle · Blender · Unreal star behavior (body extraction failed).
