// dockDrag 단위테스트 — TRD S21-slot-docking §9 「프론트(7 단계)」 중 대상 · 영역 · 세션 · pointer_seen · Esc · 파일 막기 ·
// 끊기. 끌기 출처(TabBar) · 떼어진 표시의 화면(WindowLayout) · pickTarget 골든은 각자의 시험이 잰다.
// jsdom 에는 DragEvent · DataTransfer 가 없어 MouseEvent 에 끌기 데이터 칸을 붙여 낸다 — 실제 OS 끌기의 사건 순서 ·
// 간격 · 창 사이 전달은 이 시험이 못 덮는다(§9 「시험이 못 덮는 것」).

import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest'

const tauriListen = vi.hoisted(() =>
  vi.fn(async (_event: string, _handler: unknown, _options?: unknown) => () => undefined),
)
vi.mock('@tauri-apps/api/event', () => ({ listen: tauriListen }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ label: 'main' }) }))

import type { DockPullReply } from '../../../../src-tauri/bindings/DockPullReply'
import type { DockRegions } from '../../../../src-tauri/bindings/DockRegions'
import type { Picked } from '../../../../src-tauri/bindings/Picked'
import type { PointCss } from '../../../../src-tauri/bindings/PointCss'
import type { Started } from '../../../../src-tauri/bindings/Started'
import {
  __resetDockDragForTest,
  createDockDrag,
  displayedTabOf,
  DOCK_MIME,
  ESC_SWALLOW_TAIL_MS,
  EVT_DOCK_REGIONS,
  EVT_DOCK_SESSION,
  HOVER_SWITCH_MS,
  mountDockDrag,
  type DockDragController,
  type DockIpc,
  type MountOptions,
} from './dockDrag'

const NONE: Picked = { type: 'none' }
const seam = (anchor: string | null, side: 'Left' | 'Right' | 'Top' | 'Bottom' = 'Left'): Picked => ({
  type: 'seam',
  anchor,
  side,
})
const bar = (index: number, over: string | null = null): Picked => ({ type: 'tab_bar', index, over })

function regions(over: Partial<DockRegions> = {}): DockRegions {
  return {
    drag_id: 'd1',
    rev: 1,
    bar_rev: 1,
    blocked: false,
    view_id: 'v-main',
    canvas_origin: { x: 0, y: 28 },
    canvas: { w: 800, h: 572 },
    outer: [],
    slots: [{ slot_id: 's1', frame: { x0: 0, y0: 0, x1: 800, y1: 572 }, bands: [] }],
    tab_bar: {
      rect: { x0: 0, y0: 0, x1: 800, y1: 28 },
      heads: [
        { view_id: 'v-main', rect: { x0: 0, y0: 0, x1: 100, y1: 28 } },
        { view_id: 'v-other', rect: { x0: 100, y0: 0, x1: 200, y1: 28 } },
      ],
      insert_xs: [0, 100, 200],
    },
    ...over,
  }
}

const startedFields = (s: Partial<Started> = {}): Started => ({
  source_label: 'pop-1',
  source_view: 'v-src',
  display_active: null,
  ...s,
})
const started = (id: string, seq: number, s: Partial<Started> = {}) => ({
  drag_id: id,
  seq,
  phase: { type: 'started', ...startedFields(s) },
})
const display = (id: string, seq: number, s: Partial<Started> = {}) => ({
  drag_id: id,
  seq,
  phase: { type: 'display', ...startedFields(s) },
})
const ended = (id: string, seq: number) => ({ drag_id: id, seq, phase: { type: 'ended' } })

const flush = async (): Promise<void> => {
  for (let i = 0; i < 20; i++) await Promise.resolve()
}

// ── 가짜 IPC · 의존 ──
let handlers: Map<string, { handler: (p: unknown) => void; target: string | null }>
let calls: Array<[string, Record<string, unknown> | undefined]>
let pullReply: DockPullReply | null
let unlistened: string[]
let ipc: DockIpc
let active: Record<string, string | null>
let activeSubs: Set<() => void>
let pick: Mock<(r: DockRegions, p: PointCss, src: string) => Picked>
let switchTab: Mock<(label: string, viewId: string) => void>
let ctl: DockDragController
let release: (() => void) | null

function setup(label = 'main', options?: MountOptions): void {
  ctl = createDockDrag({
    ipc,
    pick,
    activeTab: l => active[l] ?? null,
    subscribeActive: (_l, cb) => {
      activeSubs.add(cb)
      return () => activeSubs.delete(cb)
    },
    switchTab,
  })
  release = ctl.mount(label, options)
}

function emit(event: string, payload: unknown): void {
  const h = handlers.get(event)
  if (!h) throw new Error(`${event} 구독 없음`)
  h.handler(payload)
}

function setActive(label: string, viewId: string | null): void {
  active[label] = viewId
  for (const cb of [...activeSubs]) cb()
}

interface FakeDt {
  types: string[]
  dropEffect: string
  getData: (t: string) => string
}

