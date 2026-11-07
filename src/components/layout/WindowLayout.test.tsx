import { StrictMode, useEffect, useState } from 'react'
import { act, cleanup, createEvent, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ── listen mock: keep handlers per event name → the test emits directly ──
const listeners = new Map<string, (e: { payload: unknown }) => void>()
/** Event name → registration options (observes `target`). */
const listenOptions = new Map<string, unknown>()
/** IPC order observation — `listen:<이벤트>` · `invoke:window_role`. */
const ipcOrder: string[] = []
const unlistenMock = vi.fn()
let listenShouldReject = false
/** Event names whose registration is rejected (`listenShouldReject` = all of them). */
const rejectListen = new Set<string>()
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: (e: { payload: unknown }) => void, options?: unknown) => {
    ipcOrder.push(`listen:${event}`)
    if (listenShouldReject || rejectListen.has(event)) throw new Error('listen registration failed')
    listeners.set(event, handler)
    listenOptions.set(event, options)
    return unlistenMock
  }),
}))

// ── invoke mock: list_tabs/get_view pull · window_role separate (default = model window) ──
const invokeMock = vi.fn(async (_cmd: string, ..._rest: unknown[]) => undefined as unknown)
const windowRoleMock = vi.fn(async (_args?: unknown): Promise<unknown> => ({ role: 'model', generation: null }))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, ...rest: unknown[]) => {
    if (cmd !== 'window_role') return invokeMock(cmd, ...rest)
    ipcOrder.push('invoke:window_role')
    return windowRoleMock(rest[0])
  },
  Channel: class {
    onmessage: unknown = null
  },
}))

// ── First paint notice — observes only when it is sent (how it is sent is in `theme/firstPaint.test.ts`) ──
const reportFirstPaintMock = vi.hoisted(() => vi.fn())
vi.mock('../../theme/firstPaint', () => ({ reportFirstPaint: reportFirstPaintMock }))

// ── getCurrentWindow mock — label the events are subscribed for ──
const currentWindow = vi.hoisted(() => ({ label: 'main' }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ label: currentWindow.label }),
}))

/** Observation of 0-tab self-close — the window close requested from the shell (`window_control`). */
const closeCalls = () => invokeMock.mock.calls.filter(c => c[0] === 'window_control')
const KEPT = { deferred: false, pending: false, shown_all: false, focused: false, close: 'kept' }
const CLOSED = { ...KEPT, close: 'closed' }

// ── ViewLayoutRenderer stub — the canvas inside is out of scope (observes only which view it draws) + ★mount counter★.
// ★S4-F5 keep-alive no-remount★: to proxy whether the slot component is not remounted on tab switch (terminal instance survives),
//   on mount useEffect([]) raises a counter per viewId by 1. If the count does not grow after a switch and only display
//   is toggled, the keep-alive ("lossless switch", ADR-0056) structure holds.
const mountCounts = vi.hoisted(() => new Map<string, number>())
/** viewIdOverride → the props that renderer last received. */
const rendererProps = vi.hoisted(() => new Map<string, Record<string, unknown>>())
vi.mock('./ViewLayoutRenderer', async () => {
  const React = (await import('react')).default
  return {
    default: (props: { viewIdOverride?: string | null } & Record<string, unknown>) => {
      const id = props.viewIdOverride ?? ''
      rendererProps.set(id, props)
      React.useEffect(() => {
        mountCounts.set(id, (mountCounts.get(id) ?? 0) + 1)
      }, [id])
      return <div data-testid="view-renderer" data-view-id={id} />
    },
  }
})

vi.mock('@tauri-apps/api/app', () => ({ getIdentifier: async () => 'com.engram.test' }))

import '../../commands/tabCommands' // Registration of `window.close`, which 0-tab self-close calls (in production it is the contributions of `App.tsx`)
import WindowLayout from './WindowLayout'
import { __resetCanvasReportForTest } from './windowCanvasReport'
import { __resetUiMetricsReportForTest } from './uiMetricsReport'
import { __resetMinSizeReportForTest, MIN_SIZE_WAIT_CAP_MS } from './titleRow/minSizeReport'
import { localDrag } from './dock/localDrag'
import { __resetDragSourceForTest } from './tabStrip/useTabDragSource'
import { buildTabDragType, buildTabDragValue } from './dock/tabDragData'
import { useViewStore } from '../../store/viewStore'
import type { ViewSnapshot } from '../../api/layoutTypes'

function slotSnap(viewId: string, version: number): ViewSnapshot {
  return {
    view_id: viewId,
    layout: { type: 'slot', id: `s-${viewId}`, content: { type: 'empty' } }, // ADR-0060
    focused_slot_id: `s-${viewId}`,
    foreign_slots: [],
    slot_spatial: [], // ADR-0068: space derivation (this test does not use it — an empty array satisfies the type)
    slot_rects: [], // ADR-0227: shell geometry (this test does not use it)
    split_rects: [],
    ratio_min: 0.1,
    ratio_max: 0.9,
    version,
  }
}

function emit(event: string, payload: unknown): void {
  const h = listeners.get(event)
  if (!h) throw new Error(`no listener for ${event}`)
  h({ payload })
}

beforeEach(() => {
  listeners.clear()
  listenOptions.clear()
  ipcOrder.length = 0
  currentWindow.label = 'main'
  windowRoleMock.mockReset()
  windowRoleMock.mockImplementation(async () => ({ role: 'model', generation: null }))
  unlistenMock.mockClear()
  reportFirstPaintMock.mockReset()
  invokeMock.mockReset()
  mountCounts.clear()
  rendererProps.clear()
  listenShouldReject = false
  rejectListen.clear()
  __resetMinSizeReportForTest()
  useViewStore.setState({ layouts: {}, windows: {}, renderModeOverride: {} })
  invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd === 'list_tabs') {
      return {
        label: (args as { window: string }).window,
        window_id: `wid-${(args as { window: string }).window}`,
        tabs: [{ id: 'v1', name: 'Tab 1', named: true }, { id: 'v2', name: 'Tab 2', named: true }],
        active: 'v1',
        version: 1,
      }
    }
    if (cmd === 'get_view') return slotSnap((args as { viewId: string }).viewId, 1)
    if (cmd === 'window_control') return KEPT
    return undefined
  })
})

afterEach(cleanup)

describe('WindowLayout — 초기 pull + keep-alive 캔버스', () => {
  it('mount 시 list_tabs(label) pull → 탭바 + 모든 탭 캔버스 마운트(keep-alive)', async () => {
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
    expect(invokeMock).toHaveBeenCalledWith('list_tabs', { window: 'main' })
    const canvases = screen.getAllByTestId('tab-canvas')
    expect(canvases).toHaveLength(2)
    const v1 = canvases.find(c => c.getAttribute('data-view-id') === 'v1')!
    const v2 = canvases.find(c => c.getAttribute('data-view-id') === 'v2')!
    expect(v1.style.display).toBe('block')
    expect(v2.style.display).toBe('none')
  })

  it('각 탭 캔버스에 그 view 를 get_view 로 채워 ViewLayoutRenderer 에 viewIdOverride 로 내려꽂는다', async () => {
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getAllByTestId('view-renderer').length).toBe(2))
    expect(invokeMock).toHaveBeenCalledWith('get_view', { viewId: 'v1' })
    expect(invokeMock).toHaveBeenCalledWith('get_view', { viewId: 'v2' })
    const renderers = screen.getAllByTestId('view-renderer')
    const ids = renderers.map(r => r.getAttribute('data-view-id')).sort()
    expect(ids).toEqual(['v1', 'v2'])
  })

  // The rectangle array must be the cache reference as is — the divider preview recognizes a new snapshot by array reference (ADR-0227).
  it('캐시 항목의 사각형(같은 참조)·비율 한계·version·모르는 내용 슬롯을 렌더러에 넘긴다', async () => {
    render(<WindowLayout label="main" />)
    await waitFor(() => {
      const cached = useViewStore.getState().layouts['v1']
      const props = rendererProps.get('v1')
      expect(cached).toBeTruthy()
      expect(props).toBeTruthy()
      expect(props!.slotRects).toBe(cached!.slotRects)
      expect(props!.splitRects).toBe(cached!.splitRects)
      expect(props!.ratioBounds).toEqual({ min: cached!.ratioMin, max: cached!.ratioMax })
      expect(props!.version).toBe(cached!.version)
      expect(props!.foreignSlots).toBe(cached!.foreignSlots)
    })
  })
})

// ── ★S4-F4: mount-race — a newer window:tabs-updated arrives during the initial list_tabs pull await★ ──────────────
// Restores at component level the class that the old viewStore.test.ts deferred init-race harness verified.
// Scenario: WindowLayout mount → listen registered first (§7-1 "subscribe first, pull later") → while the list_tabs pull is
//   pending a newer-version window:tabs-updated arrives → the pull later
//   resolves with a stale payload → assert that the version guard of applyWindowTabsUpdated blocks the stale pull's overwrite.
describe('WindowLayout — mount-race(초기 pull vs 최신 emit, S4-F4)', () => {
  it('list_tabs pull 이 pending 인 동안 더 최신 emit 도착 → 늦게 온 stale pull 이 최신 상태를 덮지 않는다', async () => {
    let resolveListTabs: (payload: unknown) => void = () => {}
    const listTabsPending = new Promise<unknown>(res => {
      resolveListTabs = res
    })
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_tabs') return listTabsPending
      if (cmd === 'get_view') return slotSnap((args as { viewId: string }).viewId, 1)
      return undefined
    })

    render(<WindowLayout label="slot-popup-1" />)

    await waitFor(() => expect(listeners.has('window:tabs-updated')).toBe(true))

    emit('window:tabs-updated', {
      label: 'slot-popup-1',
      tabs: [{ id: 'v1', name: 'Tab 1', named: true }, { id: 'v2', name: 'Tab 2', named: true }],
      active: 'v2',
      version: 5,
    })
    await waitFor(() => expect(useViewStore.getState().windows['slot-popup-1']?.version).toBe(5))

    resolveListTabs({
      label: 'slot-popup-1',
      tabs: [{ id: 'v1', name: 'Tab 1', named: true }, { id: 'v2', name: 'Tab 2', named: true }],
      active: 'v1',
      version: 1,
    })
    await Promise.resolve()
    await Promise.resolve()

    const win = useViewStore.getState().windows['slot-popup-1']
    expect(win.version).toBe(5)
    expect(win.active).toBe('v2')
  })
})

