# Module 8b — Phase 3b: eventBus + agentStore wiring briefing (owner: dcs24, Sonnet)

From: ed12 (manager)
Source: `docs/frontend-integration-lld.md` §7 (eventBus), §8 (replacement checklist), tracking T-4.
Purpose: receive the backend events (agent-status-changed/agent-list-updated) and reflect them in agentStore. Dummy agents → real ones.

## 1. src/store/eventBus.ts (LLD §7 as written)

```ts
import { listen } from '@tauri-apps/api/event'
import { useAgentStore } from './agentStore'
import type { AgentStatus, AgentInfo } from '../api/types'

let unlistenFns: (() => void)[] = []

export async function initEventBus() {
  // HMR 재평가 시 기존 리스너 먼저 해제 후 재등록
  if (unlistenFns.length > 0) { unlistenFns.forEach(fn => fn()); unlistenFns = [] }

  unlistenFns.push(
    await listen<{ id: string; status: AgentStatus }>('agent-status-changed', (e) => {
      useAgentStore.getState().onStatusChanged(e.payload.id, e.payload.status)
    })
  )
  unlistenFns.push(
    await listen<AgentInfo[]>('agent-list-updated', (e) => {
      useAgentStore.getState().setAgents(e.payload)
    })
  )

  if (import.meta.hot) {
    import.meta.hot.dispose(() => { unlistenFns.forEach(fn => fn()); unlistenFns = [] })
  }
}
```

## 2. agentStore.ts — dummy → real (§8)

Remove the hardcoded `dummyAgents`. Add:
- `setAgents(agents: AgentInfo[])` — replaces the whole list on agent-list-updated. **This is the authoritative list (the basis for deciding existence/removal).**
- `onStatusChanged(id, status)` — updates only that agent's status (ignored if absent). **For display.**
- Initial load: at app start `ptyApi.getAgents()` → setAgents.

> **★T-4 (important)★:** decide terminal (exit) only from **agent-list-updated (disappearing from the list)**.
> The Killed/Exited received through `onStatusChanged` are **only for the badge display** — do not remove the agent from the list because of them.
> (kill's Exiting notification and drain's terminal notification occur at the same time outside the lock → they may be received in reverse order. The list is corrected by the manager's agent_list_updated.)
> The existing UI dummy fields (cost etc.) are not in AgentInfo, so manage them separately in the store or handle them as optional at the UI level.

## 3. App.tsx / main.tsx

- Call `initEventBus()` once at app mount + do the initial `getAgents()` load.
- Mind the StrictMode double mount — initEventBus releases existing listeners internally, so it is safe, but consider the useEffect cleanup.

## Rules and quality

- eventBus runs once app-wide. Location is src/store/eventBus.ts.
- A T-4 comment is required (why we decide by the list).
- If existing components (AgentTree etc.) subscribe to the store, make sure they do not break — AgentInfo.status is a {type} object, so if AgentTree's status color logic assumes a string it needs adjusting (a minimal fix within 3b scope, or a note for 3c).

## Verification and report

- `npx tsc --noEmit` passes. Bring up `npm run tauri dev` and check it mounts without console errors (there are no agents yet, so an empty list is normal).
- Report: `orch 12 "⟁dcs24 Phase3b 완료 — eventBus+agentStore 실제연결, T-4 반영, tsc/마운트 OK"`

If stuck, send an interim report within 30 minutes. If the conflict with the agentStore dummy structure is large, ask.
