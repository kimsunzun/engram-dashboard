// viewStore unit test — emit↔invoke loop (ADR-0035/0057 vertical slice, tab ownership model).
//
// Mocks invoke('@tauri-apps/api/core') + listen('@tauri-apps/api/event') to verify that actions call the correct invoke
// + that state updates on receiving the backend emit (layout:updated/window:tabs-updated) + that the version guard discards
// stale emits + ★per-window active tab decision (useCurrentViewId family)★. Pure logic only, with no real Tauri.
//
// ★Invariant verified (tab ownership model, ADR-0057)★: the old global activeViewId is gone. The window (label)
// owns the tab list, and viewStore.windows[label].active is that window's active tab. layout is a per-view_id cache (global monotonic
// version guard). window:tabs-updated also discards stale emits with a per-window version guard (G10).

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
  if (!h) throw new Error(`no listener for ${event} — subscribeViewEvents 호출했나?`)
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

describe('viewStore 탭/창 액션 → invoke (탭 소유 모델, ADR-0057)', () => {
  it('createTab → create_tab invoke(window,name) + 반환 id 전달', async () => {
    invokeMock.mockResolvedValueOnce('new-view-id')
    const id = await useViewStore.getState().createTab('main', 'My Tab')
    expect(invokeMock).toHaveBeenCalledWith('create_tab', { window: 'main', name: 'My Tab' })
    expect(id).toBe('new-view-id')
  })

  it('createTab(no name) → name=null', async () => {
    await useViewStore.getState().createTab('main')
    expect(invokeMock).toHaveBeenCalledWith('create_tab', { window: 'main', name: null })
  })

  it('closeTab/switchTab → 대응 invoke(window, view)', async () => {
    const s = useViewStore.getState()
    await s.closeTab('slot-popup-1', 'v9')
    expect(invokeMock).toHaveBeenCalledWith('close_tab', { window: 'slot-popup-1', view: 'v9' })
    await s.switchTab('main', 'v2')
    expect(invokeMock).toHaveBeenCalledWith('switch_tab', { window: 'main', view: 'v2' })
  })

  it('switchTabIf → switch_tab_if invoke(window, view, expect) · 셸의 bool 을 돌려준다', async () => {
    invokeMock.mockResolvedValueOnce(false)
    await expect(useViewStore.getState().switchTabIf('main', 'v1', 'v2')).resolves.toBe(false)
    expect(invokeMock).toHaveBeenCalledWith('switch_tab_if', { window: 'main', view: 'v1', expect: 'v2' })
  })

  it('renameTab → rename_tab invoke({ viewId, name })', async () => {
    await useViewStore.getState().renameTab('v1', 'New Name')
    expect(invokeMock).toHaveBeenCalledWith('rename_tab', { viewId: 'v1', name: 'New Name' })
  })

  it('createWindow → create_window invoke · {window, window_id, number} 를 그대로 돌려준다', async () => {
    const created = { window: 'slot-popup-3', window_id: '7d2f0b8e-1c3a-4f5e-9b6d-2a4c6e8f0a1b', number: 2 }
    invokeMock.mockResolvedValueOnce(created)
    const answer = await useViewStore.getState().createWindow()
    expect(invokeMock).toHaveBeenCalledWith('create_window')
    expect(answer).toEqual(created)
  })

  it('controlWindow → window_control invoke 하나(window · action · ifEmpty) · 셸 답을 그대로 돌려준다', async () => {
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

  it('split → split_slot invoke(viewId/slotId/dir) + 새 slot id 반환', async () => {
    invokeMock.mockResolvedValueOnce('new-slot-id')
    const id = await useViewStore.getState().split('v1', 's1', 'top_bottom')
    expect(invokeMock).toHaveBeenCalledWith('split_slot', {
      viewId: 'v1',
      slotId: 's1',
      dir: 'top_bottom',
    })
    expect(id).toBe('new-slot-id')
  })

  it('setSplitRatio → set_split_ratio invoke(viewId/splitId/ratio) + 셸 답 그대로 반환', async () => {
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

  it('setSplitRatio 실패 → 셸 오류 문자열로 reject', async () => {
    invokeMock.mockRejectedValueOnce('split not found')
    await expect(useViewStore.getState().setSplitRatio('v1', 'gone', 0.5)).rejects.toBe('split not found')
  })

  it('closeSlot/assignAgent → 대응 invoke 인자', async () => {
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

  it('setSlotContent → set_slot_content invoke(viewId/slotId/content) (ADR-0063 제네릭 배치)', async () => {
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

  it('setUsageSlot → set_usage_slot invoke — 준 칸만 값, 빠진 칸은 null(셸이 지금 값으로 지킨다) · 오버라이드 clear', async () => {
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

  it('moveSlotToWindow → move_slot_to_window invoke(viewId/slotId/toWindow) + {window,tab,window_id,number} 반환', async () => {
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

  it('moveSlotToWindow(toWindow 지정) → 그 label 을 넘긴다(기존 창 타깃)', async () => {
    invokeMock.mockResolvedValueOnce({ window: 'main', tab: 'v-new', window_id: 'main', number: 1 })
    await useViewStore.getState().moveSlotToWindow('v1', 's1', 'main')
    expect(invokeMock).toHaveBeenCalledWith('move_slot_to_window', {
      viewId: 'v1',
      slotId: 's1',
      toWindow: 'main',
    })
  })
})

describe('viewStore emit 수신 → 상태 갱신', () => {
  it('layout:updated → 그 view 캐시 항목(layout/focus/version) 채택', async () => {
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

  it('layout:updated → 셸 기하(slot_rects/split_rects)와 비율 한계를 캐시 항목에 그대로 담는다(ADR-0227)', async () => {
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
  it('layout:updated → foreign_slots 를 캐시 항목에 싣고, 다음 스냅샷에서 빠지면 함께 빠진다', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    emit('layout:updated', snap({ view_id: 'v1', version: 2, foreign_slots: ['s1'] }))
    expect(useViewStore.getState().layouts['v1'].foreignSlots).toEqual(['s1'])

    emit('layout:updated', snap({ view_id: 'v1', version: 3, foreign_slots: [] }))
    expect(useViewStore.getState().layouts['v1'].foreignSlots).toEqual([])
  })

  it('window:tabs-updated → windows[label].{windowId,number,tabs,active,version} 갱신', async () => {
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

  it('window:tabs-updated 는 창별 독립 — 다른 label 은 서로 안 건드린다', async () => {
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

describe('version 가드(전역 단조 version, G10)', () => {
  it('layout:updated — 같은 view 의 낮은 version emit 은 폐기(순서 역전 방지)', async () => {
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

  it('window:tabs-updated — 같은 창의 낮은/같은 version emit 은 폐기(stale 방어, G10)', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    emit('window:tabs-updated', tabsPayload({ label: 'main', active: 'v5', version: 5 }))
    emit('window:tabs-updated', tabsPayload({ label: 'main', active: 'v3', version: 3 }))
    emit('window:tabs-updated', tabsPayload({ label: 'main', active: 'vX', version: 5 }))
    expect(useViewStore.getState().windows['main'].active).toBe('v5')
  })

  it('layout:updated 첫 emit 은 캐시 항목이 없어 항상 채택(version 0 포함)', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    emit('layout:updated', snap({ view_id: 'v1', version: 0 }))
    expect(useViewStore.getState().layouts['v1'].version).toBe(0)
  })
})

describe('selectView (창 캔버스 렌더 selector)', () => {
  it('캐시된 view_id → 그 항목, 없으면 null', () => {
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

describe('initMainWindowFromBackend 부팅 init(read-only pull)', () => {
  it('list_tabs("main") → get_view 순서로 invoke 하고 창 탭+active 레이아웃 캐시를 채운다', async () => {
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

  // ★ADR-0102: even if list_tabs fails a few times with early transients, retry recovers it and fills the window state★.
  //   main has no event recovery path (window:tabs-updated fires only on tab mutations), so this pull must not be one-shot.
  it('list_tabs 가 2번 reject 후 resolve → 재시도로 회수해 windows["main"] 이 채워진다(ADR-0102)', async () => {
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

  // ★ADR-0102: if it still fails after retries are exhausted, do not swallow it silently but propagate the final failure with throw★
  //   (the caller eventBus surfaces it via console.error — if win is never filled and there is no signal, it is a stuck state with unknown cause).
  it('list_tabs 가 계속 reject → 재시도 소진 후 throw(최종 실패 전파, ADR-0102)', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_tabs') throw new Error('backend down')
      return undefined
    })
    await expect(initMainWindowFromBackend()).rejects.toThrow('backend down')
    expect(useViewStore.getState().windows['main']).toBeUndefined()
  }, 10000)

  it('init pull 이 더 최신 emit 을 덮지 않는다(역전 방지)', async () => {
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

describe('subscribeViewEvents 등록/해제', () => {
  it('listen 2종(layout:updated/window:tabs-updated) 등록', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    expect(listenMock).toHaveBeenCalledWith('layout:updated', expect.any(Function))
    expect(listenMock).toHaveBeenCalledWith('window:tabs-updated', expect.any(Function))
  })

  it('dispose(등록 완료 후 호출) 시 두 unlisten 모두 해제', async () => {
    const { dispose, ready } = subscribeViewEvents()
    await ready
    dispose()
    expect(unlistenMock).toHaveBeenCalledTimes(2)
  })

  it('dispose 는 idempotent — 두 번 불러도 unlisten 을 중복 호출하지 않는다', async () => {
    const { dispose, ready } = subscribeViewEvents()
    await ready
    dispose()
    dispose()
    expect(unlistenMock).toHaveBeenCalledTimes(2)
  })

  // ★Leak guard★: if dispose is called while ready is pending (listen registration unfinished), the
  // unlisten handle that arrives when registration finishes late must be called *immediately* (if not, a permanent leak). Hold listen as deferred to
  // keep the registration await window open, and reproduce the real race in the order dispose in between → then listen resolve.
  it('ready pending 중 dispose → 늦게 도착한 unlisten 핸들을 즉시 호출(누수 가드)', async () => {
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

  it('ready 는 한쪽 listen 등록이 실패해도 hang 하지 않고 성공분을 정리한다', async () => {
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

  it('ready reject 후 *나중에* resolve 된 listen 핸들도 호출자 dispose 가 즉시 해제(늦은 성공분 누수 0)', async () => {
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
describe('renderModeOverride 오버라이드 + 생명주기 정리(§5)', () => {
  beforeEach(() => {
    useViewStore.setState({ renderModeOverride: {} })
  })

  it('closeSlot 은 그 slot 의 오버라이드를 clear 한다(slot 소멸 → 엔트리 누수 방지, FIX-1)', async () => {
    useViewStore.getState().setRenderMode('s-close', 'dom')
    expect(useViewStore.getState().renderModeOverride['s-close']).toBe('dom')
    await useViewStore.getState().closeSlot('v1', 's-close')
    expect(useViewStore.getState().renderModeOverride['s-close']).toBeUndefined()
    expect(invokeMock).toHaveBeenCalledWith('close_slot', { viewId: 'v1', slotId: 's-close' })
  })

  it('assignAgent 은 그 slot 의 오버라이드를 clear 한다(이전 agent 오버라이드가 새 agent 에 새지 않게, FIX-1)', async () => {
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

  it('setSlotContent 은 그 slot 의 오버라이드를 clear 한다(콘텐츠 통째 교체 → 누수 방지, ADR-0063)', async () => {
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

  it('moveSlotToWindow 는 그 slot 의 오버라이드를 clear 한다(원본 슬롯 소멸 → 누수 방지)', async () => {
    useViewStore.getState().setRenderMode('s-move', 'dom')
    invokeMock.mockResolvedValueOnce({ window: 'slot-popup-1', tab: 'v-new' })
    await useViewStore.getState().moveSlotToWindow('v1', 's-move')
    expect(useViewStore.getState().renderModeOverride['s-move']).toBeUndefined()
  })

  it('closeSlot 은 다른 slot 의 오버라이드는 건드리지 않는다', async () => {
    useViewStore.getState().setRenderMode('s-keep', 'dom')
    useViewStore.getState().setRenderMode('s-gone', 'rich')
    await useViewStore.getState().closeSlot('v1', 's-gone')
    expect(useViewStore.getState().renderModeOverride['s-gone']).toBeUndefined()
    expect(useViewStore.getState().renderModeOverride['s-keep']).toBe('dom')
  })

  it('setRenderMode(무효 mode) → no-op(store 불변) + console.warn(FIX-4)', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    ;(useViewStore.getState().setRenderMode as (n: string, m: unknown) => void)('s1', 'bogus')
    expect(useViewStore.getState().renderModeOverride['s1']).toBeUndefined()
    expect(warn).toHaveBeenCalled()
    warn.mockRestore()
  })

  it('setRenderMode(유효 mode) → 정상 기록', () => {
    useViewStore.getState().setRenderMode('s1', 'dom')
    expect(useViewStore.getState().renderModeOverride['s1']).toBe('dom')
  })
})

// ★Window decision (§3-3/§3-4, G7)★: readWindowLabelFromHash · currentViewId decide which window this webview is and what that window's active
// tab is from the URL + windows state. Covers a popup (?window=) · main · an unknown route (main fallback).
describe('readWindowLabelFromHash + currentViewId (창 컨텍스트 해소, ADR-0057)', () => {
  const origHash2 = window.location.hash
  afterEach(() => {
    window.location.hash = origHash2
  })

  it('readWindowLabelFromHash: 팝업 hash(#/popup?window=<label>)에서 label 파싱', () => {
    window.location.hash = '#/popup?window=slot-popup-3'
    expect(readWindowLabelFromHash()).toBe('slot-popup-3')
  })

  it('readWindowLabelFromHash: 메인 hash(#/)면 main', () => {
    window.location.hash = '#/'
    expect(readWindowLabelFromHash()).toBe(MAIN_WINDOW_LABEL)
  })

  it('readWindowLabelFromHash: 모르는 라우트(#/unknown)면 main 폴백', () => {
    window.location.hash = '#/unknown'
    expect(readWindowLabelFromHash()).toBe(MAIN_WINDOW_LABEL)
  })

  it('readWindowLabelFromHash: 메인 라우트의 stray ?window=(#/?window=x)는 무시하고 main', () => {
    window.location.hash = '#/?window=x'
    expect(readWindowLabelFromHash()).toBe(MAIN_WINDOW_LABEL)
  })

  it('currentViewId: main 창이면 windows["main"].active', () => {
    window.location.hash = '#/'
    useViewStore.setState({
      windows: { main: { windowId: 'main', number: 1, tabs: [{ id: 'v9', name: 'V9', named: true }], active: 'v9', version: 1 } },
    })
    expect(currentViewId()).toBe('v9')
  })

  it('currentViewId: 팝업 창이면 그 label 의 active', () => {
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

  it('currentViewId: 모르는 라우트(#/unknown)면 windows["main"].active 폴백', () => {
    window.location.hash = '#/unknown'
    useViewStore.setState({
      windows: {
        main: { windowId: 'main', number: 1, tabs: [{ id: 'vmain', name: 'main', named: true }], active: 'vmain', version: 1 },
      },
    })
    expect(currentViewId()).toBe('vmain')
  })

  it('currentViewId: 창 상태 미도착이면 null', () => {
    window.location.hash = '#/'
    useViewStore.setState({ windows: {} })
    expect(currentViewId()).toBeNull()
  })
})
