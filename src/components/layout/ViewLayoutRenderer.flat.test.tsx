// ViewLayoutRenderer 평평한 루트 테스트(ADR-0227, TRD §2d·§2e·§5) — 재마운트 없음 · DOM 순서 · 첫 렌더 위치 ·
//   칸 틀/테두리 분리 · 슬롯 단위 렌더 오류 격리 · 구분선 배선 · 사각형 검증 · 칸 틀 지표 보고.
// 사각형은 테스트 전용 픽스처(`testing/rects.ts`)가 만든다 — 값이 셸과 같은지는 보지 않는다(기하 = Rust 테스트 몫).
// jsdom 엔 레이아웃도 ResizeObserver 도 없다. 위치는 렌더 결과 스타일(사용자 속성 `--x0..--y1`)로만 본다.

import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ── 공유 상태(호이스팅) — 목 팩토리가 모듈 import 시점에 불리므로 여기 둔다 ─────────────────
const h = vi.hoisted(() => {
  const mounts = new Map<string, number>()
  const unmounts = new Map<string, number>()
  const renders = new Map<string, number>()
  return {
    /** 이 에이전트를 받은 TerminalSlot 은 렌더 중에 던진다 — 슬롯 단위 오류 격리의 표적. */
    THROWING_AGENT: 'boom-agent',
    /** slot id(= 슬롯 컴포넌트의 viewId) → 마운트·언마운트 횟수. 재마운트는 둘 다 1 씩 는다. */
    mounts,
    unmounts,
    /** slot id → TerminalSlot 이 그려진(함수 본문이 돈) 횟수. 구분선 미리보기가 슬롯 콘텐츠를 다시 그리는지 잰다. */
    renders,
    bump(m: Map<string, number>, key: string) {
      m.set(key, (m.get(key) ?? 0) + 1)
    },
  }
})

