# 폴더 목록이 디스크 변경을 어떻게 따라가나 — 감시 대 액션 때 다시 훑기

- **상태:** medium · 설계-결정 모드 · **결정(사용자 2026-10-14) = 감시 없음 · 클라가 자기 동작 때 요청(pull) · 보이는 동안 주기 요청도 안 함 — 「화면 보고 있는 동안 바로 보여 줄 필요 없음」** · 적대 리뷰 FIX → 반영(아래 「적대 리뷰」) · 반영 뒤 재리뷰 없음.
- **날짜:** 2026-10-14
- **방법:** research medium. 수집자 3(① 게임 엔진 에디터 · 에셋 관리자 ② IDE · 노트 앱 · 파일 관리자 ③ 서버가 파일을 갖고 클라가 붙는 구조) · 메인 grounding(원문 · 소스 직접 대조 — 아래 표) · cross-family 적대 리뷰 1회(codex · effort high · 레벨 2~3).
- **출발점:** `docs/research/library-browser-building-blocks-survey-2026-10-14.md` — 그 표본(Spacedrive · Yazi · broot · Zed · watchexec · rust-analyzer · mdBook · Zola)은 전부 감시를 썼고 권고는 「notify + 다시 훑기 안전망」이었다. 이 조사는 표본을 넓혀 **감시 없이 액션 때만 훑는 성숙한 선례가 있나**를 본다.
- **확신도 범례:** 확실 = 1차 자료 + 독립 확인 · 가능성 높음 = 1차 자료 하나(메인 확인) · 불확실 = 수집자 · 리뷰어 보고만 · 사용자 글 · 검색 요약.

## 결론

1. **「언제 목록을 고치나(계기)」와 「무엇으로 바뀐 걸 아나(감지)」는 따로 간다 — 게임 엔진 에디터는 계기를 앱 포커스에 묶는다.**
   - **Unity** — 계기 = 에디터가 **포커스를 다시 얻을 때**(Auto-Refresh 켜짐) · 메뉴 Refresh · `AssetDatabase.Refresh()`. 감지 = Windows 에서는 「Directory Monitoring」 설정으로 **전체 파일을 훑는 대신 디렉터리를 감시**할 수 있다("Monitors directories instead of scanning all project files to detect asset changes."). 즉 감시는 **포커스 때 훑기를 싸게 만드는 장치**이지 화면을 실시간으로 고치는 장치가 아니다(해석). **가능성 높음**(매뉴얼 두 쪽 원문 — 메인 확인). ★초안의 「Unity 는 감시를 안 쓴다」는 틀렸다(리뷰 적출)★.
   - **Godot** — 계기 = **앱 포커스**(`NOTIFICATION_APPLICATION_FOCUS_IN` → `EditorFileSystem::scan_changes()` · 소스). 설정 `interface/editor/behavior/import_resources_when_unfocused` 를 켜면 창이 포커스 밖 · 최소화여도 (재)임포트한다 — 끄면 "resources are only (re)imported when the editor window is focused". 기본값은 XML 에 안 적혀 있다. **가능성 높음**(소스 · 설정 XML 원문 — 메인 확인). 이 설정이 CPU 비용 때문이라는 리뷰어 주장은 원문에서 확인 못 했다.
   - **Total Commander** — 창으로 돌아올 때마다 디렉터리를 다시 읽는다. FAQ 원문: "How do I keep Total Commander from re-reading the directories … each time I switch to Total Commander? With network drives this can take quite some time!" → 드라이브별로 끄는 설정(`Noreread=`) · F2 로 수동. **가능성 높음**(공식 FAQ — 메인 확인). 감시 옵션도 따로 있다는 것은 포럼 요약뿐(불확실).
   - **Blender 에셋 브라우저** — 수동 「Refresh File List」 버튼이 있다. 그것만으로 자동 감지가 없다고 볼 수는 없다(리뷰 적출). **불확실**.
   - **Adobe Bridge** — 바깥 변경이 자동 반영되지 않아 F5 를 쓴다는 사용자 글. **불확실**.
   - **「왜 감시 대신 포커스 훑기인가」를 벤더가 적은 문서는 못 찾았다.** 가장 가까운 것은 Total Commander 의 「네트워크 드라이브에서 다시 읽기가 느리다」(비용 쪽 이유)다.
