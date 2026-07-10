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

describe('hideOn 제외 조건 (ADR-0065)', () => {
  it("hideOn:['empty'] 항목은 contentType 'empty' 에서 빠지고, 다른 타입엔 그대로 남는다", () => {
    reg('s.split'); reg('s.empty'); reg('s.popout')
    registerSlotMenu('*', [
      { commandId: 's.split', group: 'slot-ops', order: 10 },
      { commandId: 's.popout', group: 'slot-ops', order: 30, hideOn: ['empty'] },
      { commandId: 's.empty', group: 'slot-ops', order: 40, hideOn: ['empty'] },
    ])
    // empty: hideOn 항목 제외.
    expect(buildSlotMenu('empty').map(i => i.id)).toEqual(['s.split'])
    // 비-empty(agent): 전부 남는다(subtraction 전용 — '*' 보편성 유지).
    expect(buildSlotMenu('agent').map(i => i.id)).toEqual(['s.split', 's.popout', 's.empty'])
  })

  it('hideOn 미지정(기존 기여) 은 무영향 — 모든 타입에서 보인다(하위호환)', () => {
    reg('a')
    registerSlotMenu('*', [{ commandId: 'a', group: 'slot-ops', order: 10 }])
    expect(buildSlotMenu('empty').map(i => i.id)).toEqual(['a'])
    expect(buildSlotMenu('agent').map(i => i.id)).toEqual(['a'])
  })

  it("FIX-2: hideOn 은 서브메뉴 자식에도 적용 — hideOn:['empty'] 자식은 empty 에서 빠지고 다른 타입엔 남는다", () => {
    reg('fill.keep'); reg('fill.hidden')
    registerSlotMenu('empty', [
      {
        title: '새 콘텐츠',
        group: 'content',
        order: 10,
        children: [
          { commandId: 'fill.keep', group: 'content', order: 10 },
          { commandId: 'fill.hidden', group: 'content', order: 20, hideOn: ['empty'] },
        ],
      },
    ])
    registerSlotMenu('agent', [
      {
        title: '새 콘텐츠',
        group: 'content',
        order: 10,
        children: [
          { commandId: 'fill.keep', group: 'content', order: 10 },
          { commandId: 'fill.hidden', group: 'content', order: 20, hideOn: ['empty'] },
        ],
      },
    ])
    // empty: hideOn:['empty'] 자식은 서브메뉴에서 빠진다(가시성 계약 = 자식 포함).
    expect(buildSlotMenu('empty')[0].children!.map(c => c.id)).toEqual(['fill.keep'])
    // agent(비-empty): 자식 그대로 유지.
    expect(buildSlotMenu('agent')[0].children!.map(c => c.id)).toEqual(['fill.keep', 'fill.hidden'])
  })

  it('FIX-1×FIX-2: hideOn 이 자식을 전부 걷어내면 컨테이너째 omit 된다', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    reg('fill.only')
    registerSlotMenu('empty', [
      {
        title: '새 콘텐츠',
        group: 'content',
        order: 10,
        // 유일한 자식이 hideOn:['empty'] → empty 에서 자식 0 → FIX-1 로 컨테이너 omit.
        children: [{ commandId: 'fill.only', group: 'content', order: 10, hideOn: ['empty'] }],
      },
    ])
    expect(buildSlotMenu('empty')).toHaveLength(0)
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/자식이 모두 skip 되어 빈 컨테이너/))
  })
})

