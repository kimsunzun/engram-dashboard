# 변경 피드 동기화 관행 — 여러 클라이언트가 각자 따라잡는 법 (2026-10-30)

- **상태:** `/research` light — 수집자 1 · 메인 grounding 스팟체크(Microsoft Graph `driveItem: delta` 문서 원문 직접 확인) · 적대 리뷰 없음(light). 결정 대기 — S25 이동 기록(`library-moves.json` · M10)을 그대로 둘지.
- **배경:** 컬렉션은 UI 마다 따로 든다(G33 — 여러 UI 가 붙을 수 있다) · 라이브러리 파일은 평범한 텍스트라 고정 id 가 없다(G33). 지금 설계 = 데몬이 「옛 경로 → 새 경로」 를 번호 매겨 최근 1,000건 남기고(루트가 바뀌면 기록 id 가 바뀜) · UI 가 「커서 N 뒤로」 를 당겨 오고 · 잘렸으면 해당 줄은 「찾을 수 없음 · 다시 연결」.
- **확신도 범례:** 확실 = 1차 문서 직접 확인 · 가능성 높음 = 1차 문서 요약 · SDK 문서 · 포럼 · 모름 = 못 찾음.

## 결론

1. **조사한 피어 다섯 모두 같은 틀이다** — 서버는 클라이언트를 모르고, 클라이언트가 「어디까지 읽었나」 토큰을 스스로 들고 「그 뒤로 바뀐 것」 을 당겨 오며, 토큰이 너무 낡았으면 서버가 오류로 알리고 처음부터 다시 훑게 한다. **우리 설계와 큰 틀이 같다.**
2. **차이 하나 — 피어는 항목 id 기반이다.** 이름 바꾸기 · 옮기기는 이벤트가 아니라 「id X 의 최신 경로는 Y」 로 나온다(OneDrive · Drive). OneDrive 는 「항상 id 로 추적하라」 · 「폴더 이름을 바꿔도 하위 항목은 안 나온다」 고 적는다(확실). **우리는 id 가 없으니 「옛 경로 → 새 경로」 쌍으로 적을 수밖에 없다** — 같은 틀을 경로 세계로 옮긴 모양이다.
3. **차이 둘 — 잘린 뒤 복구.** 피어는 전체를 다시 훑고 로컬과 대조해 스스로 복구한다(id 가 있으니 맞춰 볼 수 있다). **id 가 없는 우리는 다시 훑어도 옛 경로가 어디로 갔는지 알 수 없어서 「찾을 수 없음 · 다시 연결」 이 남는 유일한 길이다** — 그리고 R19 가 이미 「훑기는 후보를 찾아 제안하지 않는다」 로 정했다.
4. **우리 「기록 id」(루트가 바뀌면 모든 커서 무효)에 해당하는 것** = OneDrive 「서버 상태가 바뀌어 새 토큰이 필요하면 410」 · Dropbox 「reset」(같은 커서를 다른 루트 기준으로 쓰는 것도 reset 원인). 이름 붙은 별도 칸은 없지만 역할은 같다.
5. 보관 기간 · 건수 상한을 공개한 피어는 없다 — 1,000건은 우리 선택이다.

## 피어별

| 피어 | 토큰 | 너무 낡은 토큰 | 옮기기 표현 | 전체 무효 신호 |
|---|---|---|---|---|
| **Microsoft Graph / OneDrive delta** | 불투명 토큰(deltaLink URL 안) · 클라이언트 보관 · `token=latest` (확실) | `HTTP 410 Gone` + 처음부터 다시 훑는 새 링크 · 다 훑은 뒤 로컬과 대조 — 오류 코드 둘(서버 것으로 바꿔라 / 로컬 차이를 올려라) (확실) | id 기반 · 항목별 최신 상태(두 번 바꿔도 한 번) · 폴더 이름 바꿔도 하위 안 나옴 · 「항상 id 로 추적」 (확실) | 「서버 상태가 바뀌어 새 토큰 필요」 도 410 (확실) |
| **Google Drive changes** | `getStartPageToken` → `changes.list` → `newStartPageToken` · 클라이언트 보관 (확실) | 모름 | `fileId` + 새 메타데이터 — id 기반 (가능성 높음) | 모름 |
| **Dropbox list_folder** | 불투명 cursor · `list_folder/continue` (가능성 높음) | `409 reset` → `list_folder` 다시 불러 새 cursor (가능성 높음) | 모름 | reset — 경로 루트를 바꿔 쓰는 것도 원인 (가능성 높음) |
| **CouchDB `_changes`** | `since=<seq>` · 2.0 부터 seq 는 불투명 · 클라이언트 보관 (확실) | 모름 | 이름 바꾸기 없음 — 문서 id 가 키라 삭제 + 새 문서 (확실) | 모름 |
| **Apple CloudKit** | 영역 단위 불투명 change token · 클라이언트 보관 (확실) | `changeTokenExpired` → 토큰 버리고 처음부터 (가능성 높음) | id 기반 · 옮기기 모름 | 모름 |

출처:
- Microsoft Graph — https://learn.microsoft.com/en-us/graph/api/driveitem-delta (메인 직접 확인 2026-10-30: 「the service returns an `HTTP 410 Gone` error … and a `Location` header containing a new nextLink that starts a fresh delta enumeration from scratch」 · 「The delta feed shows the latest state for each item, not each change」 · 「renaming a folder doesn't result in any descendants of the folder being returned from delta. When using delta you should always track items by id」 · 410 사유에 「if server state has changed and a new token is required」)
- Google Drive — https://developers.google.com/workspace/drive/api/reference/rest/v3/changes/list · https://developers.google.com/workspace/drive/api/guides/manage-changes
- Dropbox — https://dropbox.github.io/dropbox-sdk-java/api-docs/v4.0.0/com/dropbox/core/v2/files/ListFolderContinueError.html · https://www.dropboxforum.com/t5/Dropbox-API-Support-Feedback/409-reset-error-for-list-folder-continue-api-from-business/m-p/286174
- CouchDB — https://docs.couchdb.org/en/stable/api/database/changes.html
- CloudKit — https://developer.apple.com/documentation/cloudkit/ckfetchrecordzonechangesoperation

## 우리에게

- **이동 기록은 표준 변경 피드 틀을 id 없는 경로 세계로 옮긴 것이다** — 커서는 UI 가 들고 데몬은 UI 를 모른다 · 잘렸으면 신호를 준다(우리 `truncated` = 410 / reset 자리) · 루트가 바뀌면 기록 id 가 바뀐다(= 「서버 상태가 바뀌어 새 토큰 필요」).
- **피어와 달리 자동 복구가 안 된다** — id 가 없어서다. 그래서 잘린 뒤엔 「찾을 수 없음 · 다시 연결」(R19 와 같은 길).
- 다중 루트(G25)에선 기록 · 기록 id 가 루트마다 하나다.

## 공백 · 확인 못 한 것

- Drive 토큰 만료 동작 · 보관 기간, Dropbox 옮기기 표현 · 보관 기간, CouchDB 압축이 피드에 주는 영향 — 못 찾음.
- Syncthing · rsync · git 은 보지 않았다.
- 「작은 상한 + 연속 번호」 기록을 공개적으로 쓰는 피어는 확인 못 함 — 전부 id 기반 최신 상태 피드 또는 문서 단위 seq.
