// usageCommands — the commands of the usage slot (TRD S21 usage-limit-slot §4 registration point of the 「frontend display」 row · ⟳ targets · toggles).
// The screens that call them (the ⟳ of the small display · the display toggle rows of the popup) are measured by `UsageSlot.test`, and the real right-click through the manifest
// by `ViewLayoutRenderer.test`.

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

describe('registration point', () => {
  it('three are registered and their titles are the literals as is', () => {
    expect(IDS.map(id => getCommand(id)?.title())).toEqual(['사용량 새로고침', 'Claude 표시', 'Codex 표시'])
  })

  it('none of the three has help — they do not go on the bus (TRD §3 #26 · so they do not overlap the bus usage.refresh)', () => {
    for (const id of IDS) expect(getCommand(id)?.help).toBeUndefined()
  })

  it('the commands have no when', () => {
    for (const id of IDS) expect(getCommand(id)?.when).toBeUndefined()
  })

  it('they do not contribute to the slot right-click menu — no menu of any content, the usage slot included, has usageSlot.* (user decision 2026-09-29)', () => {
    for (const type of ['usage', 'empty', 'agent', 'agent_list'] as const) {
      expect(buildSlotMenu(type).some(i => i.id.startsWith('usageSlot.'))).toBe(false)
    }
  })
})

/** Writes sent to the shell — invokes other than display field writes are filtered out. */
function usageWrites(): unknown[] {
  return invokeMock.mock.calls.filter(([cmd]) => cmd === 'set_usage_slot').map(([, args]) => args)
}

describe('display toggle = writes only its own field', () => {
  it('toggleClaude → only the Claude field with the flipped value · the Codex field is left empty (the shell keeps it at the current value)', async () => {
    await run('usageSlot.toggleClaude', { viewId: 'v1', slotId: 's1', content: usage(true, false) })
    expect(usageWrites()).toEqual([{ viewId: 'v1', slotId: 's1', showClaude: false, showCodex: null }])
  })

  it('toggleCodex → only the Codex field', async () => {
    await run('usageSlot.toggleCodex', { viewId: 'v1', slotId: 's1', content: usage(true, false) })
    expect(usageWrites()).toEqual([{ viewId: 'v1', slotId: 's1', showClaude: null, showCodex: true }])
  })

  it('consecutive presses of different vendors before the broadcast — even called from the same old content, each write is one field, so they do not overwrite each other', async () => {
    const stale = usage(true, false)
    await run('usageSlot.toggleCodex', { viewId: 'v1', slotId: 's1', content: stale })
    await run('usageSlot.toggleClaude', { viewId: 'v1', slotId: 's1', content: stale })
    expect(usageWrites()).toEqual([
      { viewId: 'v1', slotId: 's1', showClaude: null, showCodex: true },
      { viewId: 'v1', slotId: 's1', showClaude: false, showCodex: null },
    ])
  })

  it('does not write with a full replacement (set_slot_content)', async () => {
    await run('usageSlot.toggleClaude', { viewId: 'v1', slotId: 's1', content: usage(true, true) })
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === 'set_slot_content')).toBe(false)
  })

  it('throws when there is no content or it is not a usage slot · throws without coordinates too — neither writes', () => {
    expect(() => run('usageSlot.toggleClaude', { viewId: 'v1', slotId: 's1' })).toThrow(/content required/)
    expect(() => run('usageSlot.toggleClaude', { viewId: 'v1', slotId: 's1', content: { type: 'empty' } })).toThrow(
      /content required/,
    )
    expect(() => run('usageSlot.toggleCodex', { slotId: 's1', content: usage(true, true) })).toThrow(/viewId required/)
    expect(() => run('usageSlot.toggleCodex', { viewId: 'v1', content: usage(true, true) })).toThrow(/slotId required/)
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('throws when any show field is not a boolean — toggles have the same argument contract as ⟳', () => {
    const bad: Array<Record<string, unknown>> = [
      { type: 'usage', show_codex: true },
      { type: 'usage', show_claude: true },
      { type: 'usage', show_claude: 'true', show_codex: false },
      { type: 'usage', show_claude: true, show_codex: null },
    ]
    for (const content of bad) {
      expect(() => run('usageSlot.toggleClaude', { viewId: 'v1', slotId: 's1', content })).toThrow(
        /show_claude·show_codex must both be booleans/,
      )
      expect(() => run('usageSlot.refresh', { content })).toThrow(/show_claude·show_codex must both be booleans/)
    }
    expect(invokeMock).not.toHaveBeenCalled()
    expect(refreshSpy).not.toHaveBeenCalled()
  })
})

