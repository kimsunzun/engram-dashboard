# 끄는 도중의 외부 사건 — 관행 조사 (취소 · 미룸 · 그냥 둠+놓을 때 재검증)

- **상태:** 확정 (medium · 설계-결정 모드 · codex 적대 리뷰 FIX 3건 반영) · 이 보고서 자체는 결정하지 않는다 — 관행 파악용(사용자 지시 2026-10-20 「한다는게 아니라 관행을 일단 보라고」). 이 조사 뒤 사용자가 고른 결정 = **ADR-0297**(그냥 둠 + 놓을 때 다시 찾기)
- **날짜:** 2026-10-20
- **방법:** 갈래 셋(A 직접 피어 · B 진행 중 OS 끌기 취소 수단 · C 끄는 동안 IPC 의 실제 위험과 우리 불변식의 근거) 수집 → 메인 grounding → codex cross-family 적대 리뷰
- **확신도 범례:** 확실(독립 교차확증) · 가능성 높음(근거 지지 · 단일 출처) · 불확실 · contested
- **계기:** U3 코드 리뷰(2026-10-20)가 「끄는 동안 셸 호출 0」(ADR-0296 · TRD 7판 §2-2) 밖에서 새는 호출(창 틀 조회 · 캔버스 크기 보고)을 잡았다. 사용자 물음 — 끄는 중 사건이 생기면 끌기를 취소하면 모든 이슈가 없지 않나(요지 — 원문 아님) · 「끌기 때문에 메인에 있는걸 멈추면 모든 예외사항들을 추가해야되서」(원문).

## 결론 (요약)

1. **관행의 주류는 「그냥 둠 + 놓을 때 id 로 다시 찾아 없으면 무동작」이다**(예외 = dockview — 못 찾으면 무동작 대신 던진다). 조사한 피어에서 끄는 동안 모델 · 레이아웃 · 창 크기 변화를 통째로 막거나 미루는 전역 차단은 확인하지 못했다. 국소 예외는 있다 — Chromium 은 끄는 동안 탭바 편집을 막고(좁은 막음) 터치 끌기의 빈 출처 창 닫기를 끌기 끝까지 미루며, golden-layout 은 끌기 시작에 항목을 트리에서 떼어 남이 못 건드리게 한다(불확실 — 추론)(표 A). (가능성 높음 — 피어 다섯이 소스로 같은 꼴 · 「없으면」의 처리만 dockview 가 다르다)
2. **끌기를 도중에 끝내는 피어는 끄는 항목이나 그 탭바(컨테이너)가 바뀔 때만 끝낸다 — Chromium = 끄는 탭 제거 · 창 끌기 중 탭 추가(강제 완료) / Lumino = 그 탭바의 탭 추가 · 제거 · 방향 바뀜(중단). IPC · 창 크기 같은 무관한 사건으로 끝내는 피어는 없다.** 자기 끌기 루프를 가진 구현에서만 한다. 끝내는 방식은 갈린다 — Chromium 탭바는 **강제 완료**(`EndDrag(kComplete)` — 되돌리지 않음 · 취소 `kCancel` 은 `RevertDrag` 로 되돌리는 다른 길), Lumino 탭바는 **중단 + 자리 되돌림**(`_releaseMouse`). 사유는 「복잡한 특수 경우를 피하려고」. (가능성 높음)
3. **미룸은 드물고 국소적이다** — 전역 대기열이 아니라 특정 위험(끌기를 쥔 창 파괴 · 캡처 상실)만 미룬다(Chromium). **끄는 동안 저장 · IPC 를 막는 피어는 못 찾았다**(못 찾음 ≠ 없음).
4. **우리 「끄는 동안 셸 호출 0」은 기술 제약이 아니라 설계 선택이다** — 근거는 옛 엔진의 동기화 결함(ADR-0296:8 · :37)이고, 우리 스파이크는 끄는 동안 invoke · emit 이 정상임을 쟀다(작은 표본). (가능성 높음)
5. **진행 중인 HTML5 끌기를 웹 코드가 취소할 길은 없다. 셸(Tauri/wry 창 호스팅)에서도 지원되는 길이 없다.** Esc 주입은 미검(스파이크 필요). (웹 쪽 = 가능성 높음 · 셸 쪽 Esc = 불확실)

