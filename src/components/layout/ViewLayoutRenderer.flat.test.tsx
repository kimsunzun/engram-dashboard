// ViewLayoutRenderer flat-root tests (ADR-0227, TRD §2d·§2e·§5) — no remount · DOM order · first-render position · frame/border split · per-slot render error isolation · divider wiring · rectangle validation · frame metrics report.
// Rectangles come from a test-only fixture (`testing/rects.ts`) — equality with the shell's values is the Rust tests' job. jsdom has neither layout nor ResizeObserver; position is read only through the rendered style (`--x0..--y1`).

import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ── Shared state (hoisted) — the mock factory is called at module import time so it lives here ─────────────────
const h = vi.hoisted(() => {
  const mounts = new Map<string, number>()
  const unmounts = new Map<string, number>()
  const renders = new Map<string, number>()
  return {
    /** A TerminalSlot given this agent throws during render — the target of per-slot error isolation. */
    THROWING_AGENT: 'boom-agent',
    /** slot id (= the slot component's viewId) → mount and unmount counts. A remount raises both by 1. */
    mounts,
    unmounts,
    /** slot id → count of times TerminalSlot was drawn (the function body ran). Measures whether a divider preview redraws slot content. */
    renders,
    bump(m: Map<string, number>, key: string) {
      m.set(key, (m.get(key) ?? 0) + 1)
    },
  }
})

// ── Tauri / transport layer stubs ────────────────────────────────────────────────
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

// ── Slot stub — attaches a mount counter ─────────────────────────────────────────────
// A slot is drawn in a leaf with `key = slot id`; a leaf remount raises this counter again — with DOM node identity, "same instance survived" is measured from two sides.
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
  // The area frame metrics a leaf mount sends once per webview (module state) — cleared so test order does not decide whether it was reported.
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

// ── Tree and agent helpers ────────────────────────────────────────────────────────────
function slot(id: string, content: SlotContent = { type: 'empty' }): LayoutNode {
  return { type: 'slot', id, content }
}
function split(id: string, dir: SplitDir, a: LayoutNode, b: LayoutNode, ratio: number): LayoutNode {
  return { type: 'split', id, dir, ratio, a, b }
}
const LR = (id: string, a: LayoutNode, b: LayoutNode, ratio = 0.5) => split(id, 'left_right', a, b, ratio)
const TB = (id: string, a: LayoutNode, b: LayoutNode, ratio = 0.5) => split(id, 'top_bottom', a, b, ratio)

/** Assign agent `agent-x` to area x — the slot component must come up for the mount counter to run. */
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

/** Bring up a terminal agent in the given area ids (caps arrived = the slot component mounts). */
function seedFor(...slotIds: string[]): void {
  agentStoreState.agents = slotIds.map(id => terminalAgent(agentOf(id)))
}

/** A renderer drawn with a tree + fixture rectangles. `extra` adds optional props. */
function view(node: LayoutNode, extra: { viewIdOverride?: string; ratioBounds?: { min: number; max: number } } = {}) {
  return <ViewLayoutRenderer node={node} focusedSlotId={null} {...rectsFor(node)} {...extra} />
}

