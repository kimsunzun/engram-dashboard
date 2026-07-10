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

import SlotContextMenu from './SlotContextMenu'

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
