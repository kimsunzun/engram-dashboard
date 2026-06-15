// DaemonClient 단위테스트 — WS(데몬) 클라 로직(testing-strategy HIGH 갭 #1).
//
// invoke('discover_daemon') 와 globalThis.WebSocket 을 둘 다 mock 한다. FakeWebSocket 은
// 테스트가 onopen/onmessage(text·ArrayBuffer)/onclose 를 수동 발화시켜 시나리오를 조립할 수
// 있게 하고, send 된 모든 메시지를 기록해 assert 한다. 실제 데몬·소켓·Rust 접속 0.
//
// wire 계약 대조: crates/engram-dashboard-protocol/src/codec.rs
//   binary frame = [tag:1][agentId:16][epoch:4 BE][seq:8 BE][payload], 헤더 29.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ── invoke mock: discover_daemon 은 고정 DaemonInfoDto 반환 ──────────────────────────
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

// import 는 mock 선언 뒤(vi.mock 은 hoist 되므로 순서 무관하나 명시).
import { DaemonClient, decodeOutputFrame } from './daemonClient'
import type { AgentInfo, RestoreReport } from './types'

// ── 제어 가능한 FakeWebSocket ────────────────────────────────────────────────────────
// 테스트가 인스턴스를 잡고 onopen/onmessage/onclose 를 직접 호출해 서버 동작을 흉내낸다.
const OPEN = 1
const CLOSED = 3

class FakeWebSocket {
  static OPEN = OPEN
  static CLOSED = CLOSED
  // 가장 최근 생성 인스턴스(재연결 시 새 인스턴스로 갱신).
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

