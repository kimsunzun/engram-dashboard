// ConnectionNotice — the connection band (TRD S36 §3-4 · ADR-0134 decision 4). Guards: the shell's band is drawn with the right words, "reconnect" does what it says (main window only), and the user is never stuck under a band they cannot close.

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { Band } from '../../../src-tauri/bindings/Band'
import type { ConnectionState } from '../../api/agentClient'

// Stand in for the agentClient singleton with a minimal surface — only the subscription notification, no real transport/IPC. ★vi.hoisted required★: the vi.mock factory is hoisted above ordinary top-level variables.
const { listeners, fake, control } = vi.hoisted(() => {
  const listeners = new Set<(state: string) => void>()
  const fake = {
    connectionState: 'down' as string,
    connectionBand: null as unknown,
    connectionError: null as string | null,
    onConnectionStateChange(cb: (state: string) => void) {
      listeners.add(cb)
      cb(fake.connectionState)
      return () => {
        listeners.delete(cb)
      }
    },
  }
  const control = { start: vi.fn(async () => undefined as unknown) }
  return { listeners, fake, control }
})

vi.mock('../../api/clientFactory', () => ({
  agentClient: fake,
  getDaemonControl: () => control,
}))

import ConnectionNotice from './ConnectionNotice'
import { setLanguage } from '../../i18n'
import { useLanguage } from '../../i18n/react'

const PLAIN: Band = { kind: 'not_connected', refused: false }
const REFUSED: Band = { kind: 'not_connected', refused: true }
const VERSION: Band = { kind: 'version', can_stop: true }
const VERSION_FIXED: Band = { kind: 'version', can_stop: false }

