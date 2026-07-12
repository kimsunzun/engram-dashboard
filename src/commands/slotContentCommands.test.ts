// ADR-0067 / ADR-0064/0065: slotContentCommands 단위테스트 — slot.assignRunningAgent 등록·라우팅 +
//   슬롯 메뉴 기여(가시성 hideOn) + "생성" 제거 회귀. headless(DOM/Tauri 의존은 mock).
//
// ★검증 불변식(ADR-0067)★:
//   1. slot.assignRunningAgent 가 registry 에 등록된다.
//   2. run(ctx) 이 우클릭한 slot 좌표(viewId/slotId)로 monitoringPickerStore.open 을 부른다(배치 상태 없음).
//   3. viewId/slotId 없으면 throw(requireCoords fail-loud).
//   4. "에이전트 모니터링" 이 empty·agent 슬롯 메뉴엔 뜨고, 소스 슬롯(agent_list/preset_palette)엔 hideOn 으로 빠진다.
//   5. empty "새 콘텐츠" 서브메뉴에서 "생성"(slot.createAgentHere)이 제거됐다(트리·팔레트만 남음).

import { beforeEach, describe, expect, it, vi } from 'vitest'

// slotContentCommands 는 top-level 에서 tauri plugin-dialog·clientFactory 를 import 한다 → headless 에서 mock.
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

describe('slot.assignRunningAgent 등록·라우팅 (ADR-0067)', () => {
  it('registry 에 등록된다', () => {
    expect(getCommand('slot.assignRunningAgent')).toBeDefined()
    // 리터럴 기대값 — t('agent.monitor')로 비교하면 production·assert 둘 다 ko.ts 참조라 순환(잘못된 값도 통과).
    //   화면에 실제로 나가는 문자열을 리터럴로 못 박아 ko.ts 값 회귀를 잡는다.
    expect(getCommand('slot.assignRunningAgent')!.title).toBe('에이전트 모니터링')
  })

  it('run(ctx) 이 우클릭한 slot 좌표로 monitoringPickerStore.open 을 부른다', () => {
    run('slot.assignRunningAgent', { viewId: 'view-1', slotId: 'slot-9' })
    expect(useMonitoringPickerStore.getState().target).toEqual({ viewId: 'view-1', slotId: 'slot-9' })
  })

  it('viewId/slotId 없으면 throw(requireCoords fail-loud) + 스토어 미변경', () => {
    expect(() => run('slot.assignRunningAgent', {})).toThrow(/viewId 필요/)
    expect(() => run('slot.assignRunningAgent', { viewId: 'v' })).toThrow(/slotId 필요/)
    expect(useMonitoringPickerStore.getState().target).toBeNull()
  })
})

describe('슬롯 메뉴 기여 — 가시성 hideOn (ADR-0067)', () => {
  // buildSlotMenu 는 flat + 컨테이너 자식 id 를 모두 훑는 헬퍼(가시성 판정용).
  const allIds = (contentType: Parameters<typeof buildSlotMenu>[0]): string[] => {
    const flat: string[] = []
    for (const item of buildSlotMenu(contentType)) {
      flat.push(item.id)
      if (item.children) for (const c of item.children) flat.push(c.id)
    }
    return flat
  }

  it('empty·agent 슬롯엔 "에이전트 모니터링" 이 뜬다', () => {
    expect(allIds('empty')).toContain('slot.assignRunningAgent')
    expect(allIds('agent')).toContain('slot.assignRunningAgent')
  })

  it('소스 슬롯(agent_list/preset_palette)엔 hideOn 으로 빠진다', () => {
    expect(allIds('agent_list')).not.toContain('slot.assignRunningAgent')
    expect(allIds('preset_palette')).not.toContain('slot.assignRunningAgent')
  })
})

describe('스폰 정리 — "생성" 제거 회귀 (ADR-0067)', () => {
  it('empty "새 콘텐츠" 서브메뉴는 트리·팔레트만(생성 제거)', () => {
    const container = buildSlotMenu('empty').find(i => i.id === 'container:새 콘텐츠')
    expect(container).toBeDefined()
    const childIds = container!.children!.map(c => c.id)
    expect(childIds).toEqual(['slot.fill.agentList', 'slot.fill.presetPalette'])
    expect(childIds).not.toContain('slot.createAgentHere')
  })
})
