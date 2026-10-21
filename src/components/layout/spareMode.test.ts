// spareMode 단위테스트 — 듣기(자기 창 대상 · `window_role` 보다 먼저) · `window_role` 답 → 모드 · `dock:spare` 바뀜 · 정리.
//   창 모드가 화면에 닿는 것은 `WindowLayout.test.tsx` 의 대기 모드 시험이 잰다.

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

import { CMD_WINDOW_ROLE, EVT_SPARE, watchSpareMode, type WindowMode } from './spareMode'

const ticks = async (): Promise<void> => {
  for (let i = 0; i < 20; i++) await Promise.resolve()
}

function emit(event: string, payload: unknown): void {
  const l = listeners.get(event)
  if (!l) throw new Error(`${event} 구독 없음`)
  l.handler({ payload })
}

let modes: WindowMode[]
let stop: (() => void) | null

async function watched(): Promise<void> {
  stop = watchSpareMode(m => modes.push(m))
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
})
afterEach(() => {
  stop?.()
  vi.restoreAllMocks()
})

describe('watchSpareMode — 듣기 · 창 모드', () => {
  it('dock:spare 하나만 자기 창 대상으로 건 뒤 window_role 을 묻는다', async () => {
    await watched()
    expect(order).toEqual([`listen:${EVT_SPARE}`, `invoke:${CMD_WINDOW_ROLE}`])
    expect(listeners.get(EVT_SPARE)?.options).toEqual({ target: LABEL })
    expect(modes).toEqual(['normal'])
  })

  it('듣기가 걸린 뒤에야 window_role 을 묻는다', async () => {
    let finish: (off: () => void) => void = () => {}
    listenMock.mockImplementationOnce(async (event: string, handler: (e: { payload: unknown }) => void, options?: unknown) => {
      order.push(`listen:${event}`)
      listeners.set(event, { handler, options })
      return new Promise<() => void>(res => {
        finish = res
      })
    })
    stop = watchSpareMode(m => modes.push(m))
    await ticks()
    expect(order).toEqual([`listen:${EVT_SPARE}`])
    finish(() => {})
    await ticks()
    expect(order).toEqual([`listen:${EVT_SPARE}`, `invoke:${CMD_WINDOW_ROLE}`])
    expect(modes).toEqual(['normal'])
  })

  const REPLIES: { name: string; reply: () => Promise<unknown>; out: WindowMode; warns: boolean }[] = [
    { name: 'parked → parked', reply: async () => ({ role: 'parked', drag: null }), out: 'parked', warns: false },
    { name: 'model → normal', reply: async () => ({ role: 'model', drag: null }), out: 'normal', warns: false },
    { name: 'unknown → normal', reply: async () => ({ role: 'unknown', drag: null }), out: 'normal', warns: false },
    {
      name: '실패 → normal(경고 로그)',
      reply: async () => {
        throw new Error('gone')
      },
      out: 'normal',
      warns: true,
    },
  ]
  for (const c of REPLIES) {
    it(`window_role 답 ${c.name}`, async () => {
      const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
      invokeMock.mockImplementation(c.reply)
      await watched()
      expect(modes).toEqual([c.out])
      if (c.warns) {
        expect(warn).toHaveBeenCalledWith(`[spareMode] ${CMD_WINDOW_ROLE} 실패 — 보통으로 그린다:`, expect.any(Error))
      } else {
        expect(warn).not.toHaveBeenCalled()
      }
    })
  }

  it('window_role = parked → parked · 그 뒤 dock:spare 로 바뀜을 알린다', async () => {
    invokeMock.mockImplementation(async () => ({ role: 'parked', drag: null }))
    await watched()
    expect(modes).toEqual(['parked'])
    emit(EVT_SPARE, { step: 'adopted' })
    emit(EVT_SPARE, { step: 'parked' })
    expect(modes).toEqual(['parked', 'normal', 'parked'])
  })

  it('dock:spare 구독이 실패해도(오류 로그) window_role 은 묻는다', async () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    listenMock.mockImplementationOnce(async () => {
      throw new Error('listen failed')
    })
    await watched()
    expect(error).toHaveBeenCalledWith(
      `[spareMode] ${EVT_SPARE} 구독 실패 — 이 창은 대기 모드 바뀜을 못 받는다:`,
      expect.any(Error),
    )
    expect(order).toEqual([`invoke:${CMD_WINDOW_ROLE}`])
    expect(modes).toEqual(['normal'])
  })

  it('정리하면 듣기를 떼고 그 뒤 사건에 반응하지 않는다', async () => {
    await watched()
    stop?.()
    stop = null
    expect(unlistened).toEqual([EVT_SPARE])

    emit(EVT_SPARE, { step: 'parked' })
    await ticks()
    expect(modes).toEqual(['normal'])
  })

  it('window_role 답이 정리 뒤에 오면 알리지 않는다', async () => {
    let answer: (v: unknown) => void = () => {}
    invokeMock.mockImplementation(
      () =>
        new Promise(res => {
          answer = res
        }),
    )
    stop = watchSpareMode(m => modes.push(m))
    await ticks()
    stop()
    stop = null
    answer({ role: 'parked', drag: null })
    await ticks()
    expect(modes).toEqual([])
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
    stop = watchSpareMode(m => modes.push(m))
    stop()
    stop = null
    finish(offLate)
    await ticks()
    expect(offLate).toHaveBeenCalledTimes(1)
    expect(order).not.toContain(`invoke:${CMD_WINDOW_ROLE}`)
    expect(modes).toEqual([])
  })
})
