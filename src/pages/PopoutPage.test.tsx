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

// ── WindowLayout stub — internal logic is covered by WindowLayout.test · the notice layer is received as `notices` and drawn inside it (the spot below the title bar
//   is WindowLayout.test's 「the notice layer's spot」 share) ──
vi.mock('../components/layout/WindowLayout', () => ({
  default: ({ label, notices }: { label: string; notices?: ReactNode }) => (
    <div data-testid="window-layout" data-label={label}>
      {notices}
    </div>
  ),
}))

// ── Connection-state surface stand-in — to bring up the connection strip (same shape as ConnectionNotice.test) ──
const { connection } = vi.hoisted(() => {
  const listeners = new Set<() => void>()
  const connection = {
    listeners,
    connectionError: null as string | null,
    connectionBand: null as Band | null,
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

import type { Band } from '../../src-tauri/bindings/Band'
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
  connection.connectionBand = null
})

describe('PopoutPage (tab ownership model, ADR-0057)', () => {
  it('?window=<label> → WindowLayout mounts with that label', () => {
    render(<PopoutPage />)
    const wl = screen.getByTestId('window-layout')
    expect(wl.getAttribute('data-label')).toBe(POPUP_LABEL)
  })

  it('★view:closed retired (G2)★: PopoutPage does not subscribe to view:closed (the self-exit listener removed)', () => {
    render(<PopoutPage />)
    // Old bug: window self-exit via a view:closed listener → double fire/re-entry.
    expect(listeners.has('view:closed')).toBe(false)
  })
})

// jsdom cannot measure sizes — assert 「does not push」 with computed position and structure.
describe('PopoutPage — the connection band covers the layout (ADR-0277)', () => {
  function mount(connectionError: string | null) {
    connection.connectionError = connectionError
    // The shell raises the band — a cause alone shows nothing.
    connection.connectionBand = connectionError === null ? null : { kind: 'not_connected', refused: false }
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

  // The covering spot is decided by WindowLayout — the canvas area below the title bar (user decision 2026-10-26 · T-66).
  it('the connection band is inside the covering layer, and the layer goes to WindowLayout\'s notices so the computed position is absolute attached at the very top of the reference area', () => {
    const { box, overlay, windowLayout } = mount('folder problem')
    const alert = screen.getByRole('alert')
    expect(overlay.contains(alert)).toBe(true)
    const style = getComputedStyle(overlay)
    expect(style.position).toBe('absolute')
    expect([style.top, style.left, style.right]).toEqual(['0px', '0px', '0px'])
    expect(windowLayout.contains(overlay)).toBe(true)
    // Wrapping area = WindowLayout's spot (keep it positioned — the notice layer's reference is an area inside WindowLayout).
    expect(getComputedStyle(box).position).toBe('relative')
  })

  it('even with the connection band up the content area is as is — it uses the rest of the vertical stack, and the only child in flow is WindowLayout', () => {
    const { root, box, windowLayout } = mount('folder problem')
    expect(screen.getByRole('alert')).toBeTruthy()
    expect([root.style.width, root.style.height]).toEqual(['100vw', '100vh'])
    expect(root.style.display).toBe('flex')
    expect(root.style.flexDirection).toBe('column')
    expect([...root.children]).toEqual([box])
    expect(box.style.flexGrow).toBe('1')
    expect(box.style.minHeight).toBe('0px')
    expect([...box.children]).toEqual([windowLayout])
  })

  it('with no reason the layer is empty', () => {
    mount(null)
    expect(screen.queryByRole('alert')).toBeNull()
    expect(screen.getByTestId('notice-stack').childElementCount).toBe(0)
  })

  it('even in the layer it closes with ✕', () => {
    mount('folder problem')
    fireEvent.click(screen.getByLabelText('알림 닫기'))
    expect(screen.queryByRole('alert')).toBeNull()
  })
})

describe('PopoutPage — the band is text only (TRD S36 §3-4)', () => {
  it.each([
    ['band ①', { kind: 'not_connected', refused: false } as Band, '데몬에 연결하지 못했습니다.'],
    ['band ②', { kind: 'version', can_stop: true } as Band, '실행 중인 데몬이 이 앱과 버전이 다릅니다'],
  ])('shows %s with no button but ✕', (_name, band, text) => {
    connection.connectionBand = band
    render(<PopoutPage />)
    expect(screen.getByRole('alert').textContent).toContain(text)
    expect(screen.getAllByRole('button').map(b => b.getAttribute('aria-label'))).toEqual(['알림 닫기'])
  })
})