## A. 직접 피어 — 끄는 도중 외부 사건

| 피어 | 꼴 | 무엇에 | 근거 | 확신도 |
|---|---|---|---|---|
| **Chromium 탭바** (`TabDragController`) | 혼합 | 끄는 탭이 곧 지워짐 → 끌기를 **완료로** 끝냄(`EndDrag(kComplete)` — 되돌리지 않음 · 「End the drag before we remove a tab that's being dragged, to avoid complex special cases」) · 떼어 낸 창 끌기 중 남이 탭 추가 → 완료로 끝냄(「not worth the complexity」) · 다른 탭 제거 · 교체 → 추적만(끌기 유지) · 창 크기 · 자리 변화 → 흘려보내고 다시 판정 · 터치 끌기의 빈 출처 창 닫기 → 끌기 끝까지 미룸 · 탭바 편집 가능 = 끄는 동안 거짓(좁은 막음) | 로컬 사본 `.claude/handoff/attachments/chromium-src-2026-10-18/tab_drag_controller.cc:732 · :742 · :2117-2128` · `tab_strip.cc:1399` (메인이 원문 대조) · 국소 예외 둘 = 탭바 편집 막음 `IsTabStripEditable` `tab_strip.cc:1298-1301` · 터치 끌기의 빈 출처 창 닫기 미룸 `tab_drag_controller.cc:651-669 · :1620-1623`(같은 사본 · 줄 대조 2026-10-20) | 가능성 높음 |
| **VS Code** (편집기 · 그룹 DnD) | 놓을 때 재검증 | 출처 그룹을 id 로 다시 찾고 없으면 무동작 · 그룹 사이 옮김은 놓을 때 `canMove` 를 다시 봄 | `src/vs/workbench/browser/parts/editor/editorDropTarget.ts` · `editorGroupView.ts`(main) | 가능성 높음 (끄는 중 닫힌 편집기 = 불확실) |
| **dockview** | 놓을 때 재검증(던짐 — 주류의 「무동작」과 다른 유일한 피어) | 그룹 · 패널을 id 로 찾고 없으면 예외를 던짐 · 호스트가 `onWillDrop` 으로 막을 수 있음 | `packages/dockview-core/src/dockview/dockviewComponent.ts`(master) | 가능성 높음 |
| **FlexLayout** | 놓을 때 재검증(무동작) | `applyMoveNode` 가 id 로 찾고 없거나 타입이 틀리면 무동작 | `src/model/Model.ts` | 가능성 높음 |
| **Zed** | 놓을 때 재검증(무동작) | 「// Tab was closed during drag」 → `return` | `crates/workspace/src/workspace.rs:12257`(main · 메인이 원문 대조) | 가능성 높음 |
| **Lumino (JupyterLab · Theia)** | 탭바 = 중단 · 도크 = 놓을 때 재검증 | 그 탭바가 바뀌기 전에 「Release the mouse before making any changes」 → 끌기 중단 — 방향 바뀜(`orientation` setter `:332`) · 탭 추가(`:431`) · 제거(`:523`) · 모두 지움(`clearTabs` `:558`) · 도크 끌기는 놓을 때 대상을 다시 평가 | `packages/widgets/src/tabbar.ts:332 · :431 · :523 · :558`(main · 메인이 원문 대조 · `:558` = 문서 리뷰 2026-10-20 이 더함) · `dockpanel.ts` | 가능성 높음 |
| **golden-layout v2** | 시작 때 떼어 냄 · 놓을 때 캐시 | 끌기 시작에 항목을 트리에서 떼어 남이 못 건드림 · 영역은 시작 때 한 번 셈 | `src/ts/controls/drag-proxy.ts` | 불확실(결과는 추론) |
| **Firefox 탭** | 놓을 때 null 검사 + 낡은 끌기 감시 | 끌기 세션이 사라졌거나 버튼이 떼였으면 `endDragSession(false)` 로 강제 종료 | `browser/components/tabbrowser/content/drag-and-drop.js`(main) | 불확실(끄는 탭 제거 시 동작 = 모름) |

- **판단 못 한 것:** JetBrains · Obsidian(닫힌 소스) · Tabby/Hyper 등 Electron 다중 창 앱 · Zed 창 사이 탭 끌기.
- **패턴:** 끌기 짐(payload)이 대부분 id · 손잡이라 놓을 때 다시 찾고, 낡았으면 저절로 무동작이 된다.

## B. 진행 중인 HTML5 끌기를 취소할 수단 (Chromium · WebView2 · Windows)

| 수단 | 취소되나 | 근거 | 확신도 |
|---|---|---|---|
| 출처 요소 제거 · 숨김(시작 뒤) | 아니오 — dragstart 직후 한 번만 본다 | Blink `mouse_event_manager.cc` · `drag_controller.cc` | 가능성 높음 |
| `dropEffect = 'none'` · dragover 미수락 | 부분 — 놓기만 실패, 끌기는 버튼을 뗄 때까지 계속 · dragend `none` | HTML 표준 DnD 처리 모델 | 가능성 높음 |
| JS 합성 Esc | 아니오 — 신뢰 안 된 사건은 OLE 루프에 안 닿음 | `ui/base/dragdrop/drag_source_win.cc` | 가능성 높음(추론) |
| `drag` 사건 preventDefault | Chromium 에서는 아니오 — 반환값을 안 본다(「we don't care if event handler cancels default behavior」) · **표준은 취소를 끌기 중단으로 읽을 여지가 있다**(리뷰 지적 — 수집자는 「작업을 none 으로」로 읽음) | Blink `event_handler.cc` 주석 · HTML 표준 DnD | Chromium 동작 = 가능성 높음 · 표준 해석 = contested |
| 셸에서 Esc 주입(SendInput · PostMessage) | **모름** — Windows ole32 는 닫힌 소스 · 키보드 포커스가 필요할 수 있음 | Wine 참조 구현만(Windows 근거 아님) | 불확실 — 스파이크 필요 |
| WebView2 `CompositionController5.DragStarting` | 진행 중 끌기의 취소 수단이 **아니다** — WebView2 의 **처음** `DoDragDrop` 을 호스트가 가로채(`Handled`) 직접 돌리는 길이고, 그 뒤 취소는 호스트 자신의 `IDropSource`(`DRAGDROP_S_CANCEL`)가 한다(다른 메커니즘). 컴포지션 호스팅 전용 · wry 는 창 호스팅이라 못 씀 | MS Learn · `IDropSource::QueryContinueDrag` · wry-0.55.1 `src/webview2/mod.rs` | 가능성 높음 |
| 합성 마우스 떼기 | 취소가 아니라 놓기로 끝남(+ dragover 거부면 결과는 「놓기 없음」) | `drag_source_win.cc` | 불확실 |

- 참고: 우리 실측(TRD §9-2 · S14)에서 사람이 친 빠른 Esc 도 5 회 중 1 회 놓쳤다.

## C. 끄는 동안 IPC 의 실제 위험 · 우리 불변식의 근거

- **우리 근거(1차 출처):** ADR-0296:8 — 옛 Chrome식 엔진에서 셸 ↔ 웹뷰 경계 결함이 반복됐다(원인 확인 1 · 미확인 2). 「공통 뿌리 = 한 끌기를 두 프로세스가 판정하고 끄는 동안 상태 · 기하를 IPC 로 맞춤」. ADR-0296:37 — 「끌기 도중 셸 호출 0 … 되살리면 이 결정의 이유(경계 동기화 결함)가 돌아온다」. TRD §4-7 은 (조사 당시) 그 불변식 때문에 `resizePty` 를 미뤘다(크기 맞춤 자체의 해는 주장하지 않았다 · ADR-0297 로 걷음). **성격 = 설계 선택 + 옛 엔진에서 물려받음.**
- **우리 반대 측정:** `docs/research/slot-docking-drag-mechanism-2026-10-16.md:36` — 끄는 동안 invoke 왕복 p50 2.2 · 최대 약 50 ms · emit 정상 · 멈춤 없음(Tauri 2.11.3 · WebView2 155 · RDP · 22 회 — 작은 표본). DoDragDrop 은 WebView2 브라우저 프로세스 스레드에서 돈다(Tauri 메인 스레드 아님).
- **외부:** Chromium 은 일부러 DoDragDrop 루프 안에서 작업을 돌린다(`web_contents_view_aura.cc` `StartDragging` 의 `ScopedAllowApplicationTasksInNativeNestedLoop`). WebView2 메시지 · Tauri invoke 가 끄는 동안 깨진다는 보고는 못 찾았다. **약한 보고가 있는 것은 창 조작뿐** — 끄는 중 다른 창 포커스 · 끄는 중 처음 보인 창이 놓을 곳이 못 됨(블로그 한 곳).

## 우리 제약에 비춘 선택지 (결정 전 비교 — 조사 당시의 표)

→ 결정 = **ADR-0297**(그냥 둠 + 놓을 때 다시 찾기 · 2026-10-20 사용자 결정). 아래 「우리에게」 칸은 조사 당시 코드(U1 의 엄격 거절 · U3 의 미룸)를 적은 것이고 지금 코드와 다르다.

| 선택지 | 관행 선례 | 우리에게 | 대가 |
|---|---|---|---|
| **그냥 둠 + 놓을 때 재검증** | 주류(VS Code · dockview(던짐) · FlexLayout · Zed · Lumino 도크) | 셸이 이미 놓을 때 사전조건(`TargetChanged` · `SourceChanged` · `TabNotFound`)으로 거절한다 — 반쯤 이 꼴이다 | 「끄는 동안 셸 호출 0」을 「끌기가 상태를 셸과 맞추지 않는다」로 좁혀야 한다(무관한 호출은 흘려보냄). 미룸 예외(터미널 크기 · 창 틀 · 캔버스)를 걷을 수 있다 |
| **끄는 항목이나 그 탭바가 바뀔 때만 끌기를 끝냄** | Chromium 탭바(강제 완료) · Lumino 탭바(중단 + 되돌림) | 끄는 탭이 LLM 등으로 닫힐 때 | 취소 수단이 없다(B) — Esc 주입은 미검 |
| **미룸(조사 당시 설계)** | 국소 선례만(Chromium 창 파괴) | 예외를 사건마다 더해야 한다(리뷰가 잡은 새 틈 둘이 그 증거) | 누락이 곧 불변식 위반 |
| **무관한 사건에 취소** | 선례 없음 | — | 취소 수단 없음 · 관행에 없음 |

## 한계 · 공백

- 피어 소스는 대부분 main 브랜치 한 시점이고 줄 번호는 낡는다. Chromium 은 로컬 사본(리비전 미상).
- ★Chromium 근거(표 A · 적대 리뷰 1 · grounding)는 이 PC 에만 있는 사본을 읽은 것이다★ — `.claude/handoff/attachments/chromium-src-2026-10-18/` 은 git 이 추적하지 않아 다른 PC 에는 없다. 줄 번호도 그 사본 기준이다.
- 「끄는 동안 IPC 를 막는 피어 없음」은 못 찾았다는 뜻이다.
- 셸 Esc 주입 · 끄는 동안 창 조작의 영향은 미검.

## 적대 리뷰 (codex · 2026-10-20 · FIX)

1. **Chromium 을 「취소」 선례로 적은 것 = 틀린 세부** — `EndDrag(kComplete)` 는 강제 완료다(취소는 `kCancel` → `RevertDrag`). 메인이 로컬 사본 `tab_drag_controller.cc:727-746 · :2114 · :2173` 으로 확인하고 고쳤다.
2. **`drag` preventDefault 를 표준 근거로 「취소 안 됨」이라 적은 것 = 오귀속** — 표준과 Chromium 구현을 갈라 적었다(표준 해석은 contested 로 남김 — 우리 결론에는 무관: Chromium 이 무시한다).
3. **`DragStarting` 을 진행 중 취소 수단 「예」로 적은 것 = 논리 공백** — 처음 끌기를 가로채는 길로 고쳤다.
- 리뷰어 독자 탐색: 끄는 동안 IPC · 저장을 얼리는 피어 · 신뢰할 만한 SendInput/PostMessage 취소 경로 · 끌기 중 WebView2 postMessage 실패 — 셋 다 1차 근거를 못 찾음. WebView2 스레딩 문서의 조건부 재진입 위험(메시지 처리기가 중첩 루프를 열 때)은 끌기 중 IPC 일반의 실패를 뜻하지 않는다.
- grounding(메인): Chromium `tab_drag_controller.cc:732 · :742 · :2128` · `tab_strip.cc:1399` · Zed `workspace.rs:12257` · Lumino `tabbar.ts:332 · :431 · :523` 원문 대조 — 지지.
