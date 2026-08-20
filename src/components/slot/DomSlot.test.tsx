// DomSlot 에이전트 부재 표시(ADR-0148 결정 1~3).
//
// ★왜 이 슬롯에도 필요한가★: DomSlot 은 관측용 read-only 슬롯이라 입력 경로가 없지만, 부재 막 하나가
//   유일한 상태 표면이다. 종료(kill)의 실제 결말은 reaper 가 세션을 수거하며 **명부에서 지우는** 것이라
//   status 로만 판정하면 그 순간 막이 사라져 죽은 슬롯이 살아있는 것처럼 보인다.
//
// 전략: slotTagGate.test.tsx 와 동일 패턴으로 clientFactory·agentStore 를 stub 한다(구독은 no-op).

import { cleanup, render } from '@testing-library/react'
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
    // 연결 상태 표면 — 슬롯이 부재 막 판정에 읽는다(등록 즉시 1회 통지 + disposer 반환).
    onConnectionStateChange: (cb: (s: string) => void) => {
      cb('connected')
      return () => {}
    },
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

function veil(): HTMLElement | null {
  return document.querySelector('[data-slot-dead="1"]')
}

describe('DomSlot — 에이전트 부재 표시(ADR-0148 결정 1~3)', () => {
  it('명부 수신 후 해석 안 되는 에이전트면 부재 막을 띄운다', async () => {
    agentStoreState.agentsLoaded = true
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flush()

    expect(veil()).not.toBeNull()
  })

  it('명부를 아직 못 받은 구간은 부재로 보지 않는다', async () => {
    agentStoreState.agentsLoaded = false
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flush()

    expect(veil()).toBeNull()
  })

  // ★사유를 글로 남기지 않는다(사용자 결정 2026-08-20)★: 옛 배지는 `Failed: <메시지>` 를 그대로 적었다.
  //   지금은 세 슬롯이 심볼 하나로만 말한다 — 이 단언이 그 결정의 회귀망이다.
  it('terminal 상태로 발견돼도 사유를 글로 적지 않는다(심볼만)', async () => {
    agentStoreState.agentsLoaded = true
    agentStoreState.agents = [
      { id: AGENT, cwd: 'C:/x', status: { type: 'Failed', message: 'boom' } },
    ]
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flush()

    const v = veil()
    expect(v).not.toBeNull()
    expect(v!.textContent).toBe('')
    expect(v!.querySelector('svg')).not.toBeNull()
  })
})
