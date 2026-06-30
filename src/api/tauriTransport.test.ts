// TauriTransport 단위테스트 — Tauri carrier 의 reply(③-a)·output(③-b)·리스너 재등록(MED-1)·
// 연결 생명주기 가드(Fix-C ①)·출력 Channel 재등록(Fix-C ④) 평면(T7c Fix-B/Fix-C).
//
// invoke / listen / Channel 을 mock 해 WsTransport.test 와 동형으로 carrier 책임만 검증한다:
//   - send → forward_daemon_command invoke 반환(reply AgentEvent) → control InboundMessage 로 올림.
//   - 출력 Channel.onmessage(raw frame bytes) → decodeOutputFrame → output InboundMessage 로 올림.
//   - close()→재연결 시 control 리스너가 재등록되는지(MED-1).
//   - ★상태는 Rust emit(daemon-connection-state) 단일 진실원★ — doConnect 가 임의 connected 안 함(Fix-C ①).
//   - ★close 세대 가드★ — in-flight doConnect 가 close 후 뒤늦게 완료돼도 출력 Channel 재등록·부활 안 함.
//   - ★재연결 출력 Channel 재등록★ — reconnecting→connected emit 시 registerOutputChannel 재호출(Fix-C ④).
// 프로토콜 의미론(pending 매칭/dedup/epoch)은 protocolClient.test 가 본다.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ★vi.hoisted★: vi.mock factory 는 파일 최상단으로 호이스팅되므로 일반 top-level 변수를 참조할 수
//   없다. mock 이 공유하는 상태(listeners/captured channel/forward reply)와 FakeChannel 을 hoisted
//   블록에 두어 factory 와 테스트 본문이 같은 인스턴스를 본다.
const h = vi.hoisted(() => {
  type Listener = (e: { payload: unknown }) => void
  const listeners = new Map<string, Set<Listener>>()
  const state = {
    listenCalls: 0,
    capturedChannel: null as { onmessage: ((m: ArrayBuffer) => void) | null } | null,
    subscribeOutputCalls: 0, // subscribe_output invoke 횟수(Channel 등록·재등록 카운트).
    forwardReply: null as unknown, // forward_daemon_command 가 돌려줄 값(reply AgentEvent 또는 null).
    forwardShouldReject: false, // true 면 forward_daemon_command 가 reject(연결 끊김 흉내).
    // ★Rust emit 흉내★: daemon_connect/daemon_ensure invoke 가 resolve 되기 직전, Rust 가 connected
    //   를 emit 하는 동작을 흉내낼지. 기본 true(정상 connect = Rust 가 connected emit). false 면 emit
    //   없이 invoke 만 resolve(상태가 down 에 머무는지 = 단일 진실원 검증).
    emitConnectedOnConnect: true,
    // daemon_connect invoke 를 무한 대기시키는 게이트(close 세대 가드 테스트용). resolve 함수를 보관.
    connectGate: null as null | (() => void),
  }
  class FakeChannel {
    onmessage: ((m: ArrayBuffer) => void) | null = null
  }
  return { listeners, state, FakeChannel }
})

function emit(name: string, payload: unknown): void {
  for (const cb of h.listeners.get(name) ?? []) cb({ payload })
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
    // ★게이트★: connectGate 가 있으면 그 resolve 가 불릴 때까지 대기(close 세대 가드 테스트).
    if (h.state.connectGate) {
      await new Promise<void>((resolve) => {
        h.state.connectGate = resolve
      })
    }
    // ★Rust emit 흉내★: 리스너는 invoke 전에 등록됐으므로(doConnect 순서), 여기서 emit 하면
    //   u5 가 받아 setState('connected') 한다 — Rust 가 connect Ok 직전 connected 를 emit 하는 동형.
    if (h.state.emitConnectedOnConnect) emit('daemon-connection-state', 'connected')
    return undefined
  }
  if (cmd === 'daemon_close') return undefined
  if (cmd === 'subscribe_output') {
    h.state.subscribeOutputCalls += 1
    h.state.capturedChannel = args?.channel as { onmessage: ((m: ArrayBuffer) => void) | null }
    return undefined
  }
  if (cmd === 'forward_daemon_command') {
    if (h.state.forwardShouldReject) throw new Error('연결 끊김')
    return h.state.forwardReply
  }
  throw new Error('unexpected invoke: ' + cmd)
})

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: Record<string, unknown>) => invokeMock(cmd, args),
  Channel: h.FakeChannel,
}))

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

