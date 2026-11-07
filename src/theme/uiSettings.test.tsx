// uiSettings — whether the theme the shell gave reaches `data-theme` + ★whether applying it does not remount the tree★.
//
// The latter is the reason this suite exists (ADR-0149): if a slot is remounted the chat is component state, so the conversation is permanently lost.
//
// ★That `Child` **subscribes** to the theme is the premise of that assertion★ — a child that subscribes to nothing does not even
// re-render whatever the implementation does, so mount counts · node identity always pass (i.e. an implementation that remounts is also green). Only with it subscribed
// does a theme change cause a real re-render, and only then does the mount being just once become the measured value.
// We also confirm every time that the subscription is alive (whether the painted text changes) — otherwise this premise quietly rots.
//
// ★What is not measured — the production tree (known gap)★: here `Host`/`Child` are a **synthetic harness**, not the real slot tree.
// So this suite measures only up to "the theme application path does not trigger a remount", and if someone later puts into the production tree a component that subscribes to the theme while
// **also having its key change**, this stays green. To close that gap a test that brings up the real
// slot subtree must stand separately.

import { act, cleanup, render, screen } from '@testing-library/react'
import { useEffect } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invokeMock = vi.fn(async (_cmd: string) => ({ theme: 'dark' }) as unknown)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string) => invokeMock(cmd),
}))

/** A promise that hangs until opened — used by the test to hold the `listen()` registration. */
function deferred(): { promise: Promise<void>; open: () => void } {
  let open: () => void = () => {}
  const promise = new Promise<void>(resolve => {
    open = resolve
  })
  return { promise, open }
}

// listen mock: keeps registered handlers so the test imitates a shell push directly (same shape as viewStore.test).
// While listenGate is set, it holds the registration itself — the means to measure 「등록 전에는 부팅 조회를 안 낸다」 (no boot query is issued before registration).
// ★`options` is passed along too★ — the subscription target (which window's share) is carried in this argument.
const listeners = new Map<string, (e: { payload: unknown }) => void>()
let listenGate: { promise: Promise<void>; open: () => void } | null = null
const unlistenMock = vi.fn()
async function defaultListen(
  event: string,
  handler: (e: { payload: unknown }) => void,
  _options?: unknown,
) {
  if (listenGate) await listenGate.promise
  listeners.set(event, handler)
  return unlistenMock
}
const listenMock = vi.fn(defaultListen)
vi.mock('@tauri-apps/api/event', () => ({
  listen: (event: string, handler: (e: { payload: unknown }) => void, options?: unknown) =>
    listenMock(event, handler, options),
}))

const WINDOW_LABEL = 'slot-popup-1'
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ label: WINDOW_LABEL }),
}))

import { useThemeStore } from '../store/themeStore'
import { installUiSettings } from './uiSettings'

const EVT = 'ui:settings-updated'

/** Imitates a shell push — a callback that comes from outside React so wrap it in act (in production a Tauri event has the same shape). */
function push(theme: unknown): void {
  const h = listeners.get(EVT)
  if (!h) throw new Error(`${EVT} 리스너가 없다 — installUiSettings 가 등록에 실패했나?`)
  act(() => {
    h({ payload: { theme } })
  })
}

/** Flush through the boot query (microtask chain) and the render after it. */
async function settle(): Promise<void> {
  await act(async () => {
    await new Promise(resolve => setTimeout(resolve, 0))
  })
}

function themeAttr(): string | null {
  return document.documentElement.getAttribute('data-theme')
}

let childMounts = 0

/** ★Subscribes to the theme★ — without this subscription the remount assertion below catches nothing (file header). */
function Child() {
  const theme = useThemeStore(s => s.theme)
  useEffect(() => {
    childMounts += 1
  }, [])
  return <div data-testid="child">{theme}</div>
}

/** The same wiring App does — hook once on mount and release on unmount. */
function Host() {
  useEffect(() => installUiSettings(), [])
  return <Child />
}

