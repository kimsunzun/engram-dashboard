# 탭을 창 밖으로 끌 때의 금지 커서 — 바꿀 길이 있나

- 상태: 확정 — 적대 리뷰 FIX 3 건 반영 (2026-10-22) · T-66 에서 정함(사용자 2026-10-23): ④⑤ 이번엔 하지 않음 · B/C 안 감 — PRD `docs/process/S21-window-chrome/prd.md` R28
- 방법: `/research` medium — 수집자 3(① WebView2 API · ② OS 끌기 커서 · ③ 다른 앱 선례) · 메인 grounding(로컬 소스 스팟 확인) · cross-family 적대 리뷰(codex)
- 확신도 범례: 확실 · 가능성 높음 · 불확실 · contested
- 계기: 사용자 실측(2026-10-22) — 탭을 창 밖(바탕화면 · 다른 앱)으로 끌면 🚫 커서가 뜬다. 놓으면 새 창은 정상으로 생긴다(TRD `docs/process/S21-slot-docking/trd.md` §8-5 M2 · M5).

## 결론

**지금 구조(HTML5 끌기 — 끌기의 주인 = WebView2 브라우저 프로세스 · ADR-0296 / ADR-0297)를 유지한 채 커서를 바꾸는 지원된 방법은 찾지 못했다(가능성 높음 — 부재 증명은 아니다).** 바꾸려면 끌기의 주인을 우리 쪽으로 가져와야 하고, 길은 셋이며 모두 크다.

| 길 | 무엇 | 대가 | 확신도 |
|---|---|---|---|
| A. WebView2 `DragStarting` | 호스트가 끌기를 넘겨받아 자기 `IDropSource` 로 `DoDragDrop` → `GiveFeedback` 으로 커서를 정한다 | **visual hosting(composition) 모드 전용** — wry 는 창 모드 컨트롤러만 쓴다. wry 포크 또는 composition 호스팅 자작(DComp · 입력 · `IDropTarget` 전달) + webview2-com 0.39 이상. 열린 회귀 이슈 있음 | 확실(API) · 가능성 높음(대가) |
| C. 셸이 직접 `DoDragDrop`(적대 리뷰가 찾은 길) | 웹이 HTML 끌기 시작을 막고 Tauri 명령으로 셸이 자기 `IDataObject`(사설 형식만 · 빈 자료) + 자기 `IDropSource` 로 OLE 끌기를 연다 — `GiveFeedback` 으로 커서를, 끌기 그림 도우미로 라벨도 정할 수 있다(추론). `DragStarting` · composition 불필요 | **ADR-0296 「거부한 대안」 첫째(Rust 네이티브 OLE 출처 — 「덜어내고」 · 1 차에서 하지 않음)를 다시 여는 일**이고 「끄는 동안 셸 명령 0」(ADR-0297)과도 부딪힌다. 선례 `crabnebula-dev/drag-rs` 는 Tauri/wry 에서 네이티브 끌기를 여나, Windows 구현은 기본 커서를 돌려주고 빈 자료 갈래에서도 현재 폴더를 셸 자료로 내보낸다 — 그대로는 요구를 못 채워 고쳐 써야 한다 | 가능성 높음 |
| B. 크롬식 직접 창 이동 | OS 끌기를 안 쓰고 마우스를 잡아 실제 창을 OS 이동 루프로 옮긴다 | ADR-0296 이 거부하고 U4c′ 가 걷은 옛 엔진 방향의 재설계. T-66(제목 표시줄 걷기 · 마지막 탭 끌면 창 이동)과 겹친다 | 가능성 높음 |
| ✕ 무해한 형식 싣기 | 바탕화면이 받는 형식을 실어 커서를 바꾼다 | 셸이 받는 형식(파일 · 가상 개체 · URL 등)은 놓으면 실제로 소비된다 — 소비되지 않는 알맞은 형식은 찾지 못했다(M5 「새어 나감 없음」 위반 위험) | 가능성 높음 |
| ✕ 시스템 커서 갈아 끼우기 | `SetSystemCursor(OCR_NO)` 를 끄는 동안 바꿨다 되돌림 | PC 전역 — 앱이 죽으면 커서가 바뀐 채 남는다 | 확실(전역) · 가능성 높음(원복 위험) |

## 근거

### ① 커서는 끌기 출처가 정한다

