// ★reject 누수 안전은 여기서 재테스트하지 않는다★: 메뉴가 bespoke .catch 를 버리고 fireAndForget 만 부르므로
//   "sync throw·async reject·thenable 을 삼켜 누수 없음" 계약은 dispatch.test.ts 가 소유한다(구조적 상속).
//   여기 관심사는 "메뉴가 그 공유 helper 로 라우팅하는가"뿐.

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { ResolvedSlotMenuItem } from '../../commands/slotMenu'

const dispatchMock = vi.hoisted(() => ({ fireAndForget: vi.fn() }))
vi.mock('../../commands/dispatch', () => ({
  fireAndForget: (...args: unknown[]) => dispatchMock.fireAndForget(...args),
}))

import SlotContextMenu, { ANCHOR_GAP, clampMenuPosition, flyoutPosition } from './SlotContextMenu'

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
    expect(screen.getByText('새 콘텐츠')).toBeTruthy()
    expect(screen.queryByText('트리')).toBeNull()
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
describe('clampMenuPosition — 넘치면 뒤집는다(앵커는 메뉴 밖 간격만큼 떨어져 유지)', () => {
  const VW = 1000
  const VH = 800
  const W = 150
  const H = 300

  /**
   * ★요구치는 구현 상수(ANCHOR_GAP)와 따로 적는다★: `ANCHOR_GAP` 을 그대로 단언에 쓰면 그 상수를 0 으로
   * 바꿔도 단언이 함께 0 이 되어 무력화된다. 여기 값은 OS 에서 유도한 *하한*이다 — Windows 기본 더블클릭
   * 허용 사각형이 4px(=반경 2px)이므로 그 2배를 요구한다. 구현이 8 에서 6 으로 줄어도 통과하지만, 반경
   * 이하로 줄면 실패한다.
   */
  const REQUIRED_GAP = 4

  /**
   * 앵커에서 메뉴 사각형까지의 축별 거리 중 *큰* 값. 둘째 클릭이 메뉴에 떨어지려면 두 축 모두 사각형 안이어야
   * 하므로, 한 축이라도 이만큼 떨어져 있으면 그 허용 반경 안에서는 메뉴에 닿지 않는다. 0 이면 앵커가 사각형
   * 안(= 커서 밑에 항목)이다.
   */
  function anchorClearance(x: number, y: number, pos: { top: number; left: number }, w: number, h: number) {
    const dx = Math.max(pos.left - x, x - (pos.left + w), 0)
    const dy = Math.max(pos.top - y, y - (pos.top + h), 0)
    return Math.max(dx, dy)
  }

  it('넘치지 않으면 앵커에서 간격만큼 띄운 자리(앵커 아래·오른쪽)', () => {
    expect(clampMenuPosition(100, 100, W, H, VW, VH)).toEqual({
      top: 100 + ANCHOR_GAP,
      left: 100 + ANCHOR_GAP,
    })
  })

  it('하단 넘침 → 위로 뒤집고 그 방향에도 간격을 둔다(메뉴 하단이 앵커보다 위)', () => {
    const pos = clampMenuPosition(100, 700, W, H, VW, VH)
    expect(pos.top).toBe(700 - ANCHOR_GAP - H)
    expect(pos.top + H).toBeLessThanOrEqual(VH)
    expect(anchorClearance(100, 700, pos, W, H)).toBeGreaterThanOrEqual(REQUIRED_GAP)
  })

  it('우측 넘침 → 왼쪽으로 뒤집고 그 방향에도 간격을 둔다', () => {
    const pos = clampMenuPosition(950, 100, W, H, VW, VH)
    expect(pos.left).toBe(950 - ANCHOR_GAP - W)
    expect(pos.left + W).toBeLessThanOrEqual(VW)
    expect(anchorClearance(950, 100, pos, W, H)).toBeGreaterThanOrEqual(REQUIRED_GAP)
  })

  it('우하단 코너 넘침 → 두 축 다 뒤집고 두 축 다 간격을 둔다', () => {
    const pos = clampMenuPosition(950, 700, W, H, VW, VH)
    expect(pos).toEqual({ top: 700 - ANCHOR_GAP - H, left: 950 - ANCHOR_GAP - W })
    expect(anchorClearance(950, 700, pos, W, H)).toBeGreaterThanOrEqual(REQUIRED_GAP)
  })

  it('메뉴가 뷰포트보다 큼(h>vh) → 뒤집어도 안 들어가므로 밀기 폴백(상단 고정, 음수 방지)', () => {
    // 퇴화 케이스에선 "화면 안에 있다"가 앵커 규칙(간격 포함)을 이긴다 — 앵커가 안으로 들어오는 것을 허용한다.
    const { top } = clampMenuPosition(100, 700, W, 900, VW, VH)
    expect(top).toBe(4)
  })

  it('간격까지 정확히 맞음(y+GAP+h===vh)은 넘침 아님 → 뒤집지 않는다', () => {
    // 넘침 판정 경계가 간격을 포함하는지 고정한다(간격을 빼고 판정하면 메뉴가 1..GAP px 화면 밖으로 나간다).
    const y = VH - H - ANCHOR_GAP
    expect(clampMenuPosition(0, y, W, H, VW, VH).top).toBe(VH - H)
  })

  // ★속성 단언★: 특정 수치가 아니라 "앵커는 메뉴에서 요구 간격 이상 떨어져 있다 + 메뉴는 화면 안에 있다"를
  //   고정한다 — 배치 수식을 다시 손대도 이 성질이 깨지면 여기서 잡힌다(메뉴 ≤ 뷰포트인 정상 범위 전체).
  //   ★"안에 들어가지만 않으면 된다"에서 강화됨(ADR-0143 결정 4)★: 모서리에 딱 붙는 배치는 옛 단언을
  //   통과했지만 더블클릭 둘째 클릭이 항목을 실행했다.
  it('뷰포트 안 어느 앵커에서도 앵커는 메뉴에서 요구 간격 이상 떨어지고 메뉴는 화면 안에 있다', () => {
    for (const x of [0, 1, 100, 500, 841, 842, 843, 950, 999, VW]) {
      for (const y of [0, 1, 100, 400, 491, 492, 493, 700, 799, VH]) {
        const pos = clampMenuPosition(x, y, W, H, VW, VH)
        expect(
          anchorClearance(x, y, pos, W, H),
          `앵커(${x},${y})가 메뉴에 너무 가깝다`,
        ).toBeGreaterThanOrEqual(REQUIRED_GAP)
        expect(pos.left >= 0 && pos.left + W <= VW, `left 화면 밖(${x},${y})`).toBe(true)
        expect(pos.top >= 0 && pos.top + H <= VH, `top 화면 밖(${x},${y})`).toBe(true)
      }
    }
  })
})