beforeEach(() => {
  childMounts = 0
  listeners.clear()
  listenGate = null
  unlistenMock.mockClear()
  // ★Pitfall: `mockClear()` clears **only the call records** and leaves `mockImplementation`★ — so `mockReset()` that also
  //   reverts the implementation + reinstalling the default implementation is needed. If the 「listen 이 항상 실패」 (listen always fails) implementation installed by the retry-exhaustion test below
  //   survives, **every test after it** quietly runs in that world. The symptom was
  //   「관계없어 보이는 수명 테스트 여러 개가 한꺼번에 깨진다」 (several unrelated-looking lifetime tests break at once) (it actually happened — three). The same pitfall exists in
  //   `invokeMock` so the implementation is reinstalled every time below.
  listenMock.mockReset()
  listenMock.mockImplementation(defaultListen)
  invokeMock.mockClear()
  invokeMock.mockImplementation(async () => ({ theme: 'dark' }))
  document.documentElement.removeAttribute('data-theme')
  useThemeStore.setState({ theme: 'dark' })
  sessionStorage.clear()
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

describe('installUiSettings — 부팅 조회', () => {
  it('셸이 준 값이 data-theme 까지 간다', async () => {
    invokeMock.mockImplementation(async () => ({ theme: 'light' }))
    render(<Host />)
    await settle()

    expect(invokeMock).toHaveBeenCalledWith('get_ui_settings')
    expect(themeAttr()).toBe('light')
    expect(useThemeStore.getState().theme).toBe('light')
  })

  it('e-ink 도 그대로 실린다 — dark/light 로 접히지 않는다(ADR-0062)', async () => {
    invokeMock.mockImplementation(async () => ({ theme: 'e-ink' }))
    render(<Host />)
    await settle()

    expect(themeAttr()).toBe('e-ink')
  })

  // ★Does not overwrite with dark the window's theme the shell planted when creating the window★ — if it did, a light · e-ink window, merely for not having received the shell value, would
  //   change to a color that is not that window's theme.
  it('조회가 실패하면 지금 붙은 테마를 그대로 둔다', async () => {
    document.documentElement.dataset.theme = 'light'
    invokeMock.mockImplementation(async () => {
      throw new Error('셸이 아직 안 떴다')
    })
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    render(<Host />)
    await settle()

    expect(themeAttr()).toBe('light')
    expect(warn).toHaveBeenCalled()
  })

  it('조회가 실패했는데 붙은 테마가 없으면 dark 로 간다(무테마로 남지 않는다)', async () => {
    invokeMock.mockImplementation(async () => {
      throw new Error('셸이 아직 안 떴다')
    })
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    render(<Host />)
    await settle()

    expect(themeAttr()).toBe('dark')
  })

  it('모르는 이름이 오면 통과시키지 않고 지금 테마를 둔다', async () => {
    document.documentElement.dataset.theme = 'e-ink'
    invokeMock.mockImplementation(async () => ({ theme: 'solarized' }))
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    render(<Host />)
    await settle()

    expect(themeAttr()).toBe('e-ink')
    expect(warn).toHaveBeenCalled()
  })

  it('모르는 이름이 왔는데 붙은 테마도 모르는 철자면 dark 로 접는다', async () => {
    document.documentElement.dataset.theme = 'sepia'
    invokeMock.mockImplementation(async () => ({ theme: 'solarized' }))
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    render(<Host />)
    await settle()

    expect(themeAttr()).toBe('dark')
  })
})

describe('installUiSettings — 다시 읽기용 기억', () => {
  // The shell's init script reads this key first on reload (`src-tauri/src/webview_env.rs`) — if not written, a window whose theme changed
  //   is drawn once in the theme at window creation when reloaded.
  it('붙인 테마를 그 창 label 의 키로 적는다', async () => {
    invokeMock.mockImplementation(async () => ({ theme: 'light' }))
    render(<Host />)
    await settle()

    expect(sessionStorage.getItem(`engram.theme.${WINDOW_LABEL}`)).toBe('light')

    push('e-ink')

    expect(sessionStorage.getItem(`engram.theme.${WINDOW_LABEL}`)).toBe('e-ink')
  })

  it('셸 값을 못 받아 지금 테마를 두면 기억도 그대로다', async () => {
    document.documentElement.dataset.theme = 'light'
    sessionStorage.setItem(`engram.theme.${WINDOW_LABEL}`, 'light')
    invokeMock.mockImplementation(async () => ({ theme: 'solarized' }))
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    render(<Host />)
    await settle()

    expect(sessionStorage.getItem(`engram.theme.${WINDOW_LABEL}`)).toBe('light')
  })
})

describe('installUiSettings — 셸 테마 푸시', () => {
  // ★The shell sends **that window's value** to each window★ (ADR-0167). A default registration (`Any`) wakes everything regardless of the filter, so
  //   without a target this window receives other windows' values too and paints the last one to arrive — the per-window theme collapses on the spot.
  it('자기 창 label 로 구독한다', async () => {
    render(<Host />)
    await settle()

    expect(listenMock).toHaveBeenCalledWith(EVT, expect.any(Function), {
      target: WINDOW_LABEL,
    })
  })

  it('★값만 바뀌고 트리는 그대로 마운트돼 있다★', async () => {
    invokeMock.mockImplementation(async () => ({ theme: 'light' }))
    render(<Host />)
    await settle()
    const node = screen.getByTestId('child')
    expect(themeAttr()).toBe('light')
    expect(node.textContent).toBe('light')
    expect(childMounts).toBe(1)

    push('e-ink')

    expect(themeAttr()).toBe('e-ink')
    // Subscription alive = proof that the two assertions below are actually measuring something.
    expect(screen.getByTestId('child').textContent).toBe('e-ink')
    expect(childMounts).toBe(1)
    // It is the same DOM node — if React swapped the child (key swap · remount), it diverges here.
    expect(screen.getByTestId('child')).toBe(node)
    expect(node.isConnected).toBe(true)
  })

  it('푸시가 먼저 닿으면 뒤늦게 온 부팅 조회가 그것을 덮지 않는다', async () => {
    let answer: (value: unknown) => void = () => {}
    invokeMock.mockImplementation(
      () =>
        new Promise(resolve => {
          answer = resolve
        }),
    )
    render(<Host />)
    // A state where subscription registration has finished and the query is still hanging.
    await settle()

    push('e-ink')
    expect(themeAttr()).toBe('e-ink')

    await act(async () => {
      answer({ theme: 'light' })
      await new Promise(resolve => setTimeout(resolve, 0))
    })

    expect(themeAttr()).toBe('e-ink')
  })

  it('푸시 값이 깨져 있으면 지금 테마를 그대로 둔다', async () => {
    invokeMock.mockImplementation(async () => ({ theme: 'light' }))
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    render(<Host />)
    await settle()

    push(42)

    expect(themeAttr()).toBe('light')
    expect(warn).toHaveBeenCalled()
  })
})

describe('installUiSettings — 등록과 조회의 순서', () => {
  // ★This order is the whole of loss prevention★: if you launch the two side by side, a notice that comes before registration finishes does not even arrive at this window
  //   (there is no listener) and the query answer that comes late afterwards paints the old value. The `pushed` latch stands **only when a notice is received**, so
  //   it cannot stop that interleave.
  it('구독 등록이 끝나기 전에는 부팅 조회를 내지 않는다', async () => {
    const gate = deferred()
    listenGate = gate
    let shellTheme = 'light'
    invokeMock.mockImplementation(async () => ({ theme: shellTheme }))

    render(<Host />)
    await settle()

    expect(listeners.size).toBe(0)
    expect(invokeMock).not.toHaveBeenCalled()

    // In this gap an outside agent called window.setTheme — there is no listener so the notice does not reach this window.
    shellTheme = 'e-ink'

    await act(async () => {
      gate.open()
      await new Promise(resolve => setTimeout(resolve, 0))
    })

    // Instead of the missed notice, the query issued **after that** fetched the new value.
    expect(invokeMock).toHaveBeenCalledTimes(1)
    expect(themeAttr()).toBe('e-ink')
  })

  // ★If it is refused once and that is the end, that window can never receive theme pushes★ — yet the boot value is painted fine and
  //   the screen looks healthy (a signal-free mismatch). So a bounded retry is put on registration.
  it('구독 등록이 한 번 거절당해도 재시도해서 결국 붙는다', async () => {
    listenMock.mockImplementationOnce(async () => {
      throw new Error('이벤트 평면 미준비')
    })
    invokeMock.mockImplementation(async () => ({ theme: 'light' }))
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})

    render(<Host />)
    await act(async () => {
      await new Promise(resolve => setTimeout(resolve, 400))
    })

    expect(listenMock.mock.calls.length).toBeGreaterThan(1)
    expect(warn).toHaveBeenCalled()
    expect(themeAttr()).toBe('light')

    // ★Did the retry actually attach★ — if it did, later notices reach this window.
    push('e-ink')
    expect(themeAttr()).toBe('e-ink')
  })

  // If retries are exhausted too, it is not passed over silently — from then on not receiving updates is certain, so error, not warn.
  it('재시도를 다 써도 실패하면 error 로 표면화하고 부팅 조회는 그대로 나간다', async () => {
    listenMock.mockImplementation(async () => {
      throw new Error('이벤트 평면 미준비')
    })
    invokeMock.mockImplementation(async () => ({ theme: 'light' }))
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})

    render(<Host />)
    // Default retry profile (4 tries · 150ms multiplier 2) total wait ~1.05s — wait with more margin than that.
    await act(async () => {
      await new Promise(resolve => setTimeout(resolve, 1500))
    })

    expect(error).toHaveBeenCalled()
    expect(themeAttr()).toBe('light')
  })
})

