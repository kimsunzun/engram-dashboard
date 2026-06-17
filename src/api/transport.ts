// Transport — carrier(전송) 추상 (ADR-0020 결정3, TRD Stage 3).
//
// ProtocolClient(프로토콜 의미론: request_id/dedup/epoch/resubscribe)가 carrier 디테일을
// 모르게 한다. carrier(WS binary frame / Tauri Channel TauriOutbound)는 수신 프레임을
// **정규화된 InboundMessage** 로 풀어 올리고(onMessage), 명령은 AgentCommand wire 객체를
// send 로 받는다. 즉 transport = "바이트/소켓/Channel 을 만지는 모든 것".
//
//   ProtocolClient → Transport.send(AgentCommand wire)        → carrier 직렬화/전송
//   carrier 수신   → Transport.onMessage(InboundMessage)      → ProtocolClient 라우팅
//
// 연결 상태도 carrier 소유: WS 는 reconnecting/down 발생, InProc 은 항상 connected(no-op).
// ProtocolClient 는 connectionState 가 connected 로 (재)전이하면 resubscribeAll 한다 —
// carrier 별 재연결 메커니즘은 transport 내부에 숨고, ProtocolClient 는 "연결됨" 신호만 본다.

import type { ConnectionState } from './agentClient'

/**
 * carrier 가 ProtocolClient 로 올리는 **정규화된 수신 메시지**. carrier 별 인코딩(WS binary
 * frame / TauriOutbound)은 transport 가 이미 풀었다 — ProtocolClient 는 이 두 형태만 다룬다.
 *
 *  - control: JSON AgentEvent(externally-tagged). Ack/Spawned/Created/Error/SubscribeAck/
 *    ReplayComplete/AgentList/AgentListUpdated/ProfileList/ProfileListUpdated/Snapshot/
 *    StatusChanged/RestoreResult 등. ProtocolClient 가 variant 로 분기.
 *  - output: 디코드된 출력 frame. epoch/seq 가드 + dedup 후 OutputChunk 로 구독자에 배달.
 *
 * Auth/Hello 는 transport 내부(handshake)에서 소비되고 여기로 올라오지 않는다.
 */
export type InboundMessage =
  | { kind: 'control'; event: Record<string, unknown> }
  | { kind: 'output'; agentId: string; epoch: number; seq: number; bytes: Uint8Array }

/**
 * carrier 추상. ProtocolClient 가 의존하는 유일한 전송 표면.
 *
 * 구현은 2개:
 *  - WsTransport: WebSocket + discover/Auth/Hello + 지수백오프 재연결. binary frame/JSON 정규화.
 *  - InProcTransport: agent_connect(Channel 등록) + invoke('agent_command'). 항상 connected.
 */
export interface Transport {
  /** 현재 연결 상태. ProtocolClient 의 connectionState 가 이걸 그대로 노출. */
  readonly connectionState: ConnectionState

  /** 상태 변화 구독. 등록 즉시 현재 상태 1회 통지 후 변화 시 호출. 반환은 해제 함수. */
  onConnectionStateChange(cb: (state: ConnectionState) => void): () => void

  /**
   * 수신 메시지 콜백 등록. transport 가 control(AgentEvent)/output(decoded)을 정규화해 호출한다.
   * ProtocolClient 가 한 번 등록한다(단일 라우터). 반환은 해제 함수.
   */
  onMessage(cb: (msg: InboundMessage) => void): () => void

  /**
   * 명령 전송(AgentCommand wire 객체). carrier 가 직렬화/전송한다. 연결 보장은 transport 책임 —
   * WS 는 미연결 시 throw 또는 연결 대기, InProc 은 즉시 invoke. async/sync 모두 허용(Promise|void).
   * ProtocolClient 는 보내기 전에 ensureReady() 로 연결을 보장한다.
   */
  send(payload: unknown): void | Promise<void>

  /**
   * 전송 준비 보장 = **attach-only**(ADR-0021 불변식). 명령/구독 경로(ProtocolClient)가 매 호출
   * 전에 await 한다 — 이 경로는 **절대 데몬을 spawn 하지 않는다**(reconnect=attach-only 와 동치).
   *  - WS: 캐시된 host:port 로 소켓만 재오픈(discover/spawn 금지). 캐시 없거나 down 이면 reject
   *    ("daemon down — daemon_start 로 명시 시작 필요"). → 데몬 끈 뒤 키 한 번/리사이즈가 respawn 못 함.
   *  - InProc: no-op resolve(프로세스 수명=연결 수명, spawn 개념 없음).
   */
  ensureReady(): Promise<void>

  /**
   * **명시 spawn 진입점**(ADR-0021 §1: ensure(spawn)=명시 시점만). 부팅 연결/사용자 daemon_start 가
   * 이걸 통한다 — 여기서만 데몬을 띄울 수 있다(tmux `attach` 가 서버를 띄우는 것과 동치).
   *  - WS: discover_daemon(없으면 spawn) → 캐시 갱신 → Auth/Hello. closedByUser/reconnect 상태 리셋.
   *  - InProc: ensureReady 와 동일(Channel 등록, no-op spawn).
   * 명령 경로(ensureReady)와 분리해 "명령의 부수효과로 respawn" 을 차단한다.
   */
  start(): Promise<void>

  /** 명시 종료(재연결 중단 + carrier 정리). 이후 connectionState='down'. */
  close(): void
}
