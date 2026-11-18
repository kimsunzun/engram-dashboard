// WsTransport unit test — WS carrier responsibilities (handshake/reconnect/frame normalization). Mocks invoke('discover_daemon') + globalThis.WebSocket; FakeWebSocket fires onopen/onmessage/onclose manually.
// Carrier only (protocol semantics are protocolClient.test); the WsTransport+ProtocolClient integration regression is verified separately.

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

/** Trigger an explicit start (spawn allowed) + complete the handshake (discover→onopen→Auth→Hello). ADR-0021 B-1: the first connection is possible only through an explicit start (ensureReady is attach-only and rejects with no cache). */
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

describe('WsTransport handshake', () => {
  it('ensureReady → discover + sends Auth after onopen + connected on Hello', async () => {
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

  it('Hello/Auth do not come up through onMessage (consumed inside the handshake)', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    await connect(t)
    expect(got.find((m) => m.kind === 'control' && 'Hello' in m.event)).toBeUndefined()
    t.close()
  })
})

describe('WsTransport normalization', () => {
  it('control JSON → control InboundMessage', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    const ws = await connect(t)
    ws.fireText({ Ack: { request_id: 'r1' } })
    expect(got).toContainEqual({ kind: 'control', event: { Ack: { request_id: 'r1' } } })
    t.close()
  })

  it('binary frame → output InboundMessage (decoded)', async () => {
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
  it('a binary frame with an unknown tag → raised as a placeholder (tag1 Error) of the same seq and incarnation', async () => {
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

  it('a short binary frame whose seq cannot be read → not raised, the socket is closed (disconnect → reconnect)', async () => {
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

describe('WsTransport reconnect', () => {
  it('unintended onclose → reconnecting + new socket created', async () => {
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

  it('start() called again during an in-flight socket → old socket detached (onclose cleanup) → the old close does not break the new socket', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    // p1 is deliberately not awaited — when the 2nd start cleans up ws1, p1's resolve/reject (openSocket(true)) is never called (an abandoned promise); only a reject-ignore is attached.
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

    // ★mutation guard★ — without the cleanupSocket-first step in start(), ws1.onclose stays alive and this breaks; the following ws1.fireClose() would null the new socket's this.ws through handleClose.
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

  it('when an in-flight start() is replaced by a new start(), the old promise does not hang but rejects (no-hang regression guard)', async () => {
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

  it('even when an in-flight start() is replaced by a new start() in the discover window (ws null), it does not hang but rejects (pre-ws no-hang guard)', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    // ★mutation guard (pre-ws)★ — if cleanupSocket does not reject pendingReject in the discover window where ws is null, p1 never settles and this times out (the second start()'s cleanupSocket must wake it).

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

  it('close() after a normal connection = pendingReject already null → no spurious reject/throw', async () => {
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

  it('no reconnect even when onclose comes after close() (closedByUser) + handler delete (#13133)', async () => {
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

  // ── Zombie socket creation race while the reconnect read_daemon_info await yields (openGen generation guard) ──
  // openSocket(false) awaits read_daemon_info; if close()/start() (= cleanupSocket) slips in while it yields, the resumed run() body would create a new WebSocket (a zombie). pendingReject only settles the outer promise, so a generation token (openGen) must block it.

  it('close() during the read await → no zombie socket even when resumed (WS count unchanged, stays down) [Blocker-1]', async () => {
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

    // 5) The stalled read resolves late → run() resumes. Without the generation guard a new WebSocket (zombie) is created and the severed connection revives through onopen/Hello; the guard must discard it as stale.
    resolveRead(discoverInfo)
    await new Promise((r) => setTimeout(r, 0))
    await new Promise((r) => setTimeout(r, 0))

    expect(FakeWebSocket.instances.length).toBe(instancesBefore)
    expect(t.connectionState).toBe('down')
  })

  it('start() during the reconnect read await → the zombie does not hijack the proper socket [Blocker-1]', async () => {
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

    // 5) The stalled reconnect read resolves late → resumes. Without the guard the zombie hijacks this.ws (overwriting the proper socket handle); the generation guard must discard it as stale.
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
  it('concurrent requests during in-flight do not send overlapping wire Subscribes and merge into the next 1 gen', async () => {
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

  it('1 boundary per request — after the in-flight one closes, a new request is sent immediately again', async () => {
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

  // ADR-0226: the direct path has no marker, so remember the Ack's field on that request and carry it on the success boundary. ★Start fresh for every request★ — a leaked true (old daemon Ack without the field) would show loading instead of the first screen.
  it("SubscribeAck's continues_conversation goes to that request's success boundary and does not leak into the next request", async () => {
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

  // ADR-0231: the direct path has no shell marker, so take the replay head from the Ack and carry it on that request's success boundary. ★Start fresh for every request★ — a leaked head makes the view skip the history in between.
  it("SubscribeAck's replay_from goes to that request's success boundary and does not leak into the next request", async () => {
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

  it('a SubscribeAck without replay_from has head 0 — NaN does not leak into the boundary', async () => {
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

  it('a refusal (failure boundary) has continuesConversation=false', async () => {
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
describe('WsTransport replay single-flight — state reset + waiter reject on socket close (FIX-B)', () => {
  it('socket close during gen1 in-flight + gen2 pending → the pending waiter rejects, a new wire Subscribe is resent after reconnect', async () => {
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

    // 5) ★Fresh requestReplay after reconnect★: wsReplay was emptied at close, so no entry.inflight → a new wire Subscribe goes out (before FIX-B a stale inflight sent nothing and was stuck for good).
    const gen3 = await t.requestReplay(AGENT)
    expect(gen3).toBe(3n) // The gen counter keeps increasing (gen1=1, gen2=2 burned, gen3=3).
    subs = ws2.parsedSent().filter((m) => typeof m === 'object' && m && 'Subscribe' in m)
    expect(subs.length).toBe(1)
    t.close()
  })
})

// ── ADR-0021 B-1: command (ensureReady = attach-only) / explicit (start = spawn) separation ──────────────
describe('WsTransport B-1: ensureReady (attach-only) / start (spawn) split (ADR-0021)', () => {
  // ★mutation guard★: if this breaks, ensureReady is spawning (discover) — the "respawn from one key press/resize after turning the daemon off" bug (B-1) is back (reverting ensureReady to openSocket(true) calls discover instead of rejecting).
  it('ensureReady = rejects without a cache + 0 discover/spawn (commands cannot wake the daemon)', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    await expect(t.ensureReady()).rejects.toThrow(/daemon_start/)
    expect(invokeMock).not.toHaveBeenCalled() // Never calls discover (= triggers a spawn)
    expect(FakeWebSocket.instances.length).toBe(0)
    expect(t.connectionState).toBe('down')
  })

  it('ensureReady after close (explicit shutdown) = reject + 0 spawns (closedByUser guard)', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    await connect(t) // cache filled
    t.close() // closedByUser=true → down
    invokeMock.mockClear()
    await expect(t.ensureReady()).rejects.toThrow(/daemon_start/)
    expect(invokeMock).not.toHaveBeenCalled() // There is a cache but it is closedByUser, so it does not even attach
    expect(t.connectionState).toBe('down')
  })

  it('only start calls discover_daemon (causes spawn) — commands do not call it', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    await connect(t) // = start()
    expect(invokeMock).toHaveBeenCalledTimes(1)
    expect(invokeMock).toHaveBeenCalledWith('discover_daemon')
    t.close()
  })

  it('ensureReady = with a cache it attaches (reopens the socket) but does not discover/spawn', async () => {
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
    // ★No-spawn kept★: the attach path never calls discover_daemon (which triggers a spawn); read_daemon_info (no-spawn re-query, ADR-0021 hot-swap tracking) may be called during reconnect (allowed).
    expect(invokeMock).not.toHaveBeenCalledWith('discover_daemon')
    t.close()
  })

  it('reconnect must not discover_daemon (spawn) — it attaches to the current daemon by re-querying read_daemon_info (no-spawn)', async () => {
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

  it('hot-swap: the reconnect follows the new port via read_daemon_info and attaches to the new address', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const ws1 = await connect(t) // cache = 9999
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

  it('attach retries exhausted → settles down without discover (daemon dead, does not survive)', async () => {
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

    // Assume the daemon died — daemon.json is a dead daemon's, so read_daemon_info is None; after the disconnect every attach attempt fails at once with onclose (connection refused).
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

    // ★No-spawn kept★: read_daemon_info (no-spawn) may be called to follow a dead daemon, but discover_daemon (which triggers a spawn) is 0 calls to the end — the daemon does not come back and it settles at 'down'.
    expect(invokeMock).not.toHaveBeenCalledWith('discover_daemon')
    expect(t.connectionState).toBe('down')
    t.close()
    vi.useRealTimers()
  })

  it('start after down (explicit restart) → discover allowed again + the reconnect loop comes back', async () => {
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
//    The frontend no longer sends wire Subscribe/Unsubscribe through resubscribeAll (BLOCK-1). View mount · reconnect request a full re-replay through transport.requestReplay; WsTransport (legacy direct) sends its own wire Subscribe{after_seq:null},
//    assigns a per-agent gen, and synthesizes a replayBoundary on the daemon ReplayComplete; ProtocolClient flushes on that marker with the gen fence.
const V1 = 'view-1'

describe('WsTransport + ProtocolClient integration (ADR-0046 view-direct replay)', () => {
  it('subscribe → requestReplay sends wire Subscribe{after_seq:null} + ReplayComplete → marker flush', async () => {
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

  it('even when a frame with an unknown tag comes in the middle of live, its seq is filled with a placeholder and what is behind it is not held', async () => {
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

  // ADR-0231: a new agent whose ring was trimmed — the view's cursor is −1, so without the head (the Ack's replay_from) it waits for 0 and stalls holding 3 and 4.
  it("SubscribeAck's replay_from becomes the flush starting point (head jump of a truncated replay)", async () => {
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

  // ★The re-request trigger = the registry, not the socket★: a socket coming up alone sends nothing; observing the agent in the registry attaches it. Never dropped from the registry here = resuming the same session → the cursor is kept and dedup absorbs the overlapping front of the full re-replay.
  it('reconnect → roster attach → re-request (full re-replay) → the already drawn front part is dedup', async () => {
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

    // The authoritative registry has that agent **with the same marker** → same incarnation resumes → attach → WsTransport sends wire Subscribe{after_seq:null}. ★Without the marker this case does not hold★ — a registry without a marker cannot prove "the same incarnation" and is treated as a rotation, making this cursor-retention assertion meaningless.
    ws2.fireText({ AgentListUpdated: { agents: [{ id: AGENT, epoch: E }] } })
    await Promise.resolve() // myGen is fixed (requestReplay retrieved) — markers before that are pushed to held
    const resub = ws2.parsedSent().find((m) => typeof m === 'object' && m && 'Subscribe' in m) as {
      Subscribe: { agent_id: string; after_seq: number | null }
    }
    expect(resub).toBeTruthy()
    expect(resub.Subscribe.after_seq).toBeNull() // Full re-replay (full-from-oldest, ADR-0046)

    // The daemon resends 0,1,2,3 in full — the view buffers and flushes on the marker. The cursor stays at 2, so the first three drop out in dedup and only seq 3, missed during the disconnect, is newly drawn.
    ws2.fireText({ SubscribeAck: { agent_id: AGENT, current_epoch: E, replay_from: 0, truncated: false } })
    ws2.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 0 }))
    ws2.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 1 }))
    ws2.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 2 }))
    ws2.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 3 }))
    ws2.fireText({ ReplayComplete: { agent_id: AGENT, epoch: E } })
    expect(received).toEqual([0, 1, 2, 3])
    c.close()
  })

  // ★A rejection also closes an unfinished request★: otherwise this carrier's entry.inflight stays forever, every requestReplay of that agent merges, and the wire Subscribe never goes out again — the same zombie defect fixed in src-tauri. The production carrier is TauriTransport (ADR-0029/0036), but it consumes the same wire, so one side must not stay healthy while the other is broken.
  it('SubscribeFailed → failure boundary synthesized + in-flight released (the next request goes out on the wire again)', async () => {
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
  it('SubscribeFailed → sends the Subscribe of the merged next generation exactly once', async () => {
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

  // ★Defense★: a request that received the Ack (= accepted by the daemon) is not released by a rejection — otherwise the following ReplayComplete meets an empty slot and is ignored, and the view never gets a success marker.
  it('an acked request is not released by SubscribeFailed (the normal completion path survives)', async () => {
    const t = new WsTransport({ authToken: TEST_TOKEN })
    const c = new ProtocolClient(t)
    const ws1 = await connect(t)
    const received: number[] = []
    await c.subscribeOutput(V1, AGENT, (ch) => received.push(ch.seq))
    await Promise.resolve()

    const E = 4
    ws1.fireText({ SubscribeAck: { agent_id: AGENT, current_epoch: E, replay_from: 0, truncated: false } })
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 0 }))
    ws1.fireText({ SubscribeFailed: { agent_id: AGENT, reason: 'a wrongly fired refusal' } })
    expect(received).toEqual([]) // Still before the boundary — no failure boundary came either.

    ws1.fireText({ ReplayComplete: { agent_id: AGENT, epoch: E } })
    expect(received).toEqual([0]) // The success marker survived and was flushed.
    c.close()
  })
})