// ── DOM helpers ──────────────────────────────────────────────────────────────────
function frameOf(slotId: string): HTMLElement {
  const el = document.querySelector<HTMLElement>(`[data-slot-id="${slotId}"]`)
  if (el === null) throw new Error(`area frame ${slotId} is missing`)
  return el
}
function dividerOf(splitId: string): HTMLElement {
  const el = document.querySelector<HTMLElement>(`[data-split-id="${splitId}"]`)
  if (el === null) throw new Error(`divider ${splitId} is missing`)
  return el
}
/** Border element directly under the frame — outside the frame it would not be clipped by the frame's overflow:hidden. */
function borderOf(slotId: string): HTMLElement {
  const el = frameOf(slotId).querySelector<HTMLElement>(':scope > [data-slot-border]')
  if (el === null) throw new Error(`the border element of area ${slotId} is not directly under the frame`)
  return el
}
/** Custom property value. Throws if missing so an empty value is not read as 0 and matches vacuously. */
function customProp(el: HTMLElement, name: string): number {
  const raw = el.style.getPropertyValue(name)
  if (raw.trim() === '') throw new Error(`${name} is missing`)
  return Number(raw)
}
function frameRect(slotId: string): Omit<SlotRect, 'slot_id'> {
  const f = frameOf(slotId)
  return { x0: customProp(f, '--x0'), y0: customProp(f, '--y0'), x1: customProp(f, '--x1'), y1: customProp(f, '--y1') }
}
/** The flat root — area frames and dividers are all its direct children. */
function rootOf(container: HTMLElement): HTMLElement {
  const root = container.firstElementChild
  if (!(root instanceof HTMLElement)) throw new Error('the renderer root is missing')
  return root
}
/** Order of the root's direct children — areas by slot id, dividers as `split:<id>`. */
function childOrder(root: HTMLElement): string[] {
  return [...root.children].map(el => el.getAttribute('data-slot-id') ?? `split:${el.getAttribute('data-split-id')}`)
}

/**
 * Collects nodes detached under `target`. A moved node is also detached and re-attached so it is caught here. The returned function reaps the records
 * and ends observation. It gathers both the records the callback already took and the records still in the queue (`takeRecords`) — the callback is
 * deferred to a microtask, so with an `await` in between, looking at one side only leaves the records empty.
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

/** The areas that must survive — the same frame node · never detached · slot component mounted once, unmounted zero times. */
function expectSurvived(ids: string[], before: Map<string, HTMLElement>, removed: Node[]): void {
  for (const id of ids) {
    const prev = before.get(id)!
    expect(Object.is(frameOf(id), prev), `${id} frame was replaced by a new node`).toBe(true)
    expect(wasDetached(removed, prev), `${id} frame was detached from the DOM`).toBe(false)
    expect(h.mounts.get(id), `${id} slot component mount count`).toBe(1)
    expect(h.unmounts.get(id) ?? 0, `${id} slot component unmount count`).toBe(0)
  }
}
const framesOf = (...ids: string[]) => new Map(ids.map(id => [id, frameOf(id)]))

