# TRD — S14: 멀티 페이지 레이아웃 (rev.4)

**상태:** 리뷰 대기 (rev.3 BLOCK 해소 — 아키텍처 재정립)
**작성:** 2026-06-27
**근거 ADR:** ADR-0035(레이아웃 권위=src-tauri) · ADR-0036(전송 중계 통일)
**근거 리서치:**
- `docs/research/multi-window-layout-authority-topology-research-2026-06-27.md` (deep, Claude+Codex 교차)
- `docs/research/multi-tab-layout-state-management-research-2026-06-27.md`
- `docs/research/multi-window-layout-sync-research-2026-06-27.md`

---

## 아키텍처 방향 (확정 — ADR-0035/0036)

**레이아웃 권위 = src-tauri Rust. 데몬 = 에이전트만(View 불가지론). 창 = 순수 렌더러.**
**모든 트래픽(레이아웃 + 에이전트 I/O)은 src-tauri를 단일 choke point로 지난다.**

기준: engram은 **에이전트와 슬롯이 디커플링**됨(close_view해도 에이전트 생존·재배정) → 슬롯/View는 표시 레이어 → **에디터 모델**(클라 로컬 레이아웃)이지 tmux 모델(pane==PTY=서버소유)이 아님. (ADR-0035 §근거.)

```
[데몬]  에이전트만 (PTY·출력·정책). View 모름
   ▲ 단일 WS (에이전트 I/O)
[src-tauri Rust]  ViewManager(레이아웃 권위) + DaemonClient(단일 연결) + OutputRouter + 창 lifecycle
   ▼ Tauri IPC: invoke(커맨드) / emit(레이아웃 상태) / Channel(라우팅된 출력)
[main창] [popup창] [tree창]  순수 렌더러 (창마다 JS 격리 → 권위 JS 아님)
```

**거부한 대안(ADR에 박음):** JS authority(rev.1/rev.2 — 창 격리 split-brain, 2회 BLOCK) · 데몬 authority(rev.3 — 디커플링 무시한 tmux 오적용) · 창마다 데몬 직결(N중복·라우팅 재유도).

---

## Rust 측 설계 (`src-tauri/src/`)

### 데이터 구조 (레이아웃 권위 — ADR-0035)

```rust
// AppState 소유: Arc<Mutex<ViewManager>>  (invoke 스레드풀 동시접근 → 락)
pub struct ViewManager {
    pub views: Vec<View>,
    pub active_view_id: Uuid,
    pub window_bindings: HashMap<String, Uuid>,  // window_label → view_id
}

pub struct View {
    pub id: Uuid,
    pub name: String,
    pub layout: LayoutNode,
    pub focused_slot_id: Option<Uuid>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayoutNode {
    Slot  { id: Uuid, agent_id: Option<String> },         // agent_id = 데몬 에이전트 "참조"
    Split { dir: SplitDir, ratio: f32, a: Box<LayoutNode>, b: Box<LayoutNode> },
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum SplitDir { Horizontal, Vertical }
```
`agent_id`는 데몬 에이전트를 가리키는 **문자열 참조**일 뿐 — src-tauri는 에이전트를 소유하지 않는다(데몬 소유). 슬롯은 표시 바인딩.
**불변식:** View·LayoutNode·Slot 타입은 데몬 crate/protocol에 새지 않는다(데몬 UI 불가지론 — ADR-0035).

### invoke 핸들러

| invoke | 파라미터 | 반환 | phase | 설명 |
|---|---|---|---|---|
| `create_view` | `name?` | `Uuid` | A | 새 View(기본 슬롯 1개) |
| `close_view` | `view_id` | `()` | A | View 제거. active면 전환. 에이전트 kill 안 함 |
| `switch_view` | `view_id` | `()` | A | active_view_id 변경 |
| `split_slot` | `view_id, slot_id, dir` | `Uuid` | A | 슬롯 분할, 새 slot_id 반환 |
| `close_slot` | `view_id, slot_id` | `()` | A | 슬롯 제거, 형제 승격 |
| `assign_agent` | `view_id, slot_id, agent_id` | `()` | A | 슬롯에 에이전트 배정 |
| `open_view_in_popup` | `view_id` | `String` | A | 팝업 창 생성, window_label 반환. 중복 시 focus |
| `close_popup` | `window_label` | `()` | A | 팝업 닫기, window_bindings 정리 |
| `get_view` | `view_id` | `ViewSnapshot` | A | 팝업 초기 pull(race 방지) |
| `spawn`/`kill`/`write`/`resize` | … | … | B | 에이전트 커맨드 — src-tauri DaemonClient로 데몬 전달 |

