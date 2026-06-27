# TRD — S14: 멀티 페이지 레이아웃 (rev.5)

**상태:** 리뷰 반영 — 코더 대기 (rev.4 `/review trd full` FIX 반영 + phasing 폐기)
**작성:** 2026-06-27
**근거 ADR:** ADR-0035(레이아웃 권위=src-tauri) · ADR-0036(전송 중계 통일 — phasing 없이 목표 구조 바로)
**근거 리서치:** `docs/research/multi-window-layout-authority-topology-research-2026-06-27.md` (deep, Claude+Codex 교차) 외 2건
**리뷰:** rev.4 = Architect-breaker(opus) + Designer(Codex) 둘 다 **FIX, BLOCK 없음**(아키텍처 건전 판정). 본 rev.5가 FIX 반영 + 사용자 결정(레거시 없이 제대로 → phasing 폐기).

---

## 아키텍처 방향 (확정 — ADR-0035/0036, phasing 없음)

**src-tauri = 단일 데몬 클라이언트 + 프로토콜 소유 + 출력 라우터 + 레이아웃 권위.** 데몬 = 에이전트만(View 불가지론). 창 = 순수 렌더러(TauriTransport 위 thin agentClient). **레거시 각자-연결 폐기 — 임시로도 안 둠.**

```
[데몬]  에이전트만 (PTY·출력·정책). View·창 모름
   ▲ 단일 WS (에이전트 명령/출력)
[src-tauri Rust = 허브]
   · DaemonClient   — 단일 연결·재연결·resubscribe·epoch·seq dedup·routing (TS에서 이전)
   · ViewManager    — 레이아웃 권위(Views·split 트리·slot→agent·window_bindings)
   · OutputRouter   — agentId → ViewManager snapshot 조회 → 해당 창에만 fan-out
   ▼ Tauri IPC: invoke(커맨드) / emit(레이아웃) / Channel(라우팅된 출력)
[main창] [popup창] [tree창]  순수 렌더러 (창마다 JS 격리 → 권위는 src-tauri Rust)
```

기준(ADR-0035): engram은 에이전트/슬롯 **디커플링**(close_view해도 에이전트 생존·재배정) → 슬롯=표시 레이어 → 에디터 모델(클라 로컬 레이아웃). 토폴로지(ADR-0036): 단일 연결 멀티플렉싱(LSP·Chromium 관행) — 각자-연결은 레거시지 관행 아님.

---

## ★구현 리스크 게이트 (먼저 박음)★

**전송층 재배치(DaemonClient의 연결·재연결·resubscribe·epoch·seq dedup을 TS→Rust)는 이 프로젝트의 동시성-치명 부위다.** 현 `wsTransport.ts`/`protocolClient.ts`가 막고 있는 zombie-socket 가드·generation 토큰·epoch race·replay→live 순서를 Rust에서 **race 재현 없이** 보존해야 한다(불변식: ADR-0001 kill 인과 · 0005 finalize 1회 · 0006 락 순서 · 0007 epoch 재구독). → **이 모듈은 TDD 강제 + `/review code deep`(동시성 트리거).** 빠르게 짜는 게 아니라 정확하게 짜는 게 게이트.

---

## Rust 측 설계 (`src-tauri/src/`)

### ViewManager — 레이아웃 권위 (ADR-0035)

```rust
// AppState 소유: Arc<Mutex<ViewManager>>  (invoke 스레드풀 동시접근 → 락)
pub struct ViewManager {
    pub views: Vec<View>,
    pub active_view_id: Uuid,                    // = 메인 창 탭 선택(아래 의미론 참조)
    pub window_bindings: HashMap<String, Uuid>,  // window_label → view_id
}
pub struct View { pub id: Uuid, pub name: String, pub layout: LayoutNode, pub focused_slot_id: Option<Uuid> }

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayoutNode {
    Slot  { id: Uuid, agent_id: Option<String> },   // agent_id = 데몬 에이전트 "참조"(소유 아님)
    Split { dir: SplitDir, ratio: f32, a: Box<LayoutNode>, b: Box<LayoutNode> },
}
```