// ── Tauri / transport 계층 stub ────────────────────────────────────────────────
const invokeMock = vi.hoisted(() => vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
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

// ── 슬롯 stub — 마운트 카운터를 단다 ─────────────────────────────────────────────
// 슬롯 컴포넌트는 잎 안에서 `key = slot id` 로 그려진다. 잎이 재마운트되면 이 카운터가 같은 slot id 로 한 번 더 는다 —
//   DOM 노드 동일성과 함께 「같은 인스턴스가 살아남았다」를 두 쪽에서 잰다.
vi.mock('../slot/TerminalSlot', async () => {
  const { useEffect } = await import('react')
  return {
    default: ({ viewId, agentId }: { viewId: string; agentId: string }) => {
      useEffect(() => {
        h.bump(h.mounts, viewId)
        return () => h.bump(h.unmounts, viewId)
      }, []) // eslint-disable-line react-hooks/exhaustive-deps
      h.bump(h.renders, viewId)
      if (agentId === h.THROWING_AGENT) throw new Error('terminal slot render failure (test)')
      return <div data-testid="terminal-slot" data-agent-id={agentId} data-view-id={viewId} />
    },
  }
})
vi.mock('../slot/RichSlot', async () => {
  const { useEffect } = await import('react')
  return {
    default: ({ viewId, agentId }: { viewId: string; agentId: string }) => {
      useEffect(() => {
        h.bump(h.mounts, viewId)
        return () => h.bump(h.unmounts, viewId)
      }, []) // eslint-disable-line react-hooks/exhaustive-deps
      return <div data-testid="rich-slot" data-agent-id={agentId} data-view-id={viewId} />
    },
  }
})
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
import type { LayoutNode, SlotContent, SlotRect, SplitDir, SplitRect } from '../../api/layoutTypes'
import type { AgentInfo } from '../../api/types'
import { useViewStore } from '../../store/viewStore'
import { MIN_PANE_PX } from './splitPreview'
import { rectsFor, TEST_RATIO_BOUNDS } from './testing/rects'
import { __resetUiMetricsReportForTest } from './uiMetricsReport'
import { __resetCanvasReportForTest, getCanvasReport, submitCanvasSize } from './windowCanvasReport'

beforeEach(() => {
  h.mounts.clear()
  h.unmounts.clear()
  h.renders.clear()
  invokeMock.mockClear()
  invokeMock.mockImplementation(async () => undefined)
  // 잎 마운트가 웹뷰당 한 번 보내는 칸 틀 지표(모듈 상태) — 테스트 순서가 보고 여부를 가르지 않게 비운다.
  __resetUiMetricsReportForTest()
})

afterEach(() => {
  cleanup()
  __resetCanvasReportForTest()
  agentStoreState.agents = []
  agentStoreState.agentsLoaded = false
  agentStoreState.profiles = []
  agentStoreState.profilesLoaded = false
  useViewStore.setState({ renderModeOverride: {} })
})

// ── 트리·에이전트 헬퍼 ────────────────────────────────────────────────────────────
function slot(id: string, content: SlotContent = { type: 'empty' }): LayoutNode {
  return { type: 'slot', id, content }
}
function split(id: string, dir: SplitDir, a: LayoutNode, b: LayoutNode, ratio: number): LayoutNode {
  return { type: 'split', id, dir, ratio, a, b }
}
const LR = (id: string, a: LayoutNode, b: LayoutNode, ratio = 0.5) => split(id, 'left_right', a, b, ratio)
const TB = (id: string, a: LayoutNode, b: LayoutNode, ratio = 0.5) => split(id, 'top_bottom', a, b, ratio)

/** 칸 x 에는 에이전트 `agent-x` 를 배정한다 — 슬롯 컴포넌트가 떠야 마운트 카운터가 돈다. */
const agentOf = (slotId: string) => `agent-${slotId}`
const agentSlot = (id: string, agentId = agentOf(id)) => slot(id, { type: 'agent', agent_id: agentId })

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

/** 칸 id 들에 터미널 에이전트를 띄운다(caps 도착 = 슬롯 컴포넌트 마운트). */
function seedFor(...slotIds: string[]): void {
  agentStoreState.agents = slotIds.map(id => terminalAgent(agentOf(id)))
}

/** 트리 + 픽스처 사각형으로 그린 렌더러. `extra` 로 선택 props 를 더한다. */
function view(node: LayoutNode, extra: { viewIdOverride?: string; ratioBounds?: { min: number; max: number } } = {}) {
  return <ViewLayoutRenderer node={node} focusedSlotId={null} {...rectsFor(node)} {...extra} />
}

// ── DOM 헬퍼 ──────────────────────────────────────────────────────────────────
function frameOf(slotId: string): HTMLElement {
  const el = document.querySelector<HTMLElement>(`[data-slot-id="${slotId}"]`)
  if (el === null) throw new Error(`칸 틀 ${slotId} 가 없다`)
  return el
}
function dividerOf(splitId: string): HTMLElement {
  const el = document.querySelector<HTMLElement>(`[data-split-id="${splitId}"]`)
  if (el === null) throw new Error(`구분선 ${splitId} 가 없다`)
  return el
}
/** 틀 직속 테두리 요소 — 틀 밖에 있으면 틀의 overflow:hidden 에 잘리지 않는다. */
function borderOf(slotId: string): HTMLElement {
  const el = frameOf(slotId).querySelector<HTMLElement>(':scope > [data-slot-border]')
  if (el === null) throw new Error(`칸 ${slotId} 의 테두리 요소가 틀 직속에 없다`)
  return el
}
/** 사용자 속성 값. 빈 값을 0 으로 읽어 헛되이 맞지 않게, 없으면 던진다. */
function customProp(el: HTMLElement, name: string): number {
  const raw = el.style.getPropertyValue(name)
  if (raw.trim() === '') throw new Error(`${name} 가 없다`)
  return Number(raw)
}
function frameRect(slotId: string): Omit<SlotRect, 'slot_id'> {
  const f = frameOf(slotId)
  return { x0: customProp(f, '--x0'), y0: customProp(f, '--y0'), x1: customProp(f, '--x1'), y1: customProp(f, '--y1') }
}
/** 평평한 루트 — 칸 틀과 구분선이 모두 이것의 직속 자식이다. */
function rootOf(container: HTMLElement): HTMLElement {
  const root = container.firstElementChild
  if (!(root instanceof HTMLElement)) throw new Error('렌더러 루트가 없다')
  return root
}
/** 루트 직속 자식의 순서 — 칸은 slot id, 구분선은 `split:<id>`. */
function childOrder(root: HTMLElement): string[] {
  return [...root.children].map(el => el.getAttribute('data-slot-id') ?? `split:${el.getAttribute('data-split-id')}`)
}

/**
 * `target` 아래에서 떼어진 노드를 모은다. 옮겨진 노드도 떼어졌다가 다시 붙으므로 여기 잡힌다. 반환 함수는 기록을 거둬
 * 관측을 끝낸다. 콜백이 이미 가져간 기록과 아직 큐에 남은 기록(`takeRecords`)을 둘 다 모은다 — 콜백은 마이크로태스크로
 * 미뤄지므로, 사이에 `await` 가 끼면 한쪽만 봐서는 기록이 빈다.
 */
function watchDetach(target: Node): () => Node[] {
  const seen: MutationRecord[] = []
  const mo = new MutationObserver(records => {
    seen.push(...records)
  })
  mo.observe(target, { childList: true, subtree: true })
  return () => {
    const records = [...seen, ...mo.takeRecords()]
    mo.disconnect()
    return records.flatMap(r => [...r.removedNodes])
  }
}
const wasDetached = (removed: Node[], el: Node) => removed.some(n => n === el || n.contains(el))

/** 살아남아야 할 칸들 — 같은 틀 노드 · 떼어진 적 없음 · 슬롯 컴포넌트 마운트 1 회·언마운트 0 회. */
function expectSurvived(ids: string[], before: Map<string, HTMLElement>, removed: Node[]): void {
  for (const id of ids) {
    const prev = before.get(id)!
    expect(Object.is(frameOf(id), prev), `${id} 틀이 새 노드로 바뀌었다`).toBe(true)
    expect(wasDetached(removed, prev), `${id} 틀이 DOM 에서 떼어졌다`).toBe(false)
    expect(h.mounts.get(id), `${id} 슬롯 컴포넌트 마운트 횟수`).toBe(1)
    expect(h.unmounts.get(id) ?? 0, `${id} 슬롯 컴포넌트 언마운트 횟수`).toBe(0)
  }
}
const framesOf = (...ids: string[]) => new Map(ids.map(id => [id, frameOf(id)]))

// ── 재마운트 없음 ──────────────────────────────────────────────────────────────
// 칸은 트리 모양과 무관하게 루트 하나 아래 `key = slot id` 로 놓인다 — 닫기·승격·분할이 트리 부모를 바꿔도 생존 칸은
//   부모·key·상대 순서가 그대로라 재마운트도 DOM 이동도 없다(터미널 재구독·스크롤·포커스 소실 없음).
describe('ViewLayoutRenderer 평평한 루트 — 재마운트 없음', () => {
  it('닫기: LR{x, TB{y,z}} → y 닫기 → LR{x, z} — x·z 는 같은 인스턴스, y 만 내려간다', () => {
    seedFor('x', 'y', 'z')
    const { container, rerender } = render(view(LR('A', agentSlot('x'), TB('B', agentSlot('y'), agentSlot('z')))))
    const before = framesOf('x', 'z')
    const dividerA = dividerOf('A')
    const collect = watchDetach(container)

    rerender(view(LR('A', agentSlot('x'), agentSlot('z'))))

    const removed = collect()
    expectSurvived(['x', 'z'], before, removed)
    expect(document.querySelector('[data-slot-id="y"]')).toBeNull()
    expect(h.unmounts.get('y')).toBe(1)
    expect(Object.is(dividerOf('A'), dividerA)).toBe(true)
    expect(document.querySelector('[data-split-id="B"]')).toBeNull()
    // 위치만 바뀐다 — z 는 이제 오른쪽 절반 전체다.
    expect(frameRect('z')).toEqual({ x0: 0.5, y0: 0, x1: 1, y1: 1 })
  })

  it('승격: LR{TB{x,y}, z} → z 닫기 → TB{x,y} 가 루트로 — x·y 와 구분선 B 는 같은 노드', () => {
    seedFor('x', 'y', 'z')
    const { container, rerender } = render(view(LR('A', TB('B', agentSlot('x'), agentSlot('y')), agentSlot('z'))))
    const before = framesOf('x', 'y')
    const dividerB = dividerOf('B')
    const collect = watchDetach(container)

    rerender(view(TB('B', agentSlot('x'), agentSlot('y'))))

    const removed = collect()
    expectSurvived(['x', 'y'], before, removed)
    expect(Object.is(dividerOf('B'), dividerB)).toBe(true)
    expect(wasDetached(removed, dividerB)).toBe(false)
    expect(document.querySelector('[data-split-id="A"]')).toBeNull()
    expect(frameRect('x')).toEqual({ x0: 0, y0: 0, x1: 1, y1: 0.5 })
  })

  it('분할: x → LR{x, n} — x 는 같은 인스턴스, 새 칸 n 만 마운트된다', () => {
    seedFor('x', 'n')
    const { container, rerender } = render(view(agentSlot('x')))
    const before = framesOf('x')
    const collect = watchDetach(container)

    rerender(view(LR('S', agentSlot('x'), agentSlot('n'))))

    expectSurvived(['x'], before, collect())
    expect(h.mounts.get('n')).toBe(1)
    expect(frameRect('x')).toEqual({ x0: 0, y0: 0, x1: 0.5, y1: 1 })
  })

  it('같은 split id 의 방향 전환(LR → TB) — 칸·구분선 노드가 그대로이고 구분선 방향만 바뀐다', () => {
    seedFor('x', 'y')
    const { container, rerender } = render(view(LR('A', agentSlot('x'), agentSlot('y'))))
    const before = framesOf('x', 'y')
    const divider = dividerOf('A')
    const collect = watchDetach(container)

    rerender(view(TB('A', agentSlot('x'), agentSlot('y'))))

    const removed = collect()
    expectSurvived(['x', 'y'], before, removed)
    expect(Object.is(dividerOf('A'), divider)).toBe(true)
    expect(wasDetached(removed, divider)).toBe(false)
    expect(divider.getAttribute('data-dir')).toBe('top_bottom')
    expect(frameRect('y')).toEqual({ x0: 0, y0: 0.5, x1: 1, y1: 1 })
  })

  it('비율 변경 스냅샷(0.5 → 0.3) — 노드는 그대로이고 사각형 값만 바뀐다', () => {
    seedFor('x', 'y')
    const { container, rerender } = render(view(LR('A', agentSlot('x'), agentSlot('y'), 0.5)))
    const before = framesOf('x', 'y')
    const divider = dividerOf('A')
    const collect = watchDetach(container)

    rerender(view(LR('A', agentSlot('x'), agentSlot('y'), 0.3)))

    expectSurvived(['x', 'y'], before, collect())
    expect(Object.is(dividerOf('A'), divider)).toBe(true)
    expect(frameRect('x').x1).toBe(0.3)
    expect(frameRect('y').x0).toBe(0.3)
    expect(customProp(divider, '--at')).toBe(0.3)
  })

  // ADR-0148: 종료로 명부에서 수거된 에이전트의 뷰는 잎의 기억으로만 남는다(데몬 replay ring 은 이미 없다). 잎이
  //   재마운트되면 그 기억과 대화가 함께 사라진다 — 트리 모양이 바뀌어도 같은 인스턴스여야 한다.
  it('ADR-0148 보존 뷰가 승격·분할 뒤에도 같은 인스턴스다', () => {
    const GONE = 'gone-agent'
    agentStoreState.agentsLoaded = true
    agentStoreState.profilesLoaded = true
    agentStoreState.profiles = [
      { id: GONE, name: GONE, cwd: '/tmp', display_name: null, parent_id: null, created_at: 0, epoch: 1 },
    ]
    agentStoreState.agents = [terminalAgent(GONE), terminalAgent(agentOf('y')), terminalAgent(agentOf('z'))]
    const keep = () => agentSlot('k', GONE)
    const { container, rerender } = render(view(LR('A', TB('B', keep(), agentSlot('y')), agentSlot('z'))))
    const kept = within(frameOf('k')).getByTestId('terminal-slot')
    const before = framesOf('k', 'y')
    const collect = watchDetach(container)

    // 종료 — 명부에서 사라지고 프로필은 남는다(reserved) → 뷰 유지.
    agentStoreState.agents = [terminalAgent(agentOf('y')), terminalAgent(agentOf('z'))]
    rerender(view(LR('A', TB('B', keep(), agentSlot('y')), agentSlot('z'))))
    expect(Object.is(within(frameOf('k')).getByTestId('terminal-slot'), kept)).toBe(true)

    // 승격 — z 를 닫아 TB{k,y} 가 루트로 올라온다.
    rerender(view(TB('B', keep(), agentSlot('y'))))
    // 분할 — k 를 좌우로 나눠 새 빈 칸 n 이 생긴다.
    rerender(view(TB('B', LR('S', keep(), slot('n')), agentSlot('y'))))

    expectSurvived(['k', 'y'], before, collect())
    expect(Object.is(within(frameOf('k')).getByTestId('terminal-slot'), kept)).toBe(true)
    expect(within(frameOf('k')).queryByText('에이전트 연결 중…')).toBeNull()
  })
})

// ── DOM 순서 ──────────────────────────────────────────────────────────────────
// 트리 전위 순을 따르면 LLM 의 재구성(같은 id, 다른 모양)마다 React 가 노드를 옮긴다 — 끌던 구분선은 포인터 캡처를
//   잃고(`lostpointercapture`) 칸은 포커스·스크롤을 잃는다. 그래서 칸은 slot id 순, 구분선은 split id 순이다(TRD §2e).
describe('ViewLayoutRenderer 평평한 루트 — DOM 순서', () => {
  it('칸은 slot id 순, 그 뒤에 구분선이 split id 순으로 놓인다(트리 전위 순이 아니다)', () => {
    // 전위 순: 칸 z,x,y · 구분선 s-b,s-a.
    const { container } = render(view(LR('s-b', slot('z'), TB('s-a', slot('x'), slot('y')))))
    expect(childOrder(rootOf(container))).toEqual(['x', 'y', 'z', 'split:s-a', 'split:s-b'])
  })

  it('LLM 식 재구성(같은 id·다른 모양) 뒤에도 순서가 같고 어떤 칸·구분선도 옮겨지지 않는다', () => {
    const { container, rerender } = render(view(LR('s-b', slot('z'), TB('s-a', slot('x'), slot('y')))))
    const root = rootOf(container)
    const beforeNodes = [...root.children]
    const collect = watchDetach(container)

    // 전위 순: 칸 y,z,x · 구분선 s-a,s-b — 앞 트리와 뒤집혔다.
    rerender(view(TB('s-a', LR('s-b', slot('y'), slot('z')), slot('x'))))

    const removed = collect()
    expect(childOrder(root)).toEqual(['x', 'y', 'z', 'split:s-a', 'split:s-b'])
    const afterNodes = [...root.children]
    afterNodes.forEach((el, i) => expect(Object.is(el, beforeNodes[i])).toBe(true))
    for (const el of beforeNodes) expect(wasDetached(removed, el)).toBe(false)
  })
})

// ── 첫 렌더 위치 · 틀과 테두리 ─────────────────────────────────────────────────────
describe('ViewLayoutRenderer 평평한 루트 — 첫 렌더 위치·틀과 테두리', () => {
  it('첫 렌더에 칸 틀마다 --x0..--y1, 구분선마다 --at 이 박혀 있다(ResizeObserver 없이)', () => {
    // 측정·관측 없이 위치가 렌더 결과 스타일 자체여야 첫 프레임이 비지 않는다(TRD §2e 첫 페인트).
    expect('ResizeObserver' in globalThis).toBe(false)
    const node = LR('A', slot('x'), TB('B', slot('y'), slot('z'), 0.25), 0.4)
    const { slotRects, splitRects } = rectsFor(node)
    render(view(node))
    for (const r of slotRects) {
      expect(frameRect(r.slot_id)).toEqual({ x0: r.x0, y0: r.y0, x1: r.x1, y1: r.y1 })
      // 사용자 속성을 위치로 바꾸는 규칙은 이 클래스에 걸려 있다(index.css — 규칙 자체는 index.css.test.ts 가 잰다).
      expect(frameOf(r.slot_id).classList.contains('engram-slot-frame'), r.slot_id).toBe(true)
    }
    for (const s of splitRects) expect(customProp(dividerOf(s.split_id), '--at')).toBe(s.at)
  })

  // SlotContextMenu 가 position:fixed 라 조상에 transform(·contain)이 걸리면 메뉴 좌표계가 깨진다(TRD §2e).
  it('루트·칸 틀·테두리 요소 어디에도 transform·contain 이 없다', () => {
    const { container } = render(view(LR('A', slot('x'), slot('y'))))
    const targets = [rootOf(container), frameOf('x'), borderOf('x'), frameOf('y'), borderOf('y')]
    for (const el of targets) {
      for (const prop of ['transform', 'contain']) {
        expect(el.style.getPropertyValue(prop), `${prop} on ${el.outerHTML.slice(0, 60)}`).toBe('')
        expect((el.style as unknown as Record<string, unknown>)[prop] ?? '').toBe('')
      }
      expect(el.getAttribute('style') ?? '').not.toMatch(/(^|;)\s*(transform|contain)\s*:/)
    }
  })

  it('칸 틀에는 테두리가 없고 테두리 요소가 틀 직속 안에 있다 · data-slot-id 는 틀에만 있다', () => {
    render(view(LR('A', slot('x'), slot('y'))))
    const frame = frameOf('x')
    for (const prop of ['border', 'border-width', 'border-style', 'border-top-width', 'border-left-width']) {
      expect(frame.style.getPropertyValue(prop), prop).toBe('')
    }
    expect(frame.style.position).toBe('absolute')
    expect(frame.style.overflow).toBe('hidden')
    const border = borderOf('x')
    expect(border.parentElement).toBe(frame)
    expect(border.style.position).toBe('absolute')
    expect(border.style.borderWidth).toBe('1px')
    expect(border.style.boxSizing).toBe('border-box')
    expect(border.hasAttribute('data-slot-id')).toBe(false)
    expect(frame.hasAttribute('data-slot-border')).toBe(false)
  })

  // 틀이 0~1px 여도 틀 자신은 사각형 그대로여야 한다 — 테두리를 틀에 두면 테두리 폭이 할당 폭을 넘어 이웃과 겹친다.
  //   테두리 요소는 틀의 overflow:hidden 에 잘린다.
  it('0px·1px 틀 사각형에서도 틀 스타일은 사각형 그대로다(크기를 더하는 속성 없음)', () => {
    const node = LR('A', slot('zero'), LR('B', slot('one'), slot('rest')))
    const slotRects: SlotRect[] = [
      { slot_id: 'zero', x0: 0, y0: 0, x1: 0, y1: 1 },
      { slot_id: 'one', x0: 0, y0: 0, x1: 0.001, y1: 1 },
      { slot_id: 'rest', x0: 0.001, y0: 0, x1: 1, y1: 1 },
    ]
    const splitRects: SplitRect[] = [
      { split_id: 'A', dir: 'left_right', x0: 0, y0: 0, x1: 1, y1: 1, at: 0 },
      { split_id: 'B', dir: 'left_right', x0: 0, y0: 0, x1: 1, y1: 1, at: 0.001 },
    ]
    render(<ViewLayoutRenderer node={node} focusedSlotId={null} slotRects={slotRects} splitRects={splitRects} />)
    const sizeProps = ['border', 'border-width', 'padding', 'margin', 'width', 'height', 'min-width', 'min-height']
    for (const r of slotRects) {
      const frame = frameOf(r.slot_id)
      for (const name of ['--x0', '--y0', '--x1', '--y1'] as const) {
        expect(frame.style.getPropertyValue(name), `${r.slot_id} ${name}`).toBe(String(r[name.slice(2) as 'x0']))
      }
      for (const prop of sizeProps) expect(frame.style.getPropertyValue(prop), `${r.slot_id} ${prop}`).toBe('')
      expect(frame.style.overflow).toBe('hidden')
      expect(borderOf(r.slot_id).parentElement).toBe(frame)
    }
  })
})

// ── 구분선 배선 ──────────────────────────────────────────────────────────────────
// 제스처·미리보기 규칙은 `Splitter.test.tsx`·`useSplitDrag.test.tsx` 가 잰다. 여기서는 렌더러가 잠금 조건과 뷰 좌표·루트를
//   제대로 넘기는지만 본다.
describe('ViewLayoutRenderer 평평한 루트 — 구분선 배선', () => {
  const VIEW = 'v-flat'
  const ROOT_RECT = { left: 0, top: 0, width: 1000, height: 600, x: 0, y: 0, right: 1000, bottom: 600, toJSON: () => ({}) } as DOMRect

  /** 이 창이 셸에 캔버스를 보고한 상태를 만든다 — 허용 범위의 축 길이가 여기서 온다. */
  async function reportCanvas(): Promise<void> {
    submitCanvasSize(getCanvasReport(), { w: 1000, h: 600 })
    await waitFor(() => expect(getCanvasReport().lastSent).toEqual({ w: 1000, h: 600 }))
  }

  // 캔버스는 보고된 상태로 둔다 — 잠금의 사유가 축 길이 부재가 아니라 렌더러가 넘긴 조건이게.
  it('비율 한계(ratioBounds)가 없으면 구분선이 잠긴다', async () => {
    await reportCanvas()
    render(view(LR('A', slot('x'), slot('y')), { viewIdOverride: VIEW }))
    expect(dividerOf('A').style.cursor).toBe('default')
  })

  it('뷰 좌표를 모르면(오버라이드 없음 · 이 창의 활성 탭 미확정) 비율 한계가 있어도 구분선이 잠긴다', async () => {
    await reportCanvas()
    render(view(LR('A', slot('x'), slot('y')), { ratioBounds: TEST_RATIO_BOUNDS }))
    expect(dividerOf('A').style.cursor).toBe('default')
  })

  it('뷰 좌표·비율 한계·캔버스가 있으면 끌어서 미리보기가 칸에 입혀지고 뗄 때 set_split_ratio 를 한 번 부른다', async () => {
    await reportCanvas()
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === 'set_split_ratio' ? { ratio: 0.5, outcome: 'Unchanged', version: 0 } : undefined,
    )
    const { container } = render(
      view(LR('A', slot('x'), slot('y')), { viewIdOverride: VIEW, ratioBounds: TEST_RATIO_BOUNDS }),
    )
    rootOf(container).getBoundingClientRect = () => ROOT_RECT
    const divider = dividerOf('A')
    expect(divider.style.cursor).toBe('col-resize')

    fireEvent.pointerDown(divider, { clientX: 500, clientY: 300, pointerId: 1, button: 0 })
    act(() => {
      fireEvent.pointerMove(document.body, { clientX: 300, clientY: 300, pointerId: 1, buttons: 1 })
    })
    // 미리보기는 셸 사각형 위에 입혀져 칸 틀에 바로 보인다.
    expect(frameRect('x').x1).toBeCloseTo(0.3, 10)
    act(() => {
      fireEvent.pointerUp(document.body, { clientX: 300, clientY: 300, pointerId: 1 })
    })

    const calls = invokeMock.mock.calls.filter(([cmd]) => cmd === 'set_split_ratio')
    expect(calls).toHaveLength(1)
    expect(calls[0][1]).toEqual({ viewId: VIEW, splitId: 'A', ratio: expect.closeTo(0.3, 10) })
    // `Unchanged` 는 미리보기를 즉시 버린다 — 스냅샷 사각형으로 돌아온다.
    await waitFor(() => expect(frameRect('x').x1).toBe(0.5))
  })

  // 끄는 동안 바뀌는 것은 틀의 위치뿐이다 — 슬롯 콘텐츠(터미널·마크다운)를 프레임마다 다시 그리면 allotment(DOM 스타일만
  //   바꿈)보다 비싸진다. 끌리는 상자(루트 분할 A) 안의 칸 셋 모두가 대상이다.
  describe('프레임마다 미리보기', () => {
    // 구분선은 첫 이동 뒤의 이동을 애니메이션 프레임에 모아 미리보기를 낸다 — 프레임을 손으로 넘긴다.
    const frames = new Map<number, FrameRequestCallback>()
    let nextFrame = 0
    const flushFrames = () => {
      const cbs = [...frames.values()]
      frames.clear()
      act(() => cbs.forEach(cb => cb(0)))
    }
    beforeEach(() => {
      frames.clear()
      vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => {
        frames.set(++nextFrame, cb)
        return nextFrame
      })
      vi.stubGlobal('cancelAnimationFrame', (id: number) => frames.delete(id))
    })
    afterEach(() => {
      vi.unstubAllGlobals()
    })

    it('끄는 동안 끌리는 상자 안 칸은 틀의 --x* 만 바뀌고 슬롯 콘텐츠는 다시 그려지지 않는다', async () => {
      seedFor('x', 'y', 'z')
      await reportCanvas()
      invokeMock.mockImplementation(async (cmd: string) =>
        cmd === 'set_split_ratio' ? { ratio: 0.5, outcome: 'Unchanged', version: 0 } : undefined,
      )
      const { container } = render(
        view(LR('A', agentSlot('x'), TB('B', agentSlot('y'), agentSlot('z'))), {
          viewIdOverride: VIEW,
          ratioBounds: TEST_RATIO_BOUNDS,
        }),
      )
      rootOf(container).getBoundingClientRect = () => ROOT_RECT
      const ids = ['x', 'y', 'z']
      for (const id of ids) expect(h.renders.get(id) ?? 0, `${id} 첫 렌더`).toBeGreaterThan(0)
      const rendersBefore = new Map(ids.map(id => [id, h.renders.get(id)]))
      const expectNoContentRender = () => {
        for (const id of ids) expect(h.renders.get(id), `${id} 슬롯 콘텐츠 렌더 횟수`).toBe(rendersBefore.get(id))
      }

      fireEvent.pointerDown(dividerOf('A'), { clientX: 500, clientY: 300, pointerId: 1, button: 0 })
      for (const clientX of [300, 350, 400]) {
        act(() => {
          fireEvent.pointerMove(document.body, { clientX, clientY: 300, pointerId: 1, buttons: 1 })
        })
        flushFrames()
        expect(frameRect('x').x1).toBeCloseTo(clientX / 1000, 10)
        expect(frameRect('y').x0).toBeCloseTo(clientX / 1000, 10)
        expect(frameRect('z').x0).toBeCloseTo(clientX / 1000, 10)
        expectNoContentRender()
      }
      act(() => {
        fireEvent.pointerUp(document.body, { clientX: 400, clientY: 300, pointerId: 1 })
      })
      await waitFor(() => expect(frameRect('x').x1).toBe(0.5))
      expectNoContentRender()
    })
  })
})

