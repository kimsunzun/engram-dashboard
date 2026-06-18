// WsTransport 단위테스트 — WS carrier 책임(핸드셰이크/재연결/프레임 정규화).
//
// invoke('discover_daemon') + globalThis.WebSocket 을 mock. FakeWebSocket 으로 onopen/onmessage/
// onclose 를 수동 발화. carrier 만 검증(프로토콜 의미론은 protocolClient.test). 통합 회귀(재연결
// resume)는 WsTransport+ProtocolClient 조합으로 별도 검증한다.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const discoverInfo = {
  pid: 4321,
  host: '127.0.0.1',
  port: 9999,
  token: 'test-token',
  protocol_version: 1,
}
const invokeMock = vi.fn(async (cmd: string, ..._rest: unknown[]) => {
  if (cmd === 'discover_daemon') return discoverInfo
  throw new Error('unexpected invoke: ' + cmd)
})
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, ...rest: unknown[]) => invokeMock(cmd, ...rest),
}))

import { WsTransport } from './wsTransport'
import { ProtocolClient } from './protocolClient'
import type { InboundMessage } from './transport'

const OPEN = 1
const CLOSED = 3
// WsTransport.MAX_RECONNECT_ATTEMPTS 와 동기화(attach-only 재시도 소진 후 down).
const WS_MAX_ATTEMPTS = 5

class FakeWebSocket {
  static OPEN = OPEN
  static CLOSED = CLOSED
  static last: FakeWebSocket | null = null
  static instances: FakeWebSocket[] = []

  url: string
  binaryType = 'blob'
  readyState = OPEN
  sent: string[] = []
  closed = false

  onopen: ((ev?: unknown) => void) | null = null
  onmessage: ((ev: { data: unknown }) => void) | null = null
  onerror: ((ev?: unknown) => void) | null = null
  onclose: ((ev?: unknown) => void) | null = null

  constructor(url: string) {
    this.url = url
    FakeWebSocket.last = this
    FakeWebSocket.instances.push(this)
  }
  send(data: string): void {
    this.sent.push(data)
  }
  close(): void {
    this.closed = true
    this.readyState = CLOSED
  }
  fireOpen(): void {
    this.readyState = OPEN
    this.onopen?.()
  }
  fireText(obj: unknown): void {
    this.onmessage?.({ data: JSON.stringify(obj) })
  }
  fireBinary(buf: ArrayBuffer): void {
    this.onmessage?.({ data: buf })
  }
  fireClose(): void {
    this.readyState = CLOSED
    this.onclose?.()
  }
  parsedSent(): unknown[] {
    return this.sent.map((s) => {
      try {
        return JSON.parse(s)
      } catch {
        return s
      }
    })
  }
}

const FRAME_HEADER_LEN = 29
function uuidToBytes(uuid: string): Uint8Array {
  const hex = uuid.replace(/-/g, '')
  const out = new Uint8Array(16)
  for (let i = 0; i < 16; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16)
  return out
}
function buildFrame(opts: { agentId: string; epoch: number; seq: number; payload?: Uint8Array }): ArrayBuffer {
  const payload = opts.payload ?? new Uint8Array(0)
  const buf = new ArrayBuffer(FRAME_HEADER_LEN + payload.length)
  const view = new DataView(buf)
  view.setUint8(0, 0)
  const idBytes = uuidToBytes(opts.agentId)
  for (let i = 0; i < 16; i++) view.setUint8(1 + i, idBytes[i])
  view.setUint32(17, opts.epoch, false)
  view.setBigUint64(21, BigInt(opts.seq), false)
  new Uint8Array(buf, FRAME_HEADER_LEN).set(payload)
  return buf
}

let uuidCounter = 0
beforeEach(() => {
  FakeWebSocket.last = null
  FakeWebSocket.instances = []
  invokeMock.mockClear()
  uuidCounter = 0
  ;(globalThis as unknown as { WebSocket: unknown }).WebSocket = FakeWebSocket
  vi.spyOn(globalThis.crypto, 'randomUUID').mockImplementation(
    () => `req-${++uuidCounter}` as `${string}-${string}-${string}-${string}-${string}`,
  )
})
afterEach(() => {
  vi.restoreAllMocks()
  vi.useRealTimers()
})