### emit 이벤트 (src-tauri → 전 창)

```
layout:updated     { view_id, layout: LayoutNode, focused_slot_id }
view:list-updated  { views: Vec<ViewMeta>, active_view_id }   // ViewMeta = { id, name }
```
**불변식(ADR-0006):** ViewManager 락 수정 → **락 해제 후** emit(emit 중 락 보유 금지).

### 전송 중계 (ADR-0036 — phase B)

- **DaemonClient(Rust):** 데몬과 **단일 WS** 연결 + 프로토콜 의미론(재연결·epoch·seq dedup·resubscribe). 현 TS `wsTransport.ts`/`protocolClient.ts`(~580줄)를 Rust로 이전. protocol crate(ts-rs) 재사용.
- **OutputRouter:** 데몬 출력 프레임 수신 → `agentId` → ViewManager 조회("이 에이전트를 띄운 슬롯이 든 View → 그 View 보는 창") → 해당 창 Channel로만 전달. 에이전트당 데몬 구독 1회(중복 제거).
- **팝업 창 생성:**
  ```rust
  if let Some(w) = app.get_webview_window(&label) { w.set_focus()?; return Ok(label); }  // 중복 guard
  WebviewWindow::builder(&app, &label, WebviewUrl::App(format!("/popup?viewId={view_id}").into()))
      .title(&view.name).build()?;
  manager.window_bindings.insert(label.clone(), view_id);
  ```
  `on_window_event` CloseRequested에서 window_bindings 정리 + Tauri #15583 unlisten 명시 해제.

---

## React 측 변경

### 제거
| 파일 | 처리 |
|---|---|
| `src/store/slotStore.ts` | **삭제.** JS 레이아웃 권위(split-brain 원인) → src-tauri로 이관 |

### 신규
| 파일 | 내용 |
|---|---|
| `src/store/viewStore.ts` | `views: ViewMeta[]`·`activeViewId`·`currentLayout` — emit 미러용 얇은 store |
| `src/store/layoutTypes.ts` | `LayoutNode`·`SplitDir`·`ViewMeta` 타입(ts-rs 생성 미러) + 순수 렌더 헬퍼(`findSlot` 등) |
| `src/components/layout/ViewTabBar.tsx` | 탭 + "+" → create/switch/close_view invoke |
| `src/api/tauriTransport.ts` | (phase B) `Transport` 구현 — src-tauri IPC carrier(WsTransport 대체) |

### 수정
| 파일 | 변경 |
|---|---|
| `src/store/eventBus.ts` | Tauri `listen("layout:updated"/"view:list-updated")` 추가(WS agentClient와 별개 채널). `__engramLayout`=invoke 래퍼 |
| `src/components/layout/AppLayout.tsx` | ViewTabBar 상단 + viewStore 구독 → 현재 View 렌더 |
| `src/components/slot/SlotPane.tsx` | LayoutNode 재귀 렌더 + 슬롯 agent_id로 출력 구독. ⚠️ dashboard1 조율 |
| `src/components/slot/SlotContextMenu.tsx` | `window.open` 제거 → `open_view_in_popup` invoke. ⚠️ dashboard1 조율 |
| `src/components/agent/AgentTree.tsx` | dispatch → `assign_agent` invoke |
| `src/pages/PopupPage.tsx` | 마운트 시 `get_view(viewId)` pull → listen 등록(race 방지) |
| `src/api/clientFactory.ts` | (phase B) carrier `WsTransport` → `TauriTransport` |
| `tauri.conf.json` | `slot-popup` 정적 창 제거 → 동적 생성 |

### slotId 정책
`number` → **UUID**(`crypto.randomUUID()` 또는 Rust `Uuid`). 창 간 전역 고유(페이지 간 충돌 제거).

---

## LLM 제어 표면 (§5 — ADR-0035)

```javascript
window.__engramLayout = {
  splitSlot:   (viewId, slotId, dir)     => invoke('split_slot',  { viewId, slotId, dir }),
  closeSlot:   (viewId, slotId)          => invoke('close_slot',  { viewId, slotId }),
  assignAgent: (viewId, slotId, agentId) => invoke('assign_agent',{ viewId, slotId, agentId }),
  createView:  (name)                    => invoke('create_view', { name }),
  switchView:  (viewId)                  => invoke('switch_view', { viewId }),
  closeView:   (viewId)                  => invoke('close_view',  { viewId }),
  openPopup:   (viewId)                  => invoke('open_view_in_popup', { viewId }),
}
```
LLM(cdp eval)·사람 UI가 동일 핸들 호출 → 프론트는 순수 I/O 유지.

