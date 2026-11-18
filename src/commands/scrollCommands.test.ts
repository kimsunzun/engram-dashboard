// slot.scrollToBottom — in-window command test (ADR-0242 · TRD §2-5).
//
// ★Do not import `./scrollCommands` directly — it comes in through the manifest★ (reason = the head of renderModeCommands.test.ts):
//   that way this breaks if the one import line in contributions.ts disappears. The stub set is also the same as that file's.

import { afterEach, describe, expect, it, vi } from 'vitest'

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
import { registerFollow, type FollowHandle } from '../components/slot/scrollFollow/followRegistry'

const ID = 'slot.scrollToBottom'

let disposers: (() => void)[] = []
afterEach(() => {
  for (const d of disposers) d()
  disposers = []
})

function mountHandle(slotId: string): FollowHandle & { pin: ReturnType<typeof vi.fn> } {
  const handle = { pinned: false, pin: vi.fn(), unpin: vi.fn() }
  disposers.push(registerFollow(slotId, handle))
  return handle
}

describe('slot.scrollToBottom', () => {
  it('goes into the registry via the manifest (contributions)', () => {
    expect(list().map(c => c.id)).toContain(ID)
  })

  // ★Not on the bus (ADR-0167)★: scroll position is frontend state that exists separately per window — the reason is at the head of scrollCommands.ts.
  it('has no help — does not go on the bus and stays inside this window only', () => {
    expect(list().find(c => c.id === ID)!.help).toBeUndefined()
  })

  it("calls pin on that slot's handle and returns { pinned: true }", () => {
    const handle = mountHandle('s1')
    const other = mountHandle('s2')
    expect(run(ID, { slotId: 's1' })).toEqual({ pinned: true })
    expect(handle.pin).toHaveBeenCalledTimes(1)
    expect(other.pin).not.toHaveBeenCalled()
  })

  it('a bag mixed with extra fields (viewId etc.) passes as is', () => {
    const handle = mountHandle('s1')
    run(ID, { viewId: 'v1', slotId: 's1' })
    expect(handle.pin).toHaveBeenCalledTimes(1)
  })

  it('throws when there is no handle — not a silent no-op', () => {
    expect(() => run(ID, { slotId: 'nowhere' })).toThrow(/nowhere/)
  })

  it('slotId missing · empty string · not a string → throw', () => {
    mountHandle('s1')
    expect(() => run(ID, {})).toThrow(/slotId/)
    expect(() => run(ID, { slotId: '' })).toThrow(/slotId/)
    expect(() => run(ID, { slotId: 7 })).toThrow(/slotId/)
  })
})
