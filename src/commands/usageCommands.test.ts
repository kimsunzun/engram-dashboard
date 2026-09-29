// usageCommands — 사용량 슬롯 메뉴(TRD S21 usage-limit-slot §4 「프론트 표시」 행의 메뉴 `checked` · 등록점).
// 메뉴 DOM(☑ 그리기·비활성)은 `SlotContextMenu.test`, 매니페스트를 거친 실제 우클릭은 `ViewLayoutRenderer.test` 가 잰다.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => vi.fn()) }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ close: vi.fn(async () => undefined), label: () => 'main' }),
}))
vi.mock('../api/clientFactory', () => ({ agentClient: {} }))

import './usageCommands' // side-effect: register + registerSlotMenu
import type { UsageLimitSnapshot } from '../../crates/engram-dashboard-protocol/bindings/UsageLimitSnapshot'
import type { SlotContent } from '../api/layoutTypes'
import type { AgentBackendKind } from '../api/types'
import { useUsageStore } from '../store/usageStore'
import { useViewStore } from '../store/viewStore'
import { getCommand, run } from './registry'
import { buildSlotMenu, SLOT_MENU_ORIGIN, type SlotMenuCtx } from './slotMenu'

const IDS = ['usageSlot.refresh', 'usageSlot.toggleClaude', 'usageSlot.toggleCodex'] as const

function usage(show_claude: boolean, show_codex: boolean): SlotContent {
  return { type: 'usage', show_claude, show_codex }
}

function snap(vendor: AgentBackendKind, kind: 'Ready' | 'Rejected' | 'Failed'): UsageLimitSnapshot {
  const state: UsageLimitSnapshot['state'] =
    kind === 'Ready'
      ? { kind: 'Ready' }
      : kind === 'Rejected'
        ? { kind: 'Rejected', retry_in_secs: 600, detail: null }
        : { kind: 'Failed', next_attempt_in_secs: 60, detail: null }
  return {
    vendor,
    account_key: 'default',
    five_hour: null,
    weekly: null,
    model_scoped: [],
    plan: null,
    in_flight: false,
    state,
    revision: 1,
  }
}

function seed(states: Partial<Record<AgentBackendKind, 'Ready' | 'Rejected' | 'Failed'>>): void {
  const vendors: ReturnType<typeof useUsageStore.getState>['vendors'] = {}
  for (const [vendor, kind] of Object.entries(states) as [AgentBackendKind, 'Ready' | 'Rejected' | 'Failed'][]) {
    vendors[vendor] = { snapshot: snap(vendor, kind), receivedAt: 0, revision: 1 }
  }
  useUsageStore.setState({ vendors })
}

const refreshSpy = vi.fn(async (_vendor: AgentBackendKind) => {})
const setSlotContentSpy = vi.fn(async () => undefined)

beforeEach(() => {
  refreshSpy.mockClear()
  setSlotContentSpy.mockClear()
  useUsageStore.setState({ vendors: {}, socketEpoch: 0, pending: {}, refresh: refreshSpy })
  useViewStore.setState({ setSlotContent: setSlotContentSpy })
})
afterEach(() => {
  vi.restoreAllMocks()
})

describe('등록점', () => {
  it('셋이 등록되고 메뉴 문구는 리터럴 그대로다', () => {
    expect(IDS.map(id => getCommand(id)?.title)).toEqual(['사용량 새로고침', 'Claude 표시', 'Codex 표시'])
  })

  it('셋 다 help 가 없다 — 버스에 오르지 않는다(TRD §3 #26 · 버스의 usage.refresh 와 겹치지 않게)', () => {
    for (const id of IDS) expect(getCommand(id)?.help).toBeUndefined()
  })

  it('usage 슬롯 메뉴 = ⟳ · Claude 표시 · Codex 표시 차례 · ☑ 는 두 토글에만 · 활성 판정은 ⟳ 에만 · command 에 when 없음', () => {
    const own = buildSlotMenu('usage').filter(i => i.id.startsWith('usageSlot.'))
    expect(own.map(i => i.id)).toEqual([...IDS])
    expect(own.map(i => i.checked !== undefined)).toEqual([false, true, true])
    expect(own.map(i => i.enabled !== undefined)).toEqual([true, false, false])
    for (const id of IDS) expect(getCommand(id)?.when).toBeUndefined()
  })

  it('다른 콘텐츠의 메뉴엔 오르지 않는다', () => {
    for (const type of ['empty', 'agent', 'agent_list', 'preset_palette'] as const) {
      expect(buildSlotMenu(type).some(i => i.id.startsWith('usageSlot.'))).toBe(false)
    }
  })
})

describe('☑ · 활성 = ctx 만 읽는다(ADR-0064 — 스토어 무접촉)', () => {
  const own = () => buildSlotMenu('usage').filter(i => i.id.startsWith('usageSlot.'))
  const ctx = (content?: SlotContent, usageRefreshable?: boolean): SlotMenuCtx => ({
    viewId: 'v1',
    slotId: 's1',
    agentId: null,
    content,
    usageRefreshable,
  })

  it('켠 회사 = ☑ · 끈 회사 = ☐ · ⟳ 활성 = ctx.usageRefreshable — 두 스토어 어느 쪽도 읽지 않는다', () => {
    seed({ claude: 'Rejected', codex: 'Rejected' }) // 스토어를 읽는다면 ⟳ 가 비활성으로 나올 상태
    const viewRead = vi.spyOn(useViewStore, 'getState')
    const usageRead = vi.spyOn(useUsageStore, 'getState')
    const [refresh, claude, codex] = own()
    expect([claude.checked!(ctx(usage(true, false))), codex.checked!(ctx(usage(true, false)))]).toEqual([true, false])
    expect([claude.checked!(ctx(usage(false, true))), codex.checked!(ctx(usage(false, true)))]).toEqual([false, true])
    expect(refresh.enabled!(ctx(usage(true, true), true))).toBe(true)
    expect(refresh.enabled!(ctx(usage(true, true), false))).toBe(false)
    expect(refresh.enabled!(ctx(usage(true, true)))).toBe(false)
    expect(viewRead).not.toHaveBeenCalled()
    expect(usageRead).not.toHaveBeenCalled()
  })

  it('ctx 에 내용이 없거나 사용량 슬롯이 아니면 ☐', () => {
    const [, claude] = own()
    expect(claude.checked!(ctx())).toBe(false)
    expect(claude.checked!(ctx({ type: 'empty' }))).toBe(false)
  })
})

