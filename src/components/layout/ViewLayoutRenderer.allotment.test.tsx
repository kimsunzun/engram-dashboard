import { act, cleanup, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ★실 allotment 로 돈다★: 형제 스위트(ViewLayoutRenderer.test.tsx)는 allotment 를 평범한 div 로 갈아 끼워
//   분할 방향·크기가 prop 으로만 보인다. 여기서 잡는 결함은 allotment 가 방향을 마운트 때 한 번만 짓고
//   preferredSize 를 새로 합류한 pane 에만 먹인다는 외부 라이브러리 동작이라, 가짜로는 재현되지 않는다.
// jsdom 엔 ResizeObserver 가 없고 레이아웃도 없다 — 관측하는 모든 요소에 같은 크기를 즉시 알리는 가짜를 쓴다.
// 그래서 크기 단언은 최상위 split-view 에서만 의미가 있다(중첩 split 도 W×H 전체를 받는다).
const W = 1000
const H = 600
interface ObserverRecord {
  cb: ResizeObserverCallback
  els: Element[]
  self: FakeResizeObserver
}
const observers: ObserverRecord[] = []
class FakeResizeObserver {
  private rec: ObserverRecord
  constructor(cb: ResizeObserverCallback) {
    this.rec = { cb, els: [], self: this }
    observers.push(this.rec)
  }
  observe(el: Element): void {
    this.rec.els.push(el)
    this.fireOne(el)
  }
  unobserve(): void {}
  disconnect(): void {
    this.rec.els = []
  }
  fireOne(el: Element): void {
    const entry = { target: el, contentRect: { width: W, height: H } } as unknown as ResizeObserverEntry
    this.rec.cb([entry], this as unknown as ResizeObserver)
  }
}
;(globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = FakeResizeObserver

// ── Tauri / transport 계층 stub(형제 스위트와 같은 벌) ──────────────────────────────
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => undefined),
  Channel: class {
    onmessage: unknown = null
  },
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => vi.fn()),
}))
vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    subscribeOutput: vi.fn(async () => ({ unsubscribe: vi.fn() })),
    writeStdin: vi.fn(async () => undefined),
    resizePty: vi.fn(async () => undefined),
    spawnAgent: vi.fn(async () => ({ id: 'spawned-agent-id' })),
    killAgent: vi.fn(async () => undefined),
    connectionState: 'down',
    onConnectionStateChange: (cb: (s: string) => void) => {
      cb('down')
      return () => {}
    },
  },
  getAgentClient: vi.fn(),
}))
const agentStoreState = vi.hoisted(() => ({
  agents: [] as unknown[],
  agentsLoaded: false,
  profiles: [] as unknown[],
  profilesLoaded: false,
  presets: [] as unknown[],
}))
vi.mock('../../store/agentStore', () => ({
  useAgentStore: Object.assign(
    (selector: (s: typeof agentStoreState) => unknown) => selector(agentStoreState),
    { getState: () => agentStoreState },
  ),
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: vi.fn(async () => null),
}))

// ── 슬롯 stub — 실 구독/xterm 없이 마운트 여부만 ────────────────────────────────────
// ★THROWING_AGENT 를 받은 TerminalSlot 은 렌더 중에 던진다★ — 슬롯 단위 오류 격리 케이스의 표적.
const THROWING_AGENT = 'boom-agent'
vi.mock('../slot/TerminalSlot', () => ({
  default: ({ viewId, agentId }: { viewId: string; agentId: string }) => {
    if (agentId === THROWING_AGENT) throw new Error('terminal slot render failure (test)')
    return <div data-testid="terminal-slot" data-agent-id={agentId} data-view-id={viewId} />
  },
}))
vi.mock('../slot/RichSlot', () => ({
  default: ({ agentId }: { agentId: string }) => <div data-testid="rich-slot" data-agent-id={agentId} />,
}))
vi.mock('../slot/DomSlot', () => ({
  default: ({ agentId }: { agentId: string }) => <div data-testid="dom-slot" data-agent-id={agentId} />,
}))
vi.mock('../slot/PresetPalette', () => ({
  default: () => <div data-testid="preset-palette" />,
}))
vi.mock('../agent/AgentList', () => ({
  default: () => <div data-testid="agent-list" />,
}))

import ViewLayoutRenderer from './ViewLayoutRenderer'
import type { LayoutNode, SlotContent, SplitDir } from '../../api/layoutTypes'
import type { AgentInfo } from '../../api/types'
import { useViewStore } from '../../store/viewStore'

afterEach(() => {
  cleanup()
  observers.length = 0
  agentStoreState.agents = []
  useViewStore.setState({ renderModeOverride: {} })
})

