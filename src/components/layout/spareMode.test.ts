// spareMode 단위테스트 — 듣기(자기 창 대상 · `window_role` 보다 먼저) · `window_role` 답 → 모드 · `spare:adopted` 바뀜 · 한 방향 ·
//   듣기 실패(`adoptable: false`) · 채택 번호 · 정리.
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

import { CMD_WINDOW_ROLE, EVT_SPARE_ADOPTED, watchSpareMode, type ModeChange } from './spareMode'

const ticks = async (): Promise<void> => {
  for (let i = 0; i < 20; i++) await Promise.resolve()
}

function emit(event: string, payload: unknown): void {
  const l = listeners.get(event)
  if (!l) throw new Error(`${event} 구독 없음`)
  l.handler({ payload })
}

const normal = (generation: number | null = null): ModeChange => ({ mode: 'normal', generation })
const SPARE: ModeChange = { mode: 'spare', generation: null }

let modes: ModeChange[]
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
      return { role: 'model', generation: null }
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
  it('spare:adopted 하나만 자기 창 대상으로 건 뒤 window_role 을 adoptable: true 로 묻는다', async () => {
    await watched()
    expect(order).toEqual([`listen:${EVT_SPARE_ADOPTED}`, `invoke:${CMD_WINDOW_ROLE}`])
    expect(listeners.get(EVT_SPARE_ADOPTED)?.options).toEqual({ target: LABEL })
    expect(invokeMock).toHaveBeenCalledWith(CMD_WINDOW_ROLE, { adoptable: true })
    expect(modes).toEqual([normal()])
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
    expect(order).toEqual([`listen:${EVT_SPARE_ADOPTED}`])
    finish(() => {})
    await ticks()
    expect(order).toEqual([`listen:${EVT_SPARE_ADOPTED}`, `invoke:${CMD_WINDOW_ROLE}`])
    expect(modes).toEqual([normal()])
  })

  const REPLIES: { name: string; reply: () => Promise<unknown>; out: ModeChange; warns: boolean }[] = [
    { name: 'spare → spare', reply: async () => ({ role: 'spare', generation: null }), out: SPARE, warns: false },
    { name: 'model → normal', reply: async () => ({ role: 'model', generation: null }), out: normal(), warns: false },
    { name: 'model + 번호 → normal + 그 번호', reply: async () => ({ role: 'model', generation: 5 }), out: normal(5), warns: false },
    { name: 'model + 번호 0 → normal + 0', reply: async () => ({ role: 'model', generation: 0 }), out: normal(0), warns: false },
    { name: 'unknown → normal', reply: async () => ({ role: 'unknown', generation: null }), out: normal(), warns: false },
    {
      name: '실패 → normal(경고 로그)',
      reply: async () => {
        throw new Error('gone')
      },
      out: normal(),
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

  it('window_role = spare → spare · 그 뒤 spare:adopted 마다 normal + 그 번호를 알린다', async () => {
    invokeMock.mockImplementation(async () => ({ role: 'spare', generation: null }))
    await watched()
    expect(modes).toEqual([SPARE])
    emit(EVT_SPARE_ADOPTED, { generation: 3 })
    emit(EVT_SPARE_ADOPTED, { generation: 0 })
    expect(modes).toEqual([SPARE, normal(3), normal(0)])
  })

  // 몸이 깨져도 채택은 보통 모드로 받는다 — 번호만 잃는다.
  it('번호가 없는 spare:adopted 몸도 normal 이다(번호 null)', async () => {
    invokeMock.mockImplementation(async () => ({ role: 'spare', generation: null }))
    await watched()
    emit(EVT_SPARE_ADOPTED, null)
    emit(EVT_SPARE_ADOPTED, { generation: '3' })
    expect(modes).toEqual([SPARE, normal(), normal()])
  })

  it('정리하면 듣기를 떼고 그 뒤 사건에 반응하지 않는다', async () => {
    await watched()
    stop?.()
    stop = null
    expect(unlistened).toEqual([EVT_SPARE_ADOPTED])

    emit(EVT_SPARE_ADOPTED, { generation: 1 })
    await ticks()
    expect(modes).toEqual([normal()])
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
    answer({ role: 'spare', generation: null })
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

// ── 한 방향(TRD S21-slot-docking §3-9 · §9-2) — 답과 사건은 다른 길로 와 차례가 없다 ──
describe('watchSpareMode — 모드는 한 방향', () => {
  function pendingRole(): (reply: unknown) => void {
    let answer: (reply: unknown) => void = () => {}
    invokeMock.mockImplementation(
      () =>
        new Promise(res => {
          answer = res
        }),
    )
    return reply => answer(reply)
  }

  it('채택을 본 뒤 늦게 온 window_role 답 spare 는 대기로 되돌리지 않는다', async () => {
    const answer = pendingRole()
    await watched()
    emit(EVT_SPARE_ADOPTED, { generation: 4 })
    answer({ role: 'spare', generation: null })
    await ticks()
    expect(modes).toEqual([normal(4)])
  })

  it('채택을 본 뒤 늦게 온 답 model(+ 번호)도 다시 알리지 않는다', async () => {
    const answer = pendingRole()
    await watched()
    emit(EVT_SPARE_ADOPTED, { generation: 4 })
    answer({ role: 'model', generation: 4 })
    await ticks()
    expect(modes).toEqual([normal(4)])
  })

  it('답 model + 번호 뒤에 온 사건은 같은 번호로 다시 알린다(같은 값 거듭)', async () => {
    invokeMock.mockImplementation(async () => ({ role: 'model', generation: 6 }))
    await watched()
    emit(EVT_SPARE_ADOPTED, { generation: 6 })
    expect(modes).toEqual([normal(6), normal(6)])
  })
})

// ── 듣기 실패(I28) — 듣기 없는 대기 창은 채택돼도 빈 채 남는다 ──
describe('watchSpareMode — spare:adopted 듣기 실패', () => {
  function failListen() {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    listenMock.mockImplementationOnce(async () => {
      throw new Error('listen failed')
    })
    return error
  }

  it('오류 로그 · window_role 을 adoptable: false 로 묻는다', async () => {
    const error = failListen()
    await watched()
    expect(error).toHaveBeenCalledWith(
      `[spareMode] ${EVT_SPARE_ADOPTED} 구독 실패 — 대기 모드로 들지 않는다(adoptable: false):`,
      expect.any(Error),
    )
    expect(order).toEqual([`invoke:${CMD_WINDOW_ROLE}`])
    expect(invokeMock).toHaveBeenCalledWith(CMD_WINDOW_ROLE, { adoptable: false })
    expect(modes).toEqual([normal()])
  })

  const REPLIES: { name: string; reply: unknown; out: ModeChange }[] = [
    { name: 'spare 가 와도 normal', reply: { role: 'spare', generation: null }, out: normal() },
    { name: 'unknown(셸이 풀의 창을 닫음) → normal', reply: { role: 'unknown', generation: null }, out: normal() },
    { name: 'model + 번호 → normal + 그 번호', reply: { role: 'model', generation: 2 }, out: normal(2) },
  ]
  for (const c of REPLIES) {
    it(`답 ${c.name}`, async () => {
      failListen()
      invokeMock.mockImplementation(async () => c.reply)
      await watched()
      expect(modes).toEqual([c.out])
    })
  }

  // 정리된 감시가 묻으면 StrictMode 의 두 번째 감시가 듣기를 걸었어도 셸이 그 창을 닫는다.
  it('정리 뒤에 듣기가 실패하면 window_role 을 묻지 않는다', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    let fail: (err: Error) => void = () => {}
    listenMock.mockImplementationOnce(
      () =>
        new Promise<() => void>((_res, rej) => {
          fail = rej
        }),
    )
    stop = watchSpareMode(m => modes.push(m))
    stop()
    stop = null
    fail(new Error('listen failed'))
    await ticks()
    expect(invokeMock).not.toHaveBeenCalled()
    expect(modes).toEqual([])
  })
})