- `DoDragDrop` 은 대상의 `DragEnter/DragOver` 가 돌려준 DROPEFFECT 를 출처의 `IDropSource::GiveFeedback` 에 넘기고, 출처가 커서를 바꾸거나 `DRAGDROP_S_USEDEFAULTCURSORS` 로 OLE 기본 커서에 맡긴다 — 확실. https://learn.microsoft.com/en-us/windows/win32/api/oleidl/nf-oleidl-idropsource-givefeedback
- Chromium 의 `DragSourceWin::GiveFeedback` 은 `DRAGDROP_S_USEDEFAULTCURSORS` 만 돌려준다 → 대상이 NONE 이면 기본 🚫 — 확실. https://chromium.googlesource.com/chromium/src/+/HEAD/ui/base/dragdrop/drag_source_win.cc (브라우저 쪽 `WebDragSource` 본문은 열지 못함 — 같은 동작일 가능성 높음)
- 출처가 아닌 프로세스가 `GiveFeedback` 에 끼어드는 공식 길은 찾지 못했다 — 불확실(부재 증명 아님).
- HTML5 `effectAllowed` / `dropEffect` 가 창 밖 OS 커서를 바꾼다는 근거는 없다 — 창 밖은 외부 대상의 응답이 정한다(추론 · 가능성 높음).

### ② 바탕화면이 받는 무해한 형식은 없다

- 셸 대상이 받는 형식은 `CF_HDROP` · `CFSTR_FILEDESCRIPTOR/FILECONTENTS` · `CFSTR_SHELLIDLIST`(파일 · 가상 개체 모두) · URL · 프린터 · 네트워크 자원 등 — 확실. https://learn.microsoft.com/en-us/windows/win32/shell/datascenarios · https://learn.microsoft.com/en-us/windows/win32/shell/clipboard#formats-for-transferring-virtual-objects — 위 문서는 예시이지 Windows 11 바탕화면의 받는 형식 전수가 아니다(적대 리뷰 지적). 그래도 받는 형식은 놓을 때 소비되므로 「소비되지 않는 알맞은 형식은 찾지 못했다」까지가 말할 수 있는 범위다.
- `CFSTR_PREFERREDDROPEFFECT` 는 출처의 의도 힌트일 뿐 대상이 받게 만들지 않는다 — 확실(문서상 그런 효과 없음).
- `DROPDESCRIPTION` 은 끌기 그림 위 안내 글이지 커서가 아니고, 출처가 `IDragSourceHelper2::SetFlags` 를 불러야 한다 — 확실. https://devblogs.microsoft.com/oldnewthing/20210513-00/?p=105212

### ③ WebView2 `DragStarting` (길 A)

- `ICoreWebView2CompositionController5::add_DragStarting` — 안정판, Win32 SDK 1.0.3719.77 — 확실. https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2compositioncontroller5 · 스펙 https://github.com/MicrosoftEdge/WebView2Feedback/pull/4871
- 인자: `AllowedDropEffects` · `Data`(WebView2 가 넘길 `IDataObject`) · `Position` · `Handled`(TRUE 면 WebView2 가 끌기를 시작하지 않는다) · `GetDeferral` — 확실. 공식 예제가 `DoDragDrop(args->Data, 자기 IDropSource, …)` 후 `Handled=TRUE`. https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2dragstartingeventargs
- 창 모드 `ICoreWebView2Controller` 에는 같은 것이 없다 — 가능성 높음.
- **wry 는 창 모드만**: wry 0.55.1 `src/webview2/mod.rs:408,410` 이 `CreateCoreWebView2ControllerWithOptions` / `CreateCoreWebView2Controller` 를 부르고 composition 컨트롤러 생성은 없다(메인이 로컬 소스로 확인). 최신 0.57.0 도 같다(수집자 확인) — 확실.
- 이 저장소가 잠근 webview2-com-sys 0.38.2(SDK 1.0.3650.58)에는 `DragStarting` 기호가 0 이다(메인 grep 확인) — 확실. 0.39.1 에는 있다(수집자 확인).
- composition 경로의 열린 이슈: WebView2Feedback #5141(끌기 뒤 컨트롤 잠김) · #5695(composition — 화면 밖으로 반복해 끈 뒤 입력 무반응 · 회귀) — 위험 신호. ★#5613 은 composition 이 아니라 창 모드 + `HostInputProcessing` · `AllowExternalDrop` 조건의 끌기 이슈다(적대 리뷰 정정) — `DragStarting` 의 근거로 쓰지 않는다.
- 모름: 최소 런타임 버전 · 선택 글 · 링크 끌기에도 불리는지 · 창 밖에서 호스트 커서가 composition `CursorChanged` 보다 우선하는지.

