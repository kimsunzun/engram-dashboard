# If the default list cannot be deleted, can it not be renamed either (2026-10-20)

| Item | Value |
|---|---|
| **Status** | Survey done · **decided 2026-10-20** — 「기본」 cannot be renamed either (right-click 「이름 바꾸기」 greyed out · F2 does nothing · `collection.rename` rejected — user 「못바꾸기 고정해」 · PRD R16) |
| **Method** | `/research` light — 1 collector (4 searches · no direct check of original text) + main grounding (original text of Apple · Microsoft help confirmed) |
| **Question** | Can the user rename the undeletable default list that an app ships with from the start (the ★ default target) · if it is blocked, why · cases that allow it |

## Conclusion

1. **It could not be confirmed that 「못 지움 → 이름도 못 바꿈」 is the norm** (uncertain). The cases split in two.
2. **A system list whose name is itself the meaning of that list tends to have its name fixed** (low likelihood to uncertain — the sources are weak): Spotify's 「Liked Songs」 cannot be renamed and is shown under a name translated for each locale (a community post — a bug report that Siri cannot recognize the French 「Titres likés」). Chrome's 「Bookmarks bar」 · 「Other bookmarks」 cannot be deleted (help search summary) — whether they can be renamed could not be confirmed from the original text.
3. **There are designs where 「기본」 is not a name but a designated state** (likely — original text confirmed): Apple Reminders says 「You can choose a different default reminder list」 — any list can be designated as the default and changed. In this design there is no separate default list, so there is no reason to block the name.
4. **No UX guideline (NN/g · HIG) that states the reason was found.** The presumed reasons for locking the name = ① the name is itself a role marker ② it can be shown translated per locale — both are inferences.

## Cases

| Case | Delete | Rename | Source | Confidence |
|---|---|---|---|---|
| Spotify Liked Songs | No (presumed) | No | https://community.spotify.com/t5/iOS-iPhone-iPad/Siri-BUG-French-Liked-songs-playlist-called-quot-Titre-Lik%C3%A9s/m-p/5269708/highlight/true (community) | Uncertain |
| Chrome Bookmarks bar | No | Unknown | https://support.google.com/chrome/answer/188842 (search summary) | Delete = likely · name = unknown |
| Chrome Other bookmarks | No (hidden when empty) | Unknown | https://www.pcworld.com/article/477039/remove_the_other_bookmarks_folder_from_google_chrome.html | Uncertain |
| Apple Reminders default list | — | — | https://support.apple.com/guide/reminders/remn167d057f — 「choose a different default reminder list」 | Default = designated state: likely |
| Microsoft To Do 「Tasks」 | Unknown | Unknown | https://support.microsoft.com/en-us/office/use-and-customize-the-to-do-bar-4b6c5bb4-88a0-46c2-9159-6d0a0133ff79 | Uncertain |

## grounding

- The collector offered Apple Reminders and Microsoft To Do as counterexamples of 「기본 목록인데 이름을 바꿀 수 있는」. **When main opened the two original texts, neither said anything about renaming** (unsupported). All the Apple original supports is 「기본 목록을 다른 목록으로 바꿀 수 있다」 (conclusion 3). The Microsoft original covers only list deletion in general → removed from the counterexamples.

## Gaps

- Not confirmed from original text: Firefox · Edge favorites bar, Unreal Content Browser Favorites, Finder sidebar, Google · Apple Photos 「즐겨찾기」, YouTube 「나중에 볼 동영상」, Pinterest default board, Todoist Inbox, Instagram 「All posts」.
- Engram context: the target collection is already a designated state (one per PC · default value = 「기본」 — PRD R16). So the only special things about 「기본」 are two: 「못 지움 + 처음 대상」.