function drag(
  type: string,
  opts: { types?: string[]; x?: number; y?: number; related?: EventTarget | null; data?: string } = {},
): { e: MouseEvent; dt: FakeDt } {
  const e = new MouseEvent(type, {
    bubbles: true,
    cancelable: true,
    clientX: opts.x ?? 10,
    clientY: opts.y ?? 50,
    relatedTarget: opts.related ?? null,
  })
  const dt: FakeDt = {
    types: opts.types ?? [DOCK_MIME],
    dropEffect: 'unset',
    getData: t => (t === DOCK_MIME ? (opts.data ?? '') : ''),
  }
  Object.defineProperty(e, 'dataTransfer', { value: dt })
  document.body.dispatchEvent(e)
  return { e, dt }
}

const dragData = (id = 'd1', view = 'v-src') => JSON.stringify({ drag_id: id, view_id: view })

const lastOf = <T>(xs: T[]): T | undefined => xs[xs.length - 1]

function cmdCalls(cmd: string): Array<Record<string, unknown> | undefined> {
  return calls.filter(c => c[0] === cmd).map(c => c[1])
}

const reportTypes = (): string[] =>
  cmdCalls('dock_drag_over').map(a => (a?.report as { type: string }).type)

beforeEach(async () => {
  vi.useFakeTimers()
  handlers = new Map()
  calls = []
  pullReply = null
  unlistened = []
  ipc = {
    listen: vi.fn(async (event: string, handler: (p: unknown) => void, target: string | null) => {
      handlers.set(event, { handler, target })
      return () => {
        unlistened.push(event)
        handlers.delete(event)
      }
    }),
    invoke: async <T>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
      calls.push([cmd, args])
      return (cmd === 'dock_pull' ? pullReply : undefined) as T
    },
  }
  active = { main: 'v-main' }
  activeSubs = new Set()
  pick = vi.fn((_r: DockRegions, _p: PointCss, _src: string) => NONE)
  switchTab = vi.fn((_label: string, _viewId: string) => undefined)
  release = null
})

afterEach(() => {
  release?.()
  vi.useRealTimers()
  vi.restoreAllMocks()
})

async function mounted(label = 'main', options?: MountOptions): Promise<void> {
  setup(label, options)
  await flush()
}

// ── 끊기(5.1) ──────────────────────────────────────────────────────────────────

describe('끊기 — window 의 react-dnd 꼴 리스너', () => {
  let seen: string[]
  const rdnd = (e: Event): void => {
    seen.push(e.type)
    e.preventDefault()
    const dt = (e as unknown as { dataTransfer?: FakeDt }).dataTransfer
    if (dt) dt.dropEffect = 'none'
  }
  const types = ['dragenter', 'dragover', 'dragleave', 'drop']

  beforeEach(() => {
    seen = []
    for (const t of types) window.addEventListener(t, rdnd)
  })
  afterEach(() => {
    for (const t of types) window.removeEventListener(t, rdnd)
  })

  it('우리 MIME 의 dragenter · dragover · dragleave · drop 은 window 에 닿지 않고 move 가 남는다', async () => {
    await mounted()
    const enter = drag('dragenter')
    const over = drag('dragover')
    drag('dragleave', { related: document.body })
    drag('drop', { data: dragData() })
    expect(seen).toEqual([])
    expect(enter.dt.dropEffect).toBe('move')
    expect(over.dt.dropEffect).toBe('move')
    expect(over.e.defaultPrevented).toBe(true)
  })

  it('우리 MIME 이 아닌 끌기는 window 에 닿는다(트리 재부모화가 자기 dropEffect 로 덮는다)', async () => {
    await mounted()
    drag('dragenter', { types: ['application/x-tree'] })
    drag('dragover', { types: ['application/x-tree'] })
    drag('drop', { types: ['application/x-tree'] })
    expect(seen).toEqual(['dragenter', 'dragover', 'drop'])
  })
})

// ── 파일 막기 ──────────────────────────────────────────────────────────────────

describe('파일 막기 — 탐색을 부르는 끌기(파일 · URL)만', () => {
  it.each(['Files', 'text/uri-list'])('%s 끌기 = dragenter · dragover preventDefault + dropEffect none · drop preventDefault · 끊지 않는다', async type => {
    await mounted()
    const winSeen: string[] = []
    const spy = (e: Event): void => {
      winSeen.push(e.type)
    }
    window.addEventListener('dragenter', spy)
    window.addEventListener('dragover', spy)
    window.addEventListener('drop', spy)
    try {
      // 커서 밑 요소가 바뀐 걸음엔 dragenter 만 온다 — 그 걸음에서 떼도 탐색하지 않게.
      const enter = drag('dragenter', { types: [type, 'text/plain'] })
      expect(enter.e.defaultPrevented).toBe(true)
      expect(enter.dt.dropEffect).toBe('none')
      const over = drag('dragover', { types: [type, 'text/plain'] })
      const drop = drag('drop', { types: [type, 'text/plain'] })
      expect(over.e.defaultPrevented).toBe(true)
      expect(over.dt.dropEffect).toBe('none')
      expect(drop.e.defaultPrevented).toBe(true)
      expect(winSeen).toEqual(['dragenter', 'dragover', 'drop'])
      await flush()
      expect(calls).toEqual([])
    } finally {
      window.removeEventListener('dragenter', spy)
      window.removeEventListener('dragover', spy)
      window.removeEventListener('drop', spy)
    }
  })

  it('글자 끌기는 건드리지 않는다 — 입력창에 글자를 끌어 놓는 기본 동작이 선다', async () => {
    await mounted()
    const enter = drag('dragenter', { types: ['text/plain', 'text/html'] })
    const over = drag('dragover', { types: ['text/plain', 'text/html'] })
    const drop = drag('drop', { types: ['text/plain', 'text/html'] })
    expect(enter.e.defaultPrevented).toBe(false)
    expect(enter.dt.dropEffect).toBe('unset')
    expect(over.e.defaultPrevented).toBe(false)
    expect(over.dt.dropEffect).toBe('unset')
    expect(drop.e.defaultPrevented).toBe(false)
  })

  it('세션과 무관하게 늘 막는다(세션 중에도)', async () => {
    await mounted()
    emit(EVT_DOCK_SESSION, started('d1', 0))
    const over = drag('dragover', { types: ['Files'] })
    expect(over.e.defaultPrevented).toBe(true)
    expect(over.dt.dropEffect).toBe('none')
  })
})

