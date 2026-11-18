// viewStore unit test — emit↔invoke loop (ADR-0035/0057 vertical slice, tab ownership model).
// Mocks invoke + listen of `@tauri-apps/api` to verify that actions call the correct invoke, that state updates on the backend emit (layout:updated/window:tabs-updated), that the version guard discards stale emits, and ★the per-window active tab decision (useCurrentViewId family)★. Pure logic, no real Tauri.
// ★Invariant (tab ownership model, ADR-0057)★: no global activeViewId. The window (label) owns the tab list and `viewStore.windows[label].active` is that window's active tab; layout is a per-view_id cache (global monotonic version guard); window:tabs-updated also discards stale emits with a per-window version guard (G10).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { ViewSnapshot } from '../api/layoutTypes'

const invokeMock = vi.fn(async (_cmd: string, ..._rest: unknown[]) => undefined as unknown)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, ...rest: unknown[]) => invokeMock(cmd, ...rest),
}))

// listen mock: keeps registered handlers per event name so the test imitates emit directly (emit helper).
const listeners = new Map<string, (e: { payload: unknown }) => void>()
const unlistenMock = vi.fn()
const listenMock = vi.fn(async (event: string, handler: (e: { payload: unknown }) => void) => {
  listeners.set(event, handler)
  return unlistenMock
})
vi.mock('@tauri-apps/api/event', () => ({
  listen: (event: string, handler: (e: { payload: unknown }) => void) => listenMock(event, handler),
}))

import {
  currentViewId,
  initMainWindowFromBackend,
  MAIN_WINDOW_LABEL,
  readWindowLabelFromHash,
  selectView,
  subscribeViewEvents,
  useViewStore,
  type WindowTabsPayload,
} from './viewStore'

/** Imitates a backend emit — pours the payload into the handler subscribeViewEvents registered. */
function emit(event: string, payload: unknown): void {
  const h = listeners.get(event)
  if (!h) throw new Error(`no listener for ${event} — was subscribeViewEvents called?`)
  h({ payload })
}

function snap(overrides: Partial<ViewSnapshot> = {}): ViewSnapshot {
  return {
    view_id: 'v1',
    layout: { type: 'slot', id: 's1', content: { type: 'empty' } }, // ADR-0060
    focused_slot_id: 's1',
    foreign_slots: [],
    slot_spatial: [], // ADR-0068: space derivation (not used in this test — fills the type with an empty array)
    slot_rects: [{ slot_id: 's1', x0: 0, y0: 0, x1: 1, y1: 1 }], // ADR-0227
    split_rects: [],
    ratio_min: 0.1,
    ratio_max: 0.9,
    version: 1,
    ...overrides,
  }
}

function tabsPayload(overrides: Partial<WindowTabsPayload> = {}): WindowTabsPayload {
  return {
    label: 'main',
    window_id: 'main',
    number: 1,
    title: 'View 1 - Engram:1',
    tabs: [{ id: 'v1', name: 'View 1', named: true }],
    active: 'v1',
    version: 1,
    ...overrides,
  }
}

const origHash = window.location.hash

beforeEach(() => {
  invokeMock.mockClear()
  invokeMock.mockImplementation(async () => undefined)
  listeners.clear()
  unlistenMock.mockClear()
  listenMock.mockClear()
  useViewStore.setState({ layouts: {}, windows: {}, renderModeOverride: {} })
  window.location.hash = '#/' // Default = main window
})
afterEach(() => {
  vi.restoreAllMocks()
  window.location.hash = origHash
})