// ── 사각형 검증 ──────────────────────────────────────────────────────────────────
// 셸 사각형이 트리의 칸·분할 id 와 정확히 맞을 때만 그린다. 일부만 맞는 사각형으로 그리면 칸이 빠진 채 화면이 선다.
describe('ViewLayoutRenderer 평평한 루트 — 사각형 검증', () => {
  let consoleError: ReturnType<typeof vi.spyOn>
  beforeEach(() => {
    consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
  })
  afterEach(() => {
    consoleError.mockRestore()
  })

  const tree = () => LR('A', slot('x'), TB('B', slot('y'), slot('z')))

  it('분할 트리에 칸 사각형이 일부만 오면 아무것도 그리지 않고 오류를 남긴다', () => {
    const { slotRects, splitRects } = rectsFor(tree())
    const { container } = render(
      <ViewLayoutRenderer
        node={tree()}
        focusedSlotId={null}
        slotRects={slotRects.filter(r => r.slot_id !== 'z')}
        splitRects={splitRects}
      />,
    )
    expect(container.innerHTML).toBe('')
    expect(consoleError).toHaveBeenCalledWith(expect.stringContaining('아무것도 그리지 않는다'), 'A')
  })

  it('분할 트리에 분할 사각형이 트리와 어긋나면(하나 빠짐) 아무것도 그리지 않고 오류를 남긴다', () => {
    const { slotRects, splitRects } = rectsFor(tree())
    const { container } = render(
      <ViewLayoutRenderer
        node={tree()}
        focusedSlotId={null}
        slotRects={slotRects}
        splitRects={splitRects.filter(s => s.split_id !== 'B')}
      />,
    )
    expect(container.innerHTML).toBe('')
    expect(consoleError).toHaveBeenCalledWith(expect.stringContaining('아무것도 그리지 않는다'), 'A')
  })

  it('단일 칸 트리에 빈 사각형 배열이 오면 전체 상자로 그린다', () => {
    render(<ViewLayoutRenderer node={slot('solo')} focusedSlotId={null} slotRects={[]} splitRects={[]} />)
    expect(frameRect('solo')).toEqual({ x0: 0, y0: 0, x1: 1, y1: 1 })
    expect(document.querySelectorAll('[data-split-id]')).toHaveLength(0)
  })
})

