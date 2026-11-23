// ADR-0067 / ADR-0064/0065

import { beforeEach, describe, expect, it, vi } from 'vitest'

// slotContentCommands imports tauri plugin-dialog·clientFactory at top level → mocked for headless.
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }))
vi.mock('../api/clientFactory', () => ({
  agentClient: { spawnAgent: vi.fn(), killAgent: vi.fn() },
  getAgentClient: vi.fn(),
}))

import './slotContentCommands' // side-effect: register + registerSlotMenu
import { getCommand, run } from './registry'
import { buildSlotMenu } from './slotMenu'
import { useMonitoringPickerStore } from '../store/monitoringPickerStore'

beforeEach(() => {
  useMonitoringPickerStore.setState({ target: null })
})

describe('slot.assignRunningAgent registration·routing (ADR-0067)', () => {
  it('is registered in the registry', () => {
    expect(getCommand('slot.assignRunningAgent')).toBeDefined()
    // A literal expected value — comparing with t('agent.monitor') makes production and assert both reference ko.ts, which is circular (a wrong value passes too).
    //   Pinning the string that actually goes on screen as a literal catches a regression of the ko.ts value.
    expect(getCommand('slot.assignRunningAgent')!.title()).toBe('에이전트 모니터링')
  })

  it('run(ctx) calls monitoringPickerStore.open with the right-clicked slot coordinates', () => {
    run('slot.assignRunningAgent', { viewId: 'view-1', slotId: 'slot-9' })
    expect(useMonitoringPickerStore.getState().target).toEqual({ viewId: 'view-1', slotId: 'slot-9' })
  })

  it('without viewId/slotId → throw (requireCoords fail-loud) + store unchanged', () => {
    expect(() => run('slot.assignRunningAgent', {})).toThrow(/viewId required/)
    expect(() => run('slot.assignRunningAgent', { viewId: 'v' })).toThrow(/slotId required/)
    expect(useMonitoringPickerStore.getState().target).toBeNull()
  })
})

describe('slot menu contributions — visibility hideOn (ADR-0067)', () => {
  const allIds = (contentType: Parameters<typeof buildSlotMenu>[0]): string[] => {
    const flat: string[] = []
    for (const item of buildSlotMenu(contentType)) {
      flat.push(item.id)
      if (item.children) for (const c of item.children) flat.push(c.id)
    }
    return flat
  }

  it('"Monitor agent" shows on empty·agent slots', () => {
    expect(allIds('empty')).toContain('slot.assignRunningAgent')
    expect(allIds('agent')).toContain('slot.assignRunningAgent')
  })

  it('source slots (agent_list/preset_palette) and the usage slot drop it through hideOn', () => {
    expect(allIds('agent_list')).not.toContain('slot.assignRunningAgent')
    expect(allIds('preset_palette')).not.toContain('slot.assignRunningAgent')
    expect(allIds('usage')).not.toContain('slot.assignRunningAgent')
  })
})

describe('spawn cleanup — "Create" removal regression (ADR-0067)', () => {
  it('the empty "New content" submenu has only tree · palette · usage · PC list (Create removed)', () => {
    const container = buildSlotMenu('empty').find(i => i.id === 'container:slot.newContent')
    expect(container).toBeDefined()
    const childIds = container!.children!.map(c => c.id)
    expect(childIds).toEqual(['slot.fill.agentList', 'slot.fill.presetPalette', 'slot.fill.usage', 'slot.fill.peers'])
    expect(childIds).not.toContain('slot.createAgentHere')
  })
})
