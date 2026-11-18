
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { setLanguage, t } from '../i18n'
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

function reg(id: string, title = id): void {
  register({ id, title: () => title, run: vi.fn() })
}

describe('buildSlotMenu — merge + deterministic sort', () => {
  it("merging '*' common + contentType-specific puts the content group before the slot-ops group", () => {
    reg('common.a')
    reg('content.x')
    registerSlotMenu('*', [{ commandId: 'common.a', group: 'slot-ops', order: 10 }])
    registerSlotMenu('empty', [{ commandId: 'content.x', group: 'content', order: 10 }])
    const items = buildSlotMenu('empty')
    expect(items.map(i => i.id)).toEqual(['content.x', 'common.a'])
  })

  it('sorted only by (group, order), regardless of registration order', () => {
    reg('c1'); reg('c2'); reg('s1'); reg('s2')
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

  it('only the first slot-ops item at the group boundary (content→slot-ops) gets separatorBefore=true', () => {
    reg('c1'); reg('s1'); reg('s2')
    registerSlotMenu('empty', [{ commandId: 'c1', group: 'content', order: 10 }])
    registerSlotMenu('*', [
      { commandId: 's1', group: 'slot-ops', order: 10 },
      { commandId: 's2', group: 'slot-ops', order: 20 },
    ])
    const items = buildSlotMenu('empty')
    expect(items.map(i => i.separatorBefore)).toEqual([false, true, false])
  })

  it('content with only common items (no specific contributions) shows the common ones as is', () => {
    reg('s1'); reg('s2')
    registerSlotMenu('*', [
      { commandId: 's1', group: 'slot-ops', order: 10 },
      { commandId: 's2', group: 'slot-ops', order: 20 },
    ])
    const items = buildSlotMenu('preset_palette')
    expect(items.map(i => i.id)).toEqual(['s1', 's2'])
    expect(items[0].separatorBefore).toBe(false)
  })

  it('resolve carries the registry title/run as is', () => {
    const runSpy = vi.fn(() => 'ran')
    register({ id: 'x', title: () => 'ex', run: runSpy })
    registerSlotMenu('*', [{ commandId: 'x', group: 'slot-ops', order: 10 }])
    const [item] = buildSlotMenu('empty')
    expect(item.title).toBe('ex')
    expect(item.run({ a: 1 })).toBe('ran')
    expect(runSpy).toHaveBeenCalledWith({ a: 1 })
  })
})

describe('fail-loud but crash-free + HMR-safe (FIX-1)', () => {
  it('an unregistered commandId contribution → buildSlotMenu does not throw, console.error + skip (keeps rendering)', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    reg('ok')
    registerSlotMenu('*', [
      { commandId: 'not.registered', group: 'slot-ops', order: 10 },
      { commandId: 'ok', group: 'slot-ops', order: 20 },
    ])
    let items!: ReturnType<typeof buildSlotMenu>
    expect(() => { items = buildSlotMenu('empty') }).not.toThrow()
    expect(items.map(i => i.id)).toEqual(['ok'])
    expect(error).toHaveBeenCalledWith(
      expect.stringMatching(/unregistered commandId "not\.registered" \(target=empty\) — skipped/),
    )
  })

  it('duplicate (target, commandId) re-contribution → warn, then replaced (the last one wins)', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    reg('x')
    registerSlotMenu('*', [{ commandId: 'x', group: 'slot-ops', order: 10 }])
    registerSlotMenu('*', [{ commandId: 'x', group: 'slot-ops', order: 99 }])
    expect(warn).toHaveBeenCalled()
    const items = buildSlotMenu('empty')
    expect(items).toHaveLength(1)
  })
})

