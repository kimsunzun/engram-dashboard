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

  it('on a render throw it draws the whole-window fallback and two buttons', () => {
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

  it('redraw — if the error stopped the children return, and if it continues the fallback stands again', () => {
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

  it('reload — calls window.location.reload', () => {
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
