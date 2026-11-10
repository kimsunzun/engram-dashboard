// TauriTransport unit test — the plane of the Tauri carrier's reply (③-a) · output (③-b) · listener re-registration (MED-1) ·
// connection lifecycle guard (Fix-C ①) · output Channel re-registration (Fix-C ④) (T7c Fix-B/Fix-C).
//
// Protocol semantics (pending matching/dedup/epoch) are covered by protocolClient.test.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ★vi.hoisted★: the vi.mock factory is hoisted to the top of the file, so it cannot reference ordinary top-level variables. State shared by
//   the mock (listeners/captured channel/forward reply) and FakeChannel are put in the hoisted
//   block so the factory and the test bodies see the same instance.
const h = vi.hoisted(() => {
  type Listener = (e: { payload: unknown }) => void
  const listeners = new Map<string, Set<Listener>>()
  const state = {
    listenCalls: 0,
    capturedChannel: null as { onmessage: ((m: ArrayBuffer) => void) | null } | null,
    subscribeOutputCalls: 0,
    forwardReply: null as unknown,
    forwardShouldReject: false,
    // ★Imitate a Rust emit★: whether to imitate Rust emitting connected right before the daemon_connect/daemon_ensure invoke
    //   resolves. Default true (normal connect = Rust emits connected). If false, only the invoke resolves with no emit
    //   (checks that the state stays at down = single source of truth).
    emitConnectedOnConnect: true,
    // A gate that makes the daemon_connect invoke wait forever (for the close generation guard test). Holds the resolve function.
    connectGate: null as null | (() => void),
    // ★Fix-D self-heal★: the status the daemon_connection_state invoke will return (imitates the reload pull query).
    connectionStateReply: { state: 'down', band: null } as unknown,
    // ★FIX 5 gate★: hold the daemon_connection_state invoke so an event can be slipped in before the pull response.
    //   Holds the resolve function (= releases the gate). If null, resolves immediately (existing behavior).
    connectionStateGate: null as null | (() => void),
    // ★FIX 6 gate queue★: hold each subscribe_output invoke so the test controls the completion order. Each
    //   invoke pushes its resolve function into the array and waits — the test calls them in any order to release them. If the array is empty,
    //   resolves immediately (existing behavior).
    subscribeOutputGate: false,
    subscribeOutputResolvers: [] as Array<() => void>,
    // request_replay invoke count (also the counter that assigns gen — ADR-0046 F2).
    requestReplayCalls: 0,
    // ★FIX-5★: explicitly specify the gen request_replay will return (counter if null). For reproducing the beyond-safe-integer case.
    requestReplayReply: null as number | string | null,
    // This window's Tauri label (`getCurrentWindow().label`) — what the `labels` filter of the usage broadcast looks at.
    windowLabel: 'main',
    usageSnapshotReply: { socket_epoch: 0, snapshots: [] } as unknown,
  }
  class FakeChannel {
    onmessage: ((m: ArrayBuffer) => void) | null = null
  }
  return { listeners, state, FakeChannel }
})

function emit(name: string, payload: unknown): void {
  for (const cb of h.listeners.get(name) ?? []) cb({ payload })
}

/** The shell's `ConnectionStatus` payload (src-tauri binding) — what the event carries and the pull returns. */
function status(state: string, band: unknown = null): unknown {
  return { state, band }
}

// ★Yield many microtasks★: doConnect runs the chain registerListeners (6× await listen) → daemon_connect invoke →
//   the connected emit inside it → applyConnectionState → registerOutputChannel (async). To block
//   the middle with a gate and observe, several microtask ticks must be flushed so the chain advances to the gate point.
async function flush(ticks = 12): Promise<void> {
  for (let i = 0; i < ticks; i++) await Promise.resolve()
}

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, cb: (e: { payload: unknown }) => void) => {
    h.state.listenCalls += 1
    let set = h.listeners.get(name)
    if (!set) {
      set = new Set()
      h.listeners.set(name, set)
    }
    set.add(cb)
    return () => set!.delete(cb)
  }),
}))

