// WindowLayout 단위테스트 — 창별 탭바 + keep-alive 슬롯 캔버스 + 자기 창 학습 + 0탭 자가닫힘(ADR-0057).
//
// ★검증 불변식(§7-1)★:
//   1. mount 시 list_tabs(label) 초기 pull → 그 창 탭바+캔버스 렌더.
//   2. keep-alive(ADR-0056): windows[label].tabs 전부 마운트, 활성만 display:block(숨은 탭 display:none).
//   3. window:tabs-updated 는 자기 label 만 반응(다른 label emit 무시).
//   4. 0탭 신호 → getCurrentWindow().close() 자가닫힘(idempotent — 재진입 가드).
//   5. TabBar 액션(전환/추가/닫기)이 store 액션(switchTab/createTab/closeTab)을 이 label 로 부른다.

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ── listen mock: 이벤트명별 핸들러 보관 → 테스트가 직접 emit ──
const listeners = new Map<string, (e: { payload: unknown }) => void>()
const unlistenMock = vi.fn()
// listenShouldReject: FIX-2 테스트가 부팅 중 리스너 등록 reject 를 흉내낼 때 켠다(기본 false).
let listenShouldReject = false
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: (e: { payload: unknown }) => void) => {
    if (listenShouldReject) throw new Error('listen registration failed')
    listeners.set(event, handler)
    return unlistenMock
  }),
}))

// ── invoke mock: list_tabs/get_view pull ──
const invokeMock = vi.fn(async (_cmd: string, ..._rest: unknown[]) => undefined as unknown)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, ...rest: unknown[]) => invokeMock(cmd, ...rest),
  Channel: class {
    onmessage: unknown = null
  },
}))

// ── getCurrentWindow mock — 0탭 자가닫힘 관측 ──
const closeMock = vi.fn(async () => undefined)
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ close: closeMock, label: () => 'main' }),
}))

// ── ViewLayoutRenderer stub — 캔버스 내부는 관심 밖(어느 view 를 그리는지만 관측) + ★mount 카운터★.
// ★S4-F5 keep-alive no-remount★: 슬롯 컴포넌트가 탭 전환에 remount 되지 않는지(터미널 인스턴스 생존)를
//   프록시하려고, mount 시 useEffect([])가 viewId 별 카운터를 1 올린다. 전환 후 카운트가 안 늘고 display
//   만 토글되면 keep-alive("전환 무손실", ADR-0056) 구조가 성립한다.
const mountCounts = vi.hoisted(() => new Map<string, number>())
vi.mock('./ViewLayoutRenderer', async () => {
  const React = (await import('react')).default
  return {
    default: ({ viewIdOverride }: { viewIdOverride?: string | null }) => {
      const id = viewIdOverride ?? ''
      React.useEffect(() => {
        mountCounts.set(id, (mountCounts.get(id) ?? 0) + 1) // mount 1회당 +1(remount 되면 또 오름)
      }, [id])
      return <div data-testid="view-renderer" data-view-id={id} />
    },
  }
})

import WindowLayout from './WindowLayout'
import { useViewStore } from '../../store/viewStore'
import type { ViewSnapshot } from '../../api/layoutTypes'

function slotSnap(viewId: string, version: number): ViewSnapshot {
  return {
    view_id: viewId,
    layout: { type: 'slot', id: `s-${viewId}`, content: { type: 'empty' } }, // ADR-0060
    focused_slot_id: `s-${viewId}`,
    slot_spatial: [], // ADR-0068: 공간 파생(이 테스트는 안 씀 — 빈 배열로 타입 충족)
    version,
  }
}

/** listen 핸들러로 payload 를 흘려보낸다(백엔드 emit 흉내). */
function emit(event: string, payload: unknown): void {
  const h = listeners.get(event)
  if (!h) throw new Error(`no listener for ${event}`)
  h({ payload })
}

