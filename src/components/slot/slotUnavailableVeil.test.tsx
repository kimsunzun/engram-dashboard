// The absence curtain shows up with **the same look** under **the same condition** in all three slots (ADR-0148 decisions 2·3 · user decision 2026-08-20).
//
// Background: terminal·DOM slots used to write text over a dark curtain — '에이전트 대기 중' · '종료됨' ·
//   `Failed: <메시지>`. Only the rich slot had a single symbol. The same fact was told in a different sentence per slot, and in spans where the decision wavers such as a daemon
//   restart, an assertion like '종료됨' stayed put. The three were merged into one symbol, and
//   this file is that agreement's regression net — it measures both the three conditions (exit · connection lost · waiting for subscription attach) and "no text".
//
// Strategy: same stub pattern as slotReplayRestart.test.tsx. The connection state holds subscribers like the real one and notifies
//   them on transitions (including one notification right at registration).

import type { ReactElement } from 'react'
import { act, cleanup, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { OutputChunk, ViewPhase } from '../../api/agentClient'

// Two observers jsdom does not provide — TerminalSlot creates both at mount, and Radix ScrollArea also references RO.
globalThis.ResizeObserver ||= class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver
globalThis.IntersectionObserver ||= class {
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return []
  }
} as unknown as typeof IntersectionObserver

const captured = vi.hoisted(() => ({
  onState: null as ((s: ViewPhase) => void) | null,
  onReset: null as (() => void) | null,
}))
const conn = vi.hoisted(() => ({
  state: 'connected' as 'connected' | 'reconnecting' | 'down',
  cbs: new Set<(s: string) => void>(),
}))

vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    subscribeOutput: vi.fn(
      async (
        _viewId: string,
        _agentId: string,
        _onChunk: (c: OutputChunk) => void,
        onState?: (s: ViewPhase) => void,
        onReset?: () => void,
      ) => {
        captured.onState = onState ?? null
        captured.onReset = onReset ?? null
        return { unsubscribe: vi.fn() }
      },
    ),
    writeStdin: vi.fn(async () => undefined),
    resizePty: vi.fn(async () => undefined),
    get connectionState() {
      return conn.state
    },
    onConnectionStateChange: (cb: (s: string) => void) => {
      conn.cbs.add(cb)
      cb(conn.state) // Same as the real one: one notification of the current state right at registration.
      return () => conn.cbs.delete(cb)
    },
  },
  getAgentClient: vi.fn(),
}))

const agentStoreState = vi.hoisted(() => ({
  agents: [] as unknown[],
  agentsLoaded: false,
  profiles: [] as unknown[],
}))
vi.mock('../../store/agentStore', () => ({
  useAgentStore: (selector: (s: typeof agentStoreState) => unknown) => selector(agentStoreState),
}))

vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    loadAddon = vi.fn()
    open = vi.fn()
    reset = vi.fn()
    write = vi.fn()
    onData = vi.fn(() => ({ dispose: vi.fn() }))
    dispose = vi.fn()
    refresh = vi.fn()
    cols = 80
    rows = 24
  },
}))
vi.mock('@xterm/addon-fit', () => ({ FitAddon: class { fit = vi.fn() } }))
vi.mock('@xterm/addon-webgl', () => ({ WebglAddon: class { onContextLoss = vi.fn(); dispose = vi.fn() } }))
vi.mock('@xterm/xterm/css/xterm.css', () => ({}))

import TerminalSlot from './TerminalSlot'
import DomSlot from './DomSlot'
import RichSlot from './RichSlot'

const AGENT = 'aaaa-bbbb-cccc-dddd'

/** Marker of the curtain the three slots share — this one selector is the mechanical definition of "same look". */
function veil(): HTMLElement | null {
  return document.querySelector('[data-slot-dead="1"]')
}

async function flush(): Promise<void> {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

function setConnection(state: 'connected' | 'reconnecting' | 'down'): void {
  act(() => {
    conn.state = state
    for (const cb of conn.cbs) cb(state)
  })
}

// The three slots' props differ (rich has viewId optional, terminal has agentId nullable) — only the render is wrapped.
const SLOTS: Array<[string, () => ReactElement]> = [
  ['TerminalSlot', () => <TerminalSlot viewId="v1" agentId={AGENT} />],
  ['DomSlot', () => <DomSlot viewId="v1" agentId={AGENT} />],
  ['RichSlot', () => <RichSlot viewId="v1" agentId={AGENT} />],
]

beforeEach(() => {
  captured.onState = null
  captured.onReset = null
  conn.state = 'connected'
  conn.cbs.clear()
  agentStoreState.agents = []
  agentStoreState.agentsLoaded = false
  agentStoreState.profiles = []
})

afterEach(() => {
  cleanup()
})

describe.each(SLOTS)('%s — 부재 막(세 슬롯 공통)', (_name, mount) => {
  // ★Second common obligation, measured at the same place as the curtain★: a slot that does not pass the clear callback returns only the cursor to 0 and
  //   the screen still holds the previous incarnation's content, so the full replay is drawn on top of it and no error is left anywhere.
  //   When slots are added, just adding one line to the SLOTS list here puts that obligation on them too.
  it('구독에 비우기 콜백(onReset)을 넘긴다', async () => {
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Running' }, epoch: 0 }]
    agentStoreState.agentsLoaded = true
    render(mount())
    await flush()

    expect(captured.onReset).toBeTypeOf('function')
  })

  it('아무 조건도 아니면 막이 없다', async () => {
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Running' }, epoch: 0 }]
    agentStoreState.agentsLoaded = true
    render(mount())
    await flush()

    expect(veil()).toBeNull()
  })

  it('명부를 아직 모르는 구간은 부재가 아니다(대기 표시를 그리지 않는다)', async () => {
    agentStoreState.agentsLoaded = false
    render(mount())
    await flush()

    expect(veil()).toBeNull()
  })

  it('연결이 끊기면 막이 뜬다', async () => {
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Running' }, epoch: 0 }]
    agentStoreState.agentsLoaded = true
    render(mount())
    await flush()
    expect(veil()).toBeNull()

    setConnection('reconnecting')
    expect(veil()).not.toBeNull()
  })

  it('명부 수신 후에도 해석 안 되면 막이 뜬다(수거된 에이전트)', async () => {
    agentStoreState.agents = []
    agentStoreState.agentsLoaded = true
    render(mount())
    await flush()

    expect(veil()).not.toBeNull()
  })

  it("구독이 'detached' 라고 하면 막이 뜨고, 부착되면 내려간다", async () => {
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Running' }, epoch: 0 }]
    agentStoreState.agentsLoaded = true
    render(mount())
    await flush()
    expect(captured.onState).toBeTruthy()
    expect(veil()).toBeNull()

    act(() => captured.onState!('detached'))
    expect(veil()).not.toBeNull()

    act(() => captured.onState!('buffering'))
    expect(veil()).toBeNull()
  })

  // ★Write no text (user decision)★: whether failure reason, exit or waiting, all the curtain says is one symbol.
  it('막은 심볼 하나뿐 — 글자를 담지 않는다(실패 사유 포함)', async () => {
    agentStoreState.agents = [
      { id: AGENT, cwd: 'C:/x', status: { type: 'Failed', message: 'boom' }, epoch: 0 },
    ]
    agentStoreState.agentsLoaded = true
    render(mount())
    await flush()

    const v = veil()
    expect(v).not.toBeNull()
    expect(v!.textContent).toBe('')
    expect(v!.querySelector('svg')).not.toBeNull()
  })
})