const invokeMock = vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
  if (cmd === 'daemon_connect' || cmd === 'daemon_ensure') {
    if (h.state.connectGate) {
      await new Promise<void>((resolve) => {
        h.state.connectGate = resolve
      })
    }
    // ★Imitate a Rust emit★: the listeners were registered before the invoke (doConnect order), so emitting here
    //   makes u5 receive it and setState('connected') — isomorphic to Rust emitting connected right before connect Ok.
    if (h.state.emitConnectedOnConnect) emit('daemon-connection-state', status('connected'))
    return undefined
  }
  if (cmd === 'daemon_close') return undefined
  if (cmd === 'daemon_connection_state') {
    if (h.state.connectionStateGate) {
      await new Promise<void>((resolve) => {
        h.state.connectionStateGate = resolve
      })
    }
    return h.state.connectionStateReply
  }
  if (cmd === 'subscribe_output') {
    h.state.subscribeOutputCalls += 1
    h.state.capturedChannel = args?.channel as { onmessage: ((m: ArrayBuffer) => void) | null }
    if (h.state.subscribeOutputGate) {
      await new Promise<void>((resolve) => {
        h.state.subscribeOutputResolvers.push(resolve)
      })
    }
    return undefined
  }
  if (cmd === 'forward_daemon_command') {
    if (h.state.forwardShouldReject) throw new Error('연결 끊김')
    return h.state.forwardReply
  }
  if (cmd === 'get_usage_snapshot') return h.state.usageSnapshotReply
  if (cmd === 'request_replay') {
    // ADR-0046: the src-tauri single-flight assigns and returns gen (u64). The mock imitates it with a monotonic counter.
    h.state.requestReplayCalls += 1
    return h.state.requestReplayReply ?? h.state.requestReplayCalls
  }
  throw new Error('unexpected invoke: ' + cmd)
})

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: Record<string, unknown>) => invokeMock(cmd, args),
  Channel: h.FakeChannel,
}))

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ label: h.state.windowLabel }),
}))

import type { UsageLimitSnapshot } from '../../crates/engram-dashboard-protocol/bindings/UsageLimitSnapshot'
import { ProtocolClient } from './protocolClient'
import { TauriTransport } from './tauriTransport'
import type { InboundMessage } from './transport'

const AGENT = '12345678-9abc-def0-1234-56789abcdef0'

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
  view.setUint8(0, 0) // tag = TERMINAL_BYTES
  const idBytes = uuidToBytes(opts.agentId)
  for (let i = 0; i < 16; i++) view.setUint8(1 + i, idBytes[i])
  view.setUint32(17, opts.epoch, false)
  view.setBigUint64(21, BigInt(opts.seq), false)
  new Uint8Array(buf, FRAME_HEADER_LEN).set(payload)
  return buf
}

// ADR-0046 replay boundary marker frame: [tag=255][agentId:16][epoch:4 BE][gen:8 BE][flags:1][replay_from:8 BE].
const MARKER_LEN = 38
function buildMarker(opts: {
  agentId: string
  epoch: number
  gen: bigint
  truncated?: boolean
  failed?: boolean
  continuesConversation?: boolean
  replayFrom?: bigint
}): ArrayBuffer {
  const buf = new ArrayBuffer(MARKER_LEN)
  const view = new DataView(buf)
  view.setUint8(0, 255)
  const idBytes = uuidToBytes(opts.agentId)
  for (let i = 0; i < 16; i++) view.setUint8(1 + i, idBytes[i])
  view.setUint32(17, opts.epoch, false)
  view.setBigUint64(21, opts.gen, false)
  let flags = 0
  if (opts.truncated) flags |= 0x01
  if (opts.failed) flags |= 0x02
  if (opts.continuesConversation) flags |= 0x04
  view.setUint8(29, flags)
  view.setBigUint64(30, opts.replayFrom ?? 0n, false)
  return buf
}

beforeEach(() => {
  h.listeners.clear()
  h.state.listenCalls = 0
  h.state.capturedChannel = null
  h.state.subscribeOutputCalls = 0
  h.state.forwardReply = null
  h.state.forwardShouldReject = false
  h.state.emitConnectedOnConnect = true
  h.state.connectGate = null
  h.state.connectionStateReply = status('down')
  h.state.connectionStateGate = null
  h.state.subscribeOutputGate = false
  h.state.subscribeOutputResolvers = []
  h.state.requestReplayCalls = 0
  h.state.requestReplayReply = null
  h.state.windowLabel = 'main'
  h.state.usageSnapshotReply = { socket_epoch: 0, snapshots: [] }
  invokeMock.mockClear()
})
afterEach(() => {
  vi.restoreAllMocks()
})

describe('TauriTransport 연결', () => {
  it('start → daemon_connect invoke + (Rust emit 으로) connected 전이 + 출력 Channel 등록', async () => {
    const t = new TauriTransport()
    expect(t.connectionState).toBe('down')
    await t.start()
    expect(invokeMock).toHaveBeenCalledWith('daemon_connect', undefined)
    expect(invokeMock).toHaveBeenCalledWith('subscribe_output', expect.objectContaining({}))
    expect(t.connectionState).toBe('connected')
    expect(h.state.capturedChannel).not.toBeNull()
  })

  it('ensureReady → daemon_ensure(no-spawn) invoke', async () => {
    const t = new TauriTransport()
    await t.ensureReady()
    expect(invokeMock).toHaveBeenCalledWith('daemon_ensure', undefined)
  })
})

