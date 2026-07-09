# TRD — WezTerm식 창>탭>슬롯 (레이아웃 B, Phase 1 = 탭)

> 상태: **초안 (사용자 결정 전부 잠금 후 작성)** · 2026-07-09
> 정본 PRD = 같은 폴더 `PRD.md`. 이 문서는 그 §10-3이 요구한 "TRD 전 확정 명세 공백"을 메우고, 확정된 사용자 결정(D-1 C·D-2·D-3·D-6·D-7·D-8)과 ADR(0056·0046·0006·0035)을 대면 사실로 깔아 **구현 인터페이스를 확정**한다.
> 발견체인: `docs/README.md` → 레이아웃 B → PRD.md → 이 문서.
> 다음 단계: 이 TRD를 `/review trd` 통과 → 탭 소유 모델 ADR 박제 → `/implement critical`(스테이징 §10).

---

## 0. 한 줄 · 범위

한 창이 **탭 목록**(= 코드의 `View` 여러 벌)을 소유하고 그 안에서 전환하도록 백엔드 레이아웃 권위(`ViewManager`)를 바꾼다. Phase 1 = **탭 모델 + 탭바 UI + 창별 독립 라우팅 + LLM 제어 command**. 창 간 드래그(D-4)·저장복원(D-5)은 범위 밖.

**용어(PRD §1 고정):** 사용자의 "탭" = 코드의 **View**. 사용자의 "창" = **OS 웹뷰 창**(main + 런타임 팝업). 이 문서는 `View`(코드)·"탭"(개념)을 병기한다.

---

## 1. 확정 결정 (재론 금지 — 근거·거부대안은 PRD §4/§10 + 후속 ADR)

| # | 결정 | 요지 |
|---|---|---|
| **D-1 = C** | 소유자 인덱스 하이브리드 | `views`(전역 풀) + `view_owner`(View→창, 유니크) + `windows[label].{tabs,active}`. A(ref-list=소유)·B(글로벌풀 공유) 거부. |
| **D-2 = 예** | 메인도 일반 창 | 전역 `active_view_id` 제거. main/팝업 동일 코드경로. |
| **D-3** | 창 닫기 규칙 | **메인 = 항상 최소 1탭**(마지막 닫아도 빈 탭 유지) · **팝업 = 마지막 탭 닫으면 창도 닫힘**. 두 경우 모두 **에이전트(데몬 프로세스)는 생존**(§5 손발/두뇌 분리). |
| **D-6 = 예** | 빈 `create_window` | 에이전트 없이 빈 새 창(빈 탭 1개) 생성 command 추가. |
| **D-7 = 편입** | 배치 지정 스폰 | Phase 1 **마지막 슬라이스**로 `spawn_into` 얇은 래퍼(스폰+create_tab+assign 조립). 탭 모델 완성 뒤 얹음. |
| **D-8 = Ctrl+Tab만** | 키보드 내비 | Phase 1 최소 = Ctrl+Tab 창내 탭 순환만. 그 외 키맵 후속. |
| **D-4/D-5 = 후속** | 드래그·저장복원 | 범위 밖. D-1을 전역 UUID `ViewId`로 둬 나중에 얹을 수 있게만. |

**대면 ADR(강제 — 재해석 금지):**
- **ADR-0056** 탭 렌더 = keep-alive: 비활성 탭 xterm 인스턴스 살려둠(출력 계속 누적), 전환 즉시·무손실(replay 불필요). WebGL 좌석은 보이는 슬롯만.
- **ADR-0046** 뷰 직결 replay: 라우팅 표는 label-불가지, wire는 Unsubscribe(1→0)만 발화·Subscribe(0→1)는 프론트 `request_replay` 단독. 슬롯 진도(`lastDeliveredSeq`)는 뷰별 독립.
- **ADR-0006** 락 규율: `ViewManager` mutation + `OutputRouter.rebuild` + delta enqueue = 단일 임계구역(std Mutex), 락 보유 중 외부 await/emit 금지.
- **ADR-0035** 레이아웃 권위 = src-tauri. (본 TRD가 `active_view_id`=main-전용 부분을 **뒤집음** → 후속 ADR이 이 절을 supersede.)

---

## 2. 새 데이터 모델 (`src-tauri/src/layout/manager.rs`)

### 2-1. 타입 (before → after)

```rust
// ── BEFORE ──────────────────────────────────────────────
struct ViewManager {
    views: Vec<View>,                     // 전역 View 벡터 (선형 탐색)
    active_view_id: Uuid,                 // 전역 활성 뷰 1개 (메인 전용)
    window_bindings: HashMap<String, Uuid>, // label → view_id (팝업/보조창 바인딩)
    version: u64,
}

// ── AFTER (D-1 C) ───────────────────────────────────────
type ViewId = Uuid;
type WindowLabel = String;

struct ViewManager {
    views: HashMap<ViewId, View>,            // 전역 View 풀 (id lookup)
    view_owner: HashMap<ViewId, WindowLabel>,// View → 소유 창 (유니크 소유 강제)
    windows: HashMap<WindowLabel, WindowTabs>,// 창 → 탭 목록 + 활성
    version: u64,                            // 기존 유지 (stale-emit 방어, PopoutPage race)
}

struct WindowTabs {
    tabs: Vec<ViewId>,   // 탭 순서 (좌→우)
    active: ViewId,      // 이 창의 활성 탭
}
```

- `View`(id/name/layout tree/focused_slot) 자체·`tree.rs` 순수 ops·`types.rs`는 **불변**(슬롯 분할 로직 재사용). 바뀌는 건 "어느 View가 어느 창의 몇 번째 탭인가"의 **소유·활성 층**뿐.
- `agent-tree` 창은 이 모델 **밖**(§3-2). `windows`에 `"agent-tree"` 키 없음.

- **`view_owner`는 캐시된 역인덱스로 유지(중복 상태 아님, G11).** `windows[L].tabs` 스캔으로 소유 창을 파생할 수도 있으나(≤24 규모라 O(n) trivial), view-id-키 command(`assign_agent`/`split_slot`/`close_slot`)가 소속 창을 **O(1)** 로 찾아야 하고(그 창에 이벤트를 쏴야 함), 무엇보다 **유니크 소유를 타입 수준 불변식**으로 박아 준다(`HashMap<ViewId, WindowLabel>` = View당 정확히 1창). 스캔-파생은 이 불변식을 런타임 assert 로만 지킬 수 있어 채택 안 함. 정합성은 불변식 1(양방향 일관성)이 지킨다 — 갱신은 항상 `windows[L].tabs` ↔ `view_owner[v]` 쌍으로.