---

## 정책: close_view 에이전트 lifecycle · 메시지 락

- **close_view:** layout 트리에서 agentId 수집하되 **에이전트 kill 안 함**(데몬 생존). AgentTree에서 "미배정"으로 표시(재assign 가능). "닫을 때 종료" 옵션은 후속 파라미터.
- **메시지 락/게이팅(ADR-0036):** 정책 enforce는 **데몬**(단일 choke point — 모든 발신자에 일관 적용). src-tauri 중계는 나르기만(캐시 기반 조기거부는 최적화일 뿐 권위 아님). 프론트는 락 상태 받아 표시만.

---

## 구현 단계 (phasing)

- **Phase A — 레이아웃 기능(ADR-0035):** ViewManager + 레이아웃 invoke/emit + viewStore/eventBus/ViewTabBar/AppLayout/SlotPane/팝업 + slotId UUID. 에이전트 I/O는 **현행 per-window WsTransport 유지(interim)** — 각 창이 자기 슬롯 agent_id로 구독. → 멀티 View 기능 인도.
- **Phase B — 전송 중계(ADR-0036):** DaemonClient(Rust 이전) + TauriTransport carrier + OutputRouter. per-window 직결 제거 → 단일 choke point 완성·중복 제거·원격 효율. (전송층 재설계라 별도 단계.)

Phase A 동안 토폴로지가 한시적 비일관(레이아웃=중앙 / 출력=직결)이나 기능은 동작. Phase B가 통일.

---

## 수용 기준

1. 탭 바에서 View 추가/전환/닫기 → src-tauri state 변경 → emit → 전 창 리렌더
2. 팝업 창이 해당 View를 자체 LayoutRenderer로 렌더 + 독립 분할
3. 팝업 부팅 `get_view` pull 후 listen(초기 emit 놓침 없음) + 닫기 시 window_bindings 정리 + #15583 unlisten
4. `window.__engramLayout.*` invoke가 메인·팝업 모두 동작
5. slotId UUID 전역 고유(페이지 간 충돌 없음)
6. 데몬 crate에 View/Layout/Slot 타입 0 (`rg` 게이트) — 데몬 UI 불가지론
7. (Phase B) 데몬 연결 1개로 N창 동작 + 에이전트당 구독 1회 + src-tauri OutputRouter가 미표시 창엔 출력 안 보냄
8. `cargo test` + `cargo build` + `npm test` + `npx tsc --noEmit` 통과

---

## 열린 사항

- **dashboard1 조율:** SlotPane.tsx / SlotContextMenu.tsx 공통 수정 — 작업 순서 조율 필요(현재 View 터미널 + JSON 파싱 작업 중).
- **resize 경로 phasing (dashboard1 Task1 조율 — 결정됨):** 터미널 resize(cols/rows)는 에이전트 커맨드라 **Phase A=현행 WS직결(`protocolClient.resizePty`) 유지** / Phase B에서 carrier만 `TauriTransport`로 스왑(인터페이스·데몬 PTY resize 의미론 불변 → 기존 fix carry-forward). Task1(PTY 80x24 픽스)은 현행 경로에서 진행.
- **멀티뷰 resize 정책 (Phase B):** 같은 에이전트를 크기 다른 여러 창/슬롯이 띄울 때 cols/rows 누가 이기나 — tmux `window-size`(smallest/largest/latest) 식 정책 필요. src-tauri가 레이아웃 소유 → resize 권위·정책 결정. Phase A는 단일 경로라 비해당.
- **Phase B 메커니즘:** DaemonClient의 프로토콜 의미론을 Rust로 "이전" vs "프레임 라우터(얇은)" — 정확한 형태는 Phase B 착수 시 spike/TRD 보강(고대역 출력 relay 오버헤드 실측 포함).
- **빈 상태(결정됨):** 마지막 View 닫으면 **빈 화면 + 가운데 `+` 아이콘**(클릭=`create_view`). minor — 추후 수정 가능.
- **영속:** ViewManager 재시작 복원(향후 — 저장 위치는 별도 결정).
