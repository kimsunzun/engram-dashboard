# 구조 점검 — 슬롯 도킹 끌기 코드 (9c 작업 트리 · 2026-10-18)

- **상태:** 계측 완료(읽기 전용 점검 1 회 · 리뷰어 프리셋) · 재설계 방향 결정 대기 — 짝 조사 = [../research/slot-docking-chrome-desktop-drag-2026-10-18.md](../research/slot-docking-chrome-desktop-drag-2026-10-18.md).
- **계기:** 사용자 「코드 대충짜지말고 재활용할거 재활용하고 함수화할거 함수화하고 잘묶어주고 해야된다. 이거 뭔가 불안하네」(2026-10-18 · 9c 마우스 실측 뒤).
- **범위:** `src/components/layout/{TabBar,WindowLayout}.tsx` · `tabStripLayout.ts` · `src/components/layout/dock/*` · `src-tauri/src/dock/*` · `src-tauri/src/layout/chrome.rs`(시험 파일은 구조 문제만).
- **줄 번호는 9c 미커밋 트리 기준** — 커밋 · 재설계 뒤엔 어긋난다.

## 한 줄 결론

순수 기하(도킹 띠 · 탭바 폭 · 레이아웃 적용)는 살아남는다. 코드 양의 대부분 — 셸 장부 `book.rs`(운영 약 2,660 줄)와 `dockDrag.ts`(약 1,230 줄) — 은 HTML5/OLE 끌기 사건의 경주를 맞추려고 있는 것이라, Chrome 식으로 가면 고쳐 쓰는 게 아니라 새로 짠다.

## 결함 (유지보수에 아픈 순)