describe('viewStore tab/window actions → invoke (tab ownership model, ADR-0057)', () => {
  it('createTab → create_tab invoke(window,name) + passes the returned id', async () => {
    invokeMock.mockResolvedValueOnce('new-view-id')
    const id = await useViewStore.getState().createTab('main', 'My Tab')
    expect(invokeMock).toHaveBeenCalledWith('create_tab', { window: 'main', name: 'My Tab' })
    expect(id).toBe('new-view-id')
  })

  it('createTab(no name) → name=null', async () => {
    await useViewStore.getState().createTab('main')
    expect(invokeMock).toHaveBeenCalledWith('create_tab', { window: 'main', name: null })
  })

  it('closeTab/switchTab → matching invoke(window, view)', async () => {
    const s = useViewStore.getState()
    await s.closeTab('slot-popup-1', 'v9')
    expect(invokeMock).toHaveBeenCalledWith('close_tab', { window: 'slot-popup-1', view: 'v9' })
    await s.switchTab('main', 'v2')
    expect(invokeMock).toHaveBeenCalledWith('switch_tab', { window: 'main', view: 'v2' })
  })

  it("switchTabIf → switch_tab_if invoke(window, view, expect) · returns the shell's bool", async () => {
    invokeMock.mockResolvedValueOnce(false)
    await expect(useViewStore.getState().switchTabIf('main', 'v1', 'v2')).resolves.toBe(false)
    expect(invokeMock).toHaveBeenCalledWith('switch_tab_if', { window: 'main', view: 'v1', expect: 'v2' })
  })

  it('renameTab → rename_tab invoke({ viewId, name })', async () => {
    await useViewStore.getState().renameTab('v1', 'New Name')
    expect(invokeMock).toHaveBeenCalledWith('rename_tab', { viewId: 'v1', name: 'New Name' })
  })

  it('createWindow → create_window invoke · returns {window, window_id, number} as is', async () => {
    const created = { window: 'slot-popup-3', window_id: '7d2f0b8e-1c3a-4f5e-9b6d-2a4c6e8f0a1b', number: 2 }
    invokeMock.mockResolvedValueOnce(created)
    const answer = await useViewStore.getState().createWindow()
    expect(invokeMock).toHaveBeenCalledWith('create_window')
    expect(answer).toEqual(created)
  })

  it('controlWindow → a single window_control invoke (window · action · ifEmpty) · returns the shell answer as is', async () => {
    const outcome = { deferred: false, pending: false, shown_all: false, focused: false, close: 'kept' }
    invokeMock.mockResolvedValueOnce(outcome)
    await expect(useViewStore.getState().controlWindow('w-1', 'close', true)).resolves.toEqual(outcome)
    expect(invokeMock).toHaveBeenLastCalledWith('window_control', { window: 'w-1', action: 'close', ifEmpty: true })
    await useViewStore.getState().controlWindow('slot-popup-3', 'maximize')
    expect(invokeMock).toHaveBeenLastCalledWith('window_control', {
      window: 'slot-popup-3',
      action: 'maximize',
      ifEmpty: null,
    })
  })

  it('split → split_slot invoke(viewId/slotId/dir) + returns the new slot id', async () => {
    invokeMock.mockResolvedValueOnce('new-slot-id')
    const id = await useViewStore.getState().split('v1', 's1', 'top_bottom')
    expect(invokeMock).toHaveBeenCalledWith('split_slot', {
      viewId: 'v1',
      slotId: 's1',
      dir: 'top_bottom',
    })
    expect(id).toBe('new-slot-id')
  })

  it('setSplitRatio → set_split_ratio invoke(viewId/splitId/ratio) + returns the shell answer as is', async () => {
    const applied = { ratio: 0.3, outcome: 'Applied', version: 42 }
    invokeMock.mockResolvedValueOnce(applied)
    const res = await useViewStore.getState().setSplitRatio('v1', 'sp1', 0.3)
    expect(invokeMock).toHaveBeenCalledTimes(1)
    expect(invokeMock).toHaveBeenCalledWith('set_split_ratio', {
      viewId: 'v1',
      splitId: 'sp1',
      ratio: 0.3,
    })
    expect(res).toEqual(applied)
  })

  it('setSplitRatio failure → rejects with the shell error string', async () => {
    invokeMock.mockRejectedValueOnce('split not found')
    await expect(useViewStore.getState().setSplitRatio('v1', 'gone', 0.5)).rejects.toBe('split not found')
  })

  it('closeSlot/assignAgent → matching invoke arguments', async () => {
    const s = useViewStore.getState()
    await s.closeSlot('v1', 's2')
    expect(invokeMock).toHaveBeenCalledWith('close_slot', { viewId: 'v1', slotId: 's2' })
    await s.assignAgent('v1', 's1', 'agent-9')
    expect(invokeMock).toHaveBeenCalledWith('assign_agent', {
      viewId: 'v1',
      slotId: 's1',
      agentId: 'agent-9',
    })
  })

  it('setSlotContent → set_slot_content invoke(viewId/slotId/content) (ADR-0063 generic placement)', async () => {
    const s = useViewStore.getState()
    await s.setSlotContent('v1', 's1', { type: 'agent_list' })
    expect(invokeMock).toHaveBeenCalledWith('set_slot_content', {
      viewId: 'v1',
      slotId: 's1',
      content: { type: 'agent_list' },
    })
    await s.setSlotContent('v1', 's2', { type: 'empty' })
    expect(invokeMock).toHaveBeenCalledWith('set_slot_content', {
      viewId: 'v1',
      slotId: 's2',
      content: { type: 'empty' },
    })
  })

  it('setUsageSlot → set_usage_slot invoke — values only for the given fields, null for missing ones (the shell keeps the current value) · override clear', async () => {
    const s = useViewStore.getState()
    s.setRenderMode('s-usage', 'dom')
    await s.setUsageSlot('v1', 's-usage', { show_codex: true })
    expect(invokeMock).toHaveBeenLastCalledWith('set_usage_slot', {
      viewId: 'v1',
      slotId: 's-usage',
      showClaude: null,
      showCodex: true,
    })
    expect(useViewStore.getState().renderModeOverride['s-usage']).toBeUndefined()
    await s.setUsageSlot('v1', 's-usage', { show_claude: false, show_codex: false })
    expect(invokeMock).toHaveBeenLastCalledWith('set_usage_slot', {
      viewId: 'v1',
      slotId: 's-usage',
      showClaude: false,
      showCodex: false,
    })
  })

  it('moveSlotToWindow → move_slot_to_window invoke(viewId/slotId/toWindow) + returns {window,tab,window_id,number}', async () => {
    const moved = { window: 'slot-popup-2', tab: 'v-new', window_id: '7d2f0b8e-1c3a-4f5e-9b6d-2a4c6e8f0a1b', number: 2 }
    invokeMock.mockResolvedValueOnce(moved)
    const res = await useViewStore.getState().moveSlotToWindow('v1', 's1')
    // null = new popup window.
    expect(invokeMock).toHaveBeenCalledWith('move_slot_to_window', {
      viewId: 'v1',
      slotId: 's1',
      toWindow: null,
    })
    expect(res).toEqual(moved)
  })

  it('moveSlotToWindow (toWindow given) → passes that label (existing window target)', async () => {
    invokeMock.mockResolvedValueOnce({ window: 'main', tab: 'v-new', window_id: 'main', number: 1 })
    await useViewStore.getState().moveSlotToWindow('v1', 's1', 'main')
    expect(invokeMock).toHaveBeenCalledWith('move_slot_to_window', {
      viewId: 'v1',
      slotId: 's1',
      toWindow: 'main',
    })
  })
})

