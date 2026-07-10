// ADR-0064/0055: SlotContextMenu 클릭 실행 경로 테스트(FIX-3).
//
// 검증 불변식:
//   1. 항목 클릭 = 공유 dispatch(fireAndForget)로 command 를 *id* 로 실행한다(bespoke run(ctx) 재구현 금지).
//      → 팔레트/키바인딩/LLM 소비자와 동일 helper 재사용(안전망 일원화).
//   2. fireAndForget 인자 = (item.id, { viewId, slotId, agentId })(ctx 가방).
//   3. 클릭 후 onClose 가 불린다(메뉴는 항상 닫힘).
//
// ★reject 누수 안전은 여기서 재테스트하지 않는다★: 메뉴가 bespoke .catch 를 버리고 fireAndForget 만 부르므로
//   "sync throw·async reject·thenable 을 삼켜 누수 없음" 계약은 dispatch.test.ts 가 소유한다(구조적 상속).
//   여기 관심사는 "메뉴가 그 공유 helper 로 라우팅하는가"뿐.
//
// 전략: dispatch 를 mock 해 fireAndForget 호출(id + ctx)을 관측한다.

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { ResolvedSlotMenuItem } from '../../commands/slotMenu'

const dispatchMock = vi.hoisted(() => ({ fireAndForget: vi.fn() }))
vi.mock('../../commands/dispatch', () => ({
  fireAndForget: (...args: unknown[]) => dispatchMock.fireAndForget(...args),
}))

import SlotContextMenu, { clampMenuPosition, flyoutPosition } from './SlotContextMenu'

function item(id: string, over: Partial<ResolvedSlotMenuItem> = {}): ResolvedSlotMenuItem {
  return { id, title: id, run: vi.fn(), group: 'slot-ops', separatorBefore: false, ...over }
}

beforeEach(() => {
  dispatchMock.fireAndForget.mockClear()
})
afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

describe('SlotContextMenu — 공유 dispatch 경로(FIX-3)', () => {
  it('항목 클릭 → fireAndForget(item.id, ctx) 로 라우팅(run 직접 호출 아님)', () => {
    const onClose = vi.fn()
    const it0 = item('slot.close')
    render(
      <SlotContextMenu
        x={0}
        y={0}
        items={[it0]}
        ctx={{ viewId: 'v1', slotId: 's1', agentId: 'a1' }}
        onClose={onClose}
      />,
    )
    fireEvent.click(screen.getByText('slot.close'))
    expect(dispatchMock.fireAndForget).toHaveBeenCalledWith('slot.close', {
      viewId: 'v1',
      slotId: 's1',
      agentId: 'a1',
    })
    // resolve 된 run 은 메뉴가 직접 부르지 않는다(공유 경로로만).
    expect(it0.run).not.toHaveBeenCalled()
    expect(onClose).toHaveBeenCalled()
  })

  it('컨테이너(children) 항목: hover 로 flyout 이 열리고 자식 클릭 → fireAndForget(child.id, ctx)', () => {
    const onClose = vi.fn()
    const container = item('container:새 콘텐츠', {
      title: '새 콘텐츠',
      children: [item('slot.fill.agentList', { title: '트리' })],
    })
    render(
      <SlotContextMenu
        x={0}
        y={0}
        items={[container]}
        ctx={{ viewId: 'v1', slotId: 's1', agentId: null }}
        onClose={onClose}
      />,
    )
    // 컨테이너 title 은 항상 보인다. hover 전엔 자식 flyout 미마운트.
    expect(screen.getByText('새 콘텐츠')).toBeTruthy()
    expect(screen.queryByText('트리')).toBeNull()
    // hover(mouseEnter) → flyout 열림.
    fireEvent.mouseEnter(screen.getByText('새 콘텐츠'))
    const child = screen.getByText('트리')
    expect(child).toBeTruthy()
    fireEvent.click(child)
    expect(dispatchMock.fireAndForget).toHaveBeenCalledWith('slot.fill.agentList', {
      viewId: 'v1',
      slotId: 's1',
      agentId: null,
    })
    expect(onClose).toHaveBeenCalled()
  })

  it('agentId 미배정(null)도 ctx 그대로 전달', () => {
    render(
      <SlotContextMenu
        x={0}
        y={0}
        items={[item('slot.split')]}
        ctx={{ viewId: 'v1', slotId: 's1', agentId: null }}
        onClose={vi.fn()}
      />,
    )
    fireEvent.click(screen.getByText('slot.split'))
    expect(dispatchMock.fireAndForget).toHaveBeenCalledWith('slot.split', {
      viewId: 'v1',
      slotId: 's1',
      agentId: null,
    })
  })
})

