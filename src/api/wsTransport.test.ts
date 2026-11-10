// WsTransport unit test — WS carrier responsibilities (handshake/reconnect/frame normalization).
//
// Mocks invoke('discover_daemon') + globalThis.WebSocket. FakeWebSocket fires onopen/onmessage/
// onclose manually. Verifies the carrier only (protocol semantics are protocolClient.test). Integration regression (reconnect
// resume) is verified separately with the WsTransport+ProtocolClient combination.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ADR-0310: D23 ① — the discovery DTO carries no token; the direct transport gets it from its constructor.
const TEST_TOKEN = 'test-token'
const discoverInfo = {
  pid: 4321,
  host: '127.0.0.1',
  port: 9999,
  protocol_version: 1,
}
// The current daemon.json that read_daemon_info (no-spawn re-query, ADR-0021) will return. By default the same as discoverInfo
// (when not a hot-swap = the same daemon). For a test to imitate a hot-swap/death, change this value (null = death).
let liveDaemonInfo: typeof discoverInfo | null = discoverInfo
const invokeMock = vi.fn(async (cmd: string, ..._rest: unknown[]) => {
  if (cmd === 'discover_daemon') return discoverInfo
  // read_daemon_info = read-only (no spawn). The path by which reconnect follows a daemon that moved.
  if (cmd === 'read_daemon_info') return liveDaemonInfo
  throw new Error('unexpected invoke: ' + cmd)
})
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, ...rest: unknown[]) => invokeMock(cmd, ...rest),
}))

import { WS_NO_AUTH_TOKEN_ERROR, WsTransport } from './wsTransport'
import { ProtocolClient } from './protocolClient'
import { placeholderErrorPayload } from './wsFrame'
import type { InboundMessage } from './transport'

const OPEN = 1
const CLOSED = 3
// Kept in sync with WsTransport.MAX_RECONNECT_ATTEMPTS (down after the attach-only retries are exhausted).
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
  liveDaemonInfo = discoverInfo
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

/** Trigger an explicit start (spawn allowed) + complete the handshake (discover→onopen→Auth→Hello).
 * ADR-0021 B-1: the first connection is possible only through an explicit start (ensureReady is attach-only, so it rejects with no cache). */
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
    const t = new WsTransport({ authToken: TEST_TOKEN })
    expect(t.connectionState).toBe('down')
    const ws = await connect(t)
    expect(invokeMock).toHaveBeenCalledWith('discover_daemon')
    expect(ws.url).toBe('ws://127.0.0.1:9999')
    const first = ws.parsedSent()[0] as { Auth: { token: string; protocol_version: number } }
    expect(first.Auth.token).toBe(TEST_TOKEN)
    expect(first.Auth.protocol_version).toBe(1)
    expect(t.connectionState).toBe('connected')
    t.close()
  })

  it('without authToken, start rejects at once with the fixed text (no discover, no socket, stays down)', async () => {
    const t = new WsTransport()
    await expect(t.start()).rejects.toThrow(WS_NO_AUTH_TOKEN_ERROR)
    expect(invokeMock).not.toHaveBeenCalled()
    expect(FakeWebSocket.instances).toHaveLength(0)
    expect(t.connectionState).toBe('down')
    t.close()
  })

  it('without authToken, ensureReady rejects with the fixed text on a fresh transport and after close', async () => {
    const fresh = new WsTransport()
    await expect(fresh.ensureReady()).rejects.toThrow(WS_NO_AUTH_TOKEN_ERROR)
    expect(fresh.connectionState).toBe('down')

    const closed = new WsTransport()
    closed.close()
    await expect(closed.ensureReady()).rejects.toThrow(WS_NO_AUTH_TOKEN_ERROR)
    expect(closed.connectionState).toBe('down')

    expect(invokeMock).not.toHaveBeenCalled()
    expect(FakeWebSocket.instances).toHaveLength(0)
  })

  it('Hello/Auth 는 onMessage 로 안 올라온다(handshake 내부 소비)', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    await connect(t)
    expect(got.find((m) => m.kind === 'control' && 'Hello' in m.event)).toBeUndefined()
    t.close()
  })
})

describe('WsTransport 정규화', () => {
  it('control JSON → control InboundMessage', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    const ws = await connect(t)
    ws.fireText({ Ack: { request_id: 'r1' } })
    expect(got).toContainEqual({ kind: 'control', event: { Ack: { request_id: 'r1' } } })
    t.close()
  })

  it('binary frame → output InboundMessage(디코드)', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    const ws = await connect(t)
    ws.fireBinary(buildFrame({ agentId: AGENT, epoch: 3, seq: 9, payload: new Uint8Array([0x41]) }))
    const out = got.find((m) => m.kind === 'output')
    expect(out).toMatchObject({ kind: 'output', agentId: AGENT, epoch: 3, seq: 9 })
    expect(Array.from((out as { bytes: Uint8Array }).bytes)).toEqual([0x41])
    t.close()
  })

  // ADR-0231: dropping it leaves a hole at that seq and the view holding on stalls — like the shell relay, raise it as a placeholder with the same seq.
  it('모르는 tag 의 binary frame → 같은 seq·화신의 자리채움(tag1 Error)으로 올린다', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    const ws = await connect(t)
    const frame = buildFrame({ agentId: AGENT, epoch: 3, seq: 9, payload: new Uint8Array([0x41]) })
    new DataView(frame).setUint8(0, 9)
    ws.fireBinary(frame)
    const out = got.find((m) => m.kind === 'output') as { tag: number; bytes: Uint8Array }
    expect(out).toMatchObject({ kind: 'output', tag: 1, agentId: AGENT, epoch: 3, seq: 9 })
    expect(new TextDecoder().decode(out.bytes)).toBe(new TextDecoder().decode(placeholderErrorPayload()))
    expect(ws.closed).toBe(false)
    t.close()
  })

  it('seq 를 못 읽는 짧은 binary frame → 올리지 않고 소켓을 닫는다(끊기 → 재연결)', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    const ws = await connect(t)
    ws.fireBinary(new ArrayBuffer(FRAME_HEADER_LEN - 1))
    expect(got.filter((m) => m.kind === 'output')).toEqual([])
    expect(ws.closed).toBe(true)
    ws.fireClose() // The close event the browser emits after close()
    expect(t.connectionState).toBe('reconnecting')
    t.close()
  })
})

