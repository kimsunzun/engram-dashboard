// EmbeddedClient — AgentClient 의 in-process 구현(현 invoke/Channel 경로).
// daemon-design §3-a: connectionState 항상 'connected', dedup no-op(Tauri Channel 은 순서 보존).
// transport 디테일(Channel·base64·sinkId·#13133 정리)을 전부 여기 캡슐화한다.

import { Channel, invoke } from '@tauri-apps/api/core'

import type { AgentClient, ConnectionState, OutputChunk, OutputSubscription } from './agentClient'
import { ptyApi } from './ptyApi'
import { decodeBase64Bytes } from './decodeBase64'
import type { AgentInfo, AgentProfile, PtyEvent, SinkId } from './types'

export class EmbeddedClient implements AgentClient {
  // Embedded 는 프로세스 수명=연결 수명이라 끊김 개념이 없다. 항상 connected.
  readonly connectionState: ConnectionState = 'connected'

  onConnectionStateChange(cb: (state: ConnectionState) => void): () => void {
    // 즉시 connected 1회 통지 후 변화 없음 — 해제 함수는 no-op.
    cb('connected')
    return () => {}
  }

  async subscribeOutput(
    agentId: string,
    onChunk: (chunk: OutputChunk) => void,
  ): Promise<OutputSubscription> {
    // Channel 직접 생성 — base64 디코드를 여기서 수행해 인터페이스엔 바이트만 노출.
    const channel = new Channel<PtyEvent>()
    channel.onmessage = (event: PtyEvent) => {
      onChunk({ seq: event.seq, bytes: decodeBase64Bytes(event.data_b64) })
    }
    const sinkId = await invoke<SinkId>('subscribe_agent_output', { agentId, channel })
    return {
      unsubscribe: () => {
        // G-1: null 할당 아닌 delete (#13133). 그 뒤 백엔드 구독 해제.
        delete (channel as { onmessage?: unknown }).onmessage
        void ptyApi.unsubscribeOutput(agentId, sinkId)
      },
    }
  }

  spawnAgent(cwd: string): Promise<AgentInfo> {
    return ptyApi.spawnAgent(cwd)
  }
  killAgent(agentId: string): Promise<void> {
    return ptyApi.killAgent(agentId)
  }
  interruptAgent(agentId: string): Promise<void> {
    return ptyApi.interruptAgent(agentId)
  }
  writeStdin(agentId: string, data: Uint8Array): Promise<void> {
    return ptyApi.writeStdin(agentId, data)
  }
  resizePty(agentId: string, cols: number, rows: number): Promise<void> {
    return ptyApi.resizePty(agentId, cols, rows)
  }
  getAgents(): Promise<AgentInfo[]> {
    return ptyApi.getAgents()
  }
  getSnapshot(agentId: string): Promise<unknown[]> {
    return ptyApi.getSnapshot(agentId)
  }

  listProfiles(): Promise<AgentProfile[]> {
    return ptyApi.listProfiles()
  }
  createClaudeProfile(
    name: string,
    cwd: string,
    extraArgs: string[],
    env: [string, string][],
    autoRestore: boolean,
  ): Promise<AgentProfile> {
    return ptyApi.createClaudeProfile(name, cwd, extraArgs, env, autoRestore)
  }
  deleteProfile(agentId: string): Promise<void> {
    return ptyApi.deleteProfile(agentId)
  }
  spawnProfile(agentId: string, resume: boolean): Promise<AgentInfo> {
    return ptyApi.spawnProfile(agentId, resume)
  }
  setProfileAutoRestore(agentId: string, autoRestore: boolean): Promise<void> {
    return ptyApi.setProfileAutoRestore(agentId, autoRestore)
  }
}