// ── 대상 ─────────────────────────────────────────────────────────────────────

describe('대상', () => {
  it('우리 MIME 이면 세션을 몰라도 받는다 — 보고는 0 · dock_pull 한 번', async () => {
    await mounted()
    const enter = drag('dragenter')
    const over = drag('dragover')
    drag('dragover')
    await flush()
    expect(enter.e.defaultPrevented).toBe(true)
    expect(over.e.defaultPrevented).toBe(true)
    expect(over.dt.dropEffect).toBe('move')
    expect(cmdCalls('dock_drag_over')).toEqual([])
    expect(cmdCalls('dock_pull')).toHaveLength(1)
  })

  it('blocked 영역 = dropEffect none · 가이드 없음(받기는 그대로 preventDefault)', async () => {
    await mounted()
    emit(EVT_DOCK_REGIONS, regions({ blocked: true, view_id: null, slots: [], tab_bar: null }))
    pick.mockReturnValue(seam('s1'))
    const over = drag('dragover')
    expect(over.e.defaultPrevented).toBe(true)
    expect(over.dt.dropEffect).toBe('none')
    expect(ctl.getGuide()).toBeNull()
  })

  it('pickTarget 결과대로 가이드를 낸다 — 점 · 끄는 탭을 넘긴다', async () => {
    await mounted()
    emit(EVT_DOCK_SESSION, started('d1', 0))
    emit(EVT_DOCK_REGIONS, regions())
    pick.mockReturnValue(seam('s1', 'Right'))
    drag('dragover', { x: 790, y: 300 })
    expect(pick).toHaveBeenLastCalledWith(regions(), { x: 790, y: 300 }, 'v-src')
    const g = ctl.getGuide()
    expect(g?.shown).toEqual(seam('s1', 'Right'))
    expect(g?.canvas).toBe(true)
    expect(g?.cursorSlot).toBe('s1')
  })

  it('보이던 탭 ≠ 영역 view_id 면 캔버스 가이드를 그리지 않는다 — 보고 shown = none · displayed 를 싣는다', async () => {
    await mounted()
    setActive('main', 'v-other')
    emit(EVT_DOCK_REGIONS, regions({ view_id: 'v-main' }))
    pick.mockReturnValue(seam('s1'))
    drag('dragover')
    await flush()
    expect(ctl.getGuide()?.shown).toEqual(NONE)
    expect(ctl.getGuide()?.canvas).toBe(false)
    expect(lastOf(cmdCalls('dock_drag_over'))).toMatchObject({
      drag_id: 'd1',
      report: { type: 'over', shown: NONE, displayed: 'v-other', rev: 1, bar_rev: 1 },
    })
  })

  it('보이던 탭 ≠ 영역 view_id 여도 탭바 가이드는 그리고 탭바 놓기를 보낸다(5.2)', async () => {
    await mounted()
    setActive('main', 'v-other')
    emit(EVT_DOCK_REGIONS, regions({ view_id: 'v-main', rev: 4, bar_rev: 2 }))
    pick.mockReturnValue(bar(1))
    drag('dragover', { x: 120, y: 10 })
    expect(ctl.getGuide()?.shown).toEqual(bar(1))
    drag('drop', { x: 120, y: 10, data: dragData() })
    await flush()
    expect(cmdCalls('dock_drop')).toEqual([
      {
        drag_id: 'd1',
        view_id: 'v-src',
        rev: 4,
        bar_rev: 2,
        displayed: 'v-other',
        shown: bar(1),
        point: { x: 120, y: 10 },
      },
    ])
  })

  it('shown · rev · displayed 가 바뀔 때만 Over 를 보낸다(움직일 때마다가 아니다)', async () => {
    await mounted()
    emit(EVT_DOCK_REGIONS, regions())
    pick.mockReturnValue(seam('s1'))
    drag('dragenter', { x: 5, y: 40 })
    drag('dragover', { x: 6, y: 41 })
    drag('dragover', { x: 7, y: 42 })
    await flush()
    expect(cmdCalls('dock_drag_over')).toHaveLength(1)

    pick.mockReturnValue(seam('s1', 'Top'))
    drag('dragover', { x: 8, y: 30 })
    await flush()
    expect(cmdCalls('dock_drag_over')).toHaveLength(2)

    // 새 rev — 움직이지 않아도 마지막 점에서 다시 판정해 보고한다.
    emit(EVT_DOCK_REGIONS, regions({ rev: 2 }))
    await flush()
    expect(cmdCalls('dock_drag_over')).toHaveLength(3)
    expect(cmdCalls('dock_drag_over')[2]).toMatchObject({ report: { rev: 2, point: { x: 8, y: 30 } } })

    // 보이던 탭이 바뀜(모델 활성 전환 알림).
    setActive('main', 'v-other')
    await flush()
    expect(cmdCalls('dock_drag_over')).toHaveLength(4)
    expect(cmdCalls('dock_drag_over')[3]).toMatchObject({ report: { displayed: 'v-other' } })

    drag('dragover', { x: 9, y: 31 })
    await flush()
    expect(cmdCalls('dock_drag_over')).toHaveLength(4)
  })

  it('drag_id 를 모르면 보고하지 않는다 — 떠남 · 놓기 전', async () => {
    await mounted()
    drag('dragenter')
    drag('dragover')
    drag('dragleave', { related: null })
    await flush()
    expect(cmdCalls('dock_drag_over')).toEqual([])
  })

  it('문서 밖으로 나간 dragleave = Left · 요소 사이를 건넌 것은 아니다', async () => {
    await mounted()
    emit(EVT_DOCK_REGIONS, regions())
    pick.mockReturnValue(seam('s1'))
    drag('dragenter')
    drag('dragover')
    drag('dragleave', { related: document.body })
    await flush()
    expect(cmdCalls('dock_drag_over').map(a => (a?.report as { type: string }).type)).toEqual(['over'])
    expect(ctl.getGuide()).not.toBeNull()

    drag('dragleave', { related: null })
    await flush()
    expect(cmdCalls('dock_drag_over').map(a => (a?.report as { type: string }).type)).toEqual(['over', 'left'])
    expect(cmdCalls('dock_drag_over')[1]).toEqual({ drag_id: 'd1', report: { type: 'left' } })
    expect(ctl.getGuide()).toBeNull()

    // 다시 들어오면 같은 판정이어도 다시 보고한다.
    drag('dragenter')
    await flush()
    expect(cmdCalls('dock_drag_over')).toHaveLength(3)
  })

  it('같은 창의 Over · Left 는 한 줄로 보낸다 — 앞 보고의 응답 전에 다음 보고를 내지 않는다', async () => {
    const answer: Array<() => void> = []
    const base = ipc.invoke
    ipc.invoke = async <T>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
      if (cmd !== 'dock_drag_over') return base<T>(cmd, args)
      calls.push([cmd, args])
      await new Promise<void>(res => answer.push(res))
      return undefined as T
    }
    await mounted()
    emit(EVT_DOCK_REGIONS, regions())
    pick.mockReturnValue(seam('s1'))
    drag('dragenter')
    drag('dragleave', { related: null })
    await flush()
    expect(reportTypes()).toEqual(['over'])

    answer.shift()?.()
    await flush()
    expect(reportTypes()).toEqual(['over', 'left'])
  })

  it('drop = dock_drop{drag_id, view_id, rev, bar_rev, displayed, shown, point} — 마지막으로 그린 것을 그 점과 함께', async () => {
    await mounted()
    emit(EVT_DOCK_REGIONS, regions({ rev: 3, bar_rev: 2 }))
    pick.mockReturnValue(seam('s1', 'Bottom'))
    drag('dragover', { x: 400, y: 590 })
    const drop = drag('drop', { x: 401, y: 591, data: dragData('d1', 'v-src') })
    await flush()
    expect(drop.e.defaultPrevented).toBe(true)
    expect(cmdCalls('dock_drop')).toEqual([
      {
        drag_id: 'd1',
        view_id: 'v-src',
        rev: 3,
        bar_rev: 2,
        displayed: 'v-main',
        shown: seam('s1', 'Bottom'),
        point: { x: 400, y: 590 },
      },
    ])
    expect(ctl.getGuide()).toBeNull()
    // 놓은 뒤에는 Left 를 내지 않는다.
    expect(cmdCalls('dock_drag_over').map(a => (a?.report as { type: string }).type)).toEqual(['over'])
  })

  it('모르는 id 의 drop 도 보낸다 — rev 0 · 가이드 없음 · 놓은 점(셸이 이른 끝 신호로 붙든다)', async () => {
    await mounted()
    drag('dragover', { x: 3, y: 4 })
    drag('drop', { x: 5, y: 6, data: dragData('d9', 'v-x') })
    await flush()
    expect(cmdCalls('dock_drop')).toEqual([
      {
        drag_id: 'd9',
        view_id: 'v-x',
        rev: 0,
        bar_rev: 0,
        displayed: 'v-main',
        shown: NONE,
        point: { x: 5, y: 6 },
      },
    ])
  })

  it('끌기 데이터를 못 읽으면 놓기를 보내지 않는다', async () => {
    await mounted()
    vi.spyOn(console, 'warn').mockImplementation(() => undefined)
    drag('drop', { data: 'not json' })
    await flush()
    expect(cmdCalls('dock_drop')).toEqual([])
  })

  it('영역 없이 들어오면 dock_pull 한 번 — 답의 영역 · 세션을 받아 그린다', async () => {
    await mounted()
    pullReply = { started: startedFields(), regions: regions({ rev: 2 }) }
    pick.mockReturnValue(seam('s1'))
    drag('dragenter')
    drag('dragenter')
    drag('dragover')
    await flush()
    expect(cmdCalls('dock_pull')).toHaveLength(1)
    expect(ctl.getGuide()?.regions.rev).toBe(2)
    expect(ctl.getSession()).toEqual({ drag_id: 'd1', ...startedFields() })
    expect(pick).toHaveBeenLastCalledWith(regions({ rev: 2 }), { x: 10, y: 50 }, 'v-src')

    // 영역을 아는 채로 다시 들어오면 당기지 않는다.
    drag('dragleave', { related: null })
    drag('dragenter')
    await flush()
    expect(cmdCalls('dock_pull')).toHaveLength(1)
  })

  it('당긴 세션은 진짜 세션 사건에 진다', async () => {
    await mounted()
    pullReply = { started: startedFields({ display_active: 'old' }), regions: regions() }
    drag('dragenter')
    await flush()
    emit(EVT_DOCK_SESSION, started('d1', 0, { display_active: 'new' }))
    expect(ctl.getSession()?.display_active).toBe('new')
  })

  it('보이지 않는 탭 머리 위 500ms 머묾 = 탭 전환 한 번', async () => {
    await mounted()
    emit(EVT_DOCK_REGIONS, regions())
    pick.mockReturnValue(bar(2, 'v-other'))
    drag('dragover', { x: 150, y: 10 })
    expect(ctl.getGuide()?.hover?.viewId).toBe('v-other')
    vi.advanceTimersByTime(HOVER_SWITCH_MS - 1)
    drag('dragover', { x: 151, y: 10 })
    expect(switchTab).not.toHaveBeenCalled()
    vi.advanceTimersByTime(1)
    expect(switchTab).toHaveBeenCalledExactlyOnceWith('main', 'v-other')
    expect(ctl.getGuide()?.hover).toBeNull()
    // 같은 머리에 계속 머물러도 다시 걸지 않는다.
    drag('dragover', { x: 152, y: 10 })
    vi.advanceTimersByTime(HOVER_SWITCH_MS * 2)
    expect(switchTab).toHaveBeenCalledTimes(1)
  })

  it('보이는 탭의 머리 위에서는 머묾을 재지 않는다', async () => {
    await mounted()
    emit(EVT_DOCK_REGIONS, regions())
    pick.mockReturnValue(bar(0, 'v-main'))
    drag('dragover', { x: 50, y: 10 })
    expect(ctl.getGuide()?.hover).toBeNull()
    vi.advanceTimersByTime(HOVER_SWITCH_MS * 2)
    expect(switchTab).not.toHaveBeenCalled()
  })

  it('머리를 떠남 · 창 떠남 · Ended 가 머묾을 취소한다', async () => {
    await mounted()
    emit(EVT_DOCK_REGIONS, regions())
    pick.mockReturnValue(bar(2, 'v-other'))

    drag('dragover')
    vi.advanceTimersByTime(300)
    pick.mockReturnValue(bar(2, null))
    drag('dragover')
    vi.advanceTimersByTime(HOVER_SWITCH_MS)
    expect(switchTab).not.toHaveBeenCalled()

    pick.mockReturnValue(bar(2, 'v-other'))
    drag('dragover')
    drag('dragleave', { related: null })
    vi.advanceTimersByTime(HOVER_SWITCH_MS)
    expect(switchTab).not.toHaveBeenCalled()

    drag('dragenter')
    emit(EVT_DOCK_SESSION, ended('d1', 3))
    vi.advanceTimersByTime(HOVER_SWITCH_MS)
    expect(switchTab).not.toHaveBeenCalled()
    expect(ctl.getGuide()).toBeNull()
  })
})

