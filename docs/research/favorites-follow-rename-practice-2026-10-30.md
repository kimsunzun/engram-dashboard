# 즐겨찾기 · 컬렉션이 이름 바꾸기 · 옮기기를 따라가는 관행 (2026-10-30)

- **상태:** `/research` light — 수집자 1 · 메인 grounding 스팟체크(VS Code Bookmarks 관리자 답 직접 확인 · Blender 카탈로그 문서는 직접 열었으나 목차만 받아 미확인) · 적대 리뷰 없음(light). 결정 대기 — S25 이동 기록(`library-moves.json` · M10)을 둘지.
- **쓰임:** 셸이 저장하는 컬렉션이 데몬 안에서 일어난 옮기기 · 이름 바꾸기를 따라가게 하려고 데몬이 「옛 경로 → 새 경로」 기록(최근 1,000건 · 번호)을 남기고 셸이 「N 번 뒤로 바뀐 것」 을 당겨 오는 지금 설계가 관행에 맞나.
- **분류:** (A) 고정 id — 이름 · 위치가 바뀌어도 안 바뀌는 id 로 가리킨다 · (B) 바꾸는 쪽이 참조를 직접 고친다 · (C) 이동 기록 · 변경 피드를 다른 쪽이 따라 읽는다 · (D) 깨지고 사용자가 다시 잇는다 · (E) 그 밖.

## 결론

1. **관행은 (A) 고정 id 또는 (B) 바꾸는 쪽이 직접 고치기다.** OS 나 카탈로그가 id 를 주면 그것으로 가리키고(Lightroom · Eagle · Blender · macOS 북마크 · Windows 바로 가기), 참조가 바꾸는 쪽 손 안에 있으면 그 자리에서 고친다.
2. **(C) 「상한 있는 이동 기록을 다른 저장소가 당겨 읽기」 를 쓰는 피어는 못 찾았다.** 가장 가까운 것은 VS Code Bookmarks 확장이 같은 프로세스 안에서 이름 바꾸기 알림을 받아 고치는 것(밀기 · 기록 아님)이다.
3. **앱 밖(탐색기)에서 바꾼 것은 어디서나 대개 못 따라간다** — 예외는 파일 시스템 id 를 쓰는 OS 수준(macOS 북마크 · NTFS 링크 추적)뿐이고, 나머지는 (D) 다시 잇기로 떨어진다.
4. 그러니 지금 설계(셸이 컬렉션을 들고 데몬이 이동 기록을 남김)는 **드문 모양**이다. 정당화되는 조건은 「참조를 들고 있는 쪽(셸)이 바꾸는 쪽(데몬)과 떨어져 있고, 평범한 파일이라 고정 id 가 없다」 는 것 하나다 — 참조를 데몬 쪽으로 옮기면 (B) 가 되어 기록이 필요 없다.

## 피어별

| 피어 | 방식 | 앱 밖에서 바꾸면 | 확신도 |
|---|---|---|---|
| **Lightroom Classic 컬렉션** | (A) — 컬렉션은 경로가 아니라 카탈로그 사진 기록을 가리킨다 · 경로는 카탈로그가 따로 들고 앱 안 옮기기 때 고친다 | 「없음」 표시 → 「Find Missing Folder」 · 파일 이름까지 바뀌면 하나씩 다시 잇기 (D) | 가능성 높음 |
| **Eagle** | (A) — 항목마다 `<ITEM_ID>.info` 폴더 · 정체가 경로가 아니라 id · 폴더 소속은 메타데이터 | 모름 | 가능성 높음 |
| **Blender 에셋 카탈로그** | (A) — 에셋이 카탈로그 UUID 를 든다 · 카탈로그 이름 바꾸기 · 옮기기는 `blender_assets.cats.txt`(`{UUID}:{경로}:{이름}`) 한 파일만 고친다 | 카탈로그는 텍스트 파일 수정이라 안전 · 에셋 파일 경로 자체는 추적 안 함 | 가능성 높음 (수집자는 번역판 문서로 확인 · 메인 직접 확인 실패) |
| **macOS 북마크 데이터 · 별칭** | (A) + 대체 — 파일 시스템 id 로 옮기기 · 이름 바꾸기 뒤에도 찾는다 · 낡았다고 알리면 앱이 다시 저장 | 따라간다(id 가 파일 시스템에 있다) | 확실(북마크 데이터) |
| **Windows `.lnk` + 분산 링크 추적** | (A) + 추측 검색 — NTFS 객체 id + 볼륨 id · 실패하면 옛 경로 · 이름 · 생성 시각 · 크기로 몇 단계 뒤짐 | NTFS 면 따라간다 | 확실 |
| **VS Code Bookmarks 확장** | (E) 같은 프로세스 안 알림 — VS Code 의 이름 바꾸기로 하면 자동으로 고친다 | 「셸 · 다른 도구로 바꾸는 것은 지원 안 함」 | 확실 (관리자 답 — 메인 직접 확인) |
| **Obsidian 북마크** | 아마 (B) — `.obsidian/bookmarks.json` 에 경로 · 이름 바꾸기 때 고치는지 문서 못 찾음 | 모름 | 불확실 |
| **JetBrains 북마크** | 아마 (B) — `workspace/<id>.xml` 에 파일 URL · 이름 바꾸기 동작 못 찾음 | 모름 | 불확실 |