  // ── 테스트 구동 헬퍼 ──
  /** 서버가 연결 수락 → onopen 발화(코드가 Auth 전송). */
  fireOpen(): void {
    this.readyState = OPEN
    this.onopen?.()
  }
  /** 서버가 JSON text 메시지 전송. */
  fireText(obj: unknown): void {
    this.onmessage?.({ data: JSON.stringify(obj) })
  }
  /** 서버가 binary frame(ArrayBuffer) 전송. */
  fireBinary(buf: ArrayBuffer): void {
    this.onmessage?.({ data: buf })
  }
  /** 소켓 종료(비의도/의도). */
  fireClose(): void {
    this.readyState = CLOSED
    this.onclose?.()
  }
  /** send 된 메시지를 JSON 파싱해 반환(문자열 unit variant 는 그대로). */
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

// ── binary frame 빌더(codec.rs 와 동일 포맷) ────────────────────────────────────────
const FRAME_HEADER_LEN = 29
function uuidToBytes(uuid: string): Uint8Array {
  const hex = uuid.replace(/-/g, '')
  const out = new Uint8Array(16)
  for (let i = 0; i < 16; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16)
  return out
}
function buildFrame(opts: {
  tag?: number
  agentId: string
  epoch: number
  seq: number
  payload?: Uint8Array
  /** 헤더 미만 길이 테스트용 — 헤더 일부만 생성. */
  truncateTo?: number
}): ArrayBuffer {
  const payload = opts.payload ?? new Uint8Array(0)
  const buf = new ArrayBuffer(FRAME_HEADER_LEN + payload.length)
  const view = new DataView(buf)
  view.setUint8(0, opts.tag ?? 0)
  const idBytes = uuidToBytes(opts.agentId)
  for (let i = 0; i < 16; i++) view.setUint8(1 + i, idBytes[i])
  view.setUint32(17, opts.epoch, false) // BE
  view.setBigUint64(21, BigInt(opts.seq), false) // BE
  new Uint8Array(buf, FRAME_HEADER_LEN).set(payload)
  if (opts.truncateTo !== undefined) return buf.slice(0, opts.truncateTo)
  return buf
}

// ── crypto.randomUUID 보장(sendCommand 가 사용) ──────────────────────────────────────
let uuidCounter = 0
beforeEach(() => {
  FakeWebSocket.last = null
  FakeWebSocket.instances = []
  invokeMock.mockClear()
  uuidCounter = 0
  ;(globalThis as unknown as { WebSocket: unknown }).WebSocket = FakeWebSocket
  // 결정적 request_id — pending 매칭 테스트가 값을 예측 가능하게.
  // crypto 는 jsdom/node 에서 getter-only(직접 재할당 불가) 이므로 메서드만 spy 로 교체한다.
  vi.spyOn(globalThis.crypto, 'randomUUID').mockImplementation(
    () => `req-${++uuidCounter}` as `${string}-${string}-${string}-${string}-${string}`,
  )
})
afterEach(() => {
  vi.restoreAllMocks()
  vi.useRealTimers()
})

const AGENT = '12345678-9abc-def0-1234-56789abcdef0'

/** 핸드셰이크까지 완료한 연결된 클라 + 소켓 반환(connect 를 트리거하고 Hello 응답). */
async function connect(client: DaemonClient): Promise<FakeWebSocket> {
  // ensureConnected 를 트리거하는 가장 단순한 명령: subscribeOutput 은 await ensureConnected.
  // 여기선 getAgents 등 대신 직접 내부 연결을 일으키는 subscribe 를 쓰지 않고,
  // connect 만 따로 트리거하기 위해 listProfiles 의 ensureConnected 를 활용.
  const p = client.listProfiles().catch(() => {
    /* 본 테스트에서 resolve 안 시키면 무시 */
  })
  // 마이크로태스크 — invoke(discover) 후 new WebSocket 까지 진행.
  await Promise.resolve()
  await Promise.resolve()
  const ws = FakeWebSocket.last!
  ws.fireOpen()
  ws.fireText({ Hello: { protocol_version: 1 } })
  await Promise.resolve()
  void p
  return ws
}

describe('decodeOutputFrame', () => {
  it('codec.rs 포맷대로 디코드: tag/epoch/seq/payload + agentId UUID 왕복', () => {
    const payload = new Uint8Array([0x68, 0x69]) // "hi"
    const buf = buildFrame({ agentId: AGENT, epoch: 7, seq: 42, payload })
    const f = decodeOutputFrame(buf)
    expect(f).not.toBeNull()
    expect(f!.tag).toBe(0)
    expect(f!.epoch).toBe(7)
    expect(f!.seq).toBe(42)
    // 16바이트 → 8-4-4-4-12 소문자 UUID 정확 복원(알려진 uuid ↔ 바이트 왕복).
    expect(f!.agentId).toBe(AGENT)
    expect(Array.from(f!.payload)).toEqual([0x68, 0x69])
  })

  it('대문자 입력도 소문자 UUID 로 정규화한다(byte→hex 는 항상 소문자)', () => {
    const upper = 'ABCDEF01-2345-6789-ABCD-EF0123456789'
    const buf = buildFrame({ agentId: upper, epoch: 0, seq: 0 })
    const f = decodeOutputFrame(buf)
    expect(f!.agentId).toBe(upper.toLowerCase())
  })

  it('헤더 길이 미만이면 null', () => {
    const buf = buildFrame({ agentId: AGENT, epoch: 1, seq: 1, truncateTo: 28 })
    expect(decodeOutputFrame(buf)).toBeNull()
  })

  it('tag != 0(미지원 variant)면 null', () => {
    const buf = buildFrame({ tag: 1, agentId: AGENT, epoch: 1, seq: 1 })
    expect(decodeOutputFrame(buf)).toBeNull()
  })

  it('빈 payload(헤더만)도 디코드 성공(payload 길이 0)', () => {
    const buf = buildFrame({ agentId: AGENT, epoch: 3, seq: 9 })
    const f = decodeOutputFrame(buf)
    expect(f).not.toBeNull()
    expect(f!.payload.length).toBe(0)
  })
})

describe('연결 핸드셰이크', () => {
  it('ensureConnected → invoke(discover_daemon) + onopen 후 Auth 전송 + Hello 로 connected', async () => {
    const client = new DaemonClient()
    expect(client.connectionState).toBe('down')
    const ws = await connect(client)

    expect(invokeMock).toHaveBeenCalledWith('discover_daemon')
    expect(ws.url).toBe('ws://127.0.0.1:9999')
    // 첫 전송 = Auth frame(token + protocol_version echo).
    const first = ws.parsedSent()[0] as { Auth: { token: string; protocol_version: number } }
    expect(first.Auth.token).toBe('test-token')
    expect(first.Auth.protocol_version).toBe(1)
    expect(client.connectionState).toBe('connected')

    client.close()
  })
})

describe('request_id pending 매칭', () => {
  it('spawnAgent → Spawned{request_id,agent} 로 AgentInfo resolve', async () => {
    const client = new DaemonClient()
    const ws = await connect(client)
    const p = client.spawnAgent('C:/work')
    await Promise.resolve()
    // sendCommand 가 SpawnByCwd{request_id} 전송 — request_id 추출.
    const sent = ws.parsedSent().find((m) => typeof m === 'object' && m && 'SpawnByCwd' in m) as {
      SpawnByCwd: { request_id: string; cwd: string }
    }
    expect(sent.SpawnByCwd.cwd).toBe('C:/work')
    const rid = sent.SpawnByCwd.request_id
    ws.fireText({ Spawned: { request_id: rid, agent: { id: 'a1', name: 'A' } } })
    const info = await p
    expect(info).toEqual({ id: 'a1', name: 'A' })
    client.close()
  })

  it('createClaudeProfile → Created{request_id,profile} resolve', async () => {
    const client = new DaemonClient()
    const ws = await connect(client)
    const p = client.createClaudeProfile('n', 'C:/c', [], [], false)
    await Promise.resolve()
    const sent = ws.parsedSent().find((m) => typeof m === 'object' && m && 'CreateProfile' in m) as {
      CreateProfile: { request_id: string }
    }
    const rid = sent.CreateProfile.request_id
    ws.fireText({ Created: { request_id: rid, profile: { id: 'p1', name: 'n' } } })
    const prof = await p
    expect(prof).toEqual({ id: 'p1', name: 'n' })
    client.close()
  })

  it('killAgent → Ack{request_id} 로 void resolve', async () => {
    const client = new DaemonClient()
    const ws = await connect(client)
    const p = client.killAgent('a1')
    await Promise.resolve()
    const sent = ws.parsedSent().find((m) => typeof m === 'object' && m && 'Kill' in m) as {
      Kill: { request_id: string }
    }
    ws.fireText({ Ack: { request_id: sent.Kill.request_id } })
    await expect(p).resolves.toBeUndefined()
    client.close()
  })

  it('Error{request_id} 로 reject', async () => {
    const client = new DaemonClient()
    const ws = await connect(client)
    const p = client.killAgent('a1')
    await Promise.resolve()
    const sent = ws.parsedSent().find((m) => typeof m === 'object' && m && 'Kill' in m) as {
      Kill: { request_id: string }
    }
    ws.fireText({ Error: { request_id: sent.Kill.request_id, message: 'boom' } })
    await expect(p).rejects.toThrow('boom')
    client.close()
  })

  it('잘못된 request_id 의 응답은 무시(pending 유지)', async () => {
    const client = new DaemonClient()
    const ws = await connect(client)
    const p = client.killAgent('a1')
    await Promise.resolve()
    let settled = false
    void p.then(() => (settled = true)).catch(() => (settled = true))
    // 엉뚱한 request_id 의 Ack — 무시되어야 함.
    ws.fireText({ Ack: { request_id: 'nonexistent-rid' } })
    await Promise.resolve()
    expect(settled).toBe(false)
    client.close()
  })
})

describe('★ 조회 전용 reply 매칭(편승 오매칭 제거, protocol v2) ★', () => {
  /** ws.parsedSent() 에서 주어진 variant key 를 가진 마지막 명령의 request_id 추출. */
  function lastReqId(ws: FakeWebSocket, key: string): string {
    const sent = ws
      .parsedSent()
      .filter((m): m is Record<string, { request_id: string }> => typeof m === 'object' && !!m && key in m)
    return sent[sent.length - 1][key].request_id
  }

  it('getAgents → request_id 동봉 ListAgents 전송 + AgentList{request_id} 로 resolve', async () => {
    const client = new DaemonClient()
    const ws = await connect(client)
    const p = client.getAgents()
    await Promise.resolve()
    const rid = lastReqId(ws, 'ListAgents')
    const agents = [{ id: 'a1' }, { id: 'a2' }] as unknown as AgentInfo[]
    ws.fireText({ AgentList: { request_id: rid, agents } })
    await expect(p).resolves.toEqual(agents)
    client.close()
  })

  it('getAgents 진행 중 다른 request_id 의 broadcast AgentListUpdated 가 끼어도 편승하지 않는다', async () => {
    const client = new DaemonClient()
    const ws = await connect(client)
    // broadcast 구독자도 등록 — broadcast 는 여전히 cb 로 가야 한다(두 경로 공존).
    const broadcasts: AgentInfo[][] = []
    client.onAgentListUpdated((a) => broadcasts.push(a))

    const p = client.getAgents()
    await Promise.resolve()
    const rid = lastReqId(ws, 'ListAgents')

    // ① 내 요청과 무관한 broadcast(다른 클라의 CRUD 트리거 흉내) — getAgents 가 편승 resolve 하면 버그.
    const other = [{ id: 'other' }] as unknown as AgentInfo[]
    ws.fireText({ AgentListUpdated: { agents: other } })
    let settled = false
    void p.then(() => (settled = true)).catch(() => (settled = true))
    await Promise.resolve()
    expect(settled).toBe(false) // 편승 안 함(구 버그라면 여기서 other 로 resolve)
    expect(broadcasts).toEqual([other]) // broadcast 는 정상 라우팅

    // ② 내 request_id 의 전용 reply 가 와야 resolve.
    const mine = [{ id: 'mine' }] as unknown as AgentInfo[]
    ws.fireText({ AgentList: { request_id: rid, agents: mine } })
    await expect(p).resolves.toEqual(mine)
    client.close()
  })

  it('동시 2개 getAgents 가 각자 request_id 로 정확히 짝지어진다', async () => {
    const client = new DaemonClient()
    const ws = await connect(client)
    const p1 = client.getAgents()
    const p2 = client.getAgents()
    await Promise.resolve()
    const sent = ws
      .parsedSent()
      .filter((m): m is { ListAgents: { request_id: string } } => typeof m === 'object' && !!m && 'ListAgents' in m)
    expect(sent.length).toBe(2)
    const [rid1, rid2] = [sent[0].ListAgents.request_id, sent[1].ListAgents.request_id]
    expect(rid1).not.toBe(rid2)
    const list1 = [{ id: 'L1' }] as unknown as AgentInfo[]
    const list2 = [{ id: 'L2' }] as unknown as AgentInfo[]
    // 역순으로 응답해도 request_id 로 정확히 매칭.
    ws.fireText({ AgentList: { request_id: rid2, agents: list2 } })
    ws.fireText({ AgentList: { request_id: rid1, agents: list1 } })
    await expect(p1).resolves.toEqual(list1)
    await expect(p2).resolves.toEqual(list2)
    client.close()
  })

  it('listProfiles → ProfileList{request_id} 로 resolve, broadcast ProfileListUpdated 는 편승 안 함', async () => {
    const client = new DaemonClient()
    const ws = await connect(client)
    const p = client.listProfiles()
    await Promise.resolve()
    const rid = lastReqId(ws, 'ListProfiles')
    // 무관한 broadcast 가 끼어도 편승 금지.
    ws.fireText({ ProfileListUpdated: { profiles: [{ id: 'noise' }] } })
    let settled = false
    void p.then(() => (settled = true)).catch(() => (settled = true))
    await Promise.resolve()
    expect(settled).toBe(false)
    const profiles = [{ id: 'pX' }]
    ws.fireText({ ProfileList: { request_id: rid, profiles } })
    await expect(p).resolves.toEqual(profiles)
    client.close()
  })

  it('getSnapshot → Snapshot{request_id} 로 resolve(같은 agent_id 동시 조회도 정확 매칭)', async () => {
    const client = new DaemonClient()
    const ws = await connect(client)
    const p1 = client.getSnapshot(AGENT)
    const p2 = client.getSnapshot(AGENT) // 같은 agent_id 동시 2건
    await Promise.resolve()
    const sent = ws
      .parsedSent()
      .filter((m): m is { GetSnapshot: { request_id: string; agent_id: string } } => typeof m === 'object' && !!m && 'GetSnapshot' in m)
    expect(sent.length).toBe(2)
    const rid1 = sent[0].GetSnapshot.request_id
    const rid2 = sent[1].GetSnapshot.request_id
    expect(rid1).not.toBe(rid2)
    const chunks1 = [{ seq: 1, data: [1] }]
    const chunks2 = [{ seq: 2, data: [2] }]
    // 같은 agent_id 라도 request_id 로 짝지어 — 구 agent_id 매칭이면 순서대로 잘못 짝지을 위험.
    ws.fireText({ Snapshot: { request_id: rid2, agent_id: AGENT, chunks: chunks2 } })
    ws.fireText({ Snapshot: { request_id: rid1, agent_id: AGENT, chunks: chunks1 } })
    await expect(p1).resolves.toEqual(chunks1)
    await expect(p2).resolves.toEqual(chunks2)
    client.close()
  })
})

describe('★ 재연결 resume 회귀(버그 A/B) ★', () => {
  it('재연결 시 알려진 epoch + after_seq=마지막배달seq 로 resubscribe → seq 무손실·무중복', async () => {
    const client = new DaemonClient()
    const ws1 = await connect(client)

    const received: number[] = []
    await client.subscribeOutput(AGENT, (chunk) => received.push(chunk.seq))
    await Promise.resolve()

    const E = 5
    // 첫 SubscribeAck — epoch=E. replay_from 은 정보용(dedup 기준 아님).
    ws1.fireText({
      SubscribeAck: { agent_id: AGENT, current_epoch: E, replay_from: 0, truncated: false },
    })
    // frame seq 0,1,2 배달.
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 0 }))
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 1 }))
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 2 }))
    expect(received).toEqual([0, 1, 2])

    // ── 비의도 onclose → 재연결 ──
    ws1.fireClose()
    // handleClose → scheduleReconnect(첫 백오프 500ms). 실시간 setTimeout 을 await 으로 통과.
    await new Promise((r) => setTimeout(r, 600))

    const ws2 = FakeWebSocket.last!
    expect(ws2).not.toBe(ws1)
    ws2.fireOpen()
    // 재연결 핸드셰이크: Auth 재전송 확인.
    expect(ws2.parsedSent().some((m) => typeof m === 'object' && m && 'Auth' in m)).toBe(true)
    ws2.fireText({ Hello: { protocol_version: 1 } })
    await Promise.resolve()

    // ★핵심 단언★: resubscribe 가 epoch=E(null 아님) + after_seq=2(마지막 배달 seq)를 보냈는가.
    const resub = ws2.parsedSent().find((m) => typeof m === 'object' && m && 'Subscribe' in m) as {
      Subscribe: { agent_id: string; epoch: number | null; after_seq: number | null }
    }
    expect(resub).toBeTruthy()
    expect(resub.Subscribe.agent_id).toBe(AGENT)
    // 버그 A 회귀: epoch=null 이면 데몬이 FromOldest 전체 replay → 중복. 반드시 E.
    expect(resub.Subscribe.epoch).toBe(E)
    // 버그 B 회귀: after_seq 는 마지막 배달 seq(2). null/replay_from 이면 off-by-one.
    expect(resub.Subscribe.after_seq).toBe(2)

    // 데몬 Resume → seq 3 송신.
    ws2.fireText({
      SubscribeAck: { agent_id: AGENT, current_epoch: E, replay_from: 3, truncated: false },
    })
    ws2.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 3 }))

    // 무손실·무중복·순서정상.
    expect(received).toEqual([0, 1, 2, 3])
    client.close()
  })

  it('재연결 후 데몬이 이미 본 seq(0,1,2)를 다시 보내도 dedup → 중복 배달 안 함', async () => {
    const client = new DaemonClient()
    const ws1 = await connect(client)
    const received: number[] = []
    await client.subscribeOutput(AGENT, (c) => received.push(c.seq))
    await Promise.resolve()
    const E = 2
    ws1.fireText({
      SubscribeAck: { agent_id: AGENT, current_epoch: E, replay_from: 0, truncated: false },
    })
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 0 }))
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 1 }))
    ws1.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 2 }))

    ws1.fireClose()
    await new Promise((r) => setTimeout(r, 600))
    const ws2 = FakeWebSocket.last!
    ws2.fireOpen()
    ws2.fireText({ Hello: { protocol_version: 1 } })
    await Promise.resolve()
    ws2.fireText({
      SubscribeAck: { agent_id: AGENT, current_epoch: E, replay_from: 0, truncated: false },
    })
    // 데몬이(epoch=null 처럼 행동하는 옛 버그 상황 흉내) 0,1,2 를 다시 보냄 — 클라 dedup 이 막아야.
    ws2.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 0 }))
    ws2.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 1 }))
    ws2.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 2 }))
    ws2.fireBinary(buildFrame({ agentId: AGENT, epoch: E, seq: 3 }))
    // dedup: 0,1,2 재수신은 drop, 3 만 새로.
    expect(received).toEqual([0, 1, 2, 3])
    client.close()
  })
})

