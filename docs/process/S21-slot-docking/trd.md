# TRD — 슬롯 도킹 (탭을 끌어 다른 창 레이아웃에 붙이기)

> **5.4판 — 리뷰 4라운드 반영**(2026-10-16): R22 재사용은 출처 창의 모델에 보통 자리가 적혀 있을 때만 — 없으면 새 창(Create) 길로(되돌릴 자리가 없어 반쯤 옮긴 창이 남는 것을 막는다 · `plan_new_window` 고침 = 4 단계).
>
> **5.3판 — 리뷰 3라운드 반영**(2026-10-16 · doc-aware PASS · blind FIX — 둘 다 R22 재사용 되돌리기): 되돌릴 자리를 게터가 아니라 phase A 에서 락 안에 복사한 모델 자리(보통 자리 · 최대화)로 · 창 포트 세터가 결과를 돌려주고 옮기기는 첫 실패에서 멈춰 되돌리며 되돌리기는 실패해도 이어 감 · 최대화였던 출처는 최대화 풀기(tao `SW_RESTORE`)가 창을 보이고 포커스를 가져가는 것을 받아들인 경우로 적고 S1 ⑤ 에서 잼 · 숨긴 채 푸는 platform 함수는 미결.
>
> **5.2판 — 리뷰 2라운드 반영**(2026-10-16 · 1라운드 지적은 둘 다 해소 판정 · 새 FIX): 탭바 놓기는 이음새와 따로 대조(탭바 몫 `bar_rev` · 보이던 탭은 이음새만 — 탭 전환 중에도 탭바 놓기가 선다) · R22 재사용(탭 하나 팝아웃 옮기기)에 옮긴 뒤 같은 락 안의 재확인과 되돌리기 · `Apply` 는 늘 async 런타임에서(동기 명령 안에서 창을 만들지 않는다) · 붙들다 버린 id 의 늦은 시작은 거절 · 세션 깃발은 세대로 비교해 내림 · 다른 인스턴스 끌기 문구 · 표식이 내려가기 전 틈의 시작 거절을 받아들인 남은 틈으로 적음.
>
> **5.1판 — 리뷰 1라운드 반영**(2026-10-16 · 적대 리뷰 둘 = FIX · 구조는 그대로): 놓기 대조에 대상 창의 보이는 탭을 더함(웹뷰가 보이던 탭 · 모델 기대값에 창 활성 — 4 단계 고침 둘) · 「가이드 없음 = 새 창」을 「보인 것 · 지금 판정 둘 다 없음」으로 좁히고 그 밖의 어긋남 · 대상 사라짐은 무변경 · 영역 · 세션 사건의 순서(세션을 몰라도 우리 MIME 받기 · `drag_id` 별 최대 `rev` · `seq` · 시작보다 먼저 온 끝 신호 붙들기) · 적용 중(`Applying`)의 스냅숏 · 소멸은 적기만 하고 끝은 `Applied` 가 정함 · 스냅숏 계기 = 쓰기의 알림 묶음 끝 한 번 · react-arborist 의 window 끌기 리스너가 탭 끌기를 막는 것을 피함 · 창별 마지막 Over 와 떠남을 따로 적음 · 그 밖 아홉(끝 증거 = 버튼 0 이동 · 적용 중 새 시작 · 커서 못 읽음 · `rev` 올리는 때 · 게터는 락 밖 · 늦은 replay 문구 · 계측 · 세션 깃발 자리 · Esc 삼키기 상한). 3 단계 커밋(`281bd235`) 반영.
>
> **5판 — 끌기 수단을 브라우저 내장 HTML5 끌어 놓기로 바꿨다**(사용자 결정 2026-10-16 · 조사 [slot-docking-drag-mechanism-2026-10-16.md](../../research/slot-docking-drag-mechanism-2026-10-16.md)): 셸 폴링 틱 · OS 함수 · 딱지 창을 걷고, 셸은 영역 계산 · 놓기 판정 · 적용만 한다. 같은 날 결정 넷(끌기 그림 = Windows 기본 · 비정상 끝 = 마지막 점에 놓기 · 메인 마지막 탭 끌기 · LLM 명령 미룸)을 함께 반영했다. 뺀 것 = §11-3.

| 항목 | 값 |
|---|---|
| **상태** | 5.4판 — 리뷰 4라운드 반영(재리뷰 전) · 5.3판 = 리뷰 3라운드 반영 · 5.2판 = 리뷰 2라운드 반영 · 5.1판 = 리뷰 1라운드 반영 · 5판 = 수단 교체로 다시 씀 · 사용자 확인 8건 = 전부 기본값 확정(2026-10-16 · §11-1) · 남은 실측 6건(§8-2) |
| **날짜** | 2026-10-16 |
| **스텝** | S21 |
| **브랜치** | `v0.4.1/feat/slot-docking` |
| **PRD** | [prd.md](prd.md) 4.4판 — R1–R22(R16 · R17 = 다음 작업 · R20 없음) · AC1–AC9 · AC11–AC16 · AC18–AC22 · Q1–Q18 · D4 · D6–D10 · D12–D14 · W1 · W3–W5 · W7–W12 |
| **근거** | 수단 조사 · 실측(위 조사 §3 · §4 · 스파이크 기록 `.claude/handoff/attachments/spike-html5dnd-2026-10-16.md`) · [slot-docking-survey-2026-10-15.md](../../research/slot-docking-survey-2026-10-15.md) §5(코드 실태) · §7–§8(관례 · 크기) |
| **줄 포인터 기준** | HEAD `281bd235`(3 단계까지 커밋). 심볼 이름이 정본이고 줄은 보조다. 의존 crate 줄은 `~/.cargo/registry` 의 잠긴 판(tauri 2.11.3 · tauri-runtime-wry 2.11.3 · wry 0.55.1) · npm 줄은 `node_modules` 의 설치 판(react-arborist 3.9.0 · react-dnd 14.0.5 · react-dnd-html5-backend 14.1.0) |

## 0. 결론 요약

- **세 층.** ① 레이아웃 연산(셸 `layout` — 1 · 2 단계 끝 · 4 단계 적용 서비스) ② 끌기 장부(셸 `dock` — 순수 장부 + 잎 락 하나 · 스레드 · 틱 없음) ③ 웹뷰(HTML5 끌어 놓기 — 엔진이 커서 추적 · 맨 위 창 · 끌기 그림 · 버튼 · Esc 를 맡는다). OS 층(`platform`)에 새 함수는 없다(§1-6).
- **탭을 끌면 떼어진다 — 단 배치는 놓을 때 한 번만 바뀐다**(PRD R2 · R11a). 떼어진 모습은 웹뷰의 그리기이고, 끄는 동안 모델을 바꾸는 유일한 것은 R6b 탭 전환(사람 탭 클릭과 같은 쓰기)이다.
- **셸이 영역을 미리 계산해 창마다 보낸다.** 웹뷰는 dragover(움직일 때 60/s)마다 점이 어느 사각형인가만 보고 가이드를 그린다 — 셸 왕복 없이 가이드가 커서를 따른다. 보인 것이 바뀔 때만 셸에 알린다.
- **놓기 판정은 셸이 한다.** 놓은 창 · 점 · 보인 것 · 그 창이 보이던 탭을 받아 같은 규칙(`dock::hit` — 웹뷰의 TS 판은 골든 파일로 묶은 거울)으로 다시 판정한다: 보인 것 · 지금 판정이 둘 다 「가이드 없음」 = R22 새 창 · 같은 이음새면 영역이 최신이고 보이던 탭이 같을 때만 · 같은 탭바 자리면 탭바 몫(`bar_rev`)이 최신일 때만 적용(5.2 — 탭 전환 중에도 탭바 놓기가 선다) · 그 밖의 어긋남(대상이 사라짐 포함)은 무변경(M1 · R13c · Q18). 모델도 같은 락 안에서 대상 배치 · 대상 창 활성(이음새) · 탭 목록(탭바)을 다시 본다(5.1).
- **끝 신호 셋.** 대상 창 `drop`(우리 창 위에서 뗌) · 출처 `dragend` 의 `none`(창 밖에서 뗌 · Esc · 시스템이 끊음) · 끄는 중 어느 창이 본 `pointermove`(끝 신호 유실). 뒤 둘은 「마지막으로 알려진 점에 놓기」다(PRD R12 · R13b): 셸 커서가 마지막으로 끌기를 본 우리 창(W) 안이면 W 의 마지막 보고로 판정하고, W 가 그사이 사라졌으면(가이드를 보이던 중이면) 무변경, 그 밖은 그 자리에 새 창. 그래서 Esc 와 창 밖 떼기를 키 상태로 가를 필요가 없다(§1-3 「끝 신호」).
- **사건 순서는 보장되지 않는다(5.1).** emit · invoke 가 여러 스레드에서 나가므로 웹뷰는 세션을 몰라도 우리 MIME 을 받고 `drag_id` 별 가장 큰 `rev` · `seq` 만 쓰며, 장부는 시작보다 먼저 온 끝 신호를 잠깐 붙들고 적용 중(`Applying`)에는 끝을 `Applied` 하나에 맡긴다(§1-3).
- **★에이전트 트리(react-arborist)가 window 에 단 끌기 리스너가 탭 끌기를 막는다★** — 우리 MIME 사건은 `document` 에서 끊어 window 에 닿지 않게 하고, 셸 세션은 `dragstart` 가 취소되지 않았을 때만 연다(§1-4 「다른 끌기 리스너」 · S1 ⑦).
- **메인 마지막 탭도 끈다.** 떠나면 같은 연산 안에서 메인에 빈 탭 하나 — `close_tab` 의 기존 규칙과 같은 도우미(ADR-0057 ④ · 4 단계).
- **닫히는 창 = 모델 먼저 지우고 묘비**(M2 · 3 단계 `281bd235` — 그대로).
- **LLM 명령은 없다 — 의식적 예외**(PRD Q10). 적용 서비스 `apply::dock_tab` 은 마우스 놓기가 쓰므로 남고, 다음 작업이 그 위에 명령을 얹는다(§3).
- **남은 실측 여섯(§8-2 S1–S6)** — S1 끝 신호 순서(+ 트리 슬롯 · 사건 순서 — 5.1) · S2 끌기 그림 내용 · S3 바탕화면 · 파일 드롭 · S4 혼합 DPI · S5 콘솔 세션 · S6 제목줄 끌기. ★불합격이면 조용히 물러나지 않고 멈춘다★.

## 1. 아키텍처

### 1-1. 층과 흐름

```
웹뷰(출처 창)  탭 머리 draggable · dragstart(동기): setData("application/x-engram-tab", {drag_id, view_id}) · effectAllowed=move
               · setDragImage(끌기 그림, hotspot) · stopPropagation(window 의 react-dnd 에 안 닿게)
               → 디스패치 뒤(setTimeout 0) 취소되지 않았으면 invoke dock_drag_start{drag_id, view_id, hotspot}
               dragend → invoke dock_drag_end{drag_id, effect}
웹뷰(모든 창)  report_dock_chrome{canvas_origin, tab_bar, tabs[]} — 평소 보고(측정 · version 불변)
               listen "dock:session"(Started · Display · Ended — 떼어진 표시 · Esc 삼키기) · "dock:regions"(자기 창 영역)
                 — drag_id 별 가장 큰 seq · rev 만 · Started 전에 온 것도 받는다
               dragenter · dragover(우리 MIME 이면 세션을 몰라도 받음 · stopPropagation) → pick(점, 영역) → 가이드 그리기
                 · shown · rev · 보이던 탭이 바뀌면 invoke dock_drag_over{Over{rev, bar_rev, displayed, shown, point}}
               dragleave(창 밖으로) → invoke dock_drag_over{Left} · drop → invoke dock_drop{rev, bar_rev, displayed, shown, point}
               탭 머리 위 머묾 500ms → 기존 탭 전환(사람 클릭과 같은 길 · R6b)
               세션 중 첫 pointermove(마우스 · 버튼 0) · pointerdown → invoke dock_pointer_seen (끝 신호 유실 대비)
               우리 MIME 이 아닌 끌기(OS 파일) → dropEffect none + drop preventDefault (탐색 막기 · 늘 · 끊지 않는다)
셸 commands/dock.rs ─ invoke 껍데기 · 실행기(emit · 숨기기 · 보이기 · 커서 · 창 사각형 읽기(락 밖) · 시한 · set_focus · 적용 호출)
     └ dock::book::Book::step(사건, 스냅숏) → 할 일   ← 순수 · 잎 락 하나
          영역      = dock::regions::regions_for(창 스냅숏)   (hit::valid_bands · outer_strips · tab_insert_x)
          판정      = dock::regions::pick(영역, 점) = hit::locate + hit::choose_seam   (TS 거울과 골든 공유)
          놓기      = layout::apply::dock_tab(DockGuard{표식, 기대값 — 배치 · 창 활성 · 탭 목록}, Reveal)
셸 layout (권위 — ADR-0035) ─ dock.rs · ViewManager::{dock, plan_new_window, lease_tab, release_lease, mark_closed} · apply::dock_tab
셸 계기 ─ 쓰기 하나의 알림 묶음 끝(LayoutEvents::settled) · 크롬 · 캔버스 보고 뒤(세션 깃발이 섰을 때만 스냅숏 → 장부) · on_window_event Destroyed
셸 WebviewEnv::finish ─ disable_drag_drop_handler() — 모든 런타임 창(wry 의 드롭 대상을 떼어 WebView2 내장 끌어 놓기를 살린다)
platform ─ 새 함수 없음
```

| 무엇 | 어디 | 상태 · 비고 |
|---|---|---|
| 트리 연산 `docked_tree` · `DockSide` · `DockAnchor` · 새 면적 0 · `incoming_rect` | `src-tauri/src/layout/dock.rs` | 끝(1 단계 `080893c5`) |
| 도킹 모델 `dock` · `plan_new_window` · 표식 · 묘비 · `DockGuard` · `TargetExpect` · `FocusPolicy` · `DockError` · `succession` | `src/layout/manager.rs` | 끝(2 단계 `ca723ed5`) · 4 단계가 메인 마지막 탭 규칙과 `TargetExpect` 의 Seam 칸(대상 창 활성 — 5.1)을 고친다(§1-2) |
| 닫히는 창(M2) — 닫기 요청에서 모델 먼저 + 묘비 · 닫히는 창의 늦은 Subscribe 막기 | `src/commands/popout.rs` `close_requested_popup_window`(:255) · `drop_window_in_model`(:282) · `lib.rs` `CloseRequested`(:302) · `daemon_client/connection.rs` `RequestReplay` 갈래(:1166) | 끝(3 단계 `281bd235`) |
| 적용 서비스 `dock_tab` · 창 포트 `open_at` · `move_window`(★기본 본문 없음★) | `src/layout/apply.rs` · `commands/popout.rs` `TauriWindowHost`(:66) · `commands/layout.rs` `OwnedWindowHost`(:172) · 시험 가짜 넷(`tests/layout_apply.rs` · `tests/layout_commands.rs` · `daemon_client/tests.rs` 두 곳) | 4 단계 |
| `placement::place` · `land` 공개(`pub(crate)`) | `src/state/placement.rs`(:446 · :470) | 4 단계 — 새 창 자리 |
| 판정 `locate` · `choose_seam` · `choose_band` · `valid_bands` · `landing_rect` · `outer_strips` · `tab_insert_x` | `src/dock/hit.rs` | 끝(5 단계 `e73e0289`) |
| 크롬 측정 타입 `DockChrome` · `PointCss` · `RectCss` · `TabHeader` | `src/layout/chrome.rs` | 끝(5 단계) · 6 단계가 `WindowTabs` 칸과 보고를 짓는다 |
| 영역 `regions_for` · 판정 `pick` · wire 타입(`DockRegions` 등 — `GuideView` · `TabBarHint` 를 대신한다) · 골든 | `src/dock/regions.rs`(신설) · `src/dock/types.rs` · `src/dock/pick_golden.json`(신설) | 6 단계 |
| 끌기 장부 | `src/dock/book.rs`(신설 · 순수) | 6 단계 — 지운 4판 `dock/session.rs`(8ms 틱 상태 기계 · 커밋된 적 없음)를 대신한다(§11-3) |
| invoke 여섯 + `report_dock_chrome` · 실행기 | `src/commands/dock.rs`(신설) · `lib.rs` `generate_handler!`(:360) · `on_window_event` `Destroyed`(:334) | 6 단계 |
| 알림 묶음 끝 `LayoutEvents::settled`(신설 · 기본 본문 없음 — 구현자 아홉) | `src/layout/apply.rs` `LayoutEvents`(:88) · 알림 자리 `notify`(:162) · `rename_tab`(:432) · 슬롯 옮기기(:766) · `state/restore.rs` 알림 루프(:818) · `dock_tab` | 4 단계(빈 몸 — 6 단계가 `TauriEvents` 에 계기를 단다) |
| 모델 · 측정 변경 계기 | `commands/layout.rs` `TauriEvents::settled`(:57 — 소유형 `OwnedEvents` 도 이것으로 넘긴다) · `report_window_canvas`(:562) · `report_dock_chrome` | 6 단계 |
| 드롭 처리기 끄기 | `src/webview_env.rs` `WebviewEnv::finish`(:73) | 6 단계 |
| 프론트 | `TabBar.tsx` · `WindowLayout.tsx` · `DockOverlay.tsx` · `dockDrag.ts` · `dockPick.ts` · `dragImage.ts` · `dockChromeReport.ts`(뒤 다섯 신설 · 전부 `src/components/layout/`) | 7 단계 |

`dock` 모듈을 `layout` 밑에 두지 않는 이유: 장부는 레이아웃 권위가 아니라 그 권위를 부르는 손님이다 — 놓기도 적용 서비스를 부를 뿐이다. 섞으면 「레이아웃 변경은 적용 서비스 하나」가 흐려진다.

### 1-2. ① 레이아웃 연산 (OS 무관 · 셸 `layout`)

#### 트리 연산 — `layout/dock.rs` (1 단계 · 끝)

```rust
pub enum DockSide { Left, Right, Top, Bottom }      // serde rename 없음 — wire 철자 = 이름(types.rs 맞대기 시험)
pub enum DockAnchor { Slot(Uuid), Outer }           // Outer = 대상 탭 레이아웃 전체(루트)
pub const DOCK_NEW_SHARE: f64 = 1.0 / 3.0;          // R7 — 새것의 몫
/// 대상 트리에 끼운 **결과 트리**를 새로 만든다(제자리 변경 아님 — 영역 · 판정 · 적용이 같은 함수를 쓴다).
pub fn docked_tree(target: &LayoutNode, anchor: DockAnchor, side: DockSide, incoming: &LayoutNode)
    -> Result<LayoutNode, DockTreeError>;
pub enum DockTreeError { SlotNotFound(Uuid), SlotIdOverlap, ZeroArea }   // 판정 순서 = 기준 칸 → 겹침 → 면적
pub fn incoming_rect(result: &LayoutGeometry, incoming_slots: &[Uuid]) -> Option<RectF64>;  // 가이드의 「정확한 사각형」(R4)
```

- **분할 모양** — 기준 노드(슬롯 하나 또는 루트)를 새 분할 `Split{id: 새 UUID(ADR-0223), dir, ratio, a, b}` 로 감싼다.

  | side | dir(ADR-0140) | a | b | ratio(= a 몫) |
  |---|---|---|---|---|
  | Left | `LeftRight` | 새것 | 기준 | 1/3 |
  | Right | `LeftRight` | 기준 | 새것 | 2/3 |
  | Top | `TopBottom` | 새것 | 기준 | 1/3 |
  | Bottom | `TopBottom` | 기준 | 새것 | 2/3 |

  둘 다 `tree::RATIO_MIN`–`RATIO_MAX` 안이라 클램프에 걸리지 않는다. 정규화 좌표라 감싼 기준 노드 안쪽 비율은 그대로다 — 슬롯 옆은 그 슬롯만 2/3 로, 바깥 변은 기존 레이아웃 전체가 안쪽 비율을 지킨 채 2/3 로 준다(R7 · AC4 · AC5).
- **슬롯 id · 분할 id 는 바꾸지 않는다** — 출처 탭의 노드를 그대로 옮긴다(ADR-0223 「분할 id 는 노드를 따라간다」).
- **새 면적 0(R8)** — 결과 트리를 `geometry::compute` 로 재고, 결과에서 면적 0 인 칸 중 **원래 자리에서는 면적이 있던 칸**이 하나라도 있으면 `ZeroArea`. 원래부터 0 이던 칸은 빼고 본다(`set_split_ratio` ⑤ 와 같은 태도). 결과 트리 전체를 재므로 깊은 자식까지 선다. 좌표는 정규화 레이아웃 좌표다.

#### ViewManager — `manager.rs` (2 단계 · 끝 — 4 단계 고침 하나)

```rust
pub enum DockTo {
    Seam { target: ViewId, anchor: DockAnchor, side: DockSide },   // R7
    TabBar { window: WindowLabel, index: Option<usize> },          // R21 — index = 결과 탭 목록의 자리 · None = 맨 뒤
    NewWindow { label: WindowLabel },                              // R22 — 부르는 쪽이 이미 연 OS 창 label(phase C)
}
pub struct DockGuard { pub lease: Option<LeaseId>, pub expect_source: Option<LayoutNode>, pub expect_target: Option<TargetExpect> }
pub enum TargetExpect { Seam { layout: LayoutNode, active: ViewId } /* Seam — 대상 탭 배치 + 대상 창 활성(4 단계 · 5.1) */,
                        Tabs(Vec<ViewId>) /* TabBar — 대상 창 탭 목록 */ }
pub enum FocusPolicy { Keep /* 다음 작업의 명령 */, Reveal /* 마우스 — 옮긴 것을 보이고 첫 슬롯에 포커스 */ }
pub struct Docked { window: WindowLabel, slot_ids: Vec<Uuid>, source_window: WindowLabel, source_window_closed: bool }
pub enum DockError {
    TabNotFound(Uuid), WindowNotFound(String), SlotNotFound(Uuid), SlotNotInTab { slot: Uuid, tab: Uuid },
    SelfTarget, ZeroArea, Dragging, WindowClosed(String),
    StaleLease, SourceChanged, TargetChanged,             // ← 사전조건을 단 요청만
    WindowExists(String), SlotIdOverlap,                  // ← 닿지 않는 방어
    WindowOpen(String),                                   // ← 적용 서비스(새 창 열기 실패)
}                                                         // 4 단계 뒤 모양 — 2 단계의 MainLastTab 을 지운다(아래)
pub fn dock(&mut self, view: ViewId, to: DockTo, guard: &DockGuard, focus: FocusPolicy) -> Result<Docked, DockError>;
pub fn plan_new_window(&self, view: ViewId, guard: &DockGuard) -> Result<NewWindowPlan, DockError>; // R22 phase A
pub enum NewWindowPlan { Reuse { label: WindowLabel }, Create { size: Option<(f64, f64)> } }
pub fn lease_tab(&mut self, view: ViewId) -> Result<LeaseId, DockError>;  // 끌기 시작
pub fn release_lease(&mut self, id: LeaseId);                             // 끌기 끝 — 다른 표식이면 무동작
pub(crate) fn mark_closed(&mut self, label: &str);                        // 묘비(3 단계가 부른다)
pub fn succession(tabs: &[ViewId], removed: ViewId) -> Option<ViewId>;   // 활성 승계 — 떼어진 표시도 같은 규칙
```

**판정 순서가 계약이다**(앞에서 걸리면 뒤는 안 본다 · 걸리면 무변경 — `manager.rs:9`):

1. 출처 View 가 있나 → `TabNotFound`.
2. 목적지 — Seam 의 대상 View 없음 = `TabNotFound` · TabBar 의 창 없음 = `WindowNotFound`(대상 창에 묘비가 있어도 같은 사유) · NewWindow 의 label 에 묘비 = `WindowClosed` · 그 label 이 이미 모델에 있음 = `WindowExists`(닿지 않는 방어).
3. Seam 의 대상 = 출처 → `SelfTarget`(TabBar 의 대상 창 = 출처 창은 자리 옮기기로 허용 — R21).
4. Seam 의 기준 슬롯이 대상 탭에 있나 — 다른 탭이면 `SlotNotInTab`, 어디에도 없으면 `SlotNotFound`.
5. 표식 — 출처에 표식이 섰는데 `guard.lease` 가 없으면 `Dragging`. `guard.lease` 가 지금 표식과 다르거나 표식이 없으면 `StaleLease`(지난 끌기의 늦은 놓기).
6. `expect_source` 와 다르면 `SourceChanged` · `expect_target` 과 다르면 `TargetChanged`(R13c — Seam = 대상 탭 배치와 대상 창 모델 활성 · TabBar = 대상 창 탭 목록).
7. Seam 이면 `docked_tree` → `SlotIdOverlap`(닿지 않는 방어) · `ZeroArea`.

적용(전부 `&mut self` 안 — 락은 부르는 쪽 한 번 · `bump_version` 한 번):