출처:
- Lightroom — https://blog.nobledesktop.com/learn/lightroom/relinking-missing-files-folders · https://lightroomkillertips.com/when-photos-go-missing/
- Eagle — https://api.eagle.cool/llms-full.txt
- Blender — https://docs.blender.org/manual/en/latest/files/asset_libraries/catalogs.html
- macOS — https://developer.apple.com/documentation/foundation/nsurl/bookmarkdata(options:includingresourcevaluesforkeys:relativeto:).md · https://indiestack.com/?p=514
- Windows — https://learn.microsoft.com/en-ie/Windows/Win32/fileio/distributed-link-tracking-and-object-identifiers · https://msdn.microsoft.com/it-it/windows/desktop/bb774952
- VS Code Bookmarks — https://github.com/alefragnani/vscode-bookmarks/discussions/593 (메인 직접 확인 2026-10-30: 「If you rename the file inside VS Code … the bookmarks will be automatically updated」 · 「Renaming the file outside VS Code, shell or other tools/commands, isn't supported」)
- Obsidian — https://forum.obsidian.md/t/new-bookmarks-should-overwrite-old-bookmarks-if-they-have-the-same-name/109648 (파일 위치만)
- JetBrains — https://intellij-support.jetbrains.com/hc/zh-cn/community/posts/360009712499/comments/360002241419 (저장 꼴만)

## 우리에게

| | (가) 지금 — 셸이 컬렉션 · 데몬이 이동 기록 (C) | (나) 컬렉션을 데몬이 든다 (B) | (다) 데몬이 고정 id 를 발급 (A) |
|---|---|---|---|
| 선례 | 못 찾음 | 프리셋 · 에이전트를 데몬이 고치는 것과 같은 꼴 · 관행 | Lightroom · Eagle 꼴 |
| 이동 기록 | 필요(최근 1,000건 · 넘치면 「찾을 수 없음」) | 필요 없음 | 필요 없음 — 대신 id → 경로 표를 데몬이 영속 |
| LLM | 컬렉션 명령은 셸이 붙어 있을 때만 닿는다 | 데몬 명령이라 UI 없이도 닿는다(G20 · G22 「LLM 이 기본」) | 컬렉션이 어디 사느냐에 따라 다름 |
| 앱 밖 변경 | 못 따라감 | 못 따라감 | 못 따라감(평범한 파일엔 id 가 없다) |

## 공백 · 확인 못 한 것

- Obsidian · JetBrains 북마크의 이름 바꾸기 동작 — 미확인.
- Eagle 의 앱 밖 옮기기 · 스마트 폴더 내부 — 못 찾음.
- Windows 빠른 실행 고정 · Finder 사이드바의 실제 장치 — 확인 안 함.
- 「커서 N 뒤로 바뀐 것」 을 상한 두고 내주는 동기화 · 카탈로그 도구 선례 — 못 찾음(git 의 이름 바꾸기 추정은 기록이 아니라 사후 유사도 판정 — 지식 기반, 검색 안 함).
