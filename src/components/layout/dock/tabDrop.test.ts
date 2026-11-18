// TRD S21-slot-docking §3-1 · §4-6: `tab_drop` wrapper · `tab_drop:done` listening — fake IPC (ADR-0012) + the `target` of the Tauri adapter.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const listenMock = vi.hoisted(() => vi.fn(async (..._args: unknown[]) => () => {}))
const invokeMock = vi.hoisted(() => vi.fn(async (..._args: unknown[]) => ({ type: 'duplicate' })))
vi.mock('@tauri-apps/api/event', () => ({ listen: listenMock }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ label: 'slot-popup-3' }) }))

import { CMD_TAB_DROP, EVT_TAB_DROP_DONE, type TabDrop, type TabDropIpc, createTabDropClient, tabDropClient } from './tabDrop'

const DROP: TabDrop = {
  drag: 'drag-1',
  view: 'view-1',
  from: 'main',
  initial: 'view-0',
  to: { type: 'new_window', by: { type: 'drag_end', effect: 'none' }, size: { w: 800, h: 600 }, grab: { x: 40, y: 14 } },
}

/** Fake IPC — registration is held until the test releases it (`release`) · events are issued with `emit`. */
function fakeIpc() {
  const handlers = new Map<string, (payload: unknown) => void>()
  const unlisten = vi.fn()
  let release: () => void = () => {}
  const ipc: TabDropIpc = {
    invoke: vi.fn(async () => ({ type: 'applied', window: 'main', window_id: 'main', number: 1, reused: false })) as unknown as TabDropIpc['invoke'],
    listen: vi.fn(
      (event: string, handler: (payload: unknown) => void) =>
        new Promise<() => void>(resolve => {
          release = () => {
            handlers.set(event, handler)
            resolve(unlisten)
          }
        }),
    ),
  }
  return {
    ipc,
    unlisten,
    release: () => release(),
    emit: (event: string, payload: unknown) => handlers.get(event)?.(payload),
  }
}

const flush = () => new Promise(r => setTimeout(r, 0))

describe('tabDrop — send', () => {
  it('calls `tab_drop` with `{ drop }` and returns the answer as is', async () => {
    const f = fakeIpc()
    const reply = await createTabDropClient(f.ipc).send(DROP)
    expect(f.ipc.invoke).toHaveBeenCalledWith(CMD_TAB_DROP, { drop: DROP })
    expect(reply).toEqual({ type: 'applied', window: 'main', window_id: 'main', number: 1, reused: false })
  })

  it('on an IPC error it rejects (the caller releases the swapped drawing at once)', async () => {
    const f = fakeIpc()
    vi.mocked(f.ipc.invoke).mockRejectedValueOnce('ipc down')
    await expect(createTabDropClient(f.ipc).send(DROP)).rejects.toBe('ipc down')
  })
})

describe('tabDrop — onDone', () => {
  let warn: ReturnType<typeof vi.spyOn>
  beforeEach(() => {
    warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
  })
  afterEach(() => warn.mockRestore())

  it('passes on only a done of the right shape and drops a broken body after a warning', async () => {
    const f = fakeIpc()
    const seen: unknown[] = []
    createTabDropClient(f.ipc).onDone(d => seen.push(d))
    f.release()
    await flush()
    f.emit(EVT_TAB_DROP_DONE, { drag: 'drag-1', outcome: 'applied' })
    f.emit(EVT_TAB_DROP_DONE, { drag: 'drag-1', outcome: 'duplicate' })
    f.emit(EVT_TAB_DROP_DONE, { drag: 7, outcome: 'ignored' })
    f.emit(EVT_TAB_DROP_DONE, null)
    expect(seen).toEqual([{ drag: 'drag-1', outcome: 'applied' }])
    expect(warn).toHaveBeenCalledTimes(3)
  })

  it('on detach it releases the registration and does not pass on later events', async () => {
    const f = fakeIpc()
    const seen: unknown[] = []
    const off = createTabDropClient(f.ipc).onDone(d => seen.push(d))
    f.release()
    await flush()
    off()
    expect(f.unlisten).toHaveBeenCalledTimes(1)
    f.emit(EVT_TAB_DROP_DONE, { drag: 'drag-1', outcome: 'canceled' })
    expect(seen).toEqual([])
  })

  it('if detached before registration ends it releases the late-resolving registration at once', async () => {
    const f = fakeIpc()
    const off = createTabDropClient(f.ipc).onDone(() => {})
    off()
    f.release()
    await flush()
    expect(f.unlisten).toHaveBeenCalledTimes(1)
  })

  it('if registration is refused it hooks again', async () => {
    vi.useFakeTimers()
    try {
      const f = fakeIpc()
      const real = f.ipc.listen
      vi.mocked(f.ipc.listen).mockRejectedValueOnce(new Error('not ready'))
      const seen: unknown[] = []
      createTabDropClient(f.ipc).onDone(d => seen.push(d))
      await vi.advanceTimersByTimeAsync(200)
      expect(real).toHaveBeenCalledTimes(2)
      f.release()
      await vi.advanceTimersByTimeAsync(0)
      f.emit(EVT_TAB_DROP_DONE, { drag: 'drag-2', outcome: 'rejected' })
      expect(seen).toEqual([{ drag: 'drag-2', outcome: 'rejected' }])
    } finally {
      vi.useRealTimers()
    }
  })
})

describe('tabDrop — Tauri adapter', () => {
  it('done is listened to with its own window label as the target (TRD §2-2)', async () => {
    const off = tabDropClient.onDone(() => {})
    await flush()
    expect(listenMock).toHaveBeenCalledWith(EVT_TAB_DROP_DONE, expect.any(Function), { target: 'slot-popup-3' })
    off()
  })

  it('send goes by Tauri invoke', async () => {
    await tabDropClient.send(DROP)
    expect(invokeMock).toHaveBeenCalledWith(CMD_TAB_DROP, { drop: DROP })
  })
})
