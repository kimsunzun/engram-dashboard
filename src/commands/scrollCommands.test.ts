// slot.scrollToBottom — 창 안 command 시험(ADR-0242 · TRD §2-5).
//
// ★`./scrollCommands` 를 직접 import 하지 않는다 — 매니페스트를 통해 들어온다★(사유 = renderModeCommands.test.ts
//   머리): 그래야 contributions.ts 의 import 한 줄이 사라지면 여기가 깨진다. stub 집합도 그 파일과 같다.

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

import './contributions' // side-effect register — ★직접 import 금지(위 헤더)★
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

  // ★버스 미승선(ADR-0167)★: 스크롤 위치는 창마다 따로 있는 프론트 상태다 — 사유는 scrollCommands.ts 머리.
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
