// 삭제된 고정 크롬 파일의 dangling import 도 이 스위트가 잡는다 — 셸이 그것들을 import 하면
// 단언 이전에 이 테스트 파일이 로드 실패한다.

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

// 실제 탭/캔버스 배선 커버리지는 WindowLayout.test 담당이라 여기선 sentinel 로 stub.
vi.mock('./WindowLayout', () => ({
  default: ({ label }: { label: string }) => <div data-testid="window-layout" data-label={label} />,
}))

// 연결 알림을 띄우려고 연결 상태 표면만 대역한다(ConnectionNotice.test 와 같은 모양).
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

// 복원 상태는 묻지 않는 실행(`none` · `ok`) — 모달 배선은 RestoreModal.test 몫이다.
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
    expect(screen.queryByText('Agent Tree')).toBeNull() // 옛 Sidebar 헤더
    expect(screen.queryByText('Ready')).toBeNull() // 옛 StatusBar
    expect(screen.queryByText('Accept')).toBeNull() // 옛 DiffPanel
    expect(screen.queryByText('▶')).toBeNull() // 옛 사이드바 재열기 토글
  })
})

// jsdom 은 크기를 재지 못한다 — 「밀지 않는다 · 줄만큼만 덮는다」를 계산된 위치와 구조로 단언한다.
describe('AppLayout — 상태 파일 알림은 레이아웃을 덮는다(사용자 결정 2026-10-06)', () => {
  it('알림 층의 계산된 위치가 absolute 이고 레이아웃 칸의 맨 위에 붙는다', async () => {
    const { overlay } = await mount(view('none', { state_file: 'unreadable', saves: false }))
    const style = getComputedStyle(overlay)
    expect(style.position).toBe('absolute')
    expect([style.top, style.left, style.right]).toEqual(['0px', '0px', '0px'])
    expect(overlay.contains(screen.getByRole('status'))).toBe(true)

    const box = overlay.parentElement as HTMLElement
    expect(getComputedStyle(box).position).toBe('relative')
    expect(box.contains(screen.getByTestId('window-layout'))).toBe(true)
    expect(overlay.contains(screen.getByTestId('window-layout'))).toBe(false)
  })

  it('알림이 떠도 레이아웃 칸은 그대로다 — 세로 스택의 나머지를 다 쓰고, 흐름 안의 형제는 레이아웃뿐이다', async () => {
    const { overlay, layer } = await mount(view('answered', { saves: false }))
    expect(screen.getByRole('status')).toBeTruthy()
    expect(layer.style.display).toBe('flex')
    expect(layer.style.flexDirection).toBe('column')

    const windowLayout = screen.getByTestId('window-layout')
    const box = windowLayout.parentElement as HTMLElement
    expect(box.parentElement).toBe(layer)
    expect(box.style.minHeight).toBe('0px')
    expect([...box.children]).toEqual([overlay, windowLayout])
  })

  it('층은 줄만큼만 덮는다 — 높이 · 아래 끝을 정하지 않는다', async () => {
    const { overlay } = await mount(view('none'))
    expect(screen.queryByRole('status')).toBeNull()
    expect(overlay.style.height).toBe('')
    expect(overlay.style.bottom).toBe('')
    expect(overlay.className).not.toMatch(/\binset-0\b|\bbottom-0\b|\bh-full\b|\bh-screen\b/)
  })

  it('넘치면 스크롤한다 — 스크롤 노드에 높이 상한과 세로 스크롤', async () => {
    const { overlay } = await mount(view('none', { state_file: 'corrupt_not_copied' }))
    const viewport = overlay.querySelector<HTMLElement>('[data-radix-scroll-area-viewport]')
    expect(viewport).not.toBeNull()
    expect(viewport?.className).toContain('max-h-[33vh]')
    expect(viewport?.style.overflowY).toBe('scroll')
    expect(viewport?.contains(screen.getByRole('status'))).toBe(true)
  })

  it('층 안에서도 ✕ 로 닫힌다', async () => {
    await mount(view('answered', { saves: false }))
    fireEvent.click(screen.getByLabelText(t('restore.dismissNotice')))
    expect(screen.queryByRole('status')).toBeNull()
  })
})

describe('AppLayout — 연결 띠는 흐름 안이다(ADR-0180)', () => {
  it('연결 띠는 덮는 층 밖에서 레이아웃 칸 위에 쌓인다', async () => {
    const { overlay, layer } = await mount(view('answered', { saves: false }), '폴더 문제')
    const alert = screen.getByRole('alert')
    expect(overlay.contains(alert)).toBe(false)
    expect(alert.parentElement).toBe(layer)
    expect(layer.firstElementChild).toBe(alert)
    expect(alert.compareDocumentPosition(overlay) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  })
})