describe('viewStore emit receipt → state update', () => {
  it('layout:updated → adopts that view cache entry (layout/focus/version)', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    emit('layout:updated', snap({ view_id: 'v1', version: 3, focused_slot_id: 's2' }))
    const cached = useViewStore.getState().layouts['v1']
    expect(cached.version).toBe(3)
    expect(cached.focusedSlotId).toBe('s2')
    expect(cached.layout).toEqual({ type: 'slot', id: 's1', content: { type: 'empty' } })
  })

  it('layout:updated → keeps the shell geometry (slot_rects/split_rects) and the ratio limits in the cache entry as is (ADR-0227)', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    const slotRects = [
      { slot_id: 'a', x0: 0, y0: 0, x1: 0.3, y1: 1 },
      { slot_id: 'b', x0: 0.3, y0: 0, x1: 1, y1: 1 },
    ]
    const splitRects = [
      { split_id: 'p', dir: 'left_right' as const, x0: 0, y0: 0, x1: 1, y1: 1, at: 0.3 },
    ]
    emit('layout:updated', snap({
      view_id: 'v1',
      version: 2,
      layout: {
        type: 'split',
        id: 'p',
        dir: 'left_right',
        ratio: 0.3,
        a: { type: 'slot', id: 'a', content: { type: 'empty' } },
        b: { type: 'slot', id: 'b', content: { type: 'empty' } },
      },
      focused_slot_id: 'a',
      slot_rects: slotRects,
      split_rects: splitRects,
      ratio_min: 0.15,
      ratio_max: 0.85,
    }))
    const cached = useViewStore.getState().layouts['v1']
    expect(cached.slotRects).toEqual(slotRects)
    expect(cached.splitRects).toEqual(splitRects)
    expect(cached.ratioMin).toBe(0.15)
    expect(cached.ratioMax).toBe(0.85)
  })

  // TRD S21-storage §6-2: unknown-content slots are carried as `empty` — if this field is dropped the frontend picks that slot as an empty area.
  it('layout:updated → carries foreign_slots on the cache entry, and drops them when they are gone from the next snapshot', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    emit('layout:updated', snap({ view_id: 'v1', version: 2, foreign_slots: ['s1'] }))
    expect(useViewStore.getState().layouts['v1'].foreignSlots).toEqual(['s1'])

    emit('layout:updated', snap({ view_id: 'v1', version: 3, foreign_slots: [] }))
    expect(useViewStore.getState().layouts['v1'].foreignSlots).toEqual([])
  })

  it('window:tabs-updated → updates windows[label].{windowId,number,tabs,active,version}', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    emit('window:tabs-updated', tabsPayload({
      label: 'slot-popup-1',
      window_id: 'w-popup',
      number: 3,
      tabs: [{ id: 'v1', name: 'View 1', named: true }, { id: 'v2', name: 'View 2', named: true }],
      active: 'v2',
      version: 4,
    }))
    const win = useViewStore.getState().windows['slot-popup-1']
    expect(win.windowId).toBe('w-popup')
    expect(win.number).toBe(3)
    expect(win.tabs).toHaveLength(2)
    expect(win.active).toBe('v2')
    expect(win.version).toBe(4)
  })

  it('window:tabs-updated is independent per window — different labels do not touch each other', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    emit('window:tabs-updated', tabsPayload({ label: 'main', active: 'v1', version: 1 }))
    emit('window:tabs-updated', tabsPayload({
      label: 'slot-popup-1',
      tabs: [{ id: 'p1', name: 'Popup Tab', named: true }],
      active: 'p1',
      version: 2,
    }))
    expect(useViewStore.getState().windows['main'].active).toBe('v1')
    expect(useViewStore.getState().windows['slot-popup-1'].active).toBe('p1')
  })
})

