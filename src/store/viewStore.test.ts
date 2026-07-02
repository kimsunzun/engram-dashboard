// viewStore 단위테스트 — emit↔invoke 루프(ADR-0035 수직 슬라이스).
//
// invoke('@tauri-apps/api/core') + listen('@tauri-apps/api/event') 를 mock 해, 액션이 올바른 invoke 를
// 부르는지 + 백엔드 emit(layout:updated/view:list-updated)을 받아 상태가 갱신되는지 + version 가드가
// stale emit 을 폐기하는지 + ★메인 렌더는 active view 캐시 항목만 본다★(active-only, F4)를 검증한다.
// 실제 Tauri 없이 순수 로직만.
//
// ★검증하는 불변식(F1+F4 수정 핵심)★: 백엔드 ViewManager.version 은 *전역 단조 카운터*(모든 view
// 공유). viewStore 는 layout 을 view_id 별 캐시로 보유하고, 메인 창은 activeViewId 의 캐시 항목만
// 렌더한다(selectActiveView). 그래서 (1) 같은 view 의 낮은 전역 version emit 은 폐기되고, (2) 비-active
// view 의 emit 은 캐시엔 들어가도 메인 렌더 대상이 아니며, (3) switch_view 로 active 가 바뀌면 *이미
// 캐시된* 그 view layout 이 렌더된다. (옛 "view_id 가 다르면 version 무관 무조건 채택" 단언은 틀린
// 불변식이라 삭제 — 전역 단조 version 전제에서 그 분기는 stale 덮어쓰기/비-active 가로채기를 유발했다.)

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

import { selectActiveView, subscribeViewEvents, useViewStore } from './viewStore'

/** 백엔드 emit 흉내 — subscribeViewEvents 가 등록한 핸들러로 payload 를 흘려보낸다. */
function emit(event: string, payload: unknown): void {
  const h = listeners.get(event)
  if (!h) throw new Error(`no listener for ${event} — subscribeViewEvents 호출했나?`)
  h({ payload })
}

function snap(overrides: Partial<ViewSnapshot> = {}): ViewSnapshot {
  return {
    view_id: 'v1',
    layout: { type: 'slot', id: 's1', agent_id: null },
    focused_slot_id: 's1',
    version: 1,
    ...overrides,
  }
}

/** 메인 창이 실제로 렌더하는 것 = active view 캐시 항목(active-only). 단언 헬퍼. */
function rendered() {
  return selectActiveView(useViewStore.getState())
}

beforeEach(() => {
  invokeMock.mockClear()
  invokeMock.mockImplementation(async () => undefined)
  listeners.clear()
  unlistenMock.mockClear()
  listenMock.mockClear()
  // 스토어 초기화(테스트 격리).
  useViewStore.setState({
    layouts: {},
    views: [],
    activeViewId: null,
    richSlots: {},
  })
})
afterEach(() => {
  vi.restoreAllMocks()
})