describe('TauriTransport 상태 단일 진실원(Fix-C ①)', () => {
  it('Rust 가 connected 를 emit 하지 않으면 invoke resolve 만으로 connected 가 되지 않는다', async () => {
    const t = new TauriTransport()
    h.state.emitConnectedOnConnect = false // No Rust emit (e.g. stale discarded · mid-reconnect).
    await t.start()
    expect(t.connectionState).toBe('down')
    expect(h.state.capturedChannel).toBeNull()
    expect(h.state.subscribeOutputCalls).toBe(0)
    emit('daemon-connection-state', status('connected'))
    expect(t.connectionState).toBe('connected')
    await Promise.resolve()
    await Promise.resolve()
    expect(h.state.subscribeOutputCalls).toBe(1)
  })

  it('Rust daemon-connection-state 이벤트가 상태를 직접 바꾼다(connected/reconnecting/down)', async () => {
    const t = new TauriTransport()
    const seen: string[] = []
    t.onConnectionStateChange((s) => seen.push(s))
    await t.start()
    emit('daemon-connection-state', status('reconnecting'))
    emit('daemon-connection-state', status('down'))
    expect(t.connectionState).toBe('down')
    expect(seen).toEqual(['down', 'connected', 'reconnecting', 'down'])
  })
})

describe('TauriTransport close 세대 가드(Fix-C ①)', () => {
  it('close 후 뒤늦게 완료된 doConnect 는 출력 Channel 을 (재)등록하지 않는다', async () => {
    const t = new TauriTransport()
    // ★Key check★: close() removes the u5 listener, so even if Rust emits connected after the gate is released,
    //   there is no u5 and it is ignored → the output Channel is not registered. Look at that path with the emit imitation on.
    h.state.emitConnectedOnConnect = true
    h.state.connectGate = () => {}
    const p = t.start()
    expect(h.state.subscribeOutputCalls).toBe(0)
    // close → generation bump + cleanupListeners (u5 removed).
    t.close()
    expect(t.connectionState).toBe('down')
    const release = h.state.connectGate
    h.state.connectGate = null
    release?.()
    await p.catch(() => {})
    await Promise.resolve()
    await Promise.resolve()
    expect(h.state.subscribeOutputCalls).toBe(0)
    expect(t.connectionState).toBe('down')
  })
})

describe('TauriTransport start 재진입 가드(Fix-C ①)', () => {
  it('진행 중 start 두 번은 daemon_connect 를 한 번만 invoke 한다(connectPromise 재사용)', async () => {
    const t = new TauriTransport()
    h.state.connectGate = () => {}
    const p1 = t.start()
    const p2 = t.start()
    expect(p1).toBe(p2)
    const release = h.state.connectGate
    h.state.connectGate = null
    release?.()
    await Promise.all([p1, p2])
    const connectCalls = invokeMock.mock.calls.filter((c) => c[0] === 'daemon_connect')
    expect(connectCalls).toHaveLength(1)
  })
})

describe('TauriTransport 재연결 출력 Channel 재등록(Fix-C ④)', () => {
  it('reconnecting→connected 재전이 emit 시 출력 Channel 을 재등록한다', async () => {
    const t = new TauriTransport()
    await t.start()
    expect(h.state.subscribeOutputCalls).toBe(1)
    emit('daemon-connection-state', status('reconnecting'))
    emit('daemon-connection-state', status('connected'))
    // registerOutputChannel is async (invoke await) — check after yielding one microtask tick.
    await Promise.resolve()
    await Promise.resolve()
    expect(h.state.subscribeOutputCalls).toBe(2)
  })
})

