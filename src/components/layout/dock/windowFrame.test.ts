// TRD S21-slot-docking §3-8: 창 사실 받기 — 구독 뒤 당기기 · 사건이 당기기 답을 이김 · 깨진 몸 거름 · 떼기 · Tauri `target`.

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

const PULLED: WindowFrame = { restored: { w: 1200, h: 800 }, maximized: false, insets: { t: 31, r: 8, b: 8, l: 8 } }
const PUSHED: WindowFrame = { restored: { w: 1000, h: 700 }, maximized: true, insets: { t: 31, r: 8, b: 8, l: 8 } }

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

  it('구독 등록이 끝난 뒤에 당긴다 · 답이 값이 된다', async () => {
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

  it('사건이 마지막 값을 바꾼다 · 당기기 뒤에 온 사건은 늦게 닿은 답이 덮지 않는다', async () => {
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

  it('모양이 계약과 다른 몸은 버린다(사건 · 답)', async () => {
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
    ]) {
      f.push(bad)
    }
    expect(src.current()).toBeNull()
    expect(warn).toHaveBeenCalledTimes(5)
  })

  it('떼면 등록을 풀고 그 뒤 값을 바꾸지 않는다 · 등록 전에 떼면 늦은 등록을 곧바로 푼다', async () => {
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

  it('Tauri 어댑터 — 자기 창 label 을 타깃으로 듣고 `window_frame` 을 당긴다', async () => {
    const off = windowFrame.install()
    await flush()
    expect(listenMock).toHaveBeenCalledWith(EVT_WINDOW_FRAME, expect.any(Function), { target: 'slot-popup-3' })
    expect(invokeMock).toHaveBeenCalledWith(CMD_WINDOW_FRAME, undefined)
    off()
  })
})
