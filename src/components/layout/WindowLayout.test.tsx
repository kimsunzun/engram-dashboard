import { StrictMode, useEffect, useState } from 'react'
import { act, cleanup, createEvent, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ── listen mock: keep handlers per event name → the test emits directly ──
const listeners = new Map<string, (e: { payload: unknown }) => void>()
/** Event name → registration options (observes `target`). */
const listenOptions = new Map<string, unknown>()
/** IPC order observation — `listen:<event>` · `invoke:window_role`. */
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

// ── ViewLayoutRenderer stub — the canvas inside is out of scope; observes which view it draws + ★mount counter★.
// ★S4-F5 keep-alive no-remount★: mount `useEffect([])` bumps a per-viewId counter; if it does not grow after a tab switch and only display toggles, the keep-alive ("lossless switch", ADR-0056) structure holds.
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

describe('WindowLayout — initial pull + keep-alive canvas', () => {
  it('on mount list_tabs(label) pull → tab bar + all tab canvases mount (keep-alive)', async () => {
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

  it('each tab canvas fills its view with get_view and passes it down to ViewLayoutRenderer as viewIdOverride', async () => {
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getAllByTestId('view-renderer').length).toBe(2))
    expect(invokeMock).toHaveBeenCalledWith('get_view', { viewId: 'v1' })
    expect(invokeMock).toHaveBeenCalledWith('get_view', { viewId: 'v2' })
    const renderers = screen.getAllByTestId('view-renderer')
    const ids = renderers.map(r => r.getAttribute('data-view-id')).sort()
    expect(ids).toEqual(['v1', 'v2'])
  })

  // The rectangle array must be the cache reference as is — the divider preview recognizes a new snapshot by array reference (ADR-0227).
  it('passes the cache entry\'s rectangles (same reference) · ratio bounds · version · unknown content slots to the renderer', async () => {
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

// ── S4-F4: mount-race — a newer window:tabs-updated arrives during the initial list_tabs pull ──
// Scenario: listen registered first (§7-1 "subscribe first, pull later") → a newer-version window:tabs-updated arrives while list_tabs is pending → the pull resolves stale → the version guard of applyWindowTabsUpdated must block its overwrite.
describe('WindowLayout — mount-race (initial pull vs latest emit, S4-F4)', () => {
  it('while the list_tabs pull is pending a newer emit arrives → the late stale pull does not overwrite the latest state', async () => {
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

// ── ADR-0102: boot pull bounded retry + surfacing the final failure ──
// main has no event recovery path (window:tabs-updated fires only on tab mutation), so a one-shot boot list_tabs pull failing early = loading stuck forever.
describe('WindowLayout — boot pull retry/surfacing (ADR-0102)', () => {
  it('list_tabs rejects twice then resolves → recovered by retry and the tab bar appears', async () => {
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

  it('list_tabs keeps rejecting → after retries are exhausted a visible error state renders (not stuck loading)', async () => {
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

  // listen() used to await outside the try, so a rejected registration killed the async IIFE unhandled — the pull never started and bootFailed was never set. Now a listen failure surfaces as bootFailed like a pull failure.
  it('listen registration rejects → boot-error is surfaced rather than quietly stuck loading (FIX-2)', async () => {
    listenShouldReject = true
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getByTestId('window-boot-error')).toBeTruthy(), {
      timeout: 3000,
    })
    expect(screen.queryByTestId('tab-bar')).toBeNull()
  })
})

// ── First screen and first screen notice — a window the shell built hidden is shown on the notice (`theme/firstPaint.ts`) ──
describe('WindowLayout — first screen · first-paint notice', () => {
  it('before tabs are received only the background (no text) · the notice goes once after the tab bar and the active tab\'s layout are drawn', async () => {
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

  it('on boot failure the failure text is the first screen — the notice is sent', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_tabs') throw new Error('backend down')
      return undefined
    })
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getByTestId('window-boot-error')).toBeTruthy(), { timeout: 3000 })
    await waitFor(() => expect(reportFirstPaintMock).toHaveBeenCalledTimes(1))
  })

  // TRD S26-window-chrome §7-7 — so the window is widened before the shell shows it.
  it('the notice is after the title row\'s minimum window size report answers — the report goes out first and waits for its answer', async () => {
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

  it('even if the report does not answer or is refused the notice goes out after the cap', async () => {
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
          throw new Error('refused (test)')
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
  it('a waiting window does not announce the first screen', async () => {
    currentWindow.label = 'slot-popup-7'
    windowRoleMock.mockImplementation(async () => ({ role: 'spare', generation: null }))
    render(<WindowLayout label="slot-popup-7" />)
    await waitFor(() => expect(screen.getByTestId('spare-canvas')).toBeTruthy())
    await new Promise(res => setTimeout(res, 50))
    expect(reportFirstPaintMock).not.toHaveBeenCalled()
  })
})

// ── FIX-3: per-tab get_view keep-alive pull self-heals with bounded retry ──
// A one-shot get_view only console.warned on a transient failure and tabIdsKey stayed unchanged (no re-emit trigger), so that tab's canvas stayed on "View loading" until an unrelated layout:updated.
describe('WindowLayout — get_view keep-alive retry self-heal (FIX-3)', () => {
  it('get_view first rejects, succeeds on retry → that tab canvas recovers (republish observed)', async () => {
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
    expect(v1Calls).toBeGreaterThanOrEqual(3)
  })
})

describe('WindowLayout — window:tabs-updated own-label filter', () => {
  it('own label emit → active tab swap (v1→v2)', async () => {
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

  it('other label emit → ignored (own window unchanged)', async () => {
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

// ── S4-F5: keep-alive no-remount — slot components are not remounted on tab switch ──
// Unit proxy for ADR-0056 "lossless switch": the mount counter shows renderers are not remounted and only display toggles (real xterm survival = qa cdp stage 6).
describe('WindowLayout — keep-alive no-remount(ADR-0056, S4-F5)', () => {
  it('after a tab switch (v1→v2) the two slot renderers\' mount counts do not grow and only display toggles', async () => {
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

describe('WindowLayout — 0-tab self-close (§5-2/G2 · TRD S26-window-chrome §5-5)', () => {
  const CLOSE_IF_EMPTY = ['window_control', { window: 'wid-slot-popup-1', action: 'close', ifEmpty: true }]

  it('0-tab signal (window:tabs-updated{tabs:[]}) → registry window.close{if_empty} with the window_id received when it had tabs', async () => {
    render(<WindowLayout label="slot-popup-1" />)
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
    // The window_id of a 0-tab payload is not used — only the value received while tabs existed is used.
    emit('window:tabs-updated', { label: 'slot-popup-1', window_id: 'other', tabs: [], active: 'v1', version: 5 })
    await waitFor(() => expect(closeCalls()).toEqual([CLOSE_IF_EMPTY]))
  })

  it('even if the 0-tab signal arrives twice the request goes once (idempotent re-entry guard)', async () => {
    render(<WindowLayout label="slot-popup-1" />)
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
    emit('window:tabs-updated', { label: 'slot-popup-1', tabs: [], active: 'v1', version: 5 })
    emit('window:tabs-updated', { label: 'slot-popup-1', tabs: [], active: 'v1', version: 6 })
    await waitFor(() => expect(closeCalls()).toHaveLength(1))
    await Promise.resolve()
    expect(closeCalls()).toHaveLength(1)
  })

  it('after the request, if tabs come again (the shell did not close — Kept) it requests again on the next 0 tabs', async () => {
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
    ['no window_id', undefined],
    ['only the window_id of a 0-tab payload', 'wid-0tab'],
  ])('does not request without the window_id from when it had tabs (%s)', async (_case, windowId) => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
        if (cmd === 'list_tabs') return { label: 'slot-popup-1', window_id: windowId, tabs: [], active: '', version: 1 }
        if (cmd === 'get_view') return slotSnap((args as { viewId: string }).viewId, 1)
        if (cmd === 'window_control') return KEPT
        return undefined
      })
      render(<WindowLayout label="slot-popup-1" />)
      await waitFor(() => expect(warn).toHaveBeenCalledWith(expect.stringMatching(/no window_id from when .* held a tab, so not requesting/)))
      expect(closeCalls()).toEqual([])
    } finally {
      warn.mockRestore()
    }
  })
})

// ── ADR-0227: window canvas report (TRD §2d) ──
// jsdom has no ResizeObserver and vitest has no setupFiles, so only this block installs a stub and fires its callback directly.
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

describe('WindowLayout — window canvas report (ADR-0227, TRD §2d)', () => {
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

  it('even if win arrives late the observation attaches to the tab content area via the callback ref and the first measurement is reported', async () => {
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

  it('debounce → integer rounding → sent only when changed · no resend of the same size · a 0 size is not sent', async () => {
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

  it('on failure it resends by bounded retry even with no RO change, and only a successful value becomes the previous value', async () => {
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

  it('when retries are exhausted it leaves console.error and does not update the previous value', async () => {
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

  it('if a different size is observed while waiting to retry it stops the old retry without waiting for the debounce', async () => {
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

  it('a 0-size observation does not stop the retry of the value being sent', async () => {
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

  it('while an invoke is in flight a new size is sent after its response (one per window)', async () => {
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

  it('of the sizes arriving while in flight only the last one is sent after the response, and it is skipped if equal to that response value', async () => {
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

  it('a value waiting for the response is discarded when a newer observation comes so it is not sent even after the response', async () => {
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

  it('even if the hook instance changes (remount) nothing is sent before the old instance\'s invoke response', async () => {
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

  it('if it returns to the last successful value while a different value is in flight it is judged by that response result', async () => {
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

  it('a tab-switch re-render does not cut observation or debounce (ref reference fixed)', async () => {
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

  it('unmounting during debounce sends nothing and detaches observation', async () => {
    const { ro, unmount } = await mountReady()
    vi.useFakeTimers()

    ro.fire(800, 600)
    await vi.advanceTimersByTimeAsync(50)
    unmount()
    expect(ro.disconnected).toBe(true)
    await vi.advanceTimersByTimeAsync(500)
    expect(canvasReports()).toEqual([])
  })

  it('unmounting during retry stops the retry and leaves no final failure log either', async () => {
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

  it('unmounting while waiting for the response does not send the waiting size even if the response comes', async () => {
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

  it('even in an environment without ResizeObserver it does not throw and renders without a report', async () => {
    vi.stubGlobal('ResizeObserver', undefined)
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
    expect(screen.getAllByTestId('tab-canvas')).toHaveLength(2)
    await new Promise(res => setTimeout(res, 150))
    expect(canvasReports()).toEqual([])
  })
})

// TRD S26-window-chrome §7-1 · §7-5 — a title row where the tab bar used to be placed (layout · buttons · dimming details = `titleRow/TitleRow.test.tsx`).
describe('WindowLayout — title row', () => {
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

  it('main — the title row holds the tab bar · number marker = the number of the tab notice · third button = hide all windows', async () => {
    tabsWithNumber('main', 3)
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getByTestId('window-number').textContent).toBe('3'))
    const row = screen.getByTestId('title-row')
    expect(row.firstElementChild).toBe(screen.getByTestId('tab-bar'))
    expect(screen.getByTestId('caption-close').getAttribute('aria-label')).toBe('모든 창 숨기기')
  })

  it('popout — third button = close window · pressing sends window_control close with this window label', async () => {
    currentWindow.label = 'slot-popup-4'
    tabsWithNumber('slot-popup-4', 2)
    render(<WindowLayout label="slot-popup-4" />)
    await waitFor(() => expect(screen.getByTestId('window-number').textContent).toBe('2'))
    const close = screen.getByTestId('caption-close')
    expect(close.getAttribute('aria-label')).toBe('창 닫기')
    fireEvent.click(close)
    expect(closeCalls()).toEqual([['window_control', { window: 'slot-popup-4', action: 'close', ifEmpty: false }]])
  })

  it('the title row follows the focused · maximized of the window fact (window:frame)', async () => {
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

// T-66: the notice band does not cover the title row (window buttons · draggable area) — it attaches to the top of the canvas area below it. jsdom cannot measure sizes, so the position is asserted structurally (the covering layer's position · z belongs to NoticeOverlay.test).
describe('WindowLayout — place of the notice layer', () => {
  const notices = <div data-testid="notices-probe" />

  it('the notice layer is outside the title row · attached at the very front of the canvas area (positioned), and the title row is ahead of it', async () => {
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

  it('the loading screen also draws the notice layer under the title row (drag spot · window buttons — the marker is not there yet)', async () => {
    invokeMock.mockImplementation(() => new Promise(() => {}))
    render(<WindowLayout label="main" notices={notices} />)
    expect(screen.getByTestId('window-loading').textContent).toBe('')
    expectFrame('window-loading')
    await waitFor(() => expect(screen.getByTestId('caption-buttons')).toBeTruthy())
    expect(screen.getByTestId('tab-bar-standin').hasAttribute('data-tauri-drag-region')).toBe(true)
    expect(screen.queryByTestId('window-number')).toBeNull()
    expect(screen.queryByTestId('tab-bar')).toBeNull()
  })

  // The window may not be in the model (a stale popout): window buttons go by label without window_id and the shell attaches to the open popout of that label (TRD S26-window-chrome §6-2 · shell test `window_buttons_of_an_unmodeled_window_reach_its_os_window`).
  it('the boot failure screen also draws the notice layer under the title row — window buttons go to that label and the connection failure reason is readable', async () => {
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

  it('before receiving the number (loading · boot failure) the minimum window size is not sent', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_tabs') throw new Error('backend down')
      return undefined
    })
    render(<WindowLayout label="main" notices={notices} />)
    await waitFor(() => expect(screen.getByTestId('window-boot-error')).toBeTruthy(), { timeout: 3000 })
    expect(invokeMock.mock.calls.filter(c => c[0] === 'report_window_min_width')).toEqual([])
  })
})

// ADR-0277: the dismissal memory (✕) of `ConnectionNotice` · `StateFileNotice` is not withdrawn — the notice layer must not remount when the screen changes.
describe('WindowLayout — the notice\'s dismissal memory survives screen changes', () => {
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

  it('close during loading and when tabs come it stays closed', async () => {
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

  it('close on the boot failure screen and even if revived by a tab notice it stays closed', async () => {
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

describe('WindowLayout — TabBar action → store action (this label)', () => {
  it('[+] click → createTab(label) invoke', async () => {
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getByTestId('tab-add')).toBeTruthy())
    fireEvent.click(screen.getByTestId('tab-add'))
    expect(invokeMock).toHaveBeenCalledWith('create_tab', { window: 'main', name: null })
  })

  it('hidden tab press → switchTab(label, view) invoke', async () => {
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
    const tab2 = screen.getAllByTestId('tab').find(t => t.getAttribute('data-view-id') === 'v2')!
    fireEvent.pointerDown(tab2, { button: 0, pointerId: 1 })
    expect(invokeMock).toHaveBeenCalledWith('switch_tab', { window: 'main', view: 'v2' })
  })

  it('tab × click → closeTab(label, view) invoke', async () => {
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

describe('WindowLayout — tab drag wiring', () => {
  beforeEach(() => {
    stripWidth = 800
    vi.stubGlobal('matchMedia', vi.fn(() => ({ matches: false }) as MediaQueryList))
  })
  afterEach(() => {
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
  })

  it('shell drag session events (dock:drag · dock:feed) are not listened to — only spare:adopted is hooked on its own window, then window_role is asked', async () => {
    currentWindow.label = 'slot-popup-3'
    await mountDockWindow('slot-popup-3')
    expect(listenOptions.get('spare:adopted')).toEqual({ target: 'slot-popup-3' })
    expect(ipcOrder.indexOf('listen:spare:adopted')).toBeLessThan(ipcOrder.indexOf('invoke:window_role'))
    expect(ipcOrder).not.toContain('listen:dock:drag')
    expect(ipcOrder).not.toContain('listen:dock:feed')
  })

  it('while dragging in this window, outside the tab bar zone it shows the remaining tab canvases (R15 · model unchanged) and on return the dragged tab · chrome = viewport − tab canvas', async () => {
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

  it('the drag guide layer is directly inside the tab canvas box covering that box · always mounted (no guide)', async () => {
    await mountDropWindow()
    const layer = screen.getByTestId('dock-overlay-layer')
    expect(layer.parentElement).toBe(canvasOf('v1').parentElement)
    expect(layer.style.position).toBe('absolute')
    expect(layer.style.pointerEvents).toBe('none')
    expect(screen.queryByTestId('dock-overlay')).toBeNull()
  })

  it('it receives our drag at the root and draws a preview in canvas coordinates · does not call the shell while dragging · sends one Seam on drop', async () => {
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

  it('a drag that is not ours (tree row · file) is neither received nor cut — it goes through to window\'s react-dnd-style listeners', async () => {
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

  it('it blocks file · URL drag-and-drop (including in-app links) · text drag as is · detaches when taken down', async () => {
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

// ── Waiting mode (TRD S21-slot-docking §3-9) — a spare-pool window (hidden spare) is an empty frame · 0-tab self-close off · no tab pull-in · measuring reports as usual. Mode = `window_role` at mount, then `spare:adopted` (to normal). ──
describe('WindowLayout — waiting mode (window_role · spare:adopted)', () => {
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

  it('hooks spare:adopted listening on its own window first, then asks window_role', async () => {
    render(<WindowLayout label={SPARE} />)
    await waitFor(() => expect(ipcOrder).toContain('invoke:window_role'))
    expect(ipcOrder.indexOf('listen:spare:adopted')).toBeGreaterThanOrEqual(0)
    expect(ipcOrder.indexOf('listen:spare:adopted')).toBeLessThan(ipcOrder.indexOf('invoke:window_role'))
    expect(listenOptions.get('spare:adopted')).toEqual({ target: SPARE })
  })

  it('window_role = spare → empty frame (tab bar + empty canvas) · does not pull list_tabs · no boot error · loading text', async () => {
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

  it('in waiting mode it does not close even if this window\'s 0-tab notice arrives', async () => {
    await mountSpare()
    act(() =>
      useViewStore.getState().applyWindowTabsUpdated({ label: SPARE, window_id: 'w-spare', number: 2, title: 'Engram:2', tabs: [], active: '', version: 9 }),
    )
    await new Promise(res => setTimeout(res, 20))
    expect(closeCalls()).toEqual([])
    expect(screen.getByTestId('spare-canvas')).toBeTruthy()
  })

  it('in waiting mode it also sends the two measurements (canvas · area frame metrics)', async () => {
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

  it('spare:adopted → normal mode: pulls and draws the adopted tabs with list_tabs, and 0-tab self-close revives', async () => {
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

  it('a waiting window does not pull the window fact (window_frame) — outside the shell window model · it pulls when adopted', async () => {
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

  it('until it pulls its own tabs after adoption it is the empty frame as is — no loading text · the tab bar row is the same node', async () => {
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

  it('if a tab to adopt arrives first during waiting (global mirror) it is an empty frame until waiting is released · that tab the moment it is released', async () => {
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

  it('if window_role = unknown or fails it is drawn as normal', async () => {
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

  it('while the mode is unknown (before the window_role answer) it does not close even if a 0-tab notice arrives — if it answers waiting it never closes', async () => {
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
describe('WindowLayout — adoption number · first-screen re-notice', () => {
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

  it('waiting → adoption: after drawing its own tabs it notifies once with that adoption number', async () => {
    listTabsAnswers(() => true)
    windowRoleMock.mockImplementation(async () => ({ role: 'spare', generation: null }))
    render(<WindowLayout label={SPARE} />)
    await waitFor(() => expect(screen.getByTestId('spare-canvas')).toBeTruthy())
    expect(roleArgs()).toEqual([{ adoptable: true }])

    act(() => emit('spare:adopted', { generation: 4 }))
    await waitFor(() => expect(screen.getAllByTestId('view-renderer')).toHaveLength(1))
    await waitFor(() => expect(reports()).toEqual([[4]]))
  })

  it('number 0 is also carried as an adoption number', async () => {
    listTabsAnswers(() => true)
    windowRoleMock.mockImplementation(async () => ({ role: 'spare', generation: null }))
    render(<WindowLayout label={SPARE} />)
    await waitFor(() => expect(screen.getByTestId('spare-canvas')).toBeTruthy())
    act(() => emit('spare:adopted', { generation: 0 }))
    await waitFor(() => expect(reports()).toEqual([[0]]))
  })

  // A prewarmed page whose `window_role` failed already reported `bootFailed` in normal mode by pulling its own label, which the model does not know.
  it('window_role failure page — takes boot failure down on adoption, pulls again and draws tabs, then notifies again with that number', async () => {
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

  it('even if the first screen continues around adoption (taking boot failure down and reflecting tabs in one render) it notifies again with that number', async () => {
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
  it('a page mounted after spare:adopted — window_role answer model + number gives normal mode · notifies once with that number', async () => {
    listTabsAnswers(() => true)
    windowRoleMock.mockImplementation(async () => ({ role: 'model', generation: 7 }))
    render(<WindowLayout label={SPARE} />)
    await waitFor(() => expect(screen.getAllByTestId('view-renderer')).toHaveLength(1))
    expect(screen.queryByTestId('spare-canvas')).toBeNull()
    await waitFor(() => expect(reports()).toEqual([[7]]))
  })

  it('a late window_role answer spare after seeing adoption does not return it to waiting', async () => {
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

  it('if spare:adopted listening cannot be hooked it asks with adoptable: false · even if the answer is spare it is not drawn as waiting', async () => {
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

  it('StrictMode double mount — always asks with adoptable: true and the notice after adoption is once', async () => {
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