describe('TauriTransport 리로드 self-heal(Fix-D)', () => {
  it('init → 조회가 connected 면 출력 Channel 을 1회 등록한다(전이 이벤트 없이도 복구)', async () => {
    const t = new TauriTransport()
    // Reload imitation: the daemon is already Connected, so no transition event comes. Only the pull query reports connected.
    h.state.connectionStateReply = status('connected')
    await t.init()
    expect(invokeMock).toHaveBeenCalledWith('daemon_connection_state', undefined)
    expect(t.connectionState).toBe('connected')
    expect(h.state.subscribeOutputCalls).toBe(1)
    expect(h.state.capturedChannel).not.toBeNull()
  })

  it('init → 조회가 down 이면 아무 것도 하지 않는다(연결 안 됨은 정상)', async () => {
    const t = new TauriTransport()
    h.state.connectionStateReply = status('down')
    await t.init()
    expect(t.connectionState).toBe('down')
    expect(h.state.subscribeOutputCalls).toBe(0)
  })

  it('조회로 connected 등록 후 다시 connected 이벤트가 와도 이중 등록하지 않는다(멱등)', async () => {
    const t = new TauriTransport()
    h.state.connectionStateReply = status('connected')
    await t.init()
    expect(t.connectionState).toBe('connected')
    expect(h.state.subscribeOutputCalls).toBe(1)
    emit('daemon-connection-state', status('connected'))
    await Promise.resolve()
    await Promise.resolve()
    expect(h.state.subscribeOutputCalls).toBe(1)
  })

  it('조회가 connected 를 반환해도 직후 down 이벤트가 오면 이벤트가 이긴다(last-write)', async () => {
    const t = new TauriTransport()
    h.state.connectionStateReply = status('connected')
    await t.init()
    expect(t.connectionState).toBe('connected')
    emit('daemon-connection-state', status('down'))
    expect(t.connectionState).toBe('down')
  })

  // ── FIX 5: if an event slips in *before* the pull response, discard the pull (stale) (guards against event→pull order reversal) ──────
  it('조회 대기 중 down 이벤트가 끼면, 뒤늦게 온 connected 조회 결과를 폐기한다(이벤트 승)', async () => {
    const t = new TauriTransport()
    // Make the pull return a stale 'connected' but delay the response with the gate (placeholder → replaced by the real resolver).
    h.state.connectionStateReply = status('connected')
    h.state.connectionStateGate = () => {}
    const initP = t.init()
    // init goes registerListeners → selfHeal (blocked on the invoke gate) in that order. Wait for listener registration + the pull invoke to enter.
    await flush()
    emit('daemon-connection-state', status('down'))
    expect(t.connectionState).toBe('down')
    const release = h.state.connectionStateGate
    h.state.connectionStateGate = null
    release?.()
    await initP
    await flush()
    expect(t.connectionState).toBe('down')
    expect(h.state.subscribeOutputCalls).toBe(0)
  })

  it('조회 대기 중 close() 가 끼면 뒤늦은 connected 조회로 되살아나거나 Channel 을 붙이지 않는다', async () => {
    const t = new TauriTransport()
    h.state.connectionStateReply = status('connected')
    h.state.connectionStateGate = () => {}
    const initP = t.init()
    await flush()
    t.close()
    const release = h.state.connectionStateGate
    h.state.connectionStateGate = null
    release?.()
    await initP
    await flush()
    expect(t.connectionState).toBe('down')
    expect(h.state.subscribeOutputCalls).toBe(0)
  })
})

// ── FIX 7: guard against an unknown state vocabulary (retrofit trap) ───────────────────────────────────────────
describe('TauriTransport 미지 연결 상태 어휘(FIX 7)', () => {
  it('알 수 없는 상태 문자열은 down 으로 처리 + console.warn 1회', async () => {
    const t = new TauriTransport()
    await t.start() // connected.
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    // If Rust later emits a new vocabulary item ('connecting'), do not silently mistranslate — degrade to the safe side (down) + warn.
    emit('daemon-connection-state', status('connecting'))
    expect(t.connectionState).toBe('down')
    const unknownWarns = warnSpy.mock.calls.filter(
      (c) => typeof c[0] === 'string' && c[0].includes('unknown connection state'),
    )
    expect(unknownWarns.length).toBe(1)
    warnSpy.mockRestore()
  })
})

// ── Connection bands (TRD S21 trd-3-2-u5 §4-2): the shell sends `{state, band}` on the event and the pull ──────────
describe('TauriTransport connection status DTO', () => {
  it('reads the band from the event and from the pull', async () => {
    const t = new TauriTransport()
    h.state.connectionStateReply = status('down', { kind: 'version', can_stop: true })
    await t.init()
    expect(t.connectionState).toBe('down')
    expect(t.connectionBand).toEqual({ kind: 'version', can_stop: true })
    emit('daemon-connection-state', status('reconnecting', { kind: 'not_connected', refused: false }))
    expect(t.connectionState).toBe('reconnecting')
    expect(t.connectionBand).toEqual({ kind: 'not_connected', refused: false })
    emit('daemon-connection-state', status('down', { kind: 'not_connected', refused: true }))
    expect(t.connectionBand).toEqual({ kind: 'not_connected', refused: true })
    emit('daemon-connection-state', status('connected'))
    expect(t.connectionState).toBe('connected')
    expect(t.connectionBand).toBeNull()
  })

  it('notifies listeners when only the band changes, and not when nothing changed', async () => {
    const t = new TauriTransport()
    await t.start()
    emit('daemon-connection-state', status('reconnecting'))
    const seen: string[] = []
    t.onConnectionStateChange((s) => seen.push(s))
    expect(seen).toEqual(['reconnecting'])
    emit('daemon-connection-state', status('reconnecting', { kind: 'not_connected', refused: false }))
    expect(seen).toEqual(['reconnecting', 'reconnecting'])
    emit('daemon-connection-state', status('reconnecting', { kind: 'not_connected', refused: false }))
    expect(seen).toEqual(['reconnecting', 'reconnecting'])
    emit('daemon-connection-state', status('reconnecting', { kind: 'not_connected', refused: true }))
    expect(seen).toEqual(['reconnecting', 'reconnecting', 'reconnecting'])
  })

  it('reads an unknown band kind as band ① without a cause, with a warning', async () => {
    const t = new TauriTransport()
    await t.start()
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    emit('daemon-connection-state', status('down', { kind: 'identity', refused: true }))
    expect(t.connectionState).toBe('down')
    expect(t.connectionBand).toEqual({ kind: 'not_connected', refused: false })
    expect(
      warnSpy.mock.calls.filter((c) => typeof c[0] === 'string' && c[0].includes('unknown connection band')),
    ).toHaveLength(1)
    warnSpy.mockRestore()
  })

  it('reads a payload that is not a status as down without a band', async () => {
    const t = new TauriTransport()
    await t.start()
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    emit('daemon-connection-state', 'connected')
    expect(t.connectionState).toBe('down')
    expect(t.connectionBand).toBeNull()
    warnSpy.mockRestore()
  })

  it('an explicit close shows no band', async () => {
    const t = new TauriTransport()
    await t.start()
    emit('daemon-connection-state', status('down', { kind: 'not_connected', refused: false }))
    const seen: string[] = []
    t.onConnectionStateChange((s) => seen.push(s))
    t.close()
    expect(t.connectionBand).toBeNull()
    expect(seen).toEqual(['down', 'down'])
  })
})