describe('WsTransport 재연결', () => {
  it('비의도 onclose → reconnecting + 새 소켓 생성', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
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
    const t = new WsTransport({ authToken: TEST_TOKEN })
    // 1) in-flight: start → discover+openSocket → only up to onopen(Auth), Hello not received (not connected).
    // p1 is deliberately not awaited — when the 2nd start cleans up ws1, the resolve/reject of p1 (openSocket(true))
    // is never called (an abandoned promise). Only a reject-ignore is attached.
    void t.start().catch(() => {})
    await Promise.resolve()
    await Promise.resolve()
    const ws1 = FakeWebSocket.last!
    ws1.fireOpen() // Auth sent, Hello not yet received → state != 'connected'

    // 2) The orphan creation path: call start() again (clean up the previous in-flight socket, then open a new one).
    const p2 = t.start().catch(() => {})
    await Promise.resolve()
    await Promise.resolve()
    const ws2 = FakeWebSocket.last!
    expect(ws2).not.toBe(ws1)

    // ★mutation guard★ — if the cleanupSocket-first step in start() is missing, ws1.onclose stays alive and this assertion
    // breaks, and the following ws1.fireClose() clobbers this.ws of the new socket (ws2) to null through handleClose.
    expect('onclose' in ws1).toBe(false)

    // 3) A late close of the old socket — it must not break the new socket.
    ws1.fireClose()
    const instancesAfterClose = FakeWebSocket.instances.length

    ws2.fireOpen()
    ws2.fireText({ Hello: { protocol_version: 1 } })
    await p2
    expect(t.connectionState).toBe('connected')
    expect(FakeWebSocket.instances.length).toBe(instancesAfterClose)
    t.close()
  })

  it('in-flight start() 가 새 start() 로 대체되면 옛 promise 가 hang 안 하고 reject (no-hang 회귀 가드)', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    // 1) in-flight: start #1 → discover+openSocket → only up to onopen(Auth), Hello not received.
    const p1 = t.start()
    await Promise.resolve()
    await Promise.resolve()
    const ws1 = FakeWebSocket.last!
    ws1.fireOpen() // Auth sent, Hello not received → in-flight (not settled)

    // 2) start #2 → cleanupSocket(ws1) must call the pendingReject of the old in-flight.
    const p2 = t.start().catch(() => {})
    await Promise.resolve()
    await Promise.resolve()

    // ★no-hang guard★: without the pendingReject settle, p1 never settles and this await times out.
    await expect(p1).rejects.toThrow(/superseded/)

    const ws2 = FakeWebSocket.last!
    expect(ws2).not.toBe(ws1)
    ws2.fireOpen()
    ws2.fireText({ Hello: { protocol_version: 1 } })
    await p2
    expect(t.connectionState).toBe('connected')
    t.close()
  })

  it('in-flight start() 가 discover 윈도(ws null)에서 새 start() 로 대체돼도 hang 안 하고 reject (pre-ws no-hang 가드)', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    // ★mutation guard (pre-ws)★ — if cleanupSocket does not reject pendingReject even in the discover window where ws is null,
    // this assertion times out (hang). Because discover is in-flight ws is not yet created, and if the second start()'s
    // cleanupSocket → does not wake pendingReject, the first promise (p1) never settles.

    // 1) Block discover_daemon with a controllable deferred to stop the first start()'s await invoke (ws not created).
    let resolveDiscover!: (v: typeof discoverInfo) => void
    const deferred = new Promise<typeof discoverInfo>((r) => {
      resolveDiscover = r
    })
    invokeMock.mockImplementationOnce(async (cmd: string) => {
      if (cmd === 'discover_daemon') return deferred
      throw new Error('unexpected invoke: ' + cmd)
    })

    // 2) p1: discover pending → the openSocket executor sets pendingReject synchronously, ws is still null.
    const p1 = t.start()
    await Promise.resolve() // Executor synchronous body + entering run() → stops at await invoke
    await Promise.resolve()
    expect(FakeWebSocket.last).toBeNull()

    // 3) p2: the second start() → enters cleanupSocket with this.ws still null.
    const p2 = t.start().catch(() => {})

    // ★no-hang (pre-ws) guard★: bounded — if the reorder is missing, this times out here.
    await expect(p1).rejects.toThrow(/superseded/)

    // 4) The second start's discover uses the default mock (immediate discoverInfo) → handshake completes.
    await Promise.resolve()
    await Promise.resolve()
    const ws2 = FakeWebSocket.last!
    expect(ws2).not.toBeNull()
    ws2.fireOpen()
    ws2.fireText({ Hello: { protocol_version: 1 } })
    await p2
    expect(t.connectionState).toBe('connected')

    // 5) Clean up the dangling first deferred (to avoid unhandled) — p1 is already rejected, and only cachedInfo is updated.
    resolveDiscover(discoverInfo)
    t.close()
  })

  it('정상 연결 후 close() = pendingReject 이미 null → spurious reject/throw 없음', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const p = t.start()
    await Promise.resolve()
    await Promise.resolve()
    const ws = FakeWebSocket.last!
    ws.fireOpen()
    ws.fireText({ Hello: { protocol_version: 1 } })
    await expect(p).resolves.toBeUndefined()
    expect(t.connectionState).toBe('connected')
    // Healthy close — cleanupSocket must reject nothing (no unhandled rejection).
    expect(() => t.close()).not.toThrow()
    expect(t.connectionState).toBe('down')
    await expect(p).resolves.toBeUndefined()
  })

  it('close() 후 onclose 가 와도 재연결 안 함(closedByUser) + 핸들러 delete(#13133)', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const ws = await connect(t)
    t.close()
    expect('onmessage' in ws).toBe(false)
    const before = FakeWebSocket.instances.length
    ws.fireClose?.()
    await new Promise((r) => setTimeout(r, 600))
    expect(FakeWebSocket.instances.length).toBe(before)
    expect(t.connectionState).toBe('down')
  })

  // ── Blocker-1: zombie socket creation race while the reconnect read_daemon_info await yields (openGen generation guard) ──
  // Since the reconnect path openSocket(false) gained a read_daemon_info await, if close()/start() (= cleanupSocket) slips in while that await yields,
  // the resumed run() body creates a new WebSocket and becomes a zombie.
  // pendingReject only settles the outer promise and cannot stop the body from resuming → block it with a generation token (openGen).

  it('close() during read await → 재개돼도 좀비 소켓 안 생김(WS 수 불변, down 유지) [Blocker-1]', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const ws1 = await connect(t) // cache filled + connected
    invokeMock.mockClear()

    // 1) Unintentional disconnect → reconnecting. When reconnectTimer (500ms) expires, openSocket(false) runs.
    ws1.fireClose()
    expect(t.connectionState).toBe('reconnecting')

    // 2) Block the reconnect's read_daemon_info with a controllable deferred (creating the await yield window).
    let resolveRead!: (v: typeof discoverInfo) => void
    const readDeferred = new Promise<typeof discoverInfo>((r) => {
      resolveRead = r
    })
    invokeMock.mockImplementationOnce(async (cmd: string) => {
      if (cmd === 'read_daemon_info') return readDeferred
      throw new Error('unexpected invoke: ' + cmd)
    })

    // 3) reconnectTimer expires → enters openSocket(false) → stops at the read_daemon_info await (ws not created).
    await new Promise((r) => setTimeout(r, 600))
    const instancesBefore = FakeWebSocket.instances.length // No new socket yet (read is pending)

    // 4) ★race★: close() while read is pending — cleanupSocket invalidates the in-flight attempt with openGen++.
    t.close()
    expect(t.connectionState).toBe('down')

    // 5) The stalled read resolves late → the run() body resumes. Without the generation guard a new WebSocket
    //    (zombie) is created here and the severed connection revives through onopen/Hello. The guard must discard it as stale.
    resolveRead(discoverInfo)
    await new Promise((r) => setTimeout(r, 0))
    await new Promise((r) => setTimeout(r, 0))

    expect(FakeWebSocket.instances.length).toBe(instancesBefore)
    expect(t.connectionState).toBe('down')
  })

  it('start() during reconnect read await → 좀비가 정식 소켓 hijack 안 함 [Blocker-1]', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const ws1 = await connect(t)
    invokeMock.mockClear()

    ws1.fireClose()
    expect(t.connectionState).toBe('reconnecting')

    // 2) Block the reconnect read_daemon_info with a deferred.
    let resolveRead!: (v: typeof discoverInfo) => void
    const readDeferred = new Promise<typeof discoverInfo>((r) => {
      resolveRead = r
    })
    invokeMock.mockImplementationOnce(async (cmd: string) => {
      if (cmd === 'read_daemon_info') return readDeferred
      // Later calls (start's discover_daemon, etc.) use the default mock.
      if (cmd === 'discover_daemon') return discoverInfo
      throw new Error('unexpected invoke: ' + cmd)
    })

    // 3) reconnectTimer expires → openSocket(false) stops at the read await (a zombie candidate in-flight).
    await new Promise((r) => setTimeout(r, 600))

    // 4) ★race★: explicit start() — create the proper socket with cleanupSocket (openGen++) + openSocket(true) (openGen++).
    const pStart = t.start().catch(() => {})
    await Promise.resolve()
    await Promise.resolve()
    const wsReal = FakeWebSocket.last! // The proper (start) socket
    wsReal.fireOpen()
    wsReal.fireText({ Hello: { protocol_version: 1 } })
    await pStart
    expect(t.connectionState).toBe('connected')
    const instancesAfterReal = FakeWebSocket.instances.length

    // 5) The stalled reconnect read resolves late → resumes. Without the guard the zombie hijacks this.ws with a new WebSocket
    //    (overwriting the proper socket handle). The generation guard must discard it as stale.
    resolveRead(discoverInfo)
    await new Promise((r) => setTimeout(r, 0))
    await new Promise((r) => setTimeout(r, 0))

    expect(FakeWebSocket.instances.length).toBe(instancesAfterReal)
    expect(t.connectionState).toBe('connected')

    // Check that the proper socket is alive = that send goes out through the proper socket (this.ws was not hijacked).
    const before = wsReal.sent.length
    t.send({ ping: 1 })
    expect(wsReal.sent.length).toBe(before + 1)

    t.close()
  })
})

