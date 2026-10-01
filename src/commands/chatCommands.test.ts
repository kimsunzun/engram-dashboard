// chat.toolGroup.setExpanded — 창 안 command 시험(ADR-0239 · TRD S21-chat-ux §4-4).
//
// ★`./chatCommands` 를 직접 import 하지 않는다 — 매니페스트를 통해 들어온다★(사유 = renderModeCommands.test.ts
//   머리): 그래야 contributions.ts 의 import 한 줄이 사라지면 여기가 깨진다. stub 집합도 그 파일과 같다.

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

import './contributions' // side-effect register — ★직접 import 금지(위 헤더)★
import { list, run } from './registry'
import { effectiveOpen, useToolGroupStore } from '../store/toolGroupStore'

const ID = 'chat.toolGroup.setExpanded'

const bySlot = () => useToolGroupStore.getState().bySlot

beforeEach(() => {
  useToolGroupStore.setState({ bySlot: {} })
  useToolGroupStore.getState().bind('s1', 'a1')
})

describe('chat.toolGroup.setExpanded', () => {
  it('매니페스트(contributions)를 타고 레지스트리에 오른다', () => {
    expect(list().map(c => c.id)).toContain(ID)
  })

  // ★버스 미승선(ADR-0167)★: 펼침 상태는 창마다 따로 있는 프론트 상태다 — 사유는 chatCommands.ts 머리.
  it('help 가 없다 — 버스에 오르지 않고 이 창 안에만 남는다', () => {
    expect(list().find(c => c.id === ID)!.help).toBeUndefined()
  })

  it('그 슬롯의 그 묶음 펼침을 고르고 { expanded } 를 돌려준다 — 두 방향', () => {
    expect(run(ID, { slotId: 's1', groupKey: 'tool:call_1', expanded: true })).toEqual({ expanded: true })
    expect(effectiveOpen(useToolGroupStore.getState(), 's1', 'tool:call_1')).toBe(true)
    expect(run(ID, { slotId: 's1', groupKey: 'item:7', expanded: false })).toEqual({ expanded: false })
    expect(effectiveOpen(useToolGroupStore.getState(), 's1', 'item:7')).toBe(false)
    expect(bySlot().s1.open).toEqual({ 'tool:call_1': true, 'item:7': false }) // 접힘도 고른 값으로 적힌다.
  })

  it('여분 칸(viewId 등)이 섞인 가방도 그대로 통과한다', () => {
    run(ID, { viewId: 'v1', slotId: 's1', groupKey: 'tool:call_1', expanded: true })
    expect(bySlot().s1.open).toEqual({ 'tool:call_1': true })
  })

  it.each([
    ['slotId 없음', { groupKey: 'tool:x', expanded: true }, /slotId 필요/],
    ['slotId 빈 문자열', { slotId: '', groupKey: 'tool:x', expanded: true }, /slotId 필요/],
    ['groupKey 없음', { slotId: 's1', expanded: true }, /groupKey/],
    ['groupKey 가 백엔드 id 그대로', { slotId: 's1', groupKey: 'call_1', expanded: true }, /groupKey/],
    ['groupKey 가 문자열 아님', { slotId: 's1', groupKey: 3, expanded: true }, /groupKey/],
    ['expanded 없음', { slotId: 's1', groupKey: 'tool:x' }, /expanded/],
    ['expanded 가 문자열', { slotId: 's1', groupKey: 'tool:x', expanded: 'true' }, /expanded/],
  ] as const)('무효 인자는 throw 하고 상태를 안 바꾼다 — %s', (_label, args, message) => {
    const before = bySlot()
    expect(() => run(ID, { ...args })).toThrow(message)
    expect(bySlot()).toBe(before)
  })

  it('인자가 없으면 throw', () => {
    expect(() => run(ID)).toThrow(/slotId 필요/)
  })

  it('이 창에 그 슬롯의 대화 뷰가 없으면(한 번도 안 묶임) throw 하고 아무것도 적지 않는다', () => {
    const before = bySlot()
    expect(() => run(ID, { slotId: 's9', groupKey: 'tool:x', expanded: true })).toThrow(/대화 뷰가 없다/)
    expect(bySlot()).toBe(before)
  })

  // 렌더 모드 교체 · 팝아웃 이동으로 뷰가 마운트 해제된 뒤 — 성공으로 답하면 화면은 그대로인데 호출자는 바뀐 것으로 읽는다.
  it('뷰가 마운트 해제된 슬롯이면 throw 하고 아무것도 적지 않는다 — 다시 마운트되면 먹힌다', () => {
    const release = useToolGroupStore.getState().bind('s2', 'a1')
    release()
    const before = bySlot()
    expect(() => run(ID, { slotId: 's2', groupKey: 'tool:x', expanded: true })).toThrow(/대화 뷰가 없다/)
    expect(bySlot()).toBe(before)
    useToolGroupStore.getState().bind('s2', 'a1')
    expect(run(ID, { slotId: 's2', groupKey: 'tool:x', expanded: true })).toEqual({ expanded: true })
  })
})
