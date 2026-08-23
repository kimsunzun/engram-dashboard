// viewCommandBridge — 무엇을 셸에 내놓나(투영) + 내려온 봉투가 registry 를 지나 답으로 돌아가나.
//
// ★셸의 예약 이름 필터는 여기서 재지 않는다★ — 그 판정은 Rust 쪽에 한 번만 산다
// (`src-tauri/src/view_commands.rs` 의 `reserved_names`, 하네스는 `tests/layout_commands.rs`). 여기 사본을
// 두면 두 목록이 갈리고, 갈린 쪽을 믿는 순간 등록 패킷 하나가 통째로 반려된다.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invokeMock = vi.fn(async (_cmd: string, _args?: unknown) => undefined as unknown)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}))

// listen mock: 등록된 핸들러를 보관해 테스트가 셸 배달을 직접 흉내낸다(uiSettings.test 와 같은 모양).
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

/** 마이크로태스크가 다 풀릴 때까지 — invoke·listen 이 전부 async 라 결과가 다음 tick 에 온다. */
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
  // ★★이 단언을 지우지 말 것★★: `target` 이 빠지면 `listen()` 은 `Any` 로 등록되고, Tauri 는 `Any`
  //   리스너를 **필터와 무관하게 전부** 깨운다 — 셸이 창 하나를 골라 보내도 main·트리·팝아웃이 다 받아
  //   같은 명령을 각자 실행하고 각자 답한다(한 봉투에 답장 하나 — TRD §4-⑤ 위반).
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
    // 보고보다 구독이 먼저 — listen 이 풀린 뒤에야 invoke 가 나간다.
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