// ── No remount ──
// Areas sit under one root with `key = slot id` regardless of tree shape, so close · promote · split keep surviving areas' parent, key and relative order — no remount or DOM move (no loss of terminal resubscription, scroll or focus).
describe('ViewLayoutRenderer flat root — no remount', () => {
  it('close: LR{x, TB{y,z}} → close y → LR{x, z} — x·z are the same instance, only y comes down', () => {
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
    // Only the position changes — z is now the whole right half.
    expect(frameRect('z')).toEqual({ x0: 0.5, y0: 0, x1: 1, y1: 1 })
  })

  it('promote: LR{TB{x,y}, z} → close z → TB{x,y} becomes the root — x·y and divider B are the same node', () => {
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

  it('split: x → LR{x, n} — x is the same instance, only the new area n mounts', () => {
    seedFor('x', 'n')
    const { container, rerender } = render(view(agentSlot('x')))
    const before = framesOf('x')
    const collect = watchDetach(container)

    rerender(view(LR('S', agentSlot('x'), agentSlot('n'))))

    expectSurvived(['x'], before, collect())
    expect(h.mounts.get('n')).toBe(1)
    expect(frameRect('x')).toEqual({ x0: 0, y0: 0, x1: 0.5, y1: 1 })
  })

  it('direction change of the same split id (LR → TB) — area and divider nodes stay and only the divider direction changes', () => {
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

  it('ratio change snapshot (0.5 → 0.3) — nodes stay and only the rectangle values change', () => {
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

  // ADR-0148: the view of an agent reaped from the roster remains only as the leaf's memory (the daemon replay ring is gone); a leaf remount would lose it and the conversation, so it must be the same instance across tree-shape changes.
  it('an ADR-0148 preserved view is the same instance even after promote·split', () => {
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

    // Exit — vanishes from the roster and the profile stays (reserved) → view kept.
    agentStoreState.agents = [terminalAgent(agentOf('y')), terminalAgent(agentOf('z'))]
    rerender(view(LR('A', TB('B', keep(), agentSlot('y')), agentSlot('z'))))
    expect(Object.is(within(frameOf('k')).getByTestId('terminal-slot'), kept)).toBe(true)

    // Promote — closing z lifts TB{k,y} to the root.
    rerender(view(TB('B', keep(), agentSlot('y'))))
    // Split — splitting k left-right makes a new empty area n.
    rerender(view(TB('B', LR('S', keep(), slot('n')), agentSlot('y'))))

    expectSurvived(['k', 'y'], before, collect())
    expect(Object.is(within(frameOf('k')).getByTestId('terminal-slot'), kept)).toBe(true)
    // If the memory is lost the leaf draws the absence veil itself — if the veil is directly under the leaf it is not a kept view.
    expect(frameOf('k').querySelector('[data-slot-border] > [data-slot-dead]')).toBeNull()
  })
})

// ── DOM order ──
// Following tree pre-order makes React move nodes on every LLM reconstruction (same ids, different shape) — a dragged divider loses pointer capture (`lostpointercapture`) and an area loses focus and scroll. So areas go in slot id order, dividers in split id order (TRD §2e).
describe('ViewLayoutRenderer flat root — DOM order', () => {
  it('areas in slot id order, then dividers in split id order (not tree pre-order)', () => {
    // Pre-order: areas z,x,y · dividers s-b,s-a.
    const { container } = render(view(LR('s-b', slot('z'), TB('s-a', slot('x'), slot('y')))))
    expect(childOrder(rootOf(container))).toEqual(['x', 'y', 'z', 'split:s-a', 'split:s-b'])
  })

  it('even after an LLM-style reconstruction (same ids, different shape) the order is the same and no area or divider is moved', () => {
    const { container, rerender } = render(view(LR('s-b', slot('z'), TB('s-a', slot('x'), slot('y')))))
    const root = rootOf(container)
    const beforeNodes = [...root.children]
    const collect = watchDetach(container)

    // Pre-order: areas y,z,x · dividers s-a,s-b — reversed from the previous tree.
    rerender(view(TB('s-a', LR('s-b', slot('y'), slot('z')), slot('x'))))

    const removed = collect()
    expect(childOrder(root)).toEqual(['x', 'y', 'z', 'split:s-a', 'split:s-b'])
    const afterNodes = [...root.children]
    afterNodes.forEach((el, i) => expect(Object.is(el, beforeNodes[i])).toBe(true))
    for (const el of beforeNodes) expect(wasDetached(removed, el)).toBe(false)
  })
})

// ── First-render position · frame and border ─────────────────────────────────────────────────────
describe('ViewLayoutRenderer flat root — first-render position · frame and border', () => {
  it('on first render --x0..--y1 per area frame and --at per divider are set (without ResizeObserver)', () => {
    // Position must be the rendered style itself, with no measuring or observing, so the first frame is not empty (TRD §2e first paint).
    expect('ResizeObserver' in globalThis).toBe(false)
    const node = LR('A', slot('x'), TB('B', slot('y'), slot('z'), 0.25), 0.4)
    const { slotRects, splitRects } = rectsFor(node)
    render(view(node))
    for (const r of slotRects) {
      expect(frameRect(r.slot_id)).toEqual({ x0: r.x0, y0: r.y0, x1: r.x1, y1: r.y1 })
      // The rule that turns the custom properties into position hangs on this class (index.css — the rule itself is measured by index.css.test.ts).
      expect(frameOf(r.slot_id).classList.contains('engram-slot-frame'), r.slot_id).toBe(true)
    }
    for (const s of splitRects) expect(customProp(dividerOf(s.split_id), '--at')).toBe(s.at)
  })

  // SlotContextMenu is position:fixed, so a transform (or contain) on an ancestor breaks the menu coordinate system (TRD §2e).
  it('neither transform nor contain exists on the root, area frames or border elements', () => {
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

  it('the area frame has no border and the border element is directly inside the frame · data-slot-id is only on the frame', () => {
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

  // A 0–1px frame must itself be the rectangle as is — a border on the frame would exceed the allotted width and overlap its neighbour; the border element is clipped by the frame's overflow:hidden.
  it('even for 0px·1px frame rectangles the frame style is the rectangle as is (no property that adds size)', () => {
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

// ── Divider wiring ──
// Gesture and preview rules are in `Splitter.test.tsx` and `useSplitDrag.test.tsx`; here only that the renderer passes the lock condition, view coordinates and root correctly.
describe('ViewLayoutRenderer flat root — divider wiring', () => {
  const VIEW = 'v-flat'
  const ROOT_RECT = { left: 0, top: 0, width: 1000, height: 600, x: 0, y: 0, right: 1000, bottom: 600, toJSON: () => ({}) } as DOMRect

  /** Make the state where this window reported its canvas to the shell — the allowed range's axis length comes from here. */
  async function reportCanvas(): Promise<void> {
    submitCanvasSize(getCanvasReport(), { w: 1000, h: 600 })
    await waitFor(() => expect(getCanvasReport().lastSent).toEqual({ w: 1000, h: 600 }))
  }

  // The canvas is left reported — so the reason for locking is the condition the renderer passed, not a missing axis length.
  it('without ratio bounds (ratioBounds) the divider is locked', async () => {
    await reportCanvas()
    render(view(LR('A', slot('x'), slot('y')), { viewIdOverride: VIEW }))
    expect(dividerOf('A').style.cursor).toBe('default')
  })

  it('if the view coordinates are unknown (no override · this window\'s active tab not settled) the divider is locked even with ratio bounds', async () => {
    await reportCanvas()
    render(view(LR('A', slot('x'), slot('y')), { ratioBounds: TEST_RATIO_BOUNDS }))
    expect(dividerOf('A').style.cursor).toBe('default')
  })

  it('with view coordinates, ratio bounds and canvas, dragging puts the preview on the areas and on release set_split_ratio is called once', async () => {
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
    // The preview is laid over the shell rectangles and is visible on the area frame right away.
    expect(frameRect('x').x1).toBeCloseTo(0.3, 10)
    act(() => {
      fireEvent.pointerUp(document.body, { clientX: 300, clientY: 300, pointerId: 1 })
    })

    const calls = invokeMock.mock.calls.filter(([cmd]) => cmd === 'set_split_ratio')
    expect(calls).toHaveLength(1)
    expect(calls[0][1]).toEqual({ viewId: VIEW, splitId: 'A', ratio: expect.closeTo(0.3, 10) })
    // `Unchanged` discards the preview at once — it returns to the snapshot rectangles.
    await waitFor(() => expect(frameRect('x').x1).toBe(0.5))
  })

  // Only the frame position changes while dragging — slot content (terminal, markdown) is as is, so redrawing every frame only costs. All three areas inside the dragged box (root split A) are targets.
  describe('preview per frame', () => {
    // The divider gathers moves after the first move into an animation frame to emit the preview — frames are advanced by hand.
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

    it('while dragging, for areas inside the dragged box only the frame\'s --x* changes and slot content is not redrawn', async () => {
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
      for (const id of ids) expect(h.renders.get(id) ?? 0, `${id} first render`).toBeGreaterThan(0)
      const rendersBefore = new Map(ids.map(id => [id, h.renders.get(id)]))
      const expectNoContentRender = () => {
        for (const id of ids) expect(h.renders.get(id), `${id} slot content render count`).toBe(rendersBefore.get(id))
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

// ── Rectangle validation ──────────────────────────────────────────────────────────
// Draw only when the shell rectangles match the tree's area and split ids exactly. Drawing with a partially matching rectangle set leaves the screen standing with areas missing.
describe('ViewLayoutRenderer flat root — rectangle validation', () => {
  let consoleError: ReturnType<typeof vi.spyOn>
  beforeEach(() => {
    consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
  })
  afterEach(() => {
    consoleError.mockRestore()
  })

  const tree = () => LR('A', slot('x'), TB('B', slot('y'), slot('z')))

  it('if a split tree gets only some of the area rectangles nothing is drawn and an error is left', () => {
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
    expect(consoleError).toHaveBeenCalledWith(expect.stringContaining('drawing nothing'), 'A')
  })

  it('if a split tree\'s split rectangles do not match the tree (one missing) nothing is drawn and an error is left', () => {
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
    expect(consoleError).toHaveBeenCalledWith(expect.stringContaining('drawing nothing'), 'A')
  })

  it('a single-area tree given an empty rectangle array is drawn as the full box', () => {
    render(<ViewLayoutRenderer node={slot('solo')} focusedSlotId={null} slotRects={[]} splitRects={[]} />)
    expect(frameRect('solo')).toEqual({ x0: 0, y0: 0, x1: 1, y1: 1 })
    expect(document.querySelectorAll('[data-split-id]')).toHaveLength(0)
  })
})

// ── Area frame metrics report ───────────────────────────────────────────────────────
// The shell's area content = frame − border width (TRD §2d). Width is measured on the border element inside, not the frame — the frame has no border, so its computed width is not a number (jsdom gives `medium`) and no report goes out.
describe('ViewLayoutRenderer flat root — area frame metrics report', () => {
  const metricsCalls = () => invokeMock.mock.calls.filter(([cmd]) => cmd === 'report_ui_metrics')

  it('a leaf mount sends the metrics once with the border element width and does not resend as leaves grow', async () => {
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

// ── One slot\'s render error does not erase the whole window ───────────────────────────────────
// React 19 takes the whole root down on a render throw with no boundary — if one slot throws the window becomes a blank screen.
describe('ViewLayoutRenderer — per-slot render error isolation', () => {
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

  it('only the throwing slot is replaced by a fallback and sibling slots render as is', () => {
    render(view(badTree()))

    expect(fallbackIn('bad')).toBeTruthy()
    expect(fallbackIn('good')).toBeNull()
    const terminals = screen.getAllByTestId('terminal-slot')
    expect(terminals.map(t => t.getAttribute('data-agent-id'))).toEqual(['ok-agent'])
    expect(consoleError).toHaveBeenCalled()
  })

  it('when the throwing slot\'s content is cleared the fallback lifts', () => {
    const { rerender } = render(view(badTree()))
    expect(fallbackIn('bad')).toBeTruthy()

    rerender(view(LR('R', slot('bad'), agentSlot('good', 'ok-agent'))))

    expect(fallbackIn('bad')).toBeNull()
    // The `+` icon is directly under the border element inside, not the frame (ADR-0227).
    expect(document.querySelector('[data-slot-id="bad"] > [data-slot-border] > svg')).toBeTruthy()
  })

  // The same content kind (agent → agent) so the slot component stays in the same place — the boundary must still be released.
  it('assigning another agent to the throwing slot lifts the fallback and the new agent renders', () => {
    const { rerender } = render(view(badTree()))
    expect(fallbackIn('bad')).toBeTruthy()

    rerender(view(LR('R', agentSlot('bad', 'ok-agent-2'), agentSlot('good', 'ok-agent'))))

    expect(fallbackIn('bad')).toBeNull()
    const bad = document.querySelector('[data-slot-id="bad"] [data-testid="terminal-slot"]')
    expect(bad?.getAttribute('data-agent-id')).toBe('ok-agent-2')
  })

  it('changing the render mode also lifts the fallback (the throwing renderer is replaced by another)', () => {
    render(view(badTree()))
    expect(fallbackIn('bad')).toBeTruthy()

    act(() => useViewStore.getState().setRenderMode('bad', 'rich'))

    expect(fallbackIn('bad')).toBeNull()
    expect(document.querySelector('[data-slot-id="bad"] [data-testid="rich-slot"]')).toBeTruthy()
  })

  it('an update unrelated to that slot (a sibling content change) does not release the fallback', () => {
    const { rerender } = render(view(badTree()))
    expect(fallbackIn('bad')).toBeTruthy()
    const callsBefore = consoleError.mock.calls.length

    rerender(view(LR('R', agentSlot('bad', h.THROWING_AGENT), slot('good'))))

    expect(fallbackIn('bad')).toBeTruthy()
    // It did not release and redraw the same child and throw again.
    expect(consoleError.mock.calls.length).toBe(callsBefore)
  })
})
