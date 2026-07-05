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
// read_daemon_info(no-spawn 재조회, ADR-0021)가 돌려줄 현재 daemon.json. 기본은 discoverInfo 와 동일
// (hot-swap 아닌 경우 = 같은 데몬). 테스트가 hot-swap/죽음을 흉내내려면 이 값을 바꾼다(null=죽음).
let liveDaemonInfo: typeof discoverInfo | null = discoverInfo
const invokeMock = vi.fn(async (cmd: string, ..._rest: unknown[]) => {
  if (cmd === 'discover_daemon') return discoverInfo
  // read_daemon_info = read-only(spawn 안 함). 재연결이 옮겨간 데몬을 따라가는 경로.
  if (cmd === 'read_daemon_info') return liveDaemonInfo
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
  liveDaemonInfo = discoverInfo // 매 테스트 기본: read_daemon_info 는 같은 데몬을 돌려준다.
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

  // ── Blocker-1: 재연결 read_daemon_info await yield 중 좀비 소켓 생성 race (openGen 세대 가드) ──
  // 재연결 경로 openSocket(false)에 read_daemon_info await 가 생기면서, 그 await 가 yield 한 사이
  // close()/start()(=cleanupSocket)가 끼면 재개된 run() 본체가 new WebSocket 을 만들어 좀비가 된다.
  // pendingReject 는 outer promise 만 settle 하지 본체 재개를 못 막는다 → 세대 토큰(openGen)으로 차단.

  it('close() during read await → 재개돼도 좀비 소켓 안 생김(WS 수 불변, down 유지) [Blocker-1]', async () => {
    const t = new WsTransport()
    const ws1 = await connect(t) // 캐시 채움 + connected
    invokeMock.mockClear()

    // 1) 비의도 끊김 → reconnecting. reconnectTimer(500ms) 만료 시 openSocket(false) 가 돈다.
    ws1.fireClose()
    expect(t.connectionState).toBe('reconnecting')

    // 2) 재연결의 read_daemon_info 를 제어 가능한 deferred 로 막는다(await yield 윈도 생성).
    let resolveRead!: (v: typeof discoverInfo) => void
    const readDeferred = new Promise<typeof discoverInfo>((r) => {
      resolveRead = r
    })
    invokeMock.mockImplementationOnce(async (cmd: string) => {
      if (cmd === 'read_daemon_info') return readDeferred
      throw new Error('unexpected invoke: ' + cmd)
    })

    // 3) reconnectTimer 만료 → openSocket(false) 진입 → read_daemon_info await 에서 멈춤(ws 미생성).
    await new Promise((r) => setTimeout(r, 600))
    const instancesBefore = FakeWebSocket.instances.length // 아직 새 소켓 없음(read 가 pending)

    // 4) ★race★: read 가 pending 인 동안 close() — cleanupSocket 이 openGen++ 로 in-flight 시도를 무효화.
    t.close()
    expect(t.connectionState).toBe('down')

    // 5) 멈춰있던 read 가 뒤늦게 resolve → run() 본체 재개. 세대 가드가 없으면 여기서 new WebSocket
    //    (좀비)이 생기고 onopen/Hello 로 끊은 연결이 부활한다. 가드가 stale 로 폐기해야 한다.
    resolveRead(discoverInfo)
    await new Promise((r) => setTimeout(r, 0))
    await new Promise((r) => setTimeout(r, 0))

    // ★좀비 차단 단언★: 새 WS 인스턴스가 안 생겼고 상태는 down 유지(부활 없음).
    expect(FakeWebSocket.instances.length).toBe(instancesBefore)
    expect(t.connectionState).toBe('down')
  })

  it('start() during reconnect read await → 좀비가 정식 소켓 hijack 안 함 [Blocker-1]', async () => {
    const t = new WsTransport()
    const ws1 = await connect(t)
    invokeMock.mockClear()

    // 1) 끊김 → reconnecting.
    ws1.fireClose()
    expect(t.connectionState).toBe('reconnecting')

    // 2) 재연결 read_daemon_info 를 deferred 로 막는다.
    let resolveRead!: (v: typeof discoverInfo) => void
    const readDeferred = new Promise<typeof discoverInfo>((r) => {
      resolveRead = r
    })
    invokeMock.mockImplementationOnce(async (cmd: string) => {
      if (cmd === 'read_daemon_info') return readDeferred
      // 이후 호출(start 의 discover_daemon 등)은 기본 mock 으로.
      if (cmd === 'discover_daemon') return discoverInfo
      throw new Error('unexpected invoke: ' + cmd)
    })

    // 3) reconnectTimer 만료 → openSocket(false) 가 read await 에서 멈춤(좀비 후보 in-flight).
    await new Promise((r) => setTimeout(r, 600))

    // 4) ★race★: 명시 start() — cleanupSocket(openGen++) + openSocket(true)(openGen++)로 정식 소켓 생성.
    const pStart = t.start().catch(() => {})
    await Promise.resolve()
    await Promise.resolve()
    const wsReal = FakeWebSocket.last! // 정식(start) 소켓
    wsReal.fireOpen()
    wsReal.fireText({ Hello: { protocol_version: 1 } })
    await pStart
    expect(t.connectionState).toBe('connected')
    const instancesAfterReal = FakeWebSocket.instances.length

    // 5) 멈춰있던 reconnect read 가 뒤늦게 resolve → 재개. 가드 없으면 좀비가 new WebSocket 으로
    //    this.ws 를 hijack(정식 소켓 핸들 덮어씀)한다. 세대 가드가 stale 로 폐기해야 한다.
    resolveRead(discoverInfo)
    await new Promise((r) => setTimeout(r, 0))
    await new Promise((r) => setTimeout(r, 0))

    // ★hijack 차단 단언★: 좀비 소켓이 안 생겼다(WS 수 불변). this.ws 가 정식 소켓을 가리키므로
    //  정식 소켓으로 명령 전송이 정상이고, 연결은 connected 유지.
    expect(FakeWebSocket.instances.length).toBe(instancesAfterReal)
    expect(t.connectionState).toBe('connected')

    // 정식 소켓이 살아있는지 = send 가 정식 소켓으로 나가는지 확인(this.ws hijack 안 됨).
    const before = wsReal.sent.length
    t.send({ ping: 1 })
    expect(wsReal.sent.length).toBe(before + 1)

    t.close()
  })
})

