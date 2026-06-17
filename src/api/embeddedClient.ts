// EmbeddedClient — AgentClient 의 in-process 구현(현 invoke/Channel 경로).
// daemon-design §3-a: connectionState 항상 'connected', dedup no-op(Tauri Channel 은 순서 보존).
// transport 디테일(Channel·base64·sinkId·#13133 정리)을 전부 여기 캡슐화한다.

import { Channel, invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

import type { AgentClient, ConnectionState, OutputChunk, OutputSubscription } from './agentClient'
import { ptyApi } from './ptyApi'
import { decodeBase64Bytes } from './decodeBase64'
import type { AgentInfo, AgentProfile, AgentStatus, PtyEvent, RestoreReport, SinkId } from './types'

/**
 * Tauri listen(async Promise<unlisten>)을 sync disposer 로 감싼다(취소 안전).
 * listen 이 아직 resolve 안 됐을 때 disposer 가 먼저 불릴 수 있으므로 cancelled 플래그를
 * 둔다 — resolve 후 cancelled 면 즉시 unlisten 호출(리스너 누수 방지). connectionState 패턴.
 */
function syncListen<T>(event: string, handler: (payload: T) => void): () => void {
  let unlisten: UnlistenFn | null = null
  let cancelled = false
  void listen<T>(event, (e) => handler(e.payload)).then((fn) => {
    if (cancelled) fn()
    else unlisten = fn
  })
  return () => {
    cancelled = true
    if (unlisten) {
      unlisten()
      unlisten = null
    }
  }
}

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

  // ── 상태/목록/복원 이벤트 — Tauri listen 래핑(eventBus 가 하던 등록을 여기로 이동) ──
  onAgentListUpdated(cb: (agents: AgentInfo[]) => void): () => void {
    return syncListen<AgentInfo[]>('agent-list-updated', cb)
  }
  onStatusChanged(cb: (id: string, status: AgentStatus, epoch: number) => void): () => void {
    return syncListen<{ id: string; status: AgentStatus; epoch: number }>(
      'agent-status-changed',
      (p) => cb(p.id, p.status, p.epoch),
    )
  }
  onRestoreResult(cb: (report: RestoreReport) => void): () => void {
    return syncListen<RestoreReport>('agent-restore-result', cb)
  }
  onProfileListUpdated(cb: (profiles: AgentProfile[]) => void): () => void {
    // Stage 4 삭제 예정 클래스(테스트 호환 잔류). embedded 백엔드 broadcast 미도달 — no-op disposer.
    void cb
    return () => {}
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
