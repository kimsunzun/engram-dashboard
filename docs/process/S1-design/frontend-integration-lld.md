# Engram Dashboard — Frontend Integration Design LLD

**Author:** ed12, 2026-06-11  
**Basis:** finalized `backend-lld-stage1.md`  
**Purpose:** Finalize how the React/TypeScript layer connects to the Rust backend

---

## 0. Non-goals

**Note (GPT G-3):** Tauri v2 capabilities configuration is required.  
Popup windows (`slot-popup`, `agent-tree`) must also be included in the `tauri.conf.json` capability for invoke to work.  
`src-tauri/capabilities/default.json` → add every window label to the windows array.



- Rust backend code changes — finalized in Stage 1
- xterm.js rendering optimization (VT parsing etc.)
- Actual DiffPanel integration — later stage

---

## 1. TypeScript type mirror (`src/types/pty.ts`)

Mirror the Rust LLD §3 types in TypeScript. Types for Tauri invoke return values and Channel payloads.

```ts
// Rust AgentStatus enum 미러
// C1: 백엔드에 #[serde(tag="type")] 적용 → internally-tagged wire 형태
// wire: { "type": "Running" } / { "type": "Exited", "code": 0 } 등
// Starting variant 없음 (백엔드 §9 기준 — §3 잔재 제거 완료)
export type AgentStatus =
  | { type: 'Running' }
  | { type: 'Exiting' }
  | { type: 'Exited';  code: number | null }
  | { type: 'Failed';  message: string }
  | { type: 'Killed' }

export interface AgentInfo {
  id:     string    // UUID
  cwd:    string
  status: AgentStatus
  cols:   number
  rows:   number
}

// Channel<PtyEvent> 페이로드
export interface PtyEvent {
  agent_id: string
  seq:      number
  data_b64: string   // base64 encoded bytes → atob() → Uint8Array → xterm.write()
}

// subscribe_agent_output 반환값
export type SinkId = string  // UUID
```

---

## 2. Tauri invoke wrapper (`src/lib/ptyApi.ts`)

A wrapper layer so components do not call invoke directly.  
For tests/mocks, only this layer is swapped.

```ts
import { invoke } from '@tauri-apps/api/core'
import { Channel }  from '@tauri-apps/api/core'
import type { AgentInfo, PtyEvent, SinkId } from '../types/pty'

export const ptyApi = {
  spawnAgent:   (cwd: string) =>
    invoke<AgentInfo>('spawn_agent', { cwd }),

  killAgent:    (agentId: string) =>
    invoke<void>('kill_agent', { agentId }),

  getAgents:    () =>
    invoke<AgentInfo[]>('get_agents'),

  // subscribe: Channel 객체를 Rust에 전달 → SinkId 반환
  subscribeOutput: (agentId: string, onChunk: (e: PtyEvent) => void) => {
    const channel = new Channel<PtyEvent>()
    channel.onmessage = onChunk
    return invoke<SinkId>('subscribe_agent_output', { agentId, channel })
      .then(sinkId => ({ sinkId, channel }))
  },

  unsubscribeOutput: (agentId: string, sinkId: SinkId) =>
    invoke<void>('unsubscribe_agent_output', { agentId, sinkId }),

  writeStdin:   (agentId: string, data: Uint8Array) =>
    invoke<void>('write_stdin', { agentId, data: Array.from(data) }),

  resizePty:    (agentId: string, cols: number, rows: number) =>
    invoke<void>('resize_pty', { agentId, cols, rows }),

  getSnapshot:  (agentId: string) =>
    invoke<{ seq: number; data_b64: string }[]>('get_agent_snapshot', { agentId }),
}
```

---

## 3. agentStore.ts change plan

```ts
// 더미 제거 → Tauri invoke로 교체
interface AgentState {
  agents:          AgentInfo[]
  fetchAgents:     () => Promise<void>
  spawnAgent:      (cwd: string) => Promise<void>
  killAgent:       (agentId: string) => Promise<void>
  onStatusChanged: (id: string, status: AgentStatus) => void  // Event 수신 시 호출
}

// Tauri Event 구독 (저빈도 상태 알림)
// lib.ts 초기화 시 1회 등록:
listen<{ id: string; status: AgentStatus }>('agent-status-changed', (e) => {
  useAgentStore.getState().onStatusChanged(e.payload.id, e.payload.status)
})
listen<AgentInfo[]>('agent-list-updated', (e) => {
  useAgentStore.setState({ agents: e.payload })
})
```

