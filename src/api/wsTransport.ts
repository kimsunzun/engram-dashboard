// WsTransport — Transport 의 WS(데몬) carrier 구현 (ADR-0020 결정3, TRD Stage 3).
//
// DaemonClient(580줄)에서 WS-특정 부분을 추출: discover_daemon → WebSocket 생성 → Auth →
// Hello 대기 → 지수백오프 재연결 → ws.send / ws.onmessage. binary frame·JSON AgentEvent 를
// **InboundMessage 로 정규화**해 ProtocolClient 로 올린다(ProtocolClient 는 carrier 디코드를
// 모른다). Auth/Hello 는 여기서 소비(handshake) — control/output 만 위로 올린다.
//
// 프로토콜 의미론(request_id 매칭·dedup·epoch 가드·resubscribe)은 ProtocolClient 소유.
// 이 transport 는 carrier 만 책임진다: 연결·재연결·프레임 인코딩/디코딩.

import { invoke } from '@tauri-apps/api/core'

import type { ConnectionState } from './agentClient'
import type { InboundMessage, Transport } from './transport'
import { decodeOutputFrame } from './wsFrame'

// ── discover_daemon DTO(discovery.rs DaemonInfoDto 미러) ──────────────────────────
interface DaemonInfoDto {
  pid: number
  host: string
  port: number
  token: string
  protocol_version: number
}

type WireEvent = Record<string, unknown>

export class WsTransport implements Transport {
  private ws: WebSocket | null = null

  private _state: ConnectionState = 'down'
  private stateListeners = new Set<(s: ConnectionState) => void>()

  // 진행 중 연결 시도(중복 연결 방지). resolve 는 Hello 수신(=인증 성공) 시.
  private connectPromise: Promise<void> | null = null

  private closedByUser = false
  private reconnectAttempt = 0
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null

  // ProtocolClient 가 등록하는 단일 수신 콜백(control/output 정규화 메시지).
  private messageCb: ((msg: InboundMessage) => void) | null = null

  // ── 연결 상태 ──────────────────────────────────────────────────────────────────
  get connectionState(): ConnectionState {
    return this._state
  }

  onConnectionStateChange(cb: (state: ConnectionState) => void): () => void {
    this.stateListeners.add(cb)
    // 등록 즉시 현재 상태 1회 통지(ProtocolClient 가 초기 상태를 알게).
    cb(this._state)
    return () => {
      this.stateListeners.delete(cb)
    }
  }

  onMessage(cb: (msg: InboundMessage) => void): () => void {
    this.messageCb = cb
    return () => {
      if (this.messageCb === cb) this.messageCb = null
    }
  }

  private setState(s: ConnectionState): void {
    if (this._state === s) return
    this._state = s
    for (const cb of this.stateListeners) cb(s)
  }

  // ── 전송 준비 보장(lazy connect, 중복 방지) ───────────────────────────────────────
  ensureReady(): Promise<void> {
    if (this.ws && this.ws.readyState === WebSocket.OPEN && this._state === 'connected') {
      return Promise.resolve()
    }
    if (this.connectPromise) return this.connectPromise
    this.connectPromise = this.openSocket()
    return this.connectPromise
  }