// ── FIX-4: requestReplay single-flight(sole-outstanding, per-agent) ──────────────────────
describe('WsTransport requestReplay single-flight(FIX-4)', () => {
  it('in-flight 중 동시 요청은 wire Subscribe 를 겹쳐 안 보내고 다음 1개 gen 으로 병합', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const ws = await connect(t)
    ws.sent.length = 0 // Remove the handshake Auth.

    const gen1 = await t.requestReplay(AGENT)
    expect(gen1).toBe(1n)
    let subs = ws.parsedSent().filter((m) => typeof m === 'object' && m && 'Subscribe' in m)
    expect(subs.length).toBe(1)

    const p2 = t.requestReplay(AGENT)
    const p3 = t.requestReplay(AGENT)
    subs = ws.parsedSent().filter((m) => typeof m === 'object' && m && 'Subscribe' in m)
    expect(subs.length).toBe(1)

    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    ws.fireText({ SubscribeAck: { agent_id: AGENT, current_epoch: 4, replay_from: 0, truncated: false } })
    ws.fireText({ ReplayComplete: { agent_id: AGENT, epoch: 4 } })
    const b1 = got.find((m) => m.kind === 'replayBoundary')
    expect(b1).toMatchObject({ kind: 'replayBoundary', agentId: AGENT, gen: 1n, epoch: 4 })

    const [g2, g3] = await Promise.all([p2, p3])
    expect(g2).toBe(2n)
    expect(g3).toBe(2n)
    subs = ws.parsedSent().filter((m) => typeof m === 'object' && m && 'Subscribe' in m)
    expect(subs.length).toBe(2)

    got.length = 0
    ws.fireText({ SubscribeAck: { agent_id: AGENT, current_epoch: 4, replay_from: 0, truncated: false } })
    ws.fireText({ ReplayComplete: { agent_id: AGENT, epoch: 4 } })
    const b2 = got.find((m) => m.kind === 'replayBoundary')
    expect(b2).toMatchObject({ kind: 'replayBoundary', gen: 2n })
    t.close()
  })

  it('요청 1개당 boundary 1개 — in-flight 종결 후 새 요청은 다시 즉시 송신', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const ws = await connect(t)
    ws.sent.length = 0
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))

    await t.requestReplay(AGENT)
    ws.fireText({ ReplayComplete: { agent_id: AGENT, epoch: 0 } })
    expect(got.filter((m) => m.kind === 'replayBoundary').length).toBe(1)

    got.length = 0
    const gen2 = await t.requestReplay(AGENT)
    expect(gen2).toBe(2n)
    const subs = ws.parsedSent().filter((m) => typeof m === 'object' && m && 'Subscribe' in m)
    expect(subs.length).toBe(2)
    ws.fireText({ ReplayComplete: { agent_id: AGENT, epoch: 0 } })
    expect(got.filter((m) => m.kind === 'replayBoundary').length).toBe(1)
    t.close()
  })

  // ADR-0226: the direct path has no marker, so remember the Ack's field on that request and carry it on the success boundary. ★Start fresh
  //   for every request★ — if the previous request's true leaks into the next request (an old daemon Ack without the field), loading shows instead of the first screen.
  it('SubscribeAck 의 continues_conversation 이 그 요청의 성공 경계로 가고, 다음 요청에 새지 않는다', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const ws = await connect(t)
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))

    await t.requestReplay(AGENT)
    const p2 = t.requestReplay(AGENT) // Merge during in-flight — promoted after the first boundary
    ws.fireText({
      SubscribeAck: {
        agent_id: AGENT,
        current_epoch: 4,
        replay_from: 0,
        truncated: false,
        continues_conversation: true,
      },
    })
    ws.fireText({ ReplayComplete: { agent_id: AGENT, epoch: 4 } })
    expect(got.find((m) => m.kind === 'replayBoundary')).toMatchObject({
      gen: 1n,
      failed: false,
      continuesConversation: true,
    })

    await p2
    got.length = 0
    ws.fireText({ SubscribeAck: { agent_id: AGENT, current_epoch: 4, replay_from: 0, truncated: false } })
    ws.fireText({ ReplayComplete: { agent_id: AGENT, epoch: 4 } })
    expect(got.find((m) => m.kind === 'replayBoundary')).toMatchObject({
      gen: 2n,
      continuesConversation: false,
    })
    t.close()
  })

  // ADR-0231: the direct path has no shell marker, so take the replay head from the Ack and carry it on that request's success boundary.
  //   ★Start fresh for every request★ — if the previous request's head leaks into the next request, the view skips the history in between.
  it('SubscribeAck 의 replay_from 이 그 요청의 성공 경계로 가고, 다음 요청에 새지 않는다', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const ws = await connect(t)
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))

    await t.requestReplay(AGENT)
    const p2 = t.requestReplay(AGENT) // Merge during in-flight — promoted after the first boundary
    ws.fireText({ SubscribeAck: { agent_id: AGENT, current_epoch: 4, replay_from: 40, truncated: true } })
    ws.fireText({ ReplayComplete: { agent_id: AGENT, epoch: 4 } })
    expect(got.find((m) => m.kind === 'replayBoundary')).toMatchObject({ gen: 1n, replayFrom: 40 })

    await p2
    got.length = 0
    ws.fireText({ ReplayComplete: { agent_id: AGENT, epoch: 4 } }) // A request that never saw the Ack — direct-path approximation 0
    expect(got.find((m) => m.kind === 'replayBoundary')).toMatchObject({ gen: 2n, replayFrom: 0 })
    t.close()
  })

  it('replay_from 이 없는 SubscribeAck 은 머리 0 — NaN 이 경계로 새지 않는다', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const ws = await connect(t)
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))

    await t.requestReplay(AGENT)
    ws.fireText({ SubscribeAck: { agent_id: AGENT, current_epoch: 4, truncated: false } })
    ws.fireText({ ReplayComplete: { agent_id: AGENT, epoch: 4 } })
    expect(got.find((m) => m.kind === 'replayBoundary')).toMatchObject({ gen: 1n, replayFrom: 0 })
    t.close()
  })

  it('거절(실패 경계)은 continuesConversation=false', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const ws = await connect(t)
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    await t.requestReplay(AGENT)
    ws.fireText({ SubscribeFailed: { agent_id: AGENT, reason: 'no such agent' } })
    expect(got.find((m) => m.kind === 'replayBoundary')).toMatchObject({
      failed: true,
      continuesConversation: false,
    })
    t.close()
  })
})