describe('dedup·epoch 처리', () => {
  it('같은 seq 재수신 → drop(중복 배달 안 함)', async () => {
    const client = new DaemonClient()
    const ws = await connect(client)
    const received: number[] = []
    await client.subscribeOutput(AGENT, (c) => received.push(c.seq))
    await Promise.resolve()
    ws.fireText({
      SubscribeAck: { agent_id: AGENT, current_epoch: 1, replay_from: 0, truncated: false },
    })
    ws.fireBinary(buildFrame({ agentId: AGENT, epoch: 1, seq: 0 }))
    ws.fireBinary(buildFrame({ agentId: AGENT, epoch: 1, seq: 0 })) // 중복
    ws.fireBinary(buildFrame({ agentId: AGENT, epoch: 1, seq: 1 }))
    expect(received).toEqual([0, 1])
    client.close()
  })

  it('epoch 안 맞는 frame → drop(stale 세션 잔여)', async () => {
    const client = new DaemonClient()
    const ws = await connect(client)
    const received: number[] = []
    await client.subscribeOutput(AGENT, (c) => received.push(c.seq))
    await Promise.resolve()
    ws.fireText({
      SubscribeAck: { agent_id: AGENT, current_epoch: 5, replay_from: 0, truncated: false },
    })
    ws.fireBinary(buildFrame({ agentId: AGENT, epoch: 4, seq: 0 })) // 옛 epoch
    ws.fireBinary(buildFrame({ agentId: AGENT, epoch: 5, seq: 0 })) // 맞는 epoch
    expect(received).toEqual([0])
    client.close()
  })

  it('SubscribeAck.current_epoch 변경 → high-water 리셋 → 새 스트림 낮은 seq 도 배달', async () => {
    const client = new DaemonClient()
    const ws = await connect(client)
    const received: number[] = []
    await client.subscribeOutput(AGENT, (c) => received.push(c.seq))
    await Promise.resolve()
    // epoch 10, seq 0..2 배달 → high-water = 2.
    ws.fireText({
      SubscribeAck: { agent_id: AGENT, current_epoch: 10, replay_from: 0, truncated: false },
    })
    ws.fireBinary(buildFrame({ agentId: AGENT, epoch: 10, seq: 0 }))
    ws.fireBinary(buildFrame({ agentId: AGENT, epoch: 10, seq: 1 }))
    ws.fireBinary(buildFrame({ agentId: AGENT, epoch: 10, seq: 2 }))
    expect(received).toEqual([0, 1, 2])
    // 재시작 → epoch 11. 새 SubscribeAck 가 high-water 리셋해야 새 스트림 seq 0 이 배달됨.
    ws.fireText({
      SubscribeAck: { agent_id: AGENT, current_epoch: 11, replay_from: 0, truncated: false },
    })
    ws.fireBinary(buildFrame({ agentId: AGENT, epoch: 11, seq: 0 }))
    ws.fireBinary(buildFrame({ agentId: AGENT, epoch: 11, seq: 1 }))
    expect(received).toEqual([0, 1, 2, 0, 1])
    client.close()
  })

  it('SubscribeAck 전 도착 frame(epoch undefined) → epoch 가드 통과(배달)', async () => {
    const client = new DaemonClient()
    const ws = await connect(client)
    const received: number[] = []
    await client.subscribeOutput(AGENT, (c) => received.push(c.seq))
    await Promise.resolve()
    // Ack 전 frame — st.epoch===undefined 라 epoch 가드 무시, seq 가드만 적용.
    ws.fireBinary(buildFrame({ agentId: AGENT, epoch: 99, seq: 0 }))
    expect(received).toEqual([0])
    client.close()
  })
})