describe('children 1단 서브메뉴 (ADR-0065)', () => {
  it('컨테이너는 title passthrough + 각 자식 commandId 를 registry title/run 으로 resolve 한다', () => {
    const r1 = vi.fn(() => 'r1'); const r2 = vi.fn(() => 'r2')
    register({ id: 'fill.a', title: '트리', run: r1 })
    register({ id: 'fill.b', title: '팔레트', run: r2 })
    registerSlotMenu('empty', [
      {
        title: '새 콘텐츠',
        group: 'content',
        order: 10,
        children: [
          { commandId: 'fill.a', group: 'content', order: 10 },
          { commandId: 'fill.b', group: 'content', order: 20 },
        ],
      },
    ])
    const [container] = buildSlotMenu('empty')
    expect(container.title).toBe('새 콘텐츠')
    expect(container.id).toBe('container:새 콘텐츠')
    expect(container.children).toBeDefined()
    expect(container.children!.map(c => c.id)).toEqual(['fill.a', 'fill.b'])
    expect(container.children!.map(c => c.title)).toEqual(['트리', '팔레트'])
    expect(container.children![0].run()).toBe('r1')
    expect(r1).toHaveBeenCalled()
  })

  it('컨테이너 자식은 선언 순서(relative order)를 그대로 보존한다(ADR-0065 — 최상위와 달리 재정렬 없음)', () => {
    reg('fill.a'); reg('fill.b'); reg('fill.c')
    registerSlotMenu('empty', [
      {
        title: '새 콘텐츠',
        group: 'content',
        order: 10,
        children: [
          { commandId: 'fill.c', group: 'content', order: 30 },
          { commandId: 'fill.a', group: 'content', order: 10 },
          { commandId: 'fill.b', group: 'content', order: 20 },
        ],
      },
    ])
    // 자식은 group/order 로 재정렬하지 않고 선언 순서(c, a, b) 그대로 — 기여가 이미 의도 순서로 나열한다.
    expect(buildSlotMenu('empty')[0].children!.map(c => c.id)).toEqual(['fill.c', 'fill.a', 'fill.b'])
  })

  it('FIX-1: 자식이 전부 skip 되면 컨테이너를 leaf 로 emit 하지 않고 통째로 omit 한다', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    reg('s1')
    // 컨테이너 자식이 전부 미등록 commandId → 자식이 모두 skip → 빈 컨테이너.
    registerSlotMenu('empty', [
      {
        title: '새 콘텐츠',
        group: 'content',
        order: 10,
        children: [
          { commandId: 'nope.a', group: 'content', order: 10 },
          { commandId: 'nope.b', group: 'content', order: 20 },
        ],
      },
    ])
    registerSlotMenu('*', [{ commandId: 's1', group: 'slot-ops', order: 10 }])
    let items!: ReturnType<typeof buildSlotMenu>
    expect(() => { items = buildSlotMenu('empty') }).not.toThrow()
    // 컨테이너는 통째로 빠지고(container:새 콘텐츠 없음, children:[] leaf 로도 안 남음) 나머지만.
    expect(items.map(i => i.id)).toEqual(['s1'])
    expect(items.some(i => i.id === 'container:새 콘텐츠')).toBe(false)
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/자식이 모두 skip 되어 빈 컨테이너/))
  })

  it('컨테이너 + leaf 혼재 시 leaf 는 children 없이 그대로(하위호환)', () => {
    reg('fill.a'); reg('s1')
    registerSlotMenu('empty', [
      { title: '새 콘텐츠', group: 'content', order: 10, children: [{ commandId: 'fill.a', group: 'content', order: 10 }] },
    ])
    registerSlotMenu('*', [{ commandId: 's1', group: 'slot-ops', order: 10 }])
    const items = buildSlotMenu('empty')
    expect(items.map(i => i.id)).toEqual(['container:새 콘텐츠', 's1'])
    expect(items[0].children).toBeDefined()
    expect(items[1].children).toBeUndefined() // leaf 는 children 없음
    expect(items[1].separatorBefore).toBe(true) // content → slot-ops 경계
  })
})