// ── FIX-B: the replay single-flight state does not get stuck across a socket close ──────────────────────
describe('WsTransport replay single-flight — 소켓 종료 시 상태 리셋 + 대기자 reject(FIX-B)', () => {
  it('gen1 in-flight + gen2 pending 중 소켓 close → pending 대기자 reject, 재연결 후 새 wire Subscribe 재송신', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const ws1 = await connect(t)
    ws1.sent.length = 0 // Remove the handshake Auth.

    // 1) gen1 request → send the wire Subscribe immediately + an already resolved promise (gen returned synchronously).
    const gen1 = await t.requestReplay(AGENT)
    expect(gen1).toBe(1n)
    let subs = ws1.parsedSent().filter((m) => typeof m === 'object' && m && 'Subscribe' in m)
    expect(subs.length).toBe(1)

    // 2) gen2 request while gen1 is in-flight (before ReplayComplete) → no wire send, pending merge (unsettled promise).
    const p2 = t.requestReplay(AGENT)
    // p2 will be rejected, so attach a no-op catch up front to prevent unhandled-rejection noise.
    let p2Rejected: unknown = null
    void p2.catch((e) => (p2Rejected = e))
    subs = ws1.parsedSent().filter((m) => typeof m === 'object' && m && 'Subscribe' in m)
    expect(subs.length).toBe(1)

    // 3) ★Socket close before ReplayComplete★ → handleClose empties wsReplay and rejects the pending waiters.
    ws1.fireClose()
    await expect(p2).rejects.toThrow(/closed before replay complete/)
    expect(p2Rejected).not.toBeNull()

    await new Promise((r) => setTimeout(r, 600))
    const ws2 = FakeWebSocket.last!
    expect(ws2).not.toBe(ws1)
    ws2.fireOpen()
    ws2.fireText({ Hello: { protocol_version: 1 } })
    await Promise.resolve()
    expect(t.connectionState).toBe('connected')
    ws2.sent.length = 0 // Remove the reconnect Auth.

    // 5) ★Fresh requestReplay after reconnect★: wsReplay was emptied at close, so there is no entry.inflight →
    //   send a new wire Subscribe again (before FIX-B, a stale inflight meant nothing was sent and it was stuck for good).
    const gen3 = await t.requestReplay(AGENT)
    expect(gen3).toBe(3n) // The gen counter keeps increasing (gen1=1, gen2=2 burned, gen3=3).
    subs = ws2.parsedSent().filter((m) => typeof m === 'object' && m && 'Subscribe' in m)
    expect(subs.length).toBe(1)
    t.close()
  })
})

