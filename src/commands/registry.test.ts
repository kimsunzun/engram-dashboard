
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
    register({ id: 'demo.theme', title: 'set', run: spy })
    const result = run('demo.theme', { theme: 'light', extra: 1 })
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

  // ★help 가 투영에 실려야 버스로 나갈 길이 생긴다★(TRD §6 Step 4): 셸이 이 값을 카탈로그 항목으로 펴서
  //   데몬 명부에 얹는다. 빠지면 이름만 등록돼 발견한 호출자가 인자를 채울 재료가 없다(ADR-0156).
  it('list: help 를 그대로 실어 나른다(없으면 undefined)', () => {
    const help = {
      summary: '테마를 바꾼다',
      effect: 'write' as const,
      args: { theme: { type: 'string' as const } },
    }
    register({ id: 'demo.theme', title: 'set', help, run: () => {} })
    register({ id: 'local.only', title: 'local', run: () => {} })

    const items = list()
    expect(items.find(i => i.id === 'demo.theme')!.help).toEqual(help)
    expect(items.find(i => i.id === 'local.only')!.help).toBeUndefined()
  })

  // ★스냅샷이라는 계약은 help 칸에서 가장 쉽게 깨진다★: 값 필드만 있을 땐 얕은 복사로 충분했지만
  //   help 는 객체 두 겹(args → enum 배열)이라, 그대로 넘기면 소비자가 등록된 command 를 직접 고친다.
  //   toEqual 만 쓰면 별칭이어도 통과하므로 **변조 후 다시 읽어** 잰다.
  it('list/getCommand: help 는 사본이라 소비자가 고쳐도 레지스트리가 안 바뀐다', () => {
    register({
      id: 'demo.theme',
      title: 'set',
      help: {
        summary: '원본',
        effect: 'write',
        args: { theme: { type: 'string', enum: ['dark'] } },
        required: ['theme'],
      },
      run: () => {},
    })

    const leaked = list().find(i => i.id === 'demo.theme')!.help!
    leaked.summary = '변조'
    leaked.effect = 'read'
    leaked.args!.theme.type = 'number'
    leaked.args!.theme.enum!.push('침입')
    leaked.required!.push('ghost')
    const viaGet = getCommand('demo.theme')!.help!
    viaGet.summary = 'get 으로 변조'
    viaGet.args!.theme.description = '침입'

    const fresh = list().find(i => i.id === 'demo.theme')!.help!
    expect(fresh.summary).toBe('원본')
    expect(fresh.effect).toBe('write')
    expect(fresh.args!.theme).toEqual({ type: 'string', enum: ['dark'] })
    expect(fresh.required).toEqual(['theme'])
  })

  it('getCommand: 사본을 반환 → cmd.run 변조가 레지스트리로 새지 않는다(FIX-C)', () => {
    register({ id: 'guarded', title: 'g', run: () => 'original' })
    getCommand('guarded')!.run = () => 'hijacked'
    expect(run('guarded')).toBe('original')
    expect(getCommand('guarded')!.id).toBe('guarded')
    expect(getCommand('없음')).toBeUndefined()
  })
})
