// TRD S21-slot-docking §3-8: receiving window facts — pull after subscribing · an event beats the pull answer · filtering a broken body · detach · change notice · Tauri `target`.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const listenMock = vi.hoisted(() => vi.fn(async (..._args: unknown[]) => () => {}))
const invokeMock = vi.hoisted(() => vi.fn(async (..._args: unknown[]) => null))
vi.mock('@tauri-apps/api/event', () => ({ listen: listenMock }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ label: 'slot-popup-3' }) }))

import { deferred } from '../../../api/testing/fakeRestoreIpc'
import {
  CMD_WINDOW_FRAME,
  EVT_WINDOW_FRAME,
  type WindowFrame,
  type WindowFrameIpc,
  createWindowFrame,
  windowFrame,
} from './windowFrame'

const CAPTION = { buttons: true, leading_inset: 0, snap_overlay: false }
const PULLED: WindowFrame = {
  restored: { w: 1200, h: 800 },
  maximized: false,
  insets: { t: 31, r: 8, b: 8, l: 8 },
  focused: true,
  caption: CAPTION,
}
const PUSHED: WindowFrame = {
  restored: { w: 1000, h: 700 },
  maximized: true,
  insets: { t: 31, r: 8, b: 8, l: 8 },
  focused: false,
  caption: CAPTION,
}

function fakeIpc() {
  let handler: ((payload: unknown) => void) | undefined
  const reg = deferred<() => void>()
  const pull = deferred<unknown>()
  const unlisten = vi.fn()
  const ipc: WindowFrameIpc = {
    listen: vi.fn((_event: string, h: (payload: unknown) => void) => {
      handler = h
      return reg.promise
    }),
    invoke: vi.fn(() => pull.promise) as unknown as WindowFrameIpc['invoke'],
  }
  return {
    ipc,
    unlisten,
    registered: () => reg.resolve(unlisten),
    answer: (raw: unknown) => pull.resolve(raw),
    push: (raw: unknown) => handler?.(raw),
  }
}

const flush = () => new Promise(r => setTimeout(r, 0))

describe('windowFrame', () => {
  let warn: ReturnType<typeof vi.spyOn>
  beforeEach(() => {
    warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
  })
  afterEach(() => warn.mockRestore())

  it('pulls after subscription registration ends · the answer becomes the value', async () => {
    const f = fakeIpc()
    const src = createWindowFrame(f.ipc)
    src.install()
    await flush()
    expect(f.ipc.invoke).not.toHaveBeenCalled()
    f.registered()
    await flush()
    expect(f.ipc.invoke).toHaveBeenCalledWith(CMD_WINDOW_FRAME)
    expect(src.current()).toBeNull()
    f.answer(PULLED)
    await flush()
    expect(src.current()).toEqual(PULLED)
  })

  it('an event changes the last value · an answer arriving late does not overwrite an event that came after the pull', async () => {
    const f = fakeIpc()
    const src = createWindowFrame(f.ipc)
    src.install()
    f.registered()
    await flush()
    f.push(PUSHED)
    f.answer(PULLED)
    await flush()
    expect(src.current()).toEqual(PUSHED)
    f.push({ ...PUSHED, maximized: false, restored: null })
    expect(src.current()).toEqual({ ...PUSHED, maximized: false, restored: null })
  })

  it('a body whose shape differs from the contract is dropped (event · answer)', async () => {
    const f = fakeIpc()
    const src = createWindowFrame(f.ipc)
    src.install()
    f.registered()
    await flush()
    f.answer({ restored: { w: 1 }, maximized: false, insets: PULLED.insets })
    await flush()
    expect(src.current()).toBeNull()
    for (const bad of [
      null,
      { ...PUSHED, maximized: 'yes' },
      { ...PUSHED, insets: { t: 1, r: 1, b: 1 } },
      { ...PUSHED, restored: { w: 'a', h: 1 } },
      { ...PUSHED, focused: 'yes' },
      { ...PUSHED, caption: undefined },
      { ...PUSHED, caption: { buttons: true, leading_inset: 0 } },
    ]) {
      f.push(bad)
    }
    expect(src.current()).toBeNull()
    expect(warn).toHaveBeenCalledTimes(8)
  })

  it('on detach it releases the registration and does not pass on later events', async () => {
    const f = fakeIpc()
    const src = createWindowFrame(f.ipc)
    const off = src.install()
    f.registered()
    await flush()
    off()
    expect(f.unlisten).toHaveBeenCalledTimes(1)
    f.push(PUSHED)
    f.answer(PULLED)
    await flush()
    expect(src.current()).toBeNull()

    const g = fakeIpc()
    const off2 = createWindowFrame(g.ipc).install()
    off2()
    g.registered()
    await flush()
    expect(g.unlisten).toHaveBeenCalledTimes(1)
    expect(g.ipc.invoke).not.toHaveBeenCalled()
  })

  it('subscribers are called for every received value (answer · event) and not for a dropped body · current is the same reference meanwhile · not called after detach', async () => {
    const f = fakeIpc()
    const src = createWindowFrame(f.ipc)
    const seen: (WindowFrame | null)[] = []
    const off = src.subscribe(() => seen.push(src.current()))
    src.install()
    f.registered()
    await flush()
    f.answer(PULLED)
    await flush()
    expect(seen).toEqual([PULLED])
    expect(src.current()).toBe(src.current())

    f.push({ ...PUSHED, focused: 'yes' })
    f.push(PUSHED)
    expect(seen).toEqual([PULLED, PUSHED])

    off()
    f.push({ ...PUSHED, maximized: false })
    expect(seen).toHaveLength(2)
  })

  it('Tauri adapter — listens with its own window label as the target and pulls `window_frame`', async () => {
    const off = windowFrame.install()
    await flush()
    expect(listenMock).toHaveBeenCalledWith(EVT_WINDOW_FRAME, expect.any(Function), { target: 'slot-popup-3' })
    expect(invokeMock).toHaveBeenCalledWith(CMD_WINDOW_FRAME, undefined)
    off()
  })
})