const AGENT = '12345678-9abc-def0-1234-56789abcdef0'

/** 명시 start(spawn 허용) 트리거 + 핸드셰이크 완료(discover→onopen→Auth→Hello).
 * ADR-0021 B-1: 최초 연결은 명시 start 만 가능(ensureReady 는 attach-only 라 캐시 없으면 reject). */
async function connect(t: WsTransport): Promise<FakeWebSocket> {
  const p = t.start().catch(() => {})
  await Promise.resolve()
  await Promise.resolve()
  const ws = FakeWebSocket.last!
  ws.fireOpen()
  ws.fireText({ Hello: { protocol_version: 1 } })
  await p
  return ws
}

describe('WsTransport 핸드셰이크', () => {
  it('ensureReady → discover + onopen 후 Auth 전송 + Hello 로 connected', async () => {
    const t = new WsTransport()
    expect(t.connectionState).toBe('down')
    const ws = await connect(t)
    expect(invokeMock).toHaveBeenCalledWith('discover_daemon')
    expect(ws.url).toBe('ws://127.0.0.1:9999')
    const first = ws.parsedSent()[0] as { Auth: { token: string; protocol_version: number } }
    expect(first.Auth.token).toBe('test-token')
    expect(first.Auth.protocol_version).toBe(1)
    expect(t.connectionState).toBe('connected')
    t.close()
  })

  it('Hello/Auth 는 onMessage 로 안 올라온다(handshake 내부 소비)', async () => {
    const t = new WsTransport()
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    await connect(t)
    // Hello 는 소비됨 — control 로 올라오면 안 됨.
    expect(got.find((m) => m.kind === 'control' && 'Hello' in m.event)).toBeUndefined()
    t.close()
  })
})

describe('WsTransport 정규화', () => {
  it('control JSON → control InboundMessage', async () => {
    const t = new WsTransport()
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    const ws = await connect(t)
    ws.fireText({ Ack: { request_id: 'r1' } })
    expect(got).toContainEqual({ kind: 'control', event: { Ack: { request_id: 'r1' } } })
    t.close()
  })

  it('binary frame → output InboundMessage(디코드)', async () => {
    const t = new WsTransport()
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    const ws = await connect(t)
    ws.fireBinary(buildFrame({ agentId: AGENT, epoch: 3, seq: 9, payload: new Uint8Array([0x41]) }))
    const out = got.find((m) => m.kind === 'output')
    expect(out).toMatchObject({ kind: 'output', agentId: AGENT, epoch: 3, seq: 9 })
    expect(Array.from((out as { bytes: Uint8Array }).bytes)).toEqual([0x41])
    t.close()
  })
})

