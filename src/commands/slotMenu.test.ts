// ADR-0064: slotMenu 기여 API + 결정적 빌더 단위테스트(headless — DOM/Tauri 의존 0).
//
// ★검증 불변식★:
//   1. buildSlotMenu = ('*' 공통 ∪ contentType 전용) 병합, group(content→slot-ops)·order 로 결정적 정렬
//      (등록 순서 무관).
//   2. group 경계에 separatorBefore=true(콘텐츠→공통 구분선).
//   3. 미등록 commandId 기여 → fail-loud but crash-free(console.error + skip, throw 안 함 — FIX-1).
//   4. registerSlotMenu 중복 (target, commandId) → warn 후 교체(마지막이 이김, HMR-safe).
//   5. resolve 는 registry title/run 을 그대로 싣는다.
//   6. commandId dedupe('*' ∩ contentType) — 최종 순서 첫 등장만(FIX-2).
//   7. validateSlotMenuContributions — 미등록 기여마다 console.error(부팅 전수 검증, FIX-1).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { __resetRegistryForTest, register } from './registry'
import {
  __resetSlotMenuForTest,
  buildSlotMenu,
  registerSlotMenu,
  validateSlotMenuContributions,
} from './slotMenu'

beforeEach(() => {
  __resetRegistryForTest()
  __resetSlotMenuForTest()
})
afterEach(() => {
  vi.restoreAllMocks()
})

/** 테스트용 no-op command 등록 헬퍼. */
function reg(id: string, title = id): void {
  register({ id, title, run: vi.fn() })
}

describe('buildSlotMenu — 병합 + 결정적 정렬', () => {
  it("'*' 공통 + contentType 전용을 합쳐 content 그룹이 slot-ops 그룹보다 먼저 온다", () => {
    reg('common.a')
    reg('content.x')
    registerSlotMenu('*', [{ commandId: 'common.a', group: 'slot-ops', order: 10 }])
    registerSlotMenu('empty', [{ commandId: 'content.x', group: 'content', order: 10 }])
    const items = buildSlotMenu('empty')
    expect(items.map(i => i.id)).toEqual(['content.x', 'common.a']) // content 먼저, slot-ops 나중
  })

  it('등록 순서와 무관하게 (group, order) 로만 정렬된다', () => {
    reg('c1'); reg('c2'); reg('s1'); reg('s2')
    // 일부러 뒤섞어 등록.
    registerSlotMenu('*', [
      { commandId: 's2', group: 'slot-ops', order: 20 },
      { commandId: 's1', group: 'slot-ops', order: 10 },
    ])
    registerSlotMenu('empty', [
      { commandId: 'c2', group: 'content', order: 20 },
      { commandId: 'c1', group: 'content', order: 10 },
    ])
    expect(buildSlotMenu('empty').map(i => i.id)).toEqual(['c1', 'c2', 's1', 's2'])
  })

  it('그룹 경계(content→slot-ops)에서 첫 slot-ops 항목만 separatorBefore=true', () => {
    reg('c1'); reg('s1'); reg('s2')
    registerSlotMenu('empty', [{ commandId: 'c1', group: 'content', order: 10 }])
    registerSlotMenu('*', [
      { commandId: 's1', group: 'slot-ops', order: 10 },
      { commandId: 's2', group: 'slot-ops', order: 20 },
    ])
    const items = buildSlotMenu('empty')
    expect(items.map(i => i.separatorBefore)).toEqual([false, true, false])
  })

  it('공통 항목만 있는 콘텐츠(전용 기여 없음)도 공통을 그대로 보여준다', () => {
    reg('s1'); reg('s2')
    registerSlotMenu('*', [
      { commandId: 's1', group: 'slot-ops', order: 10 },
      { commandId: 's2', group: 'slot-ops', order: 20 },
    ])
    // preset_palette 전용 기여가 없어도 '*' 만으로 조립된다(첫 항목 separator 없음).
    const items = buildSlotMenu('preset_palette')
    expect(items.map(i => i.id)).toEqual(['s1', 's2'])
    expect(items[0].separatorBefore).toBe(false)
  })

  it('resolve 는 registry 의 title/run 을 그대로 싣는다', () => {
    const runSpy = vi.fn(() => 'ran')
    register({ id: 'x', title: '엑스', run: runSpy })
    registerSlotMenu('*', [{ commandId: 'x', group: 'slot-ops', order: 10 }])
    const [item] = buildSlotMenu('empty')
    expect(item.title).toBe('엑스')
    expect(item.run({ a: 1 })).toBe('ran')
    expect(runSpy).toHaveBeenCalledWith({ a: 1 })
  })
})

