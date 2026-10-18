// dockSession 단위테스트 — 듣기 셋(자기 창 대상 · `window_role` 보다 먼저) · 앱 전체 키 감시(`dock:drag` 로 켜고 끔 · 키 하나에
//   끝 하나) · 끌기 입력 구독(`dock:feed`) · 정리. 창 모드가 화면에 닿는 것은 `WindowLayout.test.tsx` 의 대기 모드 시험이 잰다.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const LABEL = 'pop-1'

const order: string[] = []
const listeners = new Map<string, { handler: (e: { payload: unknown }) => void; options: unknown }>()
const unlistened: string[] = []
const listenMock = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/event', () => ({ listen: listenMock }))

const invokeMock = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ label: LABEL }) }))

import type { TabDragEvent } from '../../../../src-tauri/bindings/TabDragEvent'
import type { TabDragFeed } from '../../../../src-tauri/bindings/TabDragFeed'
import {
  CMD_TAB_DRAG_END,
  CMD_WINDOW_ROLE,
  EVT_DRAG,
  EVT_FEED,
  EVT_SPARE,
  keyEnd,
  subscribeDrag,
  subscribeFeed,
  watchDockSession,
  type WindowMode,
} from './dockSession'

const ticks = async (): Promise<void> => {
  for (let i = 0; i < 20; i++) await Promise.resolve()
}

function emit(event: string, payload: unknown): void {
  const l = listeners.get(event)
  if (!l) throw new Error(`${event} 구독 없음`)
  l.handler({ payload })
}

const drag = (session: number, phase: TabDragEvent['phase']): TabDragEvent => ({ session, phase })

function key(k: string, opts: { repeat?: boolean } = {}): KeyboardEvent {
  const e = new KeyboardEvent('keydown', { key: k, repeat: opts.repeat ?? false, bubbles: true, cancelable: true })
  document.body.dispatchEvent(e)
  return e
}

const ends = () => invokeMock.mock.calls.filter(c => c[0] === CMD_TAB_DRAG_END).map(c => c[1])

let modes: WindowMode[]
let stop: (() => void) | null
let unsubs: Array<() => void>

async function watched(): Promise<void> {
  stop = watchDockSession(m => modes.push(m))
  await ticks()
}

beforeEach(() => {
  order.length = 0
  listeners.clear()
  unlistened.length = 0
  listenMock.mockReset()
  listenMock.mockImplementation(async (event: string, handler: (e: { payload: unknown }) => void, options?: unknown) => {
    order.push(`listen:${event}`)
    listeners.set(event, { handler, options })
    return () => {
      unlistened.push(event)
    }
  })
  invokeMock.mockReset()
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd === CMD_WINDOW_ROLE) {
      order.push(`invoke:${cmd}`)
      return { role: 'model', drag: null }
    }
    return undefined
  })
  modes = []
  stop = null
  unsubs = []
})
afterEach(() => {
  stop?.()
  unsubs.forEach(off => off())
  vi.restoreAllMocks()
})

describe('watchDockSession — 듣기 · 창 모드', () => {
  it('dock:spare · dock:drag · dock:feed 를 자기 창 대상으로 건 뒤 window_role 을 묻는다', async () => {
    await watched()
    expect(order).toEqual([`listen:${EVT_SPARE}`, `listen:${EVT_DRAG}`, `listen:${EVT_FEED}`, `invoke:${CMD_WINDOW_ROLE}`])
    for (const event of [EVT_SPARE, EVT_DRAG, EVT_FEED]) {
      expect(listeners.get(event)?.options).toEqual({ target: LABEL })
    }
    expect(modes).toEqual(['normal'])
  })

  it('window_role = parked → parked · 그 뒤 dock:spare 로 바뀜을 알린다', async () => {
    invokeMock.mockImplementation(async () => ({ role: 'parked', drag: null }))
    await watched()
    expect(modes).toEqual(['parked'])
    emit(EVT_SPARE, { step: 'adopted' })
    emit(EVT_SPARE, { step: 'parked' })
    expect(modes).toEqual(['parked', 'normal', 'parked'])
  })

  it('정리하면 듣기를 떼고 그 뒤 사건 · 키에 반응하지 않는다', async () => {
    await watched()
    emit(EVT_DRAG, drag(1, 'active'))
    stop?.()
    stop = null
    expect(unlistened.sort()).toEqual([EVT_DRAG, EVT_FEED, EVT_SPARE].sort())

    key('Escape')
    emit(EVT_DRAG, drag(2, 'active'))
    key('a')
    await ticks()
    expect(ends()).toEqual([])
  })

  it('듣기가 정리 뒤에 걸리면 곧바로 떼고 window_role 을 묻지 않는다', async () => {
    let finish: (off: () => void) => void = () => {}
    listenMock.mockImplementationOnce(
      () =>
        new Promise<() => void>(res => {
          finish = res
        }),
    )
    const offLate = vi.fn()
    stop = watchDockSession(m => modes.push(m))
    stop()
    stop = null
    finish(offLate)
    await ticks()
    expect(offLate).toHaveBeenCalledTimes(1)
    expect(order).not.toContain(`invoke:${CMD_WINDOW_ROLE}`)
    expect(modes).toEqual([])
  })
})