2. **감시를 쓰는 곳도 범위를 좁히거나 끌 수 있게 둔다.**
   - **Unreal** — 자동 재임포트 감시는 사용자가 적은 「Directories to Monitor」만 보고 「Monitor Content Directories」 하나로 통째로 끈다(문서 원문 — 메인 확인). ★단 이것은 **원본 소스 파일 재임포트** 설정이고 콘텐츠 브라우저 목록의 근거가 아니다★ — 에셋 레지스트리가 디렉터리 변경 콜백을 따로 건다는 것은 리뷰어가 미러 저장소 소스로 든 것이라 메인 미확인(**불확실**).
   - **Eclipse** — 「Refresh using native hooks or polling」 · 「Refresh on access」 둘 다 **기본 Off**(현행 도움말 — 메인 확인 · 설명문은 잘려 읽었다). **가능성 높음**.
   - **VS Code** — 감시(ParcelWatcher · `fs.watch`) + `files.watcherExclude`. 원격 개발에서는 **대상 파일 시스템 쪽에서** 감시가 돈다(리뷰어 · 공식 wiki). **불확실**(메인 미확인).
   - **Eagle** — 지정한 들여오기 폴더 하나만 감시 · LAN/NAS 는 피하라고 적는다. **불확실**(수집자).
3. **감시를 쓰면 다시 훑기 안전망을 같이 둔다 — 감시는 「최선 노력」으로 취급된다.**
   - **Syncthing** — 기본 감시 켜짐 + 한 시간마다 전체 훑기. 원문: "Even with watcher enabled it is advised to keep regular full scans enabled, as it is possible that some changes aren't picked up by it." **가능성 높음**(문서 원문 — 메인 확인).
   - **JetBrains IDE** — 「Sync external changes」 = **IDE 창으로 돌아올 때 · 에디터 탭을 열 때** 바뀐 파일을 다시 읽는다(둘째 옵션 = 15초 비활성 때 · 실험). **가능성 높음**(설정 원문 — 메인 확인). 감시와의 관계 · 이유는 포럼 요약뿐(불확실).
   - **Obsidian** — 감시만 · 직원 답 "The file watcher doesn't work on network drives." → 네트워크 드라이브 미지원(수집자 · **불확실**). **Logseq** — 감시 + 수동 Refresh · Re-index(사용자 포럼 · **불확실**).
4. **서버가 데이터를 갖고 클라가 붙는 구조의 선례.**
   - **LSP** — 감시는 **클라 한 곳**이 하고, 서버는 필요한 패턴만 **동적 등록**으로 요청한다. 원문: "file system watching is not for free especially if the implementation uses some sort of polling and keeps a file system tree in memory to compare time stamps" · "If every server runs its own file system watching it can become a CPU or memory problem." → **「필요하다고 등록한 것만 감시」의 선례**. **가능성 높음**(규격 원문 — 메인 확인).
   - **JupyterLab 파일 브라우저** — **보이는 동안 10초마다 폴링하고 숨겨지면 멈춘다**(`DEFAULT_REFRESH_INTERVAL = 10000` · `standby: 'when-hidden'` · 백오프 최대 5분). 감시 없이 「보이는 화면만 주기적으로 다시 요청」하는 모양이다. **가능성 높음**(소스 — 메인 확인). ★초안의 「설정했을 때만 폴링」은 틀렸다 — 기본이 폴링이다(리뷰 적출)★.
   - **Nextcloud notify_push** — 서버 푸시는 최선 노력: "updates might happen without a notification being sent and a notification can be sent even if no update has actually happened" · "Clients are advised to still perform periodic checks for updates on their own, although these can be run on a much lower frequency." **가능성 높음**(README 원문 — 메인 확인).
   - **Theia** — 작업 공간 루트마다 감시를 만들고 그 루트가 빠지면 감시를 버린다(감시 수명 = 열린 대상의 수명). **불확실**(리뷰어 소스 링크 · 메인 미확인).
   - 놓친 알림을 번호로 따라잡기 · 재접속 때 다시 훑기 · 모바일 배터리 근거의 문서는 못 찾았다.