// ── 영역 · 세션 ────────────────────────────────────────────────────────────────

describe('영역 · 세션', () => {
  it('dock:regions 는 자기 label 로, dock:session 은 전체로 구독한다', async () => {
    await mounted('pop-2')
    expect(handlers.get(EVT_DOCK_REGIONS)?.target).toBe('pop-2')
    expect(handlers.get(EVT_DOCK_SESSION)?.target).toBeNull()
  })

  it('drag_id 별 가장 큰 rev 만 쓴다 — 늦게 닿은 작은 rev 는 버린다', async () => {
    await mounted()
    emit(EVT_DOCK_REGIONS, regions({ rev: 3 }))
    emit(EVT_DOCK_REGIONS, regions({ rev: 2 }))
    emit(EVT_DOCK_REGIONS, regions({ rev: 3, view_id: 'other-same-rev' }))
    drag('dragover')
    expect(ctl.getGuide()?.regions.rev).toBe(3)
    expect(ctl.getGuide()?.regions.view_id).toBe('v-main')
  })

  it('drag_id 별 가장 큰 seq 만 쓴다 — Display 가 Started 보다 먼저 와도 같은 상태', async () => {
    await mounted()
    emit(EVT_DOCK_SESSION, display('d1', 2, { display_active: 'v-next' }))
    emit(EVT_DOCK_SESSION, started('d1', 0, { display_active: null }))
    const a = ctl.getSession()

    release?.()
    await mounted()
    emit(EVT_DOCK_SESSION, started('d1', 0, { display_active: null }))
    emit(EVT_DOCK_SESSION, display('d1', 2, { display_active: 'v-next' }))
    expect(ctl.getSession()).toEqual(a)
    expect(a).toEqual({ drag_id: 'd1', ...startedFields({ display_active: 'v-next' }) })
  })

  it('Started 전에 온 영역도 받아 그리고 그 id 로 보고한다', async () => {
    await mounted()
    emit(EVT_DOCK_REGIONS, regions({ drag_id: 'd7' }))
    pick.mockReturnValue(seam(null, 'Top'))
    drag('dragover')
    await flush()
    expect(ctl.getSession()).toBeNull()
    expect(ctl.getGuide()?.shown).toEqual(seam(null, 'Top'))
    expect(pick).toHaveBeenLastCalledWith(expect.anything(), expect.anything(), '')
    expect(cmdCalls('dock_drag_over')).toEqual([
      expect.objectContaining({ drag_id: 'd7', report: expect.objectContaining({ type: 'over' }) }),
    ])
  })

  it('Ended 된 id 의 늦은 영역 · 세션 사건은 버린다', async () => {
    await mounted()
    emit(EVT_DOCK_SESSION, started('d1', 0))
    emit(EVT_DOCK_SESSION, ended('d1', 4))
    emit(EVT_DOCK_SESSION, display('d1', 2))
    emit(EVT_DOCK_REGIONS, regions({ rev: 9 }))
    drag('dragover')
    await flush()
    expect(ctl.getSession()).toBeNull()
    expect(ctl.getGuide()).toBeNull()
    expect(cmdCalls('dock_drag_over')).toEqual([])
  })

  it('먼저 닿은 Ended 도 그 id 를 묻는다(Started 를 못 받은 창)', async () => {
    await mounted()
    emit(EVT_DOCK_SESSION, ended('d1', 4))
    emit(EVT_DOCK_SESSION, started('d1', 0))
    expect(ctl.getSession()).toBeNull()
  })

  it('그린 끌기의 Ended 는 가이드 · 그린 것을 무조건 지운다', async () => {
    await mounted()
    emit(EVT_DOCK_SESSION, started('d1', 0))
    emit(EVT_DOCK_REGIONS, regions())
    pick.mockReturnValue(seam('s1'))
    drag('dragover', { x: 20, y: 60 })
    expect(ctl.getGuide()).not.toBeNull()
    emit(EVT_DOCK_SESSION, ended('d1', 5))
    expect(ctl.getGuide()).toBeNull()
    drag('drop', { x: 21, y: 61, data: dragData('d1') })
    await flush()
    expect(cmdCalls('dock_drop')).toEqual([expect.objectContaining({ rev: 0, shown: NONE, point: { x: 21, y: 61 } })])
  })

  it('늦게 닿은 옛 Ended 는 새 끌기의 가이드 · 놓기 짐을 지우지 않는다 · 지금 끌기 = 끝나지 않은 것 중 가장 늦게 안 id', async () => {
    await mounted()
    emit(EVT_DOCK_SESSION, started('d1', 0))
    emit(EVT_DOCK_REGIONS, regions())
    pick.mockReturnValue(seam('s1'))
    drag('dragover')
    emit(EVT_DOCK_SESSION, started('d2', 0, { source_view: 'v-2' }))
    emit(EVT_DOCK_REGIONS, regions({ drag_id: 'd2', rev: 5, bar_rev: 3 }))
    pick.mockReturnValue(seam('s1', 'Top'))
    drag('dragover', { x: 300, y: 40 })
    const guide = ctl.getGuide()
    expect(guide?.regions.drag_id).toBe('d2')

    emit(EVT_DOCK_SESSION, ended('d1', 5))
    expect(ctl.getGuide()).toBe(guide)
    expect(ctl.getSession()?.drag_id).toBe('d2')

    // 다음 dragover 전의 놓기 — 그린 것을 그대로 싣는다.
    drag('drop', { x: 301, y: 41, data: dragData('d2', 'v-2') })
    await flush()
    expect(cmdCalls('dock_drop')).toEqual([
      {
        drag_id: 'd2',
        view_id: 'v-2',
        rev: 5,
        bar_rev: 3,
        displayed: 'v-main',
        shown: seam('s1', 'Top'),
        point: { x: 300, y: 40 },
      },
    ])
  })

  it('모르는 id 로 안에 있던 창은 Ended 에 「안에 있음」 · 점을 거둔다 — 다음 끌기의 영역이 유령 가이드 · Over 를 내지 않는다', async () => {
    await mounted()
    drag('dragenter', { x: 30, y: 60 })
    drag('dragover', { x: 31, y: 61 })
    await flush()
    emit(EVT_DOCK_SESSION, ended('x', 2))
    pick.mockReturnValue(seam('s1'))
    emit(EVT_DOCK_REGIONS, regions({ drag_id: 'd2' }))
    await flush()
    expect(ctl.getGuide()).toBeNull()
    expect(cmdCalls('dock_drag_over')).toEqual([])
  })

  it('읽을 수 없는 짐은 버린다', async () => {
    await mounted()
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined)
    emit(EVT_DOCK_SESSION, { drag_id: 'd1' })
    emit(EVT_DOCK_REGIONS, { rev: 1 })
    expect(ctl.getSession()).toBeNull()
    expect(warn).toHaveBeenCalledTimes(2)
  })
})