// ── ★ADR-0102: boot pull bounded retry + surfacing the final failure★ ──────────────────────────────────────
// main has no event recovery path (window:tabs-updated fires only on tab mutation), so if the boot list_tabs pull is
//   one-shot, an early failure = loading stuck forever.
describe('WindowLayout — 부팅 pull 재시도/표면화(ADR-0102)', () => {
  it('list_tabs 가 2번 reject 후 resolve → 재시도로 회수해 탭바가 뜬다', async () => {
    let listTabsCalls = 0
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_tabs') {
        listTabsCalls += 1
        if (listTabsCalls <= 2) throw new Error(`state not ready ${listTabsCalls}`)
        return {
          label: (args as { window: string }).window,
          tabs: [{ id: 'v1', name: 'Tab 1', named: true }],
          active: 'v1',
          version: 1,
        }
      }
      if (cmd === 'get_view') return slotSnap((args as { viewId: string }).viewId, 1)
      return undefined
    })

    render(<WindowLayout label="main" />)
    // Includes backoff waits → a generous timeout.
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy(), { timeout: 3000 })
    expect(listTabsCalls).toBeGreaterThanOrEqual(3) // First attempt + 2 or more retries.
    expect(screen.queryByTestId('window-boot-error')).toBeNull()
  })

  it('list_tabs 가 계속 reject → 재시도 소진 후 가시적 에러 상태 렌더(로딩 고착 아님)', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_tabs') throw new Error('backend down')
      return undefined
    })

    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getByTestId('window-boot-error')).toBeTruthy(), {
      timeout: 3000,
    })
    expect(screen.queryByTestId('tab-bar')).toBeNull()
  })

  // In the old structure the listen() await was outside the try, so if listener registration rejected, the async IIFE died as unhandled and
  //   the list_tabs pull never even started, so bootFailed was never set (loading placeholder stuck forever — no signal).
  //   Now a listen failure is surfaced as bootFailed just like a pull failure.
  it('listen 등록이 reject → 조용한 로딩 고착이 아니라 boot-error 표면화(FIX-2)', async () => {
    listenShouldReject = true
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getByTestId('window-boot-error')).toBeTruthy(), {
      timeout: 3000,
    })
    expect(screen.queryByTestId('tab-bar')).toBeNull()
  })
})

// ── First screen and first screen notice — a window the shell built hidden is shown on the notice (`theme/firstPaint.ts`) ──
describe('WindowLayout — 첫 화면 · 첫 페인트 알림', () => {
  it('탭을 받기 전은 바탕만(문구 없음) · 알림은 탭바와 활성 탭의 레이아웃이 그려진 뒤 한 번', async () => {
    let answerTabs: (payload: unknown) => void = () => {}
    let answerActiveView: (snap: ViewSnapshot) => void = () => {}
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_tabs') return new Promise(res => (answerTabs = res))
      if (cmd === 'get_view') {
        const viewId = (args as { viewId: string }).viewId
        if (viewId === 'v1') return new Promise<ViewSnapshot>(res => (answerActiveView = res))
        return slotSnap(viewId, 1)
      }
      return undefined
    })
    const seenAtReport: string[][] = []
    reportFirstPaintMock.mockImplementation(() => {
      seenAtReport.push(screen.queryAllByTestId('view-renderer').map(r => r.getAttribute('data-view-id') ?? ''))
    })

    render(<WindowLayout label="slot-popup-10" />)
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('list_tabs', { window: 'slot-popup-10' }))
    const blank = screen.getByTestId('window-loading')
    expect(blank.textContent).toBe('')
    expect(reportFirstPaintMock).not.toHaveBeenCalled()

    await act(async () =>
      answerTabs({
        label: 'slot-popup-10',
        tabs: [{ id: 'v1', name: 'Tab 1', named: true }, { id: 'v2', name: 'Tab 2', named: true }],
        active: 'v1',
        version: 1,
      }),
    )
    await waitFor(() => expect(screen.getAllByTestId('view-renderer')).toHaveLength(1))
    expect(screen.getByTestId('tab-bar')).toBeTruthy()
    expect(screen.queryByTestId('window-loading')).toBeNull()
    expect(reportFirstPaintMock).not.toHaveBeenCalled()

    await act(async () => answerActiveView(slotSnap('v1', 1)))
    await waitFor(() => expect(reportFirstPaintMock).toHaveBeenCalledTimes(1))
    expect(reportFirstPaintMock).toHaveBeenCalledWith(null)
    expect(seenAtReport).toEqual([['v1', 'v2']])
  })

  it('부팅 실패면 실패 문구가 첫 화면이다 — 알림을 보낸다', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_tabs') throw new Error('backend down')
      return undefined
    })
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getByTestId('window-boot-error')).toBeTruthy(), { timeout: 3000 })
    await waitFor(() => expect(reportFirstPaintMock).toHaveBeenCalledTimes(1))
  })

  // TRD S26-window-chrome §7-7 — so the window is widened before the shell shows it.
  it('알림은 제목 줄의 최소 창 크기 보고가 답한 뒤 — 보고가 먼저 나가고 그 답을 기다린다', async () => {
    let answerReport: () => void = () => {}
    const order: string[] = []
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_tabs') {
        return { label: 'main', window_id: 'wid-main', number: 1, tabs: [{ id: 'v1', name: 'Tab 1', named: true }], active: 'v1', version: 1 }
      }
      if (cmd === 'get_view') return slotSnap((args as { viewId: string }).viewId, 1)
      if (cmd === 'report_window_min_width') {
        const { width, height } = args as { width: number; height: number }
        order.push(`report:${width}x${height}`)
        return new Promise<void>(res => (answerReport = res))
      }
      return undefined
    })
    reportFirstPaintMock.mockImplementation(() => void order.push('first-paint'))

    render(<WindowLayout label="main" />)
    await waitFor(() => expect(order).toEqual(['report:286x64']))
    await new Promise(res => setTimeout(res, MIN_SIZE_WAIT_CAP_MS / 3))
    expect(reportFirstPaintMock).not.toHaveBeenCalled()

    await act(async () => answerReport())
    await waitFor(() => expect(order).toEqual(['report:286x64', 'first-paint']))
  })

  it('보고가 답하지 않거나 거절돼도 알림은 상한 뒤에 나간다', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    for (const answer of ['hang', 'reject'] as const) {
      reportFirstPaintMock.mockReset()
      __resetMinSizeReportForTest()
      invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
        if (cmd === 'list_tabs') {
          return { label: 'main', window_id: 'wid-main', number: 1, tabs: [{ id: 'v1', name: 'Tab 1', named: true }], active: 'v1', version: 1 }
        }
        if (cmd === 'get_view') return slotSnap((args as { viewId: string }).viewId, 1)
        if (cmd === 'report_window_min_width') {
          if (answer === 'hang') return new Promise<void>(() => {})
          throw new Error('거절(시험)')
        }
        return undefined
      })
      const view = render(<WindowLayout label="main" />)
      await waitFor(() => expect(reportFirstPaintMock).toHaveBeenCalledTimes(1), { timeout: MIN_SIZE_WAIT_CAP_MS * 3 })
      view.unmount()
    }
    warn.mockRestore()
  })

  // A waiting window stays hidden until it is handed over — the empty frame is not a first screen (TRD S21-slot-docking §3-9 I7).
  it('대기 창은 첫 화면을 알리지 않는다', async () => {
    currentWindow.label = 'slot-popup-7'
    windowRoleMock.mockImplementation(async () => ({ role: 'spare', generation: null }))
    render(<WindowLayout label="slot-popup-7" />)
    await waitFor(() => expect(screen.getByTestId('spare-canvas')).toBeTruthy())
    await new Promise(res => setTimeout(res, 50))
    expect(reportFirstPaintMock).not.toHaveBeenCalled()
  })
})

// ── ★FIX-3: the per-tab get_view keep-alive pull also self-heals with bounded retry★ ──────────────────────────
// The old one-shot get_view only did console.warn on a transient failure and tabIdsKey was unchanged, so there was no re-emit trigger,
//   and that tab's canvas stayed stuck on "View 로딩 중" until an unrelated layout:updated arrived.
describe('WindowLayout — get_view keep-alive 재시도 self-heal(FIX-3)', () => {
  it('get_view 가 처음엔 reject, 재시도로 성공 → 그 탭 캔버스가 회복(재발행 관측)', async () => {
    let v1Calls = 0
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_tabs') {
        return {
          label: (args as { window: string }).window,
          tabs: [{ id: 'v1', name: 'Tab 1', named: true }],
          active: 'v1',
          version: 1,
        }
      }
      if (cmd === 'get_view') {
        const viewId = (args as { viewId: string }).viewId
        if (viewId === 'v1') {
          v1Calls += 1
          if (v1Calls <= 2) throw new Error(`view not ready ${v1Calls}`)
        }
        return slotSnap(viewId, 1)
      }
      return undefined
    })

    render(<WindowLayout label="main" />)
    await waitFor(
      () => {
        const renderers = screen.queryAllByTestId('view-renderer')
        expect(renderers.some(r => r.getAttribute('data-view-id') === 'v1')).toBe(true)
      },
      { timeout: 3000 },
    )
    // If it were one-shot, it would be stuck at 1 call.
    expect(v1Calls).toBeGreaterThanOrEqual(3)
  })
})

describe('WindowLayout — window:tabs-updated 자기 label 필터', () => {
  it('자기 label emit → 활성 탭 스왑(v1→v2)', async () => {
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
    emit('window:tabs-updated', {
      label: 'main',
      tabs: [{ id: 'v1', name: 'Tab 1', named: true }, { id: 'v2', name: 'Tab 2', named: true }],
      active: 'v2',
      version: 2,
    })
    await waitFor(() => {
      const v2 = screen.getAllByTestId('tab-canvas').find(c => c.getAttribute('data-view-id') === 'v2')!
      expect(v2.style.display).toBe('block')
    })
    const v1 = screen.getAllByTestId('tab-canvas').find(c => c.getAttribute('data-view-id') === 'v1')!
    expect(v1.style.display).toBe('none')
  })

  it('다른 label emit → 무시(자기 창 불변)', async () => {
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
    emit('window:tabs-updated', {
      label: 'slot-popup-9',
      tabs: [{ id: 'x1', name: 'X', named: true }],
      active: 'x1',
      version: 99,
    })
    const v1 = screen.getAllByTestId('tab-canvas').find(c => c.getAttribute('data-view-id') === 'v1')!
    expect(v1.style.display).toBe('block')
  })
})

