// TauriTransport 단위테스트 — Tauri carrier 의 reply(③-a)·output(③-b)·리스너 재등록(MED-1)·
// 연결 생명주기 가드(Fix-C ①)·출력 Channel 재등록(Fix-C ④) 평면(T7c Fix-B/Fix-C).
//
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
    subscribeOutputCalls: 0,
    forwardReply: null as unknown,
    forwardShouldReject: false,
    // ★Rust emit 흉내★: daemon_connect/daemon_ensure invoke 가 resolve 되기 직전, Rust 가 connected
    //   를 emit 하는 동작을 흉내낼지. 기본 true(정상 connect = Rust 가 connected emit). false 면 emit
    //   없이 invoke 만 resolve(상태가 down 에 머무는지 = 단일 진실원 검증).
    emitConnectedOnConnect: true,
    // daemon_connect invoke 를 무한 대기시키는 게이트(close 세대 가드 테스트용). resolve 함수를 보관.
    connectGate: null as null | (() => void),
    // ★Fix-D self-heal★: daemon_connection_state invoke 가 돌려줄 상태 문자열(리로드 pull 조회 흉내).
    connectionStateReply: 'down',
    // ★FIX 5 게이트★: daemon_connection_state invoke 를 대기시켜, pull 응답 전에 이벤트를 끼워넣게 한다.
    //   resolve 함수(= 게이트 해제)를 보관. null 이면 즉시 resolve(기존 동작).
    connectionStateGate: null as null | (() => void),
    // ★FIX 6 게이트 큐★: subscribe_output invoke 각각을 대기시켜 완료 순서를 테스트가 통제한다. 각
    //   invoke 는 배열에 resolve 함수를 push 하고 대기 — 테스트가 임의 순서로 호출해 풀어준다. 빈 배열이면
    //   즉시 resolve(기존 동작).
    subscribeOutputGate: false,
    subscribeOutputResolvers: [] as Array<() => void>,
    // request_replay invoke 횟수(gen 부여 카운터 겸 — ADR-0046 F2).
    requestReplayCalls: 0,
    // ★FIX-5★: request_replay 가 돌려줄 gen 을 명시 지정(null 이면 카운터). 안전 정수 초과 케이스 재현용.
    requestReplayReply: null as number | string | null,
  }
  class FakeChannel {
    onmessage: ((m: ArrayBuffer) => void) | null = null
  }
  return { listeners, state, FakeChannel }
})

function emit(name: string, payload: unknown): void {
  for (const cb of h.listeners.get(name) ?? []) cb({ payload })
}

// ★microtask 다수 양보★: doConnect 는 registerListeners(6× await listen) → daemon_connect invoke →
//   그 안의 connected emit → applyConnectionState → registerOutputChannel(비동기) 체인을 탄다. 게이트로
//   중간을 막고 관측하려면 여러 microtask 틱을 흘려야 그 체인이 게이트 지점까지 진행한다.
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
    // ★Rust emit 흉내★: 리스너는 invoke 전에 등록됐으므로(doConnect 순서), 여기서 emit 하면
    //   u5 가 받아 setState('connected') 한다 — Rust 가 connect Ok 직전 connected 를 emit 하는 동형.
    if (h.state.emitConnectedOnConnect) emit('daemon-connection-state', 'connected')
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
  if (cmd === 'request_replay') {
    // ADR-0046: src-tauri single-flight 가 gen(u64)을 부여 반환. mock 은 단조 카운터로 흉내.
    h.state.requestReplayCalls += 1
    return h.state.requestReplayReply ?? h.state.requestReplayCalls
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

// ADR-0046 replay 경계 마커 frame: [tag=255][agentId:16][epoch:4 BE][gen:8 BE][flags:1].
const MARKER_LEN = 30
function buildMarker(opts: {
  agentId: string
  epoch: number
  gen: bigint
  truncated?: boolean
  failed?: boolean
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
  view.setUint8(29, flags)
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
  h.state.connectionStateReply = 'down'
  h.state.connectionStateGate = null
  h.state.subscribeOutputGate = false
  h.state.subscribeOutputResolvers = []
  h.state.requestReplayCalls = 0
  h.state.requestReplayReply = null
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
    h.state.emitConnectedOnConnect = false // Rust emit 없음(예: stale 폐기·재연결 중).
    await t.start()
    expect(t.connectionState).toBe('down')
    expect(h.state.capturedChannel).toBeNull()
    expect(h.state.subscribeOutputCalls).toBe(0)
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
    await t.start()
    emit('daemon-connection-state', 'reconnecting')
    emit('daemon-connection-state', 'down')
    expect(t.connectionState).toBe('down')
    expect(seen).toEqual(['down', 'connected', 'reconnecting', 'down'])
  })
})