### 2-2. 불변식 (★load-bearing — 코드에 `// ADR-` 앵커로 박음★)

1. **양방향 일관성:** `view_owner[v] == L` ⟺ `windows[L].tabs.contains(v)`. (한쪽만 갱신 금지 — 갱신은 항상 쌍으로.)
2. **유니크 소유:** 모든 `v ∈ views`는 `view_owner`에 정확히 1개 엔트리. **한 View는 두 창에 못 속함.** (§10-3 item 5 = 현 `output_router.rs:156` "한 View 두 창" 허용 제거.)
3. **활성 소속:** `windows[L].active ∈ windows[L].tabs` 항상.
4. **메인 최소 1탭 + 메인 non-closable:** `windows["main"].tabs.len() >= 1` 불변(D-3). **메인 창 자체는 닫히지 않는다** — `lib.rs:142` CloseRequested arm 이 `"main"` 라벨을 `prevent_close`+`hide`(트레이 상주, ADR-0026/0029)로만 처리하고 Destroyed 를 안 남긴다. 따라서 `close_window("main")` 은 **금지**(§5-2 `close_tab` 마지막 탭 분기가 main 이면 `close_window` 대신 빈 탭 강제로만 떨어진다) — command 레이어가 `main` 대상 `close_window` 를 거부한다. (G11)
5. **에이전트 참조 다중 허용:** 같은 `agent_id` 문자열이 서로 다른 두 View의 슬롯에 배정될 수 있음(→ 두 창이 같은 에이전트 봄, 진도 독립·ADR-0046). "한 View 두 창"(불변식 2 금지)과 **다른** 얘기.

### 2-3. 부팅 초기화

```
views       = { v0: View::new() }
view_owner  = { v0: "main" }
windows     = { "main": WindowTabs { tabs: [v0], active: v0 } }
```
`agent-tree`는 `windows`에 안 들어감(config 창, /tree 렌더).

---

## 3. 마이그레이션 (§10-3 item 1)

### 3-1. 데이터 마이그레이션은 없다
저장/복원(D-5)이 아직 없어 **런타임 상태만** 존재 → 디스크 스키마 이전 불필요. "마이그레이션" = 순수 **코드 리팩터**(인메모리 모델 + 부팅 init + 소비처).

### 3-2. `agent-tree` 창 (특수 창 — 비대상)
- 실측: `agent-tree`는 config 창(`tauri.conf.json` `url: index.html#/tree`, `visible:false`)으로 **`TreePage`**(에이전트 트리)를 그린다. 런타임 `window_bindings`에 **실제로 안 들어감**(`output_router.rs` 테스트의 `"agent-tree"` insert는 일반 메커니즘 예시일 뿐, 부팅 코드에 실 insert 없음).
- **결론: 탭 모델 밖 유지.** 탭바 없음. `lib.rs` Destroyed arm의 `main`/`agent-tree` 분기·`is_popup_label` prefix 판정도 그대로.
- **`agent-tree`는 슬롯 캔버스(`WindowLayout`, §7)를 마운트하지 않는다** — `TreePage`(에이전트 트리)를 그대로 그린다. 그래서 §7 통일 경로(`WindowLayout(label)`)의 대상이 아니고, `windows` 밖이라 탭 라우팅도 없다. (구현 시 확인: `TreePage`가 `ViewLayoutRenderer`를 쓰지 않음을 재확인 — 쓰면 D-2 통일 대상으로 격상.)

### 3-3. 팝업 창 + `?view=` URL (§10-3 item 1 핵심)
- **현재:** 팝업(`slot-popup-N`)은 `#/popup?view=<id>` URL로 **고정 단일 View**를 그림(`PopoutPage`, 탭 전환 없음). 창→View는 `window_bindings`에 `pop_out_slot`이 넣어둠.
- **후:** 팝업도 **탭 가진 일반 창**(D-2, PRD §5 "팝업도 탭"). 팝업 창 = `windows["slot-popup-N"] { tabs:[v], active:v }`.
  - **URL 키 전환: `?view=<id>` → `?window=<label>`.** 팝업 페이지는 이제 "고정 뷰"가 아니라 "이 창의 활성 탭"을 그리고 자기 탭바를 띄운다. (창 label이 SSOT, 활성 탭은 백엔드 `windows[label].active`에서.)
  - `pop_out_slot` → `move_slot_to_window`(§8): 슬롯 분리 시 새 팝업 창 = 그 슬롯의 에이전트를 담은 탭 1개로 시작.
  - **팝업이 "자기 활성 탭"을 배우는 경로(G3 — 필수, `?window=` 키 전환만으로는 부족).** 팝업 출력 Channel 은 URL 이 아니라 `window.label()` 로 구독한다(`agent.rs:143`) → URL 키를 `?window=` 로 바꿔도 프론트가 활성 탭을 스스로 알 방법이 없다. 그래서 `WindowLayout(label)`(§7)은 mount 시 **① 초기 pull** = `list_tabs(label)`(반환: `{tabs, active, version}`, §6) 로 활성 탭을 확정하고, **② `window:tabs-updated{label,tabs,active,version}` listen** 으로 `switch_tab`/`create_tab`/`close_tab` 시 활성 탭을 스왑·재렌더한다(자기 `label` 과 일치할 때만). 없으면 팝업이 고정 탭만 그려 D-2 "팝업도 탭"이 회귀한다.
  - **dev 리로드로 stranded 된 stale `?view=` 팝업 = accept + 문서화(결정, G3).** 팝업은 **런타임 전용**(D-5 저장복원 범위 밖)이라 dev HMR/수동 리로드 후 옛 `?view=` URL 로 다시 뜬 팝업이 이미 정리된 창이라 에러/빈 화면을 보이는 건 **수용한다**. `?window=`∥`?view=` 이중 폴백은 **넣지 않는다**(런타임-전용 상태에 영속 복원 로직을 얹지 않음 — 범위 오염). 리로드된 팝업이 유효 label 을 못 찾으면 "이 팝업 창을 닫으세요" 안내만 띄운다.