- **Seam** — 대상 View 의 `layout` 을 결과 트리로 바꾼다. **모르는 내용 곁표**(`unknown_content`)의 출처 항목을 슬롯 id 째 대상 View 키로 옮긴다(안 옮기면 `remove_view` 가 원문을 지워 새 판이 쓴 슬롯이 다음 저장에서 사라진다). 출처 탭을 출처 창 탭 목록에서 빼고(도우미) `remove_view(출처)`.
- **TabBar** — 출처 창 목록에서 빼고 대상 창 목록의 `min(index, 길이)` 자리에 넣고 `view_owner` 를 바꾼다(View 째라 id · 곁표가 따라간다). 같은 창이면 자리만 바뀐다 — 같은 자리면 무변경 · `version` 불변 · `Ok`. 단 `Reveal` 이고 그 탭이 그 창의 보이는 탭이 아니면 보이게만 한다(`version` +1 · §11-1 #6 — 확정).
- **NewWindow** — 출처 창에서 빼고 `WindowTabs::new_popout(view)` 로 새 창 항목.
- **탭 목록에서 빼기 도우미** — 활성 승계는 `close_tab` 과 같은 규칙(오른쪽 우선 · 없으면 왼쪽). 팝아웃이 비면 창 항목을 지우고 `source_window_closed = true`(R10). ★4 단계 — 메인이 비면 같은 연산 안에서 빈 탭 하나를 만들어 활성으로 둔다(아래)★.
- **보이는 탭** — 대상 창의 활성 탭은 손대지 않는다. 예외: 출처가 대상 창의 활성 탭이었으면(같은 창 도킹) 대상이 보이는 탭이 된다(Seam = 대상 탭 · TabBar = 옮긴 탭 — §11-1 #1). `Reveal` 이면 늘 대상(Seam = 대상 탭 · TabBar · NewWindow = 옮긴 탭)을 활성으로.
- **포커스** — `Reveal` 이면 Seam 은 대상 View 의 `focused_slot_id` = 끼운 첫 슬롯, TabBar · NewWindow 는 그 탭의 포커스 그대로(R11b). `Keep` 이면 손대지 않는다.
- **표식** — `guard.lease` 가 지금 표식이면 내린다.

`plan_new_window`(R22 phase A) — 판정 1 · 5 · 6 을 본 뒤(대상 쪽은 `expect_target` 이 있으면 `TargetChanged`), 출처가 **탭 하나뿐인 팝아웃이고 그 창의 모델 자리에 보통 자리(`attrs.bounds`)가 적혀 있으면** `Reuse{그 창}`(새 창 없이 그 창을 옮긴다 — §7 #24 · 적용 서비스가 옮긴 뒤 같은 락 안에서 이 판정을 다시 불러 확정한다 — 5.2 · 아래 「적용 서비스」), 아니면 `Create{size}`(보통 자리가 없는 탭 하나 팝아웃도 — 5.4 · 아래 「4 단계 고침 셋」) — size = 출처가 팝아웃이면 그 창 `attrs.bounds` 의 논리 크기(최대화 중이어도 보통 자리 크기 · §11-1 #3), 메인이면 `None`(팝아웃 기본 크기 720×500 — `popout.rs:143`).

**★4 단계 고침 — 메인 마지막 탭(PRD R14 · Q9)★** — `check_dock` 의 메인 마지막 탭 판정 · `lease_tab` 의 거절 · `plan_new_window` 의 거절을 걷고 `DockError::MainLastTab` 을 지운다. 빼기 도우미가 `Pulled::MainEmptied` 를 내면 메인에 「View 1」 빈 탭을 만들어 활성으로 둔다 — `close_tab` 의 `MainEmptied` 갈래(`manager.rs:701-708`)와 같은 일이라 도우미 `refill_main` 하나로 뽑아 둘이 나눠 쓴다(`pull_docked_tab` 의 `debug_assert_ne!(MainEmptied)` 는 지운다). 불변식 ④ 는 락 한 번 안에서 선다 — 관찰자는 빈 메인을 못 본다. 같은 창 안(Seam 대상 = 자기 · 탭바 같은 자리)은 그 전에 `SelfTarget` · 무변경으로 걸리므로 빈 탭은 다른 창 · 새 창으로 떠날 때만 생긴다. 기존 시험 중 `MainLastTab` 을 단언하던 것은 「빈 탭이 생긴다 + `assert_invariants`」로 바꾼다.

**★4 단계 고침 셋 — 재사용은 보통 자리가 있을 때만(5.4 · 리뷰 4라운드)★** — `plan_new_window` 의 `Reuse` 갈래(`manager.rs:1337` — 지금은 탭 하나뿐인 팝아웃이면 늘 `Reuse`)에 「`source.attrs.bounds` 가 `Some`」을 더하고, 없으면 `Create{size: None}`(팝아웃 기본 크기)으로 간다. 이유 = 재사용의 되돌리기는 모델의 보통 자리로만 창을 돌려놓는데(§1-2 「적용 서비스」 · 5.3), `WindowAttrs.bounds` 는 `None` 일 수 있다(`manager.rs:166` — 「이 창의 보통 자리를 아직 못 봤다」). 그때 B′ 가 반쯤 옮기거나 C′ 가 거절하면 창이 옮겨진 채 남아 R13b · R13c 의 무변경을 어긴다. `Create` 는 새 창을 열고 판정한 뒤 어긋나면 그 새 창을 닫는 길이라 출처 창을 건드리지 않는다. C′ 의 재호출도 같은 조건을 다시 본다 — 그 사이 보통 자리가 `None` 으로 돌아가면(`WindowAttrs::observe` 의 최대화 되돌림이 앞 값 `None` 을 되살리는 드문 경우) C′ 는 `Create` 로 갈려 `SourceChanged` → 되돌리기이고, 되돌릴 자리 `prev` 는 A 에서 복사한 `Some` 이라 그대로 선다. 바뀌는 곳 = `plan_new_window` 의 갈래 조건 · 그 주석 · 시험(아래 §9).

**★4 단계 고침 둘 — 대상 창 활성(5.1 · 리뷰 C1)★** — `TargetExpect::Layout(LayoutNode)` 를 `TargetExpect::Seam { layout, active }` 로 바꾸고, `check_dock` 의 대상 대조(`manager.rs:1253`)가 Seam 이면 대상 탭 배치와 함께 `windows[대상 창].active == active` 를 같은 락 안에서 본다 — 다르면 `TargetChanged` · 무변경. 이유 = `switch_tab`(`manager.rs:673` · 적용 서비스 `apply.rs:250`)은 활성을 락 안에서 바꾸고 알림은 락을 놓은 뒤 낸다. 그 틈에 장부 · 웹뷰는 옛 활성의 영역을 쥐고 있어, 배치만 대조하면 이음새 놓기가 이제는 안 보이는 탭에 들어가고 `Reveal` 이 그 탭을 도로 앞에 낸다(R6b · 사람 클릭으로 바꾼 탭이 말없이 되돌아간다). `active` = 영역을 만든 순간 그 창의 **모델** 활성이다 — 같은 창 도킹에서 끄는 탭이 활성이면 그 끄는 탭 자신이다(화면의 승계 탭이 아니다 — 웹뷰 쪽 대조는 §1-3 「놓기 판정」). TabBar · NewWindow 는 그대로. 바뀌는 곳 = `TargetExpect` 정의 · `check_dock` 의 Seam 팔 · 판정 순서 주석(7) · 시험의 `TargetExpect::Layout` 자리 여덟 + 새 시험(배치는 같고 활성만 다름 = `TargetChanged` · 무변경 · 같은 창 도킹의 기대값 = 끄는 탭이 그대로 통과).

#### 표식(lease) — 「이 탭은 지금 끄는 중」

`ViewManager.lease: Option<(LeaseId, ViewId)>` — 한 번에 하나. `lease_tab` 은 판정 1 · 「이미 표식이 있음」(= `Dragging`)을 보고 세운다. 모델 안에 두는 이유: 「끄는 중인 탭을 옮기는 명령은 실패」(D9 — 다음 작업의 명령)가 그 명령의 변경과 같은 락 안에서 서야 한다. 표식 번호 = 끌기 세션의 표식이다 — 모델은 같은지만 본다. 메모리에만 있다. 표식이 선 탭이 지워지면(`remove_view` — 탭 닫기 · 창 닫기 · 복원 수락) 표식도 내려가고, 그 표식을 쥔 늦은 놓기는 `StaleLease` 다. ★지금 표식 없이 `dock` 을 부르는 길은 없다★ — 마우스 놓기만 부르고 늘 표식을 싣는다. `Dragging` · `FocusPolicy::Keep` 은 다음 작업의 명령이 쓴다.

#### 닫히는 창 — 모델 먼저 · 묘비 (3 단계 · M2)

- **(a) 닫기 요청 = 모델에서 먼저 지움.** `on_window_event` `CloseRequested` 에서 label 이 팝아웃이면 레이아웃 락 한 번 안에서 `drop_window_in_model`(창의 탭 View 전부 · 창 항목 · 라우팅 재계산 · 구독 해제 · 사용량 관심)을 하고 `mark_closed(label)` 로 묘비를 단다. 뒤이은 `Destroyed` 는 같은 정리를 부르지만 모델에 그 창이 없어 재계산 · 묘비만 남는다(멱등). `Destroyed` 도 `mark_closed` 를 부른다 — 닫기 요청 없이 사라진 창까지 묘비가 선다.
- **근거 — 팝아웃의 닫기 요청은 되돌려지지 않는다(확실 · 소스)** — 우리 처리기는 팝아웃에 `prevent_close` 를 부르지 않고(main 만), 프론트에 닫기 요청 리스너가 없으며, 런타임은 콜백이 돌아오자마자 창을 닫는다(`tauri-runtime-wry-2.11.3/src/lib.rs:4438-4466`). ★누가 팝아웃 닫기를 막을 수 있게 만들면 이 절은 「닫는 중 표시 + 막히면 해제」로 바꿔야 한다★ — 처리기 주석과 §9 「게이트」의 점검 명령 두 줄이 그 전제를 지킨다.
- **(b) 새 창 경로는 묘비 앞에서 멈춘다.** phase C 의 판정 2 가 `WindowClosed` 로 무변경 — 탭은 출처에 남고 적용 서비스는 연 창을 `host.close` 한다. `attach_view_as_new_window` 에도 같은 검사가 있다.
- **(c) 묘비 뒤에 붙은 탭이 지워지는 일은 없다(락 순서).** 묘비와 모델 삭제는 같은 락 한 번이다. 그 뒤의 확정은 대상 없음 · 묘비로 무변경이고, 그 앞의 확정은 닫기 시점에 그 창의 탭이라 그 닫기와 함께 닫힌다(창 닫기 규칙 그대로 · 잔해 없음).
- **팝아웃 `slot.popout` 의 같은 틈도 닫혔다** — 기존 창 타깃은 phase C 의 `insert_tab_into` 가 창 없음으로, 새 창 타깃은 묘비로 롤백을 탄다(`apply.rs` `WindowHost` 머리 주석이 3 단계에서 고쳐졌다).
- **닫히는 창의 늦은 replay 요청** — 닫기 요청이 구독 해제를 낸 뒤에도 닫히는 웹뷰가 잠깐 살아 replay 를 청할 수 있다. 연결 task 가 그 에이전트를 보이는 창이 없으면 Subscribe 를 보내지 않는다(`daemon_client/connection.rs` `RequestReplay` 갈래 · 3 단계). ★보이는 창이 하나라도 있으면 지나간다★ — 도킹으로 옮긴 에이전트가 그렇다(무해 — §5 「출력 연속」).
- ADR-0057 불변식(① 양방향 · ② 유니크 소유 · ③ 활성 소속 · ④ main ≥ 1)은 그대로이고 정리의 계기만 소멸 → 닫기 요청으로 옮긴다(ADR-E · §11-1 #2 — 사용자 결정 2026-10-16). 묘비 `closed_labels` 는 메모리만이고 팝아웃 label 은 한 부팅 안에서 다시 쓰이지 않는다(`PopupCounter`).

#### 적용 서비스 — `apply.rs` (4 단계)

```rust
pub struct DockRequest { pub view: ViewId, pub to: DockDest, pub guard: DockGuard, pub focus: FocusPolicy }
pub enum DockDest {
    Seam { target: ViewId, anchor: DockAnchor, side: DockSide },
    TabBar { window: WindowLabel, index: Option<usize> },
    NewWindow { at: WindowPlace },
}
pub struct WindowPlace { pub pos: Option<(i32, i32)> /* 물리 바깥 왼쪽 위 */, pub size: Option<(f64, f64)> /* 논리 안쪽 */ }  // R22 자리 — 되돌리기는 모델 `WindowAttrs`(5.3)
pub enum MoveStep { Position, Size, Maximize }                     // 창 포트의 세터 하나(5.3)
pub struct MoveFailed { pub step: MoveStep, pub error: String }
pub struct DockOutcome { pub window: WindowLabel, pub slot_ids: Vec<Uuid>, pub closed_window: Option<WindowLabel>, pub reused: bool }
pub fn dock_tab(state, subs, events, host, labels, req: DockRequest) -> Result<DockOutcome, DockError>;
```

- **Seam · TabBar = 락 한 번.** `mgr.dock(..)?` → 대상 View · 대상 창 · (남았으면) 출처 창 스냅숏 → `subs.resync(&mgr)` → 락 드롭 → 알림(대상 배치 · 대상 창 · 출처 창 탭 목록) → `events.settled()` 한 번 → 출처 팝아웃이 비었으면 `host.close(label)`(락 밖 · `apply.rs` 머리 「락 규율」). 닫히는 대상 창은 M2 로 막히므로 `is_open` 사전 확인을 두지 않는다.
- **알림 묶음 끝 `LayoutEvents::settled()`(5.1 · 리뷰 F1)** — 쓰기 하나가 알림을 다 낸 뒤 한 번 부른다(`notify` · `rename_tab` · 슬롯 옮기기 · `restore.rs` 의 알림 루프 · `dock_tab`). 기본 본문 없음(구현자 아홉이 컴파일에서 걸린다) · `TauriEvents` 만 6 단계에서 「세션 깃발이 섰으면 스냅숏을 떠 장부에 먹인다」를 단다(§1-3 「영역」). 알림 하나마다 먹이면 첫 알림 뒤의 스냅숏이 나머지 알림보다 먼저 영역 · 끝을 내보낸다 — 웹뷰가 새 영역을 탭 목록보다 먼저 받고, 도킹 적용 중이면 장부가 그 스냅숏으로 세션을 끝낸다(§1-3 「적용 중」). 알림 자리가 이것을 빠뜨리면 영역이 늦을 뿐이다 — 그 사이 놓기는 무변경으로 떨어진다(안전 쪽).
- **NewWindow = 세 phase**(팝아웃 `move_slot_to_window` · `apply.rs:657` 을 따른다):
  - A(락) `plan_new_window`. `Reuse{label}` 이면 같은 락 안에서 그 창의 모델 자리 `prev = attrs`(보통 자리 `bounds` · `maximized` — 최대화 중이어도 `bounds` 는 보통 자리다: `WindowAttrs::observe` 가 최대화 읽기로 덮지 않는다)를 복사해 두고 아래 「재사용」 세 걸음이다. B · C 는 없다.
  - **재사용 = 옮기고 → 같은 락 안에서 다시 확인 → 어긋나면 되돌리기(5.2 · 리뷰 B2)** — 5.1 은 A 에서만 보고 락 밖에서 옮긴 뒤 성공으로 끝냈다. 그 사이 다른 경로가 출처를 바꾸면(출처 탭에 슬롯 더함 · 그 팝아웃에 탭 더함 · 탭 · 창 닫힘) R13b · R13c 가 무변경을 요구하는데도 창이 옮겨졌다(`Applying` 은 스냅숏을 적기만 하므로 장부도 못 막는다).
    - B′(락 밖) `host.move_window(label, &at)` — 세터를 검사하며 위치 → 크기 순으로 입히고 첫 실패에서 멈춘다(`MoveFailed{step}`). 실패하면 창이 반쯤 옮겨졌거나 최대화가 풀렸을 수 있으므로 **되돌리기를 한 뒤** `WindowOpen`. ★되돌릴 자리는 A 의 모델 자리이지 게터가 아니다(5.3 · 리뷰 3라운드)★ — 최대화된 창의 `outer_position` · `inner_size` 는 최대화 사각형을 돌려줘, 그것을 보통 자리로 입히면 보통 크기가 틀리고 보일 때 미룬 최대화(`apply_deferred_maximize`)가 그 틀린 사각형을 모델에 다시 적는다.
    - C′(락) `plan_new_window(view, &guard)` 를 **다시 불러** `Reuse{같은 label}` 인지 본다(표식 · `expect_source` · 「탭 하나뿐인 팝아웃」 · 그 창 존재가 함께 선다) → 맞으면 같은 락 안에서 `release_lease` · 모델 변경 0 · `reused = true` → 락 드롭 → `host.record_placement(label)`(숨긴 출처를 보이는 것은 장부의 끝 몫 — §1-3).
    - 어긋나면(그 오류 · 또는 `Create` 로 바뀜 = `SourceChanged`) 락 드롭 → **되돌리기** `host.restore_window(label, &prev)` → 그 오류. 표식은 장부 끝이 내린다.
    - **되돌리기(`restore_window`)** = 저장 자리 복원과 같은 풀이 — `prev.bounds` 를 `placement::land` 로 풀어 위치 → 크기 → (`prev.maximized` 면) 최대화 순으로 입힌다. 최대화는 숨은 창이면 보일 때로 미루고(`DeferredMaximize` — 숨은 창을 최대화하면 tao 가 창을 보인다), 보이는 창이면 곧바로 입힌다. ★한 걸음이 실패해도 나머지 걸음을 이어 간다★ — 되돌리기 뒤에는 보상할 것이 없으므로 할 수 있는 만큼 돌려놓고, 실패한 걸음은 끝 로그(`restored = false` · `failed`)로 남긴다. `prev.bounds` 는 늘 있다 — 보통 자리가 없는 창은 재사용을 고르지 않는다(5.4 · §1-2 「4 단계 고침 셋」). 그 창이 이미 사라졌으면 포트가 무동작으로 넘긴다.
    - **보이는가** — 최대화가 아니던 출처는 끄는 동안 숨어 있어(§1-3 「시작」 4) 옮기기 · 되돌리기가 화면에 안 보이고, 무변경 끝의 되보이기가 원래 자리에 보인다. ★최대화였던 출처는 보인다(확실 · 소스 · 받아들인 경우)★ — B′ 의 위치 세터가 최대화를 풀고(`placement.rs` `maximize` 머리 F8) tao 는 그 풀기를 `ShowWindow(SW_RESTORE)` 로 한다(`tao-0.35.3/src/platform_impl/windows/window_state.rs:376-386` — 창을 보이고 활성화한다 · `placement.rs:520-521` 이 같은 동작을 미검증으로 적어 둠). 그래서 B′ 에서 그 창이 놓은 자리에 보통 크기(Q13)로 떠 포커스를 가져간다 — 성공이면 끝의 `set_focus` 와 같은 결과이고, C′ 가 어긋나면 사용자는 창이 그 자리에 떴다가 원래 자리 · 최대화로 돌아가는 것을 본다. 숨기기가 실패해 보인 채였던 출처도 같다(두 겹 실패). 숨긴 채 최대화를 푸는 길은 지금 없다 — 그러려면 platform 목적 함수(Win32 `SetWindowPlacement` 류)가 필요하고 이번엔 넣지 않는다(§11-2 #24 미결 · S1 ⑤ 가 잰다).
    - 남는 틈 — C′ 확인과 그 뒤 사이의 쓰기는 이미 옮겨진 창 위의 쓰기라 그대로 그 창에 남는다(무변경 요구가 걸리는 「놓기 전」 쓰기가 아니다). Create 의 phase B → C(연 창을 닫는 되돌리기)와 같은 꼴이다.
  - B(락 밖) `labels.next_label()` → `host.open_at(label, &at_with_size, activate = focus == Reveal)`. 실패면 `WindowOpen` · 모델 무변경.
  - C(락) `mgr.dock(view, NewWindow{label}, ..)` 로 **다시 판정하고** 적용 — B 와 C 사이에 그 창이 닫혔으면 묘비로 `WindowClosed`. 실패면 락 드롭 → `host.close(label)` → 그 오류. 성공이면 `resync` → 락 드롭 → 알림 → `settled()` → `host.record_placement(label)`.
- **창 포트 확장 — 기본 본문 없음** — `fn open_at(&self, label, at: &WindowPlace, activate: bool) -> Result<(), String>` · `fn move_window(&self, label, at: &WindowPlace) -> Result<(), MoveFailed>`(위치 → 크기 · 첫 실패에서 멈춤) · `fn restore_window(&self, label, prev: &WindowAttrs) -> Vec<MoveFailed>`(위치 → 크기 → 최대화 · 실패해도 이어 감 · 실패한 걸음 목록)(5.3 — 재사용 몫 · 창이 없으면 무동작). 기본 본문을 두면 `OwnedWindowHost` 가 물려받아 자리 · 크기 · 활성을 말없이 버린다 — 본문 없이 두어 구현자 여섯(§1-1 표)이 컴파일에서 걸리게 한다. Tauri 구현: `open_at` = `build_runtime_window` 에 첫 자리와 `focused(activate)` 를 넘기고 만든 뒤 `placement::place`(위치 먼저 → 크기 · 최대화 해제)로 정확히 놓는다 · 크기 없으면 팝아웃 기본 크기 · 웹뷰 환경은 `build_window` 가 `WebviewEnv::finish` 로 붙인다. `move_window` · `restore_window` 는 지금의 `placement::place`(`placement.rs:470` — 세터 실패를 로그로만 남기고 `()`)를 쓰지 않는다 — 부팅 · 복원이 쓰는 그것은 그대로 두고, 세터마다 결과를 돌려주는 짝을 같은 모듈에 더한다(순서는 같다 — 위치 먼저 · 크기 뒤 · 최대화는 보통 자리 뒤). 장부 끝의 되보이기는 미룬 최대화를 입히는 보이기 경로(`apply_deferred_maximize`)를 탄다.
- **자리 검증** — `pos` 가 있으면 `placement::land(label, bounds, monitors, Fallback::DefaultPlace)` 로 어느 모니터에 걸치는지 본다 · 안 걸치면 기본 자리(저장 자리 복원과 같은 규칙 · 같은 info 로그). 마우스 자리(커서 − 그림 커서 자리)는 보통 모니터 위지만 화면 가장자리에서 밖으로 나갈 수 있다.
- **오류** — `DockError` 는 `Display` 를 갖는다(한국어 사유 + 괄호 속 종류 낱말). 마우스 경로는 끝 로그의 `reason` 으로 남긴다(§1-7). 레이아웃 적용 서비스의 첫 타입드 오류다 — 다음 작업의 명령이 종류로 오류 코드를 고른다.

### 1-3. ② 끌기 장부 (셸 `dock`)

#### 장부 — 순수 상태 하나 + 잎 락

```rust
// dock/book.rs — Tauri · 락 · 시계 읽기 · OS 0. 시각 · 커서 · 창 사각형 · 스냅숏은 전부 입력이다.
pub struct Book {
    drag: Option<Drag>, last_snap: u64,
    held_snap: Option<Snap>,            // 세션 전(시작 락 뒤 · `Start` 전)에 온 스냅숏 — `Start` 가 이어 먹는다(5.1 · N8)
    early: Option<Early>,               // 모르는 drag_id 의 끝 신호 — 한 번에 한 id(5.1 · 「이른 끝 신호」)
    ended: VecDeque<DragId>,            // 최근 끝난 id 넷 — 늦은 신호와 이른 신호를 가른다
}
struct Drag {
    id: DragId /* 출처 웹뷰가 만든 UUID 문자열 */, lease: LeaseId, seq: u64, hotspot: PointCss,
    source_label: WindowLabel, source_view: ViewId, expect_source: LayoutNode, source_hidden: bool,
    source_destroyed: bool /* 적용 중에 출처가 사라짐 — 되보이기를 건너뛴다 */,
    display_active: Option<ViewId>, blocked: bool,
    sent: HashMap<WindowLabel, Sent>,   // 창별 마지막 보낸 영역 — rev · bar_rev · 영역 · 기대값(TargetExpect 재료: Seam{배치, 창 활성} · Tabs)
    seen: HashMap<WindowLabel, Seen>,   // 창별 — 마지막 Over{rev, bar_rev, displayed, shown, point, 도착 순번, 시각} · 마지막 Left 시각(따로 — 5.1 · F4)
    phase: Live | AwaitDrop { at, cursor } | Grace { at } | Applying | Ended,
}
pub enum Input { Start{..}, Refused{drag_id}, Snapshot(Snap), Over{..}, Left{..}, Drop{..},
                 SourceEnd{effect, cursor: Option<..>, rects, now}, PointerSeen{now}, Timeout{kind, cursor, rects, now},
                 Destroyed{label}, Applied(Result<DockOutcome, DockError>), Pull{label} }
pub enum Action { Broadcast(SessionEvent), Regions{label, DockRegions}, Hide(label), Show(label), Focus(label),
                  Apply(DockRequest), ArmTimer{kind /* Drop · Grace · Early */, after}, ReleaseLease(LeaseId), EndLog(..) }
```

- **잎 락** — 셸 상태 `Mutex<Book>`. 쥔 채 레이아웃 락 · emit · 창 호출 · 적용 서비스 · 커서 읽기를 하지 않는다. 실행기(`commands/dock.rs`)가 필요한 값을 먼저 읽어 입력에 싣고, 장부가 낸 할 일을 락 밖에서 실행한다. 영역 계산(순수 · ms 이하)은 장부 안에서 한다.
- **창 게터는 두 락 밖에서(5.1 · N5)** — `is_visible` · `is_minimized` · `inner_position` · `inner_size` · `scale_factor` · `cursor_position` 은 레이아웃 락도 장부 락도 쥐지 않고 읽는다. 메인 스레드가 아닌 곳의 게터는 이벤트 루프에 메시지를 보내고 답을 기다리는데(`tauri-runtime-wry-2.11.3/src/lib.rs:197-211` · `:235-255`), 메인 스레드의 창 사건 처리기(`Destroyed` · `CloseRequested`)는 레이아웃 락을 잡는다 — 락을 쥔 채 게터를 부르면 교착이다(`apply.rs` `WindowHost` 머리 「락 밖에서만」과 같은 이유). 그래서 스냅숏(락 안 · 모델 값)과 사각형 · 배율 · 커서(락 밖 · 게터)는 따로 떠서 입력에 함께 싣는다. `WindowSnap.hidden` 은 게터가 아니라 장부 값(`source_hidden`)이다 — `hide()` 직후의 `is_visible` 은 늦을 수 있다.
- **스레드 · 틱 없음** — 장부는 사건(invoke · 창 사건 · 모델 계기 · 시한)이 올 때만 돈다. 시한(`AwaitDrop` · `Grace` · 이른 신호)은 `tauri::async_runtime::spawn` + `sleep` 한 건이 `Timeout` 을 먹인다. 세션 시한(`AwaitDrop` · `Grace`)은 한 번에 하나 · 이른 신호 시한은 따로 하나이고 번호를 달아 지난 시한은 버린다 — `Grace` 중 끝 신호는 평소대로 처리하고(`dragend(move)` 면 `AwaitDrop` 으로 갈아탄다), `AwaitDrop` 중 `pointer_seen` 은 버린다.
- **놓기 · 끝 신호 대조** — `drag_id` 가 최근 끝난 id(`ended`)면 늦은 신호로 버린다. 처음 보는 id 의 끝 신호(`dock_drop` · `dock_drag_end`)는 붙든다(아래 「이른 끝 신호」). `Over` · `Left` · `pointer_seen` 이 모르는 id 면 버린다. `dock_drop` 의 `view_id` 가 그 세션의 출처가 아니면 버린다(다른 인스턴스의 끌기 포함).
- **스냅숏 번호** — 실행기가 레이아웃 락 안에서 원자 셈을 올려 스냅숏에 붙인다. 장부는 `last_snap` 이하 번호를 버린다 — 두 계기가 엇갈려 옛 스냅숏이 새 영역을 덮지 않게.
- **한 세션에 적용은 많아야 한 번** — 판정과 `Applying` 표시가 장부 락 한 번 안이다. 그 뒤 오는 놓기 · 끝 신호는 늦은 신호다. `Applying` 동안의 스냅숏 · 소멸은 아래 「적용 중」.
- **명령의 실행 스레드** — `dock_drag_start` · `dock_drag_over` · `dock_pointer_seen` · `dock_pull` · `report_dock_chrome` 은 동기 명령(메인 스레드 · 도착 순서대로)이다 — 같은 웹뷰의 시작 → 끝 순서가 그렇게 선다. 창을 만들 수 있는 `dock_drop` · `dock_drag_end` 와 시한 콜백은 async 다(동기 명령에서 창을 빌드하면 교착 — `popout.rs` `move_slot_to_window`(:185) 선례).
- ★**불변식 — `Apply` 는 메인 스레드 · 동기 명령 안에서 돌지 않는다(5.2 · 리뷰 R2-1)**★ — 실행기는 `Apply` 할 일을 먹인 스레드와 무관하게 늘 `tauri::async_runtime::spawn` 으로 넘기고, 그 결과 `Applied` 와 그 끝 할 일도 그 async 작업에서 먹인다. 할 일은 보통 먹인 스레드에서 실행되는데(아래 「순서」), 동기 명령도 `Apply` 를 낼 수 있기 때문이다 — `dock_drag_start`(메인 스레드)가 `Start` 뒤에 붙든 이른 놓기를 이어 먹이면(「이른 끝 신호」) 그 자리에서 판정 → `Apply` → R22 새 창 열기가 메인 스레드의 동기 명령 안에서 돌아 교착한다. `Apply` 말고 동기 경로에서 나올 수 있는 할 일(emit · 숨기기 · 보이기 · 포커스 · 표식 내리기 · 시한 걸기)은 창을 만들지 않아 먹인 스레드에서 그대로 돈다.

| 상수 | 값 | 뜻 |
|---|---|---|
| `LEAVE_GRACE_MS` | 100 | 출처 `dragend`(none) 앞의 대상 「떠남」이 이보다 가까우면 Esc 취소가 낸 떠남으로 보고 그 창을 「아직 위」로 친다 — S1 이 간격을 잰다 |
| `DROP_WAIT_MS` | 500 | `dragend`(move) 뒤 대상의 `dock_drop` 을 기다리는 시간 — 두 웹뷰의 invoke 순서는 보장되지 않는다 |
| `END_GRACE_MS` | 300 | `pointermove` 를 본 뒤 끝 신호를 기다리는 시간 — 넘으면 신호 유실로 친다 |
| `EARLY_SIGNAL_MS` | 500 | 장부가 모르는 `drag_id` 의 끝 신호를 그 시작을 기다리며 붙드는 시간 — 시작이 거절되면 곧바로 버린다 · S1 ⑧ 이 잰다 |

#### 시작 — `dock_drag_start{drag_id, view_id, hotspot}`(동기)

1. **거절(부수 효과 0)** — 부른 창(Tauri 가 넣어 준 창)이 그 탭의 주인이 아님 = `TabNotFound` · 장부에 세션이 있음 = `AlreadyDragging` · 그 `drag_id` 가 최근 끝난 id(`ended`)임 = `LateStart`(5.2 · 리뷰 R2-2 — 붙들었던 끝 신호를 이미 버린 끌기다: 열면 끝 신호 없는 세션이 `pointer_seen` 까지 남았다가 지금 커서에 새 창으로 끝난다. 거절이면 무변경 — 안전 쪽) · 레이아웃 락 오염 = `LayoutPoisoned` · 옛 세션의 표식이 아직 안 내려감 = `Dragging`(`lease_tab` — 장부가 끝을 낸 뒤 실행기가 `release_lease` 를 부르기 전의 짧은 틈 · 받아들인 남은 틈 — 사람 손으로는 그 틈에 새 끌기를 시작하지 못한다). `AlreadyDragging` 의 옛 세션이 `Live` · `AwaitDrop` · `Grace` 면 끝 신호를 잃은 것이라 곧바로 「마지막 점에 놓기」로 끝낸다(async 한 건 · §7 #12). ★옛 세션이 `Applying` 이면 끝내지 않는다(5.1 · N2)★ — 적용 서비스가 도는 중이고(새 창 열기는 수백 ms) 그 끝은 `Applied` 가 정한다. 새 시작만 거절하고(debug · `reason = applying`) 옛 세션의 표식은 그 끝에서 내려간다(성공이면 모델이 이미 내렸다) — 그 뒤의 끌기는 받는다. 거절하면 실행기가 장부에 `Refused{drag_id}` 를 먹여 그 id 로 붙든 끝 신호를 버린다. 거절해도 OS 끌기는 이미 돌고 있다 — 다른 창은 세션을 몰라도 우리 MIME 을 받으므로(§1-4) 커서는 「이동」으로 보이지만, 그 놓기는 모르는 id 라 아무것도 바꾸지 않는다(§11-2 #21).
2. **확정(레이아웃 락 한 번)** — `lease_tab(view)`(실패면 위 사유) · 출처 배치 복제(`expect_source`) · 출처 창 label · 탭 목록 · 활성 · 「탭 하나뿐인 팝아웃인가」 · 영역 재료 스냅숏(모든 창 — §2-4 · 번호를 단다). ★세션 깃발을 이 락 안에서 세운다(5.1 · N8)★ — 장부 `Start` 에서 세우면 이 락을 놓은 뒤 `Start` 전에 끼어든 레이아웃 쓰기가 계기를 건너뛰어 장부가 옛 배치로 남는다. 그 쓰기의 스냅숏이 `Start` 보다 먼저 장부에 닿으면 장부는 그것을 `held_snap` 으로 들고 있다가 `Start` 뒤에 이어 먹는다(번호가 시작 스냅숏보다 클 때만). ★깃발은 불이 아니라 세션 세대(원자 `u64` · 0 = 없음)다(5.2 · 리뷰 R2-3)★ — 여기서 새 세대를 넣고, 장부가 끝을 낸 뒤 실행기가 **자기 세대일 때만** 0 으로 바꾼다(비교-교체). 무조건 내리면 그 사이 선 다음 세션의 깃발까지 내려 그 세션의 계기가 끊긴다. 락 드롭.
3. **장부** — `Start` → 할 일: 출처가 탭 하나뿐인 팝아웃이면 `Hide(출처)` · `Broadcast(Started{source_label, source_view, display_active})` · 창마다 `Regions`(숨긴 출처 제외). `display_active` = 출처가 그 창의 활성이면 `succession`, 아니면 없음(메인 마지막 탭이면 없음 — 메인은 빈 화면). 그다음 `held_snap` · 그 id 로 붙든 끝 신호를 도착 순서대로 먹는다 — 그 판정이 낸 `Apply` 는 이 동기 명령 안에서 돌지 않고 async 로 넘어간다(위 「명령의 실행 스레드」 불변식).
4. **실행** — 숨기기 = Tauri `hide()`(실측: 숨겨도 끌기 · `dragend` 가 이어진다 — 표본 2 · S1 ⑤). 실패하면 warn 하고 보인 채 간다(끌기는 이어진다). 숨기기는 자리 · 크기를 바꾸지 않아 자리 기록을 멈출 일이 없다.

#### 영역 — 언제 무엇을 보내나

- **보내는 때** — 시작 · 모델 계기(`TauriEvents::settled` — 쓰기 하나가 알림을 다 낸 뒤 한 번 · 버스 · invoke · 팝아웃 · 복원이 이 어댑터를 지난다 · §1-2 「알림 묶음 끝」) · 측정 계기(`report_dock_chrome` · `report_window_canvas` 처리 뒤) · `dock_pull`. 계기마다 실행기가 세션 깃발이 섰을 때만(원자 깃발 — 시작의 레이아웃 락 안에서 선다) 스냅숏을 떠 장부에 먹인다.
- **창마다 (영역, 기대값)이 마지막 보낸 것과 다를 때만** `rev` 를 올려 `emit_to(label, "dock:regions")`. `rev` 는 창별 · 세션 안 단조다. 장부는 그 `rev` 의 영역과 기대값(`TargetExpect::Seam{그 창 보이는 탭 배치, 그 창 모델 활성}` · `Tabs(그 창 탭 목록)`)을 함께 둔다 — 놓기 대조의 기준이다. ★기하가 같아도 기대값이 바뀌면 `rev` 를 올린다(5.1 · N4)★ — 대상 칸의 내용만 바뀐 경우(터미널 → 챗 · 같은 기하)다. 올리지 않으면 웹뷰는 옛 `rev` 로 놓고 장부는 옛 기대값을 실어 모델 판정 6 이 `TargetChanged` 로 막는다 — 사용자가 본 가이드 그대로인데 무변경이 된다. 올리면 웹뷰가 같은 영역을 새 `rev` 로 받아 그 뒤 놓기는 새 기대값으로 들어간다(남는 것은 영역이 닿기 전의 짧은 틈뿐이고 그것은 무변경이다). ★탭바 몫은 따로 센다(5.2 · 리뷰 B1)★ — `bar_rev` 는 창별 · 세션 안 단조이고 탭바 영역(`tab_bar` — 사각형 · 머리 · 끼울 x)이나 탭 목록 기대값(`Tabs`)이 바뀔 때만 오른다. 활성 전환은 보이는 탭 · 칸 · 창 활성만 바꾸므로 `rev` 는 오르고 `bar_rev` 는 그대로다 — 그래서 전환 중의 탭바 놓기가 선다(「놓기 판정」 4b).
- **스냅숏이 같이 판정하는 것** — 출처 View 가 없음 → `SourceGone` 끝 · 출처 배치 ≠ `expect_source` → `blocked = true`(모든 창 영역을 비우고 다시 보냄 — 웹뷰는 `dropEffect = 'none'` · 커서 「안 됨」 · R13c · AC16 · info 한 줄) · 출처 창 탭 목록 · 활성이 바뀌어 `display_active` 가 바뀜 → `Broadcast(Display)` · 스냅숏에 없는 창(닫힘)은 `sent` 를 지운다 — 그 창의 지금 영역은 없음이고 `seen` 은 남긴다(놓기 판정이 「대상 사라짐」을 알아야 한다). `Applying` 중이면 이 판정을 하지 않는다(아래 「적용 중」).
- **늦게 뜬 창 · 놓친 emit** — 웹뷰는 영역 없이 끌기를 만나면(`dragenter`) `dock_pull` 로 한 번 당긴다. 장부가 그 창 영역을 새로 만들어 돌려준다(바뀌었으면 `rev` +1).
- **순서 — 영역 · 세션 사건은 뒤바뀌어 닿을 수 있다(5.1 · C3 · F5)** — 할 일은 그것을 먹인 스레드에서 장부 락을 놓은 뒤 실행되므로(동기 명령 = 메인 · async 명령 · 시한 = 런타임 스레드 · 모델 계기 = 그 쓰기의 스레드 · 단 `Apply` 는 늘 async — 위 불변식) `rev` 4 의 emit 이 `rev` 3 보다 먼저 닿을 수 있다. 웹뷰는 `drag_id` 별로 가장 큰 `rev` 만 쓰고 작은 것은 버린다 · `Started` 전에 온 영역도 받아 둔다(§1-4). 장부는 `Over` 의 `rev` 가 그 창의 지금 `rev` 보다 작으면 지금 영역을 같은 `rev` 로 다시 보낸다(emit 유실 회복 · debug) — 웹뷰가 옛 `rev` 에 머물러 모든 놓기가 `TargetChanged` 가 되는 것을 막는다.

#### 보인 것 보고 — `dock_drag_over`(동기)

- 대상 웹뷰는 `pick` 결과(`shown`) · 쓰는 영역 `rev` · 그 창이 보이던 탭(`displayed` — 5.1 · C1)이 바뀔 때만 `Over{rev, bar_rev, displayed, shown, point}` 를, 창 밖으로 나갈 때 `Left` 를 보낸다(움직일 때마다가 아니다 · `drag_id` 를 모르면 — 영역 · 세션 전 — 보내지 않는다). 장부는 창별로 **마지막 Over**(보고 · 도착 순번 · 시각)와 **마지막 Left 의 도착 시각**을 따로 적는다(5.1 · F4 — Left 가 Over 를 지우지 않는다). 할 일은 없다 — 단 `rev` 가 그 창의 지금 `rev` 보다 작으면 지금 영역을 다시 보낸다(「영역」 순서).
- 「마지막 Over 창」 W = 마지막 Over 의 도착 순번이 가장 늦은 창 — 그 뒤에 Left 가 왔어도 W 다. 숨긴 출처 창은 후보가 아니다. W 가 **아직 위** = W 의 마지막 Over 뒤에 Left 가 없거나, 있어도 그 Left 의 도착이 끝 신호 도착 전 `LEAVE_GRACE_MS` 안. (5판처럼 창별 「마지막 보고 하나」만 두면 Left 가 Over 를 덮어 유예 안 Left 의 그 Over 를 되찾을 수 없다 — 창 안 Esc 의 떠남이 `dragend` 보다 먼저 닿으면 새 창이 되어 AC19 를 어긴다.)

#### 끝 신호 셋과 「마지막 점에 놓기」

```
우리 창 W 위에서 뗌   W: drop → dock_drop{rev, bar_rev, displayed, shown, point} 출처: dragend(move) → (이미 판정했으면 늦은 신호)
창 밖 · 틈 · 바탕화면   (drop 없음)                                     출처: dragend(none) → 마지막 점에 놓기
Esc(어디서든)           W: dragleave(취소 — 끝보다 먼저 올 수 있다)      출처: dragend(none) → 마지막 점에 놓기
끝 신호 유실            아무 창: 끝난 뒤 pointermove → dock_pointer_seen → END_GRACE 뒤 마지막 점에 놓기
```

- **`dock_drop`(W)** → 아래 「놓기 판정」을 (W, 보고의 rev · bar_rev · displayed · shown · point)로.
- **`dock_drag_end{effect}`(출처)** — 실행기가 그 순간 셸 커서(Tauri `cursor_position` — 물리 px · 못 읽으면 없음)와 보이고 최소화 안 된 우리 창들의 클라이언트 사각형(`inner_position` · `inner_size`) · 배율을 락 밖에서 읽어 싣는다.
  - `effect ≠ none`(받은 창이 있다) — 이미 판정했으면 늦은 신호. 아니면 `AwaitDrop{그 커서}` 로 두고 `DROP_WAIT_MS` 시한을 건다. 그 안에 `dock_drop` 이 오면 그것으로 판정, 시한이 지나면 「마지막 점에 놓기」(그 커서로).
  - `effect = none` → 「마지막 점에 놓기」.
- **「마지막 점에 놓기」(PRD R12 · R13b)** — `blocked` 면 판정 없이 무변경(`SourceChanged` — 놓기 판정 1 과 같고 새 창도 만들지 않는다). 아니면 W = 마지막 Over 창(위 — 뒤에 Left 가 와도 W).
  - W 가 아직 위이고 W 의 사각형이 있고 **커서가 그 안** → 「놓기 판정」을 (W, W 의 마지막 Over 보고)로.
  - W 가 아직 위인데 **W 의 사각형이 없음**(소멸 · 숨김 · 최소화 · 못 읽음) → W 의 마지막 보고가 가이드를 보였으면 **무변경**(`TargetChanged` — 대상이 사라짐 · Q18), 「가이드 없음」이었으면 커서 자리에 새 창(R22).
  - 그 밖(W 없음 · W 를 떠남 · 커서가 W 사각형 밖) → 커서 자리에 새 창(R22). 그 자리에는 사용자에게 보인 가이드가 없다 — 가이드는 창 안에만 그려지고 창을 떠나면 지운다. W 가 아직 위인데 커서가 그 밖인 것은 창 밖으로 빠르게 나가 떼어 떠남 보고가 아직 오는 중이거나(그대로 새 창이 맞다 — 무변경이면 던진 탭이 원래 창으로 돌아가 「되돌리기 없음」(Q8)과 어긋난다) 끝 신호 뒤 커서가 움직인 경우다(5판과 같다 · §11-2 #1).
  - **커서를 못 읽으면**(잠금 화면 · 보안 데스크톱 — AC21 · 5.1 · N3) — 「마지막으로 알려진 점」 = W 의 마지막 Over 점. W 가 아직 위면 「놓기 판정」(W, 그 보고)(사각형 검사 없음 — 유실과 같다). 아니면 「가이드 없음」으로 보고 그 점을 W 의 화면 좌표(안쪽 원점 + 점 × 배율)로 옮긴 자리에 새 창 · W 가 없거나 W 의 사각형도 못 읽으면 기본 자리(`pos = None`).
  - ★Esc 와 떼기를 가를 필요가 없는 이유★ — 우리 창 위에서 버튼으로 떼면 그 창의 `drop` 이 처리하므로(`dropEffect = move`), `none` 끝이 우리 창 위를 가리키면 그것은 Esc(또는 시스템 취소)다. 그리고 Esc 와 떼기는 같은 「그 자리에 놓기」라(PRD R12) 가를 것이 없다 — 남는 일은 「지금 커서가 어느 우리 창의 보인 가이드 위인가」이고, 그것을 마지막 보고 + 커서 사각형 검사가 답한다. Esc 취소는 대상 창에 `dragleave` 를 먼저 줄 수 있고(HTML 끌어 놓기 규칙 — 취소면 현재 대상에 떠남) 그 보고가 `dragend` 보다 셸에 먼저 닿을 수 있어 `LEAVE_GRACE_MS` 를 둔다.
  - 남는 오판 하나 — W 를 덮은 **다른 앱 창** 위로 나가 100ms 안에 떼면 그 밑 W 의 마지막 가이드로 붙는다(사용자가 그 가이드를 직전에 봤다 · §11-2 #1).
- **`dock_pointer_seen`** — 끄는 동안 웹뷰에는 포인터 사건이 오지 않는다(끌기가 대신한다 — 출처는 시작 때 `pointercancel`). 그래서 세션 중 어느 창이든 버튼을 뗀 마우스 이동 · 새 누름을 보면 끌기는 이미 끝났다(웹뷰 쪽 거름 = §1-4 · S1 ③). `Live` 면 `Grace` 로 두고 `END_GRACE_MS` 시한 — 그 안에 `drop` · `dragend` 가 오면 평소대로, 지나면 신호 유실: W 가 아직 위면 (W, 그 마지막 Over 보고)로 「놓기 판정」(커서는 그 뒤 움직였으므로 사각형 검사를 하지 않는다 · W 가 사라졌으면 판정이 「지금 영역 없음」으로 무변경), 아니면 W 에서 본 가이드가 없으므로 지금 커서 자리에 새 창(커서를 못 읽으면 위 「커서를 못 읽으면」).
- **그 밖의 끝** — 출처 창 `Destroyed` = `SourceDestroyed` · 스냅숏에 출처 View 없음 = `SourceGone` — 둘 다 **그냥 끝**(무변경 · 되보이기 없음 · PRD R13b 예외 · AC16). 단 `Applying` 중이면 적기만 한다(아래 「적용 중」). 최근 끝난 세션의 늦은 끝 신호 · OS 끌기는 버려진다.

#### 이른 끝 신호 — 시작보다 먼저 온 `drop` · 출처 끝 (5.1 · C3)

- **왜** — 시작(`dock_drag_start`)은 동기 명령(메인 스레드)이고 끝 신호(`dock_drop` · `dock_drag_end`)는 async 라 셸에 닿는 순서가 보장되지 않는다. 문턱 직후 곧바로 떼는 짧은 끌기(같은 창 · 이웃 창)는 끝 신호가 시작보다 먼저 장부에 올 수 있다 — 시작을 사건 디스패치 뒤로 미룬 것(§1-4 「다른 끌기 리스너」)이 이 틈을 조금 넓힌다. 5판은 그 신호를 모르는 id 로 버려 세션이 끝 신호 없이 남았다(떼어진 탭 · 표식이 `pointer_seen` 까지 남음).
- **규칙** — 장부가 모르는 `drag_id` 이고 최근 끝난 id 도 아니면 그 id 의 끝 신호를 도착 순서대로 `early` 에 붙들고 `EARLY_SIGNAL_MS`(500) 시한을 건다. 한 번에 한 id 다 — 다른 모르는 id 의 끝 신호가 오면 옛 것은 버린다. 그 안에 같은 id 의 `Start` 가 오면 시작 할 일을 낸 뒤 붙든 신호를 도착 순서대로 먹인다. 시작이 거절되면(`Refused`) · 시한이 지나면 버린다(debug). ★버린 id(시한 · 밀려남)는 `ended` 에 적는다(5.2 · 리뷰 R2-2)★ — 그 뒤에 그 id 의 시작이 오면 `LateStart` 로 거절된다(「시작」 1). 적지 않으면 그 시작이 끝 신호 없는 세션을 열어 `pointer_seen` 뒤 지금 커서에 새 창을 띄운다. 세션이 살아 있는 동안 온 모르는 id 의 끝 신호도 같다 — 그 시작은 `AlreadyDragging` 으로 거절되므로 곧 버려진다.
- **붙든 놓기의 판정** — 시작 전의 웹뷰는 영역이 없어 「가이드 없음」 · `rev` 0 을 싣는다. 놓기 판정 그대로 — 지금 판정도 「가이드 없음」이면 새 창, 시작이 보낸 영역으로 그 점이 가이드 자리면 무변경(사용자는 가이드를 못 봤다).

#### 적용 중 (`Applying` — 5.1 · F1)

- **스냅숏 · 소멸은 적기만 한다.** `Applying` 동안 스냅숏은 번호만 받아 두고(영역 · `Display` · 끝 판정 없음), `Destroyed{출처}` 는 `source_destroyed` 만 세운다. 끝은 `Applied` 하나가 정한다 — 성공이면 `applied`(대상 `set_focus` — R11b · AC3), 실패면 무변경(사유 = 그 `DockError`).
- **왜** — 성공한 Seam 은 출처 View 를 지우고 알림을 낸다. 그 알림의 계기가 먹인 스냅숏(출처 View 없음)이나, 빈 출처 팝아웃을 `host.close` 한 뒤 메인 스레드가 내는 `Destroyed` 가 `Applied` 보다 먼저 장부에 닿으면 5판 규칙으로는 세션이 `SourceGone` · `SourceDestroyed` 로 끝난다 — 대상 창에 포커스를 주지 않고(R11b · AC3 어김), `Ended` 가 그 계기의 스레드에서 출처 창의 탭 목록 알림보다 먼저 나가 떼어진 탭이 한 번 되비친다(아래 「끝」이 지키려던 순서). 계기를 알림 묶음 끝 한 번으로 모은 것(§1-2 「알림 묶음 끝」)은 두 번째 반만 막는다 — `Destroyed` 는 그 밖이다.
- `Applied` 는 `dock_tab` 을 부른 실행기가 그것이 돌아온 뒤(알림 · `settled` · `host.close` 다음) 같은 스레드에서 먹인다 — 그래서 `Ended` 는 적용 알림 뒤에 나간다.

#### 놓기 판정 (M1 · PRD R13c)

(W, rev, bar_rev, displayed, shown, point) 하나로 — `drop` · 「마지막 점에 놓기」 · 유실 · 붙든 이른 놓기가 같은 함수를 지난다. ★원칙 = 보인 것과 지금 판정이 같을 때만 무엇이든 바뀐다★:

1. `blocked` → 무변경(`SourceChanged`). (웹뷰가 `dropEffect = 'none'` 이라 보통 `drop` 은 오지 않는다 — 끝 신호로 온다.)
2. **지금 판정** — W 의 지금 영역 = 장부가 W 에 마지막으로 보낸 영역(W 가 스냅숏에서 빠졌거나 `Destroyed` 면 없음). `judged` = 지금 영역이 있으면 `pick(지금 영역, point)`, 없으면 「가이드 없음」.
3. `shown` = 가이드 없음 — `judged` 도 「가이드 없음」이면 **R22 새 창**(`rev` · `displayed` 는 보지 않는다 — 새 창은 대상 기대값이 없다), 아니면 **무변경**(`TargetChanged` — 사용자가 가이드를 못 본 자리에 지금은 가이드가 있다).
4. `shown` = 가이드 — 모드마다 따로 대조한다(5.2 · 리뷰 B1). 하나라도 다르면 **무변경**(`TargetChanged`) — W 의 영역이 사라진 경우(대상 창이 닫힘 · Q18)도 여기다. 영역을 보낸 뒤 모델이 바뀌었으면 모델 판정 6 이 같은 락 안에서 `TargetChanged` 로 막는다.
   - 4a. **이음새** — `rev` = W 의 지금 `rev` · `displayed` = 그 영역의 `view_id` · `judged` = `shown` 이 **모두** 맞을 때만 Seam{target = 그 영역의 보이는 탭, anchor, side} 를 `DockGuard{lease, expect_source, expect_target = Seam{배치, 창 활성}}` · `Reveal` 로 적용.
   - 4b. **탭바** — `bar_rev` = W 의 지금 `bar_rev` · `judged` = `shown`(같은 `index` — `over` 는 보지 않는다)이 맞을 때만 TabBar{window W, index} 를 `DockGuard{lease, expect_source, expect_target = Tabs(탭 순서)}` · `Reveal` 로 적용. ★`rev` 와 `displayed` 는 보지 않는다★ — 탭바 영역 · 탭 순서는 어느 탭이 보이는지와 무관하고, 활성 전환은 `rev` 만 올리므로 그것까지 보면 전환 알림 틈의 탭바 놓기가 무변경이 된다(웹뷰는 그 틈에도 탭바 가이드를 그린다 — §1-4). 모델 쪽 기대값 `TargetExpect::Tabs` 도 활성을 보지 않는다(이름은 2 단계 그대로 `Tabs`).

- ★5판의 「지금 판정이 가이드 없음이면 보고가 무엇이든 새 창」(4판에서 이어짐)을 버렸다(5.1 · 리뷰 C2 · F3)★ — 사용자가 가이드를 봤는데 그 영역이 바뀌거나 사라져 지금 판정이 없음이면, 새 창은 못 본 결과다(M1 · R13c 「판정이 마지막으로 보인 가이드와 다르면」 · Q18 「대상 사라짐 = 무변경」). 새 창은 사용자도 가이드를 못 봤고 셸도 가이드가 없다고 판정할 때뿐이다.
- **보이는 탭 경쟁(5.1 · C1)** — `switch_tab`(R6b 머묾 · 사람 클릭 · 다른 경로)은 활성을 레이아웃 락 안에서 바꾸고 알림은 락을 놓은 뒤 낸다. 그 사이 웹뷰 화면 · 장부 영역 · 모델이 서로 다른 탭을 가리킬 수 있다. 세 겹으로 맞춘다 — ① 웹뷰는 그 창 캔버스가 보이던 탭(`displayed` — 떼어진 표시의 승계 탭 포함)을 보고 · 놓기에 싣고, 그것이 영역의 `view_id` 와 다르면 그 영역으로 그리지 않는다(가이드 없음 — §1-4) ② 장부는 `displayed` ≠ 그 `rev` 의 `view_id` 면 이음새 놓기를 무변경으로(위 4a — 탭바 놓기는 이 대조를 하지 않는다 · 4b) ③ 기대값 `TargetExpect::Seam` 이 그 `rev` 때 대상 창의 **모델** 활성을 쥐고 모델이 같은 락 안에서 대조한다(§1-2 「4 단계 고침 둘」). ①②는 웹뷰 · 장부 사이, ③은 장부 · 모델 사이의 틈을 닫는다 — ③이 없으면 활성이 바뀐 직후의 이음새 놓기가 이제는 안 보이는 탭에 들어가고 `Reveal` 이 그 탭을 도로 앞에 낸다.
- **새 창** — `NewWindow{at: pos = 커서 − hotspot × 출처 창 배율}`(`hotspot` = 시작 때 웹뷰가 알린 그림 안 커서 자리 · 배율은 실행기가 읽는다 — 그림은 출처 웹뷰가 그 배율로 만든다 · 그림이 있던 자리에 창이 뜬다 · 크기는 적용 서비스가 R22 규칙으로) · `DockGuard{lease, expect_source}`. 커서를 못 읽었으면 「마지막으로 알려진 점」(위)으로, 그것도 없으면 `pos = None`(기본 자리).
- **적용 결말** — `Applied` 가 정한다(위 「적용 중」). 성공: 대상(새 · 재사용) 창 `set_focus()`(R11b — 첫 슬롯 포커스는 적용 서비스가 끝냈다) · 재사용이면 그 창을 보인다. 실패(사유 무관): 무변경.

#### 끝 (모든 갈래 공통)

(성공이면 적용 서비스의 알림이 이미 나갔다 — `Applied` 는 그 뒤에 먹는다) → `Broadcast(Ended)` → 숨긴 출처 되보이기(무변경 끝 · 재사용 끝 · 출처가 사라졌으면 건너뜀) → `set_focus`(성공) → `release_lease`(성공이면 이미 내려갔다 — 무동작) → 세션 깃발 내림(자기 세대일 때만 — 비교-교체 · 5.2) · `ended` 에 id 적기 → 끝 로그. `Ended` 를 적용 알림 **뒤에** 내므로 출처 창은 탭이 빠진 목록을 먼저 받고 떼어진 표시를 거둔다 — 탭이 한 번 비쳤다 사라지지 않는다(같은 스레드의 emit 순서 — 가능성 높음 · QA). `dock:session` 은 세션 안 단조 `seq` 를 싣고, 웹뷰는 `drag_id` 별로 가장 큰 `seq` 의 상태만 쓴다 — `Started` · `Display` 는 같은 칸(`source_label` · `source_view` · `display_active`)을 다 실어 어느 것이 먼저 닿아도 같은 상태가 서고, `Ended` 는 그 id 의 마지막이다(뒤에 닿은 작은 `seq` 는 버린다).

#### R6b 탭 전환

대상 웹뷰가 「보이지 않는 탭의 머리」 위 머묾을 재고(`HOVER_SWITCH_MS` 500ms) 기존 탭 전환(`viewStore` 의 탭 전환 → `switch_tab` — 사람 클릭과 같은 길)을 부른다. 모델 활성이 바뀌면(`version` +1 · 저장) 모델 계기가 그 창 영역을 다시 보낸다(`view_id` · 기대값의 활성이 바뀌어 `rev` +1). 그 알림 · 영역이 닿기 전의 이음새 놓기는 「보이는 탭 경쟁」(위)으로 무변경이다. 탭바 놓기는 탭바 몫(`bar_rev`)과 탭 순서만 보므로 그대로 들어간다(「놓기 판정」 4b — 5.2). ★끄는 동안 모델을 바꾸는 유일한 동작이고, 끝이 무변경이어도 되돌리지 않는다(§11-1 #4)★. 장부는 머묾을 모른다 — UI 시간 재기는 그리는 쪽 몫이고 쓰기는 기존 표면이다.

### 1-4. ③ 웹뷰 — HTML5 끌어 놓기 (렌더링 + 사건 전달만)

#### 출처 — 탭 머리

- **`draggable`** — 탭 머리마다(이름 입력 중 · 닫기 버튼에서 시작한 끌기는 `dragstart` 에서 `preventDefault`). 문턱 · 캡처 · 끌기 그림 이동 · 창 밖 추적은 엔진 몫이다 — 4판의 4px 문턱 · `setPointerCapture` · `pointerup` · `blur` 리스너는 없다. 끌기가 시작되면 엔진이 그 누름의 `click`(탭 전환)을 내지 않는다.
- **`dragstart`(동기 — 이 안에서 끝낼 것)** — `drag_id = crypto.randomUUID()` · `setData('application/x-engram-tab', JSON{drag_id, view_id})` ★이 형식 하나만★(`text/plain` · URL 을 싣지 않는다 — 다른 앱이 받으면 `dropEffect` 가 none 이 아니게 되고 그 앱에 글자가 들어간다) · `effectAllowed = 'move'` · `setDragImage(끌기 그림 요소, 커서 자리)`(§4-2) · `stopPropagation()`(아래 「다른 끌기 리스너」).
  - **셸 세션은 끌기가 정말 시작됐을 때만 연다(5.1 · F2)** — invoke `dock_drag_start{drag_id, view_id, hotspot = 그 커서 자리}`(기다리지 않는다)는 `dragstart` 안이 아니라 그 사건 디스패치가 끝난 뒤(`setTimeout 0`) 원 사건(`nativeEvent`)의 `defaultPrevented` 가 거짓일 때만 부른다. 뒤 리스너가 시작을 취소하면 OS 끌기가 없어 끝 신호도 오지 않는다 — 그때 세션을 열면 떼어진 탭 · 표식 · 숨긴 팝아웃이 `pointer_seen` 까지 남는다. 디스패치 안의 마이크로태스크로는 안 된다(리스너 하나가 끝날 때마다 돌아 뒤 리스너보다 먼저다). 이 미룸으로 끝 신호가 시작보다 먼저 셸에 닿을 수 있고, 그것은 장부가 붙든다(§1-3 「이른 끝 신호」).
  - ★`dragstart` 안에서 DOM(탭 머리 · 탭 목록)을 바꾸지 않는다★ — 끌리는 요소를 그 자리에서 숨기거나 빼면 Chromium 이 끌기를 곧바로 끝낸다(가능성 높음 · S1 ④). 떼어진 표시는 `Started` 를 받은 뒤다.
- **`dragend`** → invoke `dock_drag_end{drag_id, effect: dropEffect}`. ★끌린 머리 요소는 끄는 동안 DOM 에 남긴다★(React key 그대로 · CSS 로 접기) — 빼면 `dragend` 가 그 요소 밖으로 올라오지 않는다(가능성 높음 · S1 ④). 그 요소에 리스너를 단다.

#### 대상 — 모든 창(출처 창 포함 · 숨긴 출처 제외)

- **받기** — `document` 의 `dragenter` · `dragover` 에서 `types` 에 우리 MIME 이 있으면 ★세션을 몰라도(`Started` 전이어도 — 5.1 · C3)★ `preventDefault()` + `dropEffect = blocked ? 'none' : 'move'` + `stopPropagation()`. `blocked` 는 그 끌기의 영역(`DockRegions.blocked`)으로 알고, 영역 전엔 `move` 다. 5판은 `Started` 를 안 뒤에만 받았는데, 이웃 창으로 빠르게 끈 끌기는 그 창에 `Started` 보다 먼저 닿을 수 있어 그 창이 끌기를 거절하고(커서 「안 됨」 · `none` 끝) 「마지막 점에 놓기」가 보고 없는 끌기로 새 창을 띄웠다. `drag_id` 확인은 셸이 한다(모르는 id 의 놓기는 장부가 붙들었다 버린다 — §1-3). `dragover` 에서는 데이터를 못 읽는다(보호 모드 — `types` 만) — 「지금 끌기」 = `dock:session` · `dock:regions` 로 가장 늦게 알게 된, 끝나지 않은 `drag_id`.
- **영역 · 세션 받기(5.1 · C3 · F5)** — `drag_id` 별로 가장 큰 `rev`(영역) · `seq`(세션)만 쓰고 작은 것은 버린다 · `Started` 전에 온 영역 · `Display` 도 받아 둔다 · `Ended` 된 id 의 것은 버린다(§1-3 「영역」 순서).
- **판정 · 그리기** — 점 = `(clientX, clientY)`(그 창 뷰포트 CSS px) → `pickTarget(영역, 점)`(`dockPick.ts` — §2-4 의 거울) → `DockOverlay` 상태. ★그 창 캔버스가 보이는 탭(`displayed` — 모델 활성이거나 떼어진 표시의 승계 탭)이 영역의 `view_id` 와 다르면 그 영역으로 그리지 않는다(가이드 없음 · 탭바는 그대로 그리고 탭바 놓기도 선다 — 5.1 · C1 · 5.2)★ — 탭 목록 알림과 영역이 엇갈려 닿는 틈에 다른 탭의 칸 위에 옛 가이드를 그리지 않게. `shown` · 영역 `rev` · `displayed` 중 하나가 바뀌면 `dock_drag_over{Over}`(그 영역의 `bar_rev` 를 함께 싣는다)(`drag_id` 를 모르면 보내지 않는다). 영역이 없으면 `shown = 가이드 없음`이고 `dragenter` 에서 `dock_pull` 한 번.
- **떠남** — 문서 밖으로 나간 `dragleave`(관련 요소 없음)에서 가이드 · 머묾을 지우고 `dock_drag_over{Left}` · `stopPropagation()`.
- **`drop`** — `preventDefault()` + `stopPropagation()` · `getData` 로 `{drag_id, view_id}` · `dock_drop{drag_id, view_id, rev, bar_rev, displayed, shown = 마지막으로 그린 것, point}` · 가이드를 지운다.
- **머묾(R6b)** — `pick` 의 탭바 `over` 가 보이지 않는 탭이면 500ms 시한 → 탭 전환. `over` 가 바뀌거나 떠남 · 놓기 · `Ended` 면 취소. 탭바 오버레이가 대기 중인 머리에 진행 표시를 한다.

#### 세션 동안 그 밖 (`dock:session` 구독 한 곳)

- **떼어진 표시**(§4-1) — `Started` · `Display` 로 세우고 `Ended` 로 거둔다.
- **끝 신호 유실 대비** — `Started` 뒤 그 창의 첫 「끝 증거」 하나만 `dock_pointer_seen`(세션마다 창마다 한 번). 끝 증거 = `pointerType === 'mouse'` 이고 `buttons === 0` 인 `pointermove` · 또는 `pointerdown`(끄는 동안 새 누름은 없다 — 5.1 · N1). 버튼 조건을 두는 이유 — 떼어진 표시가 머리를 접는 재배치 뒤 엔진이 멈춘 커서 밑에 합성 이동을 낼 수 있다(미검 · S1 ③). 그것이 끄는 중(버튼을 쥔 채)에 오면 걸러진다. 대가 = Esc 로 끝낸 뒤 버튼을 쥔 채 움직이면 증거가 버튼을 뗄 때까지 늦는다 — 이것은 끝 신호를 잃었을 때만의 대비라 무해하다.
- **Esc 삼키기** — `Started` 부터 `Ended` 뒤 `ESC_SWALLOW_TAIL_MS`(200ms)까지 `window` capture 단계 `keydown` 에서 `Escape` 를 `preventDefault` + `stopPropagation`(보내지는 않는다 — Esc 는 OS 끌기 루프가 받아 끌기를 끝낸다). OS 가 그 keydown 을 웹뷰에 넘기는지는 모른다 — 넘기면 터미널(claude 턴 끊기) · 입력창에 닿지 않게 하는 보험이다(S1 ⑥). ★상한(5.1 · N9)★ — 그 창이 위 끝 증거를 보면 그때부터 200ms 뒤 그 창에서는 그친다(세션이 아직 살아 있어도). 끝 신호를 잃어 세션이 남아도 마우스를 다시 쓴 창의 Esc(claude 턴 끊기)를 계속 먹지 않는다 — 그 증거가 `pointer_seen` 으로 세션도 `END_GRACE_MS` 뒤 끝낸다. 남는 것 = 마우스를 안 쓰고 키보드만 쓰는 창은 세션이 끝날 때까지 Esc 가 먹힌다(세션이 남는 것 자체가 끝 신호 유실 — §11-2 #2).
- **`Ended`** — 가이드 · 머묾 · 떼어진 표시를 무조건 지운다(R13b 「남은 가이드 없음」).

#### 늘 — 파일 끌어 놓기 막기(PRD AC22 · W9)

모든 창 `document` 에 우리 MIME 이 아닌 끌기면 `dragover` 에서 `preventDefault()` + `dropEffect = 'none'`, `drop` 에서 `preventDefault()`. 드롭 처리기를 끈 창(main 은 지금도 `dragDropEnabled: false` — `tauri.conf.json:21`)에 OS 파일을 놓으면 웹뷰가 그 파일로 탐색할 수 있다(미검 — S3 ②). 세션과 무관하게 늘 둔다. ★우리 MIME 이 아닌 사건은 끊지 않는다(`stopPropagation` 없음)★ — 트리 행 재부모화 끌기는 이 리스너 뒤의 window 리스너(react-dnd)가 자기 `dropEffect` 로 덮어 그대로 돈다(아래 · S1 ⑦).

#### 다른 끌기 리스너 — react-arborist (5.1 · F2)

- **무엇이 막나(확실 · 소스)** — 에이전트 트리(`src/components/agent/AgentList.tsx` — 슬롯 내용 `agent_list` · `LayoutLeaf.tsx:319` · keep-alive 라 안 보이는 탭의 트리도 마운트돼 있다)는 react-arborist 3.9.0 이고, 그 `DndProvider` 는 react-dnd HTML5 백엔드를 `dndRootElement` 를 안 넘기면 뿌리 = `window` 로 단다(`react-arborist/dist/module/components/provider.js:54-60` · `react-dnd-html5-backend/dist/esm/OptionsReader.js:48-52`). 그 백엔드의 window `dragstart`(버블)는 자기 끌기 원천도 브라우저 기본형(파일 · URL · 글자 · HTML)도 아닌 끌기를 `preventDefault` 한다(`HTML5BackendImpl.js:112-213` — :212) — 우리 MIME 하나만 실은 탭 끌기는 OS 끌기가 시작되지 않는다. window `dragover`(버블)는 자기 끌기 중이 아니면 `preventDefault` + `dropEffect = 'none'` 이다(:280-292) — 우리 `document` 리스너가 먼저 `move` 를 두어도 덮여 놓기가 거절된다. 트리 슬롯이 하나라도 마운트된 창이면 다 그렇다. 2026-10-16 스파이크는 앱 의존이 없는 맨 페이지라 이것을 못 봤다(§8-1).
- **대응 = 우리 MIME 사건이 window 에 닿지 않게 끊는다** — 탭 머리 `dragstart`(React 처리기 — React 는 뿌리 컨테이너에서 처리기를 부르므로 원 사건이 거기서 멈춘다)와 `document` 의 `dragenter` · `dragover` · `dragleave` · `drop` 이 우리 MIME 이면 `stopPropagation()`(위 「출처」 · 「대상」). react-dnd 의 window **캡처** 리스너는 우리보다 먼저 돌지만 브라우저 기본형이 아닌 끌기에는 판정용 칸을 비우거나 세기만 한다(`:106-110` · `:225-241` · `:276-278` · `:335-369` — 읽음). `dragend` 는 끊지 않는다(window 쪽은 캡처 리스너 하나뿐이고 자기 끌기가 아니면 아무것도 안 한다 · `:216-223`).
- **뿌리 좁히기(`dndRootElement` = 트리 컨테이너)는 택하지 않는다** — prop 은 있지만(3.9.0 · `types/tree-props.d.ts:55`), react-dnd 의 `DndProvider` 는 `manager` 를 안 받으면 창(전역 문맥)마다 관리자 하나를 처음 마운트한 트리의 옵션으로 만들어 나눠 쓴다(`react-dnd/dist/esm/core/DndProvider.js:63-88`) — 한 창에 트리 슬롯이 둘이면 둘째 트리의 뿌리가 첫째 트리 컨테이너가 되어 둘째의 재부모화가 끊긴다. 트리마다 관리자(`dndManager`)를 만들려면 `dnd-core` 를 직접 의존해야 한다(지금은 간접). 끊기는 트리 코드 · 의존을 건드리지 않는다.
- **불변식** — react-dnd 의 뿌리는 `window` 이고 우리 리스너는 `document` 다(뿌리가 `document` 면 같은 노드의 등록 순서가 이겨 끊기가 안 선다 — 컨테이너로 좁히는 것은 괜찮다: 그쪽이 먼저 돌고 우리가 뒤에서 `move` 로 덮는다). 트리에 `dndRootElement` · `dndManager` 를 주거나 HTML5 끌기 라이브러리를 새로 들이면 S1 ⑦ 을 다시 돈다(§9 「게이트」의 점검 줄). `src/` 의 그 밖 HTML5 끌기 리스너는 없다(2026-10-16 · `rg` — `addEventListener('drag…' | 'drop')` · JSX `onDrag*` · `onDrop` · `draggable` · `dataTransfer` 전부 0 · 분할선은 포인터 사건).

### 1-5. 웹뷰 ↔ 셸

| 방향 | 이름 | 꼴 | 페이로드 |
|---|---|---|---|
| 웹뷰 → 셸 | `dock_drag_start` | invoke(동기) | `{ drag_id: String, view_id: Uuid, hotspot: PointCss }` → `()` · `hotspot` = 끌기 그림 안의 커서 자리(R22 새 창 자리에 쓴다 — 상수를 두 언어에 두지 않는다) · `Err` = `TabNotFound` · `AlreadyDragging` · `LateStart` · `Dragging` · `LayoutPoisoned`(웹뷰는 무시 — 그 끌기의 놓기는 아무것도 바꾸지 않고 끝난다) · 부르는 때 = `dragstart` 디스패치 뒤 · 취소되지 않았을 때만(§1-4) |
| 웹뷰 → 셸 | `dock_drag_over` | invoke(동기) | `{ drag_id, report: Over{ rev: u64, bar_rev: u64, displayed: Option<Uuid>, shown: Picked, point: PointCss } \| Left }` → `()` · 창 label 은 Tauri 가 넣어 준 창 · `displayed` = 그 창 캔버스가 보이던 탭(5.1) · `bar_rev` = 쓰던 영역의 탭바 몫(5.2) |
| 웹뷰 → 셸 | `dock_drop` | invoke(async) | `{ drag_id, view_id, rev, bar_rev, displayed: Option<Uuid>, shown: Picked, point: PointCss }` → `()` · `view_id` = 끄는 탭(끌기 데이터) · `displayed` = 놓은 창이 보이던 탭 |
| 웹뷰 → 셸 | `dock_drag_end` | invoke(async) | `{ drag_id, effect: Move \| None \| Other }` → `()` · `Other`(copy · link) = 받은 창이 있다 |
| 웹뷰 → 셸 | `dock_pointer_seen` | invoke(동기) | `{ drag_id }` → `()` |
| 웹뷰 → 셸 | `dock_pull` | invoke(동기) | `()` → `Option<{ started: Started, regions: Option<DockRegions> }>` — 지금 세션과 그 창 영역 |
| 웹뷰 → 셸 | `report_dock_chrome` | invoke(측정) | `DockChrome{ canvas_origin, tab_bar, tabs: Vec<TabHeader> }` — 끄는 동안 접힌 머리는 싣지 않는다 · 창 label = Tauri 가 넣어 준 창(`report_window_canvas` 와 같다) |
| 셸 → 웹뷰 | `dock:session` | emit(전체) | `{ drag_id, seq, phase: Started{ source_label, source_view, display_active: Option<Uuid> } \| Display{ source_label, source_view, display_active } \| Ended }` — `Display` 도 같은 칸을 다 싣는다(5.1 — 어느 것이 먼저 닿아도 같은 상태) · 웹뷰는 `drag_id` 별 가장 큰 `seq` |
| 셸 → 웹뷰 | `dock:regions` | emit_to(label) | `DockRegions`(아래) · 웹뷰는 `drag_id` 별 가장 큰 `rev` · `Started` 전에 온 것도 받는다 |
| 셸 → 웹뷰 | `layout:updated` · `window:tabs-updated` | 기존 | 도킹 결과는 **기존 알림으로만** 흐른다 |
| 웹뷰 ↔ 웹뷰 | `application/x-engram-tab` | OS 끌기 데이터 | `{"drag_id": "…", "view_id": "…"}` — 받는 쪽은 `drop` 에서만 읽는다 |

```rust
// dock/types.rs (ts-rs · 6 단계 — 5 단계의 GuideView · TabBarHint 를 대신한다. PxRectWire · Band 는 그대로)
pub struct DockRegions {
    pub drag_id: String, pub rev: u64,
    pub bar_rev: u64,                   // 탭바 몫(`tab_bar` · 탭 목록 기대값)이 바뀔 때만 오른다 — 탭바 놓기의 대조 기준(5.2)
    pub blocked: bool,                  // 출처 배치가 바뀜 — 받지 않는다(dropEffect none) · 아래는 비어 있다
    pub view_id: Option<Uuid>,          // 이 창에 보이는 탭 — None = 없음(메인 마지막 탭을 뗌 · 측정 전) · 웹뷰의 `displayed` 와 대조(C1)
    pub canvas_origin: PointCss, pub canvas: CanvasPx,
    pub outer: Vec<OuterBand>,          // 유효한 바깥 변만(R8)
    pub slots: Vec<SlotRegion>,         // 보이는 탭의 칸 전부
    pub tab_bar: Option<TabBarRegion>,  // None = 탭바 놓기를 받지 않음(크롬 보고 전)
}
pub struct OuterBand { pub side: DockSide, pub strip: PxRectWire /* 그리기 · 판정 */, pub landing: PxRectWire /* 칠할 결과 */ }
pub struct SlotRegion { pub slot_id: Uuid, pub frame: PxRectWire, pub bands: Vec<Band> /* rect = 결과 사각형 = 띠(R3) */ }
pub struct TabBarRegion { pub rect: RectCss, pub heads: Vec<TabHeader> /* 끄는 탭 뺌 */, pub insert_xs: Vec<f64> /* 자리 0..=len 의 끼울 x */ }
pub enum Picked { None, Seam { anchor: Option<Uuid> /* None = 바깥 변 */, side: DockSide }, TabBar { index: u32, over: Option<Uuid> } }
```

- 사각형은 전부 그 창 캔버스(칸 · 띠) 또는 뷰포트(탭바) 의 CSS px 다 — 웹뷰는 좌표 계산 없이 그대로 점-사각형 검사를 하고 그린다(ADR-0227 결정 1 과 같은 태도). 화면 좌표 · 배율은 웹뷰로 가지 않는다.
- **크롬 보고를 늘 하는 이유** — 끌기 시작 때 보고를 기다리면 첫 영역이 늦다. 탭바 · 캔버스 원점은 드물게 바뀌므로 평소에 보고하고(디바운스 · 한 번에 하나 — `windowCanvasReport.ts` 와 같은 꼴), 셸은 `WindowTabs` 의 측정 칸 `chrome`(`canvas` · `metrics` 옆)에 둔다 — `version` 을 올리지 않는다(ADR-0227 의 측정 규칙). 끄는 동안 떼어진 표시가 머리를 접으면 다시 보고한다.

### 1-6. ③' OS 층 — 새 함수 없음

- 커서 추적 · 맨 위 창 · 버튼 · Esc · 끌기 그림은 엔진(WebView2 브라우저 프로세스의 OS 끌기 루프)이 한다. 셸 메인 스레드는 막히지 않는다(실측 — §8-1).
- 셸이 직접 쓰는 OS 값은 셋이고 전부 Tauri API 다: 커서(`cursor_position`) · 창 사각형(`inner_position` · `inner_size` · `is_visible` · `is_minimized`) · 배율(`scale_factor`). 전부 레이아웃 · 장부 락 밖에서 읽는다(§1-3 「창 게터」). 그래서 `platform` · 게이트 4f · 4g · 4h 명단 · 4i 가 하나도 안 바뀐다.
- **드롭 처리기 끄기** — `WebviewEnv::finish` 에 `.disable_drag_drop_handler()`(tauri `webview_window.rs:1031`)를 더한다 — 창을 만드는 모든 코드가 지나는 한 곳(ADR-0283)이라 팝아웃 · 앞으로의 런타임 창이 다 덮인다. 켜 두면 wry 가 자기 `IDropTarget` 을 달아 HTML5 끌어 놓기가 막힌다(`wry-0.55.1/src/webview2/drag_drop.rs:45-75` · `mod.rs:150-157`). 잃는 것 = Tauri `DragDropEvent`(우리 코드 사용 0 — rg) · wry 의 외부 드롭 거절(→ §1-4 파일 막기).
- **OS 별 일이 생기면** — 실측(§8-2)이 Esc 구분에 키 · 버튼 상태가 필요하다고 보이면 그때 platform 목적 단위 함수 하나(「주 버튼이 지금 눌려 있나」)를 들인다(ADR-0266 · 사용자 보고). 지금은 넣지 않는다.
- **macOS** — WKWebView 의 창 사이 끌어 놓기 동작은 모른다. 이식 때 §8-2 의 같은 실측을 돌린다.

### 1-7. 계측 (`docs/reference/logging-conventions.md` 「계측 의무」)

끌기는 세션 수명 · 동시성 · 외부 경계(창 만들기 · 숨기기)를 가진 load-bearing 경로다. 그룹 = 기본 모듈 경로(`engram_dashboard::dock` · `engram_dashboard::commands::dock` · `engram_dashboard::layout`). 식별자 · 수치는 필드로, 메시지는 한국어 한 줄. 상태 전이 · 가드 발동(옛 번호 · 옛 `rev` · 모르는 id 버림)은 같은 문서 「동시성 전이」 의무다(5.1 · N7 — 「장부」 줄들).

| 어디 | 무엇 | 레벨 | 필드 |
|---|---|---|---|
| `dock_drag_start` | 끌기 시작(표식) | info | `drag` · `view` · `source` · `tabs` · `hidden` |
| 같은 곳 | 거절 | debug | `drag` · `view` · `reason`(`TabNotFound` · `AlreadyDragging` · `applying` · `LateStart` — 붙들다 버린 id 의 늦은 시작 · `Dragging` — 옛 표식이 내려가기 전 · `LayoutPoisoned`) |
| 같은 곳 | 출처 숨기기 실패(끌기는 이어짐) | warn | `label` · `: {e}` |
| 실행기 | emit · 보이기 · 커서 · 창 사각형 읽기 실패 | warn(세션당 처음 한 번) · 나머지는 끝 줄 `errors` 셈 | `label` · `: {e}` |
| 실행기 | 늦은 신호(최근 끝난 id) · 모르는 id 의 `Over` · `Left` · `pointer_seen` 버림 | debug | `drag` · `current` · `event` |
| 장부 | `blocked` 전이(출처 배치가 바뀜 — 모든 창 영역 비움) | info | `drag` · `view` |
| 장부 | 옛 스냅숏 번호 버림 · 세션 전 스냅숏 들고 있다가 이어 먹음 | debug | `drag` · `snap` · `last` · `action = drop \| hold \| replay` |
| 장부 | 옛 `rev` 의 보고 → 지금 영역 다시 보냄 | debug | `drag` · `label` · `rev` · `current` |
| 장부 | 모르는 `drag_id` 의 끝 신호 붙듦 · 시작에 이어 먹임 · 버림(시한 · 거절 · 다른 id 가 밀어냄 — 시한 · 밀려남으로 버린 id 는 `ended` 로) | debug | `drag` · `event` · `action = park \| replay \| discard` · `cause` |
| 장부 | 적용 중 스냅숏 · 출처 소멸을 적기만 함 | debug | `drag` · `event` |
| 장부 끝 | 끌기 끝 — 한 줄 | info | `drag` · `outcome = applied \| noop \| aborted` · `reason` · `end_by = drop \| source_none \| drop_wait \| signal_lost \| new_start \| source_gone` · `mode = seam \| tab_bar \| new_window \| reuse` · `window` · `over_window` · `leave_gap_ms`(「마지막 점에 놓기」에서 끝 신호와 W 떠남 보고의 간격 — S1 이 시한을 다시 잰다) · `cursor_read`(커서를 읽었나) · `early`(붙든 신호로 끝났나) · `reports` · `regions_sent` · `regions_resent` · `elapsed_ms` |
| 장부 끝 | `reason` 어휘 | — | `SourceChanged` · `TargetChanged` · `StaleLease` · `ZeroArea` · `WindowClosed` · `TabNotFound` · `WindowNotFound` · `SlotNotFound` · `SlotNotInTab` · `WindowOpen` · `SourceDestroyed` · `SourceGone` |
| `apply::dock_tab` | 적용 성공 | info | `view` · `mode` · `window` · `closed_window` · `reused` · `reuse_skipped = no_bounds`(탭 하나 팝아웃인데 보통 자리가 없어 새 창으로 — 5.4) · `main_refilled` |
| 같은 곳 | 거절(`DockError`) | debug(마우스 경로는 끝 줄이 남긴다) | `view` · `mode` · `reason` |
| 같은 곳 | 새 창 phase C 가 묘비로 멈춤 · 연 창 거두기 | warn | `label` |
| 같은 곳 | 재사용 재확인(C′) 실패 · B′ 세터 실패 → 옮긴 창 되돌림(5.2 · 5.3) | info(되돌리기 걸음이 실패하면 warn) | `label` · `reason` · `move_failed`(B′ 의 걸음) · `restored` · `failed`(되돌리기에서 실패한 걸음) · `was_maximized` |
| 실행기 | `Apply` 를 async 로 넘김(먹인 스레드가 동기 명령이었는지) | debug | `drag` · `from_sync` |
| `lib.rs` `CloseRequested` | 팝아웃 모델 먼저 지움 + 묘비(3 단계) | info | `label` · `dropped_views` |

`let _ =` 로 버리는 실패는 없다. 끄는 동안 반복되는 것은 셈으로 끝 줄에 모은다.

## 2. 대상 판정 — 순수 함수

`src-tauri/src/dock/hit.rs`(5 단계 · 끝) · `src-tauri/src/dock/regions.rs`(6 단계) — Tauri · 락 · OS 호출 0. 같은 입력이면 같은 답이다(R6a ③).

### 2-1. 입력 · 출력

```rust
// hit.rs (끝)
pub const OUTER_BAND_PX: u32 = 32;                                   // R5 — CSS px 정수(그린 띠와 판정이 같은 정수 사각형)
pub const SIDE_ORDER: [DockSide; 4] = [Left, Right, Top, Bottom];    // 동률 깨기
pub struct WindowFrame<'a> { label, inner_origin: (f64, f64), scale: f64, chrome: &'a DockChrome, canvas: CanvasPx,
                             visible_view: Uuid, slots: &'a [(Uuid, PxRect)] }
pub enum Locate { Nothing, TabBar { label, index, over }, Outer { label, view, sides, slot, p }, Slot { label, view, slot, frame, p } }
pub fn locate(cursor: (f64, f64), top: Option<&WindowFrame>, source_view: Uuid) -> Locate;            // 1 단계 — 어디 위인가
pub fn choose_seam(loc: &Locate, outer: &[(DockSide, PxRect)], bands: &[(DockSide, PxRect)]) -> Option<(DockAnchor, DockSide)>;
pub fn choose_band(p: (f64, f64), frame: PxRect, bands: &[(DockSide, PxRect)]) -> Option<DockSide>;
pub fn landing_rect(target, anchor, side, incoming, canvas) -> Option<PxRect>;   // 띠 = 놓았을 때 끼운 것의 px 사각형(R3)
pub fn valid_bands(target, anchor, incoming, canvas) -> Vec<(DockSide, PxRect)>; // 면적 0 변은 빠진다(R8)
pub fn outer_strips(canvas) -> [(DockSide, PxRect); 4];
pub fn tab_insert_x(chrome, source_view, index) -> f64;

// regions.rs (6 단계)
pub struct WindowSnap { label, tabs: Vec<ViewId>, active: ViewId, visible: Option<(ViewId, LayoutNode)>,
                        canvas: Option<CanvasPx>, chrome: Option<DockChrome>,
                        hidden: bool /* 장부 값(source_hidden) — 게터 아님 · §1-3 */, is_source: bool }
pub fn regions_for(w: &WindowSnap, source_view: Uuid, incoming: &LayoutNode, blocked: bool)
    -> Option<(DockRegions /* drag_id · rev · bar_rev 는 장부가 채운다 */, Expect /* TargetExpect 재료 — Seam{배치, active} · Tabs */)>;   // None = 숨긴 출처
pub fn pick(r: &DockRegions, p: PointCss, source_view: Uuid) -> Picked;   // = locate + choose_seam 을 영역 값 위에서
```

`pick` 은 새 규칙이 아니다 — 영역에서 `WindowFrame`(안쪽 원점 (0, 0) · 배율 1 · 크롬 = 영역의 탭바 · 머리 · 캔버스 원점)과 띠 목록을 다시 세워 `locate` → `choose_seam` 을 부른다. 규칙의 정본은 `hit.rs` 하나이고, 웹뷰의 `pickTarget` 은 그 거울이다(§2-4 골든).

### 2-2. 규칙 (`hit.rs` — 5 단계에 박힌 그대로)

1. **맨 위 창 없음** → `Nothing`. (5판: 맨 위 창은 엔진이 고른다 — 사건을 받은 창이 그 창이다.)
2. **창 안쪽 좌표** — `css = (cursor − inner_origin) / scale`. 5판의 실행 경로는 뷰포트 CSS px 를 그대로 넣는다(원점 (0, 0) · 배율 1) — 화면 좌표 변환이 판정에서 빠졌다(W4 가 좁아졌다).
3. **탭바** — `css` 가 탭바 안이면 `TabBar`. ★머리 목록에서 `view_id == source_view` 인 것을 먼저 뺀다★ — 거른 목록 = 결과 목록이라 `index` = 가운데 x 가 커서보다 왼쪽인 머리의 수 · `over` = 커서를 품은 머리. id 로 거르는 것은 목록만 고친다 — 기하는 다음 크롬 보고가 고친다(그 틈의 처리 = §2-4).
4. **캔버스** — `p = css − canvas_origin` · `p ∉ [0, W) × [0, H)` 면 `Nothing`.
5. **끄는 탭 자신** — `visible_view == source_view` 면 `Nothing`(방어 — 보이는 탭이 없는 창도 `Nothing`).
6. **바깥 띠(R5 · R6a ①)** — `outer_strips` 의 반열림 사각형에 든 변 **전부**를 캔버스 변까지 거리가 작은 순(동률 `SIDE_ORDER`)으로 `Outer.sides` 에 싣고 그 밑 칸 · `p` 도 싣는다. 슬롯 띠보다 먼저 본다(AC4).
7. **칸 찾기** — 반열림 `[x0, x1) × [y0, y1)` · 이웃 칸은 같은 정수 경계를 나눠 점 하나에 칸 하나다.
8. **띠 고르기(`choose_seam`)** — `Outer` 면 `sides` 중 유효한 첫 변 · 품은 변이 전부 무효일 때만 그 밑 칸 띠로 넘어간다(§11-1 #5). 칸에서는 점이 띠 사각형(그린 그대로 · 칸 틀 밖으로 삐진 띠는 보지 않음) 안이면 후보 · 없으면 `None`(가운데 — R6) · 둘 이상(모서리)이면 칸 변까지 거리 ÷ 띠 두께가 가장 작은 띠(R6a ②) · 동률 `SIDE_ORDER`.

**경계 포함 규칙(R6a ③)** — 칸 · 탭바 · 바깥 띠 · 칸 띠 전부 그린 사각형 그대로의 반열림(왼쪽 · 위 포함) · 탭바 자리는 가운데 x 와 같으면 오른쪽 · 동률은 늘 `Left → Right → Top → Bottom` · 부동소수 비교만(반올림 없음).

### 2-3. 바깥 띠 두께 = 32 CSS px

Eclipse 4 의 35px(조사 §7)와 같은 크기대 · 탭바 높이(28px — `TabBar.tsx:65`)보다 조금 두꺼워 노리기 쉽다 · CSS px 라 배율과 무관하게 같은 눈 크기 · 값은 `hit.rs` 상수 하나(R5 「값은 TRD」).

### 2-4. 영역 만들기 · 거울 · 골든

- **창별 영역(`regions_for`)** — 숨긴 출처는 만들지 않는다. 보이는 탭 = 모델 활성(출처 창이고 활성이 출처면 `display_active` — `session.rs` 의 `visible_tab` 규칙을 옮겨 온다). `blocked` 면 칸 · 바깥 · 탭바를 비운다. 칸 · 바깥은 크롬과 캔버스가 둘 다 있어야, 탭바는 크롬이 있어야 싣는다.
  - `outer` = `valid_bands(보이는 탭 배치, Outer, expect_source, canvas)` 의 변만 `outer_strips` 의 띠(그림 · 판정)와 결과 사각형(칠할 것)으로.
  - `slots` = 칸마다(`geometry::frame_px`) `valid_bands(배치, Slot(칸), expect_source, canvas)` — 면적 0 띠는 빠진다(R8 · AC15). 비용 = 칸 수 × 4 + 4 번의 `docked_tree` — 끄는 동안 계기마다 · 트리가 작아 ms 이하(§11-2 #9).
  - `tab_bar` = 크롬 탭바 + 출처를 id 로 뺀 머리 + 자리마다 `tab_insert_x`. 출처 창의 크롬에 접히기 전 출처 머리가 아직 남아 있으면(떼어진 표시 뒤 첫 보고 전) 기하가 옛것이라 끼울 자리 막대가 머리 하나 폭만큼 어긋나 보일 수 있다 — 그래도 그림과 판정이 같은 영역(같은 `rev`)이라 보인 그대로 들어가고, 접은 뒤의 보고가 오면 `rev` 가 올라 맞춰진다(§11-2 #10). 4판처럼 그 틈에 탭바를 비우면 출처 탭바 위 짧은 떼기가 「가이드 없음 = 새 창」이 되므로 비우지 않는다.
  - 기대값 = `Seam{보이는 탭 배치, 그 창 모델 활성}`(활성 = `WindowSnap.active`) · `Tabs(그 창 탭 목록)` — 영역이 같아도 기대값이 바뀌면 장부가 `rev` 를 올린다 · 탭바 영역 · `Tabs` 가 바뀔 때만 `bar_rev` 도(§1-3 「영역」).
- **TS 거울(`dockPick.ts` `pickTarget`)** — 영역 위에서 같은 순서로: 탭바(반열림 · `index` = 가운데 x ≤ 점인 머리 수 · `over`) → 캔버스 밖 · 보이는 탭 없음 = 없음 → 점을 품은 유효 바깥 띠를 변까지 거리 · `SIDE_ORDER` 순으로 첫 것 → 아니면 밑 칸의 띠를 정규화 거리로(`choose_band`) → 없음. 유효한 바깥 띠만 오므로 「품은 변이 전부 무효면 칸 띠로」가 따로 손대지 않아도 같다.
- **골든 `src-tauri/src/dock/pick_golden.json`** — 사례 = (영역 · 점 · 기대 `Picked`). Rust 시험(`pick`)과 vitest(`pickTarget`)가 같은 파일을 먹는다 — 규칙을 고치면 골든 · TS 판을 같은 변경에서 고친다(갈라지면 한쪽이 빨개진다 — 대기 입력 골든 `queued_input_golden.json` 과 같은 꼴). 사례는 §9 판정 행의 경계를 다 덮는다. 갈라지면 놓기가 「보인 것과 다름 = 무변경」으로 떨어진다 — 잘못 붙지도, 본 가이드 대신 새 창이 뜨지도 않는다(5.1 · §1-3 「놓기 판정」).

## 3. LLM 표면 — 이번에 없다 (의식적 예외)

- **사용자 결정(PRD Q10 · 2026-10-16)** — 이 기능은 LLM 명령(`tab.dock` 류 · `layout.get`)도 프론트 레지스트리 명령도 만들지 않는다. CLAUDE.md 「LLM-우선 제어」(「새 UI 기능엔 LLM 호출 경로를 함께」)의 **의식적 예외**이고, 도킹 바로 다음 작업이 닫는다(메모 = `.claude/handoff/attachments/llm-layout-surface-notes-2026-10-16.md` — 첫 단계 = LLM 도구 설계 관례 조사 · 흔한 요청을 1–2 호출로).
- **남기는 것** — `apply::dock_tab`(마우스 놓기가 쓰는 단일 적용 경로) · 타입드 `DockError` · `FocusPolicy::Keep` · 표식의 `Dragging` 판정. 다음 작업의 명령은 그 위에 껍데기만 얹는다 — 두 번째 변경 경로를 짓지 않는다(ADR-0081 결정 3).
- **끌기 자체는 명령이 아니다** — `dock_*` invoke 는 사람 손의 입력과 그 측정이라 레지스트리 · 버스에 싣지 않는다.
- 위험 · 후속 = §11-2 #15 · 8 단계에서 CLAUDE.md 「LLM-우선 제어」 「남은 갭」에 한 줄(`/review doc`).

## 4. 떼어진 표시 · 끌기 그림 · 가이드 (R2 · W7 · W8)

### 4-1. 떼어진 표시 — 그리기만

- **탭이 여럿인 창 · 메인** — 창은 제자리다. ★스토어의 창 탭 목록(`win.tabs`)을 거르지 않는다★ — `WindowLayout.tsx:152-155` 는 그 목록이 0 개가 되면 자기 창을 닫는다(0탭 자가닫힘). 떼어진 표시는 렌더 때만 쓰는 별도 값(`detached: { view, displayActive } | null`)이다: `TabBar` 는 그 머리를 CSS 로 접고(DOM 에는 남긴다 — §1-4), 캔버스는 모델 활성이 그 탭인 동안 `displayActive` 탭을 보인다(keep-alive 표시 토글 — `WindowLayout.tsx:196-200` · ADR-0056). 승계는 셸이 계산해 보낸다(`succession` — 모델 · 판정과 같은 규칙).
- **메인의 마지막 탭** — `displayActive = null` 이라 메인은 빈 탭바 · 빈 캔버스(테마 배경)다. 빈 탭은 놓을 때 모델이 만든다(§1-2 — 끄는 동안 모델은 그대로).
- **탭이 하나뿐인 팝아웃** — 떼어진 표시 대신 셸이 창을 숨긴다(Tauri `hide()` · §1-3 「시작」). 끝에서 무변경이면 되보이고, 재사용(R22)이면 옮긴 자리에서 보이며, 붙으면 빈 창이라 적용 서비스가 닫는다. 숨김은 작업 표시줄에서도 빠지므로 끄는 중 사용자가 그 창을 닫을 길은 없다(LLM · 다른 경로로 닫히면 `Destroyed` → `SourceDestroyed`).
- `(drag_id, seq)` 가 오래된 것은 버린다. 모델에는 아무것도 보내지 않는다.

### 4-2. 끌기 그림 (PRD R2 · Q5)

- **모양 = Windows 기본** — `setDragImage(요소, 커서 자리)` 로 넘긴 요소를 엔진이 그림으로 떠 OS 가 그린다: 반투명 · 오른쪽 · 아래로 흐려짐 · 끌어 놓기를 받지 않는 다른 앱 창 위에선 사라짐(실측 · 받아들임). 따로 된 창 · 페이지 · 권한이 없다.
- **요소 = 화면 밖에 늘 붙어 있는 그림 상자 하나**(`dragImage.ts` — `position: fixed` · 화면 밖 좌표). `setDragImage` 는 `dragstart` 안에서만 부를 수 있고 그 순간 그려진 요소를 쓰므로, 상자를 `dragstart` 안에서 동기로 채운다: 위 = 머리 띠(탭 제목 · 높이 28 논리 px) · 아래 = `<canvas>` 축소 그림.
- **축소 그림 — 되는 만큼**(결정 2 · S2) — 끄는 탭이 그 창에 보이는 탭이면 칸마다: 터미널 칸은 그 xterm WebGL 캔버스를 `drawImage` 로 줄여 옮긴다 · 그 밖(챗 · 목록 · 사용량 · 빈 칸 — DOM 이라 동기로 그림에 못 옮긴다)은 테마 패널 색 + 칸 이름 자리채움. 칸 사각형은 그 칸 DOM 의 `getBoundingClientRect` 를 캔버스 원점 기준으로 줄인다. ★WebGL 캔버스는 `preserveDrawingBuffer` 없이는 그린 프레임 밖에서 비어 읽힐 수 있다(가능성 높음)★ — S2 가 「켜기(`WebglAddon` 생성자 인자) · 끄기」로 재고, 비면 그 칸도 자리채움이다. 보이지 않던 탭이면(그 칸들은 `display:none` 이라 그릴 것이 없다) 제목만(§11-1 #7).
- **크기(논리 px · 웹뷰가 계산 — 놓기 판정과 무관한 모양 값)** — 그림 있음 = R22 새 창 크기(팝아웃 = 그 창 `innerWidth × innerHeight` · 메인 = 팝아웃 기본 720×500) × `DRAG_IMAGE_SCALE`(0.4)를 비율 그대로 `DRAG_IMAGE_MAX`(320×240) 안에 들게 줄이고, 너비가 `DRAG_IMAGE_MIN_W`(180)보다 작으면 상한 안에서 키운다 · 그림 없음 = 180×28(제목만). 예: 메인에서 떼기 → 288×200 · 1600×1000 팝아웃 → 320×200 · 400×300 팝아웃 → 180×135. 엔진의 요소 그림 상한(변 8192px)보다 한참 작다(조사 §3).
- **커서 자리** — `DRAG_IMAGE_HOTSPOT` (16, 14) CSS px — 커서가 머리 띠 왼쪽 안에 앉아 제목이 커서 옆에 보인다. 그림이 반투명이라 그 밑 가이드가 비친다(4판의 「불투명 딱지가 가이드를 가림」이 사라졌다). 이 값을 `dock_drag_start` 의 `hotspot` 으로 셸에 알리고, R22 새 창은 「커서 − hotspot × 출처 배율」에 뜬다 — 그림이 있던 자리(§1-3). 엔진 기본 그림으로 물러났을 때는 누른 점의 머리 안 자리를 싣는다.
- **실패** — 그림 만들기에서 무엇이 던지면 잡아 제목만 상자로 `setDragImage` · 그것도 실패하면 엔진 기본(끌린 머리 요소의 그림). 끌기는 늘 이어진다(PRD R2).

### 4-3. 가이드 그리기 (`DockOverlay.tsx`)

- 캔버스 div(`position: relative`) 안에 `position: absolute; pointer-events: none` 로: 커서 밑 칸의 띠(빗금 — R3) · 바깥 띠(빗금) · 고른 띠의 결과 사각형(반투명 강조 — R4 · R5 · 바깥이면 `landing`). 탭바 위에는 끼울 자리 세로 막대(R21 · `insert_xs[index]`) · 전환 대기 진행 표시(R6b). 색은 테마 CSS 변수(`--accent` 계열) · 시각 세부는 시안(아티팩트 「슬롯 도킹 시안」 v2).
- 지우는 때 = 떠남 · 놓기 · `Ended` · 영역의 `drag_id` 가 지금 끌기가 아님 · 그 창이 보이는 탭이 영역의 `view_id` 와 다름(5.1 · §1-4). `Ended` 는 무조건 지운다 — 어떤 창이 떠남을 놓쳐도 끝 방송 한 장이 지운다(R13b · AC16).

## 5. 저장 · 원자성 · 출력 연속 (R11 · R11a)

기존 장치가 그대로 덮는다 — 새로 짓는 저장 · 라우팅 장치는 없다.

- **끄는 동안 저장되는 것 = R6b 탭 전환 하나** — 표식 · 묘비 · 장부는 메모리, 떼어진 표시는 웹뷰에만 있고, 출처 팝아웃 숨기기는 자리를 바꾸지 않는다. 그래서 끄는 동안 앱이 꺼지면 다음 부팅은 끌기 전 배치에 그 사이 R6b 로 바뀐 활성 탭만 반영된 판이다(숨겼던 팝아웃도 저장 자리로 다시 뜬다).
- **원자성(R11a 앞 절반)** — 출처 탭 빼기 · 대상에 넣기 · 빈 팝아웃 항목 지우기 · ★메인이 비면 빈 탭 채우기★ · 곁표 옮기기가 `ViewManager::dock` 한 번(= 락 한 번 · `version` 한 번)이다. 새 창은 OS 창을 먼저 열고(phase B) 모델 변경은 phase C 한 번이다. 그 락을 거치지 않고 모델을 보는 관찰자는 없다(`apply.rs` 머리 「락 규율」).
- **저장(R11a 뒤 절반)** — 기록기는 변경 번호가 바뀌면 스냅숏을 같은 락 안에서 통째로 복사해 쓴다(`state/convert.rs` `StateRevision` · `state/saver.rs`). 그래서 `state.json` 은 도킹 전이거나 뒤이고, 탭이 두 곳이거나 어디에도 없는 판은 없다. 원자 쓰기(base `file`)라 쓰는 도중 꺼져도 이전 판이 남는다(AC11). 새 창 phase B 와 C 사이에 꺼지면 OS 창만 있고 모델은 도킹 전이라 다음 부팅에 그 창은 없다.
- **OS 창 닫기는 모델 뒤 — 모든 경로가** — 빈 팝아웃은 모델에서 먼저 지우고(락 안) 락 밖에서 destroy 한다(`close_tab` 과 같다). 사용자 X 도 닫기 요청에서 모델을 먼저 지운다(M2).
- **출력 연속(R11 · AC7)** — 같은 락 안의 `subs.resync(&mgr)`(`output_router.rs` 「같은 critical section」)가 라우팅 표를 바꾼다. 에이전트가 출처 창에서 대상 창으로 옮겨도 「어느 창에든 보이는가」는 그대로라 데몬 구독은 끊기지 않고 경로만 바뀐다. 대상 웹뷰에 새로 마운트된 칸은 기존 구독 효과(`[viewId, agentId]` · replay 요청)로 지난 출력을 받는다. 출처 창이 닫히면 그 창 Channel 이 해제된다. ★닫히는 출처 웹뷰의 늦은 replay 요청을 3 단계 가드가 거르는 것은 그 에이전트를 보이는 창이 하나도 없을 때뿐이다(5.1 · N6)★ — 도킹에서는 대상 창이 그 에이전트를 보이므로 그 요청은 지나가 Subscribe 가 나가고, 그 replay 는 라우팅 표의 지금 대상(대상 창)으로 배달된다. 대상 뷰는 seq 연속 규칙(CLAUDE.md 「replay→live」 — 이미 받은 seq 이하는 버린다)으로 겹친 것을 버리므로 무해하다(가능성 높음 — replay 는 에이전트당 단일 비행이라 대상 뷰 자신의 요청과 병합될 수 있다 · AC7 화면 확인). 재사용(R22 · 탭 하나 팝아웃)은 모델이 안 바뀌어 라우팅도 안 바뀐다.
- **웹뷰 안 화면 상태(D10 비목표)** — 입력 초안 · 스크롤 · 렌더 모드 오버라이드는 출처 웹뷰에만 있어 따라가지 않는다.

## 6. 사용자 결정

### 6-1. 확정 (PRD 4.3 「(결정)」 · §5-1)

- **제목줄 = 창 이동만, 탭 = 도킹**(R1 · R2) — 제목줄은 OS 기본 이동이고 이 기능은 끼어들지 않는다.
- **탭을 끌면 곧바로 떼어지고 끌기 그림이 따라간다 · 창은 따라오지 않는다 · 「원래대로」는 없다**(R2 · Q1) — §4-1.
- **끌기 그림 = Windows 기본 모양 · 내용 = 되면 탭 축소 그림, 아니면 제목 · 딱지 창 없음**(R2 · Q5 · 2026-10-16) — §4-2.
- **끌린 탭의 배치는 놓을 때만 바뀐다**(R2 · R11a) — §1-3 · §5.
- **가이드 = 슬롯 상하좌우 띠(1/3) + 정확한 결과 사각형 + 창 바깥 띠, 가운데는 없음**(R3–R6) — §2.
- **놓으면 셋 중 하나 — 이음새 · 탭바 · 그 밖은 새 창**(R7 · R21 · R22 · Q3 · Q4) — §1-2 · §1-3.
- **크기 = 새것 1/3 · 기존 2/3 하나로 · 막는 것은 면적 0 뿐**(R7 · R8) · **붙는 단위 = 탭 하나의 배치 통째**(R9).
- **Esc = 그 자리에 놓기**(R12 · Q6) · **비정상 끝 = 마지막 커서 자리에 놓기 · 예외 = 출처 탭이 사라짐**(R13b · Q8) — §1-3 「끝 신호」.
- **끌기 수단 = 브라우저 내장 HTML5 끌어 놓기 · 감싸는 라이브러리 없음**(R18 · Q7) — §1 · ADR-C.
- **메인 마지막 탭도 끌기 · 떠나면 메인에 빈 탭**(R14 · Q9) — §1-2 4 단계 고침.
- **LLM 명령은 다음 작업**(R16 · R17 · Q10) — §3 · ADR-D.
- **직전 도킹 되돌리기 = 비목표**(Q2) · **슬롯 하나 직접 끌기 · 슬롯 가운데 칸 · Ctrl 끌기는 범위 밖**(PRD §2).

### 6-2. PRD 기본값으로 따른 것 (사용자가 바꿀 수 있다)

- **D4** 같은 창 다른 탭 합치기 = 머물면 탭 전환(R6b · R15) — 대기 500ms.
- **D6** 크기 기억 · 마지막 도킹 자리 = 나중 — 설계에 자리를 두지 않았다.
- **D7** 띠 겹침 = 바깥 띠 우선 · 정규화 거리(R6a) — §2-2 규칙 6 · 8.
- **D8** 마우스로 놓은 뒤 대상(새) 창 활성화 + 첫 슬롯 포커스(R11b) — `FocusPolicy::Reveal` + `set_focus`.
- **D9** 끄는 중 같은 탭을 옮기는 명령은 실패 — `DockError::Dragging`(표식 · 같은 락 판정 — 다음 작업의 명령이 쓴다).
- **D10** 웹뷰 안 화면 상태 이전 = 비목표 — §5 끝 줄.
- **D12** 새 창 크기 = 출처 창 크기 · 메인에서 떼었으면 팝아웃 기본 크기 — `plan_new_window`.
- **D13** 탭 하나뿐인 팝아웃은 끄는 동안 숨김 — Tauri `hide()`(§4-1 · S1 ⑤).
- **D14** 메인 마지막 탭을 끄는 동안 메인 = 빈 화면 · 빈 탭은 놓을 때(§4-1).
- **R5 바깥 띠 두께** — 32 CSS px(§2-3) · **R13c** 출처 바뀜 = 받지 않음(「안 됨」) · 대상 바뀜 · 보인 것과 다름 = 무변경(§1-3).

## 7. 내가 정한 내부 선택 (보고)

**구조**
1. **끌기 장부를 `layout` 밖 `dock` 모듈에 둔다** — 장부는 레이아웃 권위를 부르는 손님이다.
2. **장부 = 순수 `step(사건, 스냅숏) → 할 일` + 잎 락 하나 · 스레드 · 틱 없음** — 가짜 시각 · 커서 · 창 사각형 · 스냅숏으로 단독 시험이 선다(ADR-0012). 커서 · 사각형은 실행기가 읽어 입력에 싣는다.
3. **영역을 셸이 미리 계산해 창마다 보내고 웹뷰는 점-사각형만 본다 · 보인 것은 바뀔 때만 보고** — 움직일 때마다 셸에 묻는 길(invoke p99 ~16ms · 최대 ~50ms — 실측)은 60Hz 가이드를 눈에 띄게 늦춘다(ADR-C 거부한 대안 ⑥). 셸은 영역을 만들고 놓기를 다시 판정하므로 권위는 그대로다.
4. **판정 규칙은 Rust 하나(`hit`) · `pick` 은 그 위의 어댑터 · 웹뷰 판은 골든으로 묶은 거울** — 둘이 갈라져도 결과는 「무변경」이지 엉뚱한 도킹이 아니다.
5. **놓기 대조 = 보인 것과 지금 판정이 같을 때만 적용(영역 `rev` 최신 · 보이던 탭 같음 포함) · 둘 다 「가이드 없음」일 때만 새 창 · 그 밖은 무변경** — 4판의 「마지막으로 보낸 오버레이」를 「마지막으로 보낸 영역 + 웹뷰가 그린 것」으로 옮겼다. M1(못 본 가이드에 놓이지 않음)은 그대로이고, 「지금 판정이 가이드 없음이면 보고가 무엇이든 새 창」은 5.1 에서 버렸다(본 가이드가 사라진 자리의 새 창은 못 본 결과다 · Q18).
6. **영역 다시 계산 계기 = 쓰기 하나의 알림 묶음 끝(`LayoutEvents::settled`) 한 자리 + 크롬 · 캔버스 보고 · 스냅숏 번호로 순서 보장** — 레이아웃을 바꾸는 쓰기는 다 그 어댑터로 알리므로 한 자리에 건다. 알림 하나마다가 아니라 묶음 끝인 것은 5.1 — 영역 · 끝이 그 쓰기의 나머지 알림을 앞지르지 않게.

**끝 신호**
7. **Esc · 창 밖 떼기를 키 상태로 가르지 않는다** — `none` 끝이 우리 창 위를 가리키면 그것은 Esc 이고, Esc · 떼기는 같은 「그 자리에 놓기」다. 판정 = 마지막 Over 창(창별로 마지막 Over 와 떠남을 따로 적는다 — 5.1) + 떠남 유예 100ms + 셸 커서가 그 창 사각형 안. OS 함수 0 · 같은 규칙이 시스템 취소에도 선다.
8. **`dragend`(move) 뒤 `drop` 을 500ms 기다린다** — 두 웹뷰 invoke 의 도착 순서는 보장되지 않는다. 시한이 지나면 「마지막 점에 놓기」(그 끝 신호 때의 커서).
9. **끝 신호 유실 = 세션 중 본 버튼 0 의 마우스 `pointermove` · `pointerdown`** — 끄는 동안 포인터 사건이 안 오는 엔진 규칙을 끝의 증거로 쓴다(웹 표준 · OS 0). 300ms 기다린 뒤 W 가 아직 위면 그 마지막 Over 보고로 놓고, 아니면 지금 커서에 새 창.
10. **세션 id = 출처 웹뷰가 `dragstart` 에서 만든 UUID** — `setData` 를 셸 왕복 전에 끝내야 하므로. 셸은 그 id 를 세션 키로 쓰고, 최근 끝난 id 의 신호는 버리며 처음 보는 id 의 끝 신호는 잠깐 붙든다(5.1).
11. **끌기 데이터에 우리 MIME 하나만** — 다른 앱(탐색기 · 편집기 · 브라우저)이 받지 않아 `none` 으로 끝나고 그 앱에 아무것도 안 들어간다(AC8).
12. **새 시작이 살아 있는 옛 세션을 만나면 새 끌기를 거절하고 옛 세션을 「마지막 점에 놓기」로 끝낸다 · 단 옛 세션이 적용 중이면 끝내지 않고 새 시작만 거절한다(5.1)** — 표식은 하나뿐이고 옛 세션은 신호를 잃은 것이다(OS 끌기는 한 번에 하나). 적용 중인 세션은 신호를 잃은 것이 아니라 적용 서비스를 기다리는 중이라 `Applied` 가 끝낸다. 사용자는 다시 끈다(§11-2 #3).
13. **시작 · 보고는 동기 명령(메인 스레드 · 도착 순서), 창을 만들 수 있는 놓기 · 끝은 async** — 같은 웹뷰의 시작 → 끝 순서가 서고, 창 빌드 교착을 피한다.
14. **시작 되감기 가드를 두지 않는다** — OS 끌기는 이미 돌고 있어 되감을 것이 표식 · 숨김뿐이고, 그 둘은 모든 끝 경로가 거둔다. 숨기기 실패는 보인 채 이어 간다.

**웹뷰**
15. **떼어진 표시는 `Started` 뒤 · `dragstart` 안에서는 DOM 을 안 바꾼다 · 끌린 머리는 DOM 에 남긴다(CSS 접기)** — 끌기가 곧바로 끝나거나 `dragend` 가 올라오지 않는 엔진 동작을 피한다.
16. **R6b 머묾은 대상 웹뷰가 재고 기존 탭 전환을 부른다** — 장부에 UI 시간을 두지 않고, 쓰기는 사람 클릭과 같은 표면이다.
17. **Esc 삼키기(세션 + 200ms · 그 창이 끝 증거를 보면 200ms 뒤 그침)와 파일 드롭 막기(늘)를 웹뷰에 둔다** — 둘 다 보험 · 회귀 막이이고 셸이 할 일이 아니다.
18. **끌기 그림 크기 · 내용은 웹뷰가 정한다(4.2 딱지의 크기 규칙 그대로)** — `setDragImage` 가 `dragstart` 안 동기만 받아 셸에 물을 틈이 없고, 놓기 판정과 무관한 모양 값이다. 픽셀은 터미널 WebGL 캔버스만 옮기고 DOM 칸은 자리채움이다.
19. **모든 런타임 창의 드롭 처리기 끄기 = `WebviewEnv::finish` 한 곳** — 창 만드는 코드가 다 지나는 곳(ADR-0283)이라 앞으로의 창도 빠지지 않는다.

**레이아웃 연산 (2 · 4 단계 — 4판에서 이어짐)**
20. **`close_tab` 의 빼기 · 승계 · 빈 창 판정을 도우미로 나눠 쓰고, 승계는 순수 함수로도 내보낸다 · 4 단계에서 메인 빈 탭 채우기(`refill_main`)도 같은 도우미로** — 규칙이 모델 · 떼어진 표시 · 판정에서 한 벌이다.
21. **끼우는 서브트리의 슬롯 id · 분할 id 를 그대로 · 탭바 · 새 창은 View 째(탭 id 그대로)**.
22. **결과 트리 함수 하나(`docked_tree`)를 영역 · 판정 · 적용이 같이 쓴다 · 새 면적 0 = 원래 면적이 있던 칸이 0** — 「가이드 = 결과」(R4)가 식 하나로 선다.
23. **사전조건 = 호출자 중립 값(`DockGuard`) · 포커스 정책(`FocusPolicy`) · 출처 대조 = 배치 값 비교** — 모델이 「마우스」를 모르고, 변경 번호를 올리는 자리를 빠뜨려도 틀리지 않는다.
24. **R22 에서 출처가 탭 하나뿐인 팝아웃이면 새 창 대신 그 창을 옮긴다** — 결과가 같고 웹뷰를 새로 띄우지 않으며 출력 경로가 안 바뀐다. 옮긴 뒤 같은 락 안에서 다시 확정하고 어긋나면 되돌린다(5.2 · #39). 그 창의 보통 자리가 모델에 없으면 되돌릴 자리가 없으므로 새 창으로 간다(5.4).
25. **탭바 `index` 넘침 = 맨 뒤 · 같은 창 같은 자리 = 무변경 · 단 `Reveal` 이고 안 보이던 탭이면 보인다** — 사용자 결정(§11-1 #6 · 2026-10-16).
26. **창 포트의 새 메서드는 기본 본문 없이 · 마우스 자리도 `placement::land` 로 검증**.
27. **(M2) 팝아웃 닫기 요청에서 모델을 먼저 지우고 묘비** — §1-2 · ADR-E.

**판정 (5 단계 — 그대로)**
28. **칠하지 않은 바깥 띠가 전부면 밑 칸 띠로 넘어간다** — §2-2 규칙 8 · §11-1 #5.
29. **그린 사각형 = 판정 사각형(네 변 반열림 · 정수 바깥 띠)** · **크롬 측정 타입은 `layout::chrome`**(창 항목이 들고 있어 `layout` → `dock` 의존을 만들지 않는다).

**5.1판 (리뷰 1라운드)**
30. **`TargetExpect::Seam` 에 대상 창 모델 활성을 더하고 웹뷰는 보이던 탭(`displayed`)을 싣는다 · 영역 `view_id` 와 다르면 그리지 않는다** — 활성 전환의 「모델 먼저 · 알림 나중」 틈에 안 보이는 탭으로 붙지 않게(§1-3 「보이는 탭 경쟁」). 새 오류 종류는 두지 않는다 — 사용자에겐 「대상이 바뀜」과 같은 무변경이다.
31. **우리 MIME 은 세션을 몰라도 받는다 · 영역 · 세션은 `drag_id` 별 최대 `rev` · `seq` · 옛 `rev` 보고에는 영역을 다시 보낸다 · 시작보다 먼저 온 끝 신호는 500ms 붙든다** — emit · invoke 가 여러 스레드에서 나가 순서가 보장되지 않는다. 붙드는 것은 끝 신호(`drop` · 출처 끝)뿐이고 한 번에 한 id 다 — 시험이 시한 하나 · 칸 하나로 선다.
32. **적용 중에는 스냅숏 · 소멸을 적기만 하고 끝은 `Applied` 가 정한다 · 스냅숏 계기는 알림 묶음 끝 한 번(`LayoutEvents::settled` — 기본 본문 없음)** — 성공한 도킹이 자기 알림 · 자기 창 닫기로 끝나지 않게.
33. **react-arborist 의 window 끌기 리스너는 우리 MIME 사건을 `document` · 탭 머리에서 끊어 피한다 · 트리 뿌리 좁히기(`dndRootElement`)는 택하지 않는다 · 셸 세션은 취소되지 않은 `dragstart` 뒤에만** — 뿌리 좁히기는 react-dnd 의 창당 관리자 하나 때문에 트리 둘에서 깨지고, 고치려면 `dnd-core` 직접 의존이 든다(§1-4).
34. **기하가 같아도 기대값이 바뀌면 `rev` 를 올린다** — 내용만 바뀐 대상 칸에 본 가이드대로 붙게.
35. **창 게터는 레이아웃 · 장부 락 밖에서 · `WindowSnap.hidden` 은 장부 값** — 메인 스레드로 넘어가 기다리는 게터와 메인 스레드 처리기의 레이아웃 락이 교착하지 않게.
36. **세션 깃발은 시작의 레이아웃 락 안에서 세우고, 세션 전에 온 스냅숏은 들고 있다가 `Start` 뒤에 먹는다** — 시작과 장부 사이에 끼어든 쓰기를 놓치지 않게.
37. **끝 증거는 버튼 0 의 마우스 이동 · 누름 · Esc 삼키기는 그 창의 끝 증거 뒤 200ms 에서 그친다 · 커서를 못 읽으면 W 의 마지막 점** — 합성 이동을 끝으로 오인하지 않고, 남은 세션이 모든 창의 Esc 를 먹지 않고, 「마지막 점」 원칙을 커서 없는 끝에도 지킨다.

**5.2판 (리뷰 2라운드)**
38. **탭바 놓기는 이음새와 따로 대조한다 — 탭바 몫 `bar_rev` · 탭 순서만 · 보이던 탭은 이음새만** — 활성 전환은 탭바를 바꾸지 않으므로 전환 알림 틈에도 탭바 놓기가 선다(§1-3 「놓기 판정」 4b). 영역을 탭바 · 칸 둘로 쪼개 보내는 안은 택하지 않았다 — 메시지 · 최대 `rev` 규칙이 둘이 된다. 번호 하나를 더 싣는 것으로 같은 효과다.
39. **R22 재사용 = 옮기고 → 같은 락 안에서 `plan_new_window` 를 다시 불러 확정 → 어긋나면 옮기기 전 자리로 되돌린다** — 락을 쥔 채 창을 옮기면 메인 스레드 처리기와 교착하고(§1-3 「창 게터」), 확인 뒤 옮기면 그 사이가 빈다. Create 의 「열고 → 판정 → 어긋나면 닫기」와 같은 꼴이고, 끄는 동안 그 창이 숨어 있어 되돌리기가 보통 화면에 안 보인다(최대화였던 출처는 예외 — 최대화 풀기가 창을 보인다 · 5.3). 되돌릴 자리는 A 에서 락 안에 복사한 모델 자리(보통 자리 · 최대화)이고 게터가 아니다 — 최대화된 창의 게터는 최대화 사각형을 준다(5.3). 창 포트의 세터는 결과를 돌려준다 — 옮기기는 첫 실패에서 멈추고 되돌리기를 부르며, 되돌리기는 실패해도 나머지 걸음을 이어 간다(보상할 다음이 없다). 재사용을 걷고 늘 새 창을 여는 안은 택하지 않았다 — 탭 하나 팝아웃의 R22 가 웹뷰를 새로 띄우고 터미널을 다시 붙인다(§7 #24 · §11-2 #13). 모델에 보통 자리가 없는 창은 재사용하지 않는다 — 되돌릴 자리가 없기 때문이다(5.4 · 리뷰 4라운드 · 사용자 쪽에서는 그 경우만 새 창이 뜬다).
40. **`Apply` 는 늘 async 런타임에서 · 동기 명령 · 메인 스레드에서는 창을 만들지 않는다** — 붙든 이른 놓기를 시작(동기)에서 이어 먹이는 길이 생겼기 때문이다(§1-3 「명령의 실행 스레드」).
41. **붙들다 버린 id 는 최근 끝난 id 로 적어 늦은 시작을 `LateStart` 로 거절 · 세션 깃발은 세대 비교-교체로 내림** — 끝 신호 없는 세션과 다음 세션의 계기 끊김을 막는다.

## 8. 실측

### 8-1. 끝난 실측 (2026-10-16 · 상세 = 조사 §4 · 스파이크 기록)

환경 = tauri 2.11.3 · wry 0.55.1 · WebView2 Edg/155 · Windows 11 · ★RDP 세션 · DPR 1.25 · 22 끌기★ · config 창(`dragDropEnabled: false`)과 런타임 창(`disable_drag_drop_handler()`) · 같은 데이터 폴더.

- **셸 메인 스레드는 막히지 않는다** — OS 끌기 루프는 WebView2 브라우저 프로세스 UI 스레드에서 돈다. 끄는 중 `run_on_main_thread` 지연 최대 4.1ms(평소 1.4) · emit → JS p99 11 · 최대 33ms · invoke 왕복 p50 2.2 · p99 ~16 · 최대 ~50ms.
- 대상 창 `dragover` 60/s(움직일 때) · ~15/s(멈춤) · `clientX/Y` 정확 · 커스텀 MIME 12/12 · `dragend` 는 모든 경우(놓음 move · 창 밖 none · Esc none) · 원본 창을 끄는 중 숨겨도 끌기 · `drop` · `dragend` 가 이어짐(2회).
- 끌기 그림 = Windows 셸 모양(반투명 · 흐려짐) · 다른 앱 위에선 사라짐. 어느 웹뷰도 아닌 자리(틈 · 바탕화면)엔 JS 사건 없음.
- 4판의 스파이크 S1–S6(포인터 직접 읽기 · 맨 위 창 · 딱지 창 · 화면 잡기)은 수단이 바뀌어 뜻을 잃었다 — §11-3.
- ★이 스파이크는 앱 의존이 없는 맨 페이지(따로 지은 Tauri 앱의 `a.html` · `b.html` + 공용 스크립트 하나)에서 돌았다(5.1)★ — React · react-arborist 처럼 window 에 끌기 리스너를 다는 앱 의존이 끌기를 막는지는 재지 않았고, 실제로 막는다(소스 — §1-4 「다른 끌기 리스너」 · S1 ⑦). 표본도 작다(숨김 2 · 창 밖 Esc 3).

### 8-2. 남은 실측

- **어디서** — 버리는 스파이크 브랜치(예: `v0.4.1/spike/dock-*`)나 7 단계 작업 트리에서 런처로 띄워(CLAUDE.md 「GUI 실측」) 잰다. 결과는 이 문서 개정으로 남긴다. S5 를 뺀 다섯은 7 단계 앞(S2 는 7 단계의 그림 만들기와 함께 해도 된다). 단 S1 ⑦ · ⑧ 은 앱 코드(끊기 · 붙들기)가 있어야 해서 7 단계 작업 트리에서 그 단계 커밋 전에 잰다.
- **★불합격 = 멈춤★** — 「불합격이면」에는 요구를 바꾸지 않는 조정만 적었다. 그것으로도 못 넘으면 구현을 멈추고 PRD 개정(사용자 결정)으로 올린다. 수치는 첫 값이다 — 비현실적이면 개정하고 보고한다.

| # | 위험 | 무엇을 어떻게 | 합격 기준 | 불합격이면 |
|---|---|---|---|---|
| **S1** | W10 · W1 — 끝 신호 | ① Esc 를 우리 창 위(띠 · 칸 가운데 · 탭바) · 틈 · 바탕화면 · 우리 창을 덮은 다른 앱 창 위에서 각 20회 — 대상 `dragleave` 와 출처 `dragend` 가 셸에 닿는 순서 · 간격 ② 창 밖 떼기 20회 — 마지막 `dragleave` → `dragend` 간격 ③ 끝 뒤 첫 `pointermove` 가 오나(그 `pointerType` · `buttons`) · 끄는 동안 0 인가 · 떼어진 표시로 머리가 접히는 재배치 뒤 커서를 멈춘 채 합성 `pointermove` 가 오나 · 그 `buttons`(5.1) ④ 끌린 머리를 CSS 로 접어도 `dragend` 가 오나 · `dragstart` 안 DOM 변경이 끌기를 끝내나(정보) ⑤ 탭 하나 팝아웃 `hide()` 20회 — 끌기 계속 · 되보일 때 자리 · 크기 · 최대화 · 그 뒤 탭 클릭 정상 · 최대화였던 탭 하나 팝아웃을 R22 재사용으로 옮기기 · 되돌리기(5.2 · 5.3) 각 10회 — B′ 에서 창이 보이고 포커스를 가져가나(소스로는 보인다) · 되돌린 뒤 모델 보통 자리 = 옮기기 전 · 보일 때 최대화가 다시 입혀지나 · 최대화 아니던 팝아웃은 숨은 채인가 ⑥ Esc keydown 이 웹뷰에 오나 · claude 터미널에 ESC 가 들어가나 ⑦ **트리 슬롯을 마운트한 채(5.1)** — 에이전트 목록 슬롯을 출처 · 대상 창에 보이는 탭 · 안 보이는 탭으로 두고 탭 끌기 시작 · 가이드 · 놓기 각 20회 · 트리 행 재부모화 끌기 10회 · 그 창에 OS 파일 놓기 10회 · 콘솔의 react-dnd 오류 ⑧ **사건 순서(5.1)** — 같은 창 · 이웃 창에서 문턱 직후 곧바로 떼기 20회 — 셸 도착 순서(시작 · 놓기 · 출처 끝) · 붙든 신호 수 · 영역 `rev` 역전 수 · 다시 보낸 영역 수 | ① 결과가 AC19 대로 20/20 · 창 안 Esc 의 「떠남 → 끝」 최대 < `LEAVE_GRACE_MS` · ② 간격이 시한 안인 비율 기록(덮은 창 오판의 크기) · ③ 20/20 · 끄는 중 0 · 합성 이동은 버튼 0 이 아님(또는 없음) · ④ 20/20 · ⑤ 20/20 · 최대화 아니던 팝아웃의 비침 0/10 · 되돌린 뒤 보통 자리 ±2 논리 px · 최대화 복원 10/10 · 최대화였던 팝아웃의 비침은 기록(합격 조건 아님 — 받아들인 경우) · ⑥ 터미널 유입 0/20 · ⑦ 끌기 · 놓기 20/20 · 재부모화 10/10 · 파일 탐색 0/10 · 오류 0 · ⑧ 붙든 신호가 시작에 이어 처리 20/20 · 옛 `rev` 에 머문 놓기 0 | ① 간격이 넘으면 시한을 늘려 다시 잰다 · 그래도 안 되면 platform 목적 함수(「주 버튼이 지금 눌려 있나」)로 Esc · 떼기를 가르는 안(§1-6 · 사용자 보고) · ③ 실패 = 유실 대비를 「다음 끌기 시작」으로만 · 합성 이동이 버튼 0 이면 재배치 뒤 짧은 시간의 이동을 증거에서 뺀다 · ④ 실패 = 머리를 접지 않고 흐리게만 · ⑤ 실패 = 숨기지 않고 빈 창으로 둠(PRD R2 개정 — 사용자) · 최대화 아니던 팝아웃이 비치면 → 원인 조사 · 되돌린 자리 · 최대화가 틀리면 **멈춤**(되돌리기가 모델을 어긋나게 한다) · 최대화였던 팝아웃이 보이는 것은 받아들임 — 거슬리면 사용자 보고 · 숨긴 채 푸는 platform 함수(미결 · §11-2 #24) · ⑥ 유입 = 삼키는 꼬리를 늘려 다시 · ⑦ 실패 = 트리마다 관리자(`dndManager` · `dnd-core` 직접 의존 — 의존 변경 보고)로 뿌리를 좁히는 안 · 그래도 → **멈춤** · ⑧ 실패 = `EARLY_SIGNAL_MS` 를 늘려 다시 |
| **S2** | W8 — 끌기 그림 내용 | ① 보이는 터미널 칸(WebGL)을 `drawImage` 로 옮긴 그림이 비지 않나 — `preserveDrawingBuffer` 끔 · 켬 · 켰을 때 출력 많은 터미널의 프레임 비용 차 ② `dragstart` 안 합성 시간(칸 4 · 1920×1080 캔버스) ③ 화면 밖 상자의 `setDragImage` 가 그 그림을 쓰나 · 320×240 · 배율 125 / 150% | ① 비지 않음 20/20(어느 설정이든) · 켜야 한다면 프레임 비용 차 ≤ 10% · ② p95 ≤ 8ms · ③ 20/20 | ① 실패 = 터미널 칸도 자리채움(Q5 의 「안 되면」 범위) · ② 느리면 칸 수 · 상한을 낮추거나 제목만 · ③ 실패 = 제목만 상자 · 그것도 안 되면 엔진 기본 그림 |
| **S3** | W9 · AC8 — 바탕화면 · 다른 앱 · 파일 | ① 실제 탐색기 바탕화면 · 탐색기 창 · 메모장 · Edge 위 떼기 각 10회 ② OS 파일을 main · 팝아웃에 끌어 놓기 — 막기 없음 · 있음 각 10회 | ① `none` · 새 창 · 그 앱에 아무것도 안 들어감 40/40 · ② 막기 있으면 탐색 0/20 · 커서 「안 됨」 | ① 받는 앱이 있으면 `DROP_WAIT` 뒤 「마지막 점에 놓기」가 맡는다 — 그 앱에 무엇이 들어갔는지 보고 · ② 탐색이 남으면 **멈춤**(회귀) |
| **S4** | W4 · AC12 — 혼합 DPI · 다중 모니터 | 100% · 150% 모니터 둘 · 각 모니터 창 · 걸친 창에서 칸 경계 ±1 CSS px 를 짚어 그린 띠와 `pick` 대조 · 다른 모니터 빈 곳에 놓아 새 창 · 걸친 창 위 Esc(셸 커서 · 창 사각형 판정) | 판정 = 그린 띠 ±1 CSS px · 새 창이 놓은 모니터에 R22 크기(±2 논리 px) · 창 안 Esc 20/20 | 창 안 판정이 어긋나면 W 의 마지막 보고 점을 화면 좌표로 옮긴 값과 대조하는 안 · 그래도 → **멈춤**(AC12) |
| **S5** | W12 — 콘솔(비 RDP) 세션 · 다른 PC | 2026-10-16 실측의 핵심 수치(메인 스레드 지연 · `dragover` 주기 · `dragend` · 좌표 · MIME)를 로컬 콘솔 세션에서 다시 | 같은 결론 | **멈춤** — 수단 재검토(사용자) |
| **S6** | W5 — tauri #10767 | 팝아웃 제목줄을 main 위로 끌어 놓고(R1) 곧바로 탭 끌기 10회 | AC1 + 뒤이은 탭 끌기 10/10 | 걸리면 탭 끌기 전 웹뷰 포커스를 다시 잡는다(`window.focus`) · 그래도 → **멈춤**(R1) |

- **macOS** — WKWebView 의 창 사이 끌어 놓기 · 끄는 중 포인터 사건 · 끌기 그림은 이식 때 S1 · S2 · S4 를 같은 꼴로 돌린다(W11).

## 9. 테스트 계획

층마다 seam 을 끊어 단독으로 잰다(ADR-0012). 대상은 CLAUDE.md 「빌드 · 검증 명령」의 기존 타깃이고 새 타깃은 만들지 않는다.

| 층 | 무엇 | 어디(타깃) |
|---|---|---|
| 트리 연산(끝) | `docked_tree` 네 변 × (슬롯 · 바깥) · 1/3 · 2/3 · 안쪽 비율 보존 · id 보존 · 새 면적 0 · `incoming_rect` | `layout/dock.rs` → `cargo test -p engram-dashboard --test lib_unit` |
| 모델(끝 + 4 단계) | `dock` 세 모드 판정 순서 · 사유마다 무변경 · 성공마다 `version` +1 한 번 · 곁표 · 탭바 자리 · 승계 · 빈 팝아웃 제거 · `Keep` / `Reveal` · 같은 창 활성 · 표식 · `SourceChanged` · `TargetChanged` · 묘비 · `WindowExists` · `plan_new_window` · `succession` = `close_tab` · **(4 단계) 메인 마지막 탭 → 다른 창 Seam · TabBar · NewWindow 셋 다 메인에 빈 탭 하나 · 그 탭이 활성 · `lease_tab` · `plan_new_window` 가 메인 마지막 탭을 받음 · 같은 창(SelfTarget · 같은 자리)은 빈 탭을 안 만듦 · `close_tab` 의 기존 빈 탭 시험이 도우미 뽑기 뒤에도 초록 · (4 단계 고침 둘) `TargetExpect::Seam` — 대상 탭 배치가 같아도 대상 창 활성이 다르면 `TargetChanged` · 무변경 · 같은 창 도킹(활성 = 끄는 탭)의 기대값은 그대로 통과 · TabBar 는 활성과 무관 · (4 단계 고침 셋 · 5.4) `plan_new_window` — 탭 하나 팝아웃 + 모델 보통 자리 있음 = `Reuse{그 창}` · 보통 자리 `None` = `Create{size: None}` · 최대화 중이어도 보통 자리가 있으면 `Reuse`** · 매 시험 뒤 `assert_invariants`(ADR-0057 ①–④) | `layout/manager.rs` → `lib_unit` |
| 닫기 요청(3 단계) | 닫기 요청 정리 뒤 그 창 탭이 모델에서 사라짐 · 묘비 · `Destroyed` 정리 멱등 · 뒤이은 Seam · TabBar · 새 창 · 팝아웃 기존 · 새 창 타깃 전부 무변경 · 닫히는 창의 늦은 replay 는 Subscribe 0 | `commands/popout.rs` · `daemon_client/tests.rs` → `lib_unit` |
| 적용 서비스(4 단계) | 가짜 포트로 Seam · TabBar 의 변경 · 스냅숏 · `resync` 가 한 락 안 · 알림은 락 밖 · 출처 창 닫힘 = 알림 뒤 `host.close` 한 번 · 실패면 알림 0 / NewWindow 열기 실패 = 무변경 · phase C 실패(묘비 포함) = 연 창을 닫고 탭은 출처 · 재사용 = `move_window` 한 번 · C′ 재확인 통과 · 같은 락 안에서 표식 내려감 · 모델 `version` 불변 · `record_placement` 한 번 / **재사용 되돌리기(5.2)** — 가짜 창 포트의 `move_window` 가 그 자리에서 끼어든 쓰기를 한다(출처 탭에 슬롯 더함 = `SourceChanged` · 그 팝아웃에 탭 더함 = `Create` 로 바뀜 → `SourceChanged` · 출처 탭 닫음 = `TabNotFound`): 결과 `Err` · `move_window` 한 번 · `restore_window` 한 번 · `record_placement` 0 · 표식 그대로(장부 끝이 내린다) / **되돌릴 자리 = 모델(5.3)** — 숨은 최대화 팝아웃(모델 = 보통 자리 B0 + `maximized`)에서 가짜 포트의 게터가 최대화 사각형을 돌려줘도 `restore_window` 가 받는 것은 B0 · `maximized = true` / **세터 실패마다(5.3 · 가짜 창 포트가 걸음별 실패를 낸다)** — B′ 위치 실패 · B′ 크기 실패 = 둘 다 `restore_window`(B0 · 최대화) 뒤 `WindowOpen` · 다른 걸음은 안 불림(첫 실패에서 멈춤) / 되돌리기의 위치 · 크기 · 최대화가 각각 실패 = 나머지 걸음은 그래도 불림 · 결과는 재확인 오류 그대로 · 끝 로그 `restored = false` · `failed` 에 그 걸음 / 모델 `bounds` 없는 탭 하나 팝아웃 = 재사용이 아니라 Create 길(새 창을 열고 · 빈 출처 팝아웃은 알림 뒤 닫힘 · `move_window` · `restore_window` 0 — 5.4) / 창이 사라져 무동작이어도 결과는 재확인 오류 · `activate` = `FocusPolicy` · 안 걸치는 자리 = 기본 자리 · 메인 빈 탭이 알림의 메인 탭 목록에 실림 · `settled` 가 쓰기마다 알림 뒤 정확히 한 번(`dock_tab` · 기존 쓰기 · 복원 — 가짜 이벤트가 순서를 적는다) · 실패한 쓰기는 `settled` 0 | `tests/layout_apply.rs` → `--test layout_apply` |
| 판정(끝) | `locate` · `choose_seam` · `choose_band` 의 규칙 전부(5 단계 시험 그대로) | `dock/hit.rs` → `lib_unit` |
| 영역 · 거울(6 단계) | `regions_for` — 숨긴 출처 없음 · `blocked` = 비움 · 크롬 · 캔버스 없음 = 칸 · 바깥 없음 · 출처 머리는 크롬에 남아 있어도 id 로 걸러짐 · 보이는 탭 = `display_active` · 면적 0 띠 빠짐 · `insert_xs` = `tab_insert_x` · 기대값(`Seam{배치, 모델 활성}` — 출처 창에서 끄는 탭이 활성이면 그 탭 · `Tabs`) / `pick` = 골든 전부 / **골든 사례가 덮을 것** — 각 변 띠 · 가운데 · 모서리 정규화 거리 · 바깥이 이김 · 무효 바깥 → 밑 칸 · 경계 픽셀(네 변 · 1px 띠) · 탭바 자리(가운데 x · 출처 머리 거름) · `over` · 캔버스 밖 · 보이는 탭 없음 / 철자 — `Picked` · `DockRegions` 의 wire 꼴(snake_case · 문자열 id) | `dock/regions.rs` · `dock/types.rs` → `lib_unit` · 같은 골든을 vitest |
| 장부(6 단계) | 가짜 시각 · 커서 · 사각형 · 스냅숏으로 `step` 의 할 일: 시작(숨김은 탭 하나 팝아웃만 · `Started` · 창마다 영역 · 메인 마지막 탭 = `display_active` 없음) · 영역은 (영역, 기대값)이 바뀐 창만 `rev` +1 · 기하가 같고 기대값(대상 칸 내용 · 창 활성)만 바뀌어도 `rev` +1 · 옛 스냅숏 번호 버림 · 세션 전 스냅숏(번호 큼)은 `Start` 뒤 이어 먹힘 · 작으면 버림 · 출처 배치 바뀜 = `blocked` 로 모든 창 다시 · `display_active` 바뀜 = `Display`(값이 바뀔 때만 · `Started` 와 같은 칸) · 출처 View 없음 = `SourceGone` 끝(되보이기 없음) · 출처 `Destroyed` = `SourceDestroyed` 끝 · 스냅숏에서 빠진 창 = `sent` 지움 · `seen` 남음 / **놓기 판정** — `blocked` = 무변경 · 보인 것 없음 + 지금 판정 없음 = 새 창(`rev` 무관) · 보인 것 없음 + 지금 판정 있음 = 무변경 · 보인 것 있음 + W 영역 없음(빠짐 · 소멸) = 무변경 · 이음새 — `rev` 다름 = 무변경 · `displayed` ≠ 그 `rev` 의 `view_id` = 무변경 · 다시 고른 것 ≠ 보인 것 = 무변경 · 모두 같음 = 적용 1회(`Reveal` · 표식 · 기대값 `Seam{배치, 창 활성}`) · **탭바(5.2)** — `bar_rev` 같고 같은 `index` = 적용 1회(기대값 `Tabs`) · `rev` · `displayed` 가 달라도(활성 전환 직후) 적용 · `bar_rev` 다름 = 무변경 · 활성만 바뀐 스냅숏 = `rev` +1 · `bar_rev` 그대로 · 탭 순서 · 탭바 기하가 바뀐 스냅숏 = 둘 다 +1 · 탭바 `over` 차이는 같음으로 / **끝 신호** — `drop` 뒤 `dragend(move)` = 늦은 신호 · `dragend(move)` 먼저 → `drop` 이 시한 안 = 적용 · 시한 지남 = 마지막 점(그 끝 신호 때 커서) · `dragend(none)`: W(마지막 Over 창 — 뒤에 Left 가 와도 W) + 아직 위 + 커서 안 = W 마지막 보고로 판정 · Left 가 유예 안 + 커서 안 = 같음 · Left 가 유예 밖 = 새 창 · 아직 위 + 커서가 W 밖 = 새 창(보인 가이드와 무관) · W 사각형 없음(소멸 · 최소화) + 가이드를 보임 = 무변경 · 가이드 없음 = 새 창 · 보고 없음 = 새 창 · 커서 못 읽음 = W 마지막 점(아직 위 = 판정 · 아니면 그 점의 화면 자리에 새 창 · W 없음 = 기본 자리) · 숨긴 출처는 후보 아님 · `blocked` 면 어느 끝이든 무변경(새 창 없음) / 시한 — 지난 번호의 `Timeout` 버림 · `Grace` 중 `dragend(move)` = `AwaitDrop` 으로 · `AwaitDrop` 중 `pointer_seen` 버림 · `drop` 의 `view_id` 가 출처가 아니면 버림 / 유실 — `pointer_seen` → 시한 안 끝 신호 = 평소 · 시한 지남 + W 아직 위 = 그 보고로 판정(사각형 검사 없음 · W 소멸이면 무변경) · W 떠남 = 지금 커서에 새 창 / **이른 끝 신호** — 모르는 id 의 `drop` · 출처 끝 = 붙듦 → 시한 안 그 id 의 `Start` = 시작 할 일 뒤 도착 순서대로 처리 · 시한 지남 = 버림 + 그 id 가 `ended` 로 · `Refused` = 버림 · 다른 모르는 id 가 오면 옛 것 버림 + `ended` 로 · 최근 끝난 id = 붙들지 않고 버림 · 버린 id 의 늦은 시작 = `LateStart` 거절(표식 · 숨기기 · `Started` 0) · 붙든 놓기(보인 것 없음)는 지금 판정이 가이드면 무변경 / **순서** — 옛 `rev` 의 `Over` = 지금 영역을 같은 `rev` 로 다시 보냄 / **적용 중** — Seam 성공 중 출처 View 없는 스냅숏 = 끝나지 않음(영역 · `Display` 도 없음) → `Applied(Ok)` = applied · 대상 `Focus` · `Ended` 는 `Applied` 뒤 / 탭 하나 팝아웃 출처가 적용 중 `Destroyed` = 끝나지 않음 → `Applied(Ok)` = applied · 출처 되보이기 없음 · 대상 `Focus` / `Applied(Err)` = 무변경(사유 그대로) / 한 세션에 적용 한 번 · `Applying` 중 끝 신호 버림 / **새 시작** — 옛 세션 `Live` = 새 것 거절 · 옛 것 마지막 점 · 옛 세션 `Applying` = 새 것만 거절(`applying`) · 옛 것은 `Applied` 로 끝 / 끝 할 일 순서 `Ended` → 되보이기 → `set_focus` → `release_lease` · 재사용 = 보이기 · `seq` 단조 · 끝난 id 가 `ended` 에 남음 | `dock/book.rs` → `lib_unit` |
| 실행기(6 단계) | 순수 조각 — 새 창 자리 = 커서 − 그림 커서 자리 × 배율 · 커서 못 읽음 = 마지막 점 · 그것도 없으면 자리 없음 · 사각형은 보이고 최소화 안 된 창만 / 계기 깃발 — 세션 없으면 스냅숏을 안 뜬다 · 깃발은 시작의 레이아웃 락 안에서 세대로 서고(`lease_tab` 성공 뒤) 끝에서 자기 세대일 때만 내려간다(다음 세션이 세운 세대는 옛 세션의 끝이 못 내림 — 5.2) · `settled` 한 번에 스냅숏 하나 / 게터 — 가짜 창 포트가 불릴 때 레이아웃 락이 풀려 있음 / **`Apply` 는 늘 spawn(5.2)** — 가짜 spawn 포트로: 동기 시작이 붙든 놓기를 이어 먹여 낸 `Apply` 도 spawn 으로 가고 시작 명령 안에서 적용 서비스 · 창 포트가 불리지 않음 · `Applied` 와 끝 할 일이 그 spawn 작업에서 먹힘 | `commands/dock.rs` `#[cfg(test)]` → `lib_unit` |
| 프론트(7 단계) | `TabBar` — 머리 `draggable` · 이름 입력 · 닫기 버튼에서 시작한 끌기 막음 · `dragstart` 가 우리 MIME 하나만 · `effectAllowed = move` · `setDragImage` 호출 · `dragstart` 안 DOM 무변경 · `dock_drag_start` 는 디스패치 뒤 · 다른 리스너가 `dragstart` 를 취소하면 0 · `dragend` → `dock_drag_end{effect}` / **끊기(5.1)** — window 에 react-dnd 꼴 리스너(`preventDefault` · `dropEffect = 'none'`)를 단 jsdom 에서 우리 `dragstart` · `dragenter` · `dragover` · `drop` 이 거기 닿지 않음 · 우리 MIME 이 아닌 끌기는 닿음 / 대상 — 우리 MIME 이면 세션을 몰라도 받음 · `blocked` = `dropEffect none` · `pickTarget` 결과대로 `DockOverlay` · 보이던 탭 ≠ 영역 `view_id` = 그리지 않음 · `shown` · `rev` · `displayed` 가 바뀔 때만 `Over` · `drag_id` 모르면 보고 0 · 떠남 = `Left` · `drop` = `dock_drop{rev, bar_rev, displayed, shown, point}` · 보이던 탭 ≠ 영역 `view_id` 여도 탭바 가이드는 그리고 탭바 놓기를 보낸다(5.2) · 영역 없음 = `dock_pull` 한 번 · 머묾 500ms = 탭 전환 · 떠남 · `Ended` 가 머묾 취소 / 영역 · 세션 — `drag_id` 별 가장 큰 `rev` · `seq` 만 · 작은 것 버림 · `Started` 전 영역 받음 · `Display` 가 `Started` 보다 먼저 와도 같은 상태 · `Ended` 된 id 버림 / 떼어진 표시 — 스토어 `win.tabs` 그대로 · 0탭 자가닫힘 안 불림 · 머리는 DOM 에 남고 접힘 · `displayActive` 로 캔버스 · 메인 마지막 탭 = 빈 캔버스 · `Ended` 로 거둠 / `pointer_seen` 은 세션당 창당 한 번 · 마우스 · `buttons === 0` 이동 · `pointerdown` 만(버튼 쥔 이동 · 펜 · 터치는 아님) / Esc — 세션 + 꼬리 동안 capture 단계에서 삼킴 · 그 창의 끝 증거 뒤 200ms 에서 그침 · 그 밖은 그대로 / 파일 막기 — 우리 MIME 이 아닌 끌기는 `dropEffect none` · `drop` `preventDefault` · 끊지 않음 / `pickTarget` = 골든 / 끌기 그림 크기(720×500 → 288×200 · 1600×1000 → 320×200 · 400×300 → 180×135 · 제목만 180×28) · 그림 실패 = 제목만 / `dockChromeReport` — 한 번에 하나 · 마지막 값이 덮음 · 접힌 머리 뺌 | vitest(`npm test`) + `npx tsc --noEmit` |
| 생성물 | 새 ts-rs 타입 · 지운 `GuideView.ts` · `TabBarHint.ts` | CI 생성물 sync 게이트(`lib_unit` 이 굽는다) — ★지운 타입의 `.ts` 는 게이트가 못 잡으므로 6 단계 커밋에서 손으로 지운다★ |
| 화면(QA) | AC1–AC9 · AC11–AC16 · AC18–AC22 를 실제 창으로 — `/qa full` 의 cdp 실측 + 팝아웃 X 와 끌기 겹치기(M2) · 문턱 직후 떼기 · Esc 위치별 · 트리 슬롯이 있는 창에서 끌기와 트리 재부모화(S1 ⑦) · 머묾 전환 직후 이음새 놓기(무변경) · 머묾 전환 직후 탭바 놓기(적용 — 5.2) · 최대화 탭 하나 팝아웃의 R22 재사용. 배율 둘 모니터(AC12)는 사람 손 확인이 필요할 수 있다 | `/qa full` |

- **게이트** — OS 분기 0 이라 4f · 4g · 4h 명단이 안 바뀌어야 한다(바뀌면 설계 위반 신호). 셸 → agent 운영 의존 0(4i). `cargo fmt --check` · 워크스페이스 회귀(`-- --test-threads=4`).
- **M2 전제 점검(3 단계부터 매 `/qa` · 저장소 루트)** — `rg -i "close.?requested" src/` → **0줄**(프론트 닫기 요청 리스너 없음) · `rg "prevent[_]close" src-tauri/src/` → **1줄**(`lib.rs` 의 main 갈래). `[_]` 는 처리기 주석의 자기 인용이 걸리지 않게 하려는 것이다. 어긋나면 M2 의 틈이 다시 열린 것이라 그 변경을 막고 §1-2 를 「닫는 중 표시」 안으로 고친다. **`tauri-plugin-*` 의존을 더하거나 판을 올릴 때** — 그 crate 소스에서 `prevent_close` 를 손으로 찾는다.
- **드롭 처리기 점검(6 단계부터)** — `rg "disable_drag_drop_handler" src-tauri/src/` → **1줄**(`webview_env.rs`) · `rg -i "DragDropEvent|onDragDropEvent" src/ src-tauri/src/` → **0줄**(끈 처리기의 사건을 기대하는 코드 없음).
- **다른 끌기 리스너 점검(7 단계부터 · 5.1)** — `rg "dndRootElement|dndManager" src/` → **0줄**(react-dnd 뿌리 = `window` — §1-4 불변식) · `package.json` 에 HTML5 끌기 라이브러리(react-dnd 계열 · dnd-kit · pragmatic-drag-and-drop 등)를 더하거나 react-arborist 판을 올릴 때 S1 ⑦ 을 다시 돈다.
- **시험이 못 덮는 것** — 실제 OS 끌기(사건 순서 · 간격 · 유실) · 실제 react-arborist 와 엔진 사이의 끌기 시작 · 거절(jsdom 은 OS 끌기를 시작하지 않는다 — S1 ⑦) · 끌기 그림의 모양 · WebGL 그림 · 숨김 · 다중 모니터 · 파일 드롭 · `lib.rs` 배선은 §8-2 와 `/qa full` 몫이다. 시험은 명세한 동작의 회귀 안전망이지 완전성 보장이 아니다.

## 10. 이행 순서

★커밋마다 빌드 · 시험 초록 — 어디서 멈춰도 선다★. 자료구조를 먼저 갈아엎고 부르는 쪽을 나중에 맞추지 않는다 — 각 단계는 새 것을 **더하고** 그 단계 안에서 부르는 쪽까지 닫는다. 커밋 제목 = `S21: <타입>(<범위>): …`.

| 단계 | 내용 | 상태 · 의존 | 멈추면 남는 것 |
|---|---|---|---|
| **1** | `layout/dock.rs` 트리 연산 + 시험 | 끝(`080893c5`) | 순수 함수 |
| **2** | `ViewManager` 도킹 모델(`dock` 세 모드 · `plan_new_window` · 표식 · 묘비 · `DockGuard` · `FocusPolicy` · `DockError` · `close_tab` 도우미 · `succession`) + 시험 | 끝(`ca723ed5`) | 모델 연산 |
| **5** | `dock/hit.rs` · `dock/types.rs` · `layout/chrome.rs` + 시험 | 끝(`e73e0289`) | 순수 판정 |
| **3** | 닫히는 창(M2) — 닫기 요청 모델 먼저 · 묘비 · `apply.rs` 주석 · 닫히는 창의 늦은 Subscribe 가드 + 시험 | 끝(`281bd235` — 미커밋이던 4판 `dock/session.rs` · `pub mod session;` 은 넣지 않고 지웠다 · §11-3) | 팝아웃 닫기의 기존 틈이 닫힌 셸(도킹 없이도 값어치) |
| **4** | ★3 단계 뒤★ 메인 마지막 탭 고침(`MainLastTab` 제거 · `refill_main` 도우미를 `close_tab` 과 나눔 · 시험 갱신) → ★대상 창 활성 고침(5.1 · C1 — 2 단계 코드 변경)★ `TargetExpect::Layout` → `TargetExpect::Seam{layout, active}`(`manager.rs` 정의 · `check_dock` 의 Seam 대조 팔 `:1253` 에 `windows[대상 창].active` · 판정 순서 주석 · 시험의 `TargetExpect::Layout` 자리 여덟 + 「활성만 바뀜 = `TargetChanged`」 새 시험) → ★재사용 조건 고침(5.4 · 2 단계 코드 변경)★ `plan_new_window` 의 `Reuse` 갈래에 「출처 창 `attrs.bounds` 가 `Some`」 · 없으면 `Create{size: None}` · 그 주석 · 시험 둘 → ★알림 묶음 끝(5.1 · F1)★ `LayoutEvents::settled`(기본 본문 없음 — 구현자 아홉 · `TauriEvents` 는 빈 몸 · `OwnedEvents` 는 `TauriEvents` 로 넘기기 · 부르는 자리 = `notify` · `rename_tab` · 슬롯 옮기기 · `restore.rs` 알림 루프) → 적용 서비스 `apply::dock_tab`(Seam · TabBar 한 락 · NewWindow 세 phase · 재사용 = 옮기고 → C′ 재확인 → 어긋나면 되돌리기(5.2) · 알림 뒤 `settled`) + `WindowHost::{open_at, move_window(검사 세터) · restore_window(모델 자리로 되돌리기)}` · `placement` 의 결과를 돌려주는 자리 입히기 짝(5.3)(★기본 본문 없이 — 구현자 여섯을 같은 커밋에서 닫는다★) + Tauri 구현 · `OwnedWindowHost` 넘기기 · `placement::place` · `land` 공개 + 시험(`lib_unit` · `layout_apply`). 화살표 순서대로 커밋을 나눠도 각각 선다 | 없음 | 아무도 안 부르는 적용 서비스(다음 작업의 명령도 그 위에 선다) |
| **6a** | `dock/regions.rs`(`regions_for` · `pick` · 기대값 = `Seam{배치, active}`) · `dock/types.rs`(`DockRegions` · `OuterBand` · `SlotRegion` · `TabBarRegion` · `Picked` · 세션 사건(`Display` 도 `Started` 와 같은 칸) · 보고(`Over` · 놓기의 `displayed` · `bar_rev`) · `DockRegions.bar_rev`(5.2) — `GuideView` · `TabBarHint` 와 그 `.ts` 를 지움) · `pick_golden.json` + 시험 | 4 의 `TargetExpect` 고침 뒤(`TargetExpect` 를 쓴다) · 그 밖은 4 와 병렬 가능(겹침 = `dock/mod.rs` 한 줄) | 순수 영역 · 판정 |
| **6b** | `dock/book.rs` 장부 + 시험 — 5.1 · 5.2 규칙(§1-3 — 창별 Over · Left · 놓기 판정(탭바는 `bar_rev`) · 이른 끝 신호(버린 id 는 `ended` 로) · 적용 중 · 세션 전 스냅숏 · 옛 `rev` 다시 보냄 · 커서 못 읽음) · 4판 `session.rs`(지움)에서 `visible_tab` · 끝 어휘(`Outcome` · `EndReason`) · 띠 캐시 꼴만 옮겨 온다(`git` 이력에 없다 — 사본 `.claude/handoff/attachments/slot-docking-session-rs-replaced-2026-10-16.rs.txt` 를 본다) | 6a 뒤 · 4 와 병렬 가능 | 순수 장부 |
| **6c** | 크롬 측정(`WindowTabs.chrome` · `set_dock_chrome` · `state/convert.rs` 구조 분해에 `chrome: _` — 저장 안 함) · `commands/dock.rs`(invoke 여섯 + `report_dock_chrome` · 실행기 · 시한 셋 · 게터는 락 밖 · 세션 깃발 = 세대(시작의 레이아웃 락 안에서 세우고 비교-교체로 내림) · `Apply` 는 늘 spawn · `Refused` · `LateStart` · 계측) · `lib.rs`(`generate_handler!` · `Destroyed` → 장부) · `TauriEvents::settled` 계기 · 측정 보고 뒤 계기 · `WebviewEnv::finish` 의 `disable_drag_drop_handler()` + 시험 | 4 · 6b 뒤(`manager.rs` · `commands/layout.rs` 겹침) | 웹뷰가 안 부르는 셸 끌기 처리(무해 — 끈 처리기의 사건은 쓰는 곳 0) |
| — | **실측 S1–S4 · S6**(§8-2) — 결과를 이 문서 개정으로. 불합격이면 멈추고 사용자에게 | — | — |
| **7** | 프론트 — `TabBar` 끌기 출처(우리 MIME `stopPropagation` · 디스패치 뒤 시작) · `dockDrag.ts`(대상 · 세션 몰라도 받기 · 끊기 · `drag_id` 별 최대 `rev` · `seq` · 보이던 탭 대조 · 세션 구독 · 유실(버튼 0) · Esc 삼키기(창별 상한) · 파일 막기) · `dockPick.ts` + 골든 vitest · `dragImage.ts` · `DockOverlay.tsx` · `dockChromeReport.ts` · `WindowLayout` 떼어진 표시 + 시험 · 커밋 전 S1 ⑦ · ⑧ — **마우스 도킹이 끝까지 된다** | S1 · S2 · S3 · S4 | 마우스 도킹 |
| **8** | 문서 — ADR(§12) · step-log · CLAUDE.md 「LLM-우선 제어」 남은 갭 한 줄 · `docs/process/S21-slot-docking/structure.html` 갱신(`/review doc`) · `/qa full` 화면 실측(AC 전부 · S5) | — | — |

- 매 단계 게이트 = `/implement`(구현 → `/review code` → `/qa`) — CLAUDE.md 「구현 실행 규약」.
- **나누는 축 = 파일 겹침** — 3 → 4 → 6c 는 `layout/*` · `commands/*` · `lib.rs` 가 겹쳐 순서대로. 6a · 6b 는 `dock/*` 만이라 4 와 병렬(단 6a 는 4 의 `TargetExpect` 고침을 쓰므로 그 커밋 뒤). 7 은 프론트만이라 6c 와 접점(§1-5 wire)을 먼저 못 박으면 병렬도 된다 — 단 실측 결과를 기다린다.
- ★3 → 4 순서는 바꾸지 않는다★ — 적용 서비스는 닫히는 창을 3 단계의 모델 먼저 지우기 · 묘비에만 기대어 막는다(사전 · 사후 `is_open` 확인 없음). §11-1 #2 의 답 = M2 확정(2026-10-16)이라 2판의 두 검사(Seam · TabBar 의 `is_open` 사전 확인 · NewWindow 의 확정 뒤 확인과 정리)는 되살리지 않는다.
- 7 단계 뒤가 PRD 의 기본 흐름(팝아웃의 탭을 끌어 붙이기 — PRD §1) 완성이다. 다음 작업(LLM 표면)은 4 단계만 있으면 시작할 수 있다.

## 11. 위험 · 미결

### 11-1. 사용자 확인 대상 — 사용자 결정 2026-10-16 · 8건 전부 기본값 확정

PRD 4.4 Q11–Q18 이 같은 결정이다. 아래 「다른 안」은 거부한 대안으로 남긴다.

1. **같은 창 안 도킹에서 출처가 보이던 탭이면 무엇이 보이나** — 기본값 = **대상이 보이게 된다**(이음새 = 대상 탭 · 탭바 = 옮긴 그 탭) — 보던 내용이 그 탭에 들어가 화면에 남는다. 다른 안 = 탭 닫기와 같은 승계(오른쪽 · 왼쪽 이웃). → **확정**.
2. **닫히는 창과 모델 변형의 조정 = M2**(§1-2 「닫히는 창」 — 3 단계에 구현) — 팝아웃 닫기 요청에서 모델을 먼저 지우고 묘비. 대가 = 팝아웃 모델 정리의 계기가 소멸 → 닫기 요청으로 옮긴다(ADR-0057 불변식 그대로 · ADR-E). 다른 안 = 열림 사전 확인 + 남는 위험 기록(몇 ms 겹침에 붙인 탭이 그 창과 함께 사라짐). → **확정** — 따로 예외 처리 없이 흐름에 맡긴다: 도킹 확정이 닫기보다 먼저면 붙은 탭은 그 창과 함께 닫히고(기존 팝아웃 닫기 규칙), 닫기가 먼저면 대상 없음으로 무변경(탭은 출처). 「닫기가 먼저면 그 자리에 새 창」은 채택하지 않았다 · 토스트 · 툴팁도 없다.
3. **R22 새 창 크기 — 출처 창이 최대화였을 때** — 기본값 = 그 창의 보통(최대화 전) 크기로 연다. 다른 안 = 최대화 상태 그대로. → **확정**(사용자가 Chrome 도 최대화 창에서 뗀 탭을 보통 크기로 연다는 것을 확인).
4. **R6b 로 바꾼 탭은 끌기가 무변경으로 끝나도 그대로 둔다** — 기본값 = 되돌리지 않음(브라우저 · VS Code). 다른 안 = 끌기 시작 때 탭으로 되돌림. → **확정**(관례대로).
5. **칠하지 않은(면적 0) 바깥 띠 위에 놓으면** — 기본값 = 밑 슬롯 띠로 넘김(그 칸 띠에 들면 그 이음새, 안 들면 R22) 다른 안 = 무효 바깥 띠 = 새 창. → **확정** — PRD R8 을 맞췄다(4.4). 면적 0 은 정규화 좌표 계산이라 극단적 중첩에서만 생긴다 — px 로 작은 칸(예: 50px)은 막히지 않는다.
6. **같은 자리 탭바 놓기 + 마우스 = 그 탭을 보인다** — 안 보이던 탭을 끌어 자기 창 탭바의 원래 자리에 놓으면 기본값 = 그 탭을 보인다(`version` +1). 다른 안 = 진짜 무변경(리뷰어 대립이던 것). → **확정**(사용자 결정).
7. **끌기 그림 내용(PRD Q5 「되는 만큼」의 범위)** — 기본값 = 끄는 탭이 보이는 탭이면 **터미널 칸 = 그 화면의 축소 그림 · DOM 칸(챗 · 목록 · 사용량 · 빈 칸) = 칸 이름 자리채움**(DOM 은 끌기 시작 순간 동기로 그림에 옮길 수 없다) · **안 보이던 탭 = 제목만**(그 칸들은 화면에 없다). 다른 안 = 누르는 순간 그 탭으로 전환해 띄운 뒤 그리기(탭 클릭이 「누를 때 전환」으로 바뀌고, 끌지 않은 누름도 탭을 바꾼다 — 브라우저 · VS Code 는 누를 때 전환). → **확정**.
8. **「보인 것과 다름」 · 출처 · 대상 바뀜의 끝 = 무변경**(R13c · M1) — 결정 Q8 의 「원래 창 되돌리기 없음」은 **비정상 끝**에 대한 것으로 읽었다. 이 무변경은 못 본 결과를 만들지 않는 안전장치라 남겼고, 결과로 탭은 원래 창에 다시 보인다. 다른 안 = 이 경우에도 그 자리에 새 창(R22) — 못 본 이음새에 붙지는 않으면서 「되돌림」이 사라진다. → **확정** — 따로 처리하지 않는다: 붙일 슬롯이 없으면 도킹이 그냥 실패하고 아무것도 안 바뀐다. 「그 자리에 새 창」 · 「툴팁 표시」는 채택하지 않았다.

**PRD 와 다른 점**
- R21 「원래 창 탭바에 놓으면 그 창의 탭으로 돌아간다」는 놓은 자리로 옮겨 넣으므로 탭 순서 바꾸기도 함께 된다.

### 11-2. 기술 위험

| # | 위험 | 확신 · 대응 |
|---|---|---|
| 1 | 「마지막 점에 놓기」의 오판 — 우리 창을 덮은 다른 앱 창 위로 나가 `LEAVE_GRACE_MS` 안에 떼면 그 밑 우리 창의 마지막 가이드로 붙는다 · Esc 취소의 떠남이 시한보다 늦게 앞서 오면 창 안 Esc 가 새 창이 된다 · 가이드 위 Esc 뒤 셸이 커서를 읽기 전에 커서가 그 창 밖으로 나가면 새 창 | 가능성 낮음 · 사용자는 그 가이드를 직전에 봤다. S1 ① ② 가 간격 분포를 재고 시한을 고친다 · 끝 줄 `leave_gap_ms` 로 현장에서도 본다 |
| 2 | 끝 신호 유실 뒤 마우스를 우리 창에 안 가져오면 장부 · 표식이 남는다 — 그 탭의 떼어진 표시가 남고 다음 끌기가 거절되며, 키보드만 쓰는 창은 Esc 가 먹힌다 | 가능성 낮음(엔진의 `dragend` 는 실측 모든 경우) · 마우스가 우리 창에 오는 순간 `pointer_seen` 이 끝내고 그 창의 Esc 삼키기는 200ms 뒤 그친다(5.1 · §1-4) |
| 3 | 새 시작이 옛 세션을 만나면 새 끌기는 거절된다 · 옛 세션이 적용 중(새 창 열기 수백 ms)이어도 거절된다 · 옛 세션의 끝과 그 표식 내리기 사이의 새 시작은 `Dragging` 으로 거절된다 | 가능성 낮음(유실 + 300ms 안 새 누름 · 놓은 직후 곧바로 다시 끌기 · 마지막 것은 ms 아래의 틈 — 받아들인 남은 틈 · 5.2) · 사용자가 다시 끈다 |
| 4 | TS 거울과 Rust 판정이 갈림 | 골든이 막는다 · 갈라지면 놓기가 무변경으로 떨어진다(엉뚱한 도킹 아님) |
| 5 | 끌기 그림 — 반투명 · 다른 앱 위 사라짐 · 터미널 그림이 비어 보일 수 있음 · DOM 칸은 자리채움 | 확실(앞 둘 — 받아들임) · S2 가 WebGL 을 잰다 · §11-1 #7 |
| 6 | 파일 드롭으로 웹뷰가 탐색 | 불확실 · 막기 리스너(§1-4) · S3 ② · main 은 지금도 처리기가 꺼져 있다 |
| 7 | 숨긴 출처의 끌기(표본 2) · 숨김이 활성 창을 다른 앱으로 넘김 | 가능성 높음(무해 — 끌기는 전경과 무관하게 엔진이 몬다 · 놓은 뒤 `set_focus`) · S1 ⑤ |
| 8 | 도킹 직후 닫히는 팝아웃 웹뷰가 탭 0 개 순간을 그림 | 가능성 높음 · 무해 — 모델에서 창을 먼저 지운 뒤 OS 창을 닫는 순서는 `close_tab` 과 같다 · 탭 하나 팝아웃은 이미 숨어 있다 |
| 9 | 영역 다시 계산 비용(계기마다 모든 창 · 칸당 `docked_tree` 넷) | 가능성 낮음 · 끄는 동안만 · 바뀐 창만 보낸다 · 트리가 작다 |
| 10 | 떼어진 표시 직후 출처 창 탭바의 끼울 자리 막대가 접히기 전 기하로 머리 하나 폭만큼 어긋나 보임 | 확실(짧은 틈 — 보고 디바운스 + 왕복) · 무해 — 그림과 판정이 같은 영역이라 보인 자리로 들어간다 · 거슬리면 떼어진 표시 직후의 크롬 보고만 디바운스 없이 |
| 11 | 같은 웹뷰 안 도킹에서 칸 컴포넌트가 다시 마운트되며 터미널이 재구독 | 확실(구조) · 팝아웃과 같은 수준(D10). 출력은 replay 로 이어진다 |
| 12 | 끌기 중 앱 종료 · 셸 패닉 | 셸 패닉은 훅이 곧 종료다(ADR-0292) — 모델 · 저장은 도킹 전 판이라 탭은 한 곳이다(§5). 장부 · 실행기 안에서는 패닉하지 않게 짠다. 표식 · 묘비 · 장부는 메모리라 다음 부팅에 없다 |
| 13 | R22 새 창이 웹뷰를 띄우는 동안(수백 ms 추정) 화면에 아무것도 없음 | 가능성 높음 · 탭 하나 팝아웃은 재사용이라 해당 없음. 거슬리면 QA 에서 판단 |
| 14 | 닫기 요청 계기의 모델 정리가 기존 팝아웃 닫기를 바꿈 · X 직후 먼저 확정된 도킹의 탭이 그 창과 함께 닫힘 | 가능성 낮음 · 확실(닫기 의미 그대로 · 잔해 없음) — 팝아웃 닫기를 막을 수 있게 되는 날 근거가 무너진다(§9 점검) |
| 15 | LLM 이 도킹 · 레이아웃 조회를 못 한다(의식적 예외) | 확실 · 사용자 결정 · 다음 작업이 닫는다 — 그 사이 CLAUDE.md 「LLM-우선 제어」 남은 갭에 적는다(8 단계) |
| 16 | 문턱 직후 캔버스에서 떼면 새 창(또는 보이던 가이드에 붙음 · 영역이 아직 안 닿았는데 그 점이 가이드 자리면 무변경 — 5.1) | 확실(PRD R22 · 가이드 그대로) · 엔진 문턱이라 값을 고치지 못한다 — 거슬리면 QA 에서 「출처 창 안 · 시작 뒤 짧은 시간의 가이드 없음 놓기 = 무변경」을 사용자에게 올린다 |
| 17 | `adopt_restored` 가 묘비를 보지 않아 닫힌 팝아웃 label 을 들임 | 확실(구조) · 무해 — 복원 ④ 의 「사라진 창」 확인 · 뒤이은 `Destroyed` 가 지운다 |
| 18 | macOS(WKWebView)의 창 사이 끌어 놓기 | 모름 · 이식 때 S1 · S2 · S4 |
| 19 | react-arborist(react-dnd HTML5 백엔드)가 window 에 단 끌기 리스너 — 트리 슬롯이 마운트된 창(안 보이는 탭 포함)에서 우리 `dragstart` 를 취소하고(`HTML5BackendImpl.js:212`) `dragover` 의 `dropEffect` 를 none 으로 덮는다(:284-292) — 끊지 않으면 그 창에서 탭 끌기가 시작되지 않고 그 창에 놓을 수 없다 | 확실(소스 · 5.1) · 우리 MIME 사건을 `document` · 탭 머리에서 끊는다 · 세션은 취소 안 된 시작만 · S1 ⑦ · 뿌리를 `document` 로 옮기거나 끌기 라이브러리를 더하면 다시 잰다(§9 점검 줄) |
| 20 | 사건 순서 — emit · invoke 가 여러 스레드라 영역 `rev` 가 뒤바뀌어 닿고, 끝 신호가 시작보다 먼저 닿을 수 있다 | 가능성 낮음(빠른 끌기) · 웹뷰 최대 `rev` · `seq` · 옛 `rev` 보고에 다시 보냄 · 끝 신호 500ms 붙듦 · S1 ⑧ · 끝 줄 `regions_resent` · `early` |
| 21 | 거절된 끌기(옛 세션 · 탭 없음 · 락 오염 · 늦은 시작)도 다른 창이 받아 커서가 「이동」으로 보인다 — 놓아도 받은 쪽은 아무것도 바꾸지 않는다(모르는 id). 다른 앱 인스턴스(다른 데이터 폴더의 Engram)에서 끈 탭을 우리 창에 놓으면 우리 쪽은 그대로이고, 출처 인스턴스는 `move` 끝을 받아 `DROP_WAIT` 뒤 「마지막 점에 놓기」로 커서 자리에 새 창을 띄운다 — 그쪽에서 우리 창은 다른 앱 창이라 PRD R22 그대로다(5.1 전에는 우리가 거절해 `none` 끝 → 같은 새 창) | 확실(세션을 몰라도 받는 대가 · 5.1 · 문구 5.2) · 드묾 · 무해 |
| 22 | 보이는 탭이 바뀌는 순간(R6b 머묾 · 클릭)의 이음새 놓기는 무변경 | 확실(짧은 틈 — 알림 · 영역 왕복) · 사용자가 다시 끈다 · 탭바 놓기는 `bar_rev` 로 따로 대조해 그대로 적용된다(5.2) |
| 23 | 동기 명령 안의 창 만들기 교착 — 시작(메인 스레드 · 동기)이 붙든 이른 놓기를 이어 먹이면 그 판정의 `Apply`(R22 새 창)가 동기 명령 안에서 돈다 | 확실(구조 — 막지 않으면) · 가능성 낮음(이른 놓기 + 새 창 판정) · `Apply` 는 늘 spawn(§1-3 불변식 · 5.2) · 실행기 시험이 spawn 포트로 잰다 |
| 24 | R22 재사용 도중 출처가 다른 경로로 바뀜(슬롯 · 탭 더함 · 닫힘) · 옮기기 세터 일부 실패 | 가능성 낮음 · 옮긴 뒤 같은 락 안 재확인 · 어긋나거나 세터가 실패하면 모델 자리로 되돌리기(§1-2 · 5.2 · 5.3) · 최대화 아니던 출처는 숨어 있어 안 보인다 · ★최대화였던 출처는 B′ 의 최대화 풀기(tao `SW_RESTORE`)가 창을 보이고 포커스를 가져간다 — 성공이면 끝의 `set_focus` 와 같고, 되돌리면 그 자리에 떴다가 원래 자리 · 최대화로 돌아가는 것이 보인다(확실 · 소스 · 받아들인 경우)★ · 숨기기가 실패한 출처도 같다(두 겹 실패) · ★미결 — 숨긴 채 최대화를 푸는 길은 platform 목적 함수(Win32 `SetWindowPlacement` 류)가 있어야 하고 이번엔 넣지 않는다★(S1 ⑤ 가 재고, 거슬리면 사용자 보고 뒤 ADR-0266 절차로) |

### 11-3. 5판에서 뺀 것

- **셸 폴링 끌기**(4판 §1-3 · §1-4) — 끌기 스레드 · 8ms 마감 틱 · `TickClock` · 세션 입력 enum(`ForegroundSeen` · `PointerSeen` · `KeySeen` · `TopSeen`) · `Probe` 질문 · 전경 유예(`INPUT_LOST_MS`) · `esc_armed` · 시작 되감기 가드(`BeginGuard`) — 엔진이 같은 일을 한다. 미커밋이던 `dock/session.rs` 는 지웠다(3 단계 커밋에 넣지 않음 · 사본 = `.claude/handoff/attachments/slot-docking-session-rs-replaced-2026-10-16.rs.txt`) — `visible_tab` · 끝 어휘 · 띠 캐시 꼴만 6b 로 옮긴다.
- **platform 함수 · 의존** — `topmost_at` · `NativeWindow` · `foreground`(+ 옛 `foreground_is_current_process` 감싸기) · `pointer` · `escape_key` · `capture_client` · (조건부) `fine_timer` · `raw-window-handle` 운영 의존 · `windows` feature 넷(4판 §11-1 #3 · ADR-C).
- **딱지 창** — `dock-chip.html` · `src/dockChip/main.ts` · Vite 입력 · `capabilities/dock-chip.json` · `dock/chip.rs` · `dock/preview.rs` · 셸 직접 의존 `png` · `base64` · `dock:chip` · `dock:chip-preview` · 자리 기록 거름 · 4판 #43–#46 · #50.
- **출처 숨기기의 화면 밖 옮기기 · 자리 기록 멈춤**(`placement.rs` `record` 거름) — `hide()` 는 자리를 안 바꾼다.
- **LLM 표면**(4판 §3 · ADR-D) — 버스 `tab.dock` · `catalog_version` 18 · 오류 코드 표 · invoke `dock_tab` · 프론트 레지스트리 `tab.dock` · `viewStore.dockTab` · `DockSide` 버스 철자 맞대기 — 다음 작업.
- **`MainLastTab`** — PRD Q9(메인 마지막 탭 끌기)로 거절 사유가 없어졌다.
- **비정상 끝 = 출처 창 탭바로**(4.2 R13b · D11) — PRD Q8.
- **4판 스파이크 S1–S6** · `spike-s2-topmost-2026-10-15.md` 의 결과 — 대상이 없어졌다(§8-1).
- (1판에서 이미 뺀 것) 직전 도킹 되돌리기 · 커서를 따라오는 창 · Esc 취소 · 명령 둘(`tab.dock` · `tab.moveToWindow`).

## 12. ADR 제안 (번호는 `/adr` 가 준다)

결정 날조 금지 — 「거부한 대안」은 PRD · 조사 · 사용자 대화 · 리뷰에서 나온 것만 적었다. 사용자 결정분은 사용자 확인 뒤 박는다(§11-1 은 2026-10-16 확정).

- **ADR-A 도킹 모델 = 탭 끌기 · 즉시 떼기 · 배치는 놓을 때 한 번 · 보인 것과 같을 때만 적용 · 붙는 단위는 탭 하나의 배치 통째 · Esc · 비정상 끝 = 그 자리에 놓기 · 메인 마지막 탭도 끌고 떠나면 빈 탭**(사용자 결정 R1 · R2 · R9 · R12 · R13b · R14 · Q1 · Q6 · Q8 · Q9 + 이 TRD 의 구현 선택)
  - 결정: 제목줄은 OS 이동만, 탭을 끌면 그 자리에서 떼어진다(그리기). 놓으면 이음새 · 탭바 · 새 창 셋 중 하나이고, 셸이 놓은 점을 다시 판정해 보인 가이드와 같을 때만 적용한다(보인 것도 판정도 가이드 없음 = 새 창 · 그 밖의 어긋남 · 대상 사라짐 = 무변경 — TRD 5.1). Esc · 비정상 끝은 마지막으로 알려진 점에 놓는다(출처 탭이 사라진 경우만 그냥 끝). 붙이기는 모델 이동이고 메인이 비면 같은 연산 안에서 빈 탭을 채운다(ADR-0057 ④).
  - 거부한 대안: ① 제목줄(네이티브 이동 루프) 도킹 — 사용자 정정(R1). ② 창이 커서를 따라오는 끌기(1판) — Q1. ③ 끄는 동안 모델을 바꾸고 취소 때 되돌리기 — R2 · R11a. ④ 놓는 순간 판정만(1판) · 마지막 가이드만(2판) · 다르면 늘 무변경(3판) — 못 본 가이드에 놓임 · 손이 움직인 사이 엉뚱한 결과 · PRD R22 어김. ⑤ 웹뷰 reparent — 슬롯 내용은 모델 데이터다(조사 §4). ⑥ 슬롯 하나 직접 끌기 — PRD §2. ⑦ 도킹 라이브러리 채택(dockview · FlexLayout · golden-layout · Lumino) — 자기 레이아웃 모델을 소유해 셸 권위(ADR-0035)와 겹친다(조사 §5). ⑧ 불투명 축소 그림 딱지 창(4.2) — Q5(Windows 기본 그림). ⑨ Esc = 취소 · Esc 없음 — R2 · Q6. ⑩ 비정상 끝 = 출처 탭바로(4.2) — Q8. ⑪ 메인 마지막 탭은 끌기 시작 안 됨(4.2 D3) — Q9.
- **ADR-B 크기 규칙 = 새것 1/3 하나 · 면적 0 만 거절**(사용자 결정 R7 · R8) — 거부한 대안: Unity 식(대상 1/3 · 최소 100px · 바깥 200px · 옆 칸 양보) · 반반 + 바깥 1/3 · 크기 기억(JetBrains · AvalonDock) · 퍼센트 + 기준 px · 200px 고정 띠 · 30%(조사 §8 · 사용자 확정 2026-10-15).
- **ADR-C 창 사이 끌기 수단 = 브라우저 내장 HTML5 끌어 놓기 · 셸은 영역 · 놓기 판정 · 적용**(사용자 결정 Q7 + 이 TRD 의 구현 선택 · 조사 `slot-docking-drag-mechanism-2026-10-16.md`)
  - 결정: 탭 머리는 `draggable` 이고 커서 추적 · 맨 위 창 · 끌기 그림 · 버튼 · Esc 를 엔진에 맡긴다. 모든 런타임 창은 wry 드롭 처리기를 끈다. 셸은 창별 영역을 미리 계산해 보내고, 웹뷰는 점-사각형 판정으로 가이드를 그리며, 놓기는 셸이 같은 규칙으로 다시 판정해 적용한다. 끝 신호 = 대상 `drop` · 출처 `dragend` · 끝 뒤 포인터 사건이고, `none` 끝은 마지막 보고 · 셸 커서로 「그 자리」를 정한다. OS 별 코드는 없다.
  - 거부한 대안: ① **셸 폴링 루프(4판 설계)** — 8ms 틱으로 OS 커서 · 버튼 · Esc · 전경 · 맨 위 창을 읽고 딱지 창을 옮기는 앱 주도 끌기. OS 함수 다섯과 `raw-window-handle` 의존(ADR-0266 규칙 개정) · 딱지 창 · 타이머 해상도 · 전경 유예가 필요했고, 같은 일을 엔진이 OS 끌기로 이미 한다(실측 — 메인 스레드 안 막힘). ② **crabnebula `tauri-plugin-drag-as-window` / `drag-rs`** — `DoDragDrop` 을 Tauri 메인 스레드에서 돌려 wry 의 위치 사건이 끄는 동안 막혔다 몰려온다(실시간 가이드 불가 · 열린 이슈 #77) · Esc = 취소 고정 · 끄는 동안 위치 없음 · 실제 임시 파일을 실음 · 그림 크기 조절 없음 · 끝나지 않아 앱이 멈추는 열린 버그 #94(조사 §1 — 소스 대조). ③ **네이티브 제목줄 끌기(창이 따라오는 이동 루프 — Chromium 탭 떼기 · 조사 §4 전제)** — 제목줄 = 이동만이라는 사용자 결정(R1)과 어긋나고 이동 루프 끝 감지에 창 서브클래스가 든다. ④ **포인터 직접 추적 + 클릭 통과 고스트 창**(block/buzz-app 등 — 조사 §2) — ① 과 같은 꼴이고 그 피어들의 「HTML5 는 창 밖으로 못 나간다」는 wry 처리기를 켠 채의 관찰이었다. ⑤ **DOM 끌기 라이브러리**(pragmatic-drag-and-drop · dnd-kit · react-dnd) — 문서 하나 안에서만 돈다. ⑥ **움직일 때마다 셸에 판정을 묻기** — invoke p99 ~16ms · 최대 ~50ms(실측)라 60Hz 가이드가 늦고 응답 순서를 따로 맞춰야 한다.
- **ADR-D 도킹의 LLM 표면을 다음 작업으로 미룬다 — CLAUDE.md 「LLM-우선 제어」의 의식적 예외**(사용자 결정 Q10)
  - 결정: 이 기능은 버스 · 프론트 레지스트리 명령을 만들지 않는다. 적용 서비스 `dock_tab` 과 타입드 오류 · `Keep` 정책 · 표식 판정을 남겨 다음 작업이 그 위에 명령만 얹게 한다. 다음 작업은 LLM 도구 설계 관례 조사로 시작한다.
  - 거부한 대안: ① **TRD 4판의 `tab.dock` 하나 + `mode` · 선택 칸** — 조합 실수 여지가 크다(사용자 기준: 흔한 요청은 1–2 호출 · 메모의 동사 셋 후보로 밀림). ② **CDP(원격 디버깅 포트)로 DOM 을 LLM 이 보기** — 포트에 인증이 없고(로컬 아무 프로세스나 앱 JS 실행) DOM 은 모델이 아닌 그림 결과라 운영 불가(메모 2026-10-16).
- **ADR-E 팝아웃 닫기 요청에서 모델 먼저 지우기 + 닫힌 label 묘비**(3판 결정 M2 · §11-1 #2 사용자 결정 2026-10-16)
  - 결정: 팝아웃의 `CloseRequested` 에서 레이아웃 락 한 번 안에 그 창의 모델 정리를 하고 label 에 묘비를 단다. 도킹 · 새 창 확정 · 팝아웃 phase C 는 없는 창 · 묘비 앞에서 무변경으로 멈춘다. 근거 = 팝아웃 닫기 요청은 되돌려지지 않는다. ADR-0057 불변식은 그대로이고 정리 계기만 옮긴다.
  - 거부한 대안: ① 열림 사전 확인 + 남는 위험 기록(2판) — 틈을 줄일 뿐 닫지 못한다. ② 닫는 중 표시만 하고 정리는 소멸 때 — 지금은 같은 결과에 칸만 늘린다(닫기를 막을 수 있게 되면 이 안으로). ③ 확정 뒤 `is_open` 으로 다시 보기 — 옮긴 탭을 되살리지 못한다. ④ 닫기가 먼저면 그 자리에 새 창 · 토스트 알림 — 사용자가 「흐름에 맡긴다」로 거절(2026-10-16).

## 부록 — 요구사항 추적

| PRD | 어디서 서나 |
|---|---|
| R1 · AC1 | 제목줄은 OS 이동 그대로 — 이 기능이 끼어들지 않는다 · S6 |
| R2 · AC2 · AC6 · AC20 | 떼어진 표시(§4-1 — 그리기만) · 탭 하나 팝아웃 숨기기(§1-3 · S1 ⑤) · 끌기 그림(§4-2 · S2) · 모델 무변경(§1-3) |
| R3 · R4 · R5 · R6 · AC2 · AC4 | 영역(띠 = 결과 사각형 · 바깥 띠 32px · 가운데 없음 — §2-4) · 그리기(§4-3) |
| R6a | §2-2 규칙 6 · 8 · 경계 포함 규칙 · 골든 |
| R6b · AC14 | 웹뷰 머묾 500ms → 기존 탭 전환(§1-3 · §1-4) · 전환 순간의 이음새 놓기 = 무변경 · 탭바 놓기는 적용(§1-3 「보이는 탭 경쟁」 · 「놓기 판정」 4b) |
| R7 · AC3 · AC4 · AC5 | `docked_tree` 분할 모양 표(§1-2) |
| R8 · AC15 | 새 면적 0(§1-2) · 빠진 띠(§2-4) |
| R9 | 출처 View 의 트리 통째 이동(§1-2) |
| R10 · AC3 · AC6 | 빈 팝아웃 항목 제거 + `host.close`(§1-2 · §5) |
| R11 · AC7 | 같은 락 `resync`(§5) · 닫히는 창 replay 가드(3 단계 — 보이는 창이 있으면 지나가고 seq 연속이 거른다 · §5) |
| R11a · AC11 | 끄는 동안 모델 무변경 · 락 한 번 · 메인 빈 탭도 같은 락 · 기록기 스냅숏(§1-2 · §5) |
| R11b · AC3 | `FocusPolicy::Reveal` + `set_focus`(§1-2 · §1-3) · 적용 중 끝 = `Applied` 하나(§1-3 「적용 중」) |
| R12 · AC19 | `dragend(none)` → 마지막 점에 놓기 · 창별 마지막 Over · 떠남 따로(§1-3) · Esc 삼키기(창별 상한 — §1-4) · S1 |
| R13b · AC16 · AC21 | 끝 신호 셋 · 마지막 점에 놓기(커서 못 읽음 = W 의 마지막 점) · 유실 대비 · 이른 끝 신호 붙들기 · 출처 사라짐 = 그냥 끝 · `Ended` 방송(§1-3 · §4-3) |
| R13c · AC16 | `blocked` · 영역 `rev` · 보인 것 ↔ 지금 판정 대조(대상 사라짐 = 무변경) · 보이던 탭(이음새만) · 탭바 몫 `bar_rev`(탭바) · `expect_source` · `expect_target`(이음새 = 배치 · 대상 창 활성 · 탭바 = 탭 순서 — §1-2 · §1-3) · 재사용 재확인(§1-2) |
| R14 · AC9 | `refill_main` · 메인 빈 화면(§1-2 · §4-1) |
| R15 · AC13 · AC14 | 엔진의 맨 위 창 · 창별 영역 · 보이는 탭 · 탭바(§1-4 · §2-4) |
| R16 · R17 | 다음 작업 — §3 · ADR-D |
| R18 | HTML5 끌어 놓기 + 셸 영역 · 판정 · 적용(§1 · ADR-C) · 다른 끌기 리스너(react-arborist) 피하기(§1-4 · S1 ⑦) |
| R19 | OS 함수 0 · Tauri API 셋(§1-6) |
| R21 · AC18 | `DockTo::TabBar`(§1-2) · 탭바 영역 · 자리(§2-2 규칙 3 · §2-4) · 탭바 몫 대조(§1-3 「놓기 판정」 4b) |
| R22 · AC8 · AC12 | `DockTo::NewWindow` · 세 phase · 묘비 · 재사용(옮긴 뒤 재확인 · 되돌리기) · 크기 · 자리 검증(§1-2) · 「보인 것 · 판정 모두 가이드 없음 = 새 창」 · 새 창 자리(§1-3) · S3 · S4 |
| AC22 | 파일 드롭 막기(§1-4) · S3 ② |
| W1 · W10 | S1(⑦ 트리 슬롯 · ⑧ 사건 순서 포함) |
| W4 | S4 |
| W5 | S6 |
| W7 · W8 | §4-2 · S2 · §11-1 #7 |
| W9 | §1-4 · S3 |
| W11 | §8-2 macOS |
| W12 | S5 |