// ── FIX 6: output Channel registration single-flight (guards against Rust arrival order reversal on overlapping registrations) ──────────
describe('TauriTransport 출력 Channel 등록 single-flight(FIX 6)', () => {
  it('진행 중 등록에 겹쳐 온 재등록은 동시 invoke 를 만들지 않고 완료 후 1회만 재실행한다', async () => {
    const t = new TauriTransport()
    h.state.subscribeOutputGate = true
    const startP = t.start()
    await flush()
    expect(h.state.subscribeOutputCalls).toBe(1)
    expect(h.state.subscribeOutputResolvers.length).toBe(1)
    emit('daemon-connection-state', status('reconnecting'))
    emit('daemon-connection-state', status('connected'))
    await flush()
    expect(h.state.subscribeOutputCalls).toBe(1)
    h.state.subscribeOutputResolvers.shift()!()
    await startP
    await flush()
    expect(h.state.subscribeOutputCalls).toBe(2)
    expect(h.state.subscribeOutputResolvers.length).toBe(1)
    h.state.subscribeOutputResolvers.shift()!()
    await flush()
    expect(h.state.subscribeOutputCalls).toBe(2)
  })

  it('close 는 진행 중 등록의 완료 후 재등록을 취소한다(좀비 Channel 방지)', async () => {
    const t = new TauriTransport()
    h.state.subscribeOutputGate = true
    const startP = t.start()
    await flush()
    expect(h.state.subscribeOutputCalls).toBe(1)
    emit('daemon-connection-state', status('reconnecting'))
    emit('daemon-connection-state', status('connected'))
    await flush()
    t.close()
    h.state.subscribeOutputResolvers.shift()!()
    await startP.catch(() => {})
    await flush()
    expect(h.state.subscribeOutputCalls).toBe(1)
  })
})

describe('TauriTransport reply 평면(③-a, HIGH-2)', () => {
  it('send → forward_daemon_command 반환(reply AgentEvent)을 control InboundMessage 로 올린다', async () => {
    const t = new TauriTransport()
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    await t.start()
    // Daemon reply imitation: Ack (request_id echo). Exactly the externally-tagged shape.
    h.state.forwardReply = { Ack: { request_id: 'req-1' } }
    await t.send({ Kill: { agent_id: AGENT, request_id: 'req-1' } })
    expect(got).toContainEqual({ kind: 'control', event: { Ack: { request_id: 'req-1' } } })
  })

  it('send → reply 가 null(fire-and-forget) 이면 아무것도 안 올린다', async () => {
    const t = new TauriTransport()
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    await t.start()
    got.length = 0 // Remove the messages that came up at connect time
    h.state.forwardReply = null // Fire-and-forget commands like Resize → Rust returns null.
    await t.send({ Resize: { agent_id: AGENT, cols: 80, rows: 24, viewport_id: null } })
    expect(got).toHaveLength(0)
  })

  it('send → invoke reject(연결 끊김/reply 타임아웃) 이면 send Promise 가 reject 된다(pending hang 차단)', async () => {
    const t = new TauriTransport()
    await t.start()
    h.state.forwardShouldReject = true
    const r = t.send({ Kill: { agent_id: AGENT, request_id: 'req-2' } })
    await expect(r as Promise<void>).rejects.toThrow('연결 끊김')
  })
})

