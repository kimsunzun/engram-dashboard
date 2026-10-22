// TRD S21-slot-docking §3-1 · §4-6: `tab_drop` 감싸개 · `tab_drop:done` 듣기 — 가짜 IPC(ADR-0012) + Tauri 어댑터의 `target`.

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

/** 가짜 IPC — 등록은 시험이 놓을 때까지 붙든다(`release`) · 사건은 `emit` 으로 낸다. */
function fakeIpc() {
  const handlers = new Map<string, (payload: unknown) => void>()
  const unlisten = vi.fn()
  let release: () => void = () => {}
  const ipc: TabDropIpc = {
    invoke: vi.fn(async () => ({ type: 'applied', window: 'main', reused: false })) as unknown as TabDropIpc['invoke'],
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
  it('`tab_drop` 을 `{ drop }` 으로 부르고 답을 그대로 돌려준다', async () => {
    const f = fakeIpc()
    const reply = await createTabDropClient(f.ipc).send(DROP)
    expect(f.ipc.invoke).toHaveBeenCalledWith(CMD_TAB_DROP, { drop: DROP })
    expect(reply).toEqual({ type: 'applied', window: 'main', reused: false })
  })

  it('IPC 오류면 거절한다(부르는 쪽이 바꿔 그리기를 곧바로 푼다)', async () => {
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

  it('모양이 맞는 done 만 넘기고 깨진 몸은 경고 후 버린다', async () => {
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

  it('떼면 등록을 풀고 그 뒤 사건은 넘기지 않는다', async () => {
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

  it('등록이 끝나기 전에 떼면 늦게 풀린 등록을 곧바로 푼다', async () => {
    const f = fakeIpc()
    const off = createTabDropClient(f.ipc).onDone(() => {})
    off()
    f.release()
    await flush()
    expect(f.unlisten).toHaveBeenCalledTimes(1)
  })

  it('등록이 거절되면 다시 단다', async () => {
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

describe('tabDrop — Tauri 어댑터', () => {
  it('done 은 자기 창 label 을 타깃으로 듣는다(TRD §2-2)', async () => {
    const off = tabDropClient.onDone(() => {})
    await flush()
    expect(listenMock).toHaveBeenCalledWith(EVT_TAB_DROP_DONE, expect.any(Function), { target: 'slot-popup-3' })
    off()
  })

  it('send 는 Tauri invoke 로 간다', async () => {
    await tabDropClient.send(DROP)
    expect(invokeMock).toHaveBeenCalledWith(CMD_TAB_DROP, { drop: DROP })
  })
})