// ── 칸 틀 지표 보고 ───────────────────────────────────────────────────────────────
// 셸의 칸 content = 틀 − 테두리 폭(TRD §2d). 폭은 틀이 아니라 그 안 테두리 요소에서 잰다 — 틀은 테두리가 없어
//   계산된 폭이 수가 아니고(jsdom 은 `medium`), 그러면 보고가 나가지 않는다.
describe('ViewLayoutRenderer 평평한 루트 — 칸 틀 지표 보고', () => {
  const metricsCalls = () => invokeMock.mock.calls.filter(([cmd]) => cmd === 'report_ui_metrics')

  it('잎 마운트가 테두리 요소의 폭으로 지표를 한 번 보내고, 잎이 늘어도 다시 보내지 않는다', async () => {
    const { rerender } = render(view(LR('A', slot('x'), TB('B', slot('y'), slot('z')))))
    await waitFor(() => expect(metricsCalls()).toHaveLength(1))
    expect(metricsCalls()[0][1]).toEqual({
      metrics: { frame_insets: { t: 1, r: 1, b: 1, l: 1 }, min_pane_px: MIN_PANE_PX },
    })
    await act(async () => {})

    rerender(view(LR('A', slot('x'), TB('B', slot('y'), LR('C', slot('z'), slot('n'))))))
    await act(async () => {})

    expect(frameOf('n')).toBeTruthy()
    expect(metricsCalls()).toHaveLength(1)
  })
})

