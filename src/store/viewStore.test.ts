// viewStore 단위테스트 — emit↔invoke 루프(ADR-0035/0057 수직 슬라이스, 탭 소유 모델).
//
// invoke('@tauri-apps/api/core') + listen('@tauri-apps/api/event') 를 mock 해, 액션이 올바른 invoke 를
// 부르는지 + 백엔드 emit(layout:updated/window:tabs-updated)을 받아 상태가 갱신되는지 + version 가드가
// stale emit 을 폐기하는지 + ★창별 active 탭 판정(useCurrentViewId 계열)★을 검증한다. 실제 Tauri 없이 순수 로직만.
//
// ★검증하는 불변식(탭 소유 모델, ADR-0057)★: 옛 전역 activeViewId 는 사라졌다. 창(label)이 탭 목록을
// 소유하고, viewStore.windows[label].active 가 그 창의 활성 탭이다. layout 은 view_id 별 캐시(전역 단조
// version 가드). window:tabs-updated 도 창별 version 가드로 stale emit 을 폐기한다(G10).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { ViewSnapshot } from '../api/layoutTypes'

const invokeMock = vi.fn(async (_cmd: string, ..._rest: unknown[]) => undefined as unknown)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, ...rest: unknown[]) => invokeMock(cmd, ...rest),
}))

// listen mock: 등록된 핸들러를 이벤트명별로 보관해, 테스트가 직접 emit 을 흉내낸다(emit helper).
const listeners = new Map<string, (e: { payload: unknown }) => void>()
const unlistenMock = vi.fn()
const listenMock = vi.fn(async (event: string, handler: (e: { payload: unknown }) => void) => {
  listeners.set(event, handler)
  return unlistenMock
})
vi.mock('@tauri-apps/api/event', () => ({
  listen: (event: string, handler: (e: { payload: unknown }) => void) => listenMock(event, handler),
}))

// getCurrentWindow mock — currentViewId/useCurrentViewId 는 URL 로 창을 판정하므로 이 mock 은 미사용이나,
// viewStore 가 re-export 하는 getCurrentWindow(팝업 자가닫힘용)의 import 해소를 위해 stub.
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ close: vi.fn(async () => undefined), label: () => 'main' }),
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

/** 백엔드 emit 흉내 — subscribeViewEvents 가 등록한 핸들러로 payload 를 흘려보낸다. */
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
    slot_spatial: [], // ADR-0068: 공간 파생(이 테스트는 안 씀 — 빈 배열로 타입 충족)
    version: 1,
    ...overrides,
  }
}