// Subscription notifications come from outside React, so they are wrapped in act (the Tauri event callback has the same shape in production).
function emit(state: ConnectionState, band: Band | null, error: string | null = null) {
  act(() => {
    fake.connectionState = state
    fake.connectionBand = band
    fake.connectionError = error
    for (const cb of [...listeners]) cb(state)
  })
}

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (err: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

const buttonNames = () => screen.getAllByRole('button').map(b => b.getAttribute('aria-label') ?? b.textContent)

beforeEach(() => {
  control.start.mockReset()
  control.start.mockImplementation(async () => undefined)
})

afterEach(() => {
  cleanup()
  listeners.clear()
  fake.connectionState = 'down'
  fake.connectionBand = null
  fake.connectionError = null
})

describe('ConnectionNotice — what the band says', () => {
  it('draws nothing without a band, even with a start cause (the shell decides)', () => {
    render(<ConnectionNotice />)
    emit('down', null, 'folder problem')
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('band ① shows the head and the start cause', () => {
    render(<ConnectionNotice />)
    emit('down', PLAIN, 'cannot write to the data folder (C:\\x): Access is denied. (os error 5) — please extract to a writable location')
    const alert = screen.getByRole('alert')
    expect(alert.getAttribute('data-band')).toBe('not_connected')
    expect(alert.textContent).toContain('데몬에 연결하지 못했습니다.')
    expect(alert.textContent).toContain('writable location')
  })

  it('band ① without a cause shows the head only', () => {
    render(<ConnectionNotice />)
    emit('down', PLAIN)
    expect(screen.getByRole('alert').firstElementChild?.textContent).toBe('데몬에 연결하지 못했습니다.')
  })

  it('band ① refused shows the refusal and hides the start cause', () => {
    render(<ConnectionNotice />)
    emit('down', REFUSED, 'folder problem')
    const text = screen.getByRole('alert').textContent
    expect(text).toContain('데몬이 이 앱의 인증을 거절했습니다')
    expect(text).not.toContain('folder problem')
  })

  it('band ② shows the version text and never the start cause', () => {
    render(<ConnectionNotice />)
    emit('down', VERSION, 'folder problem')
    const alert = screen.getByRole('alert')
    expect(alert.getAttribute('data-band')).toBe('version')
    expect(alert.firstElementChild?.textContent).toBe(
      '데몬에 연결하지 못했습니다. 실행 중인 데몬이 이 앱과 버전이 다릅니다. 데몬을 끈 뒤 다시 연결하세요',
    )
  })

  it.each([
    ['band ①', PLAIN],
    ['band ②', VERSION],
  ])('without `actions` (popouts · the spare) %s has no button but ✕', (_name, band) => {
    render(<ConnectionNotice />)
    emit('down', band)
    expect(screen.getByRole('alert')).toBeTruthy()
    expect(buttonNames()).toEqual(['알림 닫기'])
  })
})

describe('ConnectionNotice — 「reconnect」 (main window)', () => {
  it.each([
    ['band ①', PLAIN],
    ['band ① refused', REFUSED],
    ['band ② (can_stop)', VERSION],
    ['band ② (no can_stop)', VERSION_FIXED],
  ])('%s carries 「reconnect」 and ✕, no stop button', (_name, band) => {
    render(<ConnectionNotice actions />)
    emit('down', band)
    expect(buttonNames()).toEqual(['다시 연결', '알림 닫기'])
  })

  it('runs DaemonControl.start and shows 「connecting…」 disabled while it runs', async () => {
    const run = deferred<unknown>()
    control.start.mockImplementation(() => run.promise)
    render(<ConnectionNotice actions />)
    emit('down', VERSION)
    fireEvent.click(screen.getByRole('button', { name: '다시 연결' }))
    expect(control.start).toHaveBeenCalledTimes(1)
    const busy = screen.getByRole('button', { name: '연결 중…' }) as HTMLButtonElement
    expect(busy.disabled).toBe(true)
    // The band stays while the start runs — the shell holds it.
    expect(screen.getByRole('alert').getAttribute('data-band')).toBe('version')
    await act(async () => {
      run.resolve(undefined)
      await run.promise
    })
    expect((screen.getByRole('button', { name: '다시 연결' }) as HTMLButtonElement).disabled).toBe(false)
  })

  it('the button comes back after a rejected start', async () => {
    const run = deferred<unknown>()
    control.start.mockImplementation(() => run.promise)
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    render(<ConnectionNotice actions />)
    emit('down', PLAIN)
    fireEvent.click(screen.getByRole('button', { name: '다시 연결' }))
    expect(screen.getByRole('button', { name: '연결 중…' })).toBeTruthy()
    const boom = new Error('no daemon')
    await act(async () => {
      run.reject(boom)
      await run.promise.catch(() => undefined)
    })
    expect((screen.getByRole('button', { name: '다시 연결' }) as HTMLButtonElement).disabled).toBe(false)
    expect(warn).toHaveBeenCalledWith('[ConnectionNotice] reconnect failed:', boom)
    warn.mockRestore()
  })
})

// ✕ memory = band kind + cause text, cleared on `connected`; and the covering layer's row contract.
describe('ConnectionNotice', () => {
  // The row contract of the covering layer (the `NoticeOverlay` doc) — if `wrap-anywhere` is lost, a long cause pushes the ✕ off screen.
  it('the row has an opaque background and the text field wraps anywhere', () => {
    render(<ConnectionNotice />)
    emit('down', PLAIN, 'folder problem')
    const alert = screen.getByRole('alert')
    expect(alert.className).toMatch(/\bbg-elevated\b/)
    expect(alert.firstElementChild?.className).toMatch(/\bwrap-anywhere\b/)
  })

  it('it disappears on close', () => {
    render(<ConnectionNotice />)
    emit('down', PLAIN, 'folder problem')
    fireEvent.click(screen.getByLabelText('알림 닫기'))
    expect(screen.queryByRole('alert')).toBeNull()
  })

  // The band sits on the covering layer — if the next notification of the same band revives it, the ✕ is powerless.
  it('after closing, it does not reappear even if notified again for the same reason', () => {
    render(<ConnectionNotice />)
    emit('down', PLAIN, 'folder problem')
    fireEvent.click(screen.getByLabelText('알림 닫기'))
    emit('reconnecting', PLAIN, 'folder problem')
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('after closing, a different reason makes it reappear', () => {
    render(<ConnectionNotice />)
    emit('down', PLAIN, 'first reason')
    fireEvent.click(screen.getByLabelText('알림 닫기'))
    emit('down', PLAIN, 'second reason')
    expect(screen.getByRole('alert').textContent).toContain('second reason')
  })

  it('shows again for a different band kind', () => {
    render(<ConnectionNotice />)
    // ① carrying ②'s own text as its cause — the two bands differ in kind only.
    emit('down', PLAIN, '실행 중인 데몬이 이 앱과 버전이 다릅니다. 데몬을 끈 뒤 다시 연결하세요')
    fireEvent.click(screen.getByLabelText('알림 닫기'))
    emit('down', VERSION)
    expect(screen.getByRole('alert').getAttribute('data-band')).toBe('version')
  })

  it('stays closed while the band goes away and comes back without a connection', () => {
    render(<ConnectionNotice />)
    emit('down', PLAIN, 'folder problem')
    fireEvent.click(screen.getByLabelText('알림 닫기'))
    emit('reconnecting', null)
    emit('down', PLAIN, 'folder problem')
    expect(screen.queryByRole('alert')).toBeNull()
  })

  // ★F3 regression guard★: a recurrence of the same cause after a fix and a connection is the common case.
  it('shows the same band again after a connection', () => {
    render(<ConnectionNotice />)
    const same = 'cannot write to the data folder (C:\\x): Access is denied. (os error 5) — please extract to a writable location'
    emit('down', PLAIN, same)
    fireEvent.click(screen.getByLabelText('알림 닫기'))
    expect(screen.queryByRole('alert')).toBeNull()

    emit('connected', null)
    expect(screen.queryByRole('alert')).toBeNull()

    emit('down', PLAIN, same)
    expect(screen.getByRole('alert').textContent).toContain('writable location')
  })
})

// ADR-0373: the ✕ memory is built from untranslated inputs, so a language switch does not bring a closed band back.
describe('ConnectionNotice — language switch', () => {
  /** Stands in for `App`, which re-renders the tree on a switch. */
  function Root() {
    useLanguage()
    return <ConnectionNotice />
  }

  afterEach(() => {
    act(() => {
      setLanguage('ko')
    })
  })

  it.each([
    ['refused', REFUSED, null],
    ['version', VERSION, null],
    ['a raw start cause', PLAIN, 'folder problem'],
  ] as const)('a dismissed band (%s) stays dismissed across a switch', (_name, band, error) => {
    render(<Root />)
    emit('down', band, error)
    fireEvent.click(screen.getByLabelText('알림 닫기'))
    expect(screen.queryByRole('alert')).toBeNull()

    act(() => {
      setLanguage('en')
    })
    emit('reconnecting', band, error)

    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('a band left open follows the switch', () => {
    render(<Root />)
    emit('down', REFUSED)
    expect(screen.getByRole('alert').textContent).toContain('데몬이 이 앱의 인증을 거절했습니다')

    act(() => {
      setLanguage('en')
    })

    expect(screen.getByRole('alert').textContent).toContain(
      'Could not connect to the daemon. The daemon refused authentication from this app.',
    )
  })
})