// ── 보이던 탭 · 떼어진 표시 ───────────────────────────────────────────────────────

describe('보이던 탭', () => {
  const s = { drag_id: 'd1', source_label: 'pop-1', source_view: 'v-src', display_active: 'v-next' }

  it('displayedTabOf — 출처 창의 활성이 끄는 탭이면 승계 탭 · 아니면 모델 활성', () => {
    expect(displayedTabOf('v-src', 'pop-1', s)).toBe('v-next')
    expect(displayedTabOf('v-src', 'pop-1', { ...s, display_active: null })).toBeNull()
    expect(displayedTabOf('v-a', 'pop-1', s)).toBe('v-a')
    expect(displayedTabOf('v-src', 'main', s)).toBe('v-src')
    expect(displayedTabOf('v-a', 'pop-1', null)).toBe('v-a')
    expect(displayedTabOf(null, 'pop-1', null)).toBeNull()
  })

  it('출처 창은 떼어진 표시의 승계 탭을 displayed 로 싣는다', async () => {
    active = { 'pop-1': 'v-src' }
    await mounted('pop-1')
    emit(EVT_DOCK_SESSION, started('d1', 0, { display_active: 'v-next' }))
    emit(EVT_DOCK_REGIONS, regions({ view_id: 'v-next' }))
    pick.mockReturnValue(seam('s1'))
    drag('dragover')
    await flush()
    expect(ctl.getGuide()?.canvas).toBe(true)
    expect(lastOf(cmdCalls('dock_drag_over'))).toMatchObject({ report: { displayed: 'v-next', shown: seam('s1') } })
  })

  it('displayed 입력을 넘기면 그것을 쓴다', async () => {
    await mounted('main', { displayed: () => 'v-forced' })
    emit(EVT_DOCK_REGIONS, regions({ view_id: 'v-forced' }))
    pick.mockReturnValue(seam('s1'))
    drag('dragover')
    await flush()
    expect(lastOf(cmdCalls('dock_drag_over'))).toMatchObject({ report: { displayed: 'v-forced', shown: seam('s1') } })
  })
})

