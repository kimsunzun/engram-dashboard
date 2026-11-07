# Module 8c — Phase 3c: connect TerminalSlot to the real PTY briefing (owner: dcs24, Sonnet)

From: ed12 (manager)
Source: `docs/frontend-integration-lld.md` §4 (TerminalSlot), §4-1 (error/Exited guard), §8.
Purpose: **the first E2E.** Dummy write → real PTY subscribe. spawn→output→input→kill works in a real window.
Prerequisite: read the existing src/components/slot/TerminalSlot.tsx, SlotContextMenu.tsx, slotStore/agentStore and connect to them.

## 1. decodeBase64Bytes helper (src/api/ or util)
```ts
export function decodeBase64Bytes(b64: string): Uint8Array {
  // feature-detect Uint8Array.fromBase64 (신규), 없으면 atob fallback
  const f = (Uint8Array as any).fromBase64
  if (f) return f(b64)
  const bin = atob(b64); const arr = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i)
  return arr
}
```

## 2. TerminalSlot.tsx — remove the dummy, connect for real (§4 pattern as written)

**subscribe (the core — comply with all of C2/T-2/G-1):**
```ts
useEffect(() => {
  if (!agentId) return
  let sinkId = null, channel = null, cancelled = false
  terminal.reset()                          // C2: 구독 전 초기화(중복 방지)
  const lastSeqRef = { current: -1 }        // T-2/G-2: seq dedup
  ptyApi.subscribeOutput(agentId, (event) => {
    if (cancelled) return
    if (event.seq <= lastSeqRef.current) return   // 중복 drop
    lastSeqRef.current = event.seq
    terminal.write(decodeBase64Bytes(event.data_b64))
  }).then(result => {
    if (cancelled) { ptyApi.unsubscribeOutput(agentId, result.sinkId); return }
    sinkId = result.sinkId; channel = result.channel
  })
  return () => {
    cancelled = true
    if (channel) delete (channel as any).onmessage   // G-1: delete(null 아님), #13133
    if (sinkId) ptyApi.unsubscribeOutput(agentId, sinkId)
  }
}, [agentId])
```

**Key input → stdin:**
```ts
useEffect(() => {
  if (!agentId || !terminal) return
  const disp = terminal.onData(data => ptyApi.writeStdin(agentId, new TextEncoder().encode(data)))
  return () => disp.dispose()
}, [agentId, terminal])
```

**resize → resizePty:** add to the existing FitAddon flow — after fit, `ptyApi.resizePty(agentId, terminal.cols, terminal.rows)`. (If a ResizeObserver/allotment resize handler already exists, add the call there)

**Exited guard (M4):** if the agent's status.type in agentStore is Exited/Failed/Killed, disable input + show an overlay (terminated). Per T-4, status is for display.

## 3. SlotContextMenu.tsx — spawn/kill triggers (§8)
- Besides the existing splitSlot, add **spawnAgent**(cwd) + **killAgent**.
- spawnAgent: `ptyApi.spawnAgent(cwd)` → assign the returned AgentInfo.id to that slot (slotStore). For cwd, use a fixed default for now (e.g. the user's home or an input prompt — keep it simple since this is a verification stage).
- killAgent: `ptyApi.killAgent` with the current slot's agentId.

## 4. AgentTree.tsx — dummy → real (§8)
- Remove `dummyAgents`/`dummyGroups` → use `agents` (AgentInfo[]) from `useAgentStore`.
- Status color: branch on `agent.status.type` ('Running'|'Exiting'|'Exited'|'Failed'|'Killed').
- Tree click → assign the agentId to the focused slot (keep the existing slotStore flow).

## 5. (Prerequisite) 3b minor fix — initEventBus in-flight guard
Prevents the StrictMode double-mount race. At module level:
```ts
let initPromise: Promise<void> | null = null
export function initEventBus() {
  if (initPromise) return initPromise   // in-flight면 같은 promise 반환
  initPromise = (async () => { /* 기존 로직 */ })()
  return initPromise
}
// HMR dispose에서 initPromise = null 도 리셋
```

## Rules and quality
- C2 (reset) / T-2 (dedup) / G-1 (delete onmessage) / explicit unsubscribe are all required — omitting any causes memory leaks and duplicate output.
- Safe under StrictMode's double effect (cancelled flag).
- Comments: the reason for each guard (why reset, why dedup, why delete).

## Verification and report (E2E — the real screen)
Bring up `npm run tauri dev` and:
1. spawnAgent from the context menu → PTY shell output appears in the slot
2. Keyboard input → reflected in the shell (echo)
3. Window/slot resize → shell cols/rows follow (no breakage)
4. killAgent → terminated overlay, removed from the list (T-4)

Report: `orch 12 "⟁dcs24 Phase3c 완료 — TerminalSlot subscribe/입력/resize + spawn/kill, E2E 화면동작: <관찰>"`
※ This is a stage where the user checks the screen directly. Attaching a screenshot/log of the behavior is good. If stuck, send an interim report within 30 minutes.