// ── 슬롯 하나의 렌더 오류가 창 전체를 지우지 않는다 ───────────────────────────────────
// React 19 는 경계 없는 렌더 throw 에 루트를 통째로 내린다 — 슬롯 하나가 던지면 창이 빈 화면이 된다.
describe('ViewLayoutRenderer — 슬롯 단위 렌더 오류 격리', () => {
  let consoleError: ReturnType<typeof vi.spyOn>

  beforeEach(() => {
    agentStoreState.agents = [terminalAgent(h.THROWING_AGENT), terminalAgent('ok-agent'), terminalAgent('ok-agent-2')]
    consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
  })
  afterEach(() => {
    consoleError.mockRestore()
  })

  function fallbackIn(slotId: string): Element | null {
    return document.querySelector(`[data-slot-id="${slotId}"] [data-slot-error]`)
  }
  const badTree = () => LR('R', agentSlot('bad', h.THROWING_AGENT), agentSlot('good', 'ok-agent'))

  it('던지는 슬롯만 대체 표시로 바뀌고 형제 슬롯은 그대로 렌더된다', () => {
    render(view(badTree()))

    expect(fallbackIn('bad')).toBeTruthy()
    expect(fallbackIn('good')).toBeNull()
    const terminals = screen.getAllByTestId('terminal-slot')
    expect(terminals.map(t => t.getAttribute('data-agent-id'))).toEqual(['ok-agent'])
    expect(consoleError).toHaveBeenCalled()
  })

  it('던지던 슬롯의 콘텐츠를 비우면 대체 표시가 걷힌다', () => {
    const { rerender } = render(view(badTree()))
    expect(fallbackIn('bad')).toBeTruthy()

    rerender(view(LR('R', slot('bad'), agentSlot('good', 'ok-agent'))))

    expect(fallbackIn('bad')).toBeNull()
    // `+` 아이콘은 틀이 아니라 그 안의 테두리 요소 직속이다(ADR-0227).
    expect(document.querySelector('[data-slot-id="bad"] > [data-slot-border] > svg')).toBeTruthy()
  })

  // 같은 콘텐츠 종류(에이전트 → 에이전트)라 슬롯 컴포넌트는 같은 자리에 남는다 — 그래도 경계는 풀려야 한다.
  it('던지던 슬롯에 다른 에이전트를 배정하면 대체 표시가 걷히고 새 에이전트가 렌더된다', () => {
    const { rerender } = render(view(badTree()))
    expect(fallbackIn('bad')).toBeTruthy()

    rerender(view(LR('R', agentSlot('bad', 'ok-agent-2'), agentSlot('good', 'ok-agent'))))

    expect(fallbackIn('bad')).toBeNull()
    const bad = document.querySelector('[data-slot-id="bad"] [data-testid="terminal-slot"]')
    expect(bad?.getAttribute('data-agent-id')).toBe('ok-agent-2')
  })

  it('렌더 모드를 바꿔도 대체 표시가 걷힌다(던지던 렌더러를 다른 렌더러로 교체)', () => {
    render(view(badTree()))
    expect(fallbackIn('bad')).toBeTruthy()

    act(() => useViewStore.getState().setRenderMode('bad', 'rich'))

    expect(fallbackIn('bad')).toBeNull()
    expect(document.querySelector('[data-slot-id="bad"] [data-testid="rich-slot"]')).toBeTruthy()
  })

  it('그 슬롯과 무관한 갱신(형제 콘텐츠 변경)으로는 대체 표시가 풀리지 않는다', () => {
    const { rerender } = render(view(badTree()))
    expect(fallbackIn('bad')).toBeTruthy()
    const callsBefore = consoleError.mock.calls.length

    rerender(view(LR('R', agentSlot('bad', h.THROWING_AGENT), slot('good'))))

    expect(fallbackIn('bad')).toBeTruthy()
    // 풀었다가 같은 자식을 다시 그려 또 던지지 않았다.
    expect(consoleError.mock.calls.length).toBe(callsBefore)
  })
})
