// LoadingPanel contract (ADR-0226). The rotation (always spins, even under reduced-motion · e-ink — user decision 2026-09-24)
//   cannot be measured here because jsdom does not run css media · keyframes — it is for GUI measurement.

import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import { t } from '../../i18n'
import { LoadingPanel } from './LoadingPanel'

afterEach(() => cleanup())

describe('LoadingPanel', () => {
  it('status role + off-screen name = common.loading, there is no visible text', () => {
    render(<LoadingPanel />)
    const panel = screen.getByRole('status')
    expect(panel.getAttribute('aria-label')).toBe(t('common.loading'))
    expect(panel.getAttribute('aria-busy')).toBe('true')
    expect(panel.getAttribute('data-loading-panel')).toBe('1')
    expect(panel.textContent).toBe('')
    // The icon is decoration so it is hidden from the accessibility tree — the panel carries the name.
    const icon = panel.querySelector('svg')
    expect(icon).not.toBeNull()
    expect(icon?.getAttribute('aria-hidden')).toBe('true')
  })

  it('if label is given it is drawn visibly but the aria name stays as is', () => {
    render(<LoadingPanel label="이력을 불러오는 중" />)
    const panel = screen.getByRole('status')
    expect(panel.textContent).toBe('이력을 불러오는 중')
    expect(panel.getAttribute('aria-label')).toBe(t('common.loading'))
  })

  it.each([false, true, ''])('does not set up an empty text cell for a label that is not drawn (%j)', (label) => {
    render(<LoadingPanel label={label} />)
    const panel = screen.getByRole('status')
    expect(panel.children).toHaveLength(1) // Icon only
    expect(panel.firstElementChild?.tagName.toLowerCase()).toBe('svg')
  })

  it('if className is attached the place · background is decided by the caller', () => {
    render(<LoadingPanel className="absolute inset-0" />)
    expect(screen.getByRole('status').className).toContain('absolute inset-0')
  })
})