describe('commandId dedupe — \'*\' ∩ contentType (FIX-2)', () => {
  it('even when common (\'*\') and content-specific contribute the same commandId, the render list has it once (prevents duplicate keys)', () => {
    reg('shared'); reg('s1')
    registerSlotMenu('*', [
      { commandId: 'shared', group: 'slot-ops', order: 10 },
      { commandId: 's1', group: 'slot-ops', order: 20 },
    ])
    registerSlotMenu('empty', [{ commandId: 'shared', group: 'content', order: 10 }])
    const ids = buildSlotMenu('empty').map(i => i.id)
    expect(ids).toEqual(['shared', 's1'])
    expect(new Set(ids).size).toBe(ids.length)
  })

  it('even when the same group·order appears twice within one target, only the first appearance (defensive dedupe)', () => {
    reg('dup')
    // registerSlotMenu replaces the same (target, commandId), so the duplicate is made with separate array calls.
    registerSlotMenu('*', [{ commandId: 'dup', group: 'slot-ops', order: 10 }])
    registerSlotMenu('empty', [{ commandId: 'dup', group: 'slot-ops', order: 10 }])
    expect(buildSlotMenu('empty').map(i => i.id)).toEqual(['dup'])
  })
})

describe('hideOn exclusion condition (ADR-0065)', () => {
  it("an item with hideOn:['empty'] drops out of contentType 'empty' and stays on other types", () => {
    reg('s.split'); reg('s.empty'); reg('s.popout')
    registerSlotMenu('*', [
      { commandId: 's.split', group: 'slot-ops', order: 10 },
      { commandId: 's.popout', group: 'slot-ops', order: 30, hideOn: ['empty'] },
      { commandId: 's.empty', group: 'slot-ops', order: 40, hideOn: ['empty'] },
    ])
    expect(buildSlotMenu('empty').map(i => i.id)).toEqual(['s.split'])
    expect(buildSlotMenu('agent').map(i => i.id)).toEqual(['s.split', 's.popout', 's.empty'])
  })

  it('hideOn unspecified (existing contributions) has no effect — visible on every type (backward compatibility)', () => {
    reg('a')
    registerSlotMenu('*', [{ commandId: 'a', group: 'slot-ops', order: 10 }])
    expect(buildSlotMenu('empty').map(i => i.id)).toEqual(['a'])
    expect(buildSlotMenu('agent').map(i => i.id)).toEqual(['a'])
  })

  it("FIX-2: hideOn applies to submenu children too — a hideOn:['empty'] child drops out on empty and stays on other types", () => {
    reg('fill.keep'); reg('fill.hidden')
    registerSlotMenu('empty', [
      {
        id: 'newContent', title: () => 'New content',
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
        id: 'newContent', title: () => 'New content',
        group: 'content',
        order: 10,
        children: [
          { commandId: 'fill.keep', group: 'content', order: 10 },
          { commandId: 'fill.hidden', group: 'content', order: 20, hideOn: ['empty'] },
        ],
      },
    ])
    expect(buildSlotMenu('empty')[0].children!.map(c => c.id)).toEqual(['fill.keep'])
    expect(buildSlotMenu('agent')[0].children!.map(c => c.id)).toEqual(['fill.keep', 'fill.hidden'])
  })

  it('FIX-1×FIX-2: when hideOn removes every child, the whole container is omitted', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    reg('fill.only')
    registerSlotMenu('empty', [
      {
        id: 'newContent', title: () => 'New content',
        group: 'content',
        order: 10,
        children: [{ commandId: 'fill.only', group: 'content', order: 10, hideOn: ['empty'] }],
      },
    ])
    expect(buildSlotMenu('empty')).toHaveLength(0)
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/all children were skipped, leaving an empty container/))
  })
})