### 3-4. `active_view_id` 소비처 정리 (프론트 — 완전 인벤토리, G7)
- `viewStore.ts`의 전역 `active_view_id` → **창별 활성**으로 대체. `useCurrentViewId()`는 "이 웹뷰가 어느 창인지 → 그 창의 active 탭"으로 해석(main = `windows["main"].active`, 팝업 = `?window=` label의 active).
- **전역 `activeViewId`를 읽는 기존 소비처 전부 `useCurrentViewId()`(창별)로 갈아끼운다 — 누락 시 팝업/보조창이 엉뚱한 main view 를 건드림:**
  - `viewStore.ts:273` `resolveDefaultViewId()` — 지금 `readViewIdFromHash() ?? activeViewId`. `?view=` hash 파싱을 제거하고 "이 웹뷰 창의 active 탭"(§7 `useCurrentViewId`) 폴백으로 바꾼다.
  - `SlotContextMenu.tsx:31/34` — `activeViewId` + `viewIdOverride ?? activeViewId`. `WindowLayout`(§7)이 자기 창 active 탭을 `viewIdOverride` 로 내려꽂아 이 컴포넌트는 창 무지(창별 active 를 prop 으로만 받음)로 남긴다.
  - `AgentTree.tsx:209` — `vs.activeViewId`(포커스 슬롯 배치 좌표). agent-tree 는 탭 모델 밖이라 아래 규칙 적용.
- **`agent-tree` 창의 `useCurrentViewId()` 처리(G7 — 모델 밖 폴백):** agent-tree 는 `windows` 에 없어 창별 active 탭이 **없다**(undefined). agent-tree 의 "포커스 슬롯에 배치"(`AgentTree.tsx:209`)는 자기 창엔 슬롯 캔버스가 없으므로 **main 창의 active 탭**을 대상으로 배치한다 — `useCurrentViewId()` 가 agent-tree 컨텍스트에서 `windows["main"].active` 로 폴백(agent-tree 는 main 을 조작하는 config 창이라는 특례). main active 를 못 구하면 종전처럼 "활성 뷰/포커스 슬롯 없음" 에러.
- `list_views`/`view_metas`의 "바인딩된 뷰 필터 + Fix 2B active 방어가드" 로직 **제거** — 유니크 소유라 `windows[L].tabs`가 곧 탭 목록(필터 불필요). `close_view`의 "unbound 뷰로 active 승계" 로직도 §5-2 규칙으로 교체.
- **`applyViewListUpdated` 필드 개명(재리뷰 nit):** 구 `ViewListPayload {views, active_view_id}`(`layout.rs:56`) → 새 `list_tabs`/`window:tabs-updated` 페이로드는 `{tabs, active, version}`. 프론트 `applyViewListUpdated`(`viewStore.ts:187`)가 `active_view_id`→`active` 개명 + `version` 수용하도록 갱신(누락 시 활성 탭이 안 붙음). `get_view(viewId)`(id 조회)는 계약 불변.

---

## 4. 라우팅 / replay (§10-3 item 2 — ADR-0046/0056 정합)

### 4-1. `OutputRouter.rebuild` 새 로직 (`output_router.rs`)
```
by_agent: HashMap<AgentKey, Vec<WindowLabel>> = {}
for (label, wt) in &windows {
    for vid in &wt.tabs {                 // ★활성뿐 아니라 모든 탭★ (ADR-0056 keep-alive)
        collect_agents(views[vid].layout, &[label], &mut by_agent);
    }
}
// dedup/sort → ArcSwap store → prev와 diff (§4-3)
```
- **핵심 변화(G5 — 라우팅 반전):** 현 로직(`output_router.rs:159`)의 `if view.id == mgr.active_view_id { windows.push(MAIN_WINDOW_LABEL) }` 분기 + `window_bindings` 스캔을 **완전히 제거**하고, "각 창의 모든 탭 walk"로 대체. `active_view_id`·`MAIN_WINDOW_LABEL` 상수는 `by_agent` 계산에서 **전면 배제**(더는 active flag 가 라우팅에 안 들어감). 유니크 소유라 한 View는 한 창에만 → 이중 집계 없음.
- **숨은 탭도 라우팅한다** — ADR-0056 "출력 계속 누적"의 백엔드 대응. 전환 시 프레임 유실 0 → replay 불필요(핫패스).

### 4-2. 라우팅 ≠ 좌석 — 두 축은 직교 (G6 정정)
**라우팅(백엔드)과 WebGL 좌석(프론트)은 다른 자원이라 혼동 금지:**
- **라우팅** = xterm *버퍼*로 프레임을 흘리는 것. 숨은 탭도 프레임을 계속 **받아 버퍼에 쌓는다**(xterm 인스턴스 유지, §4-1 "모든 탭 walk"). 라우팅 대상 수엔 상한이 없다(버퍼는 메모리만 씀).
- **WebGL 좌석** = 프론트 `WebglAddon`이 잡는 GPU 컨텍스트(희소, 실측 16). 숨은 탭은 `WebglAddon`을 **dispose**(좌석 반납, ADR-0056) → 좌석 소비 = **보이는 슬롯만**.
- 따라서 라우팅이 "모든 탭"이어도 좌석은 안 늘어난다. 실 불변식(**동시에 보이는 슬롯 ≤ 16**)은 라우팅이 아니라 **프론트 소관 = §7**에 산다.

**보이는 슬롯 상한 가드(ADR-0056, D-6 무한 창 대응):**
- `create_window`(D-6)로 창을 무한히 열 수 있어 보이는 슬롯이 16을 넘길 수 있다. 하지만 데이터 유실은 없다 — ADR-0056 `onContextLoss→DOM` 폴백(`TerminalSlot.tsx`)이 좌석을 못 얻은 슬롯을 **DOM 렌더로 graceful-degrade**(렌더만 강등, 라우팅·버퍼는 그대로) → 프레임 0 유실.
- **ADR-0056 상한 가드:** 설계 상한 = 창3×탭2×슬롯4(보이는 12 ≤ 16). 이 상한을 넘기는 레이아웃(창/탭/슬롯 최대치 상향)은 **ADR-0056 재검토를 요한다** — command 레이어에 count 노트: `create_window`가 창 수를 늘릴 때 이 상한 근접을 로그로 남기고(하드 블록 아님), 초과 레이아웃은 재검토 트리거. (§7 상세.)

