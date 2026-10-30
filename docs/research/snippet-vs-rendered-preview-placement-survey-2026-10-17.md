# ★ 글자 발췌와 그린 미리보기를 어디에 두나 — light 조사

| 항목 | 값 |
|---|---|
| 상태 | 조사 완료 · 결정 = 카드는 미리보기만 · 발췌는 한 줄 자리(목록 · 검색 결과 · 담기 검색)에서만(관행 따름 — 사용자 2026-10-17 「관행 찾아봐」) |
| 날짜 | 2026-10-17 |
| 방법 | `/research` light — 수집자 1명(검색 약 8회 · 가져오기 없음) · 메인 grounding 스팟체크 1건(Outlook 도움말 WebFetch) · 적대 리뷰 없음 |
| 질문 | 지시서 상세 카드에 글자 발췌(2줄 …)와 그린 마크다운 미리보기를 함께 두면 같은 앞부분이 두 번 나온다 — 다른 곳은 둘을 어디에 두나 |

## 결론

- **발췌(1~3줄 평문)는 목록 · 결과 줄의 보조 글이고, 내용은 따로 된 미리보기 · 읽기 칸이 보여 준다.** Outlook = 목록 줄 「Message Preview」 1~3줄 + 읽기 칸에 메시지 전체 — 원문: 「Message Preview, where you can choose to display between one and three lines of each message in the message list」 · 「When you select a message in the message list, you can see the message in the Reading Pane」. (가능성 높음 — 메인 원문 확인 · 단일 출처)
- **한 칸에 발췌와 그린 내용을 함께 두는 예는 못 찾았다** — 못 찾음 ≠ 없음. (불확실)
- 탐색기는 세부 정보 칸(메타데이터)과 미리보기 칸(내용)을 둘 중 하나만 띄운다 · Material 목록의 「supporting text」 = 선택 보조 줄(최대 1~3줄). (가능성 높음 — 2차 요약)

## 발견

| 앱 | 발췌 자리 | 그린 내용 자리 | 한 곳에 둘? | 출처 | 확신 |
|---|---|---|---|---|---|
| Outlook | 목록 줄 1~3줄(기본 1) | 읽기 칸 | 근거 없음 | https://support.microsoft.com/outlook/use-and-configure-the-reading-pane-to-preview-messages-in-outlook (메인 확인) | 가능성 높음 |
| 탐색기 | 세부 정보 칸 = 메타데이터만 | 미리보기 칸 | 아님 — 한 번에 하나 | https://learn.microsoft.com/en-us/answers/a/7363238 | 가능성 높음(2차) |
| Finder · Spotlight | 결과 줄 = 이름 + 메타데이터 | Quick Look(Space) | 근거 없음 | https://sealsystems.net/go/preview-spotlight-results-in-ventura-using-quick-look/ | 가능성 높음(2차) |
| Obsidian | 검색 결과 = 맞은 곳 앞뒤 몇 줄 | 커뮤니티 Quick Preview(Alt 누름) — 따로 하는 동작 | 아님 | https://community.obsidian.md/plugins/quick-preview | 불확실 |
| 언리얼 전역 에셋 고르기 | 설명 줄 없음 · 올리면 창 아래 정보 | 썸네일 | 아님 | https://docs.unrealengine.com/4.26/en-US/Basics/UI/GlobalAssetPicker | 가능성 높음 |
| Material 목록 | supporting text 1~3줄(선택) | 해당 없음 | — | https://m2.material.io/go/design-lists | 가능성 높음(검색 요약) |
| Apple Notes · Bear · Gmail · VS Code · Raycast · Notion · Unity · HIG · Fluent | — | — | — | — | 기권 |

## 우리 설계에 주는 것

- 지시서 상세 카드(오른쪽 칸 · 편집 칸 👁 카드) = 그린 미리보기만.
- 발췌 = 한 줄 보조 글이 붙는 자리에서만 — 프리셋 「+ 담기」 검색 결과 줄의 「이름 · 한 줄」(스킬 = `SKILL.md` 설명 · 지시서 = 발췌). 발췌 만들기 규칙은 `docs/research/markdown-plaintext-snippet-survey-2026-10-17.md` 그대로.

## 공백

- 가져온 1차 페이지는 Outlook 하나 — 나머지는 검색 요약.
- 반례(한 칸에 둘)를 적극적으로 찾지는 않았다.
