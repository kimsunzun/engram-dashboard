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
// killAgent 도 stub — SlotContextMenu 의 '에이전트 종료'(kill) 경로가 부른다.
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
    // 연결 상태 표면 — 슬롯이 부재 막 판정에 읽는다(등록 즉시 1회 통지 + disposer 반환).
    onConnectionStateChange: (cb: (s: string) => void) => {
      cb('down')
      return () => {}
    },
  },
  getAgentClient: vi.fn(),
}))

// ── agentStore stub — ViewLayoutRenderer 가 `useAgentStore(s => s.agents)` 로 caps 를 조회한다. ──
// FIX 1(ADR-0041): 렌더러 분기가 store 의 AgentInfo 유무·caps 에 의존하므로 테스트가 agents 를 제어할 수
// 있어야 한다. vi.hoisted 로 가변 holder 를 만들어 selector 에 흘린다(TerminalSlot/RichSlot 은 stub 이라
// 자기 useAgentStore 호출은 무해). afterEach 에서 초기화.
// ADR-0148: 세 갈래 게이트(명부 미수신 / 프로필 생존 / 프로필 부재)가 agentsLoaded·profiles 를 함께 본다.
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

// ── 네이티브 폴더 다이얼로그 stub(ADR-0064) — slot.createAgentHere / preset.add / agentlist.createAgent 가
//    @tauri-apps/plugin-dialog open 을 부른다. 테스트마다 반환(경로/null)을 갈아끼운다. ──
const dialogMock = vi.hoisted(() => ({ open: vi.fn(async () => null as string | null) }))
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: (...args: unknown[]) => dialogMock.open(...(args as [])),
}))

// ── allotment stub — split 분기 렌더 시 jsdom 환경에서 ResizeObserver 에러 방지 ──
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
        'data-preferred-size': preferredSize != null ? String(preferredSize) : undefined,
        'data-pane-instance': String(instance.current),
      },
      children,
    )
  }
  // ★vertical 노출(ADR-0140)★: dir → allotment 방향 매핑이 "유일한 진실 경계"라 테스트가 단언할 수 있게
  //   prop 을 data 속성으로 새 낸다(뒤집히면 메뉴·타입이 다 맞는데 화면만 반대가 되는 자리).
  const Allotment = Object.assign(
    ({ children, vertical }: { children: React.ReactNode; vertical?: boolean }) =>
      React.createElement(
        'div',
        { 'data-testid': 'allotment', 'data-vertical': String(vertical === true) },
        children,
      ),
    { Pane },
  )
  return { Allotment }
})

// ── 슬롯 stub 3종 — 실 구독/xterm 없이 마운트 여부·전달된 prop 만 확인 ─────────────────
// ★넘기는 prop 전량을 그린다★: 상위가 무엇을 내려보내는지가 이 렌더러의 계약이라, prop 을 그리지 않으면
//   드리프트가 DOM 에서 안 보인다. ★회차(epoch)를 여기 되살리면 그 값이 곧 재마운트 트리거가 되고,
//   종료로 명부에서 수거되는 순간 값이 떨어지며 보존하려던 대화가 지워진다★(아래 회귀 케이스).
vi.mock('../slot/TerminalSlot', () => ({
  default: ({ viewId, agentId }: { viewId: string; agentId: string }) => (
    <div data-testid="terminal-slot" data-agent-id={agentId} data-view-id={viewId} />
  ),
}))

vi.mock('../slot/RichSlot', () => ({
  default: ({ viewId, agentId }: { viewId?: string; agentId: string }) => (
    <div data-testid="rich-slot" data-agent-id={agentId} data-view-id={viewId} />
  ),
}))

