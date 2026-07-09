// TabBar 단위테스트 — 탭 렌더 + 클릭 액션이 올바른 콜백을 부르는지(ADR-0057, §7-2).
//
// ★검증 불변식★:
//   1. tabs 를 렌더하고 active 탭을 강조(data-active)한다.
//   2. 탭 클릭 → onSwitch(그 view id).
//   3. [+] 클릭 → onCreate.
//   4. 탭 × 클릭 → onClose(그 view id), 부모 onSwitch 로 버블 안 함(stopPropagation).

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import TabBar from './TabBar'
import type { ViewMeta } from '../../api/layoutTypes'

afterEach(cleanup)

const TABS: ViewMeta[] = [
  { id: 'v1', name: 'View 1' },
  { id: 'v2', name: 'View 2' },
]

function renderBar(active = 'v1') {
  const onSwitch = vi.fn()
  const onCreate = vi.fn()
  const onClose = vi.fn()
  render(
    <TabBar label="main" tabs={TABS} active={active} onSwitch={onSwitch} onCreate={onCreate} onClose={onClose} />,
  )
  return { onSwitch, onCreate, onClose }
}

describe('TabBar', () => {
  it('tabs 를 렌더하고 active 탭을 data-active=true 로 강조한다', () => {
    renderBar('v2')
    const tabs = screen.getAllByTestId('tab')
    expect(tabs).toHaveLength(2)
    const active = tabs.find(t => t.getAttribute('data-view-id') === 'v2')!
    const inactive = tabs.find(t => t.getAttribute('data-view-id') === 'v1')!
    expect(active.getAttribute('data-active')).toBe('true')
    expect(inactive.getAttribute('data-active')).toBe('false')
  })

  it('탭 클릭 → onSwitch(그 view id)', () => {
    const { onSwitch } = renderBar('v1')
    const tab2 = screen.getAllByTestId('tab').find(t => t.getAttribute('data-view-id') === 'v2')!
    fireEvent.click(tab2)
    expect(onSwitch).toHaveBeenCalledWith('v2')
  })

  it('[+] 클릭 → onCreate', () => {
    const { onCreate } = renderBar()
    fireEvent.click(screen.getByTestId('tab-add'))
    expect(onCreate).toHaveBeenCalledTimes(1)
  })

  it('탭 × 클릭 → onClose(그 view id) + 부모 onSwitch 로 버블 안 함(stopPropagation)', () => {
    const { onClose, onSwitch } = renderBar('v1')
    const closeBtns = screen.getAllByTestId('tab-close')
    // v2 탭의 닫기 버튼(두 번째).
    fireEvent.click(closeBtns[1])
    expect(onClose).toHaveBeenCalledWith('v2')
    // ★stopPropagation★: 닫기 클릭이 부모 탭 onSwitch 로 새면 안 된다.
    expect(onSwitch).not.toHaveBeenCalled()
  })
})