describe('version guard (global monotonic version, G10)', () => {
  it('layout:updated — a lower version emit of the same view is discarded (prevents order inversion)', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    emit('layout:updated', snap({ view_id: 'v1', version: 5, focused_slot_id: 's5' }))
    emit('layout:updated', snap({ view_id: 'v1', version: 3, focused_slot_id: 's3' }))
    const cached = useViewStore.getState().layouts['v1']
    expect(cached.version).toBe(5)
    expect(cached.focusedSlotId).toBe('s5')
  })

  it('window:tabs-updated — a lower/equal version emit of the same window is discarded (stale defence, G10)', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    emit('window:tabs-updated', tabsPayload({ label: 'main', active: 'v5', version: 5 }))
    emit('window:tabs-updated', tabsPayload({ label: 'main', active: 'v3', version: 3 }))
    emit('window:tabs-updated', tabsPayload({ label: 'main', active: 'vX', version: 5 }))
    expect(useViewStore.getState().windows['main'].active).toBe('v5')
  })

  it('the first layout:updated emit is always adopted since there is no cache entry (version 0 included)', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    emit('layout:updated', snap({ view_id: 'v1', version: 0 }))
    expect(useViewStore.getState().layouts['v1'].version).toBe(0)
  })
})

describe('selectView (window canvas render selector)', () => {
  it('a cached view_id → that entry, otherwise null', () => {
    emit_seed_layout('v1', { focused_slot_id: 's1', version: 1 })
    const st = useViewStore.getState()
    expect(selectView(st, 'v1')?.focusedSlotId).toBe('s1')
    expect(selectView(st, 'v-missing')).toBeNull()
    expect(selectView(st, null)).toBeNull()
  })
})

/** Seed directly into the layout cache (without the emit path) — selectView unit test helper. */
function emit_seed_layout(viewId: string, over: Partial<ViewSnapshot>): void {
  useViewStore.getState().applyLayoutUpdated(snap({ view_id: viewId, ...over }))
}

