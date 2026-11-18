// chat.toolGroup.setExpanded — in-window command test (ADR-0239 · TRD S21-chat-ux §4-4).
//
// ★Do not import `./chatCommands` directly — it comes in through the manifest★ (reason = the head of renderModeCommands.test.ts):
//   that way this breaks if the one import line in contributions.ts disappears. The stub set is also the same as that file's.

import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => vi.fn()) }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ label: 'main', close: vi.fn(async () => undefined) }),
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }))
vi.mock('../api/clientFactory', () => ({
  agentClient: new Proxy({}, { get: () => vi.fn(async () => undefined) }),
  getAgentClient: vi.fn(),
  bootstrapDaemonIfNeeded: vi.fn(async () => undefined),
}))

import './contributions' // side-effect register — ★no direct import (header above)★
import { list, run } from './registry'
import { effectiveOpen, useToolGroupStore } from '../store/toolGroupStore'

const ID = 'chat.toolGroup.setExpanded'

const bySlot = () => useToolGroupStore.getState().bySlot

beforeEach(() => {
  useToolGroupStore.setState({ bySlot: {} })
  useToolGroupStore.getState().bind('s1', 'a1')
})

describe('chat.toolGroup.setExpanded', () => {
  it('goes into the registry via the manifest (contributions)', () => {
    expect(list().map(c => c.id)).toContain(ID)
  })

  // ★Not on the bus (ADR-0167)★: expanded state is frontend state that exists separately per window — the reason is at the head of chatCommands.ts.
  it('has no help — does not go on the bus and stays inside this window only', () => {
    expect(list().find(c => c.id === ID)!.help).toBeUndefined()
  })

  it('picks the expansion of that group of that slot and returns { expanded } — both directions', () => {
    expect(run(ID, { slotId: 's1', groupKey: 'tool:call_1', expanded: true })).toEqual({ expanded: true })
    expect(effectiveOpen(useToolGroupStore.getState(), 's1', 'tool:call_1')).toBe(true)
    expect(run(ID, { slotId: 's1', groupKey: 'item:7', expanded: false })).toEqual({ expanded: false })
    expect(effectiveOpen(useToolGroupStore.getState(), 's1', 'item:7')).toBe(false)
    expect(bySlot().s1.open).toEqual({ 'tool:call_1': true, 'item:7': false }) // A collapse is also recorded as the chosen value.
  })

  it('a bag mixed with extra fields (viewId etc.) passes as is', () => {
    run(ID, { viewId: 'v1', slotId: 's1', groupKey: 'tool:call_1', expanded: true })
    expect(bySlot().s1.open).toEqual({ 'tool:call_1': true })
  })

  it.each([
    ['slotId missing', { groupKey: 'tool:x', expanded: true }, /slotId required/],
    ['slotId empty string', { slotId: '', groupKey: 'tool:x', expanded: true }, /slotId required/],
    ['groupKey missing', { slotId: 's1', expanded: true }, /groupKey/],
    ['groupKey is the backend id as is', { slotId: 's1', groupKey: 'call_1', expanded: true }, /groupKey/],
    ['groupKey is not a string', { slotId: 's1', groupKey: 3, expanded: true }, /groupKey/],
    ['expanded missing', { slotId: 's1', groupKey: 'tool:x' }, /expanded/],
    ['expanded is a string', { slotId: 's1', groupKey: 'tool:x', expanded: 'true' }, /expanded/],
  ] as const)('invalid arguments throw and leave the state unchanged — %s', (_label, args, message) => {
    const before = bySlot()
    expect(() => run(ID, { ...args })).toThrow(message)
    expect(bySlot()).toBe(before)
  })

  it('throws without arguments', () => {
    expect(() => run(ID)).toThrow(/slotId required/)
  })

  it('if this window has no conversation view of that slot (never bound), throws and writes nothing', () => {
    const before = bySlot()
    expect(() => run(ID, { slotId: 's9', groupKey: 'tool:x', expanded: true })).toThrow(/no conversation view/)
    expect(bySlot()).toBe(before)
  })

  // After the view is unmounted by a render mode swap · a popout move — answering success would leave the screen unchanged while the caller reads it as changed.
  it('for a slot whose view was unmounted, throws and writes nothing — takes effect once mounted again', () => {
    const release = useToolGroupStore.getState().bind('s2', 'a1')
    release()
    const before = bySlot()
    expect(() => run(ID, { slotId: 's2', groupKey: 'tool:x', expanded: true })).toThrow(/no conversation view/)
    expect(bySlot()).toBe(before)
    useToolGroupStore.getState().bind('s2', 'a1')
    expect(run(ID, { slotId: 's2', groupKey: 'tool:x', expanded: true })).toEqual({ expanded: true })
  })
})
