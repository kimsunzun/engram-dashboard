# Module 8d — Phase 3d: Popup split + monaco briefing (owner: dcs24, Sonnet)

From: ed12 (manager)
Source: `docs/frontend-integration-lld.md` §6 (popup), tracking T-5.
Purpose: split a slot into an independent window. The last part of Phase 3. + resolve the monaco optimizeDeps warning.

## 1. Pass the Popup agentId (closes the 3c recommendation-2 gap)

Problem: a new WebviewWindow does not share zustand → if PopupPage looks up agentId through slotStore, it is always null (an empty terminal).
Fix: **pass it through the URL query**.
- SlotContextMenu '팝업으로 분리' (split out as popup): add `?agentId=<id>` to the `WebviewWindow` open URL.
- PopupPage: use `new URLSearchParams(location.search).get('agentId')` first (drop the slotStore dependency).

## 2. PopupPage.tsx (§6 pattern as written)

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
- Reuse TerminalSlot's subscribe/input/Exited guard logic in the popup as well (extracting it into a shared hook is good, but copying is allowed if the scope is large).
- Apply the input guard (§4-1) · T-2 dedup · G-1 cleanup identically.

## 3. monaco optimizeDeps (T-5)

vite.config.ts: resolve the monaco TS worker warning. Add the monaco editor worker related entries to `optimizeDeps.exclude`, or apply the setup recommended by `@monaco-editor/react`. Confirm that DiffPanel actually works (shows a diff).

## Rules and quality
- The popup's PTY connection has the same guards as the main TerminalSlot (C2/T-2/G-1/input guard).
- The resize→subscribe order is required.
- Comments: the reason for passing through the URL (zustand not shared), the reason resize goes first.

## Verification and report (E2E)
`npm run tauri dev`: right-click a slot → split out as popup → **the same PTY output in an independent window** (replay+live), input/resize work, shown at the same time as the main window (both receive). The monaco diff warning is gone.
Report: `orch 12 "⟁dcs24 Phase3d 완료 — popup URL전달+resize선행+monaco, E2E: <관찰>"`

This ends Phase 3. If stuck, send an interim report within 30 minutes.
