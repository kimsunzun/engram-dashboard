// RestoreModal — it covers main and blocks input · shortcuts stop · it closes only by state (TRD S21-storage §6-7 · §8 「frontend modal」).
//
// Run the whole wiring (AppLayout + restoreClient + the shortcut dispatcher) over a fake IPC — the modal alone cannot measure 「blocks main」
// (the lock is one set: AppLayout's `inert` plus the modal's focus · stop).

import { StrictMode, type ReactNode } from 'react'
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest'

// The canvas is out of interest — we only check whether input beneath it is locked. The notice layer is received as `notices` and drawn inside it.
vi.mock('./WindowLayout', () => ({
  default: ({ notices }: { notices?: ReactNode }) => (
    <div data-testid="window-layout">
      {notices}
      <input data-testid="under-input" />
    </div>
  ),
}))
vi.mock('./ConnectionNotice', () => ({ default: () => null }))

import AppLayout from './AppLayout'
import { formatSavedAt } from './RestoreModal'
import { createRestoreClient, type RestoreStatusView } from '../../api/restoreClient'
import { deferred, fakeRestoreIpc, flush, view } from '../../api/testing/fakeRestoreIpc'
import { installKeybindings } from '../../commands/keybindings'
import { __resetRegistryForTest, register } from '../../commands/registry'
import { setLanguage, t } from '../../i18n'
import type { AnswerReply } from '../../../src-tauri/bindings/AnswerReply'

async function settle(): Promise<void> {
  await act(async () => {
    await flush()
  })
}

function mount(initial: RestoreStatusView, opts: { strict?: boolean } = {}) {
  const f = fakeRestoreIpc(initial)
  const client = createRestoreClient(f.ipc)
  const ui = <AppLayout restore={client} />
  const r = render(opts.strict ? <StrictMode>{ui}</StrictMode> : ui)
  // The first child of the fragment = the layout layer that gets locked.
  const layer = () => r.container.firstElementChild as HTMLElement
  return { f, client, layer, ...r }
}

const dialog = () => screen.queryByRole('alertdialog')
const acceptBtn = () => screen.getByRole('button', { name: t('restore.accept') }) as HTMLButtonElement
const rejectBtn = () => screen.getByRole('button', { name: t('restore.reject') }) as HTMLButtonElement

function fireCtrlTab(target: EventTarget): KeyboardEvent {
  const e = new KeyboardEvent('keydown', { key: 'Tab', ctrlKey: true, bubbles: true, cancelable: true })
  target.dispatchEvent(e)
  return e
}

let disposeKeys: (() => void) | null = null
let shortcut: Mock<() => void>

beforeEach(() => {
  __resetRegistryForTest()
  shortcut = vi.fn<() => void>()
  register({ id: 'tab.next', title: () => 'next', run: shortcut })
  disposeKeys = installKeybindings()
  vi.spyOn(console, 'warn').mockImplementation(() => {})
})

afterEach(() => {
  cleanup()
  disposeKeys?.()
  disposeKeys = null
  vi.restoreAllMocks()
})