describe('상태/목록/복원 이벤트 라우팅(eventBus 공통 표면)', () => {
  it('AgentListUpdated → onAgentListUpdated 등록 cb 가 agents 수신, unsubscribe 후 미수신', async () => {
    const client = new DaemonClient()
    const ws = await connect(client)
    const seen: AgentInfo[][] = []
    const off = client.onAgentListUpdated((agents) => seen.push(agents))

    const a1 = [{ id: 'a1' }, { id: 'a2' }] as unknown as AgentInfo[]
    ws.fireText({ AgentListUpdated: { agents: a1 } })
    expect(seen).toEqual([a1])

    off()
    ws.fireText({ AgentListUpdated: { agents: [{ id: 'a3' }] } })
    // unsubscribe 후엔 추가 수신 없음.
    expect(seen).toEqual([a1])
    client.close()
  })

  it('StatusChanged → cb 가 (id, status, epoch) 정확히 수신', async () => {
    const client = new DaemonClient()
    const ws = await connect(client)
    const calls: Array<[string, unknown, number]> = []
    const off = client.onStatusChanged((id, status, epoch) => calls.push([id, status, epoch]))

    const status = { type: 'Running' }
    // wire 필드명 agent_id/status/epoch → cb (id, status, epoch) 매핑 검증.
    ws.fireText({ StatusChanged: { agent_id: 'agent-7', status, epoch: 3 } })
    expect(calls).toEqual([['agent-7', status, 3]])

    off()
    ws.fireText({ StatusChanged: { agent_id: 'agent-7', status, epoch: 4 } })
    expect(calls).toEqual([['agent-7', status, 3]])
    client.close()
  })

  it('RestoreResult{report} → cb 가 report 수신', async () => {
    const client = new DaemonClient()
    const ws = await connect(client)
    const seen: RestoreReport[] = []
    const off = client.onRestoreResult((report) => seen.push(report))

    const report = {
      agent_id: 'agent-9',
      epoch: 1,
      outcome: { type: 'Resumed' },
    } as RestoreReport
    // wire 는 {report} 래핑 → cb 는 report 만 받음.
    ws.fireText({ RestoreResult: { report } })
    expect(seen).toEqual([report])

    off()
    ws.fireText({ RestoreResult: { report } })
    expect(seen).toEqual([report])
    client.close()
  })

  it('여러 구독자에게 동시 broadcast(AgentListUpdated)', async () => {
    const client = new DaemonClient()
    const ws = await connect(client)
    const a: AgentInfo[][] = []
    const b: AgentInfo[][] = []
    client.onAgentListUpdated((x) => a.push(x))
    client.onAgentListUpdated((x) => b.push(x))
    const list = [{ id: 'z' }] as unknown as AgentInfo[]
    ws.fireText({ AgentListUpdated: { agents: list } })
    expect(a).toEqual([list])
    expect(b).toEqual([list])
    client.close()
  })
})

