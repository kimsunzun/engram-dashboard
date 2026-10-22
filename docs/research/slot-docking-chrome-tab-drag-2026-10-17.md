# 조사 — 슬롯 도킹을 Chrome 식 탭 끌기로 (탭바 안 = 실시간 순서 이동 · 밖 = 떼기)

- **상태:** 초안 — 적대 리뷰 전(medium · 설계-결정 모드). 결정 대기(사용자).
- **날짜:** 2026-10-17
- **계기:** 사용자 요구 「크롬처럼 해」(2026-10-17). 지금 구현(TRD 5.6 · 7 단계 미커밋)은 끌기 시작부터 「떼어진 표시」(머리 접힘) + 끼울 자리 세로 막대이고, Chrome 은 탭바 안에서는 탭이 커서를 따라 움직이고 옆 탭이 비켜나며 탭바를 벗어나야 떼어진다.
- **방법:** 수집자 4(직접 피어 · 셸이 시작하는 OS 끌기 · 웹 쪽 기법 · 소스 직접 읽기 보강) → 메인 grounding(load-bearing 출처 직접 대조 2건) → cross-family 적대 리뷰 1회.
- **확신도 범례:** 확실 = 1차 출처 + 독립 교차확증 · 가능성 높음 = 1차 출처 하나(grounding 지지) · 불확실 = 미확인 · 추론.
- **앞선 조사:** [slot-docking-survey-2026-10-15.md](slot-docking-survey-2026-10-15.md) · [slot-docking-drag-mechanism-2026-10-16.md](slot-docking-drag-mechanism-2026-10-16.md)(HTML5 끌어 놓기 채택 · crabnebula drag 플러그인 · 셸 폴링 끌기 기각).

## 결론

1. **Chrome 자신의 웹 기술 탭바(터치 모드 WebUI 탭 스트립)가 우리와 같은 수단으로 이걸 한다** — HTML5 끌어 놓기 하나로 한 제스처를 끝까지 간다. 탭바 안에서는 `dragover` 마다 **실제 탭 요소를 그 자리로 DOM 이동**(`placeTabElement`)해 실시간 순서 이동을 보이고, 탭바를 벗어나면(`dragleave`) 그 요소를 숨기며(`setDraggedOut(true)`), 창 사이 이동 · 새 창은 브라우저의 네이티브 끌기가 탭 id(커스텀 데이터 형식)를 실어 처리한다. 순서 변경은 `drop` 때만 확정하고, 놓지 않고 끝나면 원래 자리로 되돌린다. — **가능성 높음**(소스 직접 대조 · 태그 120.0.6099.0 · 이후 main 에서 그 폴더는 지워졌다 — 지운 이유 미확인). 출처: `chrome/browser/resources/tab_strip/drag_manager.ts`(L66 · L122 · L193 · L302 setDragImage · L321 setDraggedOut · L496-500 HTML5 리스너) — https://chromium.googlesource.com/chromium/src/+/refs/tags/120.0.6099.0/chrome/browser/resources/tab_strip/drag_manager.ts
2. **포인터 기반 순서 이동 → 탭바 밖에서 OS 끌기로 갈아타는 웹 기반 선례는 찾지 못했다** — 불확실(부재의 증거가 아니라 미발견). 웹 쪽만으로 제스처 도중 OS 끌기를 시작하는 API 는 없다(HTML5 `dragstart` 는 네이티브 제스처에서만 · 시작되면 `pointercancel` 뒤 포인터 사건이 끊긴다) — 가능성 높음.
3. **셸이 OS 끌기를 대신 시작하는 기성품은 없다** — Electron `webContents.startDrag` 는 파일 경로만(`file`/`files` + `icon`) — 확실(문서 직접 대조). crabnebula `drag` crate 는 Windows 에서 `Data` 종류가 미지원(파일만) — 가능성 높음. 직접 만들면 Chromium 의 커스텀 데이터 형식(`"Chromium Web Custom MIME Data Format"` · pickle)을 실은 IDataObject 를 STA 스레드에서 `DoDragDrop` 하는 길이 원리상 열려 있으나, 웹뷰가 마우스를 쥔 제스처 도중에 되는지 · 다른 프로세스가 만든 pickle 을 WebView2 가 받는지는 **미검**(스파이크 필요). macOS 는 Electron 이 합성 NSEvent 로 `beginDraggingSession` 하는 선례가 있다 — 가능성 높음.
4. **VS Code 는 Chrome 식이 아니다** — HTML5 끌어 놓기 · 옆 탭에 CSS 테두리(삽입 표시)만 · 탭이 움직이지 않음 · 떼기는 `dragend` 때 「어느 창 위도 아니었나」로 판정 — 가능성 높음(소스 `multiEditorTabsControl.ts` L1297 · L1441-1456 · L1371-1405 · `dnd.ts` L785).