describe('children one-level submenu (ADR-0065)', () => {
  it('a container resolves its title + resolves each child commandId with the registry title/run', () => {
    const r1 = vi.fn(() => 'r1'); const r2 = vi.fn(() => 'r2')
    register({ id: 'fill.a', title: () => 'Tree', run: r1 })
    register({ id: 'fill.b', title: () => 'Palette', run: r2 })
    registerSlotMenu('empty', [
      {
        id: 'newContent', title: () => 'New content',
        group: 'content',
        order: 10,
        children: [
          { commandId: 'fill.a', group: 'content', order: 10 },
          { commandId: 'fill.b', group: 'content', order: 20 },
        ],
      },
    ])
    const [container] = buildSlotMenu('empty')
    expect(container.title).toBe('New content')
    expect(container.id).toBe('container:newContent')
    expect(container.children).toBeDefined()
    expect(container.children!.map(c => c.id)).toEqual(['fill.a', 'fill.b'])
    expect(container.children!.map(c => c.title)).toEqual(['Tree', 'Palette'])
    expect(container.children![0].run()).toBe('r1')
    expect(r1).toHaveBeenCalled()
  })

  it('container children keep the declaration order (relative order) as is (ADR-0065 — no re-sorting, unlike the top level)', () => {
    reg('fill.a'); reg('fill.b'); reg('fill.c')
    registerSlotMenu('empty', [
      {
        id: 'newContent', title: () => 'New content',
        group: 'content',
        order: 10,
        children: [
          { commandId: 'fill.c', group: 'content', order: 30 },
          { commandId: 'fill.a', group: 'content', order: 10 },
          { commandId: 'fill.b', group: 'content', order: 20 },
        ],
      },
    ])
    expect(buildSlotMenu('empty')[0].children!.map(c => c.id)).toEqual(['fill.c', 'fill.a', 'fill.b'])
  })

  it('FIX-1: when every child is skipped, the container is not emitted as a leaf but omitted entirely', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    reg('s1')
    registerSlotMenu('empty', [
      {
        id: 'newContent', title: () => 'New content',
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
    expect(items.map(i => i.id)).toEqual(['s1'])
    expect(items.some(i => i.id === 'container:newContent')).toBe(false)
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/all children were skipped, leaving an empty container/))
  })

  it('with containers + leaves mixed, a leaf stays as is without children (backward compatibility)', () => {
    reg('fill.a'); reg('s1')
    registerSlotMenu('empty', [
      { id: 'newContent', title: () => 'New content', group: 'content', order: 10, children: [{ commandId: 'fill.a', group: 'content', order: 10 }] },
    ])
    registerSlotMenu('*', [{ commandId: 's1', group: 'slot-ops', order: 10 }])
    const items = buildSlotMenu('empty')
    expect(items.map(i => i.id)).toEqual(['container:newContent', 's1'])
    expect(items[0].children).toBeDefined()
    expect(items[1].children).toBeUndefined()
    expect(items[1].separatorBefore).toBe(true)
  })
})

describe('shape validation — execution item XOR container (ADR-0065, fail-loud but crash-free)', () => {
  it('2-level nesting (children inside children) → console.error + that child skipped', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    reg('ok.child'); reg('deep')
    registerSlotMenu('empty', [
      {
        id: 'newContent', title: () => 'New content',
        group: 'content',
        order: 10,
        children: [
          { commandId: 'ok.child', group: 'content', order: 10 },
          { id: 'deeper', title: () => 'Deeper', group: 'content', order: 20, children: [{ commandId: 'deep', group: 'content', order: 10 }] },
        ],
      },
    ])
    let items!: ReturnType<typeof buildSlotMenu>
    expect(() => { items = buildSlotMenu('empty') }).not.toThrow()
    expect(items[0].children!.map(c => c.id)).toEqual(['ok.child'])
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/nesting of 2\+ levels not allowed/))
  })

  it('a container that also has a commandId (both) → console.error + skip', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    reg('both'); reg('kid')
    registerSlotMenu('empty', [
      { commandId: 'both', title: () => 'Both', group: 'content', order: 10, children: [{ commandId: 'kid', group: 'content', order: 10 }] },
    ])
    let items!: ReturnType<typeof buildSlotMenu>
    expect(() => { items = buildSlotMenu('empty') }).not.toThrow()
    expect(items).toHaveLength(0)
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/cannot have both commandId and children/))
  })

  it('neither commandId nor children (both absent) → console.error + skip', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    registerSlotMenu('empty', [{ title: () => 'Ghost', group: 'content', order: 10 }])
    let items!: ReturnType<typeof buildSlotMenu>
    expect(() => { items = buildSlotMenu('empty') }).not.toThrow()
    expect(items).toHaveLength(0)
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/has neither commandId nor children/))
  })

  it('a container without a title → console.error + skip', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    reg('kid')
    registerSlotMenu('empty', [{ id: 'c', group: 'content', order: 10, children: [{ commandId: 'kid', group: 'content', order: 10 }] }])
    let items!: ReturnType<typeof buildSlotMenu>
    expect(() => { items = buildSlotMenu('empty') }).not.toThrow()
    expect(items).toHaveLength(0)
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/needs a title function/))
  })

  // ADR-0373: a title kept as text would stay in the language it was made in.
  it('a container whose title is text, not a function (a cast or a JS caller), is skipped', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    reg('kid')
    registerSlotMenu('empty', [
      {
        id: 'c',
        title: 'text' as unknown as () => string,
        group: 'content',
        order: 10,
        children: [{ commandId: 'kid', group: 'content', order: 10 }],
      },
    ])
    expect(buildSlotMenu('empty')).toHaveLength(0)
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/needs a title function/))
  })

  it('a container without an id is skipped', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    reg('kid')
    registerSlotMenu('empty', [
      { title: () => 'Bundle', group: 'content', order: 10, children: [{ commandId: 'kid', group: 'content', order: 10 }] },
    ])
    expect(buildSlotMenu('empty')).toHaveLength(0)
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/needs an id/))
  })
})

