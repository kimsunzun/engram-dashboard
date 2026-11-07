# How to put library items into a bundle (preset) — UI practice survey

- **Status:** version with the adversarial review (FIX) applied · awaiting decision (a sketch from before the preset PRD — the main way to put components in during preset editing)
- **Date:** 2026-10-14
- **Method:** research medium. 1 collection track (UI practice) · main grounding · one cross-family adversarial review (below).
- **Confidence legend:** certain = source text checked + independent cross-confirmation · likely = source text checked (single source) or secondary source · uncertain = search summary · memory.
- **Collection limit:** the collector read only search summaries (0 page bodies). The 「원문 확인」 (source text checked) items below are only those main opened separately.

## What is asked

Three paths are already known — ① the edit field's 「+ 담기」 (add) search popover (the Unreal asset-field dropdown) ② pick in the library first, then 「← 선택한 것 담기」 (add what is selected) in the edit field (Unreal's 「콘텐츠 브라우저에서 선택한 에셋 쓰기」 (use the asset selected in the content browser) — confirmed by the user in Unreal) ③ drag and drop — what exists beyond these.

## Conclusions (top level)

0. **For an ordered bundle, 「맨 끝에 붙이기」 (append to the end) and 「고른 자리에 끼워 넣기」 (insert at the chosen position) split apart (found by the review · main checked the source text).** Final Cut Pro: select a clip in the browser, put the playhead where you want it, then Insert (W key) → the clips after that point are pushed back · Append puts it at the end. In a preset, order is the order in which instructions are concatenated, so this distinction applies directly. **Likely.**
0-1. **The Unreal asset-field control (source text · UE 4.27 doc)** = a dropdown (search · copy · paste · clear) · **Use Selected Asset** (puts in the asset selected in the content browser) · **Find in Content Browser** (finds and shows the asset put in) · Reset. Drag and drop is not in that doc. **Likely.**
1. **The documented paths in general apps = drag and drop + right-clicking an item for 「…에 추가」 (add to …) (which of them is mainly used is not in the docs — the 「기본 길」 wording was withdrawn on a review finding).** Apple Music: 「라이브러리 어디서든 항목을 사이드바의 재생목록으로 끌어 놓는다」 (back-translated: drag an item from anywhere in the library onto a playlist in the sidebar) · 「Control-클릭 → Add to Playlist → 재생목록 고르기」 (back-translated: Control-click → Add to Playlist → pick a playlist) · a new playlist has the same two paths (source text). **Likely** (single source text).
2. **A transfer list (left 「고를 것」 (to pick) ↔ right 「고른 것」 (picked) + arrow buttons)** is used when picking a part of a large pool and deciding while seeing the result side by side — mostly dialogs · settings screens. Ant Design Transfer = 「두 열 사이에서 요소를 옮기는 선택 제어」 (back-translated: a selection control that moves elements between two columns) · searchable per column · **reordering the right-hand list is not in the basic features** (source text). The claim that it is more accessible than drag and drop is from Smashing Magazine (summary only). → It overlaps the always-open library slot, and for a preset where order matters, reordering must be added separately. **Likely.**
3. **A target-designation mode (Lightroom Target Collection + the B key)** is an advanced-user path where the target is set once and then items are put in from the library with a single key. Its weakness is that it is forgotten if the target is not conspicuous (blogs · forums — not official docs). **Likely.** It has the same shape as the 「★ 대상」 (★ target) of our favorites bundle.
4. **A search popover · command palette (Notion `/` · VS Code Quick Pick)** is the keyboard path for when you know the name, and it is the same track as ①. **Certain** (mechanism · summary).
5. **One item → several bundles** is the Spotify-style checklist (right-click 「…에 추가」 (add to …)) — the same as our reverse-direction popover (library right-click 「프리셋에 추가」 (add to preset)). 「묶음 편집 중 라이브러리 줄마다 소속 체크박스」 (while editing a bundle, a membership checkbox on each library row) (Plex · iTunes etc.) **could not be confirmed**.
6. The eyedropper (Unreal actor picker) is only mentioned in an issue tracker and works only with a 3D view — for our list it is merely a variant of the target-designation mode. **Uncertain.** Apple Music's 「추천 곡 + 추가 단추」 (suggested songs + add button) is not in the source text, so it was left out.