// ── ★S4-F5: keep-alive no-remount — slot components are not remounted on tab switch (terminal instance survives)★ ──
// Unit proxy for ADR-0056 keep-alive "lossless switch": asserts with the mount counter that active/hidden slot renderers are not remounted after a switch
//   and only display is toggled (actual xterm survival belongs to qa cdp stage 6 — here only the structure).
describe('WindowLayout — keep-alive no-remount(ADR-0056, S4-F5)', () => {
  it('탭 전환(v1→v2) 후 두 슬롯 렌더러 mount 횟수가 안 늘고 display 만 토글된다', async () => {
    render(<WindowLayout label="main" />)
    await waitFor(() => {
      expect(mountCounts.get('v1')).toBe(1)
      expect(mountCounts.get('v2')).toBe(1)
    })
    {
      const canvases = screen.getAllByTestId('tab-canvas')
      const v1 = canvases.find(c => c.getAttribute('data-view-id') === 'v1')!
      const v2 = canvases.find(c => c.getAttribute('data-view-id') === 'v2')!
      expect(v1.style.display).toBe('block')
      expect(v2.style.display).toBe('none')
    }

    emit('window:tabs-updated', {
      label: 'main',
      tabs: [{ id: 'v1', name: 'Tab 1', named: true }, { id: 'v2', name: 'Tab 2', named: true }],
      active: 'v2',
      version: 2,
    })
    await waitFor(() => {
      const v2 = screen.getAllByTestId('tab-canvas').find(c => c.getAttribute('data-view-id') === 'v2')!
      expect(v2.style.display).toBe('block')
    })

    expect(mountCounts.get('v1')).toBe(1)
    expect(mountCounts.get('v2')).toBe(1)
    const canvases = screen.getAllByTestId('tab-canvas')
    const v1 = canvases.find(c => c.getAttribute('data-view-id') === 'v1')!
    const v2 = canvases.find(c => c.getAttribute('data-view-id') === 'v2')!
    expect(v1.style.display).toBe('none')
    expect(v2.style.display).toBe('block')
  })
})

describe('WindowLayout — 0탭 자가닫힘(§5-2/G2 · TRD S26-window-chrome §5-5)', () => {
  const CLOSE_IF_EMPTY = ['window_control', { window: 'wid-slot-popup-1', action: 'close', ifEmpty: true }]

  it('0탭 신호(window:tabs-updated{tabs:[]}) → 레지스트리 window.close{if_empty} 를 탭이 있던 때 받은 window_id 로', async () => {
    render(<WindowLayout label="slot-popup-1" />)
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
    // The window_id of a 0-tab payload is not used — only the value received while tabs existed is used.
    emit('window:tabs-updated', { label: 'slot-popup-1', window_id: 'other', tabs: [], active: 'v1', version: 5 })
    await waitFor(() => expect(closeCalls()).toEqual([CLOSE_IF_EMPTY]))
  })

  it('0탭 신호가 두 번 와도 청하기는 한 번만(idempotent 재진입 가드)', async () => {
    render(<WindowLayout label="slot-popup-1" />)
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
    emit('window:tabs-updated', { label: 'slot-popup-1', tabs: [], active: 'v1', version: 5 })
    emit('window:tabs-updated', { label: 'slot-popup-1', tabs: [], active: 'v1', version: 6 })
    await waitFor(() => expect(closeCalls()).toHaveLength(1))
    await Promise.resolve()
    expect(closeCalls()).toHaveLength(1)
  })

  it('청한 뒤 탭이 다시 오면(셸이 닫지 않았다 — Kept) 다음 0탭에 다시 청한다', async () => {
    render(<WindowLayout label="slot-popup-1" />)
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
    emit('window:tabs-updated', { label: 'slot-popup-1', tabs: [], active: 'v1', version: 5 })
    await waitFor(() => expect(closeCalls()).toHaveLength(1))
    act(() =>
      emit('window:tabs-updated', {
        label: 'slot-popup-1',
        window_id: 'wid-slot-popup-1',
        tabs: [{ id: 'v1', name: 'Tab 1', named: true }],
        active: 'v1',
        version: 6,
      }),
    )
    emit('window:tabs-updated', { label: 'slot-popup-1', tabs: [], active: 'v1', version: 7 })
    await waitFor(() => expect(closeCalls()).toEqual([CLOSE_IF_EMPTY, CLOSE_IF_EMPTY]))
  })

  it.each([
    ['window_id 없음', undefined],
    ['0탭 페이로드의 window_id 뿐', 'wid-0tab'],
  ])('탭이 있던 때의 window_id 가 없으면 청하지 않는다(%s)', async (_case, windowId) => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
        if (cmd === 'list_tabs') return { label: 'slot-popup-1', window_id: windowId, tabs: [], active: '', version: 1 }
        if (cmd === 'get_view') return slotSnap((args as { viewId: string }).viewId, 1)
        if (cmd === 'window_control') return KEPT
        return undefined
      })
      render(<WindowLayout label="slot-popup-1" />)
      await waitFor(() => expect(warn).toHaveBeenCalledWith(expect.stringMatching(/window_id 가 없어 청하지 않는다/)))
      expect(closeCalls()).toEqual([])
    } finally {
      warn.mockRestore()
    }
  })
})

// ── ★ADR-0227: window canvas report (TRD §2d)★ ─────────────────────────────────────────────────────────
// jsdom has no ResizeObserver and vitest has no setupFiles, so only this block installs a stub. The test fires the callback
//   directly (there is no layout, so no real size change occurs).
class FakeResizeObserver {
  static instances: FakeResizeObserver[] = []
  observed: Element[] = []
  disconnected = false
  private readonly cb: ResizeObserverCallback
  constructor(cb: ResizeObserverCallback) {
    this.cb = cb
    FakeResizeObserver.instances.push(this)
  }
  observe(el: Element) {
    this.observed.push(el)
  }
  unobserve() {}
  disconnect() {
    this.disconnected = true
    this.observed = []
  }
  fire(width: number, height: number) {
    this.cb(
      [{ contentRect: { width, height } } as unknown as ResizeObserverEntry],
      this as unknown as ResizeObserver,
    )
  }
}

/** Canvas observation — excludes the width observation of the tab bar row (`TabBar`). */
function liveObserver(): FakeResizeObserver {
  const live = FakeResizeObserver.instances.filter(
    o => !o.disconnected && o.observed.length > 0 && !o.observed.some(el => (el as HTMLElement).dataset.testid === 'tab-strip-row'),
  )
  expect(live).toHaveLength(1)
  return live[0]
}

function canvasReports(): Array<{ w: number; h: number }> {
  return invokeMock.mock.calls
    .filter(c => c[0] === 'report_window_canvas')
    .map(c => c[1] as { w: number; h: number })
}

/** Keep the default mocks (list_tabs/get_view) and swap in only report_window_canvas. */
function mockCanvasReport(impl: (size: { w: number; h: number }) => Promise<unknown>): void {
  const base = invokeMock.getMockImplementation()!
  invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd === 'report_window_canvas') return impl(args as { w: number; h: number })
    return base(cmd, args)
  })
}

async function mountReady() {
  const view = render(<WindowLayout label="main" />)
  await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
  return { ro: liveObserver(), unmount: view.unmount }
}

function reportErrors(spy: { mock: { calls: unknown[][] } }): unknown[][] {
  return spy.mock.calls.filter(c => String(c[0]).includes('report_window_canvas'))
}

/** The test finishes report_window_canvas calls by hand, by call order. It also measures the maximum number in flight at once. */
function gateCanvasReport() {
  const pending: Array<{ resolve: () => void; reject: (e: Error) => void }> = []
  let inAir = 0
  let max = 0
  mockCanvasReport(
    () =>
      new Promise<void>((resolve, reject) => {
        inAir += 1
        max = Math.max(max, inAir)
        pending.push({ resolve, reject })
      }),
  )
  return {
    settle(i: number, err?: Error) {
      inAir -= 1
      if (err) pending[i].reject(err)
      else pending[i].resolve()
    },
    maxInAir: () => max,
  }
}

