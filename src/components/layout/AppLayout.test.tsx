// This suite also catches a dangling import of a deleted fixed chrome file — if the shell imports them,
// this test file fails to load before any assertion.

import type { ReactNode } from 'react'
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

// Real tab/canvas wiring coverage belongs to WindowLayout.test, so stub it here with a sentinel. The notice layer is received as `notices` and drawn inside it
//   — the area below the title bar belongs to WindowLayout.test 「알림 층의 자리」.
vi.mock('./WindowLayout', () => ({
  default: ({ label, notices }: { label: string; notices?: ReactNode }) => (
    <div data-testid="window-layout" data-label={label}>
      {notices}
    </div>
  ),
}))

// Stand in only for the connection state surface to raise a connection notice (the same shape as ConnectionNotice.test).
const { connection } = vi.hoisted(() => {
  const listeners = new Set<() => void>()
  const connection = {
    listeners,
    connectionError: null as string | null,
    onConnectionStateChange(cb: () => void) {
      listeners.add(cb)
      cb()
      return () => {
        listeners.delete(cb)
      }
    },
  }
  return { connection }
})
vi.mock('../../api/clientFactory', async importOriginal => ({
  ...(await importOriginal<typeof import('../../api/clientFactory')>()),
  agentClient: connection,
}))

import AppLayout from './AppLayout'
import { createRestoreClient, type RestoreStatusView } from '../../api/restoreClient'
import { fakeRestoreIpc, flush, view } from '../../api/testing/fakeRestoreIpc'
import { t } from '../../i18n'
import { MAIN_WINDOW_LABEL } from '../../store/viewStore'

// The restore state is a launch that does not ask (`none` · `ok`) — the modal wiring belongs to RestoreModal.test.
const idleRestore = () => createRestoreClient(fakeRestoreIpc(view('none')).ipc)

async function mount(initial: RestoreStatusView, connectionError: string | null = null) {
  connection.connectionError = connectionError
  const r = render(<AppLayout restore={createRestoreClient(fakeRestoreIpc(initial).ipc)} />)
  await act(async () => {
    await flush()
  })
  const overlay = screen.getByTestId('notice-overlay')
  return { ...r, overlay, layer: r.container.firstElementChild as HTMLElement }
}

afterEach(() => {
  cleanup()
  connection.listeners.clear()
  connection.connectionError = null
})

describe('AppLayout — 슬롯화된 셸(ADR-0063)', () => {
  it('main 창 WindowLayout 을 label="main" 으로 마운트한다', () => {
    render(<AppLayout restore={idleRestore()} />)
    const wl = screen.getByTestId('window-layout')
    expect(wl).toBeTruthy()
    expect(wl.getAttribute('data-label')).toBe(MAIN_WINDOW_LABEL)
  })

  it('옛 고정 크롬(Sidebar/DiffPanel/StatusBar) 잔재가 없다', () => {
    render(<AppLayout restore={idleRestore()} />)
    expect(screen.queryByText('Agent Tree')).toBeNull() // The old Sidebar header
    expect(screen.queryByText('Ready')).toBeNull() // The old StatusBar
    expect(screen.queryByText('Accept')).toBeNull() // The old DiffPanel
    expect(screen.queryByText('▶')).toBeNull() // The old sidebar reopen toggle
  })
})

// jsdom cannot measure size — assert 「밀지 않는다」 (does not push) by the computed position and structure. The layer itself (as many rows as there are · scroll · stacking) belongs to
// NoticeOverlay.test.
describe('AppLayout — 알림은 레이아웃을 덮는다(ADR-0276 · ADR-0277)', () => {
  // Where it covers is decided by WindowLayout — the canvas area below the title bar (user decision 2026-10-26 · T-66). Here we only check that the layer is
  //   passed as `notices` and that it is a covering layer (absolute).
  it('알림 층은 main WindowLayout 의 notices 로 넘어가고, 계산된 위치가 absolute 로 기준 칸의 맨 위에 붙는다', async () => {
    const { overlay } = await mount(view('none', { state_file: 'unreadable', saves: false }))
    const style = getComputedStyle(overlay)
    expect(style.position).toBe('absolute')
    expect([style.top, style.left, style.right]).toEqual(['0px', '0px', '0px'])
    expect(overlay.contains(screen.getByRole('status'))).toBe(true)

    const windowLayout = screen.getByTestId('window-layout')
    expect(windowLayout.getAttribute('data-label')).toBe(MAIN_WINDOW_LABEL)
    expect(windowLayout.contains(overlay)).toBe(true)
    // The wrapping area = WindowLayout's place (keep it positioned — the reference of the notice layer is the area inside WindowLayout).
    expect(getComputedStyle(windowLayout.parentElement as HTMLElement).position).toBe('relative')
  })

  it('두 알림이 한 층 안에 연결 띠 → 상태 파일 알림 차례로 쌓인다', async () => {
    const { overlay } = await mount(view('answered', { saves: false }), '폴더 문제')
    const alert = screen.getByRole('alert')
    const status = screen.getByRole('status')
    expect(overlay.contains(alert)).toBe(true)
    expect(overlay.contains(status)).toBe(true)
    expect(alert.compareDocumentPosition(status) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  })

  it('알림이 둘 다 떠도 레이아웃 칸은 그대로다 — 세로 스택의 나머지를 다 쓰고, 흐름 안의 자식은 WindowLayout 뿐이다', async () => {
    const { overlay, layer } = await mount(view('answered', { saves: false }), '폴더 문제')
    expect(screen.getByRole('alert')).toBeTruthy()
    expect(screen.getByRole('status')).toBeTruthy()
    expect(layer.style.height).toBe('100%')
    expect(layer.style.display).toBe('flex')
    expect(layer.style.flexDirection).toBe('column')

    const windowLayout = screen.getByTestId('window-layout')
    const box = windowLayout.parentElement as HTMLElement
    expect([...layer.children]).toEqual([box])
    expect(box.style.flexGrow).toBe('1')
    expect(box.style.minHeight).toBe('0px')
    expect(box.style.position).toBe('relative')
    expect([...box.children]).toEqual([windowLayout])
    expect(windowLayout.contains(overlay)).toBe(true)
  })

  it('알림이 없어도 층은 레이아웃 칸 안에 있고 비어 있다', async () => {
    const { overlay } = await mount(view('none'))
    expect(screen.queryByRole('alert')).toBeNull()
    expect(screen.queryByRole('status')).toBeNull()
    expect(screen.getByTestId('notice-stack').childElementCount).toBe(0)
    expect(overlay.contains(screen.getByTestId('notice-stack'))).toBe(true)
  })

  it('층은 묻는 동안 잠기는 층 안이다 — 두 알림의 ✕ 도 함께 잠긴다', async () => {
    const { overlay, layer } = await mount(view('awaiting', { saves: false }), '폴더 문제')
    expect(layer.hasAttribute('inert')).toBe(true)
    expect(layer.contains(overlay)).toBe(true)
    expect(overlay.contains(screen.getByLabelText('알림 닫기'))).toBe(true)
    expect(overlay.contains(screen.getByLabelText(t('restore.dismissNotice')))).toBe(true)
  })

  it('층 안에서 각 알림이 따로 ✕ 로 닫힌다', async () => {
    await mount(view('answered', { saves: false }), '폴더 문제')
    fireEvent.click(screen.getByLabelText('알림 닫기'))
    expect(screen.queryByRole('alert')).toBeNull()
    expect(screen.getByRole('status')).toBeTruthy()

    fireEvent.click(screen.getByLabelText(t('restore.dismissNotice')))
    expect(screen.queryByRole('status')).toBeNull()
  })
})
