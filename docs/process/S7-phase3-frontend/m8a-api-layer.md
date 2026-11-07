# Module 8a — Phase 3a: API layer briefing (owner: dcs24, Sonnet)

From: ed12 (manager)
Source: `docs/frontend-integration-lld.md` §1 (types), §2 (ptyApi). Read the original and follow it.
Purpose: the type and wrapper layer through which the frontend calls backend commands/events. **No screen impact** (consumed in the next step).

## Prerequisite: understand the existing code

Read `src/store/agentStore.ts` and check the current dummy Agent type. Reconcile it with the backend AgentInfo (merge or map) — on conflict the backend type is the reference, and dummy-only fields (UI ones such as cost) stay separate.

## 1. src/api/types.ts (a mirror of the Rust types — must match the backend exactly)

```ts
// AgentStatus: 백엔드 #[serde(tag="type")]와 일치 (internally-tagged)
export type AgentStatus =
  | { type: 'Running' }
  | { type: 'Exiting' }
  | { type: 'Exited'; code: number | null }
  | { type: 'Failed'; message: string }
  | { type: 'Killed' }

export interface PtyEvent { agent_id: string; seq: number; data_b64: string }
export type SinkId = string
export interface AgentInfo { id: string; cwd: string; status: AgentStatus; cols: number; rows: number }

// event payload (lib.rs emit과 일치)
export interface AgentStatusChanged { id: string; status: AgentStatus }
```

## 2. src/api/ptyApi.ts (invoke wrapper + Channel)

Use invoke + Channel from `@tauri-apps/api/core`. Wrap the 8 commands 1:1:

```ts
import { invoke, Channel } from '@tauri-apps/api/core'
import type { AgentInfo, PtyEvent, SinkId } from './types'

export const ptyApi = {
  spawnAgent: (cwd: string) => invoke<AgentInfo>('spawn_agent', { cwd }),
  killAgent:  (agentId: string) => invoke<void>('kill_agent', { agentId }),
  getAgents:  () => invoke<AgentInfo[]>('get_agents'),

  // subscribe: Channel 생성 → onChunk 연결 → command 호출 → {channel, sinkId} 반환
  subscribeOutput: async (agentId: string, onChunk: (e: PtyEvent) => void) => {
    const channel = new Channel<PtyEvent>()
    channel.onmessage = onChunk
    const sinkId = await invoke<SinkId>('subscribe_agent_output', { agentId, channel })
    return { channel, sinkId }
  },
  unsubscribeOutput: (agentId: string, sinkId: SinkId) =>
    invoke<void>('unsubscribe_agent_output', { agentId, sinkId }),

  writeStdin: (agentId: string, data: Uint8Array) =>
    invoke<void>('write_stdin', { agentId, data: Array.from(data) }),  // Vec<u8> ← number[]
  resizePty:  (agentId: string, cols: number, rows: number) =>
    invoke<void>('resize_pty', { agentId, cols, rows }),
  getSnapshot: (agentId: string) => invoke<unknown[]>('get_agent_snapshot', { agentId }),
}
```

> **Mind the argument names:** Tauri receives command arguments in camelCase (`agentId`→Rust `agent_id` is mapped automatically). Check LLD §2.
> **T-7:** the getSnapshot wire format (PtyChunk number[]) differs from the live PtyEvent base64. Whether to use the snapshot is decided in 3c — for now only the wrapper is placed, unused.

## Rules and quality

- Match the backend types 1:1 exactly. Keep AgentStatus internally-tagged (the frontend branches on `status.type`).
- Comments: one line on each type/function. State the wire-format cautions (base64, number[]).
- Do not break existing code — only add new types.ts/ptyApi.ts, no component connection yet.

## Verification and report

- `npx tsc --noEmit` (or the type check of `npm run build`) passes — zero type errors.
- Report: `orch 12 "⟁dcs24 Phase3a 완료 — types.ts/ptyApi.ts, tsc 통과"`

If stuck, send an interim report within 30 minutes. If the reconciliation with the agentStore dummy type is ambiguous, ask.
