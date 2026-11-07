// LoadingPanel contract (ADR-0226). The rotation (always spins, even under reduced-motion · e-ink — user decision 2026-09-24)
//   cannot be measured here because jsdom does not run css media · keyframes — it is for GUI measurement.

import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import { t } from '../../i18n'
import { LoadingPanel } from './LoadingPanel'

afterEach(() => cleanup())

describe('LoadingPanel', () => {
  it('상태 역할 + 화면 밖 이름 = common.loading, 보이는 텍스트는 없다', () => {
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

  it('label 을 주면 보이게 그리되 aria 이름은 그대로다', () => {
    render(<LoadingPanel label="이력을 불러오는 중" />)
    const panel = screen.getByRole('status')
    expect(panel.textContent).toBe('이력을 불러오는 중')
    expect(panel.getAttribute('aria-label')).toBe(t('common.loading'))
  })

  it.each([false, true, ''])('그리지 않는 label(%j)에는 빈 텍스트 칸을 세우지 않는다', (label) => {
    render(<LoadingPanel label={label} />)
    const panel = screen.getByRole('status')
    expect(panel.children).toHaveLength(1) // Icon only
    expect(panel.firstElementChild?.tagName.toLowerCase()).toBe('svg')
  })

  it('className 이 붙는다(자리·배경은 부르는 쪽이 정한다)', () => {
    render(<LoadingPanel className="absolute inset-0" />)
    expect(screen.getByRole('status').className).toContain('absolute inset-0')
  })
})
