# 멀티 창 레이아웃 상태 동기화 OSS 구현 패턴 연구

**상태:** 완료  
**날짜:** 2026-06-27  
**강도:** deep (Claude 팬아웃 4갈래 + Codex 독립 교차 + 적대 검증)  
**방법:** cross-family 독립 조사 → 클레임 단위 교차 대조 → 핵심 주장 반증 시도  
**확신도 범례:** [확실] = 공식 문서·코드 직접 확인 / [가능성 높음] = 복수 출처 수렴 / [불확실] = 단일 출처·추정

---

## 핵심 발견 (결론 먼저)

**공통 원칙 (전 프레임워크 합의):** "공유 레이아웃 트리를 여러 창이 직접 mutate하는 방식"은 모든 프레임워크에서 안티패턴이다. 안정적인 패턴은 일관되게 **단일 소스 authority + patch/event broadcast + 창별 독립 렌더링 + snapshot 영속**이다.

**Tauri v2 특화:** 각 창이 독립 JS 컨텍스트를 가지므로 메모리 공유 불가. Rust 백엔드를 authority로 두고 `emit/listen`으로 브로드캐스트하는 방식이 공식 권장이다. `plugin-store`는 영속용이지 실시간 동기화 버스가 아니다. `BroadcastChannel`은 작동하지만 Rust 백엔드가 상태 변화를 인식하지 못하는 한계가 있다.

---

## 1. 질문 1 — 독립 레이아웃 트리 vs 공유 트리 서브뷰

### 1-1. 독립 레이아웃 트리 (각 창이 자체 상태 보유)

**패턴:** 창마다 독립 상태 + 중앙 authority에서 동기화 이벤트 수신.

```
창 A (독립 상태)  ←── 이벤트 브로드캐스트 ──→  창 B (독립 상태)
                              ↑
                      Rust/Main authority
                      (단일 소스)
```

VS Code 실제 구현 — 창별 workbench가 독립 레이아웃을 보유하고, SharedProcess는 스토리지/서비스만 중앙화한다. Auxiliary window는 별도 container로 관리되며 `getWindows()`, `activeContainer`, `onDidChangeActiveWindow`로 레이아웃 이벤트를 받는다. [확실]

```ts
// VS Code layout.ts (요약)
hostService.onDidChangeFocus(focused => onWindowFocusChanged(focused));
hostService.onDidChangeActiveWindow(() => onActiveWindowChanged());
```

**장점:** 창 충돌 없음, 팝업 종료 시 정리가 단순(자기 상태만 폐기), 네트워크 지연·창 수에 무관하게 동작.  
**단점:** 동기화 지연 시 창 간 상태가 일시적으로 다를 수 있음. 해결책: revision gate(아래 §3 참조).

### 1-2. 공유 트리 서브뷰 (단일 상태를 여러 창이 구독)

**패턴:** 단일 상태 트리를 authority가 보유하고 창들은 서브 렌더러로만 동작.

```
Rust/Main authority (단일 레이아웃 상태)
       ↓ snapshot + patch 브로드캐스트
  창 A (렌더러)    창 B (렌더러)    팝업 C (렌더러)
```

Wezterm이 이 방식의 대표 구현이다. Mux(멀티플렉서)가 Window→Tab→Pane 계층 전체 상태를 보유하고, GUI 레이어(`TermWindow`)는 `MuxNotification` 구독자로만 동작한다. 변경은 Mux가 `notify()`로 브로드캐스트하고 GUI가 재렌더링한다. [확실]

```rust
// Wezterm: 상태 변경 → 브로드캐스트
pub fn notify(&self, notification: MuxNotification) {
    for subscriber in self.subscribers.lock().values() {
        subscriber(notification.clone());
    }
}

// Dirty detection: 변경된 라인만 전송 (전체 화면 아님)
let changes = session.compute_changes(seqno);
```

**장점:** 단일 소스라 일관성이 강함. 창 A와 창 B가 항상 같은 상태를 봄.  
**단점:** authority 장애 시 모든 창이 동시에 영향받음. 팝업에 렌더링이 필요한 창 고유 상태(스크롤 위치 등)는 별도로 관리 필요. 구현 복잡도 높음.

### 결론: Engram Dashboard 적용 판단

