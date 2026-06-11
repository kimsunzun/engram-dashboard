# 모듈 8a — Phase 3a: API 레이어 브리핑 (담당: dcs24, Sonnet)

발신: ed12 (매니저)
근거: `docs/frontend-integration-lld.md` §1(타입), §2(ptyApi). 반드시 원문 읽고 따른다.
목적: 프론트가 백엔드 command/event를 호출하는 타입·래퍼 레이어. **화면 영향 없음**(다음 단계에서 소비).

## 선행: 기존 코드 파악

`src/store/agentStore.ts` 읽어서 현재 더미 Agent 타입 확인. 백엔드 AgentInfo와 조율(통합 또는 매핑) — 충돌 시 백엔드 타입 기준, 더미 전용 필드(cost 등 UI용)는 별도 유지.

## 1. src/api/types.ts (Rust 타입 미러 — 백엔드와 정확히 일치)

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

## 2. src/api/ptyApi.ts (invoke 래퍼 + Channel)

`@tauri-apps/api/core`의 invoke + Channel 사용. 8개 command 1:1 래핑:

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

> **인자명 주의:** Tauri는 command 인자를 camelCase로 받음(`agentId`→Rust `agent_id` 자동 매핑). LLD §2 확인.
> **T-7:** getSnapshot wire 포맷(PtyChunk number[])은 live PtyEvent base64와 다름. 3c에서 snapshot 쓸지 결정 — 일단 래퍼만 두고 미사용.

## 규칙·품질

- 백엔드 타입과 1:1 정확히. AgentStatus internally-tagged 유지(프론트가 `status.type`으로 분기).
- 주석: 각 타입/함수에 1줄. wire 포맷 주의점(base64, number[]) 명시.
- 기존 코드 안 깨뜨림 — types.ts/ptyApi.ts 신규 추가만, 아직 컴포넌트 연결 안 함.

## 검증 & 보고

- `npx tsc --noEmit` (또는 `npm run build`의 타입체크) 통과 — 타입 에러 0.
- 보고: `orch 12 "⟁dcs24 Phase3a 완료 — types.ts/ptyApi.ts, tsc 통과"`

막히면 30분 내 중간보고. agentStore 더미 타입과 조율 애매하면 질문.