describe('viewStore 액션 → invoke (레이아웃 권위 = src-tauri, ADR-0035)', () => {
  it('createView → create_view invoke(name) + 반환 id 전달', async () => {
    invokeMock.mockResolvedValueOnce('new-view-id')
    const id = await useViewStore.getState().createView('My View')
    expect(invokeMock).toHaveBeenCalledWith('create_view', { name: 'My View' })
    expect(id).toBe('new-view-id')
  })

  it('createView(no name) → name=null', async () => {
    await useViewStore.getState().createView()
    expect(invokeMock).toHaveBeenCalledWith('create_view', { name: null })
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

  it('closeView/switchView/closeSlot/assignAgent → 대응 invoke 인자', async () => {
    const s = useViewStore.getState()
    await s.closeView('v1')
    expect(invokeMock).toHaveBeenCalledWith('close_view', { viewId: 'v1' })
    await s.switchView('v2')
    expect(invokeMock).toHaveBeenCalledWith('switch_view', { viewId: 'v2' })
    await s.closeSlot('v1', 's2')
    expect(invokeMock).toHaveBeenCalledWith('close_slot', { viewId: 'v1', slotId: 's2' })
    await s.assignAgent('v1', 's1', 'agent-9')
    expect(invokeMock).toHaveBeenCalledWith('assign_agent', {
      viewId: 'v1',
      slotId: 's1',
      agentId: 'agent-9',
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
    expect(cached.layout).toEqual({ type: 'slot', id: 's1', agent_id: null })
  })

  it('view:list-updated → views/activeViewId 갱신', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    emit('view:list-updated', {
      views: [{ id: 'v1', name: 'View 1' }, { id: 'v2', name: 'View 2' }],
      active_view_id: 'v2',
    })
    const st = useViewStore.getState()
    expect(st.views).toHaveLength(2)
    expect(st.activeViewId).toBe('v2')
  })

  it('split 후 emit 으로 split 트리가 active 렌더에 반영된다(end-to-end 루프 핵심)', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    // active 를 v1 으로 — 메인 렌더 대상이 되게.
    emit('view:list-updated', { views: [{ id: 'v1', name: 'View 1' }], active_view_id: 'v1' })
    // 백엔드가 split 후 보내는 layout:updated 를 흉내 — slot → split(a,b) 로 바뀐다.
    invokeMock.mockResolvedValueOnce('s2')
    await useViewStore.getState().split('v1', 's1', 'horizontal')
    emit('layout:updated', snap({
      view_id: 'v1',
      version: 2,
      layout: {
        type: 'split',
        dir: 'horizontal',
        ratio: 0.5,
        a: { type: 'slot', id: 's1', agent_id: null },
        b: { type: 'slot', id: 's2', agent_id: null },
      },
    }))
    // 메인 렌더 = active(v1) 캐시 항목. split 트리가 그려진다.
    const lyt = rendered()?.layout
    expect(lyt?.type).toBe('split')
    if (lyt?.type === 'split') {
      expect(lyt.a.type).toBe('slot')
      expect(lyt.b.type).toBe('slot')
    }
  })
})

describe('version 가드 + active-only 렌더(전역 단조 version, F1+F4)', () => {
  it('같은 view 의 낮은 (전역)version emit 은 폐기(순서 역전 방지)', async () => {
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

  it('비-active view 의 emit 은 캐시엔 들어가도 메인 렌더에는 안 뜬다(active-only, F4)', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    // active = v1. v1 레이아웃 채택.
    emit('view:list-updated', { views: [{ id: 'v1', name: 'View 1' }], active_view_id: 'v1' })
    emit('layout:updated', snap({ view_id: 'v1', version: 1, focused_slot_id: 's1' }))
    // split_slot 은 active 무관하게 해당 view 스냅샷을 emit → 비-active v2 의 emit 이 들어온다.
    // ★전역 단조라 v2 의 version(2)이 v1(1)보다 크다★ — 옛 "view_id 다르면 무조건 채택" 분기였다면
    // 이 v2 emit 이 메인 캔버스를 가로챘을 것이다(F4). 캐시+active-only 모델에선 안 가로챈다.
    emit('layout:updated', snap({
      view_id: 'v2',
      version: 2,
      focused_slot_id: 'sX',
      layout: { type: 'slot', id: 'sX', agent_id: 'should-not-render' },
    }))
    // 캐시엔 v2 항목이 들어갔지만,
    expect(useViewStore.getState().layouts['v2']).toBeDefined()
    // 메인 렌더는 여전히 active(v1) — v2 가 캔버스를 가로채지 않는다.
    const r = rendered()
    expect(useViewStore.getState().activeViewId).toBe('v1')
    expect(r?.focusedSlotId).toBe('s1')
    expect(r?.layout).toEqual({ type: 'slot', id: 's1', agent_id: null })
  })

  it('switch_view 로 active 가 바뀌면 *이미 캐시된* 그 view layout 이 렌더된다(F4)', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    // 두 view 의 레이아웃을 각각 캐시에 채운다(전역 단조 version: v1=1, v2=2).
    emit('view:list-updated', {
      views: [{ id: 'v1', name: 'View 1' }, { id: 'v2', name: 'View 2' }],
      active_view_id: 'v1',
    })
    emit('layout:updated', snap({ view_id: 'v1', version: 1, focused_slot_id: 's1' }))
    emit('layout:updated', snap({
      view_id: 'v2',
      version: 2,
      focused_slot_id: 's2',
      layout: { type: 'slot', id: 's2', agent_id: null },
    }))
    // active = v1 일 때 메인 렌더 = v1.
    expect(rendered()?.focusedSlotId).toBe('s1')
    // switch_view → 백엔드가 active=v2 로 view:list-updated 를 emit(switch 는 layout 불변).
    emit('view:list-updated', {
      views: [{ id: 'v1', name: 'View 1' }, { id: 'v2', name: 'View 2' }],
      active_view_id: 'v2',
    })
    // 메인 렌더가 *캐시된 v2 layout* 으로 즉시 전환된다 — 새 layout:updated emit 없이.
    expect(useViewStore.getState().activeViewId).toBe('v2')
    expect(rendered()?.focusedSlotId).toBe('s2')
    expect(rendered()?.layout).toEqual({ type: 'slot', id: 's2', agent_id: null })
  })

  it('첫 emit 은 캐시 항목이 없어 항상 채택(version 0 포함)', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    emit('layout:updated', snap({ view_id: 'v1', version: 0 }))
    expect(useViewStore.getState().layouts['v1'].version).toBe(0)
  })
})

describe('initFromBackend 부팅 init(read-only pull)', () => {
  it('list_views → get_view 순서로 invoke 하고 목록+active 레이아웃 캐시를 채운다', async () => {
    // 백엔드 부팅 기본 View("View 1")를 흉내 — list_views 가 목록+active 를, get_view 가 그 레이아웃을 준다.
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_views') {
        return { views: [{ id: 'v1', name: 'View 1' }], active_view_id: 'v1' }
      }
      if (cmd === 'get_view') {
        return snap({ view_id: 'v1', version: 0, focused_slot_id: 's1' })
      }
      return undefined
    })
    await useViewStore.getState().initFromBackend()
    expect(invokeMock).toHaveBeenCalledWith('list_views')
    expect(invokeMock).toHaveBeenCalledWith('get_view', { viewId: 'v1' })
    const st = useViewStore.getState()
    expect(st.views).toHaveLength(1)
    expect(st.activeViewId).toBe('v1')
    // active 뷰 레이아웃이 캐시에 들어가 메인 렌더 대상이 된다(부팅 즉시 렌더 조건, active-only).
    const r = rendered()
    expect(r?.focusedSlotId).toBe('s1')
    expect(r?.layout).toEqual({ type: 'slot', id: 's1', agent_id: null })
  })

  it('init pull 결과보다 늦게 온 더 최신 emit 이 캐시 version 비교로 살아남는다(F2, 유령 View 없음)', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_views') {
        return { views: [{ id: 'v1', name: 'View 1' }], active_view_id: 'v1' }
      }
      if (cmd === 'get_view') return snap({ view_id: 'v1', version: 0 })
      return undefined
    })
    await useViewStore.getState().initFromBackend()
    // init 도중/직후 도착한 더 최신 split emit(version 1) — 캐시 version(0) 가드 통과해 채택.
    emit('layout:updated', snap({
      view_id: 'v1',
      version: 1,
      layout: {
        type: 'split',
        dir: 'horizontal',
        ratio: 0.5,
        a: { type: 'slot', id: 's1', agent_id: null },
        b: { type: 'slot', id: 's2', agent_id: null },
      },
    }))
    expect(rendered()?.layout?.type).toBe('split')
    expect(useViewStore.getState().layouts['v1'].version).toBe(1)
  })

  it('init pull 이 더 최신 emit 을 덮지 않는다(역전 방지, F2)', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    // 구독이 먼저 걸린 상태에서, init pull 보다 *먼저* 더 최신 emit(version 5)이 캐시에 들어왔다고 가정.
    emit('view:list-updated', { views: [{ id: 'v1', name: 'View 1' }], active_view_id: 'v1' })
    emit('layout:updated', snap({
      view_id: 'v1',
      version: 5,
      focused_slot_id: 's-new',
      layout: { type: 'slot', id: 's-new', agent_id: null },
    }))
    // 그 뒤 늦게 완료된 init 의 get_view pull(낡은 version 0)이 도착 — 캐시 version(5) 이하라 폐기돼야 한다.
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_views') {
        return { views: [{ id: 'v1', name: 'View 1' }], active_view_id: 'v1' }
      }
      if (cmd === 'get_view') return snap({ view_id: 'v1', version: 0, focused_slot_id: 's-old' })
      return undefined
    })
    await useViewStore.getState().initFromBackend()
    // 옛 pull 이 새 emit 을 덮지 않음 — 최신(version 5) 유지.
    expect(useViewStore.getState().layouts['v1'].version).toBe(5)
    expect(rendered()?.focusedSlotId).toBe('s-new')
  })

  // (a) init race 가드(2차 리뷰) — ★두 await *사이*★ 에 더 최신 view:list-updated 가 도착하면, 늦게 끝난
  // stale init 의 list payload(옛 active/list)가 그 새 상태를 덮지 않는다. layout 차원은 version 가드가
  // 막지만 list/active 차원엔 가드가 없으므로 이 케이스가 핵심. ★단순 init 후 emit 이 아니라★, get_view 의
  // pull 을 deferred 로 잡아 list_views 와 get_view 사이에서 emit 을 주입해 실제 race 윈도를 재현한다.
  it('(a) init pull 도중 더 최신 view:list-updated 가 오면 stale init 이 active/list 를 덮지 않는다', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    // get_view 응답을 테스트가 임의 시점에 resolve 하도록 deferred 로 잡는다.
    let resolveGetView: (snap: ViewSnapshot) => void = () => {}
    const getViewPending = new Promise<ViewSnapshot>(res => {
      resolveGetView = res
    })
    invokeMock.mockImplementation((cmd: string) => {
      // init 이 처음 본 백엔드 상태(stale): active=v1, view 1개.
      if (cmd === 'list_views') {
        return Promise.resolve({ views: [{ id: 'v1', name: 'View 1' }], active_view_id: 'v1' })
      }
      // get_view 는 아직 resolve 안 함 — 이 await 가 열려 있는 동안 외부 emit 을 주입한다.
      if (cmd === 'get_view') return getViewPending
      return Promise.resolve(undefined)
    })

    // init 시작 — list_views 는 곧장 resolve, get_view await 에서 멈춘다.
    const initDone = useViewStore.getState().initFromBackend()
    // 두 await 사이가 열렸을 때까지 마이크로태스크 flush(list_views resolve → applyViewListUpdated 시도 →
    // get_view await 진입). isInitSuperseded 판정이 이 사이에 일어나야 하므로 emit 을 그 *뒤*에 주입한다.
    await Promise.resolve()
    await Promise.resolve()

    // ★사이에 도착한 외부 emit★: 사용자가 새 view 로 전환 → active=v2, view 2개(더 최신 권위 상태).
    emit('view:list-updated', {
      views: [{ id: 'v1', name: 'View 1' }, { id: 'v2', name: 'View 2' }],
      active_view_id: 'v2',
    })
    // 그리고 v2 의 실제 레이아웃 emit 도 도착해 v2 캐시가 채워진다(메인 렌더 대상). 이래야 아래에서
    // "stale init 의 v1 pull 이 active(v2) 렌더를 오염시키지 않는다"를 의미 있게 단언할 수 있다.
    emit('layout:updated', snap({
      view_id: 'v2',
      version: 2,
      focused_slot_id: 's2',
      layout: { type: 'slot', id: 's2', agent_id: 'v2-agent' },
    }))
    // 이 시점 active 는 이미 v2, 렌더도 v2 캐시여야 한다(외부 emit 가드 없이 즉시 채택).
    expect(useViewStore.getState().activeViewId).toBe('v2')
    expect(rendered()?.focusedSlotId).toBe('s2')

    // 이제 늦게 init 의 get_view 가 resolve — stale init 의 applyViewListUpdated(active=v1)가 이미
    // superseded 라 건너뛰어졌어야 한다. layout 차원은 active-only 가 막는다: get_view 는 stale
    // active_view_id=v1 로 불려 v1 캐시에 낡은 snapshot(s1)을 넣지만, 메인 렌더는 active(v2) 캐시만 본다.
    resolveGetView(snap({ view_id: 'v1', version: 0, focused_slot_id: 's1' }))
    await initDone

    // ★핵심 단언★: stale init 이 active 를 v1 으로 되돌리지 않았다 — 외부 emit 의 v2/2개가 살아남는다.
    expect(useViewStore.getState().activeViewId).toBe('v2')
    expect(useViewStore.getState().views).toHaveLength(2)
    // ★FIX-4(a) 회귀 안전망★: stale active_view_id=v1 로 부른 get_view 결과가 v1 캐시엔 들어갔어도
    // 메인 렌더(active=v2)는 오염되지 않는다 — active-only 가 비-active(v1) 캐시를 화면에서 차단한다.
    expect(useViewStore.getState().layouts['v1']?.focusedSlotId).toBe('s1') // v1 캐시엔 stale pull 이 들어감
    expect(rendered()?.focusedSlotId).toBe('s2') // 그러나 렌더 = active(v2), 오염 없음
    expect(rendered()?.layout).toEqual({ type: 'slot', id: 's2', agent_id: 'v2-agent' })
  })
})

