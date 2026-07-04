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
// 연결 상태도 carrier 소유: WS 는 reconnecting/down 을 발생시킨다.
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
 *
 * ★carrier 별 출처(동형)★:
 *  - WsTransport: control=WS Text frame, output=WS binary frame(decodeOutputFrame).
 *  - TauriTransport: control=Tauri listen(broadcast) + forward_daemon_command 반환(reply),
 *    output=per-window Tauri Channel(raw bytes → decodeOutputFrame). 둘 다 같은 InboundMessage 로
 *    정규화돼 ProtocolClient 는 carrier 출처를 모른다(reply/output 처리 로직 1벌).
 */
export type InboundMessage =
  | { kind: 'control'; event: Record<string, unknown> }
  // tag = frame 종류(0=터미널 바이트 / 1=StructuredEvent JSON, wsFrame.ts). ProtocolClient 가 tag 로
  // 소비 경로를 가른다(tag0→바이트 chunk, tag1→구조화 이벤트). epoch/seq 가드·dedup 은 tag 무관 공통.
  | { kind: 'output'; tag: number; agentId: string; epoch: number; seq: number; bytes: Uint8Array }

/**
 * carrier 추상. ProtocolClient 가 의존하는 유일한 전송 표면.
 *
 * 구현(daemon-only, ADR-0029):
 *  - WsTransport: WebSocket + discover/Auth/Hello + 지수백오프 재연결. binary frame/JSON 정규화.
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
   */
  ensureReady(): Promise<void>

  /**
   * **명시 spawn 진입점**(ADR-0021 §1: ensure(spawn)=명시 시점만). 부팅 연결/사용자 daemon_start 가
   * 이걸 통한다 — 여기서만 데몬을 띄울 수 있다(tmux `attach` 가 서버를 띄우는 것과 동치).
   *  - WS: discover_daemon(없으면 spawn) → 캐시 갱신 → Auth/Hello. closedByUser/reconnect 상태 리셋.
   * 명령 경로(ensureReady)와 분리해 "명령의 부수효과로 respawn" 을 차단한다.
   */
  start(): Promise<void>

  /** 명시 종료(재연결 중단 + carrier 정리). 이후 connectionState='down'. */
  close(): void

  /**
   * ★slot (re)mount 시 fresh replay 재요청(remount 대화 소실 FIX)★. RichSlot/TerminalSlot 이
   * (re)mount 하면 ProtocolClient.subscribeOutput 이 이걸 부른다 — 그 창이 보는 그 agent 의 slot 에
   * cursor 리셋 + 버퍼 전체 재전송(fresh replay)을 트리거한다.
   *
   * ## 왜 필요한가(근본원인)
   * idle tag1 slot 을 split/재배정하면 Allotment 재귀 트리 구조 변경으로 컴포넌트가 remount 되는데,
   * remount 는 창 출력 Channel 재등록이 *아니라서*(`subscribe_output` 은 창 mount 시 1회) backend 가
   * replay 를 재전송하지 않는다 → 대화 소실 + 영구 streaming 고착. reload 는 Channel 재등록으로 fresh
   * replay 가 흘러 복원되므로, 이 훅이 그 reload 복원 경로를 (re)mount 시점에 slot 단위로 재사용한다.
   *
   * ## carrier 별
   *  - TauriTransport: `invoke('resync_output', { agentId })` — src-tauri 로컬 축 B replay(BLOCK-1:
   *    데몬 wire Subscribe 를 새로 만들지 않는다). fire-and-forget(반환 무시).
   *  - WsTransport / mock: no-op(src-tauri 로컬 replay 경로가 없음 — 운영 carrier 는 Tauri 고정).
   *
   * fire-and-forget(반환 없음) — 정상 mount 에서 배정 트리거 replay 와 중복될 수 있으나 ProtocolClient
   * seq dedup(lastDeliveredSeq)이 화면 중복을 흡수한다(ADR-0037).
   */
  resyncOutput(agentId: string): void
}