## 선택지

| | 방식 | Chrome 처럼 보이나 | 비용 · 위험 | macOS | 기존 코드(6a~7) 재사용 |
|---|---|---|---|---|---|
| **A** | **HTML5 하나 유지 · Chrome WebUI 탭바 꼴** — 자기 탭바 안에서는 머리를 접지 않고 `dragover` 마다 그 머리를 끼울 자리로 옮긴다(옆 탭은 CSS 로 미끄러짐) · 탭바를 벗어나면 그때 「떼어진 표시」 · 다시 들어오면 되돌림 · 자기 탭바에 놓으면 그 자리로 순서 변경 | 탭바 안 실시간 이동 = 예 · 끄는 동안 OS 끌기 그림은 탭바 안에서도 커서를 따라온다(Chrome WebUI 도 같다) | 작음~중간 · 프론트(TabBar · dockDrag 의 자기 탭바 처리) + 셸 판정(자기 탭바 놓기 · 멈춘 채 떼기 결함 고침) · 모두 기존 수단 | 같다(웹 표준) | 거의 전부 — 다른 창 대상 · 가이드 · 셸 장부 · 실행기 그대로 |
| **B** | 포인터 기반 순서 이동 + 탭바를 벗어나는 순간 셸이 OS 끌기 시작(커스텀 IDataObject · Chromium 커스텀 데이터 pickle · STA 스레드 · macOS 합성 NSEvent) | 가장 Chrome 에 가깝다(탭바 안에 OS 그림 없음) | 큼 · 미검 위험 셋(제스처 도중 시작 · 외부 pickle 수용 · 이벤트 루프 막힘 — 앞선 조사가 같은 이유로 플러그인을 기각) · platform 목적 함수 신설(ADR-0266 절차) | 따로 구현 | 다른 창 대상 · 셸 장부는 재사용 · 출처 쪽 · 시작 경로는 새로 |
| **C** | HTML5 하나 · 투명 끌기 그림 + 우리 창 안에서만 우리가 그린 그림 | 우리 창 안에선 예 | 작음 · 창 밖(바탕화면 · 다른 앱 · 창 사이 틈)에서는 끌기 그림이 아예 안 보인다 — PRD R2(끌기 그림) 후퇴 | 같다 | 거의 전부 |
| D | 지금 그대로(삽입 막대) | 아니오 | 0 | — | — |

- **추천 = A.** Chrome 자신이 웹 기술로 같은 문제를 푼 방식이고, 우리 수단(HTML5)과 같아 지금까지 만든 것을 버리지 않는다. 남는 차이는 「탭바 안에서도 OS 끌기 그림이 따라온다」 하나이고 그건 Chrome WebUI 탭바와 같다.
- **A 의 확인 거리(실측):** 끄는 도중 끌린 요소를 DOM 에서 옮겨도 끌기가 끊기지 않나(Chrome WebUI 가 그렇게 하고 · 우리 S1 ④ 에서 시작 뒤 머리를 접어도 306/307 이어졌다 — 끊기는 것은 `dragstart` 직후 누른 점에서 머리가 사라질 때뿐 · 그러니 옮기기는 시작 신호 뒤에) · 미끄러짐 애니메이션(FLIP)이 60/s `dragover` 에서 버벅이지 않나.

## 거부 후보 (ADR 거부 대안 재료)

- **B 를 지금** — 미검 위험 셋 · OS 함수 신설 · 앞선 조사가 같은 축(셸이 끌기를 몬다)을 기각했다. A 로 사용자 요구가 채워지면 열 이유가 없다.
- **C** — 창 밖 끌기 그림이 사라져 PRD R2 를 후퇴시킨다.
- **Electron `startDrag` 꼴** — 파일만이라 탭 id 를 못 싣는다(우리는 Tauri 라 애초에 없다).
- **crabnebula `drag` 의 `Data`** — Windows 미지원.

## 공백 · 한계

- 직접 피어 서베이가 얇다 — Hyper · Theia · Min · Notion · Discord · Slack · Figma · Arc 미도달. 웹 기반 앱 중 Chrome 식을 소스로 확인한 것은 Chromium WebUI 탭바 하나.
- Chromium WebUI 탭바의 「아무 창 밖에 놓기 → 새 창」 네이티브 호출 경로 미확인(`DropTabsInNewBrowser` 의 호출자 미발견) · 미끄러짐 CSS 미확인 · 그 폴더가 main 에서 지워진 이유 미확인.
- B 의 세 위험은 문헌으로 못 닫는다 — 고를 경우 스파이크가 먼저.
