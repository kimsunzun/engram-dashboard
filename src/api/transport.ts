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
// ProtocolClient 는 connectionState 가 connected 로 (재)전이하면 모든 뷰를 buffering 리셋 + requestReplay
// 한다(ADR-0046 — 옛 resubscribeAll wire 재발행 삭제) — carrier 별 재연결 메커니즘은 transport 내부에 숨고,
// ProtocolClient 는 "연결됨" 신호만 본다.

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
  // ★replay 경계 마커(ADR-0046)★: src-tauri 가 각 replay 종결마다 같은 출력 Channel 로 흘리는 tag=255
  //   프레임을 transport 가 정규화한 제어 이벤트. 공개 agentClient 표면엔 노출하지 않는다(Designer 리뷰
  //   요구 — 마커는 프론트 내부 상태기계 전용). ProtocolClient 가 gen 펜스로 buffering→live 전이를
  //   판정한다. gen 은 u64(BigInt) — frame 이 8바이트 BE 로 싣고 requestReplay 반환값과 같은 폭이라야
  //   비교가 정확하다(§F2 결정). failed=true 면 이 replay 가 완결 없이 종결됨(deadline/단절).
  | {
      kind: 'replayBoundary'
      agentId: string
      epoch: number
      gen: bigint
      truncated: boolean
      failed: boolean
    }

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
   * ★뷰 주도 replay 요청(ADR-0046 F2)★. 뷰(slot)가 (re)mount·재연결·watchdog·실패 사다리에서
   * ProtocolClient 를 통해 부른다 — src-tauri 에 그 agent 의 **데몬 ring 전량 재replay**(wire
   * `Subscribe{after_seq:None}`)를 single-flight 로 요청하고, 부여된 **gen(u64)** 을 회수한다.
   *
   * ## gen 표현 = BigInt (설계 결정, §F2)
   * gen 은 src-tauri per-agent 단조 카운터(u64)다. 마커 frame 이 8바이트 BE 로 gen 을 싣고
   * ProtocolClient 가 `요청 gen ≤ 마커 gen` 펜스로 비교하므로, 두 값의 폭이 정확히 일치해야 한다 →
   * **BigInt 로 통일**. (실용 수명에선 Number 로도 2^53 미만이라 무해하나, frame 이 u64 를 싣는 이상
   * carrier 디코드(getBigUint64)와 invoke 반환을 BigInt 로 맞춰 정밀도 소실 가능성 자체를 제거한다.
   * invoke 는 u64 를 JSON number 로 직렬화하므로 TauriTransport 가 반환값을 BigInt 로 변환한다 —
   * 2^53 미만이면 무손실, 그 이상은 이론상 소실이나 replay 카운터가 그 값에 도달할 수 없어 무해.)
   *
   * ## carrier 별
   *  - TauriTransport: `invoke('request_replay', { agentId }) -> gen(u64)`. src-tauri single-flight 가
   *    즉시 Subscribe 를 보내거나 다음 Sub 에 병합하고 gen 을 반환한다. 종결 마커(성공/실패)는 출력
   *    Channel 로 tag=255 프레임이 별도 도착 → transport 가 replayBoundary 로 정규화.
   *  - WsTransport(legacy/직결): 자체 wire `Subscribe{after_seq:null}` + per-agent gen 카운터. 아래
   *    구현 주석의 legacy 근사 참조.
   *
   * ★계약(FIX-6 정정)★: 모든 requestReplay 는 **최소 1개의 replayBoundary 이벤트**로 종결되거나(성공/
   * 실패), 연결이 끊긴다(그때는 마커 미발행 — connected 재전이가 재구동). "정확히 1개"가 아니다: 좀비
   * 의미론에서 실패 마커(deadline) 뒤에 같은 gen 의 성공 마커(늦은 Complete)가 뒤따를 수 있다 —
   * failed→성공 쌍은 정상 경로다. gen 펜스가 이를 흡수한다(뷰는 자기 myGen 성공 마커에 flush, 실패
   * 마커는 사다리로 넘겼다가 뒤이은 성공 마커에 복구). ProtocolClient 상태기계 전제.
   */
  requestReplay(agentId: string): Promise<bigint>
}
