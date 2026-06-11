# 모듈 8d — Phase 3d: Popup 분리 + monaco 브리핑 (담당: dcs24, Sonnet)

발신: ed12 (매니저)
근거: `docs/frontend-integration-lld.md` §6(popup), tracking T-5.
목적: 슬롯을 독립 창으로 분리. Phase 3 마지막. + monaco optimizeDeps 경고 해소.

## 1. Popup agentId 전달 (3c 권고2 갭 해소)

문제: 새 WebviewWindow는 zustand 미공유 → PopupPage가 slotStore로 agentId 조회하면 항상 null(빈 터미널).
해결: **URL 쿼리로 전달**.
- SlotContextMenu '팝업으로 분리': `WebviewWindow` open URL에 `?agentId=<id>` 추가.
- PopupPage: `new URLSearchParams(location.search).get('agentId')` 우선 사용(slotStore 의존 제거).

## 2. PopupPage.tsx (§6 패턴 그대로)

```ts
useEffect(() => {
  const agentId = new URLSearchParams(location.search).get('agentId')
  if (!agentId) return

  // G-2: 팝업도 agent-status-changed 독립 listen (zustand 미공유)
  const unlistenStatus = listen('agent-status-changed', (e) => {
    if (e.payload.id === agentId) setAgentStatus(e.payload.status)
  })

  // G-1: useEffect 직후 fit()은 DOM 미완성 → ResizeObserver 첫 콜백까지 대기
  const observer = new ResizeObserver(() => {
    observer.disconnect()                 // 첫 콜백만(초기 크기 확정)
    fitAddon.fit()
    if (terminal.cols > 0) {
      // ★resize 먼저 → subscribe 순서★ (cols/rows 불일치 방지)
      ptyApi.resizePty(agentId, terminal.cols, terminal.rows)
        .then(() => ptyApi.subscribeOutput(agentId, onChunk))
        .then(({ sinkId }) => { /* cleanup용 sinkId 보관 */ })
    }
  })
  if (containerRef.current) observer.observe(containerRef.current)

  return () => {
    observer.disconnect()
    unlistenStatus.then(fn => fn())
    // subscribe 했으면 unsubscribe + delete onmessage (3c와 동일 cleanup)
  }
}, [])
```
- TerminalSlot의 subscribe/입력/Exited 가드 로직을 popup에서도 재사용(공통 훅으로 빼면 좋으나 범위 크면 복제 허용).
- 입력가드(§4-1)·T-2 dedup·G-1 cleanup 동일 적용.

## 3. monaco optimizeDeps (T-5)

vite.config.ts: monaco TS worker 경고 해소. `optimizeDeps.exclude`에 monaco editor worker 관련 추가, 또는 `@monaco-editor/react` 권장 설정 적용. DiffPanel 실제 동작(diff 표시) 확인.

## 규칙·품질
- popup의 PTY 연결은 메인 TerminalSlot과 동일 가드(C2/T-2/G-1/입력가드).
- resize→subscribe 순서 필수.
- 주석: URL 전달 이유(zustand 미공유), resize 선행 이유.

## 검증 & 보고 (E2E)
`npm run tauri dev`: 슬롯 우클릭 → 팝업 분리 → **독립 창에 같은 PTY 출력**(replay+live), 입력/리사이즈 동작, 메인 창과 동시 표시(둘 다 수신). monaco diff 경고 사라짐.
보고: `orch 12 "⟁dcs24 Phase3d 완료 — popup URL전달+resize선행+monaco, E2E: <관찰>"`

이걸로 Phase 3 종료. 막히면 30분 내 중간보고.