beforeEach(() => {
  listeners.clear()
  unlistenMock.mockClear()
  closeMock.mockClear()
  invokeMock.mockReset()
  mountCounts.clear()
  listenShouldReject = false
  useViewStore.setState({ layouts: {}, windows: {}, renderModeOverride: {} })
  // 기본: list_tabs → 탭 2개(v1 active), get_view → 그 view 스냅샷.
  invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd === 'list_tabs') {
      return {
        label: (args as { window: string }).window,
        tabs: [{ id: 'v1', name: 'Tab 1' }, { id: 'v2', name: 'Tab 2' }],
        active: 'v1',
        version: 1,
      }
    }
    if (cmd === 'get_view') return slotSnap((args as { viewId: string }).viewId, 1)
    return undefined
  })
})

afterEach(cleanup)

describe('WindowLayout — 초기 pull + keep-alive 캔버스', () => {
  it('mount 시 list_tabs(label) pull → 탭바 + 모든 탭 캔버스 마운트(keep-alive)', async () => {
    render(<WindowLayout label="main" />)
    // 초기 pull 로 창 상태가 채워지면 탭바가 뜬다.
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
    expect(invokeMock).toHaveBeenCalledWith('list_tabs', { window: 'main' })
    // ★keep-alive★: 두 탭 모두 캔버스 마운트(숨은 탭도 인스턴스 유지).
    const canvases = screen.getAllByTestId('tab-canvas')
    expect(canvases).toHaveLength(2)
    const v1 = canvases.find(c => c.getAttribute('data-view-id') === 'v1')!
    const v2 = canvases.find(c => c.getAttribute('data-view-id') === 'v2')!
    // 활성(v1)만 표시, 숨은(v2)은 display:none.
    expect(v1.style.display).toBe('block')
    expect(v2.style.display).toBe('none')
  })

  it('각 탭 캔버스에 그 view 를 get_view 로 채워 ViewLayoutRenderer 에 viewIdOverride 로 내려꽂는다', async () => {
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getAllByTestId('view-renderer').length).toBe(2))
    // keep-alive 라 숨은 탭(v2)도 get_view 로 캐시가 채워진다.
    expect(invokeMock).toHaveBeenCalledWith('get_view', { viewId: 'v1' })
    expect(invokeMock).toHaveBeenCalledWith('get_view', { viewId: 'v2' })
    const renderers = screen.getAllByTestId('view-renderer')
    const ids = renderers.map(r => r.getAttribute('data-view-id')).sort()
    expect(ids).toEqual(['v1', 'v2'])
  })
})

// ── ★S4-F4: mount-race — list_tabs 초기 pull await 중 더 최신 window:tabs-updated 도착★ ──────────────
// 옛 viewStore.test.ts 의 deferred init-race 하네스가 검증하던 클래스를 컴포넌트 레벨로 복원한다.
// 시나리오: WindowLayout mount → listen 먼저 등록(§7-1 "구독 먼저, pull 나중") → list_tabs pull 이
//   pending 인 동안 더 최신 version 의 window:tabs-updated 가 도착 → pull 이 뒤늦게 stale payload 로
//   resolve → applyWindowTabsUpdated 의 version 가드가 stale pull 의 덮어쓰기를 막는지 단언.
describe('WindowLayout — mount-race(초기 pull vs 최신 emit, S4-F4)', () => {
  it('list_tabs pull 이 pending 인 동안 더 최신 emit 도착 → 늦게 온 stale pull 이 최신 상태를 덮지 않는다', async () => {
    // list_tabs 를 deferred 로 잡아 pull 완료 전 race 창을 연다. get_view 는 즉시 resolve.
    let resolveListTabs: (payload: unknown) => void = () => {}
    const listTabsPending = new Promise<unknown>(res => {
      resolveListTabs = res
    })
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_tabs') return listTabsPending // ★pending — 아직 resolve 안 함★
      if (cmd === 'get_view') return slotSnap((args as { viewId: string }).viewId, 1)
      return undefined
    })

    render(<WindowLayout label="slot-popup-1" />)

    // listen 등록이 끝날 때까지(핸들러가 map 에 들어옴) 마이크로태스크를 흘린다.
    await waitFor(() => expect(listeners.has('window:tabs-updated')).toBe(true))

    // ① pull 이 아직 pending 인 사이에 더 최신 version(5) emit 이 먼저 상태를 채운다(v2 active).
    emit('window:tabs-updated', {
      label: 'slot-popup-1',
      tabs: [{ id: 'v1', name: 'Tab 1' }, { id: 'v2', name: 'Tab 2' }],
      active: 'v2',
      version: 5,
    })
    await waitFor(() => expect(useViewStore.getState().windows['slot-popup-1']?.version).toBe(5))

    // ② 그 뒤 늦게 list_tabs pull 이 낡은 version(1)·active v1 로 resolve 된다.
    resolveListTabs({
      label: 'slot-popup-1',
      tabs: [{ id: 'v1', name: 'Tab 1' }, { id: 'v2', name: 'Tab 2' }],
      active: 'v1',
      version: 1,
    })
    await Promise.resolve()
    await Promise.resolve()

    // ★version 가드가 stale pull(v1)의 덮어쓰기를 막는다★ — 최신(version 5·active v2) 유지.
    const win = useViewStore.getState().windows['slot-popup-1']
    expect(win.version).toBe(5)
    expect(win.active).toBe('v2')
  })
})