## If carried over to Engram (interpretation — the decision is the user's)

| Path | Shape | Precedent | Main/auxiliary |
|---|---|---|---|
| ① Search popover | edit field 「+ 담기」 → search → check | Unreal dropdown · Quick Pick | auxiliary (decided) |
| ② ← add what is selected | pick in the library, then a button in the edit field | Unreal (user-confirmed) | main candidate |
| ③ Drag and drop | library row → a position in the edit field | Apple Music · Unreal (memory) | main candidate · putting in while deciding the order |
| ④ Library right-click 「편집 중인 프리셋에 담기」 | with the open preset edit field as the target, push in from the library side | Apple Music right-click 「Add to Playlist」 · Lightroom target | the opposite-side button to ② — the same job from the library side |
| ⑥ Insert at the chosen position | if a row is picked in the edit field, ② · ④ put in **after** that row, and at the very end if none is picked | Final Cut Pro Insert ↔ Append | the behavior rule of ② · ④ |
| ⑦ Paste | copy in the library → paste into the edit field | Unreal dropdown copy · paste | auxiliary |
| ⑤ Transfer list | two columns 「고를 것 ↔ 고른 것」 inside the edit screen | Ant Transfer | overlaps the library slot, so not recommended |

## grounding (main · source text comparison)

| Claim | Source | Verdict |
|---|---|---|
| Apple Music = drag onto a sidebar playlist · Control-click Add to Playlist | https://support.apple.com/en-us/guide/music/create-edit-and-delete-standard-playlists-musd5d051981/1.3/mac/13.0 | supported |
| Apple Music suggested songs + add button | same page | **not supported** — not in the source text → removed from the conclusions |
| Ant Transfer = two-column selection control · search per column · no reordering | https://ant.design/components/transfer | supported |
| Lightroom Target Collection · B key · Ctrl+Alt+Shift+B | https://mattk.com/target-collections-the-hidden-organizing-feature/ · the reviewer pointed to Adobe's source text https://helpx.adobe.com/nz/lightroom-classic/desktop/organize-photos-in-lightroom-classic/photo-collections.html | not checked by main |
| Final Cut Pro Insert (at the playhead · W) ↔ Append (at the end) | https://support.apple.com/guide/final-cut-pro/insert-clips-ver4e2eff6/mac | supported (found by the review · main checked) |
| Unreal asset field = dropdown (search · copy/paste) · Use Selected Asset · Find in Content Browser · Reset | https://dev.epicgames.com/documentation/unreal-engine/object-reference-control?application_version=4.27 | supported (UE 4.27 doc · no mention of drag and drop) |
| A transfer list has better accessibility · precision than drag and drop | https://smashingmagazine.com/2026/02/combobox-vs-multiselect-vs-listbox/ | collector summary — not checked by main |

## Adversarial review

codex (cross-family · effort high · level 2 · web search), once. Verdict **FIX**. Applied:

| Finding | Severity | Applied |
|---|---|---|
| 「고른 자리에 끼워 넣기」 (insert at the chosen position) missing — for an ordered bundle the append ↔ insert distinction is key (Final Cut Pro) | high | conclusion 0 · table ⑥ · main checked the source text |
| A transfer list cannot insert at a chosen position, so it is unsuitable as an ordered-preset model | high | table ⑤ 「비추천」 (not recommended) kept · evidence reinforced |
| Calling Apple Music evidence a 「기본 길」 (default path) is an overstatement — the docs have no usage ranking | medium | conclusion 1 wording withdrawn |
| Unreal Use Selected (put in) ↔ Find in Content Browser (find and show) distinction · copy/paste | medium | conclusion 0-1 · table ⑦ · main checked the source text |
| Notion `/` · Quick Pick are weak evidence for a search popover | low | kept (only the statement that they are the same track) |
| Omissions: Plex Add to Playlist after picking several · the Unreal Collections check menu · JetBrains 「다른 목록에 추가」 (add to another list) · the Figma Insert instance button | — | recorded as the reviewer's claim (not checked by main) |

- No re-review was run after the changes (medium single pass).

## Limits · gaps

- Not seen: the Figma Assets panel · the Unity object picker (a library inside the edit field) · copy · paste · a 「N개를 …에 추가」 (add N to …) bar after picking several · Plex/Jellyfin edit mode.
