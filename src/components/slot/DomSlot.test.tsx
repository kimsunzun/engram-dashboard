// DomSlot agent-absent display (ADR-0148 decisions 1~3).
//
// ★Why this slot needs it too★: DomSlot is a read-only slot for observation with no input path, but the absent curtain is
//   its only status surface. The real outcome of a kill is that the reaper collects the session and **removes it from the registry**, so
//   deciding by status alone would make the curtain vanish at that moment and a dead slot would look alive.
//
// Strategy: stub clientFactory and agentStore with the same pattern as slotTagGate.test.tsx (subscriptions are no-ops).

import { act, cleanup, fireEvent, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// jsdom does not provide ResizeObserver — the Radix ScrollArea internals that DomSlot uses refer to it.
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
    // Connection state surface — the slot reads it to decide the absent curtain (one notification on registration + returns a disposer).
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
import { agentClient } from '../../api/clientFactory'
import { getFollow } from './scrollFollow/followRegistry'

const AGENT = 'aaaa-bbbb-cccc-dddd'

/** Flushes the subscription-registration microtask. */
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

  // ★Do not leave the reason as text (user decision 2026-08-20)★: the old badge wrote `Failed: <메시지>` as is.
  //   Now the three slots speak with a single symbol only — this assertion is the regression net for that decision.
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

// ★Scroll-follow wiring (ADR-0242 · TRD §2-1)★: the decision and the hook are measured by the scrollFollow/ tests. Here we only check that this slot
//   plugged the hook into the observation viewport and that clearing re-attaches it. There is no layout, so only the Radix Viewport is given geometry.
describe('DomSlot — 스크롤 따라가기(ADR-0242)', () => {
  const tops = new WeakMap<Element, number>()
  const isViewport = (el: Element): boolean => el.hasAttribute('data-radix-scroll-area-viewport')

  beforeEach(() => {
    vi.spyOn(Element.prototype, 'scrollHeight', 'get').mockImplementation(function (this: Element) {
      return isViewport(this) ? 1000 : 0
    })
    vi.spyOn(Element.prototype, 'clientHeight', 'get').mockImplementation(function (this: Element) {
      return isViewport(this) ? 200 : 0
    })
    vi.spyOn(Element.prototype, 'scrollTop', 'get').mockImplementation(function (this: Element) {
      return tops.get(this) ?? 0
    })
    vi.spyOn(Element.prototype, 'scrollTop', 'set').mockImplementation(function (this: Element, v: number) {
      tops.set(this, v)
    })
  })

  const viewport = (): HTMLElement => document.querySelector('[data-radix-scroll-area-viewport]') as HTMLElement

  it('관측 뷰포트에 붙어 시작하고 바닥으로 쓴다 · 손잡이 맵에 그 슬롯으로 오른다', async () => {
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flush()
    expect(viewport().getAttribute('data-scroll-follow')).toBe('pinned')
    expect(viewport().scrollTop).toBe(800)
    expect(getFollow('v1')).toBeDefined()
  })

  it('비우기(onReset)는 다시 붙인다', async () => {
    render(<DomSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireEvent.wheel(viewport(), { deltaY: -40 })
    expect(viewport().getAttribute('data-scroll-follow')).toBe('free')

    const calls = vi.mocked(agentClient.subscribeOutput).mock.calls
    const onReset = calls[calls.length - 1][4]!
    act(() => onReset())
    expect(viewport().getAttribute('data-scroll-follow')).toBe('pinned')
  })
})
