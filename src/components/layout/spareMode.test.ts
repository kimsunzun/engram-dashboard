// spareMode unit test — listening (own window target · before `window_role`) · `window_role` answer → mode · `spare:adopted` change · one direction ·
//   listening failure (`adoptable: false`) · adoption number · cleanup.
//   How the window mode reaches the screen is measured by the waiting mode test of `WindowLayout.test.tsx`.

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
  if (!l) throw new Error(`no subscription for ${event}`)
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

describe('watchSpareMode — listening · window mode', () => {
  it('hooks only spare:adopted on its own window, then asks window_role with adoptable: true', async () => {
    await watched()
    expect(order).toEqual([`listen:${EVT_SPARE_ADOPTED}`, `invoke:${CMD_WINDOW_ROLE}`])
    expect(listeners.get(EVT_SPARE_ADOPTED)?.options).toEqual({ target: LABEL })
    expect(invokeMock).toHaveBeenCalledWith(CMD_WINDOW_ROLE, { adoptable: true })
    expect(modes).toEqual([normal()])
  })

  it('asks window_role only after the listening is hooked', async () => {
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
    { name: 'model + number → normal + that number', reply: async () => ({ role: 'model', generation: 5 }), out: normal(5), warns: false },
    { name: 'model + number 0 → normal + 0', reply: async () => ({ role: 'model', generation: 0 }), out: normal(0), warns: false },
    { name: 'unknown → normal', reply: async () => ({ role: 'unknown', generation: null }), out: normal(), warns: false },
    {
      name: 'failure → normal (warning log)',
      reply: async () => {
        throw new Error('gone')
      },
      out: normal(),
      warns: true,
    },
  ]
  for (const c of REPLIES) {
    it(`window_role answer ${c.name}`, async () => {
      const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
      invokeMock.mockImplementation(c.reply)
      await watched()
      expect(modes).toEqual([c.out])
      if (c.warns) {
        expect(warn).toHaveBeenCalledWith(`[spareMode] ${CMD_WINDOW_ROLE} failed — drawing as normal:`, expect.any(Error))
      } else {
        expect(warn).not.toHaveBeenCalled()
      }
    })
  }

  it('window_role = spare → spare · after that on each spare:adopted notifies normal + that number', async () => {
    invokeMock.mockImplementation(async () => ({ role: 'spare', generation: null }))
    await watched()
    expect(modes).toEqual([SPARE])
    emit(EVT_SPARE_ADOPTED, { generation: 3 })
    emit(EVT_SPARE_ADOPTED, { generation: 0 })
    expect(modes).toEqual([SPARE, normal(3), normal(0)])
  })

  // Even if the body is broken, adoption is accepted as normal mode — only the number is lost.
  it('a spare:adopted body without a number is also normal (number null)', async () => {
    invokeMock.mockImplementation(async () => ({ role: 'spare', generation: null }))
    await watched()
    emit(EVT_SPARE_ADOPTED, null)
    emit(EVT_SPARE_ADOPTED, { generation: '3' })
    expect(modes).toEqual([SPARE, normal(), normal()])
  })

  it('on cleanup it detaches the listening and does not react to later events', async () => {
    await watched()
    stop?.()
    stop = null
    expect(unlistened).toEqual([EVT_SPARE_ADOPTED])

    emit(EVT_SPARE_ADOPTED, { generation: 1 })
    await ticks()
    expect(modes).toEqual([normal()])
  })

  it('if the window_role answer comes after cleanup it does not notify', async () => {
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

  it('if the listening is hooked after cleanup it detaches at once and does not ask window_role', async () => {
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

// ── one direction (TRD S21-slot-docking §3-9 · §9-2) — the answer and the event come by different paths and have no order ──
describe('watchSpareMode — the mode is one direction', () => {
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

  it('a late window_role answer spare after seeing adoption does not return it to waiting', async () => {
    const answer = pendingRole()
    await watched()
    emit(EVT_SPARE_ADOPTED, { generation: 4 })
    answer({ role: 'spare', generation: null })
    await ticks()
    expect(modes).toEqual([normal(4)])
  })

  it('a late answer model (+ number) after seeing adoption also does not notify again', async () => {
    const answer = pendingRole()
    await watched()
    emit(EVT_SPARE_ADOPTED, { generation: 4 })
    answer({ role: 'model', generation: 4 })
    await ticks()
    expect(modes).toEqual([normal(4)])
  })

  it('an event after the answer model + number notifies again with the same number (the same value repeated)', async () => {
    invokeMock.mockImplementation(async () => ({ role: 'model', generation: 6 }))
    await watched()
    emit(EVT_SPARE_ADOPTED, { generation: 6 })
    expect(modes).toEqual([normal(6), normal(6)])
  })
})

// ── listening failure (I28) — a waiting window without listening stays empty even when adopted ──
describe('watchSpareMode — spare:adopted listening failure', () => {
  function failListen() {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    listenMock.mockImplementationOnce(async () => {
      throw new Error('listen failed')
    })
    return error
  }

  it('error log · asks window_role with adoptable: false', async () => {
    const error = failListen()
    await watched()
    expect(error).toHaveBeenCalledWith(
      `[spareMode] ${EVT_SPARE_ADOPTED} subscription failed — not entering waiting mode (adoptable: false):`,
      expect.any(Error),
    )
    expect(order).toEqual([`invoke:${CMD_WINDOW_ROLE}`])
    expect(invokeMock).toHaveBeenCalledWith(CMD_WINDOW_ROLE, { adoptable: false })
    expect(modes).toEqual([normal()])
  })

  const REPLIES: { name: string; reply: unknown; out: ModeChange }[] = [
    { name: 'normal even if spare comes', reply: { role: 'spare', generation: null }, out: normal() },
    { name: 'unknown (the shell closed the pool window) → normal', reply: { role: 'unknown', generation: null }, out: normal() },
    { name: 'model + number → normal + that number', reply: { role: 'model', generation: 2 }, out: normal(2) },
  ]
  for (const c of REPLIES) {
    it(`answer ${c.name}`, async () => {
      failListen()
      invokeMock.mockImplementation(async () => c.reply)
      await watched()
      expect(modes).toEqual([c.out])
    })
  }

  // If a cleaned-up watch asks anyway, the shell closes that window even if StrictMode's second watch attached listening.
  it('if the listening fails after cleanup it does not ask window_role', async () => {
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