1. **`createDockDrag` 가 거대한 클로저 하나** — `dockDrag.ts:318-1119` · 가변 변수 약 35 개 + 스냅숏/알림 쌍 4 개 · 관심사 여덟(세션 기록 접기 · 대상 판정과 보고 · 호버 전환 타이머 · Esc 삼키기 · 포인터로 끌기 신호 유실 감지 · 출처 모드와 떠남 타이머 · 순서 이동 애니메이션 상태기 · 파일 드롭 막기). 9c 가 출처 쪽 상태기 전체(L462-605)를 이 대상 쪽 컨트롤러에 넣었다 — 파일 머리는 출처 일이 `TabBar` 몫이라고 적는데.
2. **「그린 것」 보고 모양이 약 7 번 되풀이** — Rust `DragOverReport::Over` · `DropReport` · `DrawnReport`(`types.rs:265-318`) + `book::Report` 와 똑같은 `From` 둘(`book.rs:113-146`) · TS 손조립 셋(`dockDrag.ts:708-716` · `907-928` · `992-1000`).
3. **`EndLog` 가 `EndCause` 를 칸마다 베낀다** — 16 칸(`book.rs:395-444` 대 `787-827`) · `2182-2212` 에서 또.
4. **늘 0 인 스크롤 배관** — 운영 코드 11 파일 약 120 곳(`DockChrome` · `TabBarRegion` · `ReorderRegion` · `StripStart` · 보고 셋 · `Report` · `DockGuide` · `ChromeMeasure` · `Drawn` 의 `scroll` · `dockTabBarScrolled` · `onScroll` · `same_bar` · `sameChrome` · `pick(scroll)`). 5.8판이 「잠듦 — 배관만 남긴다」로 둔 것(TRD §7 #64).
5. **TS ↔ Rust 거울 논리에 지킴이가 셋 다 다르다** — 판정 규칙(`dockPick.ts` ↔ `hit.rs`) = 골든 JSON · 시한 상수 = Rust 소스를 정규식으로 읽는 시험 · 새 창 크기(`dragImage.ts:28` ↔ `POPOUT_DEFAULT_SIZE`) = 주석뿐 · `displayedTabOf` ↔ `visible_tab` = 없음. Rust 안에서도 `book::display_active`(`book.rs:2611`)가 `regions::visible_tab`(`regions.rs:75`)를 다시 짠다.
6. **탭바 DOM 재기가 둘** — `TabBar.measureStrip`(`101-125`) · `WindowLayout.measureDockChrome`(`321-335`) + `dockChromeOf`/`contentRect`(`dockChromeReport.ts:50-70`) · `rectOf`(`TabBar:127`) — 다 `data-testid` 로 고른다. `measureDockChrome` 은 transform 을 빼지 않아 `WindowLayout` 에 그것만 피하려는 ref 둘(`reorderStandingRef` · `seenChrome` — L76-83 · L217-227)이 붙었다.
7. **순서 이동 기하가 네 곳** — 거울 · 골든으로 묶인 `hit.rs:319-346` · `dockPick.ts:77-98` + 시험 없는 `dockDrag.ts:521-531` · `TabBar.tsx:140-155`(「`[index, dragged)` 이웃이 한 폭씩 밀린다」를 암묵으로).
8. **한 번에 하나만 나가는 보고기 복제** — `dockChromeReport.ts:72-205` ≈ `windowCanvasReport.ts` · 「같은 값」이라 적은 100 ms 디바운스 상수 둘(`WindowLayout.tsx:345` · `dockChromeReport.ts:42`).
9. **작은 것** — `hit::WindowFrame` 의 `inner_origin` · `scale` · `label` 은 시험만 쓴다(운영은 늘 0 · 1 · "" — `regions.rs:190`) · `pick` 이 `DockRegions` 로 `DockChrome` 을 다시 만들어 `locate` 를 부른다 · 안 쓰는 `dragImage` `slotLabel` · 「이 창이 도킹 모드의 출처」 술어 세 번(`dockDrag.ts:187` · `:464` · `WindowLayout.tsx:97`) · 손으로 짠 같음 검사 여섯 · 매직 넘버(28 px 탭바 높이가 셋 · `TAB_MIN_ACTIVE_W = 42` 는 패딩에서 손 유도) · 시험: `book.rs` 안 약 3,900 줄 · `dockDrag.test.ts` 1,523 줄이 OLE 시각 경주를 박는다 — 재설계로 넘어갈 시험은 `pick_golden.json` · `hit.rs` 기하 · `tabStripLayout` 뿐.

## 분류 — Chrome 식 재설계에서

| 조각 | 판정 | 이유 |
|---|---|---|
| `layout::apply::dock_tab` · `DockDest` · `DockGuard` · `lease_tab` · `docked_tree` · `incoming_rect` · `succession` | 유지 | 모델 변경 · 지킴 — 입력이 어떻게 오든 같다 |
| `hit.rs` 도킹 띠(`outer_strips` · `valid_bands` · `landing_rect` · `choose_band` · `choose_seam`) | 유지 | 순수 기하(단 붙는 영역 < 미리보기로 나눔 — 조사 C) |
| `hit.rs` `locate` 탭바 쪽 · `reorder_zone` · `REORDER_SLACK_PX` | 유지 | `REORDER_SLACK_PX` 15 = Chrome `kVerticalDetachMagnetism` · 놓기 전 붙기는 화면 좌표로 판정하므로 `WindowFrame` 원점 · 배율이 다시 쓰인다 — 지우지 말고 쓸 것 |
| `regions.rs` `regions_for` · `slot_regions` · `outer_bands` | 유지 | 창별 대상 만들기 |
| `regions.rs` `rev` · `bar_rev` · `bumps` · `Expect` | 불명 | 「비동기 보고 뒤 그린 것 확인」용 — 한 프로세스가 정하면 줄 수 있다 |
| `snap.rs` · `DockState` | 유지 | 레이아웃 락 아래 스냅숏 · 세션 세대 |
| `commands/dock.rs` 포트(`DockPorts` · `DockSpawn` · 메인 스레드 숨기기/보이기 맞추기) | 유지 | 시험 가능한 seam · 커서를 따라오는 창에도 필요 |
| `commands/dock.rs` 명령 집합(`dock_drag_over` · `drop` · `end` · `pointer_seen` · `pull`) | 걷음 | HTML5 사건 프로토콜 |
| `book.rs` | 걷음(약 85%) | 기다림 단계 · 엔진 취소 · 경주 타이머 여섯 — 세션 id · 대여 · 한 번 적용 · 끝 로그 뼈대(약 15%)만 재사용 |
| `types.rs` `PxRectWire` · `Band` · `OuterBand` · `SlotRegion` · `Picked` | 유지 | 기하 wire |
| `types.rs` `DragOverReport` · `DropReport` · `DrawnReport` · `DragEffect` · `StripStart` | 걷음 | OLE 보고 프로토콜 |
| `layout/chrome.rs` · `dockChromeReport.ts` | 유지 | 놓기 전 붙기에 다른 창 탭바 사각형이 필요 |
| `tabStripLayout.ts` | 유지 | Chromium 폭 · 숨김 규칙 |
| `DockOverlay.tsx` | 대부분 유지 | 이음새 · 띠 그리기 · 끼울 자리 막대는 진짜 끼움으로 대체 |
| `dockPick.ts` | 불명 | 거울이 걷힐 수 있다 — 탭바 안 = TS 만 · 창 사이 = Rust 만 |
| `dockDrag.ts` 듣개 · Esc 삼키기 · 포인터 봄 · `lastDrawn` · 떠남 타이머 | 걷음 | HTML5 사건 · 경주 |
| `dockDrag.ts` 파일 · URL 드롭 막기(`272-277` · `833-844` · `877-881`) | 유지 | 늘 필요(AC22) — 파일과 함께 지우지 말고 떼어 낼 것 |
| `dragImage.ts` | 걷음 | 새 모델엔 OS 끌기 그림이 없다 |
| `TabBar.tsx` | 쪼갬 | 이름 바꾸기 · 배치 = 유지 · `draggable` · `dragstart`/`dragend` · `ENGINE_CANCEL_MS` · `setTimeout(0)` 세션 열기 = 걷음 · `reorderStyle` = 개념 유지 |
| `WindowLayout.tsx` 도킹 배선 | 불명 | 훅으로 뺄 것 — 내용은 재설계가 정한다 |

## 제안 모듈 구조 (재설계가 내려앉을 곳)

- **Rust `src-tauri/src/dock/`** — `geometry.rs`(도킹 띠 · 미리보기 · 이음새) · `strip.rs`(끼울 자리 · 붙기 자리 · 떼기 여유) · `targets.rs`(스냅숏 + 크롬 → 창별 대상 · `target_at(화면 점)`) · `session.rs`(누름 → 순서 이동 → 떼어져 따라옴 → W 에 붙음 → 끝 — 사건 출처 하나) · `follow.rs`(커서를 따라오는 창 포트 — 만들기 · 옮기기 · 보이기/숨기기) · `exec.rs`(포트 · 메인 스레드 맞추기 · 적용 — 얇아진 `commands/dock.rs`) · `wire.rs` · 변경 경로는 `layout::apply` 하나(놓기 전 순서 이동 · 붙기용 `move_tab` 추가).
- **TS `src/components/layout/tabStrip/`** — `layout.ts`(지금 `tabStripLayout`) · `measure.ts`(transform 뺀 DOM 재기 하나) · `reorderGeometry.ts`(따라감 · 자리 · 끼울 x · 이웃 밀림) · `useTabDrag.ts`(문턱 · 포인터 캡처 · 진짜 요소 이동 · 떼기 여유 · Esc — 셸과는 떼기 · 붙기 · 끝에만 말한다).
- **TS 그 밖** — `TabBar.tsx`(그리기 · 이름 바꾸기만) · `dock/dockSession.ts` · `dock/DockOverlay.tsx`(셸이 민 안내를 그림) · `dock/chromeReport.ts`(`util/latestValueReporter.ts` 위) · `dock/fileDropGuard.ts`(늘 켜짐) · `useDockWindow.ts`(`WindowLayout` 에서 뺀 배선).