### 4-3. Subscribe/Unsubscribe delta (ADR-0046 불변)
- 에이전트 구독 유지 조건 = **어느 창의 어느 탭에든 그 에이전트 슬롯이 존재.** rebuild가 `prev`와 diff:
  - `to_unsubscribe`(1→0, 마지막 노출 사라짐) → wire로 데몬에 Unsubscribe 발화(누수 정리).
  - `to_subscribe`(0→1) → **wire 발화 안 함.** 프론트 `request_replay`가 단독 형성(ADR-0046 BLOCK-1).
- `close_tab`/`switch_tab`은 노출 집합을 바꾸므로 rebuild 필수. `switch_tab`은 keep-alive라 **노출 집합 불변**(모든 탭이 이미 구독) → 대개 no-op delta지만 rebuild는 계약상 호출(활성 표시만 바뀜).

### 4-4. 같은 에이전트 두 창 = 진도 독립 (ADR-0046)
불변식 5로 같은 `agent_id`가 두 View에 배정 가능. replay는 뷰 직결(슬롯별 `lastDeliveredSeq`)이라 두 View의 seq 커서 독립 → 두 창 스크롤/진도 독립. 백엔드는 두 창 label 모두에 라우팅.

---

## 5. 동시성 contract + 상태기계 (§10-3 item 3·4, ADR-0006)

### 5-1. 임계구역 (ADR-0006 유지)
모든 탭/창 mutation command는 **단일 패턴**:
```
lock(ViewManager)
  → 모델 변경(views/view_owner/windows) + 불변식 유지
  → OutputRouter.rebuild(&mgr) (delta 산출)
  → send_subscription_delta(delta.to_unsubscribe 만) try_send (동기 non-blocking, 락 안)  // ★ADR-0046 BLOCK-1★
unlock
  → emit(창별 이벤트) · (async 창 빌드는 언락 후)
```
- 여러 command 동시 호출은 ViewManager Mutex로 직렬화 → rebuild 인터리브 없음(현 계약 유지). 락 보유 중 웹뷰 빌드·네트워크·emit 금지.
- **wire 발화는 `delta.to_unsubscribe`(1→0 정리)만(G11 — ADR-0046 BLOCK-1).** `delta.to_subscribe`(0→1)는 산출만 하고 **wire 로 안 보낸다** — 구독 형성(replay)은 뷰 주도 `request_replay` 단독이다. 코더가 편의상 eager Subscribe 를 재추가하면 이 불변식이 깨진다(BLOCK-1 회귀).

### 5-2. `close_tab` 상태기계 (D-3 확정)
```
close_tab(L, v):
  assert view_owner[v] == L
  remove v from windows[L].tabs, views, view_owner    // ← view 1개 드롭(불변식 1 쌍 갱신)
  if closed v == windows[L].active:
      windows[L].active = 인접 탭(오른쪽 우선, 없으면 왼쪽)   // 탭 남아있을 때
  if windows[L].tabs.is_empty():
      if L == "main":  create_tab("main")  // 빈 탭 1개 강제 (불변식 4, main non-closable)
      else:            close_window(L)      // 팝업: 창도 닫힘, 에이전트 생존 (D-3 자가닫힘)
  rebuild + emit(window:tabs-updated{L,...})
```
- 에이전트 kill 안 함(§5 분리).

#### 자가닫힘 신호 = "이 창 0탭 = 창 닫힘" (G2 — 이중 발화 재조정)
- **신호 재정의(D-3):** 팝업 자가닫힘의 트리거는 **"이 창의 `tabs` 가 0이 됨 = 창이 닫힘"** 이다 — 단일 뷰 소멸(`view:closed`)이 **아니다**. `view:closed{id}`(한 View 제거)는 탭바 갱신 신호일 뿐 창 닫힘이 아니다(둘을 혼동하면 마지막 아닌 탭을 닫아도 창이 죽는다).
- **이중 발화 재조정(회귀 방지 — `layout.rs:49` "재진입 위험"):** `close_tab(popup, last)` 는 백엔드에서 `close_window(L)` 한 경로로만 창을 닫는다 → OS Destroyed → `lib.rs` Destroyed arm 이 `cleanup_popup_window`(§5-2/G1)로 잔여 정리(구독/Channel/모델). **프론트로 별도 `view:closed`→`PopoutPage.close()` 를 쏘지 않는다**(옛 경로가 `close_window`+`view:closed` 이중 발화로 창을 두 번 죽이려 해 재진입/연쇄 붕괴했다). 즉 창 닫힘 = 백엔드 `close_window` 단일 소스, 프론트는 `window:tabs-updated{tabs:[]}`(0탭) 수신 시 자기 창을 닫는 것만 idempotent 하게(이미 닫히는 중이면 no-op) 처리한다.
- **`view:closed` 완전 은퇴 (G2 재리뷰 잔여 — 이름만 바꾸면 이중발화가 실제로 안 사라진다):** 위 재조정이 성립하려면 옛 경로를 **엔드투엔드로 제거**해야 한다: ① `close_view`/`close_tab` 의 `view:closed` **emit 제거**(더는 안 쏨), ② `PopoutPage` 의 `view:closed`→`getCurrentWindow().close()` **리스너 제거**(`PopoutPage.tsx:66-72`) — 자가닫힘은 `window:tabs-updated{tabs:[]}` 단일 신호로 대체(§7-1). 탭 제거 자체는 `window:tabs-updated` 의 줄어든 `tabs[]` 로 이미 드러나 `view:closed` 는 **잉여**다. ③ `PopoutPage.test.tsx` 의 `view:closed{own}`→`close()` 단언은 옛 계약이므로 **삭제/반전**(§8 스테이지 4 — G5 테스트 반전과 병렬). 남겨두면 `close_tab(비-마지막)` 이 `view:closed` 를 쏠 때 stale `PopoutPage.close()` 가 튀어 재진입한다.
- **정리 경로 = 아래 `cleanup_popup_window` 멀티탭 정리(G1):** 팝업 창 닫힘(자가닫힘·titlebar·강제 Destroyed 전부)의 잔여 정리는 `windows[L].tabs` **전부 순회** 드롭 + `view_owner` 제거 + rebuild 1회 + Unsubscribe delta.