describe('WsTransport 재연결', () => {
  it('비의도 onclose → reconnecting + 새 소켓 생성', async () => {
    const t = new WsTransport()
    const ws1 = await connect(t)
    ws1.fireClose()
    expect(t.connectionState).toBe('reconnecting')
    await new Promise((r) => setTimeout(r, 600))
    const ws2 = FakeWebSocket.last!
    expect(ws2).not.toBe(ws1)
    ws2.fireOpen()
    ws2.fireText({ Hello: { protocol_version: 1 } })
    await Promise.resolve()
    expect(t.connectionState).toBe('connected')
    t.close()
  })

  it('in-flight 소켓 중 start() 재호출 → 옛 소켓 detach(onclose 정리) → 옛 close 가 새 소켓 안 깸', async () => {
    const t = new WsTransport()
    // 1) in-flight: start → discover+openSocket → onopen(Auth) 까지만, Hello 미수신(connected 아님).
    // p1 은 의도적으로 await 하지 않는다 — 2번째 start 가 ws1 을 cleanup 하면 p1(openSocket(true))의
    // resolve/reject 는 영영 안 불린다(버려진 promise). reject 무시만 붙인다.
    void t.start().catch(() => {})
    await Promise.resolve()
    await Promise.resolve()
    const ws1 = FakeWebSocket.last!
    ws1.fireOpen() // Auth 전송됨, 아직 Hello 안 옴 → state != 'connected'

    // 2) orphan 생성 경로: start() 재호출(이전 in-flight 소켓을 cleanup 후 새 소켓 오픈).
    const p2 = t.start().catch(() => {})
    await Promise.resolve()
    await Promise.resolve()
    const ws2 = FakeWebSocket.last!
    expect(ws2).not.toBe(ws1)

    // ★mutation 가드★ — start()에서 cleanupSocket 선행이 빠지면 ws1.onclose 가 살아있어 이 단언이
    // 깨지고, 이어지는 ws1.fireClose() 가 handleClose 로 새 소켓(ws2)의 this.ws 를 null 로 clobber 한다.
    expect('onclose' in ws1).toBe(false)

    // 3) 옛 소켓의 뒤늦은 close — 새 소켓을 망가뜨리면 안 됨.
    ws1.fireClose()
    const instancesAfterClose = FakeWebSocket.instances.length

    // 4) 새 소켓 핸드셰이크 완료 → connected, ws1.close 로 인한 reconnect 없음(새 소켓 무사).
    ws2.fireOpen()
    ws2.fireText({ Hello: { protocol_version: 1 } })
    await p2
    expect(t.connectionState).toBe('connected')
    expect(FakeWebSocket.instances.length).toBe(instancesAfterClose) // reconnect 새 소켓 안 생김
    t.close()
  })

  it('in-flight start() 가 새 start() 로 대체되면 옛 promise 가 hang 안 하고 reject (no-hang 회귀 가드)', async () => {
    const t = new WsTransport()
    // 1) in-flight: start #1 → discover+openSocket → onopen(Auth) 까지만, Hello 미수신.
    const p1 = t.start()
    await Promise.resolve()
    await Promise.resolve()
    const ws1 = FakeWebSocket.last!
    ws1.fireOpen() // Auth 전송, Hello 미수신 → in-flight(미settle)

    // 2) start #2 → cleanupSocket(ws1) 이 옛 in-flight 의 pendingReject 를 호출해야 한다.
    const p2 = t.start().catch(() => {})
    await Promise.resolve()
    await Promise.resolve()

    // ★no-hang 가드★: pendingReject-settle 이 없으면 p1 은 영영 settle 안 돼 이 await 가 timeout.
    await expect(p1).rejects.toThrow(/superseded/)

    // 3) start #2 핸드셰이크 완료 → connected.
    const ws2 = FakeWebSocket.last!
    expect(ws2).not.toBe(ws1)
    ws2.fireOpen()
    ws2.fireText({ Hello: { protocol_version: 1 } })
    await p2
    expect(t.connectionState).toBe('connected')
    t.close()
  })

  it('in-flight start() 가 discover 윈도(ws null)에서 새 start() 로 대체돼도 hang 안 하고 reject (pre-ws no-hang 가드)', async () => {
    const t = new WsTransport()
    // ★mutation 가드(pre-ws)★ — cleanupSocket 이 pendingReject 를 ws null 인 discover 윈도에서도
    // reject 하지 않으면 이 단언이 timeout(hang). discover 가 in-flight 라 ws 는 아직 생성 전인데,
    // 두 번째 start()가 cleanupSocket → pendingReject 를 안 깨우면 첫 promise(p1)가 영영 settle 안 됨.

    // 1) discover_daemon 을 제어 가능한 deferred 로 막아 첫 start()의 await invoke 를 멈춘다(ws 미생성).
    let resolveDiscover!: (v: typeof discoverInfo) => void
    const deferred = new Promise<typeof discoverInfo>((r) => {
      resolveDiscover = r
    })
    invokeMock.mockImplementationOnce(async (cmd: string) => {
      if (cmd === 'discover_daemon') return deferred
      throw new Error('unexpected invoke: ' + cmd)
    })

    // 2) p1: discover 가 pending → openSocket 실행자는 동기로 pendingReject 설정, ws 는 아직 null.
    const p1 = t.start()
    await Promise.resolve() // 실행자 동기 본문 + run() 진입 → await invoke 에서 멈춤
    await Promise.resolve()
    expect(FakeWebSocket.last).toBeNull() // ws 생성 전(discover in-flight) 확인

    // 3) p2: 두 번째 start() → this.ws 가 null 인 채 cleanupSocket 진입.
    const p2 = t.start().catch(() => {})

    // ★no-hang(pre-ws) 가드★: bounded — reorder 빠지면 여기서 timeout.
    await expect(p1).rejects.toThrow(/superseded/)

    // 4) 두 번째 start 의 discover 는 기본 mock(즉시 discoverInfo) → 핸드셰이크 완료.
    await Promise.resolve()
    await Promise.resolve()
    const ws2 = FakeWebSocket.last!
    expect(ws2).not.toBeNull()
    ws2.fireOpen()
    ws2.fireText({ Hello: { protocol_version: 1 } })
    await p2
    expect(t.connectionState).toBe('connected')

    // 5) 매달린 첫 deferred 정리(unhandled 방지) — p1 은 이미 reject 됐고 cachedInfo 만 갱신.
    resolveDiscover(discoverInfo)
    t.close()
  })

  it('정상 연결 후 close() = pendingReject 이미 null → spurious reject/throw 없음', async () => {
    const t = new WsTransport()
    const p = t.start()
    await Promise.resolve()
    await Promise.resolve()
    const ws = FakeWebSocket.last!
    ws.fireOpen()
    ws.fireText({ Hello: { protocol_version: 1 } })
    // 정상 resolve — pendingReject 가 null 로 비워졌다.
    await expect(p).resolves.toBeUndefined()
    expect(t.connectionState).toBe('connected')
    // healthy close — cleanupSocket 이 아무것도 reject 하지 않아야 한다(unhandled rejection 없음).
    expect(() => t.close()).not.toThrow()
    expect(t.connectionState).toBe('down')
    // 이미 resolve 된 promise 는 close 에 영향받지 않는다.
    await expect(p).resolves.toBeUndefined()
  })

  it('close() 후 onclose 가 와도 재연결 안 함(closedByUser) + 핸들러 delete(#13133)', async () => {
    const t = new WsTransport()
    const ws = await connect(t)
    t.close()
    expect('onmessage' in ws).toBe(false)
    const before = FakeWebSocket.instances.length
    ws.fireClose?.()
    await new Promise((r) => setTimeout(r, 600))
    expect(FakeWebSocket.instances.length).toBe(before)
    expect(t.connectionState).toBe('down')
  })
})