describe('initMainWindowFromBackend boot init (read-only pull)', () => {
  it('invokes in the order list_tabs("main") → get_view and fills the window tabs + active layout cache', async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_tabs') {
        expect(args).toEqual({ window: 'main' })
        return tabsPayload({ label: 'main', tabs: [{ id: 'v1', name: 'View 1', named: true }], active: 'v1', version: 0 })
      }
      if (cmd === 'get_view') return snap({ view_id: 'v1', version: 0, focused_slot_id: 's1' })
      return undefined
    })
    await initMainWindowFromBackend()
    expect(invokeMock).toHaveBeenCalledWith('list_tabs', { window: 'main' })
    expect(invokeMock).toHaveBeenCalledWith('get_view', { viewId: 'v1' })
    const st = useViewStore.getState()
    expect(st.windows['main'].active).toBe('v1')
    expect(selectView(st, 'v1')?.focusedSlotId).toBe('s1')
  })

  // ★ADR-0102: even if list_tabs fails a few times with early transients, retry recovers it and fills the window state★ — main has no event recovery path (window:tabs-updated fires only on tab mutations), so this pull must not be one-shot.
  it('list_tabs rejects twice then resolves → collected by retries and windows["main"] is filled (ADR-0102)', async () => {
    let listTabsCalls = 0
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_tabs') {
        listTabsCalls += 1
        if (listTabsCalls <= 2) throw new Error(`state not ready ${listTabsCalls}`)
        return tabsPayload({ label: 'main', tabs: [{ id: 'v1', name: 'View 1', named: true }], active: 'v1', version: 0 })
      }
      if (cmd === 'get_view') return snap({ view_id: 'v1', version: 0, focused_slot_id: 's1' })
      return undefined
    })
    await initMainWindowFromBackend()
    expect(listTabsCalls).toBeGreaterThanOrEqual(3) // First attempt + 2 or more retries.
    const st = useViewStore.getState()
    expect(st.windows['main'].active).toBe('v1')
    expect(selectView(st, 'v1')?.focusedSlotId).toBe('s1')
  }, 10000)

  // ★ADR-0102: if it still fails after retries are exhausted, propagate the final failure with throw rather than swallowing it★ (the caller eventBus surfaces it via console.error — a never-filled win with no signal is a stuck state of unknown cause).
  it('list_tabs keeps rejecting → throw after the retries are exhausted (final failure propagated, ADR-0102)', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_tabs') throw new Error('backend down')
      return undefined
    })
    await expect(initMainWindowFromBackend()).rejects.toThrow('backend down')
    expect(useViewStore.getState().windows['main']).toBeUndefined()
  }, 10000)

  it('the init pull does not overwrite a newer emit (prevents inversion)', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    // With the subscription already hooked, assume a newer emit (version 5) entered the cache *before* the init pull.
    emit('layout:updated', snap({
      view_id: 'v1',
      version: 5,
      focused_slot_id: 's-new',
      layout: { type: 'slot', id: 's-new', content: { type: 'empty' } },
    }))
    // Then the init's get_view pull (stale version 0), completed late, arrives.
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_tabs') {
        return tabsPayload({ label: 'main', tabs: [{ id: 'v1', name: 'View 1', named: true }], active: 'v1', version: 0 })
      }
      if (cmd === 'get_view') return snap({ view_id: 'v1', version: 0, focused_slot_id: 's-old' })
      return undefined
    })
    await initMainWindowFromBackend()
    expect(useViewStore.getState().layouts['v1'].version).toBe(5)
    expect(selectView(useViewStore.getState(), 'v1')?.focusedSlotId).toBe('s-new')
  })
})

