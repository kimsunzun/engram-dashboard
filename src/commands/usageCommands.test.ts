// usageCommands — 사용량 슬롯의 command(TRD S21 usage-limit-slot §4 「프론트 표시」 행의 등록점 · ⟳ 대상 · 토글).
// 그것을 부르는 화면(작은 표시의 ⟳ · 팝업의 표시 토글 줄)은 `UsageSlot.test`, 매니페스트를 거친 실제 우클릭은
// `ViewLayoutRenderer.test` 가 잰다.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invokeMock = vi.hoisted(() => vi.fn(async (_cmd: string, _args?: unknown) => undefined as unknown))
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => vi.fn()) }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ close: vi.fn(async () => undefined), label: () => 'main' }),
}))
vi.mock('../api/clientFactory', () => ({ agentClient: {} }))

import './usageCommands' // side-effect: register
import type { UsageLimitSnapshot } from '../../crates/engram-dashboard-protocol/bindings/UsageLimitSnapshot'
import type { SlotContent } from '../api/layoutTypes'
import type { AgentBackendKind } from '../api/types'
import { useUsageStore } from '../store/usageStore'
import { getCommand, run } from './registry'
import { fireAndForget } from './dispatch'
import { buildSlotMenu } from './slotMenu'

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

beforeEach(() => {
  refreshSpy.mockClear()
  invokeMock.mockClear()
  useUsageStore.setState({ vendors: {}, socketEpoch: 0, pending: {}, refresh: refreshSpy })
})
afterEach(() => {
  vi.restoreAllMocks()
})

describe('등록점', () => {
  it('셋이 등록되고 제목은 리터럴 그대로다', () => {
    expect(IDS.map(id => getCommand(id)?.title)).toEqual(['사용량 새로고침', 'Claude 표시', 'Codex 표시'])
  })

  it('셋 다 help 가 없다 — 버스에 오르지 않는다(TRD §3 #26 · 버스의 usage.refresh 와 겹치지 않게)', () => {
    for (const id of IDS) expect(getCommand(id)?.help).toBeUndefined()
  })

  it('command 에 when 이 없다', () => {
    for (const id of IDS) expect(getCommand(id)?.when).toBeUndefined()
  })

  it('슬롯 우클릭 메뉴에 기여하지 않는다 — 사용량 슬롯을 비롯해 어느 콘텐츠의 메뉴에도 usageSlot.* 가 없다(사용자 결정 2026-09-29)', () => {
    for (const type of ['usage', 'empty', 'agent', 'agent_list', 'preset_palette'] as const) {
      expect(buildSlotMenu(type).some(i => i.id.startsWith('usageSlot.'))).toBe(false)
    }
  })
})

/** 셸로 나간 쓰기 — 표시 칸 쓰기 외의 invoke 는 거른다. */
function usageWrites(): unknown[] {
  return invokeMock.mock.calls.filter(([cmd]) => cmd === 'set_usage_slot').map(([, args]) => args)
}

describe('표시 토글 = 제 칸 하나만 쓴다', () => {
  it('toggleClaude → Claude 칸을 뒤집은 값 하나만 · Codex 칸은 비운다(셸이 지금 값으로 지킨다)', async () => {
    await run('usageSlot.toggleClaude', { viewId: 'v1', slotId: 's1', content: usage(true, false) })
    expect(usageWrites()).toEqual([{ viewId: 'v1', slotId: 's1', showClaude: false, showCodex: null }])
  })

  it('toggleCodex → Codex 칸 하나만', async () => {
    await run('usageSlot.toggleCodex', { viewId: 'v1', slotId: 's1', content: usage(true, false) })
    expect(usageWrites()).toEqual([{ viewId: 'v1', slotId: 's1', showClaude: null, showCodex: true }])
  })

  it('방송 전의 다른 회사 연달은 누름 — 둘 다 같은 옛 내용에서 불려도 쓰기는 칸 하나씩이라 서로 덮지 않는다', async () => {
    const stale = usage(true, false)
    await run('usageSlot.toggleCodex', { viewId: 'v1', slotId: 's1', content: stale })
    await run('usageSlot.toggleClaude', { viewId: 'v1', slotId: 's1', content: stale })
    expect(usageWrites()).toEqual([
      { viewId: 'v1', slotId: 's1', showClaude: null, showCodex: true },
      { viewId: 'v1', slotId: 's1', showClaude: false, showCodex: null },
    ])
  })

  it('전량 교체(set_slot_content)로 쓰지 않는다', async () => {
    await run('usageSlot.toggleClaude', { viewId: 'v1', slotId: 's1', content: usage(true, true) })
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === 'set_slot_content')).toBe(false)
  })

  it('내용이 없거나 사용량 슬롯이 아니면 throw · 좌표가 없어도 throw — 어느 쪽도 쓰지 않는다', () => {
    expect(() => run('usageSlot.toggleClaude', { viewId: 'v1', slotId: 's1' })).toThrow(/content 필요/)
    expect(() => run('usageSlot.toggleClaude', { viewId: 'v1', slotId: 's1', content: { type: 'empty' } })).toThrow(
      /content 필요/,
    )
    expect(() => run('usageSlot.toggleCodex', { slotId: 's1', content: usage(true, true) })).toThrow(/viewId 필요/)
    expect(() => run('usageSlot.toggleCodex', { viewId: 'v1', content: usage(true, true) })).toThrow(/slotId 필요/)
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('show 칸이 하나라도 boolean 이 아니면 throw — 토글도 ⟳ 와 같은 인자 계약이다', () => {
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
    expect(invokeMock).not.toHaveBeenCalled()
    expect(refreshSpy).not.toHaveBeenCalled()
  })
})