describe('fail-loud but crash-free + HMR-safe (FIX-1)', () => {
  it('미등록 commandId 기여 → buildSlotMenu 가 throw 하지 않고 console.error + skip(렌더 살림)', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    reg('ok')
    registerSlotMenu('*', [
      { commandId: 'not.registered', group: 'slot-ops', order: 10 },
      { commandId: 'ok', group: 'slot-ops', order: 20 },
    ])
    let items!: ReturnType<typeof buildSlotMenu>
    expect(() => { items = buildSlotMenu('empty') }).not.toThrow() // 크래시 금지
    expect(items.map(i => i.id)).toEqual(['ok']) // 미등록은 빠지고 유효 항목만
    expect(error).toHaveBeenCalledWith(
      expect.stringMatching(/unregistered commandId "not\.registered" \(target=empty\) — skipped/),
    )
  })

  it('중복 (target, commandId) 재기여 → warn 후 교체(마지막이 이김)', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    reg('x')
    registerSlotMenu('*', [{ commandId: 'x', group: 'slot-ops', order: 10 }])
    registerSlotMenu('*', [{ commandId: 'x', group: 'slot-ops', order: 99 }]) // 교체
    expect(warn).toHaveBeenCalled()
    const items = buildSlotMenu('empty')
    expect(items).toHaveLength(1) // 중복이 아니라 교체 — 1개만
  })
})

describe('commandId dedupe — \'*\' ∩ contentType (FIX-2)', () => {
  it('공통(\'*\')과 콘텐츠 전용이 같은 commandId 를 기여해도 렌더 목록은 1회만(중복 key 방지)', () => {
    reg('shared'); reg('s1')
    // '*'(slot-ops)와 콘텐츠(content)가 같은 commandId 'shared' 를 기여.
    registerSlotMenu('*', [
      { commandId: 'shared', group: 'slot-ops', order: 10 },
      { commandId: 's1', group: 'slot-ops', order: 20 },
    ])
    registerSlotMenu('empty', [{ commandId: 'shared', group: 'content', order: 10 }])
    const ids = buildSlotMenu('empty').map(i => i.id)
    expect(ids).toEqual(['shared', 's1']) // content 쪽(먼저 정렬)이 이겨 1회만, s1 은 유지
    expect(new Set(ids).size).toBe(ids.length) // 고유 commandId 보장
  })

  it('같은 target 안에서 같은 group·order 로 중복 등장해도 첫 등장만(방어적 dedupe)', () => {
    reg('dup')
    // registerSlotMenu 는 같은 (target, commandId) 를 교체하므로, 서로 다른 배열 호출로 중복을 만든다.
    // (여기선 '*' 와 콘텐츠에 각각 등록해 dedupe 자체를 확인 — 위 케이스의 최소형.)
    registerSlotMenu('*', [{ commandId: 'dup', group: 'slot-ops', order: 10 }])
    registerSlotMenu('empty', [{ commandId: 'dup', group: 'slot-ops', order: 10 }])
    expect(buildSlotMenu('empty').map(i => i.id)).toEqual(['dup'])
  })
})

describe('validateSlotMenuContributions — 부팅 전수 검증 (FIX-1)', () => {
  it('미등록 기여마다 console.error(우클릭 없이 부팅 즉시 발각)', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    reg('ok')
    registerSlotMenu('*', [{ commandId: 'ok', group: 'slot-ops', order: 10 }])
    registerSlotMenu('empty', [{ commandId: 'typo.missing', group: 'content', order: 10 }])
    validateSlotMenuContributions()
    expect(error).toHaveBeenCalledTimes(1)
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/unregistered commandId "typo\.missing"/))
  })

  it('모든 기여가 등록돼 있으면 조용하다(에러 0)', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    reg('a'); reg('b')
    registerSlotMenu('*', [{ commandId: 'a', group: 'slot-ops', order: 10 }])
    registerSlotMenu('empty', [{ commandId: 'b', group: 'content', order: 10 }])
    validateSlotMenuContributions()
    expect(error).not.toHaveBeenCalled()
  })
})