describe('형태 검증 — 실행 항목 XOR 컨테이너 (ADR-0065, fail-loud but crash-free)', () => {
  it('2단 중첩(children 안의 children) → console.error + 그 자식 skip', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    reg('ok.child'); reg('deep')
    registerSlotMenu('empty', [
      {
        title: '새 콘텐츠',
        group: 'content',
        order: 10,
        children: [
          { commandId: 'ok.child', group: 'content', order: 10 },
          // 2단 중첩 — 금지. 자식이 또 children 을 가진다.
          { title: '더깊이', group: 'content', order: 20, children: [{ commandId: 'deep', group: 'content', order: 10 }] },
        ],
      },
    ])
    let items!: ReturnType<typeof buildSlotMenu>
    expect(() => { items = buildSlotMenu('empty') }).not.toThrow() // crash-free
    expect(items[0].children!.map(c => c.id)).toEqual(['ok.child']) // 중첩 자식만 skip, 정상 자식 유지
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/2단\+ 중첩 금지/))
  })

  it('컨테이너인데 commandId 도 있음(둘 다) → console.error + skip', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    reg('both'); reg('kid')
    registerSlotMenu('empty', [
      { commandId: 'both', title: '둘다', group: 'content', order: 10, children: [{ commandId: 'kid', group: 'content', order: 10 }] },
    ])
    let items!: ReturnType<typeof buildSlotMenu>
    expect(() => { items = buildSlotMenu('empty') }).not.toThrow()
    expect(items).toHaveLength(0) // 형태 위반 → skip
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/commandId 와 children 을 동시에/))
  })

  it('commandId 도 children 도 없음(둘 다 없음) → console.error + skip', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    registerSlotMenu('empty', [{ title: '유령', group: 'content', order: 10 }])
    let items!: ReturnType<typeof buildSlotMenu>
    expect(() => { items = buildSlotMenu('empty') }).not.toThrow()
    expect(items).toHaveLength(0)
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/commandId 도 children 도 없음/))
  })

  it('컨테이너인데 title 없음 → console.error + skip', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    reg('kid')
    registerSlotMenu('empty', [{ group: 'content', order: 10, children: [{ commandId: 'kid', group: 'content', order: 10 }] }])
    let items!: ReturnType<typeof buildSlotMenu>
    expect(() => { items = buildSlotMenu('empty') }).not.toThrow()
    expect(items).toHaveLength(0)
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/컨테이너는 title 필수/))
  })
})

describe('하위호환 — 기존 flat 기여 무영향 (ADR-0065)', () => {
  it('{commandId, group, order} 만 있는 기존 기여는 이전과 동일하게 resolve 된다', () => {
    reg('c1'); reg('s1')
    registerSlotMenu('empty', [{ commandId: 'c1', group: 'content', order: 10 }])
    registerSlotMenu('*', [{ commandId: 's1', group: 'slot-ops', order: 10 }])
    const items = buildSlotMenu('empty')
    expect(items.map(i => i.id)).toEqual(['c1', 's1'])
    expect(items.every(i => i.children === undefined)).toBe(true)
    expect(items.map(i => i.separatorBefore)).toEqual([false, true])
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

  it('FIX-3: 형태 위반(both-cmd-and-children)을 부팅 전수 검증이 우클릭 없이 발각한다', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    reg('both'); reg('kid')
    // commandId 는 등록돼 있지만 children 도 동시에 가진 형태 위반 — 옛 sweep 은 못 잡았다.
    registerSlotMenu('empty', [
      { commandId: 'both', title: '둘다', group: 'content', order: 10, children: [{ commandId: 'kid', group: 'content', order: 10 }] },
    ])
    validateSlotMenuContributions()
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/commandId 와 children 을 동시에/))
  })

  it('FIX-3: 컨테이너 자식의 2단 중첩·commandId 미등록을 재귀로 발각한다', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    reg('ok.child')
    registerSlotMenu('empty', [
      {
        title: '새 콘텐츠',
        group: 'content',
        order: 10,
        children: [
          { commandId: 'ok.child', group: 'content', order: 10 },
          // 2단 중첩(자식이 또 children) — 형태 위반, 부팅에 발각돼야 한다.
          { title: '더깊이', group: 'content', order: 20, children: [{ commandId: 'x', group: 'content', order: 10 }] },
          // 미등록 commandId 자식 — commandId 존재 검증에 발각.
          { commandId: 'typo.child', group: 'content', order: 30 },
        ],
      },
    ])
    validateSlotMenuContributions()
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/2단\+ 중첩 금지/))
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/unregistered commandId "typo\.child"/))
  })
})