## 갈래별 패턴 한 줄

| 부류 | 계기 | 감지 | 대표 |
|---|---|---|---|
| 게임 엔진 에디터 | **앱 포커스** · 버튼 | 훑기 또는 감시(훑기를 싸게) | Unity · Godot |
| IDE | 감시 + **창으로 돌아올 때 동기화**, 또는 감시 기본 꺼짐 | 감시 · 훑기 | JetBrains · Eclipse · VS Code |
| 파일 관리자 | **창 활성화 때 다시 읽기**(네트워크 드라이브는 끔) · 감시 옵션 | 다시 읽기 · 감시 | Total Commander · Explorer |
| 노트 앱 | 감시 | 감시(+ 수동) | Obsidian · Logseq |
| 서버-클라 | 감시는 등록한 것만 · **보이는 동안만 폴링** · 푸시는 신호 + 낮은 주기 확인 | 감시 · 폴링 | LSP · JupyterLab · Nextcloud · Syncthing |

## Engram 에 옮기면 (세션 해석 — 결정은 사용자)

- **선례의 공통 계기는 「앱 창 활성화」다.** 바깥(탐색기 · 에이전트 셸)에서 고치고 돌아오는 순간이 바깥 변경 직후이기 때문이다(해석). 지금 잠정안(슬롯 열기 · 슬롯 포커스 · 새로고침 · 폴더 이동 · 실행 직전)에는 **Engram 창 활성화**가 빠져 있다.
- **리뷰가 짚은 빈자리 — 창을 떠나지 않은 채 바뀌는 경우.** Engram 은 같은 PC 의 에이전트가 파일을 쓰는 도구라, 사람이 Engram 을 보고 있는 동안 라이브러리가 바뀔 수 있다(해석 · 빈도 실측 없음). 활성화 계기만으로는 그 경우를 못 잡는다.
- 선택지:
  - **A. 감시 없음 · 활성화 때 훑기** = Godot · Total Commander 모델. 등록 · 방송 없음. 대가 = 보고 있는 동안의 바깥 변경은 새로고침 전까지 안 보인다.
  - **A′. 감시 없음 · 활성화 때 훑기 + 슬롯이 보이는 동안 주기 요청** = JupyterLab 모델(클라가 보이는 동안만 N초마다 요청 · 숨으면 멈춤). 데몬은 여전히 요청에만 답한다 — 등록 · 방송 · notify 없음. 대가 = 보이는 슬롯 수 × 주기만큼 훑기(수천 개 훑기 비용은 실측 없음).
  - **B. 등록한 동안만 감시 + 다시 훑기 안전망** = LSP 등록 + Theia 수명 + Syncthing 안전망. 바로 보인다. 대가 = 등록 · 해제 · 등록자에게 보내기 + notify(8.2.0 Windows 넘침 누락 → 안전망 필수).
- **A → A′ → B 는 전부 더하기다** — 「요청하면 훑어 답한다」 경로는 셋 다 그대로 쓴다.

## grounding (메인 원문 · 소스 대조)