#### `cleanup_popup_window` 멀티탭 정리 (G1 — ★concrete leak 수정, 최우선★)
- **현 버그:** `cleanup_popup_window`(`popout.rs:269`)는 `window_bindings.get(label)` **하나만** 정리 → 멀티탭 팝업을 titlebar/강제 Destroyed 로 닫으면 나머지 탭 View 들이 `views`/`view_owner` 에 **잔류** + 데몬 구독 **Unsubscribe 안 됨**(누수).
- **후(새 모델):** Destroyed 정리는 `windows[label].tabs` 를 **전부 순회**해 각 View 를 `views` + `view_owner` 에서 드롭하고, `windows` 에서 label 엔트리 제거 → `rebuild` **1회**(마지막에) → 그 rebuild 델타의 `to_unsubscribe`(어느 창에도 안 남은 agent)를 데몬에 발화. rebuild 를 탭마다 부르지 않고 전부 드롭 후 1회(락 1구간). headless 테스트(§8 스테이지 1)에 **필수** 포함 — §8 스테이지 6(GUI)로 미루지 않는다.

### 5-3. `move_slot_to_window` / pop-out mid-flight 롤백 (§10-3 item 4)
반환 타입 = **`{ window: WindowLabel, tab: ViewId }`**(G4 — bare `Result` 아님. 호출자가 옮겨간 창·탭을 안다). 현 `pop_out_slot` 2-phase 를 유지·확장하되, **기존 창 타깃의 탭 삽입을 phase C로 미룬다**:
```
phase A (lock): 소스 슬롯 에이전트 read → 대상 View 생성(임시, 아직 어느 창 tabs 에도 안 넣음) → assign → rebuild(pre-build)
phase B (unlock): WebviewWindowBuilder (async — 데드락 회피 위해 반드시 언락)
                  · 새 창 타깃: 여기서 창을 빌드 · 기존 창 타깃: 빌드 없음(창 이미 존재), 존재만 확인
phase C (lock): ┌ 새 창: 새 label 로 windows 엔트리 생성 + tab 삽입(create_tab 상당)
                ├ 기존 창: ★view_owner[?]/windows[to_window] 재검증 후★ 그 창 tabs 에 삽입 — 여기서 처음 삽입(phase A 아님)
                └ 공통: 소스 슬롯 close → rebuild + emit(양 창 window:tabs-updated)
rollback: phase B 실패 or phase C 재검증 실패 시 phase A 임시 View 원복(close_view 상당) + 소스 유지
```
- **★기존 창 타깃 orphan 방지(G4 핵심)★:** 기존 창 tabs 삽입을 **phase A 가 아니라 phase C**로 미룬다. phase A 에 선삽입하면 phase B(언락 async) 중 대상 창이 소멸/동시 close_window 될 때 orphan 탭이 남는다(현 팝업 코드는 새 창만 다뤄 이 클래스가 없었다). phase C 에서 **`to_window` 가 여전히 존재하는지 재검증**(`windows.contains_key(to_window)`) 후에만 삽입 — 부재면 삽입 안 하고 롤백. (소스 슬롯 detach 만 종전대로 2-phase = phase A 예약·phase C close.)
- **재검증 앵커:** 기존 창 삽입 직전 `windows[to_window]` 존재 + 삽입 후 `view_owner[tab] == to_window` 를 세운다(불변식 1·2 쌍 갱신). 새 창 label 은 `PopupCounter`(단조·재사용 없음)라 재사용 충돌 없음(§6/G8: `create_window` 도 같은 카운터/prefix).
- **동일 에이전트 두 탭 = 허용(불변식 5, G4 — 코더 오버가드 금지):** 같은 `agent_id` 를 이미 다른 탭이 보고 있는 창으로 옮겨도 정상이다(두 탭이 같은 에이전트, 진도 독립·ADR-0046). move 경로에 **스퓨리어스 dedup**(이미 그 창에 그 agent 가 있으면 거부/스킵)을 넣지 말 것 — "한 View 두 창"(불변식 2 금지)과 **다른** 얘기다.

---

## 6. Command 표면 (§5 LLM-우선 제어 — 불변)

모든 탭/창 조작은 command로 노출(LLM = 메인 조작 주체, 사람 클릭 = 같은 command). 기존 view-언어 → **탭-언어로 개명**(사용자·LLM 대면 표면 정합, PRD 시나리오 용어).

| command | 시그니처 | 동작 | 기존 대응 |
|---|---|---|---|
| `create_window` | `() -> WindowLabel` | 빈 새 창(빈 탭 1개) + 웹뷰 빌드. label = **`PopupCounter`/`slot-popup-*` prefix 재사용**(G8). (D-6) | 신규 |
| `create_tab` | `(window, name?) -> ViewId` | 그 창에 새 빈-슬롯 탭 추가·활성화. | `create_view`(전역) |
| `switch_tab` | `(window, view) -> Result` | 그 창의 active만 교체(타 창 불변). | `switch_view`(전역 active) |
| `close_tab` | `(window, view) -> Result` | §5-2 상태기계. | `close_view` |
| `close_window` | `(window) -> Result` | 창 통째 닫기. **`"main"` 금지**(불변식 4/§2-2 — main 은 hide only, G11). | 신규 |
| `move_slot_to_window` | `(from_view, slot, to_window?) -> {window, tab}` | 슬롯 에이전트를 다른 창 새 탭으로(없으면 새 창). §5-3(phase-C 삽입·재검증, G4). | `pop_out_slot`(확장) |
| `split_slot`/`close_slot`/`assign_agent` | `(view, …)` | View 내부 조작(view_id 전역 유니크라 시그니처 유지, 소속 창은 `view_owner`에서 O(1) 파생). | 동일 |
| `list_windows` / `list_tabs` | `() ` / `(window) -> {tabs: ViewMeta[], active, version}` | 부팅·탭바 초기화용 read-only. `list_tabs` 는 **`version` 포함**(G10, stale 방어). | `list_views`(재편) |
| `spawn_into` | `(window, tab?, slot?, backend, cwd, …) -> AgentId` | **D-7**: 스폰(데몬) → tab 없으면 create_tab → 슬롯 배정(정책 ↓). 실패 시 에이전트 생존·보고(하드 롤백 X). | 신규(합성) |