describe('⟳ = only the enabled vendors that are not a visible refusal · re-checked against the store on press', () => {
  it('both on and both fine → both refreshed', async () => {
    seed({ claude: 'Ready', codex: 'Failed' })
    await run('usageSlot.refresh', { content: usage(true, true) })
    expect(refreshSpy.mock.calls.map(c => c[0])).toEqual(['claude', 'codex'])
  })

  it('a vendor turned off is skipped', async () => {
    seed({ claude: 'Ready', codex: 'Ready' })
    await run('usageSlot.refresh', { content: usage(false, true) })
    expect(refreshSpy.mock.calls.map(c => c[0])).toEqual(['codex'])
  })

  it('a visible refusal (Rejected) is skipped', async () => {
    seed({ claude: 'Rejected', codex: 'Ready' })
    await run('usageSlot.refresh', { content: usage(true, true) })
    expect(refreshSpy.mock.calls.map(c => c[0])).toEqual(['codex'])
  })

  it('a vendor that has not received a single reading yet is included — ⟳ is the path that fetches the first value', async () => {
    await run('usageSlot.refresh', { content: usage(true, false) })
    expect(refreshSpy.mock.calls.map(c => c[0])).toEqual(['claude'])
  })

  it('throws when there is no target — no vendor on or all refused · 0 refreshes', () => {
    seed({ claude: 'Rejected', codex: 'Ready' })
    expect(() => run('usageSlot.refresh', { content: usage(true, false) })).toThrow(/no vendor to refresh/)
    expect(() => run('usageSlot.refresh', { content: usage(false, false) })).toThrow(/no vendor to refresh/)
    expect(() => run('usageSlot.refresh', {})).toThrow(/content required/)
    expect(refreshSpy).not.toHaveBeenCalled()
  })

  it('a press in the gap after ⟳ is drawn and a refusal arrives — the fireAndForget the button calls catches the throw, only logs, and the state stays', () => {
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

// ── Refreshing one vendor (ADR-0259 decision 2) — the per-vendor ⟳ of the popup and the LLM call the same command with `vendor` added ──
describe('the vendor argument of ⟳ = that one vendor', () => {
  it('when present, only that vendor — other enabled vendors are not refreshed', async () => {
    seed({ claude: 'Ready', codex: 'Ready' })
    await run('usageSlot.refresh', { content: usage(true, true), vendor: 'codex' })
    expect(refreshSpy.mock.calls.map(c => c[0])).toEqual(['codex'])
  })

  it('when absent, every enabled vendor as now', async () => {
    seed({ claude: 'Ready', codex: 'Ready' })
    await run('usageSlot.refresh', { content: usage(true, true) })
    expect(refreshSpy.mock.calls.map(c => c[0])).toEqual(['claude', 'codex'])
  })

  it('a lookup failure (Failed) · no value yet is not a refusal — it refreshes', async () => {
    seed({ codex: 'Failed' })
    await run('usageSlot.refresh', { content: usage(true, true), vendor: 'codex' })
    await run('usageSlot.refresh', { content: usage(true, true), vendor: 'claude' })
    expect(refreshSpy.mock.calls.map(c => c[0])).toEqual(['codex', 'claude'])
  })

  it('throws for a vendor turned off — not swallowed silently like the 「0 targets」 of the global ⟳', () => {
    seed({ claude: 'Ready', codex: 'Ready' })
    expect(() => run('usageSlot.refresh', { content: usage(true, false), vendor: 'codex' })).toThrow(/codex is turned off in this slot/)
    expect(refreshSpy).not.toHaveBeenCalled()
  })

  it('throws on a visible refusal (Rejected) — other vendors are not touched', () => {
    seed({ claude: 'Rejected', codex: 'Ready' })
    expect(() => run('usageSlot.refresh', { content: usage(true, true), vendor: 'claude' })).toThrow(/claude is being refused/)
    expect(refreshSpy).not.toHaveBeenCalled()
  })

  it('throws on an unknown vendor · a non-string value · the content contract stays', () => {
    for (const vendor of ['gemini', '', null, 1, 'toString', 'show_claude']) {
      expect(() => run('usageSlot.refresh', { content: usage(true, true), vendor })).toThrow(/vendor must be one of claude·codex/)
    }
    expect(() => run('usageSlot.refresh', { vendor: 'claude' })).toThrow(/content required/)
    expect(refreshSpy).not.toHaveBeenCalled()
  })
})
