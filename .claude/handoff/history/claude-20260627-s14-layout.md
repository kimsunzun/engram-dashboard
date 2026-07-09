# S14 멀티 페이지 레이아웃 — 핸드오프 (2026-06-27, 갱신)

## 현재 상태

TRD rev.3 작성 완료. `/review trd full` 결과 **BLOCK** — F1 사용자 결정 대기 후 rev.4 재작성 필요.

---

## 확정된 결정

### Rust Authority 패턴 채택 (MVP 구조) — 사용자 확정

```
Rust 데몬/앱 — Views 보유 (단일 authority)
  └── views: Vec<View>
        ├── View { id, name, layout_tree }
        └── View { id, name, layout_tree }

모든 Tauri 창 = 순수 렌더러
  - invoke로 커맨드 → Rust 처리
  - Rust emit으로 상태 push → React 리렌더
```

거부한 대안: JS authority (rev.1, rev.2) — Tauri 창마다 JS 컨텍스트 독립 격리, 2번 BLOCK.

---

## TRD rev.3 BLOCK 결과 — 핵심 블로커

### F1 (BLOCK) — ViewManager 소유권 딜레마 ★사용자 결정 필요★

| 위치 | ADR-0029 정합 | `app.emit()` 가능 | 문제점 |
|---|---|---|---|
| **A안: src-tauri** | UI 레이아웃 ≠ 에이전트 호스팅이므로 허용 해석 가능 | ✅ | ADR-0029가 "에이전트 in-proc 호스팅 X"를 레이아웃 상태까지 금지하는지 해석 문제 |
| **B안: daemon** | ✅ 완전 정합 | ❌ 데몬은 별도 프로세스, WS only | WS→src-tauri→emit 중계 필요, 복잡도 증가 |

**추천(판단 전):** A안이 현실적. ADR-0029가 금지하는 것은 "AgentManager in-proc 호스팅"이고, ViewManager(UI 레이아웃 상태)는 다른 범주.

### F2 (BLOCK) — eventBus raw listen 추가 방식
F1 결정 후 자연 해소 가능. src-tauri에서 `app.emit()`하면 기존 eventBus Tauri listen 패턴과 동일.

### F3 (BLOCK) — 팝업 부팅 race
팝업 생성 후 listener 등록 전에 emit 발생 시 놓침. 해결책: `open_view_in_popup` invoke 후 팝업이 `get_view_layout(view_id)` invoke로 초기 상태 pull.

### 기타 FIX (rev.4에서 추가 명세 필요)
- crate 경계 명시 (`View`/`ViewManager`/`LayoutNode` → src-tauri 또는 protocol crate)
- Mutex lock 해제 후 emit 순서 명시 (ADR-0006 준수)
- edge case: View 0개 처리, 팝업 crash 시 `window_bindings` 정리, 마지막 View 닫기 정책
- 영속: ViewManager 재시작 복원 경로 (향후)

---

## 다음 세션이 해야 할 일

### 0단계: F1 사용자 결정 (A안 / B안)

먼저 사용자에게 확인한다.

### 1단계: TRD rev.4 (F1~F3 해소)

rev.3 기반으로 다음을 추가:

**F1 해소 (A안 선택 시):**
```rust
// src-tauri/src/lib.rs
app.manage(Arc::new(Mutex::new(ViewManager::new())));
// invoke handler는 State<Arc<Mutex<ViewManager>>>로 접근
// 처리 후 app_handle.emit("layout:updated", snapshot)
```

**F3 해소 (팝업 초기 pull):**
- invoke 목록에 `get_view_layout(view_id: Uuid) → LayoutSnapshot` 추가
- `PopupPage.tsx`: 마운트 시 `get_view_layout(viewId)` invoke로 초기 상태 수신 후 listen 등록

**SlotContent 타입 정합:**
```rust
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SlotContent {
    Agent { agent_id: String },
    Tree,  // 기존 kind:'tree' 호환
}
```

### 2단계: /review trd → PASS

### 3단계: 코더 서브에이전트

### 4단계: QA → `orch dashboard-qa`

---

## 주의사항

- **SlotPane.tsx / SlotContextMenu.tsx**: dashboard1이 작업 중일 수 있음 — 건드리기 전 조율.
- **tauri.conf.json**: `slot-popup` 정적 창 제거 대상.
- **ADR 필요**: Rust authority 채택 결정을 ADR로 박아야 함 (F1 결정 후 함께).
- **Tauri Issue #15583**: 팝업 닫을 때 `unlisten()` 명시 필수.

---

## 참조 문서

- 리서치: `docs/research/multi-tab-layout-state-management-research-2026-06-27.md`
- 리서치: `docs/research/multi-window-layout-sync-research-2026-06-27.md`
- 현재 TRD (rev.3, BLOCK): `docs/process/S14-multi-page-layout/trd.md`
- ADR 인덱스: `docs/decisions/README.md`