describe('subscribeViewEvents 등록/해제', () => {
  it('listen 2종(layout:updated/view:list-updated) 등록', async () => {
    {
      const { ready } = subscribeViewEvents()
      await ready
    }
    expect(listenMock).toHaveBeenCalledWith('layout:updated', expect.any(Function))
    expect(listenMock).toHaveBeenCalledWith('view:list-updated', expect.any(Function))
  })

  it('dispose(등록 완료 후 호출) 시 두 unlisten 모두 해제', async () => {
    // subscribeViewEvents 는 `{ dispose, ready }` 동기 반환. ready 를 기다려 등록을 끝낸 뒤 dispose 하면
    // 두 핸들 모두 해제된다(F-listen — 등록 완료 경로).
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
    // 두 핸들 각 1회씩 = 2회. 두 번째 dispose 는 handles 가 비어 noop(이미 해제한 핸들 재호출 X).
    expect(unlistenMock).toHaveBeenCalledTimes(2)
  })

  // ★이번 결함의 회귀 안전망★: ready 가 pending(listen 등록 미완) 인 동안 dispose 가 불리면, 뒤늦게 등록이
  // 끝나 도착한 unlisten 핸들을 *즉시* 호출해야 한다(안 부르면 영구 누수 — 예전 dead-branch 가 놓친 바로 그
  // 경로). listen 을 deferred 로 잡아 등록 await 윈도를 열어두고, 그 사이 dispose → 이후 listen resolve 순으로
  // 실제 race 를 재현한다((a) init race 테스트의 deferred 패턴과 동형).
  it('ready pending 중 dispose → 늦게 도착한 unlisten 핸들을 즉시 호출(누수 가드)', async () => {
    // 각 listen 호출의 resolve 를 테스트가 임의 시점에 당기도록 deferred 로 잡는다(이벤트명별).
    // 반환 핸들 타입은 listenMock 의 mock(unlistenMock) 타입에 맞춘다(tsc 정합).
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
    // 아직 두 listen 모두 등록 미완(deferred) — 이 시점 dispose 가 불린다(ready 전 dispose).
    dispose()
    expect(unlistenMock).not.toHaveBeenCalled() // 아직 도착한 핸들이 없으니 호출 0.

    // 이제 등록이 뒤늦게 끝나 unlisten 핸들이 도착 — disposed 분기가 즉시 해제해야 한다(누수 0).
    resolveListen.get('layout:updated')!(unlistenMock)
    resolveListen.get('view:list-updated')!(unlistenMock)
    await ready // ready 는 dispose 가 먼저 와도 hang 없이 정상 종료(계약 ③).

    // ★핵심 단언★: 늦게 도착한 두 핸들 모두 즉시 호출됐다(가드 무력화 시 0 이 되어 red).
    expect(unlistenMock).toHaveBeenCalledTimes(2)
  })

  it('ready 는 한쪽 listen 등록이 실패해도 hang 하지 않고 성공분을 정리한다(계약 ③④)', async () => {
    // layout:updated 는 성공(unlisten 반환), view:list-updated 는 reject — ready 는 reject 로 종료(hang X),
    // 성공분(layout)은 dispose 가 해제해 누수 0.
    listenMock.mockImplementation((event: string, handler: (e: { payload: unknown }) => void) => {
      listeners.set(event, handler)
      if (event === 'view:list-updated') return Promise.reject(new Error('listen failed'))
      return Promise.resolve(unlistenMock)
    })

    const { dispose, ready } = subscribeViewEvents()
    await expect(ready).rejects.toThrow('listen failed') // hang 금지 — 정의된 reject 로 종료.
    dispose()
    // 성공한 layout 핸들 1개가 dispose 로 해제됨(부분 등록분 정리, 누수 0).
    expect(unlistenMock).toHaveBeenCalledTimes(1)
  })

  // ★FIX-4 갭 메움(Codex C3 계약 ④)★: 위 테스트는 "성공 listen *즉시* resolve 후 다른쪽 실패"만 본다.
  // 진짜 위험한 윈도는 ★Promise.all 이 먼저 reject 된 *뒤* 다른 listen 이 *나중에* resolve★ 되는 경우다 —
  // 그 늦은 성공 핸들은 ready 가 이미 reject 로 끝난 뒤 도착하므로, 호출자가 dispose 를 부르지 않으면 영구
  // 누수된다. 계약 ④는 "호출자가 ready reject 시 dispose 를 호출하면 그 늦은 핸들도 즉시 해제"임을 약속한다.
  // reject 를 deferred 로 잡아 그 순서를 강제 재현한다: view:list-updated 먼저 reject → dispose → layout
  // 나중 resolve → adopt 의 disposed 분기가 즉시 해제.
  it('ready reject 후 *나중에* resolve 된 listen 핸들도 호출자 dispose 가 즉시 해제(계약 ④, 늦은 성공분 누수 0)', async () => {
    let resolveLayout: (fn: typeof unlistenMock) => void = () => {}
    let rejectList: (err: Error) => void = () => {}
    listenMock.mockImplementation((event: string, handler: (e: { payload: unknown }) => void) => {
      listeners.set(event, handler)
      if (event === 'layout:updated') {
        // layout 등록은 아직 미완(deferred) — reject 가 ready 를 끝낸 *뒤* 나중에 resolve 시킬 것.
        return new Promise<typeof unlistenMock>(res => {
          resolveLayout = res
        })
      }
      // view:list-updated 는 deferred reject — 이쪽이 먼저 settle 돼 Promise.all 을 reject 시킨다.
      return new Promise<typeof unlistenMock>((_res, rej) => {
        rejectList = rej
      })
    })

    const { dispose, ready } = subscribeViewEvents()
    // ① view:list-updated 가 먼저 reject → ready(Promise.all)가 reject 로 종료(layout 은 아직 pending).
    rejectList(new Error('listen failed'))
    await expect(ready).rejects.toThrow('listen failed') // hang 금지.
    expect(unlistenMock).not.toHaveBeenCalled() // 아직 도착한 성공 핸들이 없다.

    // ② 호출자(eventBus FIX-2 의 catch)가 dispose 를 부른다 — 이 시점 disposed=true.
    dispose()
    expect(unlistenMock).not.toHaveBeenCalled() // handles 가 비어 있어 아직 0(늦은 핸들 미도착).

    // ③ 이제서야 layout 등록이 *나중에* resolve — adopt 의 disposed 분기가 즉시 해제해야 한다(누수 0).
    resolveLayout(unlistenMock)
    await Promise.resolve() // adopt(.then) 마이크로태스크 flush.
    // ★핵심 단언★: ready reject 후 늦게 도착한 성공 핸들도 dispose 트리거로 해제됐다(가드 무력화 시 0 → red).
    expect(unlistenMock).toHaveBeenCalledTimes(1)
  })
})