beforeEach(() => {
  h.listeners.clear()
  h.state.listenCalls = 0
  h.state.capturedChannel = null
  h.state.subscribeOutputCalls = 0
  h.state.forwardReply = null
  h.state.forwardShouldReject = false
  h.state.emitConnectedOnConnect = true
  h.state.connectGate = null
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
    // 상태는 Rust daemon-connection-state emit 으로 connected 가 된다(단일 진실원).
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
    h.state.emitConnectedOnConnect = false // Rust emit 없음(예: stale 폐기·재연결 중).
    await t.start()
    // doConnect 가 임의 setState('connected') 를 안 하므로 down 에 머문다 — Rust emit 이 권위.
    expect(t.connectionState).toBe('down')
    // 출력 Channel 등록도 connected 전이(u5)에 묶여 있으므로, emit 이 없으면 등록도 안 된다(단일 경로).
    expect(h.state.capturedChannel).toBeNull()
    expect(h.state.subscribeOutputCalls).toBe(0)
    // 이후 Rust 가 connected 를 emit 하면 그때 등록된다.
    emit('daemon-connection-state', 'connected')
    expect(t.connectionState).toBe('connected')
    await Promise.resolve()
    await Promise.resolve()
    expect(h.state.subscribeOutputCalls).toBe(1)
  })

  it('Rust daemon-connection-state 이벤트가 상태를 직접 바꾼다(connected/reconnecting/down)', async () => {
    const t = new TauriTransport()
    const seen: string[] = []
    t.onConnectionStateChange((s) => seen.push(s))
    await t.start() // connected emit
    emit('daemon-connection-state', 'reconnecting')
    emit('daemon-connection-state', 'down')
    expect(t.connectionState).toBe('down')
    // 초기 down(등록 즉시 통지) → connected → reconnecting → down.
    expect(seen).toEqual(['down', 'connected', 'reconnecting', 'down'])
  })
})

describe('TauriTransport close 세대 가드(Fix-C ①)', () => {
  it('close 후 뒤늦게 완료된 doConnect 는 출력 Channel 을 (재)등록하지 않는다', async () => {
    const t = new TauriTransport()
    // ★검증 핵심★: close() 가 u5 리스너를 제거하므로, 게이트 해제 후 Rust 가 connected 를 emit 해도
    //   u5 가 없어 무시된다 → 출력 Channel 등록 안 됨. emit 흉내를 켠 채로 그 경로를 본다.
    h.state.emitConnectedOnConnect = true
    // connect 를 게이트로 막아 in-flight 상태로 둔다.
    h.state.connectGate = () => {}
    const p = t.start()
    // 아직 invoke 가 게이트에 막혀 있다 — Channel 등록 전.
    expect(h.state.subscribeOutputCalls).toBe(0)
    // close → 세대 bump + cleanupListeners(u5 제거).
    t.close()
    expect(t.connectionState).toBe('down')
    // 이제 게이트 해제 → 막혀있던 doConnect 가 resolve 되며 invokeMock 이 connected 를 emit(stale).
    const release = h.state.connectGate
    h.state.connectGate = null
    release?.()
    await p.catch(() => {}) // stale doConnect 완료 대기.
    await Promise.resolve()
    await Promise.resolve()
    // ★세대 가드 + 리스너 제거★: close 후 connected emit 은 u5 부재로 무시 → 출력 Channel 미등록(좀비
    //   방지). doConnect 자신도 등록을 안 하므로(등록 권위=u5) 어느 쪽으로도 등록되지 않는다.
    expect(h.state.subscribeOutputCalls).toBe(0)
    expect(t.connectionState).toBe('down')
  })
})