describe('checked · enabled — toggle items and disabled items (ADR-0252)', () => {
  it('carries the contribution checked · enabled as the same functions — both top level and submenu children', () => {
    reg('top'); reg('kid'); reg('plain')
    const topChecked = () => true
    const topEnabled = () => false
    const kidChecked = () => false
    const kidEnabled = () => true
    registerSlotMenu('usage', [
      { commandId: 'top', group: 'content', order: 10, checked: topChecked, enabled: topEnabled },
      { commandId: 'plain', group: 'content', order: 20 },
      {
        id: 'bundle', title: () => 'Bundle',
        group: 'content',
        order: 30,
        children: [{ commandId: 'kid', group: 'content', order: 10, checked: kidChecked, enabled: kidEnabled }],
      },
    ])
    const items = buildSlotMenu('usage')
    expect([items[0].checked, items[0].enabled]).toEqual([topChecked, topEnabled])
    expect('checked' in items[1] || 'enabled' in items[1]).toBe(false)
    expect([items[2].children?.[0].checked, items[2].children?.[0].enabled]).toEqual([kidChecked, kidEnabled])
  })

  it("the command's when does not come over to the menu — only the contribution's enabled decides the menu's enabled state", () => {
    register({ id: 'gated', title: () => 'gated', when: () => false, run: vi.fn() })
    registerSlotMenu('usage', [{ commandId: 'gated', group: 'content', order: 10 }])
    const [gated] = buildSlotMenu('usage')
    expect('when' in gated).toBe(false)
    expect(gated.enabled).toBeUndefined()
  })

  it.each(['checked', 'enabled'] as const)('%s on a container → console.error + skip (both at render and at the boot check)', field => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    reg('kid')
    registerSlotMenu('usage', [
      { id: 'bundle', title: () => 'Bundle', group: 'content', order: 10, [field]: () => true, children: [{ commandId: 'kid', group: 'content', order: 10 }] },
    ])
    expect(buildSlotMenu('usage')).toHaveLength(0)
    validateSlotMenuContributions()
    expect(error).toHaveBeenCalledTimes(2)
    expect(error).toHaveBeenCalledWith(expect.stringMatching(new RegExp(`a container cannot have ${field}`)))
  })
})

describe('backward compatibility — existing flat contributions unaffected (ADR-0065)', () => {
  it('an existing contribution with only {commandId, group, order} resolves the same as before', () => {
    reg('c1'); reg('s1')
    registerSlotMenu('empty', [{ commandId: 'c1', group: 'content', order: 10 }])
    registerSlotMenu('*', [{ commandId: 's1', group: 'slot-ops', order: 10 }])
    const items = buildSlotMenu('empty')
    expect(items.map(i => i.id)).toEqual(['c1', 's1'])
    expect(items.every(i => i.children === undefined)).toBe(true)
    expect(items.map(i => i.separatorBefore)).toEqual([false, true])
  })
})