describe('⟳ = 켠 회사 중 보이는 거절이 아닌 것만 · 누를 때 스토어로 다시 본다', () => {
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

  it('대상이 없으면 throw — 켠 회사가 없거나 전부 거절 · 새로고침 0', () => {
    seed({ claude: 'Rejected', codex: 'Ready' })
    expect(() => run('usageSlot.refresh', { content: usage(true, false) })).toThrow(/새로고침할 회사가 없다/)
    expect(() => run('usageSlot.refresh', { content: usage(false, false) })).toThrow(/새로고침할 회사가 없다/)
    expect(() => run('usageSlot.refresh', {})).toThrow(/content 필요/)
    expect(refreshSpy).not.toHaveBeenCalled()
  })

  it('⟳ 를 그린 뒤 거절이 닿은 틈에 누름 — 버튼이 부르는 fireAndForget 이 throw 를 잡아 로그만 남기고 상태는 그대로', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    seed({ claude: 'Rejected' })
    const before = useUsageStore.getState().vendors
    expect(() =>
      fireAndForget('usageSlot.refresh', { viewId: 'v1', slotId: 's1', content: usage(true, false) }),
    ).not.toThrow()
    expect(warn).toHaveBeenCalledWith(expect.stringMatching(/usageSlot\.refresh/), expect.any(Error))
    expect(refreshSpy).not.toHaveBeenCalled()
    expect(useUsageStore.getState().vendors).toBe(before)
    expect(useUsageStore.getState().pending).toEqual({})
  })
})

// ── 회사 하나 새로고침(ADR-0259 결정 2) — 팝업의 회사별 ⟳ 와 LLM 이 같은 command 에 `vendor` 를 더해 부른다 ──
describe('⟳ 의 vendor 인자 = 그 회사 하나', () => {
  it('있으면 그 회사만 — 다른 켠 회사는 새로고침하지 않는다', async () => {
    seed({ claude: 'Ready', codex: 'Ready' })
    await run('usageSlot.refresh', { content: usage(true, true), vendor: 'codex' })
    expect(refreshSpy.mock.calls.map(c => c[0])).toEqual(['codex'])
  })

  it('없으면 지금처럼 켠 회사 전부', async () => {
    seed({ claude: 'Ready', codex: 'Ready' })
    await run('usageSlot.refresh', { content: usage(true, true) })
    expect(refreshSpy.mock.calls.map(c => c[0])).toEqual(['claude', 'codex'])
  })

  it('조회 실패(Failed)·아직 값 없음은 거절이 아니다 — 새로고침한다', async () => {
    seed({ codex: 'Failed' })
    await run('usageSlot.refresh', { content: usage(true, true), vendor: 'codex' })
    await run('usageSlot.refresh', { content: usage(true, true), vendor: 'claude' })
    expect(refreshSpy.mock.calls.map(c => c[0])).toEqual(['codex', 'claude'])
  })

  it('끈 회사면 throw — 전역 ⟳ 의 「대상 0」 처럼 조용히 삼키지 않는다', () => {
    seed({ claude: 'Ready', codex: 'Ready' })
    expect(() => run('usageSlot.refresh', { content: usage(true, false), vendor: 'codex' })).toThrow(/codex 는 이 슬롯에서 꺼져 있다/)
    expect(refreshSpy).not.toHaveBeenCalled()
  })

  it('보이는 거절(Rejected)이면 throw — 다른 회사는 건드리지 않는다', () => {
    seed({ claude: 'Rejected', codex: 'Ready' })
    expect(() => run('usageSlot.refresh', { content: usage(true, true), vendor: 'claude' })).toThrow(/claude 는 거절 중이다/)
    expect(refreshSpy).not.toHaveBeenCalled()
  })

  it('모르는 회사·문자열이 아닌 값이면 throw · content 계약은 그대로', () => {
    for (const vendor of ['gemini', '', null, 1, 'toString', 'show_claude']) {
      expect(() => run('usageSlot.refresh', { content: usage(true, true), vendor })).toThrow(/vendor 는 claude·codex 중 하나/)
    }
    expect(() => run('usageSlot.refresh', { vendor: 'claude' })).toThrow(/content 필요/)
    expect(refreshSpy).not.toHaveBeenCalled()
  })
})
