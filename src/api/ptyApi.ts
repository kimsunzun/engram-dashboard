// PTY 백엔드 invoke 래퍼 — frontend-integration-lld.md §2
// Tauri command 인자는 camelCase → Rust snake_case 자동 매핑.

import { Channel, invoke } from '@tauri-apps/api/core'

import type { AgentInfo, PtyEvent, SinkId } from './types'

export const ptyApi = {
  /** PTY 에이전트 spawn. cwd는 작업 디렉토리 절대 경로. */
  spawnAgent: (cwd: string) => invoke<AgentInfo>('spawn_agent', { cwd }),

  /** 에이전트 종료 요청 — Running→Exiting→Killed 전이. */
  killAgent: (agentId: string) => invoke<void>('kill_agent', { agentId }),

  /** 현재 에이전트 목록 전체 조회. */
  getAgents: () => invoke<AgentInfo[]>('get_agents'),

  /**
   * PTY 출력 구독 — Channel 생성 후 command 호출, {channel, sinkId} 반환.
   * unmount 시 반드시 unsubscribeOutput + channel.onmessage 정리 필요 (T-GitHub#13133).
   */
  subscribeOutput: async (agentId: string, onChunk: (e: PtyEvent) => void) => {
    const channel = new Channel<PtyEvent>()
    channel.onmessage = onChunk
    const sinkId = await invoke<SinkId>('subscribe_agent_output', { agentId, channel })
    return { channel, sinkId }
  },

  /** PTY 출력 구독 해제 — effect cleanup에서 sinkId로 호출. */
  unsubscribeOutput: (agentId: string, sinkId: SinkId) =>
    invoke<void>('unsubscribe_agent_output', { agentId, sinkId }),

  /**
   * PTY stdin write.
   * Uint8Array → Array.from(number[]) 변환 필수 (Tauri Vec<u8> ← JSON number[]).
   */
  writeStdin: (agentId: string, data: Uint8Array) =>
    invoke<void>('write_stdin', { agentId, data: Array.from(data) }),

  /** PTY 창 크기 변경 — fitAddon.fit() 후 cols/rows 전달. */
  resizePty: (agentId: string, cols: number, rows: number) =>
    invoke<void>('resize_pty', { agentId, cols, rows }),

  /**
   * replay buffer 스냅샷 조회 (T-7: wire 포맷은 PtyEvent base64와 다른 PtyChunk number[]).
   * Phase 3c에서 사용 여부 결정 — 현재 래퍼만 제공.
   */
  getSnapshot: (agentId: string) =>
    invoke<unknown[]>('get_agent_snapshot', { agentId }),
}