// ── FIX-4: requestReplay single-flight(sole-outstanding, per-agent) ──────────────────────
describe('WsTransport requestReplay single-flight(FIX-4)', () => {
  it('in-flight 중 동시 요청은 wire Subscribe 를 겹쳐 안 보내고 다음 1개 gen 으로 병합', async () => {
    const t = new WsTransport()
    const ws = await connect(t)
    ws.sent.length = 0 // handshake Auth 제거.

    // 1) 첫 요청 → 즉시 wire Subscribe 1개 송신 + gen1 반환.
    const gen1 = await t.requestReplay(AGENT)
    expect(gen1).toBe(1n)
    let subs = ws.parsedSent().filter((m) => typeof m === 'object' && m && 'Subscribe' in m)
    expect(subs.length).toBe(1)

    // 2) in-flight(Ack/Complete 전) 중 동시 요청 2개 → wire 안 나감(sole-outstanding). 같은 gen 공유.
    const p2 = t.requestReplay(AGENT)
    const p3 = t.requestReplay(AGENT)
    subs = ws.parsedSent().filter((m) => typeof m === 'object' && m && 'Subscribe' in m)
    expect(subs.length).toBe(1) // 아직 1개(겹쳐 안 보냄)

    // 3) 첫 replay 종결(Ack→Complete) → boundary gen = gen1(마지막값 오각인 없음) + 병합요청 승격 송신.
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    ws.fireText({ SubscribeAck: { agent_id: AGENT, current_epoch: 4, replay_from: 0, truncated: false } })
    ws.fireText({ ReplayComplete: { agent_id: AGENT, epoch: 4 } })
    const b1 = got.find((m) => m.kind === 'replayBoundary')
    expect(b1).toMatchObject({ kind: 'replayBoundary', agentId: AGENT, gen: 1n, epoch: 4 })

    // 병합요청이 경계 뒤에 정확히 1회 Subscribe 송신 + 대기자 전원 같은 gen(=2) 회수.
    const [g2, g3] = await Promise.all([p2, p3])
    expect(g2).toBe(2n)
    expect(g3).toBe(2n) // 같은 gen 공유(병합)
    subs = ws.parsedSent().filter((m) => typeof m === 'object' && m && 'Subscribe' in m)
    expect(subs.length).toBe(2) // 경계 뒤 병합요청 1개 추가 = 총 2개

    // 4) 병합 replay 종결 → boundary gen = gen2(그 replay 를 종결하는 요청과 일치).
    got.length = 0
    ws.fireText({ SubscribeAck: { agent_id: AGENT, current_epoch: 4, replay_from: 0, truncated: false } })
    ws.fireText({ ReplayComplete: { agent_id: AGENT, epoch: 4 } })
    const b2 = got.find((m) => m.kind === 'replayBoundary')
    expect(b2).toMatchObject({ kind: 'replayBoundary', gen: 2n })
    t.close()
  })

  it('요청 1개당 boundary 1개 — in-flight 종결 후 새 요청은 다시 즉시 송신', async () => {
    const t = new WsTransport()
    const ws = await connect(t)
    ws.sent.length = 0
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))

    await t.requestReplay(AGENT) // gen1 즉시 송신.
    ws.fireText({ ReplayComplete: { agent_id: AGENT, epoch: 0 } })
    expect(got.filter((m) => m.kind === 'replayBoundary').length).toBe(1) // boundary 1개.

    // in-flight 종결 후 새 요청 → 다시 즉시 송신(병합 아님).
    got.length = 0
    const gen2 = await t.requestReplay(AGENT)
    expect(gen2).toBe(2n)
    const subs = ws.parsedSent().filter((m) => typeof m === 'object' && m && 'Subscribe' in m)
    expect(subs.length).toBe(2) // 첫 + 두 번째 = 2개.
    ws.fireText({ ReplayComplete: { agent_id: AGENT, epoch: 0 } })
    expect(got.filter((m) => m.kind === 'replayBoundary').length).toBe(1) // 새 요청도 boundary 1개.
    t.close()
  })
})

