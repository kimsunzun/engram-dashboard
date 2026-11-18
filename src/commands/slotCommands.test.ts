//
// Strategy: the actions of viewStore.getState() are injected as hoisted spies (the command calls getState().split(...)).
//   agentClient·plugin-dialog are stubbed too. registry/slotMenu are the real modules (the registration side effect is observed).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const vs = vi.hoisted(() => ({
  split: vi.fn(async () => 'new'),
  closeSlot: vi.fn(async () => undefined),
  assignAgent: vi.fn(async () => undefined),
  setSlotContent: vi.fn(async () => undefined),
  moveSlotToWindow: vi.fn(async () => ({ window: 'w', tab: 't', window_id: 'id', number: 2 })),
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
import { setLanguage } from '../i18n'
import { list, run } from './registry'
import { buildSlotMenu } from './slotMenu'

const CTX = { viewId: 'v1', slotId: 's1' }

beforeEach(() => {
  vs.split.mockClear(); vs.closeSlot.mockClear(); vs.assignAgent.mockClear()
  vs.setSlotContent.mockClear(); vs.moveSlotToWindow.mockClear()
  clientMock.spawnAgent.mockClear(); clientMock.killAgent.mockClear()
  dialogMock.open.mockClear(); dialogMock.open.mockResolvedValue(null)
})
afterEach(() => vi.restoreAllMocks())

describe('common slot ops (slotCommands) routing', () => {
  // ★Label↔command↔direction wiring (ADR-0140)★: `Split horizontally` (slot.split.topBottom) → top/bottom. The label render layer is
  //   caught by ViewLayoutRenderer.test; here the command id → direction value is pinned.
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
  it('viewId/slotId missing → throw (loud fail before side effects)', () => {
    expect(() => run('slot.split.topBottom', { slotId: 's1' })).toThrow(/viewId/)
    expect(() => run('slot.close', { viewId: 'v1' })).toThrow(/slotId/)
    expect(vs.split).not.toHaveBeenCalled()
    expect(vs.closeSlot).not.toHaveBeenCalled()
  })
})

describe("'*' common contributions (every slot)", () => {
  it('the 5 common items attach to any content as the slot-ops group', () => {
    const ids = buildSlotMenu('agent').map(i => i.id)
    for (const id of ['slot.split.topBottom', 'slot.split.leftRight', 'slot.popout', 'slot.empty', 'slot.close']) {
      expect(ids).toContain(id)
    }
  })
})