// ── ★ADR-0102: 부팅 pull 유계 재시도 + 최종 실패 표면화★ ──────────────────────────────────────
// main 은 이벤트 복구 경로가 없어(window:tabs-updated 는 탭 변형 시에만 발화) 부팅 list_tabs pull 이
//   one-shot 이면 조기 실패 = 로딩 영구 고착이다. (1) N번 reject 후 resolve → 창 상태가 채워져 탭바가
//   뜬다(재시도가 회수). (2) 계속 reject → 재시도 소진 후 로딩 대신 가시적 에러 상태(window-boot-error)를
//   렌더한다(조용히 로딩에 안 머문다).
describe('WindowLayout — 부팅 pull 재시도/표면화(ADR-0102)', () => {
  it('list_tabs 가 2번 reject 후 resolve → 재시도로 회수해 탭바가 뜬다', async () => {
    let listTabsCalls = 0
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_tabs') {
        listTabsCalls += 1
        if (listTabsCalls <= 2) throw new Error(`state not ready ${listTabsCalls}`)
        return {
          label: (args as { window: string }).window,
          tabs: [{ id: 'v1', name: 'Tab 1' }],
          active: 'v1',
          version: 1,
        }
      }
      if (cmd === 'get_view') return slotSnap((args as { viewId: string }).viewId, 1)
      return undefined
    })

    render(<WindowLayout label="main" />)
    // 재시도가 성공을 회수하면 로딩이 걷히고 탭바가 뜬다(backoff 대기 포함 → 넉넉한 timeout).
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy(), { timeout: 3000 })
    expect(listTabsCalls).toBeGreaterThanOrEqual(3) // 첫 시도 + 재시도 2회 이상.
    // 에러 상태는 뜨지 않는다(성공 회수).
    expect(screen.queryByTestId('window-boot-error')).toBeNull()
  })

  it('list_tabs 가 계속 reject → 재시도 소진 후 가시적 에러 상태 렌더(로딩 고착 아님)', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_tabs') throw new Error('backend down')
      return undefined
    })

    render(<WindowLayout label="main" />)
    // 재시도 소진 후 로딩이 아니라 boot-error 가 표면화된다.
    await waitFor(() => expect(screen.getByTestId('window-boot-error')).toBeTruthy(), {
      timeout: 3000,
    })
    // 창 상태(win)는 끝내 안 채워져 탭바는 없다.
    expect(screen.queryByTestId('tab-bar')).toBeNull()
  })

  // ★FIX-2: listen() 등록 reject → 부팅 경로 전체가 깨져도 무신호 고착이 아니라 boot-error 표면화★
  // 옛 구조는 listen() await 가 try 밖이라, 리스너 등록이 reject 하면 async IIFE 가 unhandled 로 죽고
  //   list_tabs pull 이 시작조차 안 돼 bootFailed 가 영영 안 걸렸다(로딩 플레이스홀더 영구 고착 — 무신호).
  //   이제 listen 실패도 pull 실패와 동일하게 bootFailed 로 표면화한다.
  it('listen 등록이 reject → 조용한 로딩 고착이 아니라 boot-error 표면화(FIX-2)', async () => {
    listenShouldReject = true // 부팅 중 리스너 등록이 reject.
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getByTestId('window-boot-error')).toBeTruthy(), {
      timeout: 3000,
    })
    // 로딩에 조용히 머물지 않고(로딩 플레이스홀더 부재), 탭바도 없다(창 상태 미도착).
    expect(screen.queryByTestId('tab-bar')).toBeNull()
  })
})

