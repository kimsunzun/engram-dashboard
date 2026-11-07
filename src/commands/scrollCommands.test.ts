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
  it('매니페스트(contributions)를 타고 레지스트리에 오른다', () => {
    expect(list().map(c => c.id)).toContain(ID)
  })

  // ★Not on the bus (ADR-0167)★: scroll position is frontend state that exists separately per window — the reason is at the head of scrollCommands.ts.
  it('help 가 없다 — 버스에 오르지 않고 이 창 안에만 남는다', () => {
    expect(list().find(c => c.id === ID)!.help).toBeUndefined()
  })

  it('그 슬롯의 손잡이 pin 을 부르고 { pinned: true } 를 돌려준다', () => {
    const handle = mountHandle('s1')
    const other = mountHandle('s2')
    expect(run(ID, { slotId: 's1' })).toEqual({ pinned: true })
    expect(handle.pin).toHaveBeenCalledTimes(1)
    expect(other.pin).not.toHaveBeenCalled()
  })

  it('여분 칸(viewId 등)이 섞인 가방도 그대로 통과한다', () => {
    const handle = mountHandle('s1')
    run(ID, { viewId: 'v1', slotId: 's1' })
    expect(handle.pin).toHaveBeenCalledTimes(1)
  })

  it('손잡이가 없으면 throw — 조용한 no-op 이 아니다', () => {
    expect(() => run(ID, { slotId: 'nowhere' })).toThrow(/nowhere/)
  })

  it('slotId 누락 · 빈 문자열 · 문자열 아님 → throw', () => {
    mountHandle('s1')
    expect(() => run(ID, {})).toThrow(/slotId/)
    expect(() => run(ID, { slotId: '' })).toThrow(/slotId/)
    expect(() => run(ID, { slotId: 7 })).toThrow(/slotId/)
  })
})
