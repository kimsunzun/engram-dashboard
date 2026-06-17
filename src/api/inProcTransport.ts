// InProcTransport — Transport 의 in-process(embedded) carrier 구현 (ADR-0020 결정3, Stage 3).
//
// 로컬도 원격(WS)과 같은 ProtocolClient + AgentCommand 프로토콜을 타게 한다. carrier 만 다르다:
//  - 연결: 부팅 1회 agent_connect(channel) 로 단일 Channel 등록(WS 의 "1 연결" 대응).
//  - 송신: send = invoke('agent_command', {cmd})  (WS 의 ws.send 대응, generic 1개로 합침).
//  - 수신: Channel 의 TauriOutbound 를 InboundMessage 로 정규화(WS 의 binary frame/JSON 대응).
//  - 상태: 항상 'connected'(프로세스 수명=연결 수명). 재연결/dedup no-op 자연수렴(우회 분기 없음).
//
// carrier wire 계약(embedded_carrier.rs TauriOutbound, serde tag="kind"):
//   { kind:"event",  event:  AgentEvent }   — control(JSON externally-tagged)
//   { kind:"output", output: PtyEvent }     — 출력(base64 PtyEvent, 기존 embedded 인코딩 유지)

import { Channel, invoke } from '@tauri-apps/api/core'

import type { ConnectionState } from './agentClient'
import { decodeBase64Bytes } from './decodeBase64'
import type { InboundMessage, Transport } from './transport'
import type { PtyEvent } from './types'

/** embedded_carrier.rs TauriOutbound 미러 — serde tag="kind" discriminated union. */
type TauriOutbound =
  | { kind: 'event'; event: Record<string, unknown> }
  | { kind: 'output'; output: PtyEvent }

export class InProcTransport implements Transport {
  // 프로세스 수명=연결 수명 → 항상 connected. 재연결 개념 없음.
  private readonly _state: ConnectionState = 'connected'

  // ProtocolClient 가 등록하는 단일 수신 콜백.
  private messageCb: ((msg: InboundMessage) => void) | null = null

  // agent_connect 로 등록한 단일 Channel(부팅 1회). 중복 등록 방지(idempotent connect).
  private channel: Channel<TauriOutbound> | null = null
  private connectPromise: Promise<void> | null = null
  private closed = false

  get connectionState(): ConnectionState {
    return this._state
  }

  onConnectionStateChange(cb: (state: ConnectionState) => void): () => void {
    // 즉시 connected 1회 통지 후 변화 없음(WS 와 달리 reconnecting/down 미발생) — 해제는 no-op.
    cb(this._state)
    return () => {}
  }

  onMessage(cb: (msg: InboundMessage) => void): () => void {
    this.messageCb = cb
    return () => {
      if (this.messageCb === cb) this.messageCb = null
    }
  }

  /**
   * 전송 준비 보장 = 단일 Channel 등록(부팅 1회). WS 의 ensureConnected 대응이나 재연결이 없어
   * 1회만 의미 있다(이후 즉시 resolve). 모든 명령/구독 전에 ProtocolClient 가 await 한다 →
   * 첫 명령 전에 Channel 이 등록돼 control/output 수신 경로가 선다(부팅 타이밍 보장).
   */
  ensureReady(): Promise<void> {
    if (this.channel) return Promise.resolve()
    if (this.connectPromise) return this.connectPromise
    this.connectPromise = this.connect()
    return this.connectPromise
  }

  // 명시 spawn 진입점(Transport.start). InProc 은 spawn 개념이 없어 ensureReady 와 동일(Channel
  // 등록). DaemonControl.start 가 부르나 embedded 는 EmbeddedDaemonControl(no-op)이라 실제론
  // 부팅 ensure 가 이미 Channel 을 등록한다. 인터페이스 충족 + 명시 호출 시 idempotent connect.
  start(): Promise<void> {
    return this.ensureReady()
  }

  private async connect(): Promise<void> {
    const channel = new Channel<TauriOutbound>()
    channel.onmessage = (out: TauriOutbound) => this.onOutbound(out)
    // agent_connect 등록 — 직후 백엔드가 Hello + 현재 목록을 이 Channel 로 push(초기 동기화).
    await invoke<void>('agent_connect', { channel })
    // ★nit★: invoke await 중 close() 가 불렸으면 channel 을 되살리지 않는다(leak 방지). close 는
    //   this.channel/messageCb 를 정리하므로, 여기서 다시 setting 하면 닫힌 transport 가 살아난다.
    if (this.closed) {
      delete (channel as { onmessage?: unknown }).onmessage
      return
    }
    this.channel = channel
  }

  /** TauriOutbound → InboundMessage 정규화. control 은 event 그대로, output 은 base64 디코드. */
  private onOutbound(out: TauriOutbound): void {
    if (out.kind === 'output') {
      const e = out.output
      // ★epoch★(BLOCKER 1): carrier 가 PtyEvent.epoch 에 세션 epoch 을 실어 보낸다(WS binary frame
      //   헤더와 동형). 이걸 0 고정으로 두면 SubscribeAck.current_epoch≥1(resume-fallback)과 불일치해
      //   ProtocolClient epoch 가드(f.epoch !== st.epoch)가 출력을 전멸시킨다. e.epoch 을 그대로 쓴다.
      this.messageCb?.({
        kind: 'output',
        agentId: e.agent_id,
        epoch: e.epoch,
        seq: e.seq,
        bytes: decodeBase64Bytes(e.data_b64),
      })
      return
    }
    // control AgentEvent(JSON externally-tagged) — 그대로 위로.
    this.messageCb?.({ kind: 'control', event: out.event })
  }

  /** 명령 전송 = generic agent_command invoke. invoke 의 reject 는 ProtocolClient sendCommand 가 처리. */
  send(payload: unknown): Promise<void> {
    if (this.closed) return Promise.reject(new Error('client closed'))
    return invoke<void>('agent_command', { cmd: payload })
  }

  close(): void {
    this.closed = true
    const ch = this.channel
    if (ch) {
      // #13133: null 대입 아닌 delete.
      delete (ch as { onmessage?: unknown }).onmessage
      this.channel = null
    }
    this.connectPromise = null
    this.messageCb = null
  }
}