// ── 헬퍼 ──────────────────────────────────────────────────────────────────────
function slot(id: string, content: SlotContent = { type: 'empty' }): LayoutNode {
  return { type: 'slot', id, content }
}
function agentSlot(id: string, agentId: string): LayoutNode {
  return slot(id, { type: 'agent', agent_id: agentId })
}
function split(id: string, dir: SplitDir, a: LayoutNode, b: LayoutNode, ratio: number): LayoutNode {
  return { type: 'split', id, dir, ratio, a, b }
}
const LR = (id: string, a: LayoutNode, b: LayoutNode, ratio = 0.5) => split(id, 'left_right', a, b, ratio)
const TB = (id: string, a: LayoutNode, b: LayoutNode, ratio = 0.5) => split(id, 'top_bottom', a, b, ratio)

/** 터미널 모드로 떨어지는 최소 AgentInfo(caps 게이트 통과용). */
function terminalAgent(id: string): AgentInfo {
  return {
    id,
    name: id,
    cwd: '/tmp',
    status: { type: 'Running' },
    cols: 80,
    rows: 24,
    epoch: 1,
    capabilities: {
      input: { raw: true, message: false, attachment: false },
      output: { terminal_bytes: true, structured: false, markdown: false, tool_events: false, usage: false },
      control: { resize: true, interrupt: true, cancel: false, graceful_shutdown: true },
      session: { resume: true, snapshot: false, cwd_env: true },
      model: { select: false, temperature: false, max_tokens: false },
    },
  }
}

function rootSplitView(container: HTMLElement): HTMLElement {
  return container.querySelector('.split-view') as HTMLElement
}

/** 최상위 split-view 의 pane 두 개가 allotment 에게서 받은 인라인 배치. */
function rootViews(container: HTMLElement) {
  const sv = rootSplitView(container)
  return Array.from(sv.querySelector(':scope > .split-view-container')!.children).map(v => {
    const s = (v as HTMLElement).style
    return { left: s.left, width: s.width, top: s.top, height: s.height }
  })
}

// ── 다른 split 이 같은 자리에 들어올 때 ─────────────────────────────────────────────
// 닫기(형제 승격)와 팝업 분리(= 원래 창에서 닫기)가 이 모양을 만든다. 막는 것: 옛 인스턴스가 재사용돼
//   방향이 얼어붙고(pane 이 0 으로 접혀 뷰가 빈 화면) 옛 픽셀 크기가 그대로 남는 것.
describe('ViewLayoutRenderer + 실 allotment — split 교체', () => {
  it('LR{s1, TB{s2,s3}} → TB{s2,s3}: 최상위 pane 이 top/height(수직)로 배치된다', () => {
    const { container, rerender } = render(
      <ViewLayoutRenderer node={LR('R', slot('s1'), TB('T', slot('s2'), slot('s3')))} focusedSlotId={null} />,
    )
    expect(rootViews(container).map(v => v.width)).toEqual(['500px', '500px'])

    rerender(<ViewLayoutRenderer node={TB('T', slot('s2'), slot('s3'))} focusedSlotId={null} />)

    const views = rootViews(container)
    expect(views.map(v => v.height)).toEqual(['300px', '300px'])
    expect(views.map(v => v.top)).toEqual(['0px', '300px'])
    expect(views.map(v => v.width)).toEqual(['', ''])
  })

  it('사용자 시나리오: 트리 슬롯을 뺀 2x2(행 우선) 격자 → 최상위가 수직 300/300', () => {
    const grid = TB('G', LR('G1', slot('a'), slot('b')), LR('G2', slot('c'), slot('d')))
    const { container, rerender } = render(
      <ViewLayoutRenderer node={LR('R', slot('tree', { type: 'agent_list' }), grid, 0.2)} focusedSlotId={null} />,
    )
    expect(rootViews(container).map(v => v.width)).toEqual(['200px', '800px'])

    rerender(<ViewLayoutRenderer node={grid} focusedSlotId={null} />)

    const views = rootViews(container)
    expect(views.map(v => v.height)).toEqual(['300px', '300px'])
    expect(views.map(v => v.width)).toEqual(['', ''])
    for (const id of ['a', 'b', 'c', 'd']) {
      expect(container.querySelector(`[data-slot-id="${id}"]`)).toBeTruthy()
    }
  })

  it('사용자 시나리오(열 우선 격자): 같은 방향이라도 다른 split 이 올라오면 옛 200/800 이 아니라 500/500', () => {
    const grid = LR('G', TB('G1', slot('a'), slot('c')), TB('G2', slot('b'), slot('d')))
    const { container, rerender } = render(
      <ViewLayoutRenderer node={LR('R', slot('tree', { type: 'agent_list' }), grid, 0.2)} focusedSlotId={null} />,
    )
    expect(rootViews(container).map(v => v.width)).toEqual(['200px', '800px'])

    rerender(<ViewLayoutRenderer node={grid} focusedSlotId={null} />)

    expect(rootViews(container).map(v => v.width)).toEqual(['500px', '500px'])
  })

  // 운영에서 모든 split 은 0.5 로 태어난다(백엔드 split_in_tree) — 방향·비율만으로는 승격된 split 을 못 가른다.
  //   옛 인스턴스를 쓰면 사용자가 드래그해 둔 옛 split 의 픽셀 크기가 새 split 에 남는다(jsdom 은 드래그를
  //   못 해 크기로는 안 보이므로 인스턴스 교체 = DOM 노드 교체로 잰다).
  it('방향·비율이 같아도 split id 가 다른 split 이 최상위로 올라오면 allotment 를 새로 짓는다', () => {
    const grid = LR('G', TB('G1', slot('a'), slot('c')), TB('G2', slot('b'), slot('d')))
    const { container, rerender } = render(
      <ViewLayoutRenderer node={LR('R', slot('tree', { type: 'agent_list' }), grid)} focusedSlotId={null} />,
    )
    const before = rootSplitView(container)

    rerender(<ViewLayoutRenderer node={grid} focusedSlotId={null} />)

    expect(Object.is(rootSplitView(container), before)).toBe(false)
    expect(rootViews(container).map(v => v.width)).toEqual(['500px', '500px'])
  })
})

