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

  // ADR-0021: ensure(명시 spawn)와 reconnect(attach-only)를 분리하기 위한 캐시.
  // 최초 명시 연결에서 discover_daemon 으로 얻은 host/port/token 을 보관한다. 재연결 루프는
  // 이 캐시로 **소켓만 재오픈**하고 절대 discover/spawn 을 호출하지 않는다(불변식: reconnect 는
  // spawn 금지 — 데몬을 kill 하면 재연결이 못 붙어 'down' 유지, 사용자가 명시로 다시 시작).
  private cachedInfo: DaemonInfoDto | null = null

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

  // ── 전송 준비 보장 = attach-only(ADR-0021 B-1) ────────────────────────────────────
  // 명령/구독 경로(ProtocolClient.sendCommand/subscribeOutput/resizePty)가 매 호출 전에 부른다.
  // ★불변식★: 이 경로는 절대 spawn 하지 않는다 — openSocket(false)(캐시 재오픈)만. 그래야 사용자가
  // 데몬을 끈 뒤 키 한 번/창 리사이즈만 해도 데몬이 되살아나는 버그(B-1)가 안 난다. spawn 은 명시
  // start() 에서만. closedByUser/attempt 리셋도 여기서 하지 않는다(그건 명시 start 의 책임).
  ensureReady(): Promise<void> {
    if (this.ws && this.ws.readyState === WebSocket.OPEN && this._state === 'connected') {
      return Promise.resolve()
    }
    if (this.connectPromise) return this.connectPromise
    // 사용자가 명시 종료(close)했거나 재연결이 소진돼 down 인데도 명령이 들어오면 attach 시도조차
    // 하지 않고 즉시 reject — 명령이 데몬을 깨우면 안 된다(꺼진 채 유지). 복구는 명시 start 로만.
    if (this.closedByUser) {
      return Promise.reject(
        new Error('daemon down — daemon_start 로 명시 시작 필요 (ADR-0021: 명령은 respawn 안 함)'),
      )
    }
    if (!this.cachedInfo) {
      return Promise.reject(
        new Error('daemon down — daemon_start 로 명시 시작 필요 (no cached daemon, ADR-0021)'),
      )
    }
    // attach-only: 캐시 host:port 로 소켓만 재오픈. 데몬이 죽었으면 onclose → reject(respawn 안 함).
    this.connectPromise = this.openSocket(false)
    return this.connectPromise
  }

  // ── 명시 spawn 진입점(ADR-0021 §1) ───────────────────────────────────────────────
  // 부팅 연결 / 사용자 daemon_start(DaemonControl.start → client.connect → 여기) 만 호출한다.
  // discover_daemon 으로 데몬을 spawn(없으면) 하고 캐시를 채운다. 이전 close()/소진으로 멈춘
  // 재연결 상태를 리셋(closedByUser 해제 + attempt 0 + 진행 중 타이머 정리)해 다시 살아날 수 있게.
  start(): Promise<void> {
    if (this.ws && this.ws.readyState === WebSocket.OPEN && this._state === 'connected') {
      return Promise.resolve()
    }
    this.closedByUser = false
    this.reconnectAttempt = 0
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }
    // 진행 중 attach 시도(connectPromise)가 있어도 명시 start 는 discover 를 강제해야 한다 —
    // 새 openSocket(true) 로 교체(이전 attach promise 는 버려지되 소켓 onclose 가 정리).
    this.connectPromise = this.openSocket(true)
    return this.connectPromise
  }

  /**
   * 1회 소켓 열기 + Auth 전송 + Hello 대기. 성공 시 resolve, 실패 시 reject(상위가 처리).
   *
   * ADR-0021 ensure/reconnect 분리:
   *  - allowDiscover=true(명시 연결): discover_daemon 호출(없으면 spawn) → 결과를 cachedInfo 에 보관.
   *  - allowDiscover=false(재연결 루프): **discover/spawn 절대 금지.** 캐시된 host/port 로 소켓만
   *    재오픈한다. 캐시가 없으면(첫 연결도 안 한 상태) 즉시 reject — 재연결은 새 데몬을 만들지 않는다.
   */
  private openSocket(allowDiscover: boolean): Promise<void> {
    return new Promise<void>((resolve, reject) => {
      const run = async () => {
        let info: DaemonInfoDto
        if (allowDiscover) {
          // 명시 연결만 discover(없으면 데몬 spawn). 성공 시 캐시 갱신(데몬 재기동 시 port/token 반영).
          info = await invoke<DaemonInfoDto>('discover_daemon')
          this.cachedInfo = info
        } else {
          // 재연결 = attach-only. 캐시된 정보로만 재오픈. 캐시 없으면 붙을 곳을 모른다 → reject.
          if (!this.cachedInfo) {
            throw new Error('no cached daemon info — reconnect cannot discover/spawn (ADR-0021)')
          }
          info = this.cachedInfo
        }
        // host 는 일단 127.0.0.1 고정 가정(로컬 IPC).
        const ws = new WebSocket('ws://' + info.host + ':' + info.port)
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

  // attach-only 재연결 최대 시도 횟수. ADR-0021: 데몬이 죽으면(graceful stop·kill·크래시) 캐시된
  // host:port 로의 재연결이 전부 실패한다 — 무한 reconnecting 으로 매달리지 않고 이 횟수만큼 시도한
  // 뒤 'down' 으로 정착시킨다(꺼진 채 유지). 일시적 네트워크 끊김은 이 안에서 회복된다. 복구는
  // 사용자의 명시 daemon_start(=start(), discover 허용)로만 — 재연결·명령(ensureReady)은 spawn 금지.
  private static readonly MAX_RECONNECT_ATTEMPTS = 5

  private scheduleReconnect(): void {
    if (this.reconnectTimer) return
    if (this.closedByUser) return
    // attach-only 재시도 소진 → 'down' 정착(데몬이 안 살아남는다). 사용자가 명시로 다시 시작.
    if (this.reconnectAttempt >= WsTransport.MAX_RECONNECT_ATTEMPTS) {
      this.setState('down')
      return
    }
    // 500ms → 1s → 2s → … 최대 10s.
    const delay = Math.min(500 * 2 ** this.reconnectAttempt, 10000)
    this.reconnectAttempt += 1
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null
      if (this.closedByUser) return
      // ★attach-only★: discover/spawn 금지 — 캐시된 host:port 로 소켓만 재오픈(openSocket(false)).
      // 성공하면 connectPromise 가 Hello 로 resolve 되고 reconnectAttempt 가 0 으로 리셋된다.
      // 실패(데몬 죽음 → 연결 거부)하면 onclose 가 와서 다시 scheduleReconnect → 소진 시 'down'.
      this.connectPromise = this.openSocket(false)
      this.connectPromise.catch(() => {
        // openSocket reject(캐시 없음/소켓 onclose 미발화) — 직접 재스케줄(소진 시 'down').
        if (!this.reconnectTimer && !this.closedByUser) {
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
