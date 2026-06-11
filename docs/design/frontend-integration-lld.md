# Engram Dashboard — 프론트엔드 통합 설계 LLD

**작성:** ed12, 2026-06-11  
**기반:** `backend-lld-stage1.md` 확정본  
**목적:** React/TypeScript 층이 Rust 백엔드와 어떻게 연결되는지 확정

---

## 0. 비목표

**주의 (GPT G-3):** Tauri v2 capabilities 설정 필요.  
팝업 창(`slot-popup`, `agent-tree`)도 `tauri.conf.json` capability에 포함해야 invoke 가능.  
`src-tauri/capabilities/default.json` → windows 배열에 모든 window label 추가.



- Rust 백엔드 코드 변경 — Stage 1에서 확정됨
- xterm.js 렌더링 최적화 (VT 파싱 등)
- DiffPanel 실제 연동 — 추후 단계

---

## 1. TypeScript 타입 미러 (`src/types/pty.ts`)

Rust LLD §3 타입을 TypeScript로 미러. Tauri invoke 반환값과 Channel 페이로드 타입.

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

## 2. Tauri invoke 래퍼 (`src/lib/ptyApi.ts`)

컴포넌트가 invoke를 직접 호출하지 않도록 래퍼 레이어.  
테스트/목업 시 이 레이어만 교체.

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

## 3. agentStore.ts 변경 계획

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

## 4. TerminalSlot.tsx — subscribe/unsubscribe 패턴

### 핵심 요구사항
- React StrictMode: dev에서 effect 2회 실행 → cleanup 반드시 구현
- unmount 시 명시적 unsubscribe (Channel GC에 의존 금지)
- channel.onmessage 명시적 정리 (Tauri Channel memory leak 방지, GitHub #13133)

### 구현 패턴

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

### 키 입력 → PTY stdin

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

## 4-1. 에러 처리 + Exited 상태 입력 가드 (M4)

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

## 5. resize 동기화 흐름

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

**M1 멀티 창 resize 정책: 마지막 포커스 창 우선 (option b)**
- 포커스된 창만 `resizePty` 호출 권한 → 핑퐁 차단
- 비포커스 창은 PTY cols/rows와 어긋날 수 있음 (허용)
- 포커스 전환 시 즉시 resize 한 번 실행

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

**주의:**
- hidden 상태(display:none)에서 `fit()` 호출 시 cols=0 → guard 추가
- 팝업 창 open 후 첫 render에서 resize 한 번 강제 호출 (초기 크기 동기화)

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

## 6. 팝업 창 구독 패턴

팝업 창은 메인과 독립된 Tauri WebviewWindow — Zustand 공유 불가.  
각 창이 독립적으로 `subscribe_agent_output` 호출.

```
메인 창: subscribe(agentA) → sinkId_main
팝업 창: subscribe(agentA) → sinkId_popup
→ Rust PtySession.subscribers = [sink_main, sink_popup]
→ drain thread → 둘 다 Channel.send
```

**팝업 창 열릴 때 순서:**
1. URL param `?agentId=xxx`로 열림
2. `useEffect` → `ptyApi.subscribeOutput` → replay 자동 수신
3. replay 완료 후 live stream 이어짐
4. **resize 먼저**: subscribe 전에 `resizePty(agentId, cols, rows)` 호출 (cols/rows 불일치 방지)

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

## 7. Tauri Event 수신 (저빈도)

`lib.ts` 또는 App.tsx에서 앱 초기화 시 1회 등록. 절대 컴포넌트 안에서 등록 금지(중복 누적).

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

## 8. 더미 → 실제 교체 체크리스트

| 파일 | 더미 | 교체 |
|---|---|---|
| `agentStore.ts` | `dummyAgents` 하드코딩 | `ptyApi.getAgents()` + `agent-list-updated` event |
| `TerminalSlot.tsx` | `terminal.write("dummy text")` | `subscribeOutput` Channel |
| `AgentTree.tsx` | 더미 status 색상 | `AgentStatus` 타입 기반 |
| `SlotContextMenu.tsx` | `splitSlot` only | + `spawnAgent`, `killAgent` |
| App.tsx / main.ts | — | `initEventBus()` 추가 |

---

## 9. 동시성 + 메모리 안전 규칙

| 규칙 | 이유 |
|---|---|
| `channel.onmessage = null` on cleanup | Tauri Channel memory leak (GitHub #13133) |
| `cancelled` flag in subscribeOutput effect | StrictMode 2회 실행 방지 |
| Event listener는 App level 1회만 | 컴포넌트 remount 시 중복 방지 |
| resize는 visible 확인 후 | hidden 시 cols=0 방지 |
| subscribe 전 resize | 팝업 cols/rows 불일치 방지 |

---

## 10. 검토 요청 질문

1. `Channel<PtyEvent>`를 `invoke`에 파라미터로 전달하는 패턴 — Tauri v2.4 TypeScript SDK에서 올바른 방법인가? `new Channel<T>()` 직접 생성 후 `channel.onmessage` 할당하는 패턴이 맞는가?
2. `atob(data_b64) → Uint8Array` 변환이 브라우저 환경(WebView2)에서 안전한가? 더 나은 디코딩 방법?
3. React StrictMode double-effect에서 `cancelled` flag 패턴 — 경쟁 조건이 있는가?
4. Tauri Event listener를 App level에서 1회 등록하는 패턴 — HMR/dev reload 시 중복 등록 위험이 있는가? 해결책?
5. 팝업 창에서 `resize → subscribe` 순서 — Rust 쪽에서 보면 resize 전에 subscribe가 먼저 도착할 수 있는가? (async 순서 보장 여부)
6. 전체 구조에서 놓친 것, 프론트엔드 통합 시 알려진 Tauri v2.4 pitfall?
