//
// 전략: viewStore.getState() 의 액션들을 hoisted spy 로 주입(command 는 getState().split(...) 로 부른다).
//   agentClient·plugin-dialog 도 stub. registry/slotMenu 는 실제 모듈(등록 부수효과를 관측).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const vs = vi.hoisted(() => ({
  split: vi.fn(async () => 'new'),
  closeSlot: vi.fn(async () => undefined),
  assignAgent: vi.fn(async () => undefined),
  setSlotContent: vi.fn(async () => undefined),
  moveSlotToWindow: vi.fn(async () => ({ window: 'w', tab: 't' })),
}))
vi.mock('../store/viewStore', () => ({
  useViewStore: { getState: () => vs },
}))

const clientMock = vi.hoisted(() => ({
  spawnAgent: vi.fn(async () => ({ id: 'spawned' })),
  killAgent: vi.fn(async () => undefined),
}))
vi.mock('../api/clientFactory', () => ({
  agentClient: {
    spawnAgent: (...a: unknown[]) => clientMock.spawnAgent(...(a as [])),
    killAgent: (...a: unknown[]) => clientMock.killAgent(...(a as [])),
  },
  getAgentClient: vi.fn(),
}))

const dialogMock = vi.hoisted(() => ({ open: vi.fn(async () => null as string | null) }))
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: (...a: unknown[]) => dialogMock.open(...(a as [])),
}))

import './slotCommands' // side-effect register
import './slotContentCommands' // side-effect register
import { run } from './registry'
import { buildSlotMenu } from './slotMenu'

const CTX = { viewId: 'v1', slotId: 's1' }

beforeEach(() => {
  vs.split.mockClear(); vs.closeSlot.mockClear(); vs.assignAgent.mockClear()
  vs.setSlotContent.mockClear(); vs.moveSlotToWindow.mockClear()
  clientMock.spawnAgent.mockClear(); clientMock.killAgent.mockClear()
  dialogMock.open.mockClear(); dialogMock.open.mockResolvedValue(null)
})
afterEach(() => vi.restoreAllMocks())

describe('공통 슬롯 ops(slotCommands) 라우팅', () => {
  // ★라벨↔command↔방향 결선(ADR-0140)★: `가로 분할`(slot.split.topBottom) → 위/아래. 라벨 렌더 층은
  //   ViewLayoutRenderer.test 가 잡고, 여기선 command id → 방향값을 고정한다.
  it('slot.split.topBottom → split(viewId, slotId, top_bottom)', () => {
    run('slot.split.topBottom', CTX)
    expect(vs.split).toHaveBeenCalledWith('v1', 's1', 'top_bottom')
  })
  it('slot.split.leftRight → split(viewId, slotId, left_right)', () => {
    run('slot.split.leftRight', CTX)
    expect(vs.split).toHaveBeenCalledWith('v1', 's1', 'left_right')
  })
  it('slot.popout → moveSlotToWindow(viewId, slotId)', () => {
    run('slot.popout', CTX)
    expect(vs.moveSlotToWindow).toHaveBeenCalledWith('v1', 's1')
  })
  it('slot.empty → setSlotContent(…,{type:empty})', () => {
    run('slot.empty', CTX)
    expect(vs.setSlotContent).toHaveBeenCalledWith('v1', 's1', { type: 'empty' })
  })
  it('slot.close → closeSlot(viewId, slotId)', () => {
    run('slot.close', CTX)
    expect(vs.closeSlot).toHaveBeenCalledWith('v1', 's1')
  })
  it('viewId/slotId 누락 → throw(side-effect 전 loud fail)', () => {
    expect(() => run('slot.split.topBottom', { slotId: 's1' })).toThrow(/viewId/)
    expect(() => run('slot.close', { viewId: 'v1' })).toThrow(/slotId/)
    expect(vs.split).not.toHaveBeenCalled()
    expect(vs.closeSlot).not.toHaveBeenCalled()
  })
})

