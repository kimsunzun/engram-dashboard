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

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

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
// spawnAgent/killAgent 도 stub — SlotContextMenu 의 '에이전트 생성'(spawn→assign)·'에이전트 종료'(kill)
// 경로가 이들을 부른다. 컨텍스트 메뉴 테스트에서 spawn 결과 id 를 제어하려 spawnAgent 를 가변으로 둔다.
const clientMock = vi.hoisted(() => ({
  spawnAgent: vi.fn(async () => ({ id: 'spawned-agent-id' })),
  killAgent: vi.fn(async () => undefined),
}))
vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    subscribeOutput: vi.fn(async () => ({ unsubscribe: vi.fn() })),
    writeStdin: vi.fn(async () => undefined),
    resizePty: vi.fn(async () => undefined),
    spawnAgent: (...args: unknown[]) => clientMock.spawnAgent(...(args as [])),
    killAgent: (...args: unknown[]) => clientMock.killAgent(...(args as [])),
    connectionState: 'down',
  },
  getAgentClient: vi.fn(),
}))

// ── agentStore stub — ViewLayoutRenderer 가 `useAgentStore(s => s.agents)` 로 caps 를 조회한다. ──
// FIX 1(ADR-0041): 렌더러 분기가 store 의 AgentInfo 유무·caps 에 의존하므로 테스트가 agents 를 제어할 수
// 있어야 한다. vi.hoisted 로 가변 holder 를 만들어 selector 에 흘린다(TerminalSlot/RichSlot 은 stub 이라
// 자기 useAgentStore 호출은 무해). afterEach 에서 초기화.
const agentStoreState = vi.hoisted(() => ({ agents: [] as unknown[] }))
vi.mock('../../store/agentStore', () => ({
  useAgentStore: (selector: (s: { agents: unknown[] }) => unknown) => selector(agentStoreState),
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

// ── RichSlot stub(라이브 구조화 슬롯) — 실스트림 구독/누산 없이 마운트 여부만 확인 ──
vi.mock('../slot/RichSlot', () => ({
  default: () => <div data-testid="rich-slot" />,
}))

// ── DomSlot stub(§5 관측용) — 구독 배선 없이 마운트 여부·agentId prop 만 확인 ──
vi.mock('../slot/DomSlot', () => ({
  default: ({ agentId }: { agentId: string }) => (
    <div data-testid="dom-slot" data-agent-id={agentId} />
  ),
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
import type { AgentInfo, Capabilities } from '../../api/types'
import { useViewStore } from '../../store/viewStore'

afterEach(() => {
  cleanup()
  useViewStore.setState({ renderModeOverride: {} }) // 프론트 전용 오버라이드 격리(테스트 간 누수 방지)
  agentStoreState.agents = [] // agent store holder 초기화(테스트 간 누수 방지)
})

// ── 헬퍼 ──────────────────────────────────────────────────────────────────────
function slotNode(id: string, agentId: string | null): LayoutNode {
  // ADR-0060: 슬롯 점유자 = SlotContent 태그드 유니온(Empty / Agent{agent_id}).
  return {
    type: 'slot',
    id,
    content: agentId != null ? { type: 'agent', agent_id: agentId } : { type: 'empty' },
  }
}

function splitNode(a: LayoutNode, b: LayoutNode): LayoutNode {
  return { type: 'split', dir: 'horizontal', ratio: 0.5, a, b }
}

// caps 만 관건이라 나머지 필드는 최소값. structured=true → RichSlot, false → TerminalSlot 분기.
function caps(structured: boolean): Capabilities {
  return {
    input: { raw: true, message: false, attachment: false },
    output: { terminal_bytes: !structured, structured, markdown: false, tool_events: false, usage: false },
    control: { resize: true, interrupt: true, cancel: false, graceful_shutdown: true },
    session: { resume: true, snapshot: false, cwd_env: true },
    model: { select: false, temperature: false, max_tokens: false },
  }
}

function agentInfo(id: string, structured: boolean): AgentInfo {
  return {
    id,
    name: id,
    cwd: '/tmp',
    status: { type: 'Running' },
    cols: 80,
    rows: 24,
    epoch: 0,
    capabilities: caps(structured),
  }
}

/** store 에 AgentInfo 를 seed(FIX 1: caps 도착 후에만 구체 렌더러가 마운트되므로 대부분 테스트가 필요). */
function seedAgents(...infos: AgentInfo[]): void {
  agentStoreState.agents = infos
}

// ── 테스트 케이스 ─────────────────────────────────────────────────────────────

describe('ViewLayoutRenderer — slot 분기', () => {
  it('agent_id 있는 slot(비structured caps) → TerminalSlot 이 마운트되고 agentId prop 이 전달된다', () => {
    const agentId = 'aaaa-bbbb-cccc-dddd'
    seedAgents(agentInfo(agentId, false)) // FIX 1: caps 도착(비structured) → TerminalSlot 분기
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

  it('agent_id 있는 slot(caps 도착) 래퍼에는 중앙정렬 flex 가 없다(터미널 레이아웃 오염 방지)', () => {
    seedAgents(agentInfo('some-agent-id', false)) // caps 도착 → 구체 렌더러(hasContent=true)
    render(<ViewLayoutRenderer node={slotNode('s1', 'some-agent-id')} focusedSlotId={null} />)
    const wrapper = document.querySelector('[data-slot-id="s1"]') as HTMLElement
    // agent 있을 때 justifyContent: center 가 없어야 한다 — TerminalSlot 을 center 로 밀면 출력이 깨짐.
    expect(wrapper.style.justifyContent).not.toBe('center')
    expect(wrapper.style.alignItems).not.toBe('center')
  })

  // ── FIX 1(ADR-0041 replay 소유권): caps 도착 전엔 구체 렌더러를 마운트하지 않는다 ──────────────
  it('agent 배정됐지만 store 에 AgentInfo 없음 → "에이전트 연결 중…" 플레이스홀더(TerminalSlot/RichSlot 없음)', () => {
    // store 를 비워 두면(=caps 미도착) 스왑 시 replay 유실을 피하려 중립 플레이스홀더만 떠야 한다.
    render(<ViewLayoutRenderer node={slotNode('s1', 'not-in-store')} focusedSlotId={null} />)
    expect(screen.getByText('에이전트 연결 중…')).toBeTruthy()
    expect(screen.queryByTestId('terminal-slot')).toBeNull()
    expect(screen.queryByTestId('rich-slot')).toBeNull()
  })

  it('agent 가 store 에 있고 structured caps → RichSlot(TerminalSlot 없음)', () => {
    const agentId = 'struct-agent'
    seedAgents(agentInfo(agentId, true)) // structured=true → 라이브 RichSlot 분기
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    expect(screen.getByTestId('rich-slot')).toBeTruthy()
    expect(screen.queryByTestId('terminal-slot')).toBeNull()
    expect(screen.queryByText('에이전트 연결 중…')).toBeNull()
  })

  // ── RenderMode 기본 유도(defaultRenderMode): 오버라이드 없을 때 caps 로 렌더러가 정해진다 ──────────
  it('오버라이드 없음 + structured=true caps → 기본 유도로 RichSlot(TerminalSlot 없음)', () => {
    const agentId = 'derive-rich'
    seedAgents(agentInfo(agentId, true)) // defaultRenderMode → 'rich'
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    expect(screen.getByTestId('rich-slot')).toBeTruthy()
    expect(screen.queryByTestId('terminal-slot')).toBeNull()
  })

  it('오버라이드 없음 + structured=false caps → 기본 유도로 TerminalSlot(RichSlot 없음)', () => {
    const agentId = 'derive-terminal'
    seedAgents(agentInfo(agentId, false)) // defaultRenderMode → 'terminal'
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    expect(screen.getByTestId('terminal-slot')).toBeTruthy()
    expect(screen.queryByTestId('rich-slot')).toBeNull()
  })

  // ── 오버라이드가 기본을 이긴다(setRenderMode) ────────────────────────────────────────────────
  it('setRenderMode(id,"terminal")는 structured 기본(rich)을 덮어 TerminalSlot 을 마운트한다', () => {
    const agentId = 'override-terminal'
    seedAgents(agentInfo(agentId, true)) // 기본은 rich 지만 오버라이드가 이긴다
    useViewStore.getState().setRenderMode('s1', 'terminal')
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    expect(screen.getByTestId('terminal-slot')).toBeTruthy()
    expect(screen.queryByTestId('rich-slot')).toBeNull()
  })

  it('setRenderMode(id,"rich")는 비structured 기본(terminal)을 덮어 RichSlot 을 마운트한다', () => {
    const agentId = 'override-rich'
    seedAgents(agentInfo(agentId, false)) // 기본은 terminal 이지만 오버라이드가 이긴다
    useViewStore.getState().setRenderMode('s1', 'rich')
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    expect(screen.getByTestId('rich-slot')).toBeTruthy()
    expect(screen.queryByTestId('terminal-slot')).toBeNull()
  })

  // ── DOM 오버라이드(§5 관측): caps 기본 렌더러보다 우선, caps-ready 게이팅은 유지 ──────────────
  it('renderModeOverride=dom 인 slot(caps 도착) → DomSlot 이 마운트되고 Terminal/Rich 는 없다', () => {
    const agentId = 'dom-agent'
    seedAgents(agentInfo(agentId, false)) // 비structured(터미널 기본)라도 DOM 모드가 우선해야 한다
    useViewStore.getState().setRenderMode('s1', 'dom')
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    const dom = screen.getByTestId('dom-slot')
    expect(dom).toBeTruthy()
    expect(dom.getAttribute('data-agent-id')).toBe(agentId)
    expect(screen.queryByTestId('terminal-slot')).toBeNull()
    expect(screen.queryByTestId('rich-slot')).toBeNull()
  })

  it('renderModeOverride=dom 은 structured caps 기본(rich)보다 우선(DomSlot, RichSlot 아님)', () => {
    const agentId = 'dom-struct-agent'
    seedAgents(agentInfo(agentId, true)) // structured=true(기본은 RichSlot)여도 DOM 모드가 우선
    useViewStore.getState().setRenderMode('s1', 'dom')
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    expect(screen.getByTestId('dom-slot')).toBeTruthy()
    expect(screen.queryByTestId('rich-slot')).toBeNull()
  })

  it('renderModeOverride=dom 이라도 caps 미도착 → DomSlot 안 뜨고 "에이전트 연결 중…"(replay 게이팅 유지)', () => {
    // caps 미도착이면 오버라이드가 있어도 구체 렌더러를 마운트하지 않는다(스왑 전 바이트 유실 방지 — replay 소유권).
    useViewStore.getState().setRenderMode('s1', 'dom')
    render(<ViewLayoutRenderer node={slotNode('s1', 'not-in-store')} focusedSlotId={null} />)
    expect(screen.queryByTestId('dom-slot')).toBeNull()
    expect(screen.getByText('에이전트 연결 중…')).toBeTruthy()
  })

  it('clearRenderMode(id)로 오버라이드 해제 시 caps 유도 기본으로 복귀한다', () => {
    const agentId = 'clear-agent'
    seedAgents(agentInfo(agentId, false)) // 기본 = terminal
    useViewStore.getState().setRenderMode('s1', 'dom')
    useViewStore.getState().clearRenderMode('s1') // 해제 → 기본(terminal)으로 복귀
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    expect(screen.getByTestId('terminal-slot')).toBeTruthy()
    expect(screen.queryByTestId('dom-slot')).toBeNull()
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

  it('split 자식에 agent_id 있으면(caps 도착) 해당 슬롯에만 TerminalSlot 이 마운트된다', () => {
    const agentId = 'zzzz-agent'
    seedAgents(agentInfo(agentId, false))
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

// ── ★우클릭 컨텍스트 메뉴(§5, ADR-0035)★ ─────────────────────────────────────────────────
// ★이 스위트가 실제로 막는 것★: 캔버스 슬롯 우클릭 → SlotContextMenu 마운트 + 그 메뉴 액션이
// viewStore(=window.__engramLayout 이 노출하는 것과 동일 함수)로 (activeViewId, slotId) 좌표를 흘리는지.
// (Brick 1 에서 옛 LayoutRenderer→SlotPane 래핑 경로가 삭제돼 메뉴가 캔버스에서 닿지 않던 갭의 회귀 안전망.)
//
// 전략: split/closeSlot/assignAgent 를 store 에 spy 로 주입(SlotContextMenu 는 useViewStore(s=>s.split) 로
// 이 함수들을 읽는다 → window.__engramLayout 이 부르는 것과 물리적으로 동일). '에이전트 생성'은
// window.prompt + agentClient.spawnAgent(hoisted mock)를 거쳐 assignAgent 로 이어지므로 둘을 stub.
describe('ViewLayoutRenderer — 우클릭 컨텍스트 메뉴(§5 단일 제어 표면)', () => {
  const ACTIVE_VIEW = 'active-view-9'
  const splitSpy = vi.fn(async () => 'new-slot')
  const closeSlotSpy = vi.fn(async () => undefined)
  const assignAgentSpy = vi.fn(async () => undefined)
  const moveSlotToWindowSpy = vi.fn(async () => ({ window: 'slot-popup-1', tab: 'v-new' }))
  const origHash = window.location.hash

  beforeEach(() => {
    splitSpy.mockClear()
    closeSlotSpy.mockClear()
    assignAgentSpy.mockClear()
    moveSlotToWindowSpy.mockClear()
    clientMock.spawnAgent.mockClear()
    clientMock.killAgent.mockClear()
    // ★탭 소유 모델(ADR-0057)★: SlotContextMenu 는 viewIdOverride 없으면 useCurrentViewId()(이 웹뷰 창의
    //   active 탭)로 폴백한다. 메인 창(#/) 컨텍스트로 두고 windows["main"].active=ACTIVE_VIEW 를 주입 →
    //   폴백 경로가 ACTIVE_VIEW 를 집는다. 레이아웃 액션도 store 에 주입(단일 표면).
    window.location.hash = '#/'
    useViewStore.setState({
      windows: { main: { tabs: [{ id: ACTIVE_VIEW, name: 'View' }], active: ACTIVE_VIEW, version: 1 } },
      split: splitSpy,
      closeSlot: closeSlotSpy,
      assignAgent: assignAgentSpy,
      moveSlotToWindow: moveSlotToWindowSpy,
    })
  })

  afterEach(() => {
    window.location.hash = origHash
  })

  /** 슬롯 우클릭 → 메뉴 오픈. 대상 슬롯 wrapper 를 반환. viewIdOverride 를 넘기면 그 view 로 렌더한다(Fix 3). */
  function openMenu(
    slotId: string,
    agentId: string | null,
    viewIdOverride?: string | null,
  ): HTMLElement {
    render(
      <ViewLayoutRenderer
        node={slotNode(slotId, agentId)}
        focusedSlotId={null}
        viewIdOverride={viewIdOverride}
      />,
    )
    const wrapper = document.querySelector(`[data-slot-id="${slotId}"]`) as HTMLElement
    fireEvent.contextMenu(wrapper)
    return wrapper
  }

  it('빈 슬롯 우클릭 → SlotContextMenu 항목들이 뜬다(캔버스에서 도달 가능)', () => {
    openMenu('s1', null)
    // 메뉴 대표 항목들이 렌더된다 = 메뉴가 캔버스에 마운트됨(Brick 1 갭 메움 검증).
    expect(screen.getByText('가로 분할')).toBeTruthy()
    expect(screen.getByText('세로 분할')).toBeTruthy()
    expect(screen.getByText('닫기')).toBeTruthy()
    expect(screen.getByText('에이전트 생성')).toBeTruthy()
  })

  it('우클릭 전에는 메뉴가 없다(preventDefault 후 상태 기반 마운트)', () => {
    render(<ViewLayoutRenderer node={slotNode('s1', null)} focusedSlotId={null} />)
    expect(screen.queryByText('가로 분할')).toBeNull()
  })

  it('"가로 분할" → split(activeViewId, slotId, "horizontal") 호출(§5 __engramLayout 경로)', () => {
    openMenu('slot-A', null)
    fireEvent.click(screen.getByText('가로 분할'))
    expect(splitSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-A', 'horizontal')
  })

  it('"세로 분할" → split(activeViewId, slotId, "vertical") 호출', () => {
    openMenu('slot-B', null)
    fireEvent.click(screen.getByText('세로 분할'))
    expect(splitSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-B', 'vertical')
  })

  it('"닫기" → closeSlot(activeViewId, slotId) 호출', () => {
    openMenu('slot-C', null)
    fireEvent.click(screen.getByText('닫기'))
    expect(closeSlotSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-C')
  })

  it('"에이전트 생성" → spawnAgent 후 assignAgent(activeViewId, slotId, 새 agentId) 호출', async () => {
    clientMock.spawnAgent.mockResolvedValueOnce({ id: 'brand-new-agent' })
    const promptSpy = vi.spyOn(window, 'prompt').mockReturnValue('C:/work')
    openMenu('slot-D', null)
    fireEvent.click(screen.getByText('에이전트 생성'))
    // spawnAgent 는 prompt 로 받은 cwd(trim)로 불린다.
    expect(clientMock.spawnAgent).toHaveBeenCalledWith('C:/work')
    // spawn 이 resolve 된 뒤 assignAgent 가 그 agentId 로 이 슬롯에 배정한다(마이크로태스크 flush 대기).
    await vi.waitFor(() =>
      expect(assignAgentSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-D', 'brand-new-agent'),
    )
    promptSpy.mockRestore()
  })

  it('agent 배정 슬롯 우클릭 → "에이전트 종료" 클릭 시 killAgent(그 agentId) 호출', () => {
    seedAgents(agentInfo('assigned-agent', false)) // 종료 항목 활성 조건 = store 에 그 agent 존재
    openMenu('slot-E', 'assigned-agent')
    fireEvent.click(screen.getByText('에이전트 종료'))
    expect(clientMock.killAgent).toHaveBeenCalledWith('assigned-agent')
  })

  it('비활성 "에이전트 종료"(store 에 agent 없음) 클릭 → killAgent 를 부르지 않는다(enabled 가드)', () => {
    // agents store 를 비워 두면(=배정 agentId 가 실행중 목록에 없음) 종료 항목이 흐려지고 비활성이어야 한다.
    // 시각만 흐리고 action 은 그대로 도는 버그의 회귀 안전망 — 비활성 항목 클릭은 no-op 여야 한다.
    openMenu('slot-F', 'gone-agent') // store 에 'gone-agent' 없음 → 종료 항목 disabled
    fireEvent.click(screen.getByText('에이전트 종료'))
    expect(clientMock.killAgent).not.toHaveBeenCalled()
  })

  // ── ★슬롯 팝업 분리(pop-out) 메뉴★ — enabled 가드('에이전트 종료'와 동일) + 올바른 좌표로 invoke ──
  it('agent 배정 슬롯 우클릭 → "팝업으로 분리" 클릭 시 moveSlotToWindow(activeViewId, slotId) 호출', () => {
    seedAgents(agentInfo('live-agent', false)) // 활성 조건 = store 에 그 agent 존재(라이브)
    openMenu('slot-P', 'live-agent')
    fireEvent.click(screen.getByText('팝업으로 분리'))
    // §5: window.__engramLayout.moveSlotToWindow 와 동일 store 함수를 (activeViewId, slotId)로 부른다.
    expect(moveSlotToWindowSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-P')
  })

  it('빈 슬롯(agent 미배정) "팝업으로 분리" 클릭 → moveSlotToWindow 를 부르지 않는다(enabled 가드)', () => {
    // agent 없는 슬롯은 분리 대상이 없으므로 항목이 흐려지고 비활성이어야 한다(클릭 no-op).
    openMenu('slot-Q', null)
    fireEvent.click(screen.getByText('팝업으로 분리'))
    expect(moveSlotToWindowSpy).not.toHaveBeenCalled()
  })

  it('배정됐지만 store 에 없는 agent(죽음) "팝업으로 분리" → moveSlotToWindow 안 부름(에이전트 종료와 동일 가드)', () => {
    // slotAgentId 는 있으나 실행중 목록에 없음 → hasLiveAgent=false → 비활성.
    openMenu('slot-R', 'dead-agent') // store 비어 있음
    fireEvent.click(screen.getByText('팝업으로 분리'))
    expect(moveSlotToWindowSpy).not.toHaveBeenCalled()
  })

  // ── ★Fix 3: viewIdOverride 스레딩★ — 팝업 창 경로는 activeViewId(=main) 대신 넘겨받은 view 로 액션한다 ──
  // 이 스위트가 막는 것: PopoutPage → ViewLayoutRenderer → SlotContextMenu 로 viewId 오버라이드가 흘러
  // 팝업 안 분할/닫기/pop-out 이 엉뚱한 main view 가 아니라 자기 팝업 view 좌표를 쓰는지(SlotNotFound·오변형 방지).
  const POPUP_VIEW = 'popup-view-77'

  it('viewIdOverride 있으면 "가로 분할"이 activeViewId 가 아니라 오버라이드 view 로 split 을 부른다', () => {
    openMenu('slot-po', null, POPUP_VIEW)
    fireEvent.click(screen.getByText('가로 분할'))
    // 좌표의 view = 오버라이드(POPUP_VIEW), main 의 ACTIVE_VIEW 가 아님.
    expect(splitSpy).toHaveBeenCalledWith(POPUP_VIEW, 'slot-po', 'horizontal')
    expect(splitSpy).not.toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-po', 'horizontal')
  })

  it('viewIdOverride 있으면 "닫기"가 오버라이드 view 로 closeSlot 을 부른다', () => {
    openMenu('slot-pc', null, POPUP_VIEW)
    fireEvent.click(screen.getByText('닫기'))
    expect(closeSlotSpy).toHaveBeenCalledWith(POPUP_VIEW, 'slot-pc')
  })

  it('viewIdOverride 있으면 "팝업으로 분리"가 오버라이드 view 로 moveSlotToWindow 를 부른다', () => {
    seedAgents(agentInfo('po-agent', false)) // 활성 조건
    openMenu('slot-pp', 'po-agent', POPUP_VIEW)
    fireEvent.click(screen.getByText('팝업으로 분리'))
    expect(moveSlotToWindowSpy).toHaveBeenCalledWith(POPUP_VIEW, 'slot-pp')
  })

  it('viewIdOverride 없으면(메인 창 경로) 종전대로 activeViewId 로 폴백한다(하위호환)', () => {
    openMenu('slot-main', null) // 오버라이드 안 넘김
    fireEvent.click(screen.getByText('가로 분할'))
    expect(splitSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-main', 'horizontal')
  })
})
