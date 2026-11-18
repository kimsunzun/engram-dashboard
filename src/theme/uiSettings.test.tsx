// uiSettings — whether the shell-given theme reaches `data-theme`, and ★whether applying it avoids remounting the tree★ (ADR-0149: a slot remount loses chat component state — the conversation — permanently; this is why the suite exists).
// ★`Child` **subscribes** to the theme — the premise of that assertion★: a child subscribing to nothing does not re-render whatever the implementation does, so mount counts · node identity always pass (a remounting implementation is green too). Only with a subscription does a theme change cause a real re-render and make "mounted once" a measured value; we confirm every time the subscription is alive (painted text changes), or the premise quietly rots.
// ★Not measured — the production tree (known gap)★: `Host`/`Child` are a **synthetic harness**, so this measures only "the theme application path does not trigger a remount"; a component added later to the production tree that subscribes to the theme **and** changes its key stays green. Closing that gap needs a separate test bringing up the real slot subtree.

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

// listen mock: keeps registered handlers so the test imitates a shell push (same shape as viewStore.test). While listenGate is set it holds the registration itself — to measure "no boot query before registration". ★`options` is passed along too★ — the subscription target (which window's share) travels in it.
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
  if (!h) throw new Error(`no ${EVT} listener — did installUiSettings fail to register?`)
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
  // ★Pitfall: `mockClear()` clears **only call records** and keeps `mockImplementation`★ — so `mockReset()` + reinstalling the default is needed; the "listen always fails" implementation from the retry-exhaustion test would otherwise leak into **every later test** (it happened — three unrelated-looking lifetime tests broke together). Same for `invokeMock`, so its implementation is reinstalled every time below.
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

describe('installUiSettings — boot query', () => {
  it('the value the shell gave goes all the way to data-theme', async () => {
    invokeMock.mockImplementation(async () => ({ theme: 'light' }))
    render(<Host />)
    await settle()

    expect(invokeMock).toHaveBeenCalledWith('get_ui_settings')
    expect(themeAttr()).toBe('light')
    expect(useThemeStore.getState().theme).toBe('light')
  })

  it('e-ink is carried as is too — not folded into dark/light (ADR-0062)', async () => {
    invokeMock.mockImplementation(async () => ({ theme: 'e-ink' }))
    render(<Host />)
    await settle()

    expect(themeAttr()).toBe('e-ink')
  })

  // ★Never overwrite the window's shell-planted theme with dark★: a light · e-ink window that merely missed the shell value would change to a color that is not its theme.
  it('when the query fails, the theme attached now is kept', async () => {
    document.documentElement.dataset.theme = 'light'
    invokeMock.mockImplementation(async () => {
      throw new Error('the shell is not up yet')
    })
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    render(<Host />)
    await settle()

    expect(themeAttr()).toBe('light')
    expect(warn).toHaveBeenCalled()
  })

  it('when the query fails and no theme is attached, it goes dark (does not stay theme-less)', async () => {
    invokeMock.mockImplementation(async () => {
      throw new Error('the shell is not up yet')
    })
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    render(<Host />)
    await settle()

    expect(themeAttr()).toBe('dark')
  })

  it('an unknown name is not passed through and the current theme is kept', async () => {
    document.documentElement.dataset.theme = 'e-ink'
    invokeMock.mockImplementation(async () => ({ theme: 'solarized' }))
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    render(<Host />)
    await settle()

    expect(themeAttr()).toBe('e-ink')
    expect(warn).toHaveBeenCalled()
  })

  it('when an unknown name comes and the attached theme is also an unknown spelling, it folds to dark', async () => {
    document.documentElement.dataset.theme = 'sepia'
    invokeMock.mockImplementation(async () => ({ theme: 'solarized' }))
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    render(<Host />)
    await settle()

    expect(themeAttr()).toBe('dark')
  })
})

