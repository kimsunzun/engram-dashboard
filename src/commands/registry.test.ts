
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { __resetRegistryForTest, getCommand, list, register, run } from './registry'

beforeEach(() => {
  __resetRegistryForTest()
})

afterEach(() => {
  vi.restoreAllMocks()
})

describe('command registry (ADR-0055)', () => {
  it('register + run: handler 를 호출하고 반환을 그대로 흘려보낸다', () => {
    register({ id: 'a.b', title: 'A B', run: () => 42 })
    expect(run('a.b')).toBe(42)
  })

  it('run(모르는 id): 명확히 throw 한다(조용한 no-op 아님)', () => {
    expect(() => run('nope.nope')).toThrow(/알 수 없는 command id: 'nope\.nope'/)
  })

  it('인자 = 단일 객체 가방으로 전달된다(가변인자 아님)', () => {
    const spy = vi.fn((args?: Record<string, unknown>) => args?.theme)
    register({ id: 'theme.set', title: 'set', run: spy })
    const result = run('theme.set', { theme: 'light', extra: 1 })
    expect(spy).toHaveBeenCalledTimes(1)
    expect(spy).toHaveBeenCalledWith({ theme: 'light', extra: 1 })
    expect(result).toBe('light')
  })

  it('run(args 없음): handler 는 undefined 를 받는다', () => {
    const spy = vi.fn(() => 'ok')
    register({ id: 'x', title: 'x', run: spy })
    run('x')
    expect(spy).toHaveBeenCalledWith(undefined)
  })

  it('Promise 반환 handler: run 이 그 Promise 를 그대로 반환(await 가능)', async () => {
    register({ id: 'async.cmd', title: 'async', run: () => Promise.resolve('done') })
    const result = run('async.cmd')
    expect(result).toBeInstanceOf(Promise)
    await expect(result as Promise<string>).resolves.toBe('done')
  })

  it('중복 id: warn 하지만 그래도 등록(마지막이 이김)', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    register({ id: 'dup', title: 'first', run: () => 1 })
    register({ id: 'dup', title: 'second', run: () => 2 })
    expect(warn).toHaveBeenCalledOnce()
    expect(warn.mock.calls[0][0]).toContain("'dup'")
    expect(run('dup')).toBe(2)
  })

  it('list: 등록된 command 의 메타 스냅샷 반환(run 함수 제외)', () => {
    register({ id: 'c1', title: 'C1', category: 'cat', keybinding: 'Ctrl+K', run: () => {} })
    register({ id: 'c2', title: 'C2', run: () => {} })
    const items = list()
    expect(items).toHaveLength(2)
    const c1 = items.find(i => i.id === 'c1')!
    expect(c1).toEqual({ id: 'c1', title: 'C1', category: 'cat', keybinding: 'Ctrl+K' })
    expect('run' in c1).toBe(false)
    const c2 = items.find(i => i.id === 'c2')!
    expect(c2.category).toBeUndefined()
  })

  it('list: 빈 레지스트리는 빈 배열', () => {
    expect(list()).toEqual([])
  })

  it('getCommand: 사본을 반환 → cmd.run 변조가 레지스트리로 새지 않는다(FIX-C)', () => {
    register({ id: 'guarded', title: 'g', run: () => 'original' })
    getCommand('guarded')!.run = () => 'hijacked'
    expect(run('guarded')).toBe('original')
    expect(getCommand('guarded')!.id).toBe('guarded')
    expect(getCommand('없음')).toBeUndefined()
  })
})