describe('core content (slotContentCommands) routing', () => {
  it('slot.fill.agentList → setSlotContent(…,{type:agent_list})', () => {
    run('slot.fill.agentList', CTX)
    expect(vs.setSlotContent).toHaveBeenCalledWith('v1', 's1', { type: 'agent_list' })
  })
  it('slot.fill.presetPalette → setSlotContent(…,{type:preset_palette})', () => {
    run('slot.fill.presetPalette', CTX)
    expect(vs.setSlotContent).toHaveBeenCalledWith('v1', 's1', { type: 'preset_palette' })
  })
  it('slot.fill.usage → setSlotContent(…,{type:usage, both vendors on})', () => {
    run('slot.fill.usage', CTX)
    expect(vs.setSlotContent).toHaveBeenCalledWith('v1', 's1', { type: 'usage', show_claude: true, show_codex: true })
  })

  it('slot.createAgentHere: cwd picked in the dialog → spawnAgent → assignAgent(viewId, slotId, id)', async () => {
    dialogMock.open.mockResolvedValue('C:/picked')
    clientMock.spawnAgent.mockResolvedValueOnce({ id: 'brand-new' })
    await run('slot.createAgentHere', CTX)
    expect(dialogMock.open).toHaveBeenCalledWith(expect.objectContaining({ directory: true, multiple: false }))
    expect(clientMock.spawnAgent).toHaveBeenCalledWith('C:/picked', 'claude')
    expect(vs.assignAgent).toHaveBeenCalledWith('v1', 's1', 'brand-new')
  })
  it('slot.createAgentHere: dialog cancelled (null) → no spawn/assign (no-op)', async () => {
    dialogMock.open.mockResolvedValue(null)
    await run('slot.createAgentHere', CTX)
    expect(clientMock.spawnAgent).not.toHaveBeenCalled()
    expect(vs.assignAgent).not.toHaveBeenCalled()
  })

  it('agent.kill → killAgent(ctx.agentId)', () => {
    run('agent.kill', { agentId: 'a9' })
    expect(clientMock.killAgent).toHaveBeenCalledWith('a9')
  })
  it('agent.kill: agentId missing → throw', () => {
    expect(() => run('agent.kill', {})).toThrow(/agentId/)
    expect(clientMock.killAgent).not.toHaveBeenCalled()
  })

  it('empty content menu top level = Monitor agent → New content (container) → horizontal/vertical split → pop out → close (ADR-0067/0065 trim · ADR-0228)', () => {
    const items = buildSlotMenu('empty')
    expect(items.map(i => i.id)).toEqual([
      'slot.assignRunningAgent',
      'container:slot.newContent',
      'slot.split.topBottom',
      'slot.split.leftRight',
      'slot.popout',
      'slot.close',
    ])
    // ADR-0067: "Create" is left out — spawning belongs to the tree.
    expect(items[1].children?.map(c => c.id)).toEqual([
      'slot.fill.agentList',
      'slot.fill.presetPalette',
      'slot.fill.usage',
    ])
    expect(items[1].children?.map(c => c.id)).not.toContain('slot.createAgentHere')
    expect(items.map(i => i.id)).not.toContain('slot.empty')
  })

  it('§5 unchanged: the folded slot.fill.* are still directly runnable through the registry', () => {
    run('slot.fill.agentList', CTX)
    run('slot.fill.presetPalette', CTX)
    expect(vs.setSlotContent).toHaveBeenCalledWith('v1', 's1', { type: 'agent_list' })
    expect(vs.setSlotContent).toHaveBeenCalledWith('v1', 's1', { type: 'preset_palette' })
  })
  it('agent content menu = Monitor agent (order 5) → Kill (order 10), in that order (ADR-0067)', () => {
    const items = buildSlotMenu('agent')
    expect(items[0].id).toBe('slot.assignRunningAgent')
    expect(items[1].id).toBe('agent.kill')
  })
})

// ADR-0373: titles are resolved when the menu or the list is built — a switch shows the new language with the same items.
describe('language switch — slot menu and list() titles', () => {
  afterEach(() => {
    setLanguage('ko')
  })

  type Shape = { id: string; title: string; children?: Shape[] }
  const shape = (items: ReturnType<typeof buildSlotMenu>): Shape[] =>
    items.map(i => ({
      id: i.id,
      title: i.title,
      ...(i.children ? { children: i.children.map(c => ({ id: c.id, title: c.title })) } : {}),
    }))

  it('the empty-slot menu: same ids in the same order, titles in the current language', () => {
    const before = shape(buildSlotMenu('empty'))
    setLanguage('en')
    const after = shape(buildSlotMenu('empty'))

    const ids = (s: Shape[]) => s.map(i => [i.id, i.children?.map(c => c.id)])
    expect(ids(after)).toEqual(ids(before))
    const container = after.find(i => i.id === 'container:slot.newContent')!
    expect(container.title).toBe('New content')
    expect(container.children!.map(c => c.title)).toEqual(['Open agent tree', 'Open preset palette', 'Open usage limits'])
    expect(after.find(i => i.id === 'slot.split.topBottom')!.title).toBe('Split horizontally')
    expect(before.find(i => i.id === 'container:slot.newContent')!.title).toBe('새 콘텐츠')
  })

  it('list() returns titles in the current language', () => {
    const title = (id: string) => list().find(c => c.id === id)?.title
    expect(title('slot.close')).toBe('닫기')
    setLanguage('en')
    expect(title('slot.close')).toBe('Close')
    expect(title('slot.split.leftRight')).toBe('Split vertically')
  })
})