### ④ 다른 앱 선례

- **Chrome / Edge**: 탭 끌기에 OS 끌기를 쓰지 않는다 — 자기 마우스 캡처 + 떼어 낸 탭을 실제 창으로 만들어 OS 이동 루프(`RunWindowMoveLoop`)로 옮긴다 — 가능성 높음(커밋 메시지 근거). https://chromium.googlesource.com/chromium/src/+/269f66586e5fdb7d7e0e5b7f2a4ef4e7d9c92023 · 그래서 🚫 가 안 뜨는 것으로 보이나 확인 못 함 — 불확실.
- **VS Code(1.85~)**: 우리와 같은 꼴 — HTML5 끌기 + `dragend` 에서 놓은 자리에 보조 창 — 확실. https://github.com/microsoft/vscode/blob/main/src/vs/workbench/browser/parts/editor/multiEditorTabsControl.ts · 창 밖 커서는 확인 못 함 — 불확실.
- Electron 작자의 `dnd-drag-hint`(`dataTransfer.noFailureEffect` 제안)는 HTML5 떼어 내기에서 OS 실패 표시가 뜨는 것을 문제로 다룬다 — 가능성 높음. https://github.com/zcbenz/dnd-drag-hint
- Tauri / WebView2 의 탭 떼어 내기 선례는 찾지 못했다.

## 우리 구조와의 관계

- ADR-0296 / ADR-0297 은 「끄는 동안 셸 명령 0」 · 셸 OLE 출처를 거부한 대안으로 둔다 — 길 A · B · C 는 모두 그 결정을 다시 여는 일이다.
- T-66(제목 표시줄 걷기)은 「마지막 탭 · 탭바 빈 곳을 끌면 창 이동」(사용자 2026-10-22)을 하려면 어차피 마우스를 잡는 창 이동이 필요하다 — 길 B 를 그때 함께 보는 것이 싸다.
- 같은 날 사용자 지적 「탭 사이를 끌 때 도킹용 이름표(끌기 그림)가 나오는 게 어색하다」도 같은 뿌리다 — HTML5 끌기 그림은 시작 때 한 번 정하면 끄는 도중 못 바꾼다. 길 B · C 면 끌기 그림을 우리가 쥔다. T-66 에서 함께 정한다.

## 한계 · 공백

- Chrome · VS Code 의 창 밖 커서는 실측하지 않았다(한 번 끌어 보면 확정된다).
- `WebDragSource::GiveFeedback`(브라우저 쪽) 본문 · 바탕화면이 받는 최소 형식의 실측 없음.
- 길 A 의 실현 비용(wry 포크 범위)은 추정이다.

## 적대 리뷰 (codex · cross-family · FIX)

- **누락 — 길 C(셸 직접 `DoDragDrop`)**: 「두 길」이 거짓 이분법이었다 → 위 표에 더함. 근거 = OLE `DoDragDrop` · `GiveFeedback` 계약 + `crabnebula-dev/drag-rs`(그대로는 요구 미충족).
- **과장 — 「셸은 파일류만 받는다」**: 가상 개체 · URL 등도 받는다 → 「소비되지 않는 알맞은 형식은 찾지 못했다」로 낮춤.
- **오귀속 — #5613**: 창 모드 이슈 → composition 근거에서 뺌.
- 재확인 통과: `GiveFeedback` 이 커서를 정함 · `ICoreWebView2CompositionController5::add_DragStarting` SDK 1.0.3719.77 · wry 0.55.1 / 0.57.0 창 모드 컨트롤러(빌더에 composition 스위치 없음) · Chrome 탭 끌기 = 이동 루프(Edge 는 따로 확인 안 함).
- 「HTML 끌기를 유지한 채 쓸 지원된 덮어쓰기」는 리뷰어 추가 탐색에서도 없었다 — 부재 증명이 아니라 「찾지 못함」이다.