- **`create_window` label 재사용(G8):** `create_window`(D-6)는 별도 라벨 스킴을 만들지 않고 **`PopupCounter` + `slot-popup-*` prefix 를 그대로 쓴다**. 근거: `lib.rs:157` Destroyed 정리 게이트가 `is_popup_label`(`popout.rs:309`, prefix 매칭)로 걸린다 — 다른 라벨이면 창 닫힘 시 `cleanup_popup_window` 가 안 돌아 라우팅/구독/Channel 이 누수된다. (라벨 의미가 "팝업"에서 "런타임 창"으로 넓어지므로 상수 주석만 갱신 — prefix 값은 불변.)
- **`spawn_into` 슬롯 정책(G9 — 코더 추측 방지):**
  - **`slot` 미지정:** 대상 탭의 **빈 root 슬롯**에 배정한다(create_tab 로 새로 만든 탭이면 빈 슬롯 1개가 이미 있음 → 거기). 탭도 미지정이면 create_tab 로 새 탭을 만들어 그 빈 root 슬롯에.
  - **`slot` 지정·비어있음:** 그 슬롯에 배정.
  - **`slot` 지정·점유(이미 agent 있음):** **에러**(덮어쓰지 않음) — 호출자가 먼저 `split_slot` 으로 빈 슬롯을 만든 뒤 재시도한다(자동 split/replace 안 함 = 파괴적 추측 회피).
  - **실패 가시성(§5 분리):** 스폰은 성공했는데 배정 단계가 실패하면(점유 슬롯 등) **에이전트는 데몬에 살아있고** `list_agents` 로 재부착 가능하다(하드 롤백=kill 안 함). 호출자에게 "배정 실패(에이전트 <id> 는 살아있음)"을 보고 → invisible 에이전트 방지.

- **이벤트:** 창별 탭 갱신 = `window:tabs-updated { label, tabs: ViewMeta[], active, version }`(현 전역 `view:list-updated` 대체·창 스코프). **`version` 부착(G10):** `windows`/`views` 변경마다 오르는 전역 `version`(§2 유지)을 실어 stale emit 를 프론트가 폐기(PopoutPage race 미러 규율과 정합). `layout:updated`(뷰 스냅샷)는 유지. **`view:closed` 이벤트는 엔드투엔드 은퇴(G2)** — 더는 emit 안 하고 `PopoutPage` 리스너도 제거(§5-2). 탭 제거는 `window:tabs-updated` 의 줄어든 `tabs[]` 로 드러나고, 팝업 창 자가닫힘은 `window:tabs-updated{tabs:[]}`(0탭) + 백엔드 `close_window`→Destroyed→`cleanup_popup_window` 로 처리한다(§5-2/G2).
- **개명은 내부→외부 표면 변경이라 보고 대상**(동작 동일, 이름만 탭-언어). 구 이름 호출부(프론트·문서) 일괄 갱신.

---

## 7. 프론트 인터페이스 (`src/`)

### 7-1. 단일 `WindowLayout(label)` 경로 — main·팝업 통일 (G7 — D-2 "동일 코드경로")
- **신규 `WindowLayout(label)` 컴포넌트:** 창 하나의 탭바 + 활성 탭 슬롯 캔버스를 그리는 단일 컴포넌트. **main 창과 팝업 창이 둘 다 이걸 마운트**한다(각자 자기 `label`) → 옛 "`AppLayout`이 전역 active 렌더 vs `PopoutPage`가 고정 뷰 렌더"의 갈라짐(D-2 위반)을 제거한다.
  - main: `WindowLayout("main")` — Sidebar/DiffPanel/StatusBar 등 창 크롬(chrome)은 `AppLayout` 이 감싸고, 슬롯 영역만 `WindowLayout("main")` 로 교체.
  - 팝업: `WindowLayout(readWindowFromHash())`(§3-3 `?window=<label>`) — `PopoutPage` 는 이 컴포넌트를 얇게 감싸는 껍데기로 축소.
  - **`agent-tree` 는 이 경로 밖(§3-2 불변):** `TreePage`(에이전트 트리)를 그대로 그린다 — 슬롯 캔버스가 아니므로 `WindowLayout` 을 안 쓴다.
- **`WindowLayout` 동작:** mount 시 `list_tabs(label)` 초기 pull(§6, `{tabs,active,version}`) + `window:tabs-updated{label,...}` listen(자기 label 만). `windows[label].tabs` **전부 마운트·활성만 표시**(ADR-0056 keep-alive: 숨은 탭 `display:none`, xterm 유지·WebglAddon dispose). 0탭 수신 시 자기 창 자가닫힘(§5-2/G2, idempotent).

### 7-2. `useCurrentViewId()` + 탭바 + 좌석 상한
- **`TabBar`(신규):** 각 창 상단(WindowLayout 안). `windows[label].{tabs,active}` 렌더 + `[+]`(create_tab) + 탭별 닫기(close_tab). shadcn 탭 스타일(순수 내부). 위치·스타일 = 메인 결정(보고).
- **`viewStore`:** 전역 active → 창별 상태. `useCurrentViewId()` = 이 웹뷰 창의 active 탭(main = `windows["main"].active`, 팝업 = `?window=` label 의 active, **agent-tree = `windows["main"].active` 폴백** — §3-4/G7 특례). `SlotContextMenu` 는 `WindowLayout` 이 내려꽂는 `viewIdOverride`(= 그 창 active 탭)만 쓰고 전역 `activeViewId` 참조 제거(§3-4).
- **★보이는 슬롯 좌석 상한은 여기가 정본(§4-2/G6)★:** 동시에 보이는 슬롯 ≤ 16(WebGL 좌석 실측). 초과분은 `TerminalSlot` 의 `onContextLoss→DOM` 폴백(ADR-0056, **이미 존재** — 여기 `// ADR-0056` 앵커 달 곳)이 DOM 렌더로 graceful-degrade → 데이터 유실 0. 설계 상한(창3×탭2×슬롯4=보이는 12)을 넘기는 레이아웃은 ADR-0056 재검토 트리거(§4-2).
- **Ctrl+Tab(D-8):** 포커스된 창의 탭 순환 = `switch_tab(currentWindow, next)`. 프론트 키핸들 → command(사람 클릭과 동일 경로).

---

## 8. 구현 스테이징 (모듈 경계 DDD·ADR-0012 격리 — `/implement critical`)

각 스테이지 = 테스트 먼저(또는 함께) + 누적 회귀. 코어/레이아웃은 Tauri seam 밖에서 headless 검증.