| 클레임 | 출처 | 판정 |
|---|---|---|
| Unity 포커스 복귀 때 갱신 · Auto-Refresh | docs.unity3d.com Manual/AssetDatabaseRefreshing | 지지 |
| Unity Directory Monitoring(Windows) = 전체 훑기 대신 디렉터리 감시 | docs.unity3d.com Manual/preferences-asset-pipeline | 지지 |
| Godot 앱 포커스 → `scan_changes()` | `godotengine/godot` master `editor/editor_node.cpp` | 지지 |
| Godot `import_resources_when_unfocused` 설명 | `godotengine/godot` master `doc/classes/EditorSettings.xml` | 지지 · CPU 이유는 미지지 |
| Total Commander 활성화 때 다시 읽기 · 네트워크 드라이브 끄기 | ghisler.com/efaquser.htm | 지지 |
| JetBrains 「Sync external changes」 두 옵션 | jetbrains.com/help/idea/system-settings.html | 지지 |
| Eclipse 두 옵션 기본 Off | help.eclipse.org … ref-9.htm | 지지(설명문 잘림) |
| Syncthing 감시 + 시간당 전체 훑기 · 안전망 권고 | docs.syncthing.net/users/syncing | 지지 |
| LSP 클라가 감시 · 비용 이유 | LSP 3.17 didChangeWatchedFiles 원문 | 지지 |
| notify_push 최선 노력 · 주기 확인 권고 | github.com/nextcloud/notify_push | 지지 · 알림 내용 미기재 |
| JupyterLab 기본 10초 폴링 · 숨으면 멈춤 | `jupyterlab/jupyterlab` main `packages/filebrowser/src/model.ts` | 지지 |
| Unreal 자동 재임포트 감시 = 지정 폴더만 · 끄기 | dev.epicgames.com 자동 재임포트 문서 | 지지 · 콘텐츠 브라우저와 무관 |
| Unreal 에셋 레지스트리 디렉터리 콜백 · Theia 감시 수명 · VS Code 원격 감시 | 리뷰어 링크 | 메인 미확인 |
| Blender · Obsidian · Logseq · Eagle · Bridge | 수집자 보고 | 메인 미확인 |

## 적대 리뷰

codex(effort high · 웹 검색 켬) — 판정 **FIX**. 적출과 처리:

| 적출 | 심각도 | 처리 |
|---|---|---|
| Unity 를 「감시 없음」 선례로 든 것은 과장 — Windows Directory Monitoring 이 있다 | major | 메인이 매뉴얼로 확인 → 결론 1 을 「계기 · 감지 분리」로 고침 |
| JupyterLab 은 설정했을 때만이 아니라 기본 10초 폴링 | major | 메인이 소스로 확인 → 고침 · 선택지 A′ 로 반영 |
| Unreal 자동 재임포트 설정은 콘텐츠 브라우저 목록의 근거가 아님 | major | 받아들임 · 레지스트리 콜백 주장은 미확인으로 표시 |
| 「포커스 훑기 이유의 벤더 문서 없음」 반박 — Godot · Total Commander | major | Total Commander FAQ 는 확인(비용 이유) · Godot CPU 이유는 원문에서 못 찾음 → 부분 수용 |
| 데몬-클라 감시 수명 피어(Theia) 누락 | major | 추가(미확인 표시) |
| 클라 포커스를 신선도 경계로 둔 논리 공백 — 보고 있는 동안의 변경 | major | 받아들임 · 「Engram 에 옮기면」에 빈자리와 A′ 추가 |
| Blender 버튼만으로 자동 감지 없음을 추론 | minor | 받아들임 · 불확실로 내림 |

## 한계

- 벤더가 「왜 포커스 훑기인가」를 설계 이유로 적은 문서는 못 찾았다(Total Commander 의 비용 언급이 가장 가깝다).
- Unreal 콘텐츠 브라우저 목록 · VS Code 탐색기 · macOS Finder 의 실제 갱신 경로는 메인이 확인하지 않았다.
- 원격 · 모바일 클라의 놓친 알림 처리(번호 · 재접속 훑기) 문서는 못 찾았다.
- Engram 라이브러리 훑기 비용(수천 개) 실측 없음 — A′ 의 주기 · B 의 안전망 주기를 정하려면 필요하다.
- 확인 못 한 후보: code-server · JetBrains Gateway · filebrowser · WebDAV sync-collection(RFC 6578) · Dropbox `list_folder/longpoll` · Lightroom 「폴더 동기화」 · Directory Opus · Sublime Text.