---

## 4. TerminalSlot.tsx — subscribe/unsubscribe pattern

### Core requirements
- React StrictMode: effects run twice in dev → cleanup must be implemented
- Explicit unsubscribe on unmount (do not rely on Channel GC)
- Explicitly clear channel.onmessage (prevents the Tauri Channel memory leak, GitHub #13133)

### Implementation pattern

```ts
useEffect(() => {
  if (!agentId) return

  let sinkId: SinkId | null = null
  let channel: Channel<PtyEvent> | null = null
  let cancelled = false

  // C2: 구독 전 terminal 초기화 필수
  // - agent 전환: 이전 출력 위에 새 replay 중복 방지
  // - remount: 기존 화면 + replay 2배 중복 방지
  terminal.reset()

  // 1. subscribe
  // G-2: seq 중복 dedup (replay/live 중복, StrictMode remount 방지)
  const lastSeqRef = { current: -1 }

  ptyApi.subscribeOutput(agentId, (event) => {
    if (cancelled) return
    if (event.seq <= lastSeqRef.current) return  // 중복 drop
    lastSeqRef.current = event.seq
    // base64 → Uint8Array: feature-detect fromBase64, 없으면 atob fallback
    const bytes = decodeBase64Bytes(event.data_b64)
    terminal.write(bytes)
  }).then(result => {
    if (cancelled) {
      // StrictMode 2번째 실행에서 이미 cleanup됨 → 즉시 해제
      ptyApi.unsubscribeOutput(agentId, result.sinkId)
      return
    }
    sinkId = result.sinkId
    channel = result.channel
  })

  // 2. cleanup
  return () => {
    cancelled = true
    if (channel) {
      // GPT G-1: delete가 null 할당보다 안전 (GitHub #13133 workaround)
    delete (channel as any).onmessage
    }
    if (sinkId) {
      ptyApi.unsubscribeOutput(agentId, sinkId)
    }
  }
}, [agentId])
```

### Key input → PTY stdin

```ts
useEffect(() => {
  if (!agentId || !terminal) return
  const disp = terminal.onData((data: string) => {
    const bytes = new TextEncoder().encode(data)
    ptyApi.writeStdin(agentId, bytes)
  })
  return () => disp.dispose()
}, [agentId, terminal])
```

---

## 4-1. Error handling + input guard in the Exited state (M4)

```ts
// ptyApi 공통 에러 처리
const safeInvoke = async <T>(fn: () => Promise<T>): Promise<T | null> => {
  try { return await fn() }
  catch (e: any) {
    if (e?.includes?.('NotFound')) return null  // agent 없음 — 무시
    console.warn('[ptyApi]', e)
    return null
  }
}

// status 감시 → terminal 상태 UX
useEffect(() => {
  const agent = agents.find(a => a.id === agentId)
  if (!agent) return
  const isTerminal = ['Exited', 'Failed', 'Killed'].includes(agent.status.type)
  setIsTerminated(isTerminated)  // overlay 표시용
}, [agentId, agents])

// 입력 가드: 종료된 agent에 키입력 차단
const disp = terminal.onData((data: string) => {
  if (isTerminated) return  // Exited/Failed/Killed → 입력 무시
  safeInvoke(() => ptyApi.writeStdin(agentId, new TextEncoder().encode(data)))
})
```

## 5. resize synchronization flow

```ts
useEffect(() => {
  if (!agentId || !containerRef.current) return

  const observer = new ResizeObserver(
    debounce(() => {
      fitAddon.fit()
      ptyApi.resizePty(agentId, terminal.cols, terminal.rows)
    }, 50)
  )
  observer.observe(containerRef.current)
  return () => {
    observer.disconnect()
    debouncedFit.cancel()  // Minor: pending debounce 취소 (unmount 후 stale resize 방지)
  }
}, [agentId])
```

**M1 multi-window resize policy: last focused window wins (option b)**
- Only the focused window has permission to call `resizePty` → blocks ping-pong
- Non-focused windows may diverge from the PTY cols/rows (allowed)
- Run one immediate resize on focus switch

```ts
// 포커스 가드
const handleFit = () => {
  if (!document.hasFocus()) return  // 비포커스 창 skip
  if (!containerRef.current?.offsetParent) return  // hidden skip
  fitAddon.fit()
  if (terminal.cols > 0 && agentId) {
    ptyApi.resizePty(agentId, terminal.cols, terminal.rows)
  }
}
window.addEventListener('focus', handleFit)  // 창 포커스 시 즉시 sync
```

**Notes:**
- Calling `fit()` while hidden (display:none) gives cols=0 → add a guard
- On the first render after a popup window opens, force one resize call (initial size synchronization)

```ts
// hidden guard
const handleFit = () => {
  if (!containerRef.current?.offsetParent) return  // hidden이면 skip
  fitAddon.fit()
  if (terminal.cols > 0 && agentId) {
    ptyApi.resizePty(agentId, terminal.cols, terminal.rows)
  }
}
```

---

## 6. Popup window subscription pattern

A popup window is a Tauri WebviewWindow independent of the main window — Zustand cannot be shared.  
Each window calls `subscribe_agent_output` independently.

```
메인 창: subscribe(agentA) → sinkId_main
팝업 창: subscribe(agentA) → sinkId_popup
→ Rust PtySession.subscribers = [sink_main, sink_popup]
→ drain thread → 둘 다 Channel.send
```

**Order when a popup window opens:**
1. Opened with the URL param `?agentId=xxx`
2. `useEffect` → `ptyApi.subscribeOutput` → replay is received automatically
3. After replay completes, the live stream continues
4. **resize first**: call `resizePty(agentId, cols, rows)` before subscribe (prevents a cols/rows mismatch)

```ts
// PopupPage.tsx
useEffect(() => {
  const agentId = new URLSearchParams(location.search).get('agentId')
  if (!agentId) return

  // Gemini G-2: 팝업 창도 agent-status-changed 구독 필요
  // (Zustand 공유 안 됨 → 독립 리스너로 Exited/Killed 감지)
  const unlistenStatus = listen<{ id: string; status: AgentStatus }>(
    'agent-status-changed',
    (e) => { if (e.payload.id === agentId) setAgentStatus(e.payload.status) }
  )

  // Gemini G-1: useEffect 직후 fit()은 DOM 미완성 → ResizeObserver 첫 콜백까지 대기
  const observer = new ResizeObserver(() => {
    observer.disconnect()  // 첫 번째 호출만 (초기 크기 확정 시점)
    fitAddon.fit()
    if (terminal.cols > 0) {
      // 1. resize 먼저 (resize → subscribe 순서 보장)
      ptyApi.resizePty(agentId, terminal.cols, terminal.rows)
        .then(() => ptyApi.subscribeOutput(agentId, onChunk))
        .then(({ sinkId }) => { /* cleanup 등록 */ })
    }
  })
  if (containerRef.current) observer.observe(containerRef.current)

  return () => {
    observer.disconnect()
    unlistenStatus.then(fn => fn())
  }
}, [])
```

---

## 7. Receiving Tauri Events (low frequency)

Register once at app initialization in `lib.ts` or App.tsx. Never register inside a component (duplicates accumulate).

```ts
// src/lib/eventBus.ts — 앱 시작 시 1회 호출
import { listen } from '@tauri-apps/api/event'

// M5: UnlistenFn 보관 — HMR 재평가 시 이전 리스너 해제
let unlistenFns: (() => void)[] = []

export async function initEventBus() {
  // HMR 재평가 시 기존 리스너 먼저 해제 후 재등록 (Gemini 방식 — idempotent보다 안전)
  if (unlistenFns.length > 0) {
    unlistenFns.forEach(fn => fn())
    unlistenFns = []
  }

  // agent-status-changed payload: { id: string, status: AgentStatus }
  // (백엔드 §9 StatusSink → commands emit 확정 형식 — Minor 8 반영)
  unlistenFns.push(
    await listen<{ id: string; status: AgentStatus }>('agent-status-changed', (e) => {
      useAgentStore.getState().onStatusChanged(e.payload.id, e.payload.status)
    })
  )
  unlistenFns.push(
    await listen<AgentInfo[]>('agent-list-updated', (e) => {
      useAgentStore.setState({ agents: e.payload })
    })
  )

  // M5: Vite HMR 모듈 교체 시 리스너 해제
  if (import.meta.hot) {
    import.meta.hot.dispose(() => {
      unlistenFns.forEach(fn => fn())
      unlistenFns = []
    })
  }
}
```

---

## 8. Dummy → real replacement checklist

| File | Dummy | Replacement |
|---|---|---|
| `agentStore.ts` | hard-coded `dummyAgents` | `ptyApi.getAgents()` + `agent-list-updated` event |
| `TerminalSlot.tsx` | `terminal.write("dummy text")` | `subscribeOutput` Channel |
| `AgentTree.tsx` | dummy status colors | based on the `AgentStatus` type |
| `SlotContextMenu.tsx` | `splitSlot` only | + `spawnAgent`, `killAgent` |
| App.tsx / main.ts | — | add `initEventBus()` |

---

## 9. Concurrency + memory safety rules

| Rule | Reason |
|---|---|
| `channel.onmessage = null` on cleanup | Tauri Channel memory leak (GitHub #13133) |
| `cancelled` flag in subscribeOutput effect | Handles StrictMode running twice |
| Event listeners only once at App level | Prevents duplicates on component remount |
| resize only after confirming visibility | Prevents cols=0 when hidden |
| resize before subscribe | Prevents popup cols/rows mismatch |

---

## 10. Review request questions

1. The pattern of passing `Channel<PtyEvent>` to `invoke` as a parameter — is it the correct way in the Tauri v2.4 TypeScript SDK? Is creating `new Channel<T>()` directly and then assigning `channel.onmessage` the right pattern?
2. Is the `atob(data_b64) → Uint8Array` conversion safe in the browser environment (WebView2)? Is there a better decoding method?
3. In the React StrictMode double-effect, is there a race condition in the `cancelled` flag pattern?
4. For the pattern of registering a Tauri Event listener once at the App level — is there a risk of duplicate registration on HMR/dev reload? What is the fix?
5. For the `resize → subscribe` order in a popup window — from the Rust side, can subscribe arrive before resize? (Is async ordering guaranteed?)
6. What was missed in the overall structure, and what are the known Tauri v2.4 pitfalls for frontend integration?

---

## 11. S9 revision (session restore) — frontend delta

Basis: `../S9-session-restore/session-restore-lld.md` (§6) + `session-restore-code-plan.md` (§E·§H). Frontend items (e)~(g) of the 7 fable findings.

### (e) §4 subscribe effect — `[agentId, epoch]` resubscribe
- A restart is handled by **frontend resubscribe, not subscriber succession (migration)** (minimal change to the two finalized designs, visible failure modes). The only backend change is "replace the map entry with a new PtySession under the same AgentId".
- Take the `epoch: u32` added to `AgentInfo`/`agent-status-changed` and change the subscribe effect dependencies to `[agentId, epoch]`. Since a status change alone does not re-run the effect, **an epoch increment is the deterministic trigger for resubscribing**. Multi-window is uniform too: each window receives the event → each resubscribes.

```ts
// 변경 전: }, [agentId])
// 변경 후:
}, [agentId, epoch])   // epoch 증가 = 재spawn → 재구독
```

### (f) §4 restart banner order — reset → banner → replay
- Order on detecting a restart: `terminal.reset()` → show the restart banner → replay plays (via resubscribe). It takes the already-verified idempotent path as is (C2 reset + seq dedup) — the same code as switching agents.

### (g) §1/§4 status display — Exited→Running re-entry allowed
- The frontend state model must allow re-entry from terminal (`Exited`/`Killed`/`Failed`) → `Running` (the same AgentId comes alive again). The input guard (§4-1) works off the current status, so it is released automatically on re-entry. Optionally show `Restarting`.

### TS type mirror additions (§1)
- Add `epoch: number` to `AgentInfo`/`AgentStatusChanged`.
- `agent-restore-result` event payload: `{ agent_id: string, epoch: number, outcome: RestoreOutcome }` (discriminated union, tag `type`: `Resumed`/`FreshFallback`/`Blocked`/`Failed`).
