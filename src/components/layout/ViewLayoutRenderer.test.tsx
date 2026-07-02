// ViewLayoutRenderer 렌더 분기 단위테스트.
//
// 검증 불변식:
//   1. agent_id 있는 slot → TerminalSlot 이 마운트된다.
//   2. agent_id null slot  → "— empty —" 플레이스홀더가 뜨고 TerminalSlot 은 마운트되지 않는다.
//   3. focusedSlotId == node.id → 포커스 테두리(border 스타일)가 적용된다.
//   4. split 노드 → 두 자식이 재귀 렌더된다(Allotment 모킹으로 DOM 평탄화).
//
// 전략: TerminalSlot 을 vi.mock 으로 stub — xterm DOM 직접 의존 없이 마운트 여부만 단언.
// agentClient(clientFactory) / @tauri-apps/api/core / allotment / @xterm 계열도 mock 처리.

import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

// ── Tauri / transport 계층 stub ────────────────────────────────────────────────
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => undefined),
  Channel: class {
    onmessage: unknown = null
  },
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => vi.fn()),
}))

// ── agentClient / clientFactory stub ─────────────────────────────────────────
// TerminalSlot 이 내부에서 agentClient 를 import 하지만 이번 테스트에선 TerminalSlot 자체를
// mock 하므로 실제 호출은 일어나지 않는다. clientFactory 도 Tauri invoke 를 사용하므로 stub.
vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    subscribeOutput: vi.fn(async () => ({ unsubscribe: vi.fn() })),
    writeStdin: vi.fn(async () => undefined),
    resizePty: vi.fn(async () => undefined),
    connectionState: 'down',
  },
  getAgentClient: vi.fn(),
}))

// ── agentStore stub — TerminalSlot 이 agents 를 읽지만 mock 컴포넌트라 불필요. 방어용. ──
vi.mock('../../store/agentStore', () => ({
  useAgentStore: vi.fn(() => []),
}))

// ── allotment stub — split 분기 렌더 시 jsdom 환경에서 ResizeObserver 에러 방지 ──
// Allotment / Allotment.Pane 을 단순 div 로 대체해 자식을 그대로 렌더한다.
// vi.mock factory 는 호이스팅되므로 React import 를 직접 쓸 수 없다 — importOriginal 패턴으로 우회.
vi.mock('allotment', async () => {
  const React = (await import('react')).default
  const Pane = ({ children }: { children: React.ReactNode }) =>
    React.createElement('div', { 'data-testid': 'allotment-pane' }, children)
  const Allotment = Object.assign(
    ({ children }: { children: React.ReactNode }) =>
      React.createElement('div', { 'data-testid': 'allotment' }, children),
    { Pane },
  )
  return { Allotment }
})

// ── TerminalSlot stub — xterm DOM 의존 없이 마운트 여부만 확인 ─────────────────
vi.mock('../slot/TerminalSlot', () => ({
  default: ({ agentId }: { agentId: string }) => (
    <div data-testid="terminal-slot" data-agent-id={agentId} />
  ),
}))

// ── RichSlot stub(M0 스파이크, ADR-0044) — 랩 렌더 트리/fixture import 없이 마운트 여부만 확인 ──
vi.mock('../slot/RichSlot', () => ({
  default: () => <div data-testid="rich-slot" />,
}))

// ── @xterm stub — TerminalSlot 이 실제로 렌더되지 않지만 import 해소 방어용 ────
vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    loadAddon = vi.fn()
    open = vi.fn()
    reset = vi.fn()
    write = vi.fn()
    onData = vi.fn(() => ({ dispose: vi.fn() }))
    dispose = vi.fn()
    cols = 80
    rows = 24
  },
}))
vi.mock('@xterm/addon-fit', () => ({
  FitAddon: class {
    fit = vi.fn()
  },
}))

// ── 테스트 대상 ────────────────────────────────────────────────────────────────
import ViewLayoutRenderer from './ViewLayoutRenderer'
import type { LayoutNode } from '../../api/layoutTypes'
import { useViewStore } from '../../store/viewStore'

afterEach(() => {
  cleanup()
  useViewStore.setState({ richSlots: {} }) // 스파이크 오버레이 격리(테스트 간 누수 방지)
})

// ── 헬퍼 ──────────────────────────────────────────────────────────────────────
function slotNode(id: string, agentId: string | null): LayoutNode {
  return { type: 'slot', id, agent_id: agentId }
}

function splitNode(a: LayoutNode, b: LayoutNode): LayoutNode {
  return { type: 'split', dir: 'horizontal', ratio: 0.5, a, b }
}

// ── 테스트 케이스 ─────────────────────────────────────────────────────────────

