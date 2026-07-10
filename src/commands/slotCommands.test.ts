// ADR-0064/0055: 공통 슬롯 ops(slotCommands) + 코어 콘텐츠(slotContentCommands) command 라우팅 테스트.
//
// ★검증 불변식★:
//   1. slot.split.h/v · slot.popout · slot.empty · slot.close 가 등록되고 viewStore 액션으로 (viewId,slotId)
//      좌표를 흘린다(§5 단일 제어 표면, 백엔드 권위).
//   2. viewId/slotId 누락 → throw(좌표 없이 side-effecting 호출 금지).
//   3. '*' 기여에 공통 5항목이 slot-ops 그룹으로 붙는다.
//   4. slot.fill.* / slot.createAgentHere(empty) · agent.kill(agent) 콘텐츠 기여 + 라우팅.
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

import './slotCommands' // side-effect register + '*' 기여
import './slotContentCommands' // side-effect register + empty/agent 기여
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
  it('slot.split.h → split(viewId, slotId, horizontal)', () => {
    run('slot.split.h', CTX)
    expect(vs.split).toHaveBeenCalledWith('v1', 's1', 'horizontal')
  })
  it('slot.split.v → split(viewId, slotId, vertical)', () => {
    run('slot.split.v', CTX)
    expect(vs.split).toHaveBeenCalledWith('v1', 's1', 'vertical')
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
    expect(() => run('slot.split.h', { slotId: 's1' })).toThrow(/viewId/)
    expect(() => run('slot.close', { viewId: 'v1' })).toThrow(/slotId/)
    expect(vs.split).not.toHaveBeenCalled()
    expect(vs.closeSlot).not.toHaveBeenCalled()
  })
})

describe("'*' 공통 기여(모든 슬롯)", () => {
  it('공통 5항목이 slot-ops 그룹으로 어느 콘텐츠에도 붙는다', () => {
    const ids = buildSlotMenu('agent').map(i => i.id)
    for (const id of ['slot.split.h', 'slot.split.v', 'slot.popout', 'slot.empty', 'slot.close']) {
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

  it('empty 콘텐츠 메뉴 = 트리/팔레트/생성(content) 순', () => {
    const ids = buildSlotMenu('empty').map(i => i.id)
    // content 그룹이 앞(공통보다 먼저) — fill 3항목 순서.
    expect(ids.slice(0, 3)).toEqual(['slot.fill.agentList', 'slot.fill.presetPalette', 'slot.createAgentHere'])
  })
  it('agent 콘텐츠 메뉴 = 종료(content) 먼저', () => {
    expect(buildSlotMenu('agent')[0].id).toBe('agent.kill')
  })
})
