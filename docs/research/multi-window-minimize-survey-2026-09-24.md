# 다중 창 최소화 연동 — 피어 관행 · Win32 소유 창 · Tauri v2

- 상태: **조사 완료 · 결정 대기(사용자)** · 날짜: 2026-09-24
- 방법: `/research` **medium**(설계 서베이 — 사용자는 light 로 불렀으나 설계 서베이 규칙으로 승격) · 주계열 수집 2(피어 관행 · 메커니즘) + 코드 사실 수집 1 · 메인 grounding(아래 「grounding」) · cross-family 적대 리뷰 1회(아래 「적대 리뷰」)
- 질문(사용자, 2026-09-24): 「최소화했다가 아이콘을 누르면 복원되는데, 창을 여러 개로 분리했을 때는 메인 창만 적용된다. 메인 창이 최소화되면 다른 창도 다 최소화돼야 하지 않나.」
- 확신도 범례: **확실** = 1차 출처 인용 + 독립 교차확증 · **가능성 높음** = 1차 출처 하나 또는 로컬 소스 확인 · **불확실** = 약한 출처·추론 · **모름** = 출처 없음

## 결론

- **연동 수단은 둘이고, 수단이 동작을 정한다.**
  - **소유 창**(Win32 owner — Tauri `parent`/`owner`): 메인이 최소화되면 **OS 가** 팝아웃을 숨기고 복원 때 되살린다. 대가로 팝아웃은 **항상 메인 위**에 있고 메인 뒤로 못 간다. (확실)
  - **독립 창 + 이벤트 동기화**: 지금 구조(독립 최상위 창) 그대로 두고, 메인 최소화·복원을 감지해 다른 창을 따라 움직인다. z-순서·작업표시줄 버튼은 지금과 같다. Tauri 에 최소화 이벤트가 없어 감지·루프 방지를 직접 짠다. (가능성 높음)
- **성숙한 데스크톱 IDE 는 둘 다 두고 사용자가 고르게 한다.** Visual Studio 는 3단 설정이고 **기본값이 「도구 창은 메인 소유」**(= 함께 최소화)다. JetBrains 는 도구 창마다 Float(메인과 함께) / Window(독립)를 고른다. (확실)
- **웹 계열 팝아웃(VS Code 보조 창·Chrome DevTools·Teams)은 독립 쪽**이다. VS Code 에는 「보조 창이 메인 뒤로 가라앉는다」 불만이 있었고, 뒤에 창별 「항상 위」 모드가 들어왔다 — 그 둘을 이은 설계 판단(소유를 검토하고 버렸다)은 출처가 없다. (가능성 높음)
- **「메인 복원 시 독립 팝아웃도 같이 복원」을 명시한 피어는 못 찾았다** — 소유 창은 OS 가 해 주고, 독립 창 피어는 문서가 없다. 트레이 복원 경로를 다룬 피어 문서도 없다. (모름)

## 현재 코드 (읽기 확인 — 실행 안 함)

- **트레이는 더블클릭이 아니라 좌클릭 1회**(버튼을 뗄 때)다 — `src-tauri/src/tray/mod.rs:111-120`. 사용자 요청으로 더블→단일로 바꿨다는 주석이 `:106` 에 있고, `:103` 의 「좌클릭 더블=UI 열기」 주석은 낡았다.
- 복원 함수 `show_main_ui` 는 **main 하나만** `show → unminimize → set_focus` 한다(`src-tauri/src/tray/actions.rs:55-61`, 순서는 Windows 포커스 강탈 때문에 load-bearing — `:52`). 같은 함수를 단일 인스턴스 콜백(`src-tauri/src/lib.rs:36-37`)과 `show_main_ui` 명령(`src-tauri/src/commands/tray.rs:10`)도 부른다.
- 팝아웃 창은 **독립 최상위 창**이다 — `src-tauri/src/commands/popout.rs:110-120` 의 빌더에 `parent`·`owner`·`skip_taskbar`·`always_on_top` 이 없다. 그래서 창마다 작업표시줄 버튼이 있고 메인 뒤로 갈 수 있다.
- 창 이벤트 처리는 `src-tauri/src/lib.rs:178-210` 한 곳뿐 — main `CloseRequested` → `prevent_close` + `hide`(트레이로 숨김, ADR-0026), 팝업 `Destroyed` → 정리. **최소화 감지 코드는 셸·화면 어디에도 없다.**
- 메인에서 다른 창으로 번지는 동작은 **하나도 없다** — main 숨김(닫기·`hide_main_ui`·`--hidden` 기동)도 main 만 숨긴다.
- ADR: 창 간 최소화 연동이나 팝아웃의 owner·작업표시줄 설정을 정한 ADR 은 없다. ADR-0023(→0026 대체)·0026·0027 은 닫기=트레이 숨김과 재기동 시 기존 창 올리기만 정한다.