describe('subscribeViewEvents registration/release', () => {
  it('registers 2 listens (layout:updated/window:tabs-updated)', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    expect(listenMock).toHaveBeenCalledWith('layout:updated', expect.any(Function))
    expect(listenMock).toHaveBeenCalledWith('window:tabs-updated', expect.any(Function))
  })

  it('dispose (called after registration completes) releases both unlistens', async () => {
    const { dispose, ready } = subscribeViewEvents()
    await ready
    dispose()
    expect(unlistenMock).toHaveBeenCalledTimes(2)
  })

  it('dispose is idempotent — calling it twice does not call unlisten twice', async () => {
    const { dispose, ready } = subscribeViewEvents()
    await ready
    dispose()
    dispose()
    expect(unlistenMock).toHaveBeenCalledTimes(2)
  })

  // ★Leak guard★: if dispose is called while ready is pending (listen registration unfinished), the late-arriving unlisten handle must be called *immediately* (else a permanent leak). Hold listen as deferred to keep the registration await window open and reproduce the race: dispose in between → then listen resolve.
  it('dispose while ready is pending → calls the late-arriving unlisten handle immediately (leak guard)', async () => {
    const resolveListen = new Map<string, (fn: typeof unlistenMock) => void>()
    listenMock.mockImplementation(
      (event: string, handler: (e: { payload: unknown }) => void) => {
        listeners.set(event, handler)
        return new Promise<typeof unlistenMock>(res => {
          resolveListen.set(event, res)
        })
      },
    )

    const { dispose, ready } = subscribeViewEvents()
    dispose()
    expect(unlistenMock).not.toHaveBeenCalled()

    resolveListen.get('layout:updated')!(unlistenMock)
    resolveListen.get('window:tabs-updated')!(unlistenMock)
    await ready

    expect(unlistenMock).toHaveBeenCalledTimes(2)
  })

  it('ready does not hang even when one listen registration fails, and cleans up the successful one', async () => {
    listenMock.mockImplementation((event: string, handler: (e: { payload: unknown }) => void) => {
      listeners.set(event, handler)
      if (event === 'window:tabs-updated') return Promise.reject(new Error('listen failed'))
      return Promise.resolve(unlistenMock)
    })

    const { dispose, ready } = subscribeViewEvents()
    await expect(ready).rejects.toThrow('listen failed')
    dispose()
    expect(unlistenMock).toHaveBeenCalledTimes(1)
  })

  it('a listen handle that resolves *later* after ready rejected is also released immediately by the caller dispose (0 leaks of late successes)', async () => {
    let resolveLayout: (fn: typeof unlistenMock) => void = () => {}
    let rejectTabs: (err: Error) => void = () => {}
    listenMock.mockImplementation((event: string, handler: (e: { payload: unknown }) => void) => {
      listeners.set(event, handler)
      if (event === 'layout:updated') {
        return new Promise<typeof unlistenMock>(res => {
          resolveLayout = res
        })
      }
      return new Promise<typeof unlistenMock>((_res, rej) => {
        rejectTabs = rej
      })
    })

    const { dispose, ready } = subscribeViewEvents()
    rejectTabs(new Error('listen failed'))
    await expect(ready).rejects.toThrow('listen failed')
    expect(unlistenMock).not.toHaveBeenCalled()

    dispose()
    expect(unlistenMock).not.toHaveBeenCalled()

    resolveLayout(unlistenMock)
    await Promise.resolve()
    expect(unlistenMock).toHaveBeenCalledTimes(1)
  })
})

// ★Render mode override (§5, frontend-only)★: set/clear + slot lifecycle cleanup (FIX-1) + untyped entry guard (FIX-4).
describe('renderModeOverride override + lifecycle cleanup (§5)', () => {
  beforeEach(() => {
    useViewStore.setState({ renderModeOverride: {} })
  })

  it('closeSlot clears that slot\'s override (slot gone → prevents an entry leak, FIX-1)', async () => {
    useViewStore.getState().setRenderMode('s-close', 'dom')
    expect(useViewStore.getState().renderModeOverride['s-close']).toBe('dom')
    await useViewStore.getState().closeSlot('v1', 's-close')
    expect(useViewStore.getState().renderModeOverride['s-close']).toBeUndefined()
    expect(invokeMock).toHaveBeenCalledWith('close_slot', { viewId: 'v1', slotId: 's-close' })
  })

  it("assignAgent clears that slot's override (so the previous agent's override does not leak into the new agent, FIX-1)", async () => {
    useViewStore.getState().setRenderMode('s-assign', 'rich')
    expect(useViewStore.getState().renderModeOverride['s-assign']).toBe('rich')
    await useViewStore.getState().assignAgent('v1', 's-assign', 'agent-new')
    expect(useViewStore.getState().renderModeOverride['s-assign']).toBeUndefined()
    expect(invokeMock).toHaveBeenCalledWith('assign_agent', {
      viewId: 'v1',
      slotId: 's-assign',
      agentId: 'agent-new',
    })
  })

  it("setSlotContent clears that slot's override (whole content replaced → prevents a leak, ADR-0063)", async () => {
    useViewStore.getState().setRenderMode('s-set', 'dom')
    expect(useViewStore.getState().renderModeOverride['s-set']).toBe('dom')
    await useViewStore.getState().setSlotContent('v1', 's-set', { type: 'agent_list' })
    expect(useViewStore.getState().renderModeOverride['s-set']).toBeUndefined()
    expect(invokeMock).toHaveBeenCalledWith('set_slot_content', {
      viewId: 'v1',
      slotId: 's-set',
      content: { type: 'agent_list' },
    })
  })

  it("moveSlotToWindow clears that slot's override (source slot gone → prevents a leak)", async () => {
    useViewStore.getState().setRenderMode('s-move', 'dom')
    invokeMock.mockResolvedValueOnce({ window: 'slot-popup-1', tab: 'v-new' })
    await useViewStore.getState().moveSlotToWindow('v1', 's-move')
    expect(useViewStore.getState().renderModeOverride['s-move']).toBeUndefined()
  })

  it("closeSlot does not touch other slots' overrides", async () => {
    useViewStore.getState().setRenderMode('s-keep', 'dom')
    useViewStore.getState().setRenderMode('s-gone', 'rich')
    await useViewStore.getState().closeSlot('v1', 's-gone')
    expect(useViewStore.getState().renderModeOverride['s-gone']).toBeUndefined()
    expect(useViewStore.getState().renderModeOverride['s-keep']).toBe('dom')
  })

  it('setRenderMode (invalid mode) → no-op (store unchanged) + console.warn (FIX-4)', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    ;(useViewStore.getState().setRenderMode as (n: string, m: unknown) => void)('s1', 'bogus')
    expect(useViewStore.getState().renderModeOverride['s1']).toBeUndefined()
    expect(warn).toHaveBeenCalled()
    warn.mockRestore()
  })

  it('setRenderMode (valid mode) → recorded normally', () => {
    useViewStore.getState().setRenderMode('s1', 'dom')
    expect(useViewStore.getState().renderModeOverride['s1']).toBe('dom')
  })
})