Tauri v2에서 `window.open`으로 서브 창을 생성하면 별도 JS 컨텍스트가 생기므로 공유 JS 상태 트리는 불가능하다. [확실] (Discussion #11643 확인)

따라서 실질적인 선택지는 두 가지다:
- **A안:** 독립 레이아웃 트리 + Rust authority + emit/listen 동기화 (표준, 권장)
- **B안:** Rust가 레이아웃 트리 전체를 보유하고 창들은 순수 렌더러 (Wezterm식, 더 강한 일관성)

B안이 LLM 제어 친화적이다 — Rust 상태가 단일 control surface가 되어 LLM이 직접 조작 가능. (CLAUDE.md §5 "LLM-우선 제어" 원칙과 정합)

---

## 2. 질문 2 — Tauri v2 창 간 상태 동기화 권장 패턴

### 2-1. emit/listen (공식 권장)

Tauri 공식 문서가 명시적으로 권장하는 방식. Rust ↔ JS 양방향, 창 간, 글로벌 브로드캐스트 모두 지원. [확실]

```rust
// Rust: 특정 창 타겟팅
app.emit_to("popup", "layout:update", &patch).unwrap();

// Rust: 조건 필터링 (다수 창)
app.emit_filter("layout:update", &patch, |target| match target {
    EventTarget::WebviewWindow { label } => 
        label == "main" || label == "slot-popup",
    _ => false,
}).unwrap();
```

```ts
// JS: 수신 및 cleanup
const unlisten = await listen<LayoutPatch>('layout:update', e => {
    applyLayoutPatch(e.payload);
});

// 팝업 종료 시 반드시 호출
unlisten();
```

**한계:** "작은 데이터 + 낮은 지연 시간 미보장" (공식 문서). 고빈도/대용량에는 `Channel` API 사용.

### 2-2. plugin-store (영속용 — 동기화 버스 아님)

공식 KV 저장소. 재시작 시 레이아웃 복원에 적합. 순서 보장 없고 "preferences용으로 설계"라고 명시. [확실]

**적합:** 레이아웃 스냅샷 영속 (앱 재시작 복원)  
**부적합:** 실시간 창 간 동기화 버스

### 2-3. BroadcastChannel (웹 표준 응용)

브라우저 표준 API로 같은 origin의 browsing context 간 통신. Tauri 웹뷰에서 작동 가능. [가능성 높음]

```ts
const channel = new BroadcastChannel('layout-sync');
channel.postMessage({ type: 'patch', patch });
channel.onmessage = e => applyLayoutPatch(e.data.patch);
channel.close(); // cleanup
```

**치명적 한계:** Rust 백엔드가 상태 변화를 인식하지 못함. Tauri 앱에서 Rust가 authority라면 이 방식만으로는 불완전. [확실]  
**적합 케이스:** Rust 백엔드와 관계없는 순수 UI 상태 동기화 (테마, 폰트 크기 등).

### 2-4. revision gate + snapshot/invalidation 패턴 (고신뢰)

state-sync 라이브러리가 구현한 패턴. 순서 보장 문제를 해결한다.

```
웹뷰 → invoke('layout:update', patch)
  → Rust: 상태 변경 + revision++
  → app.emit("layout:invalidated", { revision })
  → 모든 웹뷰: 로컬 revision < 수신 revision이면 invoke('layout:snapshot') 호출
```

```ts
// 무한 루프 방지 + revision gate
let localRevision = 0;
await listen('layout:invalidated', async e => {
    if (e.payload.revision > localRevision) {
        const snapshot = await invoke<LayoutSnapshot>('layout:snapshot');
        localRevision = snapshot.revision;
        applySnapshot(snapshot.data);
    }
});
```

**장점:** 순서 역전·burst 이벤트 자동 처리. 상태 일관성이 높음.  
**단점:** 구현 복잡도 증가. 단순 앱에는 over-engineering.

### Tauri v2 권장 우선순위 (Engram Dashboard 맥락)

1. **Rust State + emit/listen** — 메인 레이아웃 동기화 (LLM이 Rust invoke로 직접 제어 가능)
2. **revision gate** — 고빈도 레이아웃 변경(resize, drag)이 있을 경우 추가
3. **plugin-store** — 재시작 복원용 스냅샷 영속
4. **BroadcastChannel** — 순수 UI 상태(테마)만, Rust와 무관한 경우만

---

## 3. 질문 3 — 팝업 닫을 때 cleanup 패턴

### Tauri v2

⚠️ **알려진 버그:** 웹뷰 종료 시 JS 이벤트 리스너가 백엔드 `js_event_listeners` 맵에서 자동으로 제거되지 않음. Issue #15583 (2026-06-25 등록, 미해결). [확실]

**현재 필수 workaround:**

```ts
// 팝업 창 초기화 시
const unlisten = await listen('layout:update', handler);

// onCloseRequested에서 명시적 cleanup
const unlistenClose = await getCurrentWindow().onCloseRequested(async e => {
    // 1. 메인 창에 팝업 종료 알림
    await emitTo('main', 'popup:closed', { slotId: currentSlotId });
    // 2. 리스너 해제
    unlisten();
    unlistenClose();
});

// 또는 beforeunload (fallback)
window.addEventListener('beforeunload', () => {
    unlisten();
    // 동기적 notify (async 불가)
});
```

**주의:** `onCloseRequested`는 API로 닫을 때는 트리거되지 않는 케이스 있음 (Issue #5288). Rust 측 `Window::on_close_requested`를 병행 등록하는 것이 안전하다. [가능성 높음]

```rust
// Rust 측 cleanup (더 신뢰할 수 있음)
window.on_window_event(move |event| {
    if let WindowEvent::CloseRequested { .. } = event {
        // 팝업 상태 정리 + 메인에 알림
        app_handle.emit_to("main", "popup:closed", slotId).ok();
    }
});
```

### Electron

```ts
// main process
const popup = new BrowserWindow({ ... });

popup.on('closed', () => {
    windows.delete(popup);
    ipcMain.off('layout:patch', handler);  // ipcMain.removeHandler도
    popup = null;
});

// 각 창에 알림
for (const win of windows) {
    if (!win.isDestroyed()) win.webContents.send('window:closed', { id });
}
```

**주의:** `ipcMain.on()`은 리스너를 누적 추가함. 팝업별 핸들러는 `once` 또는 명시적 `off` 필요. [확실]

### VS Code (SharedProcess)

```ts
// shared process 연결 시 sender 생사 확인
if (e.sender.isDestroyed()) return port.close();

// 앱 종료 시
utilityProcess?.postMessage(SharedProcessLifecycle.exit);
```

---

## 4. 질문 4 — 창 간 포커스/활성 슬롯 동기화

### 핵심 모델: 포커스(OS 이벤트) ≠ 활성 슬롯(앱 상태) 분리

```
OS focus event
      ↓
windowFocused: boolean   (OS 이벤트 직접 반영)
      +
activeSlotId: string     (앱 상태 — 마지막 사용자 입력 기준)
      +
timestamp: number        (동시 포커스 충돌 해결용)
```

두 필드를 묶어 관리하면 "창 A가 포커스를 받았지만 활성 슬롯은 창 B의 슬롯" 같은 상황을 명확히 처리할 수 있다.

### Tauri v2

```ts
// 창 포커스 변경 시 메인에 알림
const unlisten = await getCurrentWindow().onFocusChanged(async ({ payload: focused }) => {
    if (focused) {
        await emitTo('main', 'window:focused', {
            windowLabel: getCurrentWindow().label,
            activeSlotId: getActiveSlotId(),
            timestamp: Date.now(),
        });
    }
});
```

내장 이벤트: `TauriEvent.WINDOW_FOCUS`, `TauriEvent.WINDOW_BLUR` (공식 문서 확인). [확실]

```rust
// Rust 측 포커스 이벤트 (더 신뢰할 수 있음)
app.listen("tauri://window-created", |event| { ... });
// 또는 window event handler
```

### Electron

```ts
// main process에서 포커스 이벤트 수집
BrowserWindow.getAllWindows().forEach(win => {
    win.on('focus', () => {
        broadcast('window:active', {
            id: win.id,
            activeSlot: activeSlots.get(win.id),
            timestamp: Date.now(),
        });
    });
});
```

### VS Code

```ts
// layout.ts (요약)
hostService.onDidChangeFocus(focused => onWindowFocusChanged(focused));
hostService.onDidChangeActiveWindow(() => {
    // activeContainerId 갱신 + layout event 발행
    onActiveWindowChanged();
});
```

---

## 5. 교차검증표 (Claude ↔ Codex)

| 클레임 | Claude | Codex | 적대 검증 결과 | 확신도 |
|---|---|---|---|---|
| Tauri: 창 간 JS 컨텍스트 공유 불가 | ✅ | ✅ | Discussion #11643 직접 확인 | 확실 |
| Tauri 권장 = Rust authority + emit/listen | ✅ | ✅ | 공식 문서 확인 | 확실 |
| BroadcastChannel: Rust 백엔드 인식 불가 한계 | ✅ | ✅ | state-sync 비교표 확인 | 확실 |
| plugin-store ≠ 실시간 동기화 버스 | ✅ | ✅ | 공식 문서 "preferences용" 확인 | 확실 |
| VS Code = 창별 독립 레이아웃 + 서비스만 공유 | ✅ | ✅ | layout.ts 패턴 확인 | 확실 |
| Tauri unlisten 자동 제거 미구현 버그 | ✅ (Issue #15583) | ⚠️ 미언급 | Issue 직접 확인 (2026-06-25) | 확실 |
| revision gate가 순서 역전 문제 해결 | ⚠️ 소개 수준 | ✅ 상세 | state-sync 라이브러리 확인 | 가능성 높음 |
| onCloseRequested가 API 종료 시 미트리거 케이스 | ✅ (Issue #5288) | ⚠️ 미언급 | Issue 확인 | 가능성 높음 |

---

## 6. 라이브러리 옵션 비교 (Tauri 생태계)

| 라이브러리 | 순서 보장 | Rust 인식 | 지속성 | 적합 케이스 |
|---|---|---|---|---|
| **state-sync** | ✅ (revision) | ✅ | 별도 패키지 | 순서 중요, 고빈도 업데이트 |
| **@tauri-store/zustand** | ⚠️ 최신값 우선 | ✅ | 내장 | 단순 Tauri + Zustand |
| **tauri-plugin-store** | ❌ | ✅ | 파일 기반 | 재시작 복원, 설정 영속 |
| **zustand-sync-tabs** (BroadcastChannel) | ❌ | ❌ | localStorage | Rust 무관 순수 UI 상태 |
| **직접 emit/listen** | 앱 구현 의존 | ✅ | 없음 | 제어가 필요한 커스텀 |

---

## 7. 공백 및 한계

- **Wezterm 방식의 Tauri 포팅:** Wezterm의 Mux-authority 패턴을 Tauri Rust 상태로 포팅하는 구체적 예제는 공개 자료 없음. 개념적 유사성은 확인, 구현 패턴은 직접 설계 필요. [불확실]
- **Tauri Issue #15583:** 2026-06-25 등록 미해결. 향후 패치되면 명시적 unlisten 부담이 줄어들 수 있음.
- **고빈도 레이아웃 변경 성능:** resize/drag 이벤트를 emit/listen으로 처리할 때 throttle 없으면 IPC 폭주 가능. 50ms debounce 권장 (state-sync 기본값 참조). [가능성 높음]

---

## 출처

- Tauri v2 공식 - Calling Frontend: https://v2.tauri.app/develop/calling-frontend/
- Tauri v2 공식 - State Management: https://v2.tauri.app/develop/state-management/
- Tauri v2 공식 - Event API: https://v2.tauri.app/reference/javascript/api/namespaceevent/
- Tauri v2 공식 - Window API: https://v2.tauri.app/reference/javascript/api/namespacewindow/
- Tauri Issue #15583 (unlisten 자동 제거 버그): https://github.com/tauri-apps/tauri/issues/15583
- Tauri Issue #5288 (onCloseRequested API 미트리거): https://github.com/tauri-apps/tauri/issues/5288
- Tauri Discussion #11643 (서브 창 JS 컨텍스트): https://github.com/orgs/tauri-apps/discussions/11643
- state-sync 라이브러리 비교: https://777genius.github.io/state-sync/comparison
- Tauri Zustand 동기화 (gethopp.app): https://www.gethopp.app/blog/tauri-window-state-sync
- state-sync Tauri Medium 가이드: https://777genius.medium.com/how-to-sync-state-across-tauri-windows-with-any-state-manager-redux-zustand-jotai-mobx-9797a364b22b
- VS Code IPC 분석 (roopik.com): https://roopik.com/blog/vscode-internals-advanced-ipc
- VS Code 1.86 릴리즈 노트: https://code.visualstudio.com/updates/v1_86
- VS Code 샌드박싱 블로그: https://code.visualstudio.com/blogs/2022/11/28/vscode-sandbox
- Wezterm multiplexer 아키텍처 (deepwiki): https://deepwiki.com/wezterm/wezterm/2.2-multiplexer-architecture
- Electron IPC 공식: https://www.electronjs.org/docs/latest/tutorial/ipc
- electron-redux npm: https://www.npmjs.com/package/electron-redux
- reduxtron GitHub: https://github.com/vitordino/reduxtron
- Codex 독립 조사 결과 (VS Code sharedProcess.ts 코드 패턴 포함): 지식 기반 + 공개 소스 교차