## 플랫폼 사실

### Win32 소유 창 (1차 출처 — 메인이 원문 대조)

[Window Features — Owned Windows · Visibility](https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features):

- 「An owned window is always above its owner in the z-order.」 · 「The system automatically destroys an owned window when its owner is destroyed.」 (확실)
- 「When an owner window is minimized, the system automatically hides the associated owned windows. Similarly, when an owner window is restored, the system automatically shows the associated owned windows.」 (확실)
- 「**Hiding an owner window has no effect on the visibility state of the owned windows.**」 — 우리 main 의 「닫기 = 트레이로 숨김」은 소유 관계를 걸어도 팝아웃을 숨기지 **않는다**. (확실)
- 소유권 이전: 같은 페이지는 생성 후 이전 불가라 적고, `SetWindowLongPtr` 문서는 `GWLP_HWNDPARENT` 로 새 owner 를 설정한다고 적어 **문서끼리 어긋난다**. 우리 경로는 생성 시 지정이라 이 쟁점을 밟지 않는다. (불확실 — 우리와 무관)

[The Taskbar](https://learn.microsoft.com/en-us/windows/win32/shell/taskbar): 「The Shell creates a button on the taskbar whenever an application creates a window that isn't owned.」 `WS_EX_APPWINDOW` 는 버튼을 강제하고 `WS_EX_TOOLWINDOW` 는 없앤다. (가능성 높음 — 수집자 인용, 메인 미대조)

### Tauri v2 / tao (로컬 소스 — tauri 2.11.3 · tao 0.35.3, `Cargo.lock` 기준)

- `WebviewWindowBuilder::parent(&main)` 은 **Windows 에서 owner 를 건다** — 문서 주석(`tauri-2.11.3/src/webview/webview_window.rs:617-631`)이 위 MSDN 세 문장을 그대로 싣는다. `#[cfg(windows)] owner(...)`·`owner_raw(HWND)` 도 같다. `parent_raw(HWND)` 만 `WS_CHILD`(부모 클라이언트 영역에 갇힘)라 우리 용도가 아니다. (가능성 높음 — 로컬 소스)
- tao 는 owner 가 있으면 `WS_POPUP` 을 세우고 `WS_EX_APPWINDOW` 를 **안** 세운다(`tao-0.35.3/src/platform_impl/windows/window.rs:1159-1164`, `window_state.rs:258-277` — 메인이 대조). **그런데 모든 창 생성 끝에 `set_skip_taskbar(false)` → `ITaskbarList::AddTab` 을 부른다**(`window.rs:1334`, `:1529-1535` — 메인이 대조). 그래서 **Tauri 소유 팝아웃도 작업표시줄 버튼을 가질 가능성이 높다** — 그 버튼이 메인 최소화 중에 어떻게 보이고 눌리면 무엇이 되는지는 **실측 없음**. `skip_taskbar(true)` 로 버튼을 없앨 수 있다(Tauri 메인테이너 답 — [discussion #8574](https://github.com/orgs/tauri-apps/discussions/8574), 수집자 인용). (불확실)
- **최소화 이벤트는 없다** — `tauri-runtime-2.11.3/src/window.rs` 의 `WindowEvent` 에 Minimized/Restored 가 없다. Windows 에선 최소화가 `Resized(0,0)`(+ `Moved(-32000,-32000)`)로 오고 `is_minimized()`(= `IsIconic`)로 확정한다. (가능성 높음 — 로컬 소스 + [tauri #7664](https://github.com/tauri-apps/tauri/issues/7664) 수집자 인용)
- 프로그램으로 `minimize()` 하면 최대화 창이 한 번 복원 크기로 번쩍인 뒤 최소화된다는 보고가 있다 — [tao #1338](https://github.com/tauri-apps/tao/issues/1338). 이벤트 동기화 경로의 체감 차이. (불확실 — 단일 보고)

## 피어 관행 (요약 — 상세 출처는 수집 결과)

| 피어 | 메인 최소화 때 | 자기 작업표시줄 버튼 | 메인 뒤로 가나 | 구현 | 확신도 |
|---|---|---|---|---|---|
| Visual Studio — Floating Windows = **Tool Windows(기본)** | 함께 최소화 | 없음 | 안 감(IDE 활성 중 위) | 메인 소유 | 확실([MS Learn](https://learn.microsoft.com/en-us/visualstudio/ide/customizing-window-layouts-in-visual-studio) 원문 대조) |
| Visual Studio — Documents and Tool Windows | 함께 | 없음 | 안 감 | 메인 소유(문서 창까지) | 확실(같은 문서) |
| Visual Studio — None | **그대로 보임** | 있음 | 감 | 독립 | 확실(같은 문서 — 「FancyZones 스냅·다른 앱 작업 중에도 보이게」가 용도) |
| JetBrains — Float | 「visible only together with the main project window」 | 문서 없음 | 안 감(메인 위) | 모름(소유 추정) | 가능성 높음([JetBrains](https://www.jetbrains.com/help/idea/viewing-modes.html), 수집자 인용) |
| JetBrains — Window | 독립 | 문서 없음 | 감 | 모름 | 가능성 높음 |
| VS Code 보조 창 | 약한 근거: 메인 최소화 뒤에도 분리 터미널 창이 쓸 수 있게 남는다는 보고 하나([#322582](https://github.com/microsoft/vscode/issues/322582) — 적대 리뷰어 인용, 메인 미대조) | 지식 기반(있음) | 감 — 불만 이슈 후 창별 「항상 위」 토글(1.100) | 별도 Electron 창 | 불확실([#204912](https://github.com/microsoft/vscode/issues/204912)·[#246694](https://github.com/microsoft/vscode/issues/246694) 수집자 인용) |
| Chrome DevTools 분리 | 모름 | 있음(검색 요약) | 지식 기반(감) | 별도 브라우저 창 | 불확실 |
| Teams·Slack·Discord 팝아웃 | 모름/약한 출처 | 대체로 있음 | 모름 | 모름 | 불확실 |
| GIMP 도크 | 창 관리자 힌트 설정(Normal / Utility / Keep Above) | WM 의존 | Keep Above 면 안 감 | WM 힌트 | 확실(문서) · WM 의존 |

## 선택지 (engram 제약 대조)

engram 제약: 창·탭은 셸(백엔드)이 소유한다(ADR-0035/0057) · 새 UI 기능엔 LLM 호출 경로를 함께 만든다(CLAUDE.md 「LLM-우선 제어」) · 팝아웃은 런타임 생성 창이다(ADR-0057) · 닫기 = 트레이로 숨김(ADR-0026).

| | A. 소유 창(`parent(&main)`) | B. 독립 + 이벤트 동기화 | C. 현행 유지(독립) | D. 설정으로 고르게(A 또는 B + 끄기) |
|---|---|---|---|---|
| 메인 최소화 → 팝아웃 | OS 가 숨김 | 셸이 `minimize()` | 그대로 | 설정대로 |
| 메인 복원(작업표시줄·트레이) → 팝아웃 | OS 가 되살림 | 셸이 `unminimize()` | 그대로 | 설정대로 |
| 팝아웃이 메인 뒤로 | **못 감**(항상 위) | 감(지금과 같음) | 감 | 설정대로 |
| 팝아웃 작업표시줄 항목 ※ | 있을 가능성 높음(tao AddTab) — **미실측** · `skip_taskbar` 로 끌 수 있음 | 있음(지금과 같음) | 있음 | 설정대로 |
| 팝아웃만 따로 최소화 | 가능 — 버튼이 없으면 작은 제목줄 조각으로 떨어짐(Electron #20059 보고) | 가능 | 가능 | — |
| 메인 닫기(트레이 숨김) → 팝아웃 | **안 숨음**(MSDN 「Hiding an owner … no effect」) — 원하면 [`ShowOwnedPopups`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-showownedpopups) 로 함께 숨기고 되살릴 수 있다(되살리기는 그 함수가 숨긴 창만) | 원하면 같은 훅에 더할 수 있음 | 안 숨음 | — |
| 구현량 | 빌더 한 줄 + 실측 | 창 이벤트 감지(`Resized`+`is_minimized`) + 루프 방지 + 복원 순서·포커스 규칙 + **「동기화가 최소화한 창만 되살린다」 기억**(메인보다 먼저 사용자가 따로 최소화해 둔 팝아웃까지 `unminimize()` 로 펴 버리지 않게 — 적대 리뷰 적출) | 0 | A/B + 설정 저장·명령 |
| 위험 | 「항상 위」가 같은 모니터에서 메인을 가린다 · 작업표시줄 버튼 동작 미실측 | 최대화 창 번쩍임(tao #1338) · 창마다 최소화 애니메이션 · 팝아웃만 복원될 때 규칙 필요 | 사용자가 문제 삼은 현상 그대로 | 결정 둘로 늘어남 · **이미 열린 팝아웃에 설정 변경이 언제 먹나**를 정해야 한다 — A 는 생성 때 owner 를 거므로 즉시 적용하려면 창을 다시 만들거나 생성 뒤 owner 를 바꿔야 한다(후자는 위 「소유권 이전」 문서 충돌을 밟는다) |
| 피어 | VS 기본 · JetBrains Float | 명시 선례 못 찾음 | VS None · VS Code · Chrome | VS · JetBrains · GIMP |

- ※ **작업표시줄 「항목」과 「버튼」을 가른다.** Windows 는 같은 AppUserModelID 의 창들을 한 버튼으로 묶는다([AppUserModelID](https://learn.microsoft.com/en-us/windows/win32/properties/props-system-appusermodel-id) — 적대 리뷰어 인용). 그래서 위 표의 「있음」은 창마다 작업표시줄 항목(미리보기 썸네일)이 있다는 뜻이고, 따로 떨어진 버튼이라는 뜻이 아니다 — 묶음 설정에 따라 다르다.
- **LLM 경로:** A·B·C 는 새 사용자 조작이 아니라 창 동작이라 명령이 필요 없다. D 는 설정이 생기므로 그 설정을 바꾸는 명령이 따라와야 한다(「LLM-우선 제어」).
- **agent-tree 창**(숨은 정적 창)은 ADR-0225 가 걷어낼 예정이라 어느 선택지에서도 대상에서 뺀다.

## 미확인 — 결정 전에 스파이크로 풀 것

- **A 의 작업표시줄 동작**: Tauri 소유 팝아웃이 실제로 버튼을 갖는지, 메인 최소화 중 그 버튼이 사라지는지, 누르면 무엇이 되는지. 10분짜리 실측이면 풀린다(수집자 권고).
- WebView2 창이 OS 에 의해 숨었다 보였다 할 때 렌더링·xterm 에 부작용이 있는지 — 조사 안 함.

## grounding (메인 외부 대조)

- 지지: MSDN 소유 창 네 문장(최소화 숨김·복원 표시·항상 위·함께 파괴) + 「Hiding an owner … no effect」 — 원문 grep 대조. Visual Studio 3단 설정과 기본값·각 옵션 의미 — 원문 대조. tao owner → `WS_POPUP`·`WS_EX_APPWINDOW` 미설정·`AddTab` 호출 — 로컬 소스 대조. 현재 코드 사실 — 코드 수집자 `파일:줄`(메인은 트레이·팝아웃 빌더 줄을 재확인하지 않았다).
- 미대조(수집자 인용만): JetBrains 문구 · VS Code 이슈 · Taskbar 문서 · tauri #7664 · tao #1338 · discussion #8574 · Electron 이슈들.

## 적대 리뷰

cross-family(GPT, `codex exec` effort high · web search) 1회 — 판정 **FIX 6**, 전부 반영했다. 반증으로 뒤집힌 주장(contested)은 없다.

- 반영: ① B 의 복원이 사용자가 따로 최소화해 둔 팝아웃까지 펴는 논리 공백 → 「동기화가 최소화한 창만」 규칙 추가 ② D 의 이미 열린 팝아웃 적용 시점 미정 → 표에 명시 ③ A 의 트레이 숨김 한계에 `ShowOwnedPopups` 누락 → 추가 ④ VS Code 「모름」에 약한 반대 근거 하나 → 표에 달았다(메인 미대조) ⑤ 작업표시줄 항목과 버튼 혼용 → ※ 주 추가 ⑥ VS Code 가 소유를 버렸다는 귀속 과장 → 걷었다.
- 리뷰어 grounding 재확인(지지): MSDN 소유 창 동작 · Visual Studio 3단 설정·기본값 · Tauri `parent` = owner(`tauri-2.11.3/src/window/mod.rs:640`) · tao `AddTab` 호출(실제 표시는 미실측) · 최소화 이벤트 부재(`tauri-runtime-2.11.3/src/window.rs:30`, tao `event_loop.rs:1227`).

## 추가 조사 — 메인 닫기(트레이 숨김) 때 팝아웃 (2026-09-24, light)

사용자가 최소화 연동 대신 새 안을 냈다 — **최소화는 창마다 각자, 메인 X(트레이로 숨김)는 모든 창을 숨기고, 트레이 아이콘은 모든 창을 되살린다.** 그 안의 피어 관행을 좁게 확인했다(수집자 1 · 메인 스팟체크).

- **결론: 근거가 얇고, 확인된 두 앱은 팝아웃을 남긴다 — 그런데 그 한쪽에서 사용자가 새 안과 같은 동작을 요청하고 있다.** 트레이 복원에서 모든 창을 되살리는 출처 있는 선례는 하나다. Discord·Slack·Teams·Steam 은 공식 문서가 없다(모름).
- **OBS Studio** — 트레이로 보내도 전체화면 프로젝터 창이 남는다: 「OBS does indeed move itself over to System Tray, but the window for the Fullscreen Projector (Preview) is still on my task bar」 · 원하는 것: 「move this window too down to System Tray along with OBS?」 · 답글·설정 없음. ([포럼](https://obsproject.com/forum/threads/fullscreen-projector-preview-not-moving-to-system-tray-along-with-obs.169893/) — 메인 원문 대조 · 가능성 높음, 사용자 보고 하나)
- **Telegram Desktop** — 메인을 닫아도 분리 채팅 창이 남는다(재현 절차가 메인을 닫은 뒤 남은 채팅 창을 닫는다). 그 채팅 창을 닫으면 숨은 메인이 다시 뜨는 결함이 함께 보고됐다. ([tdesktop #27656](https://github.com/telegramdesktop/tdesktop/issues/27656) — 수집자 인용 · 가능성 높음)
- **hermes-agent**(Electron, Windows) — 트레이 클릭이 **숨은 창 전부**를 되살린다: 「for each hidden window it calls `setSkipTaskbar(false)`, then `win.restore()` if `isMinimized()`, then `win.showInactive()`」 → 그 뒤 메인에 포커스. 메인 X 가 다른 창을 어떻게 하는지는 이슈에 없다. ([#119243](https://github.com/NousResearch/hermes-agent/issues/119243) — 메인 원문 대조 · 가능성 높음)
- **Microsoft Teams** — X = 트레이가 기본이지만 팝아웃 창의 운명은 문서에 없다. 메인을 **최소화**하면 회의가 별도 팝아웃 창으로 튀어나와 남는다는 불만은 있다([Q&A](https://learn.microsoft.com/en-us/answers/questions/4430717/how-to-disable-teams-meeting-in-pop-out-window-whe) — 수집자 인용 · 가능성 높음).
- Electron 은 트레이 튜토리얼이 창 하나의 hide/show 만 다룬다 — 「모든 창 숨김」 공식 패턴은 없다(수집자 · 불확실).
- **판단에 미치는 영향:** 새 안은 피어 다수가 **하지 않는** 동작이지만, OBS 사용자 요청과 hermes-agent 의 트레이 복원이 같은 방향이다. 기각할 근거(「그렇게 하면 문제가 났다」)는 찾지 못했다.