describe('WindowLayout — 창 캔버스 보고(ADR-0227, TRD §2d)', () => {
  beforeEach(() => {
    FakeResizeObserver.instances = []
    vi.stubGlobal('ResizeObserver', FakeResizeObserver)
    // Report state is module scope, so it carries over between tests (the inAir of a hanging invoke, etc.).
    __resetCanvasReportForTest()
  })
  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
  })

  it('win 이 늦게 와도 callback ref 로 탭 내용 영역에 관측이 붙고 첫 측정이 보고된다', async () => {
    let resolveListTabs: (payload: unknown) => void = () => {}
    const listTabsPending = new Promise<unknown>(res => {
      resolveListTabs = res
    })
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_tabs') return listTabsPending
      if (cmd === 'get_view') return slotSnap((args as { viewId: string }).viewId, 1)
      return undefined
    })

    render(<WindowLayout label="main" />)
    await waitFor(() => expect(listeners.has('window:tabs-updated')).toBe(true))
    expect(FakeResizeObserver.instances.filter(o => o.observed.length > 0)).toHaveLength(0)

    resolveListTabs({
      label: 'main',
      tabs: [{ id: 'v1', name: 'Tab 1', named: true }, { id: 'v2', name: 'Tab 2', named: true }],
      active: 'v1',
      version: 1,
    })
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())

    const ro = liveObserver()
    const area = screen.getAllByTestId('tab-canvas')[0].parentElement
    expect(area).not.toBeNull()
    expect(ro.observed).toHaveLength(1)
    expect(ro.observed[0]).toBe(area)

    ro.fire(1024, 768)
    await waitFor(() => expect(canvasReports()).toEqual([{ w: 1024, h: 768 }]))
  })

  it('디바운스 → 정수 반올림 → 바뀔 때만 보낸다 · 같은 크기 재전송 없음 · 0 크기는 안 보낸다', async () => {
    const { ro } = await mountReady()
    vi.useFakeTimers()

    ro.fire(700, 500)
    await vi.advanceTimersByTimeAsync(50)
    ro.fire(800.4, 600.6) // A new value inside the debounce window overwrites the earlier value
    await vi.advanceTimersByTimeAsync(99)
    expect(canvasReports()).toEqual([])
    await vi.advanceTimersByTimeAsync(1)
    expect(canvasReports()).toEqual([{ w: 800, h: 601 }])

    ro.fire(799.6, 600.9) // Rounded, it equals the last successful value
    await vi.advanceTimersByTimeAsync(200)
    ro.fire(0, 0)
    await vi.advanceTimersByTimeAsync(200)
    ro.fire(0.4, 600) // Rounded, the width is 0
    await vi.advanceTimersByTimeAsync(200)
    expect(canvasReports()).toEqual([{ w: 800, h: 601 }])

    // Confirms that the no-send above is not because the path is dead.
    ro.fire(1000, 700)
    await vi.advanceTimersByTimeAsync(100)
    expect(canvasReports()).toEqual([
      { w: 800, h: 601 },
      { w: 1000, h: 700 },
    ])
  })

  it('실패하면 RO 변화 없이도 유계 재시도로 재전송하고, 성공한 값만 직전 값이 된다', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    let calls = 0
    mockCanvasReport(async () => {
      calls += 1
      if (calls <= 2) throw new Error(`refused ${calls}`)
      return undefined
    })
    const { ro } = await mountReady()
    vi.useFakeTimers()

    ro.fire(800, 600)
    await vi.advanceTimersByTimeAsync(100)
    expect(canvasReports()).toHaveLength(1)
    await vi.advanceTimersByTimeAsync(150) // retryAsync first backoff
    expect(canvasReports()).toHaveLength(2)
    await vi.advanceTimersByTimeAsync(300)
    expect(canvasReports()).toEqual([
      { w: 800, h: 600 },
      { w: 800, h: 600 },
      { w: 800, h: 600 },
    ])

    ro.fire(800, 600)
    await vi.advanceTimersByTimeAsync(1000)
    expect(canvasReports()).toHaveLength(3)
  })

  it('재시도가 소진되면 console.error 로 남기고 직전 값을 갱신하지 않는다', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const errSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    let refuse = true
    mockCanvasReport(async () => {
      if (refuse) throw new Error('refused')
      return undefined
    })
    const { ro } = await mountReady()
    vi.useFakeTimers()

    ro.fire(800, 600)
    await vi.advanceTimersByTimeAsync(100 + 150 + 300 + 600 + 10)
    expect(canvasReports()).toHaveLength(4)
    expect(reportErrors(errSpy)).toHaveLength(1)

    refuse = false
    ro.fire(800, 600) // A failed value is not the last value, so even the same size is sent again
    await vi.advanceTimersByTimeAsync(100)
    expect(canvasReports()).toHaveLength(5)
  })

  it('재시도 대기 중 다른 크기가 관측되면 디바운스를 기다리지 않고 옛 재시도를 멈춘다', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const errSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    mockCanvasReport(async ({ w }) => {
      if (w === 800) throw new Error('refused')
      return undefined
    })
    const { ro } = await mountReady()
    vi.useFakeTimers()

    ro.fire(800, 600)
    await vi.advanceTimersByTimeAsync(100) // t=100: 800 first attempt fails → the old retry wakes at t=250
    expect(canvasReports()).toEqual([{ w: 800, h: 600 }])
    await vi.advanceTimersByTimeAsync(100) // t=200
    ro.fire(1000, 700) // The debounce runs until t=300 — the old retry wakes inside this window (t=250)
    await vi.advanceTimersByTimeAsync(99)
    expect(canvasReports()).toEqual([{ w: 800, h: 600 }])
    await vi.advanceTimersByTimeAsync(1)
    expect(canvasReports()).toEqual([
      { w: 800, h: 600 },
      { w: 1000, h: 700 },
    ])

    await vi.advanceTimersByTimeAsync(3000)
    expect(canvasReports()).toHaveLength(2)
    expect(reportErrors(errSpy)).toHaveLength(0) // Cancel is not a failure

    ro.fire(1000, 700)
    await vi.advanceTimersByTimeAsync(200)
    expect(canvasReports()).toHaveLength(2)
  })

  it('0 크기 관측은 보내는 중인 값의 재시도를 멈추지 않는다', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    let calls = 0
    mockCanvasReport(async () => {
      calls += 1
      if (calls === 1) throw new Error('refused')
      return undefined
    })
    const { ro } = await mountReady()
    vi.useFakeTimers()

    ro.fire(800, 600)
    await vi.advanceTimersByTimeAsync(100) // First attempt fails → retry at t=250
    ro.fire(0, 0)
    await vi.advanceTimersByTimeAsync(200)
    expect(canvasReports()).toEqual([
      { w: 800, h: 600 },
      { w: 800, h: 600 },
    ])
  })

  it('날아가는 invoke 가 있으면 새 크기는 그 응답 뒤에 보낸다(창당 한 번에 하나)', async () => {
    const gate = gateCanvasReport()
    const { ro } = await mountReady()
    vi.useFakeTimers()

    ro.fire(800, 600)
    await vi.advanceTimersByTimeAsync(100)
    ro.fire(1000, 700)
    await vi.advanceTimersByTimeAsync(1000) // Nothing is sent before the response, however much time passes
    expect(canvasReports()).toEqual([{ w: 800, h: 600 }])

    gate.settle(0)
    await vi.advanceTimersByTimeAsync(0)
    expect(canvasReports()).toEqual([
      { w: 800, h: 600 },
      { w: 1000, h: 700 },
    ])
    gate.settle(1)
    await vi.advanceTimersByTimeAsync(0)
    expect(gate.maxInAir()).toBe(1)

    ro.fire(1000, 700) // Last successful value = 1000×700
    await vi.advanceTimersByTimeAsync(200)
    expect(canvasReports()).toHaveLength(2)
  })

  it('날아가는 동안 온 크기는 마지막 하나만 응답 뒤에 보내고, 그 응답 값과 같으면 건너뛴다', async () => {
    const gate = gateCanvasReport()
    const { ro } = await mountReady()
    vi.useFakeTimers()

    ro.fire(800, 600)
    await vi.advanceTimersByTimeAsync(100)
    for (const [w, h] of [
      [900, 650],
      [950, 680],
      [1000, 700],
    ]) {
      ro.fire(w, h)
      await vi.advanceTimersByTimeAsync(150) // Each finishes its debounce
    }
    expect(canvasReports()).toHaveLength(1)
    gate.settle(0)
    await vi.advanceTimersByTimeAsync(0)
    expect(canvasReports()).toEqual([
      { w: 800, h: 600 },
      { w: 1000, h: 700 },
    ])

    // While 1000×700 is in flight, if it passes through another value and returns to the same value, that response (success) becomes the last value and it is skipped.
    ro.fire(1100, 750)
    await vi.advanceTimersByTimeAsync(150)
    ro.fire(1000, 700)
    await vi.advanceTimersByTimeAsync(150)
    gate.settle(1)
    await vi.advanceTimersByTimeAsync(1000)
    expect(canvasReports()).toHaveLength(2)
    expect(gate.maxInAir()).toBe(1)
  })

  it('응답을 기다리며 대기하던 값은 더 새 관측이 오면 버려져 응답 뒤에도 나가지 않는다', async () => {
    const gate = gateCanvasReport()
    const { ro } = await mountReady()
    vi.useFakeTimers()

    ro.fire(800, 600)
    await vi.advanceTimersByTimeAsync(100) // 800×600 awaits the response
    ro.fire(900, 650)
    await vi.advanceTimersByTimeAsync(150) // 900×650 finishes its debounce and waits its turn
    ro.fire(1000, 700) // The response arrives before the debounce ends
    await vi.advanceTimersByTimeAsync(50)
    gate.settle(0)
    await vi.advanceTimersByTimeAsync(0)
    expect(canvasReports()).toEqual([{ w: 800, h: 600 }])

    await vi.advanceTimersByTimeAsync(50)
    expect(canvasReports()).toEqual([
      { w: 800, h: 600 },
      { w: 1000, h: 700 },
    ])
  })

  it('훅 인스턴스가 바뀌어도(재마운트) 옛 인스턴스의 invoke 응답 전에는 보내지 않는다', async () => {
    const gate = gateCanvasReport()
    const first = await mountReady()
    vi.useFakeTimers()

    first.ro.fire(800, 600)
    await vi.advanceTimersByTimeAsync(100) // 800×600 awaits the response
    first.unmount()

    render(<WindowLayout label="main" />)
    await vi.advanceTimersByTimeAsync(0)
    const second = liveObserver()
    expect(second).not.toBe(first.ro)
    second.fire(1000, 700)
    await vi.advanceTimersByTimeAsync(1000)
    expect(canvasReports()).toEqual([{ w: 800, h: 600 }])

    gate.settle(0)
    await vi.advanceTimersByTimeAsync(0)
    expect(canvasReports()).toEqual([
      { w: 800, h: 600 },
      { w: 1000, h: 700 },
    ])
    expect(gate.maxInAir()).toBe(1)
  })

  it('다른 값이 날아가는 중 직전 성공값으로 돌아오면 그 응답 결과로 판정한다', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const gate = gateCanvasReport()
    const { ro } = await mountReady()
    vi.useFakeTimers()
    const a = { w: 800, h: 600 }

    ro.fire(800, 600)
    await vi.advanceTimersByTimeAsync(100)
    gate.settle(0) // Last successful value = 800×600
    await vi.advanceTimersByTimeAsync(0)

    // Success: the shell changed to 1000×700, so 800×600 is sent again.
    ro.fire(1000, 700)
    await vi.advanceTimersByTimeAsync(100)
    ro.fire(800, 600)
    await vi.advanceTimersByTimeAsync(150)
    expect(canvasReports()).toEqual([a, { w: 1000, h: 700 }])
    gate.settle(1)
    await vi.advanceTimersByTimeAsync(0)
    expect(canvasReports()).toEqual([a, { w: 1000, h: 700 }, a])
    gate.settle(2)
    await vi.advanceTimersByTimeAsync(0)

    // Failure: the shell stays at 800×600, so it is not sent again, and the cancelled retry does not go out either.
    ro.fire(1100, 750)
    await vi.advanceTimersByTimeAsync(100)
    ro.fire(800, 600)
    await vi.advanceTimersByTimeAsync(150)
    gate.settle(3, new Error('refused'))
    await vi.advanceTimersByTimeAsync(3000)
    expect(canvasReports()).toEqual([a, { w: 1000, h: 700 }, a, { w: 1100, h: 750 }])
    expect(gate.maxInAir()).toBe(1)
  })

  it('탭 전환 재렌더가 관측·디바운스를 끊지 않는다(ref 참조 고정)', async () => {
    const { ro } = await mountReady()
    vi.useFakeTimers()

    ro.fire(800, 600)
    await vi.advanceTimersByTimeAsync(50)
    act(() =>
      emit('window:tabs-updated', {
        label: 'main',
        tabs: [{ id: 'v1', name: 'Tab 1', named: true }, { id: 'v2', name: 'Tab 2', named: true }],
        active: 'v2',
        version: 2,
      }),
    )
    const v2 = screen.getAllByTestId('tab-canvas').find(c => c.getAttribute('data-view-id') === 'v2')!
    expect(v2.style.display).toBe('block')

    await vi.advanceTimersByTimeAsync(50)
    expect(canvasReports()).toEqual([{ w: 800, h: 600 }])
    expect(liveObserver()).toBe(ro)
  })

  it('디바운스 중 언마운트하면 보내지 않고 관측을 뗀다', async () => {
    const { ro, unmount } = await mountReady()
    vi.useFakeTimers()

    ro.fire(800, 600)
    await vi.advanceTimersByTimeAsync(50)
    unmount()
    expect(ro.disconnected).toBe(true)
    await vi.advanceTimersByTimeAsync(500)
    expect(canvasReports()).toEqual([])
  })

  it('재시도 중 언마운트하면 재시도가 멈추고 최종 실패 로그도 없다', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const errSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    mockCanvasReport(async () => {
      throw new Error('refused')
    })
    const { ro, unmount } = await mountReady()
    vi.useFakeTimers()

    ro.fire(800, 600)
    await vi.advanceTimersByTimeAsync(100)
    expect(canvasReports()).toHaveLength(1)
    unmount()
    await vi.advanceTimersByTimeAsync(3000)
    expect(canvasReports()).toHaveLength(1)
    expect(reportErrors(errSpy)).toHaveLength(0)
  })

  it('응답을 기다리는 중 언마운트하면 응답이 와도 대기 중이던 크기를 보내지 않는다', async () => {
    const gate = gateCanvasReport()
    const { ro, unmount } = await mountReady()
    vi.useFakeTimers()

    ro.fire(800, 600)
    await vi.advanceTimersByTimeAsync(100)
    ro.fire(1000, 700)
    await vi.advanceTimersByTimeAsync(150) // 1000×700 awaits the response
    unmount()
    gate.settle(0)
    await vi.advanceTimersByTimeAsync(1000)
    expect(canvasReports()).toEqual([{ w: 800, h: 600 }])
  })

  it('ResizeObserver 가 없는 환경에서도 던지지 않고 보고 없이 렌더한다', async () => {
    vi.stubGlobal('ResizeObserver', undefined)
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
    expect(screen.getAllByTestId('tab-canvas')).toHaveLength(2)
    await new Promise(res => setTimeout(res, 150))
    expect(canvasReports()).toEqual([])
  })
})

