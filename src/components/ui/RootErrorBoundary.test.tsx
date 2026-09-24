import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { t } from '../../i18n'
import { RootErrorBoundary } from './RootErrorBoundary'

let shouldThrow = true
function Bomb() {
  if (shouldThrow) throw new Error('boom')
  return <div data-testid="child">ok</div>
}

describe('RootErrorBoundary', () => {
  let consoleError: ReturnType<typeof vi.spyOn>

  beforeEach(() => {
    shouldThrow = true
    consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
  })

  afterEach(() => {
    cleanup()
    consoleError.mockRestore()
  })

  it('렌더 중 throw 하면 창 전체 대체 표시와 두 버튼을 그린다', () => {
    render(
      <RootErrorBoundary>
        <Bomb />
      </RootErrorBoundary>,
    )
    expect(screen.getByRole('alert').textContent).toContain(t('window.renderFailed'))
    expect(screen.getByRole('button', { name: t('window.redraw') })).toBeTruthy()
    expect(screen.getByRole('button', { name: t('window.reload') })).toBeTruthy()
    expect(screen.queryByTestId('child')).toBeNull()
    expect(consoleError.mock.calls.some((c: unknown[]) => String(c[0]).includes('[RootErrorBoundary]'))).toBe(true)
  })

  it('다시 그리기 — 오류가 멎었으면 자식이 돌아오고, 계속되면 대체 표시가 다시 선다', () => {
    render(
      <RootErrorBoundary>
        <Bomb />
      </RootErrorBoundary>,
    )
    fireEvent.click(screen.getByRole('button', { name: t('window.redraw') }))
    expect(screen.getByRole('alert')).toBeTruthy()

    shouldThrow = false
    fireEvent.click(screen.getByRole('button', { name: t('window.redraw') }))
    expect(screen.getByTestId('child')).toBeTruthy()
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('새로고침 — window.location.reload 를 부른다', () => {
    const reload = vi.fn()
    const original = window.location
    Object.defineProperty(window, 'location', { configurable: true, value: { ...original, reload } })
    try {
      render(
        <RootErrorBoundary>
          <Bomb />
        </RootErrorBoundary>,
      )
      fireEvent.click(screen.getByRole('button', { name: t('window.reload') }))
      expect(reload).toHaveBeenCalledOnce()
    } finally {
      Object.defineProperty(window, 'location', { configurable: true, value: original })
    }
  })
})
