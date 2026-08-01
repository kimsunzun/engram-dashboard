// Transport — carrier(전송) 추상 (ADR-0020 결정3).
//
// ProtocolClient 가 carrier 디테일(소켓/Channel/바이트)을 모르게 하는 seam: carrier 는 수신 프레임을
// 정규화된 InboundMessage 로 풀어 올리고(onMessage), 명령은 AgentCommand wire 객체로 받는다(send).
// 연결 상태도 carrier 소유 — 비-connected → connected 전이마다 ProtocolClient 가 모든 뷰를 buffering
// 리셋 + requestReplay 하므로(ADR-0046), connected 를 남발하면 전량 재replay 가 돈다.

import type { ConnectionState } from './agentClient'

/**
 * carrier 가 ProtocolClient 로 올리는 **정규화된 수신 메시지**. carrier 별 인코딩(WS binary frame /
 * TauriOutbound)은 transport 가 이미 풀었다. control 의 event = externally-tagged AgentEvent JSON.
 *
 * Auth/Hello 는 transport 내부(handshake)에서 소비되고 여기로 올라오지 않는다.
 */
export type InboundMessage =
  | { kind: 'control'; event: Record<string, unknown> }
  // tag = frame 종류(0=터미널 바이트 / 1=StructuredEvent JSON) — 상수 정본은 wsFrame.ts.
  | { kind: 'output'; tag: number; agentId: string; epoch: number; seq: number; bytes: Uint8Array }
  // ★replay 경계 마커(ADR-0046)★: src-tauri 가 각 replay 종결마다 같은 출력 Channel 로 흘리는 tag=255
  //   프레임의 정규화. 공개 agentClient 표면엔 노출하지 않는다 — 마커는 프론트 내부 상태기계 전용
  //   (Designer 리뷰 요구). failed=true 면 이 replay 가 완결 없이 종결됨(deadline/단절).
  | {
      kind: 'replayBoundary'
      agentId: string
      epoch: number
      gen: bigint
      truncated: boolean
      failed: boolean
    }

/** carrier 추상 — ProtocolClient 가 의존하는 유일한 전송 표면(daemon 접속 전용, ADR-0029). */
export interface Transport {
  readonly connectionState: ConnectionState

  /** 등록 즉시 현재 상태를 1회 통지한 뒤 변화 시 호출. 반환은 해제 함수. */
  onConnectionStateChange(cb: (state: ConnectionState) => void): () => void

  /** 콜백은 1개만 보관된다 — 재등록은 앞의 것을 교체(ProtocolClient 가 유일 라우터). 반환은 해제 함수. */
  onMessage(cb: (msg: InboundMessage) => void): () => void

  /** 명령 전송(AgentCommand wire 객체). 호출 전 연결 보장은 호출자 몫 — ProtocolClient 는 ensureReady() 를 await 한다. */
  send(payload: unknown): void | Promise<void>

  /**
   * 전송 준비 보장 = **attach-only**(ADR-0021 불변식). 명령/구독 경로가 매 호출 전에 await 한다 — 이
   * 경로는 **절대 데몬을 spawn 하지 않는다**(데몬 끈 뒤 키 한 번·리사이즈가 respawn 하면 안 된다).
   * 명시 close 뒤엔 즉시 reject — 복구는 start() 뿐.
   */
  ensureReady(): Promise<void>

  /**
   * **명시 spawn 진입점**(ADR-0021 §1) — 데몬을 띄울 수 있는 유일한 경로다(부팅 연결 / 사용자
   * daemon_start). 명령 경로(ensureReady)와 분리해 "명령의 부수효과로 respawn" 을 차단한다.
   */
  start(): Promise<void>

  /** 명시 종료(재연결 중단 + carrier 정리). 이후 connectionState='down'. */
  close(): void

  /**
   * 뷰 주도 replay 요청(ADR-0046 F2) — 그 agent 의 **데몬 ring 전량 재replay**(증분 아님)를 요청하고
   * 부여된 gen 을 회수한다. gen 은 u64 단조 카운터 — 마커 frame 이 8바이트 BE 로 싣고 ProtocolClient 가
   * `요청 gen ≤ 마커 gen` 펜스로 비교하므로 폭을 맞춰 BigInt 로 통일한다.
   *
   * ★계약(FIX-6 정정)★: 모든 requestReplay 는 **최소 1개의 replayBoundary 이벤트**로 종결되거나 연결이
   * 끊긴다(끊기면 마커 미발행 — connected 재전이가 재구동). "정확히 1개"가 아니다: 실패 마커(deadline)
   * 뒤에 같은 gen 의 늦은 성공 마커가 오는 failed→성공 쌍은 정상 경로다.
   */
  requestReplay(agentId: string): Promise<bigint>
}