// ── ADR-0021 B-1: command (ensureReady = attach-only) / explicit (start = spawn) separation ──────────────
describe('WsTransport B-1: ensureReady(attach-only) / start(spawn) 분리 (ADR-0021)', () => {
  // ★mutation guard★: if this test breaks, ensureReady is spawning (discover) =
  // the "respawn from one key press/resize after turning the daemon off" bug (B-1) has come back. If ensureReady is reverted to openSocket(true),
  // discover is called instead of a reject and this assertion (0 spawns + reject) fails.
  it('ensureReady = 캐시 없으면 reject + discover/spawn 0회(명령이 데몬 못 깨움)', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    await expect(t.ensureReady()).rejects.toThrow(/daemon_start/)
    expect(invokeMock).not.toHaveBeenCalled() // Never calls discover (= triggers a spawn)
    expect(FakeWebSocket.instances.length).toBe(0)
    expect(t.connectionState).toBe('down')
  })

  it('close(명시 종료) 후 ensureReady = reject + spawn 0회(closedByUser 가드)', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    await connect(t) // cache filled
    t.close() // closedByUser=true → down
    invokeMock.mockClear()
    await expect(t.ensureReady()).rejects.toThrow(/daemon_start/)
    expect(invokeMock).not.toHaveBeenCalled() // There is a cache but it is closedByUser, so it does not even attach
    expect(t.connectionState).toBe('down')
  })

  it('start 만 discover_daemon 호출(spawn 유발) — 명령은 호출 안 함', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    await connect(t) // = start()
    expect(invokeMock).toHaveBeenCalledTimes(1)
    expect(invokeMock).toHaveBeenCalledWith('discover_daemon')
    t.close()
  })

  it('ensureReady = 캐시 있으면 attach(소켓 재오픈)하되 discover/spawn 은 안 함', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const ws1 = await connect(t) // start: cache filled
    invokeMock.mockClear()
    ws1.fireClose() // Unintentional disconnect → reconnecting (connectPromise cleaned up)
    // While reconnecting, the command path calls ensureReady → the connectPromise in progress or a new attach.
    await new Promise((r) => setTimeout(r, 600))
    const ws2 = FakeWebSocket.last!
    ws2.fireOpen()
    ws2.fireText({ Hello: { protocol_version: 1 } })
    await Promise.resolve()
    const er = t.ensureReady()
    await er // Already connected → resolves immediately
    // ★No-spawn kept★: the attach path never calls discover_daemon (which triggers a spawn).
    // read_daemon_info (no-spawn re-query, ADR-0021 hot-swap tracking) may be called during reconnect (allowed).
    expect(invokeMock).not.toHaveBeenCalledWith('discover_daemon')
    t.close()
  })

  it('재연결은 discover_daemon(spawn) 금지 — read_daemon_info(no-spawn) 재조회로 현재 데몬에 attach', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const ws1 = await connect(t)
    expect(invokeMock).toHaveBeenCalledTimes(1) // Only the first discover, once
    invokeMock.mockClear()

    ws1.fireClose()
    expect(t.connectionState).toBe('reconnecting')
    await new Promise((r) => setTimeout(r, 600))

    expect(invokeMock).not.toHaveBeenCalledWith('discover_daemon')
    // ★ADR-0021 hot-swap tracking★: instead, re-query the current daemon.json with read_daemon_info (no-spawn).
    expect(invokeMock).toHaveBeenCalledWith('read_daemon_info')
    // Open a new socket with the host:port that read_daemon_info returned (here the same daemon = the same url).
    const ws2 = FakeWebSocket.last!
    expect(ws2).not.toBe(ws1)
    expect(ws2.url).toBe('ws://127.0.0.1:9999')
    t.close()
  })

  it('hot-swap: 재연결이 read_daemon_info 로 새 port 를 따라가 새 주소에 attach', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const ws1 = await connect(t) // 캐시 = 9999
    invokeMock.mockClear()

    // The daemon was replaced wholesale and came up on a new port (a stop → daemon_start). daemon.json was updated.
    liveDaemonInfo = { ...discoverInfo, port: 7777 }

    ws1.fireClose() // Old connection drops → reconnect
    expect(t.connectionState).toBe('reconnecting')
    await new Promise((r) => setTimeout(r, 600))

    // It attaches to the new address (7777) that read_daemon_info gave, not the cache (9999) (not a spawn — read-only).
    expect(invokeMock).not.toHaveBeenCalledWith('discover_daemon')
    expect(invokeMock).toHaveBeenCalledWith('read_daemon_info')
    const ws2 = FakeWebSocket.last!
    expect(ws2.url).toBe('ws://127.0.0.1:7777')
    ws2.fireOpen()
    // Auth carries the constructor token — the DTO has none to follow.
    expect((ws2.parsedSent()[0] as { Auth: { token: string } }).Auth.token).toBe(TEST_TOKEN)
    // Once Hello reaches the new daemon, attempt resets and it is connected.
    ws2.fireText({ Hello: { protocol_version: 1 } })
    await Promise.resolve()
    expect(t.connectionState).toBe('connected')
    t.close()
  })

  it('attach 재시도 소진 → discover 없이 down 정착(데몬 죽음, 안 살아남음)', async () => {
    vi.useFakeTimers()
    const t = new WsTransport({ authToken: TEST_TOKEN })
    // First connection = explicit start (manual handshake so timers have no effect).
    const p = t.start().catch(() => {})
    await Promise.resolve()
    await Promise.resolve()
    const ws1 = FakeWebSocket.last!
    ws1.fireOpen()
    ws1.fireText({ Hello: { protocol_version: 1 } })
    await p
    invokeMock.mockClear()

    // Assume the daemon died — daemon.json is also a dead daemon's, so read_daemon_info is None (no live daemon).
    // After the disconnect, every attach attempt fails at once with onclose (connection refused).
    liveDaemonInfo = null
    ws1.fireClose()
    // Each of the 5 backoffs (500/1s/2s/4s/8s) proceeds → every time the new socket closes right away.
    for (let i = 0; i < WS_MAX_ATTEMPTS; i++) {
      await vi.advanceTimersByTimeAsync(11000)
      const w = FakeWebSocket.last!
      w.fireClose()
      await Promise.resolve()
    }
    await vi.advanceTimersByTimeAsync(11000)

    // ★No-spawn kept★: read_daemon_info (no-spawn) may be called to follow a dead daemon,
    // but discover_daemon (which triggers a spawn) is 0 calls to the end — the daemon does not come back and it settles at 'down'.
    expect(invokeMock).not.toHaveBeenCalledWith('discover_daemon')
    expect(t.connectionState).toBe('down')
    t.close()
    vi.useRealTimers()
  })

  it('down 후 start(명시 재시작) → 다시 discover 허용 + 재연결 루프 부활', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const ws1 = await connect(t)
    t.close() // Explicit stop → down
    expect(t.connectionState).toBe('down')
    invokeMock.mockClear()

    // Explicit user restart (the daemon_start entry point = start) — discover allowed, closedByUser cleared.
    const ws2 = await connect(t)
    expect(invokeMock).toHaveBeenCalledTimes(1)
    expect(invokeMock).toHaveBeenCalledWith('discover_daemon')
    expect(ws2).not.toBe(ws1)
    expect(t.connectionState).toBe('connected')
    t.close()
  })
})

