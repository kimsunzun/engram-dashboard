# 앱 안 휴지통은 어디서 여나 · 되살리기 · 비우기 (2026-10-20)

| 항목 | 값 |
|---|---|
| **상태** | 조사 완료 · 결정 대기(라이브러리 `.trash` 를 사람이 보고 되살리고 비우는 화면 — PRD R18 · TRD §6-5) |
| **방법** | `/research` light — 수집자 1(검색 스니펫만 · 원문 직접 읽음 없음) + 메인 grounding(Notion · Apple 사진 도움말 원문 확인) |
| **질문** | 앱이 관리하는 휴지통(OS 휴지통 아님)을 ① 어디서 여나(사이드바 고정 · 메뉴 · 없음) ② 되살리기 자리 · 이름 충돌 ③ 비우기 · 영구 삭제 확인 ④ 자동 삭제 |

## 결론

1. **사이드바 고정 항목이 관행이다**(가능성 높음 — 원문 둘 확인): Notion 「You'll see Trash at the bottom of your sidebar」 · Apple 사진 「Click Recently Deleted in the sidebar」 · Google Drive 왼쪽 내비 아래 Trash · Apple 메모 폴더 목록의 「최근 삭제된 항목」 · Joplin 3.0 노트북 목록의 Trash. **메뉴 안에만 둔 앱은 조사한 것 중 없었다.** 예외 = Notion 모바일(사이드바 `•••` 메뉴 안 — 공간 탓) · Obsidian(코어 화면 없음 — 탐색기나 플러그인 Trash Explorer 로 연다 · 불편 사례).
2. **되살리기 = 원래 자리로**(가능성 높음): Obsidian 플러그인은 원래 경로로 되돌리고 없는 상위 폴더를 새로 만든다 · Windows 휴지통은 이름이 겹치면 이름을 바꿔 되돌린다(스니펫만).
3. **자동 삭제 30일이 흔하다**(가능성 높음 — Notion · Apple 사진 원문 「30 days」 · Drive · 메모는 스니펫). 비우기 · 영구 삭제는 확인을 거친다(Drive 「Delete forever」).

## 사례

| 사례 | 여는 곳 | 되살리기 | 비우기 · 영구 삭제 | 자동 삭제 | 출처 | 확신도 |
|---|---|---|---|---|---|---|
| Notion | 사이드바 맨 아래 Trash(모바일 = `•••` 메뉴) | 열어서 restore · 검색 · 필터 | 열어서 permanently delete | 30일 | https://www.notion.com/help/duplicate-delete-and-restore-content | 확실(메인 원문 확인) |
| Apple 사진 | 사이드바 Recently Deleted(15.3 이하 = Utilities 안) | 고르고 Recover | — | 30일 | https://support.apple.com/en-us/guide/photos/delete-photos-videos-recover-deleted-pht3bfab2f96/10.0/mac/15.0 | 확실(메인 원문 확인) |
| Apple 메모 | 폴더 목록 「최근 삭제된 항목」 | Recover | — | 30일 | https://support.apple.com/en-us/guide/icloud/mm2f42f05cb9 | 가능성 높음(스니펫) |
| Google Drive | 왼쪽 내비 아래 Trash | — | Empty trash → 「Delete forever」 확인 | 30일 | 뉴스 기사(공식 문서 미확인) | 가능성 높음 |
| Joplin 3.0+ | 노트북 목록 Trash | 모름 | 우클릭 Empty trash | N일 옵션 | https://discourse.joplinapp.org/t/trash-can/34057 | 가능성 높음 |
| Obsidian | 코어 화면 없음(`.trash` 를 탐색기로 · 플러그인) | 플러그인 = 원래 경로 · 없는 폴더 생성 | 플러그인 | 코어 없음 | https://community.obsidian.md/plugins/obsidian-trash-explorer | 가능성 높음(스니펫) |
| Windows 휴지통 | 바탕화면 · 탐색기 고정 | 원래 자리 · 겹치면 이름 바꿈 | 비우기 확인 | 용량 기준(미검증) | 스니펫 | 가능성 높음 |

## 공백

- OneDrive · Dropbox · Figma · Logseq · Bear · Evernote · VS Code · Unity · Unreal · JetBrains Local History · macOS 「Put Back」 은 보지 않았다.
- 폴더를 되살릴 때 원래 상위 폴더가 사라졌으면 어떻게 하나 — Obsidian 플러그인(새로 만듦) 밖에는 확인 못 함.
- Engram 맥락: 비우기는 사람만이다(R18 · ADR 후보 28). 앱이 30일 뒤 스스로 비우는 것이 그 결정과 맞는지는 따로 정할 일이다.
