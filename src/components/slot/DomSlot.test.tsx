// DomSlot 에이전트 부재 표시(ADR-0148).
//
// ★왜 이 슬롯에도 필요한가★: DomSlot 은 관측용 read-only 슬롯이라 입력 경로가 없지만, 종료 배지 하나가
//   유일한 상태 표면이다. 종료(kill)의 실제 결말은 reaper 가 세션을 수거하며 **명부에서 지우는** 것이라
//   status 로만 판정하면 그 순간 배지가 사라져 죽은 슬롯이 살아있는 것처럼 보인다.
//
// 전략: slotTagGate.test.tsx 와 동일 패턴으로 clientFactory·agentStore 를 stub 한다(구독은 no-op).

import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// jsdom 은 ResizeObserver 를 제공하지 않는다 — DomSlot 이 쓰는 Radix ScrollArea 내부가 참조한다.
globalThis.ResizeObserver ||= class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver

vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    subscribeOutput: vi.fn(async () => ({ unsubscribe: vi.fn() })),
    writeStdin: vi.fn(async () => undefined),
    connectionState: 'connected',
  },
  getAgentClient: vi.fn(),
}))

const agentStoreState = vi.hoisted(() => ({ agents: [] as unknown[], agentsLoaded: false }))
vi.mock('../../store/agentStore', () => ({
  useAgentStore: (selector: (s: typeof agentStoreState) => unknown) => selector(agentStoreState),
}))

import DomSlot from './DomSlot'

const AGENT = 'aaaa-bbbb-cccc-dddd'

/** 구독 등록 마이크로태스크를 비운다. */
async function flush(): Promise<void> {
  await Promise.resolve()
  await Promise.resolve()
}

beforeEach(() => {
  agentStoreState.agents = []
  agentStoreState.agentsLoaded = false
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

describe('DomSlot — 에이전트 부재 표시(ADR-0148)', () => {
  it('명부 수신 후 해석 안 되는 에이전트면 종료 배지를 띄운다', async () => {
    agentStoreState.agentsLoaded = true
    render(<DomSlot viewId="v1" agentId={AGENT} epoch={2} />)
    await flush()

    expect(screen.getByText('종료됨')).toBeTruthy()
  })

  it('명부를 아직 못 받은 구간은 부재로 보지 않는다', async () => {
    agentStoreState.agentsLoaded = false
    render(<DomSlot viewId="v1" agentId={AGENT} epoch={2} />)
    await flush()

    expect(screen.queryByText('종료됨')).toBeNull()
  })

  it('terminal 상태로 발견된 경우(수거 전)에는 그 사유를 그대로 보여준다', async () => {
    agentStoreState.agentsLoaded = true
    agentStoreState.agents = [
      { id: AGENT, cwd: 'C:/x', status: { type: 'Failed', message: 'boom' } },
    ]
    render(<DomSlot viewId="v1" agentId={AGENT} epoch={2} />)
    await flush()

    expect(screen.getByText('Failed: boom')).toBeTruthy()
  })
})