describe('#13133 정리 + close', () => {
  it('재연결 시 옛 소켓의 onmessage 등이 delete(null 아님) 된다', async () => {
    const client = new DaemonClient()
    const ws1 = await connect(client)
    // close() 가 cleanupSocket 으로 delete 하는지 — 핸들러 프로퍼티가 사라지는지 관측.
    client.close()
    // delete 이후 'onmessage' in ws1 === false(null 대입이면 true 유지).
    expect('onmessage' in ws1).toBe(false)
    expect('onopen' in ws1).toBe(false)
    expect('onerror' in ws1).toBe(false)
    expect('onclose' in ws1).toBe(false)
    expect(ws1.closed).toBe(true)
  })

  it('close() → pending 명령 reject', async () => {
    const client = new DaemonClient()
    await connect(client)
    const p = client.killAgent('a1')
    await Promise.resolve()
    // close() 가 pending 명령을 reject 하는지 직접 단언한다. async 함수(sendCommand)의 promise
    // 언랩 때문에 reject → catch 도달까지 마이크로태스크 여러 틱이 필요하므로 `await Promise.resolve()`
    // 한 틱 가드는 경합으로 실패한다. rejects matcher 로 reject 자체(+에러 메시지)를 정확히 검증.
    client.close()
    await expect(p).rejects.toThrow('client closed')
  })

  it('close() 후 onclose 가 와도 재연결 안 함(closedByUser)', async () => {
    const client = new DaemonClient()
    const ws = await connect(client)
    client.close()
    const before = FakeWebSocket.instances.length
    // close 가 핸들러를 delete 했으므로 fireClose 는 no-op(onclose 없음). 재연결 타이머도 없음.
    ws.fireClose?.()
    await new Promise((r) => setTimeout(r, 600))
    expect(FakeWebSocket.instances.length).toBe(before)
    expect(client.connectionState).toBe('down')
  })
})