describe('TauriTransport start 재진입 가드(Fix-C ①)', () => {
  it('진행 중 start 두 번은 daemon_connect 를 한 번만 invoke 한다(connectPromise 재사용)', async () => {
    const t = new TauriTransport()
    h.state.connectGate = () => {} // 첫 start 를 in-flight 로 묶는다.
    const p1 = t.start()
    const p2 = t.start() // 진행 중 → connectPromise 재사용.
    expect(p1).toBe(p2)
    const release = h.state.connectGate
    h.state.connectGate = null
    release?.()
    await Promise.all([p1, p2])
    // daemon_connect 는 정확히 1회(중복 승계 방지).
    const connectCalls = invokeMock.mock.calls.filter((c) => c[0] === 'daemon_connect')
    expect(connectCalls).toHaveLength(1)
  })
})

describe('TauriTransport 재연결 출력 Channel 재등록(Fix-C ④)', () => {
  it('reconnecting→connected 재전이 emit 시 출력 Channel 을 재등록한다', async () => {
    const t = new TauriTransport()
    await t.start() // 첫 connected — subscribe_output 1회.
    expect(h.state.subscribeOutputCalls).toBe(1)
    // Rust 내부 재연결: connected → reconnecting → connected.
    emit('daemon-connection-state', 'reconnecting')
    emit('daemon-connection-state', 'connected')
    // 재연결 후 출력 Channel 재등록(멱등) — Rust 내부 재연결과 디커플 보강.
    // registerOutputChannel 은 async(invoke await) — microtask 한 틱 양보 후 검사.
    await Promise.resolve()
    await Promise.resolve()
    expect(h.state.subscribeOutputCalls).toBe(2)
  })
})

describe('TauriTransport reply 평면(③-a, HIGH-2)', () => {
  it('send → forward_daemon_command 반환(reply AgentEvent)을 control InboundMessage 로 올린다', async () => {
    const t = new TauriTransport()
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    await t.start()
    // 데몬 reply 흉내: Ack(request_id echo). externally-tagged 형태 그대로.
    h.state.forwardReply = { Ack: { request_id: 'req-1' } }
    await t.send({ Kill: { agent_id: AGENT, request_id: 'req-1' } })
    expect(got).toContainEqual({ kind: 'control', event: { Ack: { request_id: 'req-1' } } })
  })

  it('send → reply 가 null(fire-and-forget) 이면 아무것도 안 올린다', async () => {
    const t = new TauriTransport()
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    await t.start()
    got.length = 0 // 연결 시 올라온 메시지 제거
    h.state.forwardReply = null // Resize 등 fire-and-forget 명령 → Rust 가 null 반환.
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
    // Rust fan-out 흉내: Channel 로 raw frame bytes 전달.
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
    const bad = new ArrayBuffer(5) // 헤더 길이 미만 → decode null.
    h.state.capturedChannel!.onmessage!(bad)
    expect(got.find((m) => m.kind === 'output')).toBeUndefined()
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
    // close → cleanupListeners(리스너 제거).
    t.close()
    expect(t.connectionState).toBe('down')
    // close 후 이벤트는 도달하지 않는다(리스너 제거됨).
    got.length = 0
    emit('agent-list-updated', [{ id: 'gone' }])
    expect(got).toHaveLength(0)
    // 재연결 → 리스너 재등록.
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
    // 이미 connected 면 start 즉시 resolve(doConnect 미진입) — 리스너 변화 없음.
    await t.start()
    expect(h.state.listenCalls).toBe(afterFirst)
    // listener set 당 핸들러는 정확히 1개(중복 없음).
    expect(h.listeners.get('agent-list-updated')!.size).toBe(1)
  })
})