describe('TauriTransport output 평면(③-b, HIGH-1)', () => {
  it('출력 Channel.onmessage(raw frame) → decodeOutputFrame → output InboundMessage', async () => {
    const t = new TauriTransport()
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    await t.start()
    h.state.capturedChannel!.onmessage!(buildFrame({ agentId: AGENT, epoch: 3, seq: 9, payload: new Uint8Array([0x41]) }))
    const out = got.find((m) => m.kind === 'output')
    expect(out).toMatchObject({ kind: 'output', agentId: AGENT, epoch: 3, seq: 9 })
    expect(Array.from((out as { bytes: Uint8Array }).bytes)).toEqual([0x41])
  })

  it('미지원 tag frame 은 무시(decodeOutputFrame=null)', async () => {
    const t = new TauriTransport()
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    await t.start()
    const bad = new ArrayBuffer(5) // Shorter than the header length → decode null.
    h.state.capturedChannel!.onmessage!(bad)
    expect(got.find((m) => m.kind === 'output')).toBeUndefined()
  })
})

// ── ADR-0046 F2: requestReplay (invoke) + replay boundary marker normalization ──────────────────────────
describe('TauriTransport requestReplay(ADR-0046 F2)', () => {
  it('requestReplay → invoke("request_replay",{agentId}) + gen(BigInt) 반환', async () => {
    const t = new TauriTransport()
    await t.start()
    const gen = await t.requestReplay(AGENT)
    expect(invokeMock).toHaveBeenCalledWith('request_replay', { agentId: AGENT })
    // Return the u64 gen as a BigInt (§F2 — width matches the marker frame getBigUint64).
    expect(typeof gen).toBe('bigint')
    expect(gen).toBe(1n)
    expect(await t.requestReplay(AGENT)).toBe(2n)
  })

  // ── FIX-5: if the u64 gen exceeds the safe integer (number serialization), console.warn (unreachable in practice) ──────────
  it('gen 이 안전 정수 초과 number 로 오면 console.warn 1회(BigInt 변환은 유지)', async () => {
    const t = new TauriTransport()
    await t.start()
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    // A number beyond 2^53 (arrived with precision already broken) — unreachable in practice, but warn instead of a silent mis-stamp.
    h.state.requestReplayReply = Number.MAX_SAFE_INTEGER + 2
    const gen = await t.requestReplay(AGENT)
    expect(typeof gen).toBe('bigint')
    const warns = warnSpy.mock.calls.filter(
      (c) => typeof c[0] === 'string' && c[0].includes('안전 정수'),
    )
    expect(warns.length).toBe(1)
    warnSpy.mockRestore()
    t.close()
  })

  it('gen 이 안전 정수 범위 number 면 경고 없음', async () => {
    const t = new TauriTransport()
    await t.start()
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    h.state.requestReplayReply = 12345
    await t.requestReplay(AGENT)
    const warns = warnSpy.mock.calls.filter(
      (c) => typeof c[0] === 'string' && c[0].includes('안전 정수'),
    )
    expect(warns.length).toBe(0)
    warnSpy.mockRestore()
    t.close()
  })
})

// ── ADR-0231: a replay request only after this window's output Channel is registered — the shell silently skips the share of an unregistered window ──
describe('TauriTransport replay 요청 ↔ 출력 Channel 등록 순서(ADR-0231)', () => {
  const callOrder = (): string[] =>
    invokeMock.mock.calls
      .map((c) => c[0])
      .filter((cmd) => cmd === 'subscribe_output' || cmd === 'request_replay')

  it('등록이 늦게 풀려도 request_replay 는 subscribe_output 이 풀린 뒤에야 나간다', async () => {
    const t = new TauriTransport()
    h.state.subscribeOutputGate = true
    await t.start() // emit path — registration is in progress, blocked on the gate
    await flush()
    expect(h.state.subscribeOutputCalls).toBe(1)
    const replay = t.requestReplay(AGENT)
    await flush()
    expect(h.state.requestReplayCalls).toBe(0)
    h.state.subscribeOutputResolvers.shift()!()
    expect(await replay).toBe(1n)
    expect(callOrder()).toEqual(['subscribe_output', 'request_replay'])
    t.close()
  })

  // Reload · popout: the shell is already Connected, so ensure short-circuits to Ok without a transition, and the init query has not happened yet either.
  it('ensure 가 전이 없이 Ok 면 doConnect 가 조회로 연결을 받고 등록을 마친 뒤에 풀린다 — replay 는 그 뒤', async () => {
    const t = new TauriTransport()
    h.state.emitConnectedOnConnect = false
    h.state.connectionStateReply = status('connected')
    h.state.subscribeOutputGate = true
    let ready = false
    const p = t.ensureReady().then(() => {
      ready = true
    })
    await flush(30)
    expect(t.connectionState).toBe('connected')
    expect(h.state.subscribeOutputCalls).toBe(1)
    expect(ready, '등록이 풀리기 전엔 ensureReady 도 안 풀린다').toBe(false)
    const replay = t.requestReplay(AGENT)
    await flush()
    expect(h.state.requestReplayCalls).toBe(0)
    h.state.subscribeOutputResolvers.shift()!()
    await p
    await replay
    expect(callOrder()).toEqual(['subscribe_output', 'request_replay'])
    t.close()
  })

  it('한 번도 등록되지 않았으면 request_replay 앞에서 먼저 등록한다', async () => {
    const t = new TauriTransport()
    h.state.emitConnectedOnConnect = false // The query is also down — no path started a registration
    await t.start()
    expect(h.state.subscribeOutputCalls).toBe(0)
    await t.requestReplay(AGENT)
    expect(callOrder()).toEqual(['subscribe_output', 'request_replay'])
    t.close()
  })

  it('재연결 전이의 재등록이 진행 중이면 그 완료를 기다린다', async () => {
    const t = new TauriTransport()
    await t.start()
    expect(h.state.subscribeOutputCalls).toBe(1)
    h.state.subscribeOutputGate = true
    emit('daemon-connection-state', status('reconnecting'))
    emit('daemon-connection-state', status('connected'))
    await flush()
    expect(h.state.subscribeOutputCalls).toBe(2)
    const replay = t.requestReplay(AGENT)
    await flush()
    expect(h.state.requestReplayCalls).toBe(0)
    h.state.subscribeOutputResolvers.shift()!()
    await replay
    expect(callOrder()).toEqual(['subscribe_output', 'subscribe_output', 'request_replay'])
    t.close()
  })

  it('닫힌 뒤의 replay 요청은 Channel 을 새로 붙이지 않고 reject 된다', async () => {
    const t = new TauriTransport()
    await t.start()
    t.close()
    await expect(t.requestReplay(AGENT)).rejects.toThrow()
    expect(h.state.subscribeOutputCalls).toBe(1)
    expect(h.state.requestReplayCalls).toBe(0)
  })
})

