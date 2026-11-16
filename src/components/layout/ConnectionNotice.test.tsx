// ConnectionNotice — the connection band (TRD S36 §3-4 · ADR-0134 decision 4).
//
// What this suite guards: the band the shell sends is drawn with the right words, 「다시 연결」 does what it says (and
// only in the main window), and the user is never stuck under a band they cannot close.

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { Band } from '../../../src-tauri/bindings/Band'
import type { ConnectionState } from '../../api/agentClient'

// Stand in for the agentClient singleton with a minimal surface — reproduce only the subscription notification, with no real transport/IPC.
// ★vi.hoisted required★: the vi.mock factory is hoisted to the top of the file and cannot see ordinary top-level variables.
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

const PLAIN: Band = { kind: 'not_connected', refused: false }
const REFUSED: Band = { kind: 'not_connected', refused: true }
const VERSION: Band = { kind: 'version', can_stop: true }
const VERSION_FIXED: Band = { kind: 'version', can_stop: false }

// Subscription notifications come from outside React, so they are wrapped in act for the state update to flush (the
//   Tauri event callback has the same shape in production).
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
    emit('down', null, '폴더 문제')
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('band ① shows the head and the start cause', () => {
    render(<ConnectionNotice />)
    emit('down', PLAIN, '데이터 폴더에 쓸 수 없음(C:\\x) — 쓰기 가능한 위치에 압축을 풀어 주세요')
    const alert = screen.getByRole('alert')
    expect(alert.getAttribute('data-band')).toBe('not_connected')
    expect(alert.textContent).toContain('데몬에 연결하지 못했습니다.')
    expect(alert.textContent).toContain('쓰기 가능한 위치')
  })

  it('band ① without a cause shows the head only', () => {
    render(<ConnectionNotice />)
    emit('down', PLAIN)
    expect(screen.getByRole('alert').firstElementChild?.textContent).toBe('데몬에 연결하지 못했습니다.')
  })

  it('band ① refused shows the refusal and hides the start cause', () => {
    render(<ConnectionNotice />)
    emit('down', REFUSED, '폴더 문제')
    const text = screen.getByRole('alert').textContent
    expect(text).toContain('데몬이 이 앱의 인증을 거절했습니다')
    expect(text).not.toContain('폴더 문제')
  })

  it('band ② shows the version text and never the start cause', () => {
    render(<ConnectionNotice />)
    emit('down', VERSION, '폴더 문제')
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

describe('ConnectionNotice — 「다시 연결」 (main window)', () => {
  it.each([
    ['band ①', PLAIN],
    ['band ① refused', REFUSED],
    ['band ② (can_stop)', VERSION],
    ['band ② (no can_stop)', VERSION_FIXED],
  ])('%s carries 「다시 연결」 and ✕, no stop button', (_name, band) => {
    render(<ConnectionNotice actions />)
    emit('down', band)
    expect(buttonNames()).toEqual(['다시 연결', '알림 닫기'])
  })

  it('runs DaemonControl.start and shows 「연결 중…」 disabled while it runs', async () => {
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
    expect(warn).toHaveBeenCalledWith('[ConnectionNotice] 다시 연결 실패:', boom)
    warn.mockRestore()
  })
})

// ✕ memory = band kind + cause text, cleared on `connected`; and the covering layer's row contract.
describe('ConnectionNotice', () => {
  // The row contract of the covering layer (the `NoticeOverlay` doc) — if `wrap-anywhere` is lost, a long cause pushes the ✕ off screen.
  it('줄은 불투명 바탕이고 글 칸은 아무 데서나 줄을 바꾼다', () => {
    render(<ConnectionNotice />)
    emit('down', PLAIN, '폴더 문제')
    const alert = screen.getByRole('alert')
    expect(alert.className).toMatch(/\bbg-elevated\b/)
    expect(alert.firstElementChild?.className).toMatch(/\bwrap-anywhere\b/)
  })

  it('닫으면 사라진다', () => {
    render(<ConnectionNotice />)
    emit('down', PLAIN, '폴더 문제')
    fireEvent.click(screen.getByLabelText('알림 닫기'))
    expect(screen.queryByRole('alert')).toBeNull()
  })

  // The band sits on the covering layer — if the next notification of the same band revives it, the ✕ is powerless.
  it('닫은 뒤 같은 이유로 다시 통지돼도 뜨지 않는다', () => {
    render(<ConnectionNotice />)
    emit('down', PLAIN, '폴더 문제')
    fireEvent.click(screen.getByLabelText('알림 닫기'))
    emit('reconnecting', PLAIN, '폴더 문제')
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('닫은 뒤 다른 이유가 오면 다시 뜬다', () => {
    render(<ConnectionNotice />)
    emit('down', PLAIN, '첫 번째 이유')
    fireEvent.click(screen.getByLabelText('알림 닫기'))
    emit('down', PLAIN, '두 번째 이유')
    expect(screen.getByRole('alert').textContent).toContain('두 번째 이유')
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
    emit('down', PLAIN, '폴더 문제')
    fireEvent.click(screen.getByLabelText('알림 닫기'))
    emit('reconnecting', null)
    emit('down', PLAIN, '폴더 문제')
    expect(screen.queryByRole('alert')).toBeNull()
  })

  // ★F3 regression guard★: a recurrence of the same cause after a fix and a connection is the common case.
  it('shows the same band again after a connection', () => {
    render(<ConnectionNotice />)
    const same = '데이터 폴더에 쓸 수 없음(C:\\x) — 쓰기 가능한 위치에 압축을 풀어 주세요'
    emit('down', PLAIN, same)
    fireEvent.click(screen.getByLabelText('알림 닫기'))
    expect(screen.queryByRole('alert')).toBeNull()

    emit('connected', null)
    expect(screen.queryByRole('alert')).toBeNull()

    emit('down', PLAIN, same)
    expect(screen.getByRole('alert').textContent).toContain('쓰기 가능한 위치')
  })
})