// ── pointer_seen ──────────────────────────────────────────────────────────────

function pointer(type: string, pointerType: string, buttons: number): void {
  window.dispatchEvent(new PointerEvent(type, { bubbles: true, pointerType, buttons }))
}

describe('pointer_seen', () => {
  it('세션 전에는 보내지 않는다', async () => {
    await mounted()
    pointer('pointermove', 'mouse', 0)
    await flush()
    expect(cmdCalls('dock_pointer_seen')).toEqual([])
  })

  it('마우스 · 버튼 0 이동 하나만 — 세션당 창당 한 번 · 버튼 쥔 이동 · 펜 · 터치는 아니다', async () => {
    await mounted()
    emit(EVT_DOCK_SESSION, started('d1', 0))
    pointer('pointermove', 'mouse', 1)
    pointer('pointerdown', 'pen', 1)
    pointer('pointermove', 'touch', 0)
    pointer('pointerdown', 'touch', 1)
    await flush()
    expect(cmdCalls('dock_pointer_seen')).toEqual([])

    pointer('pointermove', 'mouse', 0)
    pointer('pointermove', 'mouse', 0)
    pointer('pointerdown', 'mouse', 1)
    await flush()
    expect(cmdCalls('dock_pointer_seen')).toEqual([{ drag_id: 'd1' }])

    emit(EVT_DOCK_SESSION, ended('d1', 1))
    emit(EVT_DOCK_SESSION, started('d2', 0))
    pointer('pointermove', 'mouse', 0)
    await flush()
    expect(cmdCalls('dock_pointer_seen')).toEqual([{ drag_id: 'd1' }, { drag_id: 'd2' }])
  })

  it('포인터 사건은 그린 것을 거두지만 Left 는 보내지 않는다 — 장부의 유실 대비가 이 창을 「아직 위」로 친다', async () => {
    await mounted()
    emit(EVT_DOCK_SESSION, started('d1', 0))
    emit(EVT_DOCK_REGIONS, regions())
    pick.mockReturnValue(seam('s1'))
    drag('dragenter')
    await flush()
    expect(reportTypes()).toEqual(['over'])
    pointer('pointermove', 'mouse', 0)
    pointer('pointerdown', 'mouse', 1)
    await flush()
    expect(ctl.getGuide()).toBeNull()
    expect(reportTypes()).toEqual(['over'])
    expect(cmdCalls('dock_pointer_seen')).toEqual([{ drag_id: 'd1' }])
  })

  it('마우스 pointerdown 도 끝 증거다', async () => {
    await mounted()
    emit(EVT_DOCK_SESSION, started('d1', 0))
    pointer('pointerdown', 'mouse', 1)
    await flush()
    expect(cmdCalls('dock_pointer_seen')).toEqual([{ drag_id: 'd1' }])
  })
})

