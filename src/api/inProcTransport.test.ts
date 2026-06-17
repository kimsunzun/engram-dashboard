// InProcTransport 단위테스트 — embedded carrier(agent_connect/agent_command/TauriOutbound 정규화).
//
// @tauri-apps/api/core 의 invoke·Channel 을 mock 한다. FakeChannel 은 onmessage 를 잡아 테스트가
// TauriOutbound 를 주입(서버 push 흉내)할 수 있게 한다. invoke 는 agent_connect/agent_command 를
// 기록. 실제 Tauri 접속 0. wire 계약 대조: src-tauri/src/embedded_carrier.rs TauriOutbound.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ── Channel mock(호이스팅 안전: 클래스를 팩토리 내부 정의, 인스턴스는 전역 배열로 추적) ──
// vi.mock 팩토리는 파일 top 으로 hoist 되므로 외부 변수를 참조할 수 없다. FakeChannel 을 팩토리
// 안에 두고, 생성 인스턴스는 globalThis 배열에 push 해 테스트가 잡는다(daemonClient.test 의
// globalThis.WebSocket 패턴과 동형).
const invokeMock = vi.fn(async (_cmd: string, ..._rest: unknown[]) => undefined)
vi.mock('@tauri-apps/api/core', () => {
  class FakeChannel<T = unknown> {
    onmessage: ((msg: T) => void) | null = null
    constructor() {
      ;(globalThis as unknown as { __fakeChannels: unknown[] }).__fakeChannels.push(this)
    }
  }
  return {
    invoke: (cmd: string, ...rest: unknown[]) => invokeMock(cmd, ...rest),
    Channel: FakeChannel,
  }
})

import { InProcTransport } from './inProcTransport'
import type { InboundMessage } from './transport'

interface FakeChannelInst {
  onmessage: ((msg: unknown) => void) | null
}
function fakeChannels(): FakeChannelInst[] {
  return (globalThis as unknown as { __fakeChannels: FakeChannelInst[] }).__fakeChannels
}
function lastChannel(): FakeChannelInst {
  const a = fakeChannels()
  return a[a.length - 1]
}
function fire(ch: FakeChannelInst, msg: unknown): void {
  ch.onmessage?.(msg)
}

beforeEach(() => {
  ;(globalThis as unknown as { __fakeChannels: unknown[] }).__fakeChannels = []
  invokeMock.mockClear()
})
afterEach(() => {
  vi.restoreAllMocks()
})

describe('InProcTransport 연결/상태', () => {
  it('connectionState 항상 connected; onConnectionStateChange 즉시 connected 1회', () => {
    const t = new InProcTransport()
    expect(t.connectionState).toBe('connected')
    const seen: string[] = []
    t.onConnectionStateChange((s) => seen.push(s))
    expect(seen).toEqual(['connected'])
  })

  it('ensureReady → agent_connect 1회(Channel 등록) + 재호출 시 재등록 안 함', async () => {
    const t = new InProcTransport()
    await t.ensureReady()
    expect(invokeMock).toHaveBeenCalledWith('agent_connect', expect.objectContaining({ channel: expect.anything() }))
    const connectCalls = invokeMock.mock.calls.filter((c) => c[0] === 'agent_connect').length
    expect(connectCalls).toBe(1)
    await t.ensureReady() // 이미 등록 — 재등록 없음
    const after = invokeMock.mock.calls.filter((c) => c[0] === 'agent_connect').length
    expect(after).toBe(1)
  })
})

describe('InProcTransport send', () => {
  it('send → invoke(agent_command, {cmd})', async () => {
    const t = new InProcTransport()
    await t.ensureReady()
    const cmd = { ListAgents: { request_id: 'r1' } }
    await t.send(cmd)
    expect(invokeMock).toHaveBeenCalledWith('agent_command', { cmd })
  })

  it('close 후 send → reject', async () => {
    const t = new InProcTransport()
    await t.ensureReady()
    t.close()
    await expect(t.send({ x: 1 })).rejects.toThrow('client closed')
  })
})

describe('InProcTransport TauriOutbound 정규화', () => {
  it('kind=output(base64 PtyEvent) → output InboundMessage(PtyEvent.epoch 전달, 디코드 bytes)', async () => {
    const t = new InProcTransport()
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    await t.ensureReady()
    const ch = lastChannel()
    // base64("hi") = "aGk=". carrier 가 epoch=0 을 실어 보냄.
    fire(ch, { kind: 'output', output: { agent_id: 'agent-1', seq: 7, epoch: 0, data_b64: 'aGk=' } })
    expect(got.length).toBe(1)
    expect(got[0]).toMatchObject({ kind: 'output', agentId: 'agent-1', epoch: 0, seq: 7 })
    expect(Array.from((got[0] as { bytes: Uint8Array }).bytes)).toEqual([0x68, 0x69])
  })

  // ── BLOCKER 1 회귀: output epoch 은 PtyEvent.epoch 에서 온다(0 고정 아님) ──────────────────
  //    resume-fallback 세션(epoch≥1)에서 carrier 가 PtyEvent.epoch=1 을 실어 보내면, transport 가
  //    그걸 그대로 InboundMessage.epoch 으로 올려야 SubscribeAck.current_epoch=1 가드를 통과한다.
  //    transport 가 epoch 0 으로 고정(옛 버그)하면 이 테스트가 fail 한다.
  it('output epoch=1 PtyEvent → InboundMessage.epoch=1(0 고정 금지)', async () => {
    const t = new InProcTransport()
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    await t.ensureReady()
    const ch = lastChannel()
    fire(ch, { kind: 'output', output: { agent_id: 'agent-1', seq: 0, epoch: 1, data_b64: 'aGk=' } })
    expect(got[0]).toMatchObject({ kind: 'output', agentId: 'agent-1', epoch: 1, seq: 0 })
  })

  it('kind=event(AgentEvent) → control InboundMessage(event 그대로)', async () => {
    const t = new InProcTransport()
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    await t.ensureReady()
    const ch = lastChannel()
    const event = { Ack: { request_id: 'r9' } }
    fire(ch, { kind: 'event', event })
    expect(got).toEqual([{ kind: 'control', event }])
  })
})

describe('InProcTransport close', () => {
  it('close → onmessage delete(#13133) + 이후 push 무시', async () => {
    const t = new InProcTransport()
    const got: InboundMessage[] = []
    t.onMessage((m) => got.push(m))
    await t.ensureReady()
    const ch = lastChannel()
    t.close()
    expect(ch.onmessage ?? null).toBeNull()
    fire(ch, { kind: 'event', event: { Ack: { request_id: 'late' } } })
    expect(got).toEqual([]) // delete 됐으므로 미수신
  })
})
