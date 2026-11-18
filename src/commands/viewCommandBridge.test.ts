// viewCommandBridge — what is offered to the shell (projection) + whether an envelope that came down passes the registry and returns as an answer.
//
// ★The shell's reserved name filter is not measured here★ — that decision lives once on the Rust side
// (`reserved_names` in `src-tauri/src/view_commands.rs`, the harness is `tests/layout_commands.rs`). Putting a copy here
// makes the two lists diverge. Names the daemon answers are not filtered by that filter either — the wall is the daemon's registration bounce (ADR-0270 decision 2).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invokeMock = vi.fn(async (_cmd: string, _args?: unknown) => undefined as unknown)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}))

// listen mock: keeps the registered handlers so the test imitates the shell delivery directly (the same shape as uiSettings.test).
const listeners = new Map<string, (e: { payload: unknown }) => void>()
const unlistenMock = vi.fn()
const listenMock = vi.fn(async (event: string, handler: (e: { payload: unknown }) => void) => {
  listeners.set(event, handler)
  return unlistenMock
})
const listenOptions: unknown[] = []
vi.mock('@tauri-apps/api/event', () => ({
  listen: (event: string, handler: (e: { payload: unknown }) => void, options?: unknown) => {
    listenOptions.push(options)
    return listenMock(event, handler)
  },
}))

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ label: 'popup-7' }),
}))

import { __resetRegistryForTest, register } from './registry'
import { installViewCommandBridge, offeredCommands } from './viewCommandBridge'

const EVT = 'command:request'

/** Until the microtasks are all drained — invoke · listen are all async so results arrive on the next tick. */
async function flush(): Promise<void> {
  await Promise.resolve()
  await Promise.resolve()
  await Promise.resolve()
}

function deliver(payload: unknown): void {
  const handler = listeners.get(EVT)
  if (!handler) throw new Error(`no ${EVT} listener`)
  handler({ payload })
}

function outcomeOf(): Record<string, unknown> {
  const call = invokeMock.mock.calls.find(([cmd]) => cmd === 'report_command_outcome')
  if (!call) throw new Error('no outcome reply')
  return call[1] as Record<string, unknown>
}

beforeEach(() => {
  __resetRegistryForTest()
  listeners.clear()
  listenOptions.length = 0
  invokeMock.mockClear()
  listenMock.mockClear()
  unlistenMock.mockClear()
})

afterEach(() => {
  vi.restoreAllMocks()
})

describe('offeredCommands — only the ones with help go out', () => {
  it('picks only the ones with help and projects them to {name, help}', () => {
    register({
      id: 'tab.next',
      title: () => 'next',
      help: {
        summary: 'Moves to the next tab',
        effect: 'write',
        args: { window: { type: 'string' } },
        required: ['window'],
      },
      run: () => {},
    })
    register({ id: 'slot.focus', title: () => 'focus', run: () => {} })

    expect(offeredCommands()).toEqual([
      {
        name: 'tab.next',
        help: {
          summary: 'Moves to the next tab',
          effect: 'write',
          args: { window: { type: 'string' } },
          required: ['window'],
        },
      },
    ])
  })

  it('does not offer it when summary is blank only — it cannot be called by name alone', () => {
    register({ id: 'tab.next', title: () => 'next', help: { summary: '   ', effect: 'write' }, run: () => {} })
    expect(offeredCommands()).toEqual([])
  })
})

describe('installViewCommandBridge — the envelope goes through the registry and back as an answer', () => {
  // ★★Do not delete this assertion★★: if `target` is missing `listen()` registers as `Any`, and Tauri wakes **every**
  //   `Any` listener **regardless of the filter** — even if the shell picks one window and sends, main · tree · popout all receive it,
  //   each runs the same command and each answers (one reply per envelope — a TRD §4-⑤ violation).
  it('subscribes with its own window label — otherwise the same command runs as many times as there are windows', async () => {
    installViewCommandBridge()
    await flush()
    expect(listenOptions).toEqual([{ target: 'popup-7' }])
  })

  it('reports after the subscription is set — if the order flips, the first envelope does not even arrive at this window', async () => {
    register({ id: 'slot.empty', title: () => 'empty', help: { summary: 'Empties the slot', effect: 'write' }, run: () => {} })
    installViewCommandBridge()
    await flush()

    expect(listenMock).toHaveBeenCalledWith(EVT, expect.any(Function))
    const [cmd, args] = invokeMock.mock.calls[0]
    expect(cmd).toBe('report_view_commands')
    expect(args).toEqual({
      commands: [{ name: 'slot.empty', help: { summary: 'Empties the slot', effect: 'write' } }],
    })
    // Subscription before the report — invoke goes out only after listen resolves.
    expect(listenMock.mock.invocationCallOrder[0]).toBeLessThan(
      invokeMock.mock.invocationCallOrder[0],
    )
  })

  it('envelope → run(id, args) → success outcome (same correlation key)', async () => {
    const spy = vi.fn((args?: Record<string, unknown>) => ({ got: args?.window }))
    register({ id: 'tab.next', title: () => 'next', help: { summary: 'Next tab', effect: 'write' }, run: spy })
    installViewCommandBridge()
    await flush()

    deliver({ request_id: 'req-1', name: 'tab.next', args: { window: 'main' } })
    await flush()

    expect(spy).toHaveBeenCalledWith({ window: 'main' })
    expect(outcomeOf()).toEqual({ requestId: 'req-1', ok: { got: 'main' }, error: null })
  })

  it('produces a failure outcome when the handler throws — ending without an answer leaves the shell hanging until the deadline', async () => {
    register({
      id: 'slot.empty',
      title: () => 'empty',
      help: { summary: 'Empty the slot', effect: 'write' },
      run: () => {
        throw new Error('[slot.empty] viewId required')
      },
    })
    installViewCommandBridge()
    await flush()

    deliver({ request_id: 'req-2', name: 'slot.empty', args: {} })
    await flush()

    expect(outcomeOf()).toEqual({
      requestId: 'req-2',
      ok: null,
      error: '[slot.empty] viewId required',
    })
  })

  it('an unknown name ends with an answer too (the registry throws and that becomes the outcome)', async () => {
    installViewCommandBridge()
    await flush()

    deliver({ request_id: 'req-3', name: 'nope.nope', args: {} })
    await flush()

    const outcome = outcomeOf()
    expect(outcome.requestId).toBe('req-3')
    expect(String(outcome.error)).toContain('nope.nope')
  })

  it('an envelope that comes after cleanup gets no answer — if a dead instance answers, one envelope gets two answers', async () => {
    register({ id: 'slot.empty', title: () => 'empty', help: { summary: 'Empties the slot', effect: 'write' }, run: () => {} })
    const dispose = installViewCommandBridge()
    await flush()
    dispose()
    invokeMock.mockClear()

    deliver({ request_id: 'req-4', name: 'slot.empty', args: {} })
    await flush()

    expect(unlistenMock).toHaveBeenCalledOnce()
    expect(invokeMock).not.toHaveBeenCalled()
  })
})