// ── ★서브메뉴 flyout 배치(ADR-0065)★ ──
describe('flyoutPosition — 서브메뉴 우측/좌측 전개', () => {
  const VW = 1000
  const VH = 800
  const FW = 150 // flyout 폭
  const FH = 200 // flyout 높이

  it('우측 공간 충분 → 부모 오른쪽 가장자리(anchorRight)에서 오른쪽으로 편다', () => {
    expect(flyoutPosition(100, 250, 100, FW, FH, VW, VH)).toEqual({ top: 100, left: 250 })
  })

  it('우측 오버플로 + 좌측 공간 有 → 왼쪽(anchorLeft - FW)으로 뒤집는다', () => {
    const { left } = flyoutPosition(900, 980, 100, FW, FH, VW, VH)
    expect(left).toBe(900 - FW)
    expect(left + FW).toBeLessThanOrEqual(VW)
  })

  it('하단 오버플로 → top 을 밀어올려 flyout 전체가 뷰포트 안', () => {
    const { top } = flyoutPosition(100, 250, 700, FW, FH, VW, VH)
    expect(top).toBe(VH - FH - 4)
    expect(top + FH).toBeLessThanOrEqual(VH)
  })

  it('flyout 이 뷰포트보다 큼(FH>VH) → top 은 최소 margin 으로 상단 고정(음수 방지)', () => {
    const { top } = flyoutPosition(100, 250, 700, FW, 900, VW, VH)
    expect(top).toBe(4)
  })
})

// ── 컴포넌트 경로 ──
describe('SlotContextMenu — 마운트 후 뷰포트 배치 적용(Bug1)', () => {
  const origRect = HTMLElement.prototype.getBoundingClientRect
  afterEach(() => {
    HTMLElement.prototype.getBoundingClientRect = origRect
  })

  it('하단 근처에서 열면 메뉴가 위로 뒤집히고 앵커는 메뉴 아래 간격 밖에 남는다', () => {
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
    const fixed = document.querySelector('div[style*="fixed"]') as HTMLElement
    expect(fixed).toBeTruthy()
    const topPx = parseInt(fixed.style.top, 10)
    expect(topPx).toBe(vh - 50 - ANCHOR_GAP - 300)
    expect(topPx + 300).toBeLessThanOrEqual(vh)
    // 메뉴 하단이 앵커(y)보다 ANCHOR_GAP 위 = 커서 밑에 항목이 없고, 더블클릭 둘째 클릭도 못 닿는다.
    expect(topPx + 300).toBe(vh - 50 - ANCHOR_GAP)
  })
})