// TRD S26-window-chrome §7-1 · §7-5 — a title row where the tab bar used to be placed (layout · buttons · dimming details = `titleRow/TitleRow.test.tsx`).
describe('WindowLayout — 제목 줄', () => {
  const FRAME = {
    restored: { w: 800, h: 600 },
    maximized: false,
    insets: { t: 0, r: 0, b: 0, l: 0 },
    focused: true,
    caption: { buttons: true, leading_inset: 0, snap_overlay: false },
  }

  function tabsWithNumber(label: string, number: number) {
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_tabs') {
        return { label, window_id: `wid-${label}`, number, tabs: [{ id: 'v1', name: 'Tab 1', named: true }], active: 'v1', version: 1 }
      }
      if (cmd === 'get_view') return slotSnap((args as { viewId: string }).viewId, 1)
      if (cmd === 'window_frame') return FRAME
      if (cmd === 'window_control') return KEPT
      return undefined
    })
  }

  it('main — 제목 줄이 탭바를 품고 · 번호 표식 = 탭 알림의 번호 · 셋째 단추 = 모든 창 숨기기', async () => {
    tabsWithNumber('main', 3)
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getByTestId('window-number').textContent).toBe('3'))
    const row = screen.getByTestId('title-row')
    expect(row.firstElementChild).toBe(screen.getByTestId('tab-bar'))
    expect(screen.getByTestId('caption-close').getAttribute('aria-label')).toBe('모든 창 숨기기')
  })

  it('팝아웃 — 셋째 단추 = 창 닫기 · 누르면 이 창 label 로 window_control close', async () => {
    currentWindow.label = 'slot-popup-4'
    tabsWithNumber('slot-popup-4', 2)
    render(<WindowLayout label="slot-popup-4" />)
    await waitFor(() => expect(screen.getByTestId('window-number').textContent).toBe('2'))
    const close = screen.getByTestId('caption-close')
    expect(close.getAttribute('aria-label')).toBe('창 닫기')
    fireEvent.click(close)
    expect(closeCalls()).toEqual([['window_control', { window: 'slot-popup-4', action: 'close', ifEmpty: false }]])
  })

  it('창 사실(window:frame)의 focused · maximized 를 제목 줄이 따라간다', async () => {
    tabsWithNumber('main', 1)
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(listeners.has('window:frame')).toBe(true))
    act(() => emit('window:frame', { ...FRAME, focused: false, maximized: true }))
    const row = screen.getByTestId('title-row')
    expect([row.dataset.windowActive, row.dataset.maximized]).toEqual(['false', 'true'])
    expect(screen.getByTestId('caption-maximize').getAttribute('aria-label')).toBe('이전 크기로 복원')
    act(() => emit('window:frame', FRAME))
    expect([row.dataset.windowActive, row.dataset.maximized]).toEqual(['true', 'false'])
  })
})

// User decision 2026-10-26 (T-66): the notice band does not cover the title row (window buttons · draggable area) — it attaches to the top of the canvas area under the title row.
//   jsdom cannot measure sizes — the position is asserted structurally (the position · z of the covering layer itself belongs to NoticeOverlay.test).
describe('WindowLayout — 알림 층의 자리', () => {
  const notices = <div data-testid="notices-probe" />

  it('알림 층은 제목 줄 밖 · 캔버스 칸(positioned) 맨 앞에 붙고, 제목 줄이 그보다 앞이다', async () => {
    render(<WindowLayout label="main" notices={notices} />)
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
    const probe = screen.getByTestId('notices-probe')
    const row = screen.getByTestId('title-row')
    expect(row.contains(probe)).toBe(false)
    const canvas = probe.parentElement as HTMLElement
    expect(canvas.style.position).toBe('relative')
    expect(canvas.firstElementChild).toBe(probe)
    expect(canvas.contains(screen.getAllByTestId('tab-canvas')[0])).toBe(true)
    expect(row.compareDocumentPosition(probe) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  })

  // The loading · boot failure screens share the same frame — title row → area (first = notice layer) → screen content.
  function expectFrame(content: string): void {
    const probe = screen.getByTestId('notices-probe')
    const row = screen.getByTestId('title-row')
    const box = probe.parentElement as HTMLElement
    expect(box.style.position).toBe('relative')
    expect([...box.children].map(el => (el as HTMLElement).dataset.testid)).toEqual(['notices-probe', content])
    expect(row.nextElementSibling).toBe(box)
    expect(row.contains(probe)).toBe(false)
  }

  it('로딩 화면에도 제목 줄(끌 자리 · 창 단추 — 표식은 아직 없다) 아래에 알림 층을 그린다', async () => {
    invokeMock.mockImplementation(() => new Promise(() => {}))
    render(<WindowLayout label="main" notices={notices} />)
    expect(screen.getByTestId('window-loading').textContent).toBe('')
    expectFrame('window-loading')
    await waitFor(() => expect(screen.getByTestId('caption-buttons')).toBeTruthy())
    expect(screen.getByTestId('tab-bar-standin').hasAttribute('data-tauri-drag-region')).toBe(true)
    expect(screen.queryByTestId('window-number')).toBeNull()
    expect(screen.queryByTestId('tab-bar')).toBeNull()
  })

  // That window may not be in the model (a stale popout) — the window buttons are sent by that label without window_id, and the shell attaches
  //   directly to the open popout window of that label (a window outside the model — TRD S26-window-chrome §6-2 · shell test `window_buttons_of_an_unmodeled_window_reach_its_os_window`).
  it('부팅 실패 화면에도 제목 줄 아래에 알림 층을 그린다 — 창 단추가 그 label 로 가고 연결 실패 이유도 읽힌다', async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_tabs') throw new Error('backend down')
      if (cmd === 'window_control') return (args as { action: string }).action === 'close' ? CLOSED : { ...KEPT, close: null }
      return undefined
    })
    render(<WindowLayout label="slot-popup-6" notices={notices} />)
    await waitFor(() => expect(screen.getByTestId('window-boot-error')).toBeTruthy(), { timeout: 3000 })
    expectFrame('window-boot-error')
    fireEvent.click(screen.getByTestId('caption-minimize'))
    fireEvent.click(screen.getByTestId('caption-maximize'))
    fireEvent.click(screen.getByTestId('caption-close'))
    await waitFor(() =>
      expect(closeCalls()).toEqual([
        ['window_control', { window: 'slot-popup-6', action: 'minimize', ifEmpty: null }],
        ['window_control', { window: 'slot-popup-6', action: 'maximize', ifEmpty: null }],
        ['window_control', { window: 'slot-popup-6', action: 'close', ifEmpty: false }],
      ]),
    )
  })

  it('번호를 받기 전(로딩 · 부팅 실패)에는 최소 창 크기를 보내지 않는다', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_tabs') throw new Error('backend down')
      return undefined
    })
    render(<WindowLayout label="main" notices={notices} />)
    await waitFor(() => expect(screen.getByTestId('window-boot-error')).toBeTruthy(), { timeout: 3000 })
    expect(invokeMock.mock.calls.filter(c => c[0] === 'report_window_min_width')).toEqual([])
  })
})

// ADR-0277: the memory of closing with ✕ (the state of `ConnectionNotice` · `StateFileNotice`) is not withdrawn — the notice layer must not be
//   remounted even when the screen changes, so that the memory remains.
describe('WindowLayout — 화면이 바뀌어도 알림의 닫힘 기억이 남는다', () => {
  let mounts = 0
  function DismissableProbe() {
    const [open, setOpen] = useState(true)
    useEffect(() => {
      mounts += 1
    }, [])
    return open ? (
      <button type="button" data-testid="notice-dismiss" onClick={() => setOpen(false)} />
    ) : (
      <span data-testid="notice-dismissed" />
    )
  }
  beforeEach(() => {
    mounts = 0
  })

  it('로딩 중에 닫고 탭이 오면 닫힌 채다', async () => {
    let answerTabs: (payload: unknown) => void = () => {}
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_tabs') return new Promise(res => (answerTabs = res))
      if (cmd === 'get_view') return slotSnap((args as { viewId: string }).viewId, 1)
      return undefined
    })
    render(<WindowLayout label="main" notices={<DismissableProbe />} />)
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('list_tabs', { window: 'main' }))
    expect(screen.getByTestId('window-loading')).toBeTruthy()
    fireEvent.click(screen.getByTestId('notice-dismiss'))

    await act(async () =>
      answerTabs({ label: 'main', window_id: 'wid-main', number: 1, tabs: [{ id: 'v1', name: 'Tab 1', named: true }], active: 'v1', version: 1 }),
    )
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
    expect(screen.getByTestId('notice-dismissed')).toBeTruthy()
    expect(mounts).toBe(1)
  })

  it('부팅 실패 화면에서 닫고 탭 알림으로 되살아나도 닫힌 채다', async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_tabs') throw new Error('backend down')
      if (cmd === 'get_view') return slotSnap((args as { viewId: string }).viewId, 1)
      return undefined
    })
    render(<WindowLayout label="main" notices={<DismissableProbe />} />)
    await waitFor(() => expect(screen.getByTestId('window-boot-error')).toBeTruthy(), { timeout: 3000 })
    fireEvent.click(screen.getByTestId('notice-dismiss'))

    act(() =>
      emit('window:tabs-updated', {
        label: 'main',
        window_id: 'wid-main',
        number: 1,
        tabs: [{ id: 'v1', name: 'Tab 1', named: true }],
        active: 'v1',
        version: 2,
      }),
    )
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
    expect(screen.queryByTestId('window-boot-error')).toBeNull()
    expect(screen.getByTestId('notice-dismissed')).toBeTruthy()
    expect(mounts).toBe(1)
  })
})