**하드 계약 (리뷰 FIX — Codex F2):**
- `ratio`: 0.0~1.0 **클램프**(범위 밖 입력 거부/클램프). split 시 기본 0.5.
- **root 슬롯 close:** View의 마지막 슬롯을 닫으면 → 그 View는 빈 상태(빈 화면 + 가운데 `+`). 마지막 View까지 닫으면 동일(빈 화면 + `+` = `create_view`).
- **focus fallback:** `focused_slot_id`가 가리키던 슬롯이 사라지면 → 트리 첫 슬롯으로 폴백(없으면 None).
- **invalid id:** 존재하지 않는 `view_id`/`slot_id` → **no-op + 에러 반환**(패닉/부분변경 금지).
- **agent_id 검증 안 함(FIX — opus F7):** `assign_agent`는 참조 문자열만 저장한다. 데몬에 실재 여부를 묻지 **않는다** → ViewManager 락 보유 중 DaemonClient 호출 0(ADR-0006). (미배정/죽은 에이전트는 렌더 시 표시.)

**switch_view 멀티창 의미론 (FIX — Codex F1):**
- `active_view_id` = **메인 창의 활성 탭**. 팝업·tree 창은 `window_bindings`로 **고정 View**에 바인딩(자기 뷰만 렌더, 탭 전환 안 함).
- `switch_view(view_id)` = 메인 창 활성 탭 변경. 팝업엔 영향 없음.

### DaemonClient — 프로토콜 소유 (ADR-0036, ★동시성-치명)
- 데몬과 **단일 WS** 연결 + 재연결 + **구독 union 추적** + resubscribe(재연결·epoch 변경 시) + seq dedup + epoch 가드. 현 TS 의미론을 Rust로 이전.
- 창의 spawn/kill/write/resize/subscribe는 invoke → DaemonClient → 데몬.

### OutputRouter — 출력 라우팅 (FIX — opus F6: 핫패스 락 회피)
- 데몬 출력 프레임 수신 → `agentId` → "그 에이전트를 띄운 슬롯이 든 View → 그 View 보는 창" → 해당 창 Channel로만 전달.
- **라우팅 테이블은 ViewManager 변경 시 갱신되는 lock-free snapshot**(프레임마다 ViewManager Mutex 잠그지 않음 — ADR-0006 "락 보유 중 send 금지" 준수).

### invoke 핸들러
- **레이아웃:** `create_view(name?)→Uuid` · `close_view(view_id)` · `switch_view(view_id)` · `split_slot(view_id,slot_id,dir)→Uuid` · `close_slot(view_id,slot_id)` · `assign_agent(view_id,slot_id,agent_id)` · `open_view_in_popup(view_id)→String` · `close_popup(window_label)` · `get_view(view_id)→ViewSnapshot`(version 포함, 아래 race).
- **에이전트:** `spawn`/`kill`/`write`/`resize` → DaemonClient 경유.
- **불변식(ADR-0006):** ViewManager 락 수정 → **락 해제 후** emit. 락 보유 중 외부(DaemonClient/emit) 호출 금지.

### emit / Channel
```
layout:updated     { view_id, layout, focused_slot_id, version }   // version = 팝업 race용
view:list-updated  { views: ViewMeta[], active_view_id }
(per-창 Channel)    라우팅된 에이전트 출력
```

### 팝업 (ADR-0036)
- `open_view_in_popup`: 중복 guard(`get_webview_window`) → `WebviewWindow::builder(/popup?viewId=X)` → `window_bindings.insert`.
- **race(FIX — opus F4):** 팝업 마운트 시 `get_view(viewId)` → 응답의 `version` 보관 → 그 뒤 listen 등록 → listen으로 오는 emit이 보관 version 이하면 폐기(pull↔listen 사이 윈도 닫음).
- `on_window_event` CloseRequested → `close_popup`(window_bindings 정리) + Tauri #15583 unlisten 명시.
- **tree 창도 동일 동적 생성 대상**(FIX — opus F5: `Sidebar.tsx`의 `window.open('#/tree')`도 교체).

---

## React 측 변경

### 제거
- `src/store/slotStore.ts` **삭제** (JS 레이아웃 권위 = split-brain 원인).

### 신규
- `src/store/viewStore.ts` — emit 미러(views·activeViewId·currentLayout).
- `src/store/layoutTypes.ts` — LayoutNode·SplitDir·ViewMeta(ts-rs 미러) + 순수 렌더 헬퍼.
- `src/components/layout/ViewTabBar.tsx` — 탭 + `+`(빈 상태 포함) → create/switch/close_view invoke.
- `src/api/tauriTransport.ts` — `Transport` 구현(src-tauri IPC carrier). `agentClient`=`ProtocolClient` 인터페이스(ADR-0011) 유지, 두뇌는 Rust → 이 carrier는 얇음.