// ── ADR-0021 B-1: 명령(ensureReady=attach-only) / 명시(start=spawn) 분리 ──────────────
describe('WsTransport B-1: ensureReady(attach-only) / start(spawn) 분리 (ADR-0021)', () => {
  // ★mutation 가드★: 이 테스트가 깨지면 ensureReady 가 spawn(discover)을 하고 있다는 뜻 =
  // "데몬 끈 뒤 키 한 번/리사이즈로 respawn" 버그(B-1) 재발. ensureReady 를 openSocket(true)로
  // 되돌리면 reject 대신 discover 가 불려 이 단언(spawn 0회 + reject)이 실패한다.
  it('ensureReady = 캐시 없으면 reject + discover/spawn 0회(명령이 데몬 못 깨움)', async () => {
    const t = new WsTransport()
    await expect(t.ensureReady()).rejects.toThrow(/daemon_start/)
    expect(invokeMock).not.toHaveBeenCalled() // discover(=spawn 유발) 절대 안 함
    expect(FakeWebSocket.instances.length).toBe(0) // 소켓조차 안 엶
    expect(t.connectionState).toBe('down')
  })

  it('close(명시 종료) 후 ensureReady = reject + spawn 0회(closedByUser 가드)', async () => {
    const t = new WsTransport()
    await connect(t) // 캐시 채움
    t.close() // closedByUser=true → down
    invokeMock.mockClear()
    await expect(t.ensureReady()).rejects.toThrow(/daemon_start/)
    expect(invokeMock).not.toHaveBeenCalled() // 캐시는 있지만 closedByUser 라 attach 도 안 함
    expect(t.connectionState).toBe('down')
  })

  it('start 만 discover_daemon 호출(spawn 유발) — 명령은 호출 안 함', async () => {
    const t = new WsTransport()
    await connect(t) // = start()
    expect(invokeMock).toHaveBeenCalledTimes(1)
    expect(invokeMock).toHaveBeenCalledWith('discover_daemon')
    t.close()
  })

  it('ensureReady = 캐시 있으면 attach(소켓 재오픈)하되 discover/spawn 은 안 함', async () => {
    const t = new WsTransport()
    const ws1 = await connect(t) // start: 캐시 채움
    invokeMock.mockClear()
    ws1.fireClose() // 비의도 끊김 → reconnecting (connectPromise 정리)
    // reconnecting 중 명령 경로가 ensureReady 호출 → 진행 중 connectPromise 또는 새 attach.
    await new Promise((r) => setTimeout(r, 600))
    const ws2 = FakeWebSocket.last!
    ws2.fireOpen()
    ws2.fireText({ Hello: { protocol_version: 1 } })
    await Promise.resolve()
    const er = t.ensureReady()
    await er // 이미 connected → 즉시 resolve
    expect(invokeMock).not.toHaveBeenCalled() // attach 경로는 discover 안 함
    t.close()
  })

  it('재연결은 discover_daemon 을 호출하지 않고(spawn 금지) 캐시 host:port 로 소켓만 재오픈', async () => {
    const t = new WsTransport()
    const ws1 = await connect(t)
    expect(invokeMock).toHaveBeenCalledTimes(1) // 최초 1회만
    invokeMock.mockClear()

    // 비의도 끊김 → attach-only 재연결.
    ws1.fireClose()
    expect(t.connectionState).toBe('reconnecting')
    await new Promise((r) => setTimeout(r, 600))

    // ★불변식★: 재연결 경로는 discover(=spawn 유발)를 절대 호출하지 않는다.
    expect(invokeMock).not.toHaveBeenCalled()
    // 캐시된 host:port 로 새 소켓을 연다(같은 url).
    const ws2 = FakeWebSocket.last!
    expect(ws2).not.toBe(ws1)
    expect(ws2.url).toBe('ws://127.0.0.1:9999')
    t.close()
  })

  it('attach 재시도 소진 → discover 없이 down 정착(데몬 죽음, 안 살아남음)', async () => {
    vi.useFakeTimers()
    const t = new WsTransport()
    // 최초 연결 = 명시 start(타이머 영향 없게 수동 핸드셰이크).
    const p = t.start().catch(() => {})
    await Promise.resolve()
    await Promise.resolve()
    const ws1 = FakeWebSocket.last!
    ws1.fireOpen()
    ws1.fireText({ Hello: { protocol_version: 1 } })
    await p
    invokeMock.mockClear()

    // 데몬이 죽었다고 가정 — 끊긴 뒤 매 attach 시도가 즉시 onclose(연결 거부)로 실패.
    ws1.fireClose()
    // 5회 백오프(500/1s/2s/4s/8s) 각각 진행 → 매번 새 소켓이 곧장 닫힘.
    for (let i = 0; i < WS_MAX_ATTEMPTS; i++) {
      await vi.advanceTimersByTimeAsync(11000)
      const w = FakeWebSocket.last!
      // attach-only 소켓이 핸드셰이크 전에 닫힘(데몬 죽음).
      w.fireClose()
      await Promise.resolve()
    }
    await vi.advanceTimersByTimeAsync(11000)

    expect(invokeMock).not.toHaveBeenCalled() // 끝까지 discover/spawn 없음
    expect(t.connectionState).toBe('down')
    t.close()
    vi.useRealTimers()
  })

  it('down 후 start(명시 재시작) → 다시 discover 허용 + 재연결 루프 부활', async () => {
    const t = new WsTransport()
    const ws1 = await connect(t)
    t.close() // 명시 종료 → down
    expect(t.connectionState).toBe('down')
    invokeMock.mockClear()

    // 사용자 명시 재시작(daemon_start 진입점 = start) — discover 허용, closedByUser 해제.
    const ws2 = await connect(t)
    expect(invokeMock).toHaveBeenCalledTimes(1)
    expect(invokeMock).toHaveBeenCalledWith('discover_daemon')
    expect(ws2).not.toBe(ws1)
    expect(t.connectionState).toBe('connected')
    t.close()
  })
})