describe('TauriTransport close 세대 가드(Fix-C ①)', () => {
  it('close 후 뒤늦게 완료된 doConnect 는 출력 Channel 을 (재)등록하지 않는다', async () => {
    const t = new TauriTransport()
    // ★검증 핵심★: close() 가 u5 리스너를 제거하므로, 게이트 해제 후 Rust 가 connected 를 emit 해도
    //   u5 가 없어 무시된다 → 출력 Channel 등록 안 됨. emit 흉내를 켠 채로 그 경로를 본다.
    h.state.emitConnectedOnConnect = true
    h.state.connectGate = () => {}
    const p = t.start()
    expect(h.state.subscribeOutputCalls).toBe(0)
    // close → 세대 bump + cleanupListeners(u5 제거).
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
    emit('daemon-connection-state', 'reconnecting')
    emit('daemon-connection-state', 'connected')
    // registerOutputChannel 은 async(invoke await) — microtask 한 틱 양보 후 검사.
    await Promise.resolve()
    await Promise.resolve()
    expect(h.state.subscribeOutputCalls).toBe(2)
  })
})

describe('TauriTransport 리로드 self-heal(Fix-D)', () => {
  it('init → 조회가 connected 면 출력 Channel 을 1회 등록한다(전이 이벤트 없이도 복구)', async () => {
    const t = new TauriTransport()
    // 리로드 흉내: 데몬은 이미 Connected 라 전이 이벤트가 안 온다. pull 조회만 connected 를 알려준다.
    h.state.connectionStateReply = 'connected'
    await t.init()
    expect(invokeMock).toHaveBeenCalledWith('daemon_connection_state', undefined)
    expect(t.connectionState).toBe('connected')
    expect(h.state.subscribeOutputCalls).toBe(1)
    expect(h.state.capturedChannel).not.toBeNull()
  })

  it('init → 조회가 down 이면 아무 것도 하지 않는다(연결 안 됨은 정상)', async () => {
    const t = new TauriTransport()
    h.state.connectionStateReply = 'down'
    await t.init()
    expect(t.connectionState).toBe('down')
    expect(h.state.subscribeOutputCalls).toBe(0)
  })

  it('조회로 connected 등록 후 다시 connected 이벤트가 와도 이중 등록하지 않는다(멱등)', async () => {
    const t = new TauriTransport()
    h.state.connectionStateReply = 'connected'
    await t.init()
    expect(t.connectionState).toBe('connected')
    expect(h.state.subscribeOutputCalls).toBe(1)
    emit('daemon-connection-state', 'connected')
    await Promise.resolve()
    await Promise.resolve()
    expect(h.state.subscribeOutputCalls).toBe(1)
  })

  it('조회가 connected 를 반환해도 직후 down 이벤트가 오면 이벤트가 이긴다(last-write)', async () => {
    const t = new TauriTransport()
    h.state.connectionStateReply = 'connected'
    await t.init()
    expect(t.connectionState).toBe('connected')
    emit('daemon-connection-state', 'down')
    expect(t.connectionState).toBe('down')
  })

  // ── FIX 5: pull 응답 *전* 이벤트가 끼면 pull(stale)을 폐기(event→pull 순서 역전 방어) ──────
  it('조회 대기 중 down 이벤트가 끼면, 뒤늦게 온 connected 조회 결과를 폐기한다(이벤트 승)', async () => {
    const t = new TauriTransport()
    // pull 은 stale 'connected' 를 돌려주게 하되, 게이트로 응답을 미룬다(placeholder → 실 resolver 로 교체).
    h.state.connectionStateReply = 'connected'
    h.state.connectionStateGate = () => {}
    const initP = t.init()
    // init 은 registerListeners → selfHeal(invoke 게이트에 막힘) 순. 리스너 등록 + pull invoke 진입 대기.
    await flush()
    emit('daemon-connection-state', 'down')
    expect(t.connectionState).toBe('down')
    const release = h.state.connectionStateGate
    h.state.connectionStateGate = null
    release?.()
    await initP
    await flush()
    expect(t.connectionState).toBe('down')
    expect(h.state.subscribeOutputCalls).toBe(0)
  })
})

