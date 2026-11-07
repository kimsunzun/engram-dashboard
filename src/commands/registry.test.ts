
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { __resetRegistryForTest, getCommand, list, register, run, runAsHuman } from './registry'

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

  // ★help must be carried in the projection for there to be a way onto the bus★ (TRD §6 Step 4): the shell unfolds this value into a catalog entry
  //   and puts it on the daemon registry. If missing, only the name is registered and a caller that finds it has no material to fill the arguments (ADR-0156).
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

  // ★The snapshot contract breaks most easily at the help field★: with value fields alone a shallow copy was enough, but
  //   help is two levels of object (args → enum array), so handing it over as is lets a consumer directly edit a registered command.
  //   toEqual alone passes even with an alias, so we measure by **re-reading after tampering**.
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

// ── humanOnly — a door closed on the caller axis (2026-09-08) ─────────────────────────
//
// One registry entry serves both the human menu and the LLM surface. An action that must be open to only one of the two cannot be
// told apart by `when` (`when` does not know the caller, and neither entry point reads it) — so the entry point was split in two, and this section
// is the place that measures that split.
describe('humanOnly 게이트', () => {
  it('run(LLM 진입점): 사유를 실어 throw 하고 handler 를 부르지 않는다', () => {
    const spy = vi.fn(() => 'ran')
    register({ id: 'closed.door', title: 'c', humanOnly: '사람이 지나야 하는 문이다', run: spy })
    expect(() => run('closed.door')).toThrow(/사람이 지나야 하는 문이다/)
    expect(spy).not.toHaveBeenCalled()
  })

  it('runAsHuman(사람 진입점): 같은 항목을 그대로 실행한다', () => {
    const spy = vi.fn(() => 'ran')
    register({ id: 'closed.door', title: 'c', humanOnly: '사람이 지나야 하는 문이다', run: spy })
    expect(runAsHuman('closed.door')).toBe('ran')
    expect(spy).toHaveBeenCalledTimes(1)
  })

  it('humanOnly 가 없는 항목은 두 진입점이 같다', () => {
    register({ id: 'open.door', title: 'o', run: () => 'ran' })
    expect(run('open.door')).toBe('ran')
    expect(runAsHuman('open.door')).toBe('ran')
  })

  it('runAsHuman 도 모르는 id 는 throw 한다(조용한 no-op 금지)', () => {
    expect(() => runAsHuman('nope.nope')).toThrow(/알 수 없는 command id/)
  })

  it('list: humanOnly 사유를 함께 낸다(부르기 전에 닫힌 것을 알 수 있게)', () => {
    register({ id: 'closed.door', title: 'c', humanOnly: '이유', run: () => 1 })
    register({ id: 'open.door', title: 'o', run: () => 1 })
    const items = list()
    expect(items.find(i => i.id === 'closed.door')!.humanOnly).toBe('이유')
    expect(items.find(i => i.id === 'open.door')!.humanOnly).toBeUndefined()
  })
})
