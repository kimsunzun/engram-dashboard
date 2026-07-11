// TabBar 단위테스트 — 탭 렌더 + 클릭 액션이 올바른 콜백을 부르는지(ADR-0057, §7-2).
//
// ★검증 불변식★:
//   1. tabs 를 렌더하고 active 탭을 강조(data-active)한다.
//   2. 탭 클릭 → onSwitch(그 view id).
//   3. [+] 클릭 → onCreate.
//   4. 탭 × 클릭 → onClose(그 view id), 부모 onSwitch 로 버블 안 함(stopPropagation).
//   5. 이름 더블클릭 → 인라인 편집 진입(input). Enter(새 이름) → onRename(id, 새이름). Esc/공백/미변경 → onRename X.

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
  const onRename = vi.fn()
  render(
    <TabBar
      label="main"
      tabs={TABS}
      active={active}
      onSwitch={onSwitch}
      onCreate={onCreate}
      onClose={onClose}
      onRename={onRename}
    />,
  )
  return { onSwitch, onCreate, onClose, onRename }
}

/** v1 탭의 이름 span 을 더블클릭해 편집 input 을 연다. input 요소 반환. */
function openEditForV1() {
  const nameSpan = screen
    .getAllByTestId('tab-name')
    .find(s => s.closest('[data-testid="tab"]')?.getAttribute('data-view-id') === 'v1')!
  fireEvent.doubleClick(nameSpan)
  return screen.getByTestId('tab-rename-input') as HTMLInputElement
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

  it('이름 더블클릭 → 인라인 편집 진입(input 등장, 현재 이름 시드)', () => {
    renderBar('v1')
    const input = openEditForV1()
    expect(input).toBeTruthy()
    expect(input.value).toBe('View 1') // 현재 이름으로 시드.
    expect(input.getAttribute('data-view-id')).toBe('v1')
  })

  it('Enter(새 이름) → onRename(id, trim된 새이름) — 정확히 1회(Enter+blur 이중호출 방지, FIX 3)', () => {
    const { onRename } = renderBar('v1')
    const input = openEditForV1()
    fireEvent.change(input, { target: { value: '  Renamed  ' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(onRename).toHaveBeenCalledWith('v1', 'Renamed') // trim 적용.
    // ★멱등★: Enter 가 setEditingId(null) 로 input 언마운트 → blur 가 commitEdit 을 한 번 더 부르지만
    //   editingId 는 이미 null 이라 no-op. onRename 은 정확히 1회여야 한다(중복 rename_tab invoke 방지).
    fireEvent.blur(input)
    expect(onRename).toHaveBeenCalledTimes(1)
  })

  it('편집 진입 시 select() 가 정확히 1회만 실행(매 키입력 재선택 방지, FIX 1)', () => {
    renderBar('v1')
    // input 프로토타입 select 를 스파이 — 편집 진입 시 useEffect([editingId])가 1회 호출.
    const selectSpy = vi.spyOn(HTMLInputElement.prototype, 'select')
    const input = openEditForV1()
    expect(selectSpy).toHaveBeenCalledTimes(1)
    // 키입력(setDraft→re-render)이 select 를 재실행하면 방금 친 글자가 통째로 덮어써진다 — 재실행 없어야 한다.
    fireEvent.change(input, { target: { value: 'N' } })
    fireEvent.change(input, { target: { value: 'Ne' } })
    fireEvent.change(input, { target: { value: 'New' } })
    expect(selectSpy).toHaveBeenCalledTimes(1) // 진입 시 1회 그대로.
    expect(input.value).toBe('New')
    selectSpy.mockRestore()
  })

  it('Esc → 편집 취소(onRename 안 부름, revert)', () => {
    const { onRename } = renderBar('v1')
    const input = openEditForV1()
    fireEvent.change(input, { target: { value: 'Whatever' } })
    fireEvent.keyDown(input, { key: 'Escape' })
    expect(onRename).not.toHaveBeenCalled()
    // 편집 종료 → input 사라짐.
    expect(screen.queryByTestId('tab-rename-input')).toBeNull()
  })

  it('빈/공백 이름 확정(Enter) → onRename 안 부름(revert)', () => {
    const { onRename } = renderBar('v1')
    const input = openEditForV1()
    fireEvent.change(input, { target: { value: '   ' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(onRename).not.toHaveBeenCalled()
  })

  it('이름 미변경 확정(Enter) → onRename 안 부름(no-op)', () => {
    const { onRename } = renderBar('v1')
    const input = openEditForV1()
    // 값 그대로 Enter — trim 후 원래 이름과 같으면 스킵.
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(onRename).not.toHaveBeenCalled()
  })

  it('편집 중 input 키 입력이 부모 탭 onSwitch 로 버블 안 함(stopPropagation)', () => {
    const { onSwitch } = renderBar('v1')
    const input = openEditForV1()
    fireEvent.keyDown(input, { key: 'a' })
    expect(onSwitch).not.toHaveBeenCalled()
  })

  it('이름 더블클릭 → 편집 진입 + 더블클릭 완성 클릭이 spurious switch 유발 안 함(FIX 2)', () => {
    const { onSwitch } = renderBar('v1') // 비활성 탭 v2 를 더블클릭.
    const nameSpan = screen
      .getAllByTestId('tab-name')
      .find(s => s.closest('[data-testid="tab"]')?.getAttribute('data-view-id') === 'v2')!
    // 실제 더블클릭 제스처: dblclick 전에 click 이 2번(detail 1, 2) 쏜 뒤 dblclick(detail 2).
    // span onClick 은 detail>=2 만 stopPropagation → 두 번째(완성) 클릭은 부모 onSwitch 로 안 샌다.
    fireEvent.click(nameSpan, { detail: 1 })
    fireEvent.click(nameSpan, { detail: 2 })
    fireEvent.doubleClick(nameSpan, { detail: 2 })
    // 편집 진입.
    expect(screen.getByTestId('tab-rename-input')).toBeTruthy()
    // 첫 단일 클릭(detail 1)은 정상 전환 — 그 1회만 허용, 완성 클릭(detail 2)은 추가 전환 없어야 한다.
    expect(onSwitch).toHaveBeenCalledTimes(1)
    expect(onSwitch).toHaveBeenCalledWith('v2')
  })
})