// ★Window decision (§3-3/§3-4, G7)★: readWindowLabelFromHash · currentViewId decide which window this webview is and its active tab from the URL + windows state. Covers a popup (?window=) · main · an unknown route (main fallback).
describe('readWindowLabelFromHash + currentViewId (window context resolution, ADR-0057)', () => {
  const origHash2 = window.location.hash
  afterEach(() => {
    window.location.hash = origHash2
  })

  it('readWindowLabelFromHash: parses the label from the popup hash (#/popup?window=<label>)', () => {
    window.location.hash = '#/popup?window=slot-popup-3'
    expect(readWindowLabelFromHash()).toBe('slot-popup-3')
  })

  it('readWindowLabelFromHash: main for the main hash (#/)', () => {
    window.location.hash = '#/'
    expect(readWindowLabelFromHash()).toBe(MAIN_WINDOW_LABEL)
  })

  it('readWindowLabelFromHash: falls back to main for an unknown route (#/unknown)', () => {
    window.location.hash = '#/unknown'
    expect(readWindowLabelFromHash()).toBe(MAIN_WINDOW_LABEL)
  })

  it('readWindowLabelFromHash: ignores a stray ?window= on the main route (#/?window=x) and gives main', () => {
    window.location.hash = '#/?window=x'
    expect(readWindowLabelFromHash()).toBe(MAIN_WINDOW_LABEL)
  })

  it('currentViewId: windows["main"].active for the main window', () => {
    window.location.hash = '#/'
    useViewStore.setState({
      windows: { main: { windowId: 'main', number: 1, tabs: [{ id: 'v9', name: 'V9', named: true }], active: 'v9', version: 1 } },
    })
    expect(currentViewId()).toBe('v9')
  })

  it("currentViewId: that label's active for a popup window", () => {
    window.location.hash = '#/popup?window=slot-popup-1'
    useViewStore.setState({
      windows: {
        main: { windowId: 'main', number: 1, tabs: [{ id: 'vм', name: 'main', named: true }], active: 'vм', version: 1 },
        'slot-popup-1': {
          windowId: 'w-popup',
          number: 2,
          tabs: [{ id: 'p1', name: 'p1', named: true }],
          active: 'p1',
          version: 1,
        },
      },
    })
    expect(currentViewId()).toBe('p1')
  })

  it('currentViewId: falls back to windows["main"].active for an unknown route (#/unknown)', () => {
    window.location.hash = '#/unknown'
    useViewStore.setState({
      windows: {
        main: { windowId: 'main', number: 1, tabs: [{ id: 'vmain', name: 'main', named: true }], active: 'vmain', version: 1 },
      },
    })
    expect(currentViewId()).toBe('vmain')
  })

  it('currentViewId: null when the window state has not arrived', () => {
    window.location.hash = '#/'
    useViewStore.setState({ windows: {} })
    expect(currentViewId()).toBeNull()
  })
})