vi.mock('../slot/DomSlot', () => ({
  default: ({ viewId, agentId }: { viewId: string; agentId: string }) => (
    <div data-testid="dom-slot" data-agent-id={agentId} data-view-id={viewId} />
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
// 메뉴 배치의 앵커 간격(ADR-0143 결정 4)은 SlotContextMenu 소관 — 여기선 좌표 기대값을 그 상수에서 파생시킨다.
import { ANCHOR_GAP } from '../slot/SlotContextMenu'
import type { LayoutNode, SlotContent, SplitDir } from '../../api/layoutTypes'
import type { AgentInfo, Capabilities } from '../../api/types'
import { useViewStore } from '../../store/viewStore'

afterEach(() => {
  cleanup()
  useViewStore.setState({ renderModeOverride: {} })
  agentStoreState.agents = []
  agentStoreState.agentsLoaded = false
  agentStoreState.profiles = []
  agentStoreState.profilesLoaded = false
})

// ── 헬퍼 ──────────────────────────────────────────────────────────────────────
function slotNode(id: string, agentId: string | null): LayoutNode {
  return {
    type: 'slot',
    id,
    content: agentId != null ? { type: 'agent', agent_id: agentId } : { type: 'empty' },
  }
}

function splitNode(a: LayoutNode, b: LayoutNode, ratio = 0.5, dir: SplitDir = 'left_right'): LayoutNode {
  return { type: 'split', dir, ratio, a, b }
}

/**
 * 빈 슬롯 플레이스홀더 = `+` 아이콘(ADR-0141 로 옛 `Slot <id8>` / `— empty —` 텍스트를 대체).
 * ADR-0143 로 버튼이 아니라 순수 그림이라 role·접근성 이름이 없다 — 슬롯 래퍼 직속 svg 가 유일한 표면이다.
 */
function emptyIcons(): HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>('[data-slot-id] > svg'))
}

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

/** epoch 은 이 렌더러의 판정에 안 쓰이지만 AgentInfo 필수 필드라 채운다(값에 뜻 없음). */
function agentInfo(id: string, structured: boolean, epoch = 2): AgentInfo {
  return {
    id,
    name: id,
    cwd: '/tmp',
    status: { type: 'Running' },
    cols: 80,
    rows: 24,
    epoch,
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
    seedAgents(agentInfo(agentId, false))
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    const terminal = screen.getByTestId('terminal-slot')
    expect(terminal).toBeTruthy()
    expect(terminal.getAttribute('data-agent-id')).toBe(agentId)
  })

  it('agent_id null slot → `+` 아이콘 플레이스홀더가 뜨고 TerminalSlot 은 없다(옛 텍스트 없음)', () => {
    render(<ViewLayoutRenderer node={slotNode('s1', null)} focusedSlotId={null} />)
    expect(screen.queryByTestId('terminal-slot')).toBeNull()
    expect(emptyIcons()).toHaveLength(1)
    // 옛 플레이스홀더 텍스트 회귀 방어(ADR-0141).
    expect(screen.queryByText(/^Slot /)).toBeNull()
    expect(screen.queryByText('— empty —')).toBeNull()
  })

  it('focusedSlotId == node.id → 포커스 링 오버레이(inset box-shadow, accent 65%)가 컨텐츠 위에 뜬다', () => {
    render(<ViewLayoutRenderer node={slotNode('s1', null)} focusedSlotId="s1" />)
    const wrapper = document.querySelector('[data-slot-id="s1"]') as HTMLElement
    expect(wrapper).toBeTruthy()
    // ADR-0066(focus-ring): 링은 래퍼가 아니라 컨텐츠 *위* absolute 오버레이로 그린다(overflow:hidden
    //   슬롯에서 100% 채운 자식이 inset box-shadow 를 덮던 버그 수정).
    const overlay = [...wrapper.querySelectorAll('div')].find(d => d.style.boxShadow.includes('accent'))
    expect(overlay).toBeTruthy()
    expect(overlay!.style.pointerEvents).toBe('none')
    expect(overlay!.style.position).toBe('absolute')
    expect(wrapper.style.border).toContain('border')
    expect(wrapper.style.border).not.toContain('accent')
  })

  it('focusedSlotId != node.id → 비포커스: 링 오버레이 없음', () => {
    render(<ViewLayoutRenderer node={slotNode('s1', null)} focusedSlotId="s-other" />)
    const wrapper = document.querySelector('[data-slot-id="s1"]') as HTMLElement
    expect(wrapper.style.border).toContain('border')
    expect(wrapper.style.border).not.toContain('accent')
    const overlay = [...wrapper.querySelectorAll('div')].find(d => d.style.boxShadow.includes('accent'))
    expect(overlay).toBeUndefined()
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
    expect(emptyIcons()).toHaveLength(0)
    const wrapper = document.querySelector('[data-slot-id="s1"]') as HTMLElement
    expect(wrapper.style.justifyContent).not.toBe('center')
  })

  it('data-slot-id 속성이 node.id 로 설정된다(cdp 검증용 불변식)', () => {
    const id = 'test-slot-uuid'
    render(<ViewLayoutRenderer node={slotNode(id, null)} focusedSlotId={null} />)
    expect(document.querySelector(`[data-slot-id="${id}"]`)).toBeTruthy()
  })

  it('agent_id 있는 slot(caps 도착) 래퍼에는 중앙정렬 flex 가 없다(터미널 레이아웃 오염 방지)', () => {
    seedAgents(agentInfo('some-agent-id', false))
    render(<ViewLayoutRenderer node={slotNode('s1', 'some-agent-id')} focusedSlotId={null} />)
    const wrapper = document.querySelector('[data-slot-id="s1"]') as HTMLElement
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

  // ── ADR-0148: "명부에 없는 에이전트" 를 세 갈래로 가른다 ────────────────────────────────
  //
  // 한 상태(agent==null)가 세 가지 뜻을 겹쳐 갖고 있었다. 종료(kill)의 실제 결말은 reaper 가 세션을
  // 수거하며 **명부에서 지우고 프로필은 남기는** 것이라, 옛 게이트는 죽은 슬롯을 「연결 중」으로 그리며
  // 뷰를 내렸다 — 데몬이 replay ring 을 세션과 함께 버리므로 그 순간 대화가 영구 소실됐다.
  describe('ADR-0148 — 에이전트 없는 슬롯 세 갈래', () => {
    const GONE = 'gone-agent'

    function profile(id: string, epoch = 3): unknown {
      return { id, name: id, cwd: '/tmp', display_name: null, parent_id: null, created_at: 0, epoch }
    }

    /** 살아있는 에이전트로 한 번 마운트해 "기억"을 만든다. */
    function mountAlive(structured = true, epoch = 3) {
      seedAgents(agentInfo(GONE, structured, epoch))
      agentStoreState.agentsLoaded = true
      agentStoreState.profilesLoaded = true
      agentStoreState.profiles = [profile(GONE, epoch)]
      return render(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)
    }

    it('프로필 있음 + 마운트 기억 없음 → 「에이전트 연결 중…」(스폰 대기·부팅 대기가 여기)', () => {
      // 예약 노드를 활성화한 직후가 이 상태다: 프로필은 이미 있고 에이전트는 아직 없다.
      agentStoreState.agentsLoaded = true
      agentStoreState.profilesLoaded = true
      agentStoreState.profiles = [profile(GONE)]
      render(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)

      expect(screen.getByText('에이전트 연결 중…')).toBeTruthy()
      expect(screen.queryByText('연결된 에이전트가 없습니다')).toBeNull()
      // 아직 뜬 적 없는 슬롯이라 죽은 에이전트로 구독을 걸지 않는다.
      expect(screen.queryByTestId('rich-slot')).toBeNull()
      expect(screen.queryByTestId('terminal-slot')).toBeNull()
    })

    it('명부·프로필 목록 자체를 못 받은 구간도 「에이전트 연결 중…」(「없습니다」로 새지 않는다)', () => {
      // profilesLoaded=false = refreshProfiles 미도착·실패. 여기서 「없습니다」로 새면 대화를 보존 중인
      //   뷰가 조기 언마운트된다.
      agentStoreState.agentsLoaded = true
      agentStoreState.profilesLoaded = false
      agentStoreState.profiles = []
      render(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)

      expect(screen.getByText('에이전트 연결 중…')).toBeTruthy()
      expect(screen.queryByText('연결된 에이전트가 없습니다')).toBeNull()
    })

    // ★F1 회귀★: 종료 순간 슬롯 prop 이 흔들리면 재마운트가 돌아 보존하려던 대화가 그 자리에서 지워진다
    //   (데몬 ring 도 세션과 함께 사라져 재구독으로도 못 살린다). "같은 DOM 노드가 그대로 있나" 로 본다 —
    //   텍스트 존재만 보면 재마운트와 리렌더가 구분되지 않는다.
    //   ★흔드는 값을 아예 안 넘기는 것이 방어선이다★ — 그래서 아래 케이스는 "무엇을 넘기든 안 흔들린다"
    //   가 아니라 "넘기는 것이 viewId·agentId 뿐이다" 로 지킨다(맨 아래 케이스).
    it('프로필 있음 + 기억 있음 → 같은 노드를 유지한다(대화 보존)', () => {
      const { rerender } = mountAlive(true, 3)
      const before = screen.getByTestId('rich-slot')

      // 종료 = 명부에서 사라지고 프로필만 남는다.
      agentStoreState.agents = []
      rerender(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)

      const after = screen.getByTestId('rich-slot')
      expect(Object.is(before, after)).toBe(true) // 재마운트되지 않았다 = 대화 보존
      expect(screen.queryByText('에이전트 연결 중…')).toBeNull()
      expect(screen.queryByText('연결된 에이전트가 없습니다')).toBeNull()
    })

    it('터미널 모드도 같다 — 노드 유지', () => {
      const { rerender } = mountAlive(false, 5)
      const before = screen.getByTestId('terminal-slot')

      agentStoreState.agents = []
      rerender(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)

      const after = screen.getByTestId('terminal-slot')
      expect(Object.is(before, after)).toBe(true)
    })

    // ★회차(epoch)를 슬롯에 되돌려 넣지 말 것★: 그 값은 살아있는 명부에서만 나오는데 종료가 곧 명부에서
    //   사라지는 것이라, prop 으로 내려보내는 순간 **종료 = prop 변화 = 재마운트 = 대화 소실**이 된다.
    //   화신 회전은 구독 쪽이 권위 명부로 판정해 비우기 신호로 낸다(protocolClient.observeRoster).
    it('슬롯에 넘기는 것은 viewId·agentId 뿐이다(회차를 내려보내지 않는다)', () => {
      mountAlive(true, 3)
      const slot = screen.getByTestId('rich-slot')
      expect(slot.getAttribute('data-view-id')).toBe('s1')
      expect(slot.getAttribute('data-agent-id')).toBe(GONE)
      expect(slot.getAttribute('data-epoch')).toBeNull()
    })

    // ★F2 회귀★: 기억은 그 에이전트 것이다. 다른 에이전트를 배정하면 caps 게이트를 건너뛰어선 안 된다 —
    //   B 가 터미널 모드인데 A 의 기억으로 RichSlot 이 뜨면 tag0 바이트를 버린다.
    it('슬롯에 다른 에이전트를 배정하면 기억을 쓰지 않는다(caps 게이트 유지)', () => {
      const { rerender } = mountAlive(true, 3)
      expect(screen.getByTestId('rich-slot')).toBeTruthy()

      // B 배정 — 아직 명부에 없다(caps 미도착).
      const OTHER = 'other-agent'
      agentStoreState.profiles = [profile(GONE), profile(OTHER)]
      rerender(<ViewLayoutRenderer node={slotNode('s1', OTHER)} focusedSlotId={null} />)

      expect(screen.queryByTestId('rich-slot')).toBeNull()
      expect(screen.queryByTestId('terminal-slot')).toBeNull()
      expect(screen.getByText('에이전트 연결 중…')).toBeTruthy()
    })

    // ★G1 회귀★: 프로필 목록을 못 받은 채(단발 pull 실패·지연) 종료되면 presence 는 'unknown' 이다.
    //   그걸로 뷰를 내리면 보존하려던 대화가 영구 소실된다 — "프로필이 없다" 는 목록을 받은 뒤에만 믿는다.
    it('프로필 목록을 못 받은 상태에서 종료돼도 기억이 있으면 뷰를 지킨다', () => {
      // 명부는 받았고(에이전트가 실제로 떴다) 프로필 목록만 못 받은 조합.
      seedAgents(agentInfo(GONE, true, 4))
      agentStoreState.agentsLoaded = true
      agentStoreState.profilesLoaded = false
      agentStoreState.profiles = []
      const { rerender } = render(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)
      const before = screen.getByTestId('rich-slot')

      agentStoreState.agents = []
      rerender(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)

      const after = screen.getByTestId('rich-slot')
      expect(Object.is(before, after)).toBe(true)
      expect(screen.queryByText('에이전트 연결 중…')).toBeNull()
    })

    // ★G2 회귀★: 기억을 안 지우면 배정을 뺐다가 되돌릴 때 되살아난다. 그 사이 뷰는 이미 언마운트돼
    //   대화가 없으므로, 되살아난 기억은 "빈 화면 + 흐림 + 수거된 에이전트로 구독" 을 만든다.
    it('배정을 다른 에이전트로 옮겼다가 되돌리면 기억이 되살아나지 않는다', () => {
      const OTHER = 'other-agent'
      const { rerender } = mountAlive(true, 3)
      expect(screen.getByTestId('rich-slot')).toBeTruthy()

      // 종료 → 뷰 유지(정상).
      agentStoreState.agents = []
      rerender(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)
      expect(screen.getByTestId('rich-slot')).toBeTruthy()

      // 같은 슬롯에 B 배정 → A 의 뷰는 언마운트되고(대화 state 도 여기서 죽는다) 기억도 버려져야 한다.
      agentStoreState.profiles = [profile(GONE), profile(OTHER)]
      rerender(<ViewLayoutRenderer node={slotNode('s1', OTHER)} focusedSlotId={null} />)
      expect(screen.queryByTestId('rich-slot')).toBeNull()

      // 다시 A 배정 — 되살릴 대화가 없으므로 빈 뷰를 띄우지 않는다.
      rerender(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)
      expect(screen.queryByTestId('rich-slot')).toBeNull()
      expect(screen.getByText('에이전트 연결 중…')).toBeTruthy()
    })

    it('프로필 없음 → 「연결된 에이전트가 없습니다」(보존 중이던 뷰도 여기서 내려간다 — 의도)', () => {
      const { rerender } = mountAlive(true, 3)
      expect(screen.getByTestId('rich-slot')).toBeTruthy()

      // 종료 후 프로필까지 삭제 = 사용자의 명시적 정리 동작.
      agentStoreState.agents = []
      agentStoreState.profiles = []
      rerender(<ViewLayoutRenderer node={slotNode('s1', GONE)} focusedSlotId={null} />)

      expect(screen.getByText('연결된 에이전트가 없습니다')).toBeTruthy()
      expect(screen.queryByTestId('rich-slot')).toBeNull()
    })
  })

  it('agent 가 store 에 있고 structured caps → RichSlot(TerminalSlot 없음)', () => {
    const agentId = 'struct-agent'
    seedAgents(agentInfo(agentId, true))
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    expect(screen.getByTestId('rich-slot')).toBeTruthy()
    expect(screen.queryByTestId('terminal-slot')).toBeNull()
    expect(screen.queryByText('에이전트 연결 중…')).toBeNull()
  })

  // ── RenderMode 기본 유도(defaultRenderMode): 오버라이드 없을 때 caps 로 렌더러가 정해진다 ──────────
  it('오버라이드 없음 + structured=true caps → 기본 유도로 RichSlot(TerminalSlot 없음)', () => {
    const agentId = 'derive-rich'
    seedAgents(agentInfo(agentId, true))
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    expect(screen.getByTestId('rich-slot')).toBeTruthy()
    expect(screen.queryByTestId('terminal-slot')).toBeNull()
  })

  it('오버라이드 없음 + structured=false caps → 기본 유도로 TerminalSlot(RichSlot 없음)', () => {
    const agentId = 'derive-terminal'
    seedAgents(agentInfo(agentId, false))
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    expect(screen.getByTestId('terminal-slot')).toBeTruthy()
    expect(screen.queryByTestId('rich-slot')).toBeNull()
  })

  // ── 오버라이드가 기본을 이긴다(setRenderMode) ────────────────────────────────────────────────
  it('setRenderMode(id,"terminal")는 structured 기본(rich)을 덮어 TerminalSlot 을 마운트한다', () => {
    const agentId = 'override-terminal'
    seedAgents(agentInfo(agentId, true))
    useViewStore.getState().setRenderMode('s1', 'terminal')
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    expect(screen.getByTestId('terminal-slot')).toBeTruthy()
    expect(screen.queryByTestId('rich-slot')).toBeNull()
  })

  it('setRenderMode(id,"rich")는 비structured 기본(terminal)을 덮어 RichSlot 을 마운트한다', () => {
    const agentId = 'override-rich'
    seedAgents(agentInfo(agentId, false))
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
    seedAgents(agentInfo(agentId, true))
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
    useViewStore.getState().clearRenderMode('s1')
    render(<ViewLayoutRenderer node={slotNode('s1', agentId)} focusedSlotId={null} />)
    expect(screen.getByTestId('terminal-slot')).toBeTruthy()
    expect(screen.queryByTestId('dom-slot')).toBeNull()
  })
})

describe('ViewLayoutRenderer — split 분기', () => {
  it('split 노드 → a/b 두 자식 슬롯이 재귀 렌더된다', () => {
    const node = splitNode(slotNode('s1', null), slotNode('s2', null))
    render(<ViewLayoutRenderer node={node} focusedSlotId={null} />)
    expect(document.querySelector('[data-slot-id="s1"]')).toBeTruthy()
    expect(document.querySelector('[data-slot-id="s2"]')).toBeTruthy()
  })

  it('split 자식에 agent_id 있으면(caps 도착) 해당 슬롯에만 TerminalSlot 이 마운트된다', () => {
    const agentId = 'zzzz-agent'
    seedAgents(agentInfo(agentId, false))
    const node = splitNode(slotNode('s1', agentId), slotNode('s2', null))
    render(<ViewLayoutRenderer node={node} focusedSlotId={null} />)
    const terminals = screen.getAllByTestId('terminal-slot')
    expect(terminals).toHaveLength(1)
    expect(terminals[0].getAttribute('data-agent-id')).toBe(agentId)
    expect(emptyIcons()).toHaveLength(1)
  })

  // ── ★ADR-0063: node.ratio → 첫 Pane 의 preferredSize % 초기 사이징★ ──────────────────────────────
  // 이 스위트가 막는 것: split 렌더러가 node.ratio 를 첫 pane(a=왼/위)의 preferredSize="<pct>%" 로 넘겨
  // 부팅 레이아웃 narrow-left(0.2)가 실제로 20/80 으로 뜨는지(50/50 무시 + defaultSizes-px-붕괴 회귀 안전망).
  // 드래그→백엔드 되쓰기는 이 슬라이스 밖(초기 사이징만).
  it('split(ratio=0.2) → 첫 pane preferredSize="20%" 로 초기 사이징이 전달된다', () => {
    const node = splitNode(slotNode('s1', null), slotNode('s2', null), 0.2)
    render(<ViewLayoutRenderer node={node} focusedSlotId={null} />)
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
    const initial = splitNode(slotNode('left', null), slotNode('right', null), 0.2)
    const { rerender } = render(<ViewLayoutRenderer node={initial} focusedSlotId={null} />)
    const outerPanesBefore = topLevelPanes()
    expect(outerPanesBefore).toHaveLength(2)
    const [aBefore, bBefore] = outerPanesBefore.map(p => p.getAttribute('data-pane-instance'))
    const preferredBefore = outerPanesBefore[0].getAttribute('data-preferred-size')

    const restructured = splitNode(
      slotNode('left', null),
      splitNode(slotNode('right', null), slotNode('right-2', null)),
      0.2,
    )
    rerender(<ViewLayoutRenderer node={restructured} focusedSlotId={null} />)

    const outerPanesAfter = topLevelPanes()
    const [aAfter, bAfter] = outerPanesAfter.map(p => p.getAttribute('data-pane-instance'))
    expect(aAfter).toBe(aBefore)
    expect(bAfter).toBe(bBefore)
    expect(outerPanesAfter[0].getAttribute('data-preferred-size')).toBe(preferredBefore)
    expect(preferredBefore).toBe('20%')
  })

  // ── ★dir → allotment 방향(ADR-0140 유일한 진실 경계)★ ─────────────────────────────────────────
  // 이 두 케이스가 막는 것: 매핑이 뒤집히면 라벨·command·타입·백엔드가 전부 맞는데도 화면만 반대가 된다
  // (라벨↔command 단언만으로는 절대 안 잡히는 층).
  it('dir="top_bottom" → Allotment vertical=true(위/아래로 쌓임)', () => {
    const node = splitNode(slotNode('s1', null), slotNode('s2', null), 0.5, 'top_bottom')
    render(<ViewLayoutRenderer node={node} focusedSlotId={null} />)
    expect(screen.getAllByTestId('allotment')[0].getAttribute('data-vertical')).toBe('true')
  })

  it('dir="left_right" → Allotment vertical=false(좌/우로 나란히)', () => {
    const node = splitNode(slotNode('s1', null), slotNode('s2', null), 0.5, 'left_right')
    render(<ViewLayoutRenderer node={node} focusedSlotId={null} />)
    expect(screen.getAllByTestId('allotment')[0].getAttribute('data-vertical')).toBe('false')
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

  // ADR-0144: 빈 슬롯 좌클릭은 메뉴를 열지 않는다 — 포커스 이동은 그대로다.
  it('empty 슬롯 좌클릭은 focusSlot 만 부르고 메뉴는 열지 않는다', () => {
    clickSlot({ type: 'empty' })
    expect(focusSlotSpy).toHaveBeenCalledWith(FOCUS_VIEW, 's1')
    expect(screen.queryByText('가로 분할')).toBeNull()
  })

  // ★아이콘은 클릭을 삼키지 않는다★: 아이콘 위 클릭도 컨테이너까지 닿아야 focusSlot 이 불린다. 여기가
  //   없으면 아이콘에 상호작용을 되붙여도 스위트가 초록이라 click-to-focus 가 조용히 죽는다(좌클릭이
  //   메뉴를 열지 않는 지금은 포커스링 외엔 눈으로 확인할 신호도 없다, ADR-0144).
  it('`+` 아이콘 위 클릭도 컨테이너까지 닿아 focusSlot 을 부른다', () => {
    render(<ViewLayoutRenderer node={contentSlotNode('s1', { type: 'empty' })} focusedSlotId={null} />)
    fireEvent.click(emptyIcons()[0], { clientX: 5, clientY: 5 })
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
// '에이전트 종료'(agent.kill)는 agentClient.killAgent 로 이어진다.
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

  /** viewIdOverride 를 넘기면 그 view 로 렌더한다(Fix 3). */
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
    expect(screen.getByText('에이전트 모니터링')).toBeTruthy()
    expect(screen.getByText('새 콘텐츠')).toBeTruthy()
    expect(screen.getByText('가로 분할')).toBeTruthy()
    expect(screen.getByText('세로 분할')).toBeTruthy()
    expect(screen.getByText('닫기')).toBeTruthy()
    expect(screen.queryByText('비우기')).toBeNull()
    expect(screen.queryByText('팝업으로 분리')).toBeNull()
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

  // ── ★메뉴는 우클릭 전용(ADR-0144)★ ────────────────────────────────────────────────────────
  // ADR-0141/0143 은 빈 슬롯 좌클릭도 메뉴를 열게 했으나, "클릭할 때마다 메뉴가 뜬다"는 불편 제보로
  // ADR-0144 가 되돌렸다 — 빈 슬롯 좌클릭도 이제 다른 콘텐츠 슬롯과 같이 포커스만 옮긴다. 아래 세
  // 케이스는 우클릭으로 연 메뉴에 걸려 있던 불변식(재앵커 금지·바깥 클릭 처리·command 경로)이 좌클릭
  // 오프너 제거 후에도 유지되는지를 검증한다.
  /** 메뉴 div(position:fixed 컨테이너) — 좌표 단언용. */
  function openedMenu(): HTMLElement {
    return screen.getByText('가로 분할').closest('[style*="position: fixed"]') as HTMLElement
  }

  /** 아이콘이 아닌 빈 여백 좌클릭 = 슬롯 래퍼가 표적(ADR-0144 이후엔 메뉴를 열지 않는다). */
  function leftClickSlot(slotId: string, x: number, y: number): void {
    fireEvent.click(document.querySelector(`[data-slot-id="${slotId}"]`) as HTMLElement, {
      clientX: x,
      clientY: y,
    })
  }

  /** 메뉴 오프너는 우클릭뿐(ADR-0144) — 좌표가 필요한 케이스(재앵커·바깥 클릭)는 openMenu() 대신 이걸 쓴다. */
  function rightClickSlot(slotId: string, x: number, y: number): void {
    fireEvent.contextMenu(document.querySelector(`[data-slot-id="${slotId}"]`) as HTMLElement, {
      clientX: x,
      clientY: y,
    })
  }

  // ★상호작용을 되붙이지 않는다(ADR-0143 §영향)★: 빈 슬롯 안에는 role·tabindex·버튼이 없어야 한다 —
  //   되붙이면 컨테이너 핸들러와 겹쳐 이중 오픈이 되고, 키보드로 못 빠져나오는 메뉴에 닿는 경로가 살아난다.
  //   ★한계★: React 는 핸들러를 루트에 위임하므로 아이콘에 onClick 을 되붙였는지는 DOM 으로 볼 수 없다
  //   (양쪽이 같은 좌표를 쓰면 동작으로도 구별되지 않는다). 그 조항은 리뷰가 지키는 몫으로 남는다.
  it('빈 슬롯엔 상호작용 요소가 없다(role·tabindex·button 부재)', () => {
    render(<ViewLayoutRenderer node={slotNode('slot-inert', null)} focusedSlotId={null} />)
    const wrapper = document.querySelector('[data-slot-id="slot-inert"]') as HTMLElement
    expect(wrapper.querySelector('button, [role], [tabindex]')).toBeNull()
    const icon = emptyIcons()[0]
    expect(icon.hasAttribute('tabindex')).toBe(false)
    expect(icon.hasAttribute('role')).toBe(false)
    expect(icon.hasAttribute('aria-label')).toBe(false)
    // ADR-0143 결정 2: 순수 그림 — pointer-events 를 끊어 클릭이 항상 컨테이너까지 닿는다(ADR-0144 이후도 유효).
    expect(getComputedStyle(icon).pointerEvents).toBe('none')
  })

  // ★아이콘 크기 = 32px 초과(ADR-0143 결정 3)★: Tailwind `size-N` = N×0.25rem = N×4px(기본 스케일)이라
  //   클래스에서 px 를 되짚는다. jsdom 은 Tailwind 를 적용하지 않아 계산된 값으로는 볼 수 없다(실측은 GUI).
  it('`+` 아이콘은 32px 보다 크다', () => {
    render(<ViewLayoutRenderer node={slotNode('slot-size', null)} focusedSlotId={null} />)
    const sizeClass = [...emptyIcons()[0].classList].map(c => /^size-(\d+)$/.exec(c)).find(m => m != null)
    expect(sizeClass, 'size-N 유틸리티가 없다').not.toBeUndefined()
    expect(Number(sizeClass![1]) * 4).toBeGreaterThan(32)
  })

  // ★열린 메뉴는 재앵커되지 않는다(메뉴 *안쪽* 클릭)★: SlotContextMenu 는 포털이 아니라 슬롯 래퍼 안에
  //   마운트돼 서브메뉴 컨테이너 행 클릭이 래퍼까지 버블한다. 재앵커하면 메뉴가 커서 밑으로 점프해 앵커가
  //   메뉴에 겹치고 이어지는 클릭이 커서 밑 항목을 실행한다. 좌클릭이 메뉴를 열지 않게 된 뒤에도(ADR-0144)
  //   우클릭으로 연 메뉴 안쪽 클릭에 대해선 이 불변식이 여전히 성립해야 한다.
  it('열린 메뉴의 "새 콘텐츠" 행 클릭은 메뉴를 다시 앵커하지 않는다', () => {
    render(<ViewLayoutRenderer node={slotNode('slot-reanchor', null)} focusedSlotId={null} />)
    rightClickSlot('slot-reanchor', 42, 77)
    fireEvent.click(screen.getByText('새 콘텐츠'), { clientX: 300, clientY: 400 })
    expect(openedMenu().style.left).toBe(`${42 + ANCHOR_GAP}px`)
    expect(openedMenu().style.top).toBe(`${77 + ANCHOR_GAP}px`)
  })

  // ★메뉴 *바깥*(슬롯 여백) 좌클릭은 메뉴를 닫을 뿐, 새 좌표로 다시 열지 않는다(ADR-0144)★: SlotContextMenu
  //   의 바깥닫기는 여전히 `mousedown` 에서 먼저 돌아 상태를 비운다(ADR-0143 시절과 동일). 달라진 건 그
  //   다음이다 — 좌클릭이 더는 메뉴를 열지 않으므로(위 "메뉴는 우클릭 전용" 섹션) 바깥 클릭 뒤 메뉴는 새
  //   좌표로 옮겨 열리지 않고 닫힌 채로 남는다.
  it('열린 메뉴 밖(슬롯 여백) 좌클릭은 메뉴를 닫고 새 좌표로 다시 열지 않는다', () => {
    render(<ViewLayoutRenderer node={slotNode('slot-move', null)} focusedSlotId={null} />)
    rightClickSlot('slot-move', 42, 77)
    expect(openedMenu().style.left).toBe(`${42 + ANCHOR_GAP}px`)
    const wrapper = document.querySelector('[data-slot-id="slot-move"]') as HTMLElement
    fireEvent.mouseDown(wrapper, { clientX: 300, clientY: 400 })
    leftClickSlot('slot-move', 300, 400)
    expect(screen.queryByText('가로 분할')).toBeNull()
  })

  it('우클릭으로 열린 메뉴의 "가로 분할"도 같은 command 경로로 split(…, "top_bottom") 을 부른다', () => {
    render(<ViewLayoutRenderer node={slotNode('slot-lc2', null)} focusedSlotId={null} />)
    rightClickSlot('slot-lc2', 8, 9)
    fireEvent.click(screen.getByText('가로 분할'))
    expect(splitSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-lc2', 'top_bottom')
  })

  // ★좌클릭은 어느 슬롯 타입도 메뉴를 열지 않는다(ADR-0144)★: 터미널 입력·트리 노드 선택·팔레트 항목
  //   클릭 위에 메뉴가 뜨면 그 슬롯들이 가진 자기 클릭 의미를 덮는다. 빈 슬롯도 이제 같은 규칙이고(그
  //   케이스는 focusSlot 과 함께 위 click-to-focus 게이트 스위트가 검증한다) 아래 세 케이스는 제어
  //   슬롯 몫이다('가로 분할'은 콘텐츠 종류와 무관한 공통 항목이라 "메뉴가 떴는지"의 판별자로 쓴다).
  it('agent(터미널) 슬롯 좌클릭은 메뉴를 열지 않는다', () => {
    seedAgents(agentInfo('a-lc', false))
    render(<ViewLayoutRenderer node={slotNode('slot-agent-lc', 'a-lc')} focusedSlotId={null} />)
    leftClickSlot('slot-agent-lc', 20, 30)
    expect(screen.queryByText('가로 분할')).toBeNull()
  })

  it('agent_list(트리) 슬롯 좌클릭은 메뉴를 열지 않는다', () => {
    render(<ViewLayoutRenderer node={contentSlotNode('slot-tree-lc', { type: 'agent_list' })} focusedSlotId={null} />)
    leftClickSlot('slot-tree-lc', 20, 30)
    expect(screen.queryByText('가로 분할')).toBeNull()
  })

  it('preset_palette(팔레트) 슬롯 좌클릭은 메뉴를 열지 않는다', () => {
    render(
      <ViewLayoutRenderer node={contentSlotNode('slot-pal-lc', { type: 'preset_palette' })} focusedSlotId={null} />,
    )
    leftClickSlot('slot-pal-lc', 20, 30)
    expect(screen.queryByText('가로 분할')).toBeNull()
  })

  // ★라벨↔방향 결선(ADR-0140 = vim 관례)★: 가로줄이 생겨 위/아래로 나뉜다 → top_bottom. 뒤집히면
  //   사용자가 고른 관례가 깨진다(tmux 축 관례로 회귀).
  it('"가로 분할" → split(viewId, slotId, "top_bottom") 호출(§5 command 경로)', () => {
    openMenu('slot-A', null)
    fireEvent.click(screen.getByText('가로 분할'))
    expect(splitSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-A', 'top_bottom')
  })

  it('"세로 분할" → split(viewId, slotId, "left_right") 호출', () => {
    openMenu('slot-B', null)
    fireEvent.click(screen.getByText('세로 분할'))
    expect(splitSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-B', 'left_right')
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
    openMenu('slot-P', 'agent-p')
    fireEvent.click(screen.getByText('팝업으로 분리'))
    expect(moveSlotToWindowSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-P')
  })

  // ── ★viewIdOverride 스레딩★ — 팝업 창 경로는 activeViewId(=main) 대신 넘겨받은 view 로 액션한다 ──
  // ViewLayoutRenderer 가 ctx.viewId = viewIdOverride ?? currentViewId 로 조립해 command.run 에 넘긴다.
  it('viewIdOverride 있으면 "가로 분할"이 오버라이드 view 로 split 을 부른다', () => {
    openMenu('slot-po', null, POPUP_VIEW)
    fireEvent.click(screen.getByText('가로 분할'))
    expect(splitSpy).toHaveBeenCalledWith(POPUP_VIEW, 'slot-po', 'top_bottom')
    expect(splitSpy).not.toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-po', 'top_bottom')
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
    openMenu('slot-main', null)
    fireEvent.click(screen.getByText('가로 분할'))
    expect(splitSpy).toHaveBeenCalledWith(ACTIVE_VIEW, 'slot-main', 'top_bottom')
  })
})