describe('TauriTransport replay 경계 마커(tag=255 → replayBoundary)', () => {
  it('출력 Channel 로 온 마커 frame 은 replayBoundary 로 정규화(output 아님)', async () => {
    const t = new TauriTransport()
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    await t.start()
    h.state.capturedChannel!.onmessage!(
      buildMarker({ agentId: AGENT, epoch: 7, gen: 42n, truncated: true, failed: false }),
    )
    const marker = got.find((m) => m.kind === 'replayBoundary')
    expect(marker).toMatchObject({
      kind: 'replayBoundary',
      agentId: AGENT,
      epoch: 7,
      gen: 42n,
      truncated: true,
      failed: false,
      continuesConversation: false,
      replayFrom: 0,
    })
    // The marker does not come up as output (not exposed on the public surface — Designer requirement).
    expect(got.find((m) => m.kind === 'output')).toBeUndefined()
  })

  // ADR-0226: if the resume incarnation marker (bit2) drops out of normalization, the slot always falls back to the first screen (a silent failure).
  it('이어받기 화신 표식(bit2)이 경계에 실린다', async () => {
    const t = new TauriTransport()
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    await t.start()
    h.state.capturedChannel!.onmessage!(
      buildMarker({ agentId: AGENT, epoch: 7, gen: 43n, continuesConversation: true }),
    )
    expect(got.find((m) => m.kind === 'replayBoundary')).toMatchObject({
      gen: 43n,
      failed: false,
      continuesConversation: true,
    })
  })

  // ADR-0231: the view's flush start point is this head — if it drops out of normalization, an empty replay skips seq 0.
  it('replay 머리(replay_from)가 경계에 실린다', async () => {
    const t = new TauriTransport()
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    await t.start()
    h.state.capturedChannel!.onmessage!(
      buildMarker({ agentId: AGENT, epoch: 7, gen: 44n, replayFrom: 40n }),
    )
    expect(got.find((m) => m.kind === 'replayBoundary')).toMatchObject({ gen: 44n, replayFrom: 40 })
  })

  it('failed 플래그 전파', async () => {
    const t = new TauriTransport()
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    await t.start()
    h.state.capturedChannel!.onmessage!(buildMarker({ agentId: AGENT, epoch: 1, gen: 9n, failed: true }))
    expect(got.find((m) => m.kind === 'replayBoundary')).toMatchObject({ failed: true, gen: 9n })
  })
})

describe('TauriTransport control broadcast', () => {
  it('agent-list-updated listen → control InboundMessage(AgentListUpdated)', async () => {
    const t = new TauriTransport()
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    await t.start()
    emit('agent-list-updated', [{ id: AGENT }])
    expect(got).toContainEqual({
      kind: 'control',
      event: { AgentListUpdated: { agents: [{ id: AGENT }] } },
    })
  })

  it('status-changed listen → control InboundMessage(StatusChanged, snake_case 매핑)', async () => {
    const t = new TauriTransport()
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    await t.start()
    emit('status-changed', { agentId: AGENT, status: 'Running', epoch: 2 })
    expect(got).toContainEqual({
      kind: 'control',
      event: { StatusChanged: { agent_id: AGENT, status: 'Running', epoch: 2 } },
    })
  })
})