  /** 1회 소켓 열기 + Auth 전송 + Hello 대기. 성공 시 resolve, 실패 시 reject(상위가 처리). */
  private openSocket(): Promise<void> {
    return new Promise<void>((resolve, reject) => {
      const run = async () => {
        // discover_daemon 은 매 연결마다 호출(데몬 재기동 시 port/token 바뀔 수 있음).
        const info = await invoke<DaemonInfoDto>('discover_daemon')
        // host 는 일단 127.0.0.1 고정 가정(로컬 IPC).
        const ws = new WebSocket('ws://127.0.0.1:' + info.port)
        ws.binaryType = 'arraybuffer'
        this.ws = ws

        let settled = false

        ws.onopen = () => {
          // 첫 frame = Auth(JSON text). protocol_version 은 discover 가 준 값을 echo.
          ws.send(
            JSON.stringify({
              Auth: { token: info.token, protocol_version: info.protocol_version },
            }),
          )
        }

        ws.onmessage = (event: MessageEvent) => {
          if (typeof event.data === 'string') {
            const msg = JSON.parse(event.data) as WireEvent
            // Hello 수신 = 인증 성공. connect 완료. (handshake 내부 소비 — 위로 안 올림.)
            if ('Hello' in msg && !settled) {
              settled = true
              this.reconnectAttempt = 0
              // connected 전이 → ProtocolClient 가 resubscribeAll(재연결 resume).
              this.setState('connected')
              resolve()
              return
            }
            // 인증 전 Error = Auth 실패 → reject(연결은 데몬이 닫는다).
            if ('Error' in msg && !settled) {
              settled = true
              const m = (msg.Error as { message?: string })?.message ?? 'auth failed'
              reject(new Error('daemon auth failed: ' + m))
              return
            }
            // control event — ProtocolClient 로 정규화 전달.
            this.messageCb?.({ kind: 'control', event: msg })
          } else if (event.data instanceof ArrayBuffer) {
            // binary output frame — 디코드해 정규화 output 으로 전달.
            const f = decodeOutputFrame(event.data)
            if (!f) return
            this.messageCb?.({
              kind: 'output',
              agentId: f.agentId,
              epoch: f.epoch,
              seq: f.seq,
              bytes: f.payload,
            })
          }
        }

        ws.onerror = () => {
          if (!settled) {
            settled = true
            reject(new Error('daemon websocket error'))
          }
        }

        ws.onclose = () => {
          this.handleClose(settled)
          if (!settled) {
            settled = true
            reject(new Error('daemon websocket closed before handshake'))
          }
        }
      }
      run().catch((e) => reject(e))
    })
  }

  /** 소켓 종료 처리 — 의도적 종료가 아니면 재연결 스케줄. pending reject 는 ProtocolClient 가
   * connectionState 전이(connected→reconnecting)로 처리한다(carrier 무관 위치로 승격). */
  private handleClose(wasHandshakeSettled: boolean): void {
    this.connectPromise = null
    this.ws = null

    if (this.closedByUser) {
      this.setState('down')
      return
    }
    // 핸드셰이크 중 끊김도 재연결 대상(데몬 재기동 대기). 지수 백오프.
    void wasHandshakeSettled
    this.setState('reconnecting')
    this.scheduleReconnect()
  }

  private scheduleReconnect(): void {
    if (this.reconnectTimer) return
    if (this.closedByUser) return
    // 500ms → 1s → 2s → … 최대 10s.
    const delay = Math.min(500 * 2 ** this.reconnectAttempt, 10000)
    this.reconnectAttempt += 1
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null
      if (this.closedByUser) return
      // ensureReady 가 새 connectPromise 를 만든다. 실패하면 다시 onclose→scheduleReconnect.
      this.ensureReady().catch(() => {
        // openSocket reject(예: discover 실패) — 소켓 onclose 가 안 왔을 수 있으니 직접 재스케줄.
        if (!this.reconnectTimer && !this.closedByUser) {
          this.setState('reconnecting')
          this.scheduleReconnect()
        }
      })
    }, delay)
  }

  // ── 명령 전송 ───────────────────────────────────────────────────────────────────
  send(payload: unknown): void {
    const ws = this.ws
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      throw new Error('daemon not connected')
    }
    ws.send(JSON.stringify(payload))
  }

  // ── 명시 종료(재연결 중단) ──────────────────────────────────────────────────────
  close(): void {
    this.closedByUser = true
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }
    this.cleanupSocket()
    this.setState('down')
  }

  /** #13133: 핸들러는 null 대입이 아니라 delete 로 정리한 뒤 close. */
  private cleanupSocket(): void {
    const ws = this.ws
    if (!ws) return
    delete (ws as { onmessage?: unknown }).onmessage
    delete (ws as { onopen?: unknown }).onopen
    delete (ws as { onerror?: unknown }).onerror
    delete (ws as { onclose?: unknown }).onclose
    try {
      ws.close()
    } catch {
      // 이미 닫힘 — 무시.
    }
    this.ws = null
    this.connectPromise = null
  }
}
