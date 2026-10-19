# 지울 수 없는 기본 목록은 이름도 못 바꾸나 (2026-10-20)

| 항목 | 값 |
|---|---|
| **상태** | 조사 완료 · 결정 대기(즐겨찾기 첫 컬렉션 「기본」 의 이름을 바꿀 수 있게 하나 — PRD R16) |
| **방법** | `/research` light — 수집자 1(검색 4회 · 원문 직접 확인 없음) + 메인 grounding(Apple · Microsoft 도움말 원문 확인) |
| **질문** | 앱이 처음부터 싣는 지울 수 없는 기본 목록(★ 기본 대상)을 사용자가 이름 바꿀 수 있나 · 막는다면 이유 · 허용하는 사례 |

## 결론

1. **「못 지움 → 이름도 못 바꿈」 이 규범이라고 확인하지 못했다**(불확실). 사례가 둘로 갈린다.
2. **이름이 곧 그 목록의 뜻인 시스템 목록은 이름이 고정되는 쪽이다**(가능성 낮음~불확실 — 출처가 약하다): Spotify 「Liked Songs」 는 이름을 못 바꾸고 로케일마다 번역된 이름으로 보인다(커뮤니티 글 — 프랑스어 「Titres likés」 를 Siri 가 못 알아듣는 버그 보고). Chrome 「Bookmarks bar」 · 「Other bookmarks」 는 지울 수 없다(도움말 검색 요약) — 이름 변경 여부는 원문으로 확인 못 함.
3. **「기본」 이 이름이 아니라 지정 상태인 설계가 있다**(가능성 높음 — 원문 확인): Apple 미리 알림은 「You can choose a different default reminder list」 — 어느 목록이든 기본으로 지정하고 바꿀 수 있다. 이 설계에선 기본 목록이 따로 없으니 이름을 막을 까닭도 없다.
4. **이유를 밝힌 UX 지침(NN/g · HIG)은 찾지 못했다.** 이름을 잠그는 이유로 짐작되는 것 = ① 이름이 곧 역할 표시 ② 로케일마다 번역해 보여 줄 수 있다 — 둘 다 추론이다.

## 사례

| 사례 | 지움 | 이름 바꿈 | 출처 | 확신도 |
|---|---|---|---|---|
| Spotify Liked Songs | 불가(추정) | 불가 | https://community.spotify.com/t5/iOS-iPhone-iPad/Siri-BUG-French-Liked-songs-playlist-called-quot-Titre-Lik%C3%A9s/m-p/5269708/highlight/true (커뮤니티) | 불확실 |
| Chrome Bookmarks bar | 불가 | 모름 | https://support.google.com/chrome/answer/188842 (검색 요약) | 지움 = 가능성 높음 · 이름 = 모름 |
| Chrome Other bookmarks | 불가(비면 숨김) | 모름 | https://www.pcworld.com/article/477039/remove_the_other_bookmarks_folder_from_google_chrome.html | 불확실 |
| Apple 미리 알림 기본 목록 | — | — | https://support.apple.com/guide/reminders/remn167d057f — 「choose a different default reminder list」 | 기본 = 지정 상태: 가능성 높음 |
| Microsoft To Do 「Tasks」 | 모름 | 모름 | https://support.microsoft.com/en-us/office/use-and-customize-the-to-do-bar-4b6c5bb4-88a0-46c2-9159-6d0a0133ff79 | 불확실 |

## grounding

- 수집자는 Apple 미리 알림과 Microsoft To Do 를 「기본 목록인데 이름을 바꿀 수 있는」 반례로 냈다. **메인이 두 원문을 열어 보니 이름 바꾸기 이야기는 둘 다 없었다**(미지지). Apple 원문이 받치는 것은 「기본 목록을 다른 목록으로 바꿀 수 있다」 뿐이다(결론 3). Microsoft 원문은 목록 지우기 일반만 다룬다 → 반례에서 뺐다.

## 공백

- 원문을 확인하지 못한 것: Firefox · Edge 즐겨찾기 막대, Unreal Content Browser Favorites, Finder 사이드바, Google · Apple 사진 「즐겨찾기」, YouTube 「나중에 볼 동영상」, Pinterest 기본 보드, Todoist Inbox, Instagram 「All posts」.
- Engram 맥락: 대상 컬렉션은 이미 지정 상태다(PC 마다 하나 · 기본값 = 「기본」 — PRD R16). 그래서 「기본」 의 특별함은 「못 지움 + 처음 대상」 둘뿐이다.