describe('TauriTransport 리스너 재등록(MED-1)', () => {
  it('close 후 재연결하면 control 리스너가 재등록돼 이벤트가 다시 도달한다', async () => {
    const t = new TauriTransport()
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    await t.start()
    t.close()
    expect(t.connectionState).toBe('down')
    got.length = 0
    emit('agent-list-updated', [{ id: 'gone' }])
    expect(got).toHaveLength(0)
    await t.start()
    emit('agent-list-updated', [{ id: AGENT }])
    expect(got).toContainEqual({
      kind: 'control',
      event: { AgentListUpdated: { agents: [{ id: AGENT }] } },
    })
  })

  it('연속 연결은 리스너를 중복 등록하지 않는다(멱등)', async () => {
    const t = new TauriTransport()
    t.onMessage(() => {})
    await t.start()
    const afterFirst = h.state.listenCalls
    // If already connected, start resolves immediately (doConnect not entered) — no listener change.
    await t.start()
    expect(h.state.listenCalls).toBe(afterFirst)
    expect(h.listeners.get('agent-list-updated')!.size).toBe(1)
  })
})

// ── Usage (TRD S21 usage-limit-slot §1-8) — broadcast receipt (labels filter) · shell cache pull ────────────────
describe('TauriTransport 사용량', () => {
  const snap = (vendor: 'claude' | 'codex', revision: number) =>
    ({
      vendor,
      account_key: 'default',
      five_hour: { used_pct: 40, resets_at: 1_900_000_000, age_secs: 3, expired: false },
      weekly: null,
      model_scoped: [],
      plan: 'max',
      in_flight: false,
      state: { kind: 'Ready' },
      revision,
    }) as UsageLimitSnapshot

  it('자기 label 이 든 방송만 control UsageLimitsUpdated{snapshot, socketEpoch} 로 올린다', async () => {
    h.state.windowLabel = 'popup-3'
    const t = new TauriTransport()
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    await t.init()
    emit('usage-limits-updated', { labels: ['main'], socket_epoch: 2, snapshot: snap('claude', 1) })
    expect(got).toEqual([])
    emit('usage-limits-updated', { labels: ['main', 'popup-3'], socket_epoch: 2, snapshot: snap('codex', 4) })
    expect(got).toEqual([
      { kind: 'control', event: { UsageLimitsUpdated: { snapshot: snap('codex', 4), socketEpoch: 2 } } },
    ])
  })

  it('실 ProtocolClient 를 거쳐 onUsageLimitsUpdated 까지 — 남의 창 방송은 안 닿는다', async () => {
    const t = new TauriTransport()
    const c = new ProtocolClient(t)
    const seen: Array<[number, number]> = []
    c.onUsageLimitsUpdated((s, e) => seen.push([s.revision, e]))
    await t.init()
    emit('usage-limits-updated', { labels: ['popup-9'], socket_epoch: 9, snapshot: snap('claude', 5) })
    emit('usage-limits-updated', { labels: ['main'], socket_epoch: 9, snapshot: snap('claude', 6) })
    expect(seen).toEqual([[6, 9]])
  })

  it('getUsageSnapshot → invoke(get_usage_snapshot) 의 snake_case 를 camelCase 로', async () => {
    h.state.usageSnapshotReply = { socket_epoch: 11, snapshots: [snap('claude', 2), snap('codex', 8)] }
    const t = new TauriTransport()
    await expect(t.getUsageSnapshot()).resolves.toEqual({
      socketEpoch: 11,
      snapshots: [snap('claude', 2), snap('codex', 8)],
    })
    expect(invokeMock).toHaveBeenCalledWith('get_usage_snapshot', undefined)
  })

  it('⟳ = forward_daemon_command 의 RefreshUsageLimits 한 장 · 답 Ack 로 풀린다 — 사용량 경로의 셸 커맨드는 그것과 get_usage_snapshot 뿐(관심 보고 없음)', async () => {
    vi.spyOn(globalThis.crypto, 'randomUUID').mockReturnValue('req-u1-0000-0000-0000-000000000000')
    const t = new TauriTransport()
    const c = new ProtocolClient(t)
    await t.start()
    await flush() // Finish through the output Channel registration (subscribe_output) that the connected transition started.
    invokeMock.mockClear()
    h.state.forwardReply = { Ack: { request_id: 'req-u1-0000-0000-0000-000000000000' } }
    await expect(c.refreshUsageLimits('claude')).resolves.toBeUndefined()
    await c.getUsageSnapshot()
    expect(invokeMock.mock.calls).toEqual([
      [
        'forward_daemon_command',
        { cmd: { RefreshUsageLimits: { vendor: 'claude', request_id: 'req-u1-0000-0000-0000-000000000000' } } },
      ],
      ['get_usage_snapshot', undefined],
    ])
  })
})