// ── 같은 split 이 남을 때 — 인스턴스와 영향 없는 형제를 유지한다 ─────────────────────────
// 드래그한 크기는 백엔드에 되쓰이지 않아(ADR-0063) allotment 인스턴스가 유일한 보관처다. 형제 서브트리가
//   재마운트되면 터미널이 재구독하고, 보존 중이던 죽은 에이전트 뷰(ADR-0148)가 사라진다.
describe('ViewLayoutRenderer + 실 allotment — 같은 split 유지', () => {
  it('같은 split 에서 슬롯 콘텐츠만 바뀌면(빈 슬롯 → 에이전트) 같은 allotment DOM 노드·크기를 유지한다', () => {
    agentStoreState.agents = [terminalAgent('agent-1')]
    const { container, rerender } = render(
      <ViewLayoutRenderer node={LR('R', slot('s1'), slot('s2'), 0.2)} focusedSlotId={null} />,
    )
    const before = rootSplitView(container)

    rerender(<ViewLayoutRenderer node={LR('R', agentSlot('s1', 'agent-1'), slot('s2'), 0.2)} focusedSlotId={null} />)

    expect(Object.is(rootSplitView(container), before)).toBe(true)
    expect(rootViews(container).map(v => v.width)).toEqual(['200px', '800px'])
    expect(screen.getByTestId('terminal-slot')).toBeTruthy()
  })

  it('LR(R){x, TB{y,z}} → y 닫기 → LR(R){x, z}: 최상위 allotment 와 x 의 pane·슬롯이 그대로다', () => {
    const { container, rerender } = render(
      <ViewLayoutRenderer node={LR('R', slot('x'), TB('T', slot('y'), slot('z')), 0.2)} focusedSlotId={null} />,
    )
    const before = rootSplitView(container)
    const xSlot = container.querySelector('[data-slot-id="x"]')

    rerender(<ViewLayoutRenderer node={LR('R', slot('x'), slot('z'), 0.2)} focusedSlotId={null} />)

    expect(Object.is(rootSplitView(container), before)).toBe(true)
    expect(rootViews(container).map(v => v.width)).toEqual(['200px', '800px'])
    expect(Object.is(container.querySelector('[data-slot-id="x"]'), xSlot)).toBe(true)
  })

  // 조상 split 의 첫 슬롯이 바뀌는 닫기 — key 를 첫 슬롯 id 로 파생하면 여기서 형제 z 까지 새로 지어진다.
  it('LR(R){TB{x,y}, z} → x 닫기 → LR(R){y, z}: 첫 슬롯이 바뀌어도 형제 z 의 슬롯은 그대로다', () => {
    const { container, rerender } = render(
      <ViewLayoutRenderer node={LR('R', TB('T', slot('x'), slot('y')), slot('z'))} focusedSlotId={null} />,
    )
    const before = rootSplitView(container)
    const zSlot = container.querySelector('[data-slot-id="z"]')

    rerender(<ViewLayoutRenderer node={LR('R', slot('y'), slot('z'))} focusedSlotId={null} />)

    expect(Object.is(rootSplitView(container), before)).toBe(true)
    expect(Object.is(container.querySelector('[data-slot-id="z"]'), zSlot)).toBe(true)
  })
})

