# TRD — 슬롯 도킹 (탭을 끌어 다른 창 레이아웃에 붙이기)

| 항목 | 값 |
|---|---|
| **상태** | 4판 — 3판 `/review trd full` 의 작은 FIX 여섯 반영(3판 = 2판 FIX 반영) · 재리뷰 전 · 사용자 확인 대상 5건(§11-1) · 스파이크 5건(§8 — S2 는 진행 중이고 결과는 메인이 접어 넣는다) |
| **날짜** | 2026-10-15 |
| **스텝** | S21 |
| **브랜치** | `v0.4.1/feat/slot-docking` |
| **PRD** | [prd.md](prd.md) 4.1판 — 요구 R1–R22(R12 = Esc 없음 · R20 없음) · 수용 AC1–AC16 · AC18 · 결정 Q1–Q4 · 기본값 D3–D12 · 위험 W1 · W3 · W4 · W5 · W7 |
| **근거** | [slot-docking-survey-2026-10-15.md](../../research/slot-docking-survey-2026-10-15.md) §4(기술 타당성) · §5(코드 실태) · §7–§9(관례 · 라이브러리) |
| **줄 포인터 기준** | HEAD `c1de4082` + 미커밋 PRD 4.1판. 심볼 이름이 정본이고 줄은 보조다. 의존 crate 줄은 `~/.cargo/registry` 의 잠긴 판(tauri 2.11.3 · tauri-runtime-wry 2.11.3 · tao 0.35.3 · windows 0.58) 소스 |

## 0. 결론 요약

