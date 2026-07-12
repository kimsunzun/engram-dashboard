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
const agentStoreState = vi.hoisted(() => ({ agents: [] as unknown[], presets: [] as unknown[] }))
vi.mock('../../store/agentStore', () => ({
  useAgentStore: Object.assign(
    (selector: (s: typeof agentStoreState) => unknown) => selector(agentStoreState),
    { getState: () => agentStoreState },
  ),
}))

// ── 네이티브 폴더 다이얼로그 stub(ADR-0064) — slot.createAgentHere / preset.add / agentlist.createAgent 가
//    @tauri-apps/plugin-dialog open 을 부른다. 테스트마다 반환(경로/null)을 갈아끼운다. ──
const dialogMock = vi.hoisted(() => ({ open: vi.fn(async () => null as string | null) }))
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: (...args: unknown[]) => dialogMock.open(...(args as [])),
}))

// ── allotment stub — split 분기 렌더 시 jsdom 환경에서 ResizeObserver 에러 방지 ──
// Allotment / Allotment.Pane 을 단순 div 로 대체해 자식을 그대로 렌더한다.
// vi.mock factory 는 호이스팅되므로 React import 를 직접 쓸 수 없다 — importOriginal 패턴으로 우회.
// preferredSize(=ratio 파생 초기 사이징 %, ADR-0063)를 Pane 의 data 속성으로 노출해 테스트가 단언할 수 있게
// 한다. ★Allotment 의 defaultSizes 는 비율이 아니라 픽셀이라 [0.2,0.8]=0.2px/0.8px 로 붕괴한다 — 대신
//   첫 Pane 에 preferredSize="20%"(퍼센트 문자열)로 준다(실측 스샷 회귀 수정).
// ★Bug2 key 안정성 관측★: React key 는 props 로 새 나오지 않아 DOM 에서 직접 못 읽는다. 대신 Pane 이
//   마운트마다 유일 인스턴스 id 를 만들어(useRef + 모듈 카운터) data-pane-instance 로 노출한다 —
//   key 가 바뀌어 remount 되면 새 id 가, key 가 안정하면 같은 id 가 유지된다. 콘텐츠 재구조화(slot→중첩
//   split) 리렌더 후 같은 인스턴스 id 면 = Pane 이 마운트 유지 = key 안정 = Allotment 가 사이즈 보존.
let paneInstanceCounter = 0
vi.mock('allotment', async () => {
  const React = (await import('react')).default
  const Pane = ({ children, preferredSize }: { children: React.ReactNode; preferredSize?: number | string }) => {
    const instance = React.useRef<number | null>(null)
    if (instance.current === null) instance.current = ++paneInstanceCounter
    return React.createElement(
      'div',
      {
        'data-testid': 'allotment-pane',
        // pane 초기 사이징(예: "20%") — split 렌더 테스트가 첫 pane 에서 읽어 단언.
        'data-preferred-size': preferredSize != null ? String(preferredSize) : undefined,
        // 마운트별 유일 id — remount 시 증가. key 안정성 테스트가 리렌더 전후로 비교.
        'data-pane-instance': String(instance.current),
      },
      children,
    )
  }
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

// ── PresetPalette stub(ADR-0060/0061) — 프리셋 CRUD 배선 없이 preset_palette variant 마운트 여부만 확인 ──
vi.mock('../slot/PresetPalette', () => ({
  default: () => <div data-testid="preset-palette" />,
}))
// ── AgentList stub(ADR-0060/0062) — agent_list variant 마운트 여부만 확인(내부 배선은 AgentList.test 담당) ──
vi.mock('../agent/AgentList', () => ({
  default: () => <div data-testid="agent-list" />,
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
// ADR-0064: 통합 메뉴는 buildSlotMenu(content.type) 로 command 참조를 resolve 한다 → command·기여가
//   레지스트리에 등록돼 있어야 한다. 매니페스트를 side-effect import 해 부팅과 동일하게 등록한다.
import '../../commands/contributions'
import ViewLayoutRenderer from './ViewLayoutRenderer'
import type { LayoutNode, SlotContent } from '../../api/layoutTypes'
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

function splitNode(a: LayoutNode, b: LayoutNode, ratio = 0.5): LayoutNode {
  return { type: 'split', dir: 'horizontal', ratio, a, b }
}

/** SlotContent variant 를 직접 지정하는 슬롯 노드(preset_palette / agent_list 분기 검증용, ADR-0060). */
function contentSlotNode(id: string, content: SlotContent): LayoutNode {
  return { type: 'slot', id, content }
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

  it('focusedSlotId == node.id → 포커스 인디케이터(inset box-shadow, accent 65%)가 적용된다', () => {
    render(<ViewLayoutRenderer node={slotNode('s1', null)} focusedSlotId="s1" />)
    const wrapper = document.querySelector('[data-slot-id="s1"]') as HTMLElement
    expect(wrapper).toBeTruthy()
    // ADR-0065(focus-ring): border 폭은 항상 1px(layout shift 없음), 포커스는 inset box-shadow 로 표시.
    // box-shadow 에 accent 가 포함되고 border 에는 포함되지 않아야 한다.
    expect(wrapper.style.boxShadow).toContain('accent')
    expect(wrapper.style.border).toContain('border')
    expect(wrapper.style.border).not.toContain('accent')
  })

  it('focusedSlotId != node.id → 비포커스: border=var(--border), box-shadow=none', () => {
    render(<ViewLayoutRenderer node={slotNode('s1', null)} focusedSlotId="s-other" />)
    const wrapper = document.querySelector('[data-slot-id="s1"]') as HTMLElement
    expect(wrapper.style.border).toContain('border')
    expect(wrapper.style.border).not.toContain('accent')
    // 비포커스 시 box-shadow 는 none
    expect(wrapper.style.boxShadow).toBe('none')
  })

  // ── ADR-0060/0061/0062: preset_palette·agent_list variant → 각 실 렌더러 마운트(hasContent=true) ──
  it('content.type=preset_palette slot → PresetPalette 가 마운트된다', () => {
    render(<ViewLayoutRenderer node={contentSlotNode('s1', { type: 'preset_palette' })} focusedSlotId={null} />)
    expect(screen.getByTestId('preset-palette')).toBeTruthy()
    // 프리셋 팔레트는 실 콘텐츠(hasContent=true) — 중앙정렬 flex 가 없어야 팔레트 레이아웃이 안 깨진다.
    const wrapper = document.querySelector('[data-slot-id="s1"]') as HTMLElement
    expect(wrapper.style.justifyContent).not.toBe('center')
  })

  it('content.type=agent_list slot(Slice C) → AgentList 가 마운트된다(hasContent=true)', () => {
    render(<ViewLayoutRenderer node={contentSlotNode('s1', { type: 'agent_list' })} focusedSlotId={null} />)
    expect(screen.getByTestId('agent-list')).toBeTruthy()
    // empty 플레이스홀더가 아니라 실 렌더러 — 중앙정렬 flex 없어야 목록 레이아웃이 안 깨진다.
    expect(screen.queryByText('— empty —')).toBeNull()
    const wrapper = document.querySelector('[data-slot-id="s1"]') as HTMLElement
    expect(wrapper.style.justifyContent).not.toBe('center')
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

  // ── ★ADR-0063: node.ratio → 첫 Pane 의 preferredSize % 초기 사이징★ ──────────────────────────────
  // 이 스위트가 막는 것: split 렌더러가 node.ratio 를 첫 pane(a=왼/위)의 preferredSize="<pct>%" 로 넘겨
  // 부팅 레이아웃 narrow-left(0.2)가 실제로 20/80 으로 뜨는지(50/50 무시 + defaultSizes-px-붕괴 회귀 안전망).
  // 드래그→백엔드 되쓰기는 이 슬라이스 밖(초기 사이징만).
  it('split(ratio=0.2) → 첫 pane preferredSize="20%" 로 초기 사이징이 전달된다', () => {
    const node = splitNode(slotNode('s1', null), slotNode('s2', null), 0.2)
    render(<ViewLayoutRenderer node={node} focusedSlotId={null} />)
    // 첫 pane(a=왼)에 ratio 파생 퍼센트가 붙는다. b(오)는 나머지 채움(preferredSize 없음).
    const firstPane = screen.getAllByTestId('allotment-pane')[0]
    expect(firstPane.getAttribute('data-preferred-size')).toBe('20%')
  })

  it('split(ratio=0.5) → 첫 pane preferredSize="50%" (기존 50/50 스플릿은 그대로 유지)', () => {
    const node = splitNode(slotNode('s1', null), slotNode('s2', null)) // 기본 ratio=0.5
    render(<ViewLayoutRenderer node={node} focusedSlotId={null} />)
    const firstPane = screen.getAllByTestId('allotment-pane')[0]
    expect(firstPane.getAttribute('data-preferred-size')).toBe('50%')
  })

  // ── ★Bug2: Allotment.Pane key 안정화 — 형제 콘텐츠 재구조화에도 pane 이 remount 되지 않는다★ ──────
  // 이 스위트가 막는 것: 옛 nodeKey(node.b) 파생 key 는 b pane 안 슬롯이 split 으로 재구조화되면 key 가
  // 바뀌어 pane 이 unmount+remount → Allotment 가 전 pane 을 균등 재분배 → 형제(a=왼 20%)의 비율 소실.
  // 위치 기반 안정 key("pane-a"/"pane-b")면 pane 이 마운트 유지(같은 인스턴스 id) → 사이즈 보존.
  it('b pane 콘텐츠가 slot→중첩 split 으로 재구조화돼도 두 pane 인스턴스 id 가 유지된다(remount 없음)', () => {
    // 초기: 왼(a=20%) slot + 오(b) slot.
    const initial = splitNode(slotNode('left', null), slotNode('right', null), 0.2)
    const { rerender } = render(<ViewLayoutRenderer node={initial} focusedSlotId={null} />)
    // 최상위 Allotment 의 직속 두 pane(중첩 것 제외) — 첫 Allotment 자식만 집는다.
    const outerPanesBefore = topLevelPanes()
    expect(outerPanesBefore).toHaveLength(2)
    const [aBefore, bBefore] = outerPanesBefore.map(p => p.getAttribute('data-pane-instance'))
    const preferredBefore = outerPanesBefore[0].getAttribute('data-preferred-size')

    // b(오) 슬롯이 다른 곳에서 split 돼 중첩 split 이 됨(=b 서브트리 재구조화). a(왼)는 그대로.
    const restructured = splitNode(
      slotNode('left', null),
      splitNode(slotNode('right', null), slotNode('right-2', null)),
      0.2,
    )
    rerender(<ViewLayoutRenderer node={restructured} focusedSlotId={null} />)

    const outerPanesAfter = topLevelPanes()
    const [aAfter, bAfter] = outerPanesAfter.map(p => p.getAttribute('data-pane-instance'))
    // ★핵심 단언★: 두 최상위 pane 인스턴스 id 가 리렌더 전후로 동일 = key 안정 = remount 없음.
    expect(aAfter).toBe(aBefore)
    expect(bAfter).toBe(bBefore)
    // preferredSize(=왼 20%)도 첫 pane 에 그대로.
    expect(outerPanesAfter[0].getAttribute('data-preferred-size')).toBe(preferredBefore)
    expect(preferredBefore).toBe('20%')
  })

  /** 최상위 Allotment 의 직속 Pane 두 개만(중첩 Allotment 의 pane 은 제외). */
  function topLevelPanes(): HTMLElement[] {
    const outer = screen.getAllByTestId('allotment')[0]
    return Array.from(outer.children).filter(
      c => (c as HTMLElement).getAttribute('data-testid') === 'allotment-pane',
    ) as HTMLElement[]
  }
})

// ── ★click-to-focus 게이트(제어 슬롯 포커스 제외 — ADR-0066 정제)★ ─────────────────────────────
// ★이 스위트가 막는 것★: 트리(agent_list)·팔레트(preset_palette) 슬롯 pane 클릭이 focusSlot 을 부르면
// 안 된다(작업 슬롯이 아니라 포커스 대상 아님). 이어지는 우클릭 "열기"가 그 제어 슬롯을 대상으로 잡아
// 트리를 에이전트 터미널로 덮어쓰던 선존 UX 버그의 뿌리. 콘텐츠 슬롯(empty/agent)은 기존대로 focusSlot 호출.
//
// 전략: real viewStore 에 focusSlot spy 를 주입하고(사람 클릭 = LLM = 단일 표면), windows["main"].active 를
//   채워 targetViewId 폴백이 성립하게 한다(컨텍스트 메뉴 스위트와 동형 세팅).
describe('ViewLayoutRenderer — click-to-focus 게이트(제어 슬롯 포커스 제외)', () => {
  const FOCUS_VIEW = 'focus-view-1'
  const focusSlotSpy = vi.fn(async () => undefined)
  const origHash = window.location.hash

  beforeEach(() => {
    focusSlotSpy.mockClear()
    window.location.hash = '#/'
    useViewStore.setState({
      windows: { main: { tabs: [{ id: FOCUS_VIEW, name: 'View' }], active: FOCUS_VIEW, version: 1 } },
      focusSlot: focusSlotSpy,
    })
  })
  afterEach(() => {
    window.location.hash = origHash
  })

  function clickSlot(content: SlotContent): void {
    render(<ViewLayoutRenderer node={contentSlotNode('s1', content)} focusedSlotId={null} />)
    fireEvent.click(document.querySelector('[data-slot-id="s1"]') as HTMLElement)
  }

  it('empty 슬롯 클릭 → focusSlot(viewId, slotId) 호출(콘텐츠 슬롯 = 포커스 대상)', () => {
    clickSlot({ type: 'empty' })
    expect(focusSlotSpy).toHaveBeenCalledWith(FOCUS_VIEW, 's1')
  })

  it('agent 슬롯 클릭 → focusSlot 호출(콘텐츠 슬롯)', () => {
    seedAgents(agentInfo('a-focus', false)) // caps 도착(무해 — 게이트는 content.type 만 본다)
    render(<ViewLayoutRenderer node={contentSlotNode('s1', { type: 'agent', agent_id: 'a-focus' })} focusedSlotId={null} />)
    fireEvent.click(document.querySelector('[data-slot-id="s1"]') as HTMLElement)
    expect(focusSlotSpy).toHaveBeenCalledWith(FOCUS_VIEW, 's1')
  })

  it('agent_list(트리) 슬롯 클릭 → focusSlot 미호출(제어 슬롯 = 포커스 제외)', () => {
    clickSlot({ type: 'agent_list' })
    expect(focusSlotSpy).not.toHaveBeenCalled()
  })

  it('preset_palette(팔레트) 슬롯 클릭 → focusSlot 미호출(제어 슬롯 = 포커스 제외)', () => {
    clickSlot({ type: 'preset_palette' })
    expect(focusSlotSpy).not.toHaveBeenCalled()
  })
})

// ── ★우클릭 통합 컨텍스트 메뉴(§5, ADR-0064)★ ─────────────────────────────────────────────────
// ★이 스위트가 실제로 막는 것★: 캔버스 슬롯 우클릭 → 통합 SlotContextMenu 마운트(buildSlotMenu(content.type)
// 산출) + 각 항목 클릭이 그 command.run(ctx) 를 통해 viewStore/agentClient 로 (viewId, slotId, agentId)를
// 흘리는지. 메뉴 항목 = command id 참조(ADR-0064) — 콘텐츠 전용(에이전트 종료·트리/팔레트 열기·생성) +
// 공통 '*'(가로/세로 분할·팝업 분리·비우기·닫기)가 한 메뉴에 공존한다.
//
// 전략: split/closeSlot/assignAgent/setSlotContent/moveSlotToWindow 를 real viewStore 에 spy 로 주입한다
// (command 는 useViewStore.getState().split(...) 로 이들을 부른다 → LLM/__engramCmd 와 물리적으로 동일).
// '에이전트 생성'(slot.createAgentHere)은 폴더 다이얼로그(open, hoisted mock) → agentClient.spawnAgent →
// assignAgent, '에이전트 종료'(agent.kill)는 agentClient.killAgent 로 이어진다.
describe('ViewLayoutRenderer — 우클릭 컨텍스트 메뉴(§5 단일 제어 표면)', () => {
  const ACTIVE_VIEW = 'active-view-9'
  const splitSpy = vi.fn(async () => 'new-slot')
  const closeSlotSpy = vi.fn(async () => undefined)
  const assignAgentSpy = vi.fn(async () => undefined)
  const setSlotContentSpy = vi.fn(async () => undefined)
  const moveSlotToWindowSpy = vi.fn(async () => ({ window: 'slot-popup-1', tab: 'v-new' }))
  const origHash = window.location.hash

  beforeEach(() => {
    splitSpy.mockClear()
    closeSlotSpy.mockClear()
    assignAgentSpy.mockClear()
    setSlotContentSpy.mockClear()
    moveSlotToWindowSpy.mockClear()
    clientMock.spawnAgent.mockClear()
    clientMock.killAgent.mockClear()
    dialogMock.open.mockClear()
    dialogMock.open.mockResolvedValue(null)
    // ★탭 소유 모델(ADR-0057)★: ViewLayoutRenderer 는 viewIdOverride 없으면 useCurrentViewId()(이 웹뷰 창의
    //   active 탭)로 폴백해 ctx.viewId 를 채운다. 메인 창(#/) 컨텍스트로 두고 windows["main"].active=ACTIVE_VIEW
    //   를 주입 → 폴백 경로가 ACTIVE_VIEW 를 집는다. 레이아웃 액션도 store 에 주입(단일 표면).
    window.location.hash = '#/'
    useViewStore.setState({
      windows: { main: { tabs: [{ id: ACTIVE_VIEW, name: 'View' }], active: ACTIVE_VIEW, version: 1 } },
      split: splitSpy,
      closeSlot: closeSlotSpy,
      assignAgent: assignAgentSpy,
      setSlotContent: setSlotContentSpy,
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

  const POPUP_VIEW = 'popup-view-77'

  /** ADR-0065: 빈 슬롯 fill-ops 는 "새 콘텐츠" 컨테이너로 접혔다 — hover 로 flyout 을 펴야 자식이 보인다. */
  function openNewContentFlyout(): void {
    fireEvent.mouseEnter(screen.getByText('새 콘텐츠'))
  }

  it('빈 슬롯 우클릭 → 최상위 = 에이전트 모니터링 + 새 콘텐츠(컨테이너) + 공통 슬롯 ops, 채움은 flyout 안 (ADR-0067/0065)', () => {
    openMenu('s1', null)
    // 최상위: 에이전트 모니터링(ADR-0067) + 콘텐츠 컨테이너 + 공통 slot-ops(단, empty 는 popout/비우기 트림).
    expect(screen.getByText('에이전트 모니터링')).toBeTruthy()
    expect(screen.getByText('새 콘텐츠')).toBeTruthy()
    expect(screen.getByText('가로 분할')).toBeTruthy()
    expect(screen.getByText('세로 분할')).toBeTruthy()
    expect(screen.getByText('닫기')).toBeTruthy()
    // ADR-0065 트림: 빈 슬롯엔 비우기/팝업으로 분리 없음(hideOn:['empty']).
    expect(screen.queryByText('비우기')).toBeNull()
    expect(screen.queryByText('팝업으로 분리')).toBeNull()
    // 채움 항목은 hover 전엔 미노출(서브메뉴 안).
    expect(screen.queryByText('에이전트 트리 열기')).toBeNull()
    openNewContentFlyout()
    expect(screen.getByText('에이전트 트리 열기')).toBeTruthy()
    expect(screen.getByText('프리셋 팔레트 열기')).toBeTruthy()
    // ADR-0067: "에이전트 생성"은 서브메뉴에서 제거됐다(스폰 = 트리 소관).
    expect(screen.queryByText('에이전트 생성')).toBeNull()
  })

  it('우클릭 전에는 메뉴가 없다(preventDefault 후 상태 기반 마운트)', () => {
    render(<ViewLayoutRenderer node={slotNode('s1', null)} focusedSlotId={null} />)
    expect(screen.queryByText('가로 분할')).toBeNull()
  })

  it('"가로 분할" → split(viewId, slotId, "horizontal") 호출(§5 command 경로)', () => {
    openMenu('slot-A', null)
    fireEvent.click(screen.getByText('가로 분할'))
    expect(splitSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-A', 'horizontal')
  })

  it('"세로 분할" → split(viewId, slotId, "vertical") 호출', () => {
    openMenu('slot-B', null)
    fireEvent.click(screen.getByText('세로 분할'))
    expect(splitSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-B', 'vertical')
  })

  it('"닫기" → closeSlot(viewId, slotId) 호출', () => {
    openMenu('slot-C', null)
    fireEvent.click(screen.getByText('닫기'))
    expect(closeSlotSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-C')
  })

  // ── ★empty fill-ops(ADR-0063/0064/0065)★: "새 콘텐츠" flyout 안 → setSlotContent(view, slot, {type}) ──
  it('"에이전트 트리 열기"(flyout) → setSlotContent(viewId, slotId, {type:agent_list})', () => {
    openMenu('slot-T', null)
    openNewContentFlyout()
    fireEvent.click(screen.getByText('에이전트 트리 열기'))
    expect(setSlotContentSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-T', { type: 'agent_list' })
  })

  it('"프리셋 팔레트 열기"(flyout) → setSlotContent(viewId, slotId, {type:preset_palette})', () => {
    openMenu('slot-U', null)
    openNewContentFlyout()
    fireEvent.click(screen.getByText('프리셋 팔레트 열기'))
    expect(setSlotContentSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-U', { type: 'preset_palette' })
  })

  it('빈 슬롯엔 "비우기"가 없다(ADR-0065 hideOn:["empty"] 트림 — 이미 빈 슬롯 재비우기는 no-op)', () => {
    openMenu('slot-V', null)
    expect(screen.queryByText('비우기')).toBeNull()
  })

  it('viewIdOverride 있으면 "에이전트 트리 열기"(flyout)가 오버라이드 view 로 setSlotContent 를 부른다', () => {
    openMenu('slot-to', null, POPUP_VIEW)
    openNewContentFlyout()
    fireEvent.click(screen.getByText('에이전트 트리 열기'))
    expect(setSlotContentSpy).toHaveBeenCalledWith(POPUP_VIEW, 'slot-to', { type: 'agent_list' })
  })

  // ADR-0067: "에이전트 생성"(slot.createAgentHere)은 슬롯 콘텐츠-채움 메뉴에서 제거됐다(스폰 = 트리
  //   소관). command 정의는 남지만 이 메뉴 경로가 없어져 옛 flyout spawn 테스트 2개는 삭제했다 —
  //   command 직접 라우팅 회귀는 slotCommands.test.ts 가 계속 커버한다.

  // ── ★agent 슬롯: 콘텐츠 전용 "에이전트 종료" + 공통 ops 공존(ADR-0064)★ ──────────────────────
  it('agent 배정 슬롯 우클릭 → "에이전트 종료"(콘텐츠) 클릭 시 killAgent(그 agentId) 호출', () => {
    openMenu('slot-E', 'assigned-agent')
    fireEvent.click(screen.getByText('에이전트 종료'))
    expect(clientMock.killAgent).toHaveBeenCalledWith('assigned-agent')
  })

  it('agent 슬롯에도 공통 슬롯 ops(닫기·분할·팝업)가 함께 뜬다(공통 소실 버그 방지)', () => {
    openMenu('slot-E2', 'some-agent')
    expect(screen.getByText('에이전트 종료')).toBeTruthy() // 콘텐츠
    expect(screen.getByText('닫기')).toBeTruthy() // 공통
    expect(screen.getByText('팝업으로 분리')).toBeTruthy() // 공통(agent 게이팅 제거)
  })

  it('빈 슬롯 메뉴엔 "에이전트 종료"가 없다(agent 전용 콘텐츠 항목)', () => {
    openMenu('slot-empty-x', null)
    expect(screen.queryByText('에이전트 종료')).toBeNull()
  })

  // ── ★"팝업으로 분리" = 공통(ADR-0064)★: 콘텐츠 종류와 무관하게 뜨고 (viewId, slotId)로 move.
  //    단 ADR-0065 로 빈 슬롯에선 트림(hideOn:['empty']) — 비-empty(agent) 슬롯으로 라우팅을 검증한다. ──
  it('"팝업으로 분리"(공통, 비-empty) → moveSlotToWindow(viewId, slotId) 호출', () => {
    openMenu('slot-P', 'agent-p') // agent 슬롯 — 빈 슬롯은 popout 트림(ADR-0065)
    fireEvent.click(screen.getByText('팝업으로 분리'))
    expect(moveSlotToWindowSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-P')
  })

  // ── ★viewIdOverride 스레딩★ — 팝업 창 경로는 activeViewId(=main) 대신 넘겨받은 view 로 액션한다 ──
  // ViewLayoutRenderer 가 ctx.viewId = viewIdOverride ?? currentViewId 로 조립해 command.run 에 넘긴다.
  it('viewIdOverride 있으면 "가로 분할"이 오버라이드 view 로 split 을 부른다', () => {
    openMenu('slot-po', null, POPUP_VIEW)
    fireEvent.click(screen.getByText('가로 분할'))
    expect(splitSpy).toHaveBeenCalledWith(POPUP_VIEW, 'slot-po', 'horizontal')
    expect(splitSpy).not.toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-po', 'horizontal')
  })

  it('viewIdOverride 있으면 "닫기"가 오버라이드 view 로 closeSlot 을 부른다', () => {
    openMenu('slot-pc', null, POPUP_VIEW)
    fireEvent.click(screen.getByText('닫기'))
    expect(closeSlotSpy).toHaveBeenCalledWith(POPUP_VIEW, 'slot-pc')
  })

  it('viewIdOverride 있으면 "팝업으로 분리"가 오버라이드 view 로 moveSlotToWindow 를 부른다', () => {
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
