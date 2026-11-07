// Strategy: the sibling (slotCommands.test.ts) swaps store actions for spies and measures only "what was called", but here we use the ★real
//   viewStore★ and assert renderModeOverride directly. Two reasons — ① the toggle's requirement is a round trip (dom→default→dom), and with a spy
//   the decision itself lives inside the store so it cannot be measured. ② That an invalid argument does not reach the store is
//   stronger measured as "state did not change + the store's warn guard did not fire" than as "the spy was not called".
//
// ★Do not import `./renderModeCommands` directly — it comes in through the manifest★: what the app actually attaches
//   is the one `./contributions` line (`App.tsx`), and if we picked the module directly, tests here would stay green even if that line disappeared
//   (since the direct import does the registration itself). Only by going through the manifest is 「__engramCmd.list 에 뜬다」 (it shows up in __engramCmd.list)
//   actually exercised. The price is that every command module loads, so the stub set is the same as busCommands.test.ts.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

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
import { useViewStore } from '../store/viewStore'

const IDS = [
  'slot.renderMode.set',
  'slot.renderMode.clear',
  'slot.domMode.enable',
  'slot.domMode.disable',
  'slot.domMode.toggle',
]

const override = (): Record<string, string> => useViewStore.getState().renderModeOverride

beforeEach(() => useViewStore.setState({ renderModeOverride: {} }))
afterEach(() => vi.restoreAllMocks())

describe('렌더 모드 command 라우팅', () => {
  // ★Manifest wiring lock★: if you delete `import './renderModeCommands'` from contributions.ts, this breaks first
  //   (all the routing tests below also break together with 「알 수 없는 command id」 (unknown command id)).
  it('다섯 id 가 매니페스트(contributions)를 타고 레지스트리에 오른다 — __engramCmd.list 가 보는 목록', () => {
    const ids = list().map(c => c.id)
    for (const id of IDS) expect(ids, `${id}: contributions.ts 에 모듈 import 가 있나`).toContain(id)
  })

  it('slot.renderMode.set → 그 slot 에 mode 기록', () => {
    run('slot.renderMode.set', { slotId: 's1', mode: 'rich' })
    expect(override()['s1']).toBe('rich')
    run('slot.renderMode.set', { slotId: 's1', mode: 'terminal' })
    expect(override()['s1']).toBe('terminal')
  })

  // ★Case-insensitive acceptance + declared spelling normalization (user decision 2026-09-23)★: what lands in the store is an element of RENDER_MODES,
  //   not the spelling the caller wrote — otherwise the switch in LayoutLeaf silently falls to terminal.
  it('mode 대소문자 무관 수용 → 선언 철자로 정규화해 기록', () => {
    run('slot.renderMode.set', { slotId: 's1', mode: 'RICH' })
    expect(override()['s1']).toBe('rich')
    run('slot.renderMode.set', { slotId: 's1', mode: 'Terminal' })
    expect(override()['s1']).toBe('terminal')
    run('slot.renderMode.set', { slotId: 's1', mode: 'DoM' })
    expect(override()['s1']).toBe('dom')
  })

  it('slot.renderMode.clear → 그 slot 항목만 제거(다른 slot 불변)', () => {
    run('slot.renderMode.set', { slotId: 's1', mode: 'dom' })
    run('slot.renderMode.set', { slotId: 's2', mode: 'rich' })
    run('slot.renderMode.clear', { slotId: 's1' })
    expect(override()['s1']).toBeUndefined()
    expect(override()['s2']).toBe('rich')
  })

  it('오버라이드가 없는 slot 의 clear 는 멱등(throw 없음)', () => {
    expect(() => run('slot.renderMode.clear', { slotId: 'never-set' })).not.toThrow()
    expect(override()).toEqual({})
  })

  it('slot.domMode.enable → dom', () => {
    run('slot.domMode.enable', { slotId: 's1' })
    expect(override()['s1']).toBe('dom')
  })

  // disable = clearRenderMode, so an override that is not dom is withdrawn too (the real meaning of the alias — "return to default").
  it('slot.domMode.disable → 항목 제거(rich 오버라이드도 함께 걷힌다)', () => {
    run('slot.renderMode.set', { slotId: 's1', mode: 'rich' })
    run('slot.domMode.disable', { slotId: 's1' })
    expect(override()['s1']).toBeUndefined()
  })

  it('slot.domMode.toggle 왕복: dom → 기본 → dom', () => {
    run('slot.domMode.toggle', { slotId: 's1' })
    expect(override()['s1']).toBe('dom')
    run('slot.domMode.toggle', { slotId: 's1' })
    expect(override()['s1']).toBeUndefined()
    run('slot.domMode.toggle', { slotId: 's1' })
    expect(override()['s1']).toBe('dom')
  })

  it('slot.domMode.toggle: dom 아닌 오버라이드(rich)에서는 dom 으로 올라간다', () => {
    run('slot.renderMode.set', { slotId: 's1', mode: 'rich' })
    run('slot.domMode.toggle', { slotId: 's1' })
    expect(override()['s1']).toBe('dom')
  })

  // slotId is a globally unique UUID across windows (ADR-0035) so it does not take viewId — but it must be able to take the slot menu ctx bag
  //   as is, so extra keys are ignored.
  it('여분 viewId 가 섞인 ctx 가방도 그대로 통과', () => {
    run('slot.domMode.enable', { viewId: 'v1', slotId: 's1' })
    expect(override()['s1']).toBe('dom')
  })
})

describe('무효 인자 거절(조용한 no-op 금지)', () => {
  it('slotId 누락 → 다섯 전부 throw + store 불변', () => {
    for (const id of IDS) {
      expect(() => run(id, { mode: 'dom' }), id).toThrow(/slotId/)
    }
    expect(override()).toEqual({})
  })

  it('slotId 가 문자열이 아니거나 빈 문자열 → throw', () => {
    expect(() => run('slot.domMode.enable', { slotId: 42 })).toThrow(/slotId/)
    expect(() => run('slot.domMode.enable', { slotId: '' })).toThrow(/slotId/)
    expect(override()).toEqual({})
  })

  // ★Key★: the store's setRenderMode warns and ignores an invalid mode — if it flowed down that path, it would look like
  //   success to the caller. The command layer rejects first, so it never even reaches the store's warn guard.
  it('mode 무효/누락 → throw + store 불변 + store warn 가드 미도달', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    expect(() => run('slot.renderMode.set', { slotId: 's1', mode: 'bogus' })).toThrow(/mode/)
    expect(() => run('slot.renderMode.set', { slotId: 's1' })).toThrow(/mode/)
    expect(() => run('slot.renderMode.set', { slotId: 's1', mode: 7 })).toThrow(/mode/)
    expect(override()['s1']).toBeUndefined()
    expect(warn).not.toHaveBeenCalled()
  })

  it('mode 만 무효여도 slotId 쪽 부분 적용이 남지 않는다', () => {
    run('slot.renderMode.set', { slotId: 's1', mode: 'dom' })
    expect(() => run('slot.renderMode.set', { slotId: 's1', mode: 'bogus' })).toThrow(/mode/)
    expect(override()['s1']).toBe('dom') // Old value kept — an invalid call neither clears nor overwrites it.
  })
})