describe('앱 전체 키 감시', () => {
  it('끄는 중이 아니면 키를 건드리지 않는다', async () => {
    await watched()
    const e = key('Escape')
    await ticks()
    expect(e.defaultPrevented).toBe(false)
    expect(ends()).toEqual([])
  })

  it('active 동안 Escape = cancel · 그 밖의 키 = drop — src key · at null · 키가 슬롯에 닿지 않는다', async () => {
    await watched()
    emit(EVT_DRAG, drag(3, 'active'))
    const reached = vi.fn()
    document.body.addEventListener('keydown', reached)
    try {
      const esc = key('Escape')
      const a = key('a')
      key('Shift')
      await ticks()
      expect(esc.defaultPrevented).toBe(true)
      expect(a.defaultPrevented).toBe(true)
      expect(reached).not.toHaveBeenCalled()
      expect(ends()).toEqual([
        { args: { session: 3, how: 'cancel', src: 'key', at: null } },
        { args: { session: 3, how: 'drop', src: 'key', at: null } },
        { args: { session: 3, how: 'drop', src: 'key', at: null } },
      ])
    } finally {
      document.body.removeEventListener('keydown', reached)
    }
  })

  it('키 반복도 거르지 않는다 — keydown 하나에 끝 하나', async () => {
    await watched()
    emit(EVT_DRAG, drag(1, 'active'))
    key('a')
    key('a', { repeat: true })
    key('a', { repeat: true })
    await ticks()
    expect(ends()).toHaveLength(3)
  })

  it('같은 세션의 active 를 또 받아도 키 하나에 끝 하나 · 새 세션이면 그 번호로', async () => {
    await watched()
    emit(EVT_DRAG, drag(4, 'active'))
    emit(EVT_DRAG, drag(4, 'active'))
    key('a')
    emit(EVT_DRAG, drag(5, 'active'))
    key('Escape')
    await ticks()
    expect(ends()).toEqual([
      { args: { session: 4, how: 'drop', src: 'key', at: null } },
      { args: { session: 5, how: 'cancel', src: 'key', at: null } },
    ])
  })

  it('그 세션의 ended 로 끈다 · 다른 세션의 ended 로는 끄지 않는다', async () => {
    await watched()
    emit(EVT_DRAG, drag(6, 'active'))
    emit(EVT_DRAG, drag(5, 'ended'))
    key('a')
    emit(EVT_DRAG, drag(6, 'ended'))
    const after = key('b')
    await ticks()
    expect(ends()).toEqual([{ args: { session: 6, how: 'drop', src: 'key', at: null } }])
    expect(after.defaultPrevented).toBe(false)
  })

  it('window capture 에서 키를 끊으면(탭을 쥔 창의 훅) 감시는 그 키를 보내지 않는다', async () => {
    await watched()
    emit(EVT_DRAG, drag(7, 'active'))
    const claim = (e: Event): void => e.stopPropagation()
    window.addEventListener('keydown', claim, true)
    try {
      key('Escape')
      await ticks()
      expect(ends()).toEqual([])
    } finally {
      window.removeEventListener('keydown', claim, true)
    }
  })

  it('tab_drag_end 실패는 경고 로그만 — 다음 키는 그대로 보낸다', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    await watched()
    invokeMock.mockRejectedValueOnce(new Error('gone'))
    emit(EVT_DRAG, drag(8, 'active'))
    key('a')
    key('b')
    await ticks()
    expect(ends()).toHaveLength(2)
    expect(warn).toHaveBeenCalledWith(`[dockSession] ${CMD_TAB_DRAG_END} 실패:`, expect.any(Error))
  })
})

describe('keyEnd', () => {
  it('Escape = cancel · 그 밖 = drop · at 은 그대로 싣는다', () => {
    expect(keyEnd(2, 'Escape', 1)).toEqual({ session: 2, how: 'cancel', src: 'key', at: 1 })
    expect(keyEnd(2, 'Enter', 0)).toEqual({ session: 2, how: 'drop', src: 'key', at: 0 })
    expect(keyEnd(2, 'Alt', null)).toEqual({ session: 2, how: 'drop', src: 'key', at: null })
  })
})

describe('구독 — subscribeFeed · subscribeDrag', () => {
  const attach: TabDragFeed = {
    session: 3,
    view: 'v-1',
    step: { type: 'attach', zone: { x0: 0, y0: 0, x1: 400, y1: 36 }, grab: { x_ratio: 0.5, y: 10, single_x: 80 }, x: 120, y: 14 },
  }

  it('dock:feed 의 몸을 그대로 넘긴다 — 해제는 각자 · 같은 함수를 두 번 걸어도 따로', async () => {
    await watched()
    const cb = vi.fn()
    const off1 = subscribeFeed(cb)
    const off2 = subscribeFeed(cb)
    unsubs.push(off1, off2)

    emit(EVT_FEED, attach)
    expect(cb).toHaveBeenCalledTimes(2)
    expect(cb).toHaveBeenCalledWith(attach)

    off1()
    emit(EVT_FEED, { session: 3, view: 'v-1', step: { type: 'move', x: 130, y: 14 } })
    expect(cb).toHaveBeenCalledTimes(3)

    off2()
    off2()
    emit(EVT_FEED, { session: 3, view: 'v-1', step: { type: 'release' } })
    expect(cb).toHaveBeenCalledTimes(3)
  })

  it('dock:drag 의 몸을 키 감시를 켜고 끈 뒤에 넘긴다', async () => {
    await watched()
    const seen: Array<[TabDragEvent, number]> = []
    unsubs.push(
      subscribeDrag(ev => {
        // 넘길 때 감시가 이미 바뀌어 있다 — active 면 이 자리의 키가 끝을 보낸다.
        key('x')
        seen.push([ev, ends().length])
      }),
    )
    emit(EVT_DRAG, drag(9, 'active'))
    emit(EVT_DRAG, drag(9, 'ended'))
    expect(seen).toEqual([
      [drag(9, 'active'), 1],
      [drag(9, 'ended'), 1],
    ])
  })
})