function tabsPayload(overrides: Partial<WindowTabsPayload> = {}): WindowTabsPayload {
  return {
    label: 'main',
    tabs: [{ id: 'v1', name: 'View 1' }],
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
  // 스토어 초기화(테스트 격리).
  useViewStore.setState({ layouts: {}, windows: {}, renderModeOverride: {} })
  window.location.hash = '#/' // 기본 = main 창
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

  it('renameTab → rename_tab invoke({ viewId, name })', async () => {
    await useViewStore.getState().renameTab('v1', 'New Name')
    expect(invokeMock).toHaveBeenCalledWith('rename_tab', { viewId: 'v1', name: 'New Name' })
  })

  it('createWindow/closeWindow → 대응 invoke', async () => {
    invokeMock.mockResolvedValueOnce('slot-popup-3')
    const label = await useViewStore.getState().createWindow()
    expect(invokeMock).toHaveBeenCalledWith('create_window')
    expect(label).toBe('slot-popup-3')
    await useViewStore.getState().closeWindow('slot-popup-3')
    expect(invokeMock).toHaveBeenCalledWith('close_window', { window: 'slot-popup-3' })
  })

  it('split → split_slot invoke(viewId/slotId/dir) + 새 slot id 반환', async () => {
    invokeMock.mockResolvedValueOnce('new-slot-id')
    const id = await useViewStore.getState().split('v1', 's1', 'horizontal')
    expect(invokeMock).toHaveBeenCalledWith('split_slot', {
      viewId: 'v1',
      slotId: 's1',
      dir: 'horizontal',
    })
    expect(id).toBe('new-slot-id')
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

  it('moveSlotToWindow → move_slot_to_window invoke(viewId/slotId/toWindow) + {window,tab} 반환', async () => {
    invokeMock.mockResolvedValueOnce({ window: 'slot-popup-2', tab: 'v-new' })
    const res = await useViewStore.getState().moveSlotToWindow('v1', 's1')
    // toWindow 미지정 → null(새 팝업 창).
    expect(invokeMock).toHaveBeenCalledWith('move_slot_to_window', {
      viewId: 'v1',
      slotId: 's1',
      toWindow: null,
    })
    expect(res).toEqual({ window: 'slot-popup-2', tab: 'v-new' })
  })

  it('moveSlotToWindow(toWindow 지정) → 그 label 을 넘긴다(기존 창 타깃)', async () => {
    invokeMock.mockResolvedValueOnce({ window: 'main', tab: 'v-new' })
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

  it('window:tabs-updated → windows[label].{tabs,active,version} 갱신', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    emit('window:tabs-updated', tabsPayload({
      label: 'main',
      tabs: [{ id: 'v1', name: 'View 1' }, { id: 'v2', name: 'View 2' }],
      active: 'v2',
      version: 4,
    }))
    const win = useViewStore.getState().windows['main']
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
      tabs: [{ id: 'p1', name: 'Popup Tab' }],
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
    // 늦게 도착한 과거 emit(version 3) — 같은 view 캐시 version(5) 이하라 폐기돼야 한다.
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
    // 늦게 도착한 과거(version 3) — 폐기.
    emit('window:tabs-updated', tabsPayload({ label: 'main', active: 'v3', version: 3 }))
    // 같은 version(5) 도 폐기(<=).
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

/** layout 캐시에 직접 seed(emit 경로 없이) — selectView 단위테스트 헬퍼. */
function emit_seed_layout(viewId: string, over: Partial<ViewSnapshot>): void {
  useViewStore.getState().applyLayoutUpdated(snap({ view_id: viewId, ...over }))
}

describe('initMainWindowFromBackend 부팅 init(read-only pull)', () => {
  it('list_tabs("main") → get_view 순서로 invoke 하고 창 탭+active 레이아웃 캐시를 채운다', async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_tabs') {
        expect(args).toEqual({ window: 'main' })
        return tabsPayload({ label: 'main', tabs: [{ id: 'v1', name: 'View 1' }], active: 'v1', version: 0 })
      }
      if (cmd === 'get_view') return snap({ view_id: 'v1', version: 0, focused_slot_id: 's1' })
      return undefined
    })
    await initMainWindowFromBackend()
    expect(invokeMock).toHaveBeenCalledWith('list_tabs', { window: 'main' })
    expect(invokeMock).toHaveBeenCalledWith('get_view', { viewId: 'v1' })
    const st = useViewStore.getState()
    expect(st.windows['main'].active).toBe('v1')
    // active 뷰 레이아웃이 캐시에 들어가 렌더 대상이 된다(부팅 즉시 렌더 조건).
    expect(selectView(st, 'v1')?.focusedSlotId).toBe('s1')
  })

  it('init pull 이 더 최신 emit 을 덮지 않는다(역전 방지)', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    // 구독이 먼저 걸린 상태에서, init pull 보다 *먼저* 더 최신 emit(version 5)이 캐시에 들어왔다고 가정.
    emit('layout:updated', snap({
      view_id: 'v1',
      version: 5,
      focused_slot_id: 's-new',
      layout: { type: 'slot', id: 's-new', content: { type: 'empty' } },
    }))
    // 그 뒤 늦게 완료된 init 의 get_view pull(낡은 version 0)이 도착 — 캐시 version(5) 이하라 폐기돼야 한다.
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_tabs') {
        return tabsPayload({ label: 'main', tabs: [{ id: 'v1', name: 'View 1' }], active: 'v1', version: 0 })
      }
      if (cmd === 'get_view') return snap({ view_id: 'v1', version: 0, focused_slot_id: 's-old' })
      return undefined
    })
    await initMainWindowFromBackend()
    // 옛 pull 이 새 emit 을 덮지 않음 — 최신(version 5) 유지.
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

  // ★누수 가드★: ready 가 pending(listen 등록 미완) 인 동안 dispose 가 불리면, 뒤늦게 등록이 끝나 도착한
  // unlisten 핸들을 *즉시* 호출해야 한다(안 부르면 영구 누수). listen 을 deferred 로 잡아 등록 await 윈도를
  // 열어두고, 그 사이 dispose → 이후 listen resolve 순으로 실제 race 를 재현한다.
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

// ★렌더 모드 오버라이드(§5, 프론트 전용)★: set/clear + slot 생명주기 정리(FIX-1) + 미타입 진입 가드(FIX-4).
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

// ★창 판정(§3-3/§3-4, G7)★: readWindowLabelFromHash·currentViewId 가 이 웹뷰가 어느 창인지, 그 창의 active
// 탭이 무엇인지 URL + windows 상태로 판정한다. 팝업(?window=)·main·agent-tree(main 폴백)을 커버.
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

  it('readWindowLabelFromHash: agent-tree hash(#/tree)면 main 폴백(모델 밖 config 창 특례, §3-4)', () => {
    window.location.hash = '#/tree'
    expect(readWindowLabelFromHash()).toBe(MAIN_WINDOW_LABEL)
  })

  it('readWindowLabelFromHash: 메인 라우트의 stray ?window=(#/?window=x)는 무시하고 main', () => {
    window.location.hash = '#/?window=x'
    expect(readWindowLabelFromHash()).toBe(MAIN_WINDOW_LABEL)
  })

  it('currentViewId: main 창이면 windows["main"].active', () => {
    window.location.hash = '#/'
    useViewStore.setState({ windows: { main: { tabs: [{ id: 'v9', name: 'V9' }], active: 'v9', version: 1 } } })
    expect(currentViewId()).toBe('v9')
  })

  it('currentViewId: 팝업 창이면 그 label 의 active', () => {
    window.location.hash = '#/popup?window=slot-popup-1'
    useViewStore.setState({
      windows: {
        main: { tabs: [{ id: 'vм', name: 'main' }], active: 'vм', version: 1 },
        'slot-popup-1': { tabs: [{ id: 'p1', name: 'p1' }], active: 'p1', version: 1 },
      },
    })
    // 팝업 hash 면 main 이 아니라 자기 창 active.
    expect(currentViewId()).toBe('p1')
  })

  it('currentViewId: agent-tree(#/tree)면 windows["main"].active 폴백(§3-4/G7)', () => {
    window.location.hash = '#/tree'
    useViewStore.setState({ windows: { main: { tabs: [{ id: 'vmain', name: 'main' }], active: 'vmain', version: 1 } } })
    expect(currentViewId()).toBe('vmain')
  })

  it('currentViewId: 창 상태 미도착이면 null', () => {
    window.location.hash = '#/'
    useViewStore.setState({ windows: {} })
    expect(currentViewId()).toBeNull()
  })
})
