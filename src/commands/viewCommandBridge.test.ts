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
  if (!handler) throw new Error(`${EVT} 리스너가 없다`)
  handler({ payload })
}

function outcomeOf(): Record<string, unknown> {
  const call = invokeMock.mock.calls.find(([cmd]) => cmd === 'report_command_outcome')
  if (!call) throw new Error('결말 회신이 없다')
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

describe('offeredCommands — help 를 가진 것만 밖으로 나간다', () => {
  it('help 있는 것만 골라 {name, help} 로 투영한다', () => {
    register({
      id: 'tab.next',
      title: 'next',
      help: {
        summary: '다음 탭으로 옮긴다',
        effect: 'write',
        args: { window: { type: 'string' } },
        required: ['window'],
      },
      run: () => {},
    })
    register({ id: 'slot.focus', title: 'focus', run: () => {} })

    expect(offeredCommands()).toEqual([
      {
        name: 'tab.next',
        help: {
          summary: '다음 탭으로 옮긴다',
          effect: 'write',
          args: { window: { type: 'string' } },
          required: ['window'],
        },
      },
    ])
  })

  it('summary 가 공백뿐이면 내놓지 않는다 — 이름만으로는 부를 수 없다', () => {
    register({ id: 'tab.next', title: 'next', help: { summary: '   ', effect: 'write' }, run: () => {} })
    expect(offeredCommands()).toEqual([])
  })
})

describe('installViewCommandBridge — 봉투가 registry 를 지나 답으로 돌아간다', () => {
  // ★★Do not delete this assertion★★: if `target` is missing `listen()` registers as `Any`, and Tauri wakes **every**
  //   `Any` listener **regardless of the filter** — even if the shell picks one window and sends, main · tree · popout all receive it,
  //   each runs the same command and each answers (one reply per envelope — a TRD §4-⑤ violation).
  it('자기 창 label 로 구독한다 — 안 그러면 창 수만큼 같은 명령이 실행된다', async () => {
    installViewCommandBridge()
    await flush()
    expect(listenOptions).toEqual([{ target: 'popup-7' }])
  })

  it('구독을 건 뒤에 보고한다 — 순서가 뒤집히면 첫 봉투가 이 창에 도착조차 안 한다', async () => {
    register({ id: 'slot.empty', title: 'empty', help: { summary: '슬롯을 비운다', effect: 'write' }, run: () => {} })
    installViewCommandBridge()
    await flush()

    expect(listenMock).toHaveBeenCalledWith(EVT, expect.any(Function))
    const [cmd, args] = invokeMock.mock.calls[0]
    expect(cmd).toBe('report_view_commands')
    expect(args).toEqual({
      commands: [{ name: 'slot.empty', help: { summary: '슬롯을 비운다', effect: 'write' } }],
    })
    // Subscription before the report — invoke goes out only after listen resolves.
    expect(listenMock.mock.invocationCallOrder[0]).toBeLessThan(
      invokeMock.mock.invocationCallOrder[0],
    )
  })

  it('봉투 → run(id, args) → 성공 결말(같은 상관 키)', async () => {
    const spy = vi.fn((args?: Record<string, unknown>) => ({ got: args?.window }))
    register({ id: 'tab.next', title: 'next', help: { summary: '다음 탭', effect: 'write' }, run: spy })
    installViewCommandBridge()
    await flush()

    deliver({ request_id: 'req-1', name: 'tab.next', args: { window: 'main' } })
    await flush()

    expect(spy).toHaveBeenCalledWith({ window: 'main' })
    expect(outcomeOf()).toEqual({ requestId: 'req-1', ok: { got: 'main' }, error: null })
  })

  it('handler 가 던지면 실패 결말을 낸다 — 답 없이 끝나면 셸이 마감까지 매단다', async () => {
    register({
      id: 'slot.empty',
      title: 'empty',
      help: { summary: '슬롯 비우기', effect: 'write' },
      run: () => {
        throw new Error('[slot.empty] viewId 필요')
      },
    })
    installViewCommandBridge()
    await flush()

    deliver({ request_id: 'req-2', name: 'slot.empty', args: {} })
    await flush()

    expect(outcomeOf()).toEqual({
      requestId: 'req-2',
      ok: null,
      error: '[slot.empty] viewId 필요',
    })
  })

  it('모르는 이름도 답으로 끝난다(registry 가 throw 하고 그것이 결말이 된다)', async () => {
    installViewCommandBridge()
    await flush()

    deliver({ request_id: 'req-3', name: 'nope.nope', args: {} })
    await flush()

    const outcome = outcomeOf()
    expect(outcome.requestId).toBe('req-3')
    expect(String(outcome.error)).toContain('nope.nope')
  })

  it('정리한 뒤에 온 봉투는 답하지 않는다 — 죽은 인스턴스가 답하면 한 봉투에 답이 둘이 된다', async () => {
    register({ id: 'slot.empty', title: 'empty', help: { summary: '슬롯을 비운다', effect: 'write' }, run: () => {} })
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