// ── Integration regression: the WsTransport + ProtocolClient combination (ADR-0046 view direct replay — legacy direct-connection approximation path) ──
//    After the ADR-0046 redesign: the frontend no longer sends wire Subscribe/Unsubscribe through resubscribeAll (BLOCK-1
//    rolled out everywhere). View mount · reconnect request a full re-replay through transport.requestReplay. WsTransport (legacy
//    direct) sends its own wire Subscribe{after_seq:null}, assigns a per-agent gen, and synthesizes a replayBoundary when it
//    observes the daemon ReplayComplete. ProtocolClient flushes on that marker with the gen fence.
const V1 = 'view-1'

describe('WsTransport + ProtocolClient 통합(ADR-0046 뷰 직결 replay)', () => {
  it('subscribe → requestReplay 가 wire Subscribe{after_seq:null} 송신 + ReplayComplete → 마커 flush', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const c = new ProtocolClient(t)
    const ws1 = await connect(t)
    const received: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => received.push(chunk.seq))
    await Promise.resolve()

    const sub = ws1.parsedSent().find((m) => typeof m === 'object' && m && 'Subscribe' in m) as {
      Subscribe: { agent_id: string; epoch: number | null; after_seq: number | null }
    }
    expect(sub).toBeTruthy()
    expect(sub.Subscribe.agent_id).toBe(AGENT)
    expect(sub.Subscribe.after_seq).toBeNull() // Full re-replay (full-from-oldest)

    const E = 5
    ws1.fireText({ SubscribeAck: { agent_id: AGENT, current_epoch: E, replay_from: 0, truncated: false } })
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 0 }))
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 1 }))
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 2 }))
    // Still buffering — no flush before the marker (ReplayComplete).
    expect(received).toEqual([])
    // ReplayComplete → WsTransport synthesizes a replayBoundary → ProtocolClient flush.
    ws1.fireText({ ReplayComplete: { agent_id: AGENT, epoch: E } })
    expect(received).toEqual([0, 1, 2])
    // After that, live frames go straight through.
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 3 }))
    expect(received).toEqual([0, 1, 2, 3])
    c.close()
  })

  it('live 한가운데 모르는 tag 프레임이 와도 그 seq 가 자리채움으로 채워져 뒤가 붙들리지 않는다', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const c = new ProtocolClient(t)
    const ws1 = await connect(t)
    const received: Array<[number, number]> = []
    await c.subscribeOutput(V1, AGENT, (chunk) => received.push([chunk.seq, chunk.tag]))
    await Promise.resolve()

    const E = 5
    ws1.fireText({ SubscribeAck: { agent_id: AGENT, current_epoch: E, replay_from: 0, truncated: false } })
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 0 }))
    ws1.fireText({ ReplayComplete: { agent_id: AGENT, epoch: E } })
    const unknown = buildFrame({ agentId: AGENT, epoch: E, seq: 1 })
    new DataView(unknown).setUint8(0, 9)
    ws1.fireBinary(unknown)
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 2 }))
    expect(received).toEqual([
      [0, 0],
      [1, 1],
      [2, 0],
    ])
    expect(c.getViewOutputState(V1)).toMatchObject({ phase: 'live', held: 0 })
    c.close()
  })

  // ADR-0231: a new agent whose ring was trimmed — the view's cursor is −1, so if it does not know the head (the Ack's replay_from) it waits for 0 and
  //   stalls holding 3 and 4.
  it('SubscribeAck 의 replay_from 이 flush 시작점이 된다(잘린 replay 의 머리 점프)', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const c = new ProtocolClient(t)
    const ws1 = await connect(t)
    const received: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => received.push(chunk.seq))
    await Promise.resolve()

    const E = 5
    ws1.fireText({ SubscribeAck: { agent_id: AGENT, current_epoch: E, replay_from: 3, truncated: true } })
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 3 }))
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 4 }))
    ws1.fireText({ ReplayComplete: { agent_id: AGENT, epoch: E } })
    expect(received).toEqual([3, 4])
    expect(c.getViewOutputState(V1)).toMatchObject({ phase: 'live', held: 0 })
    c.close()
  })

  // ★The re-request trigger = the registry, not the socket (user decision 2026-08-20)★: a socket coming up alone sends
  //   nothing out; the observation that the agent is in the registry attaches it. Here it was never dropped from the registry, so
  //   resuming the same session = the cursor is kept → dedup absorbs the overlapping front part of the full re-replay.
  it('재연결 → 명부 부착 → 재요청(전량 재replay) → 이미 그린 앞부분은 dedup', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
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

    ws1.fireClose()
    await new Promise((r) => setTimeout(r, 600))
    const ws2 = FakeWebSocket.last!
    ws2.fireOpen()
    ws2.fireText({ Hello: { protocol_version: 1 } })
    await Promise.resolve()
    await Promise.resolve()

    // The moment only the socket is up — nothing goes out yet (waiting for the registry).
    expect(ws2.parsedSent().some((m) => typeof m === 'object' && m && 'Subscribe' in m)).toBe(false)

    // The authoritative registry has that agent **with the same marker** → resume of the same incarnation → attach → WsTransport sends the wire
    //   Subscribe{after_seq:null}. ★Without the marker this case does not hold★ — a registry without a marker
    //   cannot prove "the same incarnation" and is treated as a rotation, making this assertion that measures cursor retention meaningless.
    ws2.fireText({ AgentListUpdated: { agents: [{ id: AGENT, epoch: E }] } })
    await Promise.resolve() // myGen is fixed (requestReplay retrieved) — markers before that are pushed to held
    const resub = ws2.parsedSent().find((m) => typeof m === 'object' && m && 'Subscribe' in m) as {
      Subscribe: { agent_id: string; after_seq: number | null }
    }
    expect(resub).toBeTruthy()
    expect(resub.Subscribe.after_seq).toBeNull() // Full re-replay (full-from-oldest, ADR-0046)

    // The daemon resends 0,1,2,3 in full — view buffering accumulates and flushes on the marker. The cursor stays at 2, so the first
    //   three drop out in dedup and only seq 3, missed during the disconnect, is newly drawn.
    ws2.fireText({ SubscribeAck: { agent_id: AGENT, current_epoch: E, replay_from: 0, truncated: false } })
    ws2.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 0 }))
    ws2.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 1 }))
    ws2.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 2 }))
    ws2.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 3 }))
    ws2.fireText({ ReplayComplete: { agent_id: AGENT, epoch: E } })
    expect(received).toEqual([0, 1, 2, 3])
    c.close()
  })

  // ★A rejection also closes an unfinished request★: otherwise this carrier's entry.inflight stays forever, every requestReplay of that agent
  //   gets merged, and the wire Subscribe never goes out again — the same defect as the zombie fixed in src-tauri (measured 2026-08-19). The
  //   production carrier is TauriTransport (ADR-0029/0036), but since it consumes the same wire, we do not leave one side
  //   healthy and the other broken.
  it('SubscribeFailed → 실패 경계 합성 + in-flight 해제(다음 요청이 다시 wire 로 나간다)', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const c = new ProtocolClient(t)
    const ws1 = await connect(t)
    await c.subscribeOutput(V1, AGENT, () => {})
    await Promise.resolve()
    const before = ws1.parsedSent().filter((m) => typeof m === 'object' && m && 'Subscribe' in m).length
    expect(before).toBe(1)

    ws1.fireText({ SubscribeFailed: { agent_id: AGENT, reason: `agent ${AGENT} not found` } })
    // Failure boundary → the view does not flush and goes to the ladder (re-request). That the view is still buffering is the proof.
    expect(c.getViewOutputState(V1)?.phase).toBe('buffering')

    // ★Key★: the slot was released, so a new request goes out on the wire again (before the fix it was merged forever).
    void t.requestReplay(AGENT)
    const after = ws1.parsedSent().filter((m) => typeof m === 'object' && m && 'Subscribe' in m).length
    expect(after).toBe(2)
    c.close()
  })

  // If there is a merge waiter, the rejection promotes that generation — it shares the same cleanup as a success close (settleReplay).
  it('SubscribeFailed → 병합된 다음 세대의 Subscribe 를 정확히 1회 송신', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const ws1 = await connect(t)
    const gen1 = await t.requestReplay(AGENT)
    const waiter = t.requestReplay(AGENT) // During in-flight → merge
    expect(ws1.parsedSent().filter((m) => typeof m === 'object' && m && 'Subscribe' in m).length).toBe(1)

    ws1.fireText({ SubscribeFailed: { agent_id: AGENT, reason: 'nope' } })
    const gen2 = await waiter
    expect(gen2).toBeGreaterThan(gen1)
    expect(ws1.parsedSent().filter((m) => typeof m === 'object' && m && 'Subscribe' in m).length).toBe(2)
    t.close()
  })

  // ★Defense★: a request that received the Ack (= accepted by the daemon) is not released by a rejection. If released, the ReplayComplete that follows
  //   meets an empty slot and is ignored, and the view waiting for that replay never gets a success marker.
  it('acked 요청은 SubscribeFailed 로 풀리지 않는다(정상 완료 경로 생존)', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const c = new ProtocolClient(t)
    const ws1 = await connect(t)
    const received: number[] = []
    await c.subscribeOutput(V1, AGENT, (ch) => received.push(ch.seq))
    await Promise.resolve()

    const E = 4
    ws1.fireText({ SubscribeAck: { agent_id: AGENT, current_epoch: E, replay_from: 0, truncated: false } })
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 0 }))
    ws1.fireText({ SubscribeFailed: { agent_id: AGENT, reason: '잘못 발화한 거절' } })
    expect(received).toEqual([]) // Still before the boundary — no failure boundary came either.

    ws1.fireText({ ReplayComplete: { agent_id: AGENT, epoch: E } })
    expect(received).toEqual([0]) // The success marker survived and was flushed.
    c.close()
  })
})
