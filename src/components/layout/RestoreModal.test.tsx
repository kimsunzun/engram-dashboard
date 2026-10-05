// RestoreModal — main 을 덮고 입력을 막는다 · 단축키 멈춤 · 상태로만 닫힌다(TRD S21-storage §6-7 · §8 「프론트 모달」).
//
// 배선 전체(AppLayout + restoreClient + 단축키 디스패처)를 가짜 IPC 위에서 돌린다 — 모달 홀로는 「main 을 막는다」를
// 잴 수 없다(잠금은 AppLayout 의 `inert` 와 모달의 포커스 · 멈춤이 한 벌이다).

import { StrictMode } from 'react'
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest'

// 캔버스는 관심 밖 — 그 아래 입력이 잠기는지만 본다.
vi.mock('./WindowLayout', () => ({
  default: () => (
    <div data-testid="window-layout">
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
import { t } from '../../i18n'
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
  // 프래그먼트의 첫 자식 = 잠기는 레이아웃 층.
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
  register({ id: 'tab.next', title: 'next', run: shortcut })
  disposeKeys = installKeybindings()
  vi.spyOn(console, 'warn').mockImplementation(() => {})
})

afterEach(() => {
  cleanup()
  disposeKeys?.()
  disposeKeys = null
  vi.restoreAllMocks()
})

describe('RestoreModal — 덮고 막는다', () => {
  it('첫 당기기가 awaiting 이면 바로 그리고, 그 아래 레이아웃을 inert 로 잠근다', async () => {
    const { layer } = mount(view('awaiting'))
    await settle()
    expect(dialog()).not.toBeNull()
    expect(layer().hasAttribute('inert')).toBe(true)
    expect(layer().contains(screen.getByTestId('under-input'))).toBe(true)
    // 모달은 잠긴 층 밖이다 — 안에 있으면 자기도 잠긴다.
    expect(layer().contains(dialog())).toBe(false)
  })

  it('알림 층도 잠기는 층 안이고, 모달은 그 밖 · 그 위(z-50 > 40)다', async () => {
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

  it('saves 칸이 없는 짐(셸과 판이 어긋남)이어도 모달이 뜬다 — 가드 ⅱ 알림은 없다', async () => {
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

  it('state_file 이 모르는 값인 짐(셸과 판이 어긋남)이어도 모달이 뜬다 — 알림은 없다', async () => {
    mount({ ...view('awaiting', { saves: false }), state_file: 'from_a_newer_shell' as never })
    await settle()
    expect(dialog()).not.toBeNull()
    expect(screen.queryByRole('status')).toBeNull()

    fireEvent.click(acceptBtn())
    await settle()
    expect(dialog()).toBeNull()
    expect(screen.queryByRole('status')).toBeNull()
  })

  it('awaiting 이 아니면 덮는 층도 inert 도 없다', async () => {
    const { layer, container } = mount(view('none'))
    await settle()
    expect(dialog()).toBeNull()
    expect(container.querySelector('[data-restore-overlay]')).toBeNull()
    expect(layer().hasAttribute('inert')).toBe(false)
  })

  it('저장 시각 · 창 수 · 탭 수만 보여 준다', async () => {
    const at = new Date(2026, 9, 5, 14, 3).getTime()
    mount(view('awaiting', { saved_at_ms: at, windows: 3, tabs: 7 }))
    await settle()
    const d = dialog() as HTMLElement
    expect(d.querySelector('[data-restore-saved-at]')?.textContent).toBe(formatSavedAt(at))
    expect(formatSavedAt(at)).toContain('14:03')
    expect(d.querySelector('[data-restore-windows]')?.textContent).toBe('3개')
    expect(d.querySelector('[data-restore-tabs]')?.textContent).toBe('7개')
  })

  it('떠 있는 동안 단축키 디스패처가 멈추고, 답한 뒤 다시 돈다(N4)', async () => {
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

  it('포커스를 안의 「이전 화면 복원」으로 옮긴다 — 그 아래 입력에 있던 포커스도', async () => {
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

  it('단추 밖을 눌러도 포커스가 떨어지지 않는다(mousedown 기본 동작을 막는다) — 단추는 그대로 눌린다', async () => {
    const { container } = mount(view('awaiting'))
    await settle()
    const overlay = container.querySelector('[data-restore-overlay]') as HTMLElement
    expect(fireEvent.mouseDown(overlay)).toBe(false)
    expect(fireEvent.mouseDown(screen.getByRole('heading'))).toBe(false)
    expect(fireEvent.mouseDown(acceptBtn())).toBe(true)
  })

  it('formatSavedAt — Date 범위(±8.64e15) 안은 형식화하고 밖 · 유한하지 않은 값은 null', () => {
    expect(formatSavedAt(8.64e15)).not.toBeNull()
    expect(formatSavedAt(-8.64e15)).not.toBeNull()
    expect(formatSavedAt(8.64e15 + 1)).toBeNull()
    expect(formatSavedAt(1.8e19)).toBeNull()
    expect(formatSavedAt(Number.POSITIVE_INFINITY)).toBeNull()
    expect(formatSavedAt(Number.NaN)).toBeNull()
  })

  it('저장 시각이 Date 로 나타낼 수 없는 값(u64 상한 근처)이면 그 줄만 빼고 모달은 그대로 쓸 수 있다', async () => {
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

  it('Tab 은 모달 안에서 돈다', async () => {
    mount(view('awaiting'))
    await settle()
    rejectBtn().focus()
    fireEvent.keyDown(rejectBtn(), { key: 'Tab' })
    expect(document.activeElement).toBe(acceptBtn())
    fireEvent.keyDown(acceptBtn(), { key: 'Tab', shiftKey: true })
    expect(document.activeElement).toBe(rejectBtn())
  })

  it('StrictMode 이중 effect 뒤에도 리스너는 하나 · 디스패처는 멈춘 채다', async () => {
    const { f } = mount(view('awaiting'), { strict: true })
    await settle()
    expect(f.hasListener()).toBe(true)
    expect(f.unlistenCalls).toBe(1)
    expect(dialog()).not.toBeNull()
    fireCtrlTab(document.body)
    expect(shortcut).not.toHaveBeenCalled()
  })

  it('main 이 내려가면(언마운트) 디스패처가 다시 돈다', async () => {
    const { unmount } = mount(view('awaiting'))
    await settle()
    unmount()
    fireCtrlTab(document.body)
    expect(shortcut).toHaveBeenCalledTimes(1)
  })
})

describe('RestoreModal — 답과 닫힘', () => {
  it('수락은 accept:true 로 답하고, 답이 끝나 다시 당긴 상태로만 닫힌다(누르는 순간 닫지 않는다)', async () => {
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

  it('거절은 accept:false 로 답한다', async () => {
    const { f } = mount(view('awaiting'))
    await settle()
    fireEvent.click(rejectBtn())
    await settle()
    expect(f.answerArgs).toEqual([false])
    expect(dialog()).toBeNull()
  })

  it('답이 처리 중이면 두 단추가 비활성이고 두 번째 답을 내지 않는다', async () => {
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

  it('실패하면 오류 줄을 띄우고, 다시 당기고, 열린 채 단추를 되살린다', async () => {
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
    // 오류 문자열을 그대로 내보이지 않는다.
    expect(dialog()?.textContent).not.toContain('INTERNAL')
  })

  it('실패 뒤 단추가 살아날 때 밖으로 떨어진 포커스를 「이전 화면 복원」으로 데려온다', async () => {
    const { f } = mount(view('awaiting'))
    await settle()
    const gate = deferred<AnswerReply>()
    f.setOnAnswer(() => gate.promise)
    // Chromium 은 비활성이 된 단추의 포커스를 놓는다 — jsdom 은 놓지 않고 비활성 단추의 blur 도 무시하므로, 누르기 전에
    //   손으로 떨어뜨려 「처리 중 포커스가 밖에 있다」를 만든다.
    acceptBtn().blur()
    fireEvent.click(rejectBtn())
    await settle()
    expect(rejectBtn().disabled).toBe(true)
    expect(document.activeElement).toBe(document.body)

    gate.reject('INTERNAL')
    await settle()
    expect(document.activeElement).toBe(acceptBtn())
  })

  it('다시 누르면 앞 오류 줄을 걷는다', async () => {
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

  it('되돌림 알림(awaiting)이 와도 열린 채다', async () => {
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
  it('알림으로 갱신된다 — 없다가 생기면 뜨고, 버스로 낸 답(answered)이면 닫힌다', async () => {
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

describe('RestoreModal — 거절 경고는 durable 로 가른다', () => {
  const warningText = () => dialog()?.querySelector('[data-restore-warning]')?.textContent

  it('durable:true 면 「지운다 · 되돌릴 수 없다」', async () => {
    mount(view('awaiting', { durable: true }))
    await settle()
    expect(warningText()).toBe(t('restore.rejectWarning'))
  })

  it.each([
    ['state.json 을 못 읽은 실행(가드 ⅰ)', 'unreadable' as const],
    ['사본을 못 뜬 실행(가드 ⅱ — state_file 은 ok)', 'ok' as const],
  ])('durable:false 면 「그대로 남고 다음 실행 때 다시 묻는다」 — %s', async (_label, stateFile) => {
    mount(view('awaiting', { durable: false, state_file: stateFile }))
    await settle()
    const text = warningText()
    expect(text).toBe(t('restore.rejectWarningNotSaving'))
    expect(text).not.toContain('되돌릴 수 없')
    // 원인이 둘이라 파일 이름을 대지 않는다.
    expect(text).not.toContain('state.json')
  })

  it('durable:true 면 state_file 이 unreadable 이어도 「지운다」 — 경고는 state_file 을 보지 않는다', async () => {
    mount(view('awaiting', { durable: true, state_file: 'unreadable' }))
    await settle()
    expect(warningText()).toBe(t('restore.rejectWarning'))
  })

  it('묻는 동안 durable 이 null 이면(오지 않는 값) 「지운다 · 되돌릴 수 없다」 쪽으로 둔다', async () => {
    mount(view('awaiting', { durable: null }))
    await settle()
    expect(warningText()).toBe(t('restore.rejectWarning'))
  })
})

describe('AppLayout — 상태 파일 알림', () => {
  it('같은 상태 한 벌에서 state_file 을 읽어 main 위에 띄운다', async () => {
    mount(view('none', { state_file: 'corrupt_copied_aside' }))
    await settle()
    expect(screen.getByRole('status').textContent).toContain('state.json.corrupt')
  })

  // 가드 ⅱ 는 `state_file` 이 `ok` 라 답한 뒤에 그 사실을 나르는 칸이 `saves` 하나다(사용자 결정 2026-10-06).
  it.each([
    ['수락', () => acceptBtn()],
    ['거절', () => rejectBtn()],
  ])('가드 ⅱ(saves:false · ok) 알림은 %s으로 모달이 닫힌 뒤에도 남는다', async (_label, button) => {
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

  it('저장하는 실행(saves:true · ok)은 답한 뒤에도 알림이 없다', async () => {
    mount(view('awaiting'))
    await settle()
    fireEvent.click(acceptBtn())
    await settle()
    expect(dialog()).toBeNull()
    expect(screen.queryByRole('status')).toBeNull()
  })
})