describe('표시 토글 = 전량 교체', () => {
  it('toggleClaude → Claude 칸만 뒤집고 두 칸을 다 써 보낸다', async () => {
    await run('usageSlot.toggleClaude', { viewId: 'v1', slotId: 's1', content: usage(true, false) })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's1', usage(false, false))
  })

  it('toggleCodex → Codex 칸만 뒤집는다', async () => {
    await run('usageSlot.toggleCodex', { viewId: 'v1', slotId: 's1', content: usage(true, false) })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's1', usage(true, true))
  })

  it('내용이 없거나 사용량 슬롯이 아니면 throw · 좌표가 없어도 throw — 어느 쪽도 쓰지 않는다', () => {
    expect(() => run('usageSlot.toggleClaude', { viewId: 'v1', slotId: 's1' })).toThrow(/content 필요/)
    expect(() => run('usageSlot.toggleClaude', { viewId: 'v1', slotId: 's1', content: { type: 'empty' } })).toThrow(
      /content 필요/,
    )
    expect(() => run('usageSlot.toggleCodex', { slotId: 's1', content: usage(true, true) })).toThrow(/viewId 필요/)
    expect(() => run('usageSlot.toggleCodex', { viewId: 'v1', content: usage(true, true) })).toThrow(/slotId 필요/)
    expect(setSlotContentSpy).not.toHaveBeenCalled()
  })

  it('show 칸이 하나라도 boolean 이 아니면 throw — 뒤집은 값을 지을 수 없다', () => {
    const bad: Array<Record<string, unknown>> = [
      { type: 'usage', show_codex: true },
      { type: 'usage', show_claude: true },
      { type: 'usage', show_claude: 'true', show_codex: false },
      { type: 'usage', show_claude: true, show_codex: null },
    ]
    for (const content of bad) {
      expect(() => run('usageSlot.toggleClaude', { viewId: 'v1', slotId: 's1', content })).toThrow(
        /show_claude·show_codex 는 둘 다 boolean/,
      )
      expect(() => run('usageSlot.refresh', { content })).toThrow(/show_claude·show_codex 는 둘 다 boolean/)
    }
    expect(setSlotContentSpy).not.toHaveBeenCalled()
    expect(refreshSpy).not.toHaveBeenCalled()
  })
})

describe('메뉴 ⟳ = 켠 회사 중 보이는 거절이 아닌 것만 · 누를 때 스토어로 다시 본다', () => {
  it('둘 다 켜고 둘 다 정상 → 둘 다 새로고침', async () => {
    seed({ claude: 'Ready', codex: 'Failed' })
    await run('usageSlot.refresh', { content: usage(true, true) })
    expect(refreshSpy.mock.calls.map(c => c[0])).toEqual(['claude', 'codex'])
  })

  it('끈 회사는 건너뛴다', async () => {
    seed({ claude: 'Ready', codex: 'Ready' })
    await run('usageSlot.refresh', { content: usage(false, true) })
    expect(refreshSpy.mock.calls.map(c => c[0])).toEqual(['codex'])
  })

  it('보이는 거절(Rejected)은 건너뛴다', async () => {
    seed({ claude: 'Rejected', codex: 'Ready' })
    await run('usageSlot.refresh', { content: usage(true, true) })
    expect(refreshSpy.mock.calls.map(c => c[0])).toEqual(['codex'])
  })

  it('아직 한 장도 못 받은 회사는 든다 — ⟳ 가 첫 값을 부르는 길이다', async () => {
    await run('usageSlot.refresh', { content: usage(true, false) })
    expect(refreshSpy.mock.calls.map(c => c[0])).toEqual(['claude'])
  })

  it('대상이 없을 때 — 메뉴에서 왔으면(origin) 조용히 건너뛰고, 직접 부르면 throw · 둘 다 새로고침 0', () => {
    const debug = vi.spyOn(console, 'debug').mockImplementation(() => {})
    seed({ claude: 'Rejected', codex: 'Ready' })
    expect(run('usageSlot.refresh', { content: usage(true, false), origin: SLOT_MENU_ORIGIN })).toBeUndefined()
    expect(debug).toHaveBeenCalledWith(expect.stringMatching(/새로고침할 회사가 없다 — 건너뜀/))
    expect(() => run('usageSlot.refresh', { content: usage(true, false) })).toThrow(/새로고침할 회사가 없다/)
    expect(() => run('usageSlot.refresh', { content: usage(false, false) })).toThrow(/새로고침할 회사가 없다/)
    expect(() => run('usageSlot.refresh', {})).toThrow(/content 필요/)
    expect(refreshSpy).not.toHaveBeenCalled()
  })
})