// ── FIX-B: replay single-flight 상태가 소켓 종료를 넘어 stuck 되지 않음 ──────────────────────
describe('WsTransport replay single-flight — 소켓 종료 시 상태 리셋 + 대기자 reject(FIX-B)', () => {
  it('gen1 in-flight + gen2 pending 중 소켓 close → pending 대기자 reject, 재연결 후 새 wire Subscribe 재송신', async () => {
    const t = new WsTransport()
    const ws1 = await connect(t)
    ws1.sent.length = 0 // handshake Auth 제거.

    // 1) gen1 요청 → 즉시 wire Subscribe 송신 + 이미 resolve 된 promise(gen 동기 반환).
    const gen1 = await t.requestReplay(AGENT)
    expect(gen1).toBe(1n)
    let subs = ws1.parsedSent().filter((m) => typeof m === 'object' && m && 'Subscribe' in m)
    expect(subs.length).toBe(1)

    // 2) gen1 in-flight(ReplayComplete 전) 중 gen2 요청 → wire 안 나감, pending 병합(미settle promise).
    const p2 = t.requestReplay(AGENT)
    // p2 가 reject 될 것이므로 unhandled-rejection 소음 방지용 no-op catch 를 미리 붙인다.
    let p2Rejected: unknown = null
    void p2.catch((e) => (p2Rejected = e))
    subs = ws1.parsedSent().filter((m) => typeof m === 'object' && m && 'Subscribe' in m)
    expect(subs.length).toBe(1) // 아직 1개(sole-outstanding).

    // 3) ★ReplayComplete 전에 소켓 close★ → handleClose 가 wsReplay 를 비우고 pending 대기자 reject.
    ws1.fireClose()
    await expect(p2).rejects.toThrow(/closed before replay complete/)
    expect(p2Rejected).not.toBeNull() // reject 관측(unhandled 아님).

    // 4) 재연결 → 새 소켓 핸드셰이크 완료.
    await new Promise((r) => setTimeout(r, 600))
    const ws2 = FakeWebSocket.last!
    expect(ws2).not.toBe(ws1)
    ws2.fireOpen()
    ws2.fireText({ Hello: { protocol_version: 1 } })
    await Promise.resolve()
    expect(t.connectionState).toBe('connected')
    ws2.sent.length = 0 // 재연결 Auth 제거.

    // 5) ★재연결 후 fresh requestReplay★: wsReplay 가 close 때 비워졌으므로 entry.inflight 가 없다 →
    //   새 wire Subscribe 를 다시 송신(FIX-B 이전엔 stale inflight 때문에 안 보내 영구 stuck).
    const gen3 = await t.requestReplay(AGENT)
    expect(gen3).toBe(3n) // gen 카운터는 계속 증가(gen1=1, gen2=2 소각, gen3=3).
    subs = ws2.parsedSent().filter((m) => typeof m === 'object' && m && 'Subscribe' in m)
    expect(subs.length).toBe(1) // 재연결 소켓으로 새 Subscribe 1개 나감.
    t.close()
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
    // ★spawn 금지 유지★: attach 경로는 discover_daemon(spawn 유발)을 절대 부르지 않는다.
    // read_daemon_info(no-spawn 재조회, ADR-0021 hot-swap 추적)는 재연결 중 호출될 수 있다(허용).
    expect(invokeMock).not.toHaveBeenCalledWith('discover_daemon')
    t.close()
  })

  it('재연결은 discover_daemon(spawn) 금지 — read_daemon_info(no-spawn) 재조회로 현재 데몬에 attach', async () => {
    const t = new WsTransport()
    const ws1 = await connect(t)
    expect(invokeMock).toHaveBeenCalledTimes(1) // 최초 discover 1회만
    invokeMock.mockClear()

    // 비의도 끊김 → attach-only 재연결.
    ws1.fireClose()
    expect(t.connectionState).toBe('reconnecting')
    await new Promise((r) => setTimeout(r, 600))

    // ★불변식★: 재연결 경로는 discover_daemon(=spawn 유발)을 절대 호출하지 않는다.
    expect(invokeMock).not.toHaveBeenCalledWith('discover_daemon')
    // ★ADR-0021 hot-swap 추적★: 대신 read_daemon_info(no-spawn)로 현재 daemon.json 을 재조회한다.
    expect(invokeMock).toHaveBeenCalledWith('read_daemon_info')
    // read_daemon_info 가 돌려준 host:port 로 새 소켓을 연다(여기선 같은 데몬 = 같은 url).
    const ws2 = FakeWebSocket.last!
    expect(ws2).not.toBe(ws1)
    expect(ws2.url).toBe('ws://127.0.0.1:9999')
    t.close()
  })

  it('hot-swap: 재연결이 read_daemon_info 로 새 port/token 을 따라가 새 주소에 attach', async () => {
    const t = new WsTransport()
    const ws1 = await connect(t) // 캐시 = 9999
    invokeMock.mockClear()

    // 데몬이 통째 교체돼 새 port 로 떴다(daemon_stop→daemon_start). daemon.json 이 갱신됨.
    liveDaemonInfo = { ...discoverInfo, port: 7777, token: 'new-token' }

    ws1.fireClose() // 옛 연결 끊김 → 재연결
    expect(t.connectionState).toBe('reconnecting')
    await new Promise((r) => setTimeout(r, 600))

    // 캐시(9999)가 아니라 read_daemon_info 가 준 새 주소(7777)로 attach 한다(spawn 아님 — read-only).
    expect(invokeMock).not.toHaveBeenCalledWith('discover_daemon')
    expect(invokeMock).toHaveBeenCalledWith('read_daemon_info')
    const ws2 = FakeWebSocket.last!
    expect(ws2.url).toBe('ws://127.0.0.1:7777')
    // 새 데몬에 Hello 까지 가면 attempt 가 리셋되고 connected.
    ws2.fireOpen()
    ws2.fireText({ Hello: { protocol_version: 1 } })
    await Promise.resolve()
    expect(t.connectionState).toBe('connected')
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

    // 데몬이 죽었다고 가정 — daemon.json 도 죽은 데몬이라 read_daemon_info 는 None(살아있는 데몬 없음).
    // 끊긴 뒤 매 attach 시도가 즉시 onclose(연결 거부)로 실패.
    liveDaemonInfo = null
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

    // ★spawn 금지 유지★: 죽은 데몬을 따라가려 read_daemon_info(no-spawn)는 부를 수 있지만,
    // discover_daemon(spawn 유발)은 끝까지 0회 — 데몬이 안 살아남고 'down' 정착.
    expect(invokeMock).not.toHaveBeenCalledWith('discover_daemon')
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

// ── 통합 회귀: WsTransport + ProtocolClient 조합(ADR-0046 뷰 직결 replay — legacy 직결 근사 경로) ──
//    ADR-0046 재설계 후: 프론트는 wire Subscribe/Unsubscribe 를 resubscribeAll 로 안 보낸다(BLOCK-1
//    전면화). 뷰 mount·재연결은 transport.requestReplay 로 전량 재replay 를 요청한다. WsTransport(legacy
//    직결)는 자체 wire Subscribe{after_seq:null} 를 보내고 per-agent gen 을 부여하며, 데몬 ReplayComplete
//    관측 시 replayBoundary 를 합성한다. ProtocolClient 는 그 마커에 gen 펜스로 flush 한다.
const V1 = 'view-1'

describe('WsTransport + ProtocolClient 통합(ADR-0046 뷰 직결 replay)', () => {
  it('subscribe → requestReplay 가 wire Subscribe{after_seq:null} 송신 + ReplayComplete → 마커 flush', async () => {
    const t = new WsTransport()
    const c = new ProtocolClient(t)
    const ws1 = await connect(t)
    const received: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => received.push(chunk.seq))
    await Promise.resolve()

    // requestReplay 가 legacy 직결 근사로 wire Subscribe{after_seq:null}(FromOldest) 를 보냈다.
    const sub = ws1.parsedSent().find((m) => typeof m === 'object' && m && 'Subscribe' in m) as {
      Subscribe: { agent_id: string; epoch: number | null; after_seq: number | null }
    }
    expect(sub).toBeTruthy()
    expect(sub.Subscribe.agent_id).toBe(AGENT)
    expect(sub.Subscribe.after_seq).toBeNull() // 전량 재replay(full-from-oldest)

    const E = 5
    ws1.fireText({ SubscribeAck: { agent_id: AGENT, current_epoch: E, replay_from: 0, truncated: false } })
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 0 }))
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 1 }))
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 2 }))
    // 아직 buffering — 마커(ReplayComplete) 전엔 flush 안 함.
    expect(received).toEqual([])
    // ReplayComplete → WsTransport 가 replayBoundary 합성 → ProtocolClient flush.
    ws1.fireText({ ReplayComplete: { agent_id: AGENT, epoch: E } })
    expect(received).toEqual([0, 1, 2])
    // 이후 라이브 프레임 직행.
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 3 }))
    expect(received).toEqual([0, 1, 2, 3])
    c.close()
  })

  it('재연결 → 재요청(전량 재replay) → 데몬이 본 seq 재전송해도 뷰별 dedup', async () => {
    const t = new WsTransport()
    const c = new ProtocolClient(t)
    const ws1 = await connect(t)
    const received: number[] = []
    await c.subscribeOutput(V1, AGENT, (ch) => received.push(ch.seq))
    await Promise.resolve()
    const E = 2
    ws1.fireText({ SubscribeAck: { agent_id: AGENT, current_epoch: E, replay_from: 0, truncated: false } })
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 0 }))
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 1 }))
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 2 }))
    ws1.fireText({ ReplayComplete: { agent_id: AGENT, epoch: E } })
    expect(received).toEqual([0, 1, 2])

    // 재연결.
    ws1.fireClose()
    await new Promise((r) => setTimeout(r, 600))
    const ws2 = FakeWebSocket.last!
    ws2.fireOpen()
    ws2.fireText({ Hello: { protocol_version: 1 } })
    await Promise.resolve()
    await Promise.resolve()

    // connected 재전이 → ProtocolClient 가 뷰 재요청 → WsTransport 가 wire Subscribe{after_seq:null} 재송신.
    const resub = ws2.parsedSent().find((m) => typeof m === 'object' && m && 'Subscribe' in m) as {
      Subscribe: { agent_id: string; after_seq: number | null }
    }
    expect(resub).toBeTruthy()
    expect(resub.Subscribe.after_seq).toBeNull() // 전량 재replay(재연결 회귀 수용, ADR-0046)

    // 데몬이 0,1,2,3 전량 재전송 — 뷰 buffering 이 축적 후 마커에 flush, dedup 로 0~2 는 이미 배달분.
    ws2.fireText({ SubscribeAck: { agent_id: AGENT, current_epoch: E, replay_from: 0, truncated: false } })
    ws2.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 0 }))
    ws2.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 1 }))
    ws2.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 2 }))
    ws2.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 3 }))
    ws2.fireText({ ReplayComplete: { agent_id: AGENT, epoch: E } })
    expect(received).toEqual([0, 1, 2, 3]) // 무중복(dedup) — seq 3 만 새로
    c.close()
  })
})