// ── Esc ──────────────────────────────────────────────────────────────────────

describe('Esc 삼키기', () => {
  let docSeen: string[]
  const docSpy = (e: KeyboardEvent): void => {
    docSeen.push(e.key)
  }
  beforeEach(() => {
    docSeen = []
    document.addEventListener('keydown', docSpy)
  })
  afterEach(() => {
    document.removeEventListener('keydown', docSpy)
  })

  function key(k: string): KeyboardEvent {
    const e = new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true })
    document.body.dispatchEvent(e)
    return e
  }

  it('세션 밖에서는 그대로 지나간다', async () => {
    await mounted()
    const e = key('Escape')
    expect(e.defaultPrevented).toBe(false)
    expect(docSeen).toEqual(['Escape'])
  })

  it('세션 동안 capture 단계에서 삼킨다 — 다른 키는 그대로', async () => {
    await mounted()
    emit(EVT_DOCK_SESSION, started('d1', 0))
    const e = key('Escape')
    key('a')
    expect(e.defaultPrevented).toBe(true)
    expect(docSeen).toEqual(['a'])
  })

  it('Ended 뒤 200ms 까지 삼키고 그 뒤로는 지나간다', async () => {
    await mounted()
    emit(EVT_DOCK_SESSION, started('d1', 0))
    emit(EVT_DOCK_SESSION, ended('d1', 1))
    vi.advanceTimersByTime(ESC_SWALLOW_TAIL_MS - 1)
    expect(key('Escape').defaultPrevented).toBe(true)
    vi.advanceTimersByTime(1)
    expect(key('Escape').defaultPrevented).toBe(false)
    expect(docSeen).toEqual(['Escape'])
  })

  it('그 창의 끝 증거 뒤 200ms 에서 그친다 — 세션이 아직 살아 있어도', async () => {
    await mounted()
    emit(EVT_DOCK_SESSION, started('d1', 0))
    pointer('pointermove', 'mouse', 0)
    vi.advanceTimersByTime(ESC_SWALLOW_TAIL_MS - 1)
    expect(key('Escape').defaultPrevented).toBe(true)
    vi.advanceTimersByTime(1)
    expect(key('Escape').defaultPrevented).toBe(false)
    // 그친 뒤의 Ended 가 꼬리를 되살리지 않는다.
    emit(EVT_DOCK_SESSION, ended('d1', 1))
    expect(key('Escape').defaultPrevented).toBe(false)
  })

  it('Started 전(영역만 받음)에는 삼키지 않는다', async () => {
    await mounted()
    emit(EVT_DOCK_REGIONS, regions())
    expect(key('Escape').defaultPrevented).toBe(false)
  })
})