describe("'*' 공통 기여(모든 슬롯)", () => {
  it('공통 5항목이 slot-ops 그룹으로 어느 콘텐츠에도 붙는다', () => {
    const ids = buildSlotMenu('agent').map(i => i.id)
    for (const id of ['slot.split.topBottom', 'slot.split.leftRight', 'slot.popout', 'slot.empty', 'slot.close']) {
      expect(ids).toContain(id)
    }
  })
})

describe('코어 콘텐츠(slotContentCommands) 라우팅', () => {
  it('slot.fill.agentList → setSlotContent(…,{type:agent_list})', () => {
    run('slot.fill.agentList', CTX)
    expect(vs.setSlotContent).toHaveBeenCalledWith('v1', 's1', { type: 'agent_list' })
  })
  it('slot.fill.presetPalette → setSlotContent(…,{type:preset_palette})', () => {
    run('slot.fill.presetPalette', CTX)
    expect(vs.setSlotContent).toHaveBeenCalledWith('v1', 's1', { type: 'preset_palette' })
  })

  it('slot.createAgentHere: 다이얼로그 고른 cwd → spawnAgent → assignAgent(viewId, slotId, id)', async () => {
    dialogMock.open.mockResolvedValue('C:/picked')
    clientMock.spawnAgent.mockResolvedValueOnce({ id: 'brand-new' })
    await run('slot.createAgentHere', CTX)
    expect(dialogMock.open).toHaveBeenCalledWith(expect.objectContaining({ directory: true, multiple: false }))
    expect(clientMock.spawnAgent).toHaveBeenCalledWith('C:/picked')
    expect(vs.assignAgent).toHaveBeenCalledWith('v1', 's1', 'brand-new')
  })
  it('slot.createAgentHere: 다이얼로그 취소(null) → spawn/assign 없음(no-op)', async () => {
    dialogMock.open.mockResolvedValue(null)
    await run('slot.createAgentHere', CTX)
    expect(clientMock.spawnAgent).not.toHaveBeenCalled()
    expect(vs.assignAgent).not.toHaveBeenCalled()
  })

  it('agent.kill → killAgent(ctx.agentId)', () => {
    run('agent.kill', { agentId: 'a9' })
    expect(clientMock.killAgent).toHaveBeenCalledWith('a9')
  })
  it('agent.kill: agentId 누락 → throw', () => {
    expect(() => run('agent.kill', {})).toThrow(/agentId/)
    expect(clientMock.killAgent).not.toHaveBeenCalled()
  })

  it('empty 콘텐츠 메뉴 최상위 = 에이전트 모니터링 → 새 콘텐츠(컨테이너) → 가로/세로 분할 → 닫기 (ADR-0067/0065 트림)', () => {
    const items = buildSlotMenu('empty')
    expect(items.map(i => i.id)).toEqual([
      'slot.assignRunningAgent',
      'container:새 콘텐츠',
      'slot.split.topBottom',
      'slot.split.leftRight',
      'slot.close',
    ])
    // ADR-0067: "생성" 은 뺀다 — 스폰은 트리 소관.
    expect(items[1].children?.map(c => c.id)).toEqual([
      'slot.fill.agentList',
      'slot.fill.presetPalette',
    ])
    expect(items[1].children?.map(c => c.id)).not.toContain('slot.createAgentHere')
    expect(items.map(i => i.id)).not.toContain('slot.empty')
    expect(items.map(i => i.id)).not.toContain('slot.popout')
  })

  it('§5 불변: 접힌 slot.fill.* 도 registry 로 여전히 직접 실행 가능', () => {
    run('slot.fill.agentList', CTX)
    run('slot.fill.presetPalette', CTX)
    expect(vs.setSlotContent).toHaveBeenCalledWith('v1', 's1', { type: 'agent_list' })
    expect(vs.setSlotContent).toHaveBeenCalledWith('v1', 's1', { type: 'preset_palette' })
  })
  it('agent 콘텐츠 메뉴 = 에이전트 모니터링(order 5) → 종료(order 10) 순 (ADR-0067)', () => {
    const items = buildSlotMenu('agent')
    expect(items[0].id).toBe('slot.assignRunningAgent')
    expect(items[1].id).toBe('agent.kill')
  })
})