describe('installUiSettings — memory for re-reading', () => {
  // The shell's init script reads this key first on reload (`src-tauri/src/webview_env.rs`); unwritten, a window whose theme changed is drawn once in its creation-time theme after reload.
  it("writes the attached theme under that window label's key", async () => {
    invokeMock.mockImplementation(async () => ({ theme: 'light' }))
    render(<Host />)
    await settle()

    expect(sessionStorage.getItem(`engram.theme.${WINDOW_LABEL}`)).toBe('light')

    push('e-ink')

    expect(sessionStorage.getItem(`engram.theme.${WINDOW_LABEL}`)).toBe('e-ink')
  })

  it('when the shell value is not received and the current theme is kept, the memory stays too', async () => {
    document.documentElement.dataset.theme = 'light'
    sessionStorage.setItem(`engram.theme.${WINDOW_LABEL}`, 'light')
    invokeMock.mockImplementation(async () => ({ theme: 'solarized' }))
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    render(<Host />)
    await settle()

    expect(sessionStorage.getItem(`engram.theme.${WINDOW_LABEL}`)).toBe('light')
  })
})

describe('installUiSettings — shell theme push', () => {
  // ★The shell sends **that window's value** to each window★ (ADR-0167). A default registration (`Any`) wakes everything regardless of the filter, so without a target this window receives other windows' values and paints the last — the per-window theme collapses.
  it('subscribes with its own window label', async () => {
    render(<Host />)
    await settle()

    expect(listenMock).toHaveBeenCalledWith(EVT, expect.any(Function), {
      target: WINDOW_LABEL,
    })
  })

  it('★only the value changes and the tree stays mounted★', async () => {
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

  it('when the push arrives first, the late boot query does not overwrite it', async () => {
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

  it('when the push value is broken, the current theme is kept', async () => {
    invokeMock.mockImplementation(async () => ({ theme: 'light' }))
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    render(<Host />)
    await settle()

    push(42)

    expect(themeAttr()).toBe('light')
    expect(warn).toHaveBeenCalled()
  })
})

describe('installUiSettings — order of registration and query', () => {
  // ★This order is the whole of loss prevention★: launched side by side, a notice arriving before registration finishes never reaches this window (no listener) and the late query answer paints the old value; the `pushed` latch stands **only on a received notice**, so it cannot stop that interleave.
  it('does not issue the boot query before the subscription registration finishes', async () => {
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

  // ★If refused once and left, that window can never receive theme pushes★ — yet the boot value paints fine and the screen looks healthy (signal-free mismatch), so registration gets a bounded retry.
  it('even when the subscription registration is refused once, it retries and eventually attaches', async () => {
    listenMock.mockImplementationOnce(async () => {
      throw new Error('event plane not ready')
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
  it('when it fails after using up the retries, it surfaces as an error and the boot query still goes out', async () => {
    listenMock.mockImplementation(async () => {
      throw new Error('event plane not ready')
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

describe('installUiSettings — lifetime', () => {
  it('unmounting releases the subscription (no duplicate pile-up on HMR · popout reattach)', async () => {
    const { unmount } = render(<Host />)
    await settle()
    expect(listenMock).toHaveBeenCalledTimes(1)

    unmount()

    expect(unlistenMock).toHaveBeenCalledTimes(1)
  })

  it('when cleaned up before registration finishes, it releases right where it receives it', async () => {
    const dispose = installUiSettings()
    dispose()
    await settle()

    expect(unlistenMock).toHaveBeenCalledTimes(1)
  })

  // ★`off()` alone lets one through★: if dispose runs while `listen()` is pending, registration is done but not yet released, so a notice delivered in that gap is painted by the dead instance (first StrictMode round · previous HMR edition) over the live window's theme. The callback is struck directly after release to imitate that delivery.
  it('a notice delivered after cleanup does not touch the screen', async () => {
    invokeMock.mockImplementation(async () => ({ theme: 'light' }))
    const dispose = installUiSettings()
    await settle()
    expect(themeAttr()).toBe('light')

    dispose()

    const handler = listeners.get(EVT)
    if (!handler) throw new Error('the listener was not registered')
    act(() => {
      handler({ payload: { theme: 'e-ink' } })
    })

    expect(themeAttr()).toBe('light')
  })
})
