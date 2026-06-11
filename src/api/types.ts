// Rust 백엔드 타입 미러 — LLD §3 / frontend-integration-lld.md §1
// 백엔드 #[serde(tag="type")]와 정확히 일치하는 discriminated union.

/** 에이전트 생명주기 상태 — `status.type`으로 분기 */
export type AgentStatus =
  | { type: 'Running' }
  | { type: 'Exiting' }
  | { type: 'Exited'; code: number | null }
  | { type: 'Failed'; message: string }
  | { type: 'Killed' }

/** PTY 출력 Channel 페이로드 — data_b64는 base64 인코딩된 raw bytes */
export interface PtyEvent {
  agent_id: string
  seq: number
  data_b64: string
}

/** subscribe_agent_output 반환값 — unsubscribe 시 사용 */
export type SinkId = string

/** 에이전트 메타데이터 스냅샷 */
export interface AgentInfo {
  id: string
  cwd: string
  status: AgentStatus
  cols: number
  rows: number
}

/** agent-status-changed Tauri event 페이로드 */
export interface AgentStatusChanged {
  id: string
  status: AgentStatus
}
