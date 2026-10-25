import type { ReactNode } from 'react'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ── listen mock ──
const listeners = new Map<string, (e: { payload: unknown }) => void>()
const unlistenMock = vi.fn()
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: (e: { payload: unknown }) => void) => {
    listeners.set(event, handler)
    return unlistenMock
  }),
}))

// ── invoke mock ──
const invokeMock = vi.fn(async (_cmd: string, ..._rest: unknown[]) => undefined as unknown)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, ...rest: unknown[]) => invokeMock(cmd, ...rest),
  Channel: class {
    onmessage: unknown = null
  },
}))

// ── getCurrentWindow mock ──
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ label: () => 'slot-popup-1' }),
}))

// ── WindowLayout stub — 내부 로직은 WindowLayout.test 가 커버 · 알림 층은 `notices` 로 받아 그 안에 그린다(제목 줄 아래
//   자리는 WindowLayout.test 「알림 층의 자리」 몫) ──
vi.mock('../components/layout/WindowLayout', () => ({
  default: ({ label, notices }: { label: string; notices?: ReactNode }) => (
    <div data-testid="window-layout" data-label={label}>
      {notices}
    </div>
  ),
}))

// ── 연결 상태 표면 대역 — 연결 띠를 띄우려고(ConnectionNotice.test 와 같은 모양) ──
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
vi.mock('../api/clientFactory', async importOriginal => ({
  ...(await importOriginal<typeof import('../api/clientFactory')>()),
  agentClient: connection,
}))

import PopoutPage from './PopoutPage'

const POPUP_LABEL = 'slot-popup-1'

const origHash = window.location.hash

beforeEach(() => {
  listeners.clear()
  unlistenMock.mockClear()
  invokeMock.mockReset()
  invokeMock.mockImplementation(async () => undefined)
  window.location.hash = `#/popup?window=${POPUP_LABEL}`
})

afterEach(() => {
  cleanup()
  window.location.hash = origHash
  connection.listeners.clear()
  connection.connectionError = null
})

describe('PopoutPage (탭 소유 모델, ADR-0057)', () => {
  it('?window=<label> → WindowLayout 이 그 label 로 마운트된다', () => {
    render(<PopoutPage />)
    const wl = screen.getByTestId('window-layout')
    expect(wl.getAttribute('data-label')).toBe(POPUP_LABEL)
  })

  it('★view:closed 은퇴(G2)★: PopoutPage 는 view:closed 를 구독하지 않는다(자가종료 리스너 제거)', () => {
    render(<PopoutPage />)
    // 옛 버그: view:closed 리스너로 창 자가종료 → 이중 발화/재진입.
    expect(listeners.has('view:closed')).toBe(false)
  })
})

// jsdom 은 크기를 재지 못한다 — 「밀지 않는다」를 계산된 위치와 구조로 단언한다.
describe('PopoutPage — 연결 띠는 레이아웃을 덮는다(ADR-0277)', () => {
  function mount(connectionError: string | null) {
    connection.connectionError = connectionError
    const r = render(<PopoutPage />)
    const root = r.container.firstElementChild as HTMLElement
    const windowLayout = screen.getByTestId('window-layout')
    return {
      root,
      windowLayout,
      box: windowLayout.parentElement as HTMLElement,
      overlay: screen.getByTestId('notice-overlay'),
    }
  }

  // 덮는 자리는 WindowLayout 이 정한다 — 제목 줄 아래 캔버스 칸(사용자 결정 2026-10-26 · T-66).
  it('연결 띠는 덮는 층 안이고, 층은 WindowLayout 의 notices 로 넘어가 계산된 위치가 absolute 로 기준 칸의 맨 위에 붙는다', () => {
    const { box, overlay, windowLayout } = mount('폴더 문제')
    const alert = screen.getByRole('alert')
    expect(overlay.contains(alert)).toBe(true)
    const style = getComputedStyle(overlay)
    expect(style.position).toBe('absolute')
    expect([style.top, style.left, style.right]).toEqual(['0px', '0px', '0px'])
    expect(windowLayout.contains(overlay)).toBe(true)
    // 감싼 칸 = WindowLayout 의 자리(positioned 로 둔다 — 알림 층의 기준은 WindowLayout 안 칸이다).
    expect(getComputedStyle(box).position).toBe('relative')
  })

  it('연결 띠가 떠도 내용 칸은 그대로다 — 세로 스택의 나머지를 다 쓰고, 흐름 안의 자식은 WindowLayout 뿐이다', () => {
    const { root, box, windowLayout } = mount('폴더 문제')
    expect(screen.getByRole('alert')).toBeTruthy()
    expect([root.style.width, root.style.height]).toEqual(['100vw', '100vh'])
    expect(root.style.display).toBe('flex')
    expect(root.style.flexDirection).toBe('column')
    expect([...root.children]).toEqual([box])
    expect(box.style.flexGrow).toBe('1')
    expect(box.style.minHeight).toBe('0px')
    expect([...box.children]).toEqual([windowLayout])
  })

  it('이유가 없으면 층은 비어 있다', () => {
    mount(null)
    expect(screen.queryByRole('alert')).toBeNull()
    expect(screen.getByTestId('notice-stack').childElementCount).toBe(0)
  })

  it('층 안에서도 ✕ 로 닫힌다', () => {
    mount('폴더 문제')
    fireEvent.click(screen.getByLabelText('알림 닫기'))
    expect(screen.queryByRole('alert')).toBeNull()
  })
})