// ── 통합 회귀: WsTransport + ProtocolClient 조합(기존 daemonClient.test 재연결 resume 이관) ──
describe('WsTransport + ProtocolClient 통합(재연결 resume, R3)', () => {
  it('재연결 시 알려진 epoch + after_seq=마지막배달seq 로 resubscribe → 무손실·무중복', async () => {
    const t = new WsTransport()
    const c = new ProtocolClient(t)
    const ws1 = await connect(t)
    const received: number[] = []
    await c.subscribeOutput(AGENT, (chunk) => received.push(chunk.seq))
    await Promise.resolve()

    const E = 5
    ws1.fireText({ SubscribeAck: { agent_id: AGENT, current_epoch: E, replay_from: 0, truncated: false } })
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 0 }))
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 1 }))
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 2 }))
    expect(received).toEqual([0, 1, 2])

    ws1.fireClose()
    await new Promise((r) => setTimeout(r, 600))
    const ws2 = FakeWebSocket.last!
    ws2.fireOpen()
    ws2.fireText({ Hello: { protocol_version: 1 } })
    await Promise.resolve()

    // resubscribe: epoch=E(null 아님) + after_seq=2.
    const resub = ws2.parsedSent().find((m) => typeof m === 'object' && m && 'Subscribe' in m) as {
      Subscribe: { agent_id: string; epoch: number | null; after_seq: number | null }
    }
    expect(resub).toBeTruthy()
    expect(resub.Subscribe.epoch).toBe(E)
    expect(resub.Subscribe.after_seq).toBe(2)

    ws2.fireText({ SubscribeAck: { agent_id: AGENT, current_epoch: E, replay_from: 3, truncated: false } })
    ws2.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 3 }))
    expect(received).toEqual([0, 1, 2, 3])
    c.close()
  })

  it('재연결 후 데몬이 본 seq 재전송해도 dedup(중복 배달 안 함)', async () => {
    const t = new WsTransport()
    const c = new ProtocolClient(t)
    const ws1 = await connect(t)
    const received: number[] = []
    await c.subscribeOutput(AGENT, (ch) => received.push(ch.seq))
    await Promise.resolve()
    const E = 2
    ws1.fireText({ SubscribeAck: { agent_id: AGENT, current_epoch: E, replay_from: 0, truncated: false } })
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 0 }))
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 1 }))
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 2 }))
    ws1.fireClose()
    await new Promise((r) => setTimeout(r, 600))
    const ws2 = FakeWebSocket.last!
    ws2.fireOpen()
    ws2.fireText({ Hello: { protocol_version: 1 } })
    await Promise.resolve()
    ws2.fireText({ SubscribeAck: { agent_id: AGENT, current_epoch: E, replay_from: 0, truncated: false } })
    ws2.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 0 }))
    ws2.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 1 }))
    ws2.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 2 }))
    ws2.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 3 }))
    expect(received).toEqual([0, 1, 2, 3])
    c.close()
  })
})