### 수정 (파일 경로 정정 — FIX opus F5)
- `src/store/eventBus.ts` — Tauri `listen("layout:updated"/"view:list-updated")` 추가(WS와 별개 채널). **`__engramLayout`의 기존 `{dispatch}` 키 제거** 후 invoke 래퍼로 교체(안 지우면 LLM이 삭제된 store 호출→crash).
- `src/api/clientFactory.ts` — carrier `WsTransport` → `TauriTransport`.
- `src/components/layout/AppLayout.tsx` — ViewTabBar + viewStore 구독.
- `src/components/layout/LayoutRenderer.tsx` — **(누락 정정)** LayoutNode 재귀 렌더 본체. viewStore 구독.
- `src/components/slot/SlotPane.tsx` **및** `src/components/layout/SlotPane.tsx` — **두 곳 모두** slotStore 의존 제거(어느 게 산 경로인지 코더가 확인 후 정리). ⚠️ dashboard1 조율.
- `src/components/slot/SlotContextMenu.tsx` — `window.open` 제거 → `open_view_in_popup` invoke. ⚠️ dashboard1 조율.
- `src/components/.../Sidebar.tsx` — tree `window.open` → 동적 창 invoke.
- `src/components/agent/AgentTree.tsx` — dispatch → `assign_agent` invoke.
- `src/pages/PopupPage.tsx` — `useSlotStore` 직접 읽기(현재 split-brain 실증) 제거 → `get_view(viewId)` pull(version) → listen.
- `tauri.conf.json` — `slot-popup` 정적 창 제거 → 동적 생성.

### slotId: `number` → **UUID** (창 간 전역 고유).

---

## LLM 제어 표면 (§5 — ADR-0035)
`window.__engramLayout = { splitSlot, closeSlot, assignAgent, createView, switchView, closeView, openPopup }` — 각 invoke 래퍼. 멀티창 어디서 호출하든 같은 src-tauri AppState ViewManager에 닿음(opus F3 확인). 프론트 순수 I/O 유지.

---

## 정책
- **close_view:** agentId 수집하되 **에이전트 kill 안 함**(데몬 생존, AgentTree "미배정" 표시). "닫을 때 종료"는 후속 파라미터.
- **메시지 락/게이팅:** 정책 enforce = **데몬**(단일 choke point). src-tauri 중계는 나르기만, 프론트는 표시만(ADR-0036).
- **교체성 가드(FIX — Codex F3):** 라우팅·창 개념을 `agentClient`·protocol crate에 누설 금지. 데몬 crate에 View/Layout/Slot 타입 0(`rg` 게이트).

---

## 수용 기준
1. 탭 바 View 추가/전환/닫기 → src-tauri ViewManager 변경 → emit → 전 창 리렌더. 마지막 View 닫기 = 빈 화면 + `+`.
2. 팝업이 해당 View를 독립 렌더 + 분할. `get_view` version pull→listen으로 초기 emit 유실·중복 없음. 닫기 시 window_bindings 정리 + #15583 unlisten.
3. `window.__engramLayout.*` 메인·팝업 동작. (기존 dispatch 키 제거 확인.)
4. slotId UUID 전역 고유. `ratio` 0~1, invalid id no-op.
5. **단일 데몬 연결**로 N창 동작 + 에이전트당 구독 1회 + OutputRouter가 미표시 창엔 출력 안 보냄(락-free snapshot).
6. 데몬 crate에 View/Layout/Slot 타입 0(`rg` 게이트) — UI 불가지론.
7. **전송 재배치 동시성 불변식 보존**(ADR-0001/0005/0006/0007) — TDD로 회귀 단언, `/review code deep` 통과.
8. `cargo test`(루트) + `cargo build` + `npm test` + `npx tsc --noEmit` 통과 + `scripts/cdp.mjs` 실측.

---

## 열린 사항
- **dashboard1 조율:** SlotPane(2곳)·SlotContextMenu 공통 수정 — 건드리기 전 핑(합의됨).
- **resize 경로(dashboard1 Task1):** 통일 후엔 resize도 src-tauri 경유(invoke→DaemonClient). Task1(80x24)은 현 경로에서 고치고 통일 시 carrier만 교체(인터페이스 불변 carry-forward). **멀티뷰 resize 정책**(같은 에이전트 크기 다른 창 → tmux window-size식 smallest/largest) = src-tauri가 레이아웃 소유하니 거기서 정책 결정.
- **OutputRouter snapshot 갱신 메커니즘:** ViewManager 변경 시 라우팅 snapshot 재생성 — 정확한 자료구조는 코더 spike.
- **영속(향후):** ViewManager 재시작 복원. ★영속 위치가 데몬 data dir(ADR-0024)와 섞이면 ADR-0029(데몬=데이터 단일소유) 부활 위험(opus F1) → 영속 결정 시 위치 경계를 ADR로 박을 것.