describe('installUiSettings — 수명', () => {
  it('언마운트하면 구독을 푼다(HMR·팝아웃 재부착에서 중복 누적 금지)', async () => {
    const { unmount } = render(<Host />)
    await settle()
    expect(listenMock).toHaveBeenCalledTimes(1)

    unmount()

    expect(unlistenMock).toHaveBeenCalledTimes(1)
  })

  it('등록이 끝나기 전에 정리되면 받은 자리에서 바로 푼다', async () => {
    const dispose = installUiSettings()
    dispose()
    await settle()

    expect(unlistenMock).toHaveBeenCalledTimes(1)
  })

  // ★`off()` alone lets one through★: if dispose runs while `listen()` is pending, registration is already done but release has not happened yet,
  //   so a notice delivered in that gap is painted by the dead instance (the first StrictMode round · a previous HMR edition overwrites the live window's
  //   theme). Here we strike the callback directly after release to imitate that delivery.
  it('정리된 뒤에 배달된 알림은 화면을 건드리지 않는다', async () => {
    invokeMock.mockImplementation(async () => ({ theme: 'light' }))
    const dispose = installUiSettings()
    await settle()
    expect(themeAttr()).toBe('light')

    dispose()

    const handler = listeners.get(EVT)
    if (!handler) throw new Error('리스너가 등록되지 않았다')
    act(() => {
      handler({ payload: { theme: 'e-ink' } })
    })

    expect(themeAttr()).toBe('light')
  })
})