// ★M0 스파이크(임시) — ADR-0044★: RichSlot 오버레이(프론트 전용, invoke 안 탐). M2 에서 제거될 자리라
// 테스트도 최소 — "set/clear 가 richSlots 를 정확히 갱신하고 실슬롯 콘텐츠(agent_id)엔 안 닿는다"만 본다.
describe('RichSlot 스파이크 오버레이(mountRich/unmountRich)', () => {
  it('mountRich → richSlots 에 slotId 표시, invoke 는 안 부른다(권위 루프 우회)', () => {
    useViewStore.getState().mountRich('slot-A')
    expect(useViewStore.getState().richSlots).toEqual({ 'slot-A': true })
    expect(invokeMock).not.toHaveBeenCalled() // 다른 액션과 달리 백엔드 invoke 없음(스파이크 예외)
  })

  it('unmountRich → 해당 slotId 만 제거(다른 rich 슬롯은 유지)', () => {
    useViewStore.getState().mountRich('slot-A')
    useViewStore.getState().mountRich('slot-B')
    useViewStore.getState().unmountRich('slot-A')
    expect(useViewStore.getState().richSlots).toEqual({ 'slot-B': true })
  })

  it('오버레이는 layout 캐시(agent_id 등 실슬롯 콘텐츠)를 건드리지 않는다', () => {
    useViewStore.setState({
      layouts: {
        v1: { layout: { type: 'slot', id: 'slot-A', agent_id: null }, focusedSlotId: 'slot-A', version: 1 },
      },
      activeViewId: 'v1',
    })
    useViewStore.getState().mountRich('slot-A')
    // rich 는 별도 오버레이 — 백엔드 권위 layout 은 불변(agent_id null 그대로).
    expect(rendered()?.layout).toEqual({ type: 'slot', id: 'slot-A', agent_id: null })
    expect(useViewStore.getState().richSlots['slot-A']).toBe(true)
  })
})

