// 전략: 형제(slotCommands.test.ts)는 store 액션을 spy 로 갈아 "무엇을 불렀나"만 재지만, 여기선 ★실
//   viewStore★ 를 쓰고 renderModeOverride 를 직접 단언한다. 이유 둘 — ① 토글은 왕복(dom→기본→dom)이
//   요구사항인데 spy 로는 판정 자체가 store 안에 있어 재지 못한다. ② 무효 인자가 store 에 닿지 않는다는
//   것은 "spy 가 안 불렸다"보다 "상태가 안 변했다 + store 의 warn 가드가 안 울렸다"로 재는 편이 강하다.
//
// ★`./renderModeCommands` 를 직접 import 하지 않는다 — 매니페스트를 통해 들어온다★: 앱이 실제로 다는
//   것은 `./contributions` 한 줄이고(`App.tsx`), 모듈을 직접 집으면 그 줄이 사라져도 여기 테스트는 계속
//   초록이다(등록 자체는 직접 import 가 해 주므로). 매니페스트를 태워야 「__engramCmd.list 에 뜬다」가
//   실제로 물린다. 그 대가로 command 모듈 전량이 로드되므로 stub 집합은 busCommands.test.ts 와 같다.

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

import './contributions' // side-effect register — ★직접 import 금지(위 헤더)★
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
  // ★매니페스트 배선 자물쇠★: contributions.ts 의 `import './renderModeCommands'` 를 지우면 여기가 먼저
  //   깨진다(아래 라우팅 테스트도 전부 「알 수 없는 command id」로 함께 깨진다).
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

  // disable = clearRenderMode 라 dom 이 아닌 오버라이드도 함께 걷힌다(별칭의 실제 의미 — "기본으로 복귀").
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

  // slotId 는 창 간 전역 고유 UUID 라(ADR-0035) viewId 를 안 받는다 — 다만 슬롯 메뉴 ctx 가방을 그대로
  //   넘길 수 있어야 하므로 여분 키는 무시한다.
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

  // ★핵심★: store 의 setRenderMode 는 무효 mode 를 warn 후 무시한다 — 그 경로로 흘러갔다면 호출자에겐
  //   성공으로 보인다. command 층이 먼저 거절하므로 store 의 warn 가드에는 닿지도 않는다.
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
    expect(override()['s1']).toBe('dom') // 옛 값 유지 — 무효 호출이 지우지도 덮지도 않는다.
  })
})