// ── ★FIX-3: 탭별 get_view keep-alive pull 도 유계 재시도로 self-heal★ ──────────────────────────
// 옛 one-shot get_view 는 transient 실패 시 console.warn 뿐이고 tabIdsKey 불변이라 재발행 트리거가 없어
//   그 탭 캔버스가 무관한 layout:updated 가 올 때까지 "View 로딩 중"에 갇혔다. 재시도로 순간적 미준비를
//   스스로 회복하는지, 재발행이 실제로 일어났는지(호출 재시도)를 단언한다.
describe('WindowLayout — get_view keep-alive 재시도 self-heal(FIX-3)', () => {
  it('get_view 가 처음엔 reject, 재시도로 성공 → 그 탭 캔버스가 회복(재발행 관측)', async () => {
    let v1Calls = 0
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_tabs') {
        return {
          label: (args as { window: string }).window,
          tabs: [{ id: 'v1', name: 'Tab 1' }],
          active: 'v1',
          version: 1,
        }
      }
      if (cmd === 'get_view') {
        const viewId = (args as { viewId: string }).viewId
        if (viewId === 'v1') {
          v1Calls += 1
          if (v1Calls <= 2) throw new Error(`view not ready ${v1Calls}`) // 처음 2회 transient 실패.
        }
        return slotSnap(viewId, 1)
      }
      return undefined
    })

    render(<WindowLayout label="main" />)
    // 재시도가 성공을 회수하면 그 탭 캔버스(v1)가 로딩을 벗고 실제 렌더러로 채워진다.
    await waitFor(
      () => {
        const renderers = screen.queryAllByTestId('view-renderer')
        expect(renderers.some(r => r.getAttribute('data-view-id') === 'v1')).toBe(true)
      },
      { timeout: 3000 },
    )
    // ★핵심★: get_view(v1)가 한 번이 아니라 여러 번(재시도) 발행됐다 — one-shot 이었으면 1회로 갇힌다.
    expect(v1Calls).toBeGreaterThanOrEqual(3)
  })
})

describe('WindowLayout — window:tabs-updated 자기 label 필터', () => {
  it('자기 label emit → 활성 탭 스왑(v1→v2)', async () => {
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
    emit('window:tabs-updated', {
      label: 'main',
      tabs: [{ id: 'v1', name: 'Tab 1' }, { id: 'v2', name: 'Tab 2' }],
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
      tabs: [{ id: 'x1', name: 'X' }],
      active: 'x1',
      version: 99,
    })
    // main 창 active 는 여전히 v1(다른 label 무시).
    const v1 = screen.getAllByTestId('tab-canvas').find(c => c.getAttribute('data-view-id') === 'v1')!
    expect(v1.style.display).toBe('block')
  })
})