describe('WindowLayout — TabBar 액션 → store 액션(이 label)', () => {
  it('[+] 클릭 → createTab(label) invoke', async () => {
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getByTestId('tab-add')).toBeTruthy())
    fireEvent.click(screen.getByTestId('tab-add'))
    expect(invokeMock).toHaveBeenCalledWith('create_tab', { window: 'main', name: null })
  })

  it('숨은 탭 누름 → switchTab(label, view) invoke', async () => {
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
    const tab2 = screen.getAllByTestId('tab').find(t => t.getAttribute('data-view-id') === 'v2')!
    fireEvent.pointerDown(tab2, { button: 0, pointerId: 1 })
    expect(invokeMock).toHaveBeenCalledWith('switch_tab', { window: 'main', view: 'v2' })
  })

  it('탭 × 클릭 → closeTab(label, view) invoke', async () => {
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
    const closeBtns = screen.getAllByTestId('tab-close')
    fireEvent.click(closeBtns[0]) // v1 close
    expect(invokeMock).toHaveBeenCalledWith('close_tab', { window: 'main', view: 'v1' })
  })
})

// ── ★Tab drag wiring (TRD S21-slot-docking §4)★ — root receiving and the guide layer (`useWindowDrop`) · blocking file drops ──
// The rectangles of the tab bar row · `+` · canvas are given by hand (jsdom has no layout): row x 0..800 · y 0..28 · `+` 32 · canvas origin (0, 29).
const TWO_TABS = [
  { id: 'v1', name: 'Tab 1', named: true },
  { id: 'v2', name: 'Tab 2', named: true },
]

async function mountDockWindow(label: string, tabs = TWO_TABS, active = 'v1') {
  invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd === 'list_tabs') return { label, tabs, active, version: 1 }
    if (cmd === 'get_view') return slotSnap((args as { viewId: string }).viewId, 1)
    return undefined
  })
  const view = render(<WindowLayout label={label} />)
  await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
  await waitFor(() => expect(ipcOrder).toContain('invoke:window_role'))
  return view
}

const headOf = (id: string) => screen.getAllByTestId('tab').find(t => t.getAttribute('data-view-id') === id)!
const canvasOf = (id: string) =>
  screen.getAllByTestId('tab-canvas').find(c => c.getAttribute('data-view-id') === id)!

let stripWidth = 800

function stubStrip(): void {
  const rect = (left: number, top: number, width: number, height: number) =>
    ({ left, top, width, height, right: left + width, bottom: top + height, x: left, y: top }) as DOMRect
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    const id = this.dataset.testid
    if (id === 'tab-strip-row') return rect(0, 0, stripWidth, 28)
    if (id === 'tab-add') return rect(0, 0, 32, 28)
    if (this.querySelector(':scope > [data-testid="tab-canvas"], :scope > [data-testid="spare-canvas"]')) {
      return rect(0, 29, stripWidth, 571)
    }
    return rect(0, 0, 0, 0)
  })
}

function dragOver(types: string[]): Event {
  const e = new Event('dragover', { bubbles: true, cancelable: true })
  Object.defineProperty(e, 'dataTransfer', { value: { types, dropEffect: 'copy' } })
  act(() => {
    screen.getByTestId('tab-bar').dispatchEvent(e)
  })
  return e
}

describe('WindowLayout — 탭 끌기 배선', () => {
  beforeEach(() => {
    stripWidth = 800
    vi.stubGlobal('matchMedia', vi.fn(() => ({ matches: false }) as MediaQueryList))
  })
  afterEach(() => {
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
  })

  it('셸 끌기 세션 사건(dock:drag · dock:feed)은 듣지 않는다 — spare:adopted 만 자기 창 대상으로 건 뒤 window_role 을 묻는다', async () => {
    currentWindow.label = 'slot-popup-3'
    await mountDockWindow('slot-popup-3')
    expect(listenOptions.get('spare:adopted')).toEqual({ target: 'slot-popup-3' })
    expect(ipcOrder.indexOf('listen:spare:adopted')).toBeLessThan(ipcOrder.indexOf('invoke:window_role'))
    expect(ipcOrder).not.toContain('listen:dock:drag')
    expect(ipcOrder).not.toContain('listen:dock:feed')
  })

  it('이 창에서 끄는 동안 탭바 구역 밖이면 남은 탭 캔버스를 보이고(R15 · 모델 그대로) 돌아오면 끄는 탭 · 크롬 = 뷰포트 − 탭 캔버스', async () => {
    stubStrip()
    __resetDragSourceForTest()
    Object.defineProperty(window.screen, 'availWidth', { value: 1920, configurable: true })
    Object.defineProperty(window.screen, 'availHeight', { value: 1080, configurable: true })
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_tabs') return { label: 'main', tabs: TWO_TABS, active: 'v1', version: 1 }
      if (cmd === 'get_view') return slotSnap((args as { viewId: string }).viewId, 1)
      // Restore size equal to the viewport (jsdom 1024 × 768) — new window size = as is.
      if (cmd === 'window_frame')
        return {
          restored: { w: 1024, h: 768 },
          maximized: false,
          insets: { t: 0, r: 0, b: 0, l: 0 },
          focused: true,
          caption: { buttons: true, leading_inset: 0, snap_overlay: false },
        }
      return undefined
    })
    try {
      render(<WindowLayout label="main" />)
      await waitFor(() => expect(screen.getAllByTestId('tab-canvas')).toHaveLength(2))
      await act(async () => {
        for (let i = 0; i < 20; i++) await Promise.resolve()
      })
      // Press → switch echo → dragstart (the drag continues in another task).
      fireEvent.pointerDown(headOf('v2'), { button: 0, clientX: 600, clientY: 14 })
      act(() => emit('window:tabs-updated', { label: 'main', tabs: TWO_TABS, active: 'v2', version: 2 }))
      const data = new Map<string, string>()
      const ev = createEvent.dragStart(headOf('v2'), {
        dataTransfer: {
          effectAllowed: 'uninitialized',
          setData: (k: string, v: string) => void data.set(k, v),
          clearData: () => data.clear(),
          setDragImage: () => {},
        },
      })
      Object.defineProperty(ev, 'clientX', { value: 604 })
      Object.defineProperty(ev, 'clientY', { value: 14 })
      act(() => void fireEvent(headOf('v2'), ev))
      // Original size = new window 1024 × 768 − chrome (1024 − 800 · 768 − 571) = tab canvas · shape = that tab's cached layout (one area).
      expect([...data.keys()][0].replace(/;drag=[^;]*/, '')).toBe('application/x-engram-tab;app=com.engram.test;w=800;h=571;shape=l')
      expect(canvasOf('v2').style.display).toBe('block')

      act(() => localDrag.setSourceStrip(null))
      expect(canvasOf('v1').style.display).toBe('block')
      expect(canvasOf('v2').style.display).toBe('none')
      expect(headOf('v1').getAttribute('data-active')).toBe('true')
      expect(useViewStore.getState().windows.main.active).toBe('v2')

      act(() => localDrag.setSourceStrip({ index: 1, left: 384, x: 600 }))
      expect(canvasOf('v2').style.display).toBe('block')
      expect(canvasOf('v1').style.display).toBe('none')
    } finally {
      act(() => {
        localDrag.setOutgoing(null)
        localDrag.setSourceStrip(null)
      })
    }
  })

  // Canvas box (0, 29) 800 × 571 · visible tab v1 = one area (ratio limits 0.1 · 0.9 · version number 1) · dragged tab original size 400 × 300.
  const DRAG_TYPE = buildTabDragType({ app: 'com.engram.test', drag: 'd9', original: { w: 400, h: 300 }, shape: 'l' })
  const DRAG_VALUE = buildTabDragValue({
    drag: 'd9',
    view: 'p1',
    from: 'slot-popup-2',
    initial: null,
    size: { w: 640, h: 480 },
    grab: { x: 40, y: 14 },
  })

  function fireDrag(type: string, at: { x: number; y: number }, types = [DRAG_TYPE]): MouseEvent {
    const e = new MouseEvent(type, { bubbles: true, cancelable: true, clientX: at.x, clientY: at.y })
    Object.defineProperty(e, 'dataTransfer', {
      value: { types, dropEffect: 'none', getData: (t: string) => (t === DRAG_TYPE ? DRAG_VALUE : '') },
    })
    act(() => {
      canvasOf('v1').dispatchEvent(e)
    })
    return e
  }

  async function mountDropWindow() {
    stubStrip()
    await mountDockWindow('main')
    // Finish the identifier answer · cache pull.
    await act(async () => {
      for (let i = 0; i < 20; i++) await Promise.resolve()
    })
  }

  it('끌기 안내 층은 탭 캔버스 상자 바로 안에 그 상자를 덮고 · 늘 마운트돼 있다(안내는 없음)', async () => {
    await mountDropWindow()
    const layer = screen.getByTestId('dock-overlay-layer')
    expect(layer.parentElement).toBe(canvasOf('v1').parentElement)
    expect(layer.style.position).toBe('absolute')
    expect(layer.style.pointerEvents).toBe('none')
    expect(screen.queryByTestId('dock-overlay')).toBeNull()
  })

  it('우리 끌기를 뿌리에서 받아 캔버스 좌표로 미리보기를 그리고 · 끄는 동안 셸을 부르지 않고 · drop 에 Seam 하나를 보낸다', async () => {
    await mountDropWindow()
    const seen: string[] = []
    const rdnd = (e: Event) => void seen.push(e.type)
    for (const t of ['dragenter', 'dragover', 'drop']) window.addEventListener(t, rdnd)
    const calls = invokeMock.mock.calls.length
    try {
      // Canvas (5, 271) = outer left band → rectangle to attach = left half (share = original width 400 ÷ 800).
      const enter = fireDrag('dragenter', { x: 5, y: 300 })
      fireDrag('dragover', { x: 6, y: 300 })
      expect(enter.defaultPrevented).toBe(true)
      expect((enter as unknown as { dataTransfer: DataTransfer }).dataTransfer.dropEffect).toBe('move')
      const preview = screen.getByTestId('dock-preview')
      expect(screen.getByTestId('dock-overlay-layer').contains(preview)).toBe(true)
      expect([preview.style.left, preview.style.top, preview.style.width, preview.style.height]).toEqual([
        '0px',
        '0px',
        '400px',
        '571px',
      ])
      expect(invokeMock.mock.calls.length).toBe(calls)

      const drop = fireDrag('drop', { x: 6, y: 300 })
      expect(drop.defaultPrevented).toBe(true)
      expect(invokeMock.mock.calls.slice(calls)).toEqual([
        [
          'tab_drop',
          {
            drop: {
              drag: 'd9',
              view: 'p1',
              from: 'slot-popup-2',
              initial: null,
              to: { type: 'seam', target: 'v1', anchor: { type: 'outer' }, side: 'Left', share: 0.5 },
            },
          },
        ],
      ])
      expect(seen).toEqual([])
    } finally {
      for (const t of ['dragenter', 'dragover', 'drop']) window.removeEventListener(t, rdnd)
    }
  })

  it('우리 것이 아닌 끌기(트리 행 · 파일)는 받지도 끊지도 않는다 — window 의 react-dnd 꼴 리스너까지 간다', async () => {
    await mountDropWindow()
    const seen: string[] = []
    const rdnd = (e: Event) => void seen.push(e.type)
    window.addEventListener('dragover', rdnd)
    try {
      const row = fireDrag('dragover', { x: 5, y: 300 }, ['application/x-tree-row'])
      expect(row.defaultPrevented).toBe(false)
      fireDrag('dragover', { x: 5, y: 300 }, ['Files'])
      expect(seen).toEqual(['dragover', 'dragover'])
      expect(screen.queryByTestId('dock-overlay')).toBeNull()
    } finally {
      window.removeEventListener('dragover', rdnd)
    }
  })

  it('파일 · URL 끌어 놓기를 막는다(앱 안 링크 포함) · 글자 끌기는 그대로 · 내리면 뗀다', async () => {
    const { unmount } = await mountDockWindow('main')
    const link = dragOver(['text/uri-list'])
    expect(link.defaultPrevented).toBe(true)
    expect((link as unknown as { dataTransfer: DataTransfer }).dataTransfer.dropEffect).toBe('none')
    expect(dragOver(['Files']).defaultPrevented).toBe(true)
    expect(dragOver(['text/plain']).defaultPrevented).toBe(false)

    unmount()
    const e = new Event('dragover', { bubbles: true, cancelable: true })
    Object.defineProperty(e, 'dataTransfer', { value: { types: ['Files'], dropEffect: 'copy' } })
    document.body.dispatchEvent(e)
    expect(e.defaultPrevented).toBe(false)
  })
})