// ★렌더 모드 오버라이드(§5, 프론트 전용)★: set/clear + slot 생명주기 정리(FIX-1) + 미타입 진입 가드(FIX-4).
// richSlots 처럼 invoke→emit 권위 루프를 안 타는 프론트 전용 상태라 순수 로직만 검증한다.
describe('renderModeOverride 오버라이드 + 생명주기 정리(§5)', () => {
  beforeEach(() => {
    // 위 공통 beforeEach 는 renderModeOverride 를 초기화하지 않으므로(setState 부분 갱신) 여기서 격리.
    useViewStore.setState({ renderModeOverride: {} })
  })

  it('closeSlot 은 그 slot 의 오버라이드를 clear 한다(slot 소멸 → 엔트리 누수 방지, FIX-1)', async () => {
    useViewStore.getState().setRenderMode('s-close', 'dom')
    expect(useViewStore.getState().renderModeOverride['s-close']).toBe('dom')
    await useViewStore.getState().closeSlot('v1', 's-close')
    // slotId 는 closeSlot 두 번째 인자에서 온다 → 그 엔트리가 제거돼야 한다.
    expect(useViewStore.getState().renderModeOverride['s-close']).toBeUndefined()
    // 대응 invoke 는 그대로 부른다(낙관 clear 는 invoke 와 병행).
    expect(invokeMock).toHaveBeenCalledWith('close_slot', { viewId: 'v1', slotId: 's-close' })
  })

  it('assignAgent 은 그 slot 의 오버라이드를 clear 한다(이전 agent 오버라이드가 새 agent 에 새지 않게, FIX-1)', async () => {
    useViewStore.getState().setRenderMode('s-assign', 'rich')
    expect(useViewStore.getState().renderModeOverride['s-assign']).toBe('rich')
    await useViewStore.getState().assignAgent('v1', 's-assign', 'agent-new')
    // slotId 는 assignAgent 두 번째 인자에서 온다 → 재배정 시 그 slot 의 오버라이드 제거.
    expect(useViewStore.getState().renderModeOverride['s-assign']).toBeUndefined()
    expect(invokeMock).toHaveBeenCalledWith('assign_agent', {
      viewId: 'v1',
      slotId: 's-assign',
      agentId: 'agent-new',
    })
  })

  it('closeSlot 은 다른 slot 의 오버라이드는 건드리지 않는다', async () => {
    useViewStore.getState().setRenderMode('s-keep', 'dom')
    useViewStore.getState().setRenderMode('s-gone', 'rich')
    await useViewStore.getState().closeSlot('v1', 's-gone')
    expect(useViewStore.getState().renderModeOverride['s-gone']).toBeUndefined()
    expect(useViewStore.getState().renderModeOverride['s-keep']).toBe('dom') // 무관 slot 은 유지
  })

  it('setRenderMode(무효 mode) → no-op(store 불변) + console.warn(FIX-4)', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    // window.__engramLayout 경유 미타입 JS 가 넘길 수 있는 잘못된 값.
    ;(useViewStore.getState().setRenderMode as (n: string, m: unknown) => void)('s1', 'bogus')
    expect(useViewStore.getState().renderModeOverride['s1']).toBeUndefined() // store 에 안 씀
    expect(warn).toHaveBeenCalled()
    warn.mockRestore()
  })

  it('setRenderMode(유효 mode) → 정상 기록', () => {
    useViewStore.getState().setRenderMode('s1', 'dom')
    expect(useViewStore.getState().renderModeOverride['s1']).toBe('dom')
  })
})
