// LoadingPanel 계약(ADR-0226). reduced-motion·e-ink 에서의 회전 정지는 jsdom 이 css 미디어·keyframes 를
//   돌리지 않아 여기서 못 잰다 — GUI 실측 몫이다.

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
    // 아이콘은 장식이라 접근성 트리에서 숨긴다 — 이름은 패널이 진다.
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
    expect(panel.children).toHaveLength(1) // 아이콘뿐
    expect(panel.firstElementChild?.tagName.toLowerCase()).toBe('svg')
  })

  it('className 이 붙는다(자리·배경은 부르는 쪽이 정한다)', () => {
    render(<LoadingPanel className="absolute inset-0" />)
    expect(screen.getByRole('status').className).toContain('absolute inset-0')
  })
})