// ── 떼기 ─────────────────────────────────────────────────────────────────────

describe('떼기', () => {
  it('떼면 리스너 · 구독이 모두 풀린다', async () => {
    await mounted()
    emit(EVT_DOCK_SESSION, started('d1', 0))
    release?.()
    release = null
    expect(unlistened.sort()).toEqual([EVT_DOCK_REGIONS, EVT_DOCK_SESSION].sort())
    expect(activeSubs.size).toBe(0)
    const over = drag('dragover')
    expect(over.e.defaultPrevented).toBe(false)
    const esc = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })
    document.body.dispatchEvent(esc)
    expect(esc.defaultPrevented).toBe(false)
    expect(ctl.getSession()).toBeNull()
  })

  it('구독 등록이 뗀 뒤에 풀리면 곧바로 해제한다', async () => {
    const pending: Array<(off: () => void) => void> = []
    const off = vi.fn()
    ipc.listen = vi.fn(() => new Promise<() => void>(res => pending.push(res)))
    setup()
    release?.()
    release = null
    expect(pending).toHaveLength(2)
    for (const res of pending) res(off)
    await flush()
    expect(off).toHaveBeenCalledTimes(2)
  })

  it('두 번 붙이면 마지막 떼기에서만 푼다', async () => {
    await mounted()
    const second = ctl.mount('main')
    second()
    expect(drag('dragover').e.defaultPrevented).toBe(true)
    release?.()
    release = null
    expect(drag('dragover').e.defaultPrevented).toBe(false)
  })
})

// ── Tauri 어댑터 ──────────────────────────────────────────────────────────────

describe('mountDockDrag — Tauri 어댑터', () => {
  it('dock:regions 는 자기 label 을 target 으로, dock:session 은 기본(전체)으로 listen 한다', async () => {
    __resetDockDragForTest()
    const off = mountDockDrag('slot-popup-3')
    try {
      await flush()
      const byEvent = new Map(tauriListen.mock.calls.map(c => [c[0], c[2]]))
      expect(byEvent.get(EVT_DOCK_REGIONS)).toEqual({ target: 'slot-popup-3' })
      expect(byEvent.has(EVT_DOCK_SESSION)).toBe(true)
      expect(byEvent.get(EVT_DOCK_SESSION)).toBeUndefined()
    } finally {
      off()
      __resetDockDragForTest()
    }
  })
})
