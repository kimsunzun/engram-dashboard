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

/** ensureReady 트리거 + 핸드셰이크 완료(onopen→Auth→Hello). */
async function connect(t: WsTransport): Promise<FakeWebSocket> {
  const p = t.ensureReady().catch(() => {})
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