1. **모델(`manager.rs`)** — 새 struct + 불변식 1~5 + `WindowTabs`. 단위테스트(headless): create/switch/close_tab 상태기계(§5-2), 유니크 소유 위반 방지, 메인 최소1탭, **`close_window("main")` 거부**(G11). **★G1 필수 headless 테스트★: 멀티탭 팝업 강제정리** — `windows[label].tabs` 에 View 2+ 를 넣고 `cleanup_popup_window` 상당 경로를 돌려 모든 View 가 `views`+`view_owner` 에서 빠지고 rebuild 델타에 남은 agent 전부의 Unsubscribe 가 나오는지 단언(잔류 0). `tree.rs` 불변.
2. **라우팅(`output_router.rs`)** — rebuild 새 로직(§4-1) + delta diff(§4-3). **★G5 — 기존 테스트 계약 반전★:** `active_view_id` 라우팅 분기(`output_router.rs:159`)를 제거하고 `diff_switch_view_changes_visible_set`(`output_router.rs:557`)는 **삭제/반전**한다 — keep-alive 에선 `switch_tab` 이 노출 집합을 안 바꿔 델타가 **no-op**이라, 옛 "switch → A 빠지고 B 들어옴" 단언이 정면 위배된다(옛 테스트·옛 active 분기를 남기면 keep-alive 가 깨진다). `active` flag 가 `by_agent` 에서 완전 배제됨을 확인하는 테스트로 대체. 기존 `agent-tree` 예시 테스트 → 새 모델(모든 탭 라우팅·두 창 같은 에이전트·close→unsubscribe)로 갱신.
3. **command + 동시성(`commands/layout.rs`·`popout.rs`)** — §6 표면 + §5 임계구역 + `move_slot_to_window` 롤백(§5-3) + 창별 이벤트.
4. **프론트(`src/`)** — 단일 `WindowLayout(label)`(§7-1) + `TabBar` + `viewStore` 창별 + `AppLayout`/`PopoutPage` 탭화·keep-alive + Ctrl+Tab. **★G2 — `view:closed` 은퇴★:** emit·`PopoutPage` 리스너 제거 + `PopoutPage.test.tsx` 의 `view:closed`→close 단언 삭제/반전(스테이지 2 G5 테스트 반전과 병렬). **필드 개명(재리뷰 nit):** `applyViewListUpdated`(`viewStore.ts:187`) `active_view_id`→`active` + `version` 수용.
5. **`spawn_into`(D-7)** — 얇은 합성 command(스테이지 1~4 위). 실패 관대(에이전트 생존·보고).
6. **GUI 실측(§9)** — cdp + EnumWindows E2E.

---

## 9. 수용 기준 (PRD §6 + D-7)

1. 메인·팝업 각 창에 **탭바**, 그 창의 탭 목록·활성 탭 표시.
2. 탭 클릭/`switch_tab` → **그 창만** 전환(타 창 불변).
3. `create_tab`/`[+]` → 그 창 새 빈-슬롯 탭 추가·활성.
4. `close_tab` → 상태기계(§5-2): 메인 마지막=빈탭 유지, 팝업 마지막=창 닫힘, **에이전트 생존**.
5. 라우팅: 각 창 자기 모든 탭 에이전트 출력 수신·전환 무손실(ADR-0056). 같은 에이전트 두 창 진도 독립(ADR-0046).
6. GUI 실측(cdp+EnumWindows): 메인 탭 2개 생성·전환 + 팝업 탭 전환 E2E.
7. Ctrl+Tab 창내 순환(D-8).
8. **(D-7)** `spawn_into(창,탭?,슬롯?)` 한 방으로 스폰+배치 → 지정 창 탭 슬롯에 에이전트 등장.
9. 모든 조작 command 경로 동반(§5). 굵은 결정 ADR.

---

## 10. ADR·열린 항목

- **필요 ADR(구현 전 박제):** 탭 소유 모델 = **D-1 C(유니크 소유 + 창별 탭)** — 거부한 A(ref-list=소유)·B(글로벌풀), 근거(WezTerm/VS Code 검증·공유요구 부재). 이 ADR이 **ADR-0035의 `active_view_id`=main-전용 절을 supersede**(전역 active 제거). 라우팅 "모든 탭 수신"은 ADR-0056 파생이라 이 ADR에 함께 기술 or 참조.
- **순수 내부(메인 결정·보고, 결정 아님):** command 개명(view→tab 언어)·`TabBar` 위치/스타일/[+]·`window:tabs-updated` 이벤트명·인접 탭 승계 방향(오른쪽 우선)·`view_owner` 캐시 역인덱스 유지(§2/G11).
- **구현 시 확인:** §3-2 `TreePage`가 슬롯 캔버스 미사용 재확인 · `?view=`→`?window=` 전환의 기존 팝업 열림 경로 회귀 · dev 리로드 stale 팝업 안내 화면(§3-3/G3).
- **후속 문서(구현 무관):** PRD §6.5 "활성 탭만 수신" 문구는 ADR-0056(모든 탭 라우팅)로 **supersede** → PRD 문구 갱신을 `/review doc` 로 후속(G11). 이 TRD §4-1 이 정본.
- **범위 밖(후속):** D-4(창 간 드래그)·D-5(저장복원, D-7 로드맵 합류·팝업 영속 여기 포함)·구조화 인박스(Phase 3).

---

## 11. 적대 리뷰 반영 (/review trd deep — 2026-07-09)

판정: **3인 전원 FIX** (Designer blind=Codex · Architect-breaker doc-aware=Opus · 마이그레이션/동시성 전문 doc-aware=Opus). BLOCK 없음 — **방향(D-1 C·keep-alive 라우팅·2-phase 롤백) 검증됨**. 정면 대립 없음(세 관점 상호보완). FIX 항목이 많아 **초안은 큰 revision 필요 → 반영 후 재리뷰 후 `/implement`**. 아래 갱신 항목(중복 제거·근거 지점 포함).

### G1. 팝업 멀티탭 정리 누수 (★concrete leak — 최우선) [BLOCK급 근거]
근거: 현 `cleanup_popup_window`는 단일 `window_bindings.get(label)` 하나만 정리(`popout.rs:269`). 멀티탭 팝업 **강제종료**(close_tab 아닌 titlebar/Destroyed) 시 나머지 탭 View들이 `views`/`view_owner`에 잔류 + 데몬 구독 **Unsubscribe 안 됨**.
→ 본문 §5-2(cleanup 멀티탭 순회) + §8 스테이지 1(headless 테스트 필수) 반영.