// ── FIX 7: 미지 상태 어휘 방어(retrofit 함정) ───────────────────────────────────────────
describe('TauriTransport 미지 연결 상태 어휘(FIX 7)', () => {
  it('알 수 없는 상태 문자열은 down 으로 처리 + console.warn 1회', async () => {
    const t = new TauriTransport()
    await t.start() // connected.
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    // Rust 가 나중에 새 어휘('connecting')를 emit 하면 조용히 오역하지 않고 안전측(down) 강등 + 경고.
    emit('daemon-connection-state', 'connecting')
    expect(t.connectionState).toBe('down')
    const unknownWarns = warnSpy.mock.calls.filter(
      (c) => typeof c[0] === 'string' && c[0].includes('알 수 없는 연결 상태'),
    )
    expect(unknownWarns.length).toBe(1)
    warnSpy.mockRestore()
  })
})

// ── FIX 6: 출력 Channel 등록 single-flight(겹치는 등록 시 Rust 도착 순서 역전 방어) ──────────
describe('TauriTransport 출력 Channel 등록 single-flight(FIX 6)', () => {
  it('진행 중 등록에 겹쳐 온 재등록은 동시 invoke 를 만들지 않고 완료 후 1회만 재실행한다', async () => {
    const t = new TauriTransport()
    h.state.subscribeOutputGate = true
    const startP = t.start()
    await flush()
    expect(h.state.subscribeOutputCalls).toBe(1)
    expect(h.state.subscribeOutputResolvers.length).toBe(1)
    emit('daemon-connection-state', 'reconnecting')
    emit('daemon-connection-state', 'connected')
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
    emit('daemon-connection-state', 'reconnecting')
    emit('daemon-connection-state', 'connected')
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

// ── ADR-0046 F2: requestReplay(invoke) + replay 경계 마커 정규화 ──────────────────────────
describe('TauriTransport requestReplay(ADR-0046 F2)', () => {
  it('requestReplay → invoke("request_replay",{agentId}) + gen(BigInt) 반환', async () => {
    const t = new TauriTransport()
    await t.start()
    const gen = await t.requestReplay(AGENT)
    expect(invokeMock).toHaveBeenCalledWith('request_replay', { agentId: AGENT })
    // u64 gen 을 BigInt 로 반환(§F2 — 마커 frame getBigUint64 와 폭 일치).
    expect(typeof gen).toBe('bigint')
    expect(gen).toBe(1n)
    expect(await t.requestReplay(AGENT)).toBe(2n)
  })

  // ── FIX-5: u64 gen 이 안전 정수 초과(number 직렬화)면 console.warn(실무 도달 불가) ──────────
  it('gen 이 안전 정수 초과 number 로 오면 console.warn 1회(BigInt 변환은 유지)', async () => {
    const t = new TauriTransport()
    await t.start()
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    // 2^53 초과 number(이미 정밀도 깨진 채 도착) — 실무 도달 불가하나 조용한 오각인 대신 경고.
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
    })
    // 마커는 output 으로 올라오지 않는다(공개 표면 미노출 — Designer 요구).
    expect(got.find((m) => m.kind === 'output')).toBeUndefined()
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
    // 이미 connected 면 start 즉시 resolve(doConnect 미진입) — 리스너 변화 없음.
    await t.start()
    expect(h.state.listenCalls).toBe(afterFirst)
    expect(h.listeners.get('agent-list-updated')!.size).toBe(1)
  })
})