- **세 층으로 가른다.** ① 레이아웃 연산(셸 `layout` — OS 무관 · 단독 시험) ② 끌기 세션(셸 신설 `dock` — 상태 기계는 순수, Tauri 는 구동 껍데기에만) ③ OS 층(`platform` — 목적 단위 함수: 「이 점의 맨 위 창이 후보 중 어느 것인가」 · 「포인터가 지금 어디이고 주 버튼이 눌려 있나(믿을 수 있을 때만)」).
- **탭을 끌면 그 자리에서 떼어진다 — 단 모델은 놓을 때 한 번만 바뀐다.** 끄는 동안 출처 창은 그 탭을 그리지만 않고(탭이 하나뿐인 팝아웃은 창째 안 보이게), 작은 딱지 창이 커서를 따라간다. 모델 · 저장은 끌기 전 그대로다(PRD R2 · R11a).
- **끌기는 웹뷰에서 시작하고 셸이 몬다.** 탭을 4px 넘게 끌면 웹뷰가 시작을 알리고, 그 뒤로는 셸의 끌기 스레드가 마감 기반 틱으로 전경 · 포인터를 읽어 판정 · 딱지 이동 · 가이드 지시 · 놓기를 한다. 웹뷰는 시작 · 떼기 힌트를 알리고 가이드를 그릴 뿐이다(R18). 네이티브 이동 루프 · 창 서브클래스는 쓰지 않는다.
- **놓으면 셋 중 하나 — 이음새 · 탭바는 「보인 것과 같을 때만」.** 이음새(R7) · 탭바(R21) · 그 밖(R22 새 창). 떼는 순간 그 점을 새 스냅숏으로 한 번 판정한다. 결과가 「가이드 없음」(슬롯 가운데 · 우리 창 밖 · 다른 앱 위)이면 무엇을 보냈든 **R22 새 창**이다(PRD R22 그대로). 결과가 이음새 · 탭바면 **마지막으로 보낸 가이드와 같을 때만** 적용하고, 다르면 무변경(`TargetChanged`)이다. 사용자가 못 본 가이드에 놓이는 일이 없다(§1-3 「놓기 판정」).
- **붙이기는 명령 하나, 적용 함수 하나.** 버스 `tab.dock`(모드 셋 — 이음새 · 탭바 · 새 창)과 마우스 놓기가 같은 `apply::dock_tab` 에 떨어진다(R16 · ADR-0081 결정 3). 마우스와 명령의 차이는 호출자 중립 사전조건(`DockGuard`)과 포커스 정책(`FocusPolicy`) 두 값뿐이다.
- **닫히는 창으로는 붙지 않는다.** 팝아웃의 닫기 요청(`CloseRequested`)을 받는 순간 레이아웃 락 안에서 그 창을 모델에서 먼저 지우고 label 에 묘비를 단다 — 그 뒤의 도킹 · 새 창 확정은 타입드 오류로 무변경이 된다. 팝아웃 `slot.popout` 의 같은 틈(`apply.rs:105-118`)도 함께 닫힌다(기본값 · 사용자 확인 대상 — §11-1 #2).
- **스파이크 다섯(§8)** 이 W1 · W3 · W4 · W5 · W7 을 잰다. 레이아웃 연산 · 명령 · 판정 · 세션(§10 1–6단계)은 스파이크와 무관해 먼저 한다. ★스파이크가 합격선을 못 넘으면 조용히 물러나지 않고 멈춘다 — PRD 개정 대상이다★.

## 1. 아키텍처 — 세 층

### 1-1. 층과 흐름

```
웹뷰(출처 창)   TabBar pointerdown → 4px 넘으면 invoke dock_drag_begin{view_id, grab} → {session}
                pointerup → dock_drag_end{session, Released} · pointercancel · blur → {session, Lost}(힌트)
웹뷰(모든 창)   report_dock_chrome{canvas_origin, tab_bar, tabs[]} — 평소 보고(측정 · version 불변)
                listen "dock:session"(떼어진 표시 — 그리기만) · "dock:overlay"(자기 label — 가이드 그리기만)
웹뷰(딱지 창)   dock-chip.html — listen "dock:chip"{title, theme, blocked} · 그리기만
셸 dock::driver ─ 끌기 스레드 하나(무제한 사건 채널 + 마감 기반 틱 · 끌기 중에만 산다)
     └ dock::session::step(상태, 사건, 스냅숏) → (상태', 할 일)   ← 순수 · 포트 없음
          전경      = platform::window::foreground() → Ours | Other | Indeterminate   (틱마다 먼저)
          포인터    = platform::input::pointer() → Read | Unavailable | Unsupported
          맨 위 창  = platform::window::topmost_at(점, 후보, 투과 = [딱지 창])
          판정      = dock::hit (순수) ← ViewManager 짧은 락 스냅숏 + 창 기하 캐시
          딱지      = 딱지 창 set_position(물리 px)
          그리기    = emit "dock:session"(seq) · emit_to(label, "dock:overlay") · emit_to(딱지, "dock:chip")
          놓기      = layout::apply::dock_tab(DockGuard{표식, 기대 배치}, FocusPolicy::Reveal)
셸 layout (권위 — ADR-0035) ─ dock.rs(순수 트리 연산) · ViewManager::{dock, plan_new_window, lease_tab, release_lease, mark_closed}
                             · apply::dock_tab · commands.rs(버스 tab.dock)
셸 lib.rs on_window_event ─ CloseRequested(팝아웃) → 모델에서 먼저 지움 + 묘비(§1-2 「닫히는 창」)
platform ─ window::{NativeWindow, native_window, topmost_at, Foreground, foreground} · input::{PointerRead, pointer} · (조건부) timer::fine_timer
```

| 무엇 | crate · 파일 | 비고 |
|---|---|---|
| 트리 연산 `docked_tree` · `DockSide` · `DockAnchor` · 새 면적 0 판정 · `incoming_rect` | src-tauri `src/layout/dock.rs`(신설) | `tree` · `geometry` 만 안다(Tauri · 락 0 — `tree.rs` 머리와 같은 격리) |
| `ViewManager::dock` · `plan_new_window` · `lease_tab` / `release_lease` · `mark_closed`(묘비) · 측정 `set_dock_chrome` | `src/layout/manager.rs` | 모델 변경은 전부 여기 |
| 적용 서비스 `dock_tab` | `src/layout/apply.rs` | 사람 · LLM · 마우스 놓기의 단일 경로. 머리의 「알려진 미확인」(`:105-118`)을 닫힌 것으로 고친다 |
| 버스 `tab.dock` | `src/layout/commands.rs` | `catalog_version` 17 → 18 · 머리 「적용 실패는 코드 하나로 나간다」 갱신 |
| 창 포트 `WindowHost::{open_at, move_window}` — ★기본 본문 없음★ | `src/layout/apply.rs`(trait) · `src/commands/popout.rs` `TauriWindowHost`(:70) · **`src/commands/layout.rs` `OwnedWindowHost`(:172-193 — 버스 경로의 포트 · 둘을 그대로 넘긴다)** · 시험 가짜 넷(`tests/layout_apply.rs:162` · `tests/layout_commands.rs:106` · `daemon_client/tests.rs:5350` · `:5749`) | §1-2 「적용 서비스」 |
| `placement::place` · `land` 공개(`pub(crate)`) | `src/state/placement.rs` | 새 창 자리 · 버스 x/y 검증 |
| invoke 껍데기 `dock_tab` · `dock_drag_begin` · `dock_drag_end` · `report_dock_chrome` | `src/commands/dock.rs`(신설) | `lib.rs` `generate_handler!` 등록 · 창을 만들 수 있는 둘(`dock_tab` · `dock_drag_begin`)은 `async fn`(창 빌드 교착 — `popout.rs:183-185` 선례) |
| 판정 `dock::hit` · 세션 `dock::session` · 구동 `dock::driver` · 딱지 창 `dock::chip` · 페이로드 타입(ts-rs) `dock::types` | `src/dock/`(신설) | 앞 둘은 순수. 타입은 `src-tauri/bindings/` 로 굽는다(CI 생성물 sync 게이트) |
| 창 사건 연결 | `src/lib.rs` `on_window_event`(:299–:338) | `CloseRequested`(팝아웃 → 모델 먼저 · §1-2) · `Destroyed` · `Focused` · `Moved` · `Resized` · `ScaleFactorChanged` → driver 채널. 딱지 창의 `Moved` · `Resized` 는 `placement::record` 앞에서 거른다 |
| 끄는 동안 출처 창 자리 기록 멈춤 | `src/state/placement.rs` `record`(:804) | 같은 함수의 `DeferredMaximize` 거름(:807–:815)과 같은 꼴 |
| OS 함수 | platform `src/window.rs` · `src/input.rs`(신설) · (조건부) `src/timer.rs` | §1-4 |
| 딱지 페이지 | `dock-chip.html` · `src/dockChip/main.ts`(신설 · React 없음) · `vite.config.ts` 입력 · `src-tauri/capabilities/dock-chip.json` | §4-2 |
| 프론트 | `TabBar.tsx` · `WindowLayout.tsx` · `DockOverlay.tsx`(신설) · `dockChromeReport.ts`(신설) · `commands/tabCommands.ts` · `store/viewStore.ts` | §1-6 |

`dock` 모듈을 `layout` 밑에 두지 않는 이유: 세션은 레이아웃 권위가 아니라 그 권위를 부르는 손님이다 — 놓기도 적용 서비스를 부를 뿐이다. 섞으면 「레이아웃 변경은 적용 서비스 하나」가 흐려진다.

### 1-2. ① 레이아웃 연산 (OS 무관 · 셸 `layout`)

#### 트리 연산 — `layout/dock.rs`

```rust
pub enum DockSide { Left, Right, Top, Bottom }      // serde rename 없음 — 버스와 같은 철자(§3-1)
pub enum DockAnchor { Slot(Uuid), Outer }           // Outer = 대상 탭 레이아웃 전체(루트)
pub const DOCK_NEW_SHARE: f64 = 1.0 / 3.0;          // R7 — 새것의 몫

/// 대상 트리에 끼운 **결과 트리**를 새로 만든다(제자리 변경 아님 — 가이드 · 판정 · 적용이 같은 함수를 쓴다).
pub fn docked_tree(target: &LayoutNode, anchor: DockAnchor, side: DockSide, incoming: &LayoutNode)
    -> Result<LayoutNode, DockTreeError>;
pub enum DockTreeError { SlotNotFound(Uuid), ZeroArea }
/// 결과 기하에서 끼운 서브트리가 차지하는 정규화 사각형(그 슬롯들의 외접 사각형) — 가이드의 「정확한 사각형」(R4).
pub fn incoming_rect(result: &LayoutGeometry, incoming_slots: &[Uuid]) -> Option<RectF64>;
```

- **분할 모양** — 기준 노드(슬롯 하나 또는 루트)를 새 분할 `Split{id: 새 UUID(ADR-0223), dir, ratio, a, b}` 로 감싼다.

  | side | dir(ADR-0140) | a | b | ratio(= a 몫) |
  |---|---|---|---|---|
  | Left | `LeftRight` | 새것 | 기준 | 1/3 |
  | Right | `LeftRight` | 기준 | 새것 | 2/3 |
  | Top | `TopBottom` | 새것 | 기준 | 1/3 |
  | Bottom | `TopBottom` | 기준 | 새것 | 2/3 |

  둘 다 `tree::RATIO_MIN`–`RATIO_MAX`(0.01–0.99 · `tree.rs:19-20`) 안이라 클램프에 걸리지 않는다. 기하는 정규화 좌표라 감싼 기준 노드 안쪽의 비율은 그대로 남는다 — 슬롯 옆은 그 슬롯만 2/3 로 줄고, 바깥 변은 기존 레이아웃 전체가 안쪽 비율을 지킨 채 2/3 로 준다(R7 · AC4 · AC5).
- **슬롯 id · 분할 id 는 바꾸지 않는다** — 끼우는 서브트리는 출처 탭의 노드를 그대로 옮긴다(이동 — 출처 View 는 같은 연산에서 지워진다). 그래서 R16 ① 이 돌려주는 「붙은 슬롯 id」 = 출처 탭의 슬롯 id 들이고(PRD 4.1), ADR-0223 의 「분할 id 는 노드를 따라간다」가 지켜진다.
- **새 면적 0 판정(R8)** — 결과 트리를 `geometry::compute` 로 재고, 결과에서 면적 0 인 칸 중 **원래 자리에서는 면적이 있던 칸**(대상 칸은 대상 기하, 끼운 칸은 출처 기하에서)이 하나라도 있으면 `ZeroArea`. 원래부터 면적 0 이던 칸은 빼고 본다 — `set_split_ratio` ⑤ 와 같은 태도다(`manager.rs:783-785`). 결과 트리 전체를 재므로 「깊은 자식 슬롯까지」(R8)가 따로 손대지 않아도 선다. 좌표는 정규화 레이아웃 좌표이고 화면 px 가 아니다.

#### ViewManager — `manager.rs`

```rust
pub enum DockTo {
    Seam { target: ViewId, anchor: DockAnchor, side: DockSide },   // R7
    TabBar { window: WindowLabel, index: Option<usize> },          // R21 — index = 결과 탭 목록의 자리 · None = 맨 뒤
    NewWindow { label: WindowLabel },                              // R22 — 부르는 쪽이 이미 연 OS 창 label(phase C)
}
/// 호출자 중립 사전조건 — 모델은 「누가」 불렀는지 모른다. 명령 = 전부 None.
pub struct DockGuard {
    pub lease: Option<LeaseId>,              // 이 요청이 쥔 표식(아래). 표식이 선 탭은 같은 표식을 쥔 요청만 옮긴다
    pub expect_source: Option<LayoutNode>,   // 출처 탭 배치가 이 값일 때만
    pub expect_target: Option<TargetExpect>, // 대상이 이 값일 때만
}
pub enum TargetExpect { Layout(LayoutNode) /* Seam — 대상 탭 배치 */, Tabs(Vec<ViewId>) /* TabBar — 대상 창 탭 목록 */ }
pub enum FocusPolicy { Keep /* 명령 — R11b · R16 */, Reveal /* 마우스 — 옮긴 것을 보이고 첫 슬롯에 포커스 */ }
pub struct Docked { window: WindowLabel, slot_ids: Vec<Uuid>, source_window: WindowLabel, source_window_closed: bool }
pub enum DockError {
    TabNotFound(Uuid), WindowNotFound(String), SlotNotFound(Uuid), SlotNotInTab { slot: Uuid, tab: Uuid },
    MainLastTab, SelfTarget, ZeroArea, Dragging,          // ← R16 의 실패 사유
    WindowClosed(String),                                 // ← NewWindow phase C 의 label 에 묘비(그 새 창이 닫힘) — NewWindow 만
    StaleLease, SourceChanged, TargetChanged,             // ← 사전조건을 단 요청만 — 명령 경로에서는 안 난다
    WindowOpen(String),                                   // ← 적용 서비스(새 창 열기 실패)
}
pub fn dock(&mut self, view: ViewId, to: DockTo, guard: &DockGuard, focus: FocusPolicy) -> Result<Docked, DockError>;
pub fn plan_new_window(&self, view: ViewId, guard: &DockGuard) -> Result<NewWindowPlan, DockError>; // R22 phase A
pub enum NewWindowPlan { Reuse { label: WindowLabel }, Create { size: Option<(f64, f64)> } }
pub fn lease_tab(&mut self, view: ViewId) -> Result<LeaseId, DockError>;  // 끌기 시작
pub fn release_lease(&mut self, id: LeaseId);                             // 끌기 끝 — 다른 표식이면 무동작
pub(crate) fn mark_closed(&mut self, label: &str);                        // 묘비 — 「닫히는 창」 절
```

**판정 순서가 계약이다**(앞에서 걸리면 뒤는 안 본다 · 걸리면 무변경 — `manager.rs:9` 의 「no-op + Err」):

1. 출처 View 가 있나 → `TabNotFound`.
2. 목적지 — Seam 의 대상 View 없음 = `TabNotFound` · TabBar 의 창 없음 = `WindowNotFound` · NewWindow 의 label 에 묘비가 있음 = `WindowClosed` · 그 label 이 이미 모델에 있음 = `WindowNotFound`(발급기가 단조라 실제로는 안 난다 — `attach_view_as_new_window` 와 같은 방어).
3. Seam 의 대상 = 출처 → `SelfTarget`. (TabBar 의 대상 창 = 출처 창은 자리 옮기기로 허용한다 — R21 「원래 창 탭바에 놓으면 그 창의 탭으로」.)
4. Seam 의 기준 슬롯이 대상 탭에 있나 — 다른 탭에 있으면 `SlotNotInTab`, 어디에도 없으면 `SlotNotFound`.
5. 결과가 main 의 마지막 탭을 main 에서 빼면 → `MainLastTab`(불변식 4 — `manager.rs:23-24`).
6. 표식 — 출처에 표식이 섰는데 `guard.lease` 가 없으면 `Dragging`(R13c · D9). `guard.lease` 가 있는데 지금 표식과 다르거나 표식이 없으면 `StaleLease`(지난 끌기의 늦은 놓기).
7. `expect_source` 와 다르면 `SourceChanged` · `expect_target` 과 다르면 `TargetChanged`(R13c).
8. Seam 이면 `docked_tree` → `ZeroArea`.

적용(전부 `&mut self` 안 — 락은 부르는 쪽 한 번 · `bump_version` 한 번):

- **Seam** — 대상 View 의 `layout` 을 결과 트리로 바꾼다. **모르는 내용 곁표**(`unknown_content` · `manager.rs:294-297`)의 출처 View 항목을 슬롯 id 째 대상 View 키로 옮긴다 — 안 옮기면 `remove_view`(`:600-604`)가 원문을 같이 지워 새 판이 쓴 슬롯이 다음 저장에서 사라진다(`prepare_detached_view` 가 원문을 따라 옮기는 것과 같은 이유 — `:973-994`). 출처 탭을 출처 창 탭 목록에서 빼고(아래 도우미) `remove_view(출처)`.
- **TabBar** — 출처 창 탭 목록에서 빼고(도우미) 대상 창 목록의 `min(index, 길이)` 자리에 넣고 `view_owner` 를 바꾼다. View 를 통째 옮기므로 id · 곁표가 그대로 따라간다(곁표는 뷰 키 — `manager.rs:291-296` 주석). 같은 창이면 자리만 바뀐다 — 같은 자리면 무변경 · `version` 불변 · `Ok`.
- **NewWindow** — 출처 창에서 빼고(도우미) `WindowTabs::new_popout(view)` 로 새 창 항목을 만든다(`attach_view_as_new_window` 와 같은 꼴 · `:1022-1040`).
- **탭 목록에서 빼기 도우미** — 활성 승계는 `close_tab` 과 같은 규칙(오른쪽 우선 · 없으면 왼쪽 — `:688-692`). 팝아웃이 비면 창 항목을 지우고 `source_window_closed = true`(R10 · ADR-0057 D-3). `close_tab` 과 나눠 쓰도록 뽑는다 — 기존 `close_*` 시험(`manager.rs:1498-1631`)이 리팩터를 지킨다. 승계 계산은 순수 함수(`succession(tabs, removed) -> Option<ViewId>`)로도 내보낸다 — 끄는 동안의 「떼어진 표시」(§4-1)가 같은 규칙을 쓴다.
- **보이는 탭(R16 · 2판 리뷰 D1/A9)** — 대상 창의 활성 탭은 손대지 않는다. ★예외 하나: 출처가 대상 창의 활성 탭이었으면(같은 창 안 도킹) 출처가 사라져 「보이는 탭 그대로」가 성립할 수 없다 — 그때는 대상이 보이는 탭이 된다★(Seam = 대상 탭 · TabBar = 옮긴 그 탭이 그대로 활성). 사용자가 보던 내용이 그 탭에 있으므로 보던 것이 화면에 남는다(사용자 확인 대상 — §11-1 #1). `Reveal` 이면 늘 대상(Seam = 대상 탭 · TabBar · NewWindow = 옮긴 탭)을 활성으로 한다.
- **포커스** — `Reveal` 이면 Seam 은 대상 View 의 `focused_slot_id` = 끼운 첫 슬롯, TabBar · NewWindow 는 그 탭의 포커스 그대로(R11b). `Keep` 이면 손대지 않는다 — 기준 슬롯 id 는 감싼 뒤에도 그대로라 기존 포커스가 유효하다.
- **표식** — `guard.lease` 가 지금 표식이면 내린다.

`plan_new_window`(R22 phase A) — 판정 1 · 5 · 6 · 7(출처만)을 본 뒤, 출처가 **탭이 그것 하나뿐인 팝아웃**이면 `Reuse{그 창}`(새 창을 만들지 않고 그 창을 옮긴다 — §7 #8), 아니면 `Create{size}` — size = 출처가 팝아웃이면 그 창 `attrs.bounds` 의 논리 크기(보통 자리 — 최대화 중이어도 · §11-1 #4), main 이면 `None`(창 포트의 팝아웃 기본 크기 — `popout.rs:143` 720×500)(R22 · D12).

#### 표식(lease) — 「이 탭은 지금 끄는 중」

`ViewManager.lease: Option<(LeaseId, ViewId)>` — 한 번에 하나(끌기는 하나씩이다). `lease_tab` 은 판정 1 · main 의 마지막 탭(= `MainLastTab` — R14) · 「이미 표식이 있음」(= `Dragging`)을 보고 세운다. 모델 안에 두는 이유: 「끄는 중인 탭을 옮기는 명령은 실패」(R13c)의 판정이 그 명령의 변경과 **같은 락 안에서** 서야 한다 — 따로 두면 판정과 적용 사이에 끌기가 시작될 수 있다. 표식 번호 = 끌기 세션 번호다 — 모델은 그 번호의 뜻(마우스 · 세션)을 모르고 같은지만 본다. 메모리에만 있고 저장하지 않는다.

#### 닫히는 창 — 모델 먼저 · 묘비 (2판 리뷰 D3/A5 · 3판 결정 M2)

- **(a) 닫기 요청 = 모델에서 먼저 지움.** `on_window_event` 의 `CloseRequested`(`lib.rs:301-307`)에서 label 이 팝아웃이면 레이아웃 락 한 번 안에서 지금 `Destroyed` 정리가 하는 모델 몫(`popout.rs:256-283` `drop_window_in_model` — 창의 탭 View 전부 · 창 항목 · 라우팅 재계산 · 구독 해제 · 사용량 관심)을 하고 `mark_closed(label)` 로 묘비를 단다. 뒤이은 `Destroyed` 는 지금처럼 같은 정리를 부르지만 모델에 그 창이 이미 없어 라우팅 재계산 · Channel 해제만 남는다(`popout.rs:219-250` · 멱등). `Destroyed` 도 `mark_closed` 를 부른다 — 닫기 요청 없이 사라진 창(`destroy()` 경로)까지 묘비가 선다.
- **그래도 되는 근거 — 팝아웃의 닫기 요청은 되돌려지지 않는다(확실 · 소스)** — 우리 처리기는 팝아웃에 `prevent_close` 를 부르지 않고(main 만 — `lib.rs:303-306`), 프론트에 `onCloseRequested` 리스너가 없으며(저장소 rg 0건), 런타임은 우리 콜백이 돌아오자마자 같은 호출 안에서 창을 닫는다(`tauri-runtime-wry-2.11.3/src/lib.rs:4438-4466` `on_close_requested` → `on_window_close`). 그래서 닫기 요청 = 닫힘 확정이다. ★누가 팝아웃 닫기를 막을 수 있게 만들면 이 절은 「지우기」 대신 「닫는 중 표시 + 막히면 표시 해제」로 바꿔야 한다★ — 프론트에 `onCloseRequested` · `tauri://close-requested` 리스너가 하나라도 생기면 Tauri 는 닫기를 웹뷰의 판단에 맡겨 되돌릴 수 있게 되고, 모델에서 지운 창이 살아남아 M2 의 틈이 다시 열린다. 그래서 (i) 그 처리기 자리에 이 전제와 점검 명령 두 줄을 주석으로 박고 (ii) 시험 계획의 게이트로 그 명령을 돌린다(§9 「게이트」 — `rg "onCloseRequested|close-requested" src/` → 0줄 · `rg "prevent_close" src-tauri/src/` → main 갈래 1줄).
- **(b) 새 창 경로는 묘비 앞에서 멈춘다.** phase C 의 판정 2 가 묘비를 보고 `WindowClosed` 를 내 무변경 — 탭은 출처에 남고 적용 서비스는 그 창을 `host.close`(이미 없으면 무동작)한다. 같은 검사를 `attach_view_as_new_window` 에도 둔다(아래).
- **(c) 정리가 묘비 뒤에 붙은 탭을 지우는 일은 없다(락 순서로 증명).** 묘비와 모델 삭제는 같은 락 한 번 안이다. 그 락보다 **뒤**의 확정은 대상이 없거나(Seam = 대상 View 가 지워져 `TabNotFound` · TabBar = `WindowNotFound`) 묘비에 걸려(`WindowClosed`) 무변경이다. 그 락보다 **앞**의 확정은 닫기 시점에 그 창의 탭이므로 그 닫기와 함께 닫힌다 — 창 닫기의 기존 규칙(ADR-0057 D-3)이고, 그 뒤의 `Destroyed` 정리는 모델에서 지울 것이 없다. 남는 것은 「사용자가 X 를 누른 직후 몇 ms 안에 명령 도킹이 먼저 확정된 경우 그 탭도 함께 닫힌다」뿐이고, 그건 닫기 의미 그대로라 잔해가 남지 않는다.
- **팝아웃 `slot.popout` 의 틈도 닫힌다** — `apply.rs:105-118` 의 두 갈래: 기존 창 타깃은 phase C 의 `insert_tab_into` 가 이미 `windows.contains_key` 를 다시 보므로(`manager.rs:1003-1008`) 모델 먼저 지우기만으로 롤백 갈래를 탄다. 새 창 타깃은 `attach_view_as_new_window` 에 묘비 검사를 더하면 phase C 가 실패해 기존 롤백(`drop_detached_view` + `host.close`)을 탄다. 같은 단계에서 그 주석을 「닫혔다」로 고친다.
- **ADR-0057 불변식(① 양방향 · ② 유니크 소유 · ③ 활성 소속 · ④ main ≥ 1)은 그대로다** — 바뀌는 것은 팝아웃 모델 정리의 계기(소멸 → 닫기 요청)뿐이고, 그 계기는 ADR 이 아니라 TRD B-tabs §5-2 · G1 의 구현 계약이다. 그래도 기존 닫기 경로의 순서를 바꾸므로 ADR 로 박는다(§12 ADR-E · 사용자 확인 뒤).
- 묘비 `closed_labels: HashSet<WindowLabel>` 는 메모리만이다. 팝아웃 label 은 한 부팅 안에서 다시 쓰이지 않으므로(`PopupCounter` — `popout.rs:37-45`) 늘기만 하고 한 부팅의 닫힌 창 수만큼이다. `adopt_restored` 의 구조 분해(`:1056-1063` — `..` 가 없어 칸이 늘면 컴파일이 깨진다)에 `lease: _` · `closed_labels: _` 를 더한다 — 복원 수락이 출처 View 를 지우면 끌기는 다음 틱에 「출처 없음」으로 끝난다(§1-3).

#### 적용 서비스 — `apply.rs`

```rust
pub struct DockRequest { pub view: ViewId, pub to: DockDest, pub guard: DockGuard, pub focus: FocusPolicy }
pub enum DockDest {
    Seam { target: ViewId, anchor: DockAnchor, side: DockSide },
    TabBar { window: WindowLabel, index: Option<usize> },
    NewWindow { at: WindowPlace },
}
pub struct WindowPlace { pub pos: Option<(i32, i32)> /* 물리 바깥 왼쪽 위 */, pub size: Option<(f64, f64)> /* 논리 안쪽 */ }
pub struct DockOutcome { pub window: WindowLabel, pub slot_ids: Vec<Uuid>, pub closed_window: Option<WindowLabel>, pub reused: bool }
pub fn dock_tab(state, subs, events, host, labels, req: DockRequest) -> Result<DockOutcome, DockError>;
```

- **Seam · TabBar = 락 한 번.** `mgr.dock(..)?` → 대상 View · 대상 창 · (남았으면) 출처 창 스냅숏 → `subs.resync(&mgr)` → 락 드롭 → `notify` → 출처 창이 비었으면 `host.close(label)`(OS 창 destroy — 락 밖 · `apply.rs` 머리 「락 규율」). 닫히는 대상 창은 모델 먼저 지우기와 묘비로 막히므로(위 절) `is_open` 사전 확인을 두지 않는다 — 모델이 하나뿐인 진실이다(M2 를 거절하면 되살린다 — §11-1 #2).
- **NewWindow = 세 phase**(팝아웃 `move_slot_to_window` · `apply.rs:665` 를 따른다):
  - A(락) `plan_new_window`. `Reuse` 면 락 밖에서 `host.move_window(label, &at)`(자리 · 크기를 준 경우) + `host.record_placement(label)` 만 하고 끝 — 모델 변경 0 · `reused = true`.
  - B(락 밖) `labels.next_label()` → `host.open_at(label, &at_with_size, activate)` — `activate = focus == Reveal`(명령은 창 활성을 바꾸지 않는다 — R11b). 실패면 `WindowOpen` · 모델 무변경.
  - C(락) `mgr.dock(view, NewWindow{label}, ..)` 로 **다시 판정하고** 적용 — B 와 C 사이에 그 새 창이 닫혔으면 묘비로 `WindowClosed`(M2 (b)). 실패면 락 드롭 → `host.close(label)` → 그 오류(탭은 출처에 남는다). 성공이면 `resync` → 락 드롭 → `notify` → `host.record_placement(label)`.
- **창 포트 확장 — 기본 본문 없음(3판 리뷰 F2)**:

  ```rust
  fn open_at(&self, label: &str, at: &WindowPlace, activate: bool) -> Result<(), String>;
  fn move_window(&self, label: &str, at: &WindowPlace);
  ```

  기본 본문을 두면 버스 경로의 `OwnedWindowHost`(`commands/layout.rs:172-193`)가 그것을 물려받아 자리 · 크기 · 활성을 말없이 버린다. 본문 없이 두어 구현자 여섯(§1-1 표)이 전부 컴파일에서 걸리게 한다 — `OwnedWindowHost` 는 `TauriWindowHost` 로 그대로 넘기고, 시험 가짜는 받은 인자를 적는다. Tauri 구현 — `open_at` = `build_runtime_window` 에 첫 자리(`placement` 의 `first_place` 꼴)와 `focused(activate)`(tao 는 처음 보일 때 `SW_SHOWNOACTIVATE` — `window_state.rs:326-333`)를 넘기고, 만든 뒤 `placement::place`(위치 먼저 → 크기 · `placement.rs:470-485`)로 정확히 놓는다. 크기를 안 주면 팝아웃 기본 크기. `move_window` = 같은 `place`. 웹뷰 환경은 `build_window` 가 `WebviewEnv::finish` 로 붙인다(`popout.rs:135-157` · ADR-0283).
- **자리 검증(3판 리뷰 F10)** — `pos` 가 있으면 `placement::land(label, bounds, monitors, Fallback::DefaultPlace)`(`placement.rs:446`)로 어느 모니터에 걸치는지 본다. 안 걸치면 그 자리를 버리고 기본 자리(label 의 계단 자리)로 연다 — 저장된 자리를 복원할 때와 같은 규칙 · 같은 info 로그. 버스가 화면 밖 좌표를 줘도 보이지 않는 창이 생기지 않는다. 마우스 놓기의 자리(딱지 왼쪽 위)는 늘 어느 모니터 위다.
- **오류** — `DockError` 는 `Display` 를 갖는다. invoke 껍데기는 문구를 `String` 으로 넘기고(기존 레이아웃 invoke 와 같은 꼴), 버스는 종류로 코드를 고른다(§3-2). 레이아웃 적용 서비스가 타입드 오류를 내는 첫 자리다 — 기존 쓰기들의 `String` 은 손대지 않는다.

### 1-3. ② 끌기 세션 (셸 `dock`)

#### 시작 — `dock_drag_begin`(async invoke · 3판 리뷰 F1)

★실패할 수 있는 준비를 전부 먼저 하고, 표식 · 숨기기 같은 부수 효과는 그 뒤에 한다. 부수 효과가 하나라도 생긴 뒤의 실패는 되감기 가드가 거둔다★.

1. **값싼 거절(부수 효과 0)** — 부른 창(Tauri 가 넣어 준 창)이 그 탭의 주인인가(읽기 락) · 끌기가 이미 있나 · main 의 마지막 탭인가.
2. **준비(부수 효과 0 — 실패하면 그냥 돌아간다)** — 딱지 창 확보(없으면 숨긴 채 만든다 — §4-2) · 후보 창마다 `NativeWindow` 와 창 기하(안쪽 원점 · 배율 · 보임 · 최소화). 출처 창 것을 못 읽으면 실패, 그 밖의 창 것을 못 읽으면 그 창을 후보에서 빼고 debug · 끌기 스레드를 띄워 「시작」 신호를 기다리게 한다(신호 없이 보내는 쪽이 떨어지면 그대로 끝난다).
3. **확정(락 한 번)** — `lease_tab(view)` · 출처 배치 복제(`expect_source`) · 출처 창 label · 탭 수 · 탭 이름 · 떼어진 표시용 승계 탭(출처가 활성이면 `succession` · 아니면 없음). ★여기서부터 되감기 가드가 선다★ — `BeginGuard` 가 해제되지 않은 채 떨어지면: 숨긴 출처 창 되보이기 · 자리 기록 재개 · `dock:session Ended` · `release_lease` · 스레드에 「중단」.
4. **부수 효과** — 출처가 탭 하나뿐인 팝아웃이면 그 창을 안 보이게(방법 = 스파이크 S1 · §4-1) + 그 label 의 자리 기록 멈춤 · `dock:session Started` 방송 · 딱지 `dock:chip` + 보이기.
5. 스레드에 「시작」 → 가드 해제 → `{session}` 반환.

| 거절 · 실패 | 언제 | 부수 효과 |
|---|---|---|
| `TabNotFound` | 1 · 3 — 부른 창이 그 탭의 주인이 아님 · 탭 없음 | 없음 |
| `AlreadyDragging` | 1 · 3(`lease_tab` 의 `Dragging`) | 없음 |
| `MainLastTab` | 1 · 3(R14) | 없음 |
| `ChipUnavailable(e)` | 2 딱지 창 만들기 · 4 보이기 실패 | 2 = 없음 · 4 = 가드가 거둠 |
| `SourceUnavailable(e)` | 2 출처 창 핸들 · 기하를 못 읽음(Windows 에서 `native_window` 가 `None` 포함) | 없음 |
| `ThreadSpawn(e)` | 2 | 없음 |
| `HideFailed(e)` | 4 출처 창 숨기기 실패 | 가드가 거둠 |
| `LayoutPoisoned` | 1 · 3 | 없음 |

웹뷰는 어느 거절이든 누름 상태를 버리고 커서를 되돌린다(`MainLastTab` 만 「안 됨」 커서). 셸 쪽 로그는 §1-7.

#### 스레드 · 틱 — 끌기 스레드 하나가 단일 소비자

- **사건 채널 = 무제한**(`std::sync::mpsc::channel`). 보내는 쪽(invoke `dock_drag_end` · `on_window_event`)은 `send` 만 하고 돌아간다 — **어떤 사건도 버리지 않는다**. 제어 사건(`End` · `Destroyed` · `Focused`)은 도착 순서대로 처리하고, 기하 사건(`Moved` · `Resized` · `ScaleFactorChanged`)은 루프가 한 번에 비우며 label 집합으로 합쳐(coalesce) 다음 틱 앞에 그 창들만 다시 읽는다. 딱지 창의 사건은 `on_window_event` 에서 보내지 않는다(그 창은 우리가 매 틱 옮긴다).
- **마감 기반 틱** — `next = 시작 + TICK(8ms)`. 루프는 `recv_timeout(next.saturating_duration_since(now))` 로 기다리고, 사건이 오면 처리한 뒤 같은 마감으로 다시 기다린다(사건이 틱을 밀지 않는다). 시간이 되면 틱 한 번 · `next += TICK` · 한 틱 넘게 밀렸으면 `next = now + TICK`(몰아서 따라잡지 않는다). 마감 계산은 순수 함수로 떼어 시험한다.
- **★실제 틱 주기는 OS 타이머 해상도에 묶인다(3판 리뷰 F6)★** — Windows 기본 타이머 해상도는 15.6ms 라 `recv_timeout(8ms)` 가 그보다 일찍 깨지 않을 수 있다(가능성 높음 — 그 프로세스에서 누가 해상도를 올려 두었는지에 달렸다). 스파이크 S2 · S4 가 실제 주기를 적는다. 딱지 지연 합격선(S4)을 그 주기로 못 넘으면 **요구를 바꾸지 않는 조정**으로 platform 의 목적 함수 `timer::fine_timer() -> FineTimer`(「이 값이 사는 동안 이 프로세스의 짧은 대기가 촘촘히 깬다」 · Windows = `timeBeginPeriod(1)` / Drop = `timeEndPeriod(1)` — `windows` feature `Win32_Media` · 그 밖 = 무동작)를 들여 끌기 동안만 쥔다. 지금은 넣지 않는다.
- **락 규율** — 끌기 스레드만 Tauri 게터 · 세터를 부르고, 그동안 **어떤 락도 쥐지 않는다**. 메인 스레드(창 사건 처리기)는 채널에 넣기만 하므로 교착 고리가 없다(`apply.rs` 머리 「락 규율」의 `Destroyed → cleanup → state.0.lock()` 교착을 피한다). 끌기 스레드는 메인 스레드가 아니므로 놓기의 새 창 만들기(phase B)를 그 스레드에서 불러도 된다.

#### 한 틱의 순서 (3판 리뷰 F7)

1. **전경** — `platform::window::foreground()`(§1-4 — 세 갈래):
   - `Some(Foreground::Other)` — 전경 창이 **확실히 다른 프로세스의 것** → 곧바로 **비정상 끝**(R13b 「앱이 포커스를 잃음」).
   - `Some(Foreground::Indeterminate)` — 전경 창이 없거나 읽는 사이 사라짐. 출처 팝아웃을 숨기거나 딱지를 보이는 순간처럼 활성이 옮겨 가는 틈에 잠깐 생긴다(3판 리뷰 N2) → `Unavailable` 과 **같은 유예**로 친다(아래 2). 보안 데스크톱(UAC · 잠금 화면)도 여기로 와 유예가 지나면 비정상 끝이 된다.
   - `Some(Foreground::Ours)` → 계속 · `None`(이 OS 에 수단 없음) → 건너뛴다.
2. **포인터** — `pointer()`:
   - `Read{at, primary_down}` — 믿을 수 있는 읽기다(§1-4 — 전경이 이 프로세스일 때만 `Read` 를 준다).
   - `Unavailable` — 지금은 못 읽음(일시 실패 · 다른 데스크톱 · 전경 불명). **떼기로 치지 않는다.** 1 의 `Indeterminate` 와 한 시계(`unavailable_since`)를 쓰고, 둘 중 어느 것이든 연속 `INPUT_LOST_MS`(500ms)를 넘으면 **비정상 끝**. `Read` 와 `Ours` 가 함께 나온 틱에 시계를 비운다.
   - `Unsupported` — 이 OS 에 수단이 없다. 커서 = Tauri `cursor_position`, 떼기는 웹뷰 `Released` 에만 기대고, 웹뷰 `Lost` 는 비정상 끝이다.
3. `Read{primary_down: false}` → **놓기**(아래 「놓기 판정」). ★`GetAsyncKeyState` 는 다른 데스크톱이거나 전경이 다른 프로세스면 0 을 준다★ — 1 · 2 의 순서와 `Read` 의 조건이 그 0 을 「뗐다」로 읽지 않게 한다.
4. `Read{primary_down: true}` → 딱지 이동(커서가 움직였을 때만) → `topmost_at` → ViewManager 짧은 락(판정 재료 복사: `version` · 맨 위 창의 보이는 탭 · 탭 목록 · 캔버스 · 크롬 · 그 탭 배치(`version` 이 올랐을 때만 복제) · 출처 View 유무와 배치) → 락 드롭 → `session::step` → 할 일 실행(emit · `switch_tab`) → 머물기(R6b).

#### 상태 기계 — `session.rs` (순수)

```
Session { id, seq, source_label, source_view, title, expect_source, source_hidden: bool, source_stale: bool,
          display_active: Option<Uuid>, shown: Shown, dwell: Option<(label, Uuid, Instant)>, last_pointer, unavailable_since }
Shown   = Unset | Shown { target: DropTarget, expect: Option<TargetExpect>, overlays: 창별 마지막 보낸 값 }
DropTarget = Seam { window, view, anchor, side } | TabBar { window, index } | NoGuide
```

| 사건 | 처리 |
|---|---|
| 틱 1 의 `Other` | **비정상 끝**(곧바로) |
| 틱 1 의 `Indeterminate` · 틱 2 의 `Unavailable` 이 합쳐 500ms 초과 | **비정상 끝** |
| 틱 3(`Read` 에서 뗌) · `End{Released}` | **놓기** |
| 틱 4 | 딱지 이동 · 판정 → 가이드 · 머물기 |
| `End{Lost}`(웹뷰 `pointercancel` · `blur`) | `pointer()` 가 `Unsupported` 가 아니면 무시(힌트) · `Unsupported` 면 **비정상 끝** |
| `Destroyed(출처 창)` · 스냅숏에 출처 View 없음 | **중단** — 무변경(탭은 그 닫기와 함께 사라졌다 · R13b 끝 문장 · R13c) |
| `Destroyed(그 밖)` | 캐시에서 빼고 그 창의 가이드를 지움 |
| 스냅숏의 출처 배치 ≠ `expect_source` | `source_stale = true` — 그 뒤로 띠 · 탭바 표시를 그리지 않고 딱지에 「안 됨」(놓아도 무변경임을 미리 보인다) |
| 출처 창 탭 목록이 바뀜 | `display_active` 를 다시 계산해 `dock:session Display{seq}` 로 보냄 |

#### 놓기 판정 — 가이드 없음이면 새 창 · 이음새 · 탭바는 보인 것과 같을 때만 (3판 결정 M1 · 4판 정정 · 2판 리뷰 A1 · 3판 리뷰 F4)

- **판정 재료 = 떼는 순간 새로 뜬 스냅숏(3판 리뷰 N4)** — 마지막 틱의 재료를 다시 쓰지 않는다. 떼는 점(`Read` 의 `at` — `Unsupported` 면 Tauri 커서)으로 `topmost_at` 를 새로 부르고, ViewManager 짧은 락으로 판정 재료(보이는 탭 · 탭 목록 · 캔버스 · 크롬 · 그 탭 배치)를 새로 복사한다. 창 기하(안쪽 원점 · 배율)는 캐시를 쓴다 — 기하 사건이 오면 루프가 그 창을 다시 읽어 두므로 처리한 사건까지는 최신이다. 그 재료로 **한 번** 판정한다(§2 — 틱과 같은 함수).
- **`source_stale` 이면 판정 없이 무변경 — 아래 두 갈래보다 먼저 본다**(4판 리뷰 약한 지적). 새 창 요청도 `DockGuard{ lease, expect_source }` 를 싣고 가므로 모델 판정 7(`SourceChanged`)이 한 번 더 막는다.
- **지금 결과가 「가이드 없음」이면 → R22 새 창**(무엇을 보냈든 · `Unset` 이어도). 슬롯 가운데 · 면적 0 이라 칠하지 않은 띠 · 우리 창 밖 · 다른 앱 위 · 바탕화면이 여기다(PRD R6 · R8 · R22). `NewWindow{ at: 딱지 왼쪽 위 }`(크기는 적용 서비스가 R22 규칙으로 채운다 · 출처 사전조건 `lease` · `expect_source` 는 싣고 대상 사전조건은 없음).
- **지금 결과가 이음새 · 탭바면 `shown.target`(마지막으로 보낸 오버레이가 보인 것)과 대조한다.** 같음 = 같은 창 + 같은 띠(대상 View · 기준 · 변) 또는 같은 탭바 자리(창 · 자리).
  - **같으면 적용** — `apply::dock_tab(DockGuard{ lease, expect_source, expect_target: shown.expect }, Reveal)`. 가이드를 보낸 뒤 대상 배치가 바뀌었으면 모델 판정 7 이 `TargetChanged` 로 무변경을 낸다(R13c).
  - **다르면(`Unset` 포함) 무변경** — 이유 = `TargetChanged` 로 로그 · 탭은 출처 탭바에 다시 보인다. 사용자가 못 본 이음새 · 탭바에 놓이지 않는다.
- **적용 결말** — 성공: 대상(또는 새 · 재사용) 창을 `set_focus()`(R11b — 첫 슬롯 포커스는 적용 서비스가 같은 락에서 끝냈다) · 재사용이면 그 창을 다시 보인다. 실패(사유 무관): **무변경**.
- **무변경 · 비정상 끝 = 같은 일** — 모델은 끌기 전 그대로이므로 R13b 의 「출처 창 탭바의 원래 자리로 넣기」는 모델로는 아무 일도 아니다. 떼어진 표시를 거두고 숨긴 출처 창을 원래대로 보인다.

#### 끝 (모든 갈래 공통)

- 순서: (성공이면 적용 서비스의 알림이 이미 나갔다) → `dock:session Ended{seq}` 방송 → 숨긴 출처 창 되보이기(무변경 · 비정상 끝) → 딱지 숨김 → 자리 기록 재개(재사용 · 되보이기면 `placement::record` 한 번) → `release_lease` → 끝 로그(§1-7) → 스레드 끝. `Ended` 를 적용 알림 **뒤에** 내므로 출처 창은 탭이 빠진 목록을 먼저 받고 떼어진 표시를 거둔다 — 탭이 한 번 다시 비쳤다 사라지지 않는다(같은 스레드의 emit 순서 — 가능성 높음 · QA 몫).
- **늦은 신호** — `dock_drag_end` 의 세션 번호가 지금 세션이 아니면 버린다. 끝맺기에 들어간 뒤 온 사건도 버린다(성공한 놓기가 닫은 출처 팝아웃의 `Destroyed` 포함 — 끝난 스레드의 채널로는 보내기가 실패하고 보내는 쪽은 그 실패를 무시한다). 놓기는 표식 번호로 모델에 다시 대조된다(판정 6 — `StaleLease`).
- **`dock:session` 순서(3판 리뷰 F10)** — 모든 `dock:session` 사건은 세션 안에서 단조 증가하는 `seq` 를 싣는다. 웹뷰는 `(session, seq)` 가 마지막으로 반영한 것보다 오래되면 버린다 — 늦게 도착한 옛 `Display` 가 새 표시를 덮지 못한다.

#### 그 밖

- **R6b 탭 전환** — 판정이 「보이지 않는 탭의 머리」를 내놓은 채 `HOVER_SWITCH_MS`(500ms) 머물면 적용 서비스 `switch_tab`(사람 탭 클릭과 같은 경로 — `apply.rs:258`)을 부르고 대기를 비운다. ★끄는 동안 모델을 바꾸는 유일한 동작이다 — 도킹의 변경이 아니라 PRD R6b 가 정한 탭 전환이고, 무변경 · 비정상 끝이어도 되돌리지 않는다(§11-1 #5)★. 탭바 오버레이가 대기 중인 탭에 진행 표시를 한다.
- **R14** — `lease_tab` 이 main 의 마지막 탭을 거절한다. 웹뷰도 같은 조건을 미리 보고 시작 요청 없이 커서를 「안 됨」으로 바꾼다(즉시 반응용 · 판정 권위는 셸).

### 1-4. ③ OS 층 (`platform`) — 목적 단위 함수만

```rust
// crates/engram-dashboard-platform/src/window.rs (기존 모듈에 더한다)
/// 최상위 창 하나. Windows = HWND 값을 isize 로 — `Copy · Send · Sync`(windows 0.58 의 `HWND` 는 포인터라 Send 가 아니다).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeWindow(isize);
pub fn native_window(w: &impl raw_window_handle::HasWindowHandle) -> Option<NativeWindow>;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenPoint { pub x: i32, pub y: i32 }            // 물리 px · 가상 화면 좌표
pub enum TopmostAt { Candidate(usize), Other, Unknown }
/// `at` 에서 `see_through` 를 투과해 본 맨 위의 보이는 최상위 창이 `candidates` 중 몇 번째인가.
/// Other = 다른 무엇(다른 앱 창 · 바탕화면 · 작업 표시줄)이 위에 있다 · Unknown = 이 OS 에서 답할 수단이 없다.
pub fn topmost_at(at: ScreenPoint, candidates: &[NativeWindow], see_through: &[NativeWindow]) -> TopmostAt;
/// 지금 전경 창이 누구의 것인가 — 세 갈래(3판 리뷰 N2). None = 이 OS 에서 답할 수단이 없다.
pub enum Foreground { Ours, Other, Indeterminate /* 전경 창 없음 · 읽는 사이 사라짐 */ }
pub fn foreground() -> Option<Foreground>;
// 기존 foreground_is_current_process() 는 이 함수 위의 얇은 감싸기로 바꾸고 답은 그대로 둔다:
//   Ours → Some(true) · Other | Indeterminate → Some(false) · None → None(유일한 호출자 = 셸 `placement.rs:729` 의 포커스 판정).

// crates/engram-dashboard-platform/src/input.rs (신설)
pub struct Pointer { pub at: ScreenPoint, pub primary_down: bool }
pub enum PointerRead {
    Read(Pointer),   // 믿을 수 있는 읽기 — 위치와 주 버튼(왼손잡이 설정이면 오른쪽)을 한 순간에
    Unavailable,     // 이 OS 에 수단은 있으나 지금은 못 믿는다(읽기 실패 · 다른 데스크톱 · 전경이 다른 프로세스)
    Unsupported,     // 이 OS 에 수단이 없다
}
pub fn pointer() -> PointerRead;

// crates/engram-dashboard-platform/src/timer.rs (조건부 — §1-3 「실제 틱 주기」)
pub struct FineTimer(/* Drop 이 되돌린다 */);
pub fn fine_timer() -> FineTimer;
```

- **Windows 구현**
  - `topmost_at` — `EnumWindows` 는 최상위 창을 z 순서(위 → 아래)로 준다. `see_through` 에 든 창 · 보이지 않는 창(`IsWindowVisible` 거짓) · 최소화(`IsIconic`) · 가려 둔 창(DWM `DWMWA_CLOAKED` — 다른 가상 데스크톱 등) · 입력 투과 창(`WS_EX_LAYERED | WS_EX_TRANSPARENT`)을 건너뛰고, 사각형이 점을 품는 첫 창을 고른다. 사각형은 `DWMWA_EXTENDED_FRAME_BOUNDS`(보이는 틀 — `GetWindowRect` 는 Win10+ 의 보이지 않는 크기 조절 테두리를 품어 옆 창의 투명 테두리를 맞힌다). 그 창이 `candidates` 에 있으면 `Candidate(i)`, 아니면 `Other`. 끝까지 없으면 `Other`. Chrome 이 탭 합치기에서 끌리는 창을 빼고 자기 창을 찾는 방식과 같은 꼴이다(조사 §3).
  - `pointer` — `GetCursorPos`(PMv2 프로세스라 물리 가상 화면 좌표)가 실패하면 `Unavailable`. ★`GetAsyncKeyState` 는 현재 데스크톱이 입력 데스크톱이 아니거나 전경 스레드가 다른 프로세스면 0 을 준다★ — 그래서 전경 창의 프로세스가 이 프로세스가 아니면 `Unavailable` 을 주고, 그때만 `GetAsyncKeyState` 를 본다(이 OS 규칙은 이 함수 안에 둔다 — 부르는 쪽은 `Read` 만 믿으면 된다). 주 버튼 = `GetSystemMetrics(SM_SWAPBUTTON)` 이면 `VK_RBUTTON`, 아니면 `VK_LBUTTON` 의 `& 0x8000`(물리 버튼을 본다).
  - `native_window` — `RawWindowHandle::Win32(h)` 의 `hwnd` 를 `isize` 로.
  - `foreground` — `GetForegroundWindow` 가 무효 핸들이면 `Indeterminate`. 그 창을 만든 스레드의 PID 를 `GetWindowThreadProcessId` 로 읽어 0 이면(읽는 사이 사라짐) `Indeterminate`, 이 프로세스 PID 면 `Ours`, 아니면 `Other`. ★지금의 `foreground_is_current_process` 는 무효 · 사라짐을 `Some(false)` 로 접는다(`platform/src/window.rs` 의 그 함수 문서)★ — 끌기가 그 답을 쓰면 출처 팝아웃을 숨기는 순간의 빈 전경을 「다른 앱」으로 읽어 끌기를 끊는다. 그래서 새 함수를 두고, 옛 함수는 답을 바꾸지 않는 감싸기로 남긴다(호출자 쪽 동작 무변경). 셋째 갈래를 옛 함수에 더하지 않는 이유 = 그 호출자의 판정(앞 창이 우리 것이면 포커스를 준다)에는 「불명 = 아니다」가 맞는 답이다.
  - `fine_timer`(조건부) — `timeBeginPeriod(1)` / Drop 에서 `timeEndPeriod(1)`.
- **Windows 밖** — 자리채움(lib.rs 「들이는 규칙」 3): `native_window` → `None` · `topmost_at` → `Unknown` · `foreground` → `None` · `pointer` → `Unsupported` · `fine_timer` → 무동작. 셸은 `Unknown` 이면 커서를 품은 후보 중 **가장 최근에 포커스를 받은 창**을 고르고(끌기 스레드가 `Focused` 사건으로 순서를 든다 — 다른 앱 창에 가려진 자리를 못 거르는 근사), `Unsupported` 면 §1-3 의 갈래로 돈다. macOS 구현(`NSWindow.windowNumberAtPoint` · `NSEvent.mouseLocation` · `pressedMouseButtons`)이 들어오면 근사가 사라진다. 부르는 쪽에 OS 분기는 없다(R19 · ADR-0266).
- **스파이크가 고르면 더하는 함수** — S1(출처 창 안 보이게) · S4(딱지 창 활성 없이 보이기 · 입력 투과)가 Tauri API 만으로 합격선을 못 넘고 OS 호출 후보가 이기면, 그 목적 단위 함수 하나(예: `window::show_inactive(NativeWindow, bool)`)를 여기 더한다. 지금은 넣지 않는다.
- **핸들 타입** — 셸은 Tauri 창을 `native_window(&webview_window)` 에 넘겨 `NativeWindow` 를 받아 캐시한다(끌기 스레드로 옮겨 가므로 Send 가 필요하다). Tauri 2.11.3 의 `WebviewWindow` 가 `raw_window_handle::HasWindowHandle` 을 구현한다(**확실** — `tauri-2.11.3/src/webview/webview_window.rs:1469`). 그래서 platform 이 `raw-window-handle`(0.6 — 타입만 있는 crate · OS 코드 0 · `Cargo.lock:3157` 에 0.6.2 가 이미 있어 새 패키지 0)을 운영 의존으로 들인다. ★platform 의 「운영 의존 서드파티 = `windows` · `tracing` 만」 규칙(TRD 1-3 §0 ① · platform `src/lib.rs` 헤더 「의존」)을 고치는 일이라 ADR 이 필요하고 사용자 확인 대상이다(§11-1 #3 · §12 ADR-C)★. OS crate 게이트 4g 의 목록(`ci.yml` platform gate 5)에는 들지 않는다 — OS 바인딩이 아니다.
- **`windows` feature 추가** — `Win32_Graphics_Dwm`(`DwmGetWindowAttribute`) · `Win32_UI_Input_KeyboardAndMouse`(`GetAsyncKeyState`) · (조건부) `Win32_Media`(`timeBeginPeriod`). `EnumWindows` · `IsWindowVisible` · `IsIconic` · `GetSystemMetrics` · `GetCursorPos` · `GetWindowLongPtrW` 는 이미 켠 `Win32_UI_WindowsAndMessaging` 에 있다. CLAUDE.md 「의존성」의 `windows` 항목과 platform `Cargo.toml` 주석에 몫을 적는다(구현 단계 몫).
- **게이트** — 새 OS 분기는 전부 platform 안이라 4f(OS `cfg` 파일 명단) · 4h(`std::os` 명단)의 platform 밖 명단은 안 바뀐다. 4g(OS crate 운영 의존 = platform 하나)도 그대로다. 셸은 agent 를 의존하지 않는다(4i). platform `src/lib.rs` 헤더의 입주자 수(여덟 → 아홉 — `input` · `timer` 가 들면 열)를 고친다.

### 1-5. 웹뷰 ↔ 셸

| 방향 | 이름 | 꼴 | 페이로드(타입 수준) |
|---|---|---|---|
| 웹뷰 → 셸 | `dock_drag_begin` | invoke(async) | `{ view_id: Uuid, grab: PointCss }` — 누른 점(그 창 안쪽 CSS px) → `Ok{ session: u64 }` · `Err` = §1-3 「시작」의 표 |
| 웹뷰 → 셸 | `dock_drag_end` | invoke | `{ session: u64, reason: Released \| Lost }` → `()` · 모르는 세션은 버린다 |
| 웹뷰 → 셸 | `report_dock_chrome` | invoke(측정) | `DockChrome{ canvas_origin: PointCss, tab_bar: RectCss, tabs: Vec<TabHeader{ view_id, rect: RectCss }> }` — 창 label 은 Tauri 가 넣어 준 창에서 꺼낸다(`report_window_canvas` 와 같다 · `commands/layout.rs:556-570`) |
| 웹뷰 → 셸 | `dock_tab` | invoke(async) | 버스 인자와 같은 칸(§3-1) · 프론트 레지스트리(`__engramCmd`)가 부른다 |
| 셸 → 웹뷰 | `dock:session` | emit(전체) | `{ session, seq, phase: Started{ source_label, source_view, display_active: Option<Uuid> } \| Display{ label, display_active } \| Ended }` |
| 셸 → 웹뷰 | `dock:overlay` | emit_to(label) | `{ session, guide: Option<GuideView>, tab_bar: Option<TabBarHint> }` — 둘 다 `None` = 그 창의 오버레이를 지움 |
| 셸 → 딱지 | `dock:chip` | emit_to(딱지) | `{ title: String, theme: String, blocked: bool }` — `blocked` = `source_stale`(놓아도 무변경) |
| 셸 → 웹뷰 | `layout:updated` · `window:tabs-updated` | 기존 | 도킹 결과는 **기존 알림으로만** 흐른다 — 도킹 전용 결과 알림은 없다 |

```rust
struct GuideView { view_id: Uuid, bands: Vec<Band>, outer: Vec<Band>, hot: Option<Band> }
struct Band { side: DockSide, anchor: Option<Uuid> /* None = 바깥 변 */, rect: PxRectWire /* 캔버스 CSS px 정수 */ }
struct TabBarHint { insert_x: Option<f64> /* 끼울 자리(탭바 CSS px) — R21 */, dwell: Option<Uuid> /* 전환 대기 중인 탭 */ }
```

- `bands` = 커서 밑 칸의 **유효한** 띠(결과 면적 0 인 띠는 빠진다 — R8 · AC15)이고 각 띠의 사각형이 곧 놓았을 때의 결과 사각형이다(R3). `outer` = 캔버스 네 변의 띠(두께 `OUTER_BAND_PX`). `hot` = 커서가 든 띠의 결과 사각형 — 바깥 띠면 레이아웃 전체 변의 1/3 사각형(R4 · R5).
- 오버레이는 셸이 창별로 마지막 보낸 값과 다를 때만 보낸다. 그 「마지막 보낸 값」이 놓기 판정의 기준이다(§1-3).
- **크롬 보고를 늘 하는 이유** — 끌기 시작 때 보고를 기다리면 첫 판정이 늦다. 탭바 · 캔버스 원점은 드물게 바뀌므로 평소에 보고하고(디바운스 · 한 번에 하나 — `windowCanvasReport.ts` 와 같은 꼴), 셸은 창 항목에 측정으로 둔다(`WindowTabs` 의 `canvas` · `metrics` 옆 — `manager.rs:226-230` · `version` 을 올리지 않는다 — ADR-0227 의 측정 규칙). 크롬에 떼어진 탭이 들어 있어도 판정이 id 로 거른다(§2-2 규칙 3).

### 1-6. 프론트 (렌더링만 — 제어는 셸)

- **`TabBar.tsx`** — 탭(닫기 버튼 · 이름 입력 제외)에서 주 버튼 `pointerdown` → 누름 상태 · `pointermove` 가 `DRAG_THRESHOLD_PX`(4) 를 넘으면: main 의 마지막 탭이면 커서 「안 됨」만(R14 · AC9), 아니면 `dock_drag_begin`. 그 뒤 `setPointerCapture` + 창 수준 `pointerup`(→ `Released`) · `pointercancel` · `blur`(→ `Lost` 힌트) 리스너. `lostpointercapture` 는 보내지 않는다 — Chromium 은 버튼을 쥔 채로도 캡처를 잠깐 잃는다(`Splitter.tsx:244-245` 의 선례). Esc 는 듣지 않는다(R12). 끌기가 시작됐으면 그 누름의 `click`(탭 전환)을 삼킨다.
- **떼어진 표시 = 그리기만(3판 리뷰 F3)** — ★스토어의 창 탭 목록(`win.tabs`)을 거르지 않는다★: `WindowLayout.tsx:142-157` 은 그 목록이 0 개가 되면 자기 창을 닫는다(0탭 자가닫힘 — 닫기 요청은 이제 §1-2 에 따라 모델에서 그 창을 지운다). 떼어진 표시는 렌더 때만 쓰는 별도 값(`detached: { view, displayActive } | null`)이고, `TabBar` 는 그 탭 머리를 건너뛰어 그리고 캔버스는 모델 활성 탭이 그 탭인 동안 `displayActive` 탭을 보인다(keep-alive 표시 토글 — `WindowLayout.tsx:190` · ADR-0056). **탭이 하나뿐인 팝아웃은 떼어진 표시를 하지 않는다** — 창 자체가 안 보이고, 보일 탭도 없다. `Started` · `Display` 로 세우고 `Ended` 로 거두며 `(session, seq)` 가 오래된 것은 버린다. 모델에는 아무것도 보내지 않는다.
- **`DockOverlay.tsx`(신설)** — 캔버스 div(`WindowLayout.tsx:190` — `position: relative`) 안에 `position:absolute; pointer-events:none` 로 띠(빗금) · `hot`(반투명 강조) · 바깥 띠를 그린다. 탭바 위 오버레이는 `tab_bar` 힌트(끼울 자리 표시 · 전환 대기 진행)를 그린다. `dock:session Ended` 를 받거나 오버레이의 세션이 지금 세션이 아니면 전부 지운다(R13b 「남은 가이드 없음」).
- **`dockChromeReport.ts`(신설)** — 탭바 요소 `ResizeObserver` · 탭바 `scroll` · 탭 목록 · 떼어진 표시 변경 · 캔버스 원점 변경 때 `report_dock_chrome`. `windowCanvasReport.ts` 의 「웹뷰당 한 번에 하나 · 마지막 값이 덮는다」 규칙을 그대로 쓴다.
- **레지스트리(`tabCommands.ts`)** — `tab.dock` 을 등록하고 `viewStore` 의 새 액션 `dockTab`(→ invoke `dock_tab`)을 부른다(ADR-0055/0064 — 사람 · LLM 같은 표면). 새 전역 핸들은 없다.

### 1-7. 계측 (3판 리뷰 F8 · `docs/reference/logging-conventions.md` 「계측 의무」)

끌기는 세션 수명 · 동시성 가드 · 외부 경계(창 만들기 · 숨기기)를 다 가진 load-bearing 경로다. 그룹 = 기본 모듈 경로(`engram_dashboard::dock` · `engram_dashboard::layout`) — 따로 `target` 을 달지 않는다. 식별자 · 수치는 필드로, 메시지는 한국어 한 줄.

| 어디 | 무엇 | 레벨 | 필드 |
|---|---|---|---|
| `dock_drag_begin` | 끌기 시작(표식 세움) | info | `session` · `view` · `source` · `tabs` · `hide = 방법` |
| 같은 곳 | 거절(`TabNotFound` · `AlreadyDragging` · `MainLastTab`) | debug | `view` · `reason` |
| 같은 곳 | 준비 · 부수 효과 실패(`ChipUnavailable` · `SourceUnavailable` · `ThreadSpawn` · `HideFailed`) + 되감기 | warn | `view` · `step` · `: {e}` |
| driver 끝 | 끌기 끝 — 한 줄 | info | `session` · `outcome = applied \| noop \| abnormal \| aborted` · `reason`(아래) · `mode` · `window` · `ticks` · `max_tick_ms` · `elapsed_ms` |
| driver 끝 | `reason` 어휘 | — | `SourceChanged` · `TargetChanged`(대조 불일치 포함) · `StaleLease` · `ZeroArea` · `WindowClosed` · `TabNotFound` · `ForegroundLost`(전경이 다른 프로세스) · `InputLost`(전경 불명 · 포인터 못 읽음이 500ms) · `WebviewLost` · `SourceDestroyed` · `SourceGone` |
| driver | 늦은 세션 신호 · 끝난 뒤 사건 버림 | debug | `session` · `current` · `event` |
| driver | 딱지 이동 · 보이기 · 숨기기 실패, 출처 숨기기 · 되보이기 실패 | warn(세션당 처음 한 번) · 나머지는 끝 줄의 `chip_errors` 셈 | `label` · `: {e}` |
| driver | `Unavailable` · 전경 `Indeterminate` 유예 시작(첫 번째) | debug | `session` · `cause` |
| driver | R6b 전환 | debug | `window` · `view` |
| `apply::dock_tab` | 적용 성공 | info | `view` · `mode` · `window` · `closed_window` · `reused` |
| 같은 곳 | 거절(`DockError`) | debug(호출자에게 돌아간다 — 마우스 경로는 끝 줄이 남긴다) | `view` · `mode` · `reason` |
| 같은 곳 | 새 창 phase C 가 묘비로 멈춤 · 연 창 거두기 | warn | `label` |
| 같은 곳 | 버스 자리가 어느 모니터에도 안 걸침 | info(`placement::land` 의 기존 줄) | `label` |
| `lib.rs` `CloseRequested` | 팝아웃 모델 먼저 지움 + 묘비 | info | `label` · `dropped_views` |

`let _ =` 로 버리는 실패는 없다 — 셸 세터 실패는 위 warn 줄로, 끄는 동안 반복되는 것은 셈으로 끝 줄에 모은다(125회/초 로그를 내지 않는다).

## 2. 대상 판정(hit-test) — 순수 함수

`src-tauri/src/dock/hit.rs` — Tauri · 락 · OS 호출 0. 입력은 전부 값이고 같은 입력이면 같은 답이다(R6a ③). 놓기 판정(§1-3)도 같은 함수를 쓴다.

### 2-1. 입력 · 출력

```rust
pub const OUTER_BAND_PX: f64 = 32.0;                          // R5 — 바깥 띠 두께(CSS px)
pub const SIDE_ORDER: [DockSide; 4] = [Left, Right, Top, Bottom];  // 동률 깨기 — 고정 순서

pub struct WindowFrame<'a> {                                  // 맨 위 창 하나(셸이 platform 으로 골랐다)
    pub label: &'a str,
    pub inner_origin: (f64, f64),                             // 안쪽(클라이언트) 왼쪽 위 — 물리 px
    pub scale: f64,                                           // 그 창의 배율(Tauri scale_factor)
    pub chrome: &'a DockChrome,                               // 캔버스 원점 · 탭바 · 탭 머리(CSS px — 보고 그대로)
    pub canvas: CanvasPx,                                     // 캔버스 크기(CSS px 정수 — ADR-0227 보고)
    pub visible_view: Uuid,                                   // 그 창에 지금 보이는 탭(출처 창이면 떼어진 표시 반영)
    pub slots: &'a [(Uuid, PxRect)],                          // 보이는 탭 칸 틀 — geometry::frame_px
}
pub enum Locate {                                             // 1 단계 — 어디 위인가
    Nothing,                                                  // 가이드 없음
    TabBar { label, index: usize, over: Option<Uuid> },       // R21 끼울 자리 · over = 머리 위의 탭(R6b 대기)
    Outer { label, view, side: DockSide },                    // 바깥 띠(R5 · R6a ①)
    Slot { label, view, slot: Uuid, frame: PxRect, p: (f64, f64) },  // 칸 위 — 띠 고르기는 2 단계
}
pub fn locate(cursor: (f64, f64), top: Option<&WindowFrame>, source_view: Uuid) -> Locate;
/// 2 단계 — 칸 안에서 어느 띠인가. `bands` = 그 칸의 유효한 띠와 그 결과 사각형(세션이 docked_tree 로 계산).
pub fn choose_band(p: (f64, f64), frame: PxRect, bands: &[(DockSide, PxRect)]) -> Option<DockSide>;
```

두 단계로 가른 이유: 띠의 사각형 = 놓았을 때의 결과 사각형(R3)이고 그 값은 `docked_tree` 를 돌려야 나온다. 1 단계는 칸만 찾고, 세션이 그 칸의 결과를 (캐시해) 계산한 뒤 2 단계가 **그려진 띠 그대로** 고른다 — 칠하지 않은 띠(면적 0 · R8)에서는 붙지 않는다는 것이 따로 검사 없이 선다.

### 2-2. 규칙

1. **맨 위 창 없음**(`topmost_at` 이 `Other`) → `Nothing`(R15 — 다른 앱 창에 가려진 자리 · 바탕화면 · 최소화된 창).
2. **창 안쪽 좌표** — `css = (cursor − inner_origin) / scale`. 커서 · 창 원점은 물리 px(PMv2 가상 화면 좌표), 배율은 **그 창의** 것이다. 창 하나엔 배율 하나라(두 모니터에 걸쳐도 Windows 는 한 DPI 를 준다) 이 나눗셈이 그 웹뷰가 그리는 CSS px 와 맞는다(W4 · AC12).
3. **탭바** — `css` 가 `chrome.tab_bar` 안이면 `TabBar`. ★탭 머리 목록에서 `view_id == source_view` 인 것을 먼저 뺀다(3판 리뷰 F9)★ — 크롬 보고는 떼어진 표시보다 늦게 올 수 있어 그 보고를 믿지 않는다. 그렇게 거른 목록 = 결과 목록이므로, `index` = 그 목록에서 가운데 x 가 커서보다 왼쪽인 머리의 수다. `over` = 커서를 품은 머리(거른 목록에서).
4. **캔버스** — `p = css − chrome.canvas_origin`. `p ∉ [0, W) × [0, H)` 면 `Nothing`.
5. **끄는 탭 자신** — `visible_view == source_view` 면 `Nothing`(떼어진 표시로 실제로는 안 생긴다 — 방어).
6. **바깥 띠(R5 · R6a ①)** — 거리 `d = (p.x, W − p.x, p.y, H − p.y)`(L · R · T · B). `d < OUTER_BAND_PX` 인 변이 있으면 그중 가장 작은 `d` 의 변(동률은 `SIDE_ORDER`)으로 `Outer`. 슬롯 띠보다 먼저 보므로 창 변에 붙은 작은 칸 위에서도 바깥 띠가 이긴다(AC4).
7. **칸 찾기** — 칸 틀은 반열림 `[x0, x1) × [y0, y1)`. 이웃 칸은 같은 정수 경계를 나누므로(`geometry.rs` 머리 불변식 · `px_edge` · `:159`) 점 하나에 칸 하나다. 못 찾으면 `Nothing`.
8. **띠 고르기(2 단계)** — 띠 사각형은 칸 바깥쪽 변에 붙어 있다. 점이 띠 사각형 안이면(칸 **안쪽으로 향한 변은 제외** — 그 선 위는 가운데 몫) 후보다. 후보가 없으면 `None`(가운데 — R6). 둘 이상이면(모서리) **정규화 거리** = 그 띠의 칸 변까지 거리 ÷ 그 띠의 두께가 가장 작은 띠(R6a ②), 동률은 `SIDE_ORDER`.

**경계 포함 규칙 한 줄(R6a ③)** — 반열림 사각형(왼쪽 · 위 포함, 오른쪽 · 아래 제외) · 바깥 띠는 `d < 두께` · 띠도 안쪽 경계선은 띠 밖 · 탭바 자리는 가운데 x 와 같으면 오른쪽 · 동률은 언제나 `Left → Right → Top → Bottom`. 부동소수 비교만 하고 반올림하지 않는다.

### 2-3. 바깥 띠 두께 = 32 CSS px

- Eclipse 4 의 바깥 띠 35px(조사 §7 `SplitDropAgent2.java`)와 같은 크기대다. 탭바 높이(28px — `TabBar.tsx:65`)보다 조금 두꺼워 노리고 들어가기 쉽다.
- 폭 300px 칸의 옆 띠(1/3 = 100px) 중 창 변에 닿은 32px 만 바깥 띠가 가져간다. CSS px 라 배율과 무관하게 같은 눈 크기다. 값은 `hit.rs` 상수 하나다(R5 「값은 TRD」 · 기본값).

### 2-4. 세션이 판정 앞뒤로 하는 일

- **후보 · 투과** — 후보 = 모델의 창 중 보이고 최소화 안 된 것에서 안 보이게 한 출처 창을 뺀 것, 투과 = 딱지 창(R15 「딱지를 빼고 본」).
- **유효 띠 캐시** — 키 = (대상 View, 전역 `version`, 칸 id). 그 칸의 네 변 · 대상 바깥 네 변 = 최대 여덟 번 `docked_tree`(끼울 것 = `expect_source`). 칸이나 `version` 이 바뀔 때만 다시 한다. `Outer` 도 그 변의 결과가 면적 0 이면 칠하지 않고 `Nothing` 으로 친다(R8).
- **보낸 가이드의 기대값** — Seam · Outer 는 `TargetExpect::Layout(그 계산에 쓴 대상 배치)`, TabBar 는 `TargetExpect::Tabs(그 창 탭 목록)`. 이 값이 놓기의 `expect_target` 이다(§1-3).
- **`source_stale`** 이면 띠 · 탭바 자리를 하나도 내지 않는다.
- **출처 창** — 그 창도 대상이다: 보이는 탭(`display_active` 또는 모델 활성)의 칸에 붙거나(같은 창 합치기 · AC14), 그 창 탭바에 놓아 자리를 옮긴다(R21).

## 3. LLM 표면

### 3-1. 버스 명령 (`src/layout/commands.rs` · `catalog_version` 17 → 18)

R16 의 「명령 하나 + 모드」를 그대로 따른다. 모드별 칸은 선택 칸으로 펴서 받고 조합은 핸들러가 검사한다 — `layout.setSlotContent` 의 「태그 + 곁칸」(`commands.rs:374-381` · 조합 검사 `slot_content` · `:1413`)이 선례다.

```
/// 붙일 쪽 — 이름이 결과 배치를 말한다. Left = 기준의 왼쪽에 새것 · Right = 오른쪽 · Top = 위 · Bottom = 아래.
enum DockSide { Left, Right, Top, Bottom }
/// Seam = 다른 탭의 슬롯 옆이나 레이아웃 바깥 변 · TabBar = 어느 창의 탭으로 · NewWindow = 그 탭만 든 새 창으로.
enum DockMode { Seam, TabBar, NewWindow }

/// 탭 하나(그 탭의 슬롯 배치 통째)를 옮긴다. mode 마다 쓰는 칸이 다르다 —
/// Seam: target_view_id(필수) · target_slot_id(빼면 대상 탭 레이아웃 전체의 그 변) · side(필수). 새것 1/3 · 기존 2/3.
/// TabBar: to_window(필수) · index(옮긴 뒤 탭 목록에서의 자리 · 빼면 맨 뒤 · 넘으면 맨 뒤). 같은 창이면 자리만 옮긴다.
/// NewWindow: x · y(새 창 바깥 왼쪽 위 — 물리 화면 px · 빼거나 어느 모니터에도 안 걸치면 기본 자리) · width · height
///   (논리 px · 빼면 원래 창 크기, main 에서 옮기면 팝아웃 기본 크기). 탭이 하나뿐인 팝아웃이면 새 창을 만들지 않고 그 창을 옮긴다.
/// 옮긴 탭은 출처 창에서 사라지고, 그 창이 팝아웃이고 비면 닫힌다(closed_window). 창 활성화 · 포커스는 바뀌지 않고,
/// 대상 창의 보이는 탭도 그대로다 — 단 옮긴 탭이 그 창에 보이던 탭이면 대상이 보이게 된다.
/// 보이지 않는 탭 · 최소화된 창에도 붙일 수 있다. 사용자가 그 탭을 끄는 중이면 CONFLICT. 대상 탭 · 창이 닫혔으면
/// (닫히는 중 포함 — 닫기 요청 순간 모델에서 빠진다) NOT_FOUND. NewWindow 의 새 창이 확정 전에 닫히면 CONFLICT.
#[effect(Write)] #[since(18)]
"tab.dock" => args TabDockArgs {
    view_id: String,                    // 옮길 탭
    mode: DockMode,
    target_view_id: Option<String>,     // Seam
    target_slot_id: Option<String>,     // Seam — 생략 = 바깥 변
    side: Option<DockSide>,             // Seam
    to_window: Option<String>,          // TabBar
    index: Option<u32>,                 // TabBar
    x: Option<i32>, y: Option<i32>,     // NewWindow
    width: Option<f64>, height: Option<f64>,  // NewWindow
} -> ok TabDockOk {
    window: String,                     // 탭(Seam 이면 그 슬롯들)이 내려앉은 창 label
    slot_ids: Vec<String>,              // Seam: 붙은 슬롯 id(옮긴 탭의 슬롯 id 그대로 · 전위 순) · 그 밖 = 빈 목록
    tab_id: Option<String>,             // TabBar · NewWindow: 옮긴 탭 id(= view_id 그대로) · Seam = 없음(탭이 대상 탭에 녹았다)
    closed_window: Option<String>,      // 탭이 빠져 닫힌 출처 팝아웃 label
} errors [NOT_FOUND, CONFLICT];
```

- **이름** — 옮기는 것이 탭(View)이므로 `tab.*` 에 둔다(`slot.popout` 은 슬롯 하나를 옮겨 `slot.*`). 인자 `view_id` = 옮길 탭이다 — 답에는 `view_id` 를 다른 뜻으로 싣지 않는다(`commands.rs:1057-1060` 의 혼동 경고). 답의 `tab_id` 는 같은 값이고 PRD 4.1 R16 의 「옮긴 탭 id(②)」 자리다.
- **조합 검사** — 모드에 필요한 칸이 없거나 다른 모드의 칸이 섞이면 `INVALID_ARGUMENT`(무변경). `x` · `y` 는 둘 다 있거나 둘 다 없어야 하고, `width` · `height` 도 같다. 크기는 유한한 양수여야 한다. 자리가 어느 모니터에도 안 걸치는 것은 오류가 아니다 — 기본 자리로 연다(§1-2 「자리 검증」).
- **`DockSide` 철자** — 셸 안쪽 `layout::dock::DockSide` 도 serde rename 없이 둬 두 표면(버스 · invoke)이 같은 철자(`Left` …)를 쓴다. 철자 맞대기 시험을 `both_surfaces_spell_the_split_ratio_outcome_the_same_way`(`tests/layout_commands.rs` · `types.rs:172-175`) 선례처럼 둔다.
- **세대 번호** — 선언이 바뀔 때마다 올린다(`commands.rs:96`). 세 모드를 **한 단계에서 한 번에** 선언해 17 → 18 한 번으로 끝낸다(§10 3단계). 단계를 나눠 선언을 바꾸게 되면 그때마다 올린다.
- **머리 갱신** — 같은 단계에서 `commands.rs` 머리 「적용 실패는 코드 하나로 나간다(`CONFLICT`) — 레이아웃 명령만」(`:24-34`)에 「`tab.dock` 은 서비스가 타입드 오류를 내므로 종류대로 옮긴다(`dock_error`)」를 더한다.

### 3-2. 실패 사유 → 오류 코드

적용 서비스가 타입드 오류(`DockError`)를 내므로 종류로 코드를 고른다. 문구는 사람 · LLM 이 읽을 한국어 사유 + 괄호 속 종류 낱말이고, 기계 분기 재료가 아니다(같은 머리글 TRD §4-⑦).

| R16 사유 | `DockError` | 코드 |
|---|---|---|
| 탭 없음(Seam 의 대상 탭이 닫히는 창과 함께 빠진 경우 포함) | `TabNotFound` | `NOT_FOUND` |
| 창 없음(TabBar 의 대상 창이 닫히는 중 · 닫힘 포함) | `WindowNotFound` | `NOT_FOUND` |
| 슬롯 없음 | `SlotNotFound` | `NOT_FOUND` |
| 그 슬롯이 대상 탭 소속이 아님 | `SlotNotInTab` | `NOT_FOUND` |
| 메인 마지막 탭 | `MainLastTab` | `CONFLICT` |
| 자기 자신에게 | `SelfTarget` | `CONFLICT` |
| 면적 0 슬롯이 생김 | `ZeroArea` | `CONFLICT` |
| 사용자가 그 탭을 끄는 중 | `Dragging` | `CONFLICT` |
| NewWindow 의 새 창이 phase C 전에 닫힘(★NewWindow 만 — Seam · TabBar 에서는 안 난다★) | `WindowClosed` | `CONFLICT` |
| (명령 경로에선 안 남) | `StaleLease` · `SourceChanged` · `TargetChanged` | `CONFLICT` |
| 새 창을 못 엶 | `WindowOpen` | `INTERNAL`(표 공통 코드 — `macros.rs:76`) |
| id 형식 불량 · 모드 칸 조합 불량 | (핸들러 파싱) | `INVALID_ARGUMENT`(기존 `uuid_arg` · `:1385`) |

실패면 아무것도 바뀌지 않는다 — 판정이 전부 변경 앞에 있고(§1-2 판정 순서) 변경은 락 한 번(새 창은 phase C 한 번)이다. 새 창 열기 실패 · phase C 실패는 모델 무변경이다.

### 3-3. 같은 길 — 마우스 · invoke · 버스

| 입구 | 껍데기 | 떨어지는 곳 |
|---|---|---|
| 버스 `tab.dock` (LLM · CLI) | `commands.rs` `verb_tab_dock`(`blocking_handler` — `slot.popout` 과 같다) · 창 포트 = `OwnedWindowHost` | `apply::dock_tab(DockGuard::NONE, Keep)` |
| 프론트 레지스트리 `tab.dock`(`window.__engramCmd`) | `tabCommands.ts` → `viewStore.dockTab` → invoke `dock_tab`(async) · 창 포트 = `TauriWindowHost` | 같은 곳 |
| 마우스 놓기 | 끌기 스레드(`dock::driver`) · 창 포트 = `TauriWindowHost` | `apply::dock_tab(DockGuard{ 표식, 기대 배치 }, Reveal)` |

마우스와 명령의 차이는 `DockGuard` · `FocusPolicy` 두 값이다 — 마우스만 R13c 대조와 R11b 포커스를 더한다. 두 번째 변경 경로는 없다(R16 · CLAUDE.md 「LLM-우선 제어」 — 기존 표면에 얹었고 새 전역 핸들은 없다).

- **R17** — 버스 명령은 셸이 데몬에 붙어 있을 때만 LLM 에 닿는다(셸 명령 표 전부와 같다). 마우스 끌기 · 프론트 레지스트리는 데몬과 무관하다.
- **프론트 레지스트리 인자** — `tab.dock{ view, mode, targetView?, targetSlot?, side?, window?, index?, x?, y?, width?, height? }`.
- **끌기 자체는 명령이 아니다** — `dock_drag_*` 는 사람 손의 입력이라 레지스트리 · 버스에 싣지 않는다. LLM 은 결과 명령(`tab.dock`)을 바로 부른다.

## 4. 떼어진 표시 · 딱지 창 · 가이드 (R2 · W1 · W7)

### 4-1. 떼어진 표시

- **탭이 여럿인 창 · main** — 창은 제자리다. 웹뷰가 그 탭 머리를 그리지 않고, 그 탭이 보이던 탭이면 승계 탭을 보인다(§1-6 — 그리기만 · 스토어 목록은 그대로). 승계 규칙은 셸이 계산해 보낸다(`succession` — 모델의 `close_tab` 과 같은 함수 · §1-2) — 웹뷰와 판정이 같은 탭을 「보이는 탭」으로 친다.
- **탭이 하나뿐인 팝아웃(W1)** — 떼어진 표시 대신 그 창을 안 보이게 한다. ★방법은 스파이크 S1 이 고른다★ — 후보 셋: ⓐ 화면 밖으로 옮기기(Tauri `set_position` — OS 코드 0 · 창이 보이는 상태로 남아 활성 · 포인터 캡처가 그대로일 것으로 본다) ⓑ Tauri `hide()`(가장 단순 · 단 Windows 는 활성 창을 숨기면 다른 창을 활성화해 전경이 다른 앱으로 갈 수 있다 — 그러면 틱 1 이 곧바로 비정상 끝을 낸다) ⓒ platform 의 목적 함수(알파 0 등). 합격선 = 버튼을 쥔 동안 끌기가 이어지고 전경이 우리 프로세스에 남으며, 되보일 때 원래 자리 · 크기 · 최대화로 돌아온다(§8 S1). ⓐ · ⓒ 는 되보일 때 `placement::place` → `maximize`(`placement.rs:470-497` 순서)로 되돌린다 — 위치 세터가 최대화 표식을 내리기 때문이다(`:486`). 끄는 동안 그 창의 자리 기록은 멈춘다 — 화면 밖 자리가 `state.json` 에 적히면 끌다 앱이 죽었을 때 창이 엉뚱한 자리로 복원된다. ⓐ 의 창은 작업 표시줄에는 남아 사용자가 거기서 닫을 수 있다 — 그러면 §1-2 의 닫기 요청 경로로 탭이 함께 닫히고 끌기는 `Destroyed(출처)` 로 끝난다(R13b 끝 문장).
- 끄는 동안 셸은 웹뷰 입력에 기대지 않는다 — 포인터는 `platform::input::pointer()` 로 직접 읽는다. 그래서 숨긴 창이 포인터 사건을 계속 받는지는 정보다(웹뷰 `Released` 는 떼기의 두 번째 신호일 뿐이다 — `Unsupported` 일 때만 유일한 신호).

### 4-2. 딱지 창 (W7)

- **창 하나를 처음 끌기 때 만들어 숨겨 두고 다시 쓴다** — label `dock-chip`(`dock::chip::CHIP_LABEL`). 팝아웃 접두(`slot-popup-` — `popout.rs:35`)가 아니라서 모델 · 저장 · 라우팅 · `Destroyed` 정리 · 닫기 요청 모델 정리(`is_popup_label` 거름 — `lib.rs:313-329`) · 트레이 보이기/숨기기(`tray/core.rs:84` `ui_windows`)의 대상이 아니다.
- **빌더** — `WebviewEnv::finish`(`webview_env.rs:73-86` · ADR-0283 · ADR-0054 — 건너뛰면 유령 창)를 지난다. 꾸밈 없음 · 크기 조절 없음 · 그림자 없음 · 작업 표시줄 없음(`skip_taskbar`) · 항상 위 · `focusable(false)`(tao = `WS_EX_NOACTIVATE` — `window_state.rs:296-298`) · `focused(false)` · 처음엔 숨김. **불투명** — 칩은 사각형이라 투명이 필요 없다(WebView2 투명 창 이슈를 피한다 — 조사 §4). 크기 = `CHIP_SIZE`(180×28 논리 px).
- **입력 투과는 켜지 않는다(3판 리뷰 F5)** — tao 의 `set_ignore_cursor_events(true)` 는 `WS_EX_TRANSPARENT | WS_EX_LAYERED` 를 달면서 `SetLayeredWindowAttributes` 를 부르지 않는다(`tao-0.35.3/src/platform_impl/windows/window_state.rs:285-287` · `window.rs:566-576`) — 속성 없는 layered 창은 그려지지 않을 수 있다. 판정은 딱지를 투과 목록으로 이미 빼고, 끄는 동안 버튼 입력은 출처 창이 캡처로 쥐므로 딱지가 마우스를 받을 일이 없다. 투과가 필요하다고 드러나면 S4 의 변수로 잰다.
- **최소 경로** — `WebviewUrl::App("dock-chip.html")`. 별도 Vite 입력(`vite.config.ts` `build.rollupOptions.input` 에 `index.html` 과 나란히)이고 React · 스토어 · 데몬 연결을 띄우지 않는다 — `src/dockChip/main.ts` 가 `dock:chip` 을 듣고 글자 · 테마 · 「안 됨」 표시만 바꾼다. 권한은 `src-tauri/capabilities/dock-chip.json`(`windows: ["dock-chip"]` · `core:event:default` 하나).
- **움직임** — 끌기 스레드가 커서가 움직인 틱마다 `set_position(커서 + CHIP_OFFSET × 그 모니터 배율)`(물리 px). tao 의 위치 세터는 `SWP_ASYNCWINDOWPOS | SWP_NOACTIVATE`(`tao-0.35.3/src/platform_impl/windows/window.rs:240-249`)라 활성을 안 가져가고 기다리지 않는다. 딱지가 커서 점을 덮지 않게 오른쪽 아래로 비켜 둔다(`CHIP_OFFSET` = (12, 16) 논리 px).
- **자리 기록 제외** — `lib.rs` 의 `Moved | Resized` 갈래(`:333-335`)에서 label 이 딱지면 `placement::record` 를 부르지 않는다. 모델에 없는 label 은 `record` 가 어차피 버리지만(`placement.rs:799-801`), 거르지 않으면 125회/초로 OS 게터 왕복 + 레이아웃 락을 낸다. 끌기 채널로도 보내지 않는다.
- **보이기 · 숨기기** — 다음 끌기의 `show()` 는 tao 가 `SW_SHOW` 로 내보낸다(`SW_SHOWNOACTIVATE` 는 `focused(false)` 의 첫 보이기뿐 — `window_state.rs:326-333`). `WS_EX_NOACTIVATE` 창에 `SW_SHOW` 가 활성을 가져가는지는 **모른다** → 스파이크 S4. 후보 = ⓐ 그대로 `show()`/`hide()` ⓑ 늘 보인 채 화면 밖에 세워 두기(Alt+Tab 에 비치는지 확인) ⓒ platform `show_inactive`. S4 가 고른다.
- **새 창(R22) 위치** — 놓는 순간 딱지의 왼쪽 위(물리 px)가 새 창 바깥 왼쪽 위다(PRD 「딱지를 놓은 자리 기준」). 크기는 R22 규칙(§1-2 `plan_new_window`). 다른 모니터 빈 곳에 놓으면 그 모니터의 배율로 크기가 풀린다(위치 먼저 → 크기 — §1-2 「창 포트 확장」 · AC12).

### 4-3. 가이드 그리기

- 셸이 `dock:overlay` 로 **무엇을 어디에** 그릴지(캔버스 CSS px 사각형)를 주고, 웹뷰는 좌표 계산 없이 그 사각형을 그린다(ADR-0227 결정 1 과 같은 태도).
- 띠 = 빗금(R3), `hot` = 반투명 강조(R4 · R5), 탭바 끼울 자리 = 세로 막대(R21), 전환 대기 = 그 탭 머리의 진행 표시(R6b). 색은 테마 CSS 변수(`--accent` 계열)를 쓴다. 시각 세부(빗금 간격 · 칩 모양)는 시안(아티팩트 「슬롯 도킹 시안」 v2)을 따른다.
- `dock:session Ended` 를 받으면 무조건 지운다 — 어떤 창이 오버레이 지움 한 장을 놓쳐도 끝 방송 한 장이 지운다(R13b · AC16). 딱지 창은 셸이 숨긴다.

## 5. 저장 · 원자성 · 출력 연속 (R11 · R11a)

기존 장치가 그대로 덮는다 — 이 기능이 새로 짓는 저장 · 라우팅 장치는 없다.

- **끄는 동안 저장되는 것 없음** — 표식 · 묘비는 메모리에만, 떼어진 표시는 웹뷰에만 있다. 숨긴 출처 팝아웃의 자리 기록은 멈춘다(§4-1). 그래서 끄는 동안 앱이 꺼지면 다음 부팅은 끌기 전 배치(탭은 출처 한 곳 · 팝아웃은 원래 자리)다.
- **원자성(R11a 앞 절반)** — 출처 탭 빼기 · 대상(이음새 · 탭바 · 새 창 항목)에 넣기 · 빈 팝아웃 항목 지우기 · 곁표 옮기기가 `ViewManager::dock` 한 번(= 락 한 번 · `version` 한 번)이다. 새 창은 OS 창을 먼저 열고(phase B) 모델 변경은 phase C 한 번이다. 그 락을 거치지 않고 모델을 보는 관찰자는 없다 — 프론트는 그 뒤의 알림 · pull 로만 보고(`apply.rs` 머리 「락 규율」), 버스 조회도 같은 락을 잡는다.
- **저장(R11a 뒤 절반)** — 기록기는 변경 번호(`version` · `attrs_rev`)가 바뀌면 스냅숏을 **같은 락 안에서 통째로 복사**해 쓴다(`state/convert.rs:24-40` `StateRevision` · `to_persisted` 는 락 안에서 부른다 — `:43` · `state/saver.rs:20` 「기록기는 락을 쥐지 않는다」). 그래서 `state.json` 에 실리는 것은 도킹 전이거나 도킹 뒤이고, 탭이 두 곳이거나 어디에도 없는 판은 생길 수 없다. 원자 쓰기(base `file`)라 디스크 쓰기 도중 꺼져도 이전 판이 남는다 — 「정확히 한 곳」(AC11)은 어느 쪽이든 선다. 새 창 phase B 와 C 사이에 꺼지면 OS 창만 있고 모델은 도킹 전이라 다음 부팅에 그 창은 없다.
- **OS 창 닫기는 모델 뒤 — 이제 모든 경로가** — 빈 팝아웃은 모델에서 먼저 지우고(락 안) 락 밖에서 OS 창을 destroy 한다(`close_tab` 의 `WindowClosed` 갈래와 같다 — `apply.rs:303-306`). 사용자 X 도 닫기 요청에서 모델을 먼저 지운다(§1-2 「닫히는 창」) — 그래서 「OS 창은 사라졌는데 모델에 남은 창」과 「모델에서 지운 뒤 붙은 탭」이 어느 경로에도 없다. destroy 는 기다리지 않는다.
- **출력 연속(R11 · AC7)** — 같은 락 안의 `subs.resync(&mgr)`(`output_router.rs:121-134` 「ViewManager 락 보유 중 · 같은 critical section」)가 라우팅 표를 바꾼다(ADR-0057 라우팅 · ADR-0046). 에이전트가 출처 창에서 대상 창으로 옮겨도 「어느 창에든 보이는가」는 그대로라 데몬 구독은 끊기지 않고(0→1 · 1→0 델타가 없다) 경로만 바뀐다. 대상 웹뷰에 새로 마운트된 칸은 기존 구독 효과(`[viewId, agentId]` · replay 요청)로 지난 출력을 받는다 — 팝아웃이 옮긴 칸과 같은 길이다. 출처 창이 닫히면 `Destroyed` 정리가 그 창의 출력 Channel 을 등록 해제한다. 재사용(R22 · 탭 하나 팝아웃)은 모델이 안 바뀌어 라우팅도 안 바뀐다.
- **웹뷰 안 화면 상태(D10 비목표)** — 입력 초안 · 스크롤 · 렌더 모드 오버라이드는 출처 웹뷰에만 있어 따라가지 않는다. 같은 웹뷰 안 도킹(같은 창 다른 탭 — AC14)에서 id 로 묶인 상태가 우연히 살아남을 수 있으나 보장하지 않는다.

## 6. 사용자 결정

### 6-1. 확정 (PRD 4.1판 「(결정)」 · §5-1 — 2026-10-15)

- **제목줄 = 창 이동만, 탭 = 도킹**(R1 · R2) — 제목줄은 OS 기본 이동이고 이 기능은 끼어들지 않는다.
- **탭을 끌면 곧바로 떼어지고 작은 딱지가 따라간다 · 창은 따라오지 않는다 · 「원래대로」는 없다**(R2 · Q1) — §4.
- **모델은 놓을 때만 바뀐다 · 끄는 동안 떼어진 모습은 화면 표시다**(R2 아래 줄 · R11a) — §1-3 · §4-1.
- **가이드 = 슬롯 상하좌우 띠(1/3) + 정확한 결과 사각형 + 창 바깥 테두리 띠, 가운데는 없음**(R3 · R4 · R5 · R6) — §2.
- **놓으면 셋 중 하나 — 이음새 · 탭바 · 그 밖은 새 창**(R7 · R21 · R22 · Q3 · Q4) — §1-2 · §1-3.
- **크기 = 새것 1/3 · 기존 2/3 하나로, 예외 없음 · 막는 것은 면적 0 뿐**(R7 · R8) — §1-2.
- **붙는 단위 = 끈 탭 하나의 슬롯 배치 통째**(R9) — §1-2.
- **Esc 없음**(R12) — 웹뷰는 Esc 를 듣지 않고 세션에 취소 사건이 없다.
- **직전 도킹 되돌리기 = 비목표**(Q2).
- **끌기 주체 = 셸, OS 의존은 platform 의 목적 단위 함수로**(R18 · R19 · 「감싸기 잘해야겠네」) — §1-3 · §1-4.
- **슬롯 하나 직접 끌기 · 슬롯 가운데 칸 · Ctrl 끌기는 범위 밖**(PRD §2).

### 6-2. PRD 기본값으로 따른 것 (사용자가 바꿀 수 있다)

- **D3** main 의 마지막 탭은 끌기가 시작되지 않는다(R14) — `lease_tab` 거절 + 웹뷰 즉시 커서.
- **D4** 같은 창 다른 탭 합치기 = 머물면 탭 전환(R6b · R15) — 대기 500ms.
- **D6** 크기 기억 · 마지막 도킹 자리 = 나중 — 설계에 자리를 두지 않았다.
- **D7** 띠 겹침 = 바깥 띠 우선 · 정규화 거리(R6a) — §2-2 규칙 6 · 8.
- **D8** 마우스로 놓은 뒤 대상(새) 창 활성화 + 첫 슬롯 포커스(R11b) — `FocusPolicy::Reveal` + `set_focus`. 명령은 바꾸지 않는다(새 창도 활성 없이 연다).
- **D9** 끄는 중 같은 탭을 옮기는 명령은 실패 — `DockError::Dragging`(표식 · 같은 락 판정).
- **D10** 웹뷰 안 화면 상태 이전 = 비목표 — §5 끝 줄.
- **D11** 비정상 끝 = 출처 창 탭바로(= 무변경 — §1-3) · Esc 없음.
- **D12** 새 창 크기 = 출처 창 크기 · main 에서 떼었으면 팝아웃 기본 크기 — §1-2 `plan_new_window`.
- **R5 바깥 띠 두께** — PRD 가 「값은 TRD」로 넘겼다 → 32 CSS px(§2-3).
- **R13c** 놓을 때 출처 배치가 바뀌었거나 가이드를 보인 뒤 대상 배치가 바뀌었으면 무변경 — `expect_source` · `expect_target`(가이드는 대상이 바뀌면 다시 그리고 기대값도 바꾼다 — 「가이드를 보인 뒤」 = 마지막으로 보낸 가이드 뒤).

## 7. 내가 정한 내부 선택 (보고)

**구조**
1. **끌기 세션을 `layout` 밖 신설 `dock` 모듈에 둔다** — 세션은 레이아웃 권위를 부르는 손님이다.
2. **네이티브 이동 루프 · 창 서브클래스를 쓰지 않는다** — 끌기가 웹뷰에서 시작하므로 OS 루프의 시작 · 끝 감지가 필요 없다.
3. **세션 = 순수 `step(상태, 사건, 스냅숏) → 할 일`** — 가짜 시계 · 포인터 · 창으로 단독 시험이 선다(ADR-0012).
4. **끌기 스레드 하나 · 무제한 채널 · 마감 기반 틱** — 세션 상태에 뮤텍스가 없고, 제어 사건을 버리지 않으며, 사건이 틱을 밀지 않는다.
5. **틱 8ms(목표) · 실제 주기는 스파이크가 적고, 모자라면 platform `fine_timer` 를 끌기 동안만 쥔다** — 요구를 바꾸지 않는 조정(3판 리뷰 F6).
6. **`close_tab` 의 「목록에서 빼기 · 활성 승계 · 빈 창 판정」을 도우미로 뽑아 도킹과 나눠 쓰고, 승계는 순수 함수로도 내보낸다** — 승계 규칙이 모델 · 떼어진 표시 · 판정에서 한 벌이다.

**레이아웃 연산 · 명령**
7. **끼우는 서브트리의 슬롯 id · 분할 id 를 그대로 둔다(이동) · 탭바 · 새 창은 View 째 옮겨 탭 id 도 그대로다** — ADR-0223 「분할 id 는 노드를 따라간다」 · 곁표가 뷰 키로 따라간다(PRD 4.1 R16).
8. **R22 에서 출처가 탭 하나뿐인 팝아웃이면 새 창 대신 그 창을 옮긴다** — 결과(그 탭만 든 창 · 출처 창 크기)가 같고, 웹뷰를 새로 띄우지 않아 바로 보이며 출력 경로가 안 바뀐다. 명령 ③ 도 같고 답의 `window` 가 그 창 label 이다.
9. **결과 트리를 새로 만드는 순수 함수 하나(`docked_tree`)를 가이드 · 판정 · 적용이 같이 쓴다** — 「가이드 = 결과」(R4)가 식 하나로 선다.
10. **새 면적 0 = 원래 면적이 있던 칸이 결과에서 0** — `set_split_ratio` ⑤ 와 같은 태도.
11. **사전조건을 호출자 중립 값(`DockGuard`) · 포커스 정책(`FocusPolicy`)으로 · 표식은 번호만 대조** — 모델이 「마우스」 · 「세션」을 모른다.
12. **출처 대조 = 레이아웃 값 비교(시작 때 복제)** — 뷰별 변경 번호는 올리는 자리를 하나만 빠뜨려도 조용히 틀린다. 트리가 작아 복제 · 비교 비용은 무시할 만하다.
13. **명령 하나(`tab.dock`) + 모드 · 선택 칸** — PRD R16 그대로. 매크로가 필드 있는 enum 을 못 실어 선택 칸 + 조합 검사로 받는다(`layout.setSlotContent` 선례).
14. **레이아웃 적용 서비스의 첫 타입드 오류 · 거절은 ok 의 결말 칸이 아니라 오류** — 버스가 `NOT_FOUND` / `CONFLICT` 를 가르고, `ok` 만 보는 호출자가 거절을 성공으로 읽지 않는다.
15. **`DockSide` 는 serde rename 없이 두 표면이 같은 철자** — 맞대기 시험을 둔다.
16. **탭바 `index` 가 넘치면 맨 뒤로 자른다(오류 아님) · 같은 창 같은 자리면 무변경 `Ok`**.
17. **창 포트의 새 메서드는 기본 본문 없이** — 버스 경로 `OwnedWindowHost` 가 말없이 기본 동작으로 떨어지는 것을 컴파일러가 막는다(3판 리뷰 F2).
18. **버스 NewWindow 의 x/y 는 `placement::land` 로 검증하고 안 걸치면 기본 자리** — 저장 자리 복원과 같은 규칙(3판 리뷰 F10).
19. **(M2) 팝아웃 닫기 요청에서 모델을 먼저 지우고 label 에 묘비** — 닫히는 창으로의 확정을 타입드 오류로 막고, 새 창 phase C 는 묘비 앞에서 멈춘다. 팝아웃의 닫기 요청은 되돌려지지 않는다는 것이 근거다(§1-2). 팝아웃 `slot.popout` 의 같은 틈도 함께 닫는다. 사용자 확인 대상(§11-1 #2) · ADR-E.

**세션 동작**
20. **(M1 · 4판 정정) 놓기 = 떼는 순간 새 스냅숏으로 그 점을 한 번 판정 · 지금 결과가 「가이드 없음」이면 무엇을 보냈든 R22 · 이음새 · 탭바면 마지막으로 보낸 가이드와 같을 때만 적용, 다르면(`Unset` 포함) 무변경** — PRD R22(「이음새 · 탭바가 아닌 곳에 놓으면 새 창」)를 그대로 지키면서, 못 본 이음새 · 탭바에 놓이지 않는다. 판정 재료를 마지막 틱 것이 아니라 떼는 순간 것으로 하는 것은 그 사이 대상이 사라졌을 때 없는 칸으로 판정하지 않기 위해서다(3판 리뷰 N4).
21. **포인터는 platform `pointer()` 로 직접 읽고, 결과를 `Read` · `Unavailable` · `Unsupported` 셋으로 가른다** — Tauri `cursor_position` 은 메인 스레드 왕복(`tauri-runtime-wry` `lib.rs:2815-2820`)이고, 「지금 못 읽음」과 「이 OS 에 없음」을 한 값으로 접으면 보안 데스크톱의 0 이 떼기로 읽힌다(3판 리뷰 F7).
22. **틱 순서 = 전경 → 포인터 → 떼기 판정 · 전경은 새 함수 `foreground()` 의 세 갈래로** — 전경이 확실히 다른 프로세스면 곧바로 비정상 끝, 전경 불명(빈 전경 · 읽는 사이 사라짐)은 유예. 출처 팝아웃을 숨기거나 딱지를 보이는 순간의 빈 전경을 「다른 앱」으로 읽어 끌기를 끊지 않는다(3판 리뷰 N2). 옛 `foreground_is_current_process` 는 답을 바꾸지 않는 감싸기로 남긴다 — 그 호출자에게는 「불명 = 아니다」가 맞다.
23. **`Unavailable` · 전경 `Indeterminate` 가 한 시계로 500ms 이어지면 비정상 끝** — 끝나지 않는 끌기를 막는다.
24. **놓기 신호 = `Read` 의 떼기 ∨ 웹뷰 `Released`, 먼저 온 것 · 웹뷰 `Lost` 는 힌트** — 숨긴 창 · 캡처 흔들림이 끌기를 끊지 않는다.
25. **탭 전환 대기 500ms · 끝이 무변경이어도 전환은 되돌리지 않는다** — §11-1 #5.
26. **출처가 바뀌면 그 뒤로 띠 · 탭바 자리를 그리지 않고 딱지에 「안 됨」**.
27. **출처 View 가 사라지면 놓기를 기다리지 않고 끝낸다** — 없는 탭의 딱지가 남지 않게.
28. **끌기 문턱 4px** — Windows 기본 끌기 문턱(`SM_CXDRAG` 4)과 같은 크기다.
29. **`Ended` 는 적용 알림 뒤에 · 숨긴 출처 창은 `Ended` 뒤에 되보인다 · `dock:session` 은 세션 안 `seq` 로 순서를 판정** — 탭이 한 번 비쳤다 사라지거나, 옛 `Display` 가 새 표시를 덮지 않게.
30. **시작은 「실패할 수 있는 준비 → 표식 → 부수 효과」 순서 + 되감기 가드** — 어떤 단계에서 실패해도 표식 · 숨긴 창 · 멈춘 자리 기록이 남지 않는다(3판 리뷰 F1).

**판정 재료 · 그리기**
31. **크롬(탭바 · 탭 머리 · 캔버스 원점)은 평소 보고하고 창 항목에 측정으로 둔다 · 판정은 출처 탭을 id 로 거른다** — 보고가 떼어진 표시보다 늦어도 틀리지 않는다(3판 리뷰 F9).
32. **창 기하(안쪽 원점 · 크기 · 배율)는 끌기 시작 때 한 번 + 그 창의 기하 사건 때만 다시 읽는다**.
33. **오버레이는 바뀔 때만 보내고, 끝 방송은 무조건 지운다**.
34. **떼어진 표시는 렌더 값 · 스토어 탭 목록은 건드리지 않음 · 탭 하나 팝아웃은 표시 없이 창째 숨김** — 0탭 자가닫힘을 건드리지 않는다(3판 리뷰 F3).
35. **딱지 = 별도 창 하나(처음 끌기 때 만들어 재사용) · 불투명 · 최소 페이지 · 입력 투과 없음** — 출처 창이 사라지는 끌기에서 딱지를 그릴 자리가 우리 창 밖뿐이다(W7). 만들기가 S4 합격선(300ms)을 넘으면 부팅 뒤 미리 만든다.

**OS 층**
36. **platform 함수 = `topmost_at(점, 후보, 투과)` 하나가 「맨 위가 우리 창인가 · 어느 창인가 · 딱지를 빼고」를 다 답한다** — R19 의 예시 문장 그대로의 목적 단위.
37. **창 사각형 = DWM 확장 틀** — 보이지 않는 크기 조절 테두리를 맞혀 옆 창을 고르는 오판을 막는다.
38. **핸들 = platform 이 소유한 `NativeWindow(isize)` · 들어오는 문은 `raw-window-handle` 하나** — Send 라 끌기 스레드로 옮길 수 있고, 셸이 OS 변형을 열어 보지 않는다(사용자 확인 대상 — §11-1 #3).
39. **Windows 밖 `Unknown` → 최근 포커스 순 근사 · `Unsupported` → 웹뷰 신호** — 부르는 쪽에 OS 분기 없이 macOS 구현 전까지 돈다.

## 8. 스파이크 계획

- **어디서** — 버리는 스파이크 브랜치(예: `v0.4.1/spike/dock-*`)에서 앱을 런처로 띄워(CLAUDE.md 「GUI 실측」 — 셸에서 직접 띄우지 않는다) 잰다. 코드는 머지하지 않고 결과만 이 문서 개정으로 남긴다.
- **언제** — §10 1–6단계는 스파이크와 무관하다. 다섯 다 7단계(platform) 앞에 끝낸다. S2 는 진행 중이다(결과는 메인이 접어 넣는다).
- **★불합격 = 멈춤★** — 아래 「불합격이면」의 조정은 **요구를 바꾸지 않는 것**만 적었다. 그것으로도 못 넘으면 구현을 멈추고 PRD 개정(사용자 결정)으로 올린다. 요구를 못 채우는 대비책으로 조용히 물러나지 않는다.
- 판정 기준의 수치는 이 문서가 정한 첫 값이다 — 재 보고 비현실적이면 개정한다(개정도 사용자에게 보고한다).

| # | 위험 | 무엇을 어떻게 | 합격 기준 | 불합격이면 |
|---|---|---|---|---|
| **S1** | W1 — 끄는 동안 입력 · 출처 창 숨기기 | 탭에서 끌기 시작 → 탭 하나 팝아웃을 후보 ⓐ ⓑ ⓒ(§4-1)로 안 보이게 → 우리 창 위 · 다른 앱 창 위 · 바탕화면 · 다른 모니터에서 떼기, 각 20회(왼손잡이 설정 포함). 재는 것: ① `pointer()` 시제품이 떼기를 한 틱 안에 보나 ② 끄는 동안 전경이 우리 프로세스인가 ③ 되보일 때 원래 자리 · 크기 · 최대화인가 · 그 뒤 탭 클릭이 정상인가(캡처 잔류 없음) ④(정보) 숨긴 창 웹뷰가 `pointerup` · `pointercancel` · `blur` 를 받나 ⑤(정보) UAC 를 띄워 끌기가 비정상 끝으로 끝나나(떼기로 읽히지 않나) | ① 20/20 · 후보 하나 이상이 ② ③ 20/20 | ① 불합격 = **멈춤** — 떼기를 셸이 못 보면 R13b(끝나지 않는 끌기 금지)를 못 지킨다 · ② ③ 을 넘는 후보가 없음 = **멈춤** — R2(창이 사라진다)나 R13b 의 「포커스 잃음」 갈래를 PRD 에서 고쳐야 한다 |
| **S2** | W3 — 맨 위 창 판정 · 틱 비용 | `topmost_at` 시제품: 우리 창 셋을 겹치고 + 메모장 하나를 그 사이에 + 최소화 창 + 다른 가상 데스크톱의 우리 창 + 딱지 창(투과)을 두고, 화면 격자 점마다 답을 사람이 본 답과 대조. 최상위 창 300개 근처에서 10,000회 호출 시간 · 우리 창 넷에서 틱 전체 시간 · **실제 틱 주기(8ms 요청에 대해 깬 간격) 기록** | 격자 전 점 일치 · `topmost_at` **p99 ≤ 1ms · 최대 ≤ 4ms** · 틱 전체 **p99 ≤ 4ms**(8ms 주기의 절반) | 판정이 틀리면 거름(`WS_EX_TOOLWINDOW` 등)을 더해 다시 잰다. 느리면 커서가 움직였을 때만 부르고(이미 그렇다) 틱을 16ms 로. 그래도 안 되면 **멈춤**(R15) |
| **S3** | W5 — tauri #10767 | 팝아웃 제목줄을 main 위로 끌어 놓고(R1) 곧바로 탭 끌기 10회. 그 사이 포커스 · 마우스 사건이 뒤집히거나 삼켜지는지 | AC1(가이드 없음) + 뒤이은 탭 끌기 10/10 정상 | 이 설계는 `start_dragging` 을 안 부르므로 해당될 길이 좁다. 걸리면 탭 끌기 시작 전에 웹뷰 포커스를 다시 잡는다(`window.focus`). 그래도 안 되면 **멈춤**(R1) |
| **S4** | W7 — 딱지 창 | 숨긴 불투명 최상위 창(§4-2 빌더)을 처음 만들 때 걸리는 시간 · 보이기 후보 ⓐ ⓑ ⓒ 각각의 첫 프레임까지 · 틱 `set_position` 으로 빠르게 · 느리게 · 모니터 경계 넘기 각 10회를 60fps 녹화 · **실제 틱 주기 기록(`fine_timer` 없이 · 있으면)** · 끄는 동안 전경 · 출처 창 활성 · `topmost_at` 투과 · Alt+Tab · 작업 표시줄 · (변수) 입력 투과가 필요해지는가 | 만들기 ≤ 300ms(처음 한 번) · 보이기 ≤ 1 프레임 · 커서와의 지연 중앙값 ≤ 1 프레임 · 깜빡임 0 · 전경이 우리 프로세스 20/20 · Alt+Tab · 작업 표시줄에 안 보임 · 판정 오염 0 · 끄는 동안 CPU ≤ 한 코어의 10% | 지연만 넘으면 `fine_timer` 를 들인다. 만들기만 느리면 부팅 뒤 미리 만든다(§7 #35). 보이기 후보가 다 불합격이면 **멈춤** — R2(우리 창 밖에서도 딱지) · W7 을 PRD 에서 고쳐야 한다 |
| **S5** | W4 — 배율이 다른 모니터 | 100% · 150% 모니터 둘. 각 모니터의 창 · 두 모니터에 걸친 창에서 칸 경계 ±1px 를 커서로 짚어 판정 대조. 다른 모니터 빈 곳에 놓아 새 창(R22) · 딱지가 모니터를 넘을 때 크기 | 판정이 그려진 띠 경계와 ±1 CSS px 안에서 일치(AC12) · 새 창이 놓은 모니터에 R22 크기(±2 논리 px)로 뜸 · 딱지 크기가 한 프레임 넘게 튀지 않음 | 걸친 창의 배율이 웹뷰 배율과 다르게 읽히면 크롬 보고에 웹뷰 `devicePixelRatio` 를 실어 그 값을 쓴다. 그래도 안 되면 **멈춤**(AC12) |

## 9. 테스트 계획

층마다 seam 을 끊어 단독으로 잰다(ADR-0012). 대상 타깃은 CLAUDE.md 「빌드 · 검증 명령」의 기존 타깃이고 새 타깃은 만들지 않는다.

| 층 | 무엇 | 어디(타깃) |
|---|---|---|
| 트리 연산 | `docked_tree` — 네 변 × (슬롯 · 바깥) 모양 · 비율 1/3 · 2/3 · 감싼 쪽 안쪽 비율 보존(AC4 · AC5) · 슬롯 id · 분할 id 보존 · 없는 슬롯 · **새** 면적 0(깊은 자식 · 반복 도킹으로 표현 한계에 닿는 칸) · 원래부터 면적 0 인 칸은 거절 사유가 아님 · `incoming_rect` 가 결과 기하의 외접 사각형과 비트 단위로 같음 | `src/layout/dock.rs` `#[cfg(test)]` → `cargo test -p engram-dashboard --test lib_unit` |
| 모델 | `dock` 세 모드 — 판정 순서(앞 사유가 이김) · 사유마다 무변경(`version` 그대로) · 성공마다 `version` +1 한 번 · Seam 곁표가 대상 View 로 따라감 · 탭바 자리(넘침 = 맨 뒤 · 같은 창 자리 옮기기 · 같은 자리 무변경) · 출처 창 활성 승계 · 빈 팝아웃 항목 제거 · main 마지막 탭 거절 · `Keep` 은 대상 활성 · 포커스 그대로 · ★같은 창에서 출처가 활성이면 대상이 활성★ · `Reveal` 은 대상 활성 + 첫 슬롯 포커스 · 표식(`Dragging` · `StaleLease` · 성공이 표식을 내림 · `release_lease` 의 다른 번호 무동작) · `SourceChanged` · `TargetChanged`(배치 · 탭 목록) · 묘비(`mark_closed` 뒤 NewWindow = `WindowClosed` · `attach_view_as_new_window` 도 거절) · `plan_new_window`(재사용 · 크기 · main = None) · `succession` 이 `close_tab` 과 같은 답. **매 시험 뒤 기존 `assert_invariants`**(`manager.rs:1305` — ADR-0057 ①–④) | `src/layout/manager.rs` `#[cfg(test)]` → `lib_unit` |
| 리팩터 회귀 | `close_tab` 에서 뽑은 도우미 — 기존 `close_*` 시험(`manager.rs:1498-1631`)이 그대로 초록 | `lib_unit` |
| 닫기 요청 | `drop_window_in_model` 을 닫기 요청 계기로 부른 뒤: 그 창 탭이 모델에서 사라짐 · 묘비 · 뒤이은 `Destroyed` 정리가 멱등 · 그 뒤 Seam(`TabNotFound`) · TabBar(`WindowNotFound`) · 새 창(`WindowClosed`) · 팝아웃 기존 창 타깃(롤백) · 팝아웃 새 창 타깃(롤백) 전부 무변경 | `src/commands/popout.rs` `#[cfg(test)]`(창 없이 서는 모델 몫 — 그 파일 머리) → `lib_unit` |
| 적용 서비스 | 가짜 포트로: Seam · TabBar 의 변경 · 스냅숏 · `resync` 가 한 락 안 · 알림은 락 밖 · 출처 창이 닫히면 `host.close` 가 알림 뒤 한 번 · 실패면 알림 0 · `resync` 0 / NewWindow — 열기 실패 = 모델 무변경 · phase C 실패(묘비 포함) = 연 창을 닫고 탭은 출처 · 재사용 = `move_window` · 모델 무변경 · `activate` 가 `FocusPolicy` 를 따름 · 어느 모니터에도 안 걸치는 자리 = 기본 자리 | `tests/layout_apply.rs` → `cargo test -p engram-dashboard --test layout_apply` |
| 버스 | `tab.dock` 선언(이름 · 인자 · 답 · 오류 목록) · 모드 칸 조합 불량 = `INVALID_ARGUMENT` · 사유 → 코드 표(§3-2) 전부 · id 형식 불량 · `DockSide` · `DockMode` 두 표면 철자 맞대기 · `catalog_version` 18 · ★NewWindow 가 x · y · 크기 · `activate = false` 를 창 포트 `open_at` 에 그대로 넘김(3판 리뷰 F2 — 가짜 `Windows` 가 받은 인자를 적는다)★ | `tests/layout_commands.rs` → `--test layout_commands` |
| 판정 | `locate` · `choose_band` — 각 변의 띠 · 가운데(R6 → `Nothing`) · 모서리 정규화 거리(R6a ②) · 바깥 띠가 이김(R6a ① · AC4) · 경계선 위 결정성(R6a ③) · 탭바 자리(가운데 x 규칙 · ★크롬에 출처 탭 머리가 남아 있어도 id 로 걸러 같은 자리★) · 탭 머리 `over` · 캔버스 밖 · 배율 1.0 / 1.5 / 걸친 창(W4) · 빠진 띠(면적 0)에서는 안 붙음(AC15) | `src/dock/hit.rs` `#[cfg(test)]` → `lib_unit` |
| 세션 | 가짜 전경 · 포인터 · 시계 · 맨 위 창 · 스냅숏으로 `step` 의 할 일을 단언: 시작 → 띠 → 놓기 = 적용 1회(`Reveal` · 표식 · 기대값) + `set_focus` · **(M1 · 4판) 지금 결과가 가이드 없음(슬롯 가운데 · 칠하지 않은 띠 · 우리 창 밖 · 다른 앱 위) → 보낸 것이 이음새 · 탭바 · 가이드 없음 · `Unset` 어느 것이든 새 창 · 지금 결과가 이음새 · 탭바이고 보낸 것과 같음 → 적용 · 다름 → 무변경(`TargetChanged`) · `Unset` 에서 이음새 · 탭바 → 무변경** · **(N4) 판정은 떼는 순간 스냅숏을 씀 — 마지막 틱 스냅숏과 다른 떼기 스냅숏(그 사이 대상 칸이 닫힘 · 탭이 바뀜)을 먹이면 결과가 떼기 스냅숏을 따른다** · **(F7 · N2) `Other` 틱의 버튼 0 = 곧바로 비정상 끝(놓기 아님) · `Indeterminate` 는 유예 — 그 사이의 버튼 0 은 떼기가 아님 · 유예 안에 `Ours` 로 돌아오면 이어짐 · `Indeterminate` 와 `Unavailable` 이 섞여 합쳐 500ms 면 비정상 끝 · `Unsupported` 는 웹뷰 `Released` 에만** · `Lost` 갈래 · 출처 창 `Destroyed` = 무변경 · 되보이기 없음(AC16) · 대상 창 `Destroyed` = 가이드 지움 · 출처 변경 → 띠 사라짐 · 놓아도 무변경(AC16 둘째 문장) · 대상 변경 → 가이드 다시 그림 · 전환 대기 500ms(R6b · AC14) · 출처 탭 목록 변경 → `Display{seq}` · 끝마다 `Ended` → 되보이기 → `release_lease` 순서 · 늦은 세션 번호 버림 · 마감 계산(사건이 틱을 안 밂 · 밀렸을 때 몰아치지 않음) | `src/dock/session.rs` `#[cfg(test)]` → `lib_unit` |
| 구동 | 채널이 제어 사건을 버리지 않음(기하 사건을 몰아넣은 뒤 `End` 가 처리됨) · 기하 사건 합치기 · **시작 되감기(F1) — 준비 단계 실패마다 표식 0 · 숨김 0 / 부수 효과 뒤 실패마다 가드가 표식 해제 · 되보이기 · 자리 기록 재개 · `Ended` 를 함** | `src/dock/driver.rs` 의 순수 조각(가짜 포트) `#[cfg(test)]` → `lib_unit` |
| OS 층 | `topmost_at` 의 z 순서 걷기를 내부 함수로 떼어 가짜 창 목록으로(투과 · 최소화 · 가림 · 입력 투과 · 사각형 밖 · 후보 아님 = `Other`). 실 창 시험: 창 둘을 만들어 겹쳐 놓고 실제 답 대조 · `pointer()` 가 Windows 에서 `Unsupported` 가 아님 · 그 밖에서 `Unsupported`(짝 시험) · `foreground()` 의 갈래 판정을 PID 입력의 순수 함수로 떼어(무효 핸들 · PID 0 = `Indeterminate` · 같은 PID = `Ours` · 다른 PID = `Other`) · 옛 `foreground_is_current_process` 가 같은 입력에 옛 답(`Indeterminate` → `Some(false)`)을 그대로 줌 · `NativeWindow: Send + Sync`(컴파일 단언). `#[cfg(windows)]` 는 그 시험 함수에만(파일째 막지 않는다 — CLAUDE.md 「플랫폼 중립」). **★실 창 시험은 `-- --test-threads=4` 에서 겹친다★** — 그 시험들끼리 프로세스 안 `static` 락 하나로 줄 세우고, 다른 앱 창과 겹치지 않게 주 모니터 밖 먼 가상 좌표(예: (-20000, -20000) 근처 · 시험마다 다른 칸)에 창을 둔다. CI 러너에서 창 만들기가 서는지는 **모른다** — 첫 CI 실행에서 보고, 안 서면 그 시험만 `#[ignore]` + 로컬 `/qa` 몫으로 돌린다. 창 만들기용 `windows` feature(예: `Win32_System_LibraryLoader`)는 dev 의존에만 켠다(「의존」 규칙은 운영 의존만 본다) | platform `src/window.rs` · `src/input.rs` `#[cfg(test)]` → `cargo test -p engram-dashboard-platform -- --test-threads=4` |
| 프론트 | `TabBar` — 4px 문턱 · 끌기 뒤 click 삼킴 · main 마지막 탭은 시작 요청 없이 「안 됨」 커서 · `pointerup` = `Released` · `pointercancel` · `blur` = `Lost` · `lostpointercapture` · Esc 는 보내지 않음 · 시작 거절이면 누름 상태를 버림 / ★떼어진 표시(F3) — 끄는 동안 스토어의 `win.tabs` 가 그대로 · 0탭 자가닫힘이 불리지 않음 · 탭 하나 팝아웃은 표시를 안 함 · 탭 머리 숨김 · `displayActive` 로 캔버스 · 오래된 `seq` 버림 · `Ended` 로 거둠★ / `DockOverlay` — 페이로드대로 그림 · 세션이 다르면 무시 · `Ended` 면 지움 / `dockChromeReport` — 한 번에 하나 · 마지막 값이 덮음 / 딱지 페이지 — `dock:chip` 대로 글자 · 테마 · 「안 됨」 / 레지스트리 `tab.dock` 인자 · invoke 이름 | vitest(`npm test`) + `npx tsc --noEmit` |
| 생성물 | 새 ts-rs 타입(`src-tauri/bindings/`) | CI 생성물 sync 게이트(`lib_unit` 이 굽는다) |
| 화면(QA) | AC1–AC16 · AC18 전부를 실제 창으로 — `/qa full` 의 cdp 실측 + 팝아웃 X 와 명령 도킹 겹치기(M2) · 문턱 직후 떼기(M1). 배율 둘 모니터(AC12)는 사람 손 확인이 필요할 수 있다 | `/qa full` |

- **게이트** — 새 OS 분기는 platform 안뿐이라 4f · 4g · 4h 명단이 안 바뀌어야 한다(바뀌면 설계 위반 신호). 셸 → agent 운영 의존 0(4i) 그대로. `cargo fmt --check` · 워크스페이스 회귀(`-- --test-threads=4`).
- **M2 전제 점검(QA · 3 단계부터 매 `/qa`)** — 팝아웃 닫기가 되돌려지지 않는다는 전제(§1-2 「닫히는 창」)를 지키는 두 줄: `rg "onCloseRequested|close-requested" src/` → **0줄**(프론트 닫기 요청 리스너 없음) · `rg "prevent_close" src-tauri/src/` → **1줄**(`lib.rs` 의 main 갈래 — 실측 2026-10-15 `lib.rs:304`). 어긋나면 M2 의 틈이 다시 열린 것이라 그 변경을 막고 §1-2 를 「닫는 중 표시」 안으로 고친다. 같은 두 줄을 `CloseRequested` 처리기 주석에도 적는다.
- **시험이 못 덮는 것** — 실제 포인터 · 창 숨기기 · WebView2 렌더링 · 다중 모니터 · `lib.rs` 의 창 사건 배선은 스파이크와 `/qa full` 화면 실측 몫이다. 시험은 명세한 동작의 회귀 안전망이지 완전성 보장이 아니다(CLAUDE.md 「개발 스텝」 4).

## 10. 이행 순서

★커밋마다 빌드 · 시험 초록 — 어디서 멈춰도 선다★. 자료구조를 먼저 갈아엎고 부르는 쪽을 나중에 맞추는 순서는 쓰지 않는다 — 각 단계는 새 것을 **더하고** 그 단계 안에서 부르는 쪽까지 닫는다. 새 함수가 아직 부르는 쪽이 없는 단계도 `pub` 이라 경고 없이 선다. 커밋 제목은 `S21: <타입>(<범위>): …`.

| 단계 | 내용 | 스파이크 의존 | 멈추면 남는 것 |
|---|---|---|---|
| **1** | `layout/dock.rs` — `DockSide` · `DockAnchor` · `docked_tree` · 새 면적 0 판정 · `incoming_rect` + 시험 | 없음 | 부르는 쪽 없는 순수 함수(무해) |
| **2** | `ViewManager` — `close_tab` 도우미 · `succession` 추출(기존 시험이 지킴) → `dock`(세 모드) · `plan_new_window` · 표식(`lease_tab` · `release_lease`) · 묘비(`mark_closed` · `attach_view_as_new_window` 의 묘비 검사) · `DockGuard` · `FocusPolicy` · `DockError` + 시험 · `adopt_restored` 구조 분해에 새 칸 둘 | 없음 | 모델 연산(아직 아무도 안 부름 — 묘비도 아무도 안 단다) |
| **3** | ★닫히는 창 조정(M2) — 4 단계와 `tab.dock` 의 어떤 출시보다도 먼저인 **필수 선행**★: `drop_window_in_model` 이 `mark_closed` 를 부르고, `lib.rs` `CloseRequested` 가 팝아웃이면 그것을 부른다(되돌려지지 않는다는 전제와 그 점검 명령을 그 자리에 주석으로 — §9 「게이트」) · `apply.rs:105-118` 주석 갱신 + 시험. §11-1 #2 에서 사용자가 M2 를 거절하면 이 단계 대신 2판의 두 검사(Seam · TabBar 의 `is_open` 사전 확인 · NewWindow 의 확정 뒤 `is_open` 확인과 모델 정리)를 4 단계에 되살린다 — 둘 중 하나 없이는 출시하지 않는다 | 없음 | 팝아웃 닫기의 기존 틈이 닫힌 셸(도킹 없이도 값어치) |
| **4** | ★3 단계(또는 그 대체 검사)가 들어간 뒤에만★ 적용 서비스 `apply::dock_tab`(Seam · TabBar 한 락 · NewWindow 세 phase · 재사용) + `WindowHost::{open_at, move_window}`(★기본 본문 없이 — 구현자 여섯을 같은 커밋에서 닫는다★) + Tauri 구현(`popout.rs` · `OwnedWindowHost` 넘기기 · `placement::place` · `land` 공개) + invoke `dock_tab`(async · `commands/dock.rs` · `generate_handler!`) + 버스 `tab.dock`(세 모드 한 번에 · `catalog_version` 18) + `commands.rs` 머리 갱신 + 프론트 레지스트리 · `viewStore.dockTab` + 시험(`layout_apply` · `layout_commands` · vitest) | 없음 | **LLM 도킹이 끝까지 된다(AC10 · AC13 · AC18 의 명령판)** — 마우스 없이도 출시 가능한 덩어리 |
| **5** | `dock/hit.rs` + `dock/types.rs`(ts-rs) + 시험 | 없음 | 순수 판정 |
| **6** | 크롬 보고 — `DockChrome` 측정 칸(`WindowTabs`) · `set_dock_chrome` · invoke `report_dock_chrome` · `dockChromeReport.ts` + 시험. `WindowTabs::new` 에 `chrome: None`, `state/convert.rs:75-83` 의 구조 분해에 `chrome: _`(측정은 저장하지 않는다) · 그다음 `dock/session.rs` 순수 상태 기계 + 시험 | 없음 | 평소 보고 · 순수 세션 |
| — | **스파이크 S1–S5**(§8) — 결과를 이 문서에 개정으로 적는다. 불합격이면 여기서 멈추고 사용자에게 올린다 | — | — |
| **7** | platform — `raw-window-handle` 운영 의존(★§11-1 #3 사용자 확인 뒤★) + `windows` feature + `window::{NativeWindow, native_window, ScreenPoint, TopmostAt, topmost_at}` · `input::{Pointer, PointerRead, pointer}`(+ S1 · S4 가 OS 함수를 고르면 그것 · S2 · S4 가 요구하면 `timer::fine_timer`) + 시험 + platform `src/lib.rs` 헤더(입주자 · 의존 규칙) · `Cargo.toml` 주석 · CLAUDE.md 「의존성」. ADR-C 를 같은 단계에서 박는다 | S1 · S2 · S4 | 부르는 쪽 없는 OS 함수 |
| **8** | 딱지 창 — `dock-chip.html` · `src/dockChip/main.ts` · Vite 입력 · `capabilities/dock-chip.json` · `dock/chip.rs`(빌더 — `WebviewEnv::finish`) · `lib.rs` 의 자리 기록 거름 + 시험 · CLAUDE.md 「창 구성」(런타임 창에 딱지 한 줄 — `/review doc`) | S4 | 쓰는 쪽 없는 숨은 창 빌더 |
| **9** | `dock/driver.rs` — 끌기 스레드 · 시작 되감기 가드 · invoke `dock_drag_begin` · `dock_drag_end` · 창 사건 연결(`lib.rs`) · 출처 숨기기(S1 방법) · 자리 기록 멈춤(`placement.rs`) · emit · 계측(§1-7) + 프론트 `TabBar` 끌기 · 떼어진 표시 · `DockOverlay` — **마우스 도킹이 끝까지 된다** | S1 · S2 · S3 · S5 | 마우스 도킹 |
| **10** | 문서 — ADR(§12) · step-log · `/qa full` 화면 실측(AC 전부) | — | — |

- 매 단계 게이트 = `/implement`(구현 → `/review code` → `/qa`) — CLAUDE.md 「구현 실행 규약」.
- **나누는 축 = 파일 겹침** — 1 → 2 → 3 → 4 는 `layout/*` · `commands/*` · `lib.rs` 가 겹쳐 **한 코더에게 순서대로**. 5 는 1 만 기대어 2–4 와 병렬로 맡길 수 있다(겹치는 것은 `lib.rs` 의 `mod dock;` 한 줄 — 먼저 끝난 쪽이 넣는다). 6 은 2 · 5 뒤. 7 · 8 은 서로 안 겹쳐 병렬. 9 는 마지막.
- ★3 → 4 순서는 바꾸지 않는다★ — 4 의 적용 서비스는 닫히는 창을 3 의 모델 먼저 지우기 · 묘비에만 기대어 막는다(사전 · 사후 `is_open` 확인을 두지 않는다). 3 없이 4 를 내보내면 닫히는 창에 확정된 탭이 사라지고, NewWindow 는 OS 창 없는 모델 창을 남길 수 있다. 그래서 §11-1 #2 의 답을 4 단계 착수 전에 받는다.
- 4 단계 뒤가 첫 출시 가능 지점이다(LLM 도킹). 9 단계 뒤가 PRD 의 기본 흐름(팝아웃의 탭을 끌어 붙이기 — PRD §1 「사용자 흐름」) 완성이다.

## 11. 위험 · 미결

### 11-1. 사용자 확인 대상

1. **같은 창 안 도킹에서 출처가 보이던 탭이면 무엇이 보이나**(R16) — R16 「대상 창의 보이는 탭은 바뀌지 않는다」는 출처가 그 창의 보이는 탭이면 성립할 수 없다(출처가 사라진다). 기본값 = **대상이 보이게 된다**(이음새 = 대상 탭 · 탭바 = 옮긴 그 탭) — 보던 내용이 그 탭에 들어가 화면에 남는다. 다른 안 = 탭 닫기와 같은 승계(오른쪽 · 왼쪽 이웃).
2. **닫히는 창과 모델 변형의 조정 규칙**(§1-2 「닫히는 창」) — 기본값 = **팝아웃 닫기 요청에서 모델을 먼저 지우고 label 에 묘비** — 닫히는 창으로의 도킹 · 새 창 확정이 무변경이 되고, 팝아웃 `slot.popout` 의 「미해결 — 사용자 판단 대기」(`apply.rs:105-118`) 두 갈래도 함께 닫힌다. 대가 = 기존 팝아웃 닫기의 모델 정리 계기가 `Destroyed` 에서 `CloseRequested` 로 옮긴다(ADR-0057 불변식은 그대로 · ADR-E). 다른 안 = 2판의 「열림 사전 확인 + 남는 위험 기록」(명령 도킹과 사용자 X 가 몇 ms 안에 겹치면 붙인 탭이 그 창과 함께 사라진다). ★M2 를 거절하면 2판의 두 검사(Seam · TabBar 의 `is_open` 사전 확인 · NewWindow 의 확정 뒤 `is_open` 확인과 모델 정리)를 출시 전에 되살려야 한다★ — 4 단계는 M2 나 그 두 검사가 들어간 뒤에만 나간다(§10).
3. **platform 에 `raw-window-handle` 운영 의존을 들이기**(§1-4 · ADR-C) — platform 의 「운영 의존 서드파티 = `windows` · `tracing` 만」(TRD 1-3 §0 ① · ADR-0266 계열 규칙)을 고친다. 타입만 있는 crate · 새 패키지 0 · OS 바인딩 아님. 거부한 대안은 §12 ADR-C.
4. **R22 새 창 크기 — 출처 창이 최대화였을 때** — 기본값 = 그 창의 보통(최대화 전) 크기로 연다(화면 크기만 한 최대화 아닌 창이 되지 않게). 다른 안 = 최대화 상태 그대로.
5. **R6b 로 바꾼 탭은 끌기가 무변경으로 끝나도 그대로 둔다** — 기본값 = 되돌리지 않음(브라우저 · VS Code). 다른 안 = 끌기 시작 때 탭으로 되돌림.

**PRD 와 다른 점** — R18 의 Esc · R16 의 반환 id · R13b 의 닿지 않는 갈래는 PRD 4.1 에서 문구를 맞췄다. 남은 둘:
- R21 「원래 창 탭바에 놓으면 그 창의 탭으로 돌아간다」는 놓은 자리로 옮겨 넣으므로 탭 순서 바꾸기도 함께 된다.
- R13b 의 「앱이 포커스를 잃음」은 출처 창을 숨기는 방법이 전경을 다른 앱으로 넘기면 끌기 시작마다 걸린다 — S1 이 전경을 지키는 방법을 못 찾으면 이 갈래나 R2 의 「창이 사라진다」를 고쳐야 한다(§8 S1).

### 11-2. 기술 위험

| # | 위험 | 확신 · 대응 |
|---|---|---|
| 6 | 출처 창 숨기기가 전경 · 캡처를 흔듦 | 불확실 · S1 이 방법을 고른다. 셸은 웹뷰 입력에 기대지 않는다(포인터 직접 읽기) |
| 7 | 딱지 창 `show()` 가 활성을 가져감 | 불확실 · S4 가 보이기 방법을 고른다. 가져가도 전경은 우리 프로세스라 끌기는 끊기지 않는다(출처 창 제목줄만 비활성 색) |
| 8 | 실제 틱 주기가 15.6ms 로 묶임 | 가능성 높음 · S2 · S4 가 적고, 딱지 지연이 합격선을 넘으면 `fine_timer` |
| 9 | 도킹 직후 닫히는 팝아웃 웹뷰가 탭 0 개 순간을 그림 | 가능성 높음 · 무해 — 모델에서 창을 먼저 지운 뒤 OS 창을 닫는 순서는 `close_tab` 과 같다(§5). 탭 하나 팝아웃은 이미 안 보인다 |
| 10 | `EnumWindows` 가 다른 프로세스의 「보이지만 투명한」 오버레이(화면 녹화 도구 등)를 가림으로 칠 수 있음 | 불확실 · 입력 투과 스타일은 건너뛰고, 그 밖은 S2 격자에서 본다 |
| 11 | 같은 웹뷰 안 도킹에서 칸 컴포넌트가 다시 마운트되며 터미널이 재구독 | 확실(구조) · 팝아웃과 같은 수준(D10 비목표). 출력은 replay 로 이어진다 |
| 12 | Windows 밖 근사가 다른 앱 창에 가려진 자리를 대상으로 고름 · `Unsupported` 에서 웹뷰 떼기 신호를 잃으면 `Lost` 가 올 때까지 끌기가 안 끝남 | 확실(설계상 한계) · macOS 구현이 들어올 때 사라진다. Windows 출시에는 영향 없다 |
| 13 | 끌기 중 앱 종료 · 셸 패닉 | 셸 패닉은 훅이 곧 종료다(ADR-0292) — 모델 · 저장은 도킹 전 판이라 탭은 한 곳에 있다(§5). 끌기 스레드 안에서는 패닉하지 않게 짠다(`placement.rs` 머리 「패닉하지 않는다」와 같은 규율). 표식 · 묘비는 메모리라 다음 부팅에 없다 |
| 14 | 125회/초 짧은 레이아웃 락이 다른 쓰기와 경합 | 가능성 낮음 · 락 안에서는 값 복사만 하고 배치 복제는 `version` 이 올랐을 때만 한다. S2 의 틱 p99 로 함께 본다 |
| 15 | R22 새 창이 웹뷰를 띄우는 동안(수백 ms 추정) 화면에 아무것도 없음 | 가능성 높음 · 탭 하나 팝아웃은 재사용이라 해당 없음. 거슬리면 새 창이 보일 때까지 딱지를 놓은 자리에 남긴다(QA 에서 판단) |
| 16 | 닫기 요청 계기의 모델 정리가 기존 팝아웃 닫기 동작을 바꿈(정리가 OS 창 소멸보다 한 이벤트 앞선다) | 가능성 낮음 · 바뀌는 것은 정리 시점뿐이고 같은 함수 · 같은 락이다. 팝아웃 닫기를 막을 수 있게 되는 날 이 근거가 무너진다 — 처리기 주석과 §1-2 가 그 조건을 적는다 |
| 17 | 사용자가 X 를 누른 직후 먼저 확정된 명령 도킹의 탭이 그 창과 함께 닫힘 | 확실(닫기 의미 그대로) · 잔해는 없다(§1-2 (c)) |
| 18 | 문턱(4px)을 막 넘기고 캔버스 쪽에서 바로 떼면 새 창이 뜸 | 확실(PRD R22 그대로 — 가이드 없는 자리는 새 창) · 탭바 위에서 떼면 자리 옮기기(같은 자리 = 무변경)라 대부분 무해. QA 에서 거슬리면 문턱 값만 조정한다(요구는 그대로) |
| 19 | 전경 불명 유예(500ms) 동안 사용자가 실제로 다른 앱으로 옮겨 가 있었음 | 가능성 낮음 · 그 사이의 버튼 0 은 떼기로 치지 않으므로 잘못된 놓기는 없고, 유예가 끝나거나 `Other` 가 보이면 비정상 끝(무변경)이다 |

### 11-3. 범위에서 뺀 것 (1판에 있었고 PRD 4판으로 빠짐)

- 직전 도킹 되돌리기(옛 R20 · `tab.undoDock` · 단축키) — Q2 비목표.
- 커서를 따라오는 창(옛 Follow 모드 · 원위치 · 반투명 · 축소) — Q1 「창이 따라오지 않는다」.
- Esc 취소 · 「원래 자리로 되돌리기」 — R12 · R2. 비정상 끝의 「출처 탭바로」는 모델 무변경으로 선다(§1-3).
- 명령 둘(`tab.dock` · `tab.moveToWindow`) — 명령 하나 + 모드로(§3-1).

## 12. ADR 제안 (번호는 `/adr` 가 준다)

결정 날조 금지 — 아래 「거부한 대안」은 PRD · 조사 · 사용자 대화 · 리뷰에서 나온 것만 적었다. 사용자 결정분은 사용자 확인 뒤 박는다.

- **ADR-A 도킹 모델 = 탭 끌기 · 즉시 떼기 · 셸이 모는 앱 주도 끌기 · 모델은 놓을 때 한 번 · 보인 것과 같을 때만 적용 · 붙는 단위는 탭 하나의 배치 통째**(사용자 결정 R1 · R2 · R9 · R18 · Q1 + 이 TRD 의 구현 선택)
  - 결정: 제목줄은 OS 이동만, 탭을 끌면 그 자리에서 떼어진다(화면 표시)고 딱지 창이 따라간다. 끌기는 웹뷰에서 시작하고 셸이 전경 · 포인터 · 판정 · 딱지 · 레이아웃 변경을 몬다. 놓으면 이음새 · 탭바 · 새 창 셋 중 하나이고, 떼는 점의 판정이 마지막으로 보낸 가이드와 같을 때만 적용한다. 붙이기는 모델 이동(웹뷰 이전 아님)이고 마우스와 명령이 한 적용 함수를 지난다.
  - 거부한 대안: ① **제목줄(네이티브 이동 루프) 도킹** — 조사 §4 의 전제였다. 창 이동이 기본이고 도킹은 선택이어야 한다는 사용자 정정(R1)으로 뒤집혔고, 이동 루프 끝 감지에 서브클래스가 필요했다. ② **창이 커서를 따라오는 끌기(1판의 Follow)** — 사용자 결정(Q1 — 언리얼식 딱지). ③ **끄는 동안 모델을 바꾸고 취소 때 되돌리기** — 사용자 결정(R2 아래 줄 · R11a — 놓을 때만 바꾼다). ④ **놓는 순간의 판정만으로 적용(1판)** · **마지막으로 보낸 가이드만으로 적용(2판)** · **보낸 가이드와 다르면 우리 창 위에서는 늘 무변경(3판)** — 1판은 사용자가 못 본 가이드에 놓이고(2판 리뷰 A1), 2판은 문턱 직후 뗀 손이나 마지막 틱 사이에 움직인 손이 엉뚱한 결과를 내며(2판 리뷰), 3판은 슬롯 가운데처럼 가이드가 없는 자리에 놓아도 새 창이 안 떠 PRD R22(「이음새 · 탭바가 아닌 곳에 놓으면 새 창」)를 어긴다(3판 리뷰 재검). 그래서 4판은 「가이드 없음」을 늘 새 창으로, 이음새 · 탭바만 보인 것과 대조한다. ⑤ **웹뷰 reparent(`Webview::reparent`)로 화면을 옮기기** — 슬롯 내용은 모델 데이터라 팝아웃처럼 모델을 옮기면 된다(조사 §4 리뷰). ⑥ **슬롯 하나를 직접 끌기** — 사용자 결정(PRD §2 비목표). ⑦ **DOM 끌기 라이브러리 · OLE 끌기**(pragmatic-drag-and-drop · dnd-kit · react-dnd · crabnebula `drag-rs`) — 앞 셋은 문서 하나 안에서만 돌고, `drag-rs` 는 데이터 전달 모델이다(조사 §9). ⑧ **도킹 라이브러리 채택**(dockview · FlexLayout · golden-layout · Lumino) — 자기 레이아웃 모델을 소유해 셸 권위(ADR-0035)와 겹친다(조사 §5).
- **ADR-B 크기 규칙 = 새것 1/3 하나 · 면적 0 만 거절**(사용자 결정 R7 · R8)
  - 결정: 슬롯 옆 · 바깥 변 모두 새것 1/3 · 기존 2/3. px 최소 · 상한 · 옆 칸 양보 없음. 결과에서 새로 면적 0 이 되는 칸이 있을 때만 거절(ADR-0227 결정 9 와 같은 태도).
  - 거부한 대안: **Unity 식(A — 대상의 1/3 · 최소 100px · 바깥 200px 고정 띠 · 옆 칸 양보)** · **반반 + 바깥 1/3(B)** · **크기 기억(C — JetBrains · AvalonDock)**(조사 §8) · 퍼센트 + 기준 px · 200px 고정 띠(작은 창에서 0 이 됨) · 30%(rc-dock 식). 근거 = 조사 머리의 사용자 확정(2026-10-15).
- **ADR-C OS 층 목적 단위 함수 + platform 이 소유하는 창 핸들(`raw-window-handle` 입구)**(이 TRD 의 구현 선택 · ADR-0266 · R19) — ★platform 의존 규칙을 고치므로 사용자 확인 뒤에 박는다(§11-1 #3)★
  - 결정: `topmost_at(점, 후보, 투과)` · `pointer() → Read | Unavailable | Unsupported` 를 platform 에 둔다(스파이크가 고르면 보이기 · 숨기기 함수 하나와 `fine_timer` 를 더한다). 핸들은 platform 이 소유한 Copy · Send 값 `NativeWindow(isize)` 이고 들어오는 문은 `raw-window-handle` 하나다. platform 의 「운영 의존 서드파티 = `windows` · `tracing` 만」 규칙에 `raw-window-handle`(타입만 · OS 코드 0)을 더한다.
  - 거부한 대안: ① **셸이 `RawWindowHandle` 의 Win32 변형을 열어 HWND 정수를 꺼내 넘기기** — `cfg` 는 없지만 부르는 쪽이 OS 규칙을 안다(「플랫폼 중립」). ② **Tauri 의 `hwnd()`** — `#[cfg(windows)]` API 라 셸에 OS `cfg` 가 생긴다(게이트 4f). ③ **창 사각형 맞대기**(platform 이 우리 프로세스 창의 사각형 · z 순서를 주고 셸이 Tauri 창 위치와 맞대기) — 겹친 창 · 같은 크기 창에서 모호하다. ④ **z 순서를 포커스 기록으로 추정**(Windows 에서도) — 다른 앱 창에 가려진 자리를 못 거른다(R15). ⑤ **함수마다 OS 를 고르는 트레이트 객체** — platform 「들이는 규칙」 2(컴파일 시점 분기)와 어긋난다. ⑥ **커서를 Tauri `cursor_position` 으로 읽기** — 메인 스레드 왕복이고 버튼 상태와 한 순간에 읽을 수 없다. ⑦ **버튼 읽기를 `Option<bool>` 하나로** — 「지금 못 믿음」과 「이 OS 에 없음」이 접혀 보안 데스크톱의 0 이 떼기로 읽힌다(3판 리뷰 F7).
- **ADR-D 도킹 명령의 오류는 타입드 — `NOT_FOUND` / `CONFLICT` 를 가른다 · 명령 하나 + 모드**(이 TRD 의 구현 선택)
  - 결정: 레이아웃 적용 서비스가 도킹에 한해 타입드 오류를 내고, 버스는 종류로 코드를 고른다. 거절은 ok 의 결말 칸이 아니라 오류다. 세 모드는 `tab.dock` 하나에 `mode` + 선택 칸으로 싣는다.
  - 거부한 대안: ① **기존 레이아웃 명령처럼 전부 `CONFLICT` + 문구**(`commands.rs:24-34`) — 서비스가 타입을 주는데 버릴 이유가 없다. ② **`split.setRatio` 식 ok + `outcome`** — `ok` 만 보는 호출자가 거절을 성공으로 읽는다. ③ **모드마다 명령 하나(1판의 `tab.dock` · `tab.moveToWindow`)** — PRD R16 이 「명령 하나 · 모드」로 정했다(1판 리뷰 D6).
- **ADR-E 팝아웃 닫기 요청에서 모델 먼저 지우기 + 닫힌 label 묘비**(3판 결정 M2 · §11-1 #2 사용자 확인 뒤)
  - 결정: 팝아웃의 `CloseRequested` 에서 레이아웃 락 한 번 안에 그 창의 모델 정리를 하고 label 에 묘비를 단다. 도킹 · 새 창 확정 · 팝아웃 phase C 는 없는 창 · 묘비 앞에서 무변경으로 멈춘다. 근거 = 팝아웃 닫기 요청은 되돌려지지 않는다(우리 처리기 · 프론트 · 런타임 소스). ADR-0057 불변식은 그대로이고 정리 계기만 옮긴다.
  - 거부한 대안: ① **열림 사전 확인 + 남는 위험 기록(2판 기본값)** — 사전 확인은 틈을 줄일 뿐 닫지 못하고, 닫히는 창에 확정된 탭이 사라진다(2판 리뷰 D3/A5 · Designer). ② **닫는 중 표시만 하고 정리는 소멸 때**(모델에 칸 하나) — 닫기 요청이 되돌려지지 않는 지금은 지우기와 같은 결과에 칸만 하나 늘린다. 닫기를 막을 수 있게 되면 이 안으로 옮긴다. ③ **확정 뒤 `is_open` 으로 다시 보기(2판의 새 창 뒤 확인)** — 이미 옮긴 탭을 되살리지 못해 「탭은 출처에 남는다」를 못 지킨다.

## 부록 — 요구사항 추적

| PRD | 어디서 서나 |
|---|---|
| R1 · AC1 | 제목줄은 OS 이동 그대로 — 이 기능이 끼어들지 않는다 · S3 |
| R2 · AC2 · AC6 | 떼어진 표시(§4-1 — 그리기만) · 탭 하나 팝아웃 숨기기(S1) · 딱지 창(§4-2 · S4) · 모델 무변경(§1-3) |
| R3 · R4 · R5 · R6 · AC2 · AC4 | 띠 = 결과 사각형 · `hot` · 바깥 띠 32px · 가운데 없음(§1-5 · §2) |
| R6a | §2-2 규칙 6 · 8 · 경계 포함 규칙 |
| R6b · AC14 | 탭 머리 대기 500ms → `switch_tab`(§1-3) |
| R7 · AC3 · AC4 · AC5 | `docked_tree` 분할 모양 표(§1-2) |
| R8 · AC15 | 새 면적 0 판정(§1-2) · 빠진 띠에서는 안 붙음(§2-1 · §2-4) |
| R9 | 출처 View 의 트리 통째 이동(§1-2) |
| R10 · AC3 · AC6 | 빈 팝아웃 항목 제거 + `host.close`(§1-2 · §5) |
| R11 · AC7 | 같은 락 `resync`(§5) |
| R11a · AC11 | 끄는 동안 모델 무변경 · 락 한 번 · 기록기 스냅숏 · 닫기 요청 모델 먼저(§1-2 · §5) |
| R11b · AC3 | `FocusPolicy::Reveal` + `set_focus`(§1-2 · §1-3) · 명령은 `Keep` · 새 창도 활성 없이 |
| R12 | Esc 없음(§1-6 · §6-1) |
| R13b · AC16 | 틱 순서(전경 → 포인터) · `Unavailable` 시한 · `Lost` 힌트 · `Destroyed` · 무변경 = 출처 탭바로 · `Ended` 방송(§1-3 · §4-3) |
| R13c · AC16 | `expect_source` · `expect_target` · 보인 것과 같을 때만(M1) · `source_stale` · `Dragging`(§1-2 · §1-3) |
| R14 · AC9 | `lease_tab` 거절 + 웹뷰 즉시 커서(§1-3 · §1-6) |
| R15 · AC13 · AC14 | `topmost_at` 후보 · 투과 = 딱지(§1-4 · §2-4) · 보이는 탭 · 탭바 |
| R16 · AC10 | `tab.dock` 세 모드 · 오류 표(§3) · 같은 창 활성 규칙(§11-1 #1) · 닫히는 창 = Seam · TabBar `NOT_FOUND` · NewWindow `WindowClosed`(§1-2 · §3-2) |
| R17 | §3-3 |
| R18 | 셸이 모는 끌기 · 웹뷰는 시작 · 떼기 힌트 · 그리기(§1-1 · §1-6) |
| R19 | platform 목적 단위 함수 · `NativeWindow`(§1-4) |
| R21 · AC18 | `DockTo::TabBar`(§1-2) · 탭바 자리 판정(§2-2 규칙 3) |
| R22 · AC8 · AC12 | `DockTo::NewWindow` · 세 phase · 묘비 · 재사용 · 크기 · 자리 검증(§1-2) · 딱지 자리(§4-2) · 놓기 판정의 「가이드 없음 = 새 창」(§1-3) · S5 |
| W1 | S1 |
| W3 | S2 |
| W4 | S5 |
| W5 | S3 |
| W7 | S4 |