describe('validateSlotMenuContributions — full check at boot (FIX-1)', () => {
  it('console.error for each unregistered contribution (caught right at boot, without a right-click)', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    reg('ok')
    registerSlotMenu('*', [{ commandId: 'ok', group: 'slot-ops', order: 10 }])
    registerSlotMenu('empty', [{ commandId: 'typo.missing', group: 'content', order: 10 }])
    validateSlotMenuContributions()
    expect(error).toHaveBeenCalledTimes(1)
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/unregistered commandId "typo\.missing"/))
  })

  it('quiet when every contribution is registered (0 errors)', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    reg('a'); reg('b')
    registerSlotMenu('*', [{ commandId: 'a', group: 'slot-ops', order: 10 }])
    registerSlotMenu('empty', [{ commandId: 'b', group: 'content', order: 10 }])
    validateSlotMenuContributions()
    expect(error).not.toHaveBeenCalled()
  })

  it('FIX-3: the full boot check catches a shape violation (both-cmd-and-children) without a right-click', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    reg('both'); reg('kid')
    registerSlotMenu('empty', [
      { commandId: 'both', title: () => 'Both', group: 'content', order: 10, children: [{ commandId: 'kid', group: 'content', order: 10 }] },
    ])
    validateSlotMenuContributions()
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/cannot have both commandId and children/))
  })

  it('FIX-3: catches 2-level nesting and unregistered commandIds of container children recursively', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    reg('ok.child')
    registerSlotMenu('empty', [
      {
        id: 'newContent', title: () => 'New content',
        group: 'content',
        order: 10,
        children: [
          { commandId: 'ok.child', group: 'content', order: 10 },
          { id: 'deeper', title: () => 'Deeper', group: 'content', order: 20, children: [{ commandId: 'x', group: 'content', order: 10 }] },
          { commandId: 'typo.child', group: 'content', order: 30 },
        ],
      },
    ])
    validateSlotMenuContributions()
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/nesting of 2\+ levels not allowed/))
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/unregistered commandId "typo\.child"/))
  })
})

// ADR-0373: a container's identity is its id — the key, the resolved id and the sort tie-break do not change with the title's language.
describe('container identity across a language switch', () => {
  afterEach(() => {
    setLanguage('ko')
  })

  it('two containers tied on (group, order) keep their ids and order; only the titles change', () => {
    reg('kid.a'); reg('kid.b')
    // In Korean 「새 콘텐츠」 sorts before 「에이전트 생성」, in English "Create agent" before "New content" — a title // hangul-ok
    //   tie-break would flip the order.
    registerSlotMenu('empty', [
      { id: 'b.new', title: () => t('slot.newContent'), group: 'content', order: 10, children: [{ commandId: 'kid.a', group: 'content', order: 10 }] },
      { id: 'a.create', title: () => t('agent.create'), group: 'content', order: 10, children: [{ commandId: 'kid.b', group: 'content', order: 10 }] },
    ])

    const ko = buildSlotMenu('empty')
    setLanguage('en')
    const en = buildSlotMenu('empty')

    expect(ko.map(i => i.id)).toEqual(['container:a.create', 'container:b.new'])
    expect(en.map(i => i.id)).toEqual(['container:a.create', 'container:b.new'])
    expect(ko.map(i => i.title)).toEqual(['에이전트 생성', '새 콘텐츠'])
    expect(en.map(i => i.title)).toEqual(['Create agent', 'New content'])
  })

  it('re-registering a container with the same id replaces it, whatever its title', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    reg('kid')
    const container = { id: 'c', group: 'content', order: 10, children: [{ commandId: 'kid', group: 'content', order: 10 }] }
    registerSlotMenu('empty', [{ ...container, title: () => t('slot.newContent') }])
    setLanguage('en')
    registerSlotMenu('empty', [{ ...container, title: () => t('slot.newContent') }])

    expect(buildSlotMenu('empty')).toHaveLength(1)
    expect(warn).toHaveBeenCalledTimes(1)
  })
})