// ── ★Waiting mode (TRD S21-slot-docking §3-9)★ — a window of the spare window pool (hidden spare window) is an empty frame · 0-tab self-close off · no tab pull-in
//   · measuring reports as usual. Mode = `window_role` at mount time, then `spare:adopted` (to normal).
describe('WindowLayout — 대기 모드(window_role · spare:adopted)', () => {
  const SPARE = 'slot-popup-7'

  beforeEach(() => {
    currentWindow.label = SPARE
    __resetUiMetricsReportForTest()
  })

  const spareRole = () => windowRoleMock.mockImplementation(async () => ({ role: 'spare', generation: null }))
  const listTabsCalls = () => invokeMock.mock.calls.filter(c => c[0] === 'list_tabs')

  async function mountSpare() {
    spareRole()
    const view = render(<WindowLayout label={SPARE} />)
    await waitFor(() => expect(screen.getByTestId('spare-canvas')).toBeTruthy())
    return view
  }

  it('spare:adopted 듣기를 자기 창 대상으로 먼저 건 뒤 window_role 을 묻는다', async () => {
    render(<WindowLayout label={SPARE} />)
    await waitFor(() => expect(ipcOrder).toContain('invoke:window_role'))
    expect(ipcOrder.indexOf('listen:spare:adopted')).toBeGreaterThanOrEqual(0)
    expect(ipcOrder.indexOf('listen:spare:adopted')).toBeLessThan(ipcOrder.indexOf('invoke:window_role'))
    expect(listenOptions.get('spare:adopted')).toEqual({ target: SPARE })
  })

  it('window_role = spare → 빈 틀(탭바 + 빈 캔버스) · list_tabs 를 당기지 않는다 · 부팅 오류 · 로딩 문구 없음', async () => {
    await mountSpare()
    expect(screen.getByTestId('tab-bar')).toBeTruthy()
    // The title row is drawn (so the tab bar is not remounted when taken in) but there are no number badge · window buttons — it is outside the shell window model.
    expect(screen.getByTestId('title-row').firstElementChild).toBe(screen.getByTestId('tab-bar'))
    expect(screen.queryByTestId('window-number')).toBeNull()
    expect(screen.queryByTestId('caption-buttons')).toBeNull()
    expect(screen.queryAllByTestId('tab')).toHaveLength(0)
    expect(screen.queryAllByTestId('tab-canvas')).toHaveLength(0)
    await new Promise(res => setTimeout(res, 50))
    expect(listTabsCalls()).toEqual([])
    expect(listeners.has('window:tabs-updated')).toBe(false)
    expect(screen.queryByTestId('window-boot-error')).toBeNull()
  })

  it('대기 모드에선 이 창의 0탭 알림이 와도 닫지 않는다', async () => {
    await mountSpare()
    act(() =>
      useViewStore.getState().applyWindowTabsUpdated({ label: SPARE, window_id: 'w-spare', number: 2, title: 'Engram:2', tabs: [], active: '', version: 9 }),
    )
    await new Promise(res => setTimeout(res, 20))
    expect(closeCalls()).toEqual([])
    expect(screen.getByTestId('spare-canvas')).toBeTruthy()
  })

  it('대기 모드에서도 재기 둘(캔버스 · 칸 틀 지표)을 보낸다', async () => {
    FakeResizeObserver.instances = []
    vi.stubGlobal('ResizeObserver', FakeResizeObserver)
    __resetCanvasReportForTest()
    try {
      await mountSpare()
      act(() => liveObserver().fire(640, 400))
      await waitFor(() => expect(canvasReports()).toEqual([{ w: 640, h: 400 }]))
      await waitFor(() =>
        expect(invokeMock.mock.calls.filter(c => c[0] === 'report_ui_metrics')).toEqual([
          ['report_ui_metrics', { metrics: expect.objectContaining({ frame_insets: { t: 1, r: 1, b: 1, l: 1 } }) }],
        ]),
      )
    } finally {
      vi.unstubAllGlobals()
    }
  })

  it('spare:adopted → 보통 모드: list_tabs 로 들인 탭을 당겨 그리고, 0탭 자가닫힘이 다시 산다', async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_tabs')
        return { label: SPARE, window_id: 'w-spare', tabs: [{ id: 'p1', name: 'Pop', named: true }], active: 'p1', version: 3 }
      if (cmd === 'get_view') return slotSnap((args as { viewId: string }).viewId, 1)
      if (cmd === 'window_control') return KEPT
      return undefined
    })
    await mountSpare()
    expect(listTabsCalls()).toEqual([])

    act(() => emit('spare:adopted', { generation: 1 }))

    await waitFor(() => expect(screen.getAllByTestId('tab-canvas')).toHaveLength(1))
    expect(listTabsCalls()).toEqual([['list_tabs', { window: SPARE }]])
    expect(screen.queryByTestId('spare-canvas')).toBeNull()

    emit('window:tabs-updated', { label: SPARE, tabs: [], active: 'p1', version: 4 })
    await waitFor(() =>
      expect(closeCalls()).toEqual([['window_control', { window: 'w-spare', action: 'close', ifEmpty: true }]]),
    )
  })

  it('대기 창은 창 사실(window_frame)을 당기지 않는다 — 셸 창 모델 밖이다 · 들이면 당긴다', async () => {
    const frameCalls = () => invokeMock.mock.calls.filter(c => c[0] === 'window_frame')
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_tabs') return { label: SPARE, tabs: [{ id: 'p1', name: 'Pop', named: true }], active: 'p1', version: 3 }
      if (cmd === 'get_view') return slotSnap((args as { viewId: string }).viewId, 1)
      if (cmd === 'window_frame')
        return {
          restored: { w: 800, h: 600 },
          maximized: false,
          insets: { t: 0, r: 0, b: 0, l: 0 },
          focused: true,
          caption: { buttons: true, leading_inset: 0, snap_overlay: false },
        }
      return undefined
    })
    await mountSpare()
    await new Promise(res => setTimeout(res, 50))
    expect(frameCalls()).toEqual([])

    act(() => emit('spare:adopted', { generation: 1 }))
    await waitFor(() => expect(screen.getAllByTestId('tab-canvas')).toHaveLength(1))
    await waitFor(() => expect(frameCalls()).toEqual([['window_frame', undefined]]))
  })

  it('들인 뒤 제 탭을 당겨 오기 전에는 빈 틀 그대로 — 로딩 문구 없음 · 탭바 줄은 같은 노드', async () => {
    let answer: (payload: unknown) => void = () => {}
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_tabs') return new Promise(res => (answer = res))
      if (cmd === 'get_view') return slotSnap((args as { viewId: string }).viewId, 1)
      return undefined
    })
    await mountSpare()
    const row = screen.getByTestId('tab-strip-row')

    act(() => emit('spare:adopted', { generation: 1 }))
    await waitFor(() => expect(listTabsCalls()).toHaveLength(1))
    expect(screen.getByTestId('spare-canvas')).toBeTruthy()
    expect(screen.queryByTestId('window-loading')).toBeNull()
    expect(screen.getByTestId('tab-strip-row')).toBe(row)

    await act(async () => answer({ label: SPARE, tabs: [{ id: 'p1', name: 'Pop', named: true }], active: 'p1', version: 3 }))
    await waitFor(() => expect(screen.getAllByTestId('tab-canvas')).toHaveLength(1))
    expect(screen.queryByTestId('spare-canvas')).toBeNull()
    expect(screen.getByTestId('tab-strip-row')).toBe(row)
  })

  const headIds = () => screen.queryAllByTestId('tab').map(h => h.getAttribute('data-view-id'))

  it('대기 중에 들일 탭이 먼저 오면(전역 미러) 대기가 풀릴 때까지 빈 틀 · 풀리는 순간 그 탭', async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_tabs') return new Promise(() => {})
      if (cmd === 'get_view') return slotSnap((args as { viewId: string }).viewId, 1)
      return undefined
    })
    await mountSpare()

    act(() =>
      useViewStore.getState().applyWindowTabsUpdated({ label: SPARE, window_id: 'w-spare', number: 2, title: 'Pop - Engram:2', tabs: [{ id: 'p1', name: 'Pop', named: true }], active: 'p1', version: 7 }),
    )
    expect(screen.getByTestId('spare-canvas')).toBeTruthy()
    expect(headIds()).toEqual([])

    act(() => emit('spare:adopted', { generation: 1 }))
    expect(headIds()).toEqual(['p1'])
    expect(screen.queryByTestId('spare-canvas')).toBeNull()
  })

  it('window_role = unknown 이거나 실패하면 보통으로 그린다', async () => {
    windowRoleMock.mockImplementation(async () => ({ role: 'unknown', generation: null }))
    const first = render(<WindowLayout label={SPARE} />)
    await waitFor(() => expect(screen.getAllByTestId('tab-canvas')).toHaveLength(2))
    first.unmount()

    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      windowRoleMock.mockImplementation(async () => {
        throw new Error('no dock state')
      })
      render(<WindowLayout label={SPARE} />)
      await waitFor(() => expect(screen.getAllByTestId('tab-canvas')).toHaveLength(2))
      expect(screen.queryByTestId('spare-canvas')).toBeNull()
    } finally {
      warn.mockRestore()
    }
  })

  it('모드를 모르는 동안(window_role 답 전)에는 0탭 알림이 와도 닫지 않는다 — 대기로 답하면 끝까지 안 닫는다', async () => {
    let answer: (reply: unknown) => void = () => {}
    windowRoleMock.mockImplementation(() => new Promise(res => (answer = res)))
    render(<WindowLayout label={SPARE} />)
    await waitFor(() => expect(ipcOrder).toContain('invoke:window_role'))
    act(() =>
      useViewStore.getState().applyWindowTabsUpdated({ label: SPARE, window_id: 'w-spare', number: 2, title: 'Engram:2', tabs: [], active: '', version: 9 }),
    )
    await new Promise(res => setTimeout(res, 20))
    expect(closeCalls()).toEqual([])

    await act(async () => answer({ role: 'spare', generation: null }))
    expect(screen.getByTestId('spare-canvas')).toBeTruthy()
    expect(closeCalls()).toEqual([])
  })
})

