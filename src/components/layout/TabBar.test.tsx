// TabBar 단위테스트 — 탭 렌더 + 클릭 액션이 올바른 콜백을 부르는지(ADR-0057, §7-2).

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

const invokeMock = vi.hoisted(() => vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined))
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))

import TabBar from './TabBar'
import type { ViewMeta } from '../../api/layoutTypes'
import { DOCK_MIME } from './dock/dockDrag'
import { __resetDragImageForTest, DRAG_IMAGE_HOTSPOT, ensureDragImageBox } from './dock/dragImage'

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
    expect(onSwitch).not.toHaveBeenCalled()
  })

  it('이름 더블클릭 → 인라인 편집 진입(input 등장, 현재 이름 시드)', () => {
    renderBar('v1')
    const input = openEditForV1()
    expect(input).toBeTruthy()
    expect(input.value).toBe('View 1')
    expect(input.getAttribute('data-view-id')).toBe('v1')
  })

  it('Enter(새 이름) → onRename(id, trim된 새이름) — 정확히 1회(Enter+blur 이중호출 방지, FIX 3)', () => {
    const { onRename } = renderBar('v1')
    const input = openEditForV1()
    fireEvent.change(input, { target: { value: '  Renamed  ' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(onRename).toHaveBeenCalledWith('v1', 'Renamed')
    // ★멱등★: Enter 가 setEditingId(null) 로 input 언마운트 → blur 가 commitEdit 을 한 번 더 부르지만
    //   editingId 는 이미 null 이라 no-op. onRename 은 정확히 1회여야 한다(중복 rename_tab invoke 방지).
    fireEvent.blur(input)
    expect(onRename).toHaveBeenCalledTimes(1)
  })

  it('편집 진입 시 select() 가 정확히 1회만 실행(매 키입력 재선택 방지, FIX 1)', () => {
    renderBar('v1')
    const selectSpy = vi.spyOn(HTMLInputElement.prototype, 'select')
    const input = openEditForV1()
    expect(selectSpy).toHaveBeenCalledTimes(1)
    // 키입력(setDraft→re-render)이 select 를 재실행하면 방금 친 글자가 통째로 덮어써진다.
    fireEvent.change(input, { target: { value: 'N' } })
    fireEvent.change(input, { target: { value: 'Ne' } })
    fireEvent.change(input, { target: { value: 'New' } })
    expect(selectSpy).toHaveBeenCalledTimes(1)
    expect(input.value).toBe('New')
    selectSpy.mockRestore()
  })

  it('Esc → 편집 취소(onRename 안 부름, revert)', () => {
    const { onRename } = renderBar('v1')
    const input = openEditForV1()
    fireEvent.change(input, { target: { value: 'Whatever' } })
    fireEvent.keyDown(input, { key: 'Escape' })
    expect(onRename).not.toHaveBeenCalled()
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
    expect(screen.getByTestId('tab-rename-input')).toBeTruthy()
    expect(onSwitch).toHaveBeenCalledTimes(1)
    expect(onSwitch).toHaveBeenCalledWith('v2')
  })
})

// ── 끌기 출처(TRD S21-slot-docking §1-4 「출처」 · §9 「프론트(7 단계)」 TabBar 줄) ──────────────────────────
// jsdom 에는 DragEvent · DataTransfer 가 없어 testing-library 가 끌기 데이터 칸을 그대로 붙인 사건을 낸다. 엔진이 끌기를
//   접는 판정(누른 자리 다시 짚기)과 실제 사건 간격은 이 시험이 못 덮는다 — 시험은 「시한 전 끝 = 접힌 끌기」를 순서로만 흉내 낸다.

interface FakeDt {
  data: Map<string, string>
  effectAllowed: string
  dropEffect: string
  setData: ReturnType<typeof vi.fn>
  getData: (k: string) => string
  clearData: ReturnType<typeof vi.fn>
  setDragImage: ReturnType<typeof vi.fn>
}

function fakeDt(dropEffect = 'none'): FakeDt {
  const data = new Map<string, string>()
  return {
    data,
    effectAllowed: 'uninitialized',
    dropEffect,
    setData: vi.fn((k: string, v: string) => void data.set(k, v)),
    getData: (k: string) => data.get(k) ?? '',
    clearData: vi.fn(() => data.clear()),
    setDragImage: vi.fn(),
  }
}

const head = (id: string) => screen.getAllByTestId('tab').find(t => t.getAttribute('data-view-id') === id)!

const dragIdOf = (dt: FakeDt) => (JSON.parse(dt.data.get(DOCK_MIME)!) as { drag_id: string }).drag_id

function cmdCalls(cmd: string): unknown[] {
  return invokeMock.mock.calls.filter(c => c[0] === cmd).map(c => c[1])
}

function renderDragBar(props: Partial<Parameters<typeof TabBar>[0]> = {}) {
  return render(
    <TabBar
      label="main"
      tabs={TABS}
      active="v1"
      onSwitch={vi.fn()}
      onCreate={vi.fn()}
      onClose={vi.fn()}
      onRename={vi.fn()}
      {...props}
    />,
  )
}

describe('TabBar — 끌기 출처', () => {
  afterEach(() => {
    vi.useRealTimers()
    invokeMock.mockClear()
    __resetDragImageForTest()
  })

  it('머리마다 draggable · 이름 편집 중인 머리만 아니다', () => {
    renderDragBar()
    expect(head('v1').getAttribute('draggable')).toBe('true')
    expect(head('v2').getAttribute('draggable')).toBe('true')
    openEditForV1()
    expect(head('v1').getAttribute('draggable')).toBe('false')
    expect(head('v2').getAttribute('draggable')).toBe('true')
  })

  it('닫기 버튼에서 시작한 끌기 → 취소 · 데이터 없음 · 세션 0 · window 까지 닿음(우리 MIME 이 아니라 끊지 않는다)', () => {
    vi.useFakeTimers()
    renderDragBar()
    const seen = vi.fn()
    window.addEventListener('dragstart', seen)
    const dt = fakeDt()
    const close = screen.getAllByTestId('tab-close')[1]
    const notCancelled = fireEvent.dragStart(close, { dataTransfer: dt })
    window.removeEventListener('dragstart', seen)
    expect(notCancelled).toBe(false)
    expect(dt.setData).not.toHaveBeenCalled()
    expect(dt.setDragImage).not.toHaveBeenCalled()
    expect(seen).toHaveBeenCalledTimes(1)
    vi.runAllTimers()
    expect(cmdCalls('dock_drag_start')).toEqual([])
  })

  it('닫기 버튼에서 누른 끌기는 사건 대상이 머리여도(엔진은 draggable 조상에 낸다) 막는다 · 다음 누름은 다시 끈다', () => {
    vi.useFakeTimers()
    renderDragBar()
    fireEvent.pointerDown(screen.getAllByTestId('tab-close')[0])
    const blocked = fakeDt()
    expect(fireEvent.dragStart(head('v1'), { dataTransfer: blocked })).toBe(false)
    expect(blocked.setData).not.toHaveBeenCalled()
    vi.runAllTimers()
    expect(cmdCalls('dock_drag_start')).toEqual([])

    fireEvent.pointerDown(screen.getAllByTestId('tab-name')[0])
    const ok = fakeDt()
    expect(fireEvent.dragStart(head('v1'), { dataTransfer: ok })).toBe(true)
    vi.runAllTimers()
    expect(cmdCalls('dock_drag_start')).toHaveLength(1)
  })

  it('이름 입력에서 시작한 끌기(글자 끌기) → 취소 · 데이터 없음 · 세션 0', () => {
    vi.useFakeTimers()
    renderDragBar()
    const input = openEditForV1()
    const dt = fakeDt()
    expect(fireEvent.dragStart(input, { dataTransfer: dt })).toBe(false)
    expect(dt.setData).not.toHaveBeenCalled()
    vi.runAllTimers()
    expect(cmdCalls('dock_drag_start')).toEqual([])
  })

  it('dragstart = 우리 MIME 하나만 · effectAllowed move · setDragImage · window 에 안 닿음 · 탭바 DOM 무변경', () => {
    vi.useFakeTimers()
    const { container } = renderDragBar()
    const box = ensureDragImageBox()
    const bar = screen.getByTestId('tab-bar')
    const mo = new MutationObserver(() => undefined)
    mo.observe(bar, { subtree: true, childList: true, attributes: true, characterData: true })
    const seen = vi.fn()
    window.addEventListener('dragstart', seen)
    const dt = fakeDt()

    const notCancelled = fireEvent.dragStart(head('v2'), { dataTransfer: dt })
    const mutations = mo.takeRecords()
    mo.disconnect()
    window.removeEventListener('dragstart', seen)

    expect(notCancelled).toBe(true)
    expect(dt.clearData).toHaveBeenCalled()
    expect(dt.setData).toHaveBeenCalledTimes(1)
    expect([...dt.data.keys()]).toEqual([DOCK_MIME])
    const payload = JSON.parse(dt.data.get(DOCK_MIME)!) as { drag_id: string; view_id: string }
    expect(payload.view_id).toBe('v2')
    expect(typeof payload.drag_id).toBe('string')
    expect(dt.effectAllowed).toBe('move')
    expect(dt.setDragImage).toHaveBeenCalledWith(box, DRAG_IMAGE_HOTSPOT.x, DRAG_IMAGE_HOTSPOT.y)
    expect(seen).not.toHaveBeenCalled()
    // ★dragstart 안에서 끌린 머리가 누른 자리 밑에서 빠지면 엔진이 끌기를 접는다★ — 처리기가 다시 그리기를 안 부르는
    //   것으로 그 길을 막는다. fireEvent 는 act 안에서 돌아 그 처리기가 낸 상태 갱신까지 그린 뒤 돌아오므로, 여기 기록이
    //   없으면 다시 그리기도 없었다.
    expect(mutations).toEqual([])
    expect(container.contains(bar)).toBe(true)
  })

  it('dock_drag_start 는 디스패치 뒤(시한 0) — drag_id · view_id · hotspot = 그림 상자의 커서 자리', () => {
    vi.useFakeTimers()
    renderDragBar()
    const dt = fakeDt()
    fireEvent.dragStart(head('v1'), { dataTransfer: dt })
    expect(cmdCalls('dock_drag_start')).toEqual([])
    vi.advanceTimersByTime(0)
    expect(cmdCalls('dock_drag_start')).toEqual([
      { drag_id: dragIdOf(dt), view_id: 'v1', hotspot: { ...DRAG_IMAGE_HOTSPOT } },
    ])
  })

  it('뒤 리스너가 dragstart 를 취소하면 세션을 열지 않는다(0 회) · 끝 신호도 안 보낸다', () => {
    vi.useFakeTimers()
    const { container } = renderDragBar()
    // React 처리기는 뿌리 컨테이너에서 돈다 — 같은 노드에 나중에 단 리스너는 우리 뒤에 돌아 끊기로 못 막는다.
    const cancel = (e: Event) => e.preventDefault()
    container.addEventListener('dragstart', cancel)
    fireEvent.dragStart(head('v1'), { dataTransfer: fakeDt() })
    container.removeEventListener('dragstart', cancel)
    vi.runAllTimers()
    expect(cmdCalls('dock_drag_start')).toEqual([])
    fireEvent.dragEnd(head('v1'), { dataTransfer: fakeDt('none') })
    expect(cmdCalls('dock_drag_end')).toEqual([])
  })

  it.each([
    ['move', 'move'],
    ['none', 'none'],
    ['copy', 'other'],
    ['link', 'other'],
  ])('dragend(dropEffect=%s) → dock_drag_end{effect: %s}', (dropEffect, effect) => {
    vi.useFakeTimers()
    renderDragBar()
    const dt = fakeDt()
    fireEvent.dragStart(head('v2'), { dataTransfer: dt })
    vi.runAllTimers()
    fireEvent.dragEnd(head('v2'), { dataTransfer: fakeDt(dropEffect) })
    expect(cmdCalls('dock_drag_end')).toEqual([{ drag_id: dragIdOf(dt), effect }])
  })

  it('시작 시한보다 dragend 가 먼저 오면(엔진이 접은 끌기) 시작도 끝도 안 보낸다 — 시한도 나중에 안 돈다', () => {
    vi.useFakeTimers()
    renderDragBar()
    fireEvent.dragStart(head('v1'), { dataTransfer: fakeDt() })
    fireEvent.dragEnd(head('v1'), { dataTransfer: fakeDt('none') })
    vi.runAllTimers()
    expect(cmdCalls('dock_drag_start')).toEqual([])
    expect(cmdCalls('dock_drag_end')).toEqual([])
    // 같은 머리의 다음 끌기는 그대로 선다.
    const dt = fakeDt()
    fireEvent.dragStart(head('v1'), { dataTransfer: dt })
    vi.runAllTimers()
    fireEvent.dragEnd(head('v1'), { dataTransfer: fakeDt('move') })
    expect(cmdCalls('dock_drag_start')).toHaveLength(1)
    expect(cmdCalls('dock_drag_end')).toEqual([{ drag_id: dragIdOf(dt), effect: 'move' }])
  })

  it('시한이 돈 뒤에 온 dragend(OS 끌기) → 시작 다음 끝, 같은 drag_id', () => {
    vi.useFakeTimers()
    renderDragBar()
    const dt = fakeDt()
    fireEvent.dragStart(head('v1'), { dataTransfer: dt })
    vi.advanceTimersByTime(0)
    fireEvent.dragEnd(head('v1'), { dataTransfer: fakeDt('none') })
    expect(invokeMock.mock.calls.map(c => c[0])).toEqual(['dock_drag_start', 'dock_drag_end'])
    expect(invokeMock.mock.calls.map(c => (c[1] as { drag_id: string }).drag_id)).toEqual([
      dragIdOf(dt),
      dragIdOf(dt),
    ])
    expect(cmdCalls('dock_drag_end')).toEqual([{ drag_id: dragIdOf(dt), effect: 'none' }])
  })

  it('잇단 끌기는 각자의 drag_id 로 끝난다 · 같은 끌기의 끝은 한 번만', () => {
    vi.useFakeTimers()
    renderDragBar()
    const dt1 = fakeDt()
    fireEvent.dragStart(head('v1'), { dataTransfer: dt1 })
    vi.runAllTimers()
    fireEvent.dragEnd(head('v1'), { dataTransfer: fakeDt('move') })
    const dt2 = fakeDt()
    fireEvent.dragStart(head('v2'), { dataTransfer: dt2 })
    vi.runAllTimers()
    fireEvent.dragEnd(head('v2'), { dataTransfer: fakeDt('none') })
    fireEvent.dragEnd(head('v2'), { dataTransfer: fakeDt('none') })
    const id1 = dragIdOf(dt1)
    const id2 = dragIdOf(dt2)
    expect(id1).not.toBe(id2)
    expect(cmdCalls('dock_drag_start').map(a => (a as { drag_id: string }).drag_id)).toEqual([id1, id2])
    expect(cmdCalls('dock_drag_end')).toEqual([
      { drag_id: id1, effect: 'move' },
      { drag_id: id2, effect: 'none' },
    ])
  })

  it('끌기 그림 재료 — 그 탭의 캔버스(dragCanvas)를 묻고 탭 제목을 쓴다', () => {
    const dragCanvas = vi.fn((_viewId: string): HTMLElement | null => null)
    renderDragBar({ dragCanvas })
    fireEvent.dragStart(head('v2'), { dataTransfer: fakeDt() })
    expect(dragCanvas).toHaveBeenCalledWith('v2')
    expect(ensureDragImageBox().textContent).toBe('View 2')
  })

  it('떼어진 표시 — 그 머리는 DOM 에 남은 채 접히고(폭 0 · 안 보임) 되살리면 같은 요소다', () => {
    const { rerender } = renderDragBar({ detached: 'v1' })
    const v1 = head('v1')
    expect(v1.getAttribute('data-dock-detached')).toBe('true')
    expect(v1.style.width).toBe('0px')
    expect(v1.style.visibility).toBe('hidden')
    expect(head('v2').getAttribute('data-dock-detached')).toBeNull()
    expect(head('v2').style.visibility).toBe('')
    rerender(
      <TabBar
        label="main"
        tabs={TABS}
        active="v1"
        onSwitch={vi.fn()}
        onCreate={vi.fn()}
        onClose={vi.fn()}
        onRename={vi.fn()}
        detached={null}
      />,
    )
    // React key 그대로라 같은 요소다 — `dragend` 를 받는 그 요소.
    expect(head('v1')).toBe(v1)
    expect(v1.getAttribute('data-dock-detached')).toBeNull()
    expect(v1.style.width).toBe('')
  })
})