describe('ViewLayoutRenderer — slot 분기', () => {
  it('agent_id 있는 slot → TerminalSlot 이 마운트되고 agentId prop 이 전달된다', () => {
    const agentId = 'aaaa-bbbb-cccc-dddd'
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    const terminal = screen.getByTestId('terminal-slot')
    expect(terminal).toBeTruthy()
    expect(terminal.getAttribute('data-agent-id')).toBe(agentId)
  })

  it('agent_id null slot → "— empty —" 플레이스홀더가 뜨고 TerminalSlot 은 없다', () => {
    render(<ViewLayoutRenderer node={slotNode('s1', null)} focusedSlotId={null} />)
    expect(screen.queryByTestId('terminal-slot')).toBeNull()
    expect(screen.getByText('— empty —')).toBeTruthy()
  })

  it('focusedSlotId == node.id → 포커스 테두리(accent border)가 적용된다', () => {
    render(<ViewLayoutRenderer node={slotNode('s1', null)} focusedSlotId="s1" />)
    const wrapper = document.querySelector('[data-slot-id="s1"]') as HTMLElement
    expect(wrapper).toBeTruthy()
    // isFocused=true 일 때 border 에 'accent' 가 포함되어야 한다(CSS 변수 참조 형태 검사).
    expect(wrapper.style.border).toContain('accent')
  })

  it('focusedSlotId != node.id → 비포커스 테두리(border 변수)가 적용된다', () => {
    render(<ViewLayoutRenderer node={slotNode('s1', null)} focusedSlotId="s-other" />)
    const wrapper = document.querySelector('[data-slot-id="s1"]') as HTMLElement
    expect(wrapper.style.border).toContain('border')
    expect(wrapper.style.border).not.toContain('accent')
  })

  it('data-slot-id 속성이 node.id 로 설정된다(cdp 검증용 불변식)', () => {
    const id = 'test-slot-uuid'
    render(<ViewLayoutRenderer node={slotNode(id, null)} focusedSlotId={null} />)
    expect(document.querySelector(`[data-slot-id="${id}"]`)).toBeTruthy()
  })

  it('agent_id 있는 slot 래퍼에는 중앙정렬 flex 가 없다(터미널 레이아웃 오염 방지)', () => {
    render(<ViewLayoutRenderer node={slotNode('s1', 'some-agent-id')} focusedSlotId={null} />)
    const wrapper = document.querySelector('[data-slot-id="s1"]') as HTMLElement
    // agent 있을 때 justifyContent: center 가 없어야 한다 — TerminalSlot 을 center 로 밀면 출력이 깨짐.
    expect(wrapper.style.justifyContent).not.toBe('center')
    expect(wrapper.style.alignItems).not.toBe('center')
  })

  // ── M0 스파이크(임시) — ADR-0044 RichSlot 분기 ──────────────────────────────────
  it('richSlots 에 든 빈 slot → RichSlot 이 마운트되고 TerminalSlot/플레이스홀더는 없다', () => {
    useViewStore.setState({ richSlots: { 's-rich': true } })
    render(<ViewLayoutRenderer node={slotNode('s-rich', null)} focusedSlotId={null} />)
    expect(screen.getByTestId('rich-slot')).toBeTruthy()
    expect(screen.queryByTestId('terminal-slot')).toBeNull()
    expect(screen.queryByText('— empty —')).toBeNull()
  })

  it('agent_id 있는 slot 은 rich 마킹이 있어도 TerminalSlot 우선(터미널 실슬롯 우선)', () => {
    useViewStore.setState({ richSlots: { s1: true } })
    render(<ViewLayoutRenderer node={slotNode('s1', 'agent-x')} focusedSlotId={null} />)
    expect(screen.getByTestId('terminal-slot')).toBeTruthy()
    expect(screen.queryByTestId('rich-slot')).toBeNull()
  })

  it('rich 마킹 없는 빈 slot → "JSON 스파이크" dev 버튼이 있다(사람 소환 경로)', () => {
    render(<ViewLayoutRenderer node={slotNode('s1', null)} focusedSlotId={null} />)
    expect(screen.getByText('JSON 스파이크')).toBeTruthy()
  })
})

describe('ViewLayoutRenderer — split 분기', () => {
  it('split 노드 → a/b 두 자식 슬롯이 재귀 렌더된다', () => {
    const node = splitNode(slotNode('s1', null), slotNode('s2', null))
    render(<ViewLayoutRenderer node={node} focusedSlotId={null} />)
    // 두 슬롯 모두 DOM 에 있어야 한다.
    expect(document.querySelector('[data-slot-id="s1"]')).toBeTruthy()
    expect(document.querySelector('[data-slot-id="s2"]')).toBeTruthy()
  })

  it('split 자식에 agent_id 있으면 해당 슬롯에만 TerminalSlot 이 마운트된다', () => {
    const agentId = 'zzzz-agent'
    const node = splitNode(slotNode('s1', agentId), slotNode('s2', null))
    render(<ViewLayoutRenderer node={node} focusedSlotId={null} />)
    const terminals = screen.getAllByTestId('terminal-slot')
    // s1 은 agent 있으므로 TerminalSlot 1개, s2 는 empty.
    expect(terminals).toHaveLength(1)
    expect(terminals[0].getAttribute('data-agent-id')).toBe(agentId)
    // s2 는 empty 플레이스홀더만.
    expect(screen.getByText('— empty —')).toBeTruthy()
  })
})