### G2. 팝업 자가닫힘 이중 실행·재진입
근거: `close_tab(popup, last)` → §5-2 `close_window(L)`(→Destroyed→cleanup) **AND** `view:closed`→`PopoutPage.close()` 이중 발화(`layout.rs:49` "재진입 위험" 회귀).
→ 본문 §5-2("자가닫힘 신호 = 0탭=창닫힘" 재조정, idempotent, **`view:closed` 엔드투엔드 은퇴**) + §6 + §8 스테이지 4(리스너·테스트 삭제/반전) 반영. **[재리뷰 잔여 FIX 적용]** — 재리뷰에서 "은퇴가 이름만 바뀌고 실제 emit/리스너/테스트 제거가 미명시"로 PARTIAL 판정 → `view:closed` emit·`PopoutPage` 리스너·`PopoutPage.test.tsx` 단언 제거를 명시(+ `list_tabs` 필드 개명 nit = §3-4).

### G3. 팝업이 "자기 활성 탭"을 배우는 경로 부재 (D-2 회귀)
근거: 팝업 출력 Channel은 `window.label()`로 구독(URL 무관, `agent.rs:143`) → **URL 키만 `?view=`→`?window=`로 바꾸면 부족**(팝업이 활성탭을 스스로 못 앎).
→ 본문 §3-3(초기 pull `list_tabs` + `window:tabs-updated` listen) + §7-1(`WindowLayout` 동작) 반영. **결정:** dev 리로드 stale 팝업 = **accept+문서화**(런타임 전용·D-5 범위밖 — §3-3), 이중 폴백 미채택.

### G4. move_slot_to_window 롤백 — 기존창 타깃 orphan
근거: §5-3 phase A가 **기존 창** tabs에 `create_tab` 선삽입 시, phase B(언락 async 빌드) 중 타깃 창 소멸/동시 close_tab이면 orphan 탭(현 팝업 코드는 새 창만 다뤄 이 클래스 없음).
→ 본문 §5-3(기존창 삽입 phase C 이연 + `view_owner`/`windows[to_window]` 재검증 + `{window,tab}` 반환 + 동일-에이전트 두 탭 불변식5 허용/dedup 금지) + §6(반환 타입) 반영.

### G5. 라우팅 반전 = 기존 테스트 계약 뒤집음
근거: 현 rebuild는 `active_view_id`만 main 라우팅(`output_router.rs:159`) → 전환 시 노출집합 변함(테스트 `diff_switch_view_changes_visible_set`, `:557`이 단언). keep-alive(모든 탭 라우팅)로 바뀌면 **switch = no-op delta**.
→ 본문 §4-1(active 분기 제거) + §8 스테이지 2(테스트 삭제/반전 명시, active flag 배제 확인) 반영.

### G6. WebGL 좌석 상한 가드 (D-6 무한 창)
근거: §4-2가 라우팅↔좌석 혼동(라우팅=xterm 버퍼, 좌석=프론트 WebglAddon dispose). 실 불변식(**보이는 슬롯 ≤16**)은 §7 소관. `create_window`(D-6)=무한 창 → 보이는 슬롯 16 초과 가능. 완화 기존재: ADR-0056 `onContextLoss→DOM` 폴백이 graceful degrade.
→ 본문 §4-2(라우팅≠좌석 직교 정정 + 상한 §7 소관 + graceful degrade + create_window count 노트) + §7-2(좌석 상한 정본·ADR-0056 재검토 트리거) 반영.

### G7. 프론트 마이그레이션 인벤토리 + main/팝업 통일
근거: `active_view_id` 소비처 누락 — `AgentTree.tsx:209`(`vs.activeViewId`)·`SlotContextMenu.tsx:31/34`(`viewIdOverride ?? activeViewId`)·`resolveDefaultViewId`(`viewStore.ts:273`). agent-tree 창은 모델 밖(`useCurrentViewId()` undefined). AppLayout/PopoutPage 분리가 D-2 "동일 경로"와 어긋남.
→ 본문 §3-4(완전 인벤토리 + agent-tree `useCurrentViewId` main 폴백) + §7-1(단일 `WindowLayout(label)`, agent-tree 제외) + §7-2(`useCurrentViewId`) 반영.

### G8. create_window 라벨 = 팝업 prefix/counter 재사용
근거: `is_popup_label`(`popout.rs:309`, `slot-popup-*` prefix)이 Destroyed 정리 게이트(`lib.rs:157`). `create_window`(D-6)가 다른 라벨 스킴이면 정리 스킵 → 누수.
→ 본문 §6(`create_window` label = `PopupCounter`/prefix 재사용) 반영.

### G9. spawn_into 슬롯 정책 + 실패 가시성
근거: §6 `spawn_into` 슬롯 없음/점유 시 focus/split/replace/error 미정 → 코더 추측.
→ 본문 §6(슬롯 정책: 미지정→빈 root 슬롯 / 점유→에러·호출자 split / 실패 시 `list_agents` 재부착) 반영.

### G10. 이벤트 version 부착
근거: `window:tabs-updated`/`list_tabs`에 `version` 없어 stale 방어 불가(§2 `version` 유지·PopoutPage race 미러와 정합 필요).
→ 본문 §6(`window:tabs-updated`·`list_tabs` 에 `version` 부착) 반영.

### G11. nits/notes
근거·결정:
- `§5-1` pseudo "**delta.to_unsubscribe만**"(ADR-0046 BLOCK-1, eager Subscribe 재추가 방지) → 본문 §5-1 반영.
- **invariant4** `close_window("main")` 금지(메인 non-closable=hide only, `lib.rs:142`) → 본문 §2-2 invariant4 + §6(`close_window` `"main"` 금지) 반영.
- **`view_owner` 결정(BAKE):** cached reverse-index로 **유지**(O(1) owner lookup + 유니크소유 타입강제) — 스캔 파생 미채택 → 본문 §2 반영.
- **PRD §6.5 "활성탭만 수신" → ADR-0056(모든 탭)로 supersede** → PRD 문구 갱신은 `/review doc` 후속(§10). 이 TRD §4-1 이 정본.

### 재리뷰 조건
G1~G8 반영 후 재리뷰(트리거된 위험=멀티탭 정리/롤백/라우팅반전이라 최소 `light`(Adversary doc-aware 1인) 이상, 여력되면 `full`). PASS 후 탭 소유 모델 ADR 박제 → `/implement critical`.