// ── 슬롯 하나의 렌더 오류가 창 전체를 지우지 않는다 ───────────────────────────────────
// React 19 는 경계 없는 렌더 throw 에 루트를 통째로 내린다 — 슬롯 하나가 던지면 창이 빈 화면이 된다.
describe('ViewLayoutRenderer — 슬롯 단위 렌더 오류 격리', () => {
  let consoleError: ReturnType<typeof vi.spyOn>

  beforeEach(() => {
    agentStoreState.agents = [terminalAgent(THROWING_AGENT), terminalAgent('ok-agent'), terminalAgent('ok-agent-2')]
    consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
  })
  afterEach(() => {
    consoleError.mockRestore()
  })

  function fallbackIn(slotId: string): Element | null {
    return document.querySelector(`[data-slot-id="${slotId}"] [data-slot-error]`)
  }

  it('던지는 슬롯만 대체 표시로 바뀌고 형제 슬롯은 그대로 렌더된다', () => {
    render(
      <ViewLayoutRenderer
        node={LR('R', agentSlot('bad', THROWING_AGENT), agentSlot('good', 'ok-agent'))}
        focusedSlotId={null}
      />,
    )

    expect(fallbackIn('bad')).toBeTruthy()
    expect(fallbackIn('good')).toBeNull()
    const terminals = screen.getAllByTestId('terminal-slot')
    expect(terminals.map(t => t.getAttribute('data-agent-id'))).toEqual(['ok-agent'])
    expect(consoleError).toHaveBeenCalled()
  })

  it('던지던 슬롯의 콘텐츠를 비우면 대체 표시가 걷힌다', () => {
    const { rerender } = render(
      <ViewLayoutRenderer
        node={LR('R', agentSlot('bad', THROWING_AGENT), agentSlot('good', 'ok-agent'))}
        focusedSlotId={null}
      />,
    )
    expect(fallbackIn('bad')).toBeTruthy()

    rerender(
      <ViewLayoutRenderer node={LR('R', slot('bad'), agentSlot('good', 'ok-agent'))} focusedSlotId={null} />,
    )

    expect(fallbackIn('bad')).toBeNull()
    expect(document.querySelector('[data-slot-id="bad"] > svg')).toBeTruthy()
  })

  // 같은 콘텐츠 종류(에이전트 → 에이전트)라 슬롯 컴포넌트는 같은 자리에 남는다 — 그래도 경계는 풀려야 한다.
  it('던지던 슬롯에 다른 에이전트를 배정하면 대체 표시가 걷히고 새 에이전트가 렌더된다', () => {
    const { rerender } = render(
      <ViewLayoutRenderer
        node={LR('R', agentSlot('bad', THROWING_AGENT), agentSlot('good', 'ok-agent'))}
        focusedSlotId={null}
      />,
    )
    expect(fallbackIn('bad')).toBeTruthy()

    rerender(
      <ViewLayoutRenderer
        node={LR('R', agentSlot('bad', 'ok-agent-2'), agentSlot('good', 'ok-agent'))}
        focusedSlotId={null}
      />,
    )

    expect(fallbackIn('bad')).toBeNull()
    const bad = document.querySelector('[data-slot-id="bad"] [data-testid="terminal-slot"]')
    expect(bad?.getAttribute('data-agent-id')).toBe('ok-agent-2')
  })

  it('렌더 모드를 바꿔도 대체 표시가 걷힌다(던지던 렌더러를 다른 렌더러로 교체)', () => {
    render(
      <ViewLayoutRenderer
        node={LR('R', agentSlot('bad', THROWING_AGENT), agentSlot('good', 'ok-agent'))}
        focusedSlotId={null}
      />,
    )
    expect(fallbackIn('bad')).toBeTruthy()

    act(() => useViewStore.getState().setRenderMode('bad', 'rich'))

    expect(fallbackIn('bad')).toBeNull()
    expect(document.querySelector('[data-slot-id="bad"] [data-testid="rich-slot"]')).toBeTruthy()
  })

  it('그 슬롯과 무관한 갱신(형제 콘텐츠 변경)으로는 대체 표시가 풀리지 않는다', () => {
    const { rerender } = render(
      <ViewLayoutRenderer
        node={LR('R', agentSlot('bad', THROWING_AGENT), agentSlot('good', 'ok-agent'))}
        focusedSlotId={null}
      />,
    )
    expect(fallbackIn('bad')).toBeTruthy()
    const callsBefore = consoleError.mock.calls.length

    rerender(<ViewLayoutRenderer node={LR('R', agentSlot('bad', THROWING_AGENT), slot('good'))} focusedSlotId={null} />)

    expect(fallbackIn('bad')).toBeTruthy()
    // 풀었다가 같은 자식을 다시 그려 또 던지지 않았다.
    expect(consoleError.mock.calls.length).toBe(callsBefore)
  })
})