// ── ★뷰포트 clamp(Bug1)★: 순수 helper clampMenuPosition 을 폭넓게 단위테스트한다. jsdom 은
//    getBoundingClientRect 가 0을 돌려 컴포넌트 경로로는 넘침을 못 만들므로, 실제 clamp 로직은 이 helper
//    테스트가 소유한다(컴포넌트는 이 helper 를 호출만 — GUI 실측은 별도). ──
describe('clampMenuPosition — 뷰포트 안으로 clamp', () => {
  const VW = 1000
  const VH = 800
  const W = 150
  const H = 300

  it('넘치지 않으면 커서 좌표 그대로', () => {
    expect(clampMenuPosition(100, 100, W, H, VW, VH)).toEqual({ top: 100, left: 100 })
  })

  it('하단 넘침 → top 을 vh-h-margin 으로 밀어올린다(전체 보이게)', () => {
    // y=700, h=300 → 700+300=1000 > 800 넘침 → top = min(700, 800-300-4)=496
    const { top } = clampMenuPosition(100, 700, W, H, VW, VH)
    expect(top).toBe(VH - H - 4)
    expect(top + H).toBeLessThanOrEqual(VH) // 하단이 뷰포트 안
  })

  it('우측 넘침 → left 를 vw-w-margin 으로 밀어들인다', () => {
    // x=950, w=150 → 950+150=1100 > 1000 넘침 → left = min(950, 1000-150-4)=846
    const { left } = clampMenuPosition(950, 100, W, H, VW, VH)
    expect(left).toBe(VW - W - 4)
    expect(left + W).toBeLessThanOrEqual(VW)
  })

  it('우하단 코너 넘침 → top·left 둘 다 clamp', () => {
    const { top, left } = clampMenuPosition(950, 700, W, H, VW, VH)
    expect(top).toBe(VH - H - 4)
    expect(left).toBe(VW - W - 4)
  })

  it('메뉴가 뷰포트보다 큼(h>vh) → top 은 최소 margin 으로 상단 고정(음수 방지)', () => {
    // h=900 > vh=800 → vh-h-4 = -104 인데 max(4, ...) 로 4 고정
    const { top } = clampMenuPosition(100, 700, W, 900, VW, VH)
    expect(top).toBe(4)
  })

  it('가장자리 정확히 맞음(y+h===vh)은 넘침 아님 → 그대로', () => {
    expect(clampMenuPosition(0, VH - H, W, H, VW, VH).top).toBe(VH - H)
  })
})

// ── ★서브메뉴 flyout 배치(ADR-0065)★: 순수 helper flyoutPosition — 기본 우측 전개, 우측 오버플로 시 좌측 뒤집기. ──
describe('flyoutPosition — 서브메뉴 우측/좌측 전개', () => {
  const VW = 1000
  const VH = 800
  const FW = 150 // flyout 폭
  const FH = 200 // flyout 높이

  it('우측 공간 충분 → 부모 오른쪽 가장자리(anchorRight)에서 오른쪽으로 편다', () => {
    // 부모 rect: left=100, right=250, top=100. right(250)+FW(150)=400 <= 1000 → 오른쪽.
    expect(flyoutPosition(100, 250, 100, FW, FH, VW, VH)).toEqual({ top: 100, left: 250 })
  })

  it('우측 오버플로 + 좌측 공간 有 → 왼쪽(anchorLeft - FW)으로 뒤집는다', () => {
    // 부모 rect: left=900, right=980. right(980)+150=1130 > 1000 넘침, left(900)-150=750 >= margin → 왼쪽.
    const { left } = flyoutPosition(900, 980, 100, FW, FH, VW, VH)
    expect(left).toBe(900 - FW) // 750
    expect(left + FW).toBeLessThanOrEqual(VW)
  })

  it('하단 오버플로 → top 을 밀어올려 flyout 전체가 뷰포트 안', () => {
    // top=700, FH=200 → 700+200=900 > 800 넘침 → top = min(700, 800-200-4)=596.
    const { top } = flyoutPosition(100, 250, 700, FW, FH, VW, VH)
    expect(top).toBe(VH - FH - 4)
    expect(top + FH).toBeLessThanOrEqual(VH)
  })

  it('flyout 이 뷰포트보다 큼(FH>VH) → top 은 최소 margin 으로 상단 고정(음수 방지)', () => {
    const { top } = flyoutPosition(100, 250, 700, FW, 900, VW, VH)
    expect(top).toBe(4)
  })
})

// ── 컴포넌트 경로: useLayoutEffect 가 getBoundingClientRect 를 재 helper 로 위치를 계산한다.
//    getBoundingClientRect 를 모킹해 하단 넘침을 만들고, 렌더된 메뉴의 top 이 clamp 됐는지 단언한다. ──
describe('SlotContextMenu — 마운트 후 뷰포트 clamp 적용(Bug1)', () => {
  const origRect = HTMLElement.prototype.getBoundingClientRect
  afterEach(() => {
    HTMLElement.prototype.getBoundingClientRect = origRect
  })

  it('하단 근처 우클릭 → 메뉴 top 이 뷰포트 안으로 clamp 된다', () => {
    // 메뉴 크기 150×300 을 강제. window.innerHeight 는 jsdom 기본(768)로 두고 y=700 → 넘침.
    HTMLElement.prototype.getBoundingClientRect = function () {
      return { width: 150, height: 300, top: 0, left: 0, right: 150, bottom: 300, x: 0, y: 0, toJSON() {} } as DOMRect
    }
    const vh = window.innerHeight
    render(
      <SlotContextMenu
        x={10}
        y={vh - 50}
        items={[item('slot.close')]}
        ctx={{ viewId: 'v1', slotId: 's1', agentId: null }}
        onClose={vi.fn()}
      />,
    )
    // 렌더된 컨테이너(position:fixed)를 찾아 top 이 vh-300-4 로 clamp 됐는지 확인.
    const fixed = document.querySelector('div[style*="fixed"]') as HTMLElement
    expect(fixed).toBeTruthy()
    const topPx = parseInt(fixed.style.top, 10)
    expect(topPx).toBe(vh - 300 - 4)
    expect(topPx + 300).toBeLessThanOrEqual(vh)
  })
})