// ── Prewarmed window adoption · first screen notice (TRD S21-slot-docking §3-9 I26 · I28) — the shell door of the adopted window receives only the notice carrying that adoption number ──
describe('WindowLayout — 채택 번호 · 첫 화면 다시 알림', () => {
  const SPARE = 'slot-popup-7'
  const TABS = { label: SPARE, window_id: 'w-spare', number: 2, title: 'Pop - Engram:2', tabs: [{ id: 'p1', name: 'Pop', named: true }], active: 'p1', version: 3 }

  beforeEach(() => {
    currentWindow.label = SPARE
  })

  const listTabsCalls = () => invokeMock.mock.calls.filter(c => c[0] === 'list_tabs')
  const reports = () => reportFirstPaintMock.mock.calls
  const roleArgs = () => windowRoleMock.mock.calls.map(c => c[0])

  function listTabsAnswers(ok: () => boolean) {
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_tabs') {
        if (!ok()) throw new Error('unknown window')
        return TABS
      }
      if (cmd === 'get_view') return slotSnap((args as { viewId: string }).viewId, 1)
      return undefined
    })
  }

  it('대기 → 채택: 제 탭을 그린 뒤 그 채택 번호로 한 번 알린다', async () => {
    listTabsAnswers(() => true)
    windowRoleMock.mockImplementation(async () => ({ role: 'spare', generation: null }))
    render(<WindowLayout label={SPARE} />)
    await waitFor(() => expect(screen.getByTestId('spare-canvas')).toBeTruthy())
    expect(roleArgs()).toEqual([{ adoptable: true }])

    act(() => emit('spare:adopted', { generation: 4 }))
    await waitFor(() => expect(screen.getAllByTestId('view-renderer')).toHaveLength(1))
    await waitFor(() => expect(reports()).toEqual([[4]]))
  })

  it('번호 0 도 채택 번호로 싣는다', async () => {
    listTabsAnswers(() => true)
    windowRoleMock.mockImplementation(async () => ({ role: 'spare', generation: null }))
    render(<WindowLayout label={SPARE} />)
    await waitFor(() => expect(screen.getByTestId('spare-canvas')).toBeTruthy())
    act(() => emit('spare:adopted', { generation: 0 }))
    await waitFor(() => expect(reports()).toEqual([[0]]))
  })

  // A prewarmed page whose `window_role` failed already reported `bootFailed` in normal mode by pulling its own label, which the model does not know.
  it('window_role 실패 페이지 — 채택에 부팅 실패를 내리고 다시 당겨 탭을 그린 뒤 그 번호로 다시 알린다', async () => {
    let adopted = false
    listTabsAnswers(() => adopted)
    windowRoleMock.mockImplementation(async () => {
      throw new Error('no dock state')
    })
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    try {
      render(<WindowLayout label={SPARE} />)
      await waitFor(() => expect(screen.getByTestId('window-boot-error')).toBeTruthy(), { timeout: 3000 })
      await waitFor(() => expect(reports()).toEqual([[null]]))
      const pulled = listTabsCalls().length

      adopted = true
      act(() => emit('spare:adopted', { generation: 2 }))
      expect(screen.queryByTestId('window-boot-error')).toBeNull()
      await waitFor(() => expect(screen.getAllByTestId('view-renderer')).toHaveLength(1))
      expect(listTabsCalls()).toHaveLength(pulled + 1)
      await waitFor(() => expect(reports()).toEqual([[null], [2]]))
    } finally {
      warn.mockRestore()
      error.mockRestore()
    }
  }, 6000)

  it('채택 전후로 첫 화면이 이어져도(부팅 실패를 내림과 탭 반영이 한 렌더) 그 번호로 다시 알린다', async () => {
    listTabsAnswers(() => false)
    windowRoleMock.mockImplementation(async () => {
      throw new Error('no dock state')
    })
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    try {
      render(<WindowLayout label={SPARE} />)
      await waitFor(() => expect(screen.getByTestId('window-boot-error')).toBeTruthy(), { timeout: 3000 })
      act(() => useViewStore.getState().applyWindowTabsUpdated(TABS))
      await waitFor(() => expect(screen.getAllByTestId('view-renderer')).toHaveLength(1))
      expect(reports()).toEqual([[null]])

      act(() => emit('spare:adopted', { generation: 9 }))
      await waitFor(() => expect(reports()).toEqual([[null], [9]]))
    } finally {
      warn.mockRestore()
      error.mockRestore()
    }
  }, 6000)

  // The shell does not check whether the page has been fully loaded before handing over — a page that missed the adoption event receives the number as the reply while waiting to become visible.
  it('spare:adopted 뒤에 마운트한 페이지 — window_role 답 model + 번호로 보통 모드 · 그 번호로 한 번 알린다', async () => {
    listTabsAnswers(() => true)
    windowRoleMock.mockImplementation(async () => ({ role: 'model', generation: 7 }))
    render(<WindowLayout label={SPARE} />)
    await waitFor(() => expect(screen.getAllByTestId('view-renderer')).toHaveLength(1))
    expect(screen.queryByTestId('spare-canvas')).toBeNull()
    await waitFor(() => expect(reports()).toEqual([[7]]))
  })

  it('채택을 본 뒤 늦게 온 window_role 답 spare 는 대기로 되돌리지 않는다', async () => {
    listTabsAnswers(() => true)
    let answer: (reply: unknown) => void = () => {}
    windowRoleMock.mockImplementation(() => new Promise(res => (answer = res)))
    render(<WindowLayout label={SPARE} />)
    await waitFor(() => expect(ipcOrder).toContain('invoke:window_role'))

    act(() => emit('spare:adopted', { generation: 5 }))
    await waitFor(() => expect(screen.getAllByTestId('view-renderer')).toHaveLength(1))
    await act(async () => answer({ role: 'spare', generation: null }))
    expect(screen.queryByTestId('spare-canvas')).toBeNull()
    expect(screen.getAllByTestId('view-renderer')).toHaveLength(1)
    await waitFor(() => expect(reports()).toEqual([[5]]))
  })

  it('spare:adopted 듣기를 못 걸면 adoptable: false 로 묻고 · 답 spare 가 와도 대기로 그리지 않는다', async () => {
    listTabsAnswers(() => true)
    rejectListen.add('spare:adopted')
    windowRoleMock.mockImplementation(async () => ({ role: 'spare', generation: null }))
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    try {
      render(<WindowLayout label={SPARE} />)
      await waitFor(() => expect(screen.getAllByTestId('view-renderer')).toHaveLength(1))
      expect(roleArgs()).toEqual([{ adoptable: false }])
      expect(screen.queryByTestId('spare-canvas')).toBeNull()
      expect(listTabsCalls()).toEqual([['list_tabs', { window: SPARE }]])
    } finally {
      error.mockRestore()
    }
  })

  it('StrictMode 두 번 마운트 — 늘 adoptable: true 로 묻고 채택 뒤 알림은 한 번', async () => {
    listTabsAnswers(() => true)
    windowRoleMock.mockImplementation(async () => ({ role: 'spare', generation: null }))
    render(
      <StrictMode>
        <WindowLayout label={SPARE} />
      </StrictMode>,
    )
    await waitFor(() => expect(screen.getByTestId('spare-canvas')).toBeTruthy())
    expect(roleArgs().length).toBeGreaterThanOrEqual(1)
    expect(roleArgs().every(a => (a as { adoptable: boolean }).adoptable === true)).toBe(true)

    act(() => emit('spare:adopted', { generation: 3 }))
    await waitFor(() => expect(screen.getAllByTestId('view-renderer')).toHaveLength(1))
    await waitFor(() => expect(reports()).toEqual([[3]]))
  })
})