describe('RestoreModal — covers and blocks', () => {
  it('if the first pull is awaiting it draws at once, and locks the layout beneath it with inert', async () => {
    const { layer } = mount(view('awaiting'))
    await settle()
    expect(dialog()).not.toBeNull()
    expect(layer().hasAttribute('inert')).toBe(true)
    expect(layer().contains(screen.getByTestId('under-input'))).toBe(true)
    // The modal is outside the locked layer — if inside, it locks itself too.
    expect(layer().contains(dialog())).toBe(false)
  })

  it('the notice layer is also inside the locked layer, and the modal is outside it · above it (z-50 > 40)', async () => {
    const { layer, container } = mount(view('awaiting', { saves: false }))
    await settle()
    const notices = screen.getByTestId('notice-overlay')
    expect(layer().hasAttribute('inert')).toBe(true)
    expect(layer().contains(notices)).toBe(true)
    expect(notices.contains(screen.getByRole('status'))).toBe(true)
    expect(getComputedStyle(notices).zIndex).toBe('40')
    const overlay = container.querySelector('[data-restore-overlay]') as HTMLElement
    expect(layer().contains(overlay)).toBe(false)
    expect(overlay.className).toMatch(/\bz-50\b/)
  })

  it('even with a load without a saves field (version mismatch with the shell) the modal shows — there is no guard ⅱ notice', async () => {
    const { saves: _omit, ...withoutSaves } = view('awaiting')
    mount(withoutSaves as RestoreStatusView)
    await settle()
    expect(dialog()).not.toBeNull()
    expect(screen.queryByRole('status')).toBeNull()

    fireEvent.click(rejectBtn())
    await settle()
    expect(dialog()).toBeNull()
    expect(screen.queryByRole('status')).toBeNull()
  })

  it('even with a load where state_file is an unknown value (version mismatch with the shell) the modal shows — there is no notice', async () => {
    mount({ ...view('awaiting', { saves: false }), state_file: 'from_a_newer_shell' as never })
    await settle()
    expect(dialog()).not.toBeNull()
    expect(screen.queryByRole('status')).toBeNull()

    fireEvent.click(acceptBtn())
    await settle()
    expect(dialog()).toBeNull()
    expect(screen.queryByRole('status')).toBeNull()
  })

  it('if not awaiting there is neither a covering layer nor inert', async () => {
    const { layer, container } = mount(view('none'))
    await settle()
    expect(dialog()).toBeNull()
    expect(container.querySelector('[data-restore-overlay]')).toBeNull()
    expect(layer().hasAttribute('inert')).toBe(false)
  })

  it('shows only the saved time · window count · tab count', async () => {
    const at = new Date(2026, 9, 5, 14, 3).getTime()
    mount(view('awaiting', { saved_at_ms: at, windows: 3, tabs: 7 }))
    await settle()
    const d = dialog() as HTMLElement
    expect(d.querySelector('[data-restore-saved-at]')?.textContent).toBe(formatSavedAt(at))
    expect(formatSavedAt(at)).toContain('14:03')
    expect(d.querySelector('[data-restore-windows]')?.textContent).toBe('3개')
    expect(d.querySelector('[data-restore-tabs]')?.textContent).toBe('7개')
  })

  it('while up the shortcut dispatcher stops, and runs again after the answer (N4)', async () => {
    mount(view('awaiting'))
    await settle()
    const blocked = fireCtrlTab(document.body)
    expect(shortcut).not.toHaveBeenCalled()
    expect(blocked.defaultPrevented).toBe(false)

    fireEvent.click(acceptBtn())
    await settle()
    expect(dialog()).toBeNull()
    fireCtrlTab(document.body)
    expect(shortcut).toHaveBeenCalledTimes(1)
  })

  it('moves focus to the inner 「restore previous screen」 — also focus that was on an input beneath it', async () => {
    const { f } = mount(view('none'))
    await settle()
    const under = screen.getByTestId('under-input')
    under.focus()
    expect(document.activeElement).toBe(under)

    f.server.view = view('awaiting')
    f.push('awaiting')
    await settle()
    expect(document.activeElement).toBe(acceptBtn())
  })

  it('even pressing outside the buttons does not drop focus (mousedown default is prevented) — the button is still pressed as is', async () => {
    const { container } = mount(view('awaiting'))
    await settle()
    const overlay = container.querySelector('[data-restore-overlay]') as HTMLElement
    expect(fireEvent.mouseDown(overlay)).toBe(false)
    expect(fireEvent.mouseDown(screen.getByRole('heading'))).toBe(false)
    expect(fireEvent.mouseDown(acceptBtn())).toBe(true)
  })

  // ADR-0373: the formatter follows the UI language, made per language when used.
  it('formatSavedAt — in the locale of the UI language', () => {
    const at = new Date(2026, 9, 21, 14, 3).getTime()
    const options = { year: 'numeric', month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit', hourCycle: 'h23' } as const
    try {
      expect(formatSavedAt(at)).toBe(new Intl.DateTimeFormat('ko-KR', options).format(at))
      setLanguage('en')
      expect(formatSavedAt(at)).toBe(new Intl.DateTimeFormat('en-US', options).format(at))
      expect(formatSavedAt(at)).not.toBe(new Intl.DateTimeFormat('ko-KR', options).format(at))
    } finally {
      setLanguage('ko')
    }
  })

  it('formatSavedAt — formats within the Date range (±8.64e15) and null outside it · for a non-finite value', () => {
    expect(formatSavedAt(8.64e15)).not.toBeNull()
    expect(formatSavedAt(-8.64e15)).not.toBeNull()
    expect(formatSavedAt(8.64e15 + 1)).toBeNull()
    expect(formatSavedAt(1.8e19)).toBeNull()
    expect(formatSavedAt(Number.POSITIVE_INFINITY)).toBeNull()
    expect(formatSavedAt(Number.NaN)).toBeNull()
  })

  it('if the saved time is a value Date cannot represent (near the u64 limit) only that line is dropped and the modal is still usable', async () => {
    const { f } = mount(view('awaiting', { saved_at_ms: 1.8e19 }))
    await settle()
    expect(dialog()).not.toBeNull()
    expect(dialog()?.querySelector('[data-restore-saved-at]')).toBeNull()
    expect(dialog()?.querySelector('[data-restore-tabs]')?.textContent).toBe('5개')

    fireEvent.click(acceptBtn())
    await settle()
    expect(f.answerArgs).toEqual([true])
    expect(dialog()).toBeNull()
  })

  it('Tab cycles inside the modal', async () => {
    mount(view('awaiting'))
    await settle()
    rejectBtn().focus()
    fireEvent.keyDown(rejectBtn(), { key: 'Tab' })
    expect(document.activeElement).toBe(acceptBtn())
    fireEvent.keyDown(acceptBtn(), { key: 'Tab', shiftKey: true })
    expect(document.activeElement).toBe(rejectBtn())
  })

  it('after a StrictMode double effect there is one listener · the dispatcher stays stopped', async () => {
    const { f } = mount(view('awaiting'), { strict: true })
    await settle()
    expect(f.hasListener()).toBe(true)
    expect(f.unlistenCalls).toBe(1)
    expect(dialog()).not.toBeNull()
    fireCtrlTab(document.body)
    expect(shortcut).not.toHaveBeenCalled()
  })

  it('when main goes down (unmount) the dispatcher runs again', async () => {
    const { unmount } = mount(view('awaiting'))
    await settle()
    unmount()
    fireCtrlTab(document.body)
    expect(shortcut).toHaveBeenCalledTimes(1)
  })
})

describe('RestoreModal — answer and closing', () => {
  it('accept answers with accept:true, and closes only in the state pulled again after the answer ends (it does not close at the moment of pressing)', async () => {
    const { f, layer } = mount(view('awaiting'))
    await settle()
    const gate = deferred<AnswerReply>()
    f.setOnAnswer(async () => {
      const reply = await gate.promise
      f.server.view = view('answered')
      return reply
    })

    fireEvent.click(acceptBtn())
    await settle()
    expect(f.answerArgs).toEqual([true])
    expect(dialog()).not.toBeNull()
    expect(layer().hasAttribute('inert')).toBe(true)

    gate.resolve({ restored_windows: 2, durable: true })
    await settle()
    expect(dialog()).toBeNull()
    expect(layer().hasAttribute('inert')).toBe(false)
  })

  it('reject answers with accept:false', async () => {
    const { f } = mount(view('awaiting'))
    await settle()
    fireEvent.click(rejectBtn())
    await settle()
    expect(f.answerArgs).toEqual([false])
    expect(dialog()).toBeNull()
  })

  it('while an answer is processing both buttons are disabled and a second answer is not made', async () => {
    const { f } = mount(view('awaiting'))
    await settle()
    const gate = deferred<AnswerReply>()
    f.setOnAnswer(() => gate.promise)

    fireEvent.click(acceptBtn())
    fireEvent.click(rejectBtn())
    await settle()
    expect(acceptBtn().disabled).toBe(true)
    expect(rejectBtn().disabled).toBe(true)
    expect(screen.getByText(t('restore.working'))).not.toBeNull()
    expect(f.answerArgs).toEqual([true])

    gate.resolve({ restored_windows: 0, durable: true })
    await settle()
  })

  it('on failure it shows an error line, pulls again, and revives the buttons while open', async () => {
    const { f } = mount(view('awaiting'))
    await settle()
    f.setOnAnswer(async () => {
      throw 'INTERNAL: shell is still starting'
    })

    fireEvent.click(acceptBtn())
    await settle()
    expect(dialog()).not.toBeNull()
    expect(screen.getByRole('alert').textContent).toBe(t('restore.answerFailed'))
    expect(acceptBtn().disabled).toBe(false)
    expect(rejectBtn().disabled).toBe(false)
    expect(f.calls.slice(-2)).toEqual(['invoke:restore_answer', 'invoke:restore_status'])
    // Do not show the error string as is.
    expect(dialog()?.textContent).not.toContain('INTERNAL')
  })

  it('when the buttons revive after a failure it brings focus that fell outside back to 「restore previous screen」', async () => {
    const { f } = mount(view('awaiting'))
    await settle()
    const gate = deferred<AnswerReply>()
    f.setOnAnswer(() => gate.promise)
    // Chromium releases the focus of a button that became disabled — jsdom does not, and ignores blur on a disabled button, so before pressing,
    //   drop it by hand to create 「focus is outside while processing」.
    acceptBtn().blur()
    fireEvent.click(rejectBtn())
    await settle()
    expect(rejectBtn().disabled).toBe(true)
    expect(document.activeElement).toBe(document.body)

    gate.reject('INTERNAL')
    await settle()
    expect(document.activeElement).toBe(acceptBtn())
  })

  it('pressing again lifts the earlier error line', async () => {
    const { f } = mount(view('awaiting'))
    await settle()
    f.setOnAnswer(async () => {
      throw 'CONFLICT'
    })
    fireEvent.click(acceptBtn())
    await settle()
    expect(screen.queryByRole('alert')).not.toBeNull()

    const gate = deferred<AnswerReply>()
    f.setOnAnswer(() => gate.promise)
    fireEvent.click(acceptBtn())
    await settle()
    expect(screen.queryByRole('alert')).toBeNull()
    gate.resolve({ restored_windows: 0, durable: true })
    await settle()
  })

  it('it stays open even when a revert notice (awaiting) arrives', async () => {
    const { f } = mount(view('awaiting'))
    await settle()
    const gate = deferred<AnswerReply>()
    f.setOnAnswer(() => gate.promise)
    fireEvent.click(acceptBtn())
    await settle()

    f.push('awaiting')
    await settle()
    expect(dialog()).not.toBeNull()

    gate.reject('INTERNAL')
    await settle()
    expect(dialog()).not.toBeNull()
    expect(acceptBtn().disabled).toBe(false)
  })
})

describe('RestoreModal — restore:changed', () => {
  it('it is updated by notices — it shows if it appears from none, and closes if it is an answer (answered) sent over the bus', async () => {
    const { f } = mount(view('none'))
    await settle()
    expect(dialog()).toBeNull()

    f.server.view = view('awaiting', { tabs: 9 })
    f.push('awaiting')
    await settle()
    expect(dialog()?.querySelector('[data-restore-tabs]')?.textContent).toBe('9개')

    f.server.view = view('answered')
    f.push('answered')
    await settle()
    expect(dialog()).toBeNull()
    fireCtrlTab(document.body)
    expect(shortcut).toHaveBeenCalledTimes(1)
  })
})

describe('RestoreModal — the reject warning splits on durable', () => {
  const warningText = () => dialog()?.querySelector('[data-restore-warning]')?.textContent

  it('with durable:true 「deletes · cannot be undone」', async () => {
    mount(view('awaiting', { durable: true }))
    await settle()
    expect(warningText()).toBe(t('restore.rejectWarning'))
  })

  it.each([
    ['a run that could not read state.json (guard ⅰ)', 'unreadable' as const],
    ['a run that could not make a copy (guard ⅱ — state_file is ok)', 'ok' as const],
  ])('with durable:false 「stays as is and is asked again on the next run」 — %s', async (_label, stateFile) => {
    mount(view('awaiting', { durable: false, state_file: stateFile }))
    await settle()
    const text = warningText()
    expect(text).toBe(t('restore.rejectWarningNotSaving'))
    expect(text).not.toContain('되돌릴 수 없')
    // There are two causes, so we do not name the file.
    expect(text).not.toContain('state.json')
  })

  it('with durable:true even if state_file is unreadable 「deletes」 — the warning does not look at state_file', async () => {
    mount(view('awaiting', { durable: true, state_file: 'unreadable' }))
    await settle()
    expect(warningText()).toBe(t('restore.rejectWarning'))
  })

  it('if durable is null while asking (a value that does not come) it leans toward 「deletes · cannot be undone」', async () => {
    mount(view('awaiting', { durable: null }))
    await settle()
    expect(warningText()).toBe(t('restore.rejectWarning'))
  })
})

describe('AppLayout — state file notice', () => {
  it('reads state_file from the same state set and shows it over main', async () => {
    mount(view('none', { state_file: 'corrupt_copied_aside' }))
    await settle()
    expect(screen.getByRole('status').textContent).toContain('state.json.corrupt')
  })

  // For guard ⅱ, `state_file` is `ok`, so after answering, the single field that carries that fact is `saves` (guard ⅱ notice — ADR-0276).
  it.each([
    ['accept', () => acceptBtn()],
    ['reject', () => rejectBtn()],
  ])('the guard ⅱ (saves:false · ok) notice stays after the modal is closed by %s', async (_label, button) => {
    mount(view('awaiting', { saves: false }))
    await settle()
    expect(dialog()).not.toBeNull()

    fireEvent.click(button())
    await settle()
    expect(dialog()).toBeNull()
    const notice = screen.getByRole('status')
    expect(notice.textContent).toContain(t('restore.stateFileNotSaving'))
    expect(notice.getAttribute('data-state-file')).toBe('ok')
  })

  it('a run that saves (saves:true · ok) has no notice even after answering', async () => {
    mount(view('awaiting'))
    await settle()
    fireEvent.click(acceptBtn())
    await settle()
    expect(dialog()).toBeNull()
    expect(screen.queryByRole('status')).toBeNull()
  })
})