// ── ★S4-F5: keep-alive no-remount — 탭 전환 시 슬롯 컴포넌트가 remount 안 됨(터미널 인스턴스 생존)★ ──
// ADR-0056 keep-alive "전환 무손실"의 유닛 프록시: 활성/숨은 슬롯 렌더러가 전환 후 재마운트되지 않고
// display 만 토글되는지 mount 카운터로 단언한다(실제 xterm 생존은 qa cdp 스테이지6 소관 — 여긴 구조만).
describe('WindowLayout — keep-alive no-remount(ADR-0056, S4-F5)', () => {
  it('탭 전환(v1→v2) 후 두 슬롯 렌더러 mount 횟수가 안 늘고 display 만 토글된다', async () => {
    render(<WindowLayout label="main" />)
    // 초기 pull + get_view 로 두 탭 캔버스가 각 1회 마운트될 때까지 대기.
    await waitFor(() => {
      expect(mountCounts.get('v1')).toBe(1)
      expect(mountCounts.get('v2')).toBe(1)
    })
    // 초기: v1 활성(block), v2 숨김(none).
    {
      const canvases = screen.getAllByTestId('tab-canvas')
      const v1 = canvases.find(c => c.getAttribute('data-view-id') === 'v1')!
      const v2 = canvases.find(c => c.getAttribute('data-view-id') === 'v2')!
      expect(v1.style.display).toBe('block')
      expect(v2.style.display).toBe('none')
    }

    // 활성 탭을 v2 로 스왑(switch) — keep-alive 면 remount 없이 display 만 바뀐다.
    emit('window:tabs-updated', {
      label: 'main',
      tabs: [{ id: 'v1', name: 'Tab 1' }, { id: 'v2', name: 'Tab 2' }],
      active: 'v2',
      version: 2,
    })
    await waitFor(() => {
      const v2 = screen.getAllByTestId('tab-canvas').find(c => c.getAttribute('data-view-id') === 'v2')!
      expect(v2.style.display).toBe('block')
    })

    // ★핵심 단언★: mount 카운트가 여전히 각 1 — 전환으로 remount 되지 않았다(인스턴스 생존).
    expect(mountCounts.get('v1')).toBe(1)
    expect(mountCounts.get('v2')).toBe(1)
    // display 만 토글: 이제 v1 숨김, v2 표시.
    const canvases = screen.getAllByTestId('tab-canvas')
    const v1 = canvases.find(c => c.getAttribute('data-view-id') === 'v1')!
    const v2 = canvases.find(c => c.getAttribute('data-view-id') === 'v2')!
    expect(v1.style.display).toBe('none')
    expect(v2.style.display).toBe('block')
  })
})

describe('WindowLayout — 0탭 자가닫힘(§5-2/G2)', () => {
  it('0탭 신호(window:tabs-updated{tabs:[]}) → getCurrentWindow().close()', async () => {
    render(<WindowLayout label="slot-popup-1" />)
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
    emit('window:tabs-updated', { label: 'slot-popup-1', tabs: [], active: 'v1', version: 5 })
    await waitFor(() => expect(closeMock).toHaveBeenCalledTimes(1))
  })

  it('0탭 신호가 두 번 와도 close 는 한 번만(idempotent 재진입 가드)', async () => {
    render(<WindowLayout label="slot-popup-1" />)
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
    emit('window:tabs-updated', { label: 'slot-popup-1', tabs: [], active: 'v1', version: 5 })
    emit('window:tabs-updated', { label: 'slot-popup-1', tabs: [], active: 'v1', version: 6 })
    await waitFor(() => expect(closeMock).toHaveBeenCalledTimes(1))
    // 마이크로태스크 더 flush 해도 여전히 1회.
    await Promise.resolve()
    expect(closeMock).toHaveBeenCalledTimes(1)
  })
})

describe('WindowLayout — TabBar 액션 → store 액션(이 label)', () => {
  it('[+] 클릭 → createTab(label) invoke', async () => {
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getByTestId('tab-add')).toBeTruthy())
    fireEvent.click(screen.getByTestId('tab-add'))
    expect(invokeMock).toHaveBeenCalledWith('create_tab', { window: 'main', name: null })
  })

  it('숨은 탭 클릭 → switchTab(label, view) invoke', async () => {
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
    const tab2 = screen.getAllByTestId('tab').find(t => t.getAttribute('data-view-id') === 'v2')!
    fireEvent.click(tab2)
    expect(invokeMock).toHaveBeenCalledWith('switch_tab', { window: 'main', view: 'v2' })
  })

  it('탭 × 클릭 → closeTab(label, view) invoke', async () => {
    render(<WindowLayout label="main" />)
    await waitFor(() => expect(screen.getByTestId('tab-bar')).toBeTruthy())
    const closeBtns = screen.getAllByTestId('tab-close')
    fireEvent.click(closeBtns[0]) // v1 닫기
    expect(invokeMock).toHaveBeenCalledWith('close_tab', { window: 'main', view: 'v1' })
  })
})
